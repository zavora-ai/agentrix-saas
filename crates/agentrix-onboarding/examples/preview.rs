//! A local preview of the onboarding pages beside their concepts, for comparing in a browser:
//! `cargo run -p agentrix-onboarding --example preview` → http://localhost:18400/.
//!
//! The views are the test views in `tests/common`; `/assess/running/events` plays a short
//! made-up run in the shape of Assess's real events (design §4.3a). Development only: none of
//! this data reaches a live page.

#[path = "../tests/common/mod.rs"]
mod common;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use agentrix_onboarding::{Pages, view::AssessPage};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use futures::stream::{self, Stream};
use serde_json::json;

const CONCEPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../concept");

fn page(pages: &Pages, id: &str) -> Option<String> {
    let mut v: AssessPage = match id {
        "running" => common::running(),
        "frame" => common::running_with_frame(),
        "done" => common::done(),
        "unreachable" => common::unreachable(),
        "refused" => common::refused(),
        _ => return None,
    };
    v.id = id.into();
    v.report_path = format!("/assess/{id}");
    v.events_path = format!("/assess/{id}/events");
    pages.render_assess(&v).ok()
}

async fn index() -> Html<&'static str> {
    Html(
        "<!doctype html><meta charset=utf-8><title>W4 previews</title><style>body{font:16px/1.6 system-ui;margin:40px}</style>\
         <h1>W4 onboarding previews</h1><ul>\
         <li><a href=/assess>Start (GET /assess)</a> · <a href=/start-error>with an error</a></li>\
         <li><a href=/assess/running>Assess: reading (plays a run)</a> · <a href=/assess/frame>with a frame</a></li>\
         <li><a href=/assess/done>Assess: report and keep it</a></li>\
         <li><a href=/assess/unreachable>Assess: unreachable</a> · <a href=/assess/refused>refused</a></li>\
         <li><a href=/account/new?next=/claim/dns&host=kilele-bakery.example>Account: create</a> · <a href=/account/signin>sign in</a> · <a href=/account/recover>recover</a> · <a href=/setup-preview>setup (Chief)</a></li>\
         <li>The Move: <a href=/move/running>running</a> · <a href=/move/waiting>waiting to go live</a> · <a href=/move/live>live</a> · <a href=/move/stopped>stopped</a> · <a href=/move/none>not started</a> · <a href=/concept/move.html>concept</a></li>\
         <li><a href=/plan-preview>Plan (W3's catalogue values)</a></li>\
         <li><a href=/claim/dns>Claim: DNS</a> · <a href=/claim/file>file, after a failed check</a> · <a href=/claim/verified>proved</a></li>\
         <li><a href=/concept/assess.html>Concept: assess.html</a> · <a href=/concept/index.html>index.html</a></li></ul>",
    )
}

fn html(body: Option<String>) -> Response {
    match body {
        Some(b) => Html(b).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn concept(Path(path): Path<String>) -> Response {
    if path.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = format!("{CONCEPT}/{path}");
    let kind = match path.rsplit('.').next() {
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("svg") => "image/svg+xml",
        _ => "text/html; charset=utf-8",
    };
    match tokio::fs::read(file).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, kind)], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A frame placeholder: a drawn page, clearly a preview.
async fn frame(Path(digest): Path<String>) -> Response {
    let n = digest.len() % 5;
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1280 800'><rect width='1280' height='800' fill='#f0efe9'/>\
         <rect x='40' y='30' width='1200' height='170' fill='#c9c3b4'/><text x='640' y='130' font-size='40' text-anchor='middle' fill='#6c6555' font-family='Georgia'>Preview frame {n}</text>\
         <rect x='40' y='230' width='760' height='24' fill='#d8d3c6'/><rect x='40' y='274' width='700' height='24' fill='#d8d3c6'/><rect x='40' y='318' width='620' height='24' fill='#d8d3c6'/>\
         <rect x='860' y='230' width='380' height='420' fill='#e1ddd2' stroke='#b9b2a2' stroke-dasharray='8'/></svg>"
    );
    ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
}

