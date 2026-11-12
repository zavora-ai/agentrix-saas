//! Badges end to end over HTTP: two sites on loopback ports (a sample site and Agentrix as
//! the operator), the PayPal link built from a mock PayPal's Vault answers and sealed through
//! the `link-paypal` ceremony with passkey evidence (an Ed25519 credential shaped like a
//! WebAuthn assertion), and agents verified from their DIDs alone.

use std::sync::Arc;

use adk_auth::web_bot_auth::Ed25519SigningKey;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use saas_badge::binding::{
    BindingConfig, BindingDesk, LINK_PAYPAL, MemoryAudit, approval_matches, approved_by,
};
use saas_badge::ceremony::{Ceremonies, LOCAL_KIND, ed25519_evidence};
use saas_badge::contract::PayPalBinding;
use saas_badge::did::{agent_did, site_did};
use saas_badge::jws;
use saas_badge::resolve::{DidKeyResolver, DidWebResolver, KeyDirectoryResolver, Resolvers};
use saas_badge::site::{self, AgentSpec, Operator, SiteIdentity, SiteState};
use saas_badge::verify::{self, Verifier, names};
use saas_core::CompanyId;
use saas_core::identity::Sponsor;
use saas_settle::LoopbackDirectorySource;
use saas_settle::keys::CompanyKeys;
use saas_settle::rails::VaultRail;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RP: &str = "agentrix.website";
const ORIGIN: &str = "https://agentrix.website";

async fn listener() -> (tokio::net::TcpListener, String) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = l.local_addr().unwrap().to_string();
    (l, host)
}

fn identity(host: &str, name: &str, company: CompanyId, sample: bool, key: &str) -> SiteIdentity {
    SiteIdentity::with_keys(
        host,
        name,
        company.clone(),
        sample,
        vec![AgentSpec::cashier(&company, key)],
        CompanyKeys::generate(&format!("http://{host}"), &[key]).unwrap(),
    )
}

fn payment_token() -> Value {
    json!({
        "id": "VAULT9876", "customer": {"id": "CUST1"},
        "payment_source": {"paypal": {"email_address": "owner@kilele.example", "payer_id": "PAYER1"}},
        "links": [],
    })
}

async fn paypal() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "A21AA-test", "expires_in": 32400})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/vault/setup-tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "TOK1", "status": "PAYER_ACTION_REQUIRED", "customer": {"id": "CUST1"},
            "links": [{"rel": "approve", "href": "https://www.sandbox.paypal.com/agreements/approve?approval_session_id=TOK1"}],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/vault/setup-tokens/TOK1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id": "TOK1", "status": "VAULTED"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/vault/payment-tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(payment_token()))
        .mount(&server)
        .await;
    server
}

struct World {
    kilele: Arc<SiteState>,
    operator: Operator,
    audit: Arc<MemoryAudit>,
    verifier: Arc<Verifier>,
    kilele_host: String,
    agentrix_host: String,
    passkey: Ed25519SigningKey,
    _paypal: MockServer,
}

/// The node's passkey approvals as saas-web's `HostApprovals` answers: options bound to the
/// fingerprint, and a proof naming the credential with its assertion (the same evidence the
/// ceremony takes, public key included). The owner's "passkey" here is an Ed25519 key, and the
/// browser's answer is just the challenge it was given: a wrong one is refused by the node.
struct NodeApprovals;

fn owner_passkey() -> Ed25519SigningKey {
    Ed25519SigningKey::from_seed([11; 32]).unwrap()
}

const OWNER_ACCOUNT: uuid::Uuid = uuid::Uuid::from_u128(0x0b0e_0000_0000_4000_8000_0000_0000_0001);

