//! Starting a Move (R4): the owner's export goes into the CAS, then the Move starts from it.
//!
//! - `PUT /sites/{id}/uploads/{name}`: the raw bytes of `export.xml` (WXR), `wp-content.tar.gz`
//!   or `dump.sql.gz`, owner only, up to 512 MB, checked by content type and magic bytes,
//!   stored by digest.
//! - `POST /sites/{id}/moves`: owner only (the owner is set only by a verified claim, R3.2);
//!   starts the Move from the site's latest uploads. For Zavora's fixture site (R3.3) with no
//!   uploads, it reads the fixture's exports from the configured directory. A Move already
//!   running is not started twice.

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_move::ports::Blobs as _;
use agentrix_move::{MoveInputs, MoveState, MoveStore, Mover};
use agentrix_types::{Digest, SiteId};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{post, put};

use super::{Fail, Reply, Sessions, fail, internal, wants_json};
use crate::launch::ports::NodeBlobs;

/// The export files a Move reads.
pub const UPLOADS: [&str; 3] = ["export.xml", "wp-content.tar.gz", "dump.sql.gz"];

/// The largest upload, in bytes.
pub const MAX_UPLOAD: usize = 512 * 1024 * 1024;

/// Whether an upload is what its name says: a WXR export is XML; the archives are gzip.
pub fn upload_ok(name: &str, content_type: Option<&str>, body: &[u8]) -> Result<(), &'static str> {
    let declared = content_type
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if name == "export.xml" {
        let ok_type = matches!(
            declared.as_str(),
            "application/xml" | "text/xml" | "application/rss+xml" | "application/octet-stream"
        );
        let head = String::from_utf8_lossy(&body[..body.len().min(512)])
            .trim_start_matches('\u{feff}')
            .trim_start()
            .to_ascii_lowercase();
        if !ok_type || !(head.starts_with("<?xml") || head.starts_with("<rss")) {
            return Err(
                "That isn't a WordPress export: it should be the XML file from Tools → Export.",
            );
        }
    } else {
        let ok_type = matches!(
            declared.as_str(),
            "application/gzip"
                | "application/x-gzip"
                | "application/x-tar"
                | "application/octet-stream"
        );
        if !ok_type || !body.starts_with(&[0x1f, 0x8b]) {
            return Err("That isn't a .gz archive.");
        }
    }
    Ok(())
}

/// What the Move routes need.
#[derive(Clone)]
pub struct MovesState {
    /// The Mover.
    pub mover: Mover,
    /// Where uploads go.
    pub blobs: NodeBlobs,
    /// Who is signed in.
    pub sessions: Arc<dyn Sessions>,
    /// The fixture's exports (`fixtures/wordpress-infected/exports/demo`), for the fixture site.
    pub fixture_exports: Option<PathBuf>,
    /// The platform's checks (the owner's Freeze pauses the Move).
    pub guards: crate::guards::Guards,
}

/// Why the Move waits while the owner's Freeze is in force.
pub const FROZEN_MOVE: &str =
    "Your site is frozen, so the Move is paused. Unfreeze it from your console to carry on.";

/// The routes, for `app.rs` to merge into the platform router.
pub fn router(state: MovesState) -> Router {
    Router::new()
        .route(
            "/sites/{site}/uploads/{name}",
            put(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD)),
        )
        .route("/sites/{site}/moves", post(start))
        .route("/moves/{id}/accept", post(accept))
        .with_state(state)
}

impl MovesState {
    fn store(&self) -> &MoveStore {
        &self.mover.store
    }

