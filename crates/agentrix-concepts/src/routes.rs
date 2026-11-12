//! The concept stage's pages, on the platform host.
//!
//! | Route | What |
//! |---|---|
//! | `GET /sites/{site}/concepts` | "Choose your new look": the newest run's three columns |
//! | `POST /sites/{site}/concepts` | Design three looks (or three more) |
//! | `GET /sites/{site}/concepts/live` | SSE: the newest run's progress, one JSON snapshot per event |
//! | `GET /sites/{site}/concepts/{concept}/preview` | The finished, cleaned page, with the preview CSP |
//! | `GET /sites/{site}/concepts/{concept}/draft` | What the agent has written so far, cleaned |
//! | `POST /sites/{site}/concepts/{concept}/choose` (form `token`) | Choose this look; deletes the rooms |
//! | `POST /sites/{site}/concepts/{concept}/guide` (form `words`, `token`) | Tell this designer what to change |
//! | `GET /sites/{site}/concepts/evidence` | The Challenge 1 rows, as an HTML fragment the page refreshes |
//! | `GET /static/concepts/concepts.{css,js}` | The page's styles and script |
//!
//! Every route needs the signed-in owner of the site, through a verified claim. Posts must come
//! from this origin, and choose and guide also carry the run's form token.

use std::convert::Infallible;
use std::sync::Arc;

use agentrix_host::{OwnerSessions, Receipts};
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use minijinja::{Environment, UndefinedBehavior, context};
use serde::{Deserialize, Serialize};

use crate::evidence::{self, Limits};
use crate::run::{Engine, Snapshot};
use crate::sanitize::{Policy, preview_csp, sanitize};
use crate::store;
use crate::stream::extract_html;

const PAGE: &str = include_str!("../templates/concepts/page.html");
const EVIDENCE: &str = include_str!("../templates/concepts/evidence.html");

/// Design runs a site may start in 24 hours.
pub const RUNS_PER_DAY: i64 = 6;
/// Changes an owner may ask designers for in 24 hours.
pub const CHANGES_PER_DAY: i64 = 12;
/// The longest change request.
pub const MAX_CHANGE_CHARS: usize = 400;
const CSS: &str = include_str!("../static/concepts.css");
const JS: &str = include_str!("../static/concepts.js");

/// Everything the routes share.
#[derive(Clone)]
pub struct Routes {
    env: Arc<Environment<'static>>,
    owners: Arc<dyn OwnerSessions>,
    receipts: Arc<dyn Receipts>,
    pool: sqlx::SqlitePool,
    blobs: Arc<dyn agentrix_host::Blobs>,
    engine: Option<Engine>,
    off: Option<&'static str>,
    origin: String,
    limits: Limits,
}

impl Routes {
    /// Loads the page template; `off` is why the stage is hidden, when `engine` is `None`.
    pub fn new(
        ctx: &agentrix_host::NodeContext,
        engine: Option<Engine>,
        off: Option<&'static str>,
        limits: Limits,
    ) -> anyhow::Result<Self> {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_formatter(format);
        env.add_template("concepts/evidence.html", EVIDENCE)?;
        env.add_template("concepts/page.html", PAGE)?;
        env.add_global("ui_v", agentrix_ui::VERSION);
        env.add_global(
            "onboarding_v",
            agentrix_onboarding::ONBOARDING_ASSETS_VERSION,
        );
        Ok(Self {
            env: Arc::new(env),
            owners: ctx.owners.clone(),
            receipts: ctx.receipts.clone(),
            pool: ctx.pool.clone(),
            blobs: ctx.blobs.clone(),
            engine,
            off,
            origin: ctx.origin.as_str().trim_end_matches('/').to_owned(),
            limits,
        })
    }
}

