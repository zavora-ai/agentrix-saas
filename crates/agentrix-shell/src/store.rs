//! The shell's own rows: quiet hours, Freeze and the owner's last visit (`schema.sql`).

use agentrix_types::EffectRequest;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::future::BoxFuture;
use sqlx::{Row, SqlitePool};

use crate::schedule::QuietHours;

/// Runs the shell's migrations (range 0500–0599) with their own table, before node mounts the
/// shell. Node lists it in its `MIGRATIONS`.
pub fn migrate(pool: &SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_shell");
        m.run(pool)
            .await
            .map_err(|e| anyhow::anyhow!("agentrix-shell migrations: {e}"))?;
        Ok(())
    })
}

fn time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// A Freeze in force.
#[derive(Debug, Clone, PartialEq)]
pub struct Freeze {
    /// Its id.
    pub id: String,
    /// When.
    pub at: DateTime<Utc>,
    /// The account that froze the site.
    pub by: String,
    /// How: `console`, later `whatsapp`, `voice`, `gesture`.
    pub how: String,
    /// The effect an unfreeze approval signs, once the owner has asked to unfreeze.
    pub unfreeze_effect: Option<EffectRequest>,
}

/// The shell's tables in `control.db` (Horizon 1; its own store in Horizon 2). [`migrate`]
/// creates them.
#[derive(Debug, Clone)]
pub struct ShellStore {
    pool: SqlitePool,
    records: Option<SqlitePool>,
}

impl ShellStore {
    /// A store over `pool`, whose migrations have run.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            records: None,
        }
    }

    /// Reads orders for missions from W2's `records.db` at `path`, read-only. The file is opened
    /// on first use; while it's missing, nothing is counted as orders.
    pub fn with_records_at(mut self, path: &std::path::Path) -> Self {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false);
        self.records = Some(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(2)
                .connect_lazy_with(options),
        );
        self
    }

    /// W2's `records.db`, when the store reads one.
    pub fn records(&self) -> Option<&SqlitePool> {
        self.records.as_ref()
    }

    /// Runs the migrations on `pool`, then opens the store (tests and previews).
    pub async fn open(pool: SqlitePool) -> anyhow::Result<Self> {
        migrate(&pool).await?;
        Ok(Self::new(pool))
    }

    /// The site's missions.
    pub fn missions(&self) -> crate::missions::Missions {
        crate::missions::Missions::new(self.pool.clone()).with_records(self.records.clone())
    }

    /// Orders and customers of a site, read-only (`commerce`).
    pub fn commerce(&self) -> crate::commerce::Commerce {
        crate::commerce::Commerce::new(self.pool.clone())
    }

    /// Front of house conversations the owner can sit in on.
    pub fn live(&self) -> crate::live::Live {
        crate::live::Live::new(self.pool.clone())
    }

    /// The site's quiet hours, or the default.
    pub async fn quiet_hours(&self, site: &str) -> Result<QuietHours, sqlx::Error> {
        let row = sqlx::query("SELECT start_hm, end_hm FROM shell_quiet_hours WHERE site_id = ?")
            .bind(site)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .and_then(|r| QuietHours::parse(&r.get::<String, _>(0), &r.get::<String, _>(1)))
            .unwrap_or_default())
    }

    /// Sets the site's quiet hours.
    pub async fn set_quiet_hours(
        &self,
        site: &str,
        hours: QuietHours,
        by: &str,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO shell_quiet_hours (site_id, start_hm, end_hm, updated_at, updated_by) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (site_id) DO UPDATE SET start_hm = excluded.start_hm, end_hm = excluded.end_hm,
             updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        )
        .bind(site)
        .bind(crate::schedule::clock(hours.start))
        .bind(crate::schedule::clock(hours.end))
        .bind(time(now))
        .bind(by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The Freeze in force for the site, if any.
    pub async fn freeze(&self, site: &str) -> Result<Option<Freeze>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, frozen_at, frozen_by, how, unfreeze_effect FROM shell_freezes
             WHERE site_id = ? AND unfrozen_at IS NULL ORDER BY frozen_at DESC LIMIT 1",
        )
        .bind(site)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| Freeze {
            id: r.get(0),
            at: parse(&r.get::<String, _>(1)).unwrap_or_default(),
            by: r.get(2),
            how: r.get(3),
            unfreeze_effect: r
                .get::<Option<String>, _>(4)
                .and_then(|s| serde_json::from_str(&s).ok()),
        }))
    }

    /// Freezes the site. Freezing twice keeps the first Freeze.
    pub async fn set_frozen(
        &self,
        site: &str,
        by: &str,
        how: &str,
        now: DateTime<Utc>,
    ) -> Result<Freeze, sqlx::Error> {
        if let Some(f) = self.freeze(site).await? {
            return Ok(f);
        }
        let id = format!("frz_{}", uuid::Uuid::now_v7().simple());
        sqlx::query("INSERT INTO shell_freezes (id, site_id, frozen_at, frozen_by, how) VALUES (?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(site)
            .bind(time(now))
            .bind(by)
            .bind(how)
            .execute(&self.pool)
            .await?;
        Ok(Freeze {
            id,
            at: now,
            by: by.to_owned(),
            how: how.to_owned(),
            unfreeze_effect: None,
        })
    }

    /// Records the effect an unfreeze approval will sign.
    pub async fn ask_unfreeze(
        &self,
        freeze: &str,
        effect: &EffectRequest,
    ) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(effect).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        sqlx::query(
            "UPDATE shell_freezes SET unfreeze_effect = ? WHERE id = ? AND unfrozen_at IS NULL",
        )
        .bind(json)
        .bind(freeze)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Lifts the Freeze. False if it was already lifted.
    pub async fn unfreeze(
        &self,
        freeze: &str,
        by: &str,
        method: &str,
        receipt: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE shell_freezes SET unfrozen_at = ?, unfrozen_by = ?, unfreeze_method = ?, unfreeze_receipt = ?
             WHERE id = ? AND unfrozen_at IS NULL",
        )
        .bind(time(now))
        .bind(by)
        .bind(method)
        .bind(receipt)
        .bind(freeze)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// When the owner last opened the console for the site, and records `now` as the new visit.
    pub async fn visit(
        &self,
        account: &str,
        site: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        let before =
            sqlx::query("SELECT last_seen_at FROM shell_visits WHERE account = ? AND site_id = ?")
                .bind(account)
                .bind(site)
                .fetch_optional(&self.pool)
                .await?
                .and_then(|r| parse(&r.get::<String, _>(0)));
        sqlx::query(
            "INSERT INTO shell_visits (account, site_id, last_seen_at) VALUES (?, ?, ?)
             ON CONFLICT (account, site_id) DO UPDATE SET last_seen_at = excluded.last_seen_at",
        )
        .bind(account)
        .bind(site)
        .bind(time(now))
        .execute(&self.pool)
        .await?;
        Ok(before)
    }
}

