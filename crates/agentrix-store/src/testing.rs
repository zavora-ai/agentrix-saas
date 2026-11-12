//! A migrated in-memory `control.db` for tests, built from the real schema: node's auth and
//! claim schemas and every migration in `crates/agentrix-node/src/db/migrations/` (read when
//! the test runs, so tests see the schema that ships), then this crate's own migrator.
//! Enable with the `testing` feature.

use std::path::PathBuf;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{AssertSqlSafe, SqlitePool};

/// `crates/agentrix-node/src/`.
fn node_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agentrix-node/src")
}

/// Node's schema files in apply order: auth, claim, then its migrations by version.
fn node_files() -> Vec<PathBuf> {
    let src = node_src();
    let mut files = vec![src.join("auth/schema.sql"), src.join("claim/schema.sql")];
    let mut migrations: Vec<PathBuf> = std::fs::read_dir(src.join("db/migrations"))
        .expect("node's migrations folder")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    migrations.sort();
    files.extend(migrations);
    files
}

async fn run_file(pool: &SqlitePool, file: &PathBuf) {
    let sql =
        std::fs::read_to_string(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("applying {}: {e}", file.display()));
}

/// The first release's `control.db` (node's schema only), in memory on one connection.
///
/// # Panics
///
/// If a schema file is missing or fails to apply.
pub async fn first_release_db() -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("in-memory sqlite");
    for file in node_files() {
        run_file(&pool, &file).await;
    }
    pool
}

/// `control.db` as node opens it in production: node's schema, then [`crate::migrate`].
///
/// # Panics
///
/// If a schema file or a migration fails.
pub async fn control_db() -> SqlitePool {
    let pool = first_release_db().await;
    crate::migrate(&pool)
        .await
        .expect("agentrix-store migrations");
    pool
}

/// Applies one of this crate's migration files by name (`0801_usage_open.sql`) without the
/// migrator, to test a migration on its own.
///
/// # Panics
///
/// If the file is missing or fails to apply.
pub async fn apply(pool: &SqlitePool, name: &str) {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("migrations/control")
        .join(name);
    run_file(pool, &file).await;
}
