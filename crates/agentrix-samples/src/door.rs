//! The agent door on the sample hosts (01a §2.9: M-01 to M-04 and M-09; D8, D9).
//!
//! Visiting AI agents find the door in the AI Catalog, read what this site offers in the AWP
//! descriptor, and call MCP tools at `/mcp` (JSON-RPC 2.0 over HTTP POST, JSON responses):
//!
//! | Tool | Tier | What it does |
//! |---|---|---|
//! | `catalog.search` | anonymous | Products with price, stock left and page, from records |
//! | `delivery.slots` | anonymous | Open slots (taken of capacity) and zones with fees |
//! | `quote.create` | known | Lines or a cake priced from the live price list, KSh and USD |
//! | `checkout.create` | known | A bag on the site and a link for a person to review and pay |
//! | `request_custom_order` | known, always held | A custom cake held for the owner |
//! | `order.status` | anonymous | An order's state in words, never personal details |
//! | `ask_site` | anonymous | The same answer engine as Ask, with its sources |
//!
//! The news archetype lists `ask_site` and `stories.search` only.
//!
//! Nothing at the door takes payment or a credential: checkout links open the site's own bag,
//! where a person signs in and pays. Tiers follow D9. An agent is `known` only when the request
//! proves who it is (v3, "Verify at sample doors"): a Web Bot Auth signature (RFC 9421) checked
//! against the key directory its `Signature-Agent` names, or a bearer token from an issuer the
//! site trusts, addressed to this door ([`crate::proof`]). A name alone (`clientInfo.name` in
//! `initialize`, a bare `Signature-Agent` header) is recorded as claimed and gives no rights:
//! the known-only tools refuse it before anything is created, with a signed receipt. What a
//! proven agent leaves behind names its verified id and who vouched for it.
//!
//! Every list of tools, tiers and terms (the descriptor, `tools/list`, the `/agents` page and
//! its terms) comes from [`tools`], so the page can't promise what the door doesn't enforce.

use std::collections::HashMap;
use std::sync::Mutex;

use agentrix_records::orders::{
    Cart, CartLine, CartView, Contact, OrderError, Placement, Zone, state_words,
};
use agentrix_records::pricing::{CAKE_GROUPS, CakeChoice};
use agentrix_records::{Records, Site, money, time};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::App;
use crate::shell::{Desk, Shell, desks, front};
use crate::web::{self, Ctx, Page, html, see_other};

/// The MCP protocol versions the door speaks, newest first (as the first release's door).
pub const MCP_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26"];

/// The words every place that states the terms carries (D8).
pub const SAMPLE_TERMS: &str = "Sample terms, not legal advice.";

/// The metadata key an agent sends an idempotency key under (as the first release's door).
const IDEMPOTENCY_KEY: &str = "agenticwebprotocol.com/idempotency-key";

/// The door's routes, merged into the sample host router.
pub fn routes() -> Router<App> {
    Router::new()
        .route("/.well-known/ai-catalog.json", get(catalog))
        .route("/.well-known/awp", get(descriptor))
        .route("/.well-known/awp.json", get(descriptor_json))
        .route("/mcp", get(mcp_get).post(mcp))
        .route("/awp/v2/calls/{id}", get(held_call))
        .route("/bag/claim", get(claim))
        .route("/agents", get(agents))
}

/// Who may call a tool (D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// Any agent, named or not.
    Anonymous,
    /// An agent that proves who it is: a Web Bot Auth signature or a trusted bearer token.
    Known,
}

impl Tier {
    fn word(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Known => "known",
        }
    }
}

/// A tool at the door, as `tools/list`, the descriptor and `/agents` describe it.
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    /// `catalog.search`.
    pub name: String,
    /// "Search the catalogue".
    pub title: String,
    /// What it does, for agents.
    pub description: String,
    /// Who may call it.
    pub tier: Tier,
    /// `read_only`, `creates_bag` or `held_for_owner`.
    pub effect: &'static str,
    /// Always held for the owner.
    pub held: bool,
    /// What an agent may do with it, in the words of the `/agents` page.
    pub may: String,
    /// The JSON Schema of its arguments.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

fn owner_of(site: &Site) -> String {
    let o = site.fact("/owner/first");
    if o.is_empty() { "the owner".into() } else { o }
}

/// The tools this site offers at the door, from its records (cake codes come from the live
/// price list).
///
/// # Errors
///
/// On a database error.
pub async fn tools(records: &Records, site: &Site) -> anyhow::Result<Vec<ToolSpec>> {
    let ask = ToolSpec {
        name: "ask_site".into(),
        title: "Ask the site".into(),
        description: format!(
            "Ask a question about {}. The answer comes only from this site's pages and records, with numbered sources; when the site doesn't say, the answer says so. Each answer says whether an AI WebAgent wrote it or the site's records answered it directly.",
            site.name
        ),
        tier: Tier::Anonymous,
        effect: "read_only",
        held: false,
        may: "ask a question, answered from this site's pages with sources".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"question": {"type": "string", "maxLength": 400}},
            "required": ["question"],
            "additionalProperties": false
        }),
    };
    if site.archetype == "news" {
        let stories = ToolSpec {
            name: "stories.search".into(),
            title: "Search stories".into(),
            description: format!(
                "Search {}'s published stories by words. Returns headline, date, byline and link. Subscriber stories return headline and link only.",
                site.name
            ),
            tier: Tier::Anonymous,
            effect: "read_only",
            held: false,
            may: "search stories (subscriber stories show headline and link only)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"query": {"type": "string", "maxLength": 200}},
                "required": ["query"],
                "additionalProperties": false
            }),
        };
        return Ok(vec![stories, ask]);
    }
    let owner = owner_of(site);
    let lines = json!({
        "type": "array", "minItems": 1, "maxItems": 20,
        "items": {
            "type": "object",
            "properties": {
                "sku": {"type": "string", "description": "From catalog.search."},
                "qty": {"type": "integer", "minimum": 1, "maximum": 20}
            },
            "required": ["sku", "qty"],
            "additionalProperties": false
        }
    });
    let zone = json!({"type": "string", "description": "A zone id or an area name from delivery.slots, such as \"Kilimani\"."});
    let slot = json!({"type": "string", "description": "A slot_id from delivery.slots."});
    let fulfilment = json!({"type": "string", "enum": ["delivery", "collection"]});
    let cake = cake_schema(records, site).await?;
    let mut out = vec![
        ToolSpec {
            name: "catalog.search".into(),
            title: "Search the catalogue".into(),
            description: format!(
                "Search {}'s catalogue by words. Returns name, price (minor units and text), size, family, stock left with the time it was read, and the product's page. An empty query lists everything.",
                site.name
            ),
            tier: Tier::Anonymous,
            effect: "read_only",
            held: false,
            may: "search the catalogue, with prices and stock left".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "maxLength": 200},
                    "section": {"type": "string", "description": "Only this section, such as \"bread\"."},
                    "max_price": {"type": "number", "description": format!("Highest price in {}, in whole units (450 means {}).", site.currency, money::fmt(&site.currency, 45_000))}
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "delivery.slots".into(),
            title: "Check delivery and collection slots".into(),
            description: "Open delivery or collection slots, each with how many orders it has taken and how many it holds, and the delivery zones with their fees.".into(),
            tier: Tier::Anonymous,
            effect: "read_only",
            held: false,
            may: "check delivery and collection slots and fees".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "kind": fulfilment,
                    "from_day": {"type": "string", "format": "date", "description": "First day, YYYY-MM-DD. Default today."},
                    "days": {"type": "integer", "minimum": 1, "maximum": 21, "description": "How many days. Default 7."}
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "quote.create".into(),
            title: "Get a quote".into(),
            description: format!(
                "Price catalogue lines (with an optional zone and slot){} from the live price list. Returns priced lines, the total in {} and in USD at the posted rate, the price list version and until when the price holds. Nothing is reserved or charged.",
                if cake.is_some() { ", or a cake," } else { "" },
                site.currency
            ),
            tier: Tier::Known,
            effect: "read_only",
            held: false,
            may: "get a quote priced from the price list".into(),
            input_schema: {
                let mut props = json!({"lines": lines, "zone": zone, "slot_id": slot});
                if let Some(c) = &cake {
                    props["cake"] = c.clone();
                }
                json!({"type": "object", "properties": props, "additionalProperties": false})
            },
        },
        ToolSpec {
            name: "checkout.create".into(),
            title: "Make a checkout link".into(),
            description: format!(
                "Put lines in a new bag on {} and return a link for a person to open, review, sign in and pay on the site. Nothing is charged until that person pays. The door never takes payment or a card.",
                site.name
            ),
            tier: Tier::Known,
            effect: "creates_bag",
            held: false,
            may: "make a checkout link for a person to review and pay".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"lines": lines, "fulfilment": fulfilment, "zone": zone, "slot_id": slot},
                "required": ["lines", "fulfilment"],
                "additionalProperties": false
            }),
        },
    ];
    if let Some(c) = cake {
        out.push(ToolSpec {
            name: "request_custom_order".into(),
            title: "Request a custom cake".into(),
            description: format!(
                "Request a custom cake priced from the live price list. It is always held for {owner}, who approves or declines it. Poll the returned status URL. Nothing is charged until she approves and the customer pays on the site."
            ),
            tier: Tier::Known,
            effect: "held_for_owner",
            held: true,
            may: format!("request a custom cake, held for {owner}"),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "cake": c,
                    "name": {"type": "string", "maxLength": 120, "description": "The name on the order."},
                    "email": {"type": "string", "format": "email", "maxLength": 254},
                    "phone": {"type": "string", "maxLength": 20},
                    "fulfilment": fulfilment,
                    "zone": zone,
                    "slot_id": slot,
                    "note": {"type": "string", "maxLength": 300}
                },
                "required": ["cake", "name", "fulfilment"],
                "additionalProperties": false
            }),
        });
    }
    out.push(ToolSpec {
        name: "order.status".into(),
        title: "Check an order".into(),
        description: "Where an order stands, in words, by its number. Never returns names, contact details or what was ordered.".into(),
        tier: Tier::Anonymous,
        effect: "read_only",
        held: false,
        may: "check an order's status by its number (status words only)".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"order_id": {"type": "string", "maxLength": 20, "description": format!("Such as {}-1042.", order_prefix(site))}},
            "required": ["order_id"],
            "additionalProperties": false
        }),
    });
    out.push(ask);
    Ok(out)
}

