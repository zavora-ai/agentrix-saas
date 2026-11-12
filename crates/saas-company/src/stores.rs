//! Where a company keeps what must survive a restart: one SQLite database per company.
//!
//! - approvals of held calls (adk-session `SqliteApprovalStore`);
//! - the action ledger, so an effect runs once across crashes (`SqliteActionLedger`);
//! - the signed audit log (`SqliteAuditLogStore`), signed with a key kept beside the database
//!   (mode 0600, outside the repository);
//! - the agents' sessions (`SqliteSessionService`);
//! - the kernel's runs and processes, so a sleeping instalment wakes after a restart
//!   (`SqliteRunStore`, adk-kernel's `SqliteProcessStore`);
//! - the flows' checkpoints (adk-graph `SqliteCheckpointer`);
//! - the kernel's own stores (`KernelStores::from_sqlite_pool`: agenda, partitions, cron
//!   watermarks, agents, schedules, spend, the control plane), and the founder's freeze,
//!   mirrored through adk-session's `SqliteGovernanceStateStore`, so a freeze outlives a
//!   restart.
//!
//! [`CompanyStores::in_memory`] is the same set in memory, for tests and judge mode.

use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adk_core::{
    ActionLedger, ApprovalStore, Clock, DurableRunStore, InMemoryActionLedger,
    InMemoryApprovalStore, InMemoryRunStore,
};
use adk_graph::{Checkpointer, MemoryCheckpointer, SqliteCheckpointer};
use adk_kernel::{InMemoryProcessStore, ProcessStore, SqliteProcessStore};
use adk_session::{
    InMemorySessionService, SessionService, SqliteActionLedger, SqliteApprovalStore,
    SqliteAuditLogStore, SqliteRunStore, SqliteSessionService,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::audit::SignedAudit;
use crate::desk::{DebtPlan, PlanBook};

/// One company's stores.
#[derive(Clone)]
pub struct CompanyStores {
    /// Decisions on held calls.
    pub approvals: Arc<dyn ApprovalStore>,
    /// Exactly-once effects.
    pub ledger: Arc<dyn ActionLedger>,
    /// The signed audit log.
    pub audit: Arc<SignedAudit>,
    /// The agents' sessions.
    pub sessions: Arc<dyn SessionService>,
    /// The kernel's runs.
    pub runs: Arc<dyn DurableRunStore>,
    /// The kernel's processes.
    pub processes: Arc<dyn ProcessStore>,
    /// The flows' checkpoints.
    pub checkpointer: Arc<dyn Checkpointer>,
    /// The agreed debt plans.
    pub plans: Arc<dyn PlanBook>,
    /// The agents the owner revoked.
    pub revocations: Arc<dyn crate::access::RevocationBook>,
    /// The kernel's own stores (agenda, partitions, cron watermarks, agents, schedules,
    /// spend, the kill switch's state, the control plane), on the same database; `None` in
    /// memory.
    pub kernel: Option<adk_kernel::stores::KernelStores>,
    /// Whether they survive a restart.
    pub durable: bool,
}

impl std::fmt::Debug for CompanyStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanyStores")
            .field("durable", &self.durable)
            .field("audit", &self.audit.log_id())
            .finish_non_exhaustive()
    }
}

