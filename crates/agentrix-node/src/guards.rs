//! What the platform crates add to the first release's governed paths at handover
//! (`specs/platform/handover-wiring.md` §2a):
//!
//! - **Dials that bite** (W1, `agentrix_kernel::Rosters::check`): a held order at the agent door,
//!   a new brief and an Ask answer each pass the dial for their WebAgent's job first.
//! - **Freeze that stops what it says** (W5, `agentrix_shell::freeze_gate`, §2b): while the
//!   owner's Freeze is in force the brief writes nothing, Ask rests, the agent door takes no
//!   calls, and the Move neither starts nor goes live. The site keeps serving its release.
//! - **Platform holds** (W3, `agentrix_operator::holds::active`, §3.2): the operator's switch on
//!   a site, checked per request. A `notice` hold serves only the neutral notice; a `read-only`
//!   hold serves the signed release with answers and forms paused.
//! - **The owner's rules** (`agentrix_kernel::taught`): the rules the owner taught a WebAgent by
//!   declining its cards, read fresh for each prompt it answers under (Ask and the agent door
//!   answer as the Concierge), quoted, with the guard that no rule widens what it may do.
//!
//! [`Guards::default`] checks nothing, so every path runs exactly as the first release built it (the
//! tests' default); `app.rs` builds [`Guards::for_node`]. A dial check that fails to read is
//! logged and the path runs as before: a held order still asks the owner, and reads stay
//! allowed. A Freeze that can't be read counts as frozen (the gate's own rule). A hold that
//! can't be read is logged and the site serves as usual: the release lookup beside it reads the
//! same database.

use agentrix_kernel::{Action, Rosters, Ruling, Verdict};
use agentrix_operator::holds::Mode;
use agentrix_shell::store::FreezeGate;
use sqlx::SqlitePool;

/// The checks a governed path makes before it runs.
#[derive(Clone, Default)]
pub struct Guards {
    rosters: Option<Rosters>,
    freeze: Option<FreezeGate>,
    holds: Option<SqlitePool>,
    rules: Option<SqlitePool>,
}

impl std::fmt::Debug for Guards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guards")
            .field("dials", &self.rosters.is_some())
            .field("freeze", &self.freeze.is_some())
            .field("holds", &self.holds.is_some())
            .field("rules", &self.rules.is_some())
            .finish()
    }
}

impl Guards {
    /// Every check, over node's database and content store.
    pub fn for_node(ctx: &agentrix_host::NodeContext) -> Self {
        Self::default()
            .with_rosters(Rosters::new(ctx.pool.clone(), ctx.blobs.clone()))
            .with_freeze(agentrix_shell::freeze_gate(ctx))
            .with_holds(ctx.pool.clone())
            .with_rules(ctx.pool.clone())
    }

    /// Reads the owner's rules for prompts (agentrix-decisions' `decision_rules`) from `pool`.
    #[must_use]
    pub fn with_rules(mut self, pool: SqlitePool) -> Self {
        self.rules = Some(pool);
        self
    }

    /// The part of `webagent`'s prompt on `site` that carries the owner's rules and standing
    /// grants ([`agentrix_kernel::taught::Taught::section`]): empty when the owner taught it
    /// nothing, or when rules aren't wired (the tests' default).
    pub async fn taught(&self, site: &str, webagent: &str) -> String {
        let Some(pool) = &self.rules else {
            return String::new();
        };
        let offset = agentrix_kernel::taught::site_offset(pool, site)
            .await
            .unwrap_or(0);
        agentrix_kernel::taught::Taught::for_prompt(
            pool,
            site,
            webagent,
            offset,
            chrono::Utc::now(),
        )
        .await
        .section()
    }

    /// Enforces the operator's holds (W3), read from `platform_holds` in `pool`.
    #[must_use]
    pub fn with_holds(mut self, pool: SqlitePool) -> Self {
        self.holds = Some(pool);
        self
    }

    /// Whether holds are enforced at all (so the edge skips the lookup when they aren't).
    pub fn enforces_holds(&self) -> bool {
        self.holds.is_some()
    }

    /// How a platform hold has the edge serve `site` now, or `None` when none is in force.
    pub async fn hold(&self, site: &str) -> Option<Mode> {
        let pool = self.holds.as_ref()?;
        match agentrix_operator::holds::active(pool, site).await {
            Ok(hold) => hold.map(|h| h.serve_mode()),
            Err(e) => {
                tracing::error!(site, error = %e, "the hold check failed; serving as usual");
                None
            }
        }
    }

    /// True while a platform hold pauses visitors' answers and forms on `site`.
    pub async fn held(&self, site: &str) -> bool {
        self.hold(site).await.is_some()
    }

    /// Checks the owner's Freeze (W5).
    #[must_use]
    pub fn with_freeze(mut self, gate: FreezeGate) -> Self {
        self.freeze = Some(gate);
        self
    }