fn order_prefix(site: &Site) -> String {
    let p = site.fact("/order_prefix");
    if p.is_empty() { "ORD".into() } else { p }
}

/// The cake's schema with the live rule codes, or `None` when the site has no cake rules.
async fn cake_schema(records: &Records, site: &Site) -> anyhow::Result<Option<Value>> {
    if site.archetype != "bakery" {
        return Ok(None);
    }
    let Some(list) = records.live_price_list(&site.site).await? else {
        return Ok(None);
    };
    let rules = records.rules(&site.site, &list.version).await?;
    if !rules.iter().any(|r| r.grp == "size") {
        return Ok(None);
    }
    let mut props = serde_json::Map::new();
    for grp in CAKE_GROUPS {
        let codes: Vec<&str> = rules
            .iter()
            .filter(|r| r.grp == grp)
            .map(|r| r.code.as_str())
            .collect();
        let words: Vec<String> = rules
            .iter()
            .filter(|r| r.grp == grp)
            .map(|r| {
                format!(
                    "{} = {} ({})",
                    r.code,
                    r.label,
                    money::fmt(&site.currency, r.amount_minor)
                )
            })
            .collect();
        props.insert(
            grp.into(),
            json!({"type": "string", "enum": codes, "description": words.join("; ")}),
        );
    }
    let max = rules
        .iter()
        .find(|r| r.code == "message")
        .and_then(|r| r.meta().get("max_chars").and_then(Value::as_u64))
        .unwrap_or(22);
    props.insert(
        "message".into(),
        json!({"type": "string", "maxLength": max, "description": "The words on the cake."}),
    );
    props.insert("date".into(), json!({"type": "string", "format": "date", "description": "The day it is wanted, YYYY-MM-DD. Larger cakes need more notice."}));
    props.insert(
        "note".into(),
        json!({"type": "string", "maxLength": 300, "description": "Such as \"no nuts\"."}),
    );
    Ok(Some(json!({
        "type": "object",
        "description": format!("A cake from {}'s own options, codes from price list {}.", site.name, list.version),
        "properties": props,
        "required": ["size", "sponge", "colour", "finish", "deco", "date"],
        "additionalProperties": false
    })))
}

/// The terms, line by line: generated from the tools so they can't promise more than the door
/// enforces.
fn terms(site: &Site, tools: &[ToolSpec]) -> Vec<(String, String)> {
    let may = |t: Tier| {
        tools
            .iter()
            .filter(|s| s.tier == t)
            .map(|s| s.title.to_lowercase())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let known = may(Tier::Known);
    let mut out = vec![
        (
            "status".to_owned(),
            SAMPLE_TERMS.trim_end_matches('.').to_owned(),
        ),
        (
            "tiers".into(),
            if known.is_empty() {
                "anonymous".into()
            } else {
                "anonymous · known".into()
            },
        ),
        ("anyone".into(), format!("may {}", may(Tier::Anonymous))),
    ];
    if !known.is_empty() {
        out.push(("known".into(), format!("may also {known}")));
    }
    let held: Vec<String> = tools.iter().filter(|s| s.held).map(held_noun).collect();
    if !held.is_empty() {
        out.push((
            "waits".into(),
            format!("{} always wait for {}", held.join(" and "), owner_of(site)),
        ));
    }
    let payment = if site.archetype == "news" {
        "none at the door; subscriptions only on this site's own pages"
    } else {
        "only by a person, on this site's own payment page"
    };
    out.push(("payment".into(), payment.into()));
    out.push((
        "never".into(),
        "asks for a password, a code or a card".into(),
    ));
    out.push((
        "replies".into(),
        "from this site's records; each says whether an AI WebAgent wrote it".into(),
    ));
    out
}

/// What a held tool's calls are called in a sentence: "custom cake requests".
fn held_noun(t: &ToolSpec) -> String {
    match t.name.as_str() {
        "request_custom_order" => "custom cake requests".into(),
        _ => format!("{} calls", t.name),
    }
}

fn terms_text(lines: &[(String, String)]) -> String {
    lines
        .iter()
        .map(|(k, v)| format!("{k:<9}{v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// How the samples decide the tier, in words.
const KNOWN_HOW: &str = "An agent is known here only when each request proves who it is: sign it with Web Bot Auth (RFC 9421; Signature-Agent names your key directory, and the signature covers @method, @path and content-digest with a fresh nonce), or send a bearer token from an issuer this site trusts, addressed to this door. A name alone (clientInfo.name, or a Signature-Agent header without a signature) is recorded as claimed and gives no rights.";

// ---- Discovery ---------------------------------------------------------------------------

fn json_response(kind: &'static str, cache: &'static str, body: &Value) -> Response {
    (
        [(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, cache)],
        body.to_string(),
    )
        .into_response()
}

async fn catalog(Extension(ctx): Extension<Ctx>) -> Response {
    let o = &ctx.host.origin;
    let sample = ctx.site.is_sample();
    let body = json!({
        "name": format!("{}{}", ctx.site.name, if sample { " (Sample)" } else { "" }),
        "description": format!("Agent doors for {}. Answers and prices come from the site's own records; nothing here takes payment or a credential.", ctx.site.name),
        "sample": sample,
        "entries": [
            {"type": "mcp", "title": "MCP tools", "url": format!("{o}/mcp"), "transport": "streamable-http", "protocolVersions": MCP_VERSIONS},
            {"type": "awp", "title": "AWP descriptor: capabilities, tiers and terms", "url": format!("{o}/.well-known/awp")},
            {"type": "terms", "title": format!("Agent terms. {SAMPLE_TERMS}"), "url": format!("{o}/agents#terms")},
            {"type": "page", "title": "The agent door, for people", "url": format!("{o}/agents")}
        ]
    });
    json_response("application/json", "public, max-age=300", &body)
}

async fn descriptor_doc(app: &App, ctx: &Ctx) -> anyhow::Result<Value> {
    let records = app.records().await?;
    let tools = tools(records, &ctx.site).await?;
    let o = &ctx.host.origin;
    let caps: Vec<Value> = tools
        .iter()
        .map(|t| {
            let mut c = json!({"id": t.name, "title": t.title, "tier": t.tier, "effect": t.effect});
            if t.held {
                c["approval"] = json!("always");
                c["heldFor"] = json!(owner_of(&ctx.site));
            }
            c
        })
        .collect();
    let terms_map: serde_json::Map<String, Value> = terms(&ctx.site, &tools)
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    Ok(json!({
        "awp": "2.0",
        "site": ctx.site.name,
        "host": ctx.host.host,
        "sample": ctx.site.is_sample(),
        "release": ctx.site.fact("/release"),
        "mcp": {"url": format!("{o}/mcp"), "protocolVersions": MCP_VERSIONS},
        "heldCalls": format!("{o}/awp/v2/calls/{{id}}"),
        "agentsPage": format!("{o}/agents"),
        "capabilities": caps,
        "tiers": {
            "anonymous": "Any agent.",
            "known": "An agent that proves who it is: a Web Bot Auth signature or a trusted bearer token.",
            "how": KNOWN_HOW
        },
        "terms": terms_map,
        "termsUrl": format!("{o}/agents#terms")
    }))
}

async fn descriptor(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    Ok(json_response(
        "application/awp+json",
        "public, max-age=300",
        &descriptor_doc(&app, &ctx).await?,
    ))
}

async fn descriptor_json(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    Ok(json_response(
        "application/json",
        "public, max-age=300",
        &descriptor_doc(&app, &ctx).await?,
    ))
}

// ---- Who is calling ------------------------------------------------------------------------

/// The caller at the door.
#[derive(Debug, Clone)]
struct Caller {
    /// The name it gave (clientInfo or Signature-Agent): claimed, not proven.
    named: Option<String>,
    /// Its User-Agent, when it gave no name.
    agent: String,
    /// Who the request proved it is, once checked.
    proven: Option<crate::proof::ProvenAgent>,
}

impl Caller {
    fn tier(&self) -> Tier {
        if self.proven.is_some() {
            Tier::Known
        } else {
            Tier::Anonymous
        }
    }
    fn label(&self) -> String {
        if let Some(p) = &self.proven {
            return p.id.clone();
        }
        self.named.clone().unwrap_or_else(|| {
            if self.agent.is_empty() {
                "an unnamed agent".into()
            } else {
                self.agent.clone()
            }
        })
    }
}

fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .trim_matches('"')
        .trim()
        .chars()
        .take(80)
        .collect()
}

/// A session id that carries the client's name: `agx1.<random>.<hex name>`. Any instance can
/// read it back, so the door keeps no session state.
fn session_id(name: &str) -> String {
    let bytes: [u8; 8] = rand::random();
    format!(
        "agx1.{}.{}",
        hex::encode(bytes),
        hex::encode(name.as_bytes())
    )
}

/// The name in a session id; `None` when the id isn't one of ours.
fn session_name(id: &str) -> Option<String> {
    let mut parts = id.splitn(3, '.');
    if parts.next()? != "agx1" || parts.next()?.len() != 16 {
        return None;
    }
    let name = String::from_utf8(hex::decode(parts.next()?).ok()?).ok()?;
    Some(clean_name(&name))
}

fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(clean_name)
        .unwrap_or_default()
}

fn caller(headers: &HeaderMap) -> Caller {
    let from_session = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .and_then(session_name)
        .filter(|n| !n.is_empty());
    let signer = Some(header_text(headers, "signature-agent")).filter(|n| !n.is_empty());
    Caller {
        named: from_session.or(signer),
        agent: header_text(headers, "user-agent"),
        proven: None,
    }
}

static CALLS: Mutex<Option<HashMap<String, Vec<i64>>>> = Mutex::new(None);

/// At most `max` calls an hour for a key.
fn allow(key: &str, max: usize, now: i64) -> bool {
    let mut guard = CALLS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() > 10_000 {
        map.retain(|_, v| v.iter().any(|t| now - t < 3600));
    }
    let list = map.entry(key.to_owned()).or_default();
    list.retain(|t| now - t < 3600);
    if list.len() >= max {
        return false;
    }
    list.push(now);
    true
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

// ---- MCP -----------------------------------------------------------------------------------

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn rpc_response(body: &Value) -> Response {
    json_response("application/json", "no-store", body)
}

async fn mcp_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST"), (header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "The agent door speaks MCP over POST: send one JSON-RPC 2.0 message per request. People can read /agents.",
    )
        .into_response()
}

async fn mcp(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    http_method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return rpc_response(&rpc_error(
            &Value::Null,
            -32700,
            "Parse error: send one JSON-RPC 2.0 message as JSON.",
        ));
    };
    if msg.is_array() {
        return rpc_response(&rpc_error(
            &Value::Null,
            -32600,
            "Send one message per request; batches aren't accepted.",
        ));
    }
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let is_notification = msg.get("id").is_none();
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    if method.is_empty() {
        if is_notification || msg.get("result").is_some() || msg.get("error").is_some() {
            return StatusCode::ACCEPTED.into_response();
        }
        return rpc_response(&rpc_error(
            &id,
            -32600,
            "Invalid request: method is required.",
        ));
    }
    if method != "initialize"
        && let Some(sid) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok())
        && session_name(sid).is_none()
    {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            rpc_error(&id, -32001, "Unknown session. Send initialize again.").to_string(),
        )
            .into_response();
    }
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
            let name = params
                .pointer("/clientInfo/name")
                .and_then(Value::as_str)
                .map(clean_name)
                .unwrap_or_default();
            let owner = owner_of(&ctx.site);
            let instructions = if ctx.site.archetype == "news" {
                format!(
                    "Answers and stories come from {}'s own records. Subscriber stories show headline and link only. Each reply says whether an AI WebAgent wrote it.",
                    ctx.site.name
                )
            } else {
                format!(
                    "Prices come from {name}'s live price list and stock from today's records. Quotes, checkout links and custom orders need a proven agent: sign each request with Web Bot Auth or send a bearer token addressed to this door (see /agents); a name alone isn't enough. Checkout links are for a person to review and pay on the site; nothing is charged here. Custom orders are always held for {owner}. {SAMPLE_TERMS}",
                    name = ctx.site.name
                )
            };
            let result = json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {
                    "name": format!("{} agent door{}", ctx.site.name, if ctx.site.is_sample() { " (Sample)" } else { "" }),
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": instructions
            });
            let mut response = rpc_response(&rpc_result(&id, result));
            if let Ok(v) = HeaderValue::from_str(&session_id(&name)) {
                response.headers_mut().insert("mcp-session-id", v);
            }
            response
        }
        _ if is_notification => StatusCode::ACCEPTED.into_response(),
        "ping" => rpc_response(&rpc_result(&id, json!({}))),
        "tools/list" => match tools_list(&app, &ctx).await {
            Ok(v) => rpc_response(&rpc_result(&id, v)),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "door tools/list failed");
                rpc_response(&rpc_error(
                    &id,
                    -32603,
                    "The door couldn't read this site's records just now. Try again in a moment.",
                ))
            }
        },
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            let meta = params.get("_meta").cloned().unwrap_or(Value::Null);
            let request = Signed {
                method: &http_method,
                uri: &uri,
                body: &body,
            };
            rpc_response(&call(&app, &ctx, &headers, request, &id, name, &args, &meta).await)
        }
        _ => rpc_response(&rpc_error(
            &id,
            -32601,
            "Method not found. The door answers initialize, ping, tools/list and tools/call.",
        )),
    }
}

