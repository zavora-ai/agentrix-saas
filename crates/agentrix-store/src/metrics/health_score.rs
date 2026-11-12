//! A site's health score from content-free signals (docs/08-control-plane.md §4.1), for the
//! operator and partner consoles only. The owner never sees it as a grade; it triggers help.
//!
//! Each of 08's nine signals is listed. Six have a source today (decisions, the "don't know"
//! trend, spend limits, incidents declared on the site, payment status from W6's
//! subscriptions, the owner's last sign-in); the other three say which workstream builds
//! theirs. The score counts only the signals with a reading and says how many
//! it rests on, so a missing source never counts as good or bad. The points are a hypothesis to
//! tune in the pilot.

use agentrix_types::SiteId;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use super::{Origin, Result};
use crate::time::stamp;

/// Where a score puts a site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Band {
    /// 70 or more.
    Healthy,
    /// 40 to 69: worth a look.
    Watch,
    /// Under 40: offer help (08 §4.1's "At risk").
    AtRisk,
    /// Fewer than three signals have a reading, so no score.
    NotEnough,
}

impl Band {
    /// "Healthy", "Worth a look", "At risk", "Not enough signals yet".
    pub fn words(self) -> &'static str {
        match self {
            Self::Healthy => "Healthy",
            Self::Watch => "Worth a look",
            Self::AtRisk => "At risk",
            Self::NotEnough => "Not enough signals yet",
        }
    }
}

/// One of 08 §4.1's signals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Signal {
    /// Stable id.
    pub id: &'static str,
    /// What it measures.
    pub name: &'static str,
    /// What it reads now, in plain words; `None` when there is no source or nothing to read.
    pub reading: Option<String>,
    /// Points earned, when it counts toward the score.
    pub points: Option<i64>,
    /// Points it can earn.
    pub max: i64,
    /// The workstream that builds the source, when there is none yet.
    pub pending_owner: Option<&'static str>,
}

/// A site's health.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteHealth {
    /// The site.
    pub site_id: SiteId,
    /// Its domain.
    pub domain: String,
    /// Real, sample or fixture.
    pub origin: Origin,
    /// 0 to 100 over the signals with a reading, or `None` with fewer than three.
    pub score: Option<i64>,
    /// Where the score puts it.
    pub band: Band,
    /// Signals with a reading.
    pub counted: usize,
    /// Whether the site is live. Only live sites are scored (08 §4.1): before go-live, "no
    /// incidents" and "nothing refused" only mean nothing happened yet.
    pub live: bool,
    /// Every signal, in 08's order.
    pub signals: Vec<Signal>,
}

impl SiteHealth {
    /// "Healthy (82), from 4 of 9 signals."
    pub fn words(&self) -> String {
        if !self.live {
            return "Not scored until the site is live.".to_owned();
        }
        match self.score {
            Some(score) => format!(
                "{} ({score}), from {} of {} signals.",
                self.band.words(),
                self.counted,
                self.signals.len()
            ),
            None => format!(
                "{}: {} of {} signals have a reading.",
                self.band.words(),
                self.counted,
                self.signals.len()
            ),
        }
    }
}

fn pending(id: &'static str, name: &'static str, max: i64, owner: &'static str) -> Signal {
    Signal {
        id,
        name,
        reading: None,
        points: None,
        max,
        pending_owner: Some(owner),
    }
}

fn read(
    id: &'static str,
    name: &'static str,
    max: i64,
    reading: String,
    points: Option<i64>,
) -> Signal {
    Signal {
        id,
        name,
        reading: Some(reading),
        points,
        max,
        pending_owner: None,
    }
}

