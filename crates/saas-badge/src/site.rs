//! A site's identity: its `did:web`, its agents' DIDs, their badges (signed by the site's
//! owner key), Agentrix's voucher for the site (signed by the operator, `did:web:<operator host>`), its
//! PayPal binding and its signed A2A card, and the routes that serve them from the site's own
//! host.
//!
//! The keys are the ones the site already publishes in its Web Bot Auth key directory
//! (`saas_settle::keys::CompanyKeys`): `owner`, and `agent:<role>` for each agent.

use std::path::Path;
use std::sync::{Arc, RwLock};

use adk_auth::web_bot_auth::{DIRECTORY_WELL_KNOWN_PATH, Ed25519SigningKey};
use axum::Router;
use axum::extract::{Path as RoutePath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, Duration, Utc};
use saas_core::CompanyId;
use saas_settle::keys::{CompanyKeys, OWNER_KID, agent_kid};
use saas_settle::tokens::{AgentTokenIssuer, DISCOVERY_PATH, JWKS_PATH};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::contract::{
    Badge, BadgeCredentials, BadgeKey, BadgeSite, BadgeStatus, BindingRef, MandateRef,
    PayPalBinding, Sponsor, WalletCheckpointRef,
};
use crate::did::{CONTEXTS, DidWeb, agent_did, site_did, verification_method};
use crate::jws::{self, sign_card, sign_document};
use saas_core::AgentId;

/// The site's DID document.
pub const SITE_DID_PATH: &str = "/.well-known/did.json";
/// Agentrix's voucher for the site.
pub const VOUCHER_PATH: &str = "/.well-known/agentrix/voucher.json";
/// The site's signed PayPal binding.
pub const BINDING_PATH: &str = "/.well-known/agentrix/paypal-binding.json";
/// The site's A2A agent card, signed.
pub const CARD_PATH: &str = "/.well-known/agent-card.json";
/// Every agent of the site.
pub const INDEX_PATH: &str = "/agents/index.json";
/// What a badge says about the agent's credentials (v3, "No agent holds a secret"): it holds
/// no secrets, and its PayPal access is a short-lived token the company's token service issues
/// per call.
pub fn credentials() -> BadgeCredentials {
    BadgeCredentials {
        holds_secrets: false,
        paypal_access: "issued_per_call".into(),
        statement: NO_SECRETS.into(),
    }
}

/// The badge's statement about secrets, in words.
pub const NO_SECRETS: &str = "Holds no secrets; PayPal access is issued per call, as a short-lived token for one resource and the agent's role.";

/// How long a badge is good for after it is signed.
pub const BADGE_DAYS: i64 = 7;

/// One agent of a site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// `cashier`, `receivables`, `payables`…: its DID's last segment and its key's `agent:<role>`.
    pub role: String,
    /// "Cashier".
    pub name: String,
    /// Its `urn:adk:agent:…` ids (a Cashier holds both v2 desks' ids).
    pub urns: Vec<String>,
    /// The key it signs with: `agent:<key>` in the site's key directory (by default its role;
    /// Kilele's Cashier signs with the `payables` key its agent door already publishes).
    pub key: String,
}

impl AgentSpec {
    /// An agent whose URN is `urn:adk:agent:<company>/<role>`.
    pub fn new(company: &CompanyId, role: &str, name: &str) -> Self {
        Self {
            role: role.into(),
            name: name.into(),
            urns: vec![format!("urn:adk:agent:{}/{role}", company.as_str())],
            key: role.into(),
        }
    }

    /// The Cashier: one agent holding the Receivables and Payables desks' ids, signing with
    /// the site's existing `key` (`payables` or `receivables`).
    pub fn cashier(company: &CompanyId, key: &str) -> Self {
        Self {
            role: "cashier".into(),
            name: "Cashier".into(),
            urns: vec![
                format!("urn:adk:agent:{}/cashier", company.as_str()),
                format!("urn:adk:agent:{}/receivables", company.as_str()),
                format!("urn:adk:agent:{}/payables", company.as_str()),
            ],
            key: key.into(),
        }
    }
}