async fn tools_list(app: &App, ctx: &Ctx) -> anyhow::Result<Value> {
    let records = app.records().await?;
    let list: Vec<Value> = tools(records, &ctx.site)
        .await?
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": t.effect == "read_only",
                    "destructiveHint": false,
                    "idempotentHint": t.effect == "read_only",
                    "openWorldHint": false
                },
                "_meta": {
                    "agentrix/tier": t.tier, "agentrix/held": t.held, "agentrix/effect": t.effect,
                    "agentrix/proof": if t.tier == Tier::Known { "required: a Web Bot Auth signature or a trusted bearer token" } else { "none" }
                }
            })
        })
        .collect();
    Ok(json!({"tools": list}))
}

/// Why a tool didn't run.
enum Refusal {
    /// Words for the agent: what stops it and what to do.
    Words(String),
    /// Anything else (logged).
    Broken(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for Refusal {
    fn from(e: E) -> Self {
        Self::Broken(e.into())
    }
}

fn refuse<T>(words: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal::Words(words.into()))
}

/// A tool's output: text for the model and the same facts as structured content.
type Out = Result<(String, Value), Refusal>;

/// The parts of the request a proof covers.
#[derive(Clone, Copy)]
struct Signed<'a> {
    method: &'a axum::http::Method,
    uri: &'a axum::http::Uri,
    body: &'a [u8],
}

/// A known-only call from an unproven caller: refused before anything is created, with the
/// checker's signed receipt.
async fn refuse_unproven(
    ctx: &Ctx,
    id: &Value,
    tool: &str,
    who: &Caller,
    code: &str,
    reason: &str,
    body: &[u8],
) -> Value {
    let receipt = match crate::proof::agent_proof() {
        Some(proof) => {
            proof
                .refusal_receipt(crate::proof::DoorRefusal {
                    door: &ctx.host.origin,
                    site: &ctx.site.name,
                    tool,
                    code,
                    reason,
                    claimed: who.named.as_deref(),
                    body,
                })
                .await
        }
        None => None,
    };
    tracing::info!(site = %ctx.site.site, tool, code, claimed = ?who.named, "door: unproven agent refused");
    rpc_result(
        id,
        json!({
            "isError": true,
            "content": [{"type": "text", "text": format!("{reason} {KNOWN_HOW} More at {}/agents.", ctx.host.origin)}],
            "structuredContent": {
                "refused": true, "code": code, "claimed_agent": who.named,
                "nothing_moved": true, "receipt": receipt,
            },
        }),
    )
}

