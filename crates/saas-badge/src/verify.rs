//! Verifying an agent from its DID alone: resolve it from the site's own domain, fetch its
//! badge, and check every signature and link, each check in plain words.
//!
//! Used by `agentrix-badge verify <did>` and by `GET /agents/verify?did=…` ([`router`]),
//! which the face proxies for its Verify box (`face-v3-contracts.md` §2).

use std::sync::Arc;

use adk_auth::web_bot_auth::jwk_thumbprint;
use axum::Router;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::binding::{approval_matches, approved_by};
use crate::contract::{Badge, BadgeStatus, PayPalBinding};
use crate::did::DidWeb;
use crate::jws;
use crate::resolve::{DidWebResolver, IdentityResolver, ResolvedIdentity, Resolvers};
use crate::site::Voucher;
use saas_settle::tokens::{AgentTokens, FederatedPrincipal};

/// Where the verify endpoint is.
pub const VERIFY_PATH: &str = "/agents/verify";
/// The same endpoint where saas-web's `HttpSettle` asks (`{base}/badge/verify?did=`).
pub const BADGE_VERIFY_PATH: &str = "/badge/verify";
/// `POST {"token": …}`: who a presented agent token says the agent is, and who vouched.
pub const VERIFY_TOKEN_PATH: &str = "/agents/verify-token";

/// What a presented agent token says, checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenReport {
    /// The token checked, and (when it names one of these sites' agents) its badge too.
    pub ok: bool,
    /// Who the agent is, as its issuer's claims say.
    pub principal: Option<FederatedPrincipal>,
    /// Who vouched: the issuer and the profile that mapped it.
    pub vouched_by: Option<String>,
    /// Why the token was refused.
    pub refused: Option<String>,
    /// The badge checks, when the token names a `did:web` agent.
    pub badge: Option<VerifyReport>,
    /// When checked.
    pub checked_at: DateTime<Utc>,
}

/// One check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// What was checked, in words.
    pub check: String,
    /// The same, under the name saas-web reads.
    pub name: String,
    /// Whether it held.
    pub ok: bool,
    /// What was found.
    pub detail: String,
}

/// The first check that failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// The check.
    pub check: String,
    /// Why, in words.
    pub message: String,
}

/// Everything checked about one agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// The DID asked about.
    pub did: String,
    /// Whether every check held.
    pub verified: bool,
    /// The same, under the name saas-web reads.
    pub ok: bool,
    /// The checks, in order.
    pub checks: Vec<Check>,
    /// The first check that failed.
    pub failure: Option<Failure>,
    /// The badge, when it was fetched.
    pub badge: Option<Value>,
    /// When checked.
    pub checked_at: DateTime<Utc>,
}

/// The checks, by name.
pub mod names {
    /// The DID resolves.
    pub const DID: &str = "did.json resolves from the site's own domain";
    /// The badge's signature.
    pub const SIGNED: &str = "badge signed by the owner key";
    /// The badge is this agent's.
    pub const BADGE: &str = "badge names this agent and site";
    /// The agent's key agrees everywhere.
    pub const KEY: &str = "key is the agent's in its DID document";
    /// The key directory publishes it.
    pub const DIRECTORY: &str = "key is in the site's key directory";
    /// Names agree.
    pub const NAMES: &str = "names match the DID document";
    /// Active.
    pub const STATUS: &str = "agent is active";
    /// No secrets.
    pub const SECRETS: &str = "agent holds no secrets";
    /// The voucher.
    pub const VOUCHER: &str = "Agentrix vouches for the site";
    /// The PayPal binding.
    pub const PAYPAL: &str = "PayPal binding sealed by the owner's passkey";
    /// The card.
    pub const CARD: &str = "agent card signed by the owner key";
}

/// What a verifier trusts and how it fetches.
#[derive(Clone)]
pub struct Verifier {
    web: DidWebResolver,
    resolvers: Resolvers,
    operator: String,
    accept_local: bool,
    tokens: Option<(Arc<dyn AgentTokens>, String)>,
}

impl std::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Verifier")
            .field("operator", &self.operator)
            .finish_non_exhaustive()
    }
}

struct Checks(Vec<Check>);

impl Checks {
    fn push(&mut self, check: &str, result: Result<String, String>) {
        let (ok, detail) = match result {
            Ok(detail) => (true, detail),
            Err(detail) => (false, detail),
        };
        self.0.push(Check {
            check: check.into(),
            name: check.into(),
            ok,
            detail,
        });
    }

