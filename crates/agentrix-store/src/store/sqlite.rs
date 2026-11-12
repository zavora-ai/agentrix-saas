//! The single-node implementation: a directory per site with its own `records.db` and CAS.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use agentrix_types::{Digest, KeyId, KeySet, Receipt, ReceiptBody, Signer, SiteId};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{AssertSqlSafe, SqlitePool};
use tokio::sync::Mutex;

use super::{
    BlobMeta, Closed, DataClass, Event, EventSink, Issuer, LogEntry, Result, Sealer, SiteBlobs,
    SiteMigrations, SiteStore, SiteStores, StoreError,
};
use crate::time::stamp;

/// W8's own tables in every site's `records.db`.
static SITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/site");

/// The first line of every chain hash, so a site-log chain can't be mistaken for another hash.
const CHAIN_DOMAIN: &str = "agentrix.site-log.v1";

/// A site id safe to use as a directory name: letters, digits, `_` and `-`, at most 80.
fn valid_site_id(site: &SiteId) -> bool {
    let s = site.as_str();
    (1..=80).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !s.starts_with('-')
}

/// Every site's stores under one directory (`data/sites/`).
pub struct SqliteSiteStores {
    root: PathBuf,
    signer: Arc<Signer>,
    sealer: Option<Arc<dyn Sealer>>,
    migrations: Vec<SiteMigrations>,
    open: Mutex<HashMap<String, SiteStore>>,
}

