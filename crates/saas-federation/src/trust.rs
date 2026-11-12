//! In: a cloud's agent proves itself to us ([`AgentTokens`]).
//!
//! The door accepts a token only from an issuer it trusts, checks it against that issuer's
//! published keys, and maps its claims onto adk-core's `AgentPrincipal`. Live, the keys come from
//! the issuer's discovery document and JWKS through adk-auth's `sso` key cache; in judge mode
//! (offline) a pinned key set stands in for the cloud's. RS256 only.
//!
//! How each profile's claims map. Entra and Google follow the live validation in
//! `specs/saas/identity-validation.md` and adk-rust's `agent-federation` profiles; AgentCore's
//! inbound token is not validated yet:
//!
//! | Profile | `AgentPrincipal.id` | `class` | `tenant` | Refused unless |
//! |---|---|---|---|---|
//! | `entra` (Microsoft Entra Agent ID) | `urn:entra:<tid>:agent:<azp or appid>`; `entra.*` attributes | the blueprint (`xms_par_app_azp`), when present | `tid` | `idtyp` is `user`. The agent-marking claims are optional: absent for a custom audience (validated live) |
//! | `agentcore` (Amazon Bedrock AgentCore Identity) | `urn:aws:agentcore:<issuer host>:<client_id or sub>` | `agentcore` | the issuer | the configured issuer signed it |
//! | `google` (Google Agent Identity) | the SPIFFE id in `sub`, unchanged (validated live); `google.*` attributes from its path | none | the SPIFFE trust domain | `sub` is a `spiffe://` id |
//! | `agentrix` (one of our sites) | the agent's `did:web` (`sub`) | none | the site | `sub` is a `did:web` agent of the issuing site's own host |
//! | `oidc` (any other issuer) | `<iss>#<sub>` | none | the issuer | the issuer signed it |

use std::collections::HashMap;

use adk_auth::sso::{JwksCache, TokenClaims};
use adk_core::{AgentId as AdkAgentId, AgentPrincipal};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeZone, Utc};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use saas_core::federation::{AgentTokens, FederatedPrincipal, TokenRefused};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::FederationError;

/// Whose agent identity a token is: the claim mapping that runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudKind {
    /// Microsoft Entra Agent ID.
    EntraAgentId,
    /// Amazon Bedrock AgentCore Identity (its agent's configured issuer).
    AwsAgentCore,
    /// Google Agent Identity.
    GoogleAgentIdentity,
    /// One of our own sites.
    Agentrix,
    /// Any other OIDC issuer.
    Oidc,
}

impl CloudKind {
    /// The profile name [`FederatedPrincipal::profile`] carries.
    pub fn profile(self) -> &'static str {
        match self {
            CloudKind::EntraAgentId => "entra",
            CloudKind::AwsAgentCore => "agentcore",
            CloudKind::GoogleAgentIdentity => "google",
            CloudKind::Agentrix => "agentrix",
            CloudKind::Oidc => "oidc",
        }
    }
}

/// Where a trusted issuer's keys come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "keys", rename_all = "snake_case")]
pub enum Keys {
    /// Its `/.well-known/openid-configuration`, then the `jwks_uri` it names.
    Discovery,
    /// A pinned key set: judge mode and tests, offline.
    Pinned {
        /// The JWKS.
        jwks: Value,
    },
}

/// One issuer the door trusts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustedIssuer {
    /// Its claim mapping.
    pub kind: CloudKind,
    /// Its `iss`, exactly.
    pub issuer: String,
    /// Its keys.
    pub keys: Keys,
    /// Who vouches, in plain words, for the badge page: "Microsoft Entra (tenant …)".
    pub label: String,
    /// The audiences its tokens may carry for this door. Empty: the door's own origin (the
    /// audience the door asks with). Set it where the cloud addresses tokens to something else:
    /// Microsoft Entra's carry the app registration's client id or `api://` URI, never our URL.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audiences: Vec<String>,
}

enum Check {
    Live(JwksCache),
    Pinned(HashMap<String, DecodingKey>),
}

/// The issuers a door trusts: [`AgentTokens`] over them.
pub struct Federation {
    trusted: Vec<(TrustedIssuer, Check)>,
}

impl std::fmt::Debug for Federation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.trusted.iter().map(|(t, _)| &t.issuer))
            .finish()
    }
}