#[async_trait::async_trait]
impl agentrix_host::Approvals for NodeApprovals {
    async fn passkey_options(
        &self,
        _account: uuid::Uuid,
        fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<Value>> {
        Ok(Some(
            json!({ "challenge": hex::encode(fingerprint), "rpId": RP }),
        ))
    }

    async fn verify(
        &self,
        account: uuid::Uuid,
        fingerprint: [u8; 32],
        request: Value,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(account == OWNER_ACCOUNT, "not the owner");
        match request["method"].as_str() {
            Some("totp") => Ok(json!({
                "method": "totp", "account": account, "fingerprint": hex::encode(fingerprint),
                "at": chrono::Utc::now(), "step": 1,
            })),
            Some("passkey") => {
                let credential = &request["credential"];
                anyhow::ensure!(
                    credential["response"]["challenge"].as_str()
                        == Some(hex::encode(fingerprint).as_str()),
                    "the passkey signed another challenge"
                );
                let mut assertion = serde_json::to_value(ed25519_evidence(
                    &owner_passkey(),
                    &fingerprint,
                    credential["rpId"].as_str().unwrap_or(RP),
                    credential["origin"].as_str().unwrap_or(ORIGIN),
                    "passkey",
                ))?;
                if credential["rsa"] == true {
                    assertion["publicKey"] = Value::Null;
                }
                // The node's form carries no `kind`.
                assertion.as_object_mut().unwrap().remove("kind");
                Ok(json!({
                    "method": "passkey", "account": account, "fingerprint": hex::encode(fingerprint),
                    "at": chrono::Utc::now(), "nonce": assertion["nonce"],
                    "credential_id": assertion["credentialId"], "assertion": assertion,
                }))
            }
            _ => anyhow::bail!("unknown method"),
        }
    }
}

async fn world() -> World {
    world_with(None).await
}

async fn world_with(ceremony_url: Option<String>) -> World {
    let (kl, kilele_host) = listener().await;
    let (al, agentrix_host) = listener().await;
    let kilele = SiteState::new(
        identity(
            &kilele_host,
            "Kilele Bakery",
            CompanyId::kilele(),
            true,
            "payables",
        ),
        Sponsor {
            owner: "owner@kilele".into(),
            name: Some("Wanjiku".into()),
            passkey: None,
            approved_at: None,
        },
    );
    let agentrix = SiteState::new(
        identity(
            &agentrix_host,
            "Agentrix",
            CompanyId::agentrix(),
            false,
            "receivables",
        ),
        Sponsor {
            owner: "founder".into(),
            name: None,
            passkey: None,
            approved_at: None,
        },
    );
    let operator = Operator::new(agentrix.identity.clone());
    let paypal = paypal().await;
    let vault = VaultRail::new(Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "s".into(),
            webhook_id: None,
            environment: Environment::Custom(paypal.uri()),
        },
        reqwest::Client::new(),
    )));
    let audit = Arc::new(MemoryAudit::default());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let desk = BindingDesk::new(BindingConfig {
        site: kilele.clone(),
        vault,
        owner: "owner@kilele".into(),
        rp_id: RP.into(),
        audit: audit.clone(),
        pool,
        public_base: format!("http://{kilele_host}/paypal-link"),
        ceremony_url,
        approvals: Some(Arc::new(NodeApprovals)),
        owner_account: Some(OWNER_ACCOUNT),
        agents: vec![agent_did(&kilele_host, "cashier")],
        accept_local: false,
    })
    .await
    .unwrap();
    let kr = site::router(kilele.clone())
        .nest("/paypal-link", saas_badge::binding::router(desk.clone()))
        .merge(Ceremonies::new().with("kilele", LINK_PAYPAL, desk).router());
    let ar = site::router(agentrix.clone());
    tokio::spawn(async move { axum::serve(kl, kr).await.unwrap() });
    tokio::spawn(async move { axum::serve(al, ar).await.unwrap() });
    let web = DidWebResolver::new(reqwest::Client::new()).allow_loopback();
    let resolvers = Resolvers::new()
        .with(Arc::new(web.clone()))
        .with(Arc::new(DidKeyResolver))
        .with(Arc::new(KeyDirectoryResolver::new(Arc::new(
            LoopbackDirectorySource::new(),
        ))));
    let verifier = Arc::new(Verifier::new(web, resolvers, site_did(&agentrix_host)));
    World {
        kilele,
        operator,
        audit,
        verifier,
        kilele_host,
        agentrix_host,
        passkey: Ed25519SigningKey::generate().unwrap(),
        _paypal: paypal,
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

/// PayPal's Vault flow up to the pending binding: its ceremony subject.
async fn pending(w: &World) -> Value {
    let base = format!("http://{}/paypal-link", w.kilele_host);
    let start = http().get(format!("{base}/start")).send().await.unwrap();
    assert_eq!(start.status().as_u16(), 303);
    let returned: Value = http()
        .get(format!(
            "{base}/returned?approval_token_id=TOK1&approval_session_id=TOK1"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(returned["moment"], LINK_PAYPAL);
    let id = returned["id"].as_str().unwrap();
    http()
        .get(format!(
            "http://{}/owner/kilele/ceremonies/link-paypal/{id}",
            w.kilele_host
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn fp_bytes(subject: &Value) -> [u8; 32] {
    hex::decode(
        subject["fingerprint"]
            .as_str()
            .unwrap()
            .trim_start_matches("sha256:"),
    )
    .unwrap()
    .try_into()
    .unwrap()
}

async fn seal(w: &World, subject: &Value, evidence: Value) -> reqwest::Response {
    http()
        .post(format!(
            "http://{}/owner/kilele/ceremonies/link-paypal/{}",
            w.kilele_host,
            subject["id"].as_str().unwrap()
        ))
        .json(&json!({ "evidence": evidence }))
        .send()
        .await
        .unwrap()
}

/// The owner links PayPal: Vault, then the passkey ceremony.
async fn link(w: &World) -> Value {
    let subject = pending(w).await;
    assert_eq!(subject["state"], "pending");
    assert!(subject["lines"].to_string().contains("o***@kilele.example"));
    let evidence = ed25519_evidence(&w.passkey, &fp_bytes(&subject), RP, ORIGIN, "passkey");
    let sealed: Value = seal(w, &subject, serde_json::to_value(evidence).unwrap())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(sealed["sealed"], true);
    assert_eq!(sealed["auditSeq"], 1);
    sealed["record"].clone()
}

#[tokio::test]
async fn the_did_documents_resolve_from_the_site_s_own_host() {
    let w = world().await;
    let did = agent_did(&w.kilele_host, "cashier");
    let web = DidWebResolver::new(reqwest::Client::new()).allow_loopback();
    let doc = web
        .fetch_json(&format!("http://{}/agents/cashier/did.json", w.kilele_host))
        .await
        .unwrap();
    assert_eq!(doc["id"], did);
    assert_eq!(doc["controller"], site_did(&w.kilele_host));
    assert_eq!(
        doc["alsoKnownAs"],
        json!([
            "urn:adk:agent:kilele-bakery/cashier",
            "urn:adk:agent:kilele-bakery/receivables",
            "urn:adk:agent:kilele-bakery/payables"
        ])
    );
    let index: Value = web
        .fetch_json(&format!("http://{}/agents/index.json", w.kilele_host))
        .await
        .unwrap();
    assert_eq!(index[0]["did"], did);
    assert_eq!(index[0]["badge"], "/agents/cashier/badge.json");
    // HTTPS only unless loopback is allowed.
    assert!(
        DidWebResolver::new(reqwest::Client::new())
            .fetch_json(&format!("http://{}/.well-known/did.json", w.kilele_host))
            .await
            .is_err()
    );
    let missing = reqwest::get(format!("http://{}/agents/impostor/did.json", w.kilele_host))
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404);
}

#[tokio::test]
async fn a_badge_verifies_from_the_did_alone_with_its_paypal_link_and_voucher() {
    let w = world().await;
    let record = link(&w).await;
    // Every PayPal id came from PayPal's answer; the email is masked.
    assert_eq!(record["paypal"]["payment_token"], "VAULT9876");
    assert_eq!(record["paypal"]["setup_token"], "TOK1");
    assert_eq!(record["paypal"]["customer_id"], "CUST1");
    assert_eq!(record["paypal"]["payer_id"], "PAYER1");
    assert_eq!(
        record["paypal"]["payer_email_masked"],
        "o***@kilele.example"
    );
    assert_eq!(
        record["paypal"]["response_digest"],
        jws::digest(&payment_token()).unwrap()
    );
    assert_eq!(record["approval"]["kind"], "passkey");
    assert_eq!(record["approval"]["rpId"], RP);
    assert_eq!(record["passkey"], record["approval"]["credentialId"]);
    assert!(approval_matches(&record));
    jws::verify_document(&record, w.kilele.identity.keys.owner.public_key()).unwrap();
    {
        let records = w.audit.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].0, "paypal.binding.linked");
    }
    w.operator.vouch(&w.kilele).unwrap();
    w.kilele
        .set_card(json!({"name": "Kilele Bakery agent door", "url": format!("http://{}/a2a", w.kilele_host)}))
        .await
        .unwrap();

    let did = agent_did(&w.kilele_host, "cashier");
    let report = w.verifier.verify(&did).await;
    assert!(report.verified, "{:#?}", report.checks);
    assert!(report.failure.is_none());
    for expected in [
        names::DID,
        names::SIGNED,
        names::BADGE,
        names::KEY,
        names::DIRECTORY,
        names::NAMES,
        names::STATUS,
        names::SECRETS,
        names::VOUCHER,
        names::PAYPAL,
        names::CARD,
    ] {
        assert!(
            report.checks.iter().any(|c| c.check == expected && c.ok),
            "{expected}: {:#?}",
            report.checks
        );
    }
    let badge = report.badge.unwrap();
    assert_eq!(badge["credentials"]["holds_secrets"], false);
    assert_eq!(badge["credentials"]["paypal_access"], "issued_per_call");
    assert_eq!(badge["paypal"]["payment_token_tail"], "9876");
    assert_eq!(badge["paypal"]["digest"], jws::digest(&record).unwrap());
    assert_eq!(badge["sponsor"]["passkey"], record["passkey"]);
    assert_eq!(badge["id"], did);

    // The same over the verify endpoint the face proxies.
    let (l, host) = listener().await;
    let router = verify::router(w.verifier.clone());
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    let url =
        reqwest::Url::parse_with_params(&format!("http://{host}/agents/verify"), &[("did", &did)])
            .unwrap();
    let over_http: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
    assert_eq!(over_http["verified"], true);
    assert!(over_http["failure"].is_null());
}

#[tokio::test]
async fn frozen_impostor_and_unvouched_agents_fail_verification() {
    let w = world().await;
    w.operator.vouch(&w.kilele).unwrap();
    let did = agent_did(&w.kilele_host, "cashier");
    assert!(
        w.verifier.verify(&did).await.verified,
        "no PayPal link yet is still a valid badge"
    );

    w.kilele.set_frozen(true);
    let frozen = w.verifier.verify(&did).await;
    assert!(!frozen.verified);
    assert_eq!(frozen.failure.as_ref().unwrap().check, names::STATUS);
    w.kilele.set_frozen(false);

    // The owner revokes one agent: its badge says so and fails; restored, it verifies again.
    w.kilele.set_revoked("cashier", true);
    let revoked = w.verifier.verify(&did).await;
    assert!(!revoked.verified);
    let failure = revoked.failure.as_ref().unwrap();
    assert_eq!(failure.check, names::STATUS);
    assert!(failure.message.contains("revoked"), "{failure:?}");
    assert_eq!(revoked.badge.as_ref().unwrap()["status"], "revoked");
    w.kilele.set_revoked("cashier", false);
    assert!(w.verifier.verify(&did).await.verified);

    assert!(
        !w.verifier
            .verify(&agent_did(&w.kilele_host, "impostor"))
            .await
            .verified
    );

    // A voucher from anyone but the trusted operator's key doesn't count.
    let rogue = Operator::new(identity(
        &w.agentrix_host,
        "Rogue",
        CompanyId::agentrix(),
        false,
        "receivables",
    ));
    rogue.vouch(&w.kilele).unwrap();
    let unvouched = w.verifier.verify(&did).await;
    assert_eq!(unvouched.failure.unwrap().check, names::VOUCHER);
    assert!(!w.verifier.verify("did:web:example.invalid").await.verified);
}

#[tokio::test]
async fn the_ceremony_refuses_evidence_that_doesn_t_check_and_seals_nothing() {
    let w = world().await;
    let subject = pending(&w).await;
    let fp = fp_bytes(&subject);

    // Evidence for another fingerprint: the subject changed since it was shown.
    let other = ed25519_evidence(&w.passkey, &[0u8; 32], RP, ORIGIN, "passkey");
    assert_eq!(
        seal(&w, &subject, serde_json::to_value(other).unwrap())
            .await
            .status()
            .as_u16(),
        409
    );

    // A signature by another key than the credential's.
    let mut forged =
        serde_json::to_value(ed25519_evidence(&w.passkey, &fp, RP, ORIGIN, "passkey")).unwrap();
    let stranger = Ed25519SigningKey::generate().unwrap();
    let theirs =
        serde_json::to_value(ed25519_evidence(&stranger, &fp, RP, ORIGIN, "passkey")).unwrap();
    forged["signature"] = theirs["signature"].clone();
    assert_eq!(seal(&w, &subject, forged).await.status().as_u16(), 403);

    // Another RP's passkey.
    let elsewhere = ed25519_evidence(
        &w.passkey,
        &fp,
        "evil.example",
        "https://evil.example",
        "passkey",
    );
    assert_eq!(
        seal(&w, &subject, serde_json::to_value(elsewhere).unwrap())
            .await
            .status()
            .as_u16(),
        403
    );

    // A local stand-in isn't a passkey on a desk that doesn't accept it.
    let local = ed25519_evidence(&w.passkey, &fp, RP, ORIGIN, LOCAL_KIND);
    assert_eq!(
        seal(&w, &subject, serde_json::to_value(local).unwrap())
            .await
            .status()
            .as_u16(),
        403
    );

    assert!(w.kilele.binding().is_none());
    assert!(w.audit.records.lock().unwrap().is_empty());
    let still: Value = http()
        .get(format!(
            "http://{}/owner/kilele/ceremonies/link-paypal/{}",
            w.kilele_host,
            subject["id"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(still["state"], "pending");

    // The owner's own passkey seals it; sealing again returns the same record.
    let good =
        serde_json::to_value(ed25519_evidence(&w.passkey, &fp, RP, ORIGIN, "passkey")).unwrap();
    let first: Value = seal(&w, &subject, good.clone()).await.json().await.unwrap();
    let again: Value = seal(&w, &subject, good).await.json().await.unwrap();
    assert_eq!(first["record"], again["record"]);
    assert_eq!(w.audit.records.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_approval_for_other_terms_doesn_t_carry_over() {
    let w = world().await;
    let mut record = link(&w).await;
    assert!(approval_matches(&record));
    record["paypal"]["payment_token"] = json!("SOMEONE-ELSES");
    assert!(!approval_matches(&record));
}

/// The face's flow: PayPal's return lands on the console page with `?binding=`, the page reads
/// the draft, and the owner's passkey goes through the node's `Approvals`; the proof it returns
/// is sealed into the binding, and the badge verifies on both verify paths.
#[tokio::test]
async fn the_console_links_paypal_through_the_node_s_passkey_approvals() {
    let w = world_with(Some("/console/kilele/link-paypal".into())).await;
    let base = format!("http://{}/paypal-link", w.kilele_host);
    assert_eq!(
        http()
            .get(format!("{base}/start"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        303
    );
    let returned = http()
        .get(format!("{base}/returned?approval_token_id=TOK1"))
        .send()
        .await
        .unwrap();
    assert_eq!(returned.status().as_u16(), 303);
    let location = returned.headers()["location"].to_str().unwrap().to_owned();
    let id = location
        .strip_prefix("/console/kilele/link-paypal?binding=")
        .unwrap()
        .to_owned();
    assert!(
        id.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    );

    let draft: Value = http()
        .get(format!("{base}/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(draft["state"], "awaiting_passkey");
    assert_eq!(draft["draft"]["paypal"]["payment_token"], "VAULT9876");
    assert_eq!(
        draft["draft"]["paypal"]["payer_email_masked"],
        "o***@kilele.example"
    );
    assert_eq!(
        draft["draft"]["agents"][0],
        agent_did(&w.kilele_host, "cashier")
    );
    let fp = draft["fingerprint"].as_str().unwrap().to_owned();
    assert_eq!(draft["passkey_options"]["challenge"], fp.as_str());
    assert!(draft["signed"].is_null());

    let approve = |body: Value| {
        let url = format!("{base}/{id}/approve");
        async move { http().post(url).json(&body).send().await.unwrap() }
    };
    // An authenticator code isn't enough to link PayPal.
    let totp = approve(json!({"method": "totp", "code": "123456"})).await;
    assert_eq!(totp.status().as_u16(), 403);
    let message: Value = totp.json().await.unwrap();
    assert!(message["message"].as_str().unwrap().contains("passkey"));
    // A passkey that signed another challenge is refused by the node.
    let other =
        approve(json!({"method": "passkey", "credential": {"response": {"challenge": "00"}}}))
            .await;
    assert_eq!(other.status().as_u16(), 403);
    // A passkey registered on another console isn't this owner's.
    let elsewhere = approve(json!({"method": "passkey", "credential": {
        "rpId": "money.agentrix.website", "origin": "https://money.agentrix.website",
        "response": {"challenge": fp},
    }}))
    .await;
    assert_eq!(elsewhere.status().as_u16(), 403);
    let message: Value = elsewhere.json().await.unwrap();
    assert!(
        message["message"]
            .as_str()
            .unwrap()
            .contains("not this console"),
        "{message}"
    );
    assert!(w.audit.records.lock().unwrap().is_empty());
    assert!(w.kilele.binding().is_none());

    let linked =
        approve(json!({"method": "passkey", "credential": {"response": {"challenge": fp}}})).await;
    assert_eq!(linked.status().as_u16(), 200);
    let body: Value = linked.json().await.unwrap();
    let record = body["binding"].clone();
    let credential_id =
        ed25519_evidence(&owner_passkey(), &[0; 32], RP, ORIGIN, "passkey").credential_id;
    assert_eq!(record["passkey"], credential_id.as_str());
    assert_eq!(record["approval"]["method"], "passkey");
    assert_eq!(record["approval"]["fingerprint"], fp.as_str());
    assert_eq!(
        record["approval"]["assertion"]["credentialId"],
        credential_id.as_str()
    );
    assert!(approval_matches(&record));
    assert_eq!(approved_by(&record), "the owner's passkey");
    // The fingerprint the passkey signed is saas-core's signing digest of the sealed binding.
    let terms: PayPalBinding = serde_json::from_value(record.clone()).unwrap();
    assert_eq!(terms.signing_digest().unwrap(), format!("sha256:{fp}"));
    jws::verify_document(&record, w.kilele.identity.keys.owner.public_key()).unwrap();
    // Approving again returns the same binding and seals nothing new.
    let again: Value =
        approve(json!({"method": "passkey", "credential": {"response": {"challenge": fp}}}))
            .await
            .json()
            .await
            .unwrap();
    assert_eq!(again["binding"], record);
    assert_eq!(w.audit.records.lock().unwrap().len(), 1);
    let bound: Value = http()
        .get(format!("{base}/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bound["state"], "bound");
    assert_eq!(bound["signed"], record);
    assert!(bound["passkey_options"].is_null());

    // The proof's terms can't be swapped for others.
    let mut swapped = record.clone();
    swapped["approval"]["credential_id"] = json!("cred-2");
    assert!(!approval_matches(&swapped));
    // Its assertion re-checks offline: a changed nonce or origin breaks it.
    let mut forged = record.clone();
    forged["approval"]["assertion"]["nonce"] = json!("00".repeat(16));
    assert!(!approval_matches(&forged));
    let mut forged = record.clone();
    forged["approval"]["assertion"]["origin"] = json!("https://evil.example");
    assert!(!approval_matches(&forged));
    // With no public key to re-check against (RSA passkeys), the record says the node checked it.
    let mut rsa = record.clone();
    rsa["approval"]["assertion"]["publicKey"] = Value::Null;
    assert!(approval_matches(&rsa));
    assert!(approved_by(&rsa).contains("checked by the node"));

    w.operator.vouch(&w.kilele).unwrap();
    w.kilele
        .set_card(json!({"name": "Kilele Bakery agent door", "url": format!("http://{}/a2a", w.kilele_host)}))
        .await
        .unwrap();
    let did = agent_did(&w.kilele_host, "cashier");
    let (l, host) = listener().await;
    let router = verify::router(w.verifier.clone());
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    for path in ["/agents/verify", "/badge/verify"] {
        let url =
            reqwest::Url::parse_with_params(&format!("http://{host}{path}"), &[("did", &did)])
                .unwrap();
        let report: Value = reqwest::get(url).await.unwrap().json().await.unwrap();
        assert_eq!(report["ok"], true, "{report:#}");
        assert_eq!(report["verified"], true);
        let paypal = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == names::PAYPAL)
            .unwrap();
        assert_eq!(paypal["ok"], true);
        assert_eq!(paypal["check"], names::PAYPAL);
        assert!(paypal["detail"].as_str().unwrap().contains("passkey"));
        assert!(report["checked_at"].is_string());
    }
}

/// The ceremony page's landing: PayPal's return puts the binding id in the page's path, and the
/// page reads the subject from the ceremony route.
#[tokio::test]
async fn paypal_s_return_lands_on_the_console_s_ceremony_page() {
    let w = world_with(Some("/console/kilele/passkey/link-paypal/{id}".into())).await;
    let base = format!("http://{}/paypal-link", w.kilele_host);
    http().get(format!("{base}/start")).send().await.unwrap();
    let returned = http()
        .get(format!("{base}/returned?approval_token_id=TOK1"))
        .send()
        .await
        .unwrap();
    assert_eq!(returned.status().as_u16(), 303);
    let location = returned.headers()["location"].to_str().unwrap().to_owned();
    let id = location
        .strip_prefix("/console/kilele/passkey/link-paypal/")
        .unwrap();
    assert!(id.starts_with("bind_") && !id.contains('?'), "{location}");
    let subject: Value = http()
        .get(format!(
            "http://{}/owner/kilele/ceremonies/link-paypal/{id}",
            w.kilele_host
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(subject["state"], "pending");
    assert_eq!(subject["id"], id);
}

/// A site's token issuer (saas-federation's, here a stand-in with a fixed key set).
struct StaticIssuer {
    issuer: String,
}

#[async_trait::async_trait]
impl saas_settle::tokens::AgentTokenIssuer for StaticIssuer {
    fn issuer(&self) -> String {
        self.issuer.clone()
    }
    fn discovery(&self) -> Value {
        json!({"issuer": self.issuer, "jwks_uri": format!("{}/.well-known/jwks.json", self.issuer),
               "id_token_signing_alg_values_supported": ["RS256"]})
    }
    fn jwks(&self) -> Value {
        // A private field slipped in must never reach the DID document.
        json!({"keys": [{"kty": "RSA", "kid": "fed-1", "alg": "RS256", "use": "sig", "n": "AQAB-n", "e": "AQAB", "d": "SECRET"}]})
    }
    async fn issue(
        &self,
        _agent_did: &str,
        _audience: &str,
        _ttl: chrono::Duration,
    ) -> anyhow::Result<saas_settle::tokens::IssuedToken> {
        anyhow::bail!("not in this test")
    }
}

/// Cloud identity: the site serves its issuer's discovery and keys from its own host and its
/// DID document lists the same keys; a presented token is checked by the federation seam, and
/// one naming the site's own agent also gets that agent's badge checked.
#[tokio::test]
async fn agent_tokens_map_to_principals_and_the_site_publishes_its_issuer() {
    use saas_settle::tokens::{AgentId, AgentPrincipal, FixedTokens};
    let w = world().await;
    let origin = format!("http://{}", w.kilele_host);
    let web = DidWebResolver::new(reqwest::Client::new()).allow_loopback();

    // Before an issuer is set, nothing is served.
    let none = reqwest::get(format!("{origin}/.well-known/openid-configuration"))
        .await
        .unwrap();
    assert_eq!(none.status().as_u16(), 404);
    w.kilele.set_issuer(Some(Arc::new(StaticIssuer {
        issuer: origin.clone(),
    })));
    let discovery = web
        .fetch_json(&format!("{origin}/.well-known/openid-configuration"))
        .await
        .unwrap();
    assert_eq!(discovery["issuer"], origin);
    let jwks = web
        .fetch_json(&format!("{origin}/.well-known/jwks.json"))
        .await
        .unwrap();
    assert_eq!(jwks["keys"][0]["kid"], "fed-1");
    let site = web
        .fetch_json(&format!("{origin}/.well-known/did.json"))
        .await
        .unwrap();
    let method = format!("{}#oidc-fed-1", site_did(&w.kilele_host));
    let listed = site["verificationMethod"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == method.as_str())
        .expect("the issuer's key is in the DID document");
    assert_eq!(listed["publicKeyJwk"]["n"], "AQAB-n");
    assert!(listed["publicKeyJwk"].get("d").is_none());
    assert!(
        site["assertionMethod"]
            .as_array()
            .unwrap()
            .contains(&json!(method))
    );
    assert!(site.to_string().contains("OpenIdConnectIssuer"));

    // The Cashier's badge verifies, so a token naming it does too.
    link(&w).await;
    w.operator.vouch(&w.kilele).unwrap();
    w.kilele
        .set_card(json!({"name": "Kilele Bakery agent door", "url": format!("{origin}/a2a")}))
        .await
        .unwrap();
    let cashier = agent_did(&w.kilele_host, "cashier");
    let audience = "https://verifier.example";
    let tokens = Arc::new(FixedTokens::default());
    tokens.accept(
        "site-token",
        audience,
        AgentPrincipal::new(AgentId::new(&cashier).unwrap()),
        &origin,
        "agentrix",
    );
    let spiffe = "spiffe://agents.google.example/ns/kilele/sa/payables";
    tokens.accept(
        "cloud-token",
        audience,
        AgentPrincipal::new(AgentId::new(spiffe).unwrap()).with_tenant("kilele-prod"),
        "https://sts.googleapis.com",
        "google",
    );
    let resolvers = Resolvers::new()
        .with(Arc::new(web.clone()))
        .with(Arc::new(DidKeyResolver))
        .with(Arc::new(KeyDirectoryResolver::new(Arc::new(
            LoopbackDirectorySource::new(),
        ))));
    let verifier = Arc::new(
        Verifier::new(web, resolvers, site_did(&w.agentrix_host)).with_tokens(tokens, audience),
    );

    let ours = verifier.verify_token("site-token").await;
    assert!(ours.ok, "{ours:#?}");
    assert_eq!(
        ours.vouched_by.as_deref(),
        Some(format!("agentrix ({origin})").as_str())
    );
    assert!(ours.badge.as_ref().unwrap().verified);

    let cloud = verifier.verify_token("cloud-token").await;
    assert!(cloud.ok);
    assert!(cloud.badge.is_none());
    assert_eq!(
        cloud
            .principal
            .as_ref()
            .unwrap()
            .principal
            .tenant
            .as_deref(),
        Some("kilele-prod")
    );
    assert_eq!(
        cloud.vouched_by.as_deref(),
        Some("google (https://sts.googleapis.com)")
    );

    // Over HTTP, as the console asks.
    let (l, host) = listener().await;
    let router = verify::router(verifier.clone());
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    let refused: Value = reqwest::Client::new()
        .post(format!("http://{host}/agents/verify-token"))
        .json(&json!({"token": "forged"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refused["ok"], false);
    assert!(refused["refused"].as_str().unwrap().contains("refused"));
    assert!(refused["principal"].is_null());
    let over_http: Value = reqwest::Client::new()
        .post(format!("http://{host}/agents/verify-token"))
        .json(&json!({"token": "cloud-token"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(over_http["ok"], true);
    assert_eq!(over_http["principal"]["principal"]["id"], spiffe);
}

/// saas-federation as the real checker and issuer behind the seam: each cloud's fixture token
/// maps onto a principal for its audience and no other, and a site's own issuer is published in
/// its DID document and issues a token that a federation trusting it checks, with the agent's
/// badge verified alongside.
#[tokio::test]
async fn saas_federation_checks_the_clouds_and_issues_for_the_site() {
    use saas_federation::{CloudKind, Federation, Keys, SiteIssuer, TrustedIssuer};
    use saas_settle::tokens::{AgentTokenIssuer, AgentTokens};

    let (clouds, fixtures) = saas_federation::judge::stand_ins(chrono::Utc::now())
        .await
        .unwrap();
    let mut profiles = Vec::new();
    for fixture in &fixtures {
        let p = clouds
            .principal(&fixture.token, &fixture.audience)
            .await
            .unwrap();
        assert!(!p.principal.id.as_str().is_empty());
        profiles.push(p.profile);
        assert!(
            clouds
                .principal(&fixture.token, "https://someone-else.example")
                .await
                .is_err()
        );
    }
    profiles.sort();
    assert_eq!(profiles.len(), 3);

    let w = world().await;
    let origin = format!("http://{}", w.kilele_host);
    let issuer = Arc::new(SiteIssuer::generate(&origin).unwrap());
    w.kilele.set_issuer(Some(issuer.clone()));
    let web = DidWebResolver::new(reqwest::Client::new()).allow_loopback();
    let site = web
        .fetch_json(&format!("{origin}/.well-known/did.json"))
        .await
        .unwrap();
    let method = format!("{}#oidc-{}", site_did(&w.kilele_host), issuer.kid());
    assert!(
        site["assertionMethod"]
            .as_array()
            .unwrap()
            .contains(&json!(method))
    );

    link(&w).await;
    w.operator.vouch(&w.kilele).unwrap();
    w.kilele
        .set_card(json!({"name": "Kilele Bakery agent door", "url": format!("{origin}/a2a")}))
        .await
        .unwrap();
    let cashier = agent_did(&w.kilele_host, "cashier");
    let audience = "sts.amazonaws.com";
    let issued = AgentTokenIssuer::issue(
        issuer.as_ref(),
        &cashier,
        audience,
        chrono::Duration::minutes(5),
    )
    .await
    .unwrap();
    let trusting = Federation::connect(vec![TrustedIssuer {
        kind: CloudKind::Oidc,
        issuer: AgentTokenIssuer::issuer(issuer.as_ref()),
        keys: Keys::Pinned {
            jwks: issuer.jwks(),
        },
        label: "Kilele Bakery (sample)".into(),
        audiences: Vec::new(),
    }])
    .await
    .unwrap();
    let resolvers = Resolvers::new()
        .with(Arc::new(web.clone()))
        .with(Arc::new(DidKeyResolver))
        .with(Arc::new(KeyDirectoryResolver::new(Arc::new(
            LoopbackDirectorySource::new(),
        ))));
    let verifier = Verifier::new(web, resolvers, site_did(&w.agentrix_host))
        .with_tokens(Arc::new(trusting), audience);
    let report = verifier.verify_token(&issued.token).await;
    assert!(report.ok, "{report:#?}");
    assert_eq!(report.principal.as_ref().unwrap().subject, cashier);
    assert!(report.badge.as_ref().unwrap().verified);

    // Another issuer claiming to speak for the same agent is refused, even when trusted.
    let impostor = Arc::new(SiteIssuer::generate("https://impostor.example").unwrap());
    let forged = AgentTokenIssuer::issue(
        impostor.as_ref(),
        &cashier,
        audience,
        chrono::Duration::minutes(5),
    )
    .await
    .unwrap();
    let both = Federation::connect(vec![TrustedIssuer {
        kind: CloudKind::Oidc,
        issuer: "https://impostor.example".into(),
        keys: Keys::Pinned {
            jwks: impostor.jwks(),
        },
        label: "Impostor".into(),
        audiences: Vec::new(),
    }])
    .await
    .unwrap();
    let web = DidWebResolver::new(reqwest::Client::new()).allow_loopback();
    let resolvers = Resolvers::new().with(Arc::new(web.clone()));
    let fooled = Verifier::new(web, resolvers, site_did(&w.agentrix_host))
        .with_tokens(Arc::new(both), audience)
        .verify_token(&forged.token)
        .await;
    assert!(!fooled.ok);
    assert!(
        fooled
            .refused
            .as_deref()
            .unwrap()
            .contains("not that agent's site"),
        "{fooled:#?}"
    );
}