impl std::fmt::Debug for SqliteSiteStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSiteStores")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl SqliteSiteStores {
    /// Stores under `root`, with site logs signed by `signer` (control's issuer key).
    pub fn new(root: impl Into<PathBuf>, signer: Arc<Signer>) -> Self {
        Self {
            root: root.into(),
            signer,
            sealer: None,
            migrations: Vec::new(),
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Runs `migrations` on every site's `records.db` when it opens.
    ///
    /// # Panics
    ///
    /// If the owner name isn't letters, digits and `_` (it names a table).
    pub fn with_migrations(mut self, migrations: SiteMigrations) -> Self {
        assert!(
            !migrations.owner.is_empty()
                && migrations
                    .owner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "site migrations owner {:?} must be letters, digits and _",
            migrations.owner
        );
        self.migrations.push(migrations);
        self
    }

    /// Seals C3 blobs with each site's data key.
    pub fn with_sealer(mut self, sealer: Arc<dyn Sealer>) -> Self {
        self.sealer = Some(sealer);
        self
    }

    fn dir(&self, site: &SiteId) -> Result<PathBuf> {
        if !valid_site_id(site) {
            return Err(StoreError::Refused(format!(
                "{:?} isn't a site id this store can hold.",
                site.as_str()
            )));
        }
        Ok(self.root.join(site.as_str()))
    }

    async fn migrate(&self, pool: &SqlitePool) -> Result<()> {
        let run = |owner: &str, source: &sqlx::migrate::Migrator| {
            let mut migrator = sqlx::migrate::Migrator {
                migrations: source.migrations.clone(),
                ..sqlx::migrate::Migrator::DEFAULT
            };
            migrator.dangerous_set_table_name(format!("_sqlx_migrations_{owner}"));
            migrator.set_ignore_missing(true);
            migrator
        };
        run("store", &SITE_MIGRATOR).run(pool).await?;
        for m in &self.migrations {
            run(m.owner, m.migrator).run(pool).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl SiteStores for SqliteSiteStores {
    async fn open(&self, site: &SiteId) -> Result<SiteStore> {
        let mut open = self.open.lock().await;
        if let Some(store) = open.get(site.as_str()) {
            return Ok(store.clone());
        }
        let dir = self.dir(site)?;
        tokio::fs::create_dir_all(dir.join("cas")).await?;
        let options = SqliteConnectOptions::new()
            .filename(dir.join("records.db"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        self.migrate(&pool).await?;
        let store = SiteStore {
            site: site.clone(),
            records: pool.clone(),
            blobs: Arc::new(DiskBlobs {
                site: site.clone(),
                dir: dir.join("cas"),
                pool: pool.clone(),
                sealer: self.sealer.clone(),
            }),
            log: SiteLog {
                site: site.clone(),
                pool: pool.clone(),
                signer: self.signer.clone(),
                key_id: self.signer.key_id().clone(),
                append: Arc::new(Mutex::new(())),
            },
            outbox: Outbox {
                site: site.clone(),
                pool,
            },
        };
        open.insert(site.as_str().to_owned(), store.clone());
        Ok(store)
    }

    async fn sites(&self) -> Result<Vec<SiteId>> {
        let mut out = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let site = SiteId::new(name);
            if valid_site_id(&site)
                && tokio::fs::try_exists(entry.path().join("records.db")).await?
            {
                out.push(site);
            }
        }
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(out)
    }

    async fn close(&self, site: &SiteId) -> Result<Closed> {
        let dir = self.dir(site)?;
        if let Some(store) = self.open.lock().await.remove(site.as_str()) {
            store.records.close().await;
        }
        let mut closed = Closed::default();
        if !tokio::fs::try_exists(&dir).await? {
            return Ok(closed);
        }
        let mut stack = vec![dir.clone()];
        while let Some(path) = stack.pop() {
            let mut entries = tokio::fs::read_dir(&path).await?;
            while let Some(entry) = entries.next_entry().await? {
                let meta = entry.metadata().await?;
                if meta.is_dir() {
                    stack.push(entry.path());
                } else {
                    closed.files += 1;
                    closed.bytes += meta.len();
                }
            }
        }
        tokio::fs::remove_dir_all(&dir).await?;
        Ok(closed)
    }
}

/// A site's blobs on disk: `cas/sha256/<aa>/<rest>`, indexed in its `blobs` table.
pub struct DiskBlobs {
    site: SiteId,
    dir: PathBuf,
    pool: SqlitePool,
    sealer: Option<Arc<dyn Sealer>>,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl DiskBlobs {
    fn path(&self, digest: &Digest) -> PathBuf {
        let hex = digest.hex();
        self.dir.join("sha256").join(&hex[..2]).join(&hex[2..])
    }
}

async fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::Refused("a blob path has no folder".to_owned()))?;
    tokio::fs::create_dir_all(parent).await?;
    let temp = parent.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    tokio::fs::write(&temp, bytes).await?;
    tokio::fs::rename(&temp, path).await?;
    Ok(())
}

#[async_trait]
impl SiteBlobs for DiskBlobs {
    async fn put(&self, bytes: &[u8], meta: &BlobMeta) -> Result<Digest> {
        let digest = Digest::of(bytes);
        let (stored, sealed) = match (meta.class, &self.sealer) {
            (DataClass::C4, _) => {
                return Err(StoreError::Refused(
                    "Secrets never go in a blob; the key holder keeps them.".to_owned(),
                ));
            }
            (DataClass::C3, None) => {
                return Err(StoreError::Refused(
                    "This blob holds personal data and the site's key holder isn't connected, \
                     so it wasn't stored."
                        .to_owned(),
                ));
            }
            (DataClass::C3, Some(sealer)) => (sealer.seal(&self.site, bytes).await?, true),
            _ => (bytes.to_vec(), false),
        };
        let path = self.path(&digest);
        if !tokio::fs::try_exists(&path).await? {
            write_atomically(&path, &stored).await?;
        }
        sqlx::query(
            "INSERT INTO blobs (site_id, digest, kind, content_type, size, class, sealed, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (site_id, digest) DO NOTHING",
        )
        .bind(self.site.as_str())
        .bind(digest.to_string())
        .bind(&meta.kind)
        .bind(&meta.content_type)
        .bind(i64::try_from(bytes.len()).unwrap_or(i64::MAX))
        .bind(meta.class.as_str())
        .bind(i64::from(sealed))
        .bind(stamp(Utc::now()))
        .execute(&self.pool)
        .await?;
        Ok(digest)
    }

    async fn get(&self, digest: &Digest) -> Result<Option<Vec<u8>>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT sealed FROM blobs WHERE site_id = ? AND digest = ?")
                .bind(self.site.as_str())
                .bind(digest.to_string())
                .fetch_optional(&self.pool)
                .await?;
        let Some((sealed,)) = row else {
            return Ok(None);
        };
        let stored = match tokio::fs::read(self.path(digest)).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let plain = if sealed == 1 {
            let sealer = self.sealer.as_ref().ok_or_else(|| {
                StoreError::Refused(
                    "This blob is sealed and the site's key holder isn't connected.".to_owned(),
                )
            })?;
            sealer.open(&self.site, &stored).await?
        } else {
            stored
        };
        if Digest::of(&plain) != *digest {
            return Err(StoreError::Refused(format!(
                "Blob {digest} doesn't match its digest on disk."
            )));
        }
        Ok(Some(plain))
    }

    async fn remove(&self, digest: &Digest) -> Result<bool> {
        let done = sqlx::query("DELETE FROM blobs WHERE site_id = ? AND digest = ?")
            .bind(self.site.as_str())
            .bind(digest.to_string())
            .execute(&self.pool)
            .await?;
        match tokio::fs::remove_file(self.path(digest)).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(done.rows_affected() > 0)
    }
}

/// The chain value after appending `receipt_digest` to a log whose chain was `prev`.
pub fn chain_of(prev: &str, receipt_digest: &Digest) -> String {
    Digest::of(format!("{CHAIN_DOMAIN}\n{prev}\n{receipt_digest}").as_bytes()).hex()
}

/// What checking a site log found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// Entries checked.
    pub entries: i64,
    /// Entries whose body was redacted for erasure (their digests still checked).
    pub redacted: i64,
    /// The chain value of the last entry.
    pub head: Option<String>,
    /// The first problem: the entry's seq and what was wrong.
    pub problem: Option<(i64, String)>,
}

/// A site's hash-chained log of receipts, and the issuer that appends to it.
#[derive(Clone)]
pub struct SiteLog {
    site: SiteId,
    pool: SqlitePool,
    signer: Arc<Signer>,
    key_id: KeyId,
    append: Arc<Mutex<()>>,
}

type LogRow = (i64, String, String, String, Option<String>);

impl SiteLog {
    /// How many entries the log holds since `since` (stored time; "" for all).
    pub async fn count_since(&self, since: &str) -> Result<i64> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM site_log WHERE issued_at >= ?")
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }

    /// Entries after `after_seq`, oldest first, at most `limit`.
    pub async fn entries(&self, after_seq: i64, limit: u32) -> Result<Vec<LogEntry>> {
        let rows: Vec<LogRow> = sqlx::query_as(
            "SELECT seq, receipt_digest, prev_chain, chain, body FROM site_log \
             WHERE seq > ? ORDER BY seq LIMIT ?",
        )
        .bind(after_seq)
        .bind(i64::from(limit.clamp(1, 1000)))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(entry_of).collect()
    }

    /// Redacts the body of the entry for `receipt_digest` (erasure method R, ADR-028): the
    /// receipt's words go, both digests stay, the chain still verifies. Returns whether a
    /// body was redacted.
    pub async fn redact(&self, receipt_digest: &Digest) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE site_log SET body = NULL, redacted_at = ? \
             WHERE receipt_digest = ? AND body IS NOT NULL",
        )
        .bind(stamp(Utc::now()))
        .bind(receipt_digest.to_string())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Checks the whole log: contiguous seqs, each chain value from the one before, each body
    /// matching its digest, and each signature against `keys`.
    pub async fn verify(&self, keys: &KeySet) -> Result<Verification> {
        let rows: Vec<LogRow> = sqlx::query_as(
            "SELECT seq, receipt_digest, prev_chain, chain, body FROM site_log ORDER BY seq",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut v = Verification {
            entries: 0,
            redacted: 0,
            head: None,
            problem: None,
        };
        let mut prev = String::new();
        for (seq, digest_text, prev_chain, chain, body) in rows {
            v.entries += 1;
            let fail = |why: String| Some((seq, why));
            if seq != v.entries {
                v.problem = fail(format!("expected entry {} here", v.entries));
                break;
            }
            if prev_chain != prev {
                v.problem = fail("its predecessor doesn't match the entry before".to_owned());
                break;
            }
            let Ok(digest) = digest_text.parse::<Digest>() else {
                v.problem = fail("its receipt digest isn't a digest".to_owned());
                break;
            };
            if chain != chain_of(&prev, &digest) {
                v.problem = fail("its chain value doesn't follow from the entry before".to_owned());
                break;
            }
            match body {
                None => v.redacted += 1,
                Some(body) => {
                    if Digest::of(body.as_bytes()) != digest {
                        v.problem = fail("its body doesn't match its digest".to_owned());
                        break;
                    }
                    let checked = serde_json::from_str::<Receipt>(&body)
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.verify_with(keys).map(|_| ()).map_err(|e| e.to_string()));
                    if let Err(why) = checked {
                        v.problem = fail(format!("its signature doesn't verify: {why}"));
                        break;
                    }
                }
            }
            prev = chain;
        }
        if v.problem.is_none() && !prev.is_empty() {
            v.head = Some(prev);
        }
        Ok(v)
    }
}

