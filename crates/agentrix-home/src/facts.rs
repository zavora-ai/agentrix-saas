//! The live numbers on the home page, read from receipts and the database.
//!
//! Nothing here is typed in. [`SqliteFacts`] reads `control.db` with plain read-only queries
//! until W8's metric APIs land; [`CachedFacts`] keeps the last good read for a minute, so the
//! page stays fast and a slow or failed read never blanks the page. Where a number doesn't
//! exist yet, the field is `None` and the page says what will appear and when.
//!
//! Facts that name a site come only from Zavora's own fixture sites (`sites.fixture = 1`): a
//! customer's site is never named on the home page. Counts are across the whole server and
//! leave out the `-sample` sites.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

/// Everything the home page shows that comes from recorded activity.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Facts {
    /// The latest finished gauntlet run on a fixture site.
    pub gauntlet: Option<GauntletRun>,
    /// The first Move on a fixture site that went live.
    pub first_live_move: Option<LiveMove>,
    /// Sites serving a signed release, leaving out the samples.
    pub sites_live: Option<u64>,
    /// Signed receipts issued, across every site.
    pub receipts_issued: Option<u64>,
    /// Free health reports finished.
    pub reports_written: Option<u64>,
    /// The latest `routing.changed` receipt: which provider serves model calls now.
    pub routing: Option<Routing>,
    /// The latest morning brief on a fixture site.
    pub brief: Option<BriefSnapshot>,
    /// Each sample's approved hero picture from W11's library, by the sample's host label
    /// (`kilele-bakery-sample`).
    pub sample_pictures: std::collections::BTreeMap<String, Picture>,
    /// When these facts were read.
    pub read_at: Option<DateTime<Utc>>,
}

/// An approved picture as a site shows it (`agentrix_media::Shown`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Picture {
    /// `/media/<hex>.webp`, served on the platform host.
    pub url: String,
    /// The alt text.
    pub alt: String,
    /// The label to show on it, such as "Sample image, made with AI".
    pub label: Option<String>,
    /// Its size in pixels, when known.
    pub width: Option<i64>,
    /// Its size in pixels, when known.
    pub height: Option<i64>,
}

/// A finished gauntlet run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GauntletRun {
    /// Attacks contained.
    pub score: u32,
    /// Attacks tried.
    pub out_of: u32,
    /// The `gauntlet.run` receipt's digest.
    pub receipt: Option<String>,
    /// When the run finished.
    pub finished_at: DateTime<Utc>,
    /// Each attack, as the run recorded it.
    pub attacks: Vec<Attack>,
}

/// One attack in a gauntlet run (`gauntlet_runs.detail`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attack {
    /// The attack's short name.
    pub name: String,
    /// What it tried, in plain words.
    #[serde(default)]
    pub plain: String,
    /// Whether the platform held.
    pub held: bool,
    /// The receipt that proves it.
    #[serde(default)]
    pub receipt: Option<String>,
}

/// A Move that reached live, with its step times.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LiveMove {
    /// The site's name, or its domain when it has none.
    pub site_name: String,
    /// Each finished step, in the order it ran.
    pub steps: Vec<StepTime>,
    /// When the go-live step finished.
    pub live_at: DateTime<Utc>,
    /// Seconds from the owner's approval to live, when both are recorded.
    pub approval_to_live_seconds: Option<f64>,
    /// How the owner approved: `passkey` or `totp`.
    pub approved_with: Option<String>,
    /// Items the clean step left behind.
    pub left_behind: Option<u64>,
    /// Estimated model cost of the Move, in millionths of a dollar.
    pub cost_micro_usd: Option<i64>,
    /// Providers that served its model calls.
    pub providers: Vec<String>,
}

/// One step of a Move, timed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepTime {
    /// `assess`, `claim`, `replay`, `clean`, `rebuild`, `prove` or `golive`.
    pub step: String,
    /// How long it ran.
    pub seconds: f64,
    /// The step's own summary, in plain words.
    pub summary: String,
    /// Its receipt's digest.
    pub receipt: Option<String>,
}

