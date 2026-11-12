//! [`ConsoleReads`] over the control database (schema `db/migrations/0001_control.sql`).
//!
//! Read-only: every query here is a `SELECT`, so each table keeps its one writer (design §3.0,
//! §5). JSON columns parse into the shapes `agentrix-types` defines for them (`DecisionCard`,
//! `Charter`, `Brief`, `GauntletAttack`, UI events). What the database can't answer comes
//! through [`Delegates`], which the integrator fills in `app.rs`: the session from `auth/`, the
//! approval ceremony from `decisions/` and frames from `db::cas`.
//!
//! ```ignore
//! // app.rs
//! let reads = SqliteReads::new(pool.clone(), receipts.keys().clone(), AppDelegates { auth, decisions, cas, pool });
//! let pages = pages::router(PagesState { console, reads: Arc::new(reads), onboarding: None, concepts: None });
//! ```

use agentrix_types::ui::{CardEvent, ParityEvent, StepName, TickEvent, UiEvent};
use agentrix_types::{
    Approval, ApprovalMethod, Brief, CardKind, Charter, DecisionCard, DecisionId, Digest,
    GauntletAttack, JobLevel, KeySet, MoveId, QuarantineId, Receipt, SiteId,
};
use anyhow::{Context as _, bail};
use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::reads::*;

/// What [`SqliteReads`] can't read from the control database.
#[async_trait]
pub trait Delegates: Send + Sync + 'static {
    /// The signed-in owner (`auth/`'s session cookie), or `None`.
    async fn session(&self, headers: &HeaderMap) -> ReadResult<Option<OwnerSession>>;
    /// See [`ConsoleReads::landing`].
    async fn landing(&self, _headers: &HeaderMap) -> ReadResult<Option<String>> {
        Ok(None)
    }
    /// Starts the approval ceremony for `card` (`decisions/`, design D9). See
    /// [`ConsoleReads::begin_approval`].
    async fn begin_approval(
        &self,
        session: &OwnerSession,
        card: &DecisionCard,
    ) -> ReadResult<Option<String>>;
    /// A frame from the CAS, under [`ConsoleReads::frame`]'s rule, which `db::cas` keeps:
    /// `Cas::get_for_site(pool, site, digest)` for the owner's site, then
    /// `Cas::get_public_frame(pool, digest)` for a free Assess's frames.
    async fn frame(
        &self,
        owner_site: Option<&SiteId>,
        digest: &Digest,
    ) -> ReadResult<Option<FrameBytes>>;
}

/// The console's reads over the control database.
#[derive(Debug, Clone)]
pub struct SqliteReads<D> {
    pool: SqlitePool,
    keys: KeySet,
    delegates: D,
}

impl<D: Delegates> SqliteReads<D> {
    /// Reads from `pool`, verifies receipts against `keys` (the receipts store's published keys)
    /// and asks `delegates` for the rest.
    pub fn new(pool: SqlitePool, keys: KeySet, delegates: D) -> Self {
        Self {
            pool,
            keys,
            delegates,
        }
    }
}

/// Step order on the Move page (design §6).
const STEPS: [StepName; 7] = [
    StepName::Assess,
    StepName::Claim,
    StepName::Replay,
    StepName::Clean,
    StepName::Rebuild,
    StepName::Prove,
    StepName::GoLive,
];

fn time(text: &str) -> ReadResult<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(text)
        .with_context(|| format!("a stored time does not parse: {text}"))?
        .with_timezone(&Utc))
}

fn digest(text: &str) -> ReadResult<Digest> {
    text.parse()
        .with_context(|| format!("a stored digest does not parse: {text}"))
}

type DecisionSql = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// A `decisions` query: [`DecisionSql`]'s columns, then the rest. sqlx takes only static SQL.
macro_rules! select_decisions {
    ($rest:literal) => {
        concat!(
            "SELECT card, fingerprint, state, method, signer, decided_at, receipt FROM decisions ",
            $rest
        )
    };
}