    fn report(self, did: &str, badge: Option<Value>) -> VerifyReport {
        let failure = self.0.iter().find(|c| !c.ok).map(|c| Failure {
            check: c.check.clone(),
            message: c.detail.clone(),
        });
        let verified = !self.0.is_empty() && failure.is_none();
        VerifyReport {
            did: did.into(),
            verified,
            ok: verified,
            checks: self.0,
            failure,
            badge,
            checked_at: Utc::now(),
        }
    }
}

impl Verifier {
    /// Fetches with `web` (whose loopback rule also applies to badges and vouchers),
    /// resolves keys with `resolvers`, and trusts vouchers from `operator`
    /// (`did:web:<operator host>`, from configuration: every DID carries the judged build's host).
    pub fn new(web: DidWebResolver, resolvers: Resolvers, operator: impl Into<String>) -> Self {
        Self {
            web,
            resolvers,
            operator: operator.into(),
            accept_local: false,
            tokens: None,
        }
    }

    /// Also checks agent tokens issued for `audience` (saas-federation's `AgentTokens`: the
    /// clouds' agent tokens and the sites' own), at [`VERIFY_TOKEN_PATH`].
    #[must_use]
    pub fn with_tokens(
        mut self,
        tokens: Arc<dyn AgentTokens>,
        audience: impl Into<String>,
    ) -> Self {
        self.tokens = Some((tokens, audience.into()));
        self
    }

    /// Checks a presented agent token: who it says the agent is and who vouched. When it speaks
    /// for a `did:web` agent (its principal, or its `sub`: a plain OIDC issuer's principal is
    /// `{iss}#{sub}`), only that DID's own site may have issued it, and the agent's badge is
    /// checked too.
    pub async fn verify_token(&self, token: &str) -> TokenReport {
        let report = |principal: Option<FederatedPrincipal>,
                      refused: Option<String>,
                      badge: Option<VerifyReport>| TokenReport {
            ok: principal.is_some() && badge.as_ref().is_none_or(|p| p.verified),
            vouched_by: principal.as_ref().map(FederatedPrincipal::vouched_by),
            principal,
            refused,
            badge,
            checked_at: Utc::now(),
        };
        let Some((tokens, audience)) = &self.tokens else {
            return report(
                None,
                Some("this verifier checks no agent tokens".into()),
                None,
            );
        };
        match tokens.principal(token, audience).await {
            Ok(principal) => {
                let named = [
                    principal.principal.id.to_string(),
                    principal.subject.clone(),
                ]
                .into_iter()
                .find_map(|id| {
                    DidWeb::parse(&id)
                        .filter(|d| d.agent_role().is_some())
                        .map(|d| (id, d))
                });
                let Some((did, parsed)) = named else {
                    return report(Some(principal), None, None);
                };
                if principal.issuer.trim_end_matches('/') != parsed.origin() {
                    return report(
                        None,
                        Some(format!(
                            "the token speaks for {did}, but {} issued it, not that agent's site",
                            principal.issuer
                        )),
                        None,
                    );
                }
                let badge = self.verify(&did).await;
                report(Some(principal), None, Some(badge))
            }
            Err(e) => report(None, Some(format!("the token was refused: {e}")), None),
        }
    }

    /// Also accepts a local run's stand-in approval (`local-owner-key`) on a PayPal binding,
    /// reported as such. Never on the node.
    #[must_use]
    pub fn accept_local(mut self) -> Self {
        self.accept_local = true;
        self
    }

    async fn key_of(&self, method: &str) -> Result<[u8; 32], String> {
        let resolved = self
            .resolvers
            .resolve(method)
            .await
            .map_err(|e| e.to_string())?;
        resolved
            .key(method)
            .map(|k| k.public_key)
            .ok_or_else(|| format!("{} publishes no key {method}", resolved.id))
    }

    async fn signed_by(&self, document: &Value, expected: &str) -> Result<(), String> {
        let method = jws::proof_method(document).ok_or("it carries no signature")?;
        if method != expected {
            return Err(format!("it is signed by {method}, not {expected}"));
        }
        let key = self.key_of(method).await?;
        jws::verify_document(document, &key).map_err(|e| e.to_string())
    }

