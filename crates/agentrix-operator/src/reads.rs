//! Read-only views over the first release's `control.db` for the operator scope: content-free by
//! construction (08 §0 principle 5). These read ids, kinds, states, counts, times and money;
//! never card bodies, conversations, receipt titles or page content.
//!
//! Until W8's metric catalogue lands, the numbers come straight from the tables the first release writes
//! (sites, moves, move_steps, releases, site_pointer, jobs, decisions, receipts, usage, budgets,
//! gauntlet_runs, briefs). Each function names its source so the page can say where a number
//! comes from.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::ledger::time;

// The SELECT behind [`SiteRow`]; its first bind is the month's start.
macro_rules! site_select {
    () => {
        "SELECT s.site_id, s.domain, s.name, s.fixture, s.created_at, \
         a.display_name AS owner_name, a.email AS owner_email, \
         p.seq AS live_seq, (SELECT r.signed_at FROM releases r WHERE r.site_id = s.site_id AND r.seq = p.seq) AS live_signed_at, \
         (SELECT m.state FROM moves m WHERE m.site_id = s.site_id ORDER BY m.updated_at DESC LIMIT 1) AS move_state, \
         (SELECT m.updated_at FROM moves m WHERE m.site_id = s.site_id ORDER BY m.updated_at DESC LIMIT 1) AS move_updated_at, \
         (SELECT g.score FROM gauntlet_runs g WHERE g.site_id = s.site_id ORDER BY g.started_at DESC LIMIT 1) AS gauntlet_score, \
         (SELECT g.out_of FROM gauntlet_runs g WHERE g.site_id = s.site_id ORDER BY g.started_at DESC LIMIT 1) AS gauntlet_out_of, \
         (SELECT COUNT(*) FROM decisions d WHERE d.site_id = s.site_id AND d.state = 'waiting') AS decisions_waiting, \
         (SELECT COALESCE(SUM(u.cost_micro_usd), 0) FROM usage u WHERE u.site_id = s.site_id AND u.at >= ?1) AS spend_month, \
         (SELECT b.limit_micro_usd FROM budgets b WHERE b.site_id = s.site_id AND b.purpose = '*' AND b.period = 'month') AS budget_month, \
         (SELECT COUNT(*) FROM assessments x WHERE x.site_id = s.site_id) AS assessments, \
         (SELECT sub.plan_id FROM subscriptions sub WHERE sub.site_id = s.site_id ORDER BY sub.created_at DESC LIMIT 1) AS plan_id, \
         (SELECT sub.state FROM subscriptions sub WHERE sub.site_id = s.site_id ORDER BY sub.created_at DESC LIMIT 1) AS plan_state \
         FROM sites s LEFT JOIN accounts a ON a.id = s.owner_account_id LEFT JOIN site_pointer p ON p.site_id = s.site_id"
    };
}

// The ORDER BY and LIMIT for [`sites`].
macro_rules! site_order {
    () => {
        "ORDER BY (p.seq IS NULL), (s.owner_account_id IS NULL), s.created_at DESC LIMIT ?2"
    };
}

/// Parses a stored time.
pub fn parse(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The start of `now`'s month, UTC.
pub fn month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now)
}

/// Days in `now`'s month.
pub fn days_in_month(now: DateTime<Utc>) -> u32 {
    let (y, m) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    let next = Utc
        .with_ymd_and_hms(y, m, 1, 0, 0, 0)
        .single()
        .unwrap_or(now);
    u32::try_from((next - month_start(now)).num_days()).unwrap_or(30)
}

/// The value at `q` (0..1) of `values`, nearest rank.
pub fn percentile(values: &mut [f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let rank = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
    Some(values[rank - 1])
}

/// A site's kind, for labels.
pub fn kind_of(domain: &str, fixture: bool) -> &'static str {
    if fixture {
        "fixture"
    } else if domain
        .split('.')
        .next()
        .is_some_and(|label| label.ends_with("-sample"))
    {
        "sample"
    } else {
        "customer"
    }
}

// ---------- Fleet ----------

/// The Move's steps after the claim, in order, as the Fleet's "Migration Factory" stages.
/// A Move's `state` names the last step it finished, so the stage it is in is the next one.
pub const STAGES: [(&str, &[&str]); 5] = [
    ("Replay", &["claimed"]),
    ("Clean", &["replayed"]),
    ("Rebuild", &["cleaned"]),
    ("Prove", &["rebuilt"]),
    ("Go-live card", &["proved", "awaiting-owner"]),
];

/// Move states that count as in flight.
pub const IN_FLIGHT: [&str; 6] = [
    "claimed",
    "replayed",
    "cleaned",
    "rebuilt",
    "proved",
    "awaiting-owner",
];

