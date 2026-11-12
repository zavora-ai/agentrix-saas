//! Federation, offline: our issuer's tokens, each cloud's token shape, and every refusal.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use serde_json::{Map, Value, json};

use crate::*;

// Hosts come from the node's `platform_host` setting; never fixed in the crate.
const PLATFORM: &str = "paypal.example.test";

fn host() -> String {
    site_host("kilele-bakery-sample", PLATFORM)
}

fn did() -> String {
    saas_core::identity::agent_did(&host(), "cashier")
}

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

async fn trusting(issuer: &SiteIssuer, kind: CloudKind) -> Federation {
    Federation::connect(vec![TrustedIssuer {
        kind,
        issuer: issuer.issuer().to_owned(),
        keys: Keys::Pinned {
            jwks: issuer.jwks(),
        },
        label: "test".into(),
        audiences: Vec::new(),
    }])
    .await
    .unwrap()
}

#[test]
fn hosts_and_issuers_derive_from_the_platform_host() {
    assert_eq!(host(), "kilele-bakery-sample.paypal.example.test");
    assert_eq!(
        issuer_url(&host()),
        "https://kilele-bakery-sample.paypal.example.test"
    );
    assert_eq!(
        issuer_url(PLATFORM),
        "https://paypal.example.test",
        "Agentrix's own issuer"
    );
}

#[tokio::test]
async fn a_sites_token_names_its_agent_by_did_and_verifies_against_its_key_set() {
    let keys = std::env::temp_dir().join(format!("saas-federation-{}", uuid::Uuid::new_v4()));
    let site = SiteIssuer::for_host(&host(), &keys).unwrap();
    let d = site.discovery();
    assert_eq!(d["issuer"], site.issuer());
    assert_eq!(
        d["jwks_uri"],
        format!("{}/.well-known/jwks.json", site.issuer())
    );
    assert_eq!(d["id_token_signing_alg_values_supported"], json!(["RS256"]));
    let jwks = site.jwks();
    assert_eq!(jwks["keys"][0]["kid"], site.kid());
    assert!(
        jwks["keys"][0].get("d").is_none(),
        "only the public key is published"
    );

    // Through the trait, as saas-badge and the node hold it.
    let issuer: Arc<dyn AgentTokenIssuer> = Arc::new(SiteIssuer::for_host(&host(), &keys).unwrap()); // the same key file
    let issued = issuer
        .issue(&did(), "sts.amazonaws.com", Duration::minutes(10))
        .await
        .unwrap();
    assert_eq!(issued.kid, site.kid());
    let fed = trusting(&site, CloudKind::Agentrix).await;
    let p = fed
        .principal(&issued.token, "sts.amazonaws.com")
        .await
        .unwrap();
    assert_eq!(p.principal.id.as_str(), did(), "sub is the agent's DID");
    assert_eq!(
        (p.profile.as_str(), p.issuer.as_str(), p.subject.as_str()),
        ("agentrix", site.issuer(), did().as_str())
    );
    assert_eq!(p.audience, vec!["sts.amazonaws.com".to_owned()]);
    assert!(p.token_digest.starts_with("sha256:") && p.token_digest.len() == 7 + 64);
    assert_eq!(p.expires_at.timestamp(), issued.expires_at.timestamp());

    let extra =
        obj(json!({"agent": "urn:adk:agent:kilele-bakery/cashier", "iss": "https://evil.example"}));
    let t = site
        .issue_at(&did(), "aud", Duration::minutes(5), Utc::now(), extra)
        .unwrap();
    assert_eq!(
        fed.principal(&t.token, "aud").await.unwrap().issuer,
        site.issuer(),
        "extra claims can't replace iss"
    );
    assert!(
        site.issue_at(&did(), "aud", Duration::hours(2), Utc::now(), Map::new())
            .is_err(),
        "at most an hour"
    );
    assert!(
        site.issue_at(
            "cashier",
            "aud",
            Duration::minutes(5),
            Utc::now(),
            Map::new()
        )
        .is_err(),
        "the subject is a DID"
    );
    let _ = std::fs::remove_dir_all(keys);
}

