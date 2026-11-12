//! Sites by Host: verified release serving, Ask this site over SSE, and Markdown twins (R8).
//!
//! Lane: `experience` (specs/first-release/design.md §3.0, §4.1, §4.5).
//!
//! Two routers:
//! - [`router`] serves every path of a site host: the signed release, `POST /ask`,
//!   `POST /ask/pass` (a question passed to the owner, R8.3; see [`pass`]), and the platform's
//!   own island assets under `/_agentrix/`. Mount it behind Host dispatch: requests
//!   whose `Host` is a site the platform serves ([`serves_host`]) go here.
//! - [`internal_router`] holds `GET /internal/tls-ask?domain=` for Caddy's on-demand TLS. Mount
//!   it on the platform router; it answers only loopback requests that did not come through
//!   the proxy.
//!
//! The release HTML is served byte for byte as signed, so `awp verify` can compare it; the
//! converted-site theme (`agentrix-console/templates/site/`) links the island itself.

pub mod ask;
pub mod gateway;
pub mod pass;
pub mod policy;
pub mod release;

use std::net::SocketAddr;
use std::sync::Arc;

use agentrix_kernel::{Action, Effect};
use agentrix_operator::holds::Mode;
use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::Stream;
use serde::{Deserialize, Serialize};

pub use ask::AskModel;
pub use gateway::GatewayModel;
pub use pass::{PassedQuestion, QuestionSink};
pub use release::{LivePointer, LoadedRelease, ReleaseCache, ReleaseSource};

/// A receipt to write through the data lane's receipts store (design §4.4). The store adds the
/// id, issuer, times and signature.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReceiptDraft {
    /// The receipt kind, such as `concierge.refusal`.
    pub kind: &'static str,
    /// The site id.
    pub site: String,
    /// What it is about, such as `release:12`.
    pub subject: String,
    /// What was observed.
    pub observed: serde_json::Value,
}

/// Where owner-facing signals go: the receipts store. The Trust tab lists them by kind.
#[async_trait]
pub trait ReceiptSink: Send + Sync {
    /// Writes a receipt and returns its id.
    async fn write(&self, draft: ReceiptDraft) -> anyhow::Result<String>;
}

/// What the site router needs.
#[derive(Clone)]
pub struct SiteState {
    /// Verified releases by host.
    pub releases: Arc<ReleaseCache>,
    /// The Model Gateway for Ask.
    pub model: Arc<dyn AskModel>,
    /// The receipts store.
    pub receipts: Arc<dyn ReceiptSink>,
    /// Where questions passed to the owner go (R8.3). Refuses until wired with
    /// [`SiteState::with_questions`].
    pub questions: Arc<dyn QuestionSink>,
    /// Questions passed per client IP in the last hour.
    pub pass_limit: Arc<pass::Limiter>,
    /// The platform's checks before Ask and the door act (dials, Freeze, holds). None until
    /// wired with [`SiteState::with_guards`].
    pub guards: crate::guards::Guards,
}

impl SiteState {
    /// A state over a release source, a model and a receipts store.
    pub fn new(
        source: Arc<dyn ReleaseSource>,
        model: Arc<dyn AskModel>,
        receipts: Arc<dyn ReceiptSink>,
    ) -> Self {
        Self {
            releases: Arc::new(ReleaseCache::new(source)),
            model,
            receipts,
            questions: Arc::new(pass::Unwired),
            pass_limit: Arc::new(pass::Limiter::default()),
            guards: crate::guards::Guards::default(),
        }
    }

    /// Checks the platform's guards (`crate::guards`) before Ask and the door act.
    pub fn with_guards(mut self, guards: crate::guards::Guards) -> Self {
        self.guards = guards;
        self
    }

    /// Stores questions visitors pass to the owner in `sink` (`db::questions::QuestionStore`).
    pub fn with_questions(mut self, sink: Arc<dyn QuestionSink>) -> Self {
        self.questions = sink;
        self
    }
}

/// The router for site hosts.
pub fn router(state: SiteState) -> Router {
    Router::new()
        .route("/ask", post(ask_handler))
        .route("/ask/pass", post(pass::pass_handler))
        .route("/_agentrix/{file}", get(asset))
        .fallback(serve)
        .with_state(state)
}