/// Everything the Fleet page counts.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Fleet {
    /// Rows in `sites`: every address anyone assessed.
    pub sites_total: i64,
    /// Sites with an owner (a verified claim).
    pub sites_owned: i64,
    /// Sites the edge serves from a signed release (`site_pointer`).
    pub sites_live: i64,
    /// Of those, sample sites (`*-sample` hosts).
    pub sites_live_samples: i64,
    /// Sites whose first signed release went live in the last 7 days.
    pub sites_live_week: i64,
    /// Assessments in the last 7 days.
    pub assessments_week: i64,
    /// Moves between the claim and go-live.
    pub moves_in_flight: i64,
    /// Of those, waiting for the owner's go-live card.
    pub moves_waiting_owner: i64,
    /// Moves that went live, in all.
    pub moves_live: i64,
    /// Moves that failed or stopped in the last 7 days.
    pub moves_stopped_week: i64,
    /// Stage counts for the Migration Factory strip.
    pub stages: Vec<Stage>,
    /// Owner decision cards waiting, fleet-wide (a count; operators never see or approve them).
    pub owner_decisions_waiting: i64,
    /// Model spend this month.
    pub spend: Spend,
    /// Sealed-room jobs.
    pub rooms: Rooms,
    /// Latest gauntlet score per live site.
    pub gauntlet: Gauntlet,
    /// Work the platform did by itself in the last 24 hours.
    pub done_24h: Done,
    /// Approved owner cards with no receipt: an invariant that must stay 0.
    pub approved_without_receipt: i64,
    /// Receipts signed, in all.
    pub receipts_total: i64,
    /// The Move, from start to the go-live card, p90 over 30 days, in seconds.
    pub move_to_card_p90_s: Option<f64>,
    /// How many Moves that p90 rests on.
    pub move_to_card_n: usize,
    /// Model calls in the last 7 days and how they ended.
    pub calls_week: Calls,
    /// The platform's own minute-by-minute health checks (the status page's), over 30 days.
    pub checks: Checks,
}

/// The status page's health checks (agentrix-store's `health_hours`: per check and UTC hour,
/// the minutes it found working, slow or down). Read only; nothing here runs a check.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Checks {
    /// Whether this database has the table (it comes with agentrix-store's migrations).
    pub recorded: bool,
    /// The Model Gateway answering its health check, last 30 days.
    pub gateway_30d: Option<Check>,
    /// The same, last 7 days: minutes it took over 1.5 s count as slow.
    pub gateway_7d: Option<Check>,
}

/// One check over a window.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// Minutes checked.
    pub minutes: i64,
    /// Minutes slow (still answering).
    pub slow: i64,
    /// Minutes down.
    pub down: i64,
    /// Minutes answering (working or slow) out of minutes checked, as a percentage with two
    /// decimals, the status page's rule.
    pub percent: f64,
    /// The same in words: "100%", "96.66%".
    pub percent_words: String,
    /// The first hour counted, `YYYY-MM-DDTHH` (UTC).
    pub since: String,
}

/// Reads `check_id` from `health_hours` since `from`, or `None` when it has no minutes there.
async fn check(
    pool: &SqlitePool,
    check_id: &str,
    from: DateTime<Utc>,
) -> Result<Option<Check>, sqlx::Error> {
    let (ok, slow, down, since): (i64, i64, i64, Option<String>) = sqlx::query_as(
        "SELECT COALESCE(SUM(ok_minutes), 0), COALESCE(SUM(slow_minutes), 0),          COALESCE(SUM(down_minutes), 0), MIN(hour) FROM health_hours WHERE check_id = ? AND hour >= ?",
    )
    .bind(check_id)
    .bind(from.format("%Y-%m-%dT%H").to_string())
    .fetch_one(pool)
    .await?;
    let minutes = ok + slow + down;
    Ok(since.filter(|_| minutes > 0).map(|since| {
        let percent = ((ok + slow) as f64 * 10_000.0 / minutes as f64).floor() / 100.0;
        let words = format!("{percent:.2}");
        Check {
            minutes,
            slow,
            down,
            percent,
            percent_words: format!("{}%", words.trim_end_matches('0').trim_end_matches('.')),
            since,
        }
    }))
}

/// One hour of one health check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckHour {
    /// The hour's start, RFC 3339 (UTC), for the page's clock filter.
    pub at: String,
    /// Minutes working.
    pub ok: i64,
    /// Minutes slow.
    pub slow: i64,
    /// Minutes down.
    pub down: i64,
}

async fn has_health_table(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let (tables,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'health_hours'",
    )
    .fetch_one(pool)
    .await?;
    Ok(tables > 0)
}

/// `check_id`'s hours from `from` to `to`, oldest first, at most 48. `None` when this database
/// doesn't record health checks at all.
pub async fn check_hours(
    pool: &SqlitePool,
    check_id: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Option<Vec<CheckHour>>, sqlx::Error> {
    if !has_health_table(pool).await? {
        return Ok(None);
    }
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT hour, ok_minutes, slow_minutes, down_minutes FROM health_hours          WHERE check_id = ? AND hour >= ? AND hour <= ? ORDER BY hour LIMIT 48",
    )
    .bind(check_id)
    .bind(from.format("%Y-%m-%dT%H").to_string())
    .bind(to.format("%Y-%m-%dT%H").to_string())
    .fetch_all(pool)
    .await?;
    Ok(Some(
        rows.into_iter()
            .map(|(hour, ok, slow, down)| CheckHour {
                at: format!("{hour}:00:00Z"),
                ok,
                slow,
                down,
            })
            .collect(),
    ))
}

/// The health checks over the last 30 and 7 days, if this database records them.
pub async fn checks(pool: &SqlitePool, now: DateTime<Utc>) -> Result<Checks, sqlx::Error> {
    if !has_health_table(pool).await? {
        return Ok(Checks::default());
    }
    Ok(Checks {
        recorded: true,
        gateway_30d: check(pool, "ask", now - Duration::days(30)).await?,
        gateway_7d: check(pool, "ask", now - Duration::days(7)).await?,
    })
}

/// One Migration Factory stage.
#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    /// The step's name.
    pub label: &'static str,
    /// Moves in it now.
    pub count: i64,
}

