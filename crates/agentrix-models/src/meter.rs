//! Metering and budgets in `control.db` (R11.3). The gateway is the one writer of `usage`.
//! agentrix-node creates and migrates the database; the gateway only opens it.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Datelike, SecondsFormat, TimeZone, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::route::Purpose;

/// Opens the existing `control.db`. It never creates or migrates it.
pub async fn open(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
}

/// How a metered call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Error,
    OverBudget,
    /// Answered from a recording (D10a): no Vultr call, no cost.
    Replayed,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::OverBudget => "over-budget",
            Self::Replayed => "replayed",
        }
    }
}

/// One `usage` row.
#[derive(Debug, Clone)]
pub struct Usage<'a> {
    pub site: &'a str,
    pub purpose: Purpose,
    pub provider: crate::route::Provider,
    pub tier: Option<crate::route::Tier>,
    pub tier_reason: &'a str,
    pub model: &'a str,
    pub move_id: Option<&'a str>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub reasoning_tokens: u64,
    pub outcome: Outcome,
    /// For image and speech calls (W11): what was made or heard, which the cost follows.
    pub media: Option<MediaUnits<'a>>,
    /// The WebAgent that made the call (`x-agentrix-webagent`: coordinator, cashier…).
    pub webagent: Option<&'a str>,
    /// The WebAgent's job (`x-agentrix-job`: owner-questions, payments…).
    pub job: Option<&'a str>,
}

/// The quantity of an image or speech call, which is billed by units rather than tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaUnits<'a> {
    /// How many: images made, or seconds of audio heard or spoken (rounded up).
    pub units: i64,
    /// `image` or `audio_second` (W8's `usage.unit`).
    pub unit: &'a str,
    /// The cost Vultr reports or the catalogue gives for this call, in micro-USD.
    pub cost_micro_usd: i64,
}

/// Writes one `usage` row, with the cost estimated from the catalogue prices.
///
/// Columns added by W8's migration 0801 (`units`, `unit`, `webagent`, `job`) are named only
/// when the row has a value for them, so a call without them still writes to a `usage` table
/// that predates 0801.
pub async fn record(pool: &SqlitePool, usage: &Usage<'_>) -> Result<(), sqlx::Error> {
    let cost = match (usage.outcome, usage.media) {
        (Outcome::Replayed | Outcome::OverBudget, _) => 0,
        (Outcome::Ok | Outcome::Error, Some(media)) => media.cost_micro_usd.max(0),
        (Outcome::Ok | Outcome::Error, None) => {
            crate::route::cost_micro_usd(usage.model, usage.tokens_in, usage.tokens_out)
        }
    };
    let to_i64 = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
    let mut columns = String::from(
        "site_id, purpose, provider, model, tier, tier_reason, move_id, tokens_in, tokens_out, \
         reasoning_tokens, cost_micro_usd, outcome, at",
    );
    let mut marks = 13;
    if usage.media.is_some() {
        columns.push_str(", units, unit");
        marks += 2;
    }
    if usage.webagent.is_some() {
        columns.push_str(", webagent");
        marks += 1;
    }
    if usage.job.is_some() {
        columns.push_str(", job");
        marks += 1;
    }
    // Fixed column names only; every value is bound.
    let sql = format!(
        "INSERT INTO usage ({columns}) VALUES ({})",
        vec!["?"; marks].join(", ")
    );
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(usage.site)
        .bind(usage.purpose.as_str())
        .bind(usage.provider.as_str())
        .bind(usage.model)
        .bind(usage.tier.map(crate::route::Tier::as_str))
        .bind(usage.tier_reason)
        .bind(usage.move_id)
        .bind(to_i64(usage.tokens_in))
        .bind(to_i64(usage.tokens_out))
        .bind(to_i64(usage.reasoning_tokens))
        .bind(cost)
        .bind(usage.outcome.as_str())
        .bind(stamp(Utc::now()));
    if let Some(media) = usage.media {
        query = query.bind(media.units).bind(media.unit);
    }
    if let Some(webagent) = usage.webagent {
        query = query.bind(webagent);
    }
    if let Some(job) = usage.job {
        query = query.bind(job);
    }
    query.execute(pool).await?;
    Ok(())
}

