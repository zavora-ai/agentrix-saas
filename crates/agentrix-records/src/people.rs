//! Customers, and the small sign-in the samples use until W6's visitor accounts are wired:
//! an emailed one-time code and a session cookie, per site.

use serde::Serialize;

use crate::{Records, new_id, sha256_hex};

/// A customer of a site.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Customer {
    /// Id: W6's visitor account id once wired (`vac_…`), else `cus-…`.
    pub customer_id: String,
    /// Name.
    pub name: String,
    /// Email.
    pub email: String,
    /// Phone.
    pub phone: Option<String>,
    /// `person` or `business`.
    pub kind: String,
    /// Where they are, roughly ("London").
    pub place: Option<String>,
    /// A sample row from the bundle.
    pub sample: i64,
    /// When.
    pub created_at: i64,
}

const COLS: &str = "customer_id, name, email, phone, kind, place, sample, created_at";

impl Records {
    /// A customer by id.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn customer(&self, site: &str, id: &str) -> anyhow::Result<Option<Customer>> {
        Ok(sqlx::query_as::<_, Customer>(sql!(
            "SELECT {COLS} FROM rec_customers WHERE site = ? AND customer_id = ?"
        ))
        .bind(site)
        .bind(id)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Deletes a customer's record (name, email, phone) when they delete their account; false
    /// when there was none. Their orders stay, as the business's sales record.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn forget_customer(&self, site: &str, id: &str) -> anyhow::Result<bool> {
        let done = sqlx::query("DELETE FROM rec_customers WHERE site = ? AND customer_id = ?")
            .bind(site)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// A customer by email.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn customer_by_email(
        &self,
        site: &str,
        email: &str,
    ) -> anyhow::Result<Option<Customer>> {
        Ok(sqlx::query_as::<_, Customer>(sql!(
            "SELECT {COLS} FROM rec_customers WHERE site = ? AND email = ?"
        ))
        .bind(site)
        .bind(email.trim().to_lowercase())
        .fetch_optional(self.pool())
        .await?)
    }

    /// Customers, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn customers(&self, site: &str) -> anyhow::Result<Vec<Customer>> {
        Ok(sqlx::query_as::<_, Customer>(sql!(
            "SELECT {COLS} FROM rec_customers WHERE site = ? ORDER BY created_at DESC"
        ))
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// Creates or updates a customer row under a given id (W6's visitor account id).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn upsert_customer(
        &self,
        site: &str,
        id: &str,
        name: &str,
        email: &str,
        phone: Option<&str>,
        now: i64,
    ) -> anyhow::Result<Customer> {
        sqlx::query(
            "INSERT INTO rec_customers (site, customer_id, name, email, phone, kind, place, sample, created_at)
             VALUES (?, ?, ?, ?, ?, 'person', NULL, 0, ?)
             ON CONFLICT (site, customer_id) DO UPDATE SET name = excluded.name, email = excluded.email, phone = COALESCE(excluded.phone, rec_customers.phone)",
        )
        .bind(site)
        .bind(id)
        .bind(name.trim())
        .bind(email.trim().to_lowercase())
        .bind(phone)
        .bind(now)
        .execute(self.pool())
        .await?;
        self.customer(site, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("customer vanished"))
    }

