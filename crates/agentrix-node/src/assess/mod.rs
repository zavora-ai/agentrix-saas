//! Assess: a free, honest health report for any public site (R1), read only inside sealed
//! rooms on the sandbox VM.
//!
//! Lane: `assess` (S1-05), handed over by the coordinator (design §3.0).
//!
//! | Route | What it does |
//! |---|---|
//! | `GET /`, `GET /assess` | The address bar (`agentrix_onboarding` Start; `Page::Start` without it) |
//! | `POST /assess` (form `url`) | Starts an assessment, no account, and redirects to it at once (R1.1, N2) |
//! | `GET /assess/fixture` | Starts (or reuses) an assessment of Zavora's fixture, labelled as such |
//! | `GET /assess/{id}` | The live receipt and frames while running; the report, or why there is none, after (`Page::Assess`) |
//! | `GET /assess/{id}/events` | SSE (design §4.3a): `step`, `receipt-tick`, `page`, `frame`, `finding`, `error`, `done`; `id:` = seq, `Last-Event-ID` replay or `snapshot`, `: ping` every 15 s |
//!
//! How one assessment runs is in [`pipeline`]; the detectors are in [`detect`]. Frames are
//! served by the console lane's `GET /frames/{digest}` from the CAS (kind `frame`).

pub mod detect;
pub mod html;
pub mod pipeline;
pub mod report;
pub mod robots;
pub mod sqlite;
pub mod store;

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use agentrix_console::{Console, Page, view};
use agentrix_types::ui::{SnapshotEvent, StepEvent, StepName, StepState};
use agentrix_types::{AssessmentId, UiEnvelope, UiEvent};
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::Utc;
use futures::stream::{self, BoxStream, StreamExt as _};
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::{Semaphore, broadcast};

use crate::broker::Broker;
use crate::broker::fetch::{DEFAULT_USER_AGENT, ROBOTS_TOKEN};
use pipeline::Ctx;
use store::{AssessStore, Assessment, AssessmentState, Emitter, Hub, stream_of};

/// Assess settings.
#[derive(Debug, Clone)]
pub struct AssessConfig {
    /// The crawler's `User-Agent`.
    pub user_agent: String,
    /// The token robots.txt rules name it by.
    pub robots_token: String,
    /// Most pages read per assessment (R1.5: 50).
    pub max_pages: u32,
    /// Least time between two requests to the site, in milliseconds.
    pub min_delay_ms: u64,
    /// Hosts that are Zavora's fixture (labelled everywhere, R3.3).
    pub fixture_hosts: Vec<String>,
    /// The fixture's start URL, for the start page's "try it" link.
    pub fixture_url: Option<String>,
    /// Assessments that may run at once.
    pub max_running: usize,
    /// How long a finished assessment of the same host is reused instead of starting another.
    pub reuse_for: Duration,
}

impl Default for AssessConfig {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.into(),
            robots_token: ROBOTS_TOKEN.into(),
            max_pages: 50,
            min_delay_ms: 500,
            fixture_hosts: Vec::new(),
            fixture_url: None,
            max_running: 3,
            reuse_for: Duration::from_secs(120),
        }
    }
}

impl AssessConfig {
    /// The defaults, with `AGENTRIX_FIXTURE_URL` (for example
    /// `https://fixture.agentrix.website/`) naming Zavora's fixture, and
    /// `AGENTRIX_ASSESS_MAX_PAGES` lowering the page budget for development (D10c).
    pub fn from_env() -> Self {
        let mut c = Self::default();
        if let Ok(u) = std::env::var("AGENTRIX_FIXTURE_URL")
            && let Ok(url) = Url::parse(u.trim())
            && let Some(h) = url.host_str()
        {
            c.fixture_hosts = pipeline::allowlist(h);
            c.fixture_url = Some(url.to_string());
        }
        if let Some(n) = std::env::var("AGENTRIX_ASSESS_MAX_PAGES")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            c.max_pages = u32::clamp(n, 1, 50);
        }
        c
    }

    /// [`AssessConfig::from_env`], with Zavora's fixture at `domain` (the node's
    /// `fixture_domain`) when given.
    pub fn with_fixture_domain(mut self, domain: Option<&str>) -> Self {
        if let Some(d) = domain.map(str::trim).filter(|d| !d.is_empty()) {
            self.fixture_hosts = pipeline::allowlist(d);
            self.fixture_url = Some(format!("https://{}/", d.to_ascii_lowercase()));
        }
        self
    }

    fn is_fixture(&self, host: &str) -> bool {
        self.fixture_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(host))
    }
}