/// The loopback-only router for Caddy's on-demand TLS check.
pub fn internal_router(state: SiteState) -> Router {
    Router::new()
        .route("/internal/tls-ask", get(tls_ask))
        .with_state(state)
}

/// Whether the platform serves a site at `host` (for the Host dispatcher).
///
/// # Errors
///
/// When the release source fails.
pub async fn serves_host(state: &SiteState, host: &str) -> anyhow::Result<bool> {
    Ok(state
        .releases
        .source()
        .live(&normalise_host(host))
        .await?
        .is_some())
}

/// `Kilele-Bakery.example:443` → `kilele-bakery.example`.
pub fn normalise_host(host: &str) -> String {
    let host = host.trim().trim_end_matches('.');
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or_default().to_owned()
    };
    host.to_ascii_lowercase()
}

fn host_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(normalise_host)
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

const UNAVAILABLE: &str = "This site isn't available right now.";

fn wants_markdown(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|a| a.to_str().ok())
        .is_some_and(|a| {
            let md = a.find("text/markdown");
            let html = a.find("text/html");
            matches!((md, html), (Some(m), Some(h)) if m < h) || (md.is_some() && html.is_none())
        })
}

fn cache_control(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".md") {
        "public, max-age=60"
    } else {
        "public, max-age=3600"
    }
}

async fn serve(State(state): State<SiteState>, request: Request) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed.");
    }
    let headers = request.headers().clone();
    let Some(host) = host_of(&headers) else {
        return plain(StatusCode::BAD_REQUEST, "No Host.");
    };
    let release = match state.releases.current(&host).await {
        Ok(Some(r)) => r,
        Ok(None) => return plain(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE),
        Err(e) => {
            tracing::error!(%e, host, "release source failed");
            return plain(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE);
        }
    };
    let path = request.uri().path();
    if let Some(r) = release.redirect(path) {
        let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::MOVED_PERMANENTLY);
        return (status, [(header::LOCATION, r.to.clone())]).into_response();
    }
    let Some((file, entry)) = release.resolve(path) else {
        return not_found(&state, &release).await;
    };
    // Markdown twins for agents (R8.5): same URL, `Accept: text/markdown`.
    let twin = if file.ends_with(".html") {
        release.twin_of(file)
    } else {
        None
    };
    let (file, entry) = match twin {
        Some(t) if wants_markdown(&headers) => t,
        _ => (file, entry),
    };
    let etag = format!("\"{}\"", entry.sha256.hex());
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let bytes = match state.releases.file(entry).await {
        Ok(Some(b)) => b,
        Ok(None) => return plain(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE),
        Err(e) => {
            tracing::error!(%e, file, "file read failed");
            return plain(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE);
        }
    };
    let mut response = Response::new(Body::from(bytes));
    let h = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&entry.content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, v);
    }
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control(file)),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Accept"));
    if twin.is_some() && file.ends_with(".html") {
        let md = format!("/{}", file.trim_end_matches(".html").to_owned() + ".md");
        if let Ok(v) = HeaderValue::from_str(&format!(
            "<{md}>; rel=\"alternate\"; type=\"text/markdown\""
        )) {
            h.insert(header::LINK, v);
        }
    }
    if let Ok(v) = HeaderValue::from_str(&release.pointer.seq.to_string()) {
        h.insert("x-awp-release", v);
    }
    response
}

async fn not_found(state: &SiteState, release: &LoadedRelease) -> Response {
    if let Some(entry) = release.files().get("404.html")
        && let Ok(Some(bytes)) = state.releases.file(entry).await
    {
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            bytes,
        )
            .into_response();
    }
    plain(StatusCode::NOT_FOUND, "Not found.")
}

