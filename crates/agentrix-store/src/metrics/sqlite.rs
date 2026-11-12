//! [`Metrics`] over the first release's `control.db`, read-only.
//!
//! Every SQL string here is built from fixed fragments; caller values are always bound.

use std::collections::HashMap;
use std::sync::Arc;

use agentrix_types::SiteId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{AssertSqlSafe, SqlitePool};

use super::{
    DecisionsWaiting, FleetByOrigin, FleetCounts, GauntletScore, LimitUse, Metrics, MoveFactory,
    MoveQuery, MoveSummary, Origin, OriginCounts, Quantiles, Reading, ReceiptCounts, ReceiptQuery,
    ReceiptRow, Result, RoomKind, RoomMinutes, RoomStats, Scope, SiteSpend, SiteState, SiteSummary,
    SiteWaiting, SpendLine, SpendReport, StepTime, StepTimes, WaitingCard,
};
use crate::time::{Span, Window, millis_between, parse, stamp};
use crate::visits::{VisitReport, VisitStore};
use crate::words::MicroUsd;

/// Real sites only: not fixtures, and no host whose first label ends in `-sample`.
const REAL_SITES: &str = "SELECT site_id FROM sites WHERE fixture = 0 \
     AND substr(domain, 1, instr(domain || '.', '.') - 1) NOT LIKE '%-sample'";

/// The Move's steps in pipeline order (`move_steps.step`).
const STEP_ORDER: [&str; 7] = [
    "assess", "claim", "replay", "clean", "rebuild", "prove", "golive",
];

/// Move states after which nothing more happens.
const MOVE_ENDED: [&str; 4] = ["live", "rolled-back", "failed", "stopped"];

/// The gateway's default hard limit per site per day (`AGENTRIX_SITE_DAILY_CAP_USD`, 5).
const DEFAULT_DAILY_CAP: MicroUsd = MicroUsd(5_000_000);

type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The metric catalogue over `control.db`.
#[derive(Clone)]
pub struct SqliteMetrics {
    pool: SqlitePool,
    daily_cap: MicroUsd,
    clock: Clock,
}

impl std::fmt::Debug for SqliteMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteMetrics")
            .field("daily_cap", &self.daily_cap)
            .finish_non_exhaustive()
    }
}