/// Model spend this month, from the gateway's `usage` rows (cost estimates from the catalogue).
#[derive(Debug, Clone, Serialize, Default)]
pub struct Spend {
    /// Month to date, micro-USD.
    pub month_micro_usd: i64,
    /// A straight-line forecast for the whole month, micro-USD. Labelled as a forecast.
    pub forecast_micro_usd: i64,
    /// Sum of the sites' monthly budgets (`budgets`, purpose `*`, period `month`), micro-USD.
    pub month_budgets_micro_usd: i64,
    /// Sites with a monthly budget row.
    pub sites_with_month_budget: i64,
    /// By provider, most first.
    pub by_provider: Vec<Amount>,
    /// By purpose, most first.
    pub by_purpose: Vec<Amount>,
    /// The month's name, for the tile ("Nov").
    pub month: String,
}

/// A labelled amount.
#[derive(Debug, Clone, Serialize)]
pub struct Amount {
    /// What it is.
    pub label: String,
    /// Micro-USD.
    pub micro_usd: i64,
    /// Calls behind it.
    pub calls: i64,
}

/// Model calls and outcomes.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Calls {
    /// Every call.
    pub total: i64,
    /// Answered by a provider.
    pub ok: i64,
    /// Failed at the provider.
    pub error: i64,
    /// Refused by a budget.
    pub over_budget: i64,
    /// Answered from a recording.
    pub replayed: i64,
}

/// Sealed-room jobs (`jobs`), the runner pool's real load.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Rooms {
    /// Running now.
    pub running: i64,
    /// Queued now.
    pub queued: i64,
    /// Finished in the last 24 hours.
    pub done_24h: i64,
    /// Failed or killed in the last 24 hours.
    pub failed_24h: i64,
    /// Queue wait p95 over 7 days, seconds.
    pub wait_p95_s: Option<f64>,
    /// Run time p95 over 7 days, seconds.
    pub run_p95_s: Option<f64>,
    /// Jobs over 7 days.
    pub week: i64,
    /// Per kind over 7 days.
    pub by_kind: Vec<KindCount>,
}

/// Jobs of one kind.
#[derive(Debug, Clone, Serialize)]
pub struct KindCount {
    /// `fetch`, `replay`, `render` or `build`.
    pub kind: String,
    /// How many.
    pub count: i64,
    /// Run time p95, seconds.
    pub run_p95_s: Option<f64>,
}

/// The gauntlet across live sites.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Gauntlet {
    /// Live sites with a run.
    pub sites_run: i64,
    /// Of those, with a perfect last score.
    pub sites_perfect: i64,
    /// The lowest last score, as "score/out of".
    pub lowest: Option<String>,
    /// Runs in the last 24 hours.
    pub runs_24h: i64,
}

/// Work the platform did by itself.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Done {
    /// Chief briefs written.
    pub briefs: i64,
    /// `routing.changed` receipts.
    pub routing_receipts: i64,
    /// Receipts signed.
    pub receipts: i64,
    /// Gauntlet runs.
    pub gauntlet_runs: i64,
}

async fn count(pool: &SqlitePool, sql: &'static str, binds: &[&str]) -> Result<i64, sqlx::Error> {
    let mut q = sqlx::query_as::<_, (i64,)>(sql);
    for b in binds {
        q = q.bind(*b);
    }
    Ok(q.fetch_one(pool).await?.0)
}

fn secs(from: &str, to: &str) -> Option<f64> {
    let d = parse(to)? - parse(from)?;
    Some(d.num_milliseconds() as f64 / 1000.0)
}