fn decision_row(
    (card, fingerprint, state, method, signer, decided_at, receipt): DecisionSql,
) -> ReadResult<DecisionRow> {
    let card: DecisionCard =
        serde_json::from_str(&card).context("a stored decision card does not parse")?;
    let fingerprint = digest(&fingerprint)?;
    if fingerprint != card.fingerprint {
        bail!(
            "decision {}: the stored fingerprint does not match its card",
            card.decision_id
        );
    }
    let state = match state.as_str() {
        "waiting" => DecisionState::Waiting,
        "approved" => DecisionState::Approved,
        "declined" => DecisionState::Declined,
        "expired" => DecisionState::Expired,
        other => bail!("decision {}: unknown state {other}", card.decision_id),
    };
    let approval = if state == DecisionState::Approved {
        // The schema's CHECK guarantees method, signer and decided_at on an approved row.
        let method = match method.as_deref() {
            Some("passkey") => ApprovalMethod::Passkey,
            Some("totp") => ApprovalMethod::StepUp,
            other => bail!(
                "decision {}: unknown approval method {other:?}",
                card.decision_id
            ),
        };
        Some(Approval {
            decision_id: card.decision_id.clone(),
            fingerprint,
            method,
            approver: signer.with_context(|| {
                format!("decision {}: approved without a signer", card.decision_id)
            })?,
            at: time(decided_at.as_deref().with_context(|| {
                format!("decision {}: approved without a time", card.decision_id)
            })?)?,
        })
    } else {
        None
    };
    let receipt = receipt.as_deref().map(digest).transpose()?;
    Ok(DecisionRow {
        card,
        state,
        approval,
        receipt,
    })
}

/// Folds a Move's UI events into its current state: the same values a `snapshot` carries.
fn fold(row: &mut MoveRow, event: UiEvent) {
    fn upsert_parity(row: &mut MoveRow, p: ParityEvent) {
        match &p.page {
            None => row.parity_site = Some(p),
            Some(page) => match row
                .parity_pages
                .iter_mut()
                .find(|q| q.page.as_ref() == Some(page))
            {
                Some(slot) => *slot = p,
                None => row.parity_pages.push(p),
            },
        }
    }
    match event {
        UiEvent::Step(s) => match row.steps.iter_mut().find(|t| t.step == s.step) {
            Some(slot) => *slot = s,
            None => row.steps.push(s),
        },
        UiEvent::ReceiptTick(t) => row.tick = t,
        UiEvent::Frame(f) => row.last_frame = Some(f),
        UiEvent::Page(p) => row.pages = Some(p),
        UiEvent::Parity(p) => upsert_parity(row, p),
        UiEvent::Quarantined(q) => {
            if !row.quarantined.iter().any(|r| r.item_id == q.item_id) {
                row.quarantined.push(q);
            }
        }
        UiEvent::Card(
            c @ CardEvent {
                kind: CardKind::GoLive,
                ..
            },
        ) => row.go_live = Some(c),
        UiEvent::Snapshot(s) => {
            for step in s.steps {
                fold(row, UiEvent::Step(step));
            }
            row.tick = s.tick;
            if let Some(f) = s.last_frame {
                row.last_frame = Some(f);
            }
            if let Some(p) = s.parity {
                upsert_parity(row, p);
            }
        }
        UiEvent::Card(_) | UiEvent::Finding(_) | UiEvent::Done(_) | UiEvent::Error(_) => {}
    }
}

#[async_trait]
impl<D: Delegates> ConsoleReads for SqliteReads<D> {
    async fn session(&self, headers: &HeaderMap) -> ReadResult<Option<OwnerSession>> {
        self.delegates.session(headers).await
    }

    async fn landing(&self, headers: &HeaderMap) -> ReadResult<Option<String>> {
        self.delegates.landing(headers).await
    }

    async fn site(&self, id: &SiteId) -> ReadResult<Option<SiteRow>> {
        let row: Option<(String, String, i64, i64, i64, i64, Option<String>)> = sqlx::query_as(
            "SELECT s.domain, s.name, s.fixture, s.utc_offset_minutes, \
                EXISTS (SELECT 1 FROM site_pointer p WHERE p.site_id = s.site_id), \
                EXISTS (SELECT 1 FROM moves m WHERE m.site_id = s.site_id), \
                (SELECT MIN(r.signed_at) FROM releases r WHERE r.site_id = s.site_id AND r.state <> 'candidate') \
             FROM sites s WHERE s.site_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some((host, name, fixture, offset, live, moving, live_since)) = row else {
            return Ok(None);
        };
        Ok(Some(SiteRow {
            id: id.clone(),
            name: if name.is_empty() { host.clone() } else { name },
            host,
            fixture: fixture != 0,
            stage: if live != 0 {
                SiteStage::Live
            } else if moving != 0 {
                SiteStage::Moving
            } else {
                SiteStage::Setup
            },
            live_since: live_since.as_deref().map(time).transpose()?,
            utc_offset_minutes: i32::try_from(offset)
                .context("utc_offset_minutes is out of range")?,
        }))
    }

    async fn move_spend(&self, move_id: &MoveId) -> ReadResult<Option<String>> {
        Ok(Some(
            crate::db::usage::for_move(&self.pool, move_id)
                .await?
                .words("on this Move"),
        ))
    }