#[allow(clippy::too_many_arguments)]
async fn call(
    app: &App,
    ctx: &Ctx,
    headers: &HeaderMap,
    request: Signed<'_>,
    id: &Value,
    name: &str,
    args: &Value,
    meta: &Value,
) -> Value {
    let records = match app.records().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "door records failed");
            return tool_error(
                id,
                "The door couldn't read this site's records just now. Nothing was changed; try again in a moment.",
            );
        }
    };
    let spec = match tools(records, &ctx.site).await {
        Ok(list) => list.into_iter().find(|t| t.name == name),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "door tools failed");
            return tool_error(
                id,
                "The door couldn't read this site's records just now. Nothing was changed; try again in a moment.",
            );
        }
    };
    let Some(spec) = spec else {
        return rpc_error(
            id,
            -32602,
            &format!("{name} isn't a tool at this door. Call tools/list for this site's tools."),
        );
    };
    let mut who = caller(headers);
    if spec.tier == Tier::Known {
        let checked = match crate::proof::agent_proof() {
            Some(proof) => {
                proof
                    .prove(crate::proof::ProofRequest {
                        door: &ctx.host.origin,
                        method: request.method,
                        uri: request.uri,
                        headers,
                        body: request.body,
                    })
                    .await
            }
            None => Err(crate::proof::ProofFailure {
                code: "agent_identity_unavailable".into(),
                message: "This door can't check an agent's proof just now, so nothing that needs a known agent can be done.".into(),
            }),
        };
        match checked {
            Ok(Some(proven)) => who.proven = Some(proven),
            Ok(None) => {
                let reason = match &who.named {
                    Some(n) => format!(
                        "{} needs a proven agent. \"{n}\" is only a name you gave; it proves nothing, so nothing was created.",
                        spec.name
                    ),
                    None => format!(
                        "{} needs a proven agent, and this request carries no proof. Nothing was created.",
                        spec.name
                    ),
                };
                return refuse_unproven(
                    ctx,
                    id,
                    &spec.name,
                    &who,
                    "agent_identity_required",
                    &reason,
                    request.body,
                )
                .await;
            }
            Err(failure) => {
                let reason = format!(
                    "{} needs a proven agent, and this request's proof didn't check: {} Nothing was created.",
                    spec.name, failure.message
                );
                return refuse_unproven(
                    ctx,
                    id,
                    &spec.name,
                    &who,
                    &failure.code,
                    &reason,
                    request.body,
                )
                .await;
            }
        }
    }
    let now = time::now();
    let result = match name {
        "catalog.search" => catalog_search(records, ctx, args, now).await,
        "delivery.slots" => delivery_slots(records, ctx, args, now).await,
        "quote.create" => quote_create(records, ctx, args, now).await,
        "checkout.create" => {
            let key = format!(
                "{}:checkout:{}:{}",
                ctx.site.site,
                who.label(),
                client_ip(headers)
            );
            if allow(&key, 30, now) {
                checkout_create(records, ctx, args, now).await
            } else {
                refuse("This agent has made 30 checkout links this hour. Try again in an hour.")
            }
        }
        "request_custom_order" => {
            let key = format!(
                "{}:custom:{}:{}",
                ctx.site.site,
                who.label(),
                client_ip(headers)
            );
            if allow(&key, 5, now) {
                request_custom_order(app, records, ctx, &who, args, meta, now).await
            } else {
                refuse(format!(
                    "This agent has sent {} five requests this hour. Try again in an hour.",
                    owner_of(&ctx.site)
                ))
            }
        }
        "order.status" => order_status(records, ctx, args).await,
        "ask_site" => ask_site(app, ctx, args).await,
        "stories.search" => stories_search(records, ctx, args).await,
        _ => refuse(format!("{name} isn't a tool at this door.")),
    };
    match result {
        Ok((mut text, mut structured)) => {
            // What a proven agent leaves behind names it and who vouched for it.
            if let Some(p) = &who.proven {
                structured["agent"] =
                    json!({"id": p.id, "vouched_by": p.vouched_by, "method": p.method});
                text.push_str(&format!(
                    "\nFor {} (verified by {}, vouched for by {}).",
                    p.id, p.method, p.vouched_by
                ));
            }
            rpc_result(
                id,
                json!({"isError": false, "content": [{"type": "text", "text": text}], "structuredContent": structured}),
            )
        }
        Err(Refusal::Words(w)) => tool_error(id, &w),
        Err(Refusal::Broken(e)) => {
            tracing::error!(error = %format!("{e:#}"), tool = name, "door tool failed");
            tool_error(
                id,
                "The door couldn't finish that just now. Nothing was changed; try again in a moment.",
            )
        }
    }
}

fn tool_error(id: &Value, message: &str) -> Value {
    rpc_result(
        id,
        json!({"isError": true, "content": [{"type": "text", "text": message}]}),
    )
}

fn text_arg(args: &Value, key: &str, max: usize) -> Result<Option<String>, Refusal> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.chars().count() > max {
                return refuse(format!("{key} is longer than {max} characters."));
            }
            Ok(Some(s.to_owned()).filter(|s| !s.is_empty()))
        }
        Some(_) => refuse(format!("{key} must be text.")),
    }
}

fn iso(ts: i64, site: &Site) -> String {
    time::local(ts, site.utc_offset_min).to_rfc3339()
}

const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "for", "with", "to", "in", "on", "any", "some", "me",
    "my", "do", "you", "have",
];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1 && !STOP.contains(w))
        .map(|w| {
            w.strip_suffix("es")
                .filter(|r| r.len() > 3)
                .or_else(|| w.strip_suffix('s'))
                .unwrap_or(w)
                .to_owned()
        })
        .collect()
}

/// How well `terms` match `text`, with a weight.
fn hits(terms: &[String], text: &str, weight: f64) -> f64 {
    let ws = words(text);
    terms
        .iter()
        .filter(|t| {
            ws.iter()
                .any(|w| w == *t || (t.len() >= 4 && w.starts_with(t.as_str())))
        })
        .count() as f64
        * weight
}

// ---- Catalogue -------------------------------------------------------------------------------

fn page_for(site: &Site, section: &str, slug: &str) -> String {
    match site.archetype.as_str() {
        "bakery" if section == "bread" => format!("/bread#item-{slug}"),
        "bakery" => format!("/shop/{slug}"),
        "shop" => format!("/fragrances/{slug}"),
        _ => "/".into(),
    }
}

async fn catalog_search(records: &Records, ctx: &Ctx, args: &Value, now: i64) -> Out {
    let site = &ctx.site;
    let query = text_arg(args, "query", 200)?.unwrap_or_default();
    let section = text_arg(args, "section", 40)?;
    let max_minor = match args.get("max_price") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_f64() {
            Some(n) if n >= 0.0 => Some((n * 100.0).round() as i64),
            _ => {
                return refuse(
                    "max_price must be a number, in whole units of the site's currency.",
                );
            }
        },
    };
    let stock = records.stock(site, now).await?;
    let terms = words(&query);
    let mut scored: Vec<(f64, agentrix_records::catalogue::Product)> = records
        .products(&site.site, section.as_deref())
        .await?
        .into_iter()
        .filter(|p| max_minor.is_none_or(|m| p.price_minor <= m))
        .map(|p| {
            let attrs = p.attrs();
            let ingredients = attrs
                .get("ingredients")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let score = if terms.is_empty() {
                1.0
            } else {
                hits(&terms, &p.name, 3.0)
                    + hits(
                        &terms,
                        &format!(
                            "{} {} {} {}",
                            p.section,
                            p.sku,
                            p.family.clone().unwrap_or_default(),
                            p.size.clone().unwrap_or_default()
                        ),
                        2.0,
                    )
                    + hits(&terms, &format!("{} {ingredients}", p.description), 1.0)
            };
            (score, p)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.sort.cmp(&b.1.sort))
    });
    let off = site.utc_offset_min;
    let read_at = crate::shell::asof(site, now);
    let items: Vec<Value> = scored
        .iter()
        .take(20)
        .map(|(_, p)| {
            let s = stock.get(&p.sku);
            let words = match site.archetype.as_str() {
                "bakery" => crate::bakery::stock_words(s, off).0,
                _ => match s.and_then(|s| s.left) {
                    Some(0) => "Out of stock".into(),
                    Some(n) => format!("{n} in stock"),
                    None => "Ask about stock".into(),
                },
            };
            json!({
                "sku": p.sku,
                "name": p.name,
                "section": p.section,
                "description": p.description,
                "size": p.size,
                "family": p.family,
                "price_minor": p.price_minor,
                "currency": site.currency,
                "price": money::fmt(&site.currency, p.price_minor),
                "stock": {
                    "left": s.and_then(|s| s.left),
                    "state": s.map(|s| s.state),
                    "words": words,
                    "as_of": iso(now, site),
                    "counted_at": s.map(|s| iso(s.as_of, site)),
                },
                "url": format!("{}{}", ctx.host.origin, page_for(site, &p.section, &p.slug)),
            })
        })
        .collect();
    let text = if items.is_empty() {
        if query.is_empty() {
            format!(
                "{} lists nothing{} right now.",
                site.name,
                section.map(|s| format!(" in {s}")).unwrap_or_default()
            )
        } else {
            format!(
                "Nothing at {} matches \u{201c}{query}\u{201d}. Try one word, such as a product name, or send an empty query to list everything.",
                site.name
            )
        }
    } else {
        let mut t = format!(
            "{} {} at {}, stock {read_at}:",
            items.len(),
            if items.len() == 1 { "item" } else { "items" },
            site.name
        );
        for i in &items {
            t.push_str(&format!(
                "\n- {} ({}): {}, {}. {}",
                i["name"].as_str().unwrap_or(""),
                i["sku"].as_str().unwrap_or(""),
                i["price"].as_str().unwrap_or(""),
                i["stock"]["words"].as_str().unwrap_or(""),
                i["url"].as_str().unwrap_or("")
            ));
        }
        t
    };
    Ok((
        text,
        json!({"query": query, "count": items.len(), "items": items, "as_of": iso(now, site)}),
    ))
}

// ---- Slots -----------------------------------------------------------------------------------

fn zone_json(site: &Site, z: &Zone) -> Value {
    json!({"zone": z.zone, "label": z.label, "areas": z.areas, "fee_minor": z.fee_minor, "fee": money::fmt(&site.currency, z.fee_minor), "note": z.note})
}

