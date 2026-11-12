//! Health checks and the status page behind them.
//!
//! A task runs [`Health::run`] once a minute: each check reads `control.db` or calls the Model
//! Gateway, and the result is added to that check's hour in `health_hours`
//! (`migrations/control/0802_health_hours.sql`). [`Health::report`] turns the rows into what
//! the status page shows: each part of the platform now, and the last 30 days, measured only
//! from the first check onward. Nothing here names a site or a person: the page is public.

use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::time::stamp;

/// A check's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Working.
    Ok,
    /// Working, but slow or partly failing.
    Degraded,
    /// Not working.
    Down,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Degraded => "slow",
            Self::Down => "down",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "down" => Self::Down,
            "slow" => Self::Degraded,
            _ => Self::Ok,
        }
    }

    /// "Working", "Slower than usual", "Down".
    pub fn words(self) -> &'static str {
        match self {
            Self::Ok => "Working",
            Self::Degraded => "Slower than usual",
            Self::Down => "Down",
        }
    }
}

/// The parts of the platform the status page shows, in order: (id, name, what it covers).
pub const PARTS: [(&str, &str, &str); 5] = [
    (
        "websites",
        "Websites",
        "Pages served from each site's signed release",
    ),
    (
        "ask",
        "Ask and answers",
        "Answers on sites, the brief and the Move's checks, through the Model Gateway",
    ),
    (
        "moves",
        "Moves",
        "Copying, cleaning and rebuilding sites in sealed rooms",
    ),
    (
        "console",
        "Owner console and sign-in",
        "Decisions, approvals, receipts and sign-in",
    ),
    (
        "security",
        "Security checks",
        "The gauntlet of attacks run against live sites after each change",
    ),
];

/// One check's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckResult {
    /// One of [`PARTS`]' ids.
    pub id: &'static str,
    /// What it found.
    pub state: State,
    /// Plain words, safe to show the public: no site, person or secret.
    pub detail: String,
}

/// One day on the status page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Day {
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    /// The worst state seen, or `None` when nothing was checked that day.
    pub state: Option<State>,
    /// Minutes down and degraded.
    pub down_minutes: i64,
    /// Minutes degraded.
    pub slow_minutes: i64,
}

/// One part of the platform on the status page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Part {
    /// Its id.
    pub id: &'static str,
    /// Its name.
    pub name: &'static str,
    /// What it covers.
    pub covers: &'static str,
    /// Its state at the last check, if it has been checked.
    pub state: Option<State>,
    /// The last check's words.
    pub detail: String,
    /// When it was last checked.
    pub checked_at: Option<String>,
    /// Share of checked minutes working (ok or slow) over the days shown, to two decimals.
    pub uptime_percent: Option<f64>,
    /// The last 30 days, oldest first.
    pub days: Vec<Day>,
}

/// The whole status page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StatusReport {
    /// The worst state now, or `None` before the first check.
    pub overall: Option<State>,
    /// One sentence: "Everything is working." or "Ask and answers is down."
    pub headline: String,
    /// Each part.
    pub parts: Vec<Part>,
    /// The first day anything was checked.
    pub measured_since: Option<String>,
}

/// Runs the checks and reads their history.
#[derive(Debug, Clone)]
pub struct Health {
    pool: SqlitePool,
    http: reqwest::Client,
    models_url: String,
}

/// Days of history kept.
pub const KEEP_DAYS: i64 = 90;

impl Health {
    /// Checks over `control.db`, and the Model Gateway at `models_url` (its `/healthz`).
    pub fn new(pool: SqlitePool, http: reqwest::Client, models_url: impl Into<String>) -> Self {
        Self {
            pool,
            http,
            models_url: models_url.into(),
        }
    }

    /// Runs every check once.
    pub async fn check(&self, now: DateTime<Utc>) -> Vec<CheckResult> {
        let db = self.check_database().await;
        let websites = match &db {
            Err(_) => down("websites", "Can't read the list of live releases."),
            Ok(_) => self.check_websites().await,
        };
        let console = match db {
            Err(e) => down("console", &format!("The database isn't answering ({e}).")),
            Ok(ms) if ms > 500 => CheckResult {
                id: "console",
                state: State::Degraded,
                detail: format!("The database took {ms} ms to answer."),
            },
            Ok(_) => ok("console", "Answering normally."),
        };
        vec![
            websites,
            self.check_models().await,
            self.check_moves(now).await,
            console,
            self.check_security().await,
        ]
    }

