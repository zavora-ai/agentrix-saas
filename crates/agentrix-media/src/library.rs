//! The media library: every picture a site has, kept by digest, and the owner's say over
//! which ones are shown (`media_images`, migration 1200).
//!
//! - [`keep`] stores a picture the pipeline made as a **draft**: the owner sees it, visitors
//!   don't.
//! - [`approve`] shows it: the slot's earlier picture becomes **replaced**, and a receipt
//!   records who approved which bytes.
//! - [`decline`] keeps the row (so the owner sees what was tried) and never shows it.
//! - [`shown`] is what a site renders: slot → approved picture, URL, alt text and label.

use std::collections::HashMap;
use std::sync::Arc;

use agentrix_host::{Blobs, Receipts};
use agentrix_types::receipt::{ReceiptKind, Subject};
use agentrix_types::{ReceiptBody, SiteId};
use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::pipeline::{Blocked, Made};
use crate::policy::Kind;

/// One picture in the library.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
pub struct Image {
    /// `img_<uuid>`.
    pub image_id: String,
    /// The site.
    pub site_id: String,
    /// Where it goes: `hero`, `product:<sku>`, `section:<name>`, `post:<slug>`, `story:<slug>`,
    /// `part:<name>`, `page:<path>#<n>`.
    pub slot: String,
    /// The [`Kind`], as text.
    pub kind: String,
    /// `draft`, `approved`, `replaced`, `declined` or `blocked`.
    pub status: String,
    /// `generated`, `edited`, `uploaded` or `moved`.
    pub origin: String,
    /// `sha256:<hex>` in the content store; empty for a blocked picture.
    pub digest: String,
    /// Its type.
    pub media_type: String,
    /// Width in pixels.
    pub width: Option<i64>,
    /// Height in pixels.
    pub height: Option<i64>,
    /// Size in bytes.
    pub bytes: i64,
    /// The alt text.
    pub alt: String,
    /// What a visitor sees on it, if anything.
    pub label: Option<String>,
    /// Whether a visitor could take it for a photograph.
    pub photographic: bool,
    /// Whether a person appears.
    pub shows_person: bool,
    /// Whether words or logos appear.
    pub shows_text: bool,
    /// What the owner should check before approving.
    pub owner_check: String,
    /// What was asked for.
    pub brief: String,
    /// What the model was sent.
    pub prompt: String,
    /// The model that drew it.
    pub model: String,
    /// The seed it was drawn with.
    pub seed: Option<i64>,
    /// The safety verdict, as JSON text.
    pub screen: String,
    /// Who asked: `owner`, a WebAgent's role, `move` or `sample`.
    pub asked_by: String,
    /// The picture this one would replace.
    pub replaces: Option<String>,
    /// What drawing it cost, in micro-USD.
    pub cost_micro_usd: i64,
    /// When it was made.
    pub created_at: String,
    /// When the owner decided.
    pub decided_at: Option<String>,
    /// Who decided.
    pub decided_by: Option<String>,
    /// The approval's receipt digest.
    pub receipt: Option<String>,
}

impl Image {
    /// Where a site serves the bytes, once approved: `/media/<hex>.<ext>`.
    pub fn url(&self) -> String {
        url_for(&self.digest, &self.media_type)
    }
}

/// `/media/<hex>.<ext>` for a digest.
pub fn url_for(digest: &str, media_type: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    format!("/media/{hex}.{}", extension(media_type))
}

/// The file extension for an image type.
pub fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "webp",
    }
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn kind_text(kind: Kind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Who asked for a picture, and for what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'a> {
    /// The site.
    pub site: &'a str,
    /// The slot.
    pub slot: &'a str,
    /// The kind.
    pub kind: Kind,
    /// What was asked for.
    pub brief: &'a str,
    /// Who asked.
    pub asked_by: &'a str,
    /// The picture this one would replace.
    pub replaces: Option<&'a str>,
}

