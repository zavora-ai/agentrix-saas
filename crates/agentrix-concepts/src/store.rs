//! The concept stage's records in `control.db`, with their own migrator (`migrations/`, 1400+).

use chrono::{SecondsFormat, Utc};
use futures::future::BoxFuture;
use serde::Serialize;
use sqlx::SqlitePool;

/// Runs this crate's migrations (node calls this once at startup, before mounting).
pub fn migrate(pool: &SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_concepts");
        m.run(pool)
            .await
            .map_err(|e| anyhow::anyhow!("agentrix-concepts migrations: {e}"))?;
        Ok(())
    })
}

/// Now, as stored.
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A run of three designs.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, PartialEq, Eq)]
pub struct RunRow {
    /// The run.
    pub run_id: String,
    /// The site.
    pub site_id: String,
    /// `planning`, `designing`, `done` or `failed`.
    pub state: String,
    /// The planner's model.
    pub planner_model: String,
    /// The designers' model.
    pub worker_model: String,
    /// What went wrong, in plain words, if anything.
    pub note: String,
    /// When it started.
    pub started_at: String,
    /// How long the whole run took.
    pub duration_ms: Option<i64>,
    /// The token the run's choose and guide forms carry.
    pub csrf: String,
    /// The planner as Vultr reported it: `vultr / glm-5.3`.
    pub planner_seen: String,
    /// The longest prompt this run sent.
    pub prompt_chars_max: i64,
    /// When the run's rooms were deleted (after the choice, or 24 hours on).
    pub rooms_deleted_at: Option<String>,
}

/// One design.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, PartialEq, Eq)]
pub struct ConceptRow {
    /// The design.
    pub concept_id: String,
    /// Its run.
    pub run_id: String,
    /// The site.
    pub site_id: String,
    /// 1, 2 or 3.
    pub n: i64,
    /// The direction's name.
    pub name: String,
    /// The direction in a sentence.
    pub direction: String,
    /// `waiting`, `writing`, `done` or `failed`.
    pub status: String,
    /// What went wrong, in plain words, if anything.
    pub note: String,
    /// The cleaned page in the content store.
    pub html_digest: Option<String>,
    /// The cleaned page's length.
    pub html_chars: i64,
    /// The model that wrote it.
    pub model: String,
    /// How long it took.
    pub duration_ms: Option<i64>,
    /// Whether the owner chose it.
    pub chosen: i64,
    /// When the owner chose it.
    pub chosen_at: Option<String>,
    /// The provider that answered, as Zavora reported it.
    pub provider: String,
    /// Check-and-fix rounds after the first write.
    pub rounds: i64,
    /// What the last check found missing, as a JSON array of keys.
    pub missing: String,
    /// Tools Zavora refused, by name, as a JSON object of counts.
    pub refused: String,
    /// `fs_write` calls that saved.
    pub writes: i64,
    /// Files in the room read to confirm the key isn't there.
    pub key_checked_files: i64,
    /// The design's receipt.
    pub page_receipt: Option<String>,
    /// Its model calls, as a JSON array of `{provider, model, kind, ms}`.
    pub calls: String,
}

/// The account that owns `site_id` (set by a verified claim), if any.
pub async fn owner_of(pool: &SqlitePool, site_id: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?1")
            .bind(site_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(o,)| o))
}

