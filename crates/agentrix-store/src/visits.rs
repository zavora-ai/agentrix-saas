//! Visit counts without trackers (06 §4.9, 09 §5.3): the beacon endpoint, the daily totals
//! and the owner's report with small cells hidden.
//!
//! What is kept, per site and day: the route (never the URL), the referring site's host, a
//! device class, the country when the edge knows it, and the release seq. No cookie, no
//! identifier, no IP address, no user agent: the handler reads the user agent only to pick a
//! device class and drops it. Browsers sending Global Privacy Control are not counted.
//!
//! Table `visit_counts` (`migrations/control/0800_visit_counts.sql`, W8). Exact counts stay in
//! the table; [`VisitReport`] shows any cell under 5 as "fewer than 5".

use std::sync::Arc;

use agentrix_types::SiteId;
use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{AssertSqlSafe, SqlitePool};

use crate::time::Span;
use crate::words::Shown;

/// Where the beacon posts, on the site's own host.
pub const BEACON_PATH: &str = "/_agx/visit";

/// Where the counter script is served, on the site's own host. An external file, because the
/// kit's pages allow no inline script (`default-src 'self'`).
pub const SCRIPT_PATH: &str = "/_agx/visit.js";

/// The counter script (`static/visit.js`): sends the route after the page has loaded, never
/// when the browser sends Global Privacy Control or the visitor said no on this device. Any
/// element with `data-agx-count="off"` or `"on"` is the visitor's switch; elements marked
/// `data-visits="on"`, `"off"` or `"gpc"` are shown or hidden to match the true state, on load
/// and after every switch.
pub const VISIT_JS: &str = include_str!("../static/visit.js");

/// The tag a page or release carries, before `</body>`: `release_seq` is the release the page
/// came from, when there is one.
pub fn script_tag(release_seq: Option<i64>) -> String {
    match release_seq.filter(|s| *s >= 1) {
        Some(seq) => format!(r#"<script src="{SCRIPT_PATH}" data-seq="{seq}" defer></script>"#),
        None => format!(r#"<script src="{SCRIPT_PATH}" defer></script>"#),
    }
}

/// Path prefixes whose next segment is an access secret, collapsed to `:id` (an artifact's
/// id is its access secret, R10).
const PRIVATE_PREFIXES: &[&str] = &["a", "artifacts", "s", "share", "plan", "quote", "orders"];

/// At most this many routes per site and day; the rest count as `(other)`.
const ROUTES_PER_DAY: i64 = 500;

/// A route as counted: the path without query or fragment, private ids collapsed, at most
/// 8 segments and 200 characters. `None` for something that isn't a path.
pub fn route_of(path: &str) -> Option<String> {
    let path = path.split(['?', '#']).next().unwrap_or("");
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    let mut collapse_next = false;
    for segment in path.split('/').filter(|s| !s.is_empty()).take(8) {
        if collapse_next || looks_like_secret(segment) {
            out.push(":id".to_owned());
            collapse_next = false;
            continue;
        }
        if segment.chars().any(|c| c.is_control()) {
            return None;
        }
        collapse_next = PRIVATE_PREFIXES.contains(&segment);
        out.push(segment.to_owned());
    }
    let route = format!("/{}", out.join("/"));
    Some(route.chars().take(200).collect())
}

/// A long token of letters and digits (an id, a hash, a share key) rather than a word.
fn looks_like_secret(segment: &str) -> bool {
    segment.len() >= 16
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && segment.chars().filter(char::is_ascii_digit).count() >= 3
}

/// The referring site's host, without `www.`; empty for a direct visit or a page on the same
/// site.
pub fn referrer_host(referrer: &str, own_host: &str) -> String {
    let Ok(url) = url::Url::parse(referrer) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return String::new();
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let bare = |h: &str| h.trim_start_matches("www.").to_owned();
    let own = bare(own_host.split(':').next().unwrap_or(""));
    let host = bare(&host);
    if host == own { String::new() } else { host }
}

/// A device class from the user agent, which is then dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    /// A phone.
    Phone,
    /// A tablet.
    Tablet,
    /// A desktop or laptop.
    Desktop,
}

impl Device {
    /// Picks a class. `None` for crawlers and tools, which aren't counted.
    pub fn of(user_agent: &str) -> Option<Self> {
        let ua = user_agent.to_ascii_lowercase();
        if ua.is_empty()
            || [
                "bot",
                "crawl",
                "spider",
                "curl",
                "wget",
                "python",
                "headless",
                "lighthouse",
            ]
            .iter()
            .any(|w| ua.contains(w))
        {
            return None;
        }
        if ua.contains("ipad") || (ua.contains("android") && !ua.contains("mobile")) {
            Some(Self::Tablet)
        } else if ua.contains("mobi") || ua.contains("iphone") {
            Some(Self::Phone)
        } else {
            Some(Self::Desktop)
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Desktop => "desktop",
        }
    }
}

/// One counted visit, already stripped to what is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    /// The site.
    pub site: SiteId,
    /// The route ([`route_of`]).
    pub route: String,
    /// The referring host ([`referrer_host`]).
    pub referrer_host: String,
    /// The device class.
    pub device: Device,
    /// ISO 3166-1 alpha-2, when the edge knows it; the IP address is never kept.
    pub country: Option<String>,
    /// The release the page came from, when the beacon says.
    pub release_seq: Option<i64>,
}