/// A site and its keys.
#[derive(Debug, Clone)]
pub struct SiteIdentity {
    /// The host (`kilele-sample.paypal.agentrix.website`, from configuration; `kilele.localhost:18182` locally).
    pub host: String,
    /// Its name.
    pub name: String,
    /// Its company in the books.
    pub company: CompanyId,
    /// One of Agentrix's sample sites.
    pub sample: bool,
    /// Its agents.
    pub agents: Vec<AgentSpec>,
    /// Its keys: `owner`, and one per agent role.
    pub keys: CompanyKeys,
}

impl SiteIdentity {
    /// The site's keys kept in `keys_dir` (0600 JWK files, created on first use), its origin
    /// derived from `host`.
    pub fn load(
        host: &str,
        name: &str,
        company: CompanyId,
        sample: bool,
        agents: Vec<AgentSpec>,
        keys_dir: &Path,
    ) -> anyhow::Result<Self> {
        let origin = DidWeb {
            host: host.into(),
            path: Vec::new(),
        }
        .origin();
        let mut roles: Vec<&str> = agents.iter().map(|a| a.key.as_str()).collect();
        roles.sort_unstable();
        roles.dedup();
        Ok(Self {
            host: host.into(),
            name: name.into(),
            company,
            sample,
            keys: CompanyKeys::load_or_create(&origin, &roles, keys_dir)?,
            agents,
        })
    }

    /// The same with keys already in hand.
    pub fn with_keys(
        host: &str,
        name: &str,
        company: CompanyId,
        sample: bool,
        agents: Vec<AgentSpec>,
        keys: CompanyKeys,
    ) -> Self {
        Self {
            host: host.into(),
            name: name.into(),
            company,
            sample,
            agents,
            keys,
        }
    }

    /// `did:web:<host>`.
    pub fn did(&self) -> String {
        site_did(&self.host)
    }

    /// `did:web:<host>#owner`: what signs badges, bindings and the card.
    pub fn owner_method(&self) -> String {
        format!("{}#{OWNER_KID}", self.did())
    }

    /// `https://<host>` (`http://` for loopback hosts).
    pub fn origin(&self) -> String {
        DidWeb {
            host: self.host.clone(),
            path: Vec::new(),
        }
        .origin()
    }

    /// The agent with `role`.
    pub fn agent(&self, role: &str) -> Option<&AgentSpec> {
        self.agents.iter().find(|a| a.role == role)
    }

    fn agent_key(&self, role: &str) -> Option<Arc<Ed25519SigningKey>> {
        self.keys.agents.get(&self.agent(role)?.key).cloned()
    }

    /// The site's DID document: its owner key, and where its agents, voucher, PayPal binding
    /// and key directory are.
    pub fn site_document(&self) -> Value {
        let did = self.did();
        let owner = self.owner_method();
        let origin = self.origin();
        let mut services = vec![
            json!({"id": format!("{did}#voucher"), "type": "AgentrixVoucher", "serviceEndpoint": format!("{origin}{VOUCHER_PATH}")}),
            json!({"id": format!("{did}#paypal"), "type": "PayPalBinding", "serviceEndpoint": format!("{origin}{BINDING_PATH}")}),
            json!({"id": format!("{did}#keys"), "type": "HttpMessageSignaturesDirectory", "serviceEndpoint": format!("{origin}{DIRECTORY_WELL_KNOWN_PATH}")}),
            json!({"id": format!("{did}#card"), "type": "A2AAgentCard", "serviceEndpoint": format!("{origin}{CARD_PATH}")}),
        ];
        for agent in &self.agents {
            services.push(json!({
                "id": format!("{did}#agent-{}", agent.role),
                "type": "AgentDID",
                "serviceEndpoint": agent_did(&self.host, &agent.role),
            }));
        }
        json!({
            "@context": CONTEXTS,
            "id": did,
            "alsoKnownAs": [format!("urn:adk:app:{}", self.company.as_str())],
            "verificationMethod": [verification_method(&owner, &did, self.keys.owner.public_key())],
            "authentication": [owner],
            "assertionMethod": [owner],
            "service": services,
        })
    }