fn events_for(id: &str) -> Vec<(u64, &'static str, serde_json::Value)> {
    let at = "2026-11-08T06:02:00Z";
    let paths = [
        "/",
        "/menu",
        "/cakes",
        "/order",
        "/delivery",
        "/about",
        "/visit",
        "/blog/summer-offers",
        "/blog/sourdough-101",
        "/contact",
        "/old-specials",
        "/gallery",
    ];
    let of = paths.len() as u32;
    let mut out = Vec::new();
    let mut seq = 0;
    let mut push = |name: &'static str, data: serde_json::Value| {
        seq += 1;
        out.push((seq, name, data));
    };
    let (mut req, mut bytes, mut refused) = (0u64, 0u64, 0u64);
    for (i, p) in paths.iter().enumerate() {
        req += 2;
        bytes += 38_000 + (i as u64 * 7_300);
        if *p == "/old-specials" {
            refused += 1;
        }
        push(
            "receipt-tick",
            json!({"hosts": 1, "requests": req, "bytes": bytes, "refusals": refused, "at": at}),
        );
        if i % 3 == 0 {
            push(
                "frame",
                json!({"kind": "sealed-screenshot", "page_url": format!("https://fixture.example{p}"), "digest": format!("sha256-{i}"), "src": format!("/frames/sha256-{i}"), "w": 1280, "h": 800, "at": at}),
            );
        }
        let (status, reason) = if *p == "/gallery" {
            ("failed", Some("HTTP 404"))
        } else {
            ("read", None)
        };
        push(
            "page",
            json!({"n": i as u32 + 1, "of": of, "url": format!("https://fixture.example{p}"), "status": status, "reason": reason, "at": at}),
        );
        if *p == "/blog/summer-offers" {
            push(
                "finding",
                json!({"id": "f1", "severity": "urgent", "category": "security", "title": "Hidden links you didn't put there", "where": p, "at": at}),
            );
        }
    }
    push(
        "done",
        json!({"outcome": "report-ready", "href": if id == "running" { Some("/assess/done") } else { None }, "at": at}),
    );
    out
}

async fn events(Path(id): Path<String>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let delay = if id == "running" || id == "frame" {
        650
    } else {
        0
    };
    let list = events_for(&id);
    let s = stream::unfold(list.into_iter(), move |mut it| async move {
        let (seq, name, data) = it.next()?;
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        let ev = Event::default()
            .id(seq.to_string())
            .event(name)
            .data(data.to_string());
        Some((Ok(ev), it))
    });
    Sse::new(s)
}

