//! The owner recovery tables (migration 0601).

use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::now;

/// A recovery hold.
#[derive(Debug, Clone)]
pub struct HoldRow {
    /// `rh_…`.
    pub id: String,
    /// When asked.
    pub requested_at: i64,
    /// When it ends.
    pub release_at: i64,
    /// The device that asked.
    pub device: String,
}

/// SHA-256(account ‖ code), hex.
pub fn code_hash(account: Uuid, code: &str) -> String {
    let mut h = Sha256::new();
    h.update(account.to_string().as_bytes());
    h.update(b"\0");
    h.update(code.as_bytes());
    hex::encode(h.finalize())
}

/// The owner recovery tables.
#[derive(Debug, Clone)]
pub struct OwnerStore {
    pool: SqlitePool,
}

impl OwnerStore {
    /// Over a pool migrated with [`crate::migrate`].
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Replaces the account's recovery codes.
    pub async fn set_recovery_codes(
        &self,
        account: Uuid,
        codes: &[String],
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM owner_recovery_codes WHERE account_id = ?")
            .bind(account.to_string())
            .execute(&mut *tx)
            .await?;
        let at = now();
        for code in codes {
            sqlx::query("INSERT INTO owner_recovery_codes (account_id, code_hash, created_at) VALUES (?, ?, ?)")
                .bind(account.to_string())
                .bind(code_hash(account, code))
                .bind(at)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    /// Spends a recovery code: true if it was unused.
    pub async fn use_recovery_code(&self, account: Uuid, code: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("UPDATE owner_recovery_codes SET used_at = ? WHERE account_id = ? AND code_hash = ? AND used_at IS NULL")
            .bind(now())
            .bind(account.to_string())
            .bind(code_hash(account, code))
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Unused codes.
    pub async fn recovery_codes_left(&self, account: Uuid) -> Result<usize, sqlx::Error> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM owner_recovery_codes WHERE account_id = ? AND used_at IS NULL",
        )
        .bind(account.to_string())
        .fetch_one(&self.pool)
        .await?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Records a code sign-in; returns its id.
    pub async fn record_sign_in(
        &self,
        account: Uuid,
        method: &str,
        device: &str,
        ip_hash: &str,
    ) -> Result<String, sqlx::Error> {
        let id = format!("ocs_{}", Uuid::now_v7().simple());
        sqlx::query("INSERT INTO owner_code_sign_ins (id, account_id, at, method, device, ip_hash) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(account.to_string())
            .bind(now())
            .bind(method)
            .bind(device)
            .bind(ip_hash)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    /// Stores the receipt digest of a code sign-in.
    pub async fn set_sign_in_receipt(&self, id: &str, receipt: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE owner_code_sign_ins SET receipt = ? WHERE id = ?")
            .bind(receipt)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Code sign-ins of an account, newest first: `(method, receipt)`.
    pub async fn sign_ins(
        &self,
        account: Uuid,
    ) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
        sqlx::query_as("SELECT method, receipt FROM owner_code_sign_ins WHERE account_id = ? ORDER BY at DESC, id DESC")
            .bind(account.to_string())
            .fetch_all(&self.pool)
            .await
    }

    /// Starts a hold; returns its id.
    pub async fn start_hold(
        &self,
        account: Uuid,
        release_at: i64,
        device: &str,
    ) -> Result<String, sqlx::Error> {
        let id = format!("rh_{}", Uuid::now_v7().simple());
        sqlx::query(
            "INSERT INTO owner_recovery_holds (id, account_id, requested_at, release_at, device, state) VALUES (?, ?, ?, ?, ?, 'held')",
        )
        .bind(&id)
        .bind(account.to_string())
        .bind(now())
        .bind(release_at)
        .bind(device)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// A held, not yet released hold: `(id, release_at)`.
    pub async fn open_hold(&self, account: Uuid) -> Result<Option<(String, i64)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, release_at FROM owner_recovery_holds WHERE account_id = ? AND state = 'held' AND release_at > ? ORDER BY requested_at DESC LIMIT 1",
        )
        .bind(account.to_string())
        .bind(now())
        .fetch_optional(&self.pool)
        .await
    }

    /// A held hold whose time has come, if any.
    pub async fn released_hold(&self, account: Uuid) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM owner_recovery_holds WHERE account_id = ? AND state = 'held' AND release_at <= ? ORDER BY requested_at LIMIT 1",
        )
        .bind(account.to_string())
        .bind(now())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Ends a hold as `completed` or `cancelled`.
    pub async fn end_hold(&self, id: &str, state: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE owner_recovery_holds SET state = ?, ended_at = ? WHERE id = ? AND state = 'held'")
            .bind(state)
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Cancels the account's hold. True if one was held.
    pub async fn cancel_hold(&self, account: Uuid, id: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("UPDATE owner_recovery_holds SET state = 'cancelled', ended_at = ? WHERE id = ? AND account_id = ? AND state = 'held'")
            .bind(now())
            .bind(id)
            .bind(account.to_string())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Held holds of the account, newest first.
    pub async fn holds(&self, account: Uuid) -> Result<Vec<HoldRow>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, requested_at, release_at, device FROM owner_recovery_holds WHERE account_id = ? AND state = 'held' ORDER BY requested_at DESC",
        )
        .bind(account.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(HoldRow {
                    id: r.try_get("id")?,
                    requested_at: r.try_get("requested_at")?,
                    release_at: r.try_get("release_at")?,
                    device: r.try_get("device")?,
                })
            })
            .collect()
    }

    /// Moves a hold's release time (tests only: lets a test pass the 24 hours).
    #[doc(hidden)]
    pub async fn set_release_at(&self, id: &str, release_at: i64) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE owner_recovery_holds SET release_at = ? WHERE id = ?")
            .bind(release_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
