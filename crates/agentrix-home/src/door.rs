//! agentrix.website's own agent door, so the platform is an Agentrix Website the way the
//! samples are: AI agents read what it offers at `/.well-known/awp` and call MCP tools at
//! `/mcp` (JSON-RPC 2.0 over HTTP POST, JSON responses).
//!
//! | Tool | What it does |
//! |---|---|
//! | `list_plans` | The plans and prices from W3's catalogue: hosting, the AI each month, the first-month AI and the 12-month discount, as data |
//! | `list_samples` | The three samples, their walkthroughs, whether each is open, and each one's own agent door |
//! | `read_how_it_works` | The How it works page, section by section |
//! | `ask_desk` | The same [`crate::desks::Desks::answer`] the page uses: cited, under the desks' dials and their rate |
//! | `start_check` | Starts (or reuses) Assess's free check of a public site; the report is read-only |
//!
//! Every tool is open to any agent and none takes personal data: a question with an email
//! address or a phone number in it is refused, and so is an address with one. Nothing here
//! pays, signs anyone up, starts a move or joins the weekly fix: those need a person, and the
//! descriptor says so. A browser on another site can't call `/mcp` (its `Origin` is refused).
//!
//! The descriptor, `tools/list` and the refusals all read [`tools`], so the door can't promise
//! what it doesn't do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use reqwest::Url;
use serde_json::{Value, json};

use crate::Home;

/// The MCP protocol versions the door speaks, newest first (as the samples' door).
pub const MCP_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26"];

/// Starts Assess's free check of a public site.
#[async_trait]
pub trait Checks: Send + Sync {
    /// Starts (or reuses) a check of `url` and gives its id, or words saying why not.
    async fn start(&self, url: &Url) -> Result<String, String>;
}

/// Assess through node's own form (`POST /assess`, field `url`), so an agent's check is the
/// same as one started from the paste box, with Assess's own limits.
pub struct HttpChecks {
    assess: String,
    http: reqwest::Client,
}

impl HttpChecks {
    /// Assess at `{origin}/assess`.
    pub fn new(origin: &str, http: reqwest::Client) -> Self {
        Self {
            assess: format!("{}/assess", origin.trim_end_matches('/')),
            http,
        }
    }
}

#[async_trait]
impl Checks for HttpChecks {
    async fn start(&self, url: &Url) -> Result<String, String> {
        const BUSY: &str = "Assess couldn't start a check just now. Try again in a minute.";
        let res = self
            .http
            .post(&self.assess)
            .form(&[("url", url.as_str())])
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "door: starting a check");
                BUSY.to_owned()
            })?;
        // A redirect to /assess/{id}, followed or not.
        let location = res
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .unwrap_or_else(|| res.url().path().to_owned());
        if res.status() == StatusCode::UNPROCESSABLE_ENTITY {
            return Err("Assess can't check that address. Send a public website address, like your-business.example.".into());
        }
        if !(res.status().is_success() || res.status().is_redirection()) {
            return Err(BUSY.into());
        }
        location
            .rsplit_once("/assess/")
            .map(|(_, id)| id.split(['?', '#', '/']).next().unwrap_or("").to_owned())
            .filter(|id| !id.is_empty() && id != "fixture")
            .ok_or_else(|| BUSY.to_owned())
    }
}

/// The door's routes, merged into the home routes.
pub fn routes() -> Router<Home> {
    Router::new()
        .route("/.well-known/awp", get(descriptor))
        .route("/.well-known/awp.json", get(descriptor_json))
        .route("/mcp", get(mcp_get).post(mcp))
}

/// One tool at the door.
pub struct Tool {
    /// The MCP tool's name.
    pub name: &'static str,
    /// The capability it is in the descriptor.
    pub capability: &'static str,
    /// Short words.
    pub title: &'static str,
    /// What it does and what it never does.
    pub description: &'static str,
    /// `read_only` or `idempotent`.
    pub effect: &'static str,
    /// Calls an hour from one client.
    pub per_hour: usize,
    /// Where people read the same thing.
    pub page: &'static str,
    /// Its arguments.
    pub input_schema: Value,
}

