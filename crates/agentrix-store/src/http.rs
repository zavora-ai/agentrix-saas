//! What node mounts from this crate (contract C11).
//!
//! Platform host (public):
//! - `GET /status`: the status page, refreshed every minute.
//! - `GET /status.json`: the same as JSON.
//! - `GET /api/numbers`: the home page's live numbers (sites live, Moves and their step
//!   times, gauntlet scores, receipts issued), each split by origin so a fixture or sample is
//!   never counted as a customer. Cached for a minute.
//!
//! Loopback only (`/internal/metrics/...`, JSON): the catalogue, sites, fleet counts, spend,
//! spend by site, Moves, step times, rooms, gauntlet, decisions waiting and receipt counts,
//! for operators and for W3's console while it is wired.
//!
//! Tasks: the health checks once a minute; retention for health hours and visit counts.

use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use agentrix_host::{Mounted, NodeContext};
use agentrix_types::SiteId;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::health::{Health, State as HealthState, StatusReport};
use crate::metrics::{Metrics, MoveQuery, Origin, Scope, SqliteMetrics};
use crate::time::Window;
use crate::visits::VisitStore;

const STATUS_TEMPLATE: &str = include_str!("../templates/store/status.html");
const STATUS_CSS: &str = include_str!("../static/status.css");
/// Changes with each release, so browsers fetch a changed stylesheet.
const STORE_V: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
struct AppState {
    metrics: Arc<dyn Metrics>,
    health: Health,
    numbers: Arc<Mutex<Option<(Instant, Value)>>>,
}

/// Mounts the status page, the home page's numbers, loopback reads and the background checks.
///
/// # Errors
///
/// Never today; the signature is node's [`agentrix_host::ModuleFn`].
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let metrics: Arc<dyn Metrics> = Arc::new(SqliteMetrics::from_env(ctx.pool.clone()));
    let health = Health::new(ctx.pool.clone(), ctx.http.clone(), ctx.models_url.clone());
    let state = AppState {
        metrics,
        health: health.clone(),
        numbers: Arc::new(Mutex::new(None)),
    };
    let public = Router::new()
        .route("/status", get(status_page))
        .route("/status.json", get(status_json))
        .route("/api/numbers", get(numbers))
        .route("/static/store/status.css", get(status_css))
        .with_state(state.clone());
    let internal = Router::new()
        .route("/internal/metrics/catalogue", get(catalogue))
        .route("/internal/metrics/sites", get(sites))
        .route("/internal/metrics/fleet", get(fleet))
        .route("/internal/metrics/fleet-by-origin", get(fleet_by_origin))
        .route("/internal/metrics/move-factory", get(move_factory))
        .route("/internal/metrics/waiting-by-site", get(waiting_by_site))
        .route("/internal/metrics/health", get(site_health))
        .route("/internal/metrics/spend", get(spend))
        .route("/internal/metrics/spend-by-site", get(spend_by_site))
        .route("/internal/metrics/moves", get(moves))
        .route("/internal/metrics/step-times", get(step_times))
        .route("/internal/metrics/rooms", get(rooms))
        .route("/internal/metrics/room-minutes", get(room_minutes))
        .route("/internal/metrics/gauntlet", get(gauntlet))
        .route("/internal/metrics/decisions", get(decisions))
        .route("/internal/metrics/receipts", get(receipt_counts))
        .route("/internal/health", get(health_now))
        .with_state(state);
    let visits = VisitStore::new(ctx.pool.clone());
    Ok(Mounted::default()
        .platform(public)
        .internal(internal)
        .task("health checks", Box::pin(health_loop(health)))
        .task("visit count retention", Box::pin(visit_retention(visits))))
}

async fn health_loop(health: Health) {
    let mut tick = tokio::time::interval(StdDuration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut runs: u64 = 0;
    loop {
        tick.tick().await;
        let now = Utc::now();
        if let Err(e) = health.run(now).await {
            tracing::warn!(error = %e, "recording health checks");
        }
        if runs.is_multiple_of(60)
            && let Err(e) = health.prune(now).await
        {
            tracing::warn!(error = %e, "pruning health checks");
        }
        runs = runs.wrapping_add(1);
    }
}

async fn visit_retention(visits: VisitStore) {
    let mut tick = tokio::time::interval(StdDuration::from_secs(6 * 3600));
    loop {
        tick.tick().await;
        if let Err(e) = visits.prune(Utc::now()).await {
            tracing::warn!(error = %e, "pruning visit counts");
        }
    }
}

fn internal_error(e: impl std::fmt::Display) -> Response {
    tracing::warn!(error = %e, "metrics read failed");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "The numbers can't be read right now. Try again in a minute.",
    )
        .into_response()
}