/// Reads the Fleet page's numbers at `now`.
pub async fn fleet(pool: &SqlitePool, now: DateTime<Utc>) -> Result<Fleet, sqlx::Error> {
    let week_ago = time(now - Duration::days(7));
    let day_ago = time(now - Duration::days(1));
    let month_ago = time(now - Duration::days(30));
    let mut f = Fleet {
        sites_total: count(pool, "SELECT COUNT(*) FROM sites", &[]).await?,
        sites_owned: count(pool, "SELECT COUNT(*) FROM sites WHERE owner_account_id IS NOT NULL", &[]).await?,
        sites_live: count(pool, "SELECT COUNT(*) FROM site_pointer", &[]).await?,
        assessments_week: count(pool, "SELECT COUNT(*) FROM assessments WHERE created_at >= ?", &[&week_ago]).await?,
        sites_live_week: count(
            pool,
            "SELECT COUNT(*) FROM (SELECT r.site_id, MIN(r.signed_at) AS first FROM releases r \
             JOIN site_pointer p ON p.site_id = r.site_id WHERE r.signed_at IS NOT NULL GROUP BY r.site_id) WHERE first >= ?",
            &[&week_ago],
        )
        .await?,
        owner_decisions_waiting: count(pool, "SELECT COUNT(*) FROM decisions WHERE state = 'waiting'", &[]).await?,
        approved_without_receipt: count(pool, "SELECT COUNT(*) FROM decisions WHERE state = 'approved' AND receipt IS NULL", &[]).await?,
        receipts_total: count(pool, "SELECT COUNT(*) FROM receipts", &[]).await?,
        moves_stopped_week: count(pool, "SELECT COUNT(*) FROM moves WHERE state IN ('failed', 'stopped') AND updated_at >= ?", &[&week_ago]).await?,
        ..Fleet::default()
    };
    let live_domains: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.domain, s.fixture FROM site_pointer p JOIN sites s ON s.site_id = p.site_id",
    )
    .fetch_all(pool)
    .await?;
    f.sites_live_samples = live_domains
        .iter()
        .filter(|(d, fx)| kind_of(d, *fx != 0) == "sample")
        .count() as i64;

    // Moves.
    let states: Vec<(String, i64)> =
        sqlx::query_as("SELECT state, COUNT(*) FROM moves GROUP BY state")
            .fetch_all(pool)
            .await?;
    let by_state: BTreeMap<String, i64> = states.into_iter().collect();
    f.moves_in_flight = IN_FLIGHT
        .iter()
        .map(|s| by_state.get(*s).copied().unwrap_or(0))
        .sum();
    f.moves_waiting_owner = ["proved", "awaiting-owner"]
        .iter()
        .map(|s| by_state.get(*s).copied().unwrap_or(0))
        .sum();
    f.moves_live = by_state.get("live").copied().unwrap_or(0);
    f.stages = STAGES
        .iter()
        .map(|(label, states)| Stage {
            label,
            count: states
                .iter()
                .map(|s| by_state.get(*s).copied().unwrap_or(0))
                .sum(),
        })
        .collect();
    let to_card: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.created_at, MIN(st.started_at) FROM moves m JOIN move_steps st ON st.move_id = m.move_id \
         WHERE st.step = 'golive' AND m.created_at >= ? GROUP BY m.move_id",
    )
    .bind(&month_ago)
    .fetch_all(pool)
    .await?;
    let mut durations: Vec<f64> = to_card
        .iter()
        .filter_map(|(a, b)| secs(a, b))
        .filter(|d| *d >= 0.0)
        .collect();
    f.move_to_card_n = durations.len();
    f.move_to_card_p90_s = percentile(&mut durations, 0.9);

    // Spend.
    let month_from = time(month_start(now));
    let by_provider: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT provider, COALESCE(SUM(cost_micro_usd), 0), COUNT(*) FROM usage WHERE at >= ? GROUP BY provider ORDER BY 2 DESC",
    )
    .bind(&month_from)
    .fetch_all(pool)
    .await?;
    let by_purpose: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT purpose, COALESCE(SUM(cost_micro_usd), 0), COUNT(*) FROM usage WHERE at >= ? GROUP BY purpose ORDER BY 2 DESC",
    )
    .bind(&month_from)
    .fetch_all(pool)
    .await?;
    let month_micro: i64 = by_provider.iter().map(|(_, c, _)| c).sum();
    let elapsed_days = ((now - month_start(now)).num_seconds() as f64 / 86_400.0).max(1.0 / 24.0);
    let forecast =
        (month_micro as f64 / elapsed_days * f64::from(days_in_month(now))).round() as i64;
    let (budgets, budget_sites): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(limit_micro_usd), 0), COUNT(*) FROM budgets WHERE purpose = '*' AND period = 'month'",
    )
    .fetch_one(pool)
    .await?;
    let amount = |(label, micro_usd, calls): (String, i64, i64)| Amount {
        label,
        micro_usd,
        calls,
    };
    f.spend = Spend {
        month_micro_usd: month_micro,
        forecast_micro_usd: forecast.max(month_micro),
        month_budgets_micro_usd: budgets,
        sites_with_month_budget: budget_sites,
        by_provider: by_provider.into_iter().map(amount).collect(),
        by_purpose: by_purpose.into_iter().map(amount).collect(),
        month: now.format("%b").to_string(),
    };
    let outcomes: Vec<(String, i64)> =
        sqlx::query_as("SELECT outcome, COUNT(*) FROM usage WHERE at >= ? GROUP BY outcome")
            .bind(&week_ago)
            .fetch_all(pool)
            .await?;
    for (outcome, n) in outcomes {
        f.calls_week.total += n;
        match outcome.as_str() {
            "ok" => f.calls_week.ok += n,
            "error" => f.calls_week.error += n,
            "over-budget" => f.calls_week.over_budget += n,
            "replayed" => f.calls_week.replayed += n,
            _ => {}
        }
    }

    // Sealed rooms.
    f.rooms.running = count(
        pool,
        "SELECT COUNT(*) FROM jobs WHERE state = 'running'",
        &[],
    )
    .await?;
    f.rooms.queued = count(
        pool,
        "SELECT COUNT(*) FROM jobs WHERE state = 'queued'",
        &[],
    )
    .await?;
    f.rooms.done_24h = count(
        pool,
        "SELECT COUNT(*) FROM jobs WHERE state = 'done' AND ended_at >= ?",
        &[&day_ago],
    )
    .await?;
    f.rooms.failed_24h = count(
        pool,
        "SELECT COUNT(*) FROM jobs WHERE state IN ('failed', 'killed') AND ended_at >= ?",
        &[&day_ago],
    )
    .await?;
    let jobs: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT kind, created_at, started_at, ended_at FROM jobs WHERE created_at >= ?",
    )
    .bind(&week_ago)
    .fetch_all(pool)
    .await?;
    f.rooms.week = jobs.len() as i64;
    let mut waits: Vec<f64> = jobs
        .iter()
        .filter_map(|(_, c, s, _)| secs(c, s.as_deref()?))
        .collect();
    let mut runs: Vec<f64> = jobs
        .iter()
        .filter_map(|(_, _, s, e)| secs(s.as_deref()?, e.as_deref()?))
        .collect();
    f.rooms.wait_p95_s = percentile(&mut waits, 0.95);
    f.rooms.run_p95_s = percentile(&mut runs, 0.95);
    let mut kinds: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut kind_counts: BTreeMap<String, i64> = BTreeMap::new();
    for (kind, _, s, e) in &jobs {
        *kind_counts.entry(kind.clone()).or_default() += 1;
        if let Some(d) = s.as_deref().zip(e.as_deref()).and_then(|(s, e)| secs(s, e)) {
            kinds.entry(kind.clone()).or_default().push(d);
        }
    }
    f.rooms.by_kind = kind_counts
        .into_iter()
        .map(|(kind, count)| {
            let run_p95_s = kinds.get_mut(&kind).and_then(|v| percentile(v, 0.95));
            KindCount {
                kind,
                count,
                run_p95_s,
            }
        })
        .collect();

    // Gauntlet: the newest run per live site.
    let last: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT g.score, g.out_of FROM gauntlet_runs g JOIN site_pointer p ON p.site_id = g.site_id \
         WHERE g.started_at = (SELECT MAX(started_at) FROM gauntlet_runs h WHERE h.site_id = g.site_id)",
    )
    .fetch_all(pool)
    .await?;
    f.gauntlet.sites_run = last.len() as i64;
    f.gauntlet.sites_perfect = last.iter().filter(|(s, o)| s == o && *o > 0).count() as i64;
    f.gauntlet.lowest = last
        .iter()
        .min_by(|a, b| (a.0 * b.1).cmp(&(b.0 * a.1)))
        .map(|(s, o)| format!("{s}/{o}"));
    f.gauntlet.runs_24h = count(
        pool,
        "SELECT COUNT(*) FROM gauntlet_runs WHERE started_at >= ?",
        &[&day_ago],
    )
    .await?;

    // Done by the platform.
    f.done_24h = Done {
        briefs: count(
            pool,
            "SELECT COUNT(*) FROM briefs WHERE written_at >= ?",
            &[&day_ago],
        )
        .await?,
        routing_receipts: count(
            pool,
            "SELECT COUNT(*) FROM receipts WHERE kind = 'routing.changed' AND issued_at >= ?",
            &[&day_ago],
        )
        .await?,
        receipts: count(
            pool,
            "SELECT COUNT(*) FROM receipts WHERE issued_at >= ?",
            &[&day_ago],
        )
        .await?,
        gauntlet_runs: f.gauntlet.runs_24h,
    };
    f.checks = checks(pool, now).await?;
    Ok(f)
}

