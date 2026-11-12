//! The seam every platform crate mounts into `agentrix-node` through (contract C11,
//! `specs/platform/02-architecture.md` §5.3).
//!
//! A crate exports one function with the [`ModuleFn`] shape:
//!
//! ```no_run
//! use agentrix_host::{Mounted, NodeContext};
//! use axum::{Router, routing::get};
//!
//! /// Mounts the operator console.
//! pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
//!     let ops_host = format!("ops.{}", ctx.platform_host);
//!     Ok(Mounted::default()
//!         .host(agentrix_host::HostMatch::Exact(ops_host), Router::new().route("/", get(|| async { "Fleet" }))))
//! }
//! ```
//!
//! and node's `app.rs` calls it once at startup. Node then:
//! - merges [`Mounted::platform`] into the platform host's router (agentrix.website);
//! - sends a request for any host a [`HostMatch`] in [`Mounted::hosts`] accepts to that router,
//!   and answers Caddy's TLS ask with yes for it, so the host gets a certificate;
//! - merges [`Mounted::internal`] into the loopback-only router;
//! - spawns each [`Task`].
//!
//! Crates never depend on `agentrix-node`. What they need from it (receipts, the signed-in
//! owner, passkey approvals bound to a fingerprint, the content store) comes through the four
//! traits here; node implements them in `wiring/`, and [`testing`] has fakes for tests.
//! Migrations stay in node's one migrator in Horizon 1, numbered in the crate's range
//! (`specs/platform/how-we-work.md`).

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use bytes::Bytes;
use futures::future::BoxFuture;
use http::HeaderMap;
use url::Url;
use uuid::Uuid;

pub use agentrix_types::ReceiptBody;

pub mod testing;

/// What node hands every module at startup.
#[derive(Clone)]
pub struct NodeContext {
    /// `control.db`, already migrated (Horizon 1; per-site stores come with W8's `agentrix-store`).
    pub pool: sqlx::SqlitePool,
    /// A shared HTTP client (the Model Gateway is at [`NodeContext::models_url`]).
    pub http: reqwest::Client,
    /// Node's data directory. A module keeps its own files under `data_dir/<crate name>/`.
    pub data_dir: PathBuf,
    /// The platform's origin as browsers see it, e.g. `https://agentrix.website`.
    pub origin: Url,
    /// The platform's host, e.g. `agentrix.website`.
    pub platform_host: String,
    /// The Model Gateway, loopback only. Call it by purpose, never a model or provider.
    pub models_url: String,
    /// Signed receipts.
    pub receipts: Arc<dyn Receipts>,
    /// Who is signed in as an owner on the platform host.
    pub owners: Arc<dyn OwnerSessions>,
    /// Passkey or authenticator approvals bound to a card's fingerprint (design §4.1, frozen).
    pub approvals: Arc<dyn Approvals>,
    /// Owner accounts, for sign-in methods built outside `auth/` (W6: email and recovery codes).
    pub owner_accounts: Arc<dyn OwnerAccounts>,
    /// The content-addressed store (releases, evidence, images, uploads).
    pub blobs: Arc<dyn Blobs>,
}

/// The function a crate exports for node to call once at startup.
pub type ModuleFn = fn(&NodeContext) -> anyhow::Result<Mounted>;

/// A crate's own migrator, run once at startup before any [`ModuleFn`]. Use a sqlx migrator
/// with its own table (`_sqlx_migrations_<crate>`), numbered in your range, and call
/// `set_ignore_missing(true)` so a binary that is older than the database still starts.
///
/// ```no_run
/// use futures::future::BoxFuture;
/// /// Runs this crate's migrations.
/// pub fn migrate(pool: &sqlx::SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
///     Box::pin(async move {
///         // let mut m = sqlx::migrate!("migrations"); m.set_ignore_missing(true); m.run(pool).await?;
///         let _ = pool;
///         Ok(())
///     })
/// }
/// ```
pub type MigrateFn = for<'a> fn(&'a sqlx::SqlitePool) -> BoxFuture<'a, anyhow::Result<()>>;

/// What a module adds to the node.
#[derive(Default)]
pub struct Mounted {
    /// Routes on the platform host (agentrix.website). Paths must not collide with another
    /// module's; the coordinator checks at merge.
    pub platform: Router,
    /// Routes on other hosts: `ops.agentrix.website`, the `-sample` sites and so on.
    pub hosts: Vec<HostRoutes>,
    /// Loopback-only routes (never reachable from the internet).
    pub internal: Router,
    /// Background work, spawned once.
    pub tasks: Vec<Task>,
}

impl Mounted {
    /// Adds routes on the platform host.
    pub fn platform(mut self, router: Router) -> Self {
        self.platform = self.platform.merge(router);
        self
    }

    /// Adds a router for the hosts `matcher` accepts.
    pub fn host(mut self, matcher: HostMatch, router: Router) -> Self {
        self.hosts.push(HostRoutes { matcher, router });
        self
    }

    /// Adds loopback-only routes.
    pub fn internal(mut self, router: Router) -> Self {
        self.internal = self.internal.merge(router);
        self
    }

