//! agentrix.website's own agent door: the descriptor says what the door does and what's off,
//! each MCP tool answers from the same data the pages use, and the door refuses what it
//! should: personal details, other sites' browser pages, private addresses and too many calls.

use std::sync::{Arc, Mutex};

use agentrix_home::door::Checks;
use agentrix_home::{FactSource, Facts, Home, HomeConfig};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use reqwest::Url;
use serde_json::{Value, json};
use tower::ServiceExt;

const ORIGIN: &str = "https://agentrix.website";

struct NoFacts;

#[async_trait]
impl FactSource for NoFacts {
    async fn read(&self) -> anyhow::Result<Facts> {
        Ok(Facts::default())
    }
}

/// Assess, recording what it was asked to check.
#[derive(Default)]
struct Recorded(Mutex<Vec<String>>);

#[async_trait]
impl Checks for Recorded {
    async fn start(&self, url: &Url) -> Result<String, String> {
        self.0.lock().unwrap().push(url.to_string());
        Ok("as-1".into())
    }
}

fn door() -> (Router, Arc<Recorded>) {
    let config = HomeConfig {
        origin: ORIGIN.into(),
        assess_action: "/assess".into(),
        fixture_href: None,
        links: Default::default(),
        platform_host: "agentrix.website".into(),
        film: None,
        contact: None,
    };
    let checks = Arc::new(Recorded::default());
    let home = Home::new(config, Arc::new(NoFacts))
        .unwrap()
        .with_checks(checks.clone());
    (home.routes(), checks)
}

async fn get(app: &Router, path: &str) -> (StatusCode, String, String) {
    let res = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let kind = res
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, kind, String::from_utf8_lossy(&bytes).into_owned())
}

