//! The control plane's operator scope (docs/08-control-plane.md §7.1), served on its own host
//! (`ops.agentrix.website`), so a bug in a site's scope can never ride an operator's session
//! (specs/platform/02-architecture.md §4).
//!
//! What it serves, all server-rendered from the concepts in `concept/control-plane/`:
//!
//! - **Sign-in** for Zavora's people: passkey first, an authenticator code without one. The proof
//!   comes from the first release's approvals API through `agentrix_host::Approvals`, bound to a sign-in
//!   fingerprint. Operators get their own session table and cookie (`__Host-agx_ops`), so an
//!   owner's session on the platform host is never accepted here.
//! - **Roles** (08 §2.3): `op.admin`, `op.sre`, `op.release`, `op.finance`, `op.support`,
//!   `op.partners`, `op.security`, `op.auditor` and `director`, from the roles file
//!   ([`OperatorState::apply_roles_file`]) or an `op.admin`. Every grant is a ledger entry.
//! - **Fleet, Customers, a site, Trust**: read-only views over the first release's `control.db` (sites,
//!   Moves, releases, receipts, decisions, usage, budgets, gauntlet runs, sealed-room jobs),
//!   content-free by construction: ids, kinds, counts, durations and money, never site content.
//! - **The operator ledger** (08 §4.6): append-only and hash-chained ([`ledger`]).
//!
//! Wiring (the coordinator, in `agentrix-node`): one line in `app.rs`,
//!
//! ```ignore
//! let ops = NodeContext { approvals: ops_approvals, ..ctx.clone() };   // see below
//! mounted.push(agentrix_operator::mount(&ops)?);
//! ```
//!
//! where `ops_approvals` is the first release's `Approvals` on a second `AuthState` whose origin is the ops
//! host (`https://ops.agentrix.website`) and whose RP id stays the platform's
//! (`agentrix.website`), so the passkeys people already have work on the ops host and a
//! ceremony from any other host fails WebAuthn's origin check. [`mount`] serves
//! `HostMatch::Exact(ops host)` and spawns the roles-file task.
//!
//! The tables are in `agentrix-node/src/db/migrations/0300_operator.sql` (W3's range).

mod access;
mod assets;
pub mod holds;
mod identity;
mod incidents;
mod lane;
pub mod ledger;
mod notice;
mod overview;
mod owner;
mod pages;
mod partners;
pub mod reads;
mod render;

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_host::{Approvals, HostMatch, Mounted, NodeContext};
use axum::Router;
use sqlx::SqlitePool;
use url::Url;

pub use identity::{Operator, Role};
pub use lane::support_lane;

/// Settings for the operator console.
#[derive(Debug, Clone)]
pub struct OperatorConfig {
    /// The ops host's origin as browsers see it, e.g. `https://ops.agentrix.website`.
    pub origin: Url,
    /// `Secure` and `__Host-` cookies. True behind TLS; false only for local development.
    pub secure_cookies: bool,
    /// The cell this node is, for the Fleet page (08 §3.2). The first release runs one: `atl-0`.
    pub cell: Cell,
    /// The platform's own origin (`https://agentrix.website`), for links to receipts.
    pub platform_origin: Option<Url>,
    /// The time zone operators read times in, as minutes east of UTC (EAT is +180).
    pub utc_offset_minutes: i32,
    /// The zone's short name shown beside times ("EAT").
    pub zone_name: String,
    /// Whether the edge enforces platform holds on this build (`AGENTRIX_HOLDS_ENFORCED=1`, set
    /// by whoever wires `holds::active` into site serving). Until then holds are recorded and
    /// receipted, and the console says they aren't enforced.
    pub holds_enforced: bool,
}

/// One cell: a region's whole stack (08 §3.2). Single-node deployments are one cell.
#[derive(Debug, Clone)]
pub struct Cell {
    /// Short id, e.g. `atl-0`.
    pub id: String,
    /// Where it runs, in words, e.g. `Atlanta`.
    pub place: String,
    /// The profile, e.g. `single node`.
    pub profile: String,
    /// The Vultr products it uses, shown as chips.
    pub products: Vec<String>,
}

