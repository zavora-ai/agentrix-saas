//! The agent door over a real signed site: discovery, MCP, a held custom order the owner
//! approves, polling, idempotency, validation, and the cited `ask_site` tool. The order flow
//! follows the framework's adk-website test (credited in the module docs).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use awp_types::{HeldCall, HeldCallState};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;
use crate::site::testkit::{DOMAIN, Site};

/// Held calls in memory; `approve` plays the owner.
#[derive(Default)]
struct Held {
    calls: Mutex<HashMap<String, HeldCall>>,
    keys: Mutex<HashMap<String, String>>,
    cards: Mutex<Vec<String>>,
}

impl Held {
    fn approve(&self, id: &str) {
        let mut calls = self.calls.lock().unwrap();
        let call = calls.get_mut(id).unwrap();
        call.state = HeldCallState::Completed;
        call.result = Some(json!({"reference": {"kind": "order", "id": "K-1043"}}));
    }
}

#[async_trait]
impl HeldCalls for Held {
    async fn raise(
        &self,
        site: &str,
        capability: &str,
        _input: Value,
        key: &str,
    ) -> anyhow::Result<HeldCall> {
        let scoped = format!("{site}:{key}");
        if let Some(id) = self.keys.lock().unwrap().get(&scoped) {
            return Ok(self.calls.lock().unwrap()[id].clone());
        }
        let id = format!("c-{}", self.calls.lock().unwrap().len() + 1);
        let call = HeldCall::pending(&id, capability, chrono::Utc::now());
        self.calls.lock().unwrap().insert(id.clone(), call.clone());
        self.keys.lock().unwrap().insert(scoped, id.clone());
        self.cards.lock().unwrap().push(id);
        Ok(call)
    }
    async fn status(&self, _site: &str, id: &str) -> anyhow::Result<Option<HeldCall>> {
        Ok(self.calls.lock().unwrap().get(id).cloned())
    }
}

async fn setup() -> (Router, Arc<Held>, Arc<crate::site::testkit::Receipts>) {
    let site = Site::new().await;
    site.release(1, "Sourdough and cakes").await;
    let (state, receipts) = site.state();
    let held = Arc::new(Held::default());
    let app = crate::site::router(state.clone()).merge(router(DoorState {
        site: state,
        held: held.clone(),
    }));
    (app, held, receipts)
}

async fn rpc(app: &Router, body: Value) -> (StatusCode, HeaderMap, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, DOMAIN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, value)
}

async fn get_json(app: &Router, path: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .uri(path)
        .header(header::HOST, DOMAIN)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn order_args() -> Value {
    json!({
        "description": "A 2-tier forest-green cake with fresh berries; Happy 7th, Amina!",
        "date": "2099-11-14",
        "name": "Njeri",
        "email": "njeri@example.com"
    })
}

#[tokio::test]
async fn discovery_lists_the_door_and_its_capabilities() {
    let (app, _, _) = setup().await;
    let (status, doc) = get_json(&app, "/.well-known/awp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(doc["mcp"]["url"], "/mcp");
    let names: Vec<&str> = doc["capabilities"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["ask_site", "request_custom_order"]);
    assert_eq!(
        doc["capabilities"]["capabilities"][1]["approval"]["mode"],
        "always"
    );
}

#[tokio::test]
async fn a_custom_order_over_mcp_is_held_for_the_owner_and_polls_to_completed() {
    let (app, held, receipts) = setup().await;

    let (status, headers, init) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "assistant", "version": "1"}}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(headers.get("mcp-session-id").is_some());

    let (status, _, _) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (_, _, list) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let tools = list["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "request_custom_order"));

    let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
        "name": "request_custom_order",
        "arguments": order_args(),
        "_meta": {"agenticwebprotocol.com/idempotency-key": "order-1"}
    }});
    let (_, _, held_reply) = rpc(&app, call.clone()).await;
    assert_eq!(held_reply["result"]["isError"], false);
    let structured = &held_reply["result"]["structuredContent"];
    assert_eq!(structured["state"], "pending");
    let id = structured["id"].as_str().unwrap().to_owned();
    assert_eq!(structured["status"], format!("/awp/v2/calls/{id}"));
    assert_eq!(
        held.cards.lock().unwrap().len(),
        1,
        "one card for the owner"
    );

    // The same key again is the same order, not a second card.
    let (_, _, again) = rpc(&app, call).await;
    assert_eq!(again["result"]["structuredContent"]["id"], id.as_str());
    assert_eq!(held.cards.lock().unwrap().len(), 1);

    let (status, polled) = get_json(&app, &format!("/awp/v2/calls/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(polled["state"], "pending");

    held.approve(&id);
    let (_, polled) = get_json(&app, &format!("/awp/v2/calls/{id}")).await;
    assert_eq!(polled["state"], "completed");
    assert_eq!(polled["result"]["reference"]["id"], "K-1043");

    let written = receipts.0.lock().unwrap();
    assert!(
        written
            .iter()
            .any(|r| r.kind == "door.order-held" && r.subject == format!("call:{id}"))
    );
    assert!(
        !written
            .iter()
            .any(|r| r.observed.to_string().contains("njeri@example.com")),
        "receipts carry no personal details"
    );
}

#[tokio::test]
async fn a_bad_order_is_refused_in_plain_words_and_nothing_is_held() {
    let (app, held, _) = setup().await;
    let mut args = order_args();
    args["date"] = json!("14/11/2099");
    let (_, _, reply) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "request_custom_order", "arguments": args}}),
    )
    .await;
    assert_eq!(reply["result"]["isError"], true);
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("YYYY-MM-DD")
    );
    assert!(held.cards.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ask_site_over_mcp_returns_a_cited_answer() {
    let (app, _, _) = setup().await;
    let (_, _, reply) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "ask_site", "arguments": {"question": "Do you deliver to Kilimani on Saturday?"}}}),
    )
    .await;
    let s = &reply["result"]["structuredContent"];
    assert_eq!(s["answer"]["kind"], "answer", "{reply}");
    assert_eq!(s["sources"][0]["path"], "/delivery/");
}

