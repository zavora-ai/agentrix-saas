//! The move lane's tables in `control.db` (design §5): `moves`, `move_steps`, `findings`,
//! `quarantine`, `decisions`, `releases`, `site_pointer` and `charters`. This module is their
//! one writer. The schema is the data lane's migration (`agentrix-node/src/db/migrations`).

use std::str::FromStr;

use agentrix_types::ui::{StepName, StepState};
use agentrix_types::{DecisionCard, DecisionId, Digest, Finding, MoveId, QuarantineId, SiteId};
use anyhow::Context as _;
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::clean::Quarantined;
use crate::state::{MoveState, parse_step, step_str};
use agentrix_types::Charter;

/// RFC 3339 UTC with microseconds, so text order is time order (the schema's convention).
pub fn time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_time(text: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(text)
        .with_context(|| format!("stored time {text}"))?
        .with_timezone(&Utc))
}

fn digest_opt(text: Option<String>) -> anyhow::Result<Option<Digest>> {
    text.map(|t| Digest::from_str(&t).map_err(anyhow::Error::from))
        .transpose()
}

fn step_state_str(state: StepState) -> &'static str {
    match state {
        StepState::Pending => "pending",
        StepState::Running => "running",
        StepState::Waiting => "waiting",
        StepState::Done => "done",
        StepState::Failed => "failed",
    }
}

fn parse_step_state(text: &str) -> Option<StepState> {
    [
        StepState::Pending,
        StepState::Running,
        StepState::Waiting,
        StepState::Done,
        StepState::Failed,
    ]
    .into_iter()
    .find(|s| step_state_str(*s) == text)
}

/// A Move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRecord {
    /// The Move.
    pub id: MoveId,
    /// The site.
    pub site: SiteId,
    /// Where it is.
    pub state: MoveState,
    /// When it started.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// One step row.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    /// Its sequence in the Move.
    pub seq: u64,
    /// The step.
    pub step: StepName,
    /// Its state.
    pub state: StepState,
    /// Plain words.
    pub summary: String,
    /// Its receipt.
    pub receipt: Option<Digest>,
    /// Resume data.
    pub resume: Option<Value>,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// When it ended.
    pub ended_at: Option<DateTime<Utc>>,
}

/// Where a quarantined item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineState {
    /// Kept out.
    Held,
    /// Restored by a signed decision.
    Restored,
    /// Deleted.
    Deleted,
}

/// A quarantined item as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRecord {
    /// The item.
    pub id: QuarantineId,
    /// The Move that left it behind.
    pub move_id: Option<MoveId>,
    /// What it is.
    pub title: String,
    /// Why it was left behind.
    pub reason: String,
    /// Where it was found.
    pub location: String,
    /// Evidence in the CAS.
    pub evidence: Vec<Digest>,
    /// Where it is.
    pub state: QuarantineState,
    /// The restore card, once asked.
    pub decision: Option<DecisionId>,
}

/// A decision's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardState {
    /// Waiting for the owner.
    Waiting,
    /// Approved.
    Approved,
    /// Declined.
    Declined,
    /// Expired unanswered.
    Expired,
}

impl CardState {
    fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(match text {
            "waiting" => Self::Waiting,
            "approved" => Self::Approved,
            "declined" => Self::Declined,
            "expired" => Self::Expired,
            other => anyhow::bail!("unknown decision state {other}"),
        })
    }
}

/// A decision card as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct CardRecord {
    /// The card.
    pub card: DecisionCard,
    /// Its state.
    pub state: CardState,
    /// `passkey` or `totp`, once approved.
    pub method: Option<String>,
    /// Who decided.
    pub signer: Option<String>,
    /// The approval evidence (assertion or step-up), as JSON.
    pub assertion: Option<Value>,
    /// The decision's receipt.
    pub receipt: Option<Digest>,
    /// When it was decided.
    pub decided_at: Option<DateTime<Utc>>,
}

/// Where a release is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseState {
    /// Proved, waiting for the owner.
    Candidate,
    /// Served.
    Live,
    /// Replaced by a newer release.
    Superseded,
    /// Undone by the owner.
    RolledBack,
}

impl ReleaseState {
    fn parse(text: &str) -> anyhow::Result<Self> {
        Ok(match text {
            "candidate" => Self::Candidate,
            "live" => Self::Live,
            "superseded" => Self::Superseded,
            "rolled-back" => Self::RolledBack,
            other => anyhow::bail!("unknown release state {other}"),
        })
    }
}