// ---------- status page ----------

#[derive(Serialize)]
struct DayView {
    class: &'static str,
    words: String,
}

#[derive(Serialize)]
struct PartView {
    name: &'static str,
    covers: &'static str,
    detail: String,
    state: &'static str,
    state_words: &'static str,
    uptime_words: String,
    days: Vec<DayView>,
}

/// The kit's `ui.status` state for a check's state.
fn kit_state(state: Option<HealthState>) -> &'static str {
    match state {
        Some(HealthState::Ok) => "ok",
        Some(HealthState::Degraded) => "waiting",
        Some(HealthState::Down) => "failed",
        None => "draft",
    }
}

async fn status_css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        STATUS_CSS,
    )
        .into_response()
}

fn class_of(state: Option<HealthState>) -> &'static str {
    match state {
        Some(HealthState::Ok) => "ok",
        Some(HealthState::Degraded) => "slow",
        Some(HealthState::Down) => "down",
        None => "none",
    }
}

/// "Sat 7 Nov" from `2026-11-07`.
fn day_words(day: &str) -> String {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|d| d.format("%a %-d %b").to_string())
        .unwrap_or_else(|_| day.to_owned())
}

fn render_status(report: &StatusReport) -> Result<String, minijinja::Error> {
    let parts: Vec<PartView> = report
        .parts
        .iter()
        .map(|p| PartView {
            name: p.name,
            covers: p.covers,
            detail: p.detail.clone(),
            state: kit_state(p.state),
            state_words: p.state.map_or("Not checked", HealthState::words),
            uptime_words: p.uptime_percent.map_or_else(
                || "Not measured yet".to_owned(),
                |u| format!("{u:.2}% working"),
            ),
            days: p
                .days
                .iter()
                .map(|d| DayView {
                    class: class_of(d.state),
                    words: match d.state {
                        None => format!("{}: not checked", day_words(&d.day)),
                        Some(HealthState::Ok) => format!("{}: working", day_words(&d.day)),
                        Some(HealthState::Degraded) => format!(
                            "{}: slower than usual for {}",
                            day_words(&d.day),
                            crate::words::duration_words(d.slow_minutes * 60_000)
                        ),
                        Some(HealthState::Down) => format!(
                            "{}: down for {}",
                            day_words(&d.day),
                            crate::words::duration_words(d.down_minutes * 60_000)
                        ),
                    },
                })
                .collect(),
        })
        .collect();
    let mut env = agentrix_ui::environment();
    env.add_template("store/status.html", STATUS_TEMPLATE)?;
    env.get_template("store/status.html")?
        .render(minijinja::context! {
            store_v => STORE_V,
            headline => report.headline,
            overall => class_of(report.overall),
            measured_since => report.measured_since.as_deref().map(day_words),
            parts => parts,
        })
}