/// One breakdown cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cell {
    /// The route, host, device or country ("" shows as `empty_words`).
    pub key: String,
    /// The count, small cells hidden.
    pub visits: Shown,
}

/// What the owner sees.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisitReport {
    /// The window.
    pub span: Span,
    /// Visits in the window.
    pub total: Shown,
    /// Top routes (at most 20); smaller ones are summed into `(other pages)`.
    pub by_route: Vec<Cell>,
    /// Referring sites (`""` is a direct visit).
    pub by_referrer: Vec<Cell>,
    /// Phone, tablet, desktop.
    pub by_device: Vec<Cell>,
    /// Countries (`""` when the edge doesn't know).
    pub by_country: Vec<Cell>,
    /// The first day anything was counted for the site.
    pub counting_since: Option<String>,
}

impl VisitReport {
    /// "41 visits today", "Fewer than 5 visits today", "No visits counted today".
    pub fn words(&self) -> String {
        match self.total {
            Shown::None => format!("No visits counted {}.", self.span.words),
            Shown::FewerThan5 => format!("Fewer than 5 visits {}.", self.span.words),
            Shown::Exact(_) => format!("{} visits {}.", self.total.words(), self.span.words),
        }
    }
}

/// The site a Host header names, and its offset from UTC.
#[async_trait]
pub trait HostSites: Send + Sync {
    /// The site serving `host` (port stripped, lower case), if any.
    async fn site_for_host(&self, host: &str) -> Option<(SiteId, i32)>;
}

/// Daily visit totals in `control.db`.
#[derive(Debug, Clone)]
pub struct VisitStore {
    pool: SqlitePool,
}

