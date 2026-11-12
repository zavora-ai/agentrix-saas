//! The gauntlet's node side (design §4.1: `GET /receipts?…`, `POST /gauntlet/runs`; R13.2).
//!
//! `agentrix_gauntlet::server` owns the routes and recomputes every verdict from verified
//! receipts; this module gives it the stores. A request is allowed for a site when it carries
//! that site's owner session, or the gauntlet token in `Authorization: Bearer`, compared in
//! constant time and never logged.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::{HeaderMap, header};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use subtle::ConstantTimeEq as _;

use agentrix_gauntlet::server::{Listed, RunBackend};
use agentrix_gauntlet::{PostedRun, Score};
use agentrix_types::{Digest, KeySet, Receipt, ReceiptBody, ReceiptKind, SiteId, Subject};

use crate::auth::AuthState;
use crate::db::receipts::ReceiptStore;

/// The gauntlet backend over node's stores.
pub struct GauntletBackend {
    /// The data lane's receipts store.
    pub receipts: Arc<ReceiptStore>,
    /// The control database (sites, gauntlet_runs).
    pub pool: SqlitePool,
    /// Sign-in, for the owner's session.
    pub auth: AuthState,
    /// The gauntlet token, if one is configured.
    pub token: Option<String>,
}

impl GauntletBackend {
    /// Reads the token from `path` (one line, hex); a missing file means only owner sessions
    /// may use the routes.
    pub fn read_token(path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        let token = text.trim().to_string();
        (!token.is_empty()).then_some(token)
    }

    fn bearer_matches(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = self.token.as_deref() else {
            return false;
        };
        let Some(given) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return false;
        };
        given.len() == expected.len() && bool::from(given.as_bytes().ct_eq(expected.as_bytes()))
    }

    async fn owns(&self, headers: &HeaderMap, site: &str) -> anyhow::Result<bool> {
        let Some(session) = self.auth.session_from(headers).await? else {
            return Ok(false);
        };
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?")
                .bind(site)
                .fetch_optional(&self.pool)
                .await?;
        Ok(matches!(row, Some((Some(owner),)) if owner == session.account.to_string()))
    }
}

fn parse_time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[async_trait]
impl RunBackend for GauntletBackend {
    async fn authorized(&self, headers: &HeaderMap, site: &str) -> bool {
        if self.bearer_matches(headers) {
            return true;
        }
        match self.owns(headers, site).await {
            Ok(owner) => owner,
            Err(e) => {
                tracing::error!(error = %e, "gauntlet authorisation check failed");
                false
            }
        }
    }

    fn keys(&self) -> KeySet {
        self.receipts.keys().clone()
    }

    async fn receipt(&self, digest: &Digest) -> anyhow::Result<Option<Receipt>> {
        self.receipts.get(digest).await
    }

    async fn list(
        &self,
        site: &str,
        since: Option<&str>,
        kind: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<Listed>> {
        let site = SiteId::new(site);
        let kind = kind.and_then(ReceiptKind::parse);
        let stored = match since {
            Some(since) => {
                let mut rows = self
                    .receipts
                    .list_since(&site, parse_time(since), limit)
                    .await?;
                if let Some(kind) = kind {
                    rows.retain(|s| s.receipt.body.kind == kind);
                }
                rows
            }
            None => self.receipts.list(&site, kind, limit).await?,
        };
        Ok(stored
            .into_iter()
            .map(|s| Listed {
                kind: s.receipt.body.kind.as_str().to_string(),
                ended_at: s.receipt.body.ended_at.to_rfc3339(),
                digest: s.digest,
            })
            .collect())
    }

    async fn record(&self, site: &str, run: &PostedRun, score: &Score) -> anyhow::Result<Digest> {
        let site_id = SiteId::new(site);
        let started = parse_time(&run.started_at);
        let ended = parse_time(&run.ended_at);
        let mut body = ReceiptBody::new(
            ReceiptKind::GauntletRun,
            site_id.clone(),
            Subject::Site(site_id),
            started,
            ended,
        );
        body.detail = Some(score.receipt_detail());
        let stored = self.receipts.issue(body).await?;
        let run_id = format!("gr_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, receipt, detail, started_at, ended_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&run_id)
        .bind(site)
        .bind(i64::from(score.score))
        .bind(i64::from(score.out_of))
        .bind(stored.digest.to_string())
        .bind(serde_json::to_string(&score.attacks_detail())?)
        .bind(crate::db::time(started))
        .bind(crate::db::time(ended))
        .execute(&self.pool)
        .await?;
        Ok(stored.digest)
    }
}