    async fn paypal(
        &self,
        url: &str,
        owner: &str,
        cited_digest: &str,
        badge: &Badge,
    ) -> Result<String, String> {
        let binding = self.web.fetch_json(url).await.map_err(|e| e.to_string())?;
        self.signed_by(&binding, owner)
            .await
            .map_err(|e| format!("the binding's signature doesn't check: {e}"))?;
        let terms: PayPalBinding = serde_json::from_value(binding.clone())
            .map_err(|e| format!("it isn't a binding: {e}"))?;
        if jws::digest(&binding).ok().as_deref() != Some(cited_digest) {
            return Err("the badge cites another binding".into());
        }
        if !terms.allows(&badge.id) {
            return Err("the binding doesn't let this agent use the PayPal link".into());
        }
        if terms.revoked_at.is_some() {
            return Err("the PayPal link was revoked".into());
        }
        let method = terms
            .approval
            .as_ref()
            .and_then(|a| a["kind"].as_str().or(a["method"].as_str()));
        if method != Some("passkey") && !self.accept_local {
            return Err(format!(
                "the link was approved by {}, not a passkey",
                approved_by(&binding)
            ));
        }
        if !approval_matches(&binding) {
            return Err(
                "the owner's passkey assertion doesn't check against the binding's terms".into(),
            );
        }
        Ok(format!(
            "PayPal {} saved payment method …{}; approved by {} (credential {}); signed by the owner key",
            terms.environment,
            badge
                .paypal
                .as_ref()
                .map(|p| p.payment_token_tail.as_str())
                .unwrap_or(""),
            approved_by(&binding),
            terms.passkey
        ))
    }