/// A time as `control.db` stores it (agentrix-node's `db::time`): RFC 3339 UTC with
/// microseconds, so text order is time order.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Whether the site and, if named, the Move exist and belong together.
pub async fn caller_known(
    pool: &SqlitePool,
    site: &str,
    move_id: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let site_known: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM sites WHERE site_id = ?")
        .bind(site)
        .fetch_optional(pool)
        .await?;
    if site_known.is_none() {
        return Ok(false);
    }
    let Some(move_id) = move_id else {
        return Ok(true);
    };
    let move_known: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM moves WHERE move_id = ? AND site_id = ?")
            .bind(move_id)
            .bind(site)
            .fetch_optional(pool)
            .await?;
    Ok(move_known.is_some())
}

/// A budget the call would break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exhausted {
    pub purpose: String,
    pub period: String,
    pub limit_micro_usd: i64,
    pub spent_micro_usd: i64,
    /// The gateway's own daily limit (D10e), rather than a budget row.
    pub hard: bool,
    /// Which limit it is.
    pub limit: Limit,
}

/// The limits a call is checked against, in order: the site's budgets, the site's daily
/// limit, then the limits sites share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// A budget row for the site.
    Budget,
    /// The gateway's hard limit per site per day (`AGENTRIX_SITE_DAILY_CAP_USD`).
    SiteDay,
    /// Every `-sample` site together, per UTC day (`AGENTRIX_SAMPLES_DAILY_CAP_USD`).
    SamplesDay,
    /// Every site together, per UTC day (`AGENTRIX_FLEET_DAILY_CAP_USD`).
    PlatformDay,
}

impl Limit {
    /// How a refusal names it, in `usage.tier_reason` and the logs.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Budget => "refused: site budget",
            Self::SiteDay => "refused: site daily limit",
            Self::SamplesDay => "refused: samples daily limit",
            Self::PlatformDay => "refused: platform daily limit",
        }
    }
}

/// Daily limits shared by many sites, in micro-USD. Zero refuses every call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedCaps {
    /// Every site together.
    pub platform_micro_usd: i64,
    /// The `-sample` sites together (their spend is the operator's).
    pub samples_micro_usd: i64,
}

/// Default platform limit: $8 a day, under the Vultr credit left on Sun 8 Nov (about $10).
pub const DEFAULT_PLATFORM_CAP_USD: f64 = 8.0;
/// Default limit for the sample sites together: $3 a day.
pub const DEFAULT_SAMPLES_CAP_USD: f64 = 3.0;

/// What every site together, and the sample sites together, have spent today (UTC).
pub async fn shared_spent_today(
    pool: &SqlitePool,
    now: DateTime<Utc>,
) -> Result<(i64, i64), sqlx::Error> {
    let since = stamp(period_start("day", now));
    let (platform,): (i64,) =
        sqlx::query_as("SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE at >= ?")
            .bind(&since)
            .fetch_one(pool)
            .await?;
    let (samples,): (i64,) = sqlx::query_as(concat!(
        "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE at >= ? AND site_id IN (",
        "SELECT site_id FROM sites ",
        "WHERE substr(domain, 1, instr(domain || '.', '.') - 1) LIKE '%-sample')"
    ))
    .bind(&since)
    .fetch_one(pool)
    .await?;
    Ok((platform, samples))
}

