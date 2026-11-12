//! The agent door on Kilele's sample host: discovery, the D9 tools, the held custom order and
//! the people's page.

use std::path::Path;

use agentrix_samples::Config;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const HOST: &str = "kilele-bakery-sample.localhost";
/// The bearer value the stand-in checker takes as proof.
const PROVEN: &str = "proven-agent";
const PROVEN_ID: &str = "wba:https://assistant.example#key-1";

/// Stands in for saas-settle's checker: one bearer value proves an agent, "tampered" is a proof
/// that doesn't check, and anything else carries no proof.
struct StandInProof;

#[async_trait::async_trait]
impl agentrix_samples::AgentProof for StandInProof {
    async fn prove(
        &self,
        request: agentrix_samples::ProofRequest<'_>,
    ) -> Result<Option<agentrix_samples::ProvenAgent>, agentrix_samples::ProofFailure> {
        let bearer = request
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        match bearer {
            Some(PROVEN) => Ok(Some(agentrix_samples::ProvenAgent {
                id: PROVEN_ID.into(),
                vouched_by: "https://assistant.example".into(),
                method: "web-bot-auth".into(),
            })),
            Some("tampered") => Err(agentrix_samples::ProofFailure {
                code: "token_refused".into(),
                message: "its signature doesn't check.".into(),
            }),
            _ => Ok(None),
        }
    }

    async fn refusal_receipt(&self, refusal: agentrix_samples::DoorRefusal<'_>) -> Option<Value> {
        Some(json!({"body": {
            "refused_by": refusal.door, "code": refusal.code, "message": refusal.tool,
            "claimed_agent": refusal.claimed, "nothing_moved": true,
        }, "signature": "stand-in"}))
    }
}

async fn router() -> Router {
    agentrix_samples::set_agent_proof(std::sync::Arc::new(StandInProof));
    let ctx = agentrix_host::testing::context().await.expect("context");
    agentrix_identity::migrate(&ctx.pool)
        .await
        .expect("identity migrations");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = Config {
        data_dir: std::env::temp_dir()
            .join(format!("agentrix-samples-door-{}", rand::random::<u64>())),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: None,
        reseed: true,
    };
    let mounted = agentrix_samples::mount_with(&ctx, config).expect("mount");
    mounted
        .hosts
        .into_iter()
        .next()
        .expect("sample hosts")
        .router
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

fn get(path: &str) -> Request<Body> {
    Request::get(path)
        .header(header::HOST, HOST)
        .body(Body::empty())
        .expect("request")
}

fn rpc(session: Option<&str>, body: &Value) -> Request<Body> {
    rpc_as(session, None, body)
}

fn rpc_as(session: Option<&str>, bearer: Option<&str>, body: &Value) -> Request<Body> {
    let mut b = Request::post("/mcp")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(s) = session {
        b = b.header("mcp-session-id", s);
    }
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).expect("request")
}

/// `initialize` as a named client; returns the session id.
async fn initialize(app: &Router) -> String {
    let (status, headers, body) = send(
        app,
        rpc(None, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "assistant.example", "version": "1"}}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    headers
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .expect("session id")
        .to_owned()
}

async fn call(app: &Router, session: Option<&str>, tool: &str, args: Value) -> Value {
    call_as(app, session, None, tool, args).await
}