    /// An agent's DID document: its key, its URNs, and where its badge, card and the site's
    /// key directory are. `None` for a role the site doesn't have.
    pub fn agent_document(&self, role: &str) -> Option<Value> {
        let agent = self.agent(role)?;
        let key = self.agent_key(role)?;
        let did = agent_did(&self.host, role);
        let method = format!("{did}#key-1");
        let origin = self.origin();
        Some(json!({
            "@context": CONTEXTS,
            "id": did,
            "controller": self.did(),
            "alsoKnownAs": agent.urns,
            "verificationMethod": [verification_method(&method, &did, key.public_key())],
            "authentication": [method],
            "assertionMethod": [method],
            "service": [
                {"id": format!("{did}#badge"), "type": "AgentBadge", "serviceEndpoint": format!("{origin}/agents/{role}/badge.json")},
                {"id": format!("{did}#card"), "type": "A2AAgentCard", "serviceEndpoint": format!("{origin}{CARD_PATH}")},
                {"id": format!("{did}#keys"), "type": "HttpMessageSignaturesDirectory", "serviceEndpoint": format!("{origin}{DIRECTORY_WELL_KNOWN_PATH}")},
                {"id": format!("{did}#site"), "type": "AgentrixSite", "serviceEndpoint": self.did()},
            ],
        }))
    }
}

/// Agentrix's voucher for a site: it hosts the site and checked its PayPal link. Signed by the
/// operator's owner key (`did:web:<operator host>#owner`; the host comes from configuration).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Voucher {
    /// The operator's DID, `did:web:<operator host>`.
    pub issuer: String,
    /// The site's DID.
    pub site: String,
    /// Its host.
    pub host: String,
    /// Its name.
    pub name: String,
    /// One of Agentrix's sample sites.
    pub sample: bool,
    /// Agentrix hosts it.
    pub hosted: bool,
    /// `sha256:` of the PayPal binding Agentrix checked, when there is one.
    pub paypal_binding: Option<String>,
    /// The statement, in plain words.
    pub statement: String,
    /// When.
    pub issued_at: DateTime<Utc>,
}

/// What a site's badges cite, kept current by the node (binding, mandate, wallet) and
/// served by [`router`].
pub struct SiteState {
    /// The site.
    pub identity: SiteIdentity,
    sponsor: RwLock<Sponsor>,
    binding: RwLock<Option<Value>>,
    mandate: RwLock<Option<MandateRef>>,
    wallet: RwLock<Option<WalletCheckpointRef>>,
    voucher: RwLock<Option<Value>>,
    card: RwLock<Option<Value>>,
    frozen: RwLock<bool>,
    revoked: RwLock<std::collections::BTreeSet<String>>,
    issuer: RwLock<Option<Arc<dyn AgentTokenIssuer>>>,
}

impl std::fmt::Debug for SiteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiteState")
            .field("host", &self.identity.host)
            .finish_non_exhaustive()
    }
}

fn read<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read().unwrap_or_else(|p| p.into_inner()).clone()
}

fn write<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(|p| p.into_inner()) = value;
}

impl SiteState {
    /// A site sponsored by `sponsor`, with nothing linked yet.
    pub fn new(identity: SiteIdentity, sponsor: Sponsor) -> Arc<Self> {
        Arc::new(Self {
            identity,
            sponsor: RwLock::new(sponsor),
            binding: RwLock::new(None),
            mandate: RwLock::new(None),
            wallet: RwLock::new(None),
            voucher: RwLock::new(None),
            card: RwLock::new(None),
            frozen: RwLock::new(false),
            revoked: RwLock::new(std::collections::BTreeSet::new()),
            issuer: RwLock::new(None),
        })
    }

    /// The site as an OIDC issuer for its own agents (saas-federation's): the site then serves
    /// its discovery document and key set, and its DID document lists the issuer's keys, so
    /// a cloud that federates with the issuer and a verifier that resolves the DID see the same
    /// keys.
    pub fn set_issuer(&self, issuer: Option<Arc<dyn AgentTokenIssuer>>) {
        write(&self.issuer, issuer);
    }

    /// The site's token issuer, if it has one.
    pub fn issuer(&self) -> Option<Arc<dyn AgentTokenIssuer>> {
        read(&self.issuer)
    }