/// Every tool at the door, in the descriptor's order.
pub fn tools() -> Vec<Tool> {
    let desk_ids: Vec<&str> = crate::desks::DESKS.iter().map(|d| d.id).collect();
    let none = json!({"type": "object", "properties": {}, "additionalProperties": false});
    vec![
        Tool {
            name: "list_plans",
            capability: "plans.read",
            title: "Plans and prices",
            description: "The plans from the price list, in US dollars: the monthly price, the price for 12 months with its discount, hosting, the AI included each month, the first month's AI, the WebAgents on each plan and those still coming. A person pays for a plan on agentrix.website with PayPal; no agent can.",
            effect: "read_only",
            per_hour: 120,
            page: "/prices",
            input_schema: none.clone(),
        },
        Tool {
            name: "list_samples",
            capability: "samples.read",
            title: "Sample sites and walkthroughs",
            description: "The three sample sites, businesses we made up: what each does, a walkthrough of what to open and ask, whether it is open now, and its own agent door.",
            effect: "read_only",
            per_hour: 120,
            page: "/samples",
            input_schema: none.clone(),
        },
        Tool {
            name: "read_how_it_works",
            capability: "how_it_works.read",
            title: "How it works",
            description: "The How it works page, section by section: how customers, owners and AI agents work with WebAgents, the move, and the ADK-Rust kernel behind them.",
            effect: "read_only",
            per_hour: 120,
            page: "/how-it-works",
            input_schema: none,
        },
        Tool {
            name: "ask_desk",
            capability: "desks.ask",
            title: "Ask a desk",
            description: "Ask one of this site's desks. The answer comes only from this site's pages and price list, cites them, and says whether an AI WebAgent wrote it. Don't send names, email addresses or phone numbers.",
            effect: "read_only",
            per_hour: 30,
            page: "/how-it-works",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "desk": {"type": "string", "enum": desk_ids, "description": "your-site, how-it-works, samples or prices."},
                    "question": {"type": "string", "maxLength": 400}
                },
                "required": ["desk", "question"],
                "additionalProperties": false
            }),
        },
        Tool {
            name: "start_check",
            capability: "checks.start",
            title: "Start a free check of a site",
            description: "Starts Assess's free check of a public website, or gives the recent one for the same site. It reads public pages only and changes nothing on that site. The report is read-only, at the address returned. It starts no move and signs no one up.",
            effect: "idempotent",
            per_hour: 5,
            page: "/",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "maxLength": 300, "description": "The site's public address, like your-business.example."}
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        },
    ]
}

/// What the door won't do, with the reason in plain words.
const DISABLED: &[(&str, &str)] = &[
    (
        "payments",
        "Plans are paid by a person on agentrix.website, with PayPal. No agent can pay here.",
    ),
    (
        "sign_up",
        "Creating an account, choosing a plan, starting a move or joining the weekly fix needs a person. No agent can do it for them.",
    ),
    ("events", "This door sends no events or webhooks."),
];

const PERSONAL: &str = "Don't send names, email addresses or phone numbers. A question or address with one is refused, and nothing an agent sends is kept beyond a count of answers.";

// ---- The descriptor ------------------------------------------------------------------------

fn json_response(kind: &'static str, cache: &'static str, body: &Value) -> Response {
    (
        [(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, cache)],
        body.to_string(),
    )
        .into_response()
}