/// Stores a made picture's bytes and keeps it as a draft.
pub async fn keep(
    pool: &SqlitePool,
    blobs: &Arc<dyn Blobs>,
    request: &Request<'_>,
    made: &Made,
) -> anyhow::Result<Image> {
    let digest = blobs.put(Bytes::from(made.bytes.clone())).await?;
    let image_id = format!("img_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(
        "INSERT INTO media_images (image_id, site_id, slot, kind, status, origin, digest, media_type, \
         width, height, bytes, alt, label, photographic, shows_person, shows_text, owner_check, brief, \
         prompt, model, seed, screen, asked_by, replaces, cost_micro_usd, created_at) \
         VALUES (?, ?, ?, ?, 'draft', 'generated', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&image_id)
    .bind(request.site)
    .bind(request.slot)
    .bind(kind_text(request.kind))
    .bind(&digest)
    .bind(&made.media_type)
    .bind(made.width.map(i64::from))
    .bind(made.height.map(i64::from))
    .bind(made.bytes.len() as i64)
    .bind(&made.description.alt)
    .bind(made.label)
    .bind(made.description.photographic)
    .bind(made.description.shows_person)
    .bind(made.description.shows_text)
    .bind(&made.description.check)
    .bind(request.brief)
    .bind(&made.prompt)
    .bind(&made.model)
    .bind(made.seed)
    .bind(serde_json::to_string(&made.verdict)?)
    .bind(request.asked_by)
    .bind(request.replaces)
    .bind(made.cost_micro_usd.max(0))
    .bind(now())
    .execute(pool)
    .await?;
    get(pool, request.site, &image_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the picture just kept is missing"))
}

/// Keeps a note of a picture the safety check stopped; its bytes are not kept.
pub async fn keep_blocked(
    pool: &SqlitePool,
    request: &Request<'_>,
    blocked: &Blocked,
) -> anyhow::Result<Image> {
    let image_id = format!("img_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(
        "INSERT INTO media_images (image_id, site_id, slot, kind, status, origin, digest, media_type, \
         bytes, alt, photographic, shows_person, shows_text, brief, prompt, model, screen, asked_by, \
         replaces, created_at) \
         VALUES (?, ?, ?, ?, 'blocked', 'generated', '', '', 0, '', 0, 0, 0, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&image_id)
    .bind(request.site)
    .bind(request.slot)
    .bind(kind_text(request.kind))
    .bind(request.brief)
    .bind(&blocked.prompt)
    .bind(&blocked.model)
    .bind(serde_json::to_string(&blocked.verdict)?)
    .bind(request.asked_by)
    .bind(request.replaces)
    .bind(now())
    .execute(pool)
    .await?;
    get(pool, request.site, &image_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the note just kept is missing"))
}

/// One picture of a site.
pub async fn get(pool: &SqlitePool, site: &str, image_id: &str) -> anyhow::Result<Option<Image>> {
    Ok(
        sqlx::query_as("SELECT * FROM media_images WHERE site_id = ? AND image_id = ?")
            .bind(site)
            .bind(image_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// A site's pictures: drafts first (they wait on the owner), then what is shown, then the
/// rest, newest first within each.
pub async fn list(pool: &SqlitePool, site: &str) -> anyhow::Result<Vec<Image>> {
    Ok(sqlx::query_as(
        "SELECT * FROM media_images WHERE site_id = ? ORDER BY \
         CASE status WHEN 'draft' THEN 0 WHEN 'approved' THEN 1 WHEN 'blocked' THEN 2 ELSE 3 END, \
         created_at DESC LIMIT 500",
    )
    .bind(site)
    .fetch_all(pool)
    .await?)
}

/// Why a decision could not be taken, in words for the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecideError {
    /// No such picture on this site.
    NotFound,
    /// The picture is not waiting for a decision.
    NotADraft(String),
}

impl std::fmt::Display for DecideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecideError::NotFound => f.write_str("That picture isn't in this site's library."),
            DecideError::NotADraft(status) => {
                write!(f, "That picture is already {status}; nothing changed.")
            }
        }
    }
}

impl std::error::Error for DecideError {}

/// Shows a draft on the site. The slot's earlier picture is marked replaced, and a receipt
/// records the approval (who, which bytes, which slot, the label).
pub async fn approve(
    pool: &SqlitePool,
    receipts: &Arc<dyn Receipts>,
    site: &str,
    image_id: &str,
    decided_by: &str,
) -> anyhow::Result<Image> {
    let image = get(pool, site, image_id)
        .await?
        .ok_or(DecideError::NotFound)?;
    if image.status != "draft" {
        return Err(DecideError::NotADraft(image.status).into());
    }
    let at = Utc::now();
    let mut body = ReceiptBody::new(
        ReceiptKind::DecisionApproved,
        SiteId::new(site),
        Subject::Site(SiteId::new(site)),
        at,
        at,
    );
    if let Ok(digest) = image.digest.parse() {
        body.outputs.push(digest);
    }
    body.detail = Some(json!({
        "media": {
            "image_id": image.image_id,
            "slot": image.slot,
            "label": image.label,
            "alt": image.alt,
            "model": image.model,
        },
        "approved_by": decided_by,
    }));
    let receipt = receipts.issue(body).await?;
    let stamp = at.to_rfc3339_opts(SecondsFormat::Micros, true);
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE media_images SET status = 'replaced' \
         WHERE site_id = ? AND slot = ? AND status = 'approved'",
    )
    .bind(site)
    .bind(&image.slot)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE media_images SET status = 'approved', decided_at = ?, decided_by = ?, receipt = ? \
         WHERE site_id = ? AND image_id = ? AND status = 'draft'",
    )
    .bind(&stamp)
    .bind(decided_by)
    .bind(&receipt)
    .bind(site)
    .bind(image_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get(pool, site, image_id)
        .await?
        .ok_or_else(|| DecideError::NotFound.into())
}

/// Keeps a draft off the site for good.
pub async fn decline(
    pool: &SqlitePool,
    site: &str,
    image_id: &str,
    decided_by: &str,
) -> anyhow::Result<Image> {
    let image = get(pool, site, image_id)
        .await?
        .ok_or(DecideError::NotFound)?;
    if image.status != "draft" {
        return Err(DecideError::NotADraft(image.status).into());
    }
    sqlx::query(
        "UPDATE media_images SET status = 'declined', decided_at = ?, decided_by = ? \
         WHERE site_id = ? AND image_id = ? AND status = 'draft'",
    )
    .bind(now())
    .bind(decided_by)
    .bind(site)
    .bind(image_id)
    .execute(pool)
    .await?;
    get(pool, site, image_id)
        .await?
        .ok_or_else(|| DecideError::NotFound.into())
}

/// A picture as a site shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shown {
    /// `/media/<hex>.<ext>`.
    pub url: String,
    /// The alt text.
    pub alt: String,
    /// The label to show on it, if any.
    pub label: Option<String>,
    /// Width in pixels.
    pub width: Option<i64>,
    /// Height in pixels.
    pub height: Option<i64>,
    /// `sha256:<hex>`.
    pub digest: String,
}

/// What a site shows: each slot's approved picture.
pub async fn shown(pool: &SqlitePool, site: &str) -> anyhow::Result<HashMap<String, Shown>> {
    let rows: Vec<Image> =
        sqlx::query_as("SELECT * FROM media_images WHERE site_id = ? AND status = 'approved'")
            .bind(site)
            .fetch_all(pool)
            .await?;
    Ok(rows
        .into_iter()
        .map(|image| {
            let shown = Shown {
                url: image.url(),
                alt: image.alt,
                label: image.label,
                width: image.width,
                height: image.height,
                digest: image.digest,
            };
            (image.slot, shown)
        })
        .collect())
}

/// The type of an approved picture with this digest, if any site shows it.
pub async fn approved_type(pool: &SqlitePool, digest: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT media_type FROM media_images WHERE digest = ? AND status = 'approved' LIMIT 1",
    )
    .bind(digest)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(t,)| t))
}