/// The gateway's own model calls this month by site and provider, micro-USD (catalogue-price
/// estimates), for the Money page's Vultr column: what the gateway says it sent to Vultr.
pub async fn provider_spend(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> Result<BTreeMap<String, BTreeMap<String, (i64, i64)>>, sqlx::Error> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT site_id, provider, COALESCE(SUM(cost_micro_usd), 0), COUNT(*) FROM usage WHERE at >= ? GROUP BY 1, 2",
    )
    .bind(time(month_start(now)))
    .fetch_all(pool)
    .await?;
    let mut out: BTreeMap<String, BTreeMap<String, (i64, i64)>> = BTreeMap::new();
    for (site, provider, micro, calls) in rows {
        out.entry(site)
            .or_default()
            .insert(provider, (micro, calls));
    }
    Ok(out)
}

// ---------- Customers: every site ----------

/// One site in the Customers list.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SiteRow {
    /// `site_…`.
    pub site_id: String,
    /// Its address.
    pub domain: String,
    /// Its name, if Assess found one.
    pub name: String,
    /// Zavora's own fixture.
    pub fixture: i64,
    /// When the site row was made (first Assess).
    pub created_at: String,
    /// The owner's name, once claimed.
    pub owner_name: Option<String>,
    /// The owner's email.
    pub owner_email: Option<String>,
    /// The live release's number.
    pub live_seq: Option<i64>,
    /// When the live release was signed.
    pub live_signed_at: Option<String>,
    /// The newest Move's state.
    pub move_state: Option<String>,
    /// When it last changed.
    pub move_updated_at: Option<String>,
    /// The newest gauntlet score.
    pub gauntlet_score: Option<i64>,
    /// Out of.
    pub gauntlet_out_of: Option<i64>,
    /// Owner cards waiting.
    pub decisions_waiting: i64,
    /// Model spend this month, micro-USD.
    pub spend_month: i64,
    /// The site's monthly budget for every purpose, if set.
    pub budget_month: Option<i64>,
    /// Assessments run.
    pub assessments: i64,
    /// The newest subscription's plan (agentrix-billing), if any.
    pub plan_id: Option<String>,
    /// That subscription's state.
    pub plan_state: Option<String>,
}

impl SiteRow {
    /// `sample`, `fixture` or `customer`.
    pub fn kind(&self) -> &'static str {
        kind_of(&self.domain, self.fixture != 0)
    }

    /// The plan's name and the subscription state in words, e.g. "Business · active".
    pub fn plan_words(&self) -> Option<String> {
        let id = self.plan_id.as_deref()?;
        let name = agentrix_billing::plan(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.to_owned());
        let state = match self.plan_state.as_deref() {
            Some("pending_approval") => "waiting on PayPal",
            Some("past_due") => "payment late",
            Some(other) => other,
            None => "",
        };
        Some(if state.is_empty() {
            name
        } else {
            format!("{name} · {state}")
        })
    }

    /// The lifecycle stage (08 §4.1), from what the records show.
    pub fn stage(&self) -> &'static str {
        if self.live_seq.is_some() {
            "Live"
        } else if self
            .move_state
            .as_deref()
            .is_some_and(|s| IN_FLIGHT.contains(&s))
        {
            "Moving"
        } else if self.owner_name.is_some() {
            "Kept"
        } else {
            "Lead"
        }
    }
}