impl VisitStore {
    /// A store over `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Adds one visit to the day's totals for the site's local day.
    pub async fn record(
        &self,
        visit: &Visit,
        utc_offset_minutes: i32,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let day = local_day(now, utc_offset_minutes);
        let (routes_today,): (i64,) = sqlx::query_as(
            "SELECT COUNT(DISTINCT route) FROM visit_counts WHERE site_id = ? AND day = ?",
        )
        .bind(visit.site.as_str())
        .bind(&day)
        .fetch_one(&self.pool)
        .await?;
        let known: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM visit_counts WHERE site_id = ? AND day = ? AND route = ? LIMIT 1",
        )
        .bind(visit.site.as_str())
        .bind(&day)
        .bind(&visit.route)
        .fetch_optional(&self.pool)
        .await?;
        let route = if known.is_none() && routes_today >= ROUTES_PER_DAY {
            "(other)"
        } else {
            visit.route.as_str()
        };
        sqlx::query(
            "INSERT INTO visit_counts (site_id, day, route, referrer_host, device, country, \
             release_seq, visits) VALUES (?, ?, ?, ?, ?, ?, ?, 1) \
             ON CONFLICT (site_id, day, route, referrer_host, device, country, release_seq) \
             DO UPDATE SET visits = visits + 1",
        )
        .bind(visit.site.as_str())
        .bind(&day)
        .bind(route)
        .bind(&visit.referrer_host)
        .bind(visit.device.as_str())
        .bind(visit.country.as_deref().unwrap_or(""))
        .bind(visit.release_seq.unwrap_or(0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The first day anything was counted for the site, if ever.
    pub async fn counting_since(&self, site: &SiteId) -> Result<Option<String>, sqlx::Error> {
        let (first,): (Option<String>,) =
            sqlx::query_as("SELECT MIN(day) FROM visit_counts WHERE site_id = ?")
                .bind(site.as_str())
                .fetch_one(&self.pool)
                .await?;
        Ok(first)
    }

    /// The report for `span`, in the site's local days.
    pub async fn report(
        &self,
        site: &SiteId,
        span: Span,
        utc_offset_minutes: i32,
    ) -> Result<VisitReport, sqlx::Error> {
        let from = span
            .from
            .map(|t| local_day(t, utc_offset_minutes))
            .unwrap_or_default();
        let breakdown = |column: &'static str| {
            // `column` is one of four fixed names below, never caller text.
            let sql = format!(
                "SELECT {column}, SUM(visits) AS n FROM visit_counts \
                 WHERE site_id = ? AND day >= ? GROUP BY {column} ORDER BY n DESC, {column}"
            );
            let pool = self.pool.clone();
            let site = site.as_str().to_owned();
            let from = from.clone();
            async move {
                sqlx::query_as::<_, (String, i64)>(AssertSqlSafe(sql))
                    .bind(site)
                    .bind(from)
                    .fetch_all(&pool)
                    .await
            }
        };
        let routes = breakdown("route").await?;
        let total: i64 = routes.iter().map(|(_, n)| n).sum();
        Ok(VisitReport {
            span,
            total: Shown::of(total.max(0) as u64),
            by_route: cells(routes, 20, "(other pages)"),
            by_referrer: cells(breakdown("referrer_host").await?, 20, "(other sites)"),
            by_device: cells(breakdown("device").await?, 3, "(other)"),
            by_country: cells(breakdown("country").await?, 20, "(other countries)"),
            counting_since: self.counting_since(site).await?,
        })
    }
}

/// Top `keep` cells by count, the rest summed into one; small cells shown as "fewer than 5"
/// and listed after the others in name order, so their order leaks nothing.
fn cells(rows: Vec<(String, i64)>, keep: usize, rest: &str) -> Vec<Cell> {
    let mut big = Vec::new();
    let mut small = Vec::new();
    let mut other = 0i64;
    for (i, (key, n)) in rows.into_iter().enumerate() {
        if i >= keep {
            other += n;
        } else if (n as u64) < crate::words::SMALL_CELL {
            small.push(key);
        } else {
            big.push(Cell {
                key,
                visits: Shown::of(n as u64),
            });
        }
    }
    small.sort();
    big.extend(small.into_iter().map(|key| Cell {
        key,
        visits: Shown::FewerThan5,
    }));
    if other > 0 {
        big.push(Cell {
            key: rest.to_owned(),
            visits: Shown::of(other as u64),
        });
    }
    big
}

/// `YYYY-MM-DD` in a place `utc_offset_minutes` from UTC.
fn local_day(at: DateTime<Utc>, utc_offset_minutes: i32) -> String {
    let offset = FixedOffset::east_opt(utc_offset_minutes.saturating_mul(60))
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
    at.with_timezone(&offset).format("%Y-%m-%d").to_string()
}

/// Days of counts kept, then deleted by [`VisitStore::prune`]: 25 months, like model usage.
pub const KEEP_DAYS: i64 = 762;

impl VisitStore {
    /// Deletes totals older than [`KEEP_DAYS`]. Returns the rows removed.
    pub async fn prune(&self, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
        let cutoff = (now - Duration::days(KEEP_DAYS))
            .format("%Y-%m-%d")
            .to_string();
        let done = sqlx::query("DELETE FROM visit_counts WHERE day < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }
}

#[derive(Deserialize)]
struct Beacon {
    r: String,
    #[serde(default, rename = "ref")]
    referrer: String,
    #[serde(default)]
    seq: Option<i64>,
}

#[derive(Clone)]
struct BeaconState {
    store: VisitStore,
    sites: Arc<dyn HostSites>,
}

/// `POST /_agx/visit` and `GET /_agx/visit.js` for site hosts. Merge it into the router that
/// serves sites by Host (node's site router; W2's sample hosts).
///
/// The POST answers 204 whatever happens, so a page never learns whether it was counted. It
/// counts only same-origin beacons from browsers that don't send Global Privacy Control.
pub fn beacon_routes(store: VisitStore, sites: Arc<dyn HostSites>) -> Router {
    Router::new()
        .route(BEACON_PATH, post(beacon))
        .route(SCRIPT_PATH, get(visit_js))
        .with_state(BeaconState { store, sites })
}

async fn visit_js() -> axum::response::Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        VISIT_JS,
    )
        .into_response()
}

async fn beacon(State(state): State<BeaconState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    if let Err(why) = count(&state, &headers, &body).await {
        tracing::debug!(why, "visit not counted");
    }
    StatusCode::NO_CONTENT
}

async fn count(state: &BeaconState, headers: &HeaderMap, body: &[u8]) -> Result<(), String> {
    let text = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    if headers.get("sec-gpc").is_some_and(|v| v.as_bytes() == b"1") {
        return Err("global privacy control".to_owned());
    }
    if body.len() > 2048 {
        return Err("body too large".to_owned());
    }
    let host = text(header::HOST).to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or("").to_owned();
    // sendBeacon sends Origin on POST; anything from another origin is not this site's page.
    let origin = text(header::ORIGIN);
    let origin_host = url::Url::parse(&origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase));
    if origin_host.as_deref() != Some(host.as_str()) {
        return Err("not same-origin".to_owned());
    }
    let device = Device::of(&text(header::USER_AGENT)).ok_or("not a browser")?;
    let beacon: Beacon = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let route = route_of(&beacon.r).ok_or("not a path")?;
    let (site, offset) = state
        .sites
        .site_for_host(&host)
        .await
        .ok_or("unknown host")?;
    let visit = Visit {
        site,
        route,
        referrer_host: referrer_host(&beacon.referrer, &host),
        device,
        country: None,
        release_seq: beacon.seq.filter(|s| *s >= 1),
    };
    state
        .store
        .record(&visit, offset, Utc::now())
        .await
        .map_err(|e| e.to_string())
}