#[tokio::test]
async fn unknown_methods_and_tools_are_errors() {
    let (app, _, _) = setup().await;
    let (_, _, reply) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list"}),
    )
    .await;
    assert_eq!(reply["error"]["code"], -32601);
    let (_, _, reply) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "pay"}}),
    )
    .await;
    assert_eq!(reply["error"]["code"], -32602);
}

#[test]
fn order_validation() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
    assert!(validate_order(&order_args(), today).is_ok());
    let mut past = order_args();
    past["date"] = json!("2026-11-01");
    assert_eq!(
        validate_order(&past, today).unwrap_err(),
        "date is in the past."
    );
    let mut extra = order_args();
    extra["price"] = json!(1);
    assert!(
        validate_order(&extra, today)
            .unwrap_err()
            .contains("Unknown field")
    );
    let mut email = order_args();
    email["email"] = json!("not-an-email");
    assert!(validate_order(&email, today).is_err());
}

#[tokio::test]
async fn with_the_concierge_off_duty_no_order_is_held() {
    let site = Site::new().await;
    site.release(1, "Sourdough and cakes").await;
    let (state, _) = site.state();
    let state = state.with_guards(crate::guards::testing::team("site-kilele", &["editor"]).await);
    let held = Arc::new(Held::default());
    let app = crate::site::router(state.clone()).merge(router(DoorState {
        site: state,
        held: held.clone(),
    }));
    let (status, _, reply) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "request_custom_order", "arguments": order_args()}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(reply["result"]["content"][0]["text"], NOT_TAKING_ORDERS);
    assert!(held.cards.lock().unwrap().is_empty(), "no card raised");
}

#[tokio::test]
async fn while_frozen_the_door_takes_no_calls() {
    let site = Site::new().await;
    site.release(1, "Sourdough and cakes").await;
    let (state, _) = site.state();
    let state = state.with_guards(crate::guards::testing::frozen_alone("site-kilele").await);
    let held = Arc::new(Held::default());
    let app = crate::site::router(state.clone()).merge(router(DoorState {
        site: state,
        held: held.clone(),
    }));
    for (name, arguments) in [
        ("request_custom_order", order_args()),
        (
            "ask_site",
            json!({"question": "Do you deliver to Kilimani?"}),
        ),
    ] {
        let (_, _, reply) = rpc(
            &app,
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                   "params": {"name": name, "arguments": arguments}}),
        )
        .await;
        assert_eq!(reply["result"]["isError"], true, "{name}");
        assert_eq!(reply["result"]["content"][0]["text"], PAUSED, "{name}");
    }
    assert!(held.cards.lock().unwrap().is_empty());
    let (status, _) = get_json(&app, "/.well-known/awp").await;
    assert_eq!(status, StatusCode::OK, "discovery still answers");
}

#[tokio::test]
async fn under_a_platform_hold_the_door_takes_no_calls() {
    let site = Site::new().await;
    site.release(1, "Sourdough and cakes").await;
    let (state, _) = site.state();
    let state = state.with_guards(crate::guards::testing::held("site-kilele", "read-only").await);
    let held = Arc::new(Held::default());
    let app = crate::site::router(state.clone()).merge(router(DoorState {
        site: state,
        held: held.clone(),
    }));
    let (_, _, reply) = rpc(
        &app,
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
               "params": {"name": "request_custom_order", "arguments": order_args()}}),
    )
    .await;
    assert_eq!(reply["result"]["content"][0]["text"], PAUSED);
    assert!(held.cards.lock().unwrap().is_empty());
}