/// Which sites to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Live, moving or claimed.
    Customers,
    /// Assessed but never claimed.
    Leads,
    /// Everything.
    All,
}

/// Sites for the Customers page, live first, then newest. Up to `limit`.
pub async fn sites(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    filter: Filter,
    limit: i64,
) -> Result<Vec<SiteRow>, sqlx::Error> {
    let sql = match filter {
        Filter::Customers => concat!(
            site_select!(),
            " WHERE s.owner_account_id IS NOT NULL OR p.seq IS NOT NULL ",
            site_order!()
        ),
        Filter::Leads => concat!(
            site_select!(),
            " WHERE s.owner_account_id IS NULL AND p.seq IS NULL ",
            site_order!()
        ),
        Filter::All => concat!(site_select!(), " ", site_order!()),
    };
    sqlx::query_as(sql)
        .bind(time(month_start(now)))
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// One site's row.
pub async fn site_row(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    site_id: &str,
) -> Result<Option<SiteRow>, sqlx::Error> {
    sqlx::query_as(concat!(site_select!(), " WHERE s.site_id = ?2 LIMIT 1"))
        .bind(time(month_start(now)))
        .bind(site_id)
        .fetch_optional(pool)
        .await
}

/// Counts for the Customers tabs.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SiteCounts {
    /// Live, moving or claimed.
    pub customers: i64,
    /// Assessed only.
    pub leads: i64,
    /// Everything.
    pub all: i64,
}

/// Counts for the Customers tabs.
pub async fn site_counts(pool: &SqlitePool) -> Result<SiteCounts, sqlx::Error> {
    let all = count(pool, "SELECT COUNT(*) FROM sites", &[]).await?;
    let customers = count(
        pool,
        "SELECT COUNT(*) FROM sites s LEFT JOIN site_pointer p ON p.site_id = s.site_id WHERE s.owner_account_id IS NOT NULL OR p.seq IS NOT NULL",
        &[],
    )
    .await?;
    Ok(SiteCounts {
        customers,
        leads: all - customers,
        all,
    })
}

// ---------- One site ----------

/// A Move and its steps.
#[derive(Debug, Clone, Serialize)]
pub struct MoveRow {
    /// `move_…`.
    pub move_id: String,
    /// Its state.
    pub state: String,
    /// Started.
    pub created_at: String,
    /// Last change.
    pub updated_at: String,
    /// Steps, in order.
    pub steps: Vec<StepRow>,
    /// Model spend on this Move, micro-USD.
    pub spend_micro_usd: i64,
}

/// One Move step.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StepRow {
    /// Order.
    pub seq: i64,
    /// `replay`, `clean`, …
    pub step: String,
    /// `done`, `failed`, …
    pub state: String,
    /// Started.
    pub started_at: String,
    /// Ended.
    pub ended_at: Option<String>,
    /// The step's receipt digest.
    pub receipt: Option<String>,
}

/// A release.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ReleaseRow {
    /// Its number.
    pub seq: i64,
    /// `candidate`, `live`, `superseded`, `rolled-back`.
    pub state: String,
    /// The manifest's digest.
    pub manifest_digest: String,
    /// Made.
    pub created_at: String,
    /// Signed.
    pub signed_at: Option<String>,
}

/// A sealed-room job, content-free.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct JobRow {
    /// `job_…`.
    pub job_id: String,
    /// `fetch`, `replay`, `render`, `build`.
    pub kind: String,
    /// `queued`, `running`, `done`, `failed`, `killed`.
    pub state: String,
    /// Queued.
    pub created_at: String,
    /// Started.
    pub started_at: Option<String>,
    /// Ended.
    pub ended_at: Option<String>,
    /// Receipt digest.
    pub receipt: Option<String>,
}

/// A receipt's index entry: kind, subject and digest, never its title or body.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ReceiptRow {
    /// `sha256:…`.
    pub digest: String,
    /// The kind, e.g. `release.signed`.
    pub kind: String,
    /// What it is about.
    pub subject_type: String,
    /// Its id.
    pub subject_id: String,
    /// Issued.
    pub issued_at: String,
}

/// An owner decision card, content-free: kind and state only.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DecisionRow {
    /// `dec_…`.
    pub decision_id: String,
    /// `go-live`, `quarantine-restore`, `order-held`.
    pub kind: String,
    /// `waiting`, `approved`, `declined`, `expired`.
    pub state: String,
    /// `passkey` or `totp` once decided.
    pub method: Option<String>,
    /// Raised.
    pub created_at: String,
    /// Decided.
    pub decided_at: Option<String>,
}

/// A gauntlet run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GauntletRow {
    /// `run_…`.
    pub run_id: String,
    /// Attacks held.
    pub score: i64,
    /// Attacks tried.
    pub out_of: i64,
    /// Started.
    pub started_at: String,
    /// Receipt digest.
    pub receipt: Option<String>,
}

/// A budget row.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BudgetRow {
    /// A purpose or `*`.
    pub purpose: String,
    /// `day`, `month` or `total`.
    pub period: String,
    /// Micro-USD.
    pub limit_micro_usd: i64,
    /// Spent in the current period, micro-USD.
    pub spent_micro_usd: i64,
}

/// Spend by purpose and provider.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SpendRow {
    /// Purpose.
    pub purpose: String,
    /// Provider.
    pub provider: String,
    /// Calls.
    pub calls: i64,
    /// Tokens in plus out.
    pub tokens: i64,
    /// Micro-USD.
    pub micro_usd: i64,
}

