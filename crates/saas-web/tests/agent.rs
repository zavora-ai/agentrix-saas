//! The board's agent end to end: adk-server's `/api/run_sse` with `uiProtocol: "ag_ui"`, as the
//! swapped-in AG Studio harness calls it, streams AG-UI events built from the Treasurer.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use saas_web::Face;
use saas_web::hosts::Hosts;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app() -> Router {
    Face::demo(Hosts::default())
        .with_today(Arc::new(|| "2026-11-10".parse().unwrap()))
        .with_board_dist(None)
        .router()
        .unwrap()
}

async fn post(app: &Router, path: &str, origin: Option<&str>, body: Value) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::HOST, "money.localhost:18183")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(o) = origin {
        req = req.header(header::ORIGIN, o);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The `data:` payloads of an SSE body, parsed.
fn events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str(d.trim()).ok())
        .collect()
}

#[tokio::test]
async fn the_treasurer_streams_ag_ui_events_through_adk_server() {
    let app = app().await;
    let (status, body) = post(
        &app,
        "/board/agent/api/apps/treasurer/users/owner/sessions/thread-1",
        Some("http://money.localhost:18183"),
        json!({}),
    )
    .await;
    assert!(status.is_success(), "creating the session: {status} {body}");

    let (status, body) = post(
        &app,
        "/board/agent/api/run_sse",
        Some("http://money.localhost:18183"),
        json!({
            "appName": "treasurer",
            "userId": "owner",
            "sessionId": "thread-1",
            "newMessage": {"role": "user", "parts": [{"text": "How is the Kilele instalment plan going?"}]},
            "streaming": true,
            "uiProtocol": "ag_ui",
            "uiTransport": "protocol_native",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let evs = events(&body);
    let kinds: Vec<&str> = evs.iter().filter_map(|e| e["type"].as_str()).collect();
    for want in [
        "RUN_STARTED",
        "TOOL_CALL_START",
        "TOOL_CALL_ARGS",
        "TOOL_CALL_END",
        "TOOL_CALL_RESULT",
        "TEXT_MESSAGE_CHUNK",
        "TEXT_MESSAGE_END",
        "RUN_FINISHED",
    ] {
        assert!(kinds.contains(&want), "no {want} in {kinds:?}\n{body}");
    }
    let tool = evs.iter().find(|e| e["type"] == "TOOL_CALL_START").unwrap();
    assert_eq!(tool["toolCallName"], "settle.debt_plans");
    let text: String = evs
        .iter()
        .filter(|e| e["type"] == "TEXT_MESSAGE_CONTENT" || e["type"] == "TEXT_MESSAGE_CHUNK")
        .filter_map(|e| e["delta"].as_str())
        .collect();
    assert!(
        text.contains("Kilele Bakery owes Agentrix $505.60 on INV-1042"),
        "{text}"
    );
    assert_eq!(
        text.matches("Kilele Bakery owes").count(),
        1,
        "the final event doesn't repeat the streamed text: {text}"
    );
}

#[tokio::test]
async fn other_origins_cannot_drive_the_agent() {
    let app = app().await;
    let (status, _) = post(
        &app,
        "/board/agent/api/apps/treasurer/users/owner/sessions/thread-2",
        Some("https://evil.example"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
