//! Migrations, the CAS and the receipts store (design §5). Lane: `data` (design §3.0).
//!
//! Other lanes read and write through this module:
//!
//! - [`open`] opens `control.db` (WAL, foreign keys on, a 5 s busy timeout), applies the
//!   control-plane's tables (`src/auth/schema.sql`, `src/claim/schema.sql`), then runs the
//!   migrations in `src/db/migrations/`. [`open_in_memory`] does the same for tests.
//! - [`usage`] reads model spend (the gateway writes it): `site_since` for the meter and the
//!   brief, `for_move` for cost per Move (S2-11), and `Spend::words` for plain words.
//!   `ReceiptStore::list_since` gives the receipts since a time, for the Chief brief.
//! - [`sites`] is the one writer of `sites`: `create_or_get` for assess/, `set_owner` and
//!   `domain_of` for claim/ (its `SiteDirectory`).
//! - [`cas::Cas`] stores blobs by digest on disk: `put`, `put_expecting`, `get`, `contains`
//!   and `path_for`. [`cas::Cas::record`] adds a row to the `artifacts` index for a site, and
//!   [`cas::Cas::get_for_site`] serves a blob only to the site that holds it;
//!   [`cas::Cas::get_public_frame`] serves frames of unclaimed sites (a free Assess).
//! - [`questions::QuestionStore`] holds the questions visitors passed to the owner from Ask
//!   (R8.3, `passed_questions`): the site's `POST /ask/pass` writes, the console's Today reads.
//! - [`receipts::ReceiptStore`] is the only writer of `receipts`. `issue` signs a
//!   `ReceiptBody` with control's key and stores it. `get` serves `GET /receipts/{digest}`,
//!   `list` feeds the Trust tab and the Chief brief, and `keys` serves
//!   `GET /.well-known/agentrix/keys`. `claim_verified` writes the `claim.verified` receipt
//!   (claim/'s `ClaimEvents`).
//!
//! Each table names its one writing lane in the migration. Times are RFC 3339 UTC text
//! ([`now`], [`time`]), and digests are `sha256:<hex>`.

pub mod cas;
pub mod questions;
pub mod receipts;
pub mod sites;
pub mod usage;

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

/// The migrations for `control.db`, embedded at build time.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("src/db/migrations");

/// The sign-in tables, owned by the control-plane lane (`auth/`). Idempotent, so it runs on
/// every open, before [`MIGRATOR`], whose tables reference `accounts`.
pub const AUTH_SCHEMA: &str = include_str!("../auth/schema.sql");

/// The `claims` table, owned by the control-plane lane (`claim/`). Idempotent.
pub const CLAIM_SCHEMA: &str = include_str!("../claim/schema.sql");

async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(AUTH_SCHEMA).execute(pool).await?;
    sqlx::raw_sql(CLAIM_SCHEMA).execute(pool).await?;
    // A database may carry versions this binary doesn't know: after `deploy.sh --rollback`, or
    // when a newer slice already migrated it. Run the known ones and leave the rest alone;
    // checksums of known versions are still checked (specs/platform/how-we-work.md).
    let mut migrator = sqlx::migrate!("src/db/migrations");
    migrator.set_ignore_missing(true);
    migrator.run(pool).await?;
    Ok(())
}

/// Opens (creating if needed) `control.db` at `path` and runs any pending migrations.
pub async fn open(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// A migrated in-memory database on one connection, for tests.
pub async fn open_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// A time as stored: RFC 3339, UTC, always with microseconds, so text order is time order.
pub fn time(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// The current time as stored.
pub fn now() -> String {
    time(Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_database_from_a_newer_binary_still_opens() {
        let pool = open_in_memory().await.unwrap();
        // A version this binary has never seen, as a newer slice or a rolled-back deploy leaves.
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES (9999, 'from a newer binary', 1, x'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        migrate(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn migrations_run_on_a_fresh_database() {
        let pool = open_in_memory().await.unwrap();
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\' \
             AND name NOT LIKE 'sqlite%' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<&str> = tables.iter().map(|(n,)| n.as_str()).collect();
        for expected in [
            "accounts",
            "totp_seeds",
            "artifacts",
            "assessments",
            "briefs",
            "budgets",
            "charters",
            "claims",
            "credentials",
            "decisions",
            "findings",
            "gauntlet_runs",
            "jobs",
            "move_steps",
            "moves",
            "passed_questions",
            "quarantine",
            "receipts",
            "releases",
            "sessions",
            "site_pointer",
            "sites",
            "ui_events",
            "usage",
        ] {
            assert!(
                names.contains(&expected),
                "missing table {expected}: {names:?}"
            );
        }
    }

    #[tokio::test]
    async fn migrations_run_on_a_fresh_file_and_again() {
        let dir = std::env::temp_dir().join(format!("agentrix-db-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("control.db");
        open(&path).await.unwrap().close().await;
        open(&path).await.unwrap().close().await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn site_rows_reference_control_plane_accounts() {
        let pool = open_in_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, created_at) \
             VALUES ('acct_1', 'o@example.com', 'Owner', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let insert = "INSERT INTO sites (site_id, domain, old_origin, owner_account_id, created_at) \
                      VALUES (?, ?, 'https://bakery.example', ?, ?)";
        sqlx::query(insert)
            .bind("site_1")
            .bind("bakery.example")
            .bind("acct_1")
            .bind(now())
            .execute(&pool)
            .await
            .unwrap();
        let orphan = sqlx::query(insert)
            .bind("site_2")
            .bind("cafe.example")
            .bind("acct_missing")
            .bind(now())
            .execute(&pool)
            .await;
        assert!(orphan.is_err(), "foreign keys are on");
    }
}
