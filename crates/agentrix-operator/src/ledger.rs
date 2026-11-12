//! The operator ledger (08 §4.6): every operator action, by a person or an operator WebAgent,
//! as an append-only, hash-chained row.
//!
//! Each entry's hash is SHA-256 over a JSON array of its fields in a fixed order, including the
//! previous entry's hash, so changing or removing any row breaks every hash after it. [`verify`]
//! walks the chain. The entry id (`op-9f1a2c`) is the start of the hash.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

/// The hash before the first entry.
pub const GENESIS: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// A time as stored: RFC 3339, UTC, with microseconds (the same as the first release's `db::time`).
pub fn time(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// The current time as stored.
pub fn now() -> String {
    time(Utc::now())
}

/// A new entry, before it is chained.
#[derive(Debug, Clone)]
pub struct Entry {
    actor: String,
    actor_name: String,
    verb: String,
    target: String,
    summary: String,
    approval: Option<Value>,
    detail: Option<Value>,
}

impl Entry {
    /// An action by a person.
    pub fn person(account: Uuid, name: &str, verb: &str, target: &str, summary: &str) -> Self {
        Self::new(format!("account:{account}"), name, verb, target, summary)
    }

    /// An action by the platform itself (startup, the roles file, a timer).
    pub fn platform(verb: &str, target: &str, summary: &str) -> Self {
        Self::new("platform".into(), "Platform", verb, target, summary)
    }

    fn new(actor: String, actor_name: &str, verb: &str, target: &str, summary: &str) -> Self {
        Self {
            actor,
            actor_name: actor_name.to_owned(),
            verb: verb.to_owned(),
            target: target.to_owned(),
            summary: summary.to_owned(),
            approval: None,
            detail: None,
        }
    }

    /// The approvals behind it: `[{account, method}, …]`.
    pub fn approval(mut self, approval: Value) -> Self {
        self.approval = Some(approval);
        self
    }

    /// Typed detail, content-free.
    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }
}

/// A stored entry.
#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    /// Position in the chain, from 1.
    pub seq: i64,
    /// `op-` and the start of the hash.
    pub entry_id: String,
    /// When.
    pub at: String,
    /// `account:<id>`, `agent:op:zavora/<role>` or `platform`.
    pub actor: String,
    /// The actor's name.
    pub actor_name: String,
    /// The verb class, e.g. `operator.signin`, `role.grant`, `grant.request`.
    pub verb: String,
    /// The tenant-path target, or `fleet`.
    pub target: String,
    /// Plain words.
    pub summary: String,
    /// The approvals, JSON text.
    pub approval: Option<String>,
    /// Detail, JSON text.
    pub detail: Option<String>,
    /// The previous entry's hash.
    pub prev_hash: String,
    /// This entry's hash.
    pub hash: String,
}

#[allow(clippy::too_many_arguments)]
fn hash_of(
    seq: i64,
    at: &str,
    actor: &str,
    actor_name: &str,
    verb: &str,
    target: &str,
    summary: &str,
    approval: Option<&str>,
    detail: Option<&str>,
    prev: &str,
) -> String {
    let canonical = json!([
        seq, at, actor, actor_name, verb, target, summary, approval, detail, prev
    ])
    .to_string();
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    )
}

/// Appends `entry` to the chain and returns it as stored. Concurrent appends retry on the
/// sequence's uniqueness, so the chain never forks.
pub async fn append(pool: &SqlitePool, entry: Entry) -> Result<Stored, sqlx::Error> {
    let approval = entry.approval.as_ref().map(Value::to_string);
    let detail = entry.detail.as_ref().map(Value::to_string);
    let mut last_error = None;
    for _ in 0..8 {
        let last: Option<(i64, String)> =
            sqlx::query_as("SELECT seq, hash FROM operator_ledger ORDER BY seq DESC LIMIT 1")
                .fetch_optional(pool)
                .await?;
        let (seq, prev) = match last {
            Some((seq, hash)) => (seq + 1, hash),
            None => (1, GENESIS.to_owned()),
        };
        let at = now();
        let hash = hash_of(
            seq,
            &at,
            &entry.actor,
            &entry.actor_name,
            &entry.verb,
            &entry.target,
            &entry.summary,
            approval.as_deref(),
            detail.as_deref(),
            &prev,
        );
        let entry_id = format!("op-{}", &hash[7..13]);
        let inserted = sqlx::query(
            "INSERT INTO operator_ledger (seq, entry_id, at, actor, actor_name, verb, target, summary, approval, detail, prev_hash, hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq)
        .bind(&entry_id)
        .bind(&at)
        .bind(&entry.actor)
        .bind(&entry.actor_name)
        .bind(&entry.verb)
        .bind(&entry.target)
        .bind(&entry.summary)
        .bind(&approval)
        .bind(&detail)
        .bind(&prev)
        .bind(&hash)
        .execute(pool)
        .await;
        match inserted {
            Ok(_) => {
                return Ok(Stored {
                    seq,
                    entry_id,
                    at,
                    actor: entry.actor,
                    actor_name: entry.actor_name,
                    verb: entry.verb,
                    target: entry.target,
                    summary: entry.summary,
                    approval,
                    detail,
                    prev_hash: prev,
                    hash,
                });
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                last_error = Some(sqlx::Error::Database(e))
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_error.unwrap_or(sqlx::Error::RowNotFound))
}

type Row = (
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
);

fn stored(r: Row) -> Stored {
    let (
        seq,
        entry_id,
        at,
        actor,
        actor_name,
        verb,
        target,
        summary,
        approval,
        detail,
        prev_hash,
        hash,
    ) = r;
    Stored {
        seq,
        entry_id,
        at,
        actor,
        actor_name,
        verb,
        target,
        summary,
        approval,
        detail,
        prev_hash,
        hash,
    }
}

macro_rules! select {
    ($rest:literal) => {
        concat!("SELECT seq, entry_id, at, actor, actor_name, verb, target, summary, approval, detail, prev_hash, hash FROM operator_ledger ", $rest)
    };
}

/// The newest `limit` entries, newest first.
pub async fn recent(pool: &SqlitePool, limit: i64) -> Result<Vec<Stored>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(select!("ORDER BY seq DESC LIMIT ?"))
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(stored).collect())
}

/// The newest `limit` entries about `target` (a tenant path such as `site:<id>`), newest first.
pub async fn for_target(
    pool: &SqlitePool,
    target: &str,
    limit: i64,
) -> Result<Vec<Stored>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(select!("WHERE target = ? ORDER BY seq DESC LIMIT ?"))
        .bind(target)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(stored).collect())
}