/// Everything the site page shows.
#[derive(Debug, Clone, Serialize)]
pub struct SiteDetail {
    /// The list row.
    pub row: SiteRow,
    /// Its kind.
    pub kind: &'static str,
    /// Its lifecycle stage.
    pub stage: &'static str,
    /// The old site's origin.
    pub old_origin: String,
    /// Moves, newest first.
    pub moves: Vec<MoveRow>,
    /// Releases, newest first.
    pub releases: Vec<ReleaseRow>,
    /// Recent sealed-room jobs.
    pub jobs: Vec<JobRow>,
    /// Receipt counts by kind.
    pub receipt_kinds: Vec<(String, i64)>,
    /// Recent receipts.
    pub receipts: Vec<ReceiptRow>,
    /// Owner cards.
    pub decisions: Vec<DecisionRow>,
    /// Gauntlet runs.
    pub gauntlet: Vec<GauntletRow>,
    /// Budgets with the current period's spend.
    pub budgets: Vec<BudgetRow>,
    /// This month's spend by purpose and provider.
    pub spend_month: Vec<SpendRow>,
    /// All-time spend, micro-USD.
    pub spend_total: i64,
    /// Items left behind in the Move.
    pub quarantined: i64,
    /// Findings by severity.
    pub findings: Vec<(String, i64)>,
}

fn period_from(period: &str, now: DateTime<Utc>) -> String {
    match period {
        "day" => time(
            Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
                .single()
                .unwrap_or(now),
        ),
        "month" => time(month_start(now)),
        _ => time(DateTime::<Utc>::UNIX_EPOCH),
    }
}

