//! The gauntlet's two platform routes (design §4.1, approved 7 Nov in 80a7230), for
//! `agentrix-node` to merge. Feature `server`.
//!
//! - `GET /receipts?site=&since=&kind=` with `Accept: application/json`: the site's receipts,
//!   newest first, as `[{digest, kind, ended_at}]` (at most [`LIST_MAX`]).
//! - `POST /gauntlet/runs` with a [`PostedRun`]: the node loads every cited receipt from its own
//!   store, keeps those that verify against its own keys and belong to the site, scores them
//!   with [`score`], and records the run (the `gauntlet.run` receipt and the `gauntlet_runs`
//!   row). The posted body carries no verdict and no score, so nothing is typed in (R13.2, N8).
//!
//! Both need the owner's session or the gauntlet bearer token. [`RunBackend::authorized`]
//! decides; the node implements it with its session and a token kept only on control.

use std::sync::Arc;

use agentrix_types::{Digest, KeySet, Receipt};
use async_trait::async_trait;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{PostedRun, Score, manifest, score};

/// The most receipts one listing returns.
pub const LIST_MAX: u32 = 500;

/// The most receipts one posted run may cite.
pub const CITE_MAX: usize = 200;

/// One row of the receipt listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    /// The receipt's content address.
    pub digest: Digest,
    /// Its kind, for example `job.fetch`.
    pub kind: String,
    /// When the work it records ended (RFC 3339).
    pub ended_at: String,
}

/// What the routes need from the node.
#[async_trait]
pub trait RunBackend: Send + Sync + 'static {
    /// Whether this request may list `site`'s receipts and record its runs: the owner's
    /// session, or `Authorization: Bearer <gauntlet token>` compared in constant time.
    async fn authorized(&self, headers: &HeaderMap, site: &str) -> bool;

    /// The published key set (`/.well-known/agentrix/keys`).
    fn keys(&self) -> KeySet;

    /// A stored receipt by digest.
    async fn receipt(&self, digest: &Digest) -> anyhow::Result<Option<Receipt>>;

    /// `site`'s receipts that ended at or after `since` (RFC 3339), newest first, optionally of
    /// one kind, at most `limit`.
    async fn list(
        &self,
        site: &str,
        since: Option<&str>,
        kind: Option<&str>,
        limit: u32,
    ) -> anyhow::Result<Vec<Listed>>;

    /// Records a scored run: issues the `gauntlet.run` receipt (subject: the site; detail:
    /// [`Score::receipt_detail`]) and writes the `gauntlet_runs` row (detail:
    /// [`Score::attacks_detail`]). Returns the receipt's digest.
    async fn record(&self, site: &str, run: &PostedRun, score: &Score) -> anyhow::Result<Digest>;
}

/// The routes, for the node to merge.
pub fn router(backend: Arc<dyn RunBackend>) -> Router {
    Router::new()
        .route("/receipts", get(list_receipts))
        .route("/gauntlet/runs", post(post_run))
        .with_state(backend)
}

/// The listing's query.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// The site.
    pub site: String,
    /// Only receipts that ended at or after this time (RFC 3339).
    #[serde(default)]
    pub since: Option<String>,
    /// Only receipts of this kind.
    #[serde(default)]
    pub kind: Option<String>,
}

fn plain(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

async fn list_receipts(
    State(backend): State<Arc<dyn RunBackend>>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    if !backend.authorized(&headers, &q.site).await {
        return plain(
            StatusCode::UNAUTHORIZED,
            "Sign in as the owner, or send the gauntlet token.",
        );
    }
    if let Some(since) = &q.since
        && chrono_like(since).is_none()
    {
        return plain(StatusCode::BAD_REQUEST, "since must be an RFC 3339 time.");
    }
    match backend
        .list(&q.site, q.since.as_deref(), q.kind.as_deref(), LIST_MAX)
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => {
            tracing_error(&e);
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The receipts could not be listed.",
            )
        }
    }
}

/// A light RFC 3339 shape check, so a bad `since` is refused before it reaches SQL.
fn chrono_like(s: &str) -> Option<()> {
    let ok = s.len() >= 20
        && s.len() <= 40
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[10] == b'T'
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | 'Z' | '+' | '.'));
    ok.then_some(())
}

async fn post_run(
    State(backend): State<Arc<dyn RunBackend>>,
    headers: HeaderMap,
    Json(run): Json<PostedRun>,
) -> Response {
    if !backend.authorized(&headers, &run.site).await {
        return plain(
            StatusCode::UNAUTHORIZED,
            "Sign in as the owner, or send the gauntlet token.",
        );
    }
    let cited = run.cited();
    if cited.len() > CITE_MAX {
        return plain(StatusCode::BAD_REQUEST, "Too many receipts cited.");
    }
    let keys = backend.keys();
    let mut loaded = Vec::new();
    for digest in &cited {
        let receipt = match backend.receipt(digest).await {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => {
                tracing_error(&e);
                return plain(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The receipts could not be read.",
                );
            }
        };
        // Re-verify, re-address, and keep only this site's receipts.
        let Ok(json) = receipt.canonical_json() else {
            continue;
        };
        if let Ok(v) = crate::verify_receipt(digest, &json, &keys)
            && v.receipt.body.site.as_str() == run.site
        {
            loaded.push(v);
        }
    }
    let s = score(manifest(), &loaded);
    match backend.record(&run.site, &run, &s).await {
        Ok(digest) => (
            StatusCode::CREATED,
            Json(json!({
                "receipt": digest,
                "score": s.score,
                "out_of": s.out_of,
                "attacks": s.attacks,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing_error(&e);
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The run could not be recorded.",
            )
        }
    }
}

fn tracing_error(e: &anyhow::Error) {
    tracing::error!(error = %e, "gauntlet route failed");
}

#[cfg(test)]
mod tests;
