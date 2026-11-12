//! A customer's own export and erasure (A-11 to A-13, ADR-028), on adk-core's subject-rights
//! framework. This crate's tables answer through [`VisitorSubjectData`], an
//! `adk_core::subject::SubjectDataStore`; other workstreams register their own stores (W2's
//! orders and customers, W1's account memory) with [`Visitors::with_subject_store`], and one
//! request reaches every store. Each store answers with a line on the erasure receipt.
//!
//! The scope is `app_names = [site id]`, `subject = SubjectRef::human(<vac_… account id>)`.
//! A store that must refuse (W2: "this customer has an open order") returns an error whose
//! code is [`OPEN_ORDERS_CODE`]; nothing is erased then.

use adk_core::principal::SubjectRef;
use adk_core::subject::{StoreErasure, StoreOutcome, StoreReport, SubjectDataStore, SubjectScope};
use adk_core::{AdkError, ErrorComponent};
use agentrix_types::SiteId;
use agentrix_types::tenancy::VisitorAccountId;
use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::Row;

use super::store::Store;
use crate::now;

/// The error code a store returns from `erase` to refuse because the customer has an open
/// order (A-12: "You have no open orders" is the condition).
pub const OPEN_ORDERS_CODE: &str = "subject.open_orders";

/// The scope for one customer on one site.
pub fn scope(site: &SiteId, account: &VisitorAccountId) -> SubjectScope {
    SubjectScope::new([site.as_str()], SubjectRef::human(account.as_str()))
}

fn err(message: impl Into<String>) -> AdkError {
    AdkError::internal(ErrorComponent::Auth, "identity.subject", message)
}

/// This crate's visitor tables as a subject-data store: the account, passkeys (names and
/// dates only), sessions, sign-in history, recovery codes (count only) and consent receipts.
#[derive(Debug, Clone)]
pub struct VisitorSubjectData {
    store: Store,
}

impl VisitorSubjectData {
    /// Over the visitor store.
    pub fn new(store: Store) -> Self {
        Self { store }
    }
}

#[async_trait]
impl SubjectDataStore for VisitorSubjectData {
    fn name(&self) -> &str {
        "identity.accounts"
    }

    async fn export(&self, scope: &SubjectScope) -> adk_core::Result<Value> {
        let account = VisitorAccountId::new(scope.subject.id.clone());
        let mut sites = Vec::new();
        for site in &scope.app_names {
            let site = SiteId::new(site.clone());
            let Some(row) = self
                .store
                .account(&site, &account)
                .await
                .map_err(|e| err(e.to_string()))?
            else {
                continue;
            };
            let passkeys: Vec<Value> = self
                .store
                .credentials(&site, &account)
                .await
                .map_err(|e| err(e.to_string()))?
                .into_iter()
                .map(|c| json!({ "label": c.label, "created_at": c.created_at, "last_used_at": c.last_used_at }))
                .collect();
            let sessions: Vec<Value> = self
                .store
                .sessions(&site, &account)
                .await
                .map_err(|e| err(e.to_string()))?
                .into_iter()
                .map(|s| json!({ "device": s.device, "method": s.method, "created_at": s.created_at, "last_seen_at": s.last_seen_at }))
                .collect();
            let sign_ins: Vec<Value> = self
                .store
                .sign_ins(&site, &account, 1000)
                .await
                .map_err(|e| err(e.to_string()))?
                .into_iter()
                .map(|e| json!({ "at": e.at, "method": e.method, "device": e.device, "new_device": e.new_device }))
                .collect();
            let consents: Vec<Value> = self
                .store
                .consents(&site, &account)
                .await
                .map_err(|e| err(e.to_string()))?
                .into_iter()
                .map(|c| {
                    json!({
                        "receipt": serde_json::from_str::<Value>(&c.body).unwrap_or(Value::Null), "digest": c.digest,
                        "signed_receipt": c.receipt, "given_at": c.given_at, "withdrawn_at": c.withdrawn_at,
                    })
                })
                .collect();
            let codes_left = self
                .store
                .recovery_codes_left(&site, &account)
                .await
                .map_err(|e| err(e.to_string()))?;
            sites.push(json!({
                "site": site,
                "account": { "id": row.id, "name": row.name, "email": row.email, "phone": row.phone, "email_verified_at": row.email_verified_at, "created_at": row.created_at },
                "passkeys": passkeys, "sessions": sessions, "sign_ins": sign_ins, "consents": consents,
                "recovery_codes_left": codes_left,
            }));
        }
        Ok(if sites.is_empty() {
            Value::Array(Vec::new())
        } else {
            Value::Array(sites)
        })
    }

