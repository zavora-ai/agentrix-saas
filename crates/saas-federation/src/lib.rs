//! Compatibility with the cloud providers' agent identities (v3, must-have).
//!
//! All three clouds speak two wire formats: OIDC tokens checked against a published key set,
//! and SPIFFE-style ids. So:
//!
//! The traits are saas-core's ([`AgentTokenIssuer`], [`AgentTokens`]); this crate implements them.
//!
//! - **Out** ([`SiteIssuer`], [`router`]): each site is a small OIDC issuer for its own agents.
//!   It serves `/.well-known/openid-configuration` and its key set, and mints short-lived RS256
//!   tokens with `iss` the site and `sub` the agent's `did:web`. A cloud accepts them through
//!   its own federation (an AWS IAM OIDC provider, a Google workload identity pool, an Entra
//!   federated credential), with no cloud secret stored anywhere.
//! - **In** ([`Federation`]): the door accepts a token only from a trusted issuer, checks it
//!   against that issuer's published keys (adk-auth's `sso` OIDC provider), and maps its claims
//!   onto adk-core's `AgentPrincipal`, the way each cloud names agents.
//! - **Exchanges** ([`AwsWebIdentity`], [`GoogleWorkloadIdentity`], [`EntraAgentIdentity`]): an
//!   agent's token traded for the cloud's own credential naming the agent, with each cloud's
//!   audience (validated live; `specs/saas/identity-validation.md`).
//! - **Judge mode** ([`judge`]): each cloud's token shape as a fixture, signed by a local
//!   stand-in for the cloud's key, so the checks run offline.
//!
//! Hosts come from configuration: every issuer URL and DID contains the judged build's host,
//! which this crate never fixes. Say "accepted by AWS, Google and Microsoft through their own
//! federation" only for the clouds proven live.

mod exchange;
mod http;
mod issuer;
pub mod judge;
mod trust;

#[cfg(test)]
mod tests;

pub use exchange::{
    ASSERTION_TTL, AWS_STS_AUDIENCE, AwsCredentials, AwsWebIdentity, ENTRA_ASSERTION_AUDIENCE,
    EntraAccessToken, EntraAgentIdentity, GoogleAccessToken, GoogleWorkloadIdentity, Secret,
};
pub use http::router;
pub use issuer::{MAX_TTL, SiteIssuer, issuer_url, site_host};
pub use saas_core::federation::{
    AgentTokenIssuer, AgentTokens, DISCOVERY_PATH, FederatedPrincipal, IssuedToken, JWKS_PATH,
    TokenRefused, bearer, token_digest,
};
pub use trust::{CloudKind, Federation, Keys, TrustedIssuer};

/// Why federation refused or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FederationError {
    /// The token isn't one this door accepts: untrusted issuer, bad signature, wrong audience,
    /// expired, or not an agent's.
    #[error("refused: {0}")]
    Refused(String),
    /// A request is malformed.
    #[error("invalid: {0}")]
    Invalid(String),
    /// A key couldn't be made, read or used.
    #[error("key: {0}")]
    Key(String),
    /// An issuer's discovery document or keys couldn't be fetched.
    #[error("unavailable: {0}")]
    Unavailable(String),
}