async fn delivery_slots(records: &Records, ctx: &Ctx, args: &Value, now: i64) -> Out {
    let site = &ctx.site;
    let off = site.utc_offset_min;
    let kind = text_arg(args, "kind", 20)?.unwrap_or_else(|| "delivery".into());
    if kind != "delivery" && kind != "collection" {
        return refuse("kind is delivery or collection.");
    }
    let start = match text_arg(args, "from_day", 10)? {
        Some(d) => {
            let Ok(day) = chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d") else {
                return refuse("from_day must be YYYY-MM-DD.");
            };
            time::at_local(day, "00:00", off)?
        }
        None => time::day_start(now, off),
    };
    let days = args
        .get("days")
        .and_then(Value::as_i64)
        .unwrap_or(7)
        .clamp(1, 21);
    let to = time::day_start(start, off) + days * 86_400;
    let slots = records.slots(site, &kind, start.max(now), to).await?;
    let (open, full): (Vec<_>, Vec<_>) = slots
        .into_iter()
        .filter(|s| s.starts_at > now)
        .partition(|s| !s.full);
    let open_json: Vec<Value> = open
        .iter()
        .map(|s| json!({"slot_id": s.slot_id, "label": s.label, "starts_at": iso(s.starts_at, site), "ends_at": iso(s.ends_at, site), "taken": s.taken, "capacity": s.capacity, "left": s.capacity - s.taken}))
        .collect();
    let zones = if kind == "delivery" {
        records.zones(&site.site).await?
    } else {
        Vec::new()
    };
    let range = format!(
        "{} to {}",
        time::fmt_day(start, off),
        time::fmt_day(to - 1, off)
    );
    let mut text = if open.is_empty() {
        format!(
            "No open {kind} slots from {range}; {} {} full. Try later days with from_day.",
            full.len(),
            if full.len() == 1 { "is" } else { "are" }
        )
    } else {
        let mut t = format!("Open {kind} slots, {range}:");
        for s in &open {
            t.push_str(&format!(
                "\n- {} ({} of {} taken), slot_id {}",
                s.label, s.taken, s.capacity, s.slot_id
            ));
        }
        if !full.is_empty() {
            t.push_str(&format!(
                "\n{} more {} full.",
                full.len(),
                if full.len() == 1 { "is" } else { "are" }
            ));
        }
        t
    };
    for z in &zones {
        text.push_str(&format!(
            "\nDelivery to {} ({}): {} per delivery. {}",
            z.label,
            z.areas.join(", "),
            money::fmt(&site.currency, z.fee_minor),
            z.note
        ));
    }
    if kind == "collection" {
        text.push_str(&format!("\nCollect from {}.", site.fact("/address")));
    }
    Ok((
        text,
        json!({
            "kind": kind,
            "from": iso(start, site),
            "to": iso(to, site),
            "slots": open_json,
            "full": full.iter().map(|s| json!({"slot_id": s.slot_id, "label": s.label})).collect::<Vec<_>>(),
            "zones": zones.iter().map(|z| zone_json(site, z)).collect::<Vec<_>>(),
            "collect_from": if kind == "collection" { Value::String(site.fact("/address")) } else { Value::Null },
        }),
    ))
}

// ---- Quotes and checkout ---------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineIn {
    sku: String,
    qty: i64,
}

async fn lines_arg(records: &Records, site: &Site, args: &Value) -> Result<Vec<CartLine>, Refusal> {
    let Some(raw) = args.get("lines") else {
        return refuse("lines is required: a list of {sku, qty} from catalog.search.");
    };
    let Ok(lines) = serde_json::from_value::<Vec<LineIn>>(raw.clone()) else {
        return refuse("lines must be a list of {\"sku\": text, \"qty\": whole number}.");
    };
    if lines.is_empty() || lines.len() > 20 {
        return refuse("Send between 1 and 20 lines.");
    }
    let mut out: Vec<CartLine> = Vec::new();
    for l in lines {
        if !(1..=20).contains(&l.qty) {
            return refuse(format!("qty for {} must be between 1 and 20.", l.sku));
        }
        if records
            .product(&site.site, &l.sku)
            .await?
            .is_none_or(|p| p.status != "published")
        {
            return refuse(format!(
                "{} isn't on {}'s catalogue. Use a sku from catalog.search.",
                l.sku, site.name
            ));
        }
        match out.iter_mut().find(|x| x.sku == l.sku) {
            Some(x) => x.qty = (x.qty + l.qty).min(20),
            None => out.push(CartLine {
                sku: l.sku,
                qty: l.qty,
                card: None,
            }),
        }
    }
    Ok(out)
}

/// A zone by id or by an area it covers.
async fn zone_arg(records: &Records, site: &Site, args: &Value) -> Result<Option<Zone>, Refusal> {
    let Some(z) = text_arg(args, "zone", 80)? else {
        return Ok(None);
    };
    let zones = records.zones(&site.site).await?;
    if let Some(found) = zones.iter().find(|x| x.zone.eq_ignore_ascii_case(&z)) {
        return Ok(Some(found.clone()));
    }
    match records.zone_for(&site.site, &z).await? {
        Some(found) => Ok(Some(found)),
        None => {
            let areas: Vec<String> = zones.iter().flat_map(|x| x.areas.clone()).collect();
            refuse(format!(
                "{} doesn't deliver to {z}. It delivers to {}.",
                site.name,
                areas.join(", ")
            ))
        }
    }
}

async fn slot_arg(
    records: &Records,
    site: &Site,
    args: &Value,
    kind: Option<&str>,
    now: i64,
) -> Result<Option<agentrix_records::orders::Slot>, Refusal> {
    let Some(id) = text_arg(args, "slot_id", 60)? else {
        return Ok(None);
    };
    let Some(slot) = records.slot(site, &id).await? else {
        return refuse(format!(
            "There's no slot {id}. Use a slot_id from delivery.slots."
        ));
    };
    if let Some(k) = kind
        && slot.kind != k
    {
        return refuse(format!("{} is a {} slot, not {k}.", slot.label, slot.kind));
    }
    if slot.starts_at <= now {
        return refuse(format!("{} has passed. Pick a later slot.", slot.label));
    }
    if slot.full {
        return refuse(format!(
            "{} is full. Pick another time from delivery.slots.",
            slot.label
        ));
    }
    Ok(Some(slot))
}

fn priced_json(site: &Site, view: &CartView) -> Value {
    let cur = &site.currency;
    json!({
        "lines": view.lines.iter().map(|l| json!({
            "sku": l.sku, "name": l.name, "qty": l.qty,
            "unit_minor": l.unit_minor, "unit": money::fmt(cur, l.unit_minor),
            "amount_minor": l.amount_minor, "amount": money::fmt(cur, l.amount_minor),
            "left": l.left, "available": l.available
        })).collect::<Vec<_>>(),
        "subtotal_minor": view.subtotal_minor, "subtotal": money::fmt(cur, view.subtotal_minor),
        "delivery_minor": view.delivery_minor, "delivery": money::fmt(cur, view.delivery_minor),
        "total_minor": view.total_minor, "total": money::fmt(cur, view.total_minor), "currency": cur,
        "usd_cents": view.usd_cents, "usd": money::usd(view.usd_cents),
        "rate": if view.usd_rate_minor > 0 { Value::String(money::rate_line(view.usd_rate_minor)) } else { Value::Null },
        "price_list": view.price_list,
        "valid_until": view.valid_until.map(|t| iso(t, site)),
        "valid_until_text": view.valid_until.map(|t| time::fmt_day_clock(t, site.utc_offset_min)),
        "problems": view.problems,
    })
}

fn priced_text(site: &Site, view: &CartView, zone: Option<&Zone>) -> String {
    let cur = &site.currency;
    let mut parts: Vec<String> = view
        .lines
        .iter()
        .map(|l| {
            format!(
                "{} \u{d7} {} {}",
                l.name,
                l.qty,
                money::fmt(cur, l.amount_minor)
            )
        })
        .collect();
    if let Some(z) = zone {
        parts.push(format!(
            "delivery to {} {}",
            z.label,
            money::fmt(cur, view.delivery_minor)
        ));
    }
    let mut t = parts.join(" \u{b7} ");
    t.push_str(&format!("\nTotal {}", money::fmt(cur, view.total_minor)));
    if view.usd_rate_minor > 0 {
        t.push_str(&format!(
            " = {} at {}",
            money::usd(view.usd_cents),
            money::rate_line(view.usd_rate_minor)
        ));
    }
    if let Some(v) = &view.price_list {
        t.push_str(&format!(" \u{b7} price list {v}"));
    }
    if let Some(u) = view.valid_until {
        t.push_str(&format!(
            " \u{b7} valid until {}",
            time::fmt_day_clock(u, site.utc_offset_min)
        ));
    }
    for p in &view.problems {
        t.push_str(&format!("\n{p}"));
    }
    t
}

