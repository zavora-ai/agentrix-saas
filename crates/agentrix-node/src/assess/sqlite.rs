//! [`AssessStore`] over `control.db`: `assessments` and `ui_events` (written by assess/),
//! `findings` rows with `assessment_id` set, and sites through `db::sites`.

use agentrix_types::{AssessmentId, Finding, SiteId, UiEnvelope, UiEvent};
use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::report::ReportDoc;
use super::store::{AssessStore, Assessment, AssessmentState};
use crate::db;

/// Assessments in SQLite.
#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// A store over `pool` (already migrated by `db::open`).
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn parse_time(s: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("bad stored time {s:?}"))?
        .with_timezone(&Utc))
}

type Row = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    i64,
);

fn assessment(row: Row) -> anyhow::Result<Assessment> {
    let (id, site, url, state, report, created, finished, domain, fixture) = row;
    Ok(Assessment {
        id: AssessmentId::new(id),
        site: SiteId::new(site),
        url,
        host: domain,
        fixture: fixture != 0,
        state: AssessmentState::parse(&state).context("bad stored assessment state")?,
        created_at: parse_time(&created)?,
        finished_at: finished.as_deref().map(parse_time).transpose()?,
        report: report
            .as_deref()
            .map(serde_json::from_str::<ReportDoc>)
            .transpose()
            .context("a stored report does not parse")?,
    })
}

#[async_trait]
impl AssessStore for SqliteStore {
    async fn site_for(&self, domain: &str, origin: &str, fixture: bool) -> anyhow::Result<SiteId> {
        Ok(db::sites::create_or_get(&self.pool, domain, origin, fixture).await?)
    }

    async fn create(&self, a: &Assessment) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO assessments (assessment_id, site_id, url, state, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(a.id.as_str())
        .bind(a.site.as_str())
        .bind(&a.url)
        .bind(a.state.as_str())
        .bind(db::time(a.created_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get(&self, id: &AssessmentId) -> anyhow::Result<Option<Assessment>> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT a.assessment_id, a.site_id, a.url, a.state, a.report, a.created_at, a.finished_at, \
             s.domain, s.fixture FROM assessments a JOIN sites s ON s.site_id = a.site_id \
             WHERE a.assessment_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(assessment).transpose()
    }

    async fn latest_for(
        &self,
        host: &str,
        since: DateTime<Utc>,
    ) -> anyhow::Result<Option<Assessment>> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT a.assessment_id, a.site_id, a.url, a.state, a.report, a.created_at, a.finished_at, \
             s.domain, s.fixture FROM assessments a JOIN sites s ON s.site_id = a.site_id \
             WHERE s.domain = ? AND a.created_at > ? ORDER BY a.created_at DESC LIMIT 1",
        )
        .bind(host)
        .bind(db::time(since))
        .fetch_optional(&self.pool)
        .await?;
        row.map(assessment).transpose()
    }

    async fn finish(
        &self,
        id: &AssessmentId,
        state: AssessmentState,
        report: &ReportDoc,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE assessments SET state = ?, report = ?, finished_at = ? WHERE assessment_id = ?",
        )
        .bind(state.as_str())
        .bind(serde_json::to_string(report)?)
        .bind(db::time(at))
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn append(
        &self,
        stream: &str,
        site: &SiteId,
        env: &UiEnvelope,
        at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO ui_events (stream, seq, site_id, name, data, at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(stream)
            .bind(i64::try_from(env.seq)?)
            .bind(site.as_str())
            .bind(env.event.name())
            .bind(env.event.data_json()?)
            .bind(db::time(at))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn events_after(&self, stream: &str, after: u64) -> anyhow::Result<Vec<UiEnvelope>> {
        let rows: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT seq, name, data FROM ui_events WHERE stream = ? AND seq > ? ORDER BY seq",
        )
        .bind(stream)
        .bind(i64::try_from(after).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(seq, name, data)| {
                Ok(UiEnvelope {
                    seq: u64::try_from(seq)?,
                    event: UiEvent::from_parts(&name, &data)?,
                })
            })
            .collect()
    }

    async fn save_findings(
        &self,
        assessment: &AssessmentId,
        findings: &[Finding],
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        for f in findings {
            sqlx::query(
                "INSERT INTO findings (finding_id, site_id, assessment_id, move_id, detector, severity, \
                 category, title, explanation, location, evidence, at) \
                 VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(f.id.as_str())
            .bind(f.site.as_str())
            .bind(assessment.as_str())
            .bind(&f.detector)
            .bind(serde_json::to_value(f.severity)?.as_str().unwrap_or("info").to_owned())
            .bind(serde_json::to_value(f.category)?.as_str().unwrap_or("security").to_owned())
            .bind(&f.title)
            .bind(&f.explanation)
            .bind(&f.location)
            .bind(serde_json::to_string(&f.evidence)?)
            .bind(db::time(f.at))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