#[tokio::test]
async fn a_key_kept_on_disk_comes_back_the_same() {
    let dir = std::env::temp_dir().join(format!("saas-federation-{}", uuid::Uuid::new_v4()));
    let a = SiteIssuer::for_host(&host(), &dir.join("keys")).unwrap();
    let b = SiteIssuer::for_host(&host(), &dir.join("keys")).unwrap();
    assert_eq!(a.kid(), b.kid());
    assert_eq!(a.issuer(), issuer_url(&host()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file = dir.join("keys").join(format!("oidc-{}.pem", host()));
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn each_host_serves_its_own_discovery_and_keys() {
    use tower::ServiceExt;
    let other_host = site_host("tana-valley-times-sample", PLATFORM);
    let kilele = Arc::new(SiteIssuer::generate(&issuer_url(&host())).unwrap());
    let other = Arc::new(SiteIssuer::generate(&issuer_url(&other_host)).unwrap());
    let app = router(HashMap::from([
        (host(), kilele.clone()),
        (other_host, other),
    ]));
    let get = |path: &str, h: &str| {
        axum::http::Request::get(path)
            .header("host", h)
            .body(axum::body::Body::empty())
            .unwrap()
    };
    let r = app
        .clone()
        .oneshot(get(
            "/.well-known/openid-configuration",
            "Kilele-Bakery-Sample.paypal.example.test:443",
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: Value =
        serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap())
            .unwrap();
    assert_eq!(body["issuer"], kilele.issuer());
    let r = app
        .clone()
        .oneshot(get("/.well-known/jwks.json", &host()))
        .await
        .unwrap();
    let keys: Value =
        serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap())
            .unwrap();
    assert_eq!(keys["keys"][0]["kid"], kilele.kid());
    assert_eq!(
        app.oneshot(get("/.well-known/jwks.json", "unknown.example.test"))
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn a_live_issuer_is_checked_through_its_discovery_document_and_key_set() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let site = SiteIssuer::generate(&server.uri()).unwrap();
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(site.discovery()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/jwks.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(site.jwks()))
        .mount(&server)
        .await;
    let fed = Federation::connect(vec![TrustedIssuer {
        kind: CloudKind::Oidc,
        issuer: server.uri(),
        keys: Keys::Discovery,
        label: "a live OIDC issuer".into(),
        audiences: Vec::new(),
    }])
    .await
    .unwrap();
    let t = site
        .issue_at(
            &did(),
            "agentrix-door",
            Duration::minutes(5),
            Utc::now(),
            Map::new(),
        )
        .unwrap();
    let p = fed.principal(&t.token, "agentrix-door").await.unwrap();
    assert_eq!(
        p.principal.id.as_str(),
        format!("{}#{}", server.uri(), did()),
        "adk-auth's key cache fetched the issuer's keys"
    );
    assert_eq!(fed.label(&server.uri()), Some("a live OIDC issuer"));
    assert!(matches!(
        fed.principal(&t.token, "someone-else").await,
        Err(TokenRefused::WrongAudience(_))
    ));
}

#[tokio::test]
async fn each_clouds_token_shape_maps_onto_an_agent_principal() {
    let (fed, tokens) = judge::stand_ins(Utc::now()).await.unwrap();
    assert_eq!(tokens.len(), 3);
    for t in &tokens {
        let fp = fed
            .principal(&t.token, &t.audience)
            .await
            .unwrap_or_else(|e| panic!("{:?}: {e}", t.kind));
        assert_eq!(fp.profile, t.kind.profile());
        assert!(
            fed.label(&fp.issuer)
                .is_some_and(|l| l.contains("stand-in")),
            "judge mode says it's a stand-in"
        );
        let p = &fp.principal;
        match t.kind {
            CloudKind::EntraAgentId => {
                // The validated live shape: no agent-marking claims for a custom audience.
                assert_eq!(
                    p.id.as_str(),
                    "urn:entra:00000000-0000-4000-8000-00000000a11e:agent:a9e1b2c3-0000-4000-8000-0000000000a9"
                );
                assert_eq!(
                    p.tenant.as_deref(),
                    Some("00000000-0000-4000-8000-00000000a11e")
                );
                assert_eq!(p.class, None, "no blueprint claim for a custom audience");
                let attr = |k: &str| p.attributes.get(k).map(String::as_str);
                assert_eq!(
                    attr("entra.agent_identity_id"),
                    Some("a9e1b2c3-0000-4000-8000-0000000000a9")
                );
                assert_eq!(attr("entra.token_version"), Some("2.0"));
                assert_eq!(attr("entra.agent_identity_facet"), None);
            }
            CloudKind::AwsAgentCore => {
                assert_eq!(
                    p.id.as_str(),
                    "urn:aws:agentcore:cognito-idp.us-east-1.amazonaws.com:us-east-1_AgEnTcOrE:7agentcorefixtureclient0001"
                );
                assert_eq!(p.class.as_deref(), Some("agentcore"));
            }
            CloudKind::GoogleAgentIdentity => {
                assert_eq!(
                    p.id.as_str(),
                    "spiffe://agents.global.org-000000000000.system.id.goog/resources/aiplatform/projects/987654321098/locations/us-central1/reasoningEngines/4242",
                    "the SPIFFE id, unchanged"
                );
                assert_eq!(
                    p.tenant.as_deref(),
                    Some("agents.global.org-000000000000.system.id.goog")
                );
                let attr = |k: &str| p.attributes.get(k).map(String::as_str);
                assert_eq!(attr("google.service"), Some("aiplatform"));
                assert_eq!(attr("google.resource_type"), Some("reasoningEngines"));
                assert_eq!(attr("google.resource_id"), Some("4242"));
                assert_eq!(attr("google.bound_token"), Some("false"));
            }
            CloudKind::Agentrix | CloudKind::Oidc => unreachable!(),
        }
    }
    // Judge mode replays tokens at their own time.
    let t = &tokens[0];
    assert!(matches!(
        fed.principal_at(&t.token, &t.audience, Utc::now() + Duration::hours(2))
            .await,
        Err(TokenRefused::Expired)
    ));
    assert!(
        fed.principal_at(&t.token, &t.audience, Utc::now() + Duration::minutes(30))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn the_door_refuses_what_it_shouldnt_trust() {
    let entra_iss = "https://login.microsoftonline.com/00000000-0000-4000-8000-00000000a11e/v2.0";
    let aud = "api://agentrix-door";
    let cloud = SiteIssuer::generate(entra_iss).unwrap();
    let fed = trusting(&cloud, CloudKind::EntraAgentId).await;
    let now = Utc::now().timestamp();
    let claims = |extra: Value| {
        let mut c = obj(
            json!({"sub": "s", "tid": "t1", "azp": "a1", "aud": aud, "iat": now, "exp": now + 600}),
        );
        c.extend(obj(extra));
        c
    };
    let sign = |extra: Value| cloud.sign_claims(claims(extra)).unwrap();
    assert!(
        fed.principal(&sign(json!({"xms_act_fct": "3"})), aud)
            .await
            .is_ok()
    );
    let plain = fed.principal(&sign(json!({})), aud).await.unwrap();
    assert_eq!(
        plain.principal.id.as_str(),
        "urn:entra:t1:agent:a1",
        "facets are optional (validated live)"
    );
    let marked = fed
        .principal(
            &sign(json!({"xms_act_fct": "3 11 9", "xms_par_app_azp": "bp1"})),
            aud,
        )
        .await
        .unwrap();
    assert_eq!(
        marked.principal.class.as_deref(),
        Some("bp1"),
        "the blueprint is the class"
    );
    assert_eq!(
        marked
            .principal
            .attributes
            .get("entra.agent_identity_facet")
            .map(String::as_str),
        Some("true")
    );
    assert!(
        matches!(
            fed.principal(&sign(json!({"idtyp": "user"})), aud).await,
            Err(TokenRefused::NotAnAgent(_))
        ),
        "a token Entra marks as a user's"
    );
    assert!(matches!(
        fed.principal(
            &sign(json!({"xms_act_fct": "3", "aud": "api://other"})),
            aud
        )
        .await,
        Err(TokenRefused::WrongAudience(_))
    ));
    assert!(
        matches!(
            fed.principal(&sign(json!({"xms_act_fct": "3", "exp": now - 120})), aud)
                .await,
            Err(TokenRefused::Expired)
        ),
        "past the usual 60 s of leeway"
    );

    let impostor = SiteIssuer::generate(entra_iss).unwrap(); // same issuer name, a key it never published
    assert!(matches!(
        fed.principal(
            &impostor
                .sign_claims(claims(json!({"xms_act_fct": "3"})))
                .unwrap(),
            aud
        )
        .await,
        Err(TokenRefused::BadSignature)
    ));
    let stranger = SiteIssuer::generate("https://idp.stranger.example").unwrap();
    assert!(matches!(
        fed.principal(
            &stranger
                .sign_claims(claims(json!({"xms_act_fct": "3"})))
                .unwrap(),
            aud
        )
        .await,
        Err(TokenRefused::UntrustedIssuer(_))
    ));

    let good = sign(json!({"xms_act_fct": "3"}));
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let mut parts: Vec<String> = good.split('.').map(str::to_owned).collect();
    let mut payload: Value =
        serde_json::from_slice(&base64::Engine::decode(&b64, &parts[1]).unwrap()).unwrap();
    payload["azp"] = json!("someone-else");
    parts[1] = base64::Engine::encode(&b64, serde_json::to_vec(&payload).unwrap());
    assert!(
        matches!(
            fed.principal(&parts.join("."), aud).await,
            Err(TokenRefused::BadSignature)
        ),
        "a changed claim breaks the signature"
    );

    // alg "none", and HS256 keyed with the public key: neither is RS256.
    let header = |alg: &str| {
        base64::Engine::encode(
            &b64,
            format!(r#"{{"alg":"{alg}","kid":"{}"}}"#, cloud.kid()),
        )
    };
    let payload = good.split('.').nth(1).unwrap();
    assert!(
        fed.principal(&format!("{}.{payload}.", header("none")), aud)
            .await
            .is_err()
    );
    assert!(matches!(
        fed.principal(
            &format!(
                "{}.{payload}.{}",
                header("HS256"),
                good.split('.').nth(2).unwrap()
            ),
            aud
        )
        .await,
        Err(TokenRefused::BadSignature)
    ));
    assert!(matches!(
        fed.principal("not-a-token", aud).await,
        Err(TokenRefused::Malformed(_))
    ));
}

#[tokio::test]
async fn a_site_speaks_only_for_its_own_agents() {
    let kilele = SiteIssuer::generate(&issuer_url(&host())).unwrap();
    let fed = trusting(&kilele, CloudKind::Agentrix).await;
    let own = kilele
        .issue_at(&did(), "door", Duration::minutes(5), Utc::now(), Map::new())
        .unwrap();
    assert_eq!(
        fed.principal(&own.token, "door")
            .await
            .unwrap()
            .principal
            .id
            .as_str(),
        did()
    );
    let other =
        saas_core::identity::agent_did(&site_host("tana-valley-times-sample", PLATFORM), "cashier");
    let claimed = kilele
        .issue_at(&other, "door", Duration::minutes(5), Utc::now(), Map::new())
        .unwrap();
    assert!(
        matches!(
            fed.principal(&claimed.token, "door").await,
            Err(TokenRefused::UntrustedIssuer(_))
        ),
        "Kilele's issuer can't speak for Tana's agent"
    );
    let site_did = kilele
        .issue_at(
            &saas_core::identity::site_did(&host()),
            "door",
            Duration::minutes(5),
            Utc::now(),
            Map::new(),
        )
        .unwrap();
    assert!(
        matches!(
            fed.principal(&site_did.token, "door").await,
            Err(TokenRefused::NotAnAgent(_))
        ),
        "a site's DID isn't an agent's"
    );
}

mod exchanges {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// The claims of the JWT in form field `field` of a recorded request.
    fn jwt_claims(req: &Request, field: &str) -> Value {
        let form: HashMap<String, String> = url_form(&req.body);
        let token = &form[field];
        let payload = token.split('.').nth(1).unwrap();
        serde_json::from_slice(
            &base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
                .unwrap(),
        )
        .unwrap()
    }

    fn url_form(body: &[u8]) -> HashMap<String, String> {
        String::from_utf8_lossy(body)
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (decode(k), decode(v)))
            .collect()
    }

    fn decode(s: &str) -> String {
        let s = s.replace('+', " ");
        let mut out = Vec::new();
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn site() -> SiteIssuer {
        SiteIssuer::generate(&issuer_url(&host())).unwrap()
    }

    #[tokio::test]
    async fn aws_assume_role_with_web_identity_names_the_agent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("Action=AssumeRoleWithWebIdentity"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"AssumeRoleWithWebIdentityResponse": {"AssumeRoleWithWebIdentityResult": {
                "Credentials": {"AccessKeyId": "ASIAFIXTURE", "SecretAccessKey": "s3cr3t", "SessionToken": "sess", "Expiration": 1_900_000_000.0},
                "SubjectFromWebIdentityToken": did(),
                "AssumedRoleUser": {"Arn": "arn:aws:sts::000000000000:assumed-role/agentrix-cashier/cashier"},
            }}})))
            .mount(&server)
            .await;
        let aws = AwsWebIdentity {
            role_arn: "arn:aws:iam::000000000000:role/agentrix-cashier".into(),
            endpoint: server.uri(),
        };
        let creds = aws
            .exchange(&reqwest::Client::new(), &site(), &did(), "cashier")
            .await
            .unwrap();
        assert_eq!(creds.subject, did());
        assert!(
            !format!("{creds:?}").contains("s3cr3t"),
            "secrets never print"
        );
        let req = &server.received_requests().await.unwrap()[0];
        let claims = jwt_claims(req, "WebIdentityToken");
        assert_eq!(claims["aud"], AWS_STS_AUDIENCE);
        assert_eq!(claims["sub"], did());
    }

    #[tokio::test]
    async fn google_sts_takes_the_https_audience_in_the_token_and_the_slashes_form_in_the_request()
    {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token": "ya29.fixture", "expires_in": 3599, "token_type": "Bearer"}),
            ))
            .mount(&server)
            .await;
        let mut google = GoogleWorkloadIdentity::new("123456789012", "agentrix", "kilele");
        google.endpoint = format!("{}/v1/token", server.uri());
        let token = google
            .exchange(&reqwest::Client::new(), &site(), &did())
            .await
            .unwrap();
        assert_eq!(token.access_token.expose(), "ya29.fixture");
        let req = &server.received_requests().await.unwrap()[0];
        let form = url_form(&req.body);
        assert_eq!(
            form["audience"],
            "//iam.googleapis.com/projects/123456789012/locations/global/workloadIdentityPools/agentrix/providers/kilele"
        );
        assert_eq!(
            jwt_claims(req, "subject_token")["aud"],
            google.token_audience()
        );
        assert!(
            google
                .token_audience()
                .starts_with("https://iam.googleapis.com/projects/123456789012/")
        );
    }

    #[tokio::test]
    async fn entra_exchanges_twice_blueprint_then_agent_identity() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tenant1/oauth2/v2.0/token"))
            .and(body_string_contains("fmi_path"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "blueprint-token", "expires_in": 3600})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/tenant1/oauth2/v2.0/token"))
            .and(body_string_contains("client_assertion=blueprint-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"access_token": "agent-identity-token", "expires_in": 3599}),
                ),
            )
            .mount(&server)
            .await;
        let mut entra = EntraAgentIdentity::new("tenant1", "blueprint-app", "agent-identity");
        entra.authority = server.uri();
        let t = entra
            .exchange(
                &reqwest::Client::new(),
                &site(),
                &did(),
                "https://graph.microsoft.com",
            )
            .await
            .unwrap();
        assert_eq!(t.access_token.expose(), "agent-identity-token");
        assert_eq!(t.scope, "https://graph.microsoft.com/.default");
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2, "two exchanges");
        let first = url_form(&reqs[0].body);
        assert_eq!(
            (first["client_id"].as_str(), first["fmi_path"].as_str()),
            ("blueprint-app", "agent-identity")
        );
        assert_eq!(
            jwt_claims(&reqs[0], "client_assertion")["aud"],
            ENTRA_ASSERTION_AUDIENCE
        );
        assert_eq!(jwt_claims(&reqs[0], "client_assertion")["sub"], did());
        let second = url_form(&reqs[1].body);
        assert_eq!(
            (
                second["client_id"].as_str(),
                second["client_assertion"].as_str()
            ),
            ("agent-identity", "blueprint-token")
        );
    }

    #[tokio::test]
    async fn a_clouds_refusal_is_reported_without_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_client", "error_description": "AADSTS700213: No matching federated identity record found"})))
            .mount(&server)
            .await;
        let mut entra = EntraAgentIdentity::new("tenant1", "blueprint-app", "agent-identity");
        entra.authority = server.uri();
        let err = entra
            .exchange(
                &reqwest::Client::new(),
                &site(),
                &did(),
                "https://graph.microsoft.com",
            )
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("AADSTS700213"), "{text}");
        assert!(!text.contains("eyJ"), "the assertion is never in the error");
    }
}

