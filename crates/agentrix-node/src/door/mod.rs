//! The agent door: a site built for AI customers (R9; design §4.1, §4.4).
//!
//! Lane: `experience` (specs/first-release/design.md §3.0).
//!
//! Routes on a site host (merge this router into [`crate::site::router`]'s, which has the
//! fallback):
//! - `GET /.well-known/awp`: discovery, with the capability manifest an anonymous agent sees.
//! - `POST /mcp`: MCP over streamable HTTP (JSON responses). Tools:
//!   - `ask_site`: a cited answer from the site's own pages (read-only); the agent race's
//!     fallback (R9.3).
//!   - `request_custom_order`: held for the owner. It raises the owner's decision card and a
//!     `door.order-held` receipt, and answers with an AWP held call whose status the agent
//!     polls (R9.2). No payment this weekend.
//! - `GET /awp/v2/calls/{id}`: the held call's status.
//!
//! The order flow follows the framework session's MCP test in adk-rust
//! (`adk-website/tests/packs.rs`,
//! `a_custom_order_over_mcp_waits_for_the_owner_and_is_kept_once_approved`): `initialize` with
//! an `Mcp-Session-Id`, `tools/list`, `tools/call` with an idempotency key in `_meta`, a pending
//! held call in `structuredContent`, then polling until the owner decides.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentrix_kernel::{Action, Effect};
use async_trait::async_trait;
use awp_types::{
    ApprovalHint, CapabilityEffect, CapabilityManifestV2, CapabilityV2, HeldCall,
    IdempotencySource, TrustTier,
};
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::site::{ReceiptDraft, SiteState, ask, normalise_host, policy};

/// The MCP protocol versions the door speaks, newest first.
pub const MCP_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26"];

/// What an agent is told while the owner's Freeze or a platform hold is in force.
pub const PAUSED: &str = "Answers and orders are paused right now. Nothing was sent.";

/// What an agent is told when the owner's dials stop the agent desk holding orders.
pub const NOT_TAKING_ORDERS: &str =
    "This shop isn't taking orders through agents right now. Nothing was sent.";

/// What an agent is told when it calls a tool the door doesn't have.
const UNKNOWN_TOOL: &str = "Unknown tool. This door has ask_site and request_custom_order.";

/// What an agent is told when it sends a method the door doesn't answer.
const METHOD_NOT_FOUND: &str =
    "Method not found. The door answers initialize, ping, tools/list and tools/call.";

/// Where held capability calls live: the move lane's decisions module raises the owner's card
/// and records the outcome.
#[async_trait]
pub trait HeldCalls: Send + Sync {
    /// Holds a call for the owner and raises their decision card. The same `idempotency_key`
    /// on the same site returns the call it first created.
    async fn raise(
        &self,
        site_id: &str,
        capability: &str,
        input: Value,
        idempotency_key: &str,
    ) -> anyhow::Result<HeldCall>;
    /// The call's current state, or `None` when the site has no such call.
    async fn status(&self, site_id: &str, id: &str) -> anyhow::Result<Option<HeldCall>>;
}

/// What the door needs: the site state (releases, model, receipts) and the held calls.
#[derive(Clone)]
pub struct DoorState {
    /// The site state shared with [`crate::site::router`].
    pub site: SiteState,
    /// Held calls and the owner's cards.
    pub held: Arc<dyn HeldCalls>,
}

/// The door's routes, to merge into the site router.
pub fn router(state: DoorState) -> Router {
    Router::new()
        .route("/.well-known/awp", get(discovery))
        .route("/mcp", post(mcp))
        .route("/awp/v2/calls/{id}", get(call_status))
        .with_state(state)
}