/// HTML-escapes text (`& < > " '`) without minijinja's `/` to `&#x2f;`, so paths and
/// "vultr / glm-5.3" read as written. Strict mode still refuses an undefined value first.
fn format(
    out: &mut minijinja::Output,
    state: &minijinja::State,
    value: &minijinja::Value,
) -> Result<(), minijinja::Error> {
    match (state.auto_escape(), value.as_str()) {
        (minijinja::AutoEscape::Html, Some(text)) if !value.is_safe() => {
            let mut escaped = String::with_capacity(text.len());
            for c in text.chars() {
                match c {
                    '&' => escaped.push_str("&amp;"),
                    '<' => escaped.push_str("&lt;"),
                    '>' => escaped.push_str("&gt;"),
                    '"' => escaped.push_str("&quot;"),
                    '\'' => escaped.push_str("&#x27;"),
                    c => escaped.push(c),
                }
            }
            Ok(out.write_str(&escaped)?)
        }
        _ => minijinja::escape_formatter(out, state, value),
    }
}

/// The routes, for node to merge into the platform host.
pub fn router(routes: Routes) -> Router {
    Router::new()
        .route("/sites/{site}/concepts", get(page).post(start))
        .route("/sites/{site}/concepts/live", get(live))
        .route("/sites/{site}/concepts/evidence", get(evidence_rows))
        .route("/sites/{site}/concepts/{concept}/preview", get(preview))
        .route("/sites/{site}/concepts/{concept}/draft", get(draft))
        .route("/sites/{site}/concepts/{concept}/choose", post(choose))
        .route("/sites/{site}/concepts/{concept}/guide", post(guide))
        .route(
            "/static/concepts/concepts.css",
            get(|| async { asset("text/css; charset=utf-8", CSS) }),
        )
        .route(
            "/static/concepts/concepts.js",
            get(|| async { asset("text/javascript; charset=utf-8", JS) }),
        )
        .with_state(routes)
}

fn asset(kind: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "no-cache"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

/// A site id as it appears in a path.
fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Who may see the stage for a site.
enum Gate {
    /// The signed-in owner of a claimed site: their account.
    Owner(String),
    /// Not signed in.
    SignIn,
    /// Signed in, but this site isn't theirs (or isn't claimed yet).
    NotYours,
}

impl Routes {
    async fn gate(&self, headers: &HeaderMap, site: &str) -> Gate {
        if !safe_id(site) {
            return Gate::NotYours;
        }
        let Ok(Some(owner)) = self.owners.owner(headers).await else {
            return Gate::SignIn;
        };
        let account = owner.account.to_string();
        match store::owner_of(&self.pool, site).await {
            Ok(Some(o)) if o == account => Gate::Owner(account),
            Ok(_) => Gate::NotYours,
            Err(e) => {
                tracing::warn!(error = %e, "concepts: reading the site's owner");
                Gate::NotYours
            }
        }
    }

    /// A POST from another site's page is refused (the cookie alone isn't enough).
    fn same_origin(&self, headers: &HeaderMap) -> bool {
        headers
            .get(header::ORIGIN)
            .and_then(|o| o.to_str().ok())
            .is_none_or(|o| o.trim_end_matches('/') == self.origin)
    }

    fn render(&self, status: StatusCode, view: impl Serialize) -> Response {
        self.render_template("concepts/page.html", status, view)
    }

    fn render_template(&self, name: &str, status: StatusCode, view: impl Serialize) -> Response {
        match self.env.get_template(name).and_then(|t| t.render(view)) {
            Ok(body) => (status, Html(body)).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "concepts: rendering the page");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "This page didn't load on our side. Nothing was changed. Try again in a moment.",
                )
                    .into_response()
            }
        }
    }

    fn refuse(&self, gate: Gate, site: &str) -> Response {
        match gate {
            Gate::SignIn => Redirect::to(&format!(
                "/account/signin?next=%2Fsites%2F{site}%2Fconcepts"
            ))
            .into_response(),
            _ => self.render(
                StatusCode::FORBIDDEN,
                context! { mode => "not-yours", site_id => site },
            ),
        }
    }

    async fn snapshot(&self, site: &str) -> Option<Snapshot> {
        let (run, concepts) = store::latest_run(&self.pool, site).await.ok()??;
        if let Some(live) = self.engine.as_ref().and_then(|e| e.hub.get(&run.run_id)) {
            let mut s = live.snapshot();
            // The live view doesn't know what was chosen since; the records do.
            for a in &mut s.agents {
                a.chosen = concepts
                    .iter()
                    .any(|c| c.concept_id == a.concept_id && c.chosen == 1);
            }
            return Some(s);
        }
        Some(Snapshot::from_records(&run, &concepts))
    }
}