/// An issuer whose tokens are addressed to something other than our URL (Microsoft Entra: the
/// app registration's client id or `api://` URI) is trusted for exactly its configured
/// audiences; an issuer with none keeps the door's own origin.
#[tokio::test]
async fn configured_audiences_replace_the_doors_origin_for_that_issuer() {
    let entra_iss = "https://login.microsoftonline.com/00000000-0000-4000-8000-00000000a11e/v2.0";
    let door = "https://kilele-bakery-sample.paypal.agentrix.example";
    let entra = SiteIssuer::generate(entra_iss).unwrap();
    let fed = Federation::connect(vec![TrustedIssuer {
        kind: CloudKind::EntraAgentId,
        issuer: entra_iss.into(),
        keys: Keys::Pinned { jwks: entra.jwks() },
        label: "Microsoft Entra (test tenant)".into(),
        audiences: vec![
            "api://agentrix-door".into(),
            "a9e1b2c3-0000-4000-8000-0000000000a9".into(),
        ],
    }])
    .await
    .unwrap();
    let now = Utc::now().timestamp();
    let token = |aud: &str| {
        entra
            .sign_claims(obj(json!({"sub": "s", "tid": "t1", "azp": "a1", "aud": aud, "iat": now, "exp": now + 600})))
            .unwrap()
    };
    for aud in [
        "api://agentrix-door",
        "a9e1b2c3-0000-4000-8000-0000000000a9",
    ] {
        let p = fed.principal(&token(aud), door).await.unwrap();
        assert_eq!(p.issuer, entra_iss);
    }
    // The configured list replaces the door's origin for this issuer.
    assert!(matches!(
        fed.principal(&token(door), door).await,
        Err(TokenRefused::WrongAudience(_))
    ));
    assert!(matches!(
        fed.principal(&token("api://someone-else"), door).await,
        Err(TokenRefused::WrongAudience(_))
    ));

    // No list: the door's own origin, as before.
    let site =
        SiteIssuer::generate("https://kilele-bakery-sample.paypal.agentrix.example").unwrap();
    let ours = trusting(&site, CloudKind::Agentrix).await;
    let did = "did:web:kilele-bakery-sample.paypal.agentrix.example:agents:cashier";
    let for_door = site
        .issue_at(did, door, Duration::minutes(5), Utc::now(), Map::new())
        .unwrap();
    assert!(ours.principal(&for_door.token, door).await.is_ok());
    assert!(
        ours.principal(&for_door.token, "https://another.example")
            .await
            .is_err()
    );
}
