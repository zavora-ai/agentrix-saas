//! The seam for cloud agent identities (specs/saas/v3.md, "Compatibility with the cloud
//! providers' agent identities"): a token presented by an agent becomes an [`AgentPrincipal`],
//! and a site issues tokens for its own agents.
//!
//! The contract is saas-core's (`saas_core::federation`), re-exported here: saas-federation
//! implements both traits (`Federation` checks the clouds' and the sites' tokens, `SiteIssuer`
//! issues ours). The agent door and the badge only call the traits, so tests run on
//! [`FixedTokens`].

use async_trait::async_trait;
use chrono::{Duration, Utc};

pub use saas_core::federation::*;

/// An in-memory [`AgentTokens`] for tests and judge mode: tokens registered by value.
#[derive(Debug, Default)]
pub struct FixedTokens {
    known: std::sync::Mutex<std::collections::HashMap<String, (FederatedPrincipal, String)>>,
}

impl FixedTokens {
    /// Accepts `token` for `audience` as `principal` from `issuer` with `profile`.
    pub fn accept(
        &self,
        token: &str,
        audience: &str,
        principal: AgentPrincipal,
        issuer: &str,
        profile: &str,
    ) {
        let now = Utc::now();
        let federated = FederatedPrincipal {
            subject: principal.id.to_string(),
            principal,
            issuer: issuer.into(),
            profile: profile.into(),
            audience: vec![audience.into()],
            issued_at: now,
            expires_at: now + Duration::minutes(10),
            token_digest: token_digest(token),
        };
        self.known
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.into(), (federated, audience.into()));
    }
}

#[async_trait]
impl AgentTokens for FixedTokens {
    async fn principal(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<FederatedPrincipal, TokenRefused> {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner());
        match known.get(token) {
            Some((principal, aud)) if aud == audience => Ok(principal.clone()),
            Some(_) => Err(TokenRefused::WrongAudience(audience.into())),
            None => Err(TokenRefused::BadSignature),
        }
    }
}