impl CompanyStores {
    /// In memory, signed with a key from `seed`: tests and judge mode.
    pub async fn in_memory(
        company: &str,
        seed: u64,
        clock: Arc<dyn Clock>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            approvals: Arc::new(InMemoryApprovalStore::new()),
            ledger: Arc::new(InMemoryActionLedger::new()),
            audit: Arc::new(SignedAudit::seeded(company, seed).await?),
            sessions: Arc::new(InMemorySessionService::new()),
            runs: Arc::new(InMemoryRunStore::new().with_clock(clock)),
            processes: Arc::new(InMemoryProcessStore::new()),
            checkpointer: Arc::new(MemoryCheckpointer::new()),
            plans: Arc::new(MemoryPlanBook::default()),
            revocations: Arc::new(crate::access::MemoryRevocations::default()),
            kernel: None,
            durable: false,
        })
    }

    /// In `<dir>/<company>.db`, with the audit key at `<dir>/keys/<company>-audit.key`.
    pub async fn sqlite(company: &str, dir: &Path, clock: Arc<dyn Clock>) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let file = dir.join(format!("{company}.db"));
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", file.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(30));
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;
        let approvals = SqliteApprovalStore::from_pool(pool.clone());
        approvals.migrate().await?;
        let ledger = SqliteActionLedger::from_pool(pool.clone());
        ledger.migrate().await?;
        let audit_store = SqliteAuditLogStore::from_pool(pool.clone());
        audit_store.migrate().await?;
        let audit = SignedAudit::with_key_file(
            Arc::new(audit_store),
            company,
            &dir.join("keys").join(format!("{company}-audit.key")),
        )
        .await?;
        let sessions = SqliteSessionService::from_pool(pool.clone());
        sessions.migrate().await?;
        let runs = SqliteRunStore::from_pool(pool.clone()).with_clock(clock);
        runs.migrate().await?;
        let processes = SqliteProcessStore::new(pool.clone());
        processes.migrate().await?;
        let checkpointer = SqliteCheckpointer::from_pool(pool.clone()).await?;
        let kernel = adk_kernel::stores::KernelStores::from_sqlite_pool(pool.clone())
            .await?
            .with_label(format!("sqlite:{company}"));
        let revocations = crate::access::SqliteRevocations::open(pool.clone()).await?;
        let plans = SqlitePlanBook::open(pool).await?;
        Ok(Self {
            approvals: Arc::new(approvals),
            ledger: Arc::new(ledger),
            audit: Arc::new(audit),
            sessions: Arc::new(sessions),
            runs: Arc::new(runs),
            processes: Arc::new(processes),
            checkpointer: Arc::new(checkpointer),
            plans: Arc::new(plans),
            revocations: Arc::new(revocations),
            kernel: Some(kernel),
            durable: true,
        })
    }
}

/// The signed audit log in `<dir>/<company>.db` alone (for an export while the node runs, or
/// after it stopped), signing with the key [`CompanyStores::sqlite`] made; it must exist.
pub async fn open_audit(company: &str, dir: &Path) -> anyhow::Result<SignedAudit> {
    let file = dir.join(format!("{company}.db"));
    let key = dir.join("keys").join(format!("{company}-audit.key"));
    anyhow::ensure!(file.is_file(), "no company database at {}", file.display());
    anyhow::ensure!(key.is_file(), "no audit key at {}", key.display());
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", file.display()))?
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(30));
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await?;
    let store = SqliteAuditLogStore::from_pool(pool);
    store.migrate().await?;
    SignedAudit::with_key_file(Arc::new(store), company, &key).await
}

/// Plans in memory.
#[derive(Debug, Default)]
pub struct MemoryPlanBook {
    plans: Mutex<std::collections::BTreeMap<String, DebtPlan>>,
}

#[async_trait::async_trait]
impl PlanBook for MemoryPlanBook {
    async fn save(&self, plan: &DebtPlan) -> anyhow::Result<()> {
        self.plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(plan.plan_id.clone(), plan.clone());
        Ok(())
    }

    async fn all(&self) -> anyhow::Result<Vec<DebtPlan>> {
        Ok(self
            .plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect())
    }
}

/// Plans in the company's SQLite database (`saas_debt_plans`).
#[derive(Debug, Clone)]
pub struct SqlitePlanBook {
    pool: sqlx::SqlitePool,
}

impl SqlitePlanBook {
    /// Over `pool`, making its table.
    pub async fn open(pool: sqlx::SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS saas_debt_plans (plan_id TEXT PRIMARY KEY, plan TEXT NOT NULL, updated_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }
}

#[async_trait::async_trait]
impl PlanBook for SqlitePlanBook {
    async fn save(&self, plan: &DebtPlan) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO saas_debt_plans (plan_id, plan, updated_at) VALUES (?, ?, ?) ON CONFLICT(plan_id) DO UPDATE SET plan = excluded.plan, updated_at = excluded.updated_at",
        )
        .bind(&plan.plan_id)
        .bind(serde_json::to_string(plan)?)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn all(&self) -> anyhow::Result<Vec<DebtPlan>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT plan FROM saas_debt_plans ORDER BY plan_id")
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|(json,)| Ok(serde_json::from_str(&json)?))
            .collect()
    }
}