/// The platform's island assets, compiled in from `agentrix-console/static/site/`.
async fn asset(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    let (body, kind): (&'static [u8], &'static str) = match file.as_str() {
        "ask.js" => (
            include_bytes!("../../../agentrix-console/static/site/ask.js"),
            "text/javascript; charset=utf-8",
        ),
        "ask.css" => (
            include_bytes!("../../../agentrix-console/static/site/ask.css"),
            "text/css; charset=utf-8",
        ),
        _ => return plain(StatusCode::NOT_FOUND, "Not found."),
    };
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "public, max-age=300"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

/// A visitor's question.
#[derive(Debug, Deserialize)]
pub struct AskRequest {
    /// The question, as typed.
    pub question: String,
}

const MAX_QUESTION_CHARS: usize = 800;

/// The desk Ask answers as, for its living mark (W1, handover-wiring 2.2).
const DESK: &str = "concierge";

/// W1's presence hub: each Ask frame moves the Concierge's mark.
fn presence() -> &'static agentrix_kernel::Presence {
    agentrix_kernel::presence::global()
}

/// What a visitor reads when the Concierge isn't answering.
const PAUSED: &str = "Answers are paused right now. You can still read the site.";

/// What a visitor reads while a platform hold keeps the site read only (W3's words; never the
/// reason).
pub const HELD: &str = "Answers and forms are paused right now. You can still read the site.";

/// Platform holds, per request on every site host (W3, handover-wiring 3.2). Under a `notice`
/// hold the host answers only the neutral "This site isn't available right now."; under a
/// `read-only` hold pages still serve and Ask, passed questions and the door's tools pause
/// (each checks [`crate::guards::Guards::held`]). Visitors never see the reason.
pub async fn holds(State(state): State<SiteState>, request: Request, next: Next) -> Response {
    if state.guards.enforces_holds()
        && let Some(host) = host_of(request.headers())
        && let Ok(Some(pointer)) = state.releases.source().live(&host).await
        && state.guards.hold(&pointer.site_id).await == Some(Mode::Notice)
    {
        return plain(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE);
    }
    next.run(request).await
}

/// Ask's resting reply: no model call and nothing written; the Concierge's mark rests.
fn resting(release: &LoadedRelease, message: &str) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ask::Frame>();
    for f in [
        ask::Frame {
            event: "ready",
            data: serde_json::json!({"desk": "Concierge", "ai": true, "disclosure": ask::DISCLOSURE, "release_seq": release.pointer.seq}),
        },
        ask::Frame {
            event: "error",
            data: serde_json::json!({ "message": message }),
        },
        ask::Frame {
            event: "awp.presence",
            data: serde_json::json!({"state": "resting"}),
        },
        ask::Frame {
            event: "done",
            data: serde_json::json!({}),
        },
    ] {
        presence().on_ask(&release.pointer.site_id, DESK, f.event, &f.data);
        let _ = tx.send(f);
    }
    drop(tx);
    sse(rx).into_response()
}

fn event_of(frame: &ask::Frame) -> Event {
    Event::default()
        .event(frame.event)
        .data(frame.data.to_string())
}

fn contact_href(release: &LoadedRelease) -> Option<String> {
    [
        "contact/index.html",
        "contact.html",
        "contact-us/index.html",
    ]
    .iter()
    .find(|p| release.files().contains_key(**p))
    .map(|p| {
        format!(
            "/{}",
            p.trim_end_matches("index.html").trim_end_matches(".html")
        )
    })
}