/// The descriptor agents read: what agentrix.website is, its tools, its limits and what's off.
pub fn descriptor_doc(home: &Home) -> Value {
    let c = &home.inner.config;
    let o = &c.origin;
    let caps: Vec<Value> = tools()
        .iter()
        .map(|t| {
            json!({
                "id": t.capability,
                "tool": t.name,
                "title": t.title,
                "description": t.description,
                "tier": "anonymous",
                "effect": t.effect,
                "limit": format!("{} calls an hour per client", t.per_hour),
                "page": format!("{o}{}", t.page),
                "inputSchema": t.input_schema,
            })
        })
        .collect();
    let disabled: Vec<Value> = DISABLED
        .iter()
        .map(|(id, why)| json!({"id": id, "why": why}))
        .collect();
    json!({
        "version": {"major": 1, "minor": 0},
        "siteName": "Agentrix",
        "siteDescription": "agentrix.website moves a business's website onto Agentrix, where WebAgents answer its customers, its owner and other AI agents. This door reads the plans, the samples and How it works, asks the site's desks, and starts a free check of a site.",
        "host": c.platform_host,
        "capabilityManifestUrl": format!("{o}/.well-known/awp"),
        "mcp": {"url": format!("{o}/mcp"), "transport": "streamable-http", "protocolVersions": MCP_VERSIONS},
        "supportedTrustLevels": ["anonymous"],
        "capabilities": caps,
        "disabledCapabilities": DISABLED.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "disabled": disabled,
        "tiers": {"anonymous": "Any agent. Every tool here is open to it, within its limit."},
        "rules": {
            "personalData": PERSONAL,
            "sameOrigin": "A browser on another site can't call /mcp: a request whose Origin isn't this site is refused.",
            "ai": "Each desk answer says whether an AI WebAgent wrote it or it came from this site's pages without a model.",
        },
        "pages": {
            "howItWorks": format!("{o}/how-it-works"),
            "prices": format!("{o}/prices"),
            "samples": format!("{o}/samples"),
        },
    })
}

async fn descriptor(State(home): State<Home>) -> Response {
    json_response(
        "application/awp+json",
        "public, max-age=300",
        &descriptor_doc(&home),
    )
}

/// The same descriptor as plain JSON, for people who open the link in a browser.
async fn descriptor_json(State(home): State<Home>) -> Response {
    json_response(
        "application/json",
        "public, max-age=300",
        &descriptor_doc(&home),
    )
}

// ---- Limits --------------------------------------------------------------------------------

static CALLS: Mutex<Option<HashMap<String, Vec<u64>>>> = Mutex::new(None);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// At most `max` calls an hour for `key`.
fn allow(key: &str, max: usize) -> bool {
    let now = now();
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

/// The client's address as the proxy saw it, for per-client limits (never stored).
fn client(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// Whether a request's `Origin`, when it has one, is this site.
fn same_origin(headers: &HeaderMap, origin: &str) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|o| o.trim_end_matches('/') == origin.trim_end_matches('/'))
}

/// Whether `text` holds an email address or a phone number (nine digits or more, spaces,
/// dashes and brackets between them allowed).
fn personal(text: &str) -> bool {
    let email = text.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        w.split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && b.contains('.'))
    });
    let mut run = 0;
    let mut phone = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            phone |= run >= 9;
        } else if !matches!(c, ' ' | '-' | '(' | ')' | '+') {
            run = 0;
        }
    }
    email || phone
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

fn tool_error(id: &Value, message: &str) -> Value {
    rpc_result(
        id,
        json!({"isError": true, "content": [{"type": "text", "text": message}]}),
    )
}

async fn mcp_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST"), (header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "The agent door speaks MCP over POST: send one JSON-RPC 2.0 message per request. The descriptor is at /.well-known/awp.",
    )
        .into_response()
}

async fn mcp(State(home): State<Home>, headers: HeaderMap, body: Bytes) -> Response {
    if !same_origin(&headers, &home.inner.config.origin) {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "application/json")],
            rpc_error(
                &Value::Null,
                -32600,
                "A page on another site can't call this door. Call it from your agent, not a browser page.",
            )
            .to_string(),
        )
            .into_response();
    }
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
            rpc_response(&rpc_result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "Agentrix agent door", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!(
                        "Read agentrix.website's plans, samples and How it works, ask its desks, or start a free check of a public site. Answers come only from this site's pages and price list. Nothing here pays or signs anyone up; a person does that on the site. {PERSONAL}"
                    ),
                }),
            ))
        }
        _ if is_notification => StatusCode::ACCEPTED.into_response(),
        "ping" => rpc_response(&rpc_result(&id, json!({}))),
        "tools/list" => rpc_response(&rpc_result(&id, tools_list())),
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
            rpc_response(&call(&home, &headers, &id, name, &args).await)
        }
        _ => rpc_response(&rpc_error(
            &id,
            -32601,
            "Method not found. The door answers initialize, ping, tools/list and tools/call.",
        )),
    }
}