async fn call_as(
    app: &Router,
    session: Option<&str>,
    bearer: Option<&str>,
    tool: &str,
    args: Value,
) -> Value {
    let (status, _, body) = send(app, rpc_as(session, bearer, &json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": tool, "arguments": args}}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).expect("json");
    v["result"].clone()
}

#[tokio::test]
async fn tools_list_names_exactly_the_d9_tools_and_ask() {
    let app = router().await;
    let (_, _, body) = send(
        &app,
        rpc(
            None,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        ),
    )
    .await;
    let v: Value = serde_json::from_str(&body).expect("json");
    let mut names: Vec<String> = v["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or("").to_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "ask_site",
            "catalog.search",
            "checkout.create",
            "delivery.slots",
            "order.status",
            "quote.create",
            "request_custom_order"
        ]
    );
    for t in v["result"]["tools"].as_array().expect("tools") {
        assert!(t["inputSchema"]["type"] == "object", "{t}");
        let tier = t["_meta"]["agentrix/tier"].as_str().unwrap_or("");
        let want = if ["quote.create", "checkout.create", "request_custom_order"]
            .contains(&t["name"].as_str().unwrap_or(""))
        {
            "known"
        } else {
            "anonymous"
        };
        assert_eq!(tier, want, "{}", t["name"]);
        let proof = t["_meta"]["agentrix/proof"].as_str().unwrap_or("");
        assert_eq!(
            proof.starts_with("required"),
            want == "known",
            "{}",
            t["name"]
        );
    }
}

#[tokio::test]
async fn catalog_search_finds_sourdough_with_its_price() {
    let app = router().await;
    let r = call(&app, None, "catalog.search", json!({"query": "sourdough"})).await;
    assert_eq!(r["isError"], false, "{r}");
    let first = &r["structuredContent"]["items"][0];
    assert_eq!(first["sku"], "sourdough");
    assert_eq!(first["price_minor"], 45_000);
    assert_eq!(first["price"], "KSh 450");
    assert!(first["stock"]["left"].is_i64(), "{first}");
    assert!(
        first["url"]
            .as_str()
            .unwrap_or("")
            .ends_with("/bread#item-sourdough")
    );
}

#[tokio::test]
async fn quotes_need_a_proven_agent_and_price_from_the_list() {
    let app = router().await;
    let lines = json!({"lines": [{"sku": "sourdough", "qty": 2}], "zone": "Kilimani"});
    // No signature, no token: refused with a signed receipt, nothing created.
    let r = call(&app, None, "quote.create", lines.clone()).await;
    assert_eq!(r["isError"], true, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["code"], "agent_identity_required");
    assert_eq!(s["nothing_moved"], true);
    assert_eq!(s["receipt"]["body"]["code"], "agent_identity_required");
    assert!(
        s["receipt"]["body"]["refused_by"]
            .as_str()
            .unwrap_or("")
            .contains(HOST)
    );
    // Only a claimed name: refused, the receipt naming the claim.
    let session = initialize(&app).await;
    let r = call(&app, Some(&session), "quote.create", lines.clone()).await;
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["claimed_agent"], "assistant.example");
    assert_eq!(
        r["structuredContent"]["receipt"]["body"]["claimed_agent"],
        "assistant.example"
    );
    assert!(
        r["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("only a name")
    );
    // A proof that doesn't check: refused.
    let r = call_as(
        &app,
        Some(&session),
        Some("tampered"),
        "quote.create",
        lines.clone(),
    )
    .await;
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(r["structuredContent"]["code"], "token_refused");
    // A proven agent: priced, and the quote names it and who vouched.
    let r = call_as(&app, Some(&session), Some(PROVEN), "quote.create", lines).await;
    assert_eq!(r["isError"], false, "{r}");
    let q = &r["structuredContent"];
    assert_eq!(q["agent"]["id"], PROVEN_ID);
    assert_eq!(q["agent"]["vouched_by"], "https://assistant.example");
    assert_eq!(q["subtotal_minor"], 90_000);
    assert_eq!(q["delivery_minor"], 30_000);
    assert_eq!(q["total_minor"], 120_000);
    assert!(q["usd"].as_str().unwrap_or("").starts_with("USD "));
}

#[tokio::test]
async fn checkout_makes_a_link_for_a_person_and_charges_nothing() {
    let app = router().await;
    let session = initialize(&app).await;
    let r = call_as(
        &app,
        Some(&session),
        Some(PROVEN),
        "checkout.create",
        json!({"lines": [{"sku": "croissant", "qty": 1}], "fulfilment": "collection"}),
    )
    .await;
    assert_eq!(r["isError"], false, "{r}");
    let link = r["structuredContent"]["link"]
        .as_str()
        .expect("link")
        .to_owned();
    assert_eq!(r["structuredContent"]["charged"], false);
    assert_eq!(r["structuredContent"]["agent"]["id"], PROVEN_ID);
    let path = link
        .split_once(HOST)
        .map(|(_, p)| p.to_owned())
        .expect("link on this host");
    let (status, headers, _) = send(&app, get(&path)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/bag");
    let cookie = headers[header::SET_COOKIE].to_str().unwrap_or("");
    assert!(cookie.starts_with("agx_cart=cart-"), "{cookie}");
}

#[tokio::test]
async fn request_custom_order_returns_a_held_kb_order() {
    let app = router().await;
    let session = initialize(&app).await;
    let day = (chrono::Utc::now() + chrono::Duration::days(10))
        .format("%Y-%m-%d")
        .to_string();
    let cake = json!({"size": "size.2", "sponge": "sponge.lemon", "colour": "colour.forest", "finish": "finish.ombre", "deco": "deco.berries", "message": "Happy 7th, Amina", "date": day, "note": "no nuts"});
    let order = json!({"cake": cake, "name": "Njeri", "email": "njeri@example.com", "fulfilment": "collection"});
    // A name alone holds nothing for the owner.
    let r = call(&app, Some(&session), "request_custom_order", order.clone()).await;
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["structuredContent"]["receipt"].is_object());
    let r = call_as(
        &app,
        Some(&session),
        Some(PROVEN),
        "request_custom_order",
        order,
    )
    .await;
    assert_eq!(r["isError"], false, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["agent"]["id"], PROVEN_ID);
    let id = s["order_id"].as_str().expect("order id").to_owned();
    assert!(id.starts_with("KB-"), "{id}");
    assert_eq!(s["state"], "held");
    assert_eq!(s["charged"], false);
    assert!(
        s["words"]
            .as_str()
            .unwrap_or("")
            .starts_with("Held for Wanjiru.")
    );
    let (status, _, body) = send(&app, get(&format!("/awp/v2/calls/{id}"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let call: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(call["state"], "pending");
    assert_eq!(call["statusWords"], "working: waiting for the owner");
    let r = call_status(&app, &id).await;
    assert!(r.contains("waiting for the owner"), "{r}");
}

async fn call_status(app: &Router, id: &str) -> String {
    let r = call(app, None, "order.status", json!({"order_id": id})).await;
    assert!(r["structuredContent"].get("contact_name").is_none());
    r["content"][0]["text"].as_str().unwrap_or("").to_owned()
}

#[tokio::test]
async fn the_news_door_lists_stories_and_locks_subscriber_ones() {
    let app = router().await;
    let tana = "tana-valley-times-sample.localhost";
    let post = |body: Value| {
        Request::post("/mcp")
            .header(header::HOST, tana)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    };
    let (_, _, body) = send(
        &app,
        post(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})),
    )
    .await;
    let v: Value = serde_json::from_str(&body).expect("json");
    let mut names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["ask_site", "stories.search"]);
    let (_, _, body) = send(&app, post(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "stories.search", "arguments": {"query": "matatu"}}}))).await;
    let v: Value = serde_json::from_str(&body).expect("json");
    let stories = v["result"]["structuredContent"]["stories"]
        .as_array()
        .expect("stories")
        .clone();
    assert!(!stories.is_empty(), "{v}");
    let locked: Vec<&Value> = stories.iter().filter(|s| s["subscriber"] == true).collect();
    assert!(!locked.is_empty(), "{v}");
    for s in locked {
        let keys: Vec<&String> = s.as_object().expect("story").keys().collect();
        assert_eq!(keys.len(), 3, "{s}");
        assert!(s["url"].as_str().unwrap_or("").contains(tana));
    }
}

#[tokio::test]
async fn discovery_and_the_door_page_say_sample_terms() {
    let app = router().await;
    let (status, _, body) = send(&app, get("/.well-known/ai-catalog.json")).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json");
    assert!(
        v["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .any(|e| e["type"] == "mcp")
    );
    let (status, _, body) = send(&app, get("/.well-known/awp")).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["sample"], true);
    let custom = v["capabilities"]
        .as_array()
        .expect("caps")
        .iter()
        .find(|c| c["id"] == "request_custom_order")
        .expect("custom")
        .clone();
    assert_eq!(custom["tier"], "known");
    assert_eq!(custom["approval"], "always");
    let (status, _, page) = send(&app, get("/agents")).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Sample terms, not legal advice."));
    assert!(page.contains("request a custom cake, held for Wanjiru"));
    assert!(!page.contains('\u{2014}'));
}