impl SqliteMetrics {
    /// Reads `control.db` through `pool` (node's pool; nothing is written).
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            daily_cap: DEFAULT_DAILY_CAP,
            clock: Arc::new(Utc::now),
        }
    }

    /// Like [`SqliteMetrics::new`], with the daily cap read from `AGENTRIX_SITE_DAILY_CAP_USD`
    /// (the gateway's variable; $5 when unset or not a number).
    pub fn from_env(pool: SqlitePool) -> Self {
        let cap = std::env::var("AGENTRIX_SITE_DAILY_CAP_USD")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|usd| usd.is_finite() && *usd >= 0.0)
            .map_or(
                DEFAULT_DAILY_CAP,
                |usd| MicroUsd((usd * 1e6).round() as i64),
            );
        Self::new(pool).with_daily_cap(cap)
    }

    /// The gateway's hard daily limit per site, as `AGENTRIX_SITE_DAILY_CAP_USD` sets it for
    /// `agentrix-models`. Pass the same value, so the console shows the limit that is enforced.
    pub fn with_daily_cap(mut self, cap: MicroUsd) -> Self {
        self.daily_cap = cap;
        self
    }

    /// A fixed clock, for tests.
    pub fn with_clock(mut self, clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }

    async fn offset_of(&self, site: &SiteId) -> Result<i32> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT utc_offset_minutes FROM sites WHERE site_id = ?")
                .bind(site.as_str())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map_or(0, |(m,)| m as i32))
    }

    async fn span(&self, scope: &Scope, window: Window) -> Result<Span> {
        let offset = match scope {
            Scope::Fleet | Scope::Everything => 0,
            Scope::Site(site) => self.offset_of(site).await?,
        };
        Ok(window.span(self.now(), offset))
    }

    async fn load_sites(&self, one: Option<&SiteId>) -> Result<Vec<SiteSummary>> {
        type Row = (
            String,
            String,
            String,
            i64,
            i64,
            i64,
            String,
            Option<i64>,
            Option<String>,
            Option<String>,
        );
        let sql = format!(
            "SELECT s.site_id, s.domain, s.name, s.fixture, s.owner_account_id IS NOT NULL, \
             s.utc_offset_minutes, s.created_at, p.seq, \
             (SELECT MIN(r.signed_at) FROM releases r WHERE r.site_id = s.site_id \
                AND r.state <> 'candidate' AND r.signed_at IS NOT NULL), \
             (SELECT m.state FROM moves m WHERE m.site_id = s.site_id \
                ORDER BY m.created_at DESC, m.move_id DESC LIMIT 1) \
             FROM sites s LEFT JOIN site_pointer p ON p.site_id = s.site_id {} \
             ORDER BY s.created_at DESC, s.site_id",
            if one.is_some() {
                "WHERE s.site_id = ?"
            } else {
                ""
            }
        );
        let mut query = sqlx::query_as::<_, Row>(AssertSqlSafe(sql));
        if let Some(site) = one {
            query = query.bind(site.as_str());
        }
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(
                |(id, domain, name, fixture, claimed, offset, created, seq, live_since, mv)| {
                    let state = site_state(seq.is_some(), mv.as_deref(), claimed != 0);
                    SiteSummary {
                        site_id: SiteId::new(id),
                        origin: Origin::of(&domain, fixture != 0),
                        domain,
                        name,
                        state,
                        claimed: claimed != 0,
                        live_seq: seq,
                        live_since: live_since.as_deref().and_then(parse),
                        utc_offset_minutes: offset as i32,
                        created_at: parse(&created).unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                    }
                },
            )
            .collect())
    }

    async fn breakdown(
        &self,
        column: &'static str,
        filter: &str,
        site: Option<&str>,
        from: &str,
    ) -> Result<Vec<SpendLine>> {
        let label = if column == "u.purpose" {
            "MAX(up.words)"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT {column}, {label}, COALESCE(SUM(u.cost_micro_usd), 0) AS c, \
             COALESCE(SUM(u.outcome IN ('ok', 'error', 'replayed')), 0) \
             FROM usage u LEFT JOIN usage_purposes up ON up.purpose = u.purpose \
             WHERE {filter} AND u.at >= ? GROUP BY 1 ORDER BY c DESC, 1"
        );
        let mut query = sqlx::query_as::<_, (String, Option<String>, i64, i64)>(AssertSqlSafe(sql));
        if let Some(site) = site {
            query = query.bind(site.to_owned());
        }
        let rows = query.bind(from.to_owned()).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|(key, label, cost, calls)| SpendLine {
                key,
                label,
                cost: MicroUsd(cost),
                calls,
            })
            .collect())
    }

    async fn spent_since(&self, site: &str, purpose: &str, since: &str) -> Result<i64> {
        let (spent,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage \
             WHERE site_id = ? AND (? = '*' OR purpose = ?) AND at >= ?",
        )
        .bind(site)
        .bind(purpose)
        .bind(purpose)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(spent)
    }

    async fn limits(&self, site: &SiteId) -> Result<Vec<LimitUse>> {
        let now = self.now();
        let budgets: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT purpose, period, limit_micro_usd FROM budgets WHERE site_id = ? \
             ORDER BY purpose = '*' DESC, purpose",
        )
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        // The gateway counts budget periods in UTC (agentrix-models meter.rs), so these do too.
        let mut out = Vec::with_capacity(budgets.len() + 1);
        for (purpose, period, limit) in budgets {
            let since = match period.as_str() {
                "day" => Window::Today.span(now, 0).from_stamp(),
                "month" => Window::ThisMonth.span(now, 0).from_stamp(),
                _ => String::new(),
            };
            let spent = self.spent_since(site.as_str(), &purpose, &since).await?;
            out.push(LimitUse {
                purpose,
                period,
                limit: MicroUsd(limit),
                spent: MicroUsd(spent),
                hard: false,
            });
        }
        let today = Window::Today.span(now, 0).from_stamp();
        out.push(LimitUse {
            purpose: "*".to_owned(),
            period: "day".to_owned(),
            limit: self.daily_cap,
            spent: MicroUsd(self.spent_since(site.as_str(), "*", &today).await?),
            hard: true,
        });
        Ok(out)
    }

    async fn move_steps(&self, move_id: &str) -> Result<Vec<StepTimes>> {
        let rows: Vec<StepRow> = sqlx::query_as(
            "SELECT step, state, summary, receipt, started_at, ended_at FROM move_steps \
             WHERE move_id = ? ORDER BY seq",
        )
        .bind(move_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(fold_steps(rows, self.now()))
    }
}

type StepRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
);

/// One entry per step, in the order steps first appear; times from the first start to the
/// latest end. A step still running counts its time up to `now`.
fn fold_steps(rows: Vec<StepRow>, now: DateTime<Utc>) -> Vec<StepTimes> {
    let mut out: Vec<StepTimes> = Vec::new();
    for (step, state, summary, receipt, started_at, ended_at) in rows {
        match out.iter_mut().find(|s| s.step == step) {
            Some(entry) => {
                entry.state = state;
                if !summary.is_empty() {
                    entry.summary = summary;
                }
                if receipt.is_some() {
                    entry.receipt = receipt;
                }
                if ended_at.is_some() {
                    entry.ended_at = ended_at;
                }
            }
            None => out.push(StepTimes {
                step,
                state,
                summary,
                started_at,
                ended_at,
                millis: None,
                receipt,
            }),
        }
    }
    for entry in &mut out {
        entry.millis = match (&entry.ended_at, entry.state.as_str()) {
            (Some(end), _) => millis_between(&entry.started_at, end),
            (None, "running" | "waiting") => millis_between(&entry.started_at, &stamp(now)),
            _ => None,
        };
    }
    out
}