/// The capabilities every site offers through the door in the first release.
pub fn capabilities() -> CapabilityManifestV2 {
    let mut ask = CapabilityV2::new(
        "ask_site",
        "Ask a question about this site. The answer comes only from its pages, with numbered sources; when the site doesn't say, the answer says so.",
        CapabilityEffect::ReadOnly,
    );
    ask.input_schema = Some(json!({
        "type": "object",
        "properties": {"question": {"type": "string", "maxLength": 800}},
        "required": ["question"],
        "additionalProperties": false
    }));
    let mut order = CapabilityV2::new(
        "request_custom_order",
        "Request a custom order. It is held for the owner, who approves or declines it; poll the returned status. Nothing is charged.",
        CapabilityEffect::NonIdempotent,
    )
    .with_approval(ApprovalHint::Always)
    .with_min_trust(TrustTier::Public);
    order.idempotency = IdempotencySource::IdempotencyKey;
    order.input_schema = Some(order_schema());
    CapabilityManifestV2::new(vec![ask, order])
}

fn order_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "description": {"type": "string", "maxLength": 2000, "description": "What to make, in the customer's words."},
            "date": {"type": "string", "format": "date", "description": "When it is wanted, YYYY-MM-DD."},
            "name": {"type": "string", "maxLength": 200},
            "email": {"type": "string", "format": "email", "maxLength": 254}
        },
        "required": ["description", "date", "name", "email"],
        "additionalProperties": false
    })
}

/// The site a request is for, from its Host: `(site_id, domain)`.
async fn site_of(state: &DoorState, headers: &HeaderMap) -> Option<(String, String)> {
    let host = headers
        .get(header::HOST)?
        .to_str()
        .ok()
        .map(normalise_host)?;
    let pointer = state.site.releases.source().live(&host).await.ok()??;
    Some((pointer.site_id, pointer.domain))
}