    /// Adds a background task.
    pub fn task(mut self, name: &'static str, future: BoxFuture<'static, ()>) -> Self {
        self.tasks.push(Task { name, future });
        self
    }
}

/// A router for a set of hosts.
#[derive(Clone)]
pub struct HostRoutes {
    /// Which hosts it serves.
    pub matcher: HostMatch,
    /// Its routes.
    pub router: Router,
}

/// Which hosts a module serves. Hosts arrive lower-case, without a port.
#[derive(Clone)]
pub enum HostMatch {
    /// Exactly this host, e.g. `ops.agentrix.website`.
    Exact(String),
    /// Any of these hosts, e.g. the three `-sample` hosts.
    AnyOf(Vec<String>),
    /// Decided per request, e.g. from a table of registered sites.
    Lookup(Arc<dyn HostLookup>),
}

impl HostMatch {
    /// Whether this matcher serves `host`.
    pub async fn serves(&self, host: &str) -> anyhow::Result<bool> {
        Ok(match self {
            HostMatch::Exact(h) => h == host,
            HostMatch::AnyOf(hs) => hs.iter().any(|h| h == host),
            HostMatch::Lookup(lookup) => lookup.serves(host).await?,
        })
    }
}

/// A host decision that needs a lookup.
#[async_trait]
pub trait HostLookup: Send + Sync {
    /// Whether this module serves `host`.
    async fn serves(&self, host: &str) -> anyhow::Result<bool>;
}

/// A background task.
pub struct Task {
    /// For logs.
    pub name: &'static str,
    /// The work. It should loop forever or end on its own; node logs when it ends.
    pub future: BoxFuture<'static, ()>,
}

/// Signs and stores receipts with control's key (design §4.4).
#[async_trait]
pub trait Receipts: Send + Sync {
    /// Signs `body`, stores it and returns its digest (`sha256:<hex>`).
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<String>;
    /// The canonical JSON of the receipt with this digest, if it exists.
    async fn get_json(&self, digest: &str) -> anyhow::Result<Option<String>>;
}

/// The signed-in owner on the platform host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    /// The owner's account.
    pub account: Uuid,
    /// How they signed in: `passkey` or `password+totp`.
    pub method: String,
}

/// Who is signed in.
#[async_trait]
pub trait OwnerSessions: Send + Sync {
    /// The owner these request headers belong to, if any and still signed in.
    async fn owner(&self, headers: &HeaderMap) -> anyhow::Result<Option<Owner>>;
}

/// An owner account as sign-in sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerAccount {
    /// The account.
    pub account: Uuid,
    /// Sign-in email, lower-case.
    pub email: String,
    /// The name the console greets.
    pub display_name: String,
    /// Whether the account has at least one passkey.
    pub has_passkey: bool,
    /// Whether the account has a password (and so an authenticator).
    pub has_password: bool,
}

/// Owner accounts, for sign-in methods built outside `auth/` (W6's email codes and recovery
/// codes). A session started here is an ordinary owner session, but approvals still need a
/// passkey or authenticator code through [`Approvals`]: an email code alone never approves.
/// Only `agentrix-identity` uses this; every session it starts must be receipted.
#[async_trait]
pub trait OwnerAccounts: Send + Sync {
    /// The owner with this email (compared without case), if any.
    async fn by_email(&self, email: &str) -> anyhow::Result<Option<OwnerAccount>>;
    /// Creates an owner with no password; it signs in with passkeys or email codes. Errors if
    /// the email is taken.
    async fn create(&self, email: &str, display_name: &str) -> anyhow::Result<Uuid>;
    /// Starts an owner session exactly as `auth/` does and returns the `Set-Cookie` value.
    /// `method` is `email-code` or `recovery-code`.
    async fn start_session(&self, account: Uuid, method: &str)
    -> anyhow::Result<http::HeaderValue>;
}

/// Approvals bound to a card's fingerprint (design §4.1). The contract is frozen: modules
/// raise cards and ask for approvals through this trait and never verify on their own.
#[async_trait]
pub trait Approvals: Send + Sync {
    /// Passkey options for `navigator.credentials.get`, bound to `fingerprint`, as JSON; `None`
    /// when the account has no passkey and the page should ask for an authenticator code.
    async fn passkey_options(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<serde_json::Value>>;

    /// Verifies an approval of the card with `fingerprint` by `account`. `request` is the first release's
    /// approve body, `{"method": "passkey", "credential": <PublicKeyCredential JSON>}` or
    /// `{"method": "totp", "code": "123456"}`, or the short form `{"passkey": <credential>}` or
    /// `{"totp": "123456"}`; node accepts both. Returns the proof as
    /// JSON (method, account, fingerprint, time, and the assertion or time step), ready to go
    /// in the decision's receipt. An error means not approved.
    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        request: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value>;
}

/// The content-addressed store.
#[async_trait]
pub trait Blobs: Send + Sync {
    /// Stores `bytes` and returns their digest (`sha256:<hex>`).
    async fn put(&self, bytes: Bytes) -> anyhow::Result<String>;
    /// The bytes with this digest, if stored.
    async fn get(&self, digest: &str) -> anyhow::Result<Option<Bytes>>;
}