    /// Checks the agent `did`.
    pub async fn verify(&self, did: &str) -> VerifyReport {
        let mut checks = Checks(Vec::new());

        // 1. The DID resolves from the site's own domain.
        let Some(parsed) = DidWeb::parse(did).filter(|d| d.agent_role().is_some()) else {
            checks.push(
                names::DID,
                Err(format!(
                    "{did} isn't an agent's did:web (did:web:<site>:agents:<role>)"
                )),
            );
            return checks.report(did, None);
        };
        let role = parsed.agent_role().unwrap_or_default().to_owned();
        let site_did = parsed.site().to_did();
        let agent: ResolvedIdentity = match self.resolvers.resolve(did).await {
            Ok(agent) => agent,
            Err(e) => {
                checks.push(names::DID, Err(e.to_string()));
                return checks.report(did, None);
            }
        };
        checks.push(names::DID, Ok(parsed.document_url()));

        // 2. The badge, signed by the site's owner key.
        let Some(badge_url) = agent.service("AgentBadge").map(str::to_owned) else {
            checks.push(names::BADGE, Err("the DID document names no badge".into()));
            return checks.report(did, None);
        };
        let value = match self.web.fetch_json(&badge_url).await {
            Ok(value) => value,
            Err(e) => {
                checks.push(names::BADGE, Err(e.to_string()));
                return checks.report(did, None);
            }
        };
        let owner = format!("{site_did}#owner");
        checks.push(
            names::SIGNED,
            self.signed_by(&value, &owner).await.map(|()| owner.clone()),
        );
        let badge: Badge = match serde_json::from_value(value.clone()) {
            Ok(p) => p,
            Err(e) => {
                checks.push(names::BADGE, Err(format!("it isn't a badge: {e}")));
                return checks.report(did, Some(value));
            }
        };
        checks.push(
            names::BADGE,
            if badge.id == did && badge.role == role && badge.site.did == site_did {
                Ok(format!(
                    "{} ({}) of {}, sponsored by {}",
                    badge.name,
                    badge.role,
                    badge.site.name,
                    badge
                        .sponsor
                        .name
                        .as_deref()
                        .unwrap_or(&badge.sponsor.owner)
                ))
            } else {
                Err("the badge is for another agent or site".into())
            },
        );

        // 3. The agent's key: the same in the DID document, the badge and the key directory.
        let agent_key = agent.keys.first().map(|k| k.public_key);
        let listed = badge.keys.first();
        checks.push(
            names::KEY,
            match (&agent_key, listed) {
                (Some(key), Some(listed)) if jwk_thumbprint(key) == listed.thumbprint => {
                    Ok(listed.thumbprint.clone())
                }
                (Some(_), Some(_)) => {
                    Err("the badge lists another key than the DID document".into())
                }
                _ => Err("no agent key is published".into()),
            },
        );
        let directory = match (agent_key, listed) {
            (Some(key), Some(listed)) => {
                match self
                    .resolvers
                    .resolve(&format!("{}#{}", parsed.origin(), listed.kid))
                    .await
                {
                    Ok(dir) => match dir.key(&listed.kid) {
                        Some(k) if k.public_key == key => Ok(format!("kid {}", listed.kid)),
                        Some(_) => {
                            Err("the key directory publishes another key under that name".into())
                        }
                        None => Err(format!("the key directory has no {}", listed.kid)),
                    },
                    Err(e) => Err(e.to_string()),
                }
            }
            _ => Err("no key to look up".into()),
        };
        checks.push(names::DIRECTORY, directory);
        checks.push(
            names::NAMES,
            if badge.also_known_as == agent.also_known_as {
                Ok(badge.also_known_as.join(", "))
            } else {
                Err("the badge and the DID document name the agent differently".into())
            },
        );

        // 4. In force.
        let now = Utc::now();
        checks.push(
            names::STATUS,
            match badge.status {
                BadgeStatus::Active if badge.expires_at > now => Ok(format!(
                    "active; this badge is good until {}",
                    badge.expires_at.format("%Y-%m-%d %H:%M UTC")
                )),
                BadgeStatus::Active => Err("the badge has expired".into()),
                BadgeStatus::Frozen => Err("the owner froze this agent".into()),
                BadgeStatus::Revoked => Err("the owner revoked this agent".into()),
            },
        );

        // The owner's signed statement about the agent's credentials.
        checks.push(
            names::SECRETS,
            match &badge.credentials {
                Some(c) if !c.holds_secrets && c.paypal_access == "issued_per_call" => {
                    Ok("PayPal access is issued per call; the owner's signed statement".into())
                }
                _ => Err("the badge doesn't say the agent holds no secrets".into()),
            },
        );

        // 5. Agentrix vouches for the site.
        let site = self.resolvers.resolve(&site_did).await.ok();
        let voucher = match site.as_ref().and_then(|s| s.service("AgentrixVoucher")) {
            Some(url) => self.web.fetch_json(url).await.map_err(|e| e.to_string()),
            None => Err("the site names no voucher".into()),
        };
        let operator_method = format!("{}#owner", self.operator);
        let voucher_check = match &voucher {
            Ok(v) => match self.signed_by(v, &operator_method).await {
                Ok(()) => match serde_json::from_value::<Voucher>(v.clone()) {
                    Ok(terms) if terms.site == site_did => {
                        let cited = badge.voucher.as_deref();
                        if cited == jws::digest(v).ok().as_deref() {
                            Ok(terms.statement)
                        } else {
                            Err("the badge cites another voucher".into())
                        }
                    }
                    Ok(_) => Err("the voucher is for another site".into()),
                    Err(e) => Err(format!("it isn't a voucher: {e}")),
                },
                Err(e) => Err(format!("the voucher's signature doesn't check: {e}")),
            },
            Err(e) => Err(e.clone()),
        };
        let voucher_check =
            voucher_check.map(|statement| format!("{statement} (signed by {})", self.operator));
        checks.push(names::VOUCHER, voucher_check);

        // 6. The PayPal binding, when the badge cites one.
        if let Some(cited) = &badge.paypal {
            let check = match site.as_ref().and_then(|s| s.service("PayPalBinding")) {
                Some(url) => self.paypal(url, &owner, &cited.digest, &badge).await,
                None => Err("the site names no PayPal binding".into()),
            };
            checks.push(names::PAYPAL, check);
        }

        // 7. The site's A2A card, when it is signed.
        if let Some(url) = site.as_ref().and_then(|s| s.service("A2AAgentCard")) {
            if let Ok(card) = self.web.fetch_json(url).await {
                if card.get("signatures").is_some() {
                    let check = match jws::card_kid(&card) {
                        Ok(kid) if kid == owner => match self.key_of(&kid).await {
                            Ok(key) => jws::verify_card(&card, &key)
                                .map(|()| kid.clone())
                                .map_err(|e| e.to_string()),
                            Err(e) => Err(e),
                        },
                        Ok(kid) => {
                            Err(format!("the card is signed by {kid}, not the site's owner"))
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    checks.push(names::CARD, check);
                }
            }
        }
        checks.report(did, Some(value))
    }
}

#[derive(Deserialize)]
struct Asked {
    did: String,
}

/// `GET /agents/verify?did=<did>` (and `/badge/verify?did=<did>`): the [`VerifyReport`] as
/// JSON.
pub fn router(verifier: Arc<Verifier>) -> Router {
    Router::new()
        .route(VERIFY_PATH, get(verify_route))
        .route(BADGE_VERIFY_PATH, get(verify_route))
        .route(VERIFY_TOKEN_PATH, post(verify_token_route))
        .with_state(verifier)
}

async fn verify_route(
    State(verifier): State<Arc<Verifier>>,
    Query(asked): Query<Asked>,
) -> Response {
    axum::Json(verifier.verify(asked.did.trim()).await).into_response()
}

#[derive(Deserialize)]
struct Presented {
    token: String,
}

async fn verify_token_route(
    State(verifier): State<Arc<Verifier>>,
    axum::Json(presented): axum::Json<Presented>,
) -> Response {
    axum::Json(verifier.verify_token(presented.token.trim()).await).into_response()
}
