//! Test helpers: the first release's `control.db` schema plus 0300, built from node's own files so tests
//! run against exactly what production migrates.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use uuid::Uuid;

use agentrix_host::Approvals;

fn node_db() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agentrix-node/src")
}

async fn migrate(pool: &SqlitePool) {
    let src = node_db();
    for file in ["auth/schema.sql", "claim/schema.sql"] {
        let sql = std::fs::read_to_string(src.join(file)).unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .unwrap();
    }
    let mut migrations: Vec<PathBuf> = std::fs::read_dir(src.join("db/migrations"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    migrations.sort();
    for path in migrations {
        let sql = std::fs::read_to_string(&path).unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

/// A migrated in-memory database on one connection.
pub async fn pool() -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    migrate(&pool).await;
    pool
}

/// A migrated database in a temporary file with several connections, for concurrency tests.
pub async fn file_pool() -> (SqlitePool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("control.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    migrate(&pool).await;
    (pool, dir)
}

/// A person with an account (the first release's `accounts`).
pub async fn account(pool: &SqlitePool, email: &str, name: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, ?, ?, 0)")
        .bind(id.to_string())
        .bind(email)
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    id
}

/// Grants roles directly.
pub async fn grant(pool: &SqlitePool, account: Uuid, roles: &[&str]) {
    for role in roles {
        sqlx::query("INSERT INTO operator_roles (account_id, role, granted_by, granted_at) VALUES (?, ?, 'test', ?)")
            .bind(account.to_string())
            .bind(role)
            .bind(crate::ledger::now())
            .execute(pool)
            .await
            .unwrap();
    }
}

/// Approvals that accept the code `246810` and passkey proofs naming the fingerprint, and
/// remember what they were asked to approve.
#[derive(Default)]
pub struct FakeApprovals {
    /// Accounts that "have" a passkey.
    pub passkeys: Mutex<Vec<Uuid>>,
    /// Every fingerprint approved.
    pub signed: Mutex<Vec<(Uuid, [u8; 32])>>,
}

#[async_trait]
impl Approvals for FakeApprovals {
    async fn passkey_options(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<Value>> {
        Ok(self
            .passkeys
            .lock()
            .unwrap()
            .contains(&account)
            .then(|| json!({ "publicKey": { "challenge": hex::encode(fingerprint) } })))
    }

    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        request: Value,
    ) -> anyhow::Result<Value> {
        let (method, ok) = if let Some(code) = request.get("totp") {
            ("totp", code == "246810")
        } else if let Some(cred) = request.get("passkey") {
            ("passkey", cred["challenge"] == hex::encode(fingerprint))
        } else {
            ("none", false)
        };
        if !ok {
            anyhow::bail!(
                "That code didn't work. Codes change every 30 seconds, so use the newest one."
            );
        }
        self.signed.lock().unwrap().push((account, fingerprint));
        Ok(json!({ "method": method, "account": account, "fingerprint": hex::encode(fingerprint) }))
    }
}

/// Fake approvals behind an Arc, with a handle kept for assertions.
pub fn approvals() -> (Arc<FakeApprovals>, Arc<dyn Approvals>) {
    let fake = Arc::new(FakeApprovals::default());
    (fake.clone(), fake)
}

/// The first release records for one live site with a finished Move, spend, a gauntlet run, a waiting card
/// and some sealed-room jobs, as the gateway, the Move and the gauntlet write them.
pub async fn seed_live_site(pool: &SqlitePool, owner: Uuid, domain: &str) -> String {
    let site = format!("site_{}", Uuid::now_v7().simple());
    let now = chrono::Utc::now();
    let t = |mins: i64| crate::ledger::time(now - chrono::Duration::minutes(mins));
    let mv = format!("move_{}", Uuid::now_v7().simple());
    let digest = format!("sha256:{}", "ab".repeat(32));
    let q = |sql: &'static str| sqlx::query(sql);
    q("INSERT INTO sites (site_id, domain, name, old_origin, owner_account_id, created_at) VALUES (?, ?, 'Test Bakery', ?, ?, ?)")
        .bind(&site).bind(domain).bind(format!("https://{domain}")).bind(owner.to_string()).bind(t(120))
        .execute(pool).await.unwrap();
    q("INSERT INTO assessments (assessment_id, site_id, url, state, created_at) VALUES (?, ?, ?, 'report-ready', ?)")
        .bind(format!("asmt_{}", Uuid::now_v7().simple())).bind(&site).bind(format!("https://{domain}")).bind(t(119))
        .execute(pool).await.unwrap();
    q("INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES (?, ?, 'live', ?, ?)")
        .bind(&mv).bind(&site).bind(t(100)).bind(t(80)).execute(pool).await.unwrap();
    for (seq, step, start) in [
        (1, "replay", 100),
        (2, "clean", 97),
        (3, "rebuild", 95),
        (4, "prove", 93),
        (5, "golive", 92),
    ] {
        q("INSERT INTO move_steps (move_id, seq, site_id, step, state, started_at, ended_at, receipt) VALUES (?, ?, ?, ?, 'done', ?, ?, ?)")
            .bind(&mv).bind(seq).bind(&site).bind(step).bind(t(start)).bind(t(start - 1)).bind(&digest)
            .execute(pool).await.unwrap();
    }
    q("INSERT INTO releases (site_id, seq, manifest_digest, checkpoint, state, created_at, signed_at) VALUES (?, 1, ?, '{}', 'live', ?, ?)")
        .bind(&site).bind(&digest).bind(t(85)).bind(t(84)).execute(pool).await.unwrap();
    q("INSERT INTO site_pointer (site_id, seq, manifest_digest, updated_at) VALUES (?, 1, ?, ?)")
        .bind(&site)
        .bind(&digest)
        .bind(t(80))
        .execute(pool)
        .await
        .unwrap();
    for (purpose, provider, cost, outcome) in [
        ("build", "vultr", 120_000, "ok"),
        ("concierge", "vultr", 30_000, "ok"),
        ("vision-parity", "anthropic", 250_000, "ok"),
        ("concierge", "vultr", 0, "error"),
    ] {
        q("INSERT INTO usage (site_id, purpose, provider, model, move_id, tokens_in, tokens_out, cost_micro_usd, outcome, at) VALUES (?, ?, ?, 'm', ?, 1000, 200, ?, ?, ?)")
            .bind(&site).bind(purpose).bind(provider).bind(&mv).bind(cost).bind(outcome).bind(t(90))
            .execute(pool).await.unwrap();
    }
    q("INSERT INTO budgets (site_id, purpose, period, limit_micro_usd) VALUES (?, '*', 'month', 5000000)")
        .bind(&site).execute(pool).await.unwrap();
    q("INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, receipt, started_at, ended_at) VALUES (?, ?, 10, 10, ?, ?, ?)")
        .bind(format!("run_{}", Uuid::now_v7().simple())).bind(&site).bind(&digest).bind(t(30)).bind(t(29))
        .execute(pool).await.unwrap();
    q("INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, created_at) VALUES (?, ?, 'order-held', '{\"secret\":\"Amina ordered a cake\"}', 'f', 'waiting', ?)")
        .bind(format!("dec_{}", Uuid::now_v7().simple())).bind(&site).bind(t(10)).execute(pool).await.unwrap();
    for (kind, state, made) in [
        ("fetch", "done", 118),
        ("replay", "done", 99),
        ("build", "failed", 96),
    ] {
        q("INSERT INTO jobs (job_id, site_id, move_id, kind, spec, state, created_at, started_at, ended_at) VALUES (?, ?, ?, ?, '{}', ?, ?, ?, ?)")
            .bind(format!("job_{}", Uuid::now_v7().simple())).bind(&site).bind(&mv).bind(kind).bind(state).bind(t(made)).bind(t(made - 1)).bind(t(made - 2))
            .execute(pool).await.unwrap();
    }
    q("INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, started_at, ended_at, issued_at, body) VALUES (?, ?, ?, 'release.signed', 'release', '1', 'k', ?, ?, ?, '{\"detail\":{\"title\":\"private words\"}}')")
        .bind(format!("sha256:{}", Uuid::now_v7().simple())).bind(format!("rcpt_{}", Uuid::now_v7().simple())).bind(&site).bind(t(84)).bind(t(84)).bind(t(84))
        .execute(pool).await.unwrap();
    site
}