fn site_state(live: bool, move_state: Option<&str>, claimed: bool) -> SiteState {
    if live {
        return SiteState::Live;
    }
    match move_state {
        Some("rolled-back" | "live") => SiteState::RolledBack,
        Some("failed" | "stopped") => SiteState::Stopped,
        Some("awaiting-owner") => SiteState::AwaitingOwner,
        Some(_) => SiteState::Moving,
        None if claimed => SiteState::Claimed,
        None => SiteState::Assessed,
    }
}

/// The value at quantile `q` (0 to 1) of sorted values, nearest rank.
fn quantile(sorted: &[i64], q: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}

/// The SQL filter and bound site id for a scope over a table aliased `alias`.
fn scope_filter(scope: &Scope, alias: &str) -> (String, Option<String>) {
    match scope {
        Scope::Fleet => (format!("{alias}.site_id IN ({REAL_SITES})"), None),
        Scope::Everything => (format!("{alias}.site_id IS NOT NULL"), None),
        Scope::Site(site) => (
            format!("{alias}.site_id = ?"),
            Some(site.as_str().to_owned()),
        ),
    }
}

#[async_trait]
impl Metrics for SqliteMetrics {
    async fn sites(&self) -> Result<Vec<SiteSummary>> {
        self.load_sites(None).await
    }

    async fn site(&self, site: &SiteId) -> Result<Option<SiteSummary>> {
        Ok(self.load_sites(Some(site)).await?.into_iter().next())
    }

    async fn fleet(&self, window: Window) -> Result<FleetCounts> {
        let span = window.span(self.now(), 0);
        let mut counts = FleetCounts::default();
        for site in self.load_sites(None).await? {
            match site.origin {
                Origin::Sample => counts.samples += 1,
                Origin::Fixture => counts.fixtures += 1,
                Origin::Real => {
                    match site.state {
                        SiteState::Live => counts.live += 1,
                        SiteState::Moving => counts.moving += 1,
                        SiteState::AwaitingOwner => counts.awaiting_owner += 1,
                        SiteState::Claimed => counts.claimed += 1,
                        SiteState::Assessed => counts.assessed += 1,
                        SiteState::RolledBack | SiteState::Stopped => counts.stopped += 1,
                    }
                    if site.state == SiteState::Live
                        && site
                            .live_since
                            .is_some_and(|t| span.from.is_none_or(|from| t >= from))
                    {
                        counts.went_live_in_window += 1;
                    }
                }
            }
        }
        Ok(counts)
    }

    async fn fleet_by_origin(&self, window: Window) -> Result<FleetByOrigin> {
        let span = window.span(self.now(), 0);
        let sites = self.load_sites(None).await?;
        let moves: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT m.state, s.domain, s.fixture FROM moves m JOIN sites s ON s.site_id = m.site_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = FleetByOrigin {
            span: span.clone(),
            real: OriginCounts::default(),
            sample: OriginCounts::default(),
            fixture: OriginCounts::default(),
        };
        for site in &sites {
            let counts = match site.origin {
                Origin::Real => &mut out.real,
                Origin::Sample => &mut out.sample,
                Origin::Fixture => &mut out.fixture,
            };
            counts.sites += 1;
            counts.claimed += i64::from(site.claimed);
            if site.live_seq.is_some() {
                counts.live += 1;
                if site
                    .live_since
                    .is_some_and(|t| span.from.is_none_or(|from| t >= from))
                {
                    counts.went_live_in_window += 1;
                }
            }
        }
        for (state, domain, fixture) in moves {
            let counts = match Origin::of(&domain, fixture != 0) {
                Origin::Real => &mut out.real,
                Origin::Sample => &mut out.sample,
                Origin::Fixture => &mut out.fixture,
            };
            if !MOVE_ENDED.contains(&state.as_str()) {
                counts.moves_in_flight += 1;
            }
            match state.as_str() {
                "awaiting-owner" => counts.moves_waiting_owner += 1,
                "live" => counts.moves_live += 1,
                _ => {}
            }
        }
        Ok(out)
    }

