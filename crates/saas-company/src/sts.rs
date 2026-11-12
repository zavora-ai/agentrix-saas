//! The company's token service: RFC 8693 token exchange, so each agent trades its own credential
//! for a short-lived token bound to one resource (the books, PayPal, the agent door) and only the
//! scopes its role is granted.
//!
//! - [`TokenService`] is the token endpoint (`POST /oauth/token`). It knows each agent's
//!   credential (minted at start, never shown to a model), checks the requested resource and
//!   scopes against [`crate::identity::granted_scopes`], and issues an opaque token it can
//!   introspect later ([`TokenService::introspect`]), which is how the books and PayPal adapters
//!   check a call's token.
//! - [`exchange_credentials`] gives the company's [`StaffCredentials`] over adk-auth's
//!   `TokenExchangeBroker`, one per agent (each over its own identity), behind a broker that
//!   routes by the acting agent.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use adk_auth::TokenExchangeBroker;
use adk_core::{
    ActingContext, AdkError, AgentCredential, CredentialBroker, CredentialKind, RevocationScope,
    ScopedToken, StaticIdentitySource, Url,
};
use async_trait::async_trait;
use axum::extract::{Form, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use crate::identity::{
    ROLES, Resource, Resources, StaffCredentials, StaffIdentity, granted_scopes,
};

const GRANT: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ACCESS: &str = "urn:ietf:params:oauth:token-type:access_token";

/// A token the service issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    /// The agent it was issued to.
    pub agent: String,
    /// The resource it is bound to.
    pub audience: String,
    /// What it allows.
    pub scopes: Vec<String>,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
}

/// One company's token endpoint.
pub struct TokenService {
    company: String,
    resources: Resources,
    /// Agent credential → the agent.
    agents: HashMap<String, StaffIdentity>,
    issued: Mutex<HashMap<String, Issued>>,
    /// Agents the owner switched off: no exchange, no introspection.
    revoked: Mutex<std::collections::BTreeSet<String>>,
    ttl: Duration,
}

impl std::fmt::Debug for TokenService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenService")
            .field("company", &self.company)
            .finish_non_exhaustive()
    }
}

fn mint() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl TokenService {
    /// The service for `company`, with a fresh credential for each of its agents. Returns the
    /// credentials by agent id, to load into each agent's identity source.
    pub fn new(
        company: &str,
        founder: &str,
        resources: Resources,
    ) -> anyhow::Result<(Arc<Self>, BTreeMap<String, String>)> {
        let mut agents = HashMap::new();
        let mut credentials = BTreeMap::new();
        for role in ROLES {
            let staff = StaffIdentity::new(company, role, founder)?;
            let credential = mint();
            credentials.insert(staff.id().as_str().to_owned(), credential.clone());
            agents.insert(credential, staff);
        }
        Ok((
            Arc::new(Self {
                company: company.to_owned(),
                resources,
                agents,
                issued: Mutex::new(HashMap::new()),
                revoked: Mutex::new(std::collections::BTreeSet::new()),
                ttl: Duration::minutes(15),
            }),
            credentials,
        ))
    }

    /// The token endpoint: `POST /oauth/token`.
    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/oauth/token", post(exchange))
            .with_state(self.clone())
    }

    /// What `token` allows, if the service issued it, it hasn't lapsed at `now`, and its agent
    /// isn't revoked.
    pub fn introspect(&self, token: &str, now: DateTime<Utc>) -> Option<Issued> {
        self.issued
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(token)
            .filter(|i| now < i.expires_at)
            .filter(|i| !self.is_revoked(&i.agent))
            .cloned()
    }

    /// Switches `agent` off: its tokens stop at their next check, and it gets no new ones. No
    /// PayPal key changes. Returns how many live tokens it held.
    pub fn revoke(&self, agent: &str) -> usize {
        self.revoked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(agent.to_owned());
        let mut issued = self.issued.lock().unwrap_or_else(|p| p.into_inner());
        let before = issued.len();
        issued.retain(|_, i| i.agent != agent);
        before - issued.len()
    }

    /// Switches `agent` back on.
    pub fn restore(&self, agent: &str) {
        self.revoked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(agent);
    }

    /// Whether `agent` is switched off.
    pub fn is_revoked(&self, agent: &str) -> bool {
        self.revoked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(agent)
    }

    /// The agents switched off.
    pub fn revoked(&self) -> Vec<String> {
        self.revoked
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn resource_of(&self, url: &str) -> Option<Resource> {
        [Resource::Books, Resource::PayPal, Resource::AgentDoor]
            .into_iter()
            .find(|r| self.resources.url(*r).as_str() == url)
    }
}

