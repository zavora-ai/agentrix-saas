//! The visitor tables (migration 0600). Every query names the site, so a row from one site is
//! never read for another (defence in depth beside the per-site store W8 brings).

use agentrix_types::SiteId;
use agentrix_types::tenancy::VisitorAccountId;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::now;

/// Errors from the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// An account with this email already exists on the site.
    #[error("email taken")]
    EmailTaken,
    /// The database failed.
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    /// A stored passkey did not parse.
    #[error("stored passkey: {0}")]
    Passkey(String),
}

/// A customer account on one site.
#[derive(Debug, Clone)]
pub struct AccountRow {
    /// `vac_…`.
    pub id: VisitorAccountId,
    /// The site.
    pub site: SiteId,
    /// Email, lower-cased.
    pub email: String,
    /// The name receipts use.
    pub name: String,
    /// Phone for order updates, if given.
    pub phone: Option<String>,
    /// The WebAuthn user handle.
    pub user_handle: Uuid,
    /// When the email was proven by a code.
    pub email_verified_at: Option<i64>,
    /// Created.
    pub created_at: i64,
}

/// A stored passkey.
#[derive(Debug, Clone)]
pub struct CredentialRow {
    /// Credential id, base64url.
    pub id: String,
    /// The passkey.
    pub passkey: webauthn_rs::prelude::Passkey,
    /// "Safari on an iPhone".
    pub label: String,
    /// Created.
    pub created_at: i64,
    /// Last used to sign in.
    pub last_used_at: Option<i64>,
}

/// A session.
#[derive(Debug, Clone)]
pub struct SessionRow {
    /// `vse_…`.
    pub id: String,
    /// The account.
    pub account: VisitorAccountId,
    /// passkey | email-code | recovery-code.
    pub method: String,
    /// Device words.
    pub device: String,
    /// Created.
    pub created_at: i64,
    /// Last request seen.
    pub last_seen_at: i64,
    /// When the person last proved who they are (fresh sign-in).
    pub authenticated_at: i64,
}

/// A sign-in event.
#[derive(Debug, Clone)]
pub struct EventRow {
    /// Id.
    pub id: String,
    /// When.
    pub at: i64,
    /// How.
    pub method: String,
    /// Device words.
    pub device: String,
    /// A device this account had not used before.
    pub new_device: bool,
    /// "It was me" pressed.
    pub dismissed_at: Option<i64>,
}

/// A consent receipt row.
#[derive(Debug, Clone)]
pub struct ConsentRow {
    /// `cr_…`.
    pub id: String,
    /// Purpose key.
    pub purpose: String,
    /// The receipt's canonical JSON.
    pub body: String,
    /// `sha256:…` of `body`.
    pub digest: String,
    /// Digest of the signed platform receipt, once issued.
    pub receipt: Option<String>,
    /// Given.
    pub given_at: i64,
    /// Withdrawn.
    pub withdrawn_at: Option<i64>,
}

/// The visitor tables.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

fn account_of(row: &sqlx::sqlite::SqliteRow) -> Result<AccountRow, StoreError> {
    let handle: String = row.try_get("user_handle")?;
    Ok(AccountRow {
        id: VisitorAccountId::new(row.try_get::<String, _>("id")?),
        site: SiteId::new(row.try_get::<String, _>("site_id")?),
        email: row.try_get("email")?,
        name: row.try_get("name")?,
        phone: row.try_get("phone")?,
        user_handle: Uuid::parse_str(&handle).map_err(|e| StoreError::Passkey(e.to_string()))?,
        email_verified_at: row.try_get("email_verified_at")?,
        created_at: row.try_get("created_at")?,
    })
}

/// SHA-256(account ‖ code), hex: how a recovery code is stored.
pub fn recovery_hash(account: &VisitorAccountId, code: &str) -> String {
    let mut h = Sha256::new();
    h.update(account.as_str().as_bytes());
    h.update(b"\0");
    h.update(code.as_bytes());
    hex::encode(h.finalize())
}

impl Store {
    /// Over a pool migrated with [`crate::migrate`].
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ---- accounts -------------------------------------------------------------------------

