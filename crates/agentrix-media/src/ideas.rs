//! Ideas: a picture of what one visitor is designing, such as the cake studio's "Show me an
//! idea of it" (James, Sun 8 Nov; ADR-032).
//!
//! An idea goes through the same pipeline as every picture: the brief is checked against
//! [`crate::policy`], drawn through the gateway's `image` purpose, screened and described. It
//! differs in three ways:
//! - **The caller composes the brief from the owner's options only**, never the visitor's own
//!   words, and the brief never asks for lettering (a model garbles it; the page shows the
//!   message itself).
//! - **It skips the owner's approval and stays out of the library.** Ideas live in
//!   `media_ideas` (migration 1202), never in `media_images`: they never wait in Pictures, never
//!   become a site's own picture, and [`crate::library::shown`] never returns one. A picture the
//!   safety check stops, or one that shows a person, is never kept or shown.
//! - **It is drawn once per design.** `key` is a hash of the brief, so a second visitor with
//!   the same choices gets the same picture without a model call.
//!
//! The bytes are served at `/media/ideas/{hex}.{ext}` ([`crate::routes::picture_router`]), so the
//! visitor and the owner's held request show the same picture.

use std::sync::Arc;

use agentrix_host::Blobs;
use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::client::GatewayError;
use crate::library::extension;
use crate::pipeline::{self, Made, MakeError, Outcome};
use crate::policy::{Kind, Refusal};
use crate::routes::{MediaState, is_sample};

/// One idea, as a page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Idea {
    /// `idea_<uuid>`.
    pub idea_id: String,
    /// The site.
    pub site_id: String,
    /// A hash of the brief.
    pub key: String,
    /// `sha256:<hex>` in the content store.
    pub digest: String,
    /// Its type.
    pub media_type: String,
    /// Width in pixels.
    pub width: Option<i64>,
    /// Height in pixels.
    pub height: Option<i64>,
    /// The alt text, from the vision model.
    pub alt: String,
    /// The label a visitor sees on it ("An idea of your cake, made with AI").
    pub label: String,
    /// Whether the vision model saw letters in it (the brief never asks for any).
    pub shows_text: bool,
    /// What was asked for.
    pub brief: String,
    /// What drawing it cost, in micro-USD.
    pub cost_micro_usd: i64,
    /// When it was drawn.
    pub created_at: String,
}

impl Idea {
    /// Where any host serves the bytes: `/media/ideas/<hex>.<ext>`.
    pub fn url(&self) -> String {
        url_for(&self.digest, &self.media_type)
    }
}

/// `/media/ideas/<hex>.<ext>` for a digest.
pub fn url_for(digest: &str, media_type: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    format!("/media/ideas/{hex}.{}", extension(media_type))
}

/// Why no idea was drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdeaError {
    /// The brief asked for something the policy refuses.
    Refused(Refusal),
    /// The safety check stopped the picture, or it showed a person; nothing was kept.
    Blocked,
    /// The gateway could not draw, screen or describe (`over_budget` when today's budget is
    /// spent).
    Gateway(GatewayError),
}

impl std::fmt::Display for IdeaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdeaError::Refused(r) => r.fmt(f),
            IdeaError::Blocked => f.write_str("The picture didn't pass the safety check."),
            IdeaError::Gateway(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for IdeaError {}

impl From<MakeError> for IdeaError {
    fn from(e: MakeError) -> Self {
        match e {
            MakeError::Refused(r) => IdeaError::Refused(r),
            MakeError::Gateway(g) => IdeaError::Gateway(g),
        }
    }
}

/// What a caller asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeaAsk<'a> {
    /// The site.
    pub site: &'a str,
    /// A hash of the brief: the same brief gives the same key.
    pub key: &'a str,
    /// The kind ([`Kind::CakeIdea`]).
    pub kind: Kind,
    /// What to draw, composed from the owner's options only.
    pub brief: &'a str,
    /// What the picture is for, for the safety check and the alt text. Data, never
    /// instructions.
    pub context: &'a str,
    /// Who asked (`studio`).
    pub asked_by: &'a str,
}

/// The idea already drawn for `key` on `site`, if any.
///
/// # Errors
///
/// On a database error.
pub async fn find(pool: &SqlitePool, site: &str, key: &str) -> anyhow::Result<Option<Idea>> {
    Ok(sqlx::query_as(
        "SELECT idea_id, site_id, key, digest, media_type, width, height, alt, label, \
             shows_text, brief, cost_micro_usd, created_at FROM media_ideas \
             WHERE site_id = ? AND key = ?",
    )
    .bind(site)
    .bind(key)
    .fetch_optional(pool)
    .await?)
}

/// The idea with this digest on `site`, if any: how a held request checks the picture a
/// visitor saw.
///
/// # Errors
///
/// On a database error.
pub async fn by_digest(
    pool: &SqlitePool,
    site: &str,
    digest: &str,
) -> anyhow::Result<Option<Idea>> {
    Ok(sqlx::query_as(
        "SELECT idea_id, site_id, key, digest, media_type, width, height, alt, label, \
             shows_text, brief, cost_micro_usd, created_at FROM media_ideas \
             WHERE site_id = ? AND digest = ? LIMIT 1",
    )
    .bind(site)
    .bind(digest)
    .fetch_optional(pool)
    .await?)
}

/// How many ideas `site` has had drawn since `since` (RFC 3339): for a daily cap.
///
/// # Errors
///
/// On a database error.
pub async fn drawn_since(pool: &SqlitePool, site: &str, since: &str) -> anyhow::Result<i64> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM media_ideas WHERE site_id = ? AND created_at >= ?")
            .bind(site)
            .bind(since)
            .fetch_one(pool)
            .await?;
    Ok(n)
}