/// Which provider serves model calls, by purpose.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Routing {
    /// `vultr` or `anthropic`.
    pub provider: String,
    /// Each purpose's route, sorted by purpose.
    pub routes: Vec<Route>,
    /// When the receipt was issued.
    pub since: DateTime<Utc>,
    /// The receipt's digest.
    pub receipt: String,
}

/// Where one purpose's model calls go.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Route {
    /// `build`, `concierge`, `embed` and so on.
    pub purpose: String,
    /// The model.
    pub model: String,
    /// The provider, when the route names one.
    pub provider: Option<String>,
}

/// The opening of a morning brief.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BriefSnapshot {
    /// The site's name.
    pub site_name: String,
    /// When the Chief WebAgent wrote it.
    pub written_at: DateTime<Utc>,
    /// Its opening line, if it has one.
    pub headline: Option<String>,
    /// Its first lines.
    pub lines: Vec<String>,
}

/// Where the home page's facts come from.
#[async_trait]
pub trait FactSource: Send + Sync {
    /// Reads every fact now. A fact that has no record yet is `None`, not an error.
    async fn read(&self) -> anyhow::Result<Facts>;
}

/// Reads [`Facts`] from `control.db` with read-only queries.
pub struct SqliteFacts {
    pool: SqlitePool,
}

impl SqliteFacts {
    /// Facts from this pool (node's `control.db`, already migrated).
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The pool, for the other readers in this crate.
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// What [`SqliteFacts::move_detail`] reads.
pub(crate) struct MoveDetail {
    pub(crate) approval_to_live_seconds: Option<f64>,
    pub(crate) approved_with: Option<String>,
    pub(crate) left_behind: Option<u64>,
    pub(crate) providers: Vec<String>,
}

pub(crate) fn time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn seconds_between(start: &str, end: &str) -> Option<f64> {
    let s = (time(end)? - time(start)?).num_milliseconds() as f64 / 1000.0;
    (s >= 0.0).then_some(s)
}

/// `gauntlet_runs`: score, out_of, receipt, detail, started_at, ended_at.
type GauntletRow = (
    i64,
    i64,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

/// `move_steps`: step, summary, receipt, started_at, ended_at.
type StepRow = (String, String, Option<String>, String, Option<String>);

impl SqliteFacts {
    async fn count(&self, sql: &'static str) -> Option<u64> {
        match sqlx::query_as::<_, (i64,)>(sql).fetch_one(&self.pool).await {
            Ok((n,)) => u64::try_from(n).ok(),
            Err(e) => {
                tracing::warn!(error = %e, sql, "home: a count failed");
                None
            }
        }
    }

    /// The latest finished run on a fixture site, with its attacks.
    pub(crate) async fn gauntlet(&self) -> anyhow::Result<Option<GauntletRun>> {
        let row: Option<GauntletRow> = sqlx::query_as(
            "SELECT g.score, g.out_of, g.receipt, g.detail, g.started_at, g.ended_at \
                 FROM gauntlet_runs g JOIN sites s ON s.site_id = g.site_id \
                 WHERE s.fixture = 1 AND g.ended_at IS NOT NULL \
                 ORDER BY g.ended_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some((score, out_of, receipt, detail, started, ended)) = row else {
            return Ok(None);
        };
        let attacks: Vec<Attack> = detail
            .as_deref()
            .and_then(|d| serde_json::from_str(d).ok())
            .unwrap_or_default();
        let finished_at = ended
            .as_deref()
            .and_then(time)
            .or_else(|| time(&started))
            .unwrap_or_default();
        Ok(Some(GauntletRun {
            score: u32::try_from(score).unwrap_or(0),
            out_of: u32::try_from(out_of).unwrap_or(0),
            receipt,
            finished_at,
            attacks,
        }))
    }

    async fn first_live_move(&self) -> anyhow::Result<Option<LiveMove>> {
        let first: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT m.move_id, m.site_id, COALESCE(NULLIF(s.name, ''), s.domain), st.ended_at, st.started_at \
             FROM move_steps st JOIN moves m ON m.move_id = st.move_id JOIN sites s ON s.site_id = m.site_id \
             WHERE st.step = 'golive' AND st.state = 'done' AND st.ended_at IS NOT NULL AND s.fixture = 1 \
             ORDER BY st.ended_at ASC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some((move_id, site_id, site_name, live_at, _)) = first else {
            return Ok(None);
        };
        let rows: Vec<StepRow> = sqlx::query_as(
            "SELECT step, summary, receipt, started_at, ended_at FROM move_steps \
             WHERE move_id = ? AND state = 'done' ORDER BY seq",
        )
        .bind(&move_id)
        .fetch_all(&self.pool)
        .await?;
        let mut steps: Vec<StepTime> = Vec::new();
        for (step, summary, receipt, started, ended) in rows {
            let Some(seconds) = ended.as_deref().and_then(|e| seconds_between(&started, e)) else {
                continue;
            };
            // A step that ran again (a retry) keeps its last finished time.
            steps.retain(|s| s.step != step);
            steps.push(StepTime {
                step,
                seconds,
                summary,
                receipt,
            });
        }
        let cost: (Option<i64>,) =
            sqlx::query_as("SELECT SUM(cost_micro_usd) FROM usage WHERE move_id = ?")
                .bind(&move_id)
                .fetch_one(&self.pool)
                .await?;
        let detail = self.move_detail(&move_id, &site_id, &live_at).await?;
        Ok(Some(LiveMove {
            site_name,
            steps,
            live_at: time(&live_at).unwrap_or_default(),
            approval_to_live_seconds: detail.approval_to_live_seconds,
            approved_with: detail.approved_with,
            left_behind: detail.left_behind,
            cost_micro_usd: cost.0,
            providers: detail.providers,
        }))
    }

    /// One gauntlet run's attacks, by its receipt (`gauntlet_runs.detail`).
    pub(crate) async fn attacks(&self, receipt: &str) -> anyhow::Result<Vec<Attack>> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT detail FROM gauntlet_runs WHERE receipt = ? LIMIT 1")
                .bind(receipt)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row
            .and_then(|(detail,)| detail)
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default())
    }

