//! The console's GET pages (design §3.0, §4.1): `/console/*`, the receipt page and frames.
//!
//! **Deprecated: the first release's owner pages are replaced by Agentrix OS at `/os`
//! (`agentrix-shell`).** [`router`] answers each old path with a 308 to its `/os` page:
//!
//! | Old route | Now |
//! |---|---|
//! | `/console`, `/console/today`, `/console/first-today` | `/os` (Back office) |
//! | `/console/decisions` | `/os/decisions` |
//! | `/console/decisions/{id}` | `/d/{id}` (the card in `/os`) |
//! | `/console/trust` | `/os/trust` |
//! | `/console/webagents` | `/os/webagents` |
//! | `/console/quarantine` | `/os/quarantine` |
//!
//! `/console/move` has no `/os` twin, so it still serves W4's Move (`agentrix-onboarding`),
//! and the first release's Move page only when node runs without W4's pages. The receipt page and frames
//! are public and stay. The first release's handlers are kept, unmounted, in [`first_release_router`] for their
//! tests until they are removed.
//!
//! Each handler reads through [`ConsoleReads`], maps the values with [`map`], and renders an
//! `agentrix-console` page. POSTs stay with their owners: `decisions/` (approve, decline,
//! quarantine restore), `auth/` and `claim/`. A page that is not signed in redirects to
//! sign-in; a read that fails shows a plain error and logs the detail.

pub mod map;
pub mod reads;
pub mod store;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod tests;

pub use reads::*;
pub use store::{Delegates, SqliteReads};

use std::str::FromStr;
use std::sync::Arc;

use agentrix_console::{Console, Page, view};
use agentrix_types::{DecisionId, Digest};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::Utc;
use serde::Serialize;

/// What the pages need: the template environment and the reads.
#[derive(Clone)]
pub struct PagesState {
    /// The console templates, built once at startup.
    pub console: Arc<Console>,
    /// The stores, through the port.
    pub reads: Arc<dyn ConsoleReads>,
    /// W4's pages: the Move drawn as `concept/move.html`. Without them the console's renders.
    pub onboarding: Option<Arc<agentrix_onboarding::Pages>>,
    /// `control.db`, for the look the owner chose before the Move (`agentrix-concepts`). `None`
    /// leaves the Move page's look line out.
    pub concepts: Option<sqlx::SqlitePool>,
}

/// The console's GET routes, for `app.rs` to merge. The first release's owner pages are deprecated: each
/// old path is a 308 to its page in Agentrix OS (see the module docs).
pub fn router(state: PagesState) -> Router {
    Router::new()
        .route("/console", get(|| async { to_os("/os") }))
        .route("/console/today", get(|| async { to_os("/os") }))
        .route("/console/first-today", get(|| async { to_os("/os") }))
        .route(
            "/console/decisions",
            get(|| async { to_os("/os/decisions") }),
        )
        .route("/console/decisions/{id}", get(card_to_os))
        .route("/console/move", get(move_page))
        .route("/console/trust", get(|| async { to_os("/os/trust") }))
        .route(
            "/console/webagents",
            get(|| async { to_os("/os/webagents") }),
        )
        .route(
            "/console/quarantine",
            get(|| async { to_os("/os/quarantine") }),
        )
        .route("/receipts/{digest}", get(receipt))
        .route("/frames/{digest}", get(frame))
        .with_state(state)
}

/// A 308 from a deprecated the first release page to its page in Agentrix OS. The targets don't name the
/// site (`/os/…` and `/d/…` find the signed-in owner's), so the redirect is the same for
/// everyone and safe to keep.
fn to_os(path: &str) -> Response {
    Redirect::permanent(path).into_response()
}

/// `/console/decisions/{id}` to the card's short link, which opens it in Agentrix OS. Only a
/// plain card id passes.
async fn card_to_os(Path(id): Path<String>) -> Response {
    let plain = !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !plain {
        return PageError::NotFound.into_response();
    }
    to_os(&format!("/d/{id}"))
}

/// Deprecated: the first release's owner pages, no longer mounted. [`router`] sends each of these paths
/// to its Agentrix OS page; the handlers stay, with their tests, until they are removed.
pub fn first_release_router(state: PagesState) -> Router {
    Router::new()
        .route("/console/first-today", get(today))
        .route("/console/decisions", get(decisions))
        .route("/console/decisions/{id}", get(decision))
        .route("/console/trust", get(trust))
        .route("/console/webagents", get(webagents))
        .route("/console/quarantine", get(quarantine))
        .with_state(state)
}

/// Why a page didn't render.
enum PageError {
    /// Not signed in: go to sign-in, then come back.
    SignIn(String),
    /// Signed in, but no site yet: their claim, or Assess.
    Go(String),
    /// No such card, receipt or frame for this owner.
    NotFound,
    /// A read or a render failed.
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for PageError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        match self {
            Self::SignIn(next) => Redirect::to(&format!("/auth/signin?next={next}")).into_response(),
            Self::Go(path) => Redirect::to(&path).into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, Html("<!doctype html><title>Not found</title><p>We couldn't find that. <a href=\"/os\">Back to your console</a></p>")).into_response(),
            Self::Internal(e) => {
                tracing::error!(error = %format!("{e:#}"), "console page failed");
                (StatusCode::INTERNAL_SERVER_ERROR, Html("<!doctype html><title>This page didn't load</title><p>This page couldn't load just now. Nothing changed. Reload in a minute.</p>")).into_response()
            }
        }
    }
}