    async fn owner(&self, headers: &HeaderMap, site: &SiteId) -> Result<String, Fail> {
        let account = self
            .sessions
            .account(headers)
            .await
            .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "Sign in first."))?;
        let record = self
            .store()
            .site(site)
            .await
            .map_err(internal)?
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "We couldn't find that site."))?;
        if record.owner.as_deref() != Some(account.as_str()) {
            return Err(fail(
                StatusCode::FORBIDDEN,
                "Prove you own this site first, then start the Move.",
            ));
        }
        Ok(account)
    }

    async fn latest_upload(&self, site: &SiteId, name: &str) -> Result<Option<Digest>, Fail> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT digest FROM artifacts WHERE site_id = ? AND kind = ? ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )
        .bind(site.as_str())
        .bind(format!("upload:{name}"))
        .fetch_optional(self.store().pool())
        .await
        .map_err(|e| internal(e.into()))?;
        row.map(|(d,)| {
            d.parse()
                .map_err(|e: agentrix_types::Error| internal(e.into()))
        })
        .transpose()
    }

    async fn latest_receipt(&self, site: &SiteId, kind: &str) -> Option<Digest> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT digest FROM receipts WHERE site_id = ? AND kind = ? ORDER BY issued_at DESC, rowid DESC LIMIT 1",
        )
        .bind(site.as_str())
        .bind(kind)
        .fetch_optional(self.store().pool())
        .await
        .ok()
        .flatten();
        row.and_then(|(d,)| d.parse().ok())
    }

    async fn upload(
        &self,
        headers: &HeaderMap,
        site: &str,
        name: &str,
        body: &[u8],
    ) -> Result<String, Fail> {
        let site = SiteId::new(site);
        self.owner(headers, &site).await?;
        if !UPLOADS.contains(&name) {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Upload export.xml, wp-content.tar.gz or dump.sql.gz.",
            ));
        }
        if body.is_empty() {
            return Err(fail(StatusCode::BAD_REQUEST, "That file is empty."));
        }
        let declared = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok());
        upload_ok(name, declared, body).map_err(|m| fail(StatusCode::UNSUPPORTED_MEDIA_TYPE, m))?;
        self.blobs
            .put(
                &site,
                &format!("upload:{name}"),
                "application/octet-stream",
                body,
            )
            .await
            .map_err(internal)?;
        Ok("/console/move".into())
    }

    /// The look the owner chose before the Move ("Choose your new look", `agentrix-concepts`),
    /// read from the chosen design's stored page. `None` keeps the old theme's colours; a
    /// failed read only logs.
    async fn chosen_look(&self, site: &SiteId) -> Option<agentrix_move::theme::Tokens> {
        let chosen = match agentrix_concepts::chosen(&self.blobs.pool, site.as_str()).await {
            Ok(c) => c?,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "reading the chosen look failed");
                return None;
            }
        };
        let digest: Digest = chosen.html_digest?.parse().ok()?;
        let bytes = self.blobs.cas.get(&digest).await.ok().flatten()?;
        let look = agentrix_concepts::look::from_html(&String::from_utf8_lossy(&bytes))?;
        Some(agentrix_move::theme::Tokens {
            bg: look.bg,
            soft: look.soft,
            ink: look.ink,
            ink2: look.ink2,
            line: look.line,
            accent: look.accent,
            on_accent: look.on_accent,
            display: look.display,
            text: look.text,
        })
    }

    async fn start(&self, headers: &HeaderMap, site: &str) -> Result<String, Fail> {
        let site = SiteId::new(site);
        self.owner(headers, &site).await?;
        // While the owner's Freeze is in force no Move starts or resumes (W5, 2b.1).
        if self.guards.frozen(site.as_str()).await {
            return Err(fail(StatusCode::CONFLICT, FROZEN_MOVE));
        }
        if let Some(latest) = self.store().latest_move(&site).await.map_err(internal)?
            && matches!(
                latest.state,
                MoveState::Claimed
                    | MoveState::Replayed
                    | MoveState::Cleaned
                    | MoveState::Rebuilt
                    | MoveState::Proved
            )
        {
            // A Move that stopped on a fault (it is not running here) resumes where it stopped;
            // one that is running is left alone.
            if !agentrix_move::pipeline::is_running(&latest.id) {
                let mover = self.mover.clone();
                let id = latest.id.clone();
                tokio::spawn(async move {
                    if let Err(e) = mover.run(&id).await {
                        tracing::warn!(move_id = %id, error = %format!("{e:#}"), "the resumed Move stopped again");
                    }
                });
            }
            return Ok("/console/move".into());
        }
        let row: (String, String, String, i64) =
            sqlx::query_as("SELECT domain, name, old_origin, fixture FROM sites WHERE site_id = ?")
                .bind(site.as_str())
                .fetch_one(self.store().pool())
                .await
                .map_err(|e| internal(e.into()))?;
        let (domain, name, origin, fixture) = row;
        let mut uploads = Vec::new();
        for file in UPLOADS {
            uploads.push(self.latest_upload(&site, file).await?);
        }
        if uploads.iter().all(Option::is_none)
            && fixture != 0
            && let Some(dir) = &self.fixture_exports
        {
            for (slot, file) in uploads.iter_mut().zip(UPLOADS) {
                if let Ok(bytes) = tokio::fs::read(dir.join(file)).await {
                    *slot = Some(
                        self.blobs
                            .put(
                                &site,
                                &format!("upload:{file}"),
                                "application/octet-stream",
                                &bytes,
                            )
                            .await
                            .map_err(internal)?,
                    );
                }
            }
        }
        let [wxr, wp_content, dump] = [uploads[0], uploads[1], uploads[2]];
        if wxr.is_none() && dump.is_none() {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Upload your WordPress export (Tools → Export → All content) first, so the Move can read your pages.",
            ));
        }
        let inputs = MoveInputs {
            domain: domain.to_ascii_lowercase(),
            origin,
            site_name: (!name.is_empty()).then_some(name),
            wxr,
            dump,
            wp_content,
            crawl: None,
            fixture: fixture != 0,
            look: self.chosen_look(&site).await,
        };
        let assessed = self.latest_receipt(&site, "job.fetch").await;
        let claimed = self.latest_receipt(&site, "claim.verified").await;
        super::start_move(self.mover.clone(), &site, &inputs, assessed, claimed)
            .await
            .map_err(internal)?;
        Ok("/console/move".into())
    }
}