/// Reads one site's page.
pub async fn site(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    site_id: &str,
) -> Result<Option<SiteDetail>, sqlx::Error> {
    let Some(row) = site_row(pool, now, site_id).await? else {
        return Ok(None);
    };
    let (old_origin,): (String,) = sqlx::query_as("SELECT old_origin FROM sites WHERE site_id = ?")
        .bind(site_id)
        .fetch_one(pool)
        .await?;
    let moves_raw: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT move_id, state, created_at, updated_at FROM moves WHERE site_id = ? ORDER BY created_at DESC")
            .bind(site_id)
            .fetch_all(pool)
            .await?;
    let mut moves = Vec::new();
    for (move_id, state, created_at, updated_at) in moves_raw {
        let steps: Vec<StepRow> = sqlx::query_as(
            "SELECT seq, step, state, started_at, ended_at, receipt FROM move_steps WHERE move_id = ? ORDER BY seq",
        )
        .bind(&move_id)
        .fetch_all(pool)
        .await?;
        let (spend,): (i64,) =
            sqlx::query_as("SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE move_id = ?")
                .bind(&move_id)
                .fetch_one(pool)
                .await?;
        moves.push(MoveRow {
            move_id,
            state,
            created_at,
            updated_at,
            steps,
            spend_micro_usd: spend,
        });
    }
    let releases: Vec<ReleaseRow> =
        sqlx::query_as("SELECT seq, state, manifest_digest, created_at, signed_at FROM releases WHERE site_id = ? ORDER BY seq DESC LIMIT 20")
            .bind(site_id)
            .fetch_all(pool)
            .await?;
    let jobs: Vec<JobRow> = sqlx::query_as(
        "SELECT job_id, kind, state, created_at, started_at, ended_at, receipt FROM jobs WHERE site_id = ? ORDER BY created_at DESC LIMIT 15",
    )
    .bind(site_id)
    .fetch_all(pool)
    .await?;
    let receipt_kinds: Vec<(String, i64)> = sqlx::query_as(
        "SELECT kind, COUNT(*) FROM receipts WHERE site_id = ? GROUP BY kind ORDER BY 2 DESC",
    )
    .bind(site_id)
    .fetch_all(pool)
    .await?;
    let receipts: Vec<ReceiptRow> = sqlx::query_as(
        "SELECT digest, kind, subject_type, subject_id, issued_at FROM receipts WHERE site_id = ? ORDER BY issued_at DESC LIMIT 12",
    )
    .bind(site_id)
    .fetch_all(pool)
    .await?;
    let decisions: Vec<DecisionRow> = sqlx::query_as(
        "SELECT decision_id, kind, state, method, created_at, decided_at FROM decisions WHERE site_id = ? ORDER BY created_at DESC LIMIT 12",
    )
    .bind(site_id)
    .fetch_all(pool)
    .await?;
    let gauntlet: Vec<GauntletRow> = sqlx::query_as(
        "SELECT run_id, score, out_of, started_at, receipt FROM gauntlet_runs WHERE site_id = ? ORDER BY started_at DESC LIMIT 8",
    )
    .bind(site_id)
    .fetch_all(pool)
    .await?;
    let budget_rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT purpose, period, limit_micro_usd FROM budgets WHERE site_id = ? ORDER BY purpose",
    )
    .bind(site_id)
    .fetch_all(pool)
    .await?;
    let mut budgets = Vec::new();
    for (purpose, period, limit) in budget_rows {
        let since = period_from(&period, now);
        let (spent,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE site_id = ? AND at >= ? AND (? = '*' OR purpose = ?)",
        )
        .bind(site_id)
        .bind(&since)
        .bind(&purpose)
        .bind(&purpose)
        .fetch_one(pool)
        .await?;
        budgets.push(BudgetRow {
            purpose,
            period,
            limit_micro_usd: limit,
            spent_micro_usd: spent,
        });
    }
    let spend_month: Vec<SpendRow> = sqlx::query_as(
        "SELECT purpose, provider, COUNT(*) AS calls, COALESCE(SUM(tokens_in + tokens_out), 0) AS tokens, \
         COALESCE(SUM(cost_micro_usd), 0) AS micro_usd FROM usage WHERE site_id = ? AND at >= ? \
         GROUP BY purpose, provider ORDER BY micro_usd DESC",
    )
    .bind(site_id)
    .bind(time(month_start(now)))
    .fetch_all(pool)
    .await?;
    let (spend_total,): (i64,) =
        sqlx::query_as("SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE site_id = ?")
            .bind(site_id)
            .fetch_one(pool)
            .await?;
    let (quarantined,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM quarantine WHERE site_id = ? AND state = 'held'")
            .bind(site_id)
            .fetch_one(pool)
            .await?;
    let findings: Vec<(String, i64)> =
        sqlx::query_as("SELECT severity, COUNT(*) FROM findings WHERE site_id = ? GROUP BY severity ORDER BY severity DESC")
            .bind(site_id)
            .fetch_all(pool)
            .await?;
    Ok(Some(SiteDetail {
        kind: row.kind(),
        stage: row.stage(),
        row,
        old_origin,
        moves,
        releases,
        jobs,
        receipt_kinds,
        receipts,
        decisions,
        gauntlet,
        budgets,
        spend_month,
        spend_total,
        quarantined,
        findings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_months() {
        let mut v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&mut v, 0.9), Some(5.0));
        assert_eq!(percentile(&mut v, 0.5), Some(3.0));
        assert_eq!(percentile(&mut [], 0.9), None);
        let nov = Utc.with_ymd_and_hms(2026, 11, 8, 9, 0, 0).unwrap();
        assert_eq!(days_in_month(nov), 30);
        assert_eq!(time(month_start(nov)), "2026-11-01T00:00:00.000000Z");
        assert_eq!(
            kind_of("kilele-bakery-sample.agentrix.website", false),
            "sample"
        );
        assert_eq!(kind_of("kilele-bakery.co.ke", false), "customer");
        assert_eq!(kind_of("fixture.zavora.example", true), "fixture");
    }
}

// ---------- Rollouts ----------

/// What reaches sites today, for the Rollouts page (08 §3.5).
#[derive(Debug, Clone, Serialize, Default)]
pub struct Rollouts {
    /// Live sites in ring 0: Zavora's own fixture and the sample sites.
    pub ring0: i64,
    /// Owners who opted into early updates: no opt-in exists yet.
    pub ring1: i64,
    /// Customers' live sites, the rest of the rings.
    pub customers: i64,
    /// Model route changes that reached sites: `(title, first issued, sites told)`.
    pub routing: Vec<(String, String, i64)>,
    /// Site releases signed per day over 7 days, in the operator's zone: `("Sun 8 Nov", count)`.
    pub releases_by_day: Vec<(String, i64)>,
    /// Rollbacks in 30 days.
    pub rollbacks_30d: i64,
    /// Gauntlet runs in 7 days and how many held every attack.
    pub gauntlet_week: (i64, i64),
}

/// Reads what the Rollouts page shows.
/// `offset_minutes` is the operator's zone (EAT is +180): release days are counted in it.
pub async fn rollouts(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    offset_minutes: i32,
) -> Result<Rollouts, sqlx::Error> {
    let live: Vec<(String, i64)> = sqlx::query_as(
        "SELECT s.domain, s.fixture FROM site_pointer p JOIN sites s ON s.site_id = p.site_id",
    )
    .fetch_all(pool)
    .await?;
    let ring0 = live
        .iter()
        .filter(|(d, f)| kind_of(d, *f != 0) != "customer")
        .count() as i64;
    let routing: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT COALESCE(json_extract(body, '$.detail.title'), 'Model routes changed'), MIN(issued_at), COUNT(DISTINCT site_id) \
         FROM receipts WHERE kind = 'routing.changed' GROUP BY json_extract(body, '$.detail.to') ORDER BY 2 DESC LIMIT 8",
    )
    .fetch_all(pool)
    .await?;
    let week = time(now - Duration::days(7));
    let signed: Vec<(String,)> =
        sqlx::query_as("SELECT signed_at FROM releases WHERE signed_at >= ?")
            .bind(&week)
            .fetch_all(pool)
            .await?;
    let zone = chrono::FixedOffset::east_opt(offset_minutes * 60)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("UTC"));
    let mut days: BTreeMap<chrono::NaiveDate, i64> = BTreeMap::new();
    for (at,) in signed {
        if let Some(t) = parse(&at) {
            *days.entry(t.with_timezone(&zone).date_naive()).or_default() += 1;
        }
    }
    let releases_by_day: Vec<(String, i64)> = days
        .into_iter()
        .rev()
        .map(|(d, n)| (d.format("%a %-d %b").to_string(), n))
        .collect();
    let (rollbacks_30d,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM receipts WHERE kind = 'release.rollback' AND issued_at >= ?",
    )
    .bind(time(now - Duration::days(30)))
    .fetch_one(pool)
    .await?;
    let gauntlet_week: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(score = out_of AND out_of > 0), 0) FROM gauntlet_runs WHERE started_at >= ?",
    )
    .bind(&week)
    .fetch_one(pool)
    .await?;
    Ok(Rollouts {
        ring0,
        ring1: 0,
        customers: live.len() as i64 - ring0,
        routing,
        releases_by_day,
        rollbacks_30d,
        gauntlet_week,
    })
}