impl Cell {
    /// The cell from `AGENTRIX_CELL` (`id|place|profile|product,product`), or this machine.
    pub fn from_env() -> Self {
        std::env::var("AGENTRIX_CELL")
            .ok()
            .and_then(|v| Self::parse(&v))
            .unwrap_or_else(|| Self {
                id: "local".into(),
                place: "This machine".into(),
                profile: "single node".into(),
                products: Vec::new(),
            })
    }

    fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split('|').map(str::trim);
        let id = parts.next().filter(|s| !s.is_empty())?.to_owned();
        let place = parts.next().unwrap_or("").to_owned();
        let profile = parts.next().unwrap_or("single node").to_owned();
        let products = parts
            .next()
            .map(|p| {
                p.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            id,
            place,
            profile,
            products,
        })
    }
}

impl OperatorConfig {
    /// Settings for an origin: secure cookies when it is `https`, the cell from the
    /// environment, times in EAT.
    pub fn new(origin: Url) -> Self {
        let secure_cookies = origin.scheme() == "https";
        Self {
            origin,
            secure_cookies,
            cell: Cell::from_env(),
            platform_origin: None,
            utc_offset_minutes: 180,
            zone_name: "EAT".into(),
            holds_enforced: std::env::var("AGENTRIX_HOLDS_ENFORCED").is_ok_and(|v| v == "1"),
        }
    }
}

/// Why the operator console could not start.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A template failed to load.
    #[error("loading the operator templates: {0}")]
    Templates(#[from] minijinja::Error),
    /// The database refused a query.
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    /// The roles file could not be read.
    #[error("reading the roles file {path}: {source}")]
    RolesFile {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
}

/// Everything the operator routes share. Cheap to clone.
#[derive(Clone)]
pub struct OperatorState {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub config: OperatorConfig,
    pub pool: SqlitePool,
    pub approvals: Arc<dyn Approvals>,
    pub http: reqwest::Client,
    pub models_url: Option<String>,
    pub receipts: Option<Arc<dyn agentrix_host::Receipts>>,
    pub render: render::Render,
    pub actions: access::Pending,
    pub notices: notice::Notices,
    pub signins: identity::Ceremonies,
    pub throttle: identity::Throttle,
}

impl std::fmt::Debug for OperatorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperatorState")
            .field("origin", &self.inner.config.origin.as_str())
            .field("cell", &self.inner.config.cell.id)
            .finish_non_exhaustive()
    }
}

impl OperatorState {
    /// The operator console over `pool` (the first release's `control.db`, migrated through 0300), proving
    /// sign-ins and approvals through `approvals` (the ops host's).
    pub fn new(
        config: OperatorConfig,
        pool: SqlitePool,
        approvals: Arc<dyn Approvals>,
    ) -> Result<Self, Error> {
        let render = render::Render::new(config.utc_offset_minutes)?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                pool,
                approvals,
                http: reqwest::Client::new(),
                models_url: None,
                receipts: None,
                render,
                actions: access::Pending::default(),
                notices: notice::Notices::default(),
                signins: identity::Ceremonies::default(),
                throttle: identity::Throttle::default(),
            }),
        })
    }

    /// Reads the Model Gateway's status (provider, routes, daily cap) from `models_url`.
    pub fn with_gateway(mut self, http: reqwest::Client, models_url: String) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.http = http;
            inner.models_url = Some(models_url);
        }
        self
    }

    /// Signs owner-facing receipts (support grants, break-glass, holds) with node's key.
    pub fn with_receipts(mut self, receipts: Arc<dyn agentrix_host::Receipts>) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.receipts = Some(receipts);
        }
        self
    }

    /// The database, for tests and wiring checks.
    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    /// Applies the roles file: one person per line, `email role role…`, `#` for comments.
    /// Roles the file grants are added; roles it granted before and no longer lists are revoked.
    /// Roles an `op.admin` granted in the console are left alone. Each change is a ledger entry.
    /// Returns how many grants and revocations it made. A missing file changes nothing.
    pub async fn apply_roles_file(&self, path: &std::path::Path) -> Result<usize, Error> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no operator roles file; no operator can sign in yet");
                return Ok(0);
            }
            Err(source) => {
                return Err(Error::RolesFile {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        identity::apply_roles(&self.inner.pool, &text).await
    }
}

/// The ops host: `AGENTRIX_OPS_HOST`, or `ops.` and the platform's host.
pub fn ops_host(platform_host: &str) -> String {
    std::env::var("AGENTRIX_OPS_HOST")
        .ok()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| format!("ops.{platform_host}"))
}

