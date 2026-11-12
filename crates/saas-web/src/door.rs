//! `pay.agentrix.website`: the agent door.
//!
//! - `GET /awp/v2/manifest`, `/awp/v2/trust`, `/awp/v2/status`, `POST /awp/v2/capabilities/{name}`:
//!   adk-awp's AWP 2.0 edge. This slice declares the read-only plan capabilities (`list_plans`,
//!   `quote_plan`, from the platform's own price list); slice C adds its settlement
//!   capabilities with [`Door::capability`].
//! - `GET /.well-known/awp.json`: where the manifest is.
//! - `GET /.well-known/http-message-signatures-directory`: the agents' public keys (Web Bot Auth,
//!   IETF working-group draft), as a JWKS. Slice C supplies the keys ([`Door::keys`]).
//! - `GET /.well-known/agentrix/agents.json`: who each key belongs to.
//! - Everything else (`/a2a`, `/paypal/webhook`, checkout) is slice C's router, mounted with
//!   [`Door::routes`]. Until it is, those paths answer 503 with a JSON note.

use std::sync::Arc;

use adk_awp::v2::capability::{
    CapabilityCall, CapabilityError, CapabilityHandler, CapabilityRegistry,
};
use adk_awp::v2::routes::{EdgeState, awp_v2_routes};
use async_trait::async_trait;
use awp_types::{CapabilityEffect, CapabilityV2};
use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::{Value, json};

/// One agent's public signing key, as the key directory publishes it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentKey {
    /// The key id requests are signed with (`keyid` in `Signature-Input`).
    pub kid: String,
    /// `agentrix` or `kilele`.
    pub company: String,
    /// The agent's id.
    pub agent: String,
    /// The Ed25519 public key, base64url without padding (the JWK's `x`).
    pub x: String,
}

/// A capability slice C adds to the door, with the code behind it.
pub struct ExtraCapability {
    /// The declaration published in the manifest.
    pub declaration: CapabilityV2,
    /// The code that runs it.
    pub handler: Arc<dyn CapabilityHandler>,
}

/// What the agent door serves beyond its own pages.
#[derive(Default)]
pub struct Door {
    keys: Vec<AgentKey>,
    routes: Option<Router>,
    extra: Vec<ExtraCapability>,
}

impl Door {
    /// An empty door: plan capabilities only, no keys, no A2A.
    pub fn new() -> Self {
        Self::default()
    }

    /// The agent keys this door publishes (shown on `/proofs` too).
    pub fn published_keys(&self) -> &[AgentKey] {
        &self.keys
    }

    /// Publishes these agent keys in the directory (when slice C's routes don't serve one).
    pub fn keys(mut self, keys: Vec<AgentKey>) -> Self {
        self.keys = keys;
        self
    }

    /// Mounts slice C's routes (A2A, the PayPal webhook, checkout) on the door.
    pub fn routes(mut self, routes: Router) -> Self {
        self.routes = Some(routes);
        self
    }

    /// Declares one more AWP capability.
    pub fn capability(
        mut self,
        declaration: CapabilityV2,
        handler: Arc<dyn CapabilityHandler>,
    ) -> Self {
        self.extra.push(ExtraCapability {
            declaration,
            handler,
        });
        self
    }

    /// The door's router for the pay host. `domain` is the door's public name; `feed` and
    /// `today` answer `/.well-known/agentrix/agents.json` with each company's staff as it is now.
    pub fn router(
        self,
        domain: &str,
        feed: Arc<dyn crate::feed::FeedSource>,
        today: crate::agent::Today,
    ) -> anyhow::Result<Router> {
        let mut registry = CapabilityRegistry::new(domain);
        let mut list = CapabilityV2::new(
            "list_plans",
            "Agentrix's WebAgent plans, with monthly and 12-month prices in US cents.",
            CapabilityEffect::ReadOnly,
        );
        list.input_schema = Some(json!({"type": "object", "additionalProperties": false}));
        registry.declare(list, Arc::new(ListPlans))?;
        let mut quote = CapabilityV2::new(
            "quote_plan",
            "The price of one plan for a period (month or year), in US cents.",
            CapabilityEffect::ReadOnly,
        );
        quote.input_schema = Some(json!({
            "type": "object",
            "required": ["plan"],
            "properties": {
                "plan": {"type": "string", "description": "essential, business, commerce …"},
                "period": {"type": "string", "enum": ["month", "year"]}
            },
            "additionalProperties": false
        }));
        registry.declare(quote, Arc::new(QuotePlan))?;
        for c in self.extra {
            registry.declare(c.declaration, c.handler)?;
        }

        let keys = Arc::new(self.keys);
        let domain_owned = domain.to_owned();
        let mut router = Router::new()
            .route("/", get(page))
            .route(
                "/.well-known/awp.json",
                get(move || {
                    let domain = domain_owned.clone();
                    async move { awp_discovery(&domain) }
                }),
            )
            .route(
                "/.well-known/agentrix/agents.json",
                get(move || {
                    let feed = Arc::clone(&feed);
                    let today = Arc::clone(&today);
                    async move { who_signs(feed.as_ref(), today()).await }
                }),
            )
            .merge(awp_v2_routes(EdgeState::builder(registry).build()));
        router = match self.routes {
            // Slice C's door serves its own key directory, agent card and A2A.
            Some(extra) => router.merge(extra),
            None => router
                .route(
                    "/.well-known/http-message-signatures-directory",
                    get(move || {
                        let keys = Arc::clone(&keys);
                        async move { directory(&keys) }
                    }),
                )
                .route("/a2a", axum::routing::any(not_mounted))
                .route("/a2a/{*rest}", axum::routing::any(not_mounted))
                .route("/paypal/webhook", axum::routing::any(not_mounted)),
        };
        Ok(router)
    }
}