/// Finds a site with a function: for hosts whose sites live outside `control.db`, such as the
/// sample sites in `agentrix-samples`' own store.
pub struct FnHostSites<F>(pub F);

#[async_trait]
impl<F> HostSites for FnHostSites<F>
where
    F: Fn(&str) -> Option<(SiteId, i32)> + Send + Sync,
{
    async fn site_for_host(&self, host: &str) -> Option<(SiteId, i32)> {
        (self.0)(host)
    }
}

/// Finds a site by its domain in `sites`, with or without `www.`.
#[derive(Debug, Clone)]
pub struct SqliteHostSites {
    pool: SqlitePool,
}

impl SqliteHostSites {
    /// A lookup over `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HostSites for SqliteHostSites {
    async fn site_for_host(&self, host: &str) -> Option<(SiteId, i32)> {
        let bare = host.trim_start_matches("www.");
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT site_id, utc_offset_minutes FROM sites WHERE domain = ? OR domain = ? LIMIT 1",
        )
        .bind(host)
        .bind(bare)
        .fetch_optional(&self.pool)
        .await
        .ok()?;
        row.map(|(id, offset)| (SiteId::new(id), offset as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_not_urls() {
        assert_eq!(route_of("/").as_deref(), Some("/"));
        assert_eq!(route_of("/cakes?utm=x#top").as_deref(), Some("/cakes"));
        assert_eq!(route_of("/cakes/").as_deref(), Some("/cakes"));
        assert_eq!(route_of("/a/k29x-secret").as_deref(), Some("/a/:id"));
        assert_eq!(
            route_of("/plan/plan-k29x/share").as_deref(),
            Some("/plan/:id/share")
        );
        assert_eq!(
            route_of("/x/0f3a9c2b77e14d5a8e21").as_deref(),
            Some("/x/:id")
        );
        assert_eq!(
            route_of("/journal/scents-for-the-long-rains").as_deref(),
            Some("/journal/scents-for-the-long-rains")
        );
        assert_eq!(route_of("https://evil.example/x"), None);
        assert_eq!(route_of("//evil.example/x"), None);
    }

    #[test]
    fn referrers_are_hosts() {
        assert_eq!(
            referrer_host("https://www.google.com/search?q=cake", "kilele.example"),
            "google.com"
        );
        assert_eq!(
            referrer_host("https://kilele.example/cakes", "kilele.example:443"),
            ""
        );
        assert_eq!(referrer_host("", "kilele.example"), "");
        assert_eq!(referrer_host("android-app://x", "kilele.example"), "");
    }

    #[test]
    fn devices_and_crawlers() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148";
        assert_eq!(Device::of(iphone), Some(Device::Phone));
        assert_eq!(
            Device::of("Mozilla/5.0 (Linux; Android 14; SM-X200) Safari"),
            Some(Device::Tablet)
        );
        assert_eq!(
            Device::of("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)"),
            Some(Device::Desktop)
        );
        assert_eq!(Device::of("Googlebot/2.1"), None);
        assert_eq!(Device::of(""), None);
    }

    #[test]
    fn small_cells_sort_by_name_after_the_rest() {
        let rows = vec![
            ("/".to_owned(), 30),
            ("/cakes".to_owned(), 7),
            ("/visit".to_owned(), 4),
            ("/bread".to_owned(), 2),
        ];
        let out = cells(rows, 20, "(other pages)");
        let keys: Vec<_> = out.iter().map(|c| (c.key.as_str(), c.visits)).collect();
        assert_eq!(
            keys,
            vec![
                ("/", Shown::Exact(30)),
                ("/cakes", Shown::Exact(7)),
                ("/bread", Shown::FewerThan5),
                ("/visit", Shown::FewerThan5),
            ]
        );
        let many: Vec<_> = (0..25).map(|i| (format!("/p{i:02}"), 10)).collect();
        let out = cells(many, 20, "(other pages)");
        assert_eq!(out.len(), 21);
        assert_eq!(out[20].visits, Shown::Exact(50));
    }

    #[test]
    fn the_script_declares_only_its_fields() {
        assert!(VISIT_JS.contains("globalPrivacyControl"));
        assert!(VISIT_JS.contains(&format!("sendBeacon(\"{BEACON_PATH}\"")));
        assert!(VISIT_JS.contains("{ r: location.pathname, ref: document.referrer }"));
        assert!(!VISIT_JS.contains("location.href"), "routes, never URLs");
        // The page's own lines follow the true state on load, not only after a click.
        assert!(VISIT_JS.contains("[data-visits]"));
        assert!(VISIT_JS.contains(r#"[data-visits="gpc"]"#));
        assert_eq!(
            script_tag(Some(14)),
            r#"<script src="/_agx/visit.js" data-seq="14" defer></script>"#
        );
        assert_eq!(
            script_tag(None),
            r#"<script src="/_agx/visit.js" defer></script>"#
        );
    }

    #[tokio::test]
    async fn the_endpoint_counts_same_origin_browsers_for_sites_outside_control_db() {
        use axum::body::Body;
        use tower::ServiceExt;

        let pool = crate::testing::control_db().await;
        let store = VisitStore::new(pool.clone());
        // A sample site: not a row in control.db's `sites`.
        let sites = Arc::new(FnHostSites(|host: &str| {
            (host == "kilele-bakery-sample.agentrix.website")
                .then(|| (SiteId::new("kilele-bakery-sample"), 180))
        }));
        let app = beacon_routes(store.clone(), sites);
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148";
        let post = |origin: &str, gpc: bool| {
            let mut req = axum::http::Request::post(BEACON_PATH)
                .header("host", "kilele-bakery-sample.agentrix.website")
                .header("origin", origin)
                .header("user-agent", iphone);
            if gpc {
                req = req.header("sec-gpc", "1");
            }
            req.body(Body::from(
                r#"{"r":"/cakes?x=1","ref":"https://www.google.com/","seq":3}"#,
            ))
            .unwrap()
        };
        let same = "https://kilele-bakery-sample.agentrix.website";
        for (origin, gpc) in [(same, false), (same, true), ("https://evil.example", false)] {
            let res = app.clone().oneshot(post(origin, gpc)).await.unwrap();
            assert_eq!(res.status(), StatusCode::NO_CONTENT);
        }
        let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
            "SELECT site_id, route, referrer_host, release_seq, visits FROM visit_counts",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![(
                "kilele-bakery-sample".to_owned(),
                "/cakes".to_owned(),
                "google.com".to_owned(),
                3,
                1
            )],
            "one visit: the GPC and cross-origin beacons were not counted"
        );
        let js = app
            .oneshot(
                axum::http::Request::get(SCRIPT_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            js.headers()["content-type"],
            "text/javascript; charset=utf-8"
        );
    }
}
