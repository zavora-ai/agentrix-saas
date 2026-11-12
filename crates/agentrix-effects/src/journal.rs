//! The effects journal: one row per fingerprint, so a request that comes twice runs once.

use sqlx::SqlitePool;

use crate::{Outcome, now};

/// A journal row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EffectRow {
    /// `sha256:…`.
    pub fingerprint: String,
    /// started | executed | not_executed | unknown.
    pub state: String,
    /// The platform receipt.
    pub receipt: Option<String>,
}

/// Claims `fingerprint` for a new effect. `Ok(None)` when this call owns it now; `Ok(Some(row))`
/// when it already existed (the caller returns that row's result instead of running again).
pub async fn claim(
    pool: &SqlitePool,
    fingerprint: &str,
    site: &str,
    tool: &str,
    subject: &str,
) -> Result<Option<EffectRow>, sqlx::Error> {
    let at = now();
    let done = sqlx::query(
        "INSERT OR IGNORE INTO effects (fingerprint, site_id, tool, subject, state, created_at, updated_at) VALUES (?, ?, ?, ?, 'started', ?, ?)",
    )
    .bind(fingerprint)
    .bind(site)
    .bind(tool)
    .bind(subject)
    .bind(at)
    .bind(at)
    .execute(pool)
    .await?;
    if done.rows_affected() == 1 {
        return Ok(None);
    }
    get(pool, fingerprint).await
}

/// A row by fingerprint.
pub async fn get(pool: &SqlitePool, fingerprint: &str) -> Result<Option<EffectRow>, sqlx::Error> {
    sqlx::query_as("SELECT fingerprint, state, receipt FROM effects WHERE fingerprint = ?")
        .bind(fingerprint)
        .fetch_optional(pool)
        .await
}

/// Records how an effect ended.
pub async fn finish(
    pool: &SqlitePool,
    fingerprint: &str,
    outcome: Outcome,
    provider_ref: Option<&str>,
    detail: &serde_json::Value,
    receipt: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE effects SET state = ?, provider_ref = COALESCE(?, provider_ref), detail = ?, receipt = COALESCE(?, receipt), updated_at = ? WHERE fingerprint = ?",
    )
    .bind(outcome.as_str())
    .bind(provider_ref)
    .bind(detail.to_string())
    .bind(receipt)
    .bind(now())
    .bind(fingerprint)
    .execute(pool)
    .await?;
    Ok(())
}

/// Lets a fingerprint run again after it ended `not_executed` (PayPal refused before anything
/// moved, for example a network failure on the very first call that we know didn't reach it).
pub async fn release(pool: &SqlitePool, fingerprint: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM effects WHERE fingerprint = ? AND state IN ('started', 'not_executed')",
    )
    .bind(fingerprint)
    .execute(pool)
    .await?;
    Ok(())
}
