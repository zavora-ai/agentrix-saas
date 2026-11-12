//! v3 identity contracts: who an agent is to anyone outside ([`Badge`]), and the owner's
//! passkey-approved link between a site and its PayPal account ([`PayPalBinding`]).
//!
//! The fields are slices P (badge) and K (PayPal binding)'s, as they specified them. These
//! are plain bodies with no crypto: sealing (Ed25519 or JWS over the RFC 8785 JSON) stays in
//! saas-badge and saas-settle. [`digest`] is the `sha256:<hex>` of a body's canonical JSON,
//! the digest saas-settle's sealed documents already use.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::ids::{AgentId, CompanyId};
use crate::money::Money;
use crate::wallet::WalletCheckpointRef;

/// `sha256:<hex>` of `body`'s RFC 8785 canonical JSON.
pub fn digest<T: Serialize>(body: &T) -> Result<String, serde_json::Error> {
    let bytes = serde_jcs::to_vec(body)?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(&bytes))))
}

/// A `did:web` host part: lowercase, with a port's `:` percent-encoded, as did:web requires.
fn did_host(host: &str) -> String {
    host.to_ascii_lowercase().replace(':', "%3A")
}

/// `did:web:<host>`: a site.
pub fn site_did(host: &str) -> String {
    format!("did:web:{}", did_host(host))
}

/// `did:web:<host>:agents:<role>`: one of a site's agents.
pub fn agent_did(host: &str, role: &str) -> String {
    format!("did:web:{}:agents:{role}", did_host(host))
}

/// `urn:adk:agent:<company>/<role>`: the internal name policy files and audit records keep.
pub fn agent_urn(agent: &AgentId) -> String {
    format!("urn:adk:agent:{agent}")
}

/// An agent's badge, served at `GET https://<site-host>/agents/<role>/badge.json` and
/// signed by the site's owner key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Badge {
    /// The agent's DID: `did:web:<site-host>:agents:<role>`.
    pub id: String,
    /// The URNs audit records carry: `urn:adk:agent:<company>/<role>`, …
    pub also_known_as: Vec<String>,
    /// `cashier`, `receivables`, `payables`, …
    pub role: String,
    /// "Cashier".
    pub name: String,
    /// The site it works for.
    pub site: BadgeSite,
    /// Who answers for it.
    pub sponsor: Sponsor,
    /// Its keys.
    pub keys: Vec<BadgeKey>,
    /// The site's PayPal link, when there is one.
    pub paypal: Option<BindingRef>,
    /// The owner's mandate this agent acts under.
    pub mandate: Option<MandateRef>,
    /// The wallet's latest checkpoint (slice W fills it).
    pub wallet: Option<WalletCheckpointRef>,
    /// `sha256:<hex>` of Agentrix's voucher for the site.
    pub voucher: Option<String>,
    /// Active, frozen or revoked.
    pub status: BadgeStatus,
    /// When it was issued.
    pub issued_at: DateTime<Utc>,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
    /// What the agent holds: no secrets; PayPal access issued per call. Absent on badges signed
    /// before it was added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<BadgeCredentials>,
}

/// What an agent holds, as its badge states it ("No agent holds a secret").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeCredentials {
    /// Whether the agent holds any secret itself. Always `false` on this platform.
    pub holds_secrets: bool,
    /// How it reaches PayPal, for example `issued_per_call`.
    pub paypal_access: String,
    /// The statement in plain words.
    pub statement: String,
}

impl Badge {
    /// `did:web:<host>:agents:<role>`.
    pub fn agent_did(host: &str, role: &str) -> String {
        agent_did(host, role)
    }

    /// `urn:adk:agent:<company>/<role>`.
    pub fn urn(agent: &AgentId) -> String {
        agent_urn(agent)
    }

    /// [`digest`] of the badge body.
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        digest(self)
    }
}

/// The site, as a badge names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeSite {
    /// `did:web:<host>`.
    pub did: String,
    /// The host.
    pub host: String,
    /// The site's name.
    pub name: String,
    /// The company the site is.
    pub company: CompanyId,
    /// A sample site.
    pub sample: bool,
}

/// Who answers for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sponsor {
    /// The owner's account id.
    pub owner: String,
    /// The owner's name.
    pub name: Option<String>,
    /// The WebAuthn credential id that vouches.
    pub passkey: Option<String>,
    /// When the owner approved.
    pub approved_at: Option<DateTime<Utc>>,
}

/// One of the agent's keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeKey {
    /// A DID URL, for example `did:web:<host>:agents:<role>#key-1`.
    pub id: String,
    /// `agent:<role>` in the key directory.
    pub kid: String,
    /// The RFC 7638 JWK thumbprint.
    pub thumbprint: String,
}

/// The site's PayPal link, as a badge cites it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingRef {
    /// [`PayPalBinding::binding_id`].
    pub binding_id: String,
    /// `sha256:<hex>` of the sealed [`PayPalBinding`].
    pub digest: String,
    /// `sandbox` or `live`.
    pub environment: String,
    /// The last four characters of the vault id.
    pub payment_token_tail: String,
}

/// The owner's mandate, as a badge cites it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateRef {
    /// The mandate's id.
    pub mandate_id: String,
    /// Its digest.
    pub digest: String,
    /// What it allows, in plain words.
    pub summary: String,
    /// The most for one invoice.
    pub max_per_invoice: Money,
    /// The most in a month.
    pub max_per_month: Money,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
}

/// Whether an agent may act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeStatus {
    /// It acts inside its mandate.
    Active,
    /// The owner froze it: it moves no money until unfrozen.
    Frozen,
    /// The owner revoked it: its tokens stop working at the next check, no PayPal key is
    /// rotated, and switching it back on takes the passkey. When is in the audit log.
    Revoked,
}

