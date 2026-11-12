//! Adapters between lanes' ports (integration lane, design §3.0).
//!
//! Each lane exposes a port trait; this module connects one lane's port to another lane's
//! implementation. Where the implementing lane hasn't landed yet, a `Pending*` adapter answers
//! with a plain-words error and is listed by [`pending`] so `main` logs every gap at startup.

use std::sync::Arc;

pub mod gauntlet;
pub mod host;

use async_trait::async_trait;
use axum::http::HeaderMap;
use bytes::Bytes;
use chrono::Utc;
use serde_json::Value;
use sqlx::SqlitePool;
use url::Url;
use uuid::Uuid;

use agentrix_types::{Digest, ReceiptBody, ReceiptKind, SiteId, Subject};

use crate::auth::AuthState;
use crate::claim::{ClaimEvents, ClaimProbe, FileSeen, ProbeError, SiteDirectory, Verified};
use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;
use crate::door::HeldCalls;
use crate::pages::reads::{FrameBytes, OwnerSession, ReadResult};
use crate::pages::store::Delegates;
use crate::site::release::{LivePointer, ReleaseSource};
use crate::site::{ReceiptDraft, ReceiptSink};
use awp_types::HeldCall;

/// The ports still answered by a `Pending*` adapter, for the startup log and the board.
pub fn pending() -> &'static [(&'static str, &'static str)] {
    &[]
}

/// The site lane's receipts, written through the data lane's receipts store.
pub struct DbReceiptSink {
    /// The data lane's receipts store.
    pub store: Arc<ReceiptStore>,
}

#[async_trait]
impl ReceiptSink for DbReceiptSink {
    async fn write(&self, draft: ReceiptDraft) -> anyhow::Result<String> {
        let kind = ReceiptKind::parse(draft.kind)
            .ok_or_else(|| anyhow::anyhow!("unknown receipt kind {}", draft.kind))?;
        let site = SiteId::new(draft.site);
        let now = Utc::now();
        let mut body = ReceiptBody::new(kind, site.clone(), Subject::Site(site), now, now);
        body.detail =
            Some(serde_json::json!({ "subject": draft.subject, "observed": draft.observed }));
        let stored = self.store.issue(body).await?;
        Ok(stored.digest.to_string())
    }
}

/// No releases yet: the move lane's LaunchGate writes them (S2-01).
pub struct PendingReleases;

#[async_trait]
impl ReleaseSource for PendingReleases {
    async fn live(&self, _host: &str) -> anyhow::Result<Option<LivePointer>> {
        Ok(None)
    }
    async fn blob(&self, _digest: &str) -> anyhow::Result<Option<Bytes>> {
        Ok(None)
    }
    async fn log_file(&self, _site_id: &str, _path: &str) -> anyhow::Result<Option<Bytes>> {
        Ok(None)
    }
}

/// Agent-door orders wait for the move lane's decisions module (S2-07, S2-01).
pub struct PendingHeld;

#[async_trait]
impl HeldCalls for PendingHeld {
    async fn raise(
        &self,
        _site: &str,
        _capability: &str,
        _input: Value,
        _key: &str,
    ) -> anyhow::Result<HeldCall> {
        anyhow::bail!("this site can't take orders through its agent door yet")
    }
    async fn status(&self, _site: &str, _id: &str) -> anyhow::Result<Option<HeldCall>> {
        Ok(None)
    }
}

/// Claim checks run from the sandbox through the broker (S1-05); until then they fail plainly.
pub struct PendingProbe;

#[async_trait]
impl ClaimProbe for PendingProbe {
    async fn txt(&self, _name: &str) -> Result<Vec<String>, ProbeError> {
        Err(ProbeError(
            "ownership checks from the sandbox aren't connected yet".into(),
        ))
    }
    async fn file(&self, _url: &Url) -> Result<FileSeen, ProbeError> {
        Err(ProbeError(
            "ownership checks from the sandbox aren't connected yet".into(),
        ))
    }
}

/// The data lane's sites table, for the claim module.
pub struct DbSites {
    /// The control database.
    pub pool: SqlitePool,
}

#[async_trait]
impl SiteDirectory for DbSites {
    async fn domain_of(&self, site: &str) -> Option<String> {
        crate::db::sites::domain_of(&self.pool, &SiteId::new(site))
            .await
            .ok()
            .flatten()
    }
    async fn set_owner(&self, site: &str, account: Uuid) -> Result<(), String> {
        crate::db::sites::set_owner(&self.pool, &SiteId::new(site), &account.to_string())
            .await
            .map_err(|e| e.to_string())
    }
}

/// A verified claim becomes a `claim.verified` receipt (R3, design §4.4).
pub struct ClaimReceipts {
    /// The data lane's receipts store.
    pub store: Arc<ReceiptStore>,
}

#[async_trait]
impl ClaimEvents for ClaimReceipts {
    async fn verified(&self, claim: &Verified) {
        // The receipt binds what the sandbox saw (the TXT answer or the file) and names the
        // method that proved it: `dns` or `file`.
        let evidence =
            serde_json::json!({ "account": claim.account.to_string(), "seen": claim.evidence });
        if let Err(e) = self
            .store
            .claim_verified(
                &SiteId::new(claim.site.clone()),
                &claim.claim.to_string(),
                &claim.domain,
                claim.method,
                &evidence,
                Utc::now(),
            )
            .await
        {
            tracing::error!(error = %e, site = %claim.site, "claim.verified receipt was not written");
        }
    }
}