/// One JSON-RPC message to `/mcp` from `client` (its forwarded address), with extra headers.
async fn rpc(
    app: &Router,
    client: &str,
    headers: &[(&str, &str)],
    body: &Value,
) -> (StatusCode, Value) {
    let mut req = Request::post("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", client);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `tools/call`: the result object.
async fn call(app: &Router, client: &str, name: &str, args: Value) -> Value {
    let (status, v) = rpc(
        app,
        client,
        &[],
        &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    v["result"].clone()
}

fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn the_descriptor_says_what_the_door_does_and_what_is_off() {
    let (app, _) = door();
    let (status, kind, body) = get(&app, "/.well-known/awp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind, "application/awp+json");
    let d: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(d["siteName"], "Agentrix");
    assert_eq!(d["version"], json!({"major": 1, "minor": 0}));
    assert_eq!(d["mcp"]["url"], format!("{ORIGIN}/mcp"));
    assert_eq!(
        d["capabilityManifestUrl"],
        format!("{ORIGIN}/.well-known/awp")
    );
    assert_eq!(d["supportedTrustLevels"], json!(["anonymous"]));
    let tools: Vec<&str> = d["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["tool"].as_str().unwrap())
        .collect();
    assert_eq!(
        tools,
        [
            "list_plans",
            "list_samples",
            "read_how_it_works",
            "ask_desk",
            "start_check"
        ]
    );
    for c in d["capabilities"].as_array().unwrap() {
        assert_eq!(c["tier"], "anonymous", "{c}");
        assert!(c["limit"].as_str().unwrap().ends_with("an hour per client"));
        assert!(c["page"].as_str().unwrap().starts_with(ORIGIN));
    }
    // What's off is said plainly.
    let off = d["disabledCapabilities"].as_array().unwrap();
    assert!(off.contains(&json!("payments")) && off.contains(&json!("sign_up")));
    assert!(
        d["disabled"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| !x["why"].as_str().unwrap().is_empty())
    );
    assert!(
        d["rules"]["personalData"]
            .as_str()
            .unwrap()
            .contains("email")
    );

    // The same descriptor as plain JSON, for the link people open.
    let (status, kind, json_body) = get(&app, "/.well-known/awp.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kind, "application/json");
    assert_eq!(json_body, body);

    // tools/list carries the same tools, read-only except the check.
    let (_, init) = rpc(
        &app,
        "10.1.0.1",
        &[],
        &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
    )
    .await;
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("Nothing here pays or signs anyone up")
    );
    let (_, list) = rpc(
        &app,
        "10.1.0.1",
        &[],
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let listed = list["result"]["tools"].as_array().unwrap();
    assert_eq!(listed.len(), tools.len());
    for t in listed {
        let read_only = t["annotations"]["readOnlyHint"].as_bool().unwrap();
        assert_eq!(read_only, t["name"] != "start_check", "{t}");
    }
}

#[tokio::test]
async fn list_plans_reads_the_catalogue_with_hosting_and_both_ais() {
    let (app, _) = door();
    let r = call(&app, "10.2.0.1", "list_plans", json!({})).await;
    assert_eq!(r["isError"], false, "{r}");
    let plans = r["structuredContent"]["plans"].as_array().unwrap();
    let catalogue = agentrix_billing::catalogue();
    assert_eq!(plans.len(), catalogue.plans.len());
    for (p, c) in plans.iter().zip(&catalogue.plans) {
        assert_eq!(p["id"], c.id.as_str());
        assert_eq!(p["price_cents"], c.price_cents);
        assert_eq!(
            p["hosting_included"],
            c.includes.iter().any(|l| l.contains("hosting"))
        );
        assert_eq!(
            p["ai_each_month_usd_cents"],
            c.included_ai_micro_usd / 10_000
        );
        if c.kind == "site" {
            // The door quotes the catalogue's one discount, the number /setup/plan charges.
            let pct = catalogue.annual_discount_percent;
            assert_eq!(pct, 20);
            assert_eq!(p["twelve_months"]["price_cents"], c.annual_cents());
            assert_eq!(
                p["twelve_months"]["price_cents"],
                c.price_cents * 12 * (100 - pct) / 100
            );
            assert_eq!(p["twelve_months"]["discount_percent"], pct);
        } else {
            assert!(p["twelve_months"].is_null(), "{p}");
        }
        if c.first_month_ai_micro_usd > 0 {
            assert_eq!(
                p["first_month_ai_usd_cents"],
                c.first_month_ai_micro_usd / 10_000
            );
        }
    }
    assert_eq!(r["structuredContent"]["pay_with"], json!(["PayPal"]));
    assert!(text(&r).contains("by a person"));
}

#[tokio::test]
async fn list_samples_and_how_it_works_read_the_pages_data() {
    let (app, _) = door();
    let r = call(&app, "10.3.0.1", "list_samples", json!({})).await;
    let samples = r["structuredContent"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 3);
    let kilele = &samples[0];
    assert_eq!(
        kilele["url"],
        "https://kilele-bakery-sample.agentrix.website/"
    );
    assert_eq!(
        kilele["agent_door"],
        "https://kilele-bakery-sample.agentrix.website/.well-known/awp"
    );
    assert_eq!(kilele["made_up"], true);
    assert!(!kilele["walkthrough"].as_array().unwrap().is_empty());
    assert!(text(&r).contains("businesses we made up"));

    let r = call(&app, "10.3.0.1", "read_how_it_works", json!({})).await;
    assert_eq!(r["isError"], false, "{r}");
    let sections = r["structuredContent"]["sections"].as_array().unwrap();
    assert!(!sections.is_empty());
    assert!(sections.iter().all(|s| {
        s["url"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{ORIGIN}/how-it-works"))
    }));
}

#[tokio::test]
async fn ask_desk_answers_as_the_page_does_with_its_sources() {
    let (app, _) = door();
    let r = call(
        &app,
        "10.4.0.1",
        "ask_desk",
        json!({"desk": "prices", "question": "What does the Business plan cost?"}),
    )
    .await;
    assert_eq!(r["isError"], false, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["desk"], "prices");
    // No model here: the answer says so, and never says AI.
    assert_eq!(s["ai"], false);
    assert!(s["by"].as_str().unwrap().contains("without a model"), "{s}");
    for src in s["sources"].as_array().unwrap() {
        assert!(src["url"].as_str().unwrap().starts_with(ORIGIN), "{src}");
    }
}

#[tokio::test]
async fn start_check_starts_assess_on_a_public_address_only() {
    let (app, checks) = door();
    let r = call(
        &app,
        "10.5.0.1",
        "start_check",
        json!({"url": "kilele.co.ke/menu?ref=jane@mail.example#top"}),
    )
    .await;
    assert_eq!(r["isError"], false, "{r}");
    let s = &r["structuredContent"];
    assert_eq!(s["report"], format!("{ORIGIN}/assess/as-1"));
    assert_eq!(s["read_only"], true);
    // The query went nowhere: Assess got the bare address.
    assert_eq!(*checks.0.lock().unwrap(), ["https://kilele.co.ke/menu"]);

    for url in [
        "localhost:8080",
        "http://10.0.0.1/",
        "https://me:pw@shop.co.ke/",
    ] {
        let r = call(&app, "10.5.0.2", "start_check", json!({ "url": url })).await;
        assert_eq!(r["isError"], true, "{url}: {r}");
    }
    assert_eq!(
        checks.0.lock().unwrap().len(),
        1,
        "nothing else reached Assess"
    );
}

#[tokio::test]
async fn the_door_refuses_personal_details_other_sites_and_too_many_calls() {
    let (app, checks) = door();
    // Personal details in a question.
    let r = call(
        &app,
        "10.6.0.1",
        "ask_desk",
        json!({"desk": "your-site", "question": "Email me at jane@mail.example about my site"}),
    )
    .await;
    assert_eq!(r["isError"], true);
    assert!(text(&r).contains("Don't send names, email addresses or phone numbers"));
    let r = call(
        &app,
        "10.6.0.1",
        "ask_desk",
        json!({"desk": "prices", "question": "Call me on 0712 345 678"}),
    )
    .await;
    assert_eq!(r["isError"], true);
    // A desk that isn't one, and a missing question.
    let r = call(
        &app,
        "10.6.0.1",
        "ask_desk",
        json!({"desk": "billing", "question": "Hi"}),
    )
    .await;
    assert!(text(&r).contains("The desks are your-site, how-it-works, samples, prices"));
    let r = call(&app, "10.6.0.1", "ask_desk", json!({"desk": "prices"})).await;
    assert!(text(&r).contains("question is required"));
    // A tool that isn't one.
    let (_, v) = rpc(
        &app,
        "10.6.0.1",
        &[],
        &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "buy_plan", "arguments": {}}}),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);

    // A browser page on another site.
    let (status, v) = rpc(
        &app,
        "10.6.0.2",
        &[("origin", "https://evil.example")],
        &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("another site")
    );
    // This site's own pages may.
    let (status, _) = rpc(
        &app,
        "10.6.0.2",
        &[("origin", ORIGIN)],
        &json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Five checks an hour from one client.
    for i in 0..5 {
        let r = call(
            &app,
            "10.6.0.3",
            "start_check",
            json!({"url": format!("shop{i}.co.ke")}),
        )
        .await;
        assert_eq!(r["isError"], false, "{r}");
    }
    let r = call(
        &app,
        "10.6.0.3",
        "start_check",
        json!({"url": "shop9.co.ke"}),
    )
    .await;
    assert_eq!(r["isError"], true);
    assert!(text(&r).contains("5 times this hour"));
    assert_eq!(checks.0.lock().unwrap().len(), 5);

    // GET and batches are refused with words.
    let (status, _, body) = get(&app, "/mcp").await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(body.contains("/.well-known/awp"));
    let (_, v) = rpc(
        &app,
        "10.6.0.4",
        &[],
        &json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]),
    )
    .await;
    assert_eq!(v["error"]["code"], -32600);
}