    async fn move_factory(&self, window: Window) -> Result<MoveFactory> {
        let span = window.span(self.now(), 0);
        let ended = MOVE_ENDED
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(", ");
        // Each in-flight Move's last step row: the step it is on, or the next one if done.
        let sql = format!(
            "SELECT st.step, st.state FROM move_steps st JOIN moves m ON m.move_id = st.move_id \
             WHERE m.state NOT IN ({ended}) AND st.seq = \
             (SELECT MAX(seq) FROM move_steps x WHERE x.move_id = st.move_id)"
        );
        let last: Vec<(String, String)> = sqlx::query_as(AssertSqlSafe(sql))
            .fetch_all(&self.pool)
            .await?;
        let mut at_step: Vec<(String, i64)> =
            STEP_ORDER.iter().map(|s| ((*s).to_owned(), 0)).collect();
        for (step, state) in last {
            let Some(i) = STEP_ORDER.iter().position(|s| *s == step) else {
                continue;
            };
            let on = if state == "done" {
                (i + 1).min(STEP_ORDER.len() - 1)
            } else {
                i
            };
            at_step[on].1 += 1;
        }
        // From the Move's start to its first go-live row that waits for the owner (the card).
        let times: Vec<(String, String)> = sqlx::query_as(
            "SELECT m.created_at, (SELECT COALESCE(x.ended_at, x.started_at) FROM move_steps x \
                WHERE x.move_id = m.move_id AND x.step = 'golive' AND x.state IN ('waiting', 'done') \
                ORDER BY x.seq LIMIT 1) AS card \
             FROM moves m WHERE m.created_at >= ? AND card IS NOT NULL",
        )
        .bind(span.from_stamp())
        .fetch_all(&self.pool)
        .await?;
        let mut ms: Vec<i64> = times
            .iter()
            .filter_map(|(start, card)| millis_between(start, card))
            .collect();
        ms.sort_unstable();
        Ok(MoveFactory {
            span,
            at_step,
            to_go_live: Quantiles {
                n: ms.len() as i64,
                p50_ms: quantile(&ms, 0.5),
                p90_ms: quantile(&ms, 0.9),
                p95_ms: quantile(&ms, 0.95),
            },
        })
    }

    async fn gauntlet_runs(&self, window: Window) -> Result<i64> {
        let span = window.span(self.now(), 0);
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM gauntlet_runs WHERE started_at >= ?")
                .bind(span.from_stamp())
                .fetch_one(&self.pool)
                .await?;
        Ok(n)
    }