type PageResult = Result<Response, PageError>;

fn render(state: &PagesState, page: Page, view: &impl Serialize) -> PageResult {
    let html = state
        .console
        .render(page, view)
        .map_err(|e| PageError::Internal(e.into()))?;
    Ok(Html(html).into_response())
}

/// The signed-in owner and their site, or a redirect to sign-in that returns to `uri`.
async fn owner(
    state: &PagesState,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<(OwnerSession, SiteRow), PageError> {
    let Some(session) = state.reads.session(headers).await? else {
        if let Some(path) = state.reads.landing(headers).await? {
            return Err(PageError::Go(path));
        }
        return Err(PageError::SignIn(uri.path().to_owned()));
    };
    let site = state
        .reads
        .site(&session.site)
        .await?
        .ok_or(PageError::NotFound)?;
    Ok((session, site))
}

async fn waiting(state: &PagesState, site: &SiteRow) -> Result<Vec<DecisionRow>, PageError> {
    Ok(state
        .reads
        .decisions(&site.id)
        .await?
        .into_iter()
        .filter(|d| d.state == DecisionState::Waiting)
        .collect())
}

async fn receipt_rows(
    state: &PagesState,
    site: &SiteRow,
    limit: usize,
) -> Result<Vec<view::ReceiptRow>, PageError> {
    let keys = state.reads.keys().await?;
    let now = Utc::now();
    Ok(state
        .reads
        .receipts(&site.id, limit)
        .await?
        .iter()
        .filter_map(|r| map::receipt_row(r, &keys, site.utc_offset_minutes, now))
        .collect())
}

async fn today(State(state): State<PagesState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let waiting = waiting(&state, &site).await?;
    let brief = state.reads.brief(&site.id).await?;
    let recent = receipt_rows(&state, &site, 5).await?;
    let passed = state.reads.passed_questions(&site.id).await?;
    let meter = state.reads.meter(&site.id).await?;
    let chrome = map::chrome(&site, &session, view::NavTab::Today, waiting.len(), now);
    let mut view = map::today(
        chrome,
        &site,
        &session,
        brief.as_ref(),
        &waiting,
        recent,
        &passed,
        now,
    );
    view.meter = meter.as_ref().map(map::meter);
    render(&state, Page::Today, &view)
}

async fn decisions(State(state): State<PagesState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let all = state.reads.decisions(&site.id).await?;
    let (open, closed): (Vec<_>, Vec<_>) =
        all.iter().partition(|d| d.state == DecisionState::Waiting);
    let view = view::DecisionsView {
        chrome: map::chrome(&site, &session, view::NavTab::Decisions, open.len(), now),
        waiting: open
            .iter()
            .map(|d| map::card_summary(d, site.utc_offset_minutes, now))
            .collect(),
        decided: closed
            .iter()
            .map(|d| map::card_summary(d, site.utc_offset_minutes, now))
            .collect(),
    };
    render(&state, Page::Decisions, &view)
}

async fn decision(
    State(state): State<PagesState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let row = state
        .reads
        .decision(&site.id, &DecisionId::new(id))
        .await?
        .ok_or(PageError::NotFound)?;
    let options = if row.state == DecisionState::Waiting && session.has_passkey {
        state.reads.begin_approval(&session, &row.card).await?
    } else {
        None
    };
    let levels = if row.card.kind == agentrix_types::CardKind::GoLive {
        state.reads.levels(&site.id).await?
    } else {
        Vec::new()
    };
    let waiting = waiting(&state, &site).await?;
    let view = view::DecisionView {
        chrome: map::chrome(&site, &session, view::NavTab::Decisions, waiting.len(), now),
        card: map::card(
            &row,
            &levels,
            options,
            session.has_passkey,
            site.utc_offset_minutes,
            now,
        ),
    };
    render(&state, Page::Decision, &view)
}

async fn move_page(State(state): State<PagesState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let waiting = waiting(&state, &site).await?;
    let row = state.reads.latest_move(&site.id).await?;
    let chrome = map::chrome(&site, &session, view::NavTab::Move, waiting.len(), now);
    let mut view = map::move_view(chrome, row.as_ref(), site.utc_offset_minutes, now);
    if let Some(m) = &row {
        view.spend_words = state.reads.move_spend(&m.id).await?;
    }
    view.uploads = state
        .reads
        .uploads(&site.id)
        .await?
        .iter()
        .map(|u| view::UploadRow {
            name: u.name.clone(),
            size_words: map::bytes_words(u.size),
            when_words: map::when(u.at, site.utc_offset_minutes, now),
        })
        .collect();
    if let Some(p) = &state.onboarding {
        let live = view.run.state == view::MoveState::Live;
        let mut page = agentrix_onboarding::view::MovePage::from(view);
        // The look chosen before the Move ("Choose your new look"); a failed read only hides it.
        if let Some(pool) = &state.concepts {
            match agentrix_concepts::chosen(pool, site.id.as_str()).await {
                Ok(Some(c)) => {
                    page.look = Some(agentrix_onboarding::view::LookRef {
                        name: c.name,
                        sentence: c.direction,
                        href: format!("/sites/{}/concepts", site.id.as_str()),
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "reading the chosen look failed")
                }
            }
        }
        // Pictures the fill drafted after go-live wait in Pictures (1.3). A failed count only
        // hides the line.
        if live {
            match state.reads.pictures_waiting(&site.id).await {
                Ok(n) if n > 0 => {
                    page.pictures_href =
                        Some(agentrix_media::library::pictures_url(site.id.as_str()));
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "counting waiting pictures failed")
                }
            }
        }
        let html = p
            .render_move(&page)
            .map_err(|e| PageError::Internal(e.into()))?;
        return Ok(Html(html).into_response());
    }
    render(&state, Page::Move, &view)
}

async fn trust(State(state): State<PagesState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let waiting = waiting(&state, &site).await?;
    let keys = state.reads.keys().await?;
    let gauntlet = state.reads.gauntlet(&site.id).await?;
    let rows = receipt_rows(&state, &site, 100).await?;
    let chrome = map::chrome(&site, &session, view::NavTab::Trust, waiting.len(), now);
    render(
        &state,
        Page::Trust,
        &map::trust(
            chrome,
            gauntlet.as_ref(),
            rows,
            &keys,
            site.utc_offset_minutes,
            now,
        ),
    )
}

async fn webagents(State(state): State<PagesState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let waiting = waiting(&state, &site).await?;
    let levels = state.reads.levels(&site.id).await?;
    let chrome = map::chrome(&site, &session, view::NavTab::Webagents, waiting.len(), now);
    render(&state, Page::WebAgents, &map::webagents(chrome, &levels))
}

async fn quarantine(State(state): State<PagesState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let (session, site) = owner(&state, &headers, &uri).await?;
    let now = Utc::now();
    let waiting = waiting(&state, &site).await?;
    let rows = state.reads.quarantine(&site.id).await?;
    let chrome = map::chrome(&site, &session, view::NavTab::Move, waiting.len(), now);
    render(&state, Page::Quarantine, &map::quarantine(chrome, &rows))
}

/// `GET /receipts/{digest}`: anyone may open and verify a receipt (§4.1). JSON on
/// `Accept: application/json` (the signed receipt, canonical), otherwise the page.
async fn receipt(
    State(state): State<PagesState>,
    Path(digest): Path<String>,
    headers: HeaderMap,
) -> PageResult {
    let digest = Digest::from_str(&digest).map_err(|_| PageError::NotFound)?;
    let r = state
        .reads
        .receipt(&digest)
        .await?
        .ok_or(PageError::NotFound)?;
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("application/json"));
    if wants_json {
        let body = r
            .canonical_json()
            .map_err(|e| PageError::Internal(e.into()))?;
        return Ok((
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            body,
        )
            .into_response());
    }
    let now = Utc::now();
    let site = state
        .reads
        .site(&r.body.site)
        .await?
        .ok_or(PageError::NotFound)?;
    let keys = state.reads.keys().await?;
    let session = state
        .reads
        .session(&headers)
        .await?
        .filter(|s| s.site == site.id);
    let chrome = match &session {
        Some(s) => map::chrome(
            &site,
            s,
            view::NavTab::Trust,
            waiting(&state, &site).await?.len(),
            now,
        ),
        None => view::Chrome {
            site: map::site_ref(&site, now),
            nav: view::Nav {
                active: view::NavTab::Trust,
                decisions_waiting: 0,
            },
            owner: view::OwnerRef {
                name: String::new(),
            },
        },
    };
    let detail =
        map::receipt_detail(&r, &keys, site.utc_offset_minutes, now).ok_or(PageError::NotFound)?;
    render(
        &state,
        Page::Receipt,
        &view::ReceiptView {
            chrome,
            receipt: detail,
        },
    )
}

/// `GET /frames/{digest}`: a sealed-browser frame from the CAS. The owner sees their site's
/// frames, and anyone sees a free Assess's (design §4.1). Content-addressed, so it never
/// changes, but `private`: a shared cache must not hand an owner's frame to someone else.
async fn frame(
    State(state): State<PagesState>,
    headers: HeaderMap,
    Path(digest): Path<String>,
) -> PageResult {
    let digest = Digest::from_str(&digest).map_err(|_| PageError::NotFound)?;
    let owner_site = state.reads.session(&headers).await?.map(|s| s.site);
    let f = state
        .reads
        .frame(owner_site.as_ref(), &digest)
        .await?
        .ok_or(PageError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, f.content_type),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".to_owned(),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        f.bytes,
    )
        .into_response())
}