    async fn check_database(&self) -> Result<u128, String> {
        let start = Instant::now();
        let read = tokio::time::timeout(
            StdDuration::from_secs(5),
            sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM sites").fetch_one(&self.pool),
        )
        .await;
        match read {
            Ok(Ok(_)) => Ok(start.elapsed().as_millis()),
            Ok(Err(_)) => Err("a read failed".to_owned()),
            Err(_) => Err("no answer in 5 s".to_owned()),
        }
    }

    async fn check_websites(&self) -> CheckResult {
        let live: Result<(i64,), _> = sqlx::query_as("SELECT COUNT(*) FROM site_pointer")
            .fetch_one(&self.pool)
            .await;
        match live {
            Ok((0,)) => ok("websites", "No site is live yet."),
            Ok((n,)) => ok(
                "websites",
                &format!(
                    "{} served from signed releases.",
                    crate::words::plural(n, "site", "sites")
                ),
            ),
            Err(_) => down("websites", "Can't read the list of live releases."),
        }
    }

    async fn check_models(&self) -> CheckResult {
        let url = format!("{}/healthz", self.models_url.trim_end_matches('/'));
        let start = Instant::now();
        let answer = self
            .http
            .get(&url)
            .timeout(StdDuration::from_secs(3))
            .send()
            .await;
        let ms = start.elapsed().as_millis();
        match answer {
            Ok(r) if r.status().is_success() && ms > 1500 => CheckResult {
                id: "ask",
                state: State::Degraded,
                detail: format!("The Model Gateway took {ms} ms to answer."),
            },
            Ok(r) if r.status().is_success() => ok("ask", "The Model Gateway is answering."),
            Ok(r) => down(
                "ask",
                &format!("The Model Gateway answered {}.", r.status().as_u16()),
            ),
            Err(_) => down(
                "ask",
                "The Model Gateway isn't answering. Pages still load; answers wait.",
            ),
        }
    }

    async fn check_moves(&self, now: DateTime<Utc>) -> CheckResult {
        let stale = stamp(now - Duration::minutes(10));
        let queued: Result<(i64,), _> =
            sqlx::query_as("SELECT COUNT(*) FROM jobs WHERE state = 'queued' AND created_at < ?")
                .bind(&stale)
                .fetch_one(&self.pool)
                .await;
        let hour = stamp(now - Duration::hours(1));
        let recent: Result<(i64, i64), _> = sqlx::query_as(
            "SELECT COALESCE(SUM(state = 'done'), 0), COALESCE(SUM(state IN ('failed', 'killed')), 0) \
             FROM jobs WHERE created_at >= ?",
        )
        .bind(&hour)
        .fetch_one(&self.pool)
        .await;
        match (queued, recent) {
            (Ok((q,)), _) if q > 0 => CheckResult {
                id: "moves",
                state: State::Degraded,
                detail: format!(
                    "{} waiting more than 10 minutes for a sealed room.",
                    crate::words::plural(q, "job is", "jobs are")
                ),
            },
            (Ok(_), Ok((done, failed))) if failed >= 3 && done == 0 => CheckResult {
                id: "moves",
                state: State::Degraded,
                detail: format!(
                    "{failed} sealed-room jobs failed in the last hour, none finished."
                ),
            },
            (Ok(_), Ok(_)) => ok("moves", "Sealed rooms are taking work."),
            _ => down("moves", "Can't read the sealed-room queue."),
        }
    }

    async fn check_security(&self) -> CheckResult {
        // The latest finished run (it has a receipt) on each site that is live now; a run in
        // progress has no score yet.
        let latest: Result<Vec<(i64, i64)>, _> = sqlx::query_as(
            "SELECT g.score, g.out_of FROM gauntlet_runs g JOIN site_pointer p ON p.site_id = g.site_id \
             WHERE g.run_id = (SELECT g2.run_id FROM gauntlet_runs g2 WHERE g2.site_id = g.site_id \
                AND g2.receipt IS NOT NULL ORDER BY g2.started_at DESC, g2.run_id DESC LIMIT 1)",
        )
        .fetch_all(&self.pool)
        .await;
        match latest {
            Err(_) => down("security", "Can't read the gauntlet's results."),
            Ok(runs) if runs.is_empty() => ok("security", "No live site has been tested yet."),
            Ok(runs) => {
                let (stopped, tried) = runs
                    .iter()
                    .fold((0, 0), |(s, t), (score, out)| (s + score, t + out));
                let state = if stopped < tried {
                    State::Degraded
                } else {
                    State::Ok
                };
                CheckResult {
                    id: "security",
                    state,
                    detail: format!(
                        "The last runs stopped {stopped} of {tried} attacks across {}.",
                        crate::words::plural(runs.len() as i64, "live site", "live sites")
                    ),
                }
            }
        }
    }