/// The owner's link between a site and its PayPal account (slice K): built only from PayPal's
/// own Vault responses, approved by the owner's passkey, sealed with the site's owner key,
/// written to the signed audit log, and referenced by every badge of the site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayPalBinding {
    /// `bind_<uuid v7>`.
    pub binding_id: String,
    /// `did:web:<site-host>`.
    pub site: String,
    /// The company.
    pub company: CompanyId,
    /// `sandbox` or `live`.
    pub environment: String,
    /// The ids PayPal returned.
    pub paypal: PayPalVaultIds,
    /// The agent DIDs allowed to use it.
    pub agents: Vec<String>,
    /// The owner's account id.
    pub owner: String,
    /// The WebAuthn credential id that approved it.
    pub passkey: String,
    /// The passkey proof (method, fingerprint, time, assertion). `None` in the draft the passkey
    /// signs ([`PayPalBinding::signing_digest`]); `Some` in the sealed copy.
    pub approval: Option<serde_json::Value>,
    /// When it was linked.
    pub linked_at: DateTime<Utc>,
    /// Active or revoked.
    pub status: BindingStatus,
    /// When the owner unlinked it: unlinking re-seals the same binding with this set, approved
    /// by passkey; a new link is a new binding.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl PayPalBinding {
    /// [`digest`] of the binding.
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        digest(self)
    }

    /// The digest the owner's passkey signs (the fingerprint shown to the owner): the body with
    /// no approval and an empty `passkey`, because the credential id comes from the assertion and
    /// isn't known yet when the owner is asked. The verifier checks that the sealed copy's proof
    /// carries this fingerprint (saas-badge's `binding::fingerprint` and `approval_matches`
    /// compute the same).
    pub fn signing_digest(&self) -> Result<String, serde_json::Error> {
        digest(&PayPalBinding {
            approval: None,
            passkey: String::new(),
            ..self.clone()
        })
    }

    /// Whether the agent with DID `agent` may use the link: listed, and not revoked.
    pub fn allows(&self, agent: &str) -> bool {
        self.status == BindingStatus::Active
            && self.revoked_at.is_none()
            && self.agents.iter().any(|a| a == agent)
    }
}

/// The ids PayPal's Vault returned for a binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayPalVaultIds {
    /// `POST /v3/vault/setup-tokens` → `id`.
    pub setup_token: String,
    /// `POST /v3/vault/payment-tokens` → `id` (the vault id).
    pub payment_token: String,
    /// → `customer.id`.
    pub customer_id: Option<String>,
    /// → `payment_source.paypal.payer_id`.
    pub payer_id: Option<String>,
    /// → `payment_source.paypal.email_address`, masked (`j***@example.com`).
    pub payer_email_masked: Option<String>,
    /// The merchant app PayPal's token response names (`APP-…`).
    pub app_id: Option<String>,
    /// `sha256:<hex>` of PayPal's payment-token response body as received.
    pub response_digest: String,
}

/// Whether a binding is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    /// In force.
    Active,
    /// Unlinked by the owner.
    Revoked,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dids_and_digests() {
        assert_eq!(
            agent_did("Kilele-Sample.agentrix.website", "cashier"),
            "did:web:kilele-sample.agentrix.website:agents:cashier"
        );
        assert_eq!(site_did("localhost:18183"), "did:web:localhost%3A18183");
        let agent = AgentId::new(CompanyId::kilele(), "cashier").unwrap();
        assert_eq!(agent_urn(&agent), "urn:adk:agent:kilele-bakery/cashier");
        let a = digest(&serde_json::json!({"b": 1, "a": [1, 2]})).unwrap();
        let b = digest(&serde_json::json!({"a": [1, 2], "b": 1})).unwrap();
        assert_eq!(a, b, "canonical JSON: key order doesn't change the digest");
        assert!(a.starts_with("sha256:") && a.len() == 7 + 64);
        assert_eq!(serde_json::to_value(BadgeStatus::Frozen).unwrap(), "frozen");
    }

    #[test]
    fn a_binding_allows_only_its_agents_while_active() {
        let cashier = agent_did("kilele-sample.agentrix.website", "cashier");
        let mut binding = PayPalBinding {
            binding_id: "bind_01".into(),
            site: site_did("kilele-sample.agentrix.website"),
            company: CompanyId::kilele(),
            environment: "sandbox".into(),
            paypal: PayPalVaultIds {
                setup_token: "37R143433F051950D".into(),
                payment_token: "5an08729le1870910".into(),
                customer_id: Some("GHeVXOJkXI".into()),
                payer_id: None,
                payer_email_masked: None,
                app_id: None,
                response_digest: "sha256:00".into(),
            },
            agents: vec![cashier.clone()],
            owner: "owner-kilele".into(),
            passkey: "AbC".into(),
            approval: None,
            linked_at: "2026-11-10T17:00:00Z".parse().unwrap(),
            status: BindingStatus::Active,
            revoked_at: None,
        };
        assert!(binding.allows(&cashier));
        assert!(!binding.allows(&agent_did("kilele-sample.agentrix.website", "chief")));
        // The passkey signs the body without its proof; adding the proof doesn't change that.
        let signed = binding.signing_digest().unwrap();
        binding.approval = Some(serde_json::json!({"method": "passkey", "fingerprint": signed}));
        binding.passkey = "credential-from-the-assertion".into();
        assert_eq!(
            binding.signing_digest().unwrap(),
            signed,
            "the proof and the credential id come after"
        );
        let d = binding.digest().unwrap();
        binding.status = BindingStatus::Revoked;
        assert!(!binding.allows(&cashier), "a revoked link allows no one");
        assert_ne!(d, binding.digest().unwrap());
    }
}