async fn page(State(r): State<Routes>, Path(site): Path<String>, headers: HeaderMap) -> Response {
    let gate = r.gate(&headers, &site).await;
    let Gate::Owner(_) = gate else {
        return r.refuse(gate, &site);
    };
    let (domain, name) = store::site(&r.pool, &site)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    let snapshot = r.snapshot(&site).await;
    let running = snapshot
        .as_ref()
        .is_some_and(|s| matches!(s.state.as_str(), "planning" | "designing" | "revising"));
    let chosen = store::chosen(&r.pool, &site).await.ok().flatten();
    let latest = store::latest_run(&r.pool, &site).await.ok().flatten();
    let token = latest
        .as_ref()
        .map(|(run, _)| run.csrf.clone())
        .unwrap_or_default();
    let rooms_open = latest
        .as_ref()
        .is_some_and(|(run, _)| run.rooms_deleted_at.is_none());
    let rows = evidence::for_site(&r.pool, &site, r.limits)
        .await
        .unwrap_or_default();
    let stage = snapshot
        .as_ref()
        .map_or("facts", |s| s.stage_now())
        .to_owned();
    r.render(
        StatusCode::OK,
        context! {
            mode => "stage",
            site_id => site,
            host => domain,
            name => if name.is_empty() { domain.clone() } else { name },
            off => r.off,
            run => snapshot,
            running => running,
            stage => stage,
            token => token,
            can_guide => rooms_open && r.engine.is_some(),
            rows => rows,
            chosen => chosen.map(|c| context! { concept_id => c.concept_id, name => c.name, sentence => c.direction }),
            move_href => "/console/move",
        },
    )
}

async fn start(State(r): State<Routes>, Path(site): Path<String>, headers: HeaderMap) -> Response {
    let gate = r.gate(&headers, &site).await;
    let Gate::Owner(account) = gate else {
        return r.refuse(gate, &site);
    };
    let back = format!("/sites/{site}/concepts");
    if !r.same_origin(&headers) {
        return (
            StatusCode::FORBIDDEN,
            "This request came from another site, so nothing was started.",
        )
            .into_response();
    }
    let Some(engine) = &r.engine else {
        return Redirect::to(&back).into_response();
    };
    if let Ok(Some((run, _))) = store::latest_run(&r.pool, &site).await
        && matches!(run.state.as_str(), "planning" | "designing")
    {
        // One run at a time per site.
        return Redirect::to(&back).into_response();
    }
    // Each run costs model time: a few sets a day per site is plenty to choose from.
    match store::runs_since(&r.pool, &site, chrono::Duration::hours(24)).await {
        Ok(n) if n >= RUNS_PER_DAY => {
            return r.render(
                StatusCode::TOO_MANY_REQUESTS,
                context! { mode => "enough", site_id => site, runs => RUNS_PER_DAY },
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "concepts: counting today's runs"),
    }
    match crate::facts::site_facts(&r.pool, &r.blobs, &site).await {
        Ok(_) => {}
        Err(e) => {
            tracing::info!(error = %e, "concepts: no facts to design from");
            return r.render(
                StatusCode::CONFLICT,
                context! { mode => "no-report", site_id => site },
            );
        }
    }
    match engine.start(&site, &account).await {
        Ok(_) => Redirect::to(&back).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "concepts: starting a run");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "The design run didn't start on our side. Try again in a moment.",
            )
                .into_response()
        }
    }
}

