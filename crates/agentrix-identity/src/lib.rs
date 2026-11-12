//! Who people are on the platform, and how they prove it (02 §8, 03-trust, ADR-018).
//! Owner: W6 (identity, payments and trust).
//!
//! Horizon 1 ships [`visitor`]: customer accounts on each site's own host. A passkey comes
//! first; without one, an email with a 6-digit code (no magic links, 06 §4.10). Accounts get
//! recovery codes, a list of devices and sessions, "Sign out everywhere", a new-device alert
//! and consent receipts for each optional use of their details.
//!
//! The first release's owner sign-in stays in `agentrix-node/src/auth/` (its approve contract is frozen);
//! this crate takes it over in Horizon 2.
//!
//! **Wiring** (the coordinator, in node):
//! ```ignore
//! agentrix_identity::migrate(&pool).await?;                 // its own migrations table
//! // W2's mount, for the sample hosts:
//! let visitors = agentrix_identity::Visitors::for_node(ctx, Arc::new(KileleShell));
//! // Every site host's router: insert `agentrix_types::tenancy::SiteHost` as an extension on
//! // each request (a layer outside this router), then merge:
//! site_router.merge(visitors.router())
//! ```
//! Ports: [`CodeMailer`] (W9 sends the emails) and [`SiteShell`] (W2's themes wrap the pages).

pub mod mailer;
pub mod owner;
pub mod people;
pub mod shell;
pub mod team;
pub mod visitor;

mod device;
mod pending;
mod throttle;

pub use mailer::{CodeMailer, LogMailer, Mail, MailKind};
pub use owner::{OwnerError, Owners};
pub use people::PeopleApp;
pub use shell::{PlainShell, SiteShell};
pub use team::{Role, Team};
pub use visitor::{Visitor, VisitorError, Visitors};

/// The name of this crate's migrations table, so its versions never mix with node's.
pub const MIGRATIONS_TABLE: &str = "_sqlx_migrations_identity";

/// Applies this crate's migrations (versions 0600–0699) to `pool`, tracked in
/// [`MIGRATIONS_TABLE`] so they never mix with node's. Idempotent; node runs it once at startup,
/// before the mounts.
pub fn migrate(pool: &sqlx::SqlitePool) -> futures::future::BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
        // An older binary (after a rollback) must still start on a database with newer versions.
        migrator.set_ignore_missing(true);
        migrator.run(pool).await?;
        Ok(())
    })
}

/// Mounts this crate on node: owner email codes, recovery codes and recovery holds
/// (`/auth/email/*`, `/auth/recovery-codes*`, `/auth/recover/*`) and team members and roles
/// (`/team/api/*`) on the platform host. Customer
/// accounts are mounted by W2 on the sample hosts ([`Visitors::for_node`]). Codes go through
/// the log-only mailer until W9's driver is wired with [`mount_with_mailer`].
pub fn mount(ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    mount_with_mailer(ctx, std::sync::Arc::new(LogMailer))
}

/// [`mount`] with W9's mailer.
pub fn mount_with_mailer(
    ctx: &agentrix_host::NodeContext,
    mailer: std::sync::Arc<dyn CodeMailer>,
) -> anyhow::Result<agentrix_host::Mounted> {
    let owners = Owners::for_node(ctx, mailer.clone());
    let team = Team::for_node(ctx, mailer);
    Ok(agentrix_host::Mounted::default()
        .platform(owners.router())
        .platform(team.router()))
}

/// Seconds since the Unix epoch.
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