async fn discovery(State(state): State<DoorState>, headers: HeaderMap) -> Response {
    let Some((_, domain)) = site_of(&state, &headers).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let doc = json!({
        "awp": "2.0",
        "site": domain,
        "mcp": {"url": "/mcp", "protocolVersions": MCP_VERSIONS},
        "heldCalls": "/awp/v2/calls/{id}",
        "markdown": "Request a page with Accept: text/markdown to get it as Markdown, where the site has a Markdown copy.",
        "capabilities": capabilities().visible_to(TrustTier::Public),
        "terms": {
            "orders": "Every order is held for the owner's approval.",
            "payments": "No payment is taken through this door.",
            "answers": "Answers come only from this site's pages and cite them."
        }
    });
    (
        [
            (header::CONTENT_TYPE, "application/awp+json"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        doc.to_string(),
    )
        .into_response()
}

async fn call_status(
    State(state): State<DoorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some((site_id, _)) = site_of(&state, &headers).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.held.status(&site_id, &id).await {
        Ok(Some(call)) => ([(header::CACHE_CONTROL, "no-store")], Json(call)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(%e, "held call status failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_error(id: &Value, message: &str) -> Value {
    rpc_result(
        id,
        json!({"isError": true, "content": [{"type": "text", "text": message}]}),
    )
}

async fn mcp(
    State(state): State<DoorState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some((site_id, domain)) = site_of(&state, &headers).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = body.get("params").cloned().unwrap_or(Value::Null);
    let is_notification = body.get("id").is_none();

    match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = MCP_VERSIONS
                .iter()
                .find(|v| **v == asked)
                .copied()
                .unwrap_or(MCP_VERSIONS[0]);
            let result = json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": format!("{domain} agent door"), "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Answers come only from this site's pages, with citations. Orders are held for the owner's approval; poll the status URL. No payment is taken here."
            });
            let mut response = Json(rpc_result(&id, result)).into_response();
            if let Ok(v) = HeaderValue::from_str(&uuid::Uuid::new_v4().to_string()) {
                response.headers_mut().insert("mcp-session-id", v);
            }
            response
        }
        _ if is_notification => StatusCode::ACCEPTED.into_response(),
        "ping" => Json(rpc_result(&id, json!({}))).into_response(),
        "tools/list" => {
            let tools: Vec<Value> = capabilities()
                .visible_to(TrustTier::Public)
                .capabilities
                .iter()
                .map(|c| {
                    json!({
                        "name": c.name,
                        "description": c.description,
                        "inputSchema": c.input_schema,
                        "annotations": {
                            "readOnlyHint": c.effect == CapabilityEffect::ReadOnly,
                            "destructiveHint": false,
                            "idempotentHint": c.effect != CapabilityEffect::NonIdempotent,
                            "openWorldHint": false
                        }
                    })
                })
                .collect();
            Json(rpc_result(&id, json!({"tools": tools}))).into_response()
        }
        "tools/call"
            if state.site.guards.frozen(&site_id).await
                || state.site.guards.held(&site_id).await =>
        {
            // The owner's Freeze stops the door's WebAgents too (W5, handover-wiring 2b.1), and
            // so does a platform hold (W3, 3.2).
            Json(tool_error(&id, PAUSED)).into_response()
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let meta = params.get("_meta").cloned().unwrap_or(Value::Null);
            let agent = headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let reply = match name {
                "ask_site" => ask_site(&state, &headers, &id, &args).await,
                "request_custom_order" => {
                    request_custom_order(&state, &site_id, &id, &args, &meta, &agent).await
                }
                _ => rpc_error(&id, -32602, UNKNOWN_TOOL),
            };
            Json(reply).into_response()
        }
        _ => Json(rpc_error(&id, -32601, METHOD_NOT_FOUND)).into_response(),
    }
}

/// Checks a custom order's arguments; returns the cleaned input or a plain-words error.
pub fn validate_order(args: &Value, today: chrono::NaiveDate) -> Result<Value, String> {
    let obj = args.as_object().ok_or("Send the order as an object.")?;
    let allowed = ["description", "date", "name", "email"];
    if let Some(extra) = obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!(
            "Unknown field: {extra}. The fields are description, date, name and email."
        ));
    }
    let text = |key: &str, max: usize| -> Result<String, String> {
        let v = obj
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if v.is_empty() {
            return Err(format!("{key} is required."));
        }
        if v.chars().count() > max {
            return Err(format!("{key} is longer than {max} characters."));
        }
        Ok(v.to_owned())
    };
    let description = text("description", 2000)?;
    let name = text("name", 200)?;
    let email = text("email", 254)?;
    let (local, domain) = email
        .split_once('@')
        .ok_or("email is not an email address.")?;
    if local.is_empty() || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.')
    {
        return Err("email is not an email address.".into());
    }
    let date_text = text("date", 10)?;
    let date = chrono::NaiveDate::parse_from_str(&date_text, "%Y-%m-%d")
        .map_err(|_| "date must be YYYY-MM-DD.".to_owned())?;
    if date < today {
        return Err("date is in the past.".into());
    }
    Ok(json!({"description": description, "date": date_text, "name": name, "email": email}))
}

async fn request_custom_order(
    state: &DoorState,
    site_id: &str,
    id: &Value,
    args: &Value,
    meta: &Value,
    agent: &str,
) -> Value {
    let input = match validate_order(args, chrono::Utc::now().date_naive()) {
        Ok(v) => v,
        Err(message) => return tool_error(id, &message),
    };
    // The agent desk holds orders only while the owner's dials let it (W1, handover-wiring
    // 2a.1): a refusal raises no card.
    let holding = Action::new("agent-desk", "held-orders", Effect::Financial);
    if state.site.guards.refusal(site_id, &holding).await.is_some() {
        return tool_error(id, NOT_TAKING_ORDERS);
    }
    // Repeating the call must not repeat the order: the agent's key, else one derived from
    // the site and the order itself.
    let key = meta
        .get("agenticwebprotocol.com/idempotency-key")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let mut h = Sha256::new();
            h.update(site_id.as_bytes());
            h.update(input.to_string().as_bytes());
            format!("derived:{}", hex::encode(&h.finalize()[..16]))
        });
    let call = match state
        .held
        .raise(site_id, "request_custom_order", input.clone(), &key)
        .await
    {
        Ok(call) => call,
        Err(e) => {
            tracing::error!(%e, "could not hold the order");
            return tool_error(
                id,
                "The order could not be held right now. Nothing was sent; try again later.",
            );
        }
    };
    let draft = ReceiptDraft {
        kind: "door.order-held",
        site: site_id.to_owned(),
        subject: format!("call:{}", call.id),
        observed: json!({
            "call": call.id,
            "capability": "request_custom_order",
            "date": input["date"],
            "idempotency_key": key,
            "tier": "public",
            "agent": agent.chars().take(200).collect::<String>(),
        }),
    };
    if let Err(e) = state.site.receipts.write(draft).await {
        tracing::error!(%e, "could not write the door.order-held receipt");
    }
    let text = format!(
        "Held for the owner's approval. Nothing is charged. Poll {} for the decision.",
        call.status
    );
    rpc_result(
        id,
        json!({
            "isError": false,
            "content": [{"type": "text", "text": text}],
            "structuredContent": call
        }),
    )
}