/// Computes one site's health at `now`. `None` if the site isn't in `sites`.
pub(super) async fn site_health(
    pool: &SqlitePool,
    site: &SiteId,
    now: DateTime<Utc>,
) -> Result<Option<SiteHealth>> {
    let row: Option<(String, i64, Option<String>)> =
        sqlx::query_as("SELECT domain, fixture, owner_account_id FROM sites WHERE site_id = ?")
            .bind(site.as_str())
            .fetch_optional(pool)
            .await?;
    let Some((domain, fixture, owner)) = row else {
        return Ok(None);
    };
    let days = |d: i64| stamp(now - Duration::days(d));

    // Decisions handled versus expired, 30 days.
    let (handled, expired): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(state IN ('approved', 'declined')), 0), \
         COALESCE(SUM(state = 'expired'), 0) FROM decisions WHERE site_id = ? AND created_at >= ?",
    )
    .bind(site.as_str())
    .bind(days(30))
    .fetch_one(pool)
    .await?;
    let decisions = if handled + expired == 0 {
        read(
            "decisions",
            "Decisions handled versus expired",
            20,
            "No decisions in 30 days.".to_owned(),
            None,
        )
    } else {
        read(
            "decisions",
            "Decisions handled versus expired",
            20,
            format!("{handled} handled, {expired} expired in 30 days."),
            Some((20 * handled + (handled + expired) / 2) / (handled + expired)),
        )
    };

    // The Concierge's "don't know" trend: questions passed to the owner, this week and last.
    let (this_week, last_week): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(passed_at >= ?2), 0), COALESCE(SUM(passed_at < ?2), 0) \
         FROM passed_questions WHERE site_id = ?1 AND passed_at >= ?3",
    )
    .bind(site.as_str())
    .bind(days(7))
    .bind(days(14))
    .fetch_one(pool)
    .await?;
    let rising = this_week > last_week.max(2);
    let dont_know = read(
        "dont_know",
        "The Concierge's \"don't know\" trend",
        15,
        format!("{this_week} questions passed on this week, {last_week} the week before."),
        Some(if rising { 7 } else { 15 }),
    );

    // Spend versus budget: calls a budget or limit stopped, 7 days.
    let (refused,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM usage WHERE site_id = ? AND outcome = 'over-budget' AND at >= ?",
    )
    .bind(site.as_str())
    .bind(days(7))
    .fetch_one(pool)
    .await?;
    let spend = read(
        "spend",
        "Spend versus budget",
        15,
        if refused == 0 {
            "No model calls stopped by a limit in 7 days.".to_owned()
        } else {
            format!("{refused} model calls stopped by a limit in 7 days.")
        },
        Some(if refused == 0 { 15 } else { 5 }),
    );

    // Days since the owner last signed in (sessions keep Unix seconds).
    let recency = match owner {
        None => read(
            "owner_recency",
            "Days since the owner last opened the console",
            20,
            "Nobody has claimed the site yet.".to_owned(),
            None,
        ),
        Some(account) => {
            let (last,): (Option<i64>,) =
                sqlx::query_as("SELECT MAX(created_at) FROM sessions WHERE account_id = ?")
                    .bind(account)
                    .fetch_one(pool)
                    .await?;
            match last {
                None => read(
                    "owner_recency",
                    "Days since the owner last opened the console",
                    20,
                    "No sign-in on record.".to_owned(),
                    None,
                ),
                Some(secs) => {
                    let ago = (now.timestamp() - secs).max(0) / 86_400;
                    let points = match ago {
                        0..=7 => 20,
                        8..=14 => 12,
                        15..=30 => 6,
                        _ => 0,
                    };
                    let reading = match ago {
                        0 => "Signed in today.".to_owned(),
                        1 => "Last signed in yesterday.".to_owned(),
                        n => format!("Last signed in {n} days ago."),
                    };
                    read(
                        "owner_recency",
                        "Days since the owner last opened the console",
                        20,
                        reading,
                        Some(points),
                    )
                }
            }
        }
    };

    // Incidents on the site (W3's incidents): until per-site SLOs are measured, the incidents a
    // person declared stand for SLO breaches. Open and serious counts most.
    let incidents: std::result::Result<(i64, i64, i64), sqlx::Error> = sqlx::query_as(
        "SELECT COALESCE(SUM(state <> 'resolved' AND severity IN ('sev1', 'sev2')), 0), \
         COALESCE(SUM(state <> 'resolved'), 0), COUNT(*) FROM incidents \
         WHERE site_id = ? AND (state <> 'resolved' OR opened_at >= ?)",
    )
    .bind(site.as_str())
    .bind(days(30))
    .fetch_one(pool)
    .await;
    const SLO: &str = "SLO breaches and incidents on the site";
    let slo = match incidents {
        Err(sqlx::Error::Database(e)) if e.message().contains("no such table") => {
            pending("slo", SLO, 10, "W3")
        }
        Err(e) => return Err(e.into()),
        Ok((serious, open, recent)) => {
            let (reading, points) = if serious > 0 {
                (
                    format!("{serious} serious incident (SEV1 or SEV2) open."),
                    0,
                )
            } else if open > 0 {
                (format!("{open} incident open."), 3)
            } else if recent > 0 {
                (
                    format!("{recent} incident in the last 30 days, all resolved."),
                    7,
                )
            } else {
                ("No incidents declared in 30 days.".to_owned(), 10)
            };
            read("slo", SLO, 10, reading, Some(points))
        }
    };

    // Payment status: the site's latest plan subscription (W6's plan_subscriptions, which
    // W3's billing syncs from; reading it directly never lags).
    let latest: std::result::Result<Option<(String,)>, sqlx::Error> = sqlx::query_as(
        "SELECT state FROM plan_subscriptions WHERE site_id = ? \
         ORDER BY updated_at DESC, id DESC LIMIT 1",
    )
    .bind(site.as_str())
    .fetch_optional(pool)
    .await;
    let payment = match latest {
        Err(sqlx::Error::Database(e)) if e.message().contains("no such table") => {
            pending("payment", "Payment status", 10, "W6")
        }
        Err(e) => return Err(e.into()),
        Ok(None) => read(
            "payment",
            "Payment status",
            10,
            "No plan yet.".to_owned(),
            None,
        ),
        Ok(Some((state,))) => {
            let (reading, points) = match state.as_str() {
                "active" => ("Plan paid and active.", 10),
                "pending_approval" => ("Plan waiting for the owner's approval at PayPal.", 5),
                "past_due" => ("Payment is late.", 3),
                "suspended" => ("Plan suspended.", 0),
                "cancelled" => ("Plan cancelled.", 0),
                "expired" => ("Plan expired.", 0),
                _ => ("Plan in a state this score doesn't know.", 5),
            };
            read(
                "payment",
                "Payment status",
                10,
                reading.to_owned(),
                Some(points),
            )
        }
    };

    let signals = vec![
        pending("owner_minutes", "Owner minutes per week", 10, "W5"),
        decisions,
        pending("corrections", "Corrections per 100 actions", 10, "W1"),
        dont_know,
        spend,
        slo,
        pending("tickets", "Open tickets", 10, "W3"),
        payment,
        recency,
    ];
    let (live,): (i64,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM site_pointer WHERE site_id = ?)")
            .bind(site.as_str())
            .fetch_one(pool)
            .await?;
    let live = live != 0;
    let counted: Vec<&Signal> = signals.iter().filter(|s| s.points.is_some()).collect();
    let (earned, possible) = counted
        .iter()
        .fold((0, 0), |(e, p), s| (e + s.points.unwrap_or(0), p + s.max));
    let score = (live && counted.len() >= 3 && possible > 0)
        .then(|| (earned * 100 + possible / 2) / possible);
    let band = match score {
        None => Band::NotEnough,
        Some(s) if s >= 70 => Band::Healthy,
        Some(s) if s >= 40 => Band::Watch,
        Some(_) => Band::AtRisk,
    };
    Ok(Some(SiteHealth {
        origin: Origin::of(&domain, fixture != 0),
        site_id: site.clone(),
        domain,
        score,
        band,
        counted: counted.len(),
        live,
        signals,
    }))
}
