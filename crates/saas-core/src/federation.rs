//! v3 federation contracts: agent identities the cloud providers can check, and theirs at our
//! door. saas-federation implements both traits; saas-badge and saas-settle's door consume them;
//! saas-node wires them together.
//!
//! - **Out** ([`AgentTokenIssuer`]): a site as a small OIDC issuer for its own agents: `iss` the
//!   site (`https://<site host>`, from configuration), `sub` the agent's `did:web`.
//! - **In** ([`AgentTokens`]): a presented token, checked against its issuer's published keys
//!   (trusted issuers only) and mapped onto adk-core's `AgentPrincipal`, the way that issuer's
//!   cloud names agents.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// adk-core's agent identity types, as the door and the badge use them. (Not
/// [`crate::AgentId`], which names an agent on a company's staff.)
pub use adk_core::principal::{AgentId, AgentPrincipal};

/// Where a site serves its OIDC discovery document.
pub const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
/// Where a site serves its key set (`jwks_uri`).
pub const JWKS_PATH: &str = "/.well-known/jwks.json";

/// `sha256:<hex>` of a token, as receipts and logs cite it.
pub fn token_digest(token: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(token.as_bytes())))
}

/// The bearer token in an `Authorization` header, if any.
pub fn bearer(headers: &http::HeaderMap) -> Option<&str> {
    headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// A presented token, checked and mapped. Never holds the token itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FederatedPrincipal {
    /// Who: `id` is the DID, the `spiffe://` id or the cloud's agent id; `class` the Entra
    /// blueprint; `tenant`; `sponsor`.
    pub principal: AgentPrincipal,
    /// The token's `iss`.
    pub issuer: String,
    /// Which claim mapping ran: `agentrix`, `entra`, `agentcore`, `google` or `oidc`.
    pub profile: String,
    /// The token's `sub`.
    pub subject: String,
    /// The token's `aud`.
    pub audience: Vec<String>,
    /// When it was issued.
    pub issued_at: DateTime<Utc>,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
    /// `sha256:<hex>` of the token, for receipts and logs.
    pub token_digest: String,
}

impl FederatedPrincipal {
    /// Who vouched, in a few words: `entra (https://login.microsoftonline.com/…)`.
    pub fn vouched_by(&self) -> String {
        format!("{} ({})", self.profile, self.issuer)
    }
}

/// Why a token was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenRefused {
    /// The issuer isn't one this door trusts.
    #[error("the issuer {0} isn't trusted here")]
    UntrustedIssuer(String),
    /// The signature doesn't check against the issuer's published keys.
    #[error("its signature doesn't check against the issuer's published keys")]
    BadSignature,
    /// It has expired, or isn't valid yet.
    #[error("it has expired")]
    Expired,
    /// It wasn't issued for this audience.
    #[error("it wasn't issued for {0}")]
    WrongAudience(String),
    /// It's genuine, but not an agent's (an Entra token without agent facets).
    #[error("it isn't an agent's token: {0}")]
    NotAnAgent(String),
    /// The issuer's keys couldn't be fetched.
    #[error("the issuer's keys couldn't be fetched: {0}")]
    Unreachable(String),
    /// Not a token this door can read.
    #[error("it isn't a token: {0}")]
    Malformed(String),
}

/// In: a cloud's (or another site's) agent token, to who it is.
#[async_trait::async_trait]
pub trait AgentTokens: Send + Sync {
    /// Checks `token` against its issuer's published keys (trusted issuers only) and that
    /// `audience` is in its `aud`, then maps the claims for that issuer's profile.
    async fn principal(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<FederatedPrincipal, TokenRefused>;
}

/// A token the site minted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedToken {
    /// The JWT.
    pub token: String,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
    /// The signing key's id.
    pub kid: String,
}

/// Out: the site as an OIDC issuer for its own agents.
#[async_trait::async_trait]
pub trait AgentTokenIssuer: Send + Sync {
    /// `https://<site host>`, from configuration, never hard-coded.
    fn issuer(&self) -> String;
    /// `/.well-known/openid-configuration`.
    fn discovery(&self) -> serde_json::Value;
    /// The body at its `jwks_uri` (`/.well-known/jwks.json`): RS256 keys.
    fn jwks(&self) -> serde_json::Value;
    /// A token for the agent with DID `agent_did`, for `audience`, valid for `ttl`: `sub` the
    /// agent's DID, `iss` [`AgentTokenIssuer::issuer`].
    async fn issue(
        &self,
        agent_did: &str,
        audience: &str,
        ttl: chrono::Duration,
    ) -> anyhow::Result<IssuedToken>;
}