    /// Creates an account. `verified` when the email was just proven by a code.
    pub async fn create_account(
        &self,
        site: &SiteId,
        email: &str,
        name: &str,
        phone: Option<&str>,
        user_handle: Uuid,
        verified: bool,
    ) -> Result<AccountRow, StoreError> {
        let id = VisitorAccountId::generate();
        let at = now();
        let result = sqlx::query(
            "INSERT INTO visitor_accounts (id, site_id, email, name, phone, user_handle, email_verified_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(site.as_str())
        .bind(email)
        .bind(name)
        .bind(phone)
        .bind(user_handle.to_string())
        .bind(verified.then_some(at))
        .bind(at)
        .execute(&self.pool)
        .await;
        match result {
            Ok(_) => {}
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                return Err(StoreError::EmailTaken);
            }
            Err(e) => return Err(e.into()),
        }
        Ok(AccountRow {
            id,
            site: site.clone(),
            email: email.to_owned(),
            name: name.to_owned(),
            phone: phone.map(str::to_owned),
            user_handle,
            email_verified_at: verified.then_some(at),
            created_at: at,
        })
    }

    /// An account by email on a site.
    pub async fn account_by_email(
        &self,
        site: &SiteId,
        email: &str,
    ) -> Result<Option<AccountRow>, StoreError> {
        let row = sqlx::query("SELECT * FROM visitor_accounts WHERE site_id = ? AND email = ?")
            .bind(site.as_str())
            .bind(email)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(account_of).transpose()
    }

    /// An account by id on a site.
    pub async fn account(
        &self,
        site: &SiteId,
        id: &VisitorAccountId,
    ) -> Result<Option<AccountRow>, StoreError> {
        let row = sqlx::query("SELECT * FROM visitor_accounts WHERE site_id = ? AND id = ?")
            .bind(site.as_str())
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(account_of).transpose()
    }

