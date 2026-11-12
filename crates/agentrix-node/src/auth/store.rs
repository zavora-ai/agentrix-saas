//! The sign-in tables (`schema.sql`) over SQLite. This module is their only writer (design §5).

use sqlx::SqlitePool;
use uuid::Uuid;

/// Unix seconds now.
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// An account as the auth module sees it.
#[derive(Debug, Clone)]
pub struct Account {
    /// Account id (UUID v7).
    pub id: Uuid,
    /// Sign-in email, unique, compared without case.
    pub email: String,
    /// The name the console greets.
    pub display_name: String,
    /// argon2id PHC string, if the account has a password.
    pub password_hash: Option<String>,
}

/// A stored passkey row.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    /// The credential id, base64url.
    pub id: String,
    /// webauthn-rs `Passkey` as JSON.
    pub passkey_json: String,
}

/// A live session.
#[derive(Debug, Clone)]
pub struct SessionRow {
    /// Whose session.
    pub account: Uuid,
    /// `passkey` or `password+totp`.
    pub method: String,
    /// Unix seconds.
    pub expires_at: i64,
}

/// Store errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The email already has an account.
    #[error("an account with this email exists")]
    EmailTaken,
    /// A stored value could not be read back.
    #[error("stored data is unreadable: {0}")]
    Corrupt(String),
    /// SQLite.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn parse_id(raw: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(raw).map_err(|e| StoreError::Corrupt(format!("account id {raw}: {e}")))
}

fn map_unique(error: sqlx::Error) -> StoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => StoreError::EmailTaken,
        _ => StoreError::Db(error),
    }
}

/// What a new account starts with. The account, its first credential and its TOTP seed are
/// written in one transaction, so an abandoned sign-up leaves nothing behind.
#[derive(Debug)]
pub struct NewAccount<'a> {
    /// Sign-in email.
    pub email: &'a str,
    /// Display name.
    pub display_name: &'a str,
    /// Password hash, for password + TOTP accounts.
    pub password_hash: Option<&'a str>,
    /// First passkey `(credential id, Passkey JSON, label)`, for passkey accounts.
    pub passkey: Option<(&'a str, &'a str, &'a str)>,
    /// Sealed TOTP seed and the step its confirming code used.
    pub totp: Option<(&'a [u8], i64)>,
}

/// SQLite access for the auth module.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Wraps a pool whose schema includes `schema.sql`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Applies `schema.sql`. For tests and for running before the data lane's migrator
    /// includes it; every statement is `IF NOT EXISTS`.
    pub async fn ensure_schema(&self) -> Result<(), StoreError> {
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Creates account `id` with its first way to sign in. For passkey sign-ups `id` is the
    /// passkey's user handle, chosen when the ceremony started.
    pub async fn create_account(&self, id: Uuid, new: NewAccount<'_>) -> Result<Uuid, StoreError> {
        let at = now();
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO accounts (id, email, display_name, password_hash, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id.to_string())
            .bind(new.email)
            .bind(new.display_name)
            .bind(new.password_hash)
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(map_unique)?;
        if let Some((cred_id, passkey, label)) = new.passkey {
            sqlx::query("INSERT INTO credentials (id, account_id, passkey, label, created_at) VALUES (?, ?, ?, ?, ?)")
                .bind(cred_id)
                .bind(id.to_string())
                .bind(passkey)
                .bind(label)
                .bind(at)
                .execute(&mut *tx)
                .await?;
        }
        if let Some((sealed, step)) = new.totp {
            sqlx::query("INSERT INTO totp_seeds (account_id, sealed_seed, last_step, created_at) VALUES (?, ?, ?, ?)")
                .bind(id.to_string())
                .bind(sealed)
                .bind(step)
                .bind(at)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(id)
    }

    /// The account with this email, if any.
    pub async fn account_by_email(&self, email: &str) -> Result<Option<Account>, StoreError> {
        let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, email, display_name, password_hash FROM accounts WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, email, display_name, password_hash)| {
            Ok(Account {
                id: parse_id(&id)?,
                email,
                display_name,
                password_hash,
            })
        })
        .transpose()
    }

    /// The account with this id, if any.
    pub async fn account(&self, id: Uuid) -> Result<Option<Account>, StoreError> {
        let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, email, display_name, password_hash FROM accounts WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, email, display_name, password_hash)| {
            Ok(Account {
                id: parse_id(&id)?,
                email,
                display_name,
                password_hash,
            })
        })
        .transpose()
    }

    /// Adds a passkey to an existing account.
    pub async fn add_credential(
        &self,
        account: Uuid,
        cred_id: &str,
        passkey_json: &str,
        label: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO credentials (id, account_id, passkey, label, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(cred_id)
            .bind(account.to_string())
            .bind(passkey_json)
            .bind(label)
            .bind(now())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The account's passkeys.
    pub async fn credentials(&self, account: Uuid) -> Result<Vec<StoredCredential>, StoreError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, passkey FROM credentials WHERE account_id = ? ORDER BY created_at",
        )
        .bind(account.to_string())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, passkey_json)| StoredCredential { id, passkey_json })
            .collect())
    }

    /// Saves a passkey after a sign-in moved its counter, and stamps its last use.
    pub async fn touch_credential(
        &self,
        cred_id: &str,
        passkey_json: Option<&str>,
    ) -> Result<(), StoreError> {
        match passkey_json {
            Some(json) => {
                sqlx::query("UPDATE credentials SET passkey = ?, last_used_at = ? WHERE id = ?")
                    .bind(json)
            }
            None => sqlx::query("UPDATE credentials SET last_used_at = ? WHERE id = ?"),
        }
        .bind(now())
        .bind(cred_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The account's sealed TOTP seed and the last step it accepted.
    pub async fn totp_seed(&self, account: Uuid) -> Result<Option<(Vec<u8>, i64)>, StoreError> {
        Ok(
            sqlx::query_as("SELECT sealed_seed, last_step FROM totp_seeds WHERE account_id = ?")
                .bind(account.to_string())
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Records `step` as used, only if it is later than the last one. `false` means another
    /// request used this step first, so the code must be refused.
    pub async fn advance_totp(&self, account: Uuid, step: i64) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "UPDATE totp_seeds SET last_step = ? WHERE account_id = ? AND last_step < ?",
        )
        .bind(step)
        .bind(account.to_string())
        .bind(step)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Stores a session by the hash of its token.
    pub async fn create_session(
        &self,
        token_hash: &str,
        account: Uuid,
        method: &str,
        expires_at: i64,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO sessions (token_hash, account_id, method, created_at, expires_at) VALUES (?, ?, ?, ?, ?)")
            .bind(token_hash)
            .bind(account.to_string())
            .bind(method)
            .bind(now())
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The live session for a token hash; expired sessions are deleted and not returned.
    pub async fn session(&self, token_hash: &str) -> Result<Option<SessionRow>, StoreError> {
        let row: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT account_id, method, expires_at FROM sessions WHERE token_hash = ?",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some((account, method, expires_at)) = row else {
            return Ok(None);
        };
        if expires_at <= now() {
            self.delete_session(token_hash).await?;
            return Ok(None);
        }
        Ok(Some(SessionRow {
            account: parse_id(&account)?,
            method,
            expires_at,
        }))
    }

    /// Ends a session.
    pub async fn delete_session(&self, token_hash: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