/// The type of an idea with this digest, on any site: for `/media/ideas/{hex}.{ext}`.
///
/// # Errors
///
/// On a database error.
pub async fn media_type(pool: &SqlitePool, digest: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT media_type FROM media_ideas WHERE digest = ? LIMIT 1")
            .bind(digest)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(t,)| t))
}

/// A seed from the key, so the same design draws the same picture even if its row is lost.
fn seed_of(key: &str) -> Option<i64> {
    key.get(..7).and_then(|h| i64::from_str_radix(h, 16).ok())
}

/// The idea for `ask`: the one already drawn for its key, or a new one, checked, drawn,
/// screened and described. The second value says whether it was drawn now (a model was
/// called and the caller's cap should count it).
///
/// # Errors
///
/// [`IdeaError::Refused`] before any model is called; [`IdeaError::Blocked`] when the safety
/// check stops it or it shows a person; [`IdeaError::Gateway`] when the gateway can't answer.
pub async fn draw(state: &MediaState, ask: &IdeaAsk<'_>) -> Result<(Idea, bool), IdeaError> {
    let ctx = state.ctx();
    if let Some(found) = find(&ctx.pool, ask.site, ask.key)
        .await
        .map_err(|e| gateway_down(&e))?
    {
        return Ok((found, false));
    }
    let gateway = state.gateway(ask.site);
    let first = draw_one(&gateway, ask, seed_of(ask.key)).await?;
    // The pipeline already draws once more when a person appears; a person after that means
    // nothing is shown (no owner looks at an idea before a visitor does). Letters the brief
    // never asked for get one more draw with another seed, kept only if it is clean.
    if first.description.shows_person {
        return Err(IdeaError::Blocked);
    }
    let made = if first.description.shows_text {
        match draw_one(&gateway, ask, seed_of(ask.key).map(|s| s + 1_000)).await {
            Ok(again) if !again.description.shows_person && !again.description.shows_text => again,
            _ => first,
        }
    } else {
        first
    };
    let idea = keep(&ctx.pool, &ctx.blobs, ask, &made)
        .await
        .map_err(|e| gateway_down(&e))?;
    Ok((idea, true))
}

/// One picture through the pipeline.
async fn draw_one(
    gateway: &crate::client::Gateway,
    ask: &IdeaAsk<'_>,
    seed: Option<i64>,
) -> Result<Made, IdeaError> {
    let one = pipeline::Ask {
        kind: ask.kind,
        brief: ask.brief.to_owned(),
        look: String::new(),
        context: ask.context.to_owned(),
        n: 1,
        seed,
        tier: "fast",
        sample: is_sample(ask.site),
    };
    match pipeline::make(gateway, &one).await?.into_iter().next() {
        Some(Outcome::Made(m)) => Ok(*m),
        Some(Outcome::Blocked(b)) => {
            tracing::warn!(
                site = ask.site,
                model = %b.model,
                categories = ?b.verdict.categories,
                "an idea was stopped by the safety check"
            );
            Err(IdeaError::Blocked)
        }
        None => Err(IdeaError::Gateway(GatewayError {
            code: "unreadable".into(),
            message: "The image answer had no pictures.".into(),
        })),
    }
}

fn gateway_down(err: &anyhow::Error) -> IdeaError {
    tracing::error!(error = %err, "could not keep or read an idea");
    IdeaError::Gateway(GatewayError {
        code: "not_kept".into(),
        message: "The picture couldn't be kept.".into(),
    })
}

/// Stores the bytes and remembers the idea under its key. A second draw of the same key (two
/// visitors at once) keeps the first.
async fn keep(
    pool: &SqlitePool,
    blobs: &Arc<dyn Blobs>,
    ask: &IdeaAsk<'_>,
    made: &Made,
) -> anyhow::Result<Idea> {
    let digest = blobs.put(Bytes::from(made.bytes.clone())).await?;
    let label = made.label.unwrap_or("Made with AI, not a photo").to_owned();
    sqlx::query(
        "INSERT INTO media_ideas (idea_id, site_id, key, kind, digest, media_type, width, height, \
         bytes, alt, label, shows_text, brief, prompt, model, seed, screen, asked_by, \
         cost_micro_usd, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (site_id, key) DO NOTHING",
    )
    .bind(format!("idea_{}", uuid::Uuid::now_v7().simple()))
    .bind(ask.site)
    .bind(ask.key)
    .bind(
        serde_json::to_value(ask.kind)?
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    )
    .bind(&digest)
    .bind(&made.media_type)
    .bind(made.width.map(i64::from))
    .bind(made.height.map(i64::from))
    .bind(made.bytes.len() as i64)
    .bind(&made.description.alt)
    .bind(&label)
    .bind(made.description.shows_text)
    .bind(ask.brief)
    .bind(&made.prompt)
    .bind(&made.model)
    .bind(made.seed)
    .bind(serde_json::to_string(&made.verdict)?)
    .bind(ask.asked_by)
    .bind(made.cost_micro_usd.max(0))
    .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true))
    .execute(pool)
    .await?;
    find(pool, ask.site, ask.key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the idea just kept is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ideas_are_served_apart_from_approved_pictures() {
        assert_eq!(
            url_for("sha256:ab12", "image/webp"),
            "/media/ideas/ab12.webp"
        );
        assert_eq!(seed_of("0000010ffff"), Some(16));
        assert_eq!(seed_of("zz"), None);
    }
}
