//! Commerce and content records for sites (W2, specs/platform/03-data.md §1.3–1.5).
//!
//! One SQLite file, `records.db`, with its own migrator (migrations 0200–0299). Every row is keyed
//! by the site's slug. Money is in minor units of the row's currency. Times are unix seconds;
//! each site has a fixed UTC offset (Nairobi is +03:00 all year).
//!
//! Numbers on screen are computed from rows here, never typed into a page: stock left is the
//! day's plan less what orders and holds took, a slot is full when its orders reach capacity,
//! a price is the sum of its rule lines.
//!
//! Sample sites are loaded from bundles under `fixtures/samples/<site>/` by [`seed`].

/// `format!` for SQL built only from constants in this crate (never from input).
macro_rules! sql {
    ($($t:tt)*) => { sqlx::AssertSqlSafe(format!($($t)*)) };
}

pub mod addresses;
pub mod catalogue;
pub mod confirmations;
pub mod feedback;
pub mod money;
pub mod news;
pub mod orders;
pub mod people;
pub mod pricing;
pub mod questions;
pub mod seed;
pub mod time;

use std::path::Path;
use std::str::FromStr;

use anyhow::Context as _;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// The records store.
#[derive(Clone, Debug)]
pub struct Records {
    pool: SqlitePool,
}

/// A site the records store knows.
#[derive(Debug, Clone, Serialize)]
pub struct Site {
    /// The slug, such as `kilele-bakery-sample`.
    pub site: String,
    /// The canonical host.
    pub host: String,
    /// The business name.
    pub name: String,
    /// `bakery`, `shop` or `news`.
    pub archetype: String,
    /// ISO currency of its prices.
    pub currency: String,
    /// Minutes east of UTC.
    pub utc_offset_min: i32,
    /// `sample` for the sample sites.
    pub origin: String,
    /// Facts the pages and desks cite: hours, address, owner, policies (from the bundle).
    pub facts: Value,
    /// The bundle version loaded.
    pub seed_version: String,
    /// The instant the bundle's anchor was mapped to.
    pub anchor_at: i64,
}

impl Site {
    /// Whether this is a sample site.
    pub fn is_sample(&self) -> bool {
        self.origin == "sample"
    }

    /// A fact by JSON pointer, as text (`""` when absent).
    pub fn fact(&self, pointer: &str) -> String {
        match self.facts.pointer(pointer) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(v) => v.to_string(),
        }
    }
}

impl Records {
    /// Opens (creating if needed) the store at `path` and runs its migrations.
    ///
    /// # Errors
    ///
    /// When the file can't be opened or a migration fails.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        Self::from_pool(pool).await
    }

    /// An in-memory store, for tests and previews.
    ///
    /// # Errors
    ///
    /// When a migration fails.
    pub async fn open_in_memory() -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        Self::from_pool(pool).await
    }

    async fn from_pool(pool: SqlitePool) -> anyhow::Result<Self> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("running the records migrations")?;
        Ok(Self { pool })
    }

    /// The pool: for modules in this crate, and for reads other crates make over the same
    /// records (the AI budget's currency and posted rate, `agentrix_store::budget`).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Every site, by slug.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn sites(&self) -> anyhow::Result<Vec<Site>> {
        let rows: Vec<SiteRow> = sqlx::query_as(
            "SELECT site, host, name, archetype, currency, utc_offset_min, origin, facts, seed_version, anchor_at FROM rec_sites ORDER BY site",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(SiteRow::into_site).collect()
    }

    /// One site by slug.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn site(&self, slug: &str) -> anyhow::Result<Option<Site>> {
        let row: Option<SiteRow> = sqlx::query_as(
            "SELECT site, host, name, archetype, currency, utc_offset_min, origin, facts, seed_version, anchor_at FROM rec_sites WHERE site = ?",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?;
        row.map(SiteRow::into_site).transpose()
    }

    /// The next number in a site's named sequence (`order` → 1045), starting at `start`.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn next_number(&self, site: &str, name: &str, start: i64) -> anyhow::Result<i64> {
        let mut tx = self.pool.begin().await?;
        let current: Option<(i64,)> =
            sqlx::query_as("SELECT next FROM rec_counters WHERE site = ? AND name = ?")
                .bind(site)
                .bind(name)
                .fetch_optional(&mut *tx)
                .await?;
        let n = current.map_or(start, |(n,)| n.max(start));
        sqlx::query(
            "INSERT INTO rec_counters (site, name, next) VALUES (?, ?, ?)
             ON CONFLICT (site, name) DO UPDATE SET next = excluded.next",
        )
        .bind(site)
        .bind(name)
        .bind(n + 1)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(n)
    }
}

#[derive(sqlx::FromRow)]
struct SiteRow {
    site: String,
    host: String,
    name: String,
    archetype: String,
    currency: String,
    utc_offset_min: i64,
    origin: String,
    facts: String,
    seed_version: String,
    anchor_at: i64,
}

impl SiteRow {
    fn into_site(self) -> anyhow::Result<Site> {
        Ok(Site {
            site: self.site,
            host: self.host,
            name: self.name,
            archetype: self.archetype,
            currency: self.currency,
            utc_offset_min: i32::try_from(self.utc_offset_min).unwrap_or(0),
            origin: self.origin,
            facts: serde_json::from_str(&self.facts).context("site facts")?,
            seed_version: self.seed_version,
            anchor_at: self.anchor_at,
        })
    }
}

/// A random id with a prefix: `cus-3f9a…` (16 hex characters).
pub fn new_id(prefix: &str) -> String {
    let bytes: [u8; 8] = rand::random();
    format!("{prefix}-{}", hex::encode(bytes))
}

/// SHA-256 of `text`, hex.
pub fn sha256_hex(text: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}