    async fn latest_move(&self, site: &SiteId) -> ReadResult<Option<MoveRow>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT move_id, state, created_at FROM moves WHERE site_id = ? ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(site.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some((id, state, created_at)) = row else {
            return Ok(None);
        };
        let started_at = time(&created_at)?;
        let mut row = MoveRow {
            id: MoveId::new(id),
            stage: match state.as_str() {
                "awaiting-owner" => MoveStage::Waiting,
                "live" => MoveStage::Live,
                "rolled-back" | "failed" | "stopped" => MoveStage::Stopped,
                _ => MoveStage::Running,
            },
            started_at,
            steps: Vec::new(),
            pages: None,
            tick: TickEvent {
                job: None,
                hosts: 0,
                requests: 0,
                bytes: 0,
                refusals: 0,
                at: started_at,
            },
            last_frame: None,
            parity_site: None,
            parity_pages: Vec::new(),
            quarantined: Vec::new(),
            go_live: None,
        };
        let events: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT seq, name, data FROM ui_events WHERE stream = ? AND site_id = ? ORDER BY seq",
        )
        .bind(format!("move:{}", row.id))
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        for (seq, name, data) in events {
            match UiEvent::from_parts(&name, &data) {
                Ok(event) => fold(&mut row, event),
                // One bad event shouldn't take the whole page down; the stream shows the rest.
                Err(e) => {
                    tracing::warn!(move_id = %row.id, seq, event = %name, error = %e, "skipping a UI event that does not parse")
                }
            }
        }
        row.steps
            .sort_by_key(|s| STEPS.iter().position(|n| *n == s.step));
        Ok(Some(row))
    }

    async fn decisions(&self, site: &SiteId) -> ReadResult<Vec<DecisionRow>> {
        let rows: Vec<DecisionSql> = sqlx::query_as(select_decisions!(
            "WHERE site_id = ? ORDER BY created_at DESC, rowid DESC"
        ))
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(decision_row).collect()
    }

    async fn decision(&self, site: &SiteId, id: &DecisionId) -> ReadResult<Option<DecisionRow>> {
        let row: Option<DecisionSql> =
            sqlx::query_as(select_decisions!("WHERE site_id = ? AND decision_id = ?"))
                .bind(site.as_str())
                .bind(id.as_str())
                .fetch_optional(&self.pool)
                .await?;
        row.map(decision_row).transpose()
    }

    async fn begin_approval(
        &self,
        session: &OwnerSession,
        card: &DecisionCard,
    ) -> ReadResult<Option<String>> {
        self.delegates.begin_approval(session, card).await
    }

    async fn levels(&self, site: &SiteId) -> ReadResult<Vec<JobLevel>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT body FROM charters WHERE site_id = ? ORDER BY version DESC LIMIT 1",
        )
        .bind(site.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some((body,)) = row else {
            return Ok(Vec::new());
        };
        let charter: Charter =
            serde_json::from_str(&body).context("a stored charter does not parse")?;
        Ok(charter.jobs)
    }

    async fn receipts(&self, site: &SiteId, limit: usize) -> ReadResult<Vec<Receipt>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT body FROM receipts WHERE site_id = ? ORDER BY issued_at DESC, rowid DESC LIMIT ?")
                .bind(site.as_str())
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|(body,)| serde_json::from_str(&body).context("a stored receipt does not parse"))
            .collect()
    }

    async fn receipt(&self, digest: &Digest) -> ReadResult<Option<Receipt>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT body FROM receipts WHERE digest = ?")
            .bind(digest.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|(body,)| serde_json::from_str(&body).context("a stored receipt does not parse"))
            .transpose()
    }

    /// The latest run with a receipt: a run still going has none, and Trust shows a finished one.
    async fn gauntlet(&self, site: &SiteId) -> ReadResult<Option<GauntletRow>> {
        let row: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT receipt, started_at, ended_at, detail FROM gauntlet_runs \
             WHERE site_id = ? AND receipt IS NOT NULL ORDER BY started_at DESC, rowid DESC LIMIT 1",
        )
        .bind(site.as_str())
        .fetch_optional(&self.pool)
        .await?;
        let Some((receipt, started_at, ended_at, detail)) = row else {
            return Ok(None);
        };
        let attacks: Vec<GauntletAttack> = match detail {
            Some(detail) => serde_json::from_str(&detail)
                .context("a stored gauntlet run's detail does not parse")?,
            None => Vec::new(),
        };
        Ok(Some(GauntletRow {
            run_at: time(ended_at.as_deref().unwrap_or(&started_at))?,
            receipt: digest(&receipt)?,
            attacks,
        }))
    }

    async fn pictures_waiting(&self, site: &SiteId) -> ReadResult<i64> {
        agentrix_media::library::waiting(&self.pool, site.as_str(), Some("move")).await
    }

    async fn uploads(&self, site: &SiteId) -> ReadResult<Vec<UploadInfo>> {
        let rows: Vec<(String, i64, String)> = sqlx::query_as(
            "SELECT kind, size, created_at FROM artifacts WHERE site_id = ? AND kind LIKE 'upload:%' \
             ORDER BY created_at DESC",
        )
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (kind, size, at) in rows {
            let name = kind.trim_start_matches("upload:").to_owned();
            if !seen.insert(name.clone()) {
                continue;
            }
            let at = DateTime::parse_from_rfc3339(&at)
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            out.push(UploadInfo {
                name,
                size: u64::try_from(size).unwrap_or(0),
                at,
            });
        }
        Ok(out)
    }

    async fn meter(&self, site: &SiteId) -> ReadResult<Option<MeterRow>> {
        use chrono::{Datelike as _, TimeZone as _};
        let now = Utc::now();
        let first = Utc
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .unwrap_or(now);
        let ever: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usage WHERE site_id = ?")
            .bind(site.as_str())
            .fetch_one(&self.pool)
            .await?;
        if ever.0 == 0 {
            return Ok(None);
        }
        let month = crate::db::usage::site_since(&self.pool, site, first).await?;
        // The owner's number is the monthly cap in the site's currency; the gateway's daily
        // limit stays an internal guard (specs/platform/facts.md, "AI budget").
        let budget = agentrix_shell::budget::read(&self.pool, None, site.as_str(), "", now).await?;
        Ok(Some(MeterRow { month, budget }))
    }

    async fn passed_questions(&self, site: &SiteId) -> ReadResult<PassedRows> {
        let store = crate::db::questions::QuestionStore::new(self.pool.clone());
        let recent = store
            .recent(site, 10)
            .await?
            .into_iter()
            .map(|q| {
                Ok(PassedRow {
                    question: q.question,
                    contact: q.contact,
                    at: time(&q.passed_at)?,
                })
            })
            .collect::<ReadResult<Vec<_>>>()?;
        Ok(PassedRows {
            total: store.count(site, None).await?,
            recent,
        })
    }

    async fn brief(&self, site: &SiteId) -> ReadResult<Option<BriefRow>> {
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT written_at, body FROM briefs WHERE site_id = ? ORDER BY written_at DESC LIMIT 1")
                .bind(site.as_str())
                .fetch_optional(&self.pool)
                .await?;
        let Some((written_at, body)) = row else {
            return Ok(None);
        };
        let brief: Brief = serde_json::from_str(&body).context("a stored brief does not parse")?;
        Ok(Some(BriefRow {
            written_at: time(&written_at)?,
            brief,
        }))
    }

    async fn quarantine(&self, site: &SiteId) -> ReadResult<Vec<QuarantineRow>> {
        let rows: Vec<(String, String, String, String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT q.item_id, q.title, q.reason, q.location, q.evidence, q.state, q.decision_id, d.state \
             FROM quarantine q LEFT JOIN decisions d ON d.decision_id = q.decision_id \
             WHERE q.site_id = ? AND q.state <> 'deleted' ORDER BY q.created_at, q.rowid",
        )
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(
                |(id, title, reason, location, evidence, state, decision, decision_state)| {
                    let evidence: Vec<Digest> = serde_json::from_str(&evidence)
                        .with_context(|| format!("quarantine {id}: evidence does not parse"))?;
                    let stage = match (state.as_str(), decision_state.as_deref()) {
                        ("restored", _) => QuarantineStage::Restored,
                        ("held", Some("waiting")) => QuarantineStage::RestoreAsked,
                        ("held", _) => QuarantineStage::LeftBehind,
                        (other, _) => bail!("quarantine {id}: unknown state {other}"),
                    };
                    Ok(QuarantineRow {
                        id: QuarantineId::new(id),
                        title,
                        plain: reason,
                        location,
                        evidence,
                        stage,
                        decision: decision.map(DecisionId::new),
                    })
                },
            )
            .collect()
    }

    async fn frame(
        &self,
        owner_site: Option<&SiteId>,
        digest: &Digest,
    ) -> ReadResult<Option<FrameBytes>> {
        self.delegates.frame(owner_site, digest).await
    }

    async fn keys(&self) -> ReadResult<KeySet> {
        Ok(self.keys.clone())
    }
}