async fn status_page(State(state): State<AppState>) -> Response {
    let report = match state.health.report(Utc::now()).await {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    match render_status(&report) {
        Ok(html) => ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn status_json(State(state): State<AppState>) -> Response {
    match state.health.report(Utc::now()).await {
        Ok(r) => ([(header::CACHE_CONTROL, "no-store")], Json(r)).into_response(),
        Err(e) => internal_error(e),
    }
}

// ---------- the home page's numbers ----------

async fn numbers(State(state): State<AppState>) -> Response {
    let mut cache = state.numbers.lock().await;
    if let Some((at, value)) = cache.as_ref()
        && at.elapsed() < StdDuration::from_secs(60)
    {
        return (
            [(header::CACHE_CONTROL, "public, max-age=60")],
            Json(value.clone()),
        )
            .into_response();
    }
    match home_numbers(state.metrics.as_ref()).await {
        Ok(value) => {
            *cache = Some((Instant::now(), value.clone()));
            ([(header::CACHE_CONTROL, "public, max-age=60")], Json(value)).into_response()
        }
        Err(e) => internal_error(e),
    }
}

async fn home_numbers(m: &dyn Metrics) -> crate::metrics::Result<Value> {
    let sites = m.sites().await?;
    let live = |origin: Origin| {
        sites
            .iter()
            .filter(|s| s.origin == origin && s.live_seq.is_some())
            .count()
    };
    let moves = m.moves(&MoveQuery::default()).await?;
    let moves_by = |origin: Origin, in_flight: bool| {
        moves
            .iter()
            .filter(|mv| mv.origin == origin && (!in_flight || mv.in_flight))
            .count()
    };
    let steps = m.step_times(Window::Last30Days).await?;
    let gauntlet = m.gauntlet(None).await?;
    let latest_run = gauntlet.iter().map(|g| g.started_at.clone()).max();
    let receipts = m.receipt_counts(&Scope::Everything, Window::All).await?;
    let reports = m.reports_written(Window::All).await?;
    Ok(json!({
        "sites_live": {
            "real": live(Origin::Real),
            "fixture": live(Origin::Fixture),
            "sample": live(Origin::Sample),
        },
        "moves": {
            "real": moves_by(Origin::Real, false),
            "fixture": moves_by(Origin::Fixture, false),
            "in_flight": moves.iter().filter(|mv| mv.in_flight).count(),
            "words": "Moves of Zavora's fixture site are real runs on a real WordPress copy, counted apart from customers' sites.",
        },
        "move_steps_last_30_days": steps.iter().map(|s| json!({
            "step": s.step,
            "runs": s.runs,
            "p50_ms": s.p50_ms,
            "p95_ms": s.p95_ms,
            "words": format!("{} usually takes {}", s.step, crate::words::duration_words(s.p50_ms)),
        })).collect::<Vec<_>>(),
        "gauntlet": {
            "stopped": gauntlet.iter().map(|g| g.score).sum::<i64>(),
            "tried": gauntlet.iter().map(|g| g.out_of).sum::<i64>(),
            "sites": gauntlet.len(),
            "last_run": latest_run,
        },
        "receipts_issued": receipts.total,
        "reports_written": reports,
        "updated_at": crate::time::stamp(Utc::now()),
    }))
}

// ---------- loopback reads ----------

#[derive(Deserialize)]
struct Params {
    site: Option<String>,
    window: Option<String>,
    in_flight: Option<bool>,
    limit: Option<u32>,
}

impl Params {
    fn window(&self) -> Window {
        match self.window.as_deref() {
            Some("today") => Window::Today,
            Some("24h") => Window::Last24Hours,
            Some("7d") => Window::Last7Days,
            Some("30d") => Window::Last30Days,
            Some("month") => Window::ThisMonth,
            Some("all") => Window::All,
            _ => Window::Today,
        }
    }

    fn scope(&self) -> Scope {
        match self.site.as_deref() {
            Some("*") => Scope::Everything,
            Some(site) => Scope::Site(SiteId::new(site)),
            None => Scope::Fleet,
        }
    }
}

fn json_or_error<T: Serialize>(read: crate::metrics::Result<T>) -> Response {
    match read {
        Ok(value) => Json(value).into_response(),
        Err(e) => internal_error(e),
    }
}

async fn catalogue() -> Response {
    let (sourced, partial, pending) = crate::catalogue::coverage();
    Json(json!({
        "sourced": sourced,
        "partial": partial,
        "pending": pending,
        "metrics": crate::catalogue::CATALOGUE,
    }))
    .into_response()
}

async fn sites(State(s): State<AppState>) -> Response {
    json_or_error(s.metrics.sites().await)
}

async fn fleet(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.fleet(p.window()).await)
}

async fn fleet_by_origin(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.fleet_by_origin(p.window()).await)
}

async fn move_factory(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.move_factory(p.window()).await)
}

async fn site_health(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    match p.site.as_deref() {
        Some(site) => json_or_error(s.metrics.site_health(&SiteId::new(site)).await),
        None => json_or_error(s.metrics.health_by_site().await),
    }
}

async fn waiting_by_site(State(s): State<AppState>) -> Response {
    json_or_error(s.metrics.waiting_by_site().await)
}

async fn spend(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.spend(&p.scope(), p.window()).await)
}

async fn spend_by_site(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.spend_by_site(p.window()).await)
}

async fn moves(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    let query = MoveQuery {
        site: p.site.as_deref().map(SiteId::new),
        in_flight: p.in_flight.unwrap_or(false),
        limit: p.limit,
    };
    json_or_error(s.metrics.moves(&query).await)
}