fn refuse(status: StatusCode, error: &str, description: String) -> Response {
    (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

async fn exchange(
    State(sts): State<Arc<TokenService>>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let get = |k: &str| form.get(k).map(String::as_str).unwrap_or_default();
    if get("grant_type") != GRANT {
        return refuse(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "only token exchange is served".into(),
        );
    }
    let Some(staff) = sts.agents.get(get("subject_token")) else {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "invalid_grant",
            "that credential isn't one of this company's agents".into(),
        );
    };
    if sts.is_revoked(staff.id().as_str()) {
        tracing::info!(agent = %staff.id(), "token exchange refused: the owner revoked this agent");
        return refuse(
            StatusCode::FORBIDDEN,
            "access_denied",
            format!("the owner revoked {}", staff.id()),
        );
    }
    let audience = if form.contains_key("resource") {
        get("resource")
    } else {
        get("audience")
    };
    let Some(resource) = sts.resource_of(audience) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            format!("{audience} isn't a resource of {}", sts.company),
        );
    };
    let granted: Vec<String> = granted_scopes(&staff.role)
        .remove(&resource)
        .unwrap_or_default()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let asked: Vec<String> = get("scope").split_whitespace().map(str::to_owned).collect();
    let scopes = if asked.is_empty() {
        granted.clone()
    } else {
        asked
    };
    if scopes.is_empty() || scopes.iter().any(|s| !granted.contains(s)) {
        tracing::info!(agent = %staff.id(), ?scopes, "token exchange refused: scope not granted");
        return refuse(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            format!("{} isn't granted {}", staff.role, scopes.join(" ")),
        );
    }
    let token = mint();
    let issued = Issued {
        agent: staff.id().as_str().to_owned(),
        audience: audience.to_owned(),
        scopes: scopes.clone(),
        expires_at: Utc::now() + sts.ttl,
    };
    sts.issued
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(token.clone(), issued);
    Json(json!({
        "access_token": token,
        "issued_token_type": ACCESS,
        "token_type": "Bearer",
        "expires_in": sts.ttl.num_seconds(),
        "scope": scopes.join(" "),
    }))
    .into_response()
}

impl crate::identity::TokenCheck for TokenService {
    fn revoked(&self, agent: &str) -> bool {
        self.is_revoked(agent)
    }

    fn holds(&self, agent: &str, token: &str) -> bool {
        self.introspect(token, Utc::now())
            .is_some_and(|issued| issued.agent == agent)
    }
}

/// Routes each request to the broker of the acting agent: adk-auth's exchange broker works over
/// one agent's own identity.
#[derive(Debug)]
pub struct PerAgentBroker {
    brokers: BTreeMap<String, Arc<dyn CredentialBroker>>,
}

#[async_trait]
impl CredentialBroker for PerAgentBroker {
    async fn token_for(
        &self,
        acting: &ActingContext,
        resource: &Url,
        scopes: &[String],
    ) -> adk_core::Result<ScopedToken> {
        let broker = self
            .brokers
            .get(acting.agent.id.as_str())
            .ok_or_else(|| AdkError::agent(format!("no credential for {}", acting.agent.id)))?;
        broker.token_for(acting, resource, scopes).await
    }

    async fn revoke(&self, scope: &RevocationScope) -> adk_core::Result<u64> {
        let mut n = 0;
        for broker in self.brokers.values() {
            n += broker.revoke(scope).await?;
        }
        Ok(n)
    }
}

/// The company's credentials over RFC 8693 token exchange at `endpoint`, with each agent's own
/// credential from `credentials` (what [`TokenService::new`] returned).
pub fn exchange_credentials(
    company: &str,
    founder: &str,
    endpoint: Url,
    resources: Resources,
    credentials: &BTreeMap<String, String>,
) -> anyhow::Result<StaffCredentials> {
    let mut brokers: BTreeMap<String, Arc<dyn CredentialBroker>> = BTreeMap::new();
    for role in ROLES {
        let staff = StaffIdentity::new(company, role, founder)?;
        let id = staff.id().clone();
        let Some(secret) = credentials.get(id.as_str()) else {
            continue;
        };
        let source = StaticIdentitySource::new(staff.principal.clone()).with_credential(
            AgentCredential::new(id.clone(), CredentialKind::Opaque, secret.clone()),
        );
        brokers.insert(
            id.as_str().to_owned(),
            Arc::new(TokenExchangeBroker::new(endpoint.clone(), Arc::new(source))),
        );
    }
    Ok(StaffCredentials::new(
        Arc::new(PerAgentBroker { brokers }),
        resources,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{PAYABLES, SUPPORT};

    #[tokio::test]
    async fn agents_exchange_their_credentials_for_tokens_bound_to_one_resource_and_role() {
        let resources = Resources::hosted("agentrix");
        let (sts, credentials) =
            TokenService::new("agentrix", "f@agentrix.website", resources.clone()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = sts.router();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let endpoint = Url::parse(&format!("http://{addr}/oauth/token")).unwrap();
        let creds = exchange_credentials(
            "agentrix",
            "f@agentrix.website",
            endpoint,
            resources,
            &credentials,
        )
        .unwrap();

        let payables = StaffIdentity::new("agentrix", PAYABLES, "f@agentrix.website").unwrap();
        let token = creds
            .token(&payables.acting(), Resource::PayPal, "paypal:payout")
            .await
            .unwrap();
        let issued = sts
            .introspect(token.secret.expose(), Utc::now())
            .expect("the service issued it");
        assert_eq!(issued.agent, "urn:adk:agent:agentrix/payables");
        assert_eq!(issued.scopes, vec!["paypal:payout".to_owned()]);
        assert_eq!(issued.audience, "https://api-m.sandbox.paypal.com/");
        assert!(
            token
                .authorize_for(&Url::parse("https://books.agentrix.website/agentrix/").unwrap())
                .is_err(),
            "a PayPal token is no good at the books"
        );

        let support = StaffIdentity::new("agentrix", SUPPORT, "f@agentrix.website").unwrap();
        assert!(
            creds
                .token(&support.acting(), Resource::PayPal, "paypal:payout")
                .await
                .is_err(),
            "Support is never granted payouts"
        );
        let stranger = StaffIdentity::new("kilele", PAYABLES, "f@kilele.example").unwrap();
        assert!(
            creds
                .token(&stranger.acting(), Resource::PayPal, "paypal:payout")
                .await
                .is_err()
        );
    }
}