fn step(n: u32, title: &str, words: &str, done: bool) -> agentrix_onboarding::view::SetupStep {
    agentrix_onboarding::view::SetupStep {
        n,
        title: title.into(),
        words: words.into(),
        done,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pages = Arc::new(Pages::new()?);
    let app =
        Router::new()
            .route("/", get(index))
            .route(
                "/assess",
                get(|State(p): State<Arc<Pages>>| async move {
                    html(p.render_start(&common::start()).ok())
                }),
            )
            .route(
                "/start-error",
                get(|State(p): State<Arc<Pages>>| async move {
                    html(p.render_start(&common::start_error()).ok())
                }),
            )
            .route(
                "/assess/{id}",
                get(
                    |State(p): State<Arc<Pages>>, Path(id): Path<String>| async move {
                        html(page(&p, &id))
                    },
                ),
            )
            .route("/assess/{id}/events", get(events))
            .route(
                "/claim/{state}",
                get(
                    |State(p): State<Arc<Pages>>, Path(state): Path<String>| async move {
                        let view = match state.as_str() {
                            "dns" => common::claim_dns(),
                            "file" => common::claim_file_failed(),
                            "verified" => common::claim_verified(),
                            _ => return StatusCode::NOT_FOUND.into_response(),
                        };
                        html(p.render_claim(&view).ok())
                    },
                ),
            )
            .route(
                "/move/{state}",
                get(
                    |State(p): State<Arc<Pages>>, Path(state): Path<String>| async move {
                        use agentrix_onboarding::view::MoveState as M;
                        let s = match state.as_str() {
                            "running" => M::Running,
                            "waiting" => M::Waiting,
                            "live" => M::Live,
                            "stopped" => M::Stopped,
                            "none" => M::None,
                            _ => return StatusCode::NOT_FOUND.into_response(),
                        };
                        html(p.render_move(&common::move_view(s)).ok())
                    },
                ),
            )
            .route(
                "/plan-preview",
                get(|State(p): State<Arc<Pages>>| async move {
                    use agentrix_onboarding::view::{PlanInput, PlanPage};
                    let plan = |id: &str, name: &str, cents: i64, ai: i64, agents: &[&str], inc: &[&str]| PlanInput {
                        id: id.into(),
                        name: name.into(),
                        kind: "site".into(),
                        price_cents: cents,
                        annual_cents: agentrix_billing::annual_cents(cents, agentrix_billing::catalogue().annual_discount_percent),
                        included_ai_micro_usd: ai * 1_000_000,
                        webagents: agents.iter().map(|a| a.to_string()).collect(),
                        includes: inc.iter().map(|a| a.to_string()).collect(),
                    };
                    let plans = [
                        plan("essential", "Essential", 2900, 5, &["Chief", "Concierge", "Editor", "Guardian", "Caretaker"], &["Your site on Agentrix", "The morning brief", "$5 of AI a month"]),
                        plan("business", "Business", 7900, 15, &["Chief", "Concierge", "Editor", "Guardian", "Caretaker", "Sales", "Reputation"], &["Everything in Essential", "Bookings and forms", "WhatsApp channel", "$15 of AI a month"]),
                        plan("commerce", "Commerce", 14900, 30, &["Chief", "Concierge", "Editor", "Guardian", "Caretaker", "Sales", "Reputation", "Cashier"], &["Everything in Business", "Checkout and orders", "Agent-to-agent commerce", "$30 of AI a month"]),
                    ];
                    let view = PlanPage::new("site_0193test", Some("kilele-bakery.example".into()), &plans, "/claim/dns");
                    html(p.render(agentrix_onboarding::OnboardingPage::Plan, &view).ok())
                }),
            )
            .route("/frames/{digest}", get(frame))
            .route("/concept/{*path}", get(concept))
            .route(
                "/setup-preview",
                get(|State(p): State<Arc<Pages>>| async move {
                    let view = agentrix_onboarding::view::SetupPage {
                        host: Some("kilele-bakery.example".into()),
                        steps: vec![
                            step(1, "Your account", "Made, and signed in on this device.", true),
                            step(2, "Prove kilele-bakery.example is yours", "One DNS record or one file. The Caretaker checks it from the sandbox.", false),
                            step(3, "The Move", "The Mover copies your site, leaves behind anything infected, rebuilds it and checks every page against the old one.", false),
                            step(4, "Go live", "One card that says exactly what going live does. You approve it with your passkey or a code from your authenticator app, and undo is one tap for 30 days.", false),
                        ],
                        chief_words: "Next, prove kilele-bakery.example is yours, and the Mover will start.".into(),
                        next_href: "/claim/dns".into(),
                        next_words: "Next: prove kilele-bakery.example is yours".into(),
                    };
                    html(p.render(agentrix_onboarding::OnboardingPage::Setup, &view).ok())
                }),
            )
            .with_state(pages)
            .merge(agentrix_onboarding::mount(&agentrix_host::testing::context().await?)?.platform);
    let addr = std::env::var("PREVIEW_LISTEN").unwrap_or_else(|_| "127.0.0.1:18400".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!(
        "preview on http://{}/",
        addr.replace("127.0.0.1", "localhost")
    );
    axum::serve(listener, app).await?;
    Ok(())
}