    async fn waiting_by_site(&self) -> Result<Vec<SiteWaiting>> {
        let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT d.site_id, s.domain, s.fixture, COUNT(*) AS n FROM decisions d \
             JOIN sites s ON s.site_id = d.site_id WHERE d.state = 'waiting' \
             GROUP BY d.site_id ORDER BY n DESC, s.domain",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, domain, fixture, count)| SiteWaiting {
                site_id: SiteId::new(id),
                origin: Origin::of(&domain, fixture != 0),
                domain,
                count,
            })
            .collect())
    }

    async fn spend(&self, scope: &Scope, window: Window) -> Result<SpendReport> {
        let span = self.span(scope, window).await?;
        let from = span.from_stamp();
        let (filter, site) = scope_filter(scope, "u");
        let sql = format!(
            "SELECT COALESCE(SUM(u.cost_micro_usd), 0), \
             COALESCE(SUM(u.outcome IN ('ok', 'error', 'replayed')), 0), \
             COALESCE(SUM(u.outcome = 'replayed'), 0), \
             COALESCE(SUM(u.outcome = 'over-budget'), 0), \
             COALESCE(SUM(u.outcome = 'error'), 0), \
             COALESCE(SUM(u.tokens_in), 0), \
             COALESCE(SUM(u.tokens_out + u.reasoning_tokens), 0) \
             FROM usage u WHERE {filter} AND u.at >= ?"
        );
        let mut totals =
            sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64)>(AssertSqlSafe(sql));
        if let Some(site) = &site {
            totals = totals.bind(site.clone());
        }
        let (cost, calls, replayed, refused, errors, tokens_in, tokens_out) =
            totals.bind(from.clone()).fetch_one(&self.pool).await?;
        let units_sql = format!(
            "SELECT u.unit, COALESCE(SUM(u.units), 0) FROM usage u \
             WHERE {filter} AND u.at >= ? AND u.unit IS NOT NULL GROUP BY u.unit ORDER BY u.unit"
        );
        let mut units = sqlx::query_as::<_, (String, i64)>(AssertSqlSafe(units_sql));
        if let Some(site) = &site {
            units = units.bind(site.clone());
        }
        let units = units.bind(from.clone()).fetch_all(&self.pool).await?;
        let site_ref = site.as_deref();
        Ok(SpendReport {
            cost: MicroUsd(cost),
            calls,
            ok: calls - replayed - errors,
            replayed,
            refused,
            errors,
            tokens_in,
            tokens_out,
            by_purpose: self
                .breakdown("u.purpose", &filter, site_ref, &from)
                .await?,
            by_provider: self
                .breakdown("u.provider", &filter, site_ref, &from)
                .await?,
            by_model: self.breakdown("u.model", &filter, site_ref, &from).await?,
            by_family: self
                .breakdown("COALESCE(up.family, 'other')", &filter, site_ref, &from)
                .await?,
            by_webagent: self
                .breakdown(
                    "u.webagent",
                    &format!("{filter} AND u.webagent IS NOT NULL"),
                    site_ref,
                    &from,
                )
                .await?,
            by_job: self
                .breakdown(
                    "u.job",
                    &format!("{filter} AND u.job IS NOT NULL"),
                    site_ref,
                    &from,
                )
                .await?,
            units,
            limits: match scope {
                Scope::Fleet | Scope::Everything => Vec::new(),
                Scope::Site(site) => self.limits(site).await?,
            },
            span,
        })
    }

    async fn spend_by_site(&self, window: Window) -> Result<Vec<SiteSpend>> {
        let now = self.now();
        let from = window.span(now, 0).from_stamp();
        let today = Window::Today.span(now, 0).from_stamp();
        type Row = (String, String, i64, i64, i64, i64, i64, i64, i64);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT s.site_id, s.domain, s.fixture, \
             COALESCE(SUM(CASE WHEN u.at >= ?1 THEN u.cost_micro_usd END), 0) AS c, \
             COALESCE(SUM(CASE WHEN u.at >= ?1 THEN u.outcome IN ('ok', 'error', 'replayed') END), 0), \
             COALESCE(SUM(CASE WHEN u.at >= ?1 THEN u.outcome = 'over-budget' END), 0), \
             COALESCE(SUM(CASE WHEN u.at >= ?2 THEN u.cost_micro_usd END), 0), \
             COALESCE(SUM(CASE WHEN u.at >= ?1 THEN u.outcome = 'replayed' END), 0), \
             COALESCE(SUM(CASE WHEN u.at >= ?1 THEN u.outcome = 'error' END), 0) \
             FROM usage u JOIN sites s ON s.site_id = u.site_id \
             WHERE u.at >= MIN(?1, ?2) GROUP BY s.site_id \
             HAVING c > 0 OR SUM(u.at >= ?1) > 0 ORDER BY c DESC, s.domain",
        )
        .bind(&from)
        .bind(&today)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, domain, fixture, cost, calls, refused, today, replayed, errors) in rows {
            let site_id = SiteId::new(id);
            let budgets = self
                .limits(&site_id)
                .await?
                .into_iter()
                .filter(|l| !l.hard)
                .collect();
            out.push(SiteSpend {
                origin: Origin::of(&domain, fixture != 0),
                site_id,
                domain,
                cost: MicroUsd(cost),
                calls,
                refused,
                replayed,
                errors,
                today: LimitUse {
                    purpose: "*".to_owned(),
                    period: "day".to_owned(),
                    limit: self.daily_cap,
                    spent: MicroUsd(today),
                    hard: true,
                },
                budgets,
            });
        }
        Ok(out)
    }

    async fn receipts(&self, site: &SiteId, query: &ReceiptQuery) -> Result<Vec<ReceiptRow>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let kinds = if query.kinds.is_empty() {
            String::new()
        } else {
            format!(" AND kind IN ({})", vec!["?"; query.kinds.len()].join(", "))
        };
        let before = if query.before.is_some() {
            " AND issued_at < ?"
        } else {
            ""
        };
        let ended_from = if query.ended_from.is_some() {
            " AND ended_at >= ?"
        } else {
            ""
        };
        let ended_before = if query.ended_before.is_some() {
            " AND ended_at < ?"
        } else {
            ""
        };
        let order = if query.ended_from.is_some() || query.ended_before.is_some() {
            "ended_at DESC"
        } else {
            "issued_at DESC"
        };
        let sql = format!(
            "SELECT digest, receipt_id, kind, subject_type, subject_id, issuer, issued_at, \
             json_extract(body, '$.detail.title'), ended_at, \
             COALESCE(json_extract(body, '$.detail.state'), json_extract(body, '$.detail.outcome'), '') \
                IN ('failed', 'error', 'stopped') \
             FROM receipts WHERE site_id = ?{kinds}{before}{ended_from}{ended_before} \
             ORDER BY {order}, digest DESC LIMIT ?"
        );
        type Row = (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            String,
            i64,
        );
        let mut q = sqlx::query_as::<_, Row>(AssertSqlSafe(sql)).bind(site.as_str());
        for kind in &query.kinds {
            q = q.bind(kind.clone());
        }
        if let Some(before) = &query.before {
            q = q.bind(before.clone());
        }
        if let Some(from) = query.ended_from {
            q = q.bind(stamp(from));
        }
        if let Some(before) = query.ended_before {
            q = q.bind(stamp(before));
        }
        let rows = q.bind(i64::from(limit)).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let (digest, receipt_id, kind, subject_type, subject_id, issuer, issued_at) =
                    (row.0, row.1, row.2, row.3, row.4, row.5, row.6);
                ReceiptRow {
                    digest,
                    receipt_id,
                    kind,
                    subject_type,
                    subject_id,
                    issuer,
                    issued_at,
                    title: if query.content_free { None } else { row.7 },
                    ended_at: row.8,
                    failed: row.9 != 0,
                }
            })
            .collect())
    }

    async fn receipt_counts(&self, scope: &Scope, window: Window) -> Result<ReceiptCounts> {
        let span = self.span(scope, window).await?;
        let (filter, site) = scope_filter(scope, "r");
        let sql = format!(
            "SELECT r.kind, COUNT(*) AS n, MAX(r.issued_at) FROM receipts r \
             WHERE {filter} AND r.issued_at >= ? GROUP BY r.kind ORDER BY n DESC, r.kind"
        );
        let mut q = sqlx::query_as::<_, (String, i64, String)>(AssertSqlSafe(sql));
        if let Some(site) = &site {
            q = q.bind(site.clone());
        }
        let rows = q.bind(span.from_stamp()).fetch_all(&self.pool).await?;
        let total = rows.iter().map(|(_, n, _)| n).sum();
        let newest = rows.iter().map(|(_, _, at)| at.clone()).max();
        let since_go_live = match scope {
            Scope::Fleet | Scope::Everything => None,
            Scope::Site(site) => match self.site(site).await?.and_then(|s| s.live_since) {
                None => None,
                Some(live) => {
                    let (n,): (i64,) = sqlx::query_as(
                        "SELECT COUNT(*) FROM receipts WHERE site_id = ? AND issued_at >= ?",
                    )
                    .bind(site.as_str())
                    .bind(stamp(live))
                    .fetch_one(&self.pool)
                    .await?;
                    Some(n)
                }
            },
        };
        Ok(ReceiptCounts {
            total,
            since_go_live,
            by_kind: rows.into_iter().map(|(k, n, _)| (k, n)).collect(),
            newest,
        })
    }

    async fn moves(&self, query: &MoveQuery) -> Result<Vec<MoveSummary>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let ended = MOVE_ENDED
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut filters = Vec::new();
        if query.site.is_some() {
            filters.push("m.site_id = ?".to_owned());
        }
        if query.in_flight {
            filters.push(format!("m.state NOT IN ({ended})"));
        }
        let filter = if filters.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", filters.join(" AND "))
        };
        let sql = format!(
            "SELECT m.move_id, m.site_id, s.domain, s.fixture, m.state, m.created_at, m.updated_at, \
             (SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage u WHERE u.move_id = m.move_id) \
             FROM moves m JOIN sites s ON s.site_id = m.site_id {filter} \
             ORDER BY m.created_at DESC, m.move_id DESC LIMIT ?"
        );
        type Row = (String, String, String, i64, String, String, String, i64);
        let mut q = sqlx::query_as::<_, Row>(AssertSqlSafe(sql));
        if let Some(site) = &query.site {
            q = q.bind(site.as_str().to_owned());
        }
        let rows = q.bind(i64::from(limit)).fetch_all(&self.pool).await?;
        let mut out = Vec::with_capacity(rows.len());
        for (move_id, site_id, domain, fixture, state, created_at, updated_at, cost) in rows {
            let steps = self.move_steps(&move_id).await?;
            let current_step = steps
                .iter()
                .rev()
                .find(|s| s.state != "done")
                .map(|s| s.step.clone());
            out.push(MoveSummary {
                in_flight: !MOVE_ENDED.contains(&state.as_str()),
                move_id,
                site_id: SiteId::new(site_id),
                origin: Origin::of(&domain, fixture != 0),
                domain,
                state,
                created_at,
                updated_at,
                steps,
                current_step,
                cost: MicroUsd(cost),
            });
        }
        Ok(out)
    }

    async fn step_times(&self, window: Window) -> Result<Vec<StepTime>> {
        let span = window.span(self.now(), 0);
        let rows: Vec<(String, String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT move_id, step, state, started_at, ended_at FROM move_steps \
             WHERE move_id IN (SELECT move_id FROM move_steps WHERE started_at >= ?) \
             ORDER BY move_id, seq",
        )
        .bind(span.from_stamp())
        .fetch_all(&self.pool)
        .await?;
        // (move, step) → (first start, latest state, latest end).
        let mut runs: HashMap<(String, String), (String, String, Option<String>)> = HashMap::new();
        for (move_id, step, state, started, ended) in rows {
            let entry = runs
                .entry((move_id, step))
                .or_insert_with(|| (started.clone(), state.clone(), None));
            entry.1 = state;
            if ended.is_some() {
                entry.2 = ended;
            }
        }
        let mut by_step: HashMap<String, Vec<i64>> = HashMap::new();
        for ((_, step), (started, state, ended)) in runs {
            if state != "done" {
                continue;
            }
            if let Some(ms) = ended
                .as_deref()
                .and_then(|end| millis_between(&started, end))
            {
                by_step.entry(step).or_default().push(ms);
            }
        }
        Ok(STEP_ORDER
            .iter()
            .filter_map(|step| {
                let mut times = by_step.remove(*step)?;
                times.sort_unstable();
                Some(StepTime {
                    step: (*step).to_owned(),
                    runs: times.len() as i64,
                    p50_ms: quantile(&times, 0.5)?,
                    p95_ms: quantile(&times, 0.95)?,
                })
            })
            .collect())
    }

    async fn rooms(&self, scope: &Scope, window: Window) -> Result<RoomStats> {
        let span = self.span(scope, window).await?;
        let (filter, site) = scope_filter(scope, "j");
        let now_sql = format!(
            "SELECT COALESCE(SUM(j.state = 'running'), 0), COALESCE(SUM(j.state = 'queued'), 0) \
             FROM jobs j WHERE {filter}"
        );
        let mut q = sqlx::query_as::<_, (i64, i64)>(AssertSqlSafe(now_sql));
        if let Some(site) = &site {
            q = q.bind(site.clone());
        }
        let (running, queued) = q.fetch_one(&self.pool).await?;
        let window_sql = format!(
            "SELECT j.kind, j.state, j.started_at, j.ended_at, j.created_at FROM jobs j \
             WHERE {filter} AND j.created_at >= ?"
        );
        type JobRow = (String, String, Option<String>, Option<String>, String);
        let mut q = sqlx::query_as::<_, JobRow>(AssertSqlSafe(window_sql));
        if let Some(site) = &site {
            q = q.bind(site.clone());
        }
        let rows = q.bind(span.from_stamp()).fetch_all(&self.pool).await?;
        let mut stats = RoomStats {
            running,
            queued,
            ..RoomStats::default()
        };
        let mut kinds: HashMap<String, (i64, i64, Vec<i64>, Vec<i64>)> = HashMap::new();
        for (kind, state, started, ended, created) in rows {
            let entry = kinds.entry(kind).or_default();
            entry.0 += 1;
            if let Some(ms) = started.as_deref().and_then(|s| millis_between(&created, s)) {
                entry.3.push(ms);
            }
            match state.as_str() {
                "done" => {
                    stats.done += 1;
                    if let (Some(s), Some(e)) = (started, ended)
                        && let Some(ms) = millis_between(&s, &e)
                    {
                        entry.2.push(ms);
                    }
                }
                "failed" => {
                    stats.failed += 1;
                    entry.1 += 1;
                }
                "killed" => {
                    stats.killed += 1;
                    entry.1 += 1;
                }
                _ => {}
            }
        }
        let mut by_kind: Vec<RoomKind> = kinds
            .into_iter()
            .map(|(kind, (jobs, not_done, mut times, mut waits))| {
                times.sort_unstable();
                waits.sort_unstable();
                RoomKind {
                    kind,
                    jobs,
                    not_done,
                    p50_ms: quantile(&times, 0.5),
                    p95_ms: quantile(&times, 0.95),
                    runs: times.len() as i64,
                    waits: waits.len() as i64,
                    wait_p50_ms: quantile(&waits, 0.5),
                    wait_p95_ms: quantile(&waits, 0.95),
                }
            })
            .collect();
        by_kind.sort_by(|a, b| b.jobs.cmp(&a.jobs).then(a.kind.cmp(&b.kind)));
        stats.by_kind = by_kind;
        Ok(stats)
    }

    async fn room_minutes(&self, scope: &Scope, window: Window) -> Result<Vec<RoomMinutes>> {
        let span = self.span(scope, window).await?;
        let now = span.to;
        let (filter, site) = scope_filter(scope, "j");
        let sql = format!(
            "SELECT j.site_id, s.domain, s.fixture, j.started_at, j.ended_at FROM jobs j \
             JOIN sites s ON s.site_id = j.site_id \
             WHERE {filter} AND j.started_at IS NOT NULL \
             AND (j.ended_at IS NULL OR j.ended_at >= ?)"
        );
        let mut q =
            sqlx::query_as::<_, (String, String, i64, String, Option<String>)>(AssertSqlSafe(sql));
        if let Some(site) = &site {
            q = q.bind(site.clone());
        }
        let rows = q.bind(span.from_stamp()).fetch_all(&self.pool).await?;
        let mut by_site: Vec<RoomMinutes> = Vec::new();
        for (site_id, domain, fixture, started, ended) in rows {
            let Some(start) = parse(&started) else {
                continue;
            };
            let end = ended.as_deref().and_then(parse).unwrap_or(now).min(now);
            let start = span.from.map_or(start, |from| start.max(from));
            let millis = (end - start).num_milliseconds();
            if millis <= 0 {
                continue;
            }
            match by_site.iter_mut().find(|r| r.site_id.as_str() == site_id) {
                Some(entry) => {
                    entry.jobs += 1;
                    entry.millis += millis;
                }
                None => by_site.push(RoomMinutes {
                    site_id: SiteId::new(site_id),
                    origin: Origin::of(&domain, fixture != 0),
                    domain,
                    jobs: 1,
                    millis,
                }),
            }
        }
        by_site.sort_by(|a, b| b.millis.cmp(&a.millis).then(a.domain.cmp(&b.domain)));
        Ok(by_site)
    }

    async fn gauntlet(&self, site: Option<&SiteId>) -> Result<Vec<GauntletScore>> {
        let sql = format!(
            "SELECT g.site_id, s.domain, g.score, g.out_of, g.started_at, g.ended_at, g.receipt, \
             EXISTS (SELECT 1 FROM site_pointer p WHERE p.site_id = g.site_id) \
             FROM gauntlet_runs g JOIN sites s ON s.site_id = g.site_id \
             WHERE g.run_id = (SELECT g2.run_id FROM gauntlet_runs g2 WHERE g2.site_id = g.site_id \
                AND g2.receipt IS NOT NULL ORDER BY g2.started_at DESC, g2.run_id DESC LIMIT 1) {} \
             ORDER BY g.started_at DESC",
            if site.is_some() {
                "AND g.site_id = ?"
            } else {
                ""
            }
        );
        type Row = (
            String,
            String,
            i64,
            i64,
            String,
            Option<String>,
            Option<String>,
            i64,
        );
        let mut q = sqlx::query_as::<_, Row>(AssertSqlSafe(sql));
        if let Some(site) = site {
            q = q.bind(site.as_str().to_owned());
        }
        Ok(q.fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(
                |(site_id, domain, score, out_of, started_at, ended_at, receipt, live)| {
                    GauntletScore {
                        site_id: SiteId::new(site_id),
                        domain,
                        score,
                        out_of,
                        started_at,
                        ended_at,
                        receipt,
                        live: live != 0,
                    }
                },
            )
            .collect())
    }

    async fn decisions_waiting(&self, scope: &Scope) -> Result<DecisionsWaiting> {
        let (filter, site) = scope_filter(scope, "d");
        let counts_sql = format!(
            "SELECT d.kind, COUNT(*) AS n FROM decisions d WHERE {filter} AND d.state = 'waiting' \
             GROUP BY d.kind ORDER BY n DESC, d.kind"
        );
        let mut q = sqlx::query_as::<_, (String, i64)>(AssertSqlSafe(counts_sql));
        if let Some(site) = &site {
            q = q.bind(site.clone());
        }
        let by_kind = q.fetch_all(&self.pool).await?;
        // Only the card's title: its body may name a customer (03-data.md §2.4).
        let cards_sql = format!(
            "SELECT d.decision_id, d.site_id, d.kind, \
             COALESCE(json_extract(d.card, '$.title'), ''), d.created_at \
             FROM decisions d WHERE {filter} AND d.state = 'waiting' \
             ORDER BY d.created_at, d.decision_id LIMIT 50"
        );
        let mut q =
            sqlx::query_as::<_, (String, String, String, String, String)>(AssertSqlSafe(cards_sql));
        if let Some(site) = &site {
            q = q.bind(site.clone());
        }
        let cards = q
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(
                |(decision_id, site_id, kind, title, created_at)| WaitingCard {
                    decision_id,
                    site_id: SiteId::new(site_id),
                    kind,
                    title,
                    created_at,
                },
            )
            .collect();
        let cards: Vec<WaitingCard> = cards;
        Ok(DecisionsWaiting {
            count: by_kind.iter().map(|(_, n)| n).sum(),
            oldest: cards.first().and_then(|c| parse(&c.created_at)),
            by_kind,
            cards,
        })
    }

    async fn reports_written(&self, window: Window) -> Result<i64> {
        let span = window.span(self.now(), 0);
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM assessments WHERE state = 'report-ready' \
             AND COALESCE(finished_at, created_at) >= ?",
        )
        .bind(span.from_stamp())
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn questions_passed(&self, site: &SiteId, window: Window) -> Result<i64> {
        let span = self.span(&Scope::Site(site.clone()), window).await?;
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM passed_questions WHERE site_id = ? AND passed_at >= ?",
        )
        .bind(site.as_str())
        .bind(span.from_stamp())
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    async fn site_health(&self, site: &SiteId) -> Result<Option<super::SiteHealth>> {
        super::health_score::site_health(&self.pool, site, self.now()).await
    }

    async fn health_by_site(&self) -> Result<Vec<super::SiteHealth>> {
        let live: Vec<(String,)> = sqlx::query_as("SELECT site_id FROM site_pointer")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(live.len());
        for (site,) in live {
            if let Some(h) =
                super::health_score::site_health(&self.pool, &SiteId::new(site), self.now()).await?
            {
                out.push(h);
            }
        }
        out.sort_by_key(|h| (h.score.is_none(), h.score.unwrap_or(0)));
        Ok(out)
    }

    async fn visits(&self, site: &SiteId, window: Window) -> Result<Reading<VisitReport>> {
        let store = VisitStore::new(self.pool.clone());
        if store.counting_since(site).await?.is_none() {
            return Ok(crate::catalogue::no_source("site.visits"));
        }
        let offset = self.offset_of(site).await?;
        let span = window.span(self.now(), offset);
        Ok(Reading::Value {
            value: store.report(site, span, offset).await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_states_from_rows() {
        assert_eq!(site_state(true, Some("rolled-back"), true), SiteState::Live);
        assert_eq!(
            site_state(false, Some("rolled-back"), true),
            SiteState::RolledBack
        );
        assert_eq!(site_state(false, Some("replayed"), true), SiteState::Moving);
        assert_eq!(
            site_state(false, Some("awaiting-owner"), true),
            SiteState::AwaitingOwner
        );
        assert_eq!(site_state(false, None, true), SiteState::Claimed);
        assert_eq!(site_state(false, None, false), SiteState::Assessed);
    }

    #[test]
    fn quantiles_are_nearest_rank() {
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(quantile(&[7], 0.95), Some(7));
        let v: Vec<i64> = (1..=20).collect();
        assert_eq!(quantile(&v, 0.5), Some(10));
        assert_eq!(quantile(&v, 0.95), Some(19));
    }
}