/// A release as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRecord {
    /// The release number.
    pub seq: u64,
    /// The manifest digest.
    pub manifest_digest: Digest,
    /// The signed checkpoint note, once signed.
    pub checkpoint: Option<String>,
    /// Where it is.
    pub state: ReleaseState,
    /// The go-live decision.
    pub decision: Option<DecisionId>,
    /// When it was staged.
    pub created_at: DateTime<Utc>,
    /// When it was signed.
    pub signed_at: Option<DateTime<Utc>>,
}

/// A site, as the move lane reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteRecord {
    /// The site.
    pub id: SiteId,
    /// Its domain.
    pub domain: String,
    /// The business name.
    pub name: String,
    /// The owner's account, once claimed.
    pub owner: Option<String>,
    /// Zavora's fixture site.
    pub fixture: bool,
}

/// The move lane's store.
#[derive(Debug, Clone)]
pub struct MoveStore {
    pool: SqlitePool,
}

type ReleaseRow = (
    i64,
    String,
    Option<String>,
    String,
    Option<String>,
    String,
    Option<String>,
);

fn release_of(row: ReleaseRow) -> anyhow::Result<ReleaseRecord> {
    let (seq, digest, checkpoint, state, decision, created, signed) = row;
    Ok(ReleaseRecord {
        seq: u64::try_from(seq)?,
        manifest_digest: digest.parse()?,
        checkpoint,
        state: ReleaseState::parse(&state)?,
        decision: decision.map(DecisionId::new),
        created_at: parse_time(&created)?,
        signed_at: signed.as_deref().map(parse_time).transpose()?,
    })
}

type CardRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn card_of(row: CardRow) -> anyhow::Result<CardRecord> {
    let (card, state, method, signer, assertion, receipt, decided) = row;
    Ok(CardRecord {
        card: serde_json::from_str(&card).context("a stored card does not parse")?,
        state: CardState::parse(&state)?,
        method,
        signer,
        assertion: assertion.as_deref().map(serde_json::from_str).transpose()?,
        receipt: digest_opt(receipt)?,
        decided_at: decided.as_deref().map(parse_time).transpose()?,
    })
}