async fn ask_handler(
    State(state): State<SiteState>,
    headers: HeaderMap,
    Json(body): Json<AskRequest>,
) -> Response {
    let Some(host) = host_of(&headers) else {
        return plain(StatusCode::BAD_REQUEST, "No Host.");
    };
    let release = match state.releases.current(&host).await {
        Ok(Some(r)) => r,
        _ => return plain(StatusCode::SERVICE_UNAVAILABLE, UNAVAILABLE),
    };
    let question: String = body
        .question
        .trim()
        .chars()
        .take(MAX_QUESTION_CHARS)
        .collect();
    if question.is_empty() {
        return plain(StatusCode::BAD_REQUEST, "Ask a question about this site.");
    }
    // While the owner's Freeze is in force the Concierge rests (W5, handover-wiring 2b.1).
    if state.guards.frozen(&release.pointer.site_id).await {
        return resting(&release, PAUSED);
    }
    // A platform read-only hold pauses answers (W3, handover-wiring 3.2).
    if state.guards.held(&release.pointer.site_id).await {
        return resting(&release, HELD);
    }
    // The Concierge answers only while it is on duty (W1's dials, handover-wiring 2a.3).
    let answering = Action::new(DESK, "presentations", Effect::Read);
    if state
        .guards
        .refusal(&release.pointer.site_id, &answering)
        .await
        .is_some()
    {
        return resting(&release, PAUSED);
    }
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ask::Frame>();
    let offer_href = contact_href(&release);

    // The refusal policy runs before any model call (R8.4).
    let broken = policy::check(&question);
    if !broken.is_empty() {
        let reply = policy::reply(&broken);
        let draft = ReceiptDraft {
            kind: "concierge.refusal",
            site: release.pointer.site_id.clone(),
            subject: format!("release:{}", release.pointer.seq),
            observed: serde_json::json!({
                "excerpt": policy::excerpt(&question),
                "rules": broken.iter().map(|r| r.id()).collect::<Vec<_>>(),
                "reply": reply,
                "desk": "Concierge",
            }),
        };
        let receipt = match state.receipts.write(draft).await {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::error!(%e, "could not write the refusal receipt");
                None
            }
        };
        let frames = [
            ask::Frame {
                event: "ready",
                data: serde_json::json!({"desk": "Concierge", "ai": true, "disclosure": ask::DISCLOSURE, "release_seq": release.pointer.seq}),
            },
            ask::Frame {
                event: "reply",
                data: serde_json::json!({"kind": "refused", "text": reply, "receipt": receipt}),
            },
            ask::Frame {
                event: "awp.presence",
                data: serde_json::json!({"state": "idle"}),
            },
            ask::Frame {
                event: "done",
                data: serde_json::json!({}),
            },
        ];
        for f in frames {
            presence().on_ask(&release.pointer.site_id, DESK, f.event, &f.data);
            let _ = tx.send(f);
        }
        drop(tx);
        return sse(rx).into_response();
    }

    // Retrieval through Vultr's vector store (S3-06): index this release once, in the
    // background, and only after the refusal policy above has run. Until it is ready, and
    // whenever the store fails, Ask ranks the release's passages locally.
    if state.model.store_enabled() {
        let (releases, live, model) =
            (state.releases.clone(), release.clone(), state.model.clone());
        tokio::spawn(async move { ask::index_release_in_store(&releases, &live, model).await });
    }

    let site_name = release.pointer.domain.clone();
    let taught = state
        .guards
        .taught(&release.pointer.site_id, ask::WEBAGENT)
        .await;
    tokio::spawn(async move {
        let site_id = release.pointer.site_id.clone();
        let send = |mut frame: ask::Frame| {
            if frame.event == "reply"
                && let Some(href) = &offer_href
                && let Some(obj) = frame.data.as_object_mut()
                && obj.contains_key("offer")
            {
                obj.insert("offer_href".into(), serde_json::Value::String(href.clone()));
            }
            presence().on_ask(&site_id, DESK, frame.event, &frame.data);
            let _ = tx.send(frame);
        };
        ask::answer(
            &state.releases,
            &release,
            state.model.as_ref(),
            &question,
            &site_name,
            &taught,
            &send,
        )
        .await;
    });
    sse(rx).into_response()
}

fn sse(
    rx: tokio::sync::mpsc::UnboundedReceiver<ask::Frame>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    use futures::StreamExt;
    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx).map(|f| Ok(event_of(&f)));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// `?domain=` for the TLS check.
#[derive(Debug, Deserialize)]
pub struct TlsAsk {
    /// The domain Caddy wants a certificate for.
    pub domain: String,
}

async fn tls_ask(
    State(state): State<SiteState>,
    Query(q): Query<TlsAsk>,
    request: Request,
) -> Response {
    let peer_loopback = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .is_none_or(|c| c.0.ip().is_loopback());
    let proxied = request.headers().contains_key("x-forwarded-for")
        || request.headers().contains_key("forwarded");
    let host_internal = host_of(request.headers())
        .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]");
    if !peer_loopback || proxied || !host_internal {
        return plain(StatusCode::NOT_FOUND, "Not found.");
    }
    match serves_host(&state, &q.domain).await {
        Ok(true) => StatusCode::OK.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(%e, "tls-ask lookup failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

#[cfg(test)]
mod pass_tests;
#[cfg(test)]
mod retrieval_tests;
#[cfg(test)]
pub(crate) mod testkit;
#[cfg(test)]
mod tests;