/// The site's domain and name.
pub async fn site(pool: &SqlitePool, site_id: &str) -> anyhow::Result<Option<(String, String)>> {
    Ok(
        sqlx::query_as("SELECT domain, name FROM sites WHERE site_id = ?1")
            .bind(site_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Records a new run.
pub async fn insert_run(
    pool: &SqlitePool,
    run_id: &str,
    site_id: &str,
    account: &str,
    planner_model: &str,
    worker_model: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO concept_runs (run_id, site_id, account, state, planner_model, worker_model, started_at, csrf)
         VALUES (?1, ?2, ?3, 'planning', ?4, ?5, ?6, ?7)",
    )
    .bind(run_id)
    .bind(site_id)
    .bind(account)
    .bind(planner_model)
    .bind(worker_model)
    .bind(now())
    .bind(format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    ))
    .execute(pool)
    .await?;
    Ok(())
}

/// Moves a run to `state`, with a note; `done` and `failed` record the end and duration.
pub async fn set_run_state(
    pool: &SqlitePool,
    run_id: &str,
    state: &str,
    note: &str,
    duration_ms: Option<i64>,
) -> anyhow::Result<()> {
    let ended = matches!(state, "done" | "failed").then(now);
    sqlx::query(
        "UPDATE concept_runs SET state = ?2, note = ?3, finished_at = COALESCE(?4, finished_at),
         duration_ms = COALESCE(?5, duration_ms) WHERE run_id = ?1",
    )
    .bind(run_id)
    .bind(state)
    .bind(note)
    .bind(ended)
    .bind(duration_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// How many runs `site_id` started in the last `window`.
pub async fn runs_since(
    pool: &SqlitePool,
    site_id: &str,
    window: chrono::Duration,
) -> anyhow::Result<i64> {
    let since = (Utc::now() - window).to_rfc3339_opts(SecondsFormat::Millis, true);
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM concept_runs WHERE site_id = ?1 AND started_at >= ?2")
            .bind(site_id)
            .bind(since)
            .fetch_one(pool)
            .await?;
    Ok(n)
}

/// Runs left unfinished by a restart are marked failed, so a new one can start.
pub async fn fail_unfinished(pool: &SqlitePool) -> anyhow::Result<u64> {
    Ok(sqlx::query(
        "UPDATE concept_runs SET state = 'failed', note = 'Stopped when Agentrix restarted. Try three more.', finished_at = ?1
         WHERE state IN ('planning', 'designing')",
    )
    .bind(now())
    .execute(pool)
    .await?
    .rows_affected())
}

/// Records a design waiting to be written.
#[allow(clippy::too_many_arguments)]
pub async fn insert_concept(
    pool: &SqlitePool,
    concept_id: &str,
    run_id: &str,
    site_id: &str,
    n: i64,
    name: &str,
    direction: &str,
    model: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO concepts (concept_id, run_id, site_id, n, name, direction, status, model)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'waiting', ?7)",
    )
    .bind(concept_id)
    .bind(run_id)
    .bind(site_id)
    .bind(n)
    .bind(name)
    .bind(direction)
    .bind(model)
    .execute(pool)
    .await?;
    Ok(())
}

/// A design started.
pub async fn concept_started(pool: &SqlitePool, concept_id: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE concepts SET status = 'writing', started_at = ?2 WHERE concept_id = ?1")
        .bind(concept_id)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(())
}

/// What a finished design records.
#[derive(Debug, Clone, Default)]
pub struct Finished<'a> {
    /// The cleaned page's digest and length, or why there is none.
    pub page: Option<(&'a str, i64)>,
    /// Why it failed, when it did.
    pub note: &'a str,
    /// The model and provider Zavora reported.
    pub model: Option<&'a str>,
    /// The provider Zavora reported.
    pub provider: Option<&'a str>,
    /// How long it took.
    pub duration_ms: i64,
    /// Check-and-fix rounds.
    pub rounds: i64,
    /// What the last check found missing (keys).
    pub missing: Vec<&'static str>,
    /// Files read to confirm the key isn't in the room.
    pub key_checked_files: i64,
    /// The design's receipt.
    pub receipt: Option<&'a str>,
}

/// A design finished: `done` with its stored page, or `failed` with a note.
pub async fn concept_finished(
    pool: &SqlitePool,
    concept_id: &str,
    f: &Finished<'_>,
) -> anyhow::Result<()> {
    let (status, digest, chars) = match f.page {
        Some((digest, chars)) => ("done", Some(digest), chars),
        None => ("failed", None, 0),
    };
    sqlx::query(
        "UPDATE concepts SET status = ?2, html_digest = COALESCE(?3, html_digest), html_chars = ?4,
         note = ?5, model = COALESCE(?6, model), provider = COALESCE(?7, provider), finished_at = ?8,
         duration_ms = ?9, rounds = ?10, missing = ?11, key_checked_files = MAX(key_checked_files, ?12),
         page_receipt = COALESCE(?13, page_receipt) WHERE concept_id = ?1",
    )
    .bind(concept_id)
    .bind(status)
    .bind(digest)
    .bind(chars)
    .bind(f.note)
    .bind(f.model)
    .bind(f.provider)
    .bind(now())
    .bind(f.duration_ms)
    .bind(f.rounds)
    .bind(serde_json::to_string(&f.missing).unwrap_or_else(|_| "[]".into()))
    .bind(f.key_checked_files)
    .bind(f.receipt)
    .execute(pool)
    .await?;
    Ok(())
}

/// A revision of a finished design: its new page, rounds added, what's still missing.
pub async fn concept_revised(
    pool: &SqlitePool,
    concept_id: &str,
    page: (&str, i64),
    rounds_added: i64,
    missing: &[&'static str],
    receipt: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE concepts SET html_digest = ?2, html_chars = ?3, rounds = rounds + ?4, missing = ?5,
         page_receipt = COALESCE(?6, page_receipt) WHERE concept_id = ?1",
    )
    .bind(concept_id)
    .bind(page.0)
    .bind(page.1)
    .bind(rounds_added)
    .bind(serde_json::to_string(missing).unwrap_or_else(|_| "[]".into()))
    .bind(receipt)
    .execute(pool)
    .await?;
    Ok(())
}

/// Live progress while a designer writes: characters in its file and saved writes.
pub async fn concept_progress(
    pool: &SqlitePool,
    concept_id: &str,
    chars: i64,
    writes: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE concepts SET html_chars = ?2, writes = ?3 WHERE concept_id = ?1")
        .bind(concept_id)
        .bind(chars)
        .bind(writes)
        .execute(pool)
        .await?;
    Ok(())
}

/// The designer's model calls so far.
pub async fn concept_calls(
    pool: &SqlitePool,
    concept_id: &str,
    calls: &serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE concepts SET calls = ?2 WHERE concept_id = ?1")
        .bind(concept_id)
        .bind(calls.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Tools Zavora refused this designer, by name.
pub async fn concept_refused(
    pool: &SqlitePool,
    concept_id: &str,
    refused: &std::collections::BTreeMap<String, i64>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE concepts SET refused = ?2 WHERE concept_id = ?1")
        .bind(concept_id)
        .bind(serde_json::to_string(refused).unwrap_or_else(|_| "{}".into()))
        .execute(pool)
        .await?;
    Ok(())
}

/// The planner as Vultr reported it, and the longest prompt so far.
pub async fn run_seen(
    pool: &SqlitePool,
    run_id: &str,
    planner_seen: Option<&str>,
    prompt_chars: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE concept_runs SET planner_seen = COALESCE(?2, planner_seen),
         prompt_chars_max = MAX(prompt_chars_max, ?3) WHERE run_id = ?1",
    )
    .bind(run_id)
    .bind(planner_seen)
    .bind(prompt_chars)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records that `run_id`'s rooms were deleted.
pub async fn rooms_deleted(pool: &SqlitePool, run_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE concept_runs SET rooms_deleted_at = ?2 WHERE run_id = ?1 AND rooms_deleted_at IS NULL",
    )
    .bind(run_id)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Finished runs whose rooms still exist: all of `site_id`'s, or (with no site) those older
/// than `older_than`.
pub async fn runs_with_rooms(
    pool: &SqlitePool,
    site_id: Option<&str>,
    older_than: chrono::Duration,
) -> anyhow::Result<Vec<String>> {
    let cutoff = (Utc::now() - older_than).to_rfc3339_opts(SecondsFormat::Millis, true);
    let rows: Vec<(String,)> = match site_id {
        Some(site) => {
            sqlx::query_as(
                "SELECT run_id FROM concept_runs WHERE site_id = ?1 AND rooms_deleted_at IS NULL
                 AND state IN ('done', 'failed')",
            )
            .bind(site)
            .fetch_all(pool)
            .await?
        }
        None => sqlx::query_as(
            "SELECT run_id FROM concept_runs WHERE rooms_deleted_at IS NULL AND started_at < ?1",
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await?,
    };
    Ok(rows.into_iter().map(|(r,)| r).collect())
}

/// The designs of a run, by id.
pub async fn concept_ids(pool: &SqlitePool, run_id: &str) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT concept_id FROM concepts WHERE run_id = ?1")
        .bind(run_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(c,)| c).collect())
}

/// Records the owner's request to change one design.
pub async fn insert_change(
    pool: &SqlitePool,
    change_id: &str,
    concept_id: &str,
    site_id: &str,
    account: &str,
    words: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO concept_changes (change_id, concept_id, site_id, account, words, state, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
    )
    .bind(change_id)
    .bind(concept_id)
    .bind(site_id)
    .bind(account)
    .bind(words)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// A change finished, or didn't.
pub async fn change_finished(
    pool: &SqlitePool,
    change_id: &str,
    done: bool,
    note: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE concept_changes SET state = ?2, note = ?3, finished_at = ?4 WHERE change_id = ?1",
    )
    .bind(change_id)
    .bind(if done { "done" } else { "failed" })
    .bind(note)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// The owner's changes for `site_id` in the last `window`.
pub async fn changes_since(
    pool: &SqlitePool,
    site_id: &str,
    window: chrono::Duration,
) -> anyhow::Result<i64> {
    let since = (Utc::now() - window).to_rfc3339_opts(SecondsFormat::Millis, true);
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM concept_changes WHERE site_id = ?1 AND created_at >= ?2",
    )
    .bind(site_id)
    .bind(since)
    .fetch_one(pool)
    .await?;
    Ok(n)
}

/// The owner's finished changes to each design of `run_id`: `(concept_id, count)`.
pub async fn changes_by_concept(
    pool: &SqlitePool,
    run_id: &str,
) -> anyhow::Result<Vec<(String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT c.concept_id, COUNT(*) FROM concept_changes c JOIN concepts k ON k.concept_id = c.concept_id
         WHERE k.run_id = ?1 AND c.state = 'done' GROUP BY c.concept_id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?)
}

/// `SELECT <every column ConceptRow reads> FROM concepts <rest>`, as one literal.
macro_rules! select_concepts {
    ($rest:literal) => {
        concat!(
            "SELECT concept_id, run_id, site_id, n, name, direction, status, note, html_digest, html_chars, model, duration_ms, chosen, chosen_at, provider, rounds, missing, refused, writes, key_checked_files, page_receipt, calls FROM concepts ",
            $rest
        )
    };
}

/// The site's newest run and its designs.
pub async fn latest_run(
    pool: &SqlitePool,
    site_id: &str,
) -> anyhow::Result<Option<(RunRow, Vec<ConceptRow>)>> {
    let run: Option<RunRow> = sqlx::query_as(
        "SELECT run_id, site_id, state, planner_model, worker_model, note, started_at, duration_ms,
         csrf, planner_seen, prompt_chars_max, rooms_deleted_at
         FROM concept_runs WHERE site_id = ?1 ORDER BY started_at DESC, rowid DESC LIMIT 1",
    )
    .bind(site_id)
    .fetch_optional(pool)
    .await?;
    let Some(run) = run else {
        return Ok(None);
    };
    let concepts = sqlx::query_as(select_concepts!("WHERE run_id = ?1 ORDER BY n"))
        .bind(&run.run_id)
        .fetch_all(pool)
        .await?;
    Ok(Some((run, concepts)))
}

/// One design of `site_id`.
pub async fn concept(
    pool: &SqlitePool,
    site_id: &str,
    concept_id: &str,
) -> anyhow::Result<Option<ConceptRow>> {
    Ok(
        sqlx::query_as(select_concepts!("WHERE site_id = ?1 AND concept_id = ?2"))
            .bind(site_id)
            .bind(concept_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// The design the owner chose for `site_id`, if any. The Move page shows it and the rebuild
/// takes its look from it.
pub async fn chosen(pool: &SqlitePool, site_id: &str) -> anyhow::Result<Option<ConceptRow>> {
    Ok(sqlx::query_as(select_concepts!(
        "WHERE site_id = ?1 AND chosen = 1 ORDER BY chosen_at DESC LIMIT 1"
    ))
    .bind(site_id)
    .fetch_optional(pool)
    .await?)
}

/// Records `concept_id` as the chosen look for `site_id`, replacing an earlier choice.
pub async fn choose(
    pool: &SqlitePool,
    site_id: &str,
    concept_id: &str,
    receipt: Option<&str>,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE concepts SET chosen = 0, chosen_at = NULL WHERE site_id = ?1 AND chosen = 1",
    )
    .bind(site_id)
    .execute(&mut *tx)
    .await?;
    let done = sqlx::query(
        "UPDATE concepts SET chosen = 1, chosen_at = ?3, receipt = ?4
         WHERE site_id = ?1 AND concept_id = ?2 AND status = 'done'",
    )
    .bind(site_id)
    .bind(concept_id)
    .bind(now())
    .bind(receipt)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        == 1;
    if done {
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    Ok(done)
}