    /// The site's DID document, with its token issuer's keys when it has one.
    pub fn site_document(&self) -> Value {
        let mut document = self.identity.site_document();
        let Some(issuer) = self.issuer() else {
            return document;
        };
        let did = self.identity.did();
        let jwks = issuer.jwks();
        for (i, jwk) in jwks["keys"].as_array().into_iter().flatten().enumerate() {
            let kid = jwk["kid"]
                .as_str()
                .map_or_else(|| i.to_string(), str::to_owned);
            let id = format!("{did}#oidc-{kid}");
            let public: serde_json::Map<String, Value> = jwk
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "d" | "p" | "q" | "dp" | "dq" | "qi" | "oth" | "k"
                    )
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if let Some(methods) = document["verificationMethod"].as_array_mut() {
                methods.push(json!({"id": id, "type": "JsonWebKey2020", "controller": did, "publicKeyJwk": public}));
            }
            if let Some(assertion) = document["assertionMethod"].as_array_mut() {
                assertion.push(json!(id));
            }
        }
        if let Some(services) = document["service"].as_array_mut() {
            services.push(json!({"id": format!("{did}#oidc"), "type": "OpenIdConnectIssuer", "serviceEndpoint": issuer.issuer()}));
        }
        document
    }

    /// Sets who answers for the site's agents.
    pub fn set_sponsor(&self, sponsor: Sponsor) {
        write(&self.sponsor, sponsor);
    }

    /// Records the owner's passkey (the credential that sealed the PayPal link) as the
    /// sponsor's.
    pub fn set_sponsor_passkey(&self, credential_id: String, at: DateTime<Utc>) {
        let mut sponsor = read(&self.sponsor);
        sponsor.passkey = Some(credential_id);
        sponsor.approved_at = Some(at);
        write(&self.sponsor, sponsor);
    }

    /// Sets the signed PayPal binding (from [`crate::binding`]).
    pub fn set_binding(&self, signed: Option<Value>) {
        write(&self.binding, signed);
    }

    /// The signed PayPal binding.
    pub fn binding(&self) -> Option<Value> {
        read(&self.binding)
    }

    /// Sets the owner's mandate the agents act under.
    pub fn set_mandate(&self, mandate: Option<MandateRef>) {
        write(&self.mandate, mandate);
    }

    /// Sets the wallet's latest checkpoint (slice W: `Wallets::latest_ref`).
    pub fn set_wallet(&self, wallet: Option<WalletCheckpointRef>) {
        write(&self.wallet, wallet);
    }

    /// Sets Agentrix's signed voucher for the site.
    pub fn set_voucher(&self, voucher: Option<Value>) {
        write(&self.voucher, voucher);
    }

    /// The signed voucher.
    pub fn voucher(&self) -> Option<Value> {
        read(&self.voucher)
    }

    /// Freezes (or unfreezes) every agent of the site: badges say so.
    pub fn set_frozen(&self, frozen: bool) {
        write(&self.frozen, frozen);
    }

    /// Revokes (or restores) one agent: its badge says `revoked` and fails verification. The
    /// owner's console does this; restoring takes the passkey there, and the token service
    /// stops honouring the agent's tokens at the next check.
    pub fn set_revoked(&self, role: &str, revoked: bool) {
        let mut set = self.revoked.write().unwrap_or_else(|p| p.into_inner());
        if revoked {
            set.insert(role.to_owned());
        } else {
            set.remove(role);
        }
    }

    /// Whether the owner revoked the agent with `role`.
    pub fn is_revoked(&self, role: &str) -> bool {
        self.revoked
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(role)
    }

    /// Signs and keeps the site's A2A agent card (`signatures` with the owner key; `jku` is
    /// the site's DID document).
    pub async fn set_card(&self, card: Value) -> anyhow::Result<()> {
        let signed = sign_card(
            card,
            self.identity.keys.owner.clone(),
            &self.identity.owner_method(),
        )
        .await?;
        write(&self.card, Some(signed));
        Ok(())
    }

    /// The signed card.
    pub fn card(&self) -> Option<Value> {
        read(&self.card)
    }

    fn bound(&self) -> Option<(Value, PayPalBinding)> {
        let signed = self.binding()?;
        let binding: PayPalBinding = serde_json::from_value(signed.clone()).ok()?;
        (binding.revoked_at.is_none() && binding.status == crate::contract::BindingStatus::Active)
            .then_some((signed, binding))
    }

    /// The agent's `AgentId`: `<company>/<role>`.
    pub fn agent_id(&self, role: &str) -> Option<AgentId> {
        AgentId::new(self.identity.company.clone(), role).ok()
    }

    /// The agent's badge, as it stands now (unsigned).
    pub fn badge(&self, role: &str) -> Option<Badge> {
        let identity = &self.identity;
        let agent = identity.agent(role)?;
        let key = identity.agent_key(role)?;
        let did = agent_did(&identity.host, role);
        let now = Utc::now();
        let paypal = self.bound().and_then(|(signed, binding)| {
            Some(BindingRef {
                binding_id: binding.binding_id.clone(),
                digest: jws::digest(&signed).ok()?,
                environment: binding.environment.clone(),
                payment_token_tail: binding
                    .paypal
                    .payment_token
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
            })
        });
        Some(Badge {
            id: did.clone(),
            also_known_as: agent.urns.clone(),
            role: role.into(),
            name: agent.name.clone(),
            site: BadgeSite {
                did: identity.did(),
                host: identity.host.clone(),
                name: identity.name.clone(),
                company: identity.company.clone(),
                sample: identity.sample,
            },
            sponsor: read(&self.sponsor),
            keys: vec![BadgeKey {
                id: format!("{did}#key-1"),
                kid: agent_kid(&agent.key),
                thumbprint: key.thumbprint().into(),
            }],
            paypal,
            mandate: read(&self.mandate),
            wallet: read(&self.wallet),
            voucher: self.voucher().and_then(|v| jws::digest(&v).ok()),
            status: if self.is_revoked(role) {
                BadgeStatus::Revoked
            } else if read(&self.frozen) {
                BadgeStatus::Frozen
            } else {
                BadgeStatus::Active
            },
            credentials: Some(credentials()),
            issued_at: now,
            expires_at: now + Duration::days(BADGE_DAYS),
        })
    }

    /// The agent's badge, signed by the site's owner key.
    pub fn signed_badge(&self, role: &str) -> Option<anyhow::Result<Value>> {
        let badge = self.badge(role)?;
        Some((|| {
            let value = serde_json::to_value(&badge)?;
            Ok(sign_document(
                value,
                &self.identity.keys.owner,
                &self.identity.owner_method(),
            )?)
        })())
    }

    /// Every agent of the site: role, name, DID and where its badge is.
    pub fn index(&self) -> Value {
        Value::Array(
            self.identity
                .agents
                .iter()
                .map(|a| {
                    json!({
                        "role": a.role,
                        "name": a.name,
                        "did": agent_did(&self.identity.host, &a.role),
                        "badge": format!("/agents/{}/badge.json", a.role),
                    })
                })
                .collect(),
        )
    }
}