async fn quote_create(records: &Records, ctx: &Ctx, args: &Value, now: i64) -> Out {
    let site = &ctx.site;
    let zone = zone_arg(records, site, args).await?;
    if let Some(cake) = args.get("cake") {
        if args.get("lines").is_some() {
            return refuse("Send lines or a cake, not both.");
        }
        return quote_cake(records, ctx, cake, zone.as_ref(), now).await;
    }
    let lines = lines_arg(records, site, args).await?;
    let slot = slot_arg(records, site, args, None, now).await?;
    let cart = Cart {
        cart_id: "quote".into(),
        customer_id: None,
        lines,
        slot_id: slot.as_ref().map(|s| s.slot_id.clone()),
        zone: zone.as_ref().map(|z| z.zone.clone()),
    };
    let view = records.price_cart(site, &cart, now).await?;
    let mut text = priced_text(site, &view, zone.as_ref());
    if let Some(s) = &slot {
        text.push_str(&format!("\nSlot: {}", s.label));
    }
    text.push_str("\nNothing is reserved or charged. checkout.create with the same lines makes a link for a person to pay.");
    let mut out = priced_json(site, &view);
    out["kind"] = json!("lines");
    out["slot"] = slot.map_or(
        Value::Null,
        |s| json!({"slot_id": s.slot_id, "label": s.label}),
    );
    out["charged"] = json!(false);
    Ok((text, out))
}

fn cake_arg(cake: &Value) -> Result<CakeChoice, Refusal> {
    let allowed = [
        "size", "sponge", "colour", "finish", "deco", "message", "date", "note",
    ];
    if let Some(extra) = cake
        .as_object()
        .and_then(|o| o.keys().find(|k| !allowed.contains(&k.as_str())))
    {
        return refuse(format!(
            "cake has an unknown field: {extra}. Fields: {}.",
            allowed.join(", ")
        ));
    }
    serde_json::from_value::<CakeChoice>(cake.clone())
        .map_err(|_| Refusal::Words("cake must be an object of rule codes (size, sponge, colour, finish, deco), a date (YYYY-MM-DD) and an optional message. The codes are in this tool's input schema.".into()))
}

async fn quote_cake(
    records: &Records,
    ctx: &Ctx,
    cake: &Value,
    zone: Option<&Zone>,
    now: i64,
) -> Out {
    let site = &ctx.site;
    if site.archetype != "bakery" {
        return refuse(format!(
            "{} doesn't make custom cakes. Send lines instead.",
            site.name
        ));
    }
    let choice = cake_arg(cake)?;
    let q = records.quote_cake(site, &choice, now).await?;
    let cur = &site.currency;
    let delivery = zone.map_or(0, |z| z.fee_minor);
    let total = q.total_minor + delivery;
    let usd = money::usd_cents(total, q.usd_rate_minor);
    let mut parts: Vec<String> = q
        .lines
        .iter()
        .map(|l| format!("{} {}", l.label, money::fmt(cur, l.amount_minor)))
        .collect();
    if let Some(z) = zone {
        parts.push(format!(
            "delivery to {} {}",
            z.label,
            money::fmt(cur, z.fee_minor)
        ));
    }
    let mut text = format!(
        "{}\nTotal {} = {} at {} \u{b7} price list {} \u{b7} valid until {}",
        parts.join(" \u{b7} "),
        money::fmt(cur, total),
        money::usd(usd),
        money::rate_line(q.usd_rate_minor),
        q.price_list,
        time::fmt_day_clock(q.valid_until, site.utc_offset_min)
    );
    if let Some(s) = &q.serves {
        text.push_str(&format!("\n{s}."));
    }
    for f in &q.flags {
        text.push_str(&format!("\n{}", f.text));
    }
    if q.complete {
        text.push_str(&format!("\nComplete. request_custom_order with the same cake holds it for {}. Nothing is charged here.", owner_of(site)));
    } else if !q.flags.iter().any(|f| f.level == "error") {
        text.push_str("\nNot complete yet: every choice and a date are needed.");
    }
    Ok((
        text,
        json!({
            "kind": "cake",
            "lines": q.lines.iter().map(|l| json!({"code": l.code, "label": l.label, "amount_minor": l.amount_minor, "amount": money::fmt(cur, l.amount_minor)})).collect::<Vec<_>>(),
            "delivery_minor": delivery, "delivery": money::fmt(cur, delivery),
            "total_minor": total, "total": money::fmt(cur, total), "currency": cur,
            "usd_cents": usd, "usd": money::usd(usd), "rate": money::rate_line(q.usd_rate_minor),
            "price_list": q.price_list,
            "valid_until": iso(q.valid_until, site),
            "valid_until_text": time::fmt_day_clock(q.valid_until, site.utc_offset_min),
            "complete": q.complete,
            "flags": q.flags,
            "serves": q.serves,
            "earliest_day": q.earliest_day,
            "charged": false,
        }),
    ))
}

fn fulfilment_arg(args: &Value) -> Result<String, Refusal> {
    match text_arg(args, "fulfilment", 20)?.as_deref() {
        Some(f @ ("delivery" | "collection")) => Ok(f.to_owned()),
        _ => refuse("fulfilment is delivery or collection."),
    }
}

async fn checkout_create(records: &Records, ctx: &Ctx, args: &Value, now: i64) -> Out {
    let site = &ctx.site;
    let lines = lines_arg(records, site, args).await?;
    let fulfilment = fulfilment_arg(args)?;
    let zone = if fulfilment == "delivery" {
        match zone_arg(records, site, args).await? {
            Some(z) => Some(z),
            None => {
                return refuse(
                    "Delivery needs a zone: an area name or zone id from delivery.slots.",
                );
            }
        }
    } else {
        None
    };
    let slot = slot_arg(records, site, args, Some(&fulfilment), now).await?;
    let cart = Cart {
        cart_id: agentrix_records::new_id("cart"),
        customer_id: None,
        lines,
        slot_id: slot.as_ref().map(|s| s.slot_id.clone()),
        zone: zone.as_ref().map(|z| z.zone.clone()),
    };
    let view = records.price_cart(site, &cart, now).await?;
    if !view.problems.is_empty() {
        return refuse(format!(
            "{} Change the lines or the slot and try again.",
            view.problems.join(" ")
        ));
    }
    records.save_cart(&site.site, &cart).await?;
    let link = format!("{}/bag/claim?cart={}", ctx.host.origin, cart.cart_id);
    let words = format!(
        "A person opens this link on {}'s own site, checks the bag, signs in and pays there. Nothing is charged until they pay. The door never takes payment or a card.",
        site.name
    );
    let mut text = format!(
        "Bag ready for a person to review and pay: {link}\n{}",
        priced_text(site, &view, zone.as_ref())
    );
    if let Some(s) = &slot {
        text.push_str(&format!(
            "\n{}: {}",
            if fulfilment == "delivery" {
                "Delivery"
            } else {
                "Collection"
            },
            s.label
        ));
    }
    text.push_str(&format!("\n{words}"));
    let mut out = priced_json(site, &view);
    out["cart_id"] = json!(cart.cart_id);
    out["link"] = json!(link);
    out["fulfilment"] = json!(fulfilment);
    out["slot"] = slot.map_or(
        Value::Null,
        |s| json!({"slot_id": s.slot_id, "label": s.label}),
    );
    out["charged"] = json!(false);
    out["words"] = json!(words);
    Ok((text, out))
}

// ---- Custom orders and held calls --------------------------------------------------------------

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !e.contains(char::is_whitespace)
}