    /// Marks the email proven.
    pub async fn verify_email(
        &self,
        site: &SiteId,
        id: &VisitorAccountId,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE visitor_accounts SET email_verified_at = COALESCE(email_verified_at, ?) WHERE site_id = ? AND id = ?")
            .bind(now())
            .bind(site.as_str())
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- passkeys -------------------------------------------------------------------------

    /// The account's passkeys, oldest first.
    pub async fn credentials(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<Vec<CredentialRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, passkey, label, created_at, last_used_at FROM visitor_credentials
             WHERE site_id = ? AND account_id = ? ORDER BY created_at",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let json: String = row.try_get("passkey")?;
                Ok(CredentialRow {
                    id: row.try_get("id")?,
                    passkey: serde_json::from_str(&json)
                        .map_err(|e| StoreError::Passkey(e.to_string()))?,
                    label: row.try_get("label")?,
                    created_at: row.try_get("created_at")?,
                    last_used_at: row.try_get("last_used_at")?,
                })
            })
            .collect()
    }

    /// Stores a new passkey.
    pub async fn add_credential(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        id: &str,
        passkey: &webauthn_rs::prelude::Passkey,
        label: &str,
    ) -> Result<(), StoreError> {
        let json =
            serde_json::to_string(passkey).map_err(|e| StoreError::Passkey(e.to_string()))?;
        sqlx::query(
            "INSERT INTO visitor_credentials (id, site_id, account_id, passkey, label, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(json)
        .bind(label)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Saves a passkey after a sign-in moved its counter, and stamps its use.
    pub async fn touch_credential(
        &self,
        site: &SiteId,
        id: &str,
        passkey: Option<&webauthn_rs::prelude::Passkey>,
    ) -> Result<(), StoreError> {
        match passkey {
            Some(pk) => {
                let json =
                    serde_json::to_string(pk).map_err(|e| StoreError::Passkey(e.to_string()))?;
                sqlx::query("UPDATE visitor_credentials SET passkey = ?, last_used_at = ? WHERE site_id = ? AND id = ?")
                    .bind(json)
                    .bind(now())
                    .bind(site.as_str())
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
            }
            None => {
                sqlx::query(
                    "UPDATE visitor_credentials SET last_used_at = ? WHERE site_id = ? AND id = ?",
                )
                .bind(now())
                .bind(site.as_str())
                .bind(id)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Removes a passkey. Returns whether one was removed.
    pub async fn remove_credential(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        id: &str,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "DELETE FROM visitor_credentials WHERE site_id = ? AND account_id = ? AND id = ?",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    // ---- recovery codes -------------------------------------------------------------------

    /// Replaces the account's recovery codes with `codes` (plain; stored hashed).
    pub async fn set_recovery_codes(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        codes: &[String],
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM visitor_recovery_codes WHERE site_id = ? AND account_id = ?")
            .bind(site.as_str())
            .bind(account.as_str())
            .execute(&mut *tx)
            .await?;
        let at = now();
        for code in codes {
            sqlx::query("INSERT INTO visitor_recovery_codes (account_id, code_hash, site_id, created_at) VALUES (?, ?, ?, ?)")
                .bind(account.as_str())
                .bind(recovery_hash(account, code))
                .bind(site.as_str())
                .bind(at)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Uses a recovery code: true if it was unused and is now spent.
    pub async fn use_recovery_code(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        code: &str,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "UPDATE visitor_recovery_codes SET used_at = ? WHERE site_id = ? AND account_id = ? AND code_hash = ? AND used_at IS NULL",
        )
        .bind(now())
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(recovery_hash(account, code))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Unused recovery codes left.
    pub async fn recovery_codes_left(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<usize, StoreError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM visitor_recovery_codes WHERE site_id = ? AND account_id = ? AND used_at IS NULL",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .fetch_one(&self.pool)
        .await?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    // ---- sessions -------------------------------------------------------------------------

    /// Starts a session.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_session(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        token_hash: &str,
        method: &str,
        device: &str,
        expires_at: i64,
    ) -> Result<String, StoreError> {
        let id = format!("vse_{}", Uuid::now_v7().simple());
        let at = now();
        sqlx::query(
            "INSERT INTO visitor_sessions (token_hash, id, site_id, account_id, method, device, created_at, last_seen_at, authenticated_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(token_hash)
        .bind(&id)
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(method)
        .bind(device)
        .bind(at)
        .bind(at)
        .bind(at)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// The live session for a token hash on a site; stamps `last_seen_at` at most once a minute.
    pub async fn session(
        &self,
        site: &SiteId,
        token_hash: &str,
    ) -> Result<Option<SessionRow>, StoreError> {
        let at = now();
        let row = sqlx::query(
            "SELECT id, account_id, method, device, created_at, last_seen_at, authenticated_at FROM visitor_sessions
             WHERE token_hash = ? AND site_id = ? AND ended_at IS NULL AND expires_at > ?",
        )
        .bind(token_hash)
        .bind(site.as_str())
        .bind(at)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let session = SessionRow {
            id: row.try_get("id")?,
            account: VisitorAccountId::new(row.try_get::<String, _>("account_id")?),
            method: row.try_get("method")?,
            device: row.try_get("device")?,
            created_at: row.try_get("created_at")?,
            last_seen_at: row.try_get("last_seen_at")?,
            authenticated_at: row.try_get("authenticated_at")?,
        };
        if at - session.last_seen_at >= 60 {
            sqlx::query("UPDATE visitor_sessions SET last_seen_at = ? WHERE token_hash = ?")
                .bind(at)
                .bind(token_hash)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(session))
    }

    /// Marks a session freshly authenticated (a new passkey assertion or code).
    pub async fn refresh_authentication(
        &self,
        site: &SiteId,
        token_hash: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE visitor_sessions SET authenticated_at = ? WHERE token_hash = ? AND site_id = ?",
        )
        .bind(now())
        .bind(token_hash)
        .bind(site.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The account's live sessions, newest first.
    pub async fn sessions(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<Vec<SessionRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, account_id, method, device, created_at, last_seen_at, authenticated_at FROM visitor_sessions
             WHERE site_id = ? AND account_id = ? AND ended_at IS NULL AND expires_at > ? ORDER BY last_seen_at DESC",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(now())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SessionRow {
                    id: row.try_get("id")?,
                    account: VisitorAccountId::new(row.try_get::<String, _>("account_id")?),
                    method: row.try_get("method")?,
                    device: row.try_get("device")?,
                    created_at: row.try_get("created_at")?,
                    last_seen_at: row.try_get("last_seen_at")?,
                    authenticated_at: row.try_get("authenticated_at")?,
                })
            })
            .collect()
    }

    /// Ends one session of the account (by its public id). Returns whether one ended.
    pub async fn end_session(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        id: &str,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "UPDATE visitor_sessions SET ended_at = ? WHERE site_id = ? AND account_id = ? AND id = ? AND ended_at IS NULL",
        )
        .bind(now())
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Ends the session behind a token hash.
    pub async fn end_session_by_token(
        &self,
        site: &SiteId,
        token_hash: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE visitor_sessions SET ended_at = ? WHERE site_id = ? AND token_hash = ? AND ended_at IS NULL")
            .bind(now())
            .bind(site.as_str())
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends every session of the account. Returns how many ended.
    pub async fn end_all_sessions(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<u64, StoreError> {
        let done = sqlx::query("UPDATE visitor_sessions SET ended_at = ? WHERE site_id = ? AND account_id = ? AND ended_at IS NULL")
            .bind(now())
            .bind(site.as_str())
            .bind(account.as_str())
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }

    // ---- sign-in events -------------------------------------------------------------------

    /// Records a sign-in; `new_device` when the account never signed in on this device before.
    pub async fn record_sign_in(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        method: &str,
        device: &str,
    ) -> Result<bool, StoreError> {
        let (seen,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM visitor_sign_in_events WHERE site_id = ? AND account_id = ? AND device = ?",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(device)
        .fetch_one(&self.pool)
        .await?;
        let (any,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM visitor_sign_in_events WHERE site_id = ? AND account_id = ?",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .fetch_one(&self.pool)
        .await?;
        // The first sign-in ever is the device that made the account: not "new".
        let new_device = seen == 0 && any > 0;
        sqlx::query(
            "INSERT INTO visitor_sign_in_events (id, site_id, account_id, at, method, device, new_device) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("vev_{}", Uuid::now_v7().simple()))
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(now())
        .bind(method)
        .bind(device)
        .bind(i64::from(new_device))
        .execute(&self.pool)
        .await?;
        Ok(new_device)
    }

    /// The account's sign-ins in the last 13 months, newest first, at most `limit`.
    pub async fn sign_ins(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        limit: i64,
    ) -> Result<Vec<EventRow>, StoreError> {
        let since = now() - 395 * 24 * 3600;
        let rows = sqlx::query(
            "SELECT id, at, method, device, new_device, dismissed_at FROM visitor_sign_in_events
             WHERE site_id = ? AND account_id = ? AND at > ? ORDER BY at DESC LIMIT ?",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(EventRow {
                    id: row.try_get("id")?,
                    at: row.try_get("at")?,
                    method: row.try_get("method")?,
                    device: row.try_get("device")?,
                    new_device: row.try_get::<i64, _>("new_device")? != 0,
                    dismissed_at: row.try_get("dismissed_at")?,
                })
            })
            .collect()
    }

    /// "It was me": dismisses every new-device alert of the account.
    pub async fn dismiss_alerts(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE visitor_sign_in_events SET dismissed_at = ? WHERE site_id = ? AND account_id = ? AND new_device = 1 AND dismissed_at IS NULL",
        )
        .bind(now())
        .bind(site.as_str())
        .bind(account.as_str())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---- consent receipts -----------------------------------------------------------------

    /// Stores a receipt that was just given.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_consent(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        id: &str,
        purpose: &str,
        body: &str,
        digest: &str,
        receipt: Option<&str>,
        given_at: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO consent_receipts (id, site_id, account_id, purpose, body, digest, receipt, given_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(purpose)
        .bind(body)
        .bind(digest)
        .bind(receipt)
        .bind(given_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Withdraws the live receipt for a purpose, recording the `consent.withdrawn` receipt's
    /// digest. Returns the withdrawn row's id, if one was live.
    pub async fn withdraw_consent(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        purpose: &str,
        receipt: Option<&str>,
    ) -> Result<Option<String>, StoreError> {
        let live: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM consent_receipts WHERE site_id = ? AND account_id = ? AND purpose = ? AND withdrawn_at IS NULL",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(purpose)
        .fetch_optional(&self.pool)
        .await?;
        let Some((id,)) = live else { return Ok(None) };
        sqlx::query("UPDATE consent_receipts SET withdrawn_at = ?, withdrawn_receipt = ? WHERE id = ? AND withdrawn_at IS NULL")
            .bind(now())
            .bind(receipt)
            .bind(&id)
            .execute(&self.pool)
            .await?;
        Ok(Some(id))
    }

    /// The live receipt's id for a purpose, if any.
    pub async fn live_consent(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        purpose: &str,
    ) -> Result<Option<String>, StoreError> {
        let live: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM consent_receipts WHERE site_id = ? AND account_id = ? AND purpose = ? AND withdrawn_at IS NULL",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .bind(purpose)
        .fetch_optional(&self.pool)
        .await?;
        Ok(live.map(|(id,)| id))
    }

    /// Every receipt of the account, newest first.
    pub async fn consents(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<Vec<ConsentRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, purpose, body, digest, receipt, given_at, withdrawn_at FROM consent_receipts
             WHERE site_id = ? AND account_id = ? ORDER BY given_at DESC, id DESC",
        )
        .bind(site.as_str())
        .bind(account.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ConsentRow {
                    id: row.try_get("id")?,
                    purpose: row.try_get("purpose")?,
                    body: row.try_get("body")?,
                    digest: row.try_get("digest")?,
                    receipt: row.try_get("receipt")?,
                    given_at: row.try_get("given_at")?,
                    withdrawn_at: row.try_get("withdrawn_at")?,
                })
            })
            .collect()
    }
}