async fn page() -> Response {
    crate::pages::render_static("saas/door.html")
}

fn awp_discovery(domain: &str) -> Response {
    let body = json!({
        "awp": "2.0",
        "domain": domain,
        "manifest": format!("https://{domain}/awp/v2/manifest"),
        "trust": format!("https://{domain}/awp/v2/trust"),
        "status": format!("https://{domain}/awp/v2/status"),
        "keys": format!("https://{domain}/.well-known/http-message-signatures-directory"),
        "agents": format!("https://{domain}/.well-known/agentrix/agents.json"),
        "a2a": format!("https://{domain}/a2a"),
        "operator": "Agentrix (Zavora Technologies Ltd)",
    });
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        body.to_string(),
    )
        .into_response()
}

fn directory(keys: &[AgentKey]) -> Response {
    let jwks: Vec<Value> = keys
        .iter()
        .map(|k| json!({"kty": "OKP", "crv": "Ed25519", "kid": k.kid, "x": k.x, "use": "sig", "alg": "EdDSA"}))
        .collect();
    (
        [
            (
                header::CONTENT_TYPE,
                "application/http-message-signatures-directory+json",
            ),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        json!({ "keys": jwks }).to_string(),
    )
        .into_response()
}

/// Each company and its agents, with the key id each signs requests with.
async fn who_signs(feed: &dyn crate::feed::FeedSource, today: chrono::NaiveDate) -> Response {
    match feed.feed(today).await {
        Ok(f) => {
            let companies: Vec<Value> = f
                .companies
                .iter()
                .map(|c| {
                    json!({
                        "company": c.id, "name": c.name, "host": c.host,
                        "agents": c.agents.iter().map(|a| json!({"id": a.id, "name": a.name, "job": a.job, "keyId": a.key_id})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            axum::Json(json!({ "companies": companies })).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "door: reading the staff");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

async fn not_mounted() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        json!({
            "error": "not_mounted",
            "message": "Agent-to-agent settlement mounts here (slice C). Nothing was received or charged."
        })
        .to_string(),
    )
        .into_response()
}

/// The plans as the door lists them, from the platform's catalogue (agentrix-billing).
pub fn plans() -> Value {
    let cat = agentrix_billing::catalogue();
    let rows: Vec<Value> = cat
        .plans
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "currency": "USD",
                "monthlyCents": p.price_cents,
                "annualCents": p.annual_cents(),
                "includes": p.includes,
                "webagents": p.webagents,
            })
        })
        .collect();
    json!({ "version": cat.version, "annualDiscountPercent": cat.annual_discount_percent, "plans": rows })
}

struct ListPlans;

#[async_trait]
impl CapabilityHandler for ListPlans {
    async fn handle(&self, _call: CapabilityCall) -> Result<Value, CapabilityError> {
        Ok(plans())
    }
}

struct QuotePlan;

#[async_trait]
impl CapabilityHandler for QuotePlan {
    async fn handle(&self, call: CapabilityCall) -> Result<Value, CapabilityError> {
        let id = call
            .input
            .get("plan")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let period = call
            .input
            .get("period")
            .and_then(Value::as_str)
            .unwrap_or("month");
        let plan = agentrix_billing::catalogue()
            .plans
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| {
                CapabilityError::refused(
                    awp_types::v2::ErrorCode::ResourceNotFound,
                    format!("No plan called {id:?}."),
                )
            })?;
        let cents = if period == "year" {
            plan.annual_cents()
        } else {
            plan.price_cents
        };
        Ok(
            json!({ "plan": plan.id, "name": plan.name, "period": period, "currency": "USD", "cents": cents }),
        )
    }
}