async fn live(State(r): State<Routes>, Path(site): Path<String>, headers: HeaderMap) -> Response {
    let gate = r.gate(&headers, &site).await;
    let Gate::Owner(_) = gate else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Subscribe before reading the snapshot, so no update falls between them.
    let run_id = store::latest_run(&r.pool, &site)
        .await
        .ok()
        .flatten()
        .map(|(run, _)| run.run_id)
        .unwrap_or_default();
    let rx = r
        .engine
        .as_ref()
        .and_then(|e| e.hub.get(&run_id))
        .map(|live| live.subscribe());
    let first = r.snapshot(&site).await;
    let finished = first
        .as_ref()
        .is_none_or(|s| matches!(s.state.as_str(), "done" | "failed"));
    let first_json = first
        .as_ref()
        .and_then(|s| serde_json::to_string(s).ok())
        .unwrap_or_else(|| "{}".into());
    let head =
        futures::stream::once(
            async move { Ok::<_, Infallible>(Event::default().data(first_json)) },
        );
    let stream: futures::stream::BoxStream<'static, Result<Event, Infallible>> = match rx {
        Some(rx) if !finished => {
            // The stream ends with the run's last snapshot.
            let updates = tokio_stream::wrappers::BroadcastStream::new(rx)
                .filter_map(|m| async move { m.ok() })
                .scan(false, |ended, json| {
                    let out = (!*ended).then(|| {
                        *ended = json.contains("\"state\":\"done\"")
                            || json.contains("\"state\":\"failed\"");
                        Ok(Event::default().data(json))
                    });
                    async move { out }
                });
            head.chain(updates).boxed()
        }
        _ => head.boxed(),
    };
    let mut res = Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

fn preview_response(origin: &str, body: String) -> Response {
    let csp = HeaderValue::from_str(&preview_csp(origin))
        .unwrap_or_else(|_| HeaderValue::from_static("sandbox; default-src 'none'"));
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CONTENT_SECURITY_POLICY, csp),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

const NOTHING_YET: &str = "<!DOCTYPE html><html><body style=\"font:15px system-ui;color:#6b645a;padding:24px\"><p>Nothing to show yet. The page appears here as it's written.</p></body></html>";

