//! Pass a question to the owner (R8.3): `POST /ask/pass` on a site's host.
//!
//! When Ask can't answer, it offers to pass the question on. The visitor sends
//! `{question, contact?}`. The question is at most 500 characters and the contact (how the owner
//! can reply) at most 120, both stripped of control characters. Each client IP may pass at most
//! [`PER_HOUR`] questions an hour. The question is stored as data through a [`QuestionSink`]
//! and never reaches a model. The reply is plain words.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{Extensions, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{SiteState, host_of};

/// The longest question, in characters.
pub const MAX_QUESTION: usize = 500;
/// The longest contact, in characters.
pub const MAX_CONTACT: usize = 120;
/// Questions one client IP may pass in an hour.
pub const PER_HOUR: usize = 5;

/// What the visitor sees once the question is stored.
pub const SENT: &str = "Sent to the owner. They can only reply if you left a way to reach you.";

/// A question a visitor passed to the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassedQuestion {
    /// The site id.
    pub site: String,
    /// The live release the visitor was reading.
    pub release_seq: u64,
    /// The question, as data.
    pub question: String,
    /// How the owner can reply, if the visitor left one.
    pub contact: Option<String>,
    /// When it was passed.
    pub at: DateTime<Utc>,
}

/// Where passed questions go: the `passed_questions` table (`db::questions::QuestionStore`).
#[async_trait]
pub trait QuestionSink: Send + Sync {
    /// Stores one question.
    async fn pass(&self, question: PassedQuestion) -> anyhow::Result<()>;
}

/// The sink before one is wired: it refuses, so a question is never silently dropped.
pub struct Unwired;

#[async_trait]
impl QuestionSink for Unwired {
    async fn pass(&self, _question: PassedQuestion) -> anyhow::Result<()> {
        anyhow::bail!("no question store is wired")
    }
}

/// Questions passed per client in the last hour.
#[derive(Default)]
pub struct Limiter {
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Limiter {
    /// Records a pass by `client` at `now`, unless it already passed [`PER_HOUR`] in the hour.
    pub fn allow(&self, client: &str, now: Instant) -> bool {
        let hour = Duration::from_secs(3600);
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if hits.len() > 10_000 {
            hits.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < hour));
        }
        let q = hits.entry(client.to_owned()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= hour) {
            q.pop_front();
        }
        if q.len() >= PER_HOUR {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// The request body.
#[derive(Debug, Deserialize)]
pub struct PassRequest {
    /// The question.
    pub question: String,
    /// How the owner can reply, optional.
    #[serde(default)]
    pub contact: Option<String>,
}

fn said(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "message": message }))).into_response()
}

/// Text without control characters (newlines become spaces), trimmed.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The client as the edge saw it: the last `X-Forwarded-For` entry (Caddy appends the peer it
/// accepted), else the TCP peer.
fn client(headers: &HeaderMap, extensions: &Extensions) -> String {
    if let Some(ip) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
    {
        return ip.to_owned();
    }
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| "unknown".to_owned(), |c| c.0.ip().to_string())
}

/// `POST /ask/pass`.
pub(super) async fn pass_handler(
    State(state): State<SiteState>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(body): Json<PassRequest>,
) -> Response {
    let Some(host) = host_of(&headers) else {
        return said(StatusCode::BAD_REQUEST, "No Host.");
    };
    let question = clean(&body.question);
    if question.is_empty() {
        return said(
            StatusCode::BAD_REQUEST,
            "Write the question you'd like to pass on.",
        );
    }
    if question.chars().count() > MAX_QUESTION {
        return said(
            StatusCode::BAD_REQUEST,
            "Keep the question to 500 characters or fewer.",
        );
    }
    let contact = body.contact.as_deref().map(clean).filter(|c| !c.is_empty());
    if contact
        .as_ref()
        .is_some_and(|c| c.chars().count() > MAX_CONTACT)
    {
        return said(
            StatusCode::BAD_REQUEST,
            "Keep your phone number or email to 120 characters or fewer.",
        );
    }
    let release = match state.releases.current(&host).await {
        Ok(Some(r)) => r,
        _ => {
            return said(
                StatusCode::SERVICE_UNAVAILABLE,
                "This site isn't available right now.",
            );
        }
    };
    // A platform read-only hold pauses forms too (W3, handover-wiring 3.2).
    if state.guards.held(&release.pointer.site_id).await {
        return said(StatusCode::SERVICE_UNAVAILABLE, super::HELD);
    }
    if !state
        .pass_limit
        .allow(&client(&headers, &extensions), Instant::now())
    {
        return said(
            StatusCode::TOO_MANY_REQUESTS,
            "You've passed 5 questions in the last hour. Try again later, or use the contact page.",
        );
    }
    let passed = PassedQuestion {
        site: release.pointer.site_id.clone(),
        release_seq: release.pointer.seq,
        question,
        contact,
        at: Utc::now(),
    };
    match state.questions.pass(passed).await {
        Ok(()) => said(StatusCode::OK, SENT),
        Err(e) => {
            tracing::error!(%e, "a passed question could not be stored");
            said(
                StatusCode::SERVICE_UNAVAILABLE,
                "Your question didn't reach the owner. Please use the contact page.",
            )
        }
    }
}