/// What the routes share.
#[derive(Clone)]
pub struct AssessState {
    ctx: Arc<Ctx>,
    console: Arc<Console>,
    /// W4's pages, drawn as `concept/assess.html`. Without them the console's templates render.
    pages: Option<Arc<agentrix_onboarding::Pages>>,
}

impl std::fmt::Debug for AssessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssessState")
            .field("ctx", &self.ctx)
            .finish_non_exhaustive()
    }
}

impl AssessState {
    /// Assess for the node: SQLite through `pool` (already migrated by `db::open`).
    pub fn for_node(
        broker: Broker,
        pool: sqlx::SqlitePool,
        console: Arc<Console>,
        config: AssessConfig,
    ) -> Self {
        Self::new(
            broker,
            Arc::new(sqlite::SqliteStore::new(pool)),
            console,
            config,
        )
    }

    /// Assess over `broker` and `store`, rendering with `console`.
    pub fn new(
        broker: Broker,
        store: Arc<dyn AssessStore>,
        console: Arc<Console>,
        config: AssessConfig,
    ) -> Self {
        let slots = Arc::new(Semaphore::new(config.max_running.max(1)));
        Self {
            ctx: Arc::new(Ctx {
                broker,
                store,
                hub: Hub::default(),
                config,
                slots,
            }),
            console,
            pages: None,
        }
    }

    /// Renders Start and Assess with W4's onboarding pages (`agentrix-onboarding`).
    pub fn with_pages(mut self, pages: Arc<agentrix_onboarding::Pages>) -> Self {
        self.pages = Some(pages);
        self
    }

    /// Starts an assessment of `url` (already normalised) and returns its id, or the id of a
    /// recent one of the same host.
    pub async fn start(&self, url: &Url) -> anyhow::Result<AssessmentId> {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let since = Utc::now() - chrono::Duration::from_std(self.ctx.config.reuse_for)?;
        if let Some(recent) = self.ctx.store.latest_for(&host, since).await? {
            return Ok(recent.id);
        }
        let fixture = self.ctx.config.is_fixture(&host);
        let origin = url.origin().ascii_serialization();
        let site = self.ctx.store.site_for(&host, &origin, fixture).await?;
        let now = Utc::now();
        let a = Assessment {
            id: AssessmentId::generate(),
            site: site.clone(),
            url: url.to_string(),
            host,
            fixture,
            state: AssessmentState::Running,
            created_at: now,
            finished_at: None,
            report: None,
        };
        self.ctx.store.create(&a).await?;
        let live = self.ctx.hub.start(&a.id, now);
        let emit = Arc::new(Emitter::new(
            self.ctx.store.clone(),
            live,
            stream_of(&a.id),
            site,
        ));
        let id = a.id.clone();
        tokio::spawn(pipeline::run(self.ctx.clone(), a, emit));
        Ok(id)
    }

    fn render<T: serde::Serialize>(&self, page: Page, view: &T, status: StatusCode) -> Response {
        Self::respond(self.console.render(page, view), status)
    }

    fn render_start(&self, view: view::StartView, status: StatusCode) -> Response {
        match &self.pages {
            Some(p) => Self::respond(
                p.render_start(&agentrix_onboarding::view::StartPage::from(view)),
                status,
            ),
            None => self.render(Page::Start, &view, status),
        }
    }

    fn render_assess(&self, view: view::AssessView) -> Response {
        match &self.pages {
            Some(p) => Self::respond(
                p.render_assess(&agentrix_onboarding::view::AssessPage::from(view)),
                StatusCode::OK,
            ),
            None => self.render(Page::Assess, &view, StatusCode::OK),
        }
    }