    /// What a Move's summary doesn't carry: the owner's approval and how long it took to go
    /// live after it, the items left behind, and who served its model calls.
    pub(crate) async fn move_detail(
        &self,
        move_id: &str,
        site_id: &str,
        live_at: &str,
    ) -> anyhow::Result<MoveDetail> {
        let decision: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT decided_at, method FROM decisions \
             WHERE site_id = ? AND kind = 'go-live' AND state = 'approved' AND decided_at <= ? \
             ORDER BY decided_at DESC LIMIT 1",
        )
        .bind(site_id)
        .bind(live_at)
        .fetch_optional(&self.pool)
        .await?;
        let (approved_at, approved_with) = decision.unwrap_or_default();
        let left_behind: Option<(i64,)> =
            sqlx::query_as("SELECT COUNT(*) FROM quarantine WHERE move_id = ?")
                .bind(move_id)
                .fetch_optional(&self.pool)
                .await?;
        let providers: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT provider FROM usage WHERE move_id = ? AND outcome = 'ok' ORDER BY provider",
        )
        .bind(move_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(MoveDetail {
            approval_to_live_seconds: approved_at
                .as_deref()
                .and_then(|a| seconds_between(a, live_at)),
            approved_with,
            left_behind: left_behind.and_then(|(n,)| u64::try_from(n).ok()),
            providers: providers.into_iter().map(|(p,)| p).collect(),
        })
    }

    /// Free health reports finished, until W8's `reports_written` reaches platform.
    pub(crate) async fn reports_written(&self) -> Option<u64> {
        self.count("SELECT COUNT(*) FROM assessments WHERE state = 'report-ready'")
            .await
    }

    pub(crate) async fn routing(&self) -> anyhow::Result<Option<Routing>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT digest, issued_at, body FROM receipts WHERE kind = 'routing.changed' \
             ORDER BY issued_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some((digest, issued_at, body)) = row else {
            return Ok(None);
        };
        let body: serde_json::Value = serde_json::from_str(&body)?;
        let Some(to) = body.pointer("/detail/to") else {
            return Ok(None);
        };
        let provider = to
            .get("provider")
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_owned();
        let mut routes: Vec<Route> = to
            .get("routes")
            .and_then(|r| r.as_object())
            .map(|r| {
                r.iter()
                    .filter_map(|(purpose, route)| {
                        let model = route
                            .get("model")
                            .and_then(|m| m.as_str())
                            .or_else(|| route.as_str())?;
                        Some(Route {
                            purpose: purpose.clone(),
                            model: model.to_owned(),
                            provider: route
                                .get("provider")
                                .and_then(|p| p.as_str())
                                .map(str::to_owned),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        routes.sort();
        if provider.is_empty() {
            return Ok(None);
        }
        Ok(Some(Routing {
            provider,
            routes,
            since: time(&issued_at).unwrap_or_default(),
            receipt: digest,
        }))
    }

    pub(crate) async fn brief(&self) -> anyhow::Result<Option<BriefSnapshot>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT COALESCE(NULLIF(s.name, ''), s.domain), b.written_at, b.body \
             FROM briefs b JOIN sites s ON s.site_id = b.site_id WHERE s.fixture = 1 \
             ORDER BY b.written_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some((site_name, written_at, body)) = row else {
            return Ok(None);
        };
        let body: serde_json::Value = serde_json::from_str(&body)?;
        let headline = body
            .get("headline")
            .and_then(|h| h.as_str())
            .map(str::to_owned);
        let lines = body
            .get("lines")
            .and_then(|l| l.as_array())
            .map(|l| {
                l.iter()
                    .filter_map(|line| line.get("text").and_then(|t| t.as_str()))
                    .take(3)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(BriefSnapshot {
            site_name,
            written_at: time(&written_at).unwrap_or_default(),
            headline,
            lines,
        }))
    }
}

/// Logs a failed read and carries on without that fact.
pub(crate) fn soft<T: Default>(what: &str, result: anyhow::Result<T>) -> T {
    result.unwrap_or_else(|e| {
        tracing::warn!(error = %e, what, "home: a fact could not be read");
        T::default()
    })
}

#[async_trait]
impl FactSource for SqliteFacts {
    async fn read(&self) -> anyhow::Result<Facts> {
        Ok(Facts {
            gauntlet: soft("gauntlet", self.gauntlet().await),
            first_live_move: soft("first live move", self.first_live_move().await),
            sites_live: self
                .count(
                    "SELECT COUNT(*) FROM site_pointer p JOIN sites s ON s.site_id = p.site_id \
                     WHERE s.domain NOT LIKE '%-sample.%'",
                )
                .await,
            receipts_issued: self.count("SELECT COUNT(*) FROM receipts").await,
            reports_written: self.reports_written().await,
            routing: soft("routing", self.routing().await),
            brief: soft("brief", self.brief().await),
            sample_pictures: Default::default(),
            read_at: Some(Utc::now()),
        })
    }
}

/// Keeps the last good [`Facts`] for `ttl`, so the page costs one read a minute.
pub struct CachedFacts {
    source: Arc<dyn FactSource>,
    ttl: Duration,
    last: Mutex<Option<(Instant, Arc<Facts>)>>,
}

impl CachedFacts {
    /// A cache in front of `source`.
    pub fn new(source: Arc<dyn FactSource>, ttl: Duration) -> Self {
        Self {
            source,
            ttl,
            last: Mutex::new(None),
        }
    }

    /// The facts, read again when the cached copy is older than the ttl. A failed read keeps
    /// the last good copy; with none, every fact is unknown.
    pub async fn get(&self) -> Arc<Facts> {
        let mut last = self.last.lock().await;
        if let Some((at, facts)) = last.as_ref()
            && at.elapsed() < self.ttl
        {
            return facts.clone();
        }
        match self.source.read().await {
            Ok(facts) => {
                let facts = Arc::new(facts);
                *last = Some((Instant::now(), facts.clone()));
                facts
            }
            Err(e) => {
                tracing::warn!(error = %e, "home: reading the facts failed");
                last.as_ref()
                    .map(|(_, f)| f.clone())
                    .unwrap_or_else(|| Arc::new(Facts::default()))
            }
        }
    }
}
