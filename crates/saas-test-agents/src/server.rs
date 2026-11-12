//! The agents' hosts, by `Host`:
//!
//! - `test-agent-<slug>.<platform host>`: the agent's labelled page, its `did.json`, its Web Bot
//!   Auth key directory, and `/paypal/save` (a person approves the agent's sandbox buyer once).
//! - `test-agents.<platform host>`: what the agents did lately, whether they are on, and
//!   `POST /control/stop` and `/control/start` (bearer `TEST_AGENTS_CONTROL_TOKEN`).

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::json;
use tower::ServiceExt as _;

use crate::config::LABEL;
use crate::identity::Agent;
use crate::runner::Shared;

/// The whole app: one router per agent host and one for the control host.
pub fn app(shared: Arc<Shared>, agents: &[Agent]) -> Router {
    let mut hosts: Vec<(String, Router)> = agents
        .iter()
        .map(|a| {
            (
                a.host.to_ascii_lowercase(),
                agent_router(shared.clone(), a.clone()),
            )
        })
        .collect();
    hosts.push((
        shared.config.control_host().to_ascii_lowercase(),
        control_router(shared.clone(), agents.to_vec()),
    ));
    let hosts = Arc::new(hosts);
    Router::new().fallback(move |request: Request| {
        let hosts = hosts.clone();
        async move {
            let host = request
                .headers()
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let bare = host.split(':').next().unwrap_or_default().to_owned();
            match hosts
                .iter()
                .find(|(h, _)| *h == host || h.split(':').next() == Some(bare.as_str()))
            {
                Some((_, router)) => router
                    .clone()
                    .oneshot(request)
                    .await
                    .unwrap_or_else(|e| match e {}),
                None => (StatusCode::NOT_FOUND, "Not a test agent's host.").into_response(),
            }
        }
    })
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><meta name=robots content=noindex>\
         <title>{title}</title><style>body{{font:16px/1.5 system-ui,sans-serif;max-width:46rem;margin:2rem auto;padding:0 1rem;color:#1d1d1f}}\
         .label{{display:inline-block;background:#fff3cd;border:1px solid #e0c36b;border-radius:4px;padding:.1rem .5rem;font-weight:600}}\
         table{{border-collapse:collapse;width:100%;font-size:14px}}td,th{{border-bottom:1px solid #ddd;padding:.3rem;text-align:left;vertical-align:top}}\
         code{{font-size:13px}}</style><p class=label>{LABEL}</p>{body}</html>"
    ))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn agent_router(shared: Arc<Shared>, agent: Agent) -> Router {
    let did = agent.did_document().unwrap_or_default();
    let a = agent.clone();
    let directory = agent.keys.directory_router();
    Router::new()
        .route(
            "/",
            get(move || {
                let a = a.clone();
                async move {
                    page(
                        &a.display(),
                        &format!(
                            "<h1>{name}</h1><p>A scripted test shopper Agentrix runs against its sample sites, on sandbox PayPal only. Not a person and not a real customer.</p>\
                             <p>Its DID is <code>{did}</code>; it signs every request that needs a known agent with Web Bot Auth, from <a href=\"/.well-known/http-message-signatures-directory\">its key directory</a>.</p>",
                            name = escape(&a.display()),
                            did = escape(&a.did()),
                        ),
                    )
                }
            }),
        )
        .route(
            "/.well-known/did.json",
            get(move || {
                let did = did.clone();
                async move { ([(header::CONTENT_TYPE, "application/did+json")], did.to_string()) }
            }),
        )
        .route("/paypal/save", get(save))
        .route("/paypal/returned", get(returned))
        .route("/paypal/cancelled", get(|| async { page("Not saved", "<p>Nothing was saved.</p>") }))
        .with_state((shared, agent))
        .merge(directory)
}

async fn save(State((shared, agent)): State<(Arc<Shared>, Agent)>) -> Response {
    let Some(buyers) = &shared.buyers else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "No sandbox PayPal app is configured.",
        )
            .into_response();
    };
    match buyers.start(&agent).await {
        Ok(url) => Redirect::to(&url).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("PayPal didn't start: {e}")).into_response(),
    }
}

#[derive(Deserialize)]
struct Back {
    approval_token_id: Option<String>,
    approval_session_id: Option<String>,
}

async fn returned(
    State((shared, agent)): State<(Arc<Shared>, Agent)>,
    Query(q): Query<Back>,
) -> Response {
    let Some(buyers) = &shared.buyers else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Some(token) = q
        .approval_token_id
        .or(q.approval_session_id)
        .filter(|t| !t.is_empty() && t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric()))
    else {
        return (
            StatusCode::BAD_REQUEST,
            "This link didn't come from PayPal's approval.",
        )
            .into_response();
    };
    match buyers.finish(&agent, &token).await {
        Ok(id) => {
            shared.log.record(
                &agent.slug,
                "",
                "pay",
                true,
                format!("saved a sandbox buyer (…{})", tail(&id)),
            );
            page(
                "Saved",
                &format!(
                    "<p>{} now pays sample-site orders in sandbox with this buyer (…{}).</p>",
                    escape(&agent.display()),
                    escape(&tail(&id))
                ),
            )
            .into_response()
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("Not saved: {e}")).into_response(),
    }
}

fn tail(id: &str) -> String {
    id.chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn control_router(shared: Arc<Shared>, agents: Vec<Agent>) -> Router {
    let agents = Arc::new(agents);
    Router::new()
        .route("/", get(control_page))
        .route("/control/status", get(status))
        .route("/control/stop", post(stop))
        .route("/control/start", post(start))
        .with_state((shared, agents))
}

type Control = State<(Arc<Shared>, Arc<Vec<Agent>>)>;

fn status_json(shared: &Shared, agents: &[Agent]) -> serde_json::Value {
    json!({
        "label": LABEL,
        "on": !shared.config.is_off(),
        "started": shared.started,
        "ends": shared.started + shared.config.run_for,
        "agents": agents.iter().map(|a| json!({
            "slug": a.slug, "name": a.display(), "did": a.did(), "host": a.host,
            "model_usd_today": shared.brain.spent_today(&a.slug),
            "buyer_saved": shared.buyers.as_ref().and_then(|b| b.vault_id(a)).is_some(),
        })).collect::<Vec<_>>(),
        "recent": shared.log.recent(50),
    })
}

async fn status(State((shared, agents)): Control) -> Response {
    axum::Json(status_json(&shared, &agents)).into_response()
}

async fn control_page(State((shared, agents)): Control) -> Html<String> {
    let s = status_json(&shared, &agents);
    let mut rows = String::new();
    for a in shared.log.recent(60) {
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            a.at.format("%H:%M:%S"),
            escape(&a.agent),
            escape(
                a.site
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
            ),
            escape(&a.kind),
            if a.ok {
                escape(&a.detail)
            } else {
                format!("<b>{}</b>", escape(&a.detail))
            },
        ));
    }
    let who: Vec<String> = agents
        .iter()
        .map(|a| {
            format!(
                "<li><a href=\"{}/\">{}</a>: <code>{}</code></li>",
                escape(&a.origin),
                escape(&a.display()),
                escape(&a.did())
            )
        })
        .collect();
    page(
        "Test customer agents",
        &format!(
            "<h1>Test customer agents</h1><p>Three scripted test shoppers on the sample sites, sandbox PayPal only. They are <b>{}</b>; the run ends {}.</p><ul>{}</ul>\
             <table><tr><th>UTC</th><th>Agent</th><th>Site</th><th>What</th><th>Detail</th></tr>{rows}</table>",
            if s["on"] == true { "on" } else { "stopped" },
            escape(s["ends"].as_str().unwrap_or("")),
            who.join(""),
        ),
    )
}

fn authorised(shared: &Shared, headers: &HeaderMap) -> bool {
    let Some(token) = &shared.config.control_token else {
        return false;
    };
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| {
            given.len() == token.len()
                && given
                    .bytes()
                    .zip(token.bytes())
                    .fold(0, |acc, (a, b)| acc | (a ^ b))
                    == 0
        })
}

async fn stop(State((shared, agents)): Control, headers: HeaderMap) -> Response {
    if !authorised(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(dir) = shared.config.off_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(
        &shared.config.off_file,
        b"stopped from the control endpoint\n",
    ) {
        Ok(()) => {
            shared
                .log
                .record("all", "", "stopped", true, "stopped by the operator");
            axum::Json(status_json(&shared, &agents)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn start(State((shared, agents)): Control, headers: HeaderMap) -> Response {
    if !authorised(&shared, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let _ = std::fs::remove_file(&shared.config.off_file);
    shared
        .log
        .record("all", "", "started", true, "started by the operator");
    axum::Json(status_json(&shared, &agents)).into_response()
}
