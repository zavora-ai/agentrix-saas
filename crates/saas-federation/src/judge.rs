//! Judge mode: each cloud's agent token shape as a fixture, checked offline.
//!
//! The fixtures (`fixtures/*.json`) carry each cloud's documented claim shape with made-up ids.
//! At start a local stand-in key per cloud signs them, and the [`Federation`] trusts that
//! stand-in's pinned key set, so the whole path (signature, issuer, audience, expiry, the claim
//! mapping onto `AgentPrincipal`) runs with no network. They prove the mapping, not the clouds:
//! the live proofs replace the stand-ins with the clouds' own issuers.

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{CloudKind, Federation, FederationError, Keys, SiteIssuer, TrustedIssuer};

const FIXTURES: [&str; 3] = [
    include_str!("../fixtures/entra-agent-id.json"),
    include_str!("../fixtures/aws-agentcore.json"),
    include_str!("../fixtures/google-agent-identity.json"),
];

#[derive(Deserialize)]
struct Fixture {
    kind: CloudKind,
    label: String,
    source: String,
    issuer: String,
    audience: String,
    claims: Map<String, Value>,
}

/// One cloud's fixture token.
#[derive(Debug, Clone)]
pub struct FixtureToken {
    /// Whose shape it is.
    pub kind: CloudKind,
    /// Where the shape comes from, in plain words.
    pub source: String,
    /// The audience it was issued for (pass it to `principal`).
    pub audience: String,
    /// The signed token.
    pub token: String,
}

/// The stand-in issuers and one token per cloud, valid for an hour from `now`.
pub async fn stand_ins(
    now: DateTime<Utc>,
) -> Result<(Federation, Vec<FixtureToken>), FederationError> {
    let mut trusted = Vec::new();
    let mut tokens = Vec::new();
    for text in FIXTURES {
        let f: Fixture = serde_json::from_str(text)
            .map_err(|e| FederationError::Invalid(format!("fixture: {e}")))?;
        let stand_in = SiteIssuer::generate(&f.issuer)?;
        let mut claims = f.claims;
        // A fixture may carry its cloud's own `aud` form (Google's is an array).
        claims.entry("aud").or_insert(json!(f.audience));
        claims.insert("iat".into(), json!(now.timestamp()));
        claims.insert("nbf".into(), json!(now.timestamp()));
        claims.insert("exp".into(), json!((now + Duration::hours(1)).timestamp()));
        tokens.push(FixtureToken {
            kind: f.kind,
            source: f.source,
            audience: f.audience,
            token: stand_in.sign_claims(claims)?,
        });
        trusted.push(TrustedIssuer {
            kind: f.kind,
            issuer: f.issuer,
            keys: Keys::Pinned {
                jwks: stand_in.jwks(),
            },
            label: format!("{} (judge-mode stand-in)", f.label),
            audiences: Vec::new(),
        });
    }
    Ok((Federation::connect(trusted).await?, tokens))
}