/// Whether `site` is a sample site.
pub async fn is_sample(pool: &SqlitePool, site: &str) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(concat!(
        "SELECT 1 FROM sites WHERE site_id = ? AND site_id IN (",
        "SELECT site_id FROM sites ",
        "WHERE substr(domain, 1, instr(domain || '.', '.') - 1) LIKE '%-sample')"
    ))
    .bind(site)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Checks the limits sites share, after the site's own: the sample sites' limit (for a
/// sample site), then the platform's. Returns the first one already spent.
pub async fn over_shared_cap(
    pool: &SqlitePool,
    site: &str,
    now: DateTime<Utc>,
    caps: SharedCaps,
) -> Result<Option<Exhausted>, sqlx::Error> {
    let (platform, samples) = shared_spent_today(pool, now).await?;
    let exhausted = |limit, cap, spent| Exhausted {
        purpose: "*".to_owned(),
        period: "day".to_owned(),
        limit_micro_usd: cap,
        spent_micro_usd: spent,
        hard: true,
        limit,
    };
    if samples >= caps.samples_micro_usd && is_sample(pool, site).await? {
        return Ok(Some(exhausted(
            Limit::SamplesDay,
            caps.samples_micro_usd,
            samples,
        )));
    }
    if platform >= caps.platform_micro_usd {
        return Ok(Some(exhausted(
            Limit::PlatformDay,
            caps.platform_micro_usd,
            platform,
        )));
    }
    Ok(None)
}

/// Seconds until the next UTC midnight, when the daily limits start again.
pub fn seconds_to_midnight(now: DateTime<Utc>) -> u64 {
    let next = period_start("day", now) + chrono::Duration::days(1);
    u64::try_from((next - now).num_seconds().max(1)).unwrap_or(1)
}

/// What the site has spent on models today (UTC), in micro-USD.
pub async fn spent_today(
    pool: &SqlitePool,
    site: &str,
    now: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let (spent,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE site_id = ? AND at >= ?",
    )
    .bind(site)
    .bind(stamp(period_start("day", now)))
    .fetch_one(pool)
    .await?;
    Ok(spent)
}

/// Checks the site's budgets for this purpose and for every purpose (`*`), then the gateway's
/// hard daily limit per site (D10e). Returns the first one already spent.
pub async fn over_budget(
    pool: &SqlitePool,
    site: &str,
    purpose: Purpose,
    now: DateTime<Utc>,
    daily_cap_micro_usd: i64,
) -> Result<Option<Exhausted>, sqlx::Error> {
    let budgets: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT purpose, period, limit_micro_usd FROM budgets \
         WHERE site_id = ? AND (purpose = ? OR purpose = '*') ORDER BY purpose DESC",
    )
    .bind(site)
    .bind(purpose.as_str())
    .fetch_all(pool)
    .await?;
    for (budget_purpose, period, limit) in budgets {
        let since = stamp(period_start(&period, now));
        let (spent,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage \
             WHERE site_id = ? AND (? = '*' OR purpose = ?) AND at >= ?",
        )
        .bind(site)
        .bind(&budget_purpose)
        .bind(&budget_purpose)
        .bind(since)
        .fetch_one(pool)
        .await?;
        if spent >= limit {
            return Ok(Some(Exhausted {
                purpose: budget_purpose,
                period,
                limit_micro_usd: limit,
                spent_micro_usd: spent,
                hard: false,
                limit: Limit::Budget,
            }));
        }
    }
    let (today,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE site_id = ? AND at >= ?",
    )
    .bind(site)
    .bind(stamp(period_start("day", now)))
    .fetch_one(pool)
    .await?;
    if today >= daily_cap_micro_usd {
        return Ok(Some(Exhausted {
            purpose: "*".to_owned(),
            period: "day".to_owned(),
            limit_micro_usd: daily_cap_micro_usd,
            spent_micro_usd: today,
            hard: true,
            limit: Limit::SiteDay,
        }));
    }
    Ok(None)
}

fn period_start(period: &str, now: DateTime<Utc>) -> DateTime<Utc> {
    let day = |y, m, d| {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0)
            .single()
            .unwrap_or(now)
    };
    match period {
        "day" => day(now.year(), now.month(), now.day()),
        "month" => day(now.year(), now.month(), 1),
        _ => DateTime::<Utc>::UNIX_EPOCH,
    }
}

/// Plain words for a spent budget, for the caller and the console (R11.4's honesty).
pub fn exhausted_words(e: &Exhausted) -> String {
    let usd = |micro: i64| micro as f64 / 1e6;
    match e.limit {
        Limit::PlatformDay => {
            return format!(
                "Answers are paused for today: the platform daily limit for model calls is used up (${:.2} spent of ${:.2}). Pages still load. Model calls start again at midnight UTC.",
                usd(e.spent_micro_usd),
                usd(e.limit_micro_usd),
            );
        }
        Limit::SamplesDay => {
            return format!(
                "Answers are paused for today on the sample sites: their shared daily limit is used up (${:.2} spent of ${:.2}). Pages still load. Model calls start again at midnight UTC.",
                usd(e.spent_micro_usd),
                usd(e.limit_micro_usd),
            );
        }
        Limit::Budget | Limit::SiteDay => {}
    }
    if e.hard {
        return format!(
            "This site has spent ${:.2} on models today, the gateway's limit of ${:.2} a day. Model calls stop until midnight UTC; nothing was sent to Vultr.",
            e.spent_micro_usd as f64 / 1e6,
            e.limit_micro_usd as f64 / 1e6,
        );
    }
    let what = if e.purpose == "*" {
        "model budget".to_owned()
    } else {
        format!("{} budget", e.purpose)
    };
    let window = match e.period.as_str() {
        "day" => " for today",
        "month" => " for this month",
        _ => "",
    };
    format!(
        "This site's {what}{window} is used up: ${:.2} spent of ${:.2}. Raise it in the console, or wait for the next period.",
        e.spent_micro_usd as f64 / 1e6,
        e.limit_micro_usd as f64 / 1e6,
    )
}