fn pinned(jwks: &Value) -> Result<HashMap<String, DecodingKey>, FederationError> {
    let mut out = HashMap::new();
    for k in jwks["keys"].as_array().into_iter().flatten() {
        if k["kty"].as_str() != Some("RSA") {
            continue;
        }
        let (Some(kid), Some(n), Some(e)) = (k["kid"].as_str(), k["n"].as_str(), k["e"].as_str())
        else {
            continue;
        };
        let key = DecodingKey::from_rsa_components(n, e)
            .map_err(|e| FederationError::Key(e.to_string()))?;
        out.insert(kid.to_owned(), key);
    }
    if out.is_empty() {
        return Err(FederationError::Key(
            "the pinned key set has no RSA key".into(),
        ));
    }
    Ok(out)
}

/// The issuer an unverified token claims: only to pick the keys that then check it.
fn peek_issuer(token: &str) -> Result<String, TokenRefused> {
    let malformed = || TokenRefused::Malformed("not a JWT".into());
    let payload = token.split('.').nth(1).ok_or_else(malformed)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| malformed())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
    v["iss"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| TokenRefused::Malformed("no iss".into()))
}

fn refused(e: jsonwebtoken::errors::Error, audience: &str, issuer: &str) -> TokenRefused {
    match e.kind() {
        ErrorKind::ExpiredSignature | ErrorKind::ImmatureSignature => TokenRefused::Expired,
        ErrorKind::InvalidAudience => TokenRefused::WrongAudience(audience.to_owned()),
        ErrorKind::InvalidIssuer => TokenRefused::UntrustedIssuer(issuer.to_owned()),
        ErrorKind::InvalidSignature | ErrorKind::InvalidAlgorithm | ErrorKind::InvalidRsaKey(_) => {
            TokenRefused::BadSignature
        }
        other => TokenRefused::Malformed(format!("{other:?}")),
    }
}