async fn preview(
    State(r): State<Routes>,
    Path((site, concept)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Gate::Owner(_) = r.gate(&headers, &site).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Some(c)) = store::concept(&r.pool, &site, &concept).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(digest) = c.html_digest else {
        return preview_response(&r.origin, NOTHING_YET.into());
    };
    match r.blobs.get(&digest).await {
        Ok(Some(bytes)) => {
            preview_response(&r.origin, String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn draft(
    State(r): State<Routes>,
    Path((site, concept)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Gate::Owner(_) = r.gate(&headers, &site).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Some(c)) = store::concept(&r.pool, &site, &concept).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // The designer's own index.html in its room, as it grows.
    let text = r
        .engine
        .as_ref()
        .and_then(|e| e.page_in_room(&c.concept_id))
        .unwrap_or_default();
    let host = store::site(&r.pool, &site)
        .await
        .ok()
        .flatten()
        .map(|(d, _)| d)
        .unwrap_or_default();
    let body = extract_html(&text)
        .map(|h| sanitize(h, &Policy::for_host(&host)))
        .unwrap_or_else(|| NOTHING_YET.into());
    preview_response(&r.origin, body)
}

/// The form a choose or guide post carries.
#[derive(Debug, Default, Deserialize)]
struct PostForm {
    #[serde(default)]
    token: String,
    #[serde(default)]
    words: String,
}

impl Routes {
    /// Whether `token` is the form token of the run that `concept` belongs to.
    async fn token_ok(&self, concept: &store::ConceptRow, token: &str) -> bool {
        let Ok(Some((run, _))) = store::latest_run(&self.pool, &concept.site_id).await else {
            return false;
        };
        !run.csrf.is_empty() && run.run_id == concept.run_id && run.csrf == token
    }
}

fn forbidden(words: &'static str) -> Response {
    (StatusCode::FORBIDDEN, words).into_response()
}

async fn choose(
    State(r): State<Routes>,
    Path((site, concept)): Path<(String, String)>,
    headers: HeaderMap,
    Form(form): Form<PostForm>,
) -> Response {
    let gate = r.gate(&headers, &site).await;
    let Gate::Owner(account) = gate else {
        return r.refuse(gate, &site);
    };
    if !r.same_origin(&headers) {
        return forbidden("This request came from another site, so nothing was chosen.");
    }
    let Ok(Some(row)) = store::concept(&r.pool, &site, &concept).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !r.token_ok(&row, &form.token).await {
        return forbidden(
            "This page is out of date, so nothing was chosen. Reload it and choose again.",
        );
    }
    let receipt = crate::receipts::issue(
        &r.receipts,
        "concept.chosen",
        &site,
        row.html_digest.as_deref(),
        serde_json::json!({ "concept": concept, "direction": row.name, "account": account, "page_receipt": row.page_receipt }),
    )
    .await;
    match store::choose(&r.pool, &site, &concept, receipt.as_deref()).await {
        Ok(true) => {
            // The choice is made: the designers' rooms go now.
            if let Some(engine) = &r.engine
                && let Ok(runs) =
                    store::runs_with_rooms(&r.pool, Some(&site), chrono::Duration::zero()).await
            {
                for run in runs {
                    if let Err(e) = engine.delete_rooms(&run).await {
                        tracing::warn!(error = %e, "concepts: deleting rooms after the choice");
                    }
                }
            }
            Redirect::to(&format!("/sites/{site}/concepts#chosen")).into_response()
        }
        Ok(false) => (
            StatusCode::CONFLICT,
            "That design isn't finished, so it can't be chosen. Pick one that's ready.",
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "concepts: recording the choice");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Your choice wasn't saved on our side. Try again in a moment.",
            )
                .into_response()
        }
    }
}

async fn guide(
    State(r): State<Routes>,
    Path((site, concept)): Path<(String, String)>,
    headers: HeaderMap,
    Form(form): Form<PostForm>,
) -> Response {
    let gate = r.gate(&headers, &site).await;
    let Gate::Owner(account) = gate else {
        return r.refuse(gate, &site);
    };
    if !r.same_origin(&headers) {
        return forbidden("This request came from another site, so nothing was changed.");
    }
    let Some(engine) = &r.engine else {
        return Redirect::to(&format!("/sites/{site}/concepts")).into_response();
    };
    let Ok(Some(row)) = store::concept(&r.pool, &site, &concept).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !r.token_ok(&row, &form.token).await {
        return forbidden(
            "This page is out of date, so nothing was changed. Reload it and try again.",
        );
    }
    let words = form.words.trim();
    let back = format!("/sites/{site}/concepts#c-col-{}", row.n);
    if words.is_empty() {
        return Redirect::to(&back).into_response();
    }
    if words.chars().count() > MAX_CHANGE_CHARS {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "That's longer than a designer can take in one go. Keep it under 400 characters.",
        )
            .into_response();
    }
    if row.status != "done" || engine.page_in_room(&row.concept_id).is_none() {
        return (
            StatusCode::CONFLICT,
            "This design can't take changes now: it isn't finished, a look has been chosen, or it's more than 24 hours old. Try three more to keep designing.",
        )
            .into_response();
    }
    if let Some(live) = engine.live(&site, &row.run_id).await
        && live.busy(row.n)
    {
        return Redirect::to(&back).into_response();
    }
    match store::changes_since(&r.pool, &site, chrono::Duration::hours(24)).await {
        Ok(n) if n >= CHANGES_PER_DAY => {
            return r.render(
                StatusCode::TOO_MANY_REQUESTS,
                context! { mode => "enough-changes", site_id => site, runs => CHANGES_PER_DAY },
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "concepts: counting today's changes"),
    }
    let change_id = format!("cchg_{}", uuid::Uuid::now_v7().simple());
    if let Err(e) =
        store::insert_change(&r.pool, &change_id, &row.concept_id, &site, &account, words).await
    {
        tracing::error!(error = %e, "concepts: recording a change");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Your change wasn't sent on our side. Try again in a moment.",
        )
            .into_response();
    }
    if let Err(e) = engine.revise(&site, &row, &change_id, words).await {
        tracing::warn!(error = %e, "concepts: starting a change");
        let _ = store::change_finished(&r.pool, &change_id, false, "It didn't start.").await;
    }
    Redirect::to(&back).into_response()
}

async fn evidence_rows(
    State(r): State<Routes>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Gate::Owner(_) = r.gate(&headers, &site).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let rows = evidence::for_site(&r.pool, &site, r.limits)
        .await
        .unwrap_or_default();
    let mut res = r.render_template(
        "concepts/evidence.html",
        StatusCode::OK,
        context! { rows => rows },
    );
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}