/// The console's delegates: the owner's session (auth/), frames (CAS), approvals (pending).
pub struct NodeDelegates {
    /// Sign-in, for the owner's session.
    pub auth: AuthState,
    /// The control database.
    pub pool: SqlitePool,
    /// The content-addressed store, for frames.
    pub cas: Arc<Cas>,
    /// The move lane's decisions, for passkey options on cards.
    pub decisions: crate::decisions::DecisionsState,
}

#[async_trait]
impl Delegates for NodeDelegates {
    async fn session(&self, headers: &HeaderMap) -> ReadResult<Option<OwnerSession>> {
        let Some(session) = self.auth.session_from(headers).await? else {
            return Ok(None);
        };
        let Some(summary) = self.auth.account_summary(session.account).await? else {
            return Ok(None);
        };
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT site_id FROM sites WHERE owner_account_id = ? ORDER BY created_at LIMIT 1",
        )
        .bind(session.account.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let Some((site_id,)) = row else {
            return Ok(None);
        };
        Ok(Some(OwnerSession {
            account: summary.id.to_string(),
            name: summary.display_name,
            site: SiteId::new(site_id),
            has_passkey: summary.has_passkey,
        }))
    }

    /// Signed in without a site: the newest pending claim on a site nobody owns yet, else
    /// Assess (`/`), where a site is added. Not signed in: `None`.
    async fn landing(&self, headers: &HeaderMap) -> ReadResult<Option<String>> {
        let Some(session) = self.auth.session_from(headers).await? else {
            return Ok(None);
        };
        let claim: Option<(String,)> = sqlx::query_as(
            "SELECT c.site_id FROM claims c JOIN sites s ON s.site_id = c.site_id \
             WHERE c.account_id = ? AND c.state = 'pending' AND s.owner_account_id IS NULL \
             ORDER BY c.created_at DESC LIMIT 1",
        )
        .bind(session.account.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(Some(claim.map_or_else(
            || "/".to_owned(),
            |(site,)| format!("/sites/{site}/claim"),
        )))
    }

    async fn begin_approval(
        &self,
        session: &OwnerSession,
        card: &agentrix_types::DecisionCard,
    ) -> ReadResult<Option<String>> {
        self.decisions.begin_approval(session, card).await
    }

    async fn frame(
        &self,
        owner_site: Option<&SiteId>,
        digest: &Digest,
    ) -> ReadResult<Option<FrameBytes>> {
        let found = match owner_site {
            Some(site) => self.cas.get_for_site(&self.pool, site, digest).await?,
            None => self.cas.get_public_frame(&self.pool, digest).await?,
        };
        Ok(found.map(|(bytes, content_type)| FrameBytes {
            bytes: Bytes::from(bytes),
            content_type,
        }))
    }
}

/// The sites the edge serves, for W8's visit beacon (handover-wiring 3.1): a host's live
/// release names its site, at its own domain or its platform address, and the site's record
/// its time zone, so a day's visits are counted on the owner's calendar.
pub struct ServedSites {
    /// Live releases by host.
    pub releases: Arc<dyn ReleaseSource>,
    /// `control.db`, for the site's UTC offset.
    pub pool: SqlitePool,
}

#[async_trait]
impl agentrix_store::visits::HostSites for ServedSites {
    async fn site_for_host(&self, host: &str) -> Option<(SiteId, i32)> {
        let pointer = self.releases.live(host).await.ok()??;
        let (offset,): (i64,) =
            sqlx::query_as("SELECT utc_offset_minutes FROM sites WHERE site_id = ?")
                .bind(&pointer.site_id)
                .fetch_optional(&self.pool)
                .await
                .ok()??;
        Some((SiteId::new(pointer.site_id), i32::try_from(offset).ok()?))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt as _;

    use super::ServedSites;
    use crate::launch::tests::World;

    #[tokio::test]
    async fn a_live_sites_visits_are_counted_at_its_domain_and_platform_address() {
        let world = World::new().await;
        let card = world.card().await;
        let receipt = world.approve(&card).await;
        world
            .gate
            .go_live(&card, &receipt, "01930000-0000-7000-8000-000000000001")
            .await
            .unwrap();
        agentrix_store::migrate(&world.pool).await.unwrap();
        let releases = world
            .gate
            .releases()
            .with_platform_host(Some("agentrix.website"));
        let beacon = agentrix_store::visits::beacon_routes(
            agentrix_store::visits::VisitStore::new(world.pool.clone()),
            Arc::new(ServedSites {
                releases: Arc::new(releases),
                pool: world.pool.clone(),
            }),
        );
        for host in [
            "kilele-bakery.example",
            "kilele-bakery-example.agentrix.website",
            "elsewhere.example",
        ] {
            let response = beacon
                .clone()
                .oneshot(
                    Request::post(agentrix_store::visits::BEACON_PATH)
                        .header(header::HOST, host)
                        .header(header::ORIGIN, format!("https://{host}"))
                        .header(
                            header::USER_AGENT,
                            "Mozilla/5.0 (Linux; Android 14) Mobile Safari/537.36",
                        )
                        .body(Body::from(r#"{"r":"/about/","seq":1}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
        let (counted,): (i64,) =
            sqlx::query_as("SELECT COALESCE(SUM(visits), 0) FROM visit_counts WHERE site_id = ?")
                .bind(world.site.as_str())
                .fetch_one(&world.pool)
                .await
                .unwrap();
        assert_eq!(
            counted, 2,
            "the domain and the platform address; not a stranger"
        );
    }
}