impl Federation {
    /// The door's trusted issuers. Live ones read their discovery document now.
    pub async fn connect(trusted: Vec<TrustedIssuer>) -> Result<Self, FederationError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| FederationError::Unavailable(e.to_string()))?;
        let mut out = Vec::with_capacity(trusted.len());
        for t in trusted {
            let check = match &t.keys {
                Keys::Discovery => {
                    let url = format!(
                        "{}/.well-known/openid-configuration",
                        t.issuer.trim_end_matches('/')
                    );
                    let doc: Value = http
                        .get(&url)
                        .send()
                        .await
                        .and_then(|r| r.error_for_status())
                        .map_err(|e| FederationError::Unavailable(format!("{url}: {e}")))?
                        .json()
                        .await
                        .map_err(|e| FederationError::Unavailable(format!("{url}: {e}")))?;
                    if doc["issuer"].as_str() != Some(t.issuer.as_str()) {
                        return Err(FederationError::Invalid(format!(
                            "{url} names issuer {}, not {}",
                            doc["issuer"], t.issuer
                        )));
                    }
                    let jwks_uri = doc["jwks_uri"].as_str().ok_or_else(|| {
                        FederationError::Invalid(format!("{url} has no jwks_uri"))
                    })?;
                    Check::Live(JwksCache::new(jwks_uri))
                }
                Keys::Pinned { jwks } => Check::Pinned(pinned(jwks)?),
            };
            out.push((t, check));
        }
        Ok(Self { trusted: out })
    }

    /// The issuers trusted.
    pub fn issuers(&self) -> impl Iterator<Item = &TrustedIssuer> {
        self.trusted.iter().map(|(t, _)| t)
    }

    /// Who vouches for tokens from `issuer`, in plain words.
    pub fn label(&self, issuer: &str) -> Option<&str> {
        self.issuers()
            .find(|t| t.issuer == issuer)
            .map(|t| t.label.as_str())
    }

    /// [`AgentTokens::principal`] as of `now`, for judge mode's recorded tokens.
    pub async fn principal_at(
        &self,
        token: &str,
        audience: &str,
        now: DateTime<Utc>,
    ) -> Result<FederatedPrincipal, TokenRefused> {
        self.check(token, audience, Some(now)).await
    }

    async fn check(
        &self,
        token: &str,
        audience: &str,
        now: Option<DateTime<Utc>>,
    ) -> Result<FederatedPrincipal, TokenRefused> {
        let iss = peek_issuer(token)?;
        let (trusted, check) = self
            .trusted
            .iter()
            .find(|(t, _)| t.issuer == iss)
            .ok_or_else(|| TokenRefused::UntrustedIssuer(iss.clone()))?;
        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| TokenRefused::Malformed(e.to_string()))?;
        if header.alg != Algorithm::RS256 {
            return Err(TokenRefused::BadSignature);
        }
        let kid = header
            .kid
            .ok_or_else(|| TokenRefused::Malformed("no kid".into()))?;
        let key = match check {
            Check::Pinned(keys) => keys.get(&kid).cloned().ok_or(TokenRefused::BadSignature)?,
            Check::Live(cache) => cache.get_key(&kid).await.map_err(|e| match e {
                adk_auth::sso::TokenError::KeyNotFound(_) => TokenRefused::BadSignature,
                other => TokenRefused::Unreachable(other.to_string()),
            })?,
        };
        let accepted: Vec<&str> = if trusted.audiences.is_empty() {
            vec![audience]
        } else {
            trusted.audiences.iter().map(String::as_str).collect()
        };
        let mut v = Validation::new(Algorithm::RS256);
        v.set_issuer(&[&trusted.issuer]);
        v.set_audience(&accepted);
        v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        if now.is_some() {
            v.validate_exp = false;
            v.validate_nbf = false;
        }
        let claims = jsonwebtoken::decode::<TokenClaims>(token, &key, &v)
            .map_err(|e| refused(e, &accepted.join(" or "), &iss))?
            .claims;
        if let Some(now) = now {
            let t = now.timestamp().max(0) as u64;
            if claims.exp + 60 <= t || claims.nbf.is_some_and(|nbf| nbf > t + 60) {
                return Err(TokenRefused::Expired);
            }
        }
        let principal = map(trusted.kind, &claims)?;
        let at = |s: u64| Utc.timestamp_opt(s as i64, 0).single().unwrap_or_default();
        Ok(FederatedPrincipal {
            principal,
            issuer: trusted.issuer.clone(),
            profile: trusted.kind.profile().to_owned(),
            subject: claims.sub.clone(),
            audience: match &claims.aud {
                adk_auth::sso::Audience::Single(a) => vec![a.clone()],
                adk_auth::sso::Audience::Multiple(a) => a.clone(),
                _ => Vec::new(),
            },
            issued_at: at(claims.iat),
            expires_at: at(claims.exp),
            token_digest: saas_core::federation::token_digest(token),
        })
    }
}

#[async_trait::async_trait]
impl AgentTokens for Federation {
    async fn principal(
        &self,
        token: &str,
        audience: &str,
    ) -> Result<FederatedPrincipal, TokenRefused> {
        self.check(token, audience, None).await
    }
}

fn custom_str(claims: &TokenClaims, key: &str) -> Option<String> {
    claims.custom.get(key).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => Some(
            a.iter()
                .map(|x| {
                    x.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| x.to_string())
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    })
}

fn agent_id(id: String) -> Result<AdkAgentId, TokenRefused> {
    AdkAgentId::new(id).map_err(|e| TokenRefused::Malformed(e.to_string()))
}

