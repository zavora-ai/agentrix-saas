//! Reads of what the owner taught a WebAgent, for the prompts it answers under (the kernel's
//! `taught` section and the Chief's `rules` tool).
//!
//! Only a pool is needed, so a WebAgent's prompt can be built wherever it runs. Rules are the
//! active ones (not removed), newest first. Grants are the ones in force (not stopped, not
//! ended); this crate still applies them to cards, so a prompt shows them as context only. A
//! database whose migrations haven't run has none of either.

use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::{Grant, Rule, missing_table, parse_time};

/// `webagent`'s active rules on `site` (every WebAgent's when `None`), newest first, at most
/// `limit` of them, and how many active rules there are in all.
pub async fn rules(
    pool: &SqlitePool,
    site: &str,
    webagent: Option<&str>,
    limit: usize,
) -> anyhow::Result<(Vec<Rule>, usize)> {
    type RuleSql = (
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
        String,
    );
    let total: (i64,) = match sqlx::query_as(
        "SELECT COUNT(*) FROM decision_rules WHERE site_id = ? AND removed_at IS NULL AND (? IS NULL OR webagent = ?)",
    )
    .bind(site)
    .bind(webagent)
    .bind(webagent)
    .fetch_one(pool)
    .await
    {
        Ok(n) => n,
        Err(e) if missing_table(&e) => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e.into()),
    };
    let rows: Vec<RuleSql> = sqlx::query_as(
        "SELECT id, site_id, webagent, job, kind, words, from_card, receipt, created_at FROM decision_rules WHERE site_id = ? AND removed_at IS NULL AND (? IS NULL OR webagent = ?) ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )
    .bind(site)
    .bind(webagent)
    .bind(webagent)
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(pool)
    .await?;
    let rules = rows
        .into_iter()
        .map(
            |(id, site, webagent, job, kind, words, from_card, receipt, at)| {
                Ok(Rule {
                    id,
                    site,
                    webagent,
                    job,
                    kind,
                    words,
                    from_card,
                    receipt,
                    created_at: parse_time(&at)?,
                })
            },
        )
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok((rules, usize::try_from(total.0).unwrap_or(0)))
}

/// `webagent`'s standing grants in force on `site` at `now` (every WebAgent's when `None`),
/// newest first, at most `limit` of them, and how many are in force in all.
pub async fn grants(
    pool: &SqlitePool,
    site: &str,
    webagent: Option<&str>,
    now: DateTime<Utc>,
    limit: usize,
) -> anyhow::Result<(Vec<Grant>, usize)> {
    type GrantSql = (String, String, String, String, String, String, String, i64);
    let rows: Vec<GrantSql> = match sqlx::query_as(
        "SELECT id, site_id, terms, from_card, receipt, created_at, expires_at, uses FROM decision_grants WHERE site_id = ? AND revoked_at IS NULL AND (? IS NULL OR webagent = ?) ORDER BY created_at DESC, rowid DESC",
    )
    .bind(site)
    .bind(webagent)
    .bind(webagent)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) if missing_table(&e) => return Ok((Vec::new(), 0)),
        Err(e) => return Err(e.into()),
    };
    let mut in_force = Vec::new();
    for (id, site, terms, from_card, receipt, created, expires, uses) in rows {
        let expires_at = parse_time(&expires)?;
        if expires_at <= now {
            continue;
        }
        in_force.push(Grant {
            id,
            site,
            terms: serde_json::from_str(&terms).context("a stored grant's terms do not parse")?,
            from_card,
            receipt,
            created_at: parse_time(&created)?,
            expires_at,
            uses,
        });
    }
    let total = in_force.len();
    in_force.truncate(limit);
    Ok((in_force, total))
}
