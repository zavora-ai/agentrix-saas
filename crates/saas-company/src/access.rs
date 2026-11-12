//! Revoke beside Freeze: the owner switches one agent off, and its tokens stop at the next
//! check through the company's token service ([`crate::sts::TokenService`]). No PayPal key is
//! rotated; the agent never held one. Switching it back on takes the owner's passkey.
//!
//! - [`AgentAccess::revoke`] marks the agent revoked in the token service (its live tokens are
//!   dropped, a new exchange is refused, and the credential path checks every token before use),
//!   keeps it in the company's database so it survives a restart, and writes a signed
//!   `agent.revoke` record.
//! - [`AgentAccess::restore`] needs the passkey proof the console verified (`userVerified`), and
//!   writes a signed `agent.restore` record carrying the proof's digest.

use std::sync::{Arc, Mutex};

use adk_core::SubjectRef;
use adk_core::audit_log::{
    AuditActor, AuditDecision, AuditRecordDraft, AuditRecordKind, AuditRecordOutcome,
};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::audit::SignedAudit;
use crate::identity::{ROLES, StaffIdentity};
use crate::sts::TokenService;

/// Where revocations are kept.
#[async_trait]
pub trait RevocationBook: Send + Sync {
    /// Records `agent` as revoked, with why.
    async fn save(&self, agent: &str, why: &str) -> anyhow::Result<()>;
    /// Removes `agent`.
    async fn remove(&self, agent: &str) -> anyhow::Result<()>;
    /// Every revoked agent.
    async fn all(&self) -> anyhow::Result<Vec<String>>;
}

/// Revocations in memory.
#[derive(Debug, Default)]
pub struct MemoryRevocations(Mutex<std::collections::BTreeSet<String>>);

#[async_trait]
impl RevocationBook for MemoryRevocations {
    async fn save(&self, agent: &str, _why: &str) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(agent.to_owned());
        Ok(())
    }

    async fn remove(&self, agent: &str) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(agent);
        Ok(())
    }

    async fn all(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect())
    }
}

/// Revocations in the company's SQLite database (`saas_revoked_agents`).
#[derive(Debug, Clone)]
pub struct SqliteRevocations {
    pool: sqlx::SqlitePool,
}

impl SqliteRevocations {
    /// Over `pool`, making its table.
    pub async fn open(pool: sqlx::SqlitePool) -> anyhow::Result<Self> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS saas_revoked_agents (agent_id TEXT PRIMARY KEY, reason TEXT NOT NULL, revoked_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl RevocationBook for SqliteRevocations {
    async fn save(&self, agent: &str, why: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO saas_revoked_agents (agent_id, reason, revoked_at) VALUES (?, ?, ?) ON CONFLICT(agent_id) DO UPDATE SET reason = excluded.reason, revoked_at = excluded.revoked_at",
        )
        .bind(agent)
        .bind(why)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn remove(&self, agent: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM saas_revoked_agents WHERE agent_id = ?")
            .bind(agent)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn all(&self) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT agent_id FROM saas_revoked_agents ORDER BY agent_id")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(a,)| a).collect())
    }
}

/// One company's switch per agent.
pub struct AgentAccess {
    company: String,
    founder: String,
    sts: Arc<TokenService>,
    book: Arc<dyn RevocationBook>,
    audit: Arc<SignedAudit>,
}

impl std::fmt::Debug for AgentAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentAccess")
            .field("company", &self.company)
            .field("revoked", &self.sts.revoked())
            .finish_non_exhaustive()
    }
}

impl AgentAccess {
    /// Over the company's token service, putting every revocation in `book` back in force.
    pub async fn open(
        company: &str,
        founder: &str,
        sts: Arc<TokenService>,
        book: Arc<dyn RevocationBook>,
        audit: Arc<SignedAudit>,
    ) -> anyhow::Result<Self> {
        for agent in book.all().await? {
            sts.revoke(&agent);
        }
        Ok(Self {
            company: company.to_owned(),
            founder: founder.to_owned(),
            sts,
            book,
            audit,
        })
    }

    /// The agent id of `role` (`urn:adk:agent:<company>/<role>`).
    pub fn agent_id(&self, role: &str) -> anyhow::Result<String> {
        anyhow::ensure!(
            ROLES.contains(&role),
            "{role} isn't one of {}'s agents",
            self.company
        );
        Ok(StaffIdentity::new(&self.company, role, &self.founder)?
            .id()
            .as_str()
            .to_owned())
    }

    fn founder(&self) -> AuditActor {
        AuditActor::subject(SubjectRef::human(&self.founder))
    }

    /// Switches `role` off: its tokens stop at their next check. Returns what was done.
    pub async fn revoke(&self, role: &str, why: &str) -> anyhow::Result<Value> {
        let agent = self.agent_id(role)?;
        if self.sts.is_revoked(&agent) {
            // Already off: the same answer, nothing written again.
            return Ok(json!({ "agent": agent, "revoked": true, "already": true }));
        }
        let dropped = self.sts.revoke(&agent);
        self.book.save(&agent, why).await?;
        let record = self
            .audit
            .record(
                AuditRecordDraft::new(AuditRecordKind::Governance, "agent.revoke")
                    .actor(self.founder())
                    .decision(AuditDecision::Denied)
                    .outcome(AuditRecordOutcome::Succeeded)
                    .ext("agent", json!(agent))
                    .ext("why", json!(why))
                    .ext("tokens_dropped", json!(dropped))
                    .ext("paypal_keys_rotated", json!(false)),
            )
            .await?;
        tracing::info!(agent = %agent, "the owner revoked an agent");
        Ok(
            json!({ "agent": agent, "revoked": true, "tokens_dropped": dropped, "audit_seq": record.seq }),
        )
    }

    /// Switches `role` back on, with `proof`: the passkey assertion the console verified (it
    /// must say `userVerified`).
    pub async fn restore(&self, role: &str, proof: &Value) -> anyhow::Result<Value> {
        let agent = self.agent_id(role)?;
        anyhow::ensure!(
            proof["userVerified"] == json!(true),
            "switching an agent back on takes the owner's passkey"
        );
        self.sts.restore(&agent);
        self.book.remove(&agent).await?;
        let record = self
            .audit
            .record(
                AuditRecordDraft::new(AuditRecordKind::Governance, "agent.restore")
                    .actor(self.founder())
                    .decision(AuditDecision::Approved)
                    .outcome(AuditRecordOutcome::Succeeded)
                    .args_digest(self.audit.digest(proof))
                    .ext("agent", json!(agent))
                    .ext("how", json!("passkey")),
            )
            .await?;
        tracing::info!(agent = %agent, "the owner restored an agent with a passkey");
        Ok(json!({ "agent": agent, "revoked": false, "audit_seq": record.seq }))
    }

    /// Whether `role` is switched off.
    pub fn is_revoked(&self, role: &str) -> bool {
        self.agent_id(role)
            .map(|a| self.sts.is_revoked(&a))
            .unwrap_or(false)
    }

    /// Every agent switched off.
    pub fn revoked(&self) -> Vec<String> {
        self.sts.revoked()
    }
}