/// Whether a site is frozen, for the governed paths that must stop (the brief, Ask, the door).
/// `agentrix-node` hands this to them; it reads the same table the shell writes.
#[derive(Debug, Clone)]
pub struct FreezeGate {
    store: ShellStore,
}

impl FreezeGate {
    /// A gate over the shell's store.
    pub fn new(store: ShellStore) -> Self {
        Self { store }
    }

    /// True while the owner's Freeze is in force. A read error counts as frozen: stopping is the
    /// safe side.
    pub async fn is_frozen(&self, site: &str) -> bool {
        match self.store.freeze(site).await {
            Ok(f) => f.is_some(),
            Err(e) => {
                tracing::error!(error = %e, site, "freeze check failed; treating the site as frozen");
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn store() -> ShellStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("pool");
        ShellStore::open(pool).await.expect("schema")
    }

    #[tokio::test]
    async fn quiet_hours_default_then_saved() {
        let s = store().await;
        assert_eq!(s.quiet_hours("s1").await.unwrap(), QuietHours::default());
        let q = QuietHours::parse("21:30", "05:45").unwrap();
        s.set_quiet_hours("s1", q, "acc", Utc::now()).await.unwrap();
        assert_eq!(s.quiet_hours("s1").await.unwrap(), q);
        assert_eq!(s.quiet_hours("s2").await.unwrap(), QuietHours::default());
    }

    #[tokio::test]
    async fn freeze_then_unfreeze_once() {
        let s = store().await;
        let gate = FreezeGate::new(s.clone());
        assert!(!gate.is_frozen("s1").await);
        let f = s
            .set_frozen("s1", "acc", "console", Utc::now())
            .await
            .unwrap();
        let again = s
            .set_frozen("s1", "acc", "console", Utc::now())
            .await
            .unwrap();
        assert_eq!(f.id, again.id, "freezing twice keeps the first Freeze");
        assert!(gate.is_frozen("s1").await);
        assert!(!gate.is_frozen("s2").await);
        assert!(
            s.unfreeze(&f.id, "acc", "passkey", "sha256:00", Utc::now())
                .await
                .unwrap()
        );
        assert!(
            !s.unfreeze(&f.id, "acc", "passkey", "sha256:00", Utc::now())
                .await
                .unwrap()
        );
        assert!(!gate.is_frozen("s1").await);
    }

    #[tokio::test]
    async fn visits_return_the_previous_one() {
        let s = store().await;
        let t1 = Utc::now();
        assert_eq!(s.visit("acc", "s1", t1).await.unwrap(), None);
        let seen = s
            .visit("acc", "s1", t1 + chrono::Duration::minutes(5))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(seen.timestamp_millis(), t1.timestamp_millis());
    }
}