fn tools_list() -> Value {
    let list: Vec<Value> = tools()
        .into_iter()
        .map(|t| {
            let read_only = t.effect == "read_only";
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": read_only,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": t.name == "start_check"
                },
                "_meta": {"agentrix/tier": "anonymous", "agentrix/effect": t.effect, "agentrix/perHour": t.per_hour}
            })
        })
        .collect();
    json!({"tools": list})
}

/// A tool's output, text for the model and the same facts as structured content, or words
/// saying why it didn't run.
type Out = Result<(String, Value), String>;

async fn call(home: &Home, headers: &HeaderMap, id: &Value, name: &str, args: &Value) -> Value {
    let Some(tool) = tools().into_iter().find(|t| t.name == name) else {
        return rpc_error(
            id,
            -32602,
            &format!("{name} isn't a tool at this door. Call tools/list for its tools."),
        );
    };
    let who = client(headers);
    if !allow(&format!("{name}:{who}"), tool.per_hour) {
        return tool_error(
            id,
            &format!(
                "This client has called {name} {} times this hour. Try again later.",
                tool.per_hour
            ),
        );
    }
    let result = match name {
        "list_plans" => Ok(list_plans(home)),
        "list_samples" => Ok(list_samples(home)),
        "read_how_it_works" => read_how_it_works(home).await,
        "ask_desk" => ask_desk(home, args).await,
        "start_check" => start_check(home, args).await,
        _ => Err(format!("{name} isn't a tool at this door.")),
    };
    match result {
        Ok((text, structured)) => rpc_result(
            id,
            json!({"isError": false, "content": [{"type": "text", "text": text}], "structuredContent": structured}),
        ),
        Err(words) => tool_error(id, &words),
    }
}

fn text_arg(args: &Value, key: &str, max: usize) -> Result<String, String> {
    match args.get(key) {
        Some(Value::String(s)) if s.chars().count() > max => {
            Err(format!("{key} is longer than {max} characters."))
        }
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.trim().to_owned()),
        Some(Value::String(_)) | None | Some(Value::Null) => Err(format!("{key} is required.")),
        Some(_) => Err(format!("{key} must be text.")),
    }
}

/// "$29", "$278.40".
fn usd(cents: i64) -> String {
    if cents % 100 == 0 {
        format!("${}", cents / 100)
    } else {
        format!("${}.{:02}", cents / 100, cents % 100)
    }
}

fn list_plans(home: &Home) -> (String, Value) {
    let catalogue = agentrix_billing::catalogue();
    // The price list's one discount on a 12-month plan, the number /setup/plan charges.
    let discount = catalogue.annual_discount_percent;
    let mut lines = Vec::new();
    let plans: Vec<Value> = catalogue
        .plans
        .iter()
        .map(|p| {
            let hosting = p
                .includes
                .iter()
                .any(|l| l.to_lowercase().contains("hosting"));
            let site = p.kind == "site";
            let year = site.then(|| p.annual_cents());
            let mut line = format!(
                "{}: {} a {}{}",
                p.name,
                usd(p.price_cents),
                p.interval,
                if site { "" } else { " per client site" }
            );
            if let Some(y) = year {
                line.push_str(&format!(
                    ", or {} for 12 months ({discount}% off)",
                    usd(y)
                ));
            }
            line.push_str(&format!(". It includes: {}.", p.includes.join("; ")));
            if p.first_month_ai_micro_usd > 0 {
                line.push_str(&format!(
                    " The first month comes with {} of AI.",
                    usd(p.first_month_ai_micro_usd / 10_000)
                ));
            }
            lines.push(line);
            json!({
                "id": p.id,
                "name": p.name,
                "kind": p.kind,
                "currency": catalogue.currency,
                "price_cents": p.price_cents,
                "interval": p.interval,
                "price": format!("{} a {}", usd(p.price_cents), p.interval),
                "twelve_months": year.map(|y| json!({
                    "price_cents": y,
                    "price": usd(y),
                    "discount_percent": discount,
                })),
                "hosting_included": hosting,
                "ai_each_month_usd_cents": p.included_ai_micro_usd / 10_000,
                "first_month_ai_usd_cents": (p.first_month_ai_micro_usd > 0).then_some(p.first_month_ai_micro_usd / 10_000),
                "minimum_sites": p.minimum_sites,
                "webagents": p.webagents,
                "webagents_coming": p.webagents_soon.iter().map(|w| json!({"name": w.name, "role": w.role})).collect::<Vec<_>>(),
                "includes": p.includes,
            })
        })
        .collect();
    let o = &home.inner.config.origin;
    let text = format!(
        "{}\n\nPlans are paid monthly or for 12 months, in US dollars, with PayPal, by a person on {o}/prices. Moving a site is free.",
        lines.join("\n")
    );
    (
        text,
        json!({
            "currency": catalogue.currency,
            "plans": plans,
            "pay_with": ["PayPal"],
            "billed": ["monthly", "12 months"],
            "move": "free",
            "page": format!("{o}/prices"),
            "paid_by": "a person, on the site",
        }),
    )
}

