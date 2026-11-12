//! Platform core (W8): the data and plumbing the product stands on.
//!
//! - [`metrics`]: the read side behind every number the owner console, the control plane and
//!   the home page show. [`metrics::Metrics`] is the trait the consoles call;
//!   [`metrics::SqliteMetrics`] reads the first release's `control.db` and the Model Gateway's `usage`.
//! - [`catalogue`]: every number the concepts draw, its source, and the honest words to show
//!   while it has none (specs/platform/03-data.md §7).
//! - [`visits`]: visit counts without trackers (06 §4.9): the beacon endpoint, daily totals,
//!   and reports with cells under 5 shown as "fewer than 5".
//! - [`health`]: the checks behind the public status page.
//! - [`store`]: the store layer (contract C5): a store per site, per-site blobs, a hash-chained
//!   site log that issues receipts, and the outbox. Not wired into node yet.
//! - [`mount`] and [`migrate`]: how node runs this crate (contract C11): the status page,
//!   the home page's numbers, loopback JSON reads, and the background checks.
//! - [`subject`]: `control.db`'s personal data about a customer (questions passed on, orders
//!   placed by agents) as one more store in W6's "Download my data" and "Delete my account".
//! - [`budget`]: the owner's AI budget: the monthly cap in the site's currency, and this
//!   month's spend against it.
//! - [`time`] and [`words`]: windows, stored times, and numbers as people read them.
//!
//! This crate's tables (`visit_counts`, `usage_purposes`, `health_hours`, and the rebuilt
//! `usage`) live in `control.db` and come from its own migrator in `migrations/control/`, numbered in
//! W8's range (0800–0899), tracked in `_sqlx_migrations_store`.

pub mod budget;
pub mod catalogue;
pub mod health;
mod http;
pub mod metrics;
pub mod store;
pub mod subject;
pub mod time;
pub mod visits;
pub mod words;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use http::mount;

use futures::future::BoxFuture;

/// This crate's migrations, run on `control.db` after node's own.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/control");

/// Runs this crate's migrations (an `agentrix_host::MigrateFn`). Its own table,
/// `_sqlx_migrations_store`, keeps its versions apart from node's; a database carrying a
/// version this binary doesn't know still opens, so a rollback starts.
pub fn migrate(pool: &sqlx::SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut migrator = sqlx::migrate::Migrator {
            migrations: MIGRATOR.migrations.clone(),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        migrator.dangerous_set_table_name("_sqlx_migrations_store");
        migrator.set_ignore_missing(true);
        migrator.run(pool).await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn migrations_rerun_keep_their_own_table_and_tolerate_newer_versions() {
        let pool = crate::testing::control_db().await;
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations_store")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 4);
        let (node,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE name = '_sqlx_migrations'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(node, 0, "node's own table is untouched");
        crate::migrate(&pool).await.unwrap();
        // A version from a newer binary (a rollback) doesn't stop this one starting.
        sqlx::query(
            "INSERT INTO _sqlx_migrations_store (version, description, success, checksum, \
             execution_time) VALUES (899, 'from the future', 1, x'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        crate::migrate(&pool).await.unwrap();
    }
}