    async fn erase(&self, scope: &SubjectScope) -> adk_core::Result<StoreErasure> {
        let mut deleted = 0;
        for site in &scope.app_names {
            deleted += self
                .store
                .erase_account(
                    &SiteId::new(site.clone()),
                    &VisitorAccountId::new(scope.subject.id.clone()),
                )
                .await
                .map_err(|e| err(e.to_string()))?;
        }
        Ok(StoreErasure::deleted(deleted))
    }
}

impl Store {
    /// Deletes every row about an account on a site (no reliance on cascades, which SQLite
    /// runs only with foreign keys on). Returns how many rows went.
    pub async fn erase_account(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
    ) -> Result<u64, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        let mut deleted = 0;
        for statement in [
            "DELETE FROM consent_receipts WHERE site_id = ? AND account_id = ?",
            "DELETE FROM visitor_sign_in_events WHERE site_id = ? AND account_id = ?",
            "DELETE FROM visitor_sessions WHERE site_id = ? AND account_id = ?",
            "DELETE FROM visitor_recovery_codes WHERE site_id = ? AND account_id = ?",
            "DELETE FROM visitor_credentials WHERE site_id = ? AND account_id = ?",
        ] {
            let done = sqlx::query(statement)
                .bind(site.as_str())
                .bind(account.as_str())
                .execute(&mut *tx)
                .await?;
            deleted += done.rows_affected();
        }
        let done = sqlx::query("DELETE FROM visitor_accounts WHERE site_id = ? AND id = ?")
            .bind(site.as_str())
            .bind(account.as_str())
            .execute(&mut *tx)
            .await?;
        deleted += done.rows_affected();
        tx.commit().await?;
        Ok(deleted)
    }

    /// Records a finished erasure's report, for the receipt page after sign-out.
    pub async fn record_erasure(
        &self,
        site: &SiteId,
        subject_hash: &str,
        report: &Value,
        digest: &str,
    ) -> Result<String, sqlx::Error> {
        let id = format!("er_{}", uuid::Uuid::now_v7().simple());
        sqlx::query("INSERT INTO visitor_erasures (id, site_id, subject_hash, report, digest, completed_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(site.as_str())
            .bind(subject_hash)
            .bind(report.to_string())
            .bind(digest)
            .bind(now())
            .execute(self.pool())
            .await?;
        Ok(id)
    }

    /// An erasure report by id on a site: `(report, digest, completed_at)`.
    pub async fn erasure(
        &self,
        site: &SiteId,
        id: &str,
    ) -> Result<Option<(Value, String, i64)>, sqlx::Error> {
        let row = sqlx::query("SELECT report, digest, completed_at FROM visitor_erasures WHERE site_id = ? AND id = ?")
            .bind(site.as_str())
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.map(|r| {
            let report: String = r.get("report");
            (
                serde_json::from_str(&report).unwrap_or(Value::Null),
                r.get("digest"),
                r.get("completed_at"),
            )
        }))
    }
}

/// A store's name as a customer reads it on the receipt. Other workstreams name their stores
/// in words already ("Orders and receipts").
fn store_words(name: &str) -> String {
    match name {
        "identity.accounts" => "Account, sign-in and choices".to_owned(),
        other => other.to_owned(),
    }
}

/// The words for one store's line on the receipt.
pub fn line(report: &StoreReport) -> Value {
    let (state, ok) = match &report.outcome {
        StoreOutcome::Erased { deleted, retained } if *retained > 0 => (
            format!("Erased {deleted}, kept {retained} as the law requires"),
            true,
        ),
        StoreOutcome::Erased { deleted, .. } if *deleted == 0 => {
            ("Nothing to erase".to_owned(), true)
        }
        StoreOutcome::Erased { deleted, .. } => (format!("Erased ({deleted})"), true),
        StoreOutcome::Scheduled { completes_by, .. } => (
            format!(
                "Scheduled, done by {}",
                completes_by.format("%a %-d %b, %H:%M UTC")
            ),
            true,
        ),
        StoreOutcome::Unsupported => (
            "Kept for now: erasing one person's data here isn't built yet".to_owned(),
            false,
        ),
        StoreOutcome::Unreachable => ("Outside this site: the owner was told".to_owned(), false),
        StoreOutcome::Failed { message, .. } => (format!("Not erased yet: {message}"), false),
        _ => ("Answered".to_owned(), true),
    };
    json!({ "store": store_words(&report.store), "state": state, "ok": ok })
}