fn list_samples(home: &Home) -> (String, Value) {
    let c = &home.inner.config;
    let open = home.inner.probe.open();
    let mut lines = Vec::new();
    let samples: Vec<Value> = home
        .inner
        .samples
        .iter()
        .map(|s| {
            let host = format!("{}.{}", s.slug, c.platform_host);
            let url = crate::view::site_url(&c.origin, &host);
            let open_now = open.contains(&host);
            lines.push(format!(
                "{} ({}): {} {} {} {}",
                s.name,
                s.tag,
                url,
                s.line,
                s.try_this,
                if open_now {
                    "Open now."
                } else {
                    "Opening soon."
                }
            ));
            json!({
                "id": s.id,
                "name": s.name,
                "tag": s.tag,
                "url": url,
                "open": open_now,
                "made_up": true,
                "line": s.line,
                "try_this": s.try_this,
                "walkthrough": s.walk,
                "owner_side": format!("{}/samples/{}/", c.origin, s.slug),
                "agent_door": format!("{}/.well-known/awp", url.trim_end_matches('/')),
            })
        })
        .collect();
    (
        format!(
            "Three sample sites, businesses we made up, each run by WebAgents:\n{}",
            lines.join("\n")
        ),
        json!({"samples": samples, "page": format!("{}/samples", c.origin)}),
    )
}