fn entry_of((seq, digest, prev_chain, chain, body): LogRow) -> Result<LogEntry> {
    let receipt_digest = digest
        .parse::<Digest>()
        .map_err(|e| StoreError::Receipt(e.to_string()))?;
    let receipt = body
        .map(|b| serde_json::from_str::<Receipt>(&b))
        .transpose()
        .map_err(|e| StoreError::Receipt(e.to_string()))?;
    Ok(LogEntry {
        seq,
        receipt_digest,
        chain,
        prev_chain,
        receipt,
    })
}

#[async_trait]
impl Issuer for SiteLog {
    fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    async fn issue(&self, body: ReceiptBody) -> Result<LogEntry> {
        if body.site != self.site {
            return Err(StoreError::Refused(format!(
                "A receipt for {} can't go in {}'s log.",
                body.site.as_str(),
                self.site.as_str()
            )));
        }
        let receipt =
            Receipt::sign(body, &self.signer).map_err(|e| StoreError::Receipt(e.to_string()))?;
        let json = receipt
            .canonical_json()
            .map_err(|e| StoreError::Receipt(e.to_string()))?;
        let digest = Digest::of(&json);
        let json = String::from_utf8(json).map_err(|e| StoreError::Receipt(e.to_string()))?;
        let _one_at_a_time = self.append.lock().await;
        let last: Option<(i64, String)> =
            sqlx::query_as("SELECT seq, chain FROM site_log ORDER BY seq DESC LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;
        let (seq, prev) = last.map_or((1, String::new()), |(s, c)| (s + 1, c));
        let chain = chain_of(&prev, &digest);
        sqlx::query(
            "INSERT INTO site_log (seq, site_id, receipt_digest, prev_chain, chain, kind, issuer, \
             issued_at, body) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(seq)
        .bind(self.site.as_str())
        .bind(digest.to_string())
        .bind(&prev)
        .bind(&chain)
        .bind(receipt.body.kind.as_str())
        .bind(receipt.body.issuer.as_str())
        .bind(stamp(Utc::now()))
        .bind(json)
        .execute(&self.pool)
        .await?;
        Ok(LogEntry {
            seq,
            receipt_digest: digest,
            chain,
            prev_chain: prev,
            receipt: Some(receipt),
        })
    }
}

/// A site's outbox.
#[derive(Clone)]
pub struct Outbox {
    site: SiteId,
    pool: SqlitePool,
}

impl Outbox {
    /// Writes `event` on `conn`, so it commits or rolls back with the caller's own writes.
    pub async fn emit_in(&self, conn: &mut SqliteConnection, event: &Event) -> Result<i64> {
        if event.class > DataClass::C1 {
            return Err(StoreError::Refused(format!(
                "{} events stay inside the site; only C0 and C1 go in the outbox.",
                event.class.as_str()
            )));
        }
        if !event.topic.starts_with("agx.") || !event.payload.is_object() {
            return Err(StoreError::Refused(
                "An event needs an agx.* topic and an object payload.".to_owned(),
            ));
        }
        let (seq,): (i64,) = sqlx::query_as(
            "INSERT INTO outbox (site_id, topic, key, class, payload, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) RETURNING seq",
        )
        .bind(self.site.as_str())
        .bind(&event.topic)
        .bind(&event.key)
        .bind(event.class.as_str())
        .bind(event.payload.to_string())
        .bind(stamp(Utc::now()))
        .fetch_one(&mut *conn)
        .await?;
        Ok(seq)
    }

    /// Events `relay` hasn't acknowledged, oldest first.
    pub async fn pending(&self, relay: &str, limit: u32) -> Result<Vec<(i64, Event)>> {
        let rows: Vec<(i64, String, String, String, String)> = sqlx::query_as(AssertSqlSafe(
            "SELECT seq, topic, key, class, payload FROM outbox WHERE seq > \
             COALESCE((SELECT seq FROM relay_cursors WHERE relay = ?), 0) ORDER BY seq LIMIT ?",
        ))
        .bind(relay)
        .bind(i64::from(limit.clamp(1, 1000)))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(seq, topic, key, class, payload)| {
                Ok((
                    seq,
                    Event {
                        topic,
                        key,
                        class: if class == "C0" {
                            DataClass::C0
                        } else {
                            DataClass::C1
                        },
                        payload: serde_json::from_str(&payload)
                            .map_err(|e| StoreError::Receipt(e.to_string()))?,
                    },
                ))
            })
            .collect()
    }

    /// Records that `relay` has everything up to `seq`. Never moves a cursor back.
    pub async fn ack(&self, relay: &str, seq: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO relay_cursors (relay, seq, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT (relay) DO UPDATE SET seq = MAX(seq, excluded.seq), \
             updated_at = excluded.updated_at",
        )
        .bind(relay)
        .bind(seq)
        .bind(stamp(Utc::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl EventSink for Outbox {
    async fn emit(&self, event: &Event) -> Result<i64> {
        let mut conn = self.pool.acquire().await?;
        self.emit_in(&mut conn, event).await
    }
}