    /// Adds each result to its hour in `health_hours`.
    pub async fn record(
        &self,
        results: &[CheckResult],
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let hour = now.format("%Y-%m-%dT%H").to_string();
        let at = stamp(now);
        for r in results {
            let (ok, slow, down) = match r.state {
                State::Ok => (1, 0, 0),
                State::Degraded => (0, 1, 0),
                State::Down => (0, 0, 1),
            };
            sqlx::query(
                "INSERT INTO health_hours (check_id, hour, ok_minutes, slow_minutes, down_minutes, \
                 last_state, last_detail, last_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (check_id, hour) DO UPDATE SET \
                 ok_minutes = ok_minutes + excluded.ok_minutes, \
                 slow_minutes = slow_minutes + excluded.slow_minutes, \
                 down_minutes = down_minutes + excluded.down_minutes, \
                 last_state = excluded.last_state, last_detail = excluded.last_detail, \
                 last_at = excluded.last_at",
            )
            .bind(r.id)
            .bind(&hour)
            .bind(ok)
            .bind(slow)
            .bind(down)
            .bind(r.state.as_str())
            .bind(&r.detail)
            .bind(&at)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Runs the checks and records them.
    pub async fn run(&self, now: DateTime<Utc>) -> Result<Vec<CheckResult>, sqlx::Error> {
        let results = self.check(now).await;
        self.record(&results, now).await?;
        Ok(results)
    }

    /// Deletes hours older than [`KEEP_DAYS`].
    pub async fn prune(&self, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
        let cutoff = (now - Duration::days(KEEP_DAYS))
            .format("%Y-%m-%dT%H")
            .to_string();
        Ok(sqlx::query("DELETE FROM health_hours WHERE hour < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    /// The status page: each part now and over the last 30 days (UTC).
    pub async fn report(&self, now: DateTime<Utc>) -> Result<StatusReport, sqlx::Error> {
        let first_day = (now - Duration::days(29)).format("%Y-%m-%d").to_string();
        let rows: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
            "SELECT check_id, substr(hour, 1, 10), SUM(ok_minutes), SUM(slow_minutes), \
             SUM(down_minutes) FROM health_hours WHERE hour >= ? GROUP BY 1, 2",
        )
        .bind(&first_day)
        .fetch_all(&self.pool)
        .await?;
        let latest: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT h.check_id, h.last_state, h.last_detail, h.last_at FROM health_hours h \
             WHERE h.hour = (SELECT MAX(hour) FROM health_hours h2 WHERE h2.check_id = h.check_id)",
        )
        .fetch_all(&self.pool)
        .await?;
        let (measured_since,): (Option<String>,) =
            sqlx::query_as("SELECT MIN(substr(hour, 1, 10)) FROM health_hours")
                .fetch_one(&self.pool)
                .await?;
        let days: Vec<String> = (0..30)
            .rev()
            .map(|d| (now - Duration::days(d)).format("%Y-%m-%d").to_string())
            .collect();
        let mut parts = Vec::with_capacity(PARTS.len());
        for (id, name, covers) in PARTS {
            let last = latest.iter().find(|(c, ..)| c == id);
            // A check older than 5 minutes says nothing about now.
            let fresh = last.is_some_and(|(_, _, _, at)| {
                crate::time::parse(at).is_some_and(|t| now - t <= Duration::minutes(5))
            });
            let (mut up, mut all) = (0i64, 0i64);
            let day_rows = days
                .iter()
                .map(|day| {
                    let row = rows.iter().find(|(c, d, ..)| c == id && d == day);
                    match row {
                        None => Day {
                            day: day.clone(),
                            state: None,
                            down_minutes: 0,
                            slow_minutes: 0,
                        },
                        Some((_, _, ok, slow, down)) => {
                            up += ok + slow;
                            all += ok + slow + down;
                            let state = if *down > 0 {
                                State::Down
                            } else if *slow > 0 {
                                State::Degraded
                            } else {
                                State::Ok
                            };
                            Day {
                                day: day.clone(),
                                state: Some(state),
                                down_minutes: *down,
                                slow_minutes: *slow,
                            }
                        }
                    }
                })
                .collect();
            parts.push(Part {
                id,
                name,
                covers,
                state: if fresh {
                    last.map(|(_, s, ..)| State::parse(s))
                } else {
                    None
                },
                detail: if fresh {
                    last.map(|(_, _, d, _)| d.clone()).unwrap_or_default()
                } else {
                    "Not checked in the last 5 minutes.".to_owned()
                },
                checked_at: last.map(|(.., at)| at.clone()),
                uptime_percent: (all > 0)
                    .then(|| ((up as f64 * 10_000.0) / all as f64).floor() / 100.0),
                days: day_rows,
            });
        }
        let overall = parts.iter().filter_map(|p| p.state).max();
        let headline = match overall {
            None => "Checks haven't run yet.".to_owned(),
            Some(State::Ok) if parts.iter().all(|p| p.state.is_some()) => {
                "Everything is working.".to_owned()
            }
            Some(State::Ok) => "Everything checked is working.".to_owned(),
            Some(worst) => {
                let names: Vec<&str> = parts
                    .iter()
                    .filter(|p| p.state == Some(worst))
                    .map(|p| p.name)
                    .collect();
                let verb = if worst == State::Down {
                    "down"
                } else {
                    "slower than usual"
                };
                format!(
                    "{} {} {verb}.",
                    names.join(" and "),
                    if names.len() == 1 { "is" } else { "are" }
                )
            }
        };
        Ok(StatusReport {
            overall,
            headline,
            parts,
            measured_since,
        })
    }
}

fn ok(id: &'static str, detail: &str) -> CheckResult {
    CheckResult {
        id,
        state: State::Ok,
        detail: detail.to_owned(),
    }
}

fn down(id: &'static str, detail: &str) -> CheckResult {
    CheckResult {
        id,
        state: State::Down,
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::control_db;

    fn now() -> DateTime<Utc> {
        crate::time::parse("2026-11-08T09:00:30Z").unwrap()
    }

    #[tokio::test]
    async fn checks_record_and_report() {
        let pool = control_db().await;
        // Nothing listens on port 9 on loopback: the gateway check reports down.
        let health = Health::new(pool.clone(), reqwest::Client::new(), "http://127.0.0.1:9");
        let before = health.report(now()).await.unwrap();
        assert_eq!(before.overall, None);
        assert_eq!(before.headline, "Checks haven't run yet.");

        let results = health.run(now()).await.unwrap();
        let ask = results.iter().find(|r| r.id == "ask").unwrap();
        assert_eq!(ask.state, State::Down);
        let websites = results.iter().find(|r| r.id == "websites").unwrap();
        assert_eq!(websites.detail, "No site is live yet.");
        health.run(now() + Duration::seconds(60)).await.unwrap();

        let report = health.report(now() + Duration::seconds(90)).await.unwrap();
        assert_eq!(report.overall, Some(State::Down));
        assert_eq!(report.headline, "Ask and answers is down.");
        let ask = report.parts.iter().find(|p| p.id == "ask").unwrap();
        assert_eq!(ask.uptime_percent, Some(0.0));
        assert_eq!(ask.days.last().unwrap().down_minutes, 2);
        let console = report.parts.iter().find(|p| p.id == "console").unwrap();
        assert_eq!(
            (console.state, console.uptime_percent),
            (Some(State::Ok), Some(100.0))
        );
        assert_eq!(console.days.len(), 30);
        assert_eq!(console.days[0].state, None, "no checks then");
        assert_eq!(report.measured_since.as_deref(), Some("2026-11-08"));

        // Ten minutes later, without checks, the page says so instead of guessing.
        let stale = health.report(now() + Duration::minutes(10)).await.unwrap();
        assert_eq!(stale.overall, None);
        assert_eq!(stale.parts[0].detail, "Not checked in the last 5 minutes.");
        assert_eq!(health.prune(now()).await.unwrap(), 0);
    }
}