async fn request_custom_order(
    app: &App,
    records: &Records,
    ctx: &Ctx,
    who: &Caller,
    args: &Value,
    meta: &Value,
    now: i64,
) -> Out {
    let site = &ctx.site;
    let owner = owner_of(site);
    if site.archetype != "bakery" {
        return refuse(format!(
            "{} doesn't take custom orders at the door.",
            site.name
        ));
    }
    let Some(cake) = args.get("cake") else {
        return refuse("cake is required. quote.create prices it first.");
    };
    let choice = cake_arg(cake)?;
    let Some(name) = text_arg(args, "name", 120)? else {
        return refuse("name is required: the name on the order.");
    };
    let email = text_arg(args, "email", 254)?;
    let phone = text_arg(args, "phone", 20)?;
    if email.is_none() && phone.is_none() {
        return refuse(format!(
            "Add an email or a phone number so {owner} can reach the customer."
        ));
    }
    if let Some(e) = &email
        && !valid_email(e)
    {
        return refuse("email isn't an email address.");
    }
    if let Some(p) = &phone
        && !p.chars().all(|c| c.is_ascii_digit() || " +-()".contains(c))
    {
        return refuse("phone has characters a phone number doesn't.");
    }
    let fulfilment = fulfilment_arg(args)?;
    let zone = if fulfilment == "delivery" {
        match zone_arg(records, site, args).await? {
            Some(z) => Some(z),
            None => {
                return refuse(
                    "Delivery needs a zone: an area name or zone id from delivery.slots.",
                );
            }
        }
    } else {
        None
    };
    let slot = slot_arg(records, site, args, Some(&fulfilment), now).await?;
    let note = text_arg(args, "note", 300)?;
    let quote = records.quote_cake(site, &choice, now).await?;
    let key = meta
        .get(IDEMPOTENCY_KEY)
        .and_then(Value::as_str)
        .map(|k| k.chars().take(120).collect::<String>())
        .unwrap_or_else(|| {
            let mut h = Sha256::new();
            h.update(site.site.as_bytes());
            h.update(who.label().as_bytes());
            h.update(args.to_string().as_bytes());
            format!("door:{}", hex::encode(&h.finalize()[..16]))
        });
    let contact = Contact {
        name,
        email,
        phone,
        recipient: None,
    };
    let how = Placement {
        customer_id: None,
        channel: "agent door".into(),
        agent: Some(match &who.proven {
            Some(p) => format!(
                "{} \u{b7} verified by {}, vouched for by {}",
                p.id, p.method, p.vouched_by
            ),
            None => format!("{} \u{b7} {}", who.label(), who.tier().word()),
        }),
        proof: who
            .proven
            .as_ref()
            .map(|p| agentrix_records::orders::AgentProof {
                id: p.id.clone(),
                vouched_by: p.vouched_by.clone(),
                method: p.method.clone(),
            }),
        fulfilment,
        slot_id: slot.map(|s| s.slot_id),
        zone: zone.map(|z| z.zone),
        note,
        idempotency_key: Some(key),
    };
    let view = match records
        .request_custom(site, &choice, &quote, &contact, &how, now)
        .await
    {
        Ok(v) => v,
        Err(OrderError::Refused(w)) => return refuse(w),
        Err(OrderError::Other(e)) => return Err(Refusal::Broken(e)),
    };
    app.tell_held(site, &ctx.host.host, &ctx.host.origin, &view)
        .await;
    let o = &view.order;
    let status_url = format!("{}/awp/v2/calls/{}", ctx.host.origin, o.order_id);
    let words =
        format!("Held for {owner}. Nothing is charged until she approves and the customer pays.");
    let cur = &site.currency;
    let text = format!(
        "{words}\nOrder {}: {} = {} from price list {}. Check {status_url} for her decision.",
        o.order_id,
        money::fmt(cur, o.total_minor),
        money::usd(o.usd_cents()),
        o.price_list.clone().unwrap_or_default(),
    );
    Ok((
        text,
        json!({
            "order_id": o.order_id,
            "state": "held",
            "status_url": status_url,
            "held_call": held_call_json(site, ctx, o),
            "total_minor": o.total_minor, "total": money::fmt(cur, o.total_minor), "currency": cur,
            "usd_cents": o.usd_cents(), "usd": money::usd(o.usd_cents()),
            "price_list": o.price_list,
            "valid_until": o.quote_valid_until.map(|t| iso(t, site)),
            "charged": false,
            "words": words,
        }),
    ))
}

/// A held call as AWP (`HeldCall`) and A2A (task state) describe it, from the order.
fn held_call_json(site: &Site, ctx: &Ctx, o: &agentrix_records::orders::Order) -> Value {
    let owner = owner_of(site);
    let (state, decision, task, words) = match o.state.as_str() {
        "held" => (
            "pending",
            "waiting",
            "working",
            format!(
                "Held for {owner}. Nothing is charged until she approves and the customer pays."
            ),
        ),
        "approved" => (
            "approved",
            "approved",
            "completed",
            format!(
                "{owner} approved it. The customer pays on {}'s order page; nothing is charged until they do.",
                site.name
            ),
        ),
        "declined" => (
            "rejected",
            "declined",
            "rejected",
            format!("{owner} declined it. Nothing was charged."),
        ),
        "lapsed" => (
            "expired",
            "lapsed",
            "canceled",
            format!("It lapsed before {owner} decided. Nothing was charged."),
        ),
        "cancelled" => (
            "rejected",
            "cancelled",
            "canceled",
            "It was cancelled. Nothing is owed.".to_owned(),
        ),
        other => (
            "completed",
            "approved",
            "completed",
            format!("{owner} approved it. The order is {}.", state_words(other)),
        ),
    };
    let status = if decision == "waiting" {
        "working: waiting for the owner".to_owned()
    } else {
        decision.to_owned()
    };
    json!({
        "id": o.order_id,
        "capability": "request_custom_order",
        "state": state,
        "status": format!("{}/awp/v2/calls/{}", ctx.host.origin, o.order_id),
        "decision": decision,
        "statusWords": status,
        "task": {"state": task, "message": status},
        "words": words,
        "createdAt": iso(o.created_at, site),
        "updatedAt": iso(o.updated_at, site),
    })
}

async fn held_call(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
) -> Page {
    let records = app.records().await?;
    let found = if id.len() <= 20 {
        records.order(&ctx.site, &id).await?
    } else {
        None
    };
    match found.filter(|v| v.order.channel == "agent door" && v.order.kind == "custom") {
        Some(v) => Ok(json_response("application/json", "no-store", &held_call_json(&ctx.site, &ctx, &v.order))),
        None => Ok((
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")],
            json!({"error": "There's no held call with that id on this site. Held calls are requests made through the agent door."}).to_string(),
        )
            .into_response()),
    }
}

async fn order_status(records: &Records, ctx: &Ctx, args: &Value) -> Out {
    let site = &ctx.site;
    let prefix = order_prefix(site);
    let Some(raw) = text_arg(args, "order_id", 20)? else {
        return refuse(format!("order_id is required, such as {prefix}-1042."));
    };
    let id = raw.to_uppercase();
    let shaped = id.split_once('-').is_some_and(|(p, n)| {
        !p.is_empty()
            && p.chars().all(|c| c.is_ascii_alphabetic())
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
    });
    let found = if shaped {
        records.order(site, &id).await?
    } else {
        None
    };
    let Some(v) = found else {
        return refuse(format!(
            "There's no order {id} at {}. Order numbers look like {prefix}-1042.",
            site.name
        ));
    };
    let words = state_words(&v.order.state);
    // What PayPal itself confirmed about the order's money, from its verified webhooks.
    let paypal: Vec<Value> = records
        .paypal_confirmations(&site.site, &v.order.order_id)
        .await?
        .into_iter()
        .map(|c| json!({"kind": c.kind, "id": c.object_id, "status": c.status, "at": iso(c.confirmed_at, site)}))
        .collect();
    let mut text = format!("Order {} is {words}.", v.order.order_id);
    for c in &paypal {
        text.push_str(&format!(
            " PayPal confirmed the {} {} ({}).",
            c["kind"].as_str().unwrap_or(""),
            c["id"].as_str().unwrap_or(""),
            c["status"].as_str().unwrap_or("")
        ));
    }
    Ok((
        text,
        json!({"order_id": v.order.order_id, "state": v.order.state, "status": words, "updated_at": iso(v.order.updated_at, site), "confirmed_by_paypal": paypal}),
    ))
}

// ---- Ask and stories -------------------------------------------------------------------------

fn desk_name(site: &Site, id: &str) -> String {
    desks(site, &Default::default())
        .into_iter()
        .find(|d| d.id == id)
        .map_or_else(|| front(site).name, |d| d.name)
}

async fn ask_site(app: &App, ctx: &Ctx, args: &Value) -> Out {
    let Some(q) = text_arg(args, "question", 400)? else {
        return refuse("question is required.");
    };
    let a = crate::ask::answer_for(app, ctx, &q, "", &[]).await?;
    let answer = a
        .phrases
        .iter()
        .map(|p| p.replace("**", ""))
        .collect::<Vec<_>>()
        .join(" ");
    let desk = desk_name(&ctx.site, &a.desk);
    let sources: Vec<Value> = a
        .sources
        .iter()
        // A taught answer points at no page.
        .map(|s| json!({"n": s.n, "label": s.label, "url": (!s.href.is_empty()).then(|| format!("{}{}", ctx.host.origin, s.href)), "subscriber": s.lock}))
        .collect();
    let mut text = answer.clone();
    if !sources.is_empty() {
        text.push_str("\n\nSources:");
        for s in &sources {
            text.push_str(&format!(
                "\n[{}] {}",
                s["n"],
                s["label"].as_str().unwrap_or("")
            ));
            if let Some(url) = s["url"].as_str() {
                text.push_str(&format!(": {url}"));
            }
        }
    }
    let passes = a.actions.iter().any(|x| x.kind == "pass");
    let owner = owner_of(&ctx.site);
    if passes {
        text.push_str(&format!(
            "\n\nA person can ask {owner} through Ask on {}/.",
            ctx.host.origin
        ));
    }
    // Say AI only when a model wrote the words (Ask's basis).
    let by = match a.basis {
        crate::ask::Basis::Ai => format!("Written by the {desk} desk, an AI WebAgent"),
        crate::ask::Basis::Instant => {
            format!("Answered by the {desk} desk from the site's records, without a model")
        }
        crate::ask::Basis::Rules => {
            format!("Answered by the {desk} desk from the site's own rules, without a model")
        }
        crate::ask::Basis::Owner => {
            format!("Answered by the {desk} desk with {owner}'s own answer, without a model")
        }
    };
    text.push_str(&format!("\n\n{by}. {}.", a.meta.trim_end_matches('.')));
    Ok((
        text,
        json!({"answer": answer, "desk": desk, "ai": a.ai(), "basis": a.basis, "sources": sources, "meta": a.meta, "site_does_not_say": passes}),
    ))
}