/// One entry by id.
pub async fn get(pool: &SqlitePool, entry_id: &str) -> Result<Option<Stored>, sqlx::Error> {
    let row: Option<Row> = sqlx::query_as(select!("WHERE entry_id = ?"))
        .bind(entry_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(stored))
}

/// The chain check: how many entries, and the first one that doesn't hold, if any.
#[derive(Debug, Clone, Serialize)]
pub struct Verified {
    /// Every hash and link holds.
    pub ok: bool,
    /// Entries checked.
    pub entries: i64,
    /// The newest hash.
    pub head: String,
    /// The first broken entry: its seq and why.
    pub broken: Option<(i64, String)>,
}

/// Recomputes every hash and link, oldest first.
pub async fn verify(pool: &SqlitePool) -> Result<Verified, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(select!("ORDER BY seq ASC"))
        .fetch_all(pool)
        .await?;
    let mut prev = GENESIS.to_owned();
    let entries = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    for (expected_seq, r) in (1_i64..).zip(rows) {
        let e = stored(r);
        let why = if e.seq != expected_seq {
            Some(format!("entry {expected_seq} is missing"))
        } else if e.prev_hash != prev {
            Some("its link to the previous entry doesn't match".to_owned())
        } else if hash_of(
            e.seq,
            &e.at,
            &e.actor,
            &e.actor_name,
            &e.verb,
            &e.target,
            &e.summary,
            e.approval.as_deref(),
            e.detail.as_deref(),
            &e.prev_hash,
        ) != e.hash
        {
            Some("its contents changed after it was written".to_owned())
        } else {
            None
        };
        if let Some(why) = why {
            return Ok(Verified {
                ok: false,
                entries,
                head: e.hash,
                broken: Some((e.seq, why)),
            });
        }
        prev = e.hash;
    }
    Ok(Verified {
        ok: true,
        entries,
        head: prev,
        broken: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn the_chain_holds_and_catches_an_edit() {
        let pool = testing::pool().await;
        for i in 0..5 {
            append(
                &pool,
                Entry::platform("test.entry", "fleet", &format!("Entry {i}")),
            )
            .await
            .unwrap();
        }
        let v = verify(&pool).await.unwrap();
        assert!(v.ok && v.entries == 5, "{v:?}");
        let newest = recent(&pool, 1).await.unwrap().remove(0);
        assert_eq!(newest.seq, 5);
        assert!(newest.entry_id.starts_with("op-") && newest.entry_id.len() == 9);
        assert!(get(&pool, &newest.entry_id).await.unwrap().is_some());

        sqlx::query("UPDATE operator_ledger SET summary = 'Changed' WHERE seq = 3")
            .execute(&pool)
            .await
            .unwrap();
        let v = verify(&pool).await.unwrap();
        assert!(!v.ok);
        assert_eq!(v.broken.unwrap().0, 3);
    }

    #[tokio::test]
    async fn concurrent_appends_do_not_fork() {
        let pool = testing::file_pool().await;
        let mut tasks = Vec::new();
        for i in 0..8 {
            let pool = pool.0.clone();
            tasks.push(tokio::spawn(async move {
                append(
                    &pool,
                    Entry::platform("test.entry", "fleet", &format!("Entry {i}")),
                )
                .await
                .unwrap()
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        let v = verify(&pool.0).await.unwrap();
        assert!(v.ok && v.entries == 8, "{v:?}");
    }
}