type QuarantineRow = (
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

fn quarantine_of(row: QuarantineRow) -> anyhow::Result<QuarantineRecord> {
    let (id, move_id, title, reason, location, evidence, state, decision) = row;
    Ok(QuarantineRecord {
        id: QuarantineId::new(id),
        move_id: move_id.map(MoveId::new),
        title,
        reason,
        location,
        evidence: serde_json::from_str(&evidence)?,
        state: match state.as_str() {
            "held" => QuarantineState::Held,
            "restored" => QuarantineState::Restored,
            _ => QuarantineState::Deleted,
        },
        decision: decision.map(DecisionId::new),
    })
}

impl MoveStore {
    /// A store over `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ---------- sites (read only; the data lane writes them) ----------

    /// A site.
    pub async fn site(&self, id: &SiteId) -> anyhow::Result<Option<SiteRecord>> {
        let row: Option<(String, String, String, Option<String>, i64)> = sqlx::query_as(
            "SELECT site_id, domain, name, owner_account_id, fixture FROM sites WHERE site_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id, domain, name, owner, fixture)| SiteRecord {
            id: SiteId::new(id),
            domain,
            name,
            owner,
            fixture: fixture != 0,
        }))
    }

    /// The site served at `host` (lowercase, no port).
    pub async fn site_by_host(&self, host: &str) -> anyhow::Result<Option<SiteRecord>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT site_id FROM sites WHERE domain = ?")
            .bind(host)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some((id,)) => self.site(&SiteId::new(id)).await,
            None => Ok(None),
        }
    }

    // ---------- moves ----------

    /// Starts a Move in `state`.
    pub async fn create_move(
        &self,
        site: &SiteId,
        state: MoveState,
        now: DateTime<Utc>,
    ) -> anyhow::Result<MoveId> {
        let id = MoveId::generate();
        sqlx::query(
            "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(site.as_str())
        .bind(state.as_str())
        .bind(time(now))
        .bind(time(now))
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// A Move.
    pub async fn move_record(&self, id: &MoveId) -> anyhow::Result<Option<MoveRecord>> {
        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT move_id, site_id, state, created_at, updated_at FROM moves WHERE move_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::move_of).transpose()
    }

    /// The site's latest Move.
    pub async fn latest_move(&self, site: &SiteId) -> anyhow::Result<Option<MoveRecord>> {
        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT move_id, site_id, state, created_at, updated_at FROM moves WHERE site_id = ? \
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(site.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Self::move_of).transpose()
    }

    /// Moves that a crash may have interrupted: neither waiting for the owner nor ended.
    pub async fn unfinished_moves(&self) -> anyhow::Result<Vec<MoveId>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT move_id FROM moves WHERE state IN ('claimed', 'replayed', 'cleaned', 'rebuilt', 'proved') \
             ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| MoveId::new(id)).collect())
    }

    fn move_of(row: (String, String, String, String, String)) -> anyhow::Result<MoveRecord> {
        let (id, site, state, created, updated) = row;
        Ok(MoveRecord {
            id: MoveId::new(id),
            site: SiteId::new(site),
            state: MoveState::parse(&state).with_context(|| format!("move state {state}"))?,
            created_at: parse_time(&created)?,
            updated_at: parse_time(&updated)?,
        })
    }

    /// Sets a Move's state.
    pub async fn set_state(
        &self,
        id: &MoveId,
        state: MoveState,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE moves SET state = ?, updated_at = ? WHERE move_id = ?")
            .bind(state.as_str())
            .bind(time(now))
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---------- steps ----------

    /// Appends a step row.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_step(
        &self,
        move_id: &MoveId,
        site: &SiteId,
        step: StepName,
        state: StepState,
        summary: &str,
        receipt: Option<&Digest>,
        resume: Option<&Value>,
        started_at: DateTime<Utc>,
        ended_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<u64> {
        let resume = resume.map(serde_json::to_string).transpose()?;
        let (seq,): (i64,) = sqlx::query_as(
            "INSERT INTO move_steps (move_id, seq, site_id, step, state, summary, receipt, resume, started_at, ended_at) \
             VALUES (?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM move_steps WHERE move_id = ?), ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING seq",
        )
        .bind(move_id.as_str())
        .bind(move_id.as_str())
        .bind(site.as_str())
        .bind(step_str(step))
        .bind(step_state_str(state))
        .bind(summary)
        .bind(receipt.map(ToString::to_string))
        .bind(resume)
        .bind(time(started_at))
        .bind(ended_at.map(time))
        .fetch_one(&self.pool)
        .await?;
        Ok(u64::try_from(seq)?)
    }

    /// Every step row of a Move, in order.
    pub async fn steps(&self, move_id: &MoveId) -> anyhow::Result<Vec<StepRecord>> {
        type Row = (
            i64,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
        );
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT seq, step, state, summary, receipt, resume, started_at, ended_at FROM move_steps \
             WHERE move_id = ? ORDER BY seq",
        )
        .bind(move_id.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(
                |(seq, step, state, summary, receipt, resume, started, ended)| {
                    Ok(StepRecord {
                        seq: u64::try_from(seq)?,
                        step: parse_step(&step).with_context(|| format!("step {step}"))?,
                        state: parse_step_state(&state)
                            .with_context(|| format!("step state {state}"))?,
                        summary,
                        receipt: digest_opt(receipt)?,
                        resume: resume.as_deref().map(serde_json::from_str).transpose()?,
                        started_at: parse_time(&started)?,
                        ended_at: ended.as_deref().map(parse_time).transpose()?,
                    })
                },
            )
            .collect()
    }

    /// The resume data of the step's latest completed row.
    pub async fn resume_of(
        &self,
        move_id: &MoveId,
        step: StepName,
    ) -> anyhow::Result<Option<Value>> {
        Ok(self
            .steps(move_id)
            .await?
            .into_iter()
            .rev()
            .find(|s| s.step == step && matches!(s.state, StepState::Done | StepState::Waiting))
            .and_then(|s| s.resume))
    }

    // ---------- findings and quarantine ----------

    /// Stores a Move's findings.
    pub async fn insert_findings(
        &self,
        move_id: &MoveId,
        findings: &[Finding],
    ) -> anyhow::Result<()> {
        for f in findings {
            sqlx::query(
                "INSERT INTO findings (finding_id, site_id, move_id, detector, severity, category, title, \
                 explanation, location, evidence, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (finding_id) DO NOTHING",
            )
            .bind(f.id.as_str())
            .bind(f.site.as_str())
            .bind(move_id.as_str())
            .bind(&f.detector)
            .bind(serde_json::to_value(f.severity)?.as_str().unwrap_or("info").to_owned())
            .bind(serde_json::to_value(f.category)?.as_str().unwrap_or("security").to_owned())
            .bind(&f.title)
            .bind(&f.explanation)
            .bind(&f.location)
            .bind(serde_json::to_string(&f.evidence)?)
            .bind(time(f.at))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Stores a Move's quarantined items as held.
    pub async fn insert_quarantine(
        &self,
        site: &SiteId,
        move_id: &MoveId,
        items: &[Quarantined],
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        for q in items {
            sqlx::query(
                "INSERT INTO quarantine (item_id, site_id, move_id, title, reason, location, evidence, state, \
                 created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'held', ?, ?) ON CONFLICT (item_id) DO NOTHING",
            )
            .bind(q.id.as_str())
            .bind(site.as_str())
            .bind(move_id.as_str())
            .bind(&q.title)
            .bind(&q.reason)
            .bind(&q.location)
            .bind(serde_json::to_string(&q.evidence)?)
            .bind(time(now))
            .bind(time(now))
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// The site's quarantined items, newest first.
    pub async fn quarantine(&self, site: &SiteId) -> anyhow::Result<Vec<QuarantineRecord>> {
        let rows: Vec<QuarantineRow> = sqlx::query_as(
            "SELECT item_id, move_id, title, reason, location, evidence, state, decision_id FROM quarantine \
             WHERE site_id = ? ORDER BY created_at DESC, rowid DESC",
        )
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(quarantine_of).collect()
    }

    /// One quarantined item of the site.
    pub async fn quarantine_item(
        &self,
        site: &SiteId,
        id: &QuarantineId,
    ) -> anyhow::Result<Option<QuarantineRecord>> {
        let row: Option<QuarantineRow> = sqlx::query_as(
            "SELECT item_id, move_id, title, reason, location, evidence, state, decision_id FROM quarantine \
             WHERE site_id = ? AND item_id = ?",
        )
        .bind(site.as_str())
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(quarantine_of).transpose()
    }

    /// Links an item to the restore card the owner asked for.
    pub async fn ask_restore(
        &self,
        site: &SiteId,
        id: &QuarantineId,
        decision: &DecisionId,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE quarantine SET decision_id = ?, updated_at = ? WHERE site_id = ? AND item_id = ? AND state = 'held'",
        )
        .bind(decision.as_str())
        .bind(time(now))
        .bind(site.as_str())
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Marks an item restored after its card was approved.
    pub async fn mark_restored(
        &self,
        site: &SiteId,
        id: &QuarantineId,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE quarantine SET state = 'restored', updated_at = ? WHERE site_id = ? AND item_id = ?",
        )
        .bind(time(now))
        .bind(site.as_str())
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Clears the restore request when its card is declined.
    pub async fn clear_restore(
        &self,
        site: &SiteId,
        id: &QuarantineId,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE quarantine SET decision_id = NULL, updated_at = ? WHERE site_id = ? AND item_id = ? AND state = 'held'",
        )
        .bind(time(now))
        .bind(site.as_str())
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---------- decisions ----------

    /// Stores a waiting card.
    pub async fn insert_card(&self, card: &DecisionCard) -> anyhow::Result<()> {
        let kind = serde_json::to_value(card.kind)?;
        sqlx::query(
            "INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, created_at) \
             VALUES (?, ?, ?, ?, ?, 'waiting', ?)",
        )
        .bind(card.decision_id.as_str())
        .bind(card.site.as_str())
        .bind(kind.as_str().unwrap_or_default().to_owned())
        .bind(serde_json::to_string(card)?)
        .bind(card.fingerprint.to_string())
        .bind(time(card.created_at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// One card.
    pub async fn card(&self, id: &DecisionId) -> anyhow::Result<Option<CardRecord>> {
        let row: Option<CardRow> = sqlx::query_as(
            "SELECT card, state, method, signer, assertion, receipt, decided_at FROM decisions WHERE decision_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(card_of).transpose()
    }

    /// The site's cards, newest first.
    pub async fn cards(&self, site: &SiteId) -> anyhow::Result<Vec<CardRecord>> {
        let rows: Vec<CardRow> = sqlx::query_as(
            "SELECT card, state, method, signer, assertion, receipt, decided_at FROM decisions WHERE site_id = ? \
             ORDER BY created_at DESC, rowid DESC",
        )
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(card_of).collect()
    }

    /// Records an approval. False when the card was no longer waiting (decided twice).
    pub async fn approve(
        &self,
        id: &DecisionId,
        method: &str,
        signer: &str,
        assertion: &Value,
        receipt: &Digest,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "UPDATE decisions SET state = 'approved', method = ?, signer = ?, assertion = ?, receipt = ?, decided_at = ? \
             WHERE decision_id = ? AND state = 'waiting'",
        )
        .bind(method)
        .bind(signer)
        .bind(serde_json::to_string(assertion)?)
        .bind(receipt.to_string())
        .bind(time(now))
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Records a decline. False when the card was no longer waiting.
    pub async fn decline(
        &self,
        id: &DecisionId,
        signer: &str,
        receipt: &Digest,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "UPDATE decisions SET state = 'declined', signer = ?, receipt = ?, decided_at = ? \
             WHERE decision_id = ? AND state = 'waiting'",
        )
        .bind(signer)
        .bind(receipt.to_string())
        .bind(time(now))
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Marks a waiting card expired (never as declined).
    pub async fn expire(&self, id: &DecisionId) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE decisions SET state = 'expired' WHERE decision_id = ? AND state = 'waiting'",
        )
        .bind(id.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---------- releases and the pointer ----------

    /// Stages a candidate at `seq`, replacing an earlier unsigned candidate at the same number.
    pub async fn put_candidate(
        &self,
        site: &SiteId,
        seq: u64,
        manifest: &Digest,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM releases WHERE site_id = ? AND seq = ? AND state = 'candidate'")
            .bind(site.as_str())
            .bind(i64::try_from(seq)?)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO releases (site_id, seq, manifest_digest, state, created_at) VALUES (?, ?, ?, 'candidate', ?)",
        )
        .bind(site.as_str())
        .bind(i64::try_from(seq)?)
        .bind(manifest.to_string())
        .bind(time(now))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One release.
    pub async fn release(&self, site: &SiteId, seq: u64) -> anyhow::Result<Option<ReleaseRecord>> {
        let row: Option<ReleaseRow> = sqlx::query_as(
            "SELECT seq, manifest_digest, checkpoint, state, decision_id, created_at, signed_at FROM releases \
             WHERE site_id = ? AND seq = ?",
        )
        .bind(site.as_str())
        .bind(i64::try_from(seq)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(release_of).transpose()
    }

    /// Every release of the site, by number.
    pub async fn releases(&self, site: &SiteId) -> anyhow::Result<Vec<ReleaseRecord>> {
        let rows: Vec<ReleaseRow> = sqlx::query_as(
            "SELECT seq, manifest_digest, checkpoint, state, decision_id, created_at, signed_at FROM releases \
             WHERE site_id = ? ORDER BY seq",
        )
        .bind(site.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(release_of).collect()
    }

    /// The signed releases, by number: the site log's entries.
    pub async fn signed_releases(&self, site: &SiteId) -> anyhow::Result<Vec<ReleaseRecord>> {
        Ok(self
            .releases(site)
            .await?
            .into_iter()
            .filter(|r| r.checkpoint.is_some())
            .collect())
    }

    /// Signs release `seq` live in one transaction: the release gets its checkpoint and
    /// decision, the previous live release is superseded, and the pointer moves (R7.3).
    pub async fn go_live(
        &self,
        site: &SiteId,
        seq: u64,
        checkpoint: &str,
        decision: &DecisionId,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let seq_i = i64::try_from(seq)?;
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE releases SET state = 'live', checkpoint = ?, decision_id = ?, signed_at = ? \
             WHERE site_id = ? AND seq = ? AND state = 'candidate'",
        )
        .bind(checkpoint)
        .bind(decision.as_str())
        .bind(time(now))
        .bind(site.as_str())
        .bind(seq_i)
        .execute(&mut *tx)
        .await?;
        anyhow::ensure!(
            changed.rows_affected() == 1,
            "release {seq} is not a candidate"
        );
        sqlx::query("UPDATE releases SET state = 'superseded' WHERE site_id = ? AND state = 'live' AND seq <> ?")
            .bind(site.as_str())
            .bind(seq_i)
            .execute(&mut *tx)
            .await?;
        // Every signed release carries the newest checkpoint, which covers them all.
        sqlx::query(
            "UPDATE releases SET checkpoint = ? WHERE site_id = ? AND checkpoint IS NOT NULL",
        )
        .bind(checkpoint)
        .bind(site.as_str())
        .execute(&mut *tx)
        .await?;
        Self::point(&mut tx, site, seq_i, now).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn point(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        site: &SiteId,
        seq: i64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO site_pointer (site_id, seq, manifest_digest, updated_at) \
             SELECT site_id, seq, manifest_digest, ? FROM releases WHERE site_id = ? AND seq = ? \
             ON CONFLICT (site_id) DO UPDATE SET seq = excluded.seq, manifest_digest = excluded.manifest_digest, \
             updated_at = excluded.updated_at",
        )
        .bind(time(now))
        .bind(site.as_str())
        .bind(seq)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Rolls the pointer back to signed release `to` in one transaction (R7.4): the current
    /// live release becomes rolled back and `to` is live again.
    pub async fn roll_back_to(
        &self,
        site: &SiteId,
        to: u64,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let to_i = i64::try_from(to)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE releases SET state = 'rolled-back' WHERE site_id = ? AND state = 'live'",
        )
        .bind(site.as_str())
        .execute(&mut *tx)
        .await?;
        let changed = sqlx::query(
            "UPDATE releases SET state = 'live' WHERE site_id = ? AND seq = ? AND checkpoint IS NOT NULL",
        )
        .bind(site.as_str())
        .bind(to_i)
        .execute(&mut *tx)
        .await?;
        anyhow::ensure!(
            changed.rows_affected() == 1,
            "release {to} was never signed"
        );
        Self::point(&mut tx, site, to_i, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Takes the site off the platform: the pointer is removed, so the old site (kept as it
    /// was) is what the domain shows again (R7.4).
    pub async fn roll_back_to_old_site(&self, site: &SiteId) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM site_pointer WHERE site_id = ?")
            .bind(site.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE releases SET state = 'rolled-back' WHERE site_id = ? AND state = 'live'",
        )
        .bind(site.as_str())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The live pointer: release number and manifest digest.
    pub async fn pointer(&self, site: &SiteId) -> anyhow::Result<Option<(u64, Digest)>> {
        let row: Option<(i64, String)> =
            sqlx::query_as("SELECT seq, manifest_digest FROM site_pointer WHERE site_id = ?")
                .bind(site.as_str())
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(seq, d)| Ok((u64::try_from(seq)?, d.parse()?)))
            .transpose()
    }

    // ---------- charters ----------

    /// Stores the charter at `version`, unsigned, unless a signed one exists.
    pub async fn put_charter(
        &self,
        site: &SiteId,
        version: u32,
        charter: &Charter,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO charters (site_id, version, body, digest, created_at) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (site_id, version) DO UPDATE SET body = excluded.body, digest = excluded.digest \
             WHERE charters.signed_by IS NULL",
        )
        .bind(site.as_str())
        .bind(i64::from(version))
        .bind(String::from_utf8(agentrix_types::canonical_json(charter)?)?)
        .bind(crate::golive::charter_digest(charter)?.to_string())
        .bind(time(now))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The charter at `version`, with its digest and signer.
    pub async fn charter(
        &self,
        site: &SiteId,
        version: u32,
    ) -> anyhow::Result<Option<(Charter, Digest, Option<String>)>> {
        let row: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT body, digest, signed_by FROM charters WHERE site_id = ? AND version = ?",
        )
        .bind(site.as_str())
        .bind(i64::from(version))
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(body, digest, signer)| {
            Ok((serde_json::from_str(&body)?, digest.parse()?, signer))
        })
        .transpose()
    }

    /// Records the owner's signature on the charter (the go-live approval's evidence).
    pub async fn sign_charter(
        &self,
        site: &SiteId,
        version: u32,
        signer: &str,
        signature: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE charters SET signed_by = ?, signature = ? WHERE site_id = ? AND version = ? AND signed_by IS NULL",
        )
        .bind(signer)
        .bind(signature)
        .bind(site.as_str())
        .bind(i64::from(version))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
