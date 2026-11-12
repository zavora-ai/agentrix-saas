//! The PayPal binding (specs/saas/v3.md, technology 2, "Link PayPal"): the record that links a
//! site's agents to the PayPal account its owner saved, approved once with the owner's passkey.
//!
//! 1. **Start.** The owner asks to link PayPal; PayPal issues a setup token and an approval
//!    link (Vault, through saas-settle's rail), and the owner approves on PayPal's own page.
//! 2. **Returned.** PayPal sends the owner back; the setup token becomes a saved payment
//!    method, and the pending binding is built from **PayPal's own response**: the vault id,
//!    customer, payer id and masked email, with the call and the response's digest as
//!    evidence. No id is ever typed in. The owner goes on to the face's passkey page.
//! 3. **Seal** (the `link-paypal` ceremony, [`crate::ceremony`]): the face shows the subject
//!    and its fingerprint, runs the passkey and posts its evidence; the evidence is checked
//!    here offline, the binding is signed with the site's `owner` key, written to the signed
//!    audit log, and shown in every badge of the site.
//!
//! HTTP ([`router`], mounted at the `public_base` PayPal returns to): `GET start` (303 to
//! PayPal), `GET returned?approval_token_id=` (303 to the face's page), `GET cancelled`.

use std::sync::{Arc, Mutex};

use agentrix_host::Approvals;
use async_trait::async_trait;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Utc};
use saas_settle::rails::{VaultRail, VaultedWallet};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::ceremony::{
    Ceremony, CeremonyError, PasskeyEvidence, SealedRecord, Subject, check_evidence,
};
use crate::contract::{BindingStatus, PayPalBinding, PayPalVaultIds};
use crate::jws;
use crate::site::SiteState;

/// The name of this crate's migrations table.
pub const MIGRATIONS_TABLE: &str = "_sqlx_migrations_saas_badge";
/// The moment's name in the ceremony routes.
pub const LINK_PAYPAL: &str = "link-paypal";
/// How long the owner has to approve a pending binding.
pub const PENDING_MINUTES: i64 = 30;

/// Applies this crate's migrations (1800–1809) to `pool`.
pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
    migrator.set_ignore_missing(true);
    migrator.run(pool).await?;
    Ok(())
}

/// Where a signed record goes: the company's signed audit log (adk-auth's Merkle log) on the
/// node. Returns the record's sequence in the log.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Appends `record` as `kind`.
    async fn append(&self, kind: &str, record: &Value) -> anyhow::Result<u64>;
}

/// An audit log in memory (tests, local runs).
#[derive(Debug, Default)]
pub struct MemoryAudit {
    /// Everything appended, in order.
    pub records: Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl AuditSink for MemoryAudit {
    async fn append(&self, kind: &str, record: &Value) -> anyhow::Result<u64> {
        let mut records = self.records.lock().unwrap_or_else(|p| p.into_inner());
        records.push((kind.to_owned(), record.clone()));
        Ok(records.len() as u64)
    }
}

/// `j***@example.com`.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((user, host)) => format!("{}***@{host}", user.chars().next().unwrap_or('*')),
        None => "***".into(),
    }
}

/// The fingerprint the owner's passkey signs: SHA-256 of the canonical JSON of the binding
/// with no approval and no credential id (both come from the assertion itself).
pub fn fingerprint(binding: &PayPalBinding) -> anyhow::Result<[u8; 32]> {
    let draft = PayPalBinding {
        approval: None,
        passkey: String::new(),
        ..binding.clone()
    };
    let digest = draft.signing_digest()?;
    let hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    hex::decode(hex)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("a SHA-256 digest is 32 bytes"))
}

/// The owner's passkey evidence a sealed binding carries.
pub fn evidence_of(binding: &PayPalBinding) -> Option<PasskeyEvidence> {
    serde_json::from_value(binding.approval.clone()?).ok()
}