/// Maps a checked token's claims onto an agent, the way its issuer's cloud names agents.
fn map(kind: CloudKind, c: &TokenClaims) -> Result<AgentPrincipal, TokenRefused> {
    Ok(match kind {
        CloudKind::EntraAgentId => {
            // Validated live (identity-validation.md): for a custom audience the agent-marking
            // claims are absent, so `azp` (v2.0) or `appid` (v1.0) names the agent and the
            // facets are optional extras. A token Entra marks as a user's is not an agent's.
            if custom_str(c, "idtyp").as_deref() == Some("user") {
                return Err(TokenRefused::NotAnAgent(
                    "Entra marks this token as a user's (idtyp user)".into(),
                ));
            }
            let tid = c
                .tid
                .clone()
                .ok_or_else(|| TokenRefused::Malformed("an Entra token without tid".into()))?;
            let agent = custom_str(c, "azp")
                .or_else(|| custom_str(c, "appid"))
                .ok_or_else(|| {
                    TokenRefused::Malformed("an Entra token with neither azp nor appid".into())
                })?;
            let mut p = AgentPrincipal::new(agent_id(format!("urn:entra:{tid}:agent:{agent}"))?)
                .with_tenant(tid.clone())
                .with_attribute("entra.tenant_id", tid)
                .with_attribute("entra.agent_identity_id", agent);
            if let Some(blueprint) = custom_str(c, "xms_par_app_azp") {
                p = p.with_class(blueprint);
            }
            for (attribute, claim) in [
                ("entra.object_id", "oid"),
                ("entra.blueprint_app_id", "xms_par_app_azp"),
                ("entra.token_version", "ver"),
                ("entra.identity_type", "idtyp"),
                ("entra.actor_facets", "xms_act_fct"),
                ("entra.subject_facets", "xms_sub_fct"),
            ] {
                if let Some(v) = custom_str(c, claim) {
                    p = p.with_attribute(attribute, v);
                }
            }
            if custom_str(c, "xms_act_fct").is_some() || custom_str(c, "xms_sub_fct").is_some() {
                p = p.with_attribute("entra.agent_identity_facet", "true");
            }
            p
        }
        CloudKind::AwsAgentCore => {
            let host = c
                .iss
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .replace('/', ":");
            let who = custom_str(c, "client_id").unwrap_or_else(|| c.sub.clone());
            let mut p = AgentPrincipal::new(agent_id(format!("urn:aws:agentcore:{host}:{who}"))?)
                .with_tenant(c.iss.clone())
                .with_class("agentcore");
            if let Some(scope) = custom_str(c, "scope") {
                p = p.with_attribute("scope", scope);
            }
            p
        }
        CloudKind::GoogleAgentIdentity => {
            // Validated live: the subject is the agent's SPIFFE id, used unchanged, in the
            // organisation's agent trust domain:
            // spiffe://<domain>/resources/<service>/projects/<n>/locations/<l>/<type>/<id>.
            let spiffe = [Some(c.sub.clone()), custom_str(c, "spiffe_id")]
                .into_iter()
                .flatten()
                .find(|s| s.starts_with("spiffe://"))
                .ok_or_else(|| {
                    TokenRefused::NotAnAgent(
                        "a Google agent token's subject is not a spiffe:// id".into(),
                    )
                })?;
            let rest = spiffe.trim_start_matches("spiffe://");
            let (domain, path) = rest.split_once('/').unwrap_or((rest, ""));
            let mut p =
                AgentPrincipal::new(agent_id(spiffe.clone())?).with_tenant(domain.to_owned());
            let parts: Vec<&str> = path.split('/').collect();
            if let [
                "resources",
                service,
                "projects",
                project,
                "locations",
                location,
                kind,
                id,
            ] = parts.as_slice()
            {
                p = p
                    .with_attribute("google.service", *service)
                    .with_attribute("google.project_number", *project)
                    .with_attribute("google.location", *location)
                    .with_attribute("google.resource_type", *kind)
                    .with_attribute("google.resource_id", *id);
            }
            let cnf = c.custom.get("cnf");
            p = p.with_attribute("google.bound_token", cnf.is_some().to_string());
            if let Some(thumb) = cnf.and_then(|v| v["x5t#S256"].as_str()) {
                p = p.with_attribute("google.cnf_x5t_s256", thumb);
            }
            p
        }
        CloudKind::Agentrix => {
            // One of our sites speaks only for its own agents: `sub` is
            // `did:web:<host>:agents:<role>`, and `iss` must be that same host.
            let host = c
                .sub
                .strip_prefix("did:web:")
                .and_then(|rest| rest.split_once(":agents:"))
                .map(|(host, _)| host.replace("%3A", ":").to_ascii_lowercase())
                .ok_or_else(|| {
                    TokenRefused::NotAnAgent(format!("{} is not an agent's did:web", c.sub))
                })?;
            let iss_host = c
                .iss
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_ascii_lowercase();
            if host != iss_host {
                return Err(TokenRefused::UntrustedIssuer(format!(
                    "{} can't speak for {}",
                    c.iss, c.sub
                )));
            }
            AgentPrincipal::new(agent_id(c.sub.clone())?).with_tenant(c.iss.clone())
        }
        CloudKind::Oidc => AgentPrincipal::new(agent_id(format!("{}#{}", c.iss, c.sub))?)
            .with_tenant(c.iss.clone()),
    })
}