async fn stories_search(records: &Records, ctx: &Ctx, args: &Value) -> Out {
    let site = &ctx.site;
    let Some(q) = text_arg(args, "query", 200)? else {
        return refuse("query is required.");
    };
    let terms = words(&q);
    let authors: HashMap<String, String> = records
        .authors(&site.site)
        .await?
        .into_iter()
        .map(|a| (a.author_id, a.name))
        .collect();
    let mut scored: Vec<(f64, agentrix_records::news::Story)> = records
        .stories(&site.site, None, 500)
        .await?
        .into_iter()
        .map(|s| {
            let open = if s.locked() {
                String::new()
            } else {
                s.paragraphs().join(" ")
            };
            let score = hits(&terms, &s.title, 3.0)
                + hits(
                    &terms,
                    &format!(
                        "{} {} {}",
                        s.kicker.clone().unwrap_or_default(),
                        s.section,
                        s.topic.clone().unwrap_or_default()
                    ),
                    2.0,
                )
                + hits(
                    &terms,
                    &format!(
                        "{} {open}",
                        if s.locked() {
                            ""
                        } else {
                            s.standfirst.as_str()
                        }
                    ),
                    1.0,
                );
            (score, s)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.1.published_at.cmp(&a.1.published_at))
    });
    let off = site.utc_offset_min;
    let items: Vec<Value> = scored
        .iter()
        .take(10)
        .map(|(_, s)| {
            let url = format!(
                "{}/{}/{}",
                ctx.host.origin,
                time::local(s.published_at, off).format("%Y/%m"),
                s.slug
            );
            if s.locked() {
                json!({"headline": s.title, "url": url, "subscriber": true})
            } else {
                json!({
                    "headline": s.title,
                    "date": time::fmt_day(s.published_at, off),
                    "published_at": iso(s.published_at, site),
                    "byline": authors.get(&s.author_id).cloned().unwrap_or_default(),
                    "standfirst": s.standfirst,
                    "url": url,
                    "subscriber": false
                })
            }
        })
        .collect();
    let text = if items.is_empty() {
        format!(
            "No stories at {} match \u{201c}{q}\u{201d}. Try one word from a headline.",
            site.name
        )
    } else {
        let mut t = format!(
            "{} {} at {}:",
            items.len(),
            if items.len() == 1 { "story" } else { "stories" },
            site.name
        );
        for i in &items {
            if i["subscriber"].as_bool() == Some(true) {
                t.push_str(&format!(
                    "\n- {} (for subscribers) {}",
                    i["headline"].as_str().unwrap_or(""),
                    i["url"].as_str().unwrap_or("")
                ));
            } else {
                t.push_str(&format!(
                    "\n- {}, {}, by {}. {}",
                    i["headline"].as_str().unwrap_or(""),
                    i["date"].as_str().unwrap_or(""),
                    i["byline"].as_str().unwrap_or(""),
                    i["url"].as_str().unwrap_or("")
                ));
            }
        }
        t
    };
    Ok((
        text,
        json!({"query": q, "count": items.len(), "stories": items}),
    ))
}

// ---- The bag link ------------------------------------------------------------------------------

#[derive(Deserialize)]
struct ClaimQuery {
    #[serde(default)]
    cart: String,
}

/// `GET /bag/claim?cart=<id>`: the link a checkout made at the door opens. It puts that bag in
/// this browser and shows it; the person reviews, signs in and pays on the site.
async fn claim(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<ClaimQuery>,
) -> Page {
    let id = q.cart.trim();
    let shaped = id.starts_with("cart-")
        && id.len() <= 40
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !shaped {
        return Ok(see_other("/bag"));
    }
    let records = app.records().await?;
    if records.cart(&ctx.site.site, id).await?.lines.is_empty() {
        return Ok(see_other("/bag"));
    }
    let mut response = see_other("/bag");
    response.headers_mut().append(
        header::SET_COOKIE,
        web::set_cookie(&app, web::CART, id, web::CART_SECONDS),
    );
    Ok(response)
}

// ---- The door page (M-09) ----------------------------------------------------------------------

#[derive(Serialize)]
struct Endpoint {
    path: String,
    url: String,
    what: String,
}

#[derive(Serialize)]
struct AgentsView {
    shell: Shell,
    door_desk: Desk,
    anyone: Vec<String>,
    known: Vec<String>,
    held: Vec<String>,
    never: Vec<String>,
    known_how: &'static str,
    endpoints: Vec<Endpoint>,
    terms: String,
    sample_terms: &'static str,
    facts: Vec<(String, String)>,
    waiting: String,
    example: String,
    mcp_host: String,
}

async fn agents(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let owner = owner_of(site);
    let list = tools(records, site).await?;
    let o = &ctx.host.origin;
    let shell = app
        .shell(&ctx, "", "For AI agents", &format!("How AI agents can use {}: what each tier may do, what waits for {owner}, and the endpoints.", site.name))
        .await?;
    let door_desk = shell
        .desks
        .iter()
        .find(|d| d.id == "orders")
        .cloned()
        .unwrap_or_else(|| shell.front.clone());
    let mut facts: Vec<(String, String)> = Vec::new();
    let mut waiting = String::new();
    if site.archetype == "news" {
        let n = records.stories(&site.site, None, 500).await?.len();
        facts.push(("Stories searchable".into(), n.to_string()));
    } else {
        let n = records.products(&site.site, None).await?.len();
        facts.push((
            "Products searchable".into(),
            format!("{n}, stock read live"),
        ));
        if let Some(pl) = records.live_price_list(&site.site).await? {
            facts.push((
                "Price list".into(),
                format!(
                    "{}, valid until {}",
                    pl.version,
                    time::fmt_day_clock(pl.valid_until, site.utc_offset_min)
                ),
            ));
            if pl.usd_rate_minor > 0 {
                facts.push((
                    "Paying in dollars".into(),
                    money::rate_line(pl.usd_rate_minor),
                ));
            }
        }
        if list.iter().any(|t| t.held) {
            let held = records
                .orders_for_owner(site, Some("custom"), 200)
                .await?
                .into_iter()
                .filter(|v| v.order.channel == "agent door" && v.order.state == "held")
                .count();
            waiting = match held {
                0 => format!("No agent requests are waiting for {owner} right now."),
                1 => format!("1 request from an agent is waiting for {owner} right now."),
                n => format!("{n} requests from agents are waiting for {owner} right now."),
            };
        }
    }
    facts.push(("As of".into(), time::fmt_clock(now, site.utc_offset_min)));
    let endpoints = vec![
        Endpoint {
            path: "/.well-known/ai-catalog.json".into(),
            url: format!("{o}/.well-known/ai-catalog.json"),
            what: "The AI Catalog. It lists the doors below.".into(),
        },
        Endpoint {
            path: "/.well-known/awp".into(),
            url: format!("{o}/.well-known/awp"),
            what: "What this site offers agents, with tiers and terms.".into(),
        },
        Endpoint {
            path: "/mcp".into(),
            url: format!("{o}/mcp"),
            what: format!(
                "MCP tools: JSON-RPC 2.0 over POST, protocol {}.",
                MCP_VERSIONS.join(" or ")
            ),
        },
        Endpoint {
            path: "/awp/v2/calls/{id}".into(),
            url: format!("{o}/awp/v2/calls/"),
            what: format!("Where a request held for {owner} stands."),
        },
        Endpoint {
            path: "/agents".into(),
            url: format!("{o}/agents"),
            what: "This page.".into(),
        },
    ];
    let mut never = vec![
        "pays, or holds a payment credential".to_owned(),
        "asks for or accepts a password, a code or a card".to_owned(),
    ];
    if site.archetype != "news" {
        never.push("releases a hold or approves anything".into());
        never.push("sets a price or a discount".into());
    }
    never.push("sees or changes a customer's account".into());
    let view = AgentsView {
        door_desk,
        anyone: list
            .iter()
            .filter(|t| t.tier == Tier::Anonymous)
            .map(|t| t.may.clone())
            .collect(),
        known: list
            .iter()
            .filter(|t| t.tier == Tier::Known)
            .map(|t| t.may.clone())
            .collect(),
        held: list.iter().filter(|t| t.held).map(held_noun).collect(),
        never,
        known_how: KNOWN_HOW,
        endpoints,
        terms: terms_text(&terms(site, &list)),
        sample_terms: SAMPLE_TERMS,
        facts,
        waiting,
        example: format!(
            "curl -s {o}/mcp \\\n  -H 'content-type: application/json' \\\n  -d '{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}}'"
        ),
        mcp_host: format!("{}/mcp", ctx.host.host),
        shell,
    };
    Ok(html(app.render("door/agents.html", &view)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_carries_the_name_back() {
        let id = session_id("assistant.example");
        assert_eq!(session_name(&id).as_deref(), Some("assistant.example"));
        assert_eq!(session_name("not-ours"), None);
        assert_eq!(session_name(&session_id("")).as_deref(), Some(""));
    }

    #[test]
    fn words_match_plurals_and_prefixes() {
        let t = words("sourdough loaves");
        assert!(hits(&t, "Country sourdough", 1.0) > 0.0);
        assert_eq!(hits(&words("cake"), "Butter croissant", 1.0), 0.0);
    }
}