/// Whether a signed binding's approval is the owner's passkey over exactly its own terms,
/// checked offline.
pub fn approval_matches(binding: &Value) -> bool {
    let Ok(terms) = serde_json::from_value::<PayPalBinding>(binding.clone()) else {
        return false;
    };
    let Ok(fp) = fingerprint(&terms) else {
        return false;
    };
    // The ceremony's evidence: re-checked offline, signature and all.
    if let Some(evidence) = evidence_of(&terms) {
        return terms.passkey == evidence.credential_id
            && check_evidence(&evidence, &fp, Some(&evidence.rp_id)).is_ok();
    }
    // The node's approval proof (agentrix-host `Approvals`): it approves exactly these terms
    // with that credential, and its assertion, when it carries the passkey's public key,
    // re-checks offline like the ceremony's evidence.
    let Some(proof) = terms.approval.as_ref() else {
        return false;
    };
    let names_these_terms = proof["method"].as_str() == Some("passkey")
        && proof["fingerprint"].as_str() == Some(hex::encode(fp).as_str())
        && proof["credential_id"].as_str() == Some(terms.passkey.as_str())
        && !terms.passkey.is_empty();
    names_these_terms
        && match proof_assertion(proof) {
            None => true,
            Some(Err(_)) => false,
            Some(Ok(assertion)) => {
                assertion.kind == "passkey"
                    && assertion.credential_id == terms.passkey
                    && check_evidence(&assertion, &fp, Some(&assertion.rp_id)).is_ok()
            }
        }
}

/// The assertion in the node's approval proof, as evidence, when it can be re-checked offline:
/// `None` when it carries no public key (RSA passkeys come without one), an error when it
/// carries one but isn't evidence.
pub fn proof_assertion(proof: &Value) -> Option<anyhow::Result<PasskeyEvidence>> {
    let mut assertion = proof.get("assertion")?.clone();
    if !assertion["publicKey"].is_object() {
        return None;
    }
    if assertion.get("kind").is_none() {
        assertion["kind"] = json!("passkey");
    }
    Some(serde_json::from_value(assertion).map_err(anyhow::Error::from))
}

/// How the owner approved a signed binding, in words.
pub fn approved_by(binding: &Value) -> String {
    let approval = &binding["approval"];
    match approval["kind"].as_str().or(approval["method"].as_str()) {
        // The node's proof with no public key to re-check its assertion against: the node
        // checked it when the owner approved; here, only that it names these terms.
        Some("passkey") if approval["kind"].is_null() && proof_assertion(approval).is_none() => {
            "the owner's passkey (assertion checked by the node)".into()
        }
        Some("passkey") => "the owner's passkey".into(),
        Some(other) => format!("the owner ({other})"),
        None => "nobody on record".into(),
    }
}

/// Builds the pending binding from PayPal's payment-token response, as received.
pub fn draft_from_paypal(
    site: &SiteState,
    setup_token: &str,
    wallet: &VaultedWallet,
    environment: &str,
    agents: Vec<String>,
    owner: &str,
) -> anyhow::Result<PayPalBinding> {
    let response = &wallet.response;
    anyhow::ensure!(
        response["id"].as_str() == Some(wallet.vault_id.as_str()),
        "PayPal's response doesn't name the saved payment method"
    );
    let paypal = &response["payment_source"]["paypal"];
    Ok(PayPalBinding {
        binding_id: format!("bind_{}", Uuid::now_v7().simple()),
        site: site.identity.did(),
        company: site.identity.company.clone(),
        environment: environment.to_owned(),
        paypal: PayPalVaultIds {
            setup_token: setup_token.to_owned(),
            payment_token: wallet.vault_id.clone(),
            customer_id: response["customer"]["id"].as_str().map(str::to_owned),
            payer_id: paypal["payer_id"].as_str().map(str::to_owned),
            payer_email_masked: paypal["email_address"].as_str().map(mask_email),
            app_id: None,
            response_digest: jws::digest(response)?,
        },
        agents,
        owner: owner.to_owned(),
        passkey: String::new(),
        approval: None,
        linked_at: Utc::now(),
        status: BindingStatus::Active,
        revoked_at: None,
    })
}

/// A pending binding as the face's page reads it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BindingDraft {
    /// The binding.
    pub binding_id: String,
    /// `awaiting_paypal`, `awaiting_passkey`, `bound` or `revoked`.
    pub state: String,
    /// The draft, once PayPal answered.
    pub draft: Option<PayPalBinding>,
    /// The hex fingerprint the owner's passkey signs.
    pub fingerprint: Option<String>,
    /// Options for `navigator.credentials.get`, bound to the fingerprint.
    pub passkey_options: Option<Value>,
    /// The sealed binding, once approved.
    pub signed: Option<Value>,
}