    fn respond<E: std::fmt::Display>(html: Result<String, E>, status: StatusCode) -> Response {
        match html {
            Ok(html) => (status, Html(html)).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "rendering failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong on our side. Reload the page in a moment.",
                )
                    .into_response()
            }
        }
    }
}

/// The routes: `GET /assess`, `POST /assess`, `GET /assess/fixture`, `GET /assess/{id}` and
/// `GET /assess/{id}/events`. `/` is W10's home page (`agentrix_home::mount_with_front_door`).
pub fn router(state: AssessState) -> Router {
    Router::new()
        .route("/assess", get(start_page).post(start_assess))
        .route("/assess/fixture", get(start_fixture))
        .route("/assess/{id}", get(assess_page))
        .route("/assess/{id}/events", get(events))
        .with_state(state)
}

/// Turns what a visitor typed into a site URL, or says in plain words what's wrong.
pub fn normalise(input: &str) -> Result<Url, &'static str> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter your website's address.");
    }
    if input.len() > 2000 {
        return Err("That address is too long.");
    }
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut url =
        Url::parse(&with_scheme).map_err(|_| "That doesn't look like a website address.")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Use an address that starts with http:// or https://.");
    }
    let host = match url.host() {
        Some(url::Host::Domain(d)) => d.to_ascii_lowercase(),
        Some(_) => {
            return Err("Enter your site's name, like your-business.example, not an IP address.");
        }
        None => return Err("That address has no site name in it."),
    };
    let local = [
        "localhost",
        ".localhost",
        ".local",
        ".internal",
        ".lan",
        ".home.arpa",
        ".corp",
    ];
    if !host.contains('.')
        || local
            .iter()
            .any(|l| host == l.trim_start_matches('.') || host.ends_with(l))
    {
        return Err("That address isn't on the public internet.");
    }
    if url.port().is_some() {
        return Err(
            "We can only check sites on the standard web ports. Take the port number (like :8080) out of the address.",
        );
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    Ok(url)
}

async fn start_page(State(s): State<AssessState>) -> Response {
    let view = view::StartView {
        error: None,
        url: None,
        fixture_url: s
            .ctx
            .config
            .fixture_url
            .as_ref()
            .map(|_| "/assess/fixture".to_owned()),
    };
    s.render_start(view, StatusCode::OK)
}

/// The start form.
#[derive(Debug, Deserialize)]
pub struct StartForm {
    /// What the visitor typed.
    #[serde(default)]
    pub url: String,
}

async fn start_assess(State(s): State<AssessState>, Form(form): Form<StartForm>) -> Response {
    let fixture_url = s
        .ctx
        .config
        .fixture_url
        .as_ref()
        .map(|_| "/assess/fixture".to_owned());
    let url = match normalise(&form.url) {
        Ok(u) => u,
        Err(msg) => {
            let view = view::StartView {
                error: Some(msg.to_owned()),
                url: Some(form.url.chars().take(200).collect()),
                fixture_url,
            };
            return s.render_start(view, StatusCode::UNPROCESSABLE_ENTITY);
        }
    };
    match s.start(&url).await {
        Ok(id) => Redirect::to(&format!("/assess/{id}")).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "could not start an assessment");
            let view = view::StartView {
                error: Some("We couldn't start the check just now. Try again in a minute.".into()),
                url: Some(form.url.chars().take(200).collect()),
                fixture_url,
            };
            s.render_start(view, StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

async fn start_fixture(State(s): State<AssessState>) -> Response {
    let Some(url) = s
        .ctx
        .config
        .fixture_url
        .as_deref()
        .and_then(|u| Url::parse(u).ok())
    else {
        return Redirect::to("/").into_response();
    };
    match s.start(&url).await {
        Ok(id) => Redirect::to(&format!("/assess/{id}")).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "could not start the fixture assessment");
            Redirect::to("/").into_response()
        }
    }
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html("<p>There's no such check. <a href=\"/\">Start one</a>.</p>"),
    )
        .into_response()
}