async fn read_how_it_works(home: &Home) -> Out {
    let o = &home.inner.config.origin;
    let html = home.render_view("how.html").await.map_err(|e| {
        tracing::error!(error = %e, "door: rendering How it works");
        "The door couldn't read How it works just now. Try again in a moment.".to_owned()
    })?;
    let sections = crate::desks::sections("How it works", "/how-it-works", &html);
    let text = sections
        .iter()
        .map(|s| format!("{}\n{}", s.label, s.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let list: Vec<Value> = sections
        .iter()
        .map(|s| json!({"label": s.label, "url": format!("{o}{}", s.href), "text": s.text}))
        .collect();
    Ok((
        text,
        json!({"sections": list, "page": format!("{o}/how-it-works")}),
    ))
}

async fn ask_desk(home: &Home, args: &Value) -> Out {
    let desk = text_arg(args, "desk", 40)?;
    let question = text_arg(args, "question", 400)?;
    if crate::desks::desk(&desk).is_none() {
        let ids: Vec<&str> = crate::desks::DESKS.iter().map(|d| d.id).collect();
        return Err(format!(
            "There's no desk called {desk}. The desks are {}.",
            ids.join(", ")
        ));
    }
    if personal(&question) {
        return Err(format!("The question was refused. {PERSONAL}"));
    }
    let a = home
        .ask(&desk, &question, &[])
        .await
        .ok_or_else(|| format!("There's no desk called {desk}."))?;
    let o = &home.inner.config.origin;
    let answer = a
        .phrases
        .iter()
        .map(|p| p.replace("**", ""))
        .collect::<Vec<_>>()
        .join(" ");
    let link = |href: &str| {
        if href.starts_with("http") {
            href.to_owned()
        } else {
            format!("{o}{href}")
        }
    };
    let sources: Vec<Value> = a
        .sources
        .iter()
        .map(|s| json!({"n": s.n, "label": s.label, "url": link(&s.href)}))
        .collect();
    let by = if a.ai {
        format!("Written by the {} desk, an AI WebAgent", a.name)
    } else {
        format!(
            "Answered by the {} desk from this site's pages, without a model",
            a.name
        )
    };
    let mut text = answer.clone();
    for s in &a.sources {
        text.push_str(&format!("\n[{}] {}: {}", s.n, s.label, link(&s.href)));
    }
    text.push_str(&format!("\n\n{by}."));
    Ok((
        text,
        json!({
            "answer": answer,
            "desk": a.desk,
            "desk_name": a.name,
            "ai": a.ai,
            "covered": a.covered,
            "sources": sources,
            "next": a.step.as_ref().map(|s| json!({"label": s.label, "url": link(&s.href)})),
            "by": by,
        }),
    ))
}

/// A public website address Assess may read, with any query or fragment dropped so nothing
/// personal travels in it.
fn public_site(input: &str) -> Result<Url, String> {
    let words = "Send a public website address, like your-business.example.";
    let input = input.trim();
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut url = Url::parse(&with_scheme).map_err(|_| words.to_owned())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(words.into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("The address has sign-in details in it. {words}"));
    }
    // A name, never an IP address.
    let Some(host) = url.domain().map(str::to_ascii_lowercase) else {
        return Err(words.into());
    };
    let local = [
        "localhost",
        ".localhost",
        ".local",
        ".internal",
        ".lan",
        ".home.arpa",
        ".corp",
        ".test",
        ".example",
        ".invalid",
    ];
    let reserved = local
        .iter()
        .any(|l| host == l.trim_start_matches('.') || host.ends_with(l));
    if !host.contains('.') || reserved {
        return Err(words.into());
    }
    url.set_query(None);
    url.set_fragment(None);
    if url.path().contains('@') {
        return Err(format!("The address was refused. {PERSONAL}"));
    }
    Ok(url)
}

async fn start_check(home: &Home, args: &Value) -> Out {
    let url = public_site(&text_arg(args, "url", 300)?)?;
    // Across every agent, a bound on the checks the door starts.
    if !allow("start_check:all", 60) {
        return Err("The door has started its checks for this hour. Try again later, or paste the address on the home page.".into());
    }
    let id = home.inner.checks.start(&url).await?;
    let o = &home.inner.config.origin;
    let report = format!("{o}/assess/{id}");
    Ok((
        format!(
            "The check of {} has started (or is the recent one for that site). It reads public pages only and changes nothing on the site. The report is read-only, at {report}. Its progress streams at {report}/events.",
            url.host_str().unwrap_or_default()
        ),
        json!({
            "check": id,
            "site": url.as_str(),
            "report": report,
            "events": format!("{report}/events"),
            "read_only": true,
        }),
    ))
}

/// A stand-in that starts nothing, for previews and tests without Assess.
pub struct NoChecks;

#[async_trait]
impl Checks for NoChecks {
    async fn start(&self, _url: &Url) -> Result<String, String> {
        Err(
            "Assess isn't running on this server. Paste the address on the home page instead."
                .into(),
        )
    }
}

/// Assess as node runs it, at the platform's own origin.
pub fn checks_for(origin: &str, http: reqwest::Client) -> Arc<dyn Checks> {
    Arc::new(HttpChecks::new(origin, http))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_details_are_spotted() {
        assert!(personal("reach me at jane@example.com please"));
        assert!(personal("call 0712 345 678"));
        assert!(personal("+254712345678"));
        assert!(!personal("How much is the Business plan?"));
        assert!(!personal("We have 120 products and 3 shops"));
        assert!(!personal("Is it $79.00 a month?"));
    }

    #[test]
    fn only_public_addresses_are_checked() {
        assert_eq!(
            public_site("kilele.co.ke/menu?ref=jane@x.com#top")
                .unwrap()
                .as_str(),
            "https://kilele.co.ke/menu"
        );
        assert!(public_site("localhost:8080").is_err());
        assert!(public_site("http://10.0.0.1/").is_err());
        assert!(public_site("https://user:pass@shop.co.ke/").is_err());
        assert!(public_site("ftp://shop.co.ke/").is_err());
        assert!(public_site("intranet").is_err());
    }

    #[test]
    fn twelve_months_is_the_monthly_price_less_the_discount() {
        let essential = agentrix_billing::plan("essential").unwrap();
        assert_eq!(usd(essential.annual_cents()), "$278.40");
        assert_eq!(usd(7900), "$79");
    }
}