/// How the desk is set up.
pub struct BindingConfig {
    /// The site.
    pub site: Arc<SiteState>,
    /// Vault on Agentrix's PayPal app.
    pub vault: VaultRail,
    /// The owner's account, as the node knows them.
    pub owner: String,
    /// The passkey RP ID: the owner's console host, without the port. The face registers each
    /// owner's passkeys under their own console (`kilele-sample.paypal.agentrix.website`;
    /// `kilele.money.localhost` on a laptop).
    pub rp_id: String,
    /// The signed audit log.
    pub audit: Arc<dyn AuditSink>,
    /// The desk's store.
    pub pool: SqlitePool,
    /// Where [`router`] is mounted, as PayPal will reach it.
    pub public_base: String,
    /// The face's page the owner continues on after PayPal. `{id}` in it is replaced by the
    /// binding id (`/console/kilele/passkey/link-paypal/{id}`, the ceremony page); without it,
    /// `?binding=<id>` is appended (`/console/kilele/link-paypal`, the approve page). `None`
    /// answers `{moment, id}` as JSON.
    pub ceremony_url: Option<String>,
    /// The node's passkey approvals (saas-web's `HostApprovals`), for the face's approve route.
    pub approvals: Option<Arc<dyn Approvals>>,
    /// The owner's account for `approvals` (`HostApprovals::account(site)`).
    pub owner_account: Option<Uuid>,
    /// The agent DIDs allowed to use the link.
    pub agents: Vec<String>,
    /// Accept a local run's stand-in evidence ([`crate::ceremony::LOCAL_KIND`]) as well as a
    /// passkey. Never on the node.
    pub accept_local: bool,
}

/// Links one site's PayPal.
pub struct BindingDesk {
    site: Arc<SiteState>,
    vault: VaultRail,
    owner: String,
    rp_id: String,
    audit: Arc<dyn AuditSink>,
    pool: SqlitePool,
    public_base: String,
    ceremony_url: Option<String>,
    approvals: Option<Arc<dyn Approvals>>,
    owner_account: Option<Uuid>,
    agents: Vec<String>,
    accept_local: bool,
}

impl std::fmt::Debug for BindingDesk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindingDesk")
            .field("site", &self.site.identity.host)
            .finish_non_exhaustive()
    }
}

fn now() -> i64 {
    Utc::now().timestamp()
}

type Row5 = (
    String,
    Option<PayPalBinding>,
    Option<String>,
    Option<Value>,
    DateTime<Utc>,
);

impl BindingDesk {
    /// The desk; a binding sealed earlier goes into the site's badges.
    pub async fn new(config: BindingConfig) -> anyhow::Result<Arc<Self>> {
        migrate(&config.pool).await?;
        let desk = Arc::new(Self {
            site: config.site,
            vault: config.vault,
            owner: config.owner,
            rp_id: config.rp_id,
            audit: config.audit,
            pool: config.pool,
            public_base: config.public_base.trim_end_matches('/').to_owned(),
            ceremony_url: config.ceremony_url,
            approvals: config.approvals,
            owner_account: config.owner_account,
            agents: config.agents,
            accept_local: config.accept_local,
        });
        if let Some(signed) = desk.current().await? {
            desk.site.set_binding(Some(signed));
        }
        Ok(desk)
    }