async fn upload(
    State(s): State<MovesState>,
    Path((site, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.upload(&headers, &site, &name, &body).await,
    }
}

async fn start(State(s): State<MovesState>, Path(site): Path<String>, headers: HeaderMap) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.start(&headers, &site).await,
    }
}

impl MovesState {
    /// The owner accepts flagged pages as they are (R6.4); the Move continues to go-live when
    /// that clears every flag and site parity is at the bar.
    async fn accept(&self, headers: &HeaderMap, id: &str, body: &[u8]) -> Result<String, Fail> {
        let id = agentrix_types::MoveId::new(id);
        let mv = self
            .store()
            .move_record(&id)
            .await
            .map_err(internal)?
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "We couldn't find that Move."))?;
        self.owner(headers, &mv.site).await?;
        if self.guards.frozen(mv.site.as_str()).await {
            return Err(fail(StatusCode::CONFLICT, FROZEN_MOVE));
        }
        let pages: Vec<String> = if wants_json(headers) {
            serde_json::from_slice::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("pages").cloned())
                .and_then(|p| serde_json::from_value(p).ok())
                .unwrap_or_default()
        } else {
            url::form_urlencoded::parse(body)
                .filter(|(k, _)| k == "page")
                .map(|(_, v)| v.into_owned())
                .collect()
        };
        if pages.is_empty() {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Choose the pages you accept as they are.",
            ));
        }
        if mv.state != MoveState::Rebuilt {
            return Err(fail(
                StatusCode::CONFLICT,
                "This Move has no pages waiting for you.",
            ));
        }
        let state = self.mover.accept_pages(&id, &pages).await.map_err(|e| {
            tracing::warn!(error = %format!("{e:#}"), "accepting pages failed");
            fail(
                StatusCode::CONFLICT,
                e.to_string()
                    .lines()
                    .next()
                    .unwrap_or("That didn't work.")
                    .to_owned(),
            )
        })?;
        Ok(match state {
            MoveState::AwaitingOwner => format!("/console/{}/decisions", mv.site),
            _ => "/console/move".into(),
        })
    }
}

async fn accept(
    State(s): State<MovesState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.accept(&headers, &id, &body).await,
    }
}