async fn ask_site(state: &DoorState, headers: &HeaderMap, id: &Value, args: &Value) -> Value {
    let question: String = args
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(800)
        .collect();
    if question.is_empty() {
        return tool_error(id, "question is required.");
    }
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(normalise_host)
    else {
        return tool_error(id, "No site.");
    };
    let release = match state.site.releases.current(&host).await {
        Ok(Some(r)) => r,
        _ => return tool_error(id, "Answers are paused right now."),
    };
    let broken = policy::check(&question);
    if !broken.is_empty() {
        let reply = policy::reply(&broken);
        let draft = ReceiptDraft {
            kind: "concierge.refusal",
            site: release.pointer.site_id.clone(),
            subject: format!("release:{}", release.pointer.seq),
            observed: json!({
                "excerpt": policy::excerpt(&question),
                "rules": broken.iter().map(|r| r.id()).collect::<Vec<_>>(),
                "reply": reply,
                "channel": "agent door",
            }),
        };
        if let Err(e) = state.site.receipts.write(draft).await {
            tracing::error!(%e, "could not write the refusal receipt");
        }
        return rpc_result(
            id,
            json!({"isError": false, "content": [{"type": "text", "text": reply}], "structuredContent": {"kind": "refused", "text": reply}}),
        );
    }
    let frames: Arc<Mutex<Vec<ask::Frame>>> = Arc::default();
    let sink = frames.clone();
    let send = move |f: ask::Frame| {
        if let Ok(mut all) = sink.lock() {
            all.push(f);
        }
    };
    let site_name = release.pointer.domain.clone();
    // The agent door answers as the Concierge, under the owner's rules for it.
    let taught = state
        .site
        .guards
        .taught(&release.pointer.site_id, ask::WEBAGENT)
        .await;
    ask::answer(
        &state.site.releases,
        &release,
        state.site.model.as_ref(),
        &question,
        &site_name,
        &taught,
        &send,
    )
    .await;
    let frames = frames.lock().map(|f| f.clone()).unwrap_or_default();
    let mut citations: HashMap<u64, Value> = HashMap::new();
    let mut reply = json!({"kind": "dont_know", "text": ask::DONT_KNOW});
    for f in frames {
        match f.event {
            "awp.citation" => {
                if let Some(n) = f.data.get("n").and_then(Value::as_u64) {
                    citations.insert(n, f.data);
                }
            }
            "reply" => reply = f.data,
            "error" => {
                return tool_error(
                    id,
                    f.data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Answers are paused right now."),
                );
            }
            _ => {}
        }
    }
    let mut cited: Vec<(u64, Value)> = citations.into_iter().collect();
    cited.sort_by_key(|(n, _)| *n);
    let sources: Vec<Value> = cited.into_iter().map(|(_, v)| v).collect();
    let text = reply
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    rpc_result(
        id,
        json!({
            "isError": false,
            "content": [{"type": "text", "text": text}],
            "structuredContent": {"answer": reply, "sources": sources}
        }),
    )
}

#[cfg(test)]
mod tests;