    /// Issues a 6-digit code for an email and purpose; returns it (the caller emails it).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn issue_code(
        &self,
        site: &str,
        email: &str,
        purpose: &str,
        now: i64,
    ) -> anyhow::Result<String> {
        let n: u32 = rand::random::<u32>() % 1_000_000;
        let code = format!("{n:06}");
        sqlx::query(
            "INSERT INTO rec_codes (site, email, purpose, code_hash, expires_at, attempts, created_at) VALUES (?, ?, ?, ?, ?, 0, ?)
             ON CONFLICT (site, email, purpose) DO UPDATE SET code_hash = excluded.code_hash, expires_at = excluded.expires_at, attempts = 0, created_at = excluded.created_at",
        )
        .bind(site)
        .bind(email.trim().to_lowercase())
        .bind(purpose)
        .bind(sha256_hex(&format!("{site}:{code}")))
        .bind(now + 600)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(code)
    }

    /// Checks a code. Five wrong tries or ten minutes end it.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn check_code(
        &self,
        site: &str,
        email: &str,
        purpose: &str,
        code: &str,
        now: i64,
    ) -> anyhow::Result<CodeCheck> {
        let email = email.trim().to_lowercase();
        let row: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT code_hash, expires_at, attempts FROM rec_codes WHERE site = ? AND email = ? AND purpose = ?",
        )
        .bind(site)
        .bind(&email)
        .bind(purpose)
        .fetch_optional(self.pool())
        .await?;
        let Some((hash, expires_at, attempts)) = row else {
            return Ok(CodeCheck::Expired);
        };
        if now > expires_at || attempts >= 5 {
            return Ok(CodeCheck::Expired);
        }
        if hash == sha256_hex(&format!("{site}:{}", code.trim())) {
            sqlx::query("DELETE FROM rec_codes WHERE site = ? AND email = ? AND purpose = ?")
                .bind(site)
                .bind(&email)
                .bind(purpose)
                .execute(self.pool())
                .await?;
            Ok(CodeCheck::Ok)
        } else {
            sqlx::query("UPDATE rec_codes SET attempts = attempts + 1 WHERE site = ? AND email = ? AND purpose = ?")
                .bind(site)
                .bind(&email)
                .bind(purpose)
                .execute(self.pool())
                .await?;
            Ok(CodeCheck::Wrong { left: 4 - attempts })
        }
    }

    /// Starts a session for a customer; returns the cookie token.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn start_session(
        &self,
        site: &str,
        customer_id: &str,
        device: Option<&str>,
        now: i64,
    ) -> anyhow::Result<String> {
        let token = format!(
            "{}{}",
            hex::encode(rand::random::<[u8; 16]>()),
            hex::encode(rand::random::<[u8; 16]>())
        );
        sqlx::query("INSERT INTO rec_sessions (site, token_hash, customer_id, device, created_at, last_seen) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(site)
            .bind(sha256_hex(&token))
            .bind(customer_id)
            .bind(device)
            .bind(now)
            .bind(now)
            .execute(self.pool())
            .await?;
        Ok(token)
    }

    /// The customer a session token belongs to, if it is live (30 days idle ends it).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn session_customer(
        &self,
        site: &str,
        token: &str,
        now: i64,
    ) -> anyhow::Result<Option<Customer>> {
        let hash = sha256_hex(token);
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT customer_id FROM rec_sessions WHERE site = ? AND token_hash = ? AND ended_at IS NULL AND last_seen > ?",
        )
        .bind(site)
        .bind(&hash)
        .bind(now - 30 * 86_400)
        .fetch_optional(self.pool())
        .await?;
        let Some((id,)) = row else { return Ok(None) };
        sqlx::query("UPDATE rec_sessions SET last_seen = ? WHERE site = ? AND token_hash = ?")
            .bind(now)
            .bind(site)
            .bind(&hash)
            .execute(self.pool())
            .await?;
        self.customer(site, &id).await
    }

    /// Who holds a live session token (the id it was started for), without a customer row:
    /// the samples' owner sign-in uses it.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn session_holder(
        &self,
        site: &str,
        token: &str,
        now: i64,
    ) -> anyhow::Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT customer_id FROM rec_sessions WHERE site = ? AND token_hash = ? AND ended_at IS NULL AND last_seen > ?",
        )
        .bind(site)
        .bind(sha256_hex(token))
        .bind(now - 7 * 86_400)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Ends a session.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn end_session(&self, site: &str, token: &str, now: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE rec_sessions SET ended_at = ? WHERE site = ? AND token_hash = ?")
            .bind(now)
            .bind(site)
            .bind(sha256_hex(token))
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// A new customer id for the samples' own sign-in.
    pub fn new_customer_id() -> String {
        new_id("cus")
    }
}

/// The result of checking a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeCheck {
    /// It matched.
    Ok,
    /// It didn't; tries left.
    Wrong {
        /// Tries left.
        left: i64,
    },
    /// No live code: expired, used or too many tries.
    Expired,
}
