//! The store layer (contract C5, specs/platform/02-architecture.md §6.1): a store per site,
//! blobs per site, receipt issuers that write a hash-chained site log, and the outbox.
//!
//! Single node (09 §3.3): `data/sites/<site>/records.db` and `data/sites/<site>/cas/`. The
//! fleet profile implements the same traits over a Postgres schema and role per site (03 §3.3);
//! callers don't change.
//!
//! - [`SiteStores::open`] opens (creating if needed) a site's store and runs every registered
//!   crate's site migrations on it, each crate in its own migrations table.
//! - [`SiteBlobs`] keeps a site's files apart from every other site's. C3 blobs need a
//!   [`Sealer`] (the site's data key, held by W6's `agentrix-keys`); without one they are
//!   refused rather than stored in the clear. C4 never goes in a blob.
//! - [`Issuer`] signs a receipt and appends it to the site log, whose `chain` column links each
//!   entry to the one before (the "chain predecessor" the Trust centre shows). [`SiteLog`]
//!   verifies the chain and redacts a body for erasure while keeping both digests.
//! - [`EventSink`] writes content-free events (C0, C1) to the outbox, in the same transaction
//!   as the change when the caller passes its connection; a relay drains them in order.
//!
//! Nothing here is wired into node yet (Horizon 1): the first release's `control.db` stays the store
//! for its own tables until the importer moves them (02 §9 item 4).

mod sqlite;
#[cfg(test)]
mod tests;

use agentrix_types::{Digest, KeyId, Receipt, ReceiptBody, SiteId};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use sqlite::{DiskBlobs, Outbox, SiteLog, SqliteSiteStores, Verification, chain_of};

/// How sensitive a record or event is (ADR-025, 03-data.md §1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DataClass {
    /// Public.
    C0,
    /// Content-free: counts, ids, kinds, durations.
    C1,
    /// Business-confidential.
    C2,
    /// Personal.
    C3,
    /// Secret: never stored outside the key holder.
    C4,
}

impl DataClass {
    /// `C0` … `C4`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::C0 => "C0",
            Self::C1 => "C1",
            Self::C2 => "C2",
            Self::C3 => "C3",
            Self::C4 => "C4",
        }
    }
}

/// What a store operation can fail with.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or failed.
    #[error("store database: {0}")]
    Db(#[from] sqlx::Error),
    /// A migration failed.
    #[error("store migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// The file system failed.
    #[error("store files: {0}")]
    Io(#[from] std::io::Error),
    /// The store refused the request, with the reason in plain words.
    #[error("{0}")]
    Refused(String),
    /// Signing or reading a receipt failed.
    #[error("receipt: {0}")]
    Receipt(String),
}

/// A store result.
pub type Result<T> = std::result::Result<T, StoreError>;

/// One crate's migrations for every site's `records.db`. Register them with
/// [`SqliteSiteStores::with_migrations`]; each runs with its own table
/// (`_sqlx_migrations_<owner>`) and tolerates versions it doesn't know, so a rollback starts.
#[derive(Debug, Clone, Copy)]
pub struct SiteMigrations {
    /// The owning crate, short: `records`, `kernel`, `identity`. Letters, digits and `_`.
    pub owner: &'static str,
    /// The crate's embedded migrator (`sqlx::migrate!("./migrations/site")`).
    pub migrator: &'static sqlx::migrate::Migrator,
}

/// One site's open stores.
#[derive(Clone)]
pub struct SiteStore {
    /// The site.
    pub site: SiteId,
    /// The site's records (its own `records.db`): every crate's site tables.
    pub records: sqlx::SqlitePool,
    /// The site's blobs.
    pub blobs: std::sync::Arc<dyn SiteBlobs>,
    /// The site log, as the control issuer.
    pub log: SiteLog,
    /// The site's outbox.
    pub outbox: Outbox,
}

impl std::fmt::Debug for SiteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiteStore")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

/// What closing a site removed (erasure class S, ADR-028).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Closed {
    /// Files removed.
    pub files: u64,
    /// Bytes removed.
    pub bytes: u64,
}

/// Where every site's stores live.
#[async_trait]
pub trait SiteStores: Send + Sync {
    /// Opens the site's stores, creating and migrating them if needed.
    async fn open(&self, site: &SiteId) -> Result<SiteStore>;
    /// Every site that has stores.
    async fn sites(&self) -> Result<Vec<SiteId>>;
    /// Removes the site's stores at closure. The caller has exported what the owner asked for
    /// and recorded the closure; this cannot be undone.
    async fn close(&self, site: &SiteId) -> Result<Closed>;
}

/// What a blob is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobMeta {
    /// What kind of thing: `page`, `evidence`, `image`, `export` …
    pub kind: String,
    /// Its media type.
    pub content_type: String,
    /// Its class. C3 needs a sealer; C4 is refused.
    pub class: DataClass,
}

/// Seals and opens a site's C3 blobs with the site's data key (ADR-030). W6's `agentrix-keys`
/// implements it; the key never leaves the key holder.
#[async_trait]
pub trait Sealer: Send + Sync {
    /// Encrypts `plain` for `site`.
    async fn seal(&self, site: &SiteId, plain: &[u8]) -> Result<Vec<u8>>;
    /// Decrypts what [`Sealer::seal`] made for `site`.
    async fn open(&self, site: &SiteId, sealed: &[u8]) -> Result<Vec<u8>>;
}

/// A site's blobs, addressed by the digest of their plain bytes.
#[async_trait]
pub trait SiteBlobs: Send + Sync {
    /// Stores `bytes` and returns their digest. Storing the same bytes again is a no-op.
    async fn put(&self, bytes: &[u8], meta: &BlobMeta) -> Result<Digest>;
    /// The bytes with this digest, if this site holds them.
    async fn get(&self, digest: &Digest) -> Result<Option<Vec<u8>>>;
    /// Unlinks the blob from this site and deletes its bytes. Returns whether it was there.
    async fn remove(&self, digest: &Digest) -> Result<bool>;
}

/// One entry of a site log.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// Its place in the log, from 1.
    pub seq: i64,
    /// The signed receipt's digest.
    pub receipt_digest: Digest,
    /// The chain value up to and including this entry (hex).
    pub chain: String,
    /// The chain value before it ("" for the first).
    pub prev_chain: String,
    /// The signed receipt, unless its body was redacted for erasure.
    pub receipt: Option<Receipt>,
}

/// Signs receipts for one service (control, edge, effects …) and appends them to a site log.
#[async_trait]
pub trait Issuer: Send + Sync {
    /// The key this issuer signs with.
    fn key_id(&self) -> &KeyId;
    /// Signs `body` (its site must be the log's), appends it and returns the entry.
    async fn issue(&self, body: ReceiptBody) -> Result<LogEntry>;
}

/// A content-free event for the outbox.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// What happened, as `agx.<area>.v<n>`: `agx.usage.v1`, `agx.ledger.v1` …
    pub topic: String,
    /// A partition key: an id, never a person.
    pub key: String,
    /// C0 or C1. Anything more sensitive stays inside the site.
    pub class: DataClass,
    /// The event's fields.
    pub payload: serde_json::Value,
}

/// Where events go.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Writes one event; returns its sequence number.
    async fn emit(&self, event: &Event) -> Result<i64>;
}