    fn environment(&self) -> &'static str {
        if self.vault.sandbox() {
            "sandbox"
        } else {
            "live"
        }
    }

    /// The site's sealed PayPal link, if any.
    pub async fn current(&self) -> anyhow::Result<Option<Value>> {
        let json: Option<String> = sqlx::query_scalar::<_, Option<String>>(
            "SELECT signed_json FROM badge_bindings WHERE site = ? AND state = 'bound'
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(self.site.identity.did())
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    /// Starts linking: PayPal's approval link for the owner.
    pub async fn start(&self) -> anyhow::Result<String> {
        let binding_id = format!("bind_{}", Uuid::now_v7().simple());
        let setup = self
            .vault
            .start_setup(
                &self.site.identity.name,
                &format!(
                    "{}'s agents use this PayPal inside the mandate its owner signs",
                    self.site.identity.name
                ),
                &format!("{}/returned", self.public_base),
                &format!("{}/cancelled", self.public_base),
                &format!("agentrix-saas-{binding_id}"),
            )
            .await?;
        let url = setup
            .approve_url
            .ok_or_else(|| anyhow::anyhow!("PayPal gave no approval link"))?;
        let at = now();
        sqlx::query(
            "INSERT INTO badge_bindings (binding_id, site, setup_token, state, created_at, updated_at)
             VALUES (?, ?, ?, 'awaiting_paypal', ?, ?)",
        )
        .bind(&binding_id)
        .bind(self.site.identity.did())
        .bind(&setup.setup_token)
        .bind(at)
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(url)
    }

    async fn store_pending(
        &self,
        document: &PayPalBinding,
        setup_token: &str,
    ) -> anyhow::Result<()> {
        let fp = hex::encode(fingerprint(document)?);
        let at = now();
        sqlx::query(
            "INSERT INTO badge_bindings (binding_id, site, setup_token, state, draft_json, fingerprint, created_at, updated_at)
             VALUES (?, ?, ?, 'awaiting_passkey', ?, ?, ?, ?)
             ON CONFLICT (binding_id) DO UPDATE SET state = 'awaiting_passkey', draft_json = excluded.draft_json,
               fingerprint = excluded.fingerprint, created_at = excluded.created_at, updated_at = excluded.updated_at",
        )
        .bind(&document.binding_id)
        .bind(self.site.identity.did())
        .bind(setup_token)
        .bind(serde_json::to_string(document)?)
        .bind(fp)
        .bind(at)
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// PayPal returned the owner with `setup_token`: saves the payment method and builds the
    /// pending binding from PayPal's answer. Returns its id. Idempotent.
    pub async fn returned(&self, setup_token: &str) -> anyhow::Result<String> {
        let row = sqlx::query(
            "SELECT binding_id, state FROM badge_bindings WHERE setup_token = ? AND site = ?",
        )
        .bind(setup_token)
        .bind(self.site.identity.did())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no PayPal link was started with that token"))?;
        let binding_id: String = row.get("binding_id");
        if row.get::<String, _>("state") != "awaiting_paypal" {
            return Ok(binding_id);
        }
        let setup = self.vault.setup(setup_token).await?;
        anyhow::ensure!(
            matches!(setup.status.as_str(), "APPROVED" | "VAULTED"),
            "PayPal says the link isn't approved ({})",
            setup.status
        );
        let wallet = self.vault.complete_setup(setup_token).await?;
        let mut document = draft_from_paypal(
            &self.site,
            setup_token,
            &wallet,
            self.environment(),
            self.agents.clone(),
            &self.owner,
        )?;
        document.binding_id = binding_id.clone();
        self.store_pending(&document, setup_token).await?;
        Ok(binding_id)
    }

    /// Binds a payment method PayPal already saved, read back from PayPal now (never typed
    /// in): for a wallet saved before linking existed. Returns the pending binding's id.
    pub async fn adopt(&self, vault_id: &str, setup_token: &str) -> anyhow::Result<String> {
        let wallet = self.vault.payment_token(vault_id).await?;
        let document = draft_from_paypal(
            &self.site,
            setup_token,
            &wallet,
            self.environment(),
            self.agents.clone(),
            &self.owner,
        )?;
        self.store_pending(&document, "").await?;
        Ok(document.binding_id)
    }

    /// The pending binding as the face's page reads it: its state, the draft, its fingerprint
    /// (hex) and the passkey options for it; or the sealed binding.
    pub async fn draft(&self, id: &str) -> anyhow::Result<BindingDraft> {
        let (state, draft, fingerprint, signed, _) =
            self.row(id).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let passkey_options = match (&self.approvals, self.owner_account, &fingerprint) {
            (Some(approvals), Some(account), Some(fp)) if state == "awaiting_passkey" => {
                let bytes: [u8; 32] = hex::decode(fp)?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("bad fingerprint"))?;
                approvals.passkey_options(account, bytes).await?
            }
            _ => None,
        };
        Ok(BindingDraft {
            binding_id: id.to_owned(),
            state,
            draft,
            fingerprint,
            passkey_options,
            signed,
        })
    }

    /// The face's approve route: the owner's passkey answer (`{"method": "passkey",
    /// "credential": …}`) verified by the node's `Approvals` over the draft's fingerprint, then
    /// the binding sealed exactly as through the ceremony. Repeating returns the same binding.
    pub async fn approve(&self, id: &str, request: Value) -> anyhow::Result<Value> {
        let (state, draft, _, signed, created) =
            self.row(id).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        if let (Some(signed), "bound") = (&signed, state.as_str()) {
            return Ok(signed.clone());
        }
        anyhow::ensure!(
            state == "awaiting_passkey"
                && Utc::now() < created + Duration::minutes(PENDING_MINUTES),
            "this link is {state}; start again"
        );
        let (Some(approvals), Some(account)) = (&self.approvals, self.owner_account) else {
            anyhow::bail!("passkey approvals aren't set up on this node");
        };
        let mut binding = draft.ok_or_else(|| anyhow::anyhow!("PayPal hasn't answered yet"))?;
        let fp = fingerprint(&binding)?;
        let proof = approvals.verify(account, fp, request).await?;
        anyhow::ensure!(
            proof["method"].as_str() == Some("passkey"),
            "only a passkey links PayPal"
        );
        anyhow::ensure!(
            proof["fingerprint"].as_str() == Some(hex::encode(fp).as_str()),
            "the passkey approved other terms"
        );
        binding.passkey = proof["credential_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("the passkey proof names no credential"))?
            .to_owned();
        if let Some(assertion) = proof_assertion(&proof) {
            let assertion = assertion?;
            anyhow::ensure!(
                assertion.rp_id == self.rp_id,
                "the passkey is for {}, not this console ({})",
                assertion.rp_id,
                self.rp_id
            );
        }
        binding.approval = Some(proof);
        anyhow::ensure!(
            approval_matches(&serde_json::to_value(&binding)?),
            "the passkey's assertion doesn't check against these terms"
        );
        let record = self.seal_record(id, &binding).await?;
        Ok(record.record)
    }

    async fn seal_record(&self, id: &str, binding: &PayPalBinding) -> anyhow::Result<SealedRecord> {
        let record = jws::sign_document(
            serde_json::to_value(binding)?,
            &self.site.identity.keys.owner,
            &self.site.identity.owner_method(),
        )?;
        let audit_seq = self.audit.append("paypal.binding.linked", &record).await?;
        sqlx::query(
            "UPDATE badge_bindings SET state = 'revoked', updated_at = ? WHERE site = ? AND state = 'bound'",
        )
        .bind(now())
        .bind(self.site.identity.did())
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE badge_bindings SET state = 'bound', signed_json = ?, audit_ref = ?, updated_at = ? WHERE binding_id = ?",
        )
        .bind(record.to_string())
        .bind(audit_seq.to_string())
        .bind(now())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.site.set_binding(Some(record.clone()));
        self.site
            .set_sponsor_passkey(binding.passkey.clone(), Utc::now());
        tracing::info!(site = %self.site.identity.host, binding = id, audit_seq, approved_by = %approved_by(&record), "badge: PayPal linked");
        Ok(SealedRecord {
            sealed: true,
            record,
            audit_seq,
        })
    }

    async fn row(&self, id: &str) -> Result<Row5, CeremonyError> {
        let row = sqlx::query(
            "SELECT state, draft_json, fingerprint, signed_json, created_at FROM badge_bindings WHERE binding_id = ? AND site = ?",
        )
        .bind(id)
        .bind(self.site.identity.did())
        .fetch_optional(&self.pool)
        .await
        .map_err(anyhow::Error::from)?
        .ok_or(CeremonyError::NotFound)?;
        let draft = row
            .get::<Option<String>, _>("draft_json")
            .map(|j| serde_json::from_str(&j))
            .transpose()
            .map_err(anyhow::Error::from)?;
        let signed = row
            .get::<Option<String>, _>("signed_json")
            .map(|j| serde_json::from_str(&j))
            .transpose()
            .map_err(anyhow::Error::from)?;
        let created =
            DateTime::from_timestamp(row.get::<i64, _>("created_at"), 0).unwrap_or_else(Utc::now);
        Ok((
            row.get("state"),
            draft,
            row.get("fingerprint"),
            signed,
            created,
        ))
    }
}

fn tail(value: Option<&str>) -> String {
    value
        .map(|v| {
            v.chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Ceremony for BindingDesk {
    async fn subject(&self, id: &str) -> Result<Subject, CeremonyError> {
        let (state, draft, fingerprint, _, created) = self.row(id).await?;
        let document = draft.ok_or(CeremonyError::NotFound)?;
        let expires_at = created + Duration::minutes(PENDING_MINUTES);
        let state = match state.as_str() {
            "bound" => "sealed",
            "awaiting_passkey" if Utc::now() >= expires_at => "expired",
            "awaiting_passkey" => "pending",
            _ => "expired",
        };
        let paypal = &document.paypal;
        let mut lines = vec![format!(
            "PayPal {} saved payment method …{}",
            document.environment,
            tail(Some(&paypal.payment_token))
        )];
        if let Some(email) = &paypal.payer_email_masked {
            lines.push(format!("PayPal account {email}"));
        }
        if let Some(payer) = &paypal.payer_id {
            lines.push(format!("PayPal payer id {payer}"));
        }
        lines.push(format!(
            "Agents allowed: {}",
            document
                .agents
                .iter()
                .map(|a| a.rsplit(':').next().unwrap_or(a).to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        lines.push(format!(
            "From PayPal's own answer ({})",
            paypal.response_digest
        ));
        Ok(Subject {
            moment: LINK_PAYPAL.into(),
            id: id.to_owned(),
            site: self.site.identity.host.clone(),
            state: state.into(),
            title: format!("Link PayPal to {}'s Cashier", self.site.identity.name),
            lines,
            fingerprint: format!("sha256:{}", fingerprint.unwrap_or_default()),
            expires_at,
        })
    }

    async fn seal(
        &self,
        id: &str,
        evidence: PasskeyEvidence,
    ) -> Result<SealedRecord, CeremonyError> {
        let (state, draft, _, signed, created) = self.row(id).await?;
        if state == "bound" {
            if let Some(record) = signed {
                return Ok(SealedRecord {
                    sealed: true,
                    record,
                    audit_seq: 0,
                });
            }
        }
        if state != "awaiting_passkey" || Utc::now() >= created + Duration::minutes(PENDING_MINUTES)
        {
            return Err(CeremonyError::Expired);
        }
        let mut document = draft.ok_or(CeremonyError::NotFound)?;
        let fp = fingerprint(&document)?;
        if evidence.kind != "passkey" && !self.accept_local {
            return Err(CeremonyError::Evidence(
                crate::ceremony::EvidenceError::Invalid(format!(
                    "{} isn't a passkey",
                    evidence.kind
                )),
            ));
        }
        check_evidence(&evidence, &fp, Some(&self.rp_id))?;
        // Nothing else changes after the passkey signed: the approval is for exactly these terms.
        document.passkey = evidence.credential_id.clone();
        document.approval = Some(serde_json::to_value(&evidence).map_err(anyhow::Error::from)?);
        Ok(self.seal_record(id, &document).await?)
    }
}

#[derive(Deserialize)]
struct Returned {
    approval_token_id: Option<String>,
    approval_session_id: Option<String>,
}

fn error(status: StatusCode, message: impl std::fmt::Display) -> Response {
    (
        status,
        axum::Json(json!({ "message": message.to_string() })),
    )
        .into_response()
}

/// The Vault routes, relative to `public_base`.
pub fn router(desk: Arc<BindingDesk>) -> Router {
    Router::new()
        .route("/start", get(start))
        .route("/returned", get(returned))
        .route("/cancelled", get(cancelled))
        .route("/{binding_id}", get(draft_route))
        .route("/{binding_id}/approve", post(approve_route))
        .with_state(desk)
}

async fn draft_route(
    State(desk): State<Arc<BindingDesk>>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match desk.draft(&id).await {
        Ok(draft) => axum::Json(draft).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, e),
    }
}

async fn approve_route(
    State(desk): State<Arc<BindingDesk>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    match desk.approve(&id, body).await {
        Ok(signed) => axum::Json(json!({ "binding": signed })).into_response(),
        Err(e) => error(StatusCode::FORBIDDEN, format!("Not linked: {e}")),
    }
}

async fn start(State(desk): State<Arc<BindingDesk>>) -> Response {
    match desk.start().await {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(e) => error(
            StatusCode::BAD_GATEWAY,
            format!("PayPal didn't start the link: {e}"),
        ),
    }
}

async fn returned(State(desk): State<Arc<BindingDesk>>, Query(q): Query<Returned>) -> Response {
    let Some(token) = q
        .approval_token_id
        .or(q.approval_session_id)
        .filter(|t| !t.is_empty() && t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric()))
    else {
        return error(
            StatusCode::BAD_REQUEST,
            "This link didn't come from PayPal's approval.",
        );
    };
    match desk.returned(&token).await {
        Ok(id) => match &desk.ceremony_url {
            Some(url) if url.contains("{id}") => {
                Redirect::to(&url.replace("{id}", &id)).into_response()
            }
            Some(url) => Redirect::to(&format!("{url}?binding={id}")).into_response(),
            None => axum::Json(json!({ "moment": LINK_PAYPAL, "id": id })).into_response(),
        },
        Err(e) => error(StatusCode::CONFLICT, e),
    }
}

async fn cancelled() -> Response {
    axum::Json(json!({ "cancelled": true, "message": "PayPal wasn't linked. Nothing was saved." }))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_are_masked() {
        assert_eq!(mask_email("owner@kilele.example"), "o***@kilele.example");
    }
}