async fn step_times(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.step_times(p.window()).await)
}

async fn rooms(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.rooms(&p.scope(), p.window()).await)
}

async fn room_minutes(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.room_minutes(&p.scope(), p.window()).await)
}

async fn gauntlet(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    let site = p.site.as_deref().map(SiteId::new);
    json_or_error(s.metrics.gauntlet(site.as_ref()).await)
}

async fn decisions(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.decisions_waiting(&p.scope()).await)
}

async fn receipt_counts(State(s): State<AppState>, Query(p): Query<Params>) -> Response {
    json_or_error(s.metrics.receipt_counts(&p.scope(), p.window()).await)
}

async fn health_now(State(s): State<AppState>) -> Response {
    Json(s.health.check(Utc::now()).await).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util_shim::collect;
    use tower::ServiceExt;

    mod http_body_util_shim {
        pub async fn collect(body: axum::body::Body) -> String {
            let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }
    }

    async fn router() -> (Router, Router, sqlx::SqlitePool) {
        let pool = crate::testing::control_db().await;
        let mut ctx = agentrix_host::testing::context().await.unwrap();
        ctx.pool = pool.clone();
        let mounted = mount(&ctx).unwrap();
        assert_eq!(mounted.tasks.len(), 2);
        (mounted.platform, mounted.internal, pool)
    }

    async fn get(router: &Router, path: &str) -> (StatusCode, String) {
        let response = router
            .clone()
            .oneshot(axum::http::Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        (status, collect(response.into_body()).await)
    }

    #[tokio::test]
    async fn status_page_before_and_after_checks() {
        let (platform, _internal, pool) = router().await;
        let (code, html) = get(&platform, "/status").await;
        assert_eq!(code, StatusCode::OK);
        assert!(
            html.contains("Checks haven&#x27;t run yet.")
                || html.contains("Checks haven't run yet.")
        );
        assert!(html.contains("Not checked"));

        let health = Health::new(pool, reqwest::Client::new(), "http://127.0.0.1:9");
        health.run(Utc::now()).await.unwrap();
        let (_, html) = get(&platform, "/status").await;
        assert!(html.contains("Ask and answers is down."), "{html}");
        assert!(html.contains("Owner console and sign-in"));
        assert!(html.contains("100.00% working"));
        let (_, json) = get(&platform, "/status.json").await;
        let report: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(report["overall"], "down");
    }

    #[tokio::test]
    async fn home_numbers_and_loopback_reads() {
        let (platform, internal, _pool) = router().await;
        let (code, body) = get(&platform, "/api/numbers").await;
        assert_eq!(code, StatusCode::OK);
        let numbers: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(numbers["sites_live"]["real"], 0);
        assert_eq!(numbers["receipts_issued"], 0);
        assert!(
            numbers["move_steps_last_30_days"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let (code, body) = get(&internal, "/internal/metrics/catalogue").await;
        assert_eq!(code, StatusCode::OK);
        let cat: Value = serde_json::from_str(&body).unwrap();
        assert!(cat["metrics"].as_array().unwrap().len() >= 70);
        for path in [
            "/internal/metrics/sites",
            "/internal/metrics/fleet-by-origin?window=7d",
            "/internal/metrics/move-factory?window=30d",
            "/internal/metrics/waiting-by-site",
            "/internal/metrics/health",
            "/internal/metrics/health?site=site_x",
            "/internal/metrics/fleet?window=7d",
            "/internal/metrics/spend?window=month",
            "/internal/metrics/spend?site=site_x&window=today",
            "/internal/metrics/spend-by-site?window=30d",
            "/internal/metrics/moves?in_flight=true",
            "/internal/metrics/step-times?window=30d",
            "/internal/metrics/rooms",
            "/internal/metrics/room-minutes?site=*&window=month",
            "/internal/metrics/gauntlet",
            "/internal/metrics/decisions",
            "/internal/metrics/receipts?site=*&window=all",
        ] {
            let (code, body) = get(&internal, path).await;
            assert_eq!(code, StatusCode::OK, "{path}: {body}");
        }
    }

    #[test]
    fn days_read_as_people_say_them() {
        assert_eq!(day_words("2026-11-07"), "Sat 7 Nov");
    }
}