/// How many pictures wait for the owner on `site`, optionally only those `asked_by` one asker
/// (`move` for the Move's fill). One indexed count, cheap enough for any page.
pub async fn waiting(pool: &SqlitePool, site: &str, asked_by: Option<&str>) -> anyhow::Result<i64> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM media_images WHERE site_id = ? AND status = 'draft' \
         AND (? IS NULL OR asked_by = ?)",
    )
    .bind(site)
    .bind(asked_by)
    .bind(asked_by)
    .fetch_one(pool)
    .await?;
    Ok(n)
}

/// The console's Pictures page for `site` (W5's shell, contract C9).
pub fn pictures_url(site: &str) -> String {
    format!("/console/{site}/apps/pictures")
}

/// What the owner sees about spend: pictures made this month and what they cost.
pub async fn month_spend(pool: &SqlitePool, site: &str) -> anyhow::Result<Value> {
    let since = Utc::now().format("%Y-%m-01T00:00:00").to_string();
    let (made, cost): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(cost_micro_usd), 0) FROM media_images \
         WHERE site_id = ? AND status != 'blocked' AND asked_by != 'sample' AND created_at >= ?",
    )
    .bind(site)
    .bind(since)
    .fetch_one(pool)
    .await?;
    Ok(json!({ "made": made, "cost_micro_usd": cost }))
}