/// Agentrix as the operator that vouches for every site it hosts: `did:web:<operator host>`,
/// signing with its owner key.
#[derive(Debug, Clone)]
pub struct Operator {
    /// Agentrix's own site identity (its host from configuration).
    pub identity: SiteIdentity,
}

impl Operator {
    /// Agentrix, over its own site identity.
    pub fn new(identity: SiteIdentity) -> Self {
        Self { identity }
    }

    /// Signs Agentrix's voucher for `site`: it hosts the site and (when there is one) checked
    /// its PayPal binding's signature. Sets it on the site, and returns it.
    pub fn vouch(&self, site: &SiteState) -> anyhow::Result<Value> {
        let binding = match site.bound() {
            Some((signed, _)) => {
                jws::verify_document(&signed, site.identity.keys.owner.public_key())
                    .map_err(|e| anyhow::anyhow!("the site's PayPal binding doesn't check: {e}"))?;
                Some(jws::digest(&signed)?)
            }
            None => None,
        };
        let statement = match (&binding, site.bound()) {
            (Some(_), Some((signed, _))) => format!(
                "Agentrix hosts {} for its owner and checked that its PayPal link was approved by {} and signed by the site's owner key.",
                site.identity.host,
                crate::binding::approved_by(&signed)
            ),
            _ => format!(
                "Agentrix hosts {} for its owner. Its PayPal isn't linked yet.",
                site.identity.host
            ),
        };
        let voucher = Voucher {
            issuer: self.identity.did(),
            site: site.identity.did(),
            host: site.identity.host.clone(),
            name: site.identity.name.clone(),
            sample: site.identity.sample,
            hosted: true,
            paypal_binding: binding,
            statement,
            issued_at: Utc::now(),
        };
        let signed = sign_document(
            serde_json::to_value(&voucher)?,
            &self.identity.keys.owner,
            &self.identity.owner_method(),
        )?;
        site.set_voucher(Some(signed.clone()));
        Ok(signed)
    }
}