async fn assess_page(State(s): State<AssessState>, Path(id): Path<String>) -> Response {
    let id = AssessmentId::new(id);
    let a = match s.ctx.store.get(&id).await {
        Ok(Some(a)) => a,
        Ok(None) => return not_found(),
        Err(e) => {
            tracing::error!(error = %e, "reading an assessment");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong on our side. Reload the page in a moment.",
            )
                .into_response();
        }
    };
    let live = s.ctx.hub.get(&id).map(|l| l.state());
    let mut a = a;
    if a.state == AssessmentState::Running && live.is_none() {
        // The process restarted mid-run: say so rather than show a frozen "running".
        a.state = AssessmentState::Failed;
        a.report = Some(report::ReportDoc::without_report(
            report::Outcome::Failed,
            "This check was interrupted before it finished, so there's no report.".into(),
            "Run the check again in a few minutes.".into(),
        ));
    }
    let view = report::view(&a, live.as_ref());
    s.render_assess(view)
}

fn sse_event(env: &UiEnvelope) -> Event {
    Event::default()
        .id(env.seq.to_string())
        .event(env.event.name())
        .data(env.event.data_json().unwrap_or_else(|_| "{}".into()))
}

struct Tail {
    rx: Option<broadcast::Receiver<UiEnvelope>>,
    pending: VecDeque<UiEnvelope>,
    cursor: u64,
    store: Arc<dyn AssessStore>,
    stream: String,
    finished: bool,
}

async fn events(
    State(s): State<AssessState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let id = AssessmentId::new(id);
    match s.ctx.store.get(&id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    let stream_name = stream_of(&id);
    let last: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let live = s.ctx.hub.get(&id);
    let rx = live.as_ref().map(|l| l.tx.subscribe());

    let mut pending = VecDeque::new();
    let cursor;
    match (&live, last) {
        (Some(l), None) if !l.state().done => {
            // A fresh page: it rendered the current state; send a snapshot to align.
            let st = l.state();
            let step = st.step.clone().unwrap_or_else(|| {
                StepEvent::new(
                    StepName::Assess,
                    StepState::Running,
                    0,
                    "Starting",
                    Utc::now(),
                )
            });
            pending.push_back(UiEnvelope {
                seq: st.seq,
                event: UiEvent::Snapshot(SnapshotEvent {
                    steps: vec![step],
                    tick: st.tick.clone(),
                    last_frame: st.frame.clone(),
                    parity: None,
                    at: Utc::now(),
                }),
            });
            cursor = st.seq;
        }
        (_, after) => {
            let after = after.unwrap_or(0);
            let backlog = s
                .ctx
                .store
                .events_after(&stream_name, after)
                .await
                .unwrap_or_default();
            cursor = backlog.last().map_or(after, |e| e.seq);
            pending.extend(backlog);
        }
    }
    let finished = rx.is_none() || pending.iter().any(|e| matches!(e.event, UiEvent::Done(_)));
    let tail = Tail {
        rx,
        pending,
        cursor,
        store: s.ctx.store.clone(),
        stream: stream_name,
        finished,
    };
    let body: BoxStream<'static, Result<Event, Infallible>> =
        stream::unfold(tail, |mut t| async move {
            loop {
                if let Some(env) = t.pending.pop_front() {
                    if matches!(env.event, UiEvent::Done(_)) {
                        t.finished = true;
                        t.pending.clear();
                        t.rx = None;
                    }
                    return Some((Ok(sse_event(&env)), t));
                }
                if t.finished {
                    return None;
                }
                let rx = t.rx.as_mut()?;
                match rx.recv().await {
                    Ok(env) if env.seq > t.cursor => {
                        t.cursor = env.seq;
                        t.pending.push_back(env);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let missed = t
                            .store
                            .events_after(&t.stream, t.cursor)
                            .await
                            .unwrap_or_default();
                        if let Some(last) = missed.last() {
                            t.cursor = last.seq;
                        }
                        t.pending.extend(missed);
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed();
    Sse::new(body)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

#[cfg(test)]
mod tests;