/// The ops host's origin: the platform's scheme and port with the ops host.
pub fn ops_origin(platform_origin: &Url, ops_host: &str) -> Url {
    let mut origin = platform_origin.clone();
    if origin.set_host(Some(ops_host)).is_err() {
        tracing::warn!(ops_host, "the ops host isn't a valid host name");
    }
    origin.set_path("/");
    origin
}

/// The roles file: `AGENTRIX_OPERATORS_FILE`, or `operators.txt` in this crate's data folder.
pub fn roles_file(data_dir: &std::path::Path) -> PathBuf {
    std::env::var_os("AGENTRIX_OPERATORS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("agentrix-operator").join("operators.txt"))
}

/// Mounts the operator console on the ops host (contract C11). `ctx.approvals` must be the ops
/// host's approvals (see the crate docs). Spawns one task that applies the roles file at start
/// and every five minutes, so a change to the file takes effect without a restart.
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let host = ops_host(&ctx.platform_host);
    let mut config = OperatorConfig::new(ops_origin(&ctx.origin, &host));
    config.platform_origin = Some(ctx.origin.clone());
    let state = OperatorState::new(config, ctx.pool.clone(), ctx.approvals.clone())?
        .with_gateway(ctx.http.clone(), ctx.models_url.clone())
        .with_receipts(ctx.receipts.clone());
    let roles = roles_file(&ctx.data_dir);
    let task_state = state.clone();
    tracing::info!(%host, roles = %roles.display(), "operator console mounted");
    Ok(Mounted::default()
        .host(HostMatch::Exact(host), router(state))
        .task(
            "operator-roles",
            Box::pin(async move {
                loop {
                    match task_state.apply_roles_file(&roles).await {
                        Ok(0) => {}
                        Ok(n) => tracing::info!(
                            changes = n,
                            "operator roles updated from the roles file"
                        ),
                        Err(e) => tracing::warn!(error = %e, "operator roles file"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                }
            }),
        ))
}

/// Mounts the owner's side on the platform host (contract C11), with the platform's own
/// approvals and owner sessions: `/support-access/{grant}`, where an owner reads a support
/// request and grants it with a passkey or code, declines it, or ends it early, and
/// `/support-access.json`, what waits for the signed-in owner (for the console's lane).
pub fn mount_platform(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let host = ops_host(&ctx.platform_host);
    let state = owner::OwnerState::new(
        ctx.pool.clone(),
        ctx.approvals.clone(),
        ctx.owners.clone(),
        Some(ctx.receipts.clone()),
        ops_origin(&ctx.origin, &host),
    )?;
    Ok(Mounted::default().platform(owner::router(state)))
}

/// The ops host's routes: sign-in, the operator pages and their static files.
pub fn router(state: OperatorState) -> Router {
    Router::new()
        .merge(identity::routes())
        .merge(pages::routes())
        .merge(access::routes())
        .merge(incidents::routes())
        .merge(partners::routes())
        .merge(overview::routes())
        .layer(axum::middleware::from_fn(notice::scope))
        .with_state(state)
        .merge(assets::routes())
        // W7's kit (tokens, self-hosted fonts, shell) under /static/ui/ on the ops host.
        .merge(agentrix_ui::assets())
}

#[cfg(test)]
mod access_tests;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