fn json_response(value: Value, media: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, media),
            (header::CACHE_CONTROL, "max-age=60"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        value.to_string(),
    )
        .into_response()
}

fn not_found(what: &str) -> Response {
    (StatusCode::NOT_FOUND, format!("No {what} here.")).into_response()
}

/// The routes a site's host serves: its DID document, each agent's DID document and badge,
/// Agentrix's voucher, the PayPal binding, the signed agent card, and the key directory.
pub fn router(site: Arc<SiteState>) -> Router {
    let directory = site.identity.keys.directory_router();
    Router::new()
        .route(SITE_DID_PATH, get(site_document))
        .route("/agents/{role}/did.json", get(agent_document))
        .route("/agents/{role}/badge.json", get(badge))
        .route(INDEX_PATH, get(index))
        .route(VOUCHER_PATH, get(voucher))
        .route(BINDING_PATH, get(binding))
        .route(CARD_PATH, get(card))
        .route(DISCOVERY_PATH, get(discovery))
        .route(JWKS_PATH, get(jwks))
        .with_state(site)
        .merge(directory)
}

/// Whether `path` is one [`router`] answers (for a node that mounts it in front of others).
pub fn is_badge_path(path: &str) -> bool {
    path == SITE_DID_PATH
        || path == VOUCHER_PATH
        || path == BINDING_PATH
        || path == CARD_PATH
        || path == DIRECTORY_WELL_KNOWN_PATH
        || path == INDEX_PATH
        || path == crate::verify::VERIFY_PATH
        || path == crate::verify::BADGE_VERIFY_PATH
        || path == crate::verify::VERIFY_TOKEN_PATH
        || path == DISCOVERY_PATH
        || path == JWKS_PATH
        || (path.starts_with("/agents/")
            && (path.ends_with("/did.json") || path.ends_with("/badge.json")))
}

async fn site_document(State(site): State<Arc<SiteState>>) -> Response {
    json_response(site.site_document(), "application/did+json")
}

async fn discovery(State(site): State<Arc<SiteState>>) -> Response {
    match site.issuer() {
        Some(issuer) => json_response(issuer.discovery(), "application/json"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn jwks(State(site): State<Arc<SiteState>>) -> Response {
    match site.issuer() {
        Some(issuer) => json_response(issuer.jwks(), "application/jwk-set+json"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn agent_document(
    State(site): State<Arc<SiteState>>,
    RoutePath(role): RoutePath<String>,
) -> Response {
    match site.identity.agent_document(&role) {
        Some(document) => json_response(document, "application/did+json"),
        None => not_found("such agent"),
    }
}

async fn badge(State(site): State<Arc<SiteState>>, RoutePath(role): RoutePath<String>) -> Response {
    match site.signed_badge(&role) {
        Some(Ok(signed)) => json_response(signed, "application/json"),
        Some(Err(e)) => {
            tracing::error!(error = %e, "badge: not signed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "The badge couldn't be signed.",
            )
                .into_response()
        }
        None => not_found("such agent"),
    }
}

async fn index(State(site): State<Arc<SiteState>>) -> Response {
    json_response(site.index(), "application/json")
}

async fn voucher(State(site): State<Arc<SiteState>>) -> Response {
    match site.voucher() {
        Some(v) => json_response(v, "application/json"),
        None => not_found("voucher"),
    }
}

async fn binding(State(site): State<Arc<SiteState>>) -> Response {
    match site.binding() {
        Some(b) => json_response(b, "application/json"),
        None => not_found("PayPal binding"),
    }
}

async fn card(State(site): State<Arc<SiteState>>) -> Response {
    match site.card() {
        Some(c) => json_response(c, "application/json"),
        None => not_found("agent card"),
    }
}