    /// True while the owner's Freeze is in force on `site`.
    pub async fn frozen(&self, site: &str) -> bool {
        match &self.freeze {
            Some(gate) => gate.is_frozen(site).await,
            None => false,
        }
    }

    /// Checks the WebAgents' dials (W1).
    #[must_use]
    pub fn with_rosters(mut self, rosters: Rosters) -> Self {
        self.rosters = Some(rosters);
        self
    }

    /// The dial's refusal of `action` on `site`, or `None` when it may go ahead (it acts,
    /// proposes or asks the owner as before).
    pub async fn refusal(&self, site: &str, action: &Action<'_>) -> Option<Ruling> {
        let rosters = self.rosters.as_ref()?;
        match rosters.check(site, action).await {
            Ok(ruling) if ruling.verdict == Verdict::Refuse => {
                tracing::info!(
                    site,
                    webagent = action.webagent,
                    job = action.job,
                    code = ?ruling.code,
                    "the dial refused"
                );
                Some(ruling)
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(site, error = %format!("{e:#}"), "the dial check failed; going ahead as before");
                None
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// Dials over a fresh database where `site`'s team is `on_duty` (the Chief is always on).
    pub(crate) async fn team(site: &str, on_duty: &[&str]) -> Guards {
        let ctx = agentrix_host::testing::context().await.unwrap();
        agentrix_kernel::migrate(&ctx.pool).await.unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        let look = agentrix_kernel::looks::sample("kilele").unwrap();
        rosters.set_team(site, &look, on_duty, &[]).await.unwrap();
        Guards::default().with_rosters(rosters)
    }

    /// A Freeze gate over `pool`, with `site` frozen by its owner.
    pub(crate) async fn frozen(pool: &sqlx::SqlitePool, site: &str) -> Guards {
        let store = agentrix_shell::store::ShellStore::open(pool.clone())
            .await
            .unwrap();
        store
            .set_frozen(site, "acct_owner", "console", chrono::Utc::now())
            .await
            .unwrap();
        Guards::default().with_freeze(FreezeGate::new(store))
    }

    /// A Freeze gate over a fresh database, with `site` frozen.
    pub(crate) async fn frozen_alone(site: &str) -> Guards {
        let ctx = agentrix_host::testing::context().await.unwrap();
        frozen(&ctx.pool, site).await
    }

    /// Holds over a fresh control database, with `site` held in `mode` (`read-only`, `notice`).
    pub(crate) async fn held(site: &str, mode: &str) -> Guards {
        let pool = crate::db::open_in_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO platform_holds (hold_id, scope, target, mode, reason, fingerprint, placed_by, placed_method, placed_at) \
             VALUES ('hold_1', 'site', ?, ?, 'Phishing pages reported', 'sha256:00', 'op_1', 'passkey', '2026-11-08T09:00:00Z')",
        )
        .bind(site)
        .bind(mode)
        .execute(&pool)
        .await
        .unwrap();
        Guards::default().with_holds(pool)
    }
}

#[cfg(test)]
mod tests {
    use agentrix_kernel::{Effect, RuleCode};

    use super::*;

    #[tokio::test]
    async fn the_dials_refuse_only_what_they_refuse() {
        let ask = Action::new("concierge", "presentations", Effect::Read);
        let order = Action::new("agent-desk", "held-orders", Effect::Financial);
        assert!(Guards::default().refusal("s1", &ask).await.is_none());

        let on = testing::team("s1", &["concierge", "editor"]).await;
        assert!(on.refusal("s1", &ask).await.is_none());
        assert!(
            on.refusal("s1", &order).await.is_none(),
            "a held order asks the owner, as before"
        );

        let off = testing::team("s1", &["editor"]).await;
        let refused = off.refusal("s1", &ask).await.unwrap();
        assert_eq!(refused.code, RuleCode::NotOnTeam);
        assert!(off.refusal("s1", &order).await.is_some());
    }

    #[tokio::test]
    async fn a_freeze_is_read_per_site() {
        assert!(!Guards::default().frozen("s1").await);
        let g = testing::frozen_alone("s1").await;
        assert!(g.frozen("s1").await);
        assert!(!g.frozen("s2").await);
    }

    #[tokio::test]
    async fn a_hold_is_read_per_site_until_lifted() {
        assert_eq!(Guards::default().hold("s1").await, None);
        let g = testing::held("s1", "read-only").await;
        assert_eq!(g.hold("s1").await, Some(Mode::ReadOnly));
        assert!(!g.held("s2").await);
        assert_eq!(
            testing::held("s1", "notice").await.hold("s1").await,
            Some(Mode::Notice)
        );
        let pool = g.holds.clone().unwrap();
        sqlx::query(
            "UPDATE platform_holds SET lifted_at = '2026-11-08T10:00:00Z', lifted_by = 'op_1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(!g.held("s1").await);
    }
}
