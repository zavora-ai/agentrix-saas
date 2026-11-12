//! The sample sites' pictures (W2's three samples, the home page), drawn once with the
//! `agentrix-media` CLI, committed with their manifest and embedded here. At startup they go
//! into the content store by digest and into the library as approved sample pictures, so a
//! deploy, a page load or a test never draws them again (W12's cost rule 3).

use std::sync::Arc;

use agentrix_host::Blobs;
use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use serde_json::Value;
use sqlx::SqlitePool;

/// One sample site's manifest and files.
pub struct SampleSite {
    /// `manifest.json`: `{site, images: {slot: {file, alt, label, …}}}`.
    pub manifest: &'static str,
    /// `(file name, bytes)`.
    pub files: &'static [(&'static str, &'static [u8])],
}

include!(concat!(env!("OUT_DIR"), "/samples.rs"));

/// Seeds every embedded sample picture. A slot whose owner approved a picture of their own
/// keeps it; an older sample picture for the slot is marked replaced. Returns how many rows
/// were added.
pub async fn seed(pool: &SqlitePool, blobs: &Arc<dyn Blobs>) -> anyhow::Result<usize> {
    seed_from(pool, blobs, SAMPLES).await
}

/// [`seed`] from any set of sample sites (tests pass their own).
pub async fn seed_from(
    pool: &SqlitePool,
    blobs: &Arc<dyn Blobs>,
    sites: &[SampleSite],
) -> anyhow::Result<usize> {
    let mut added = 0;
    for sample in sites {
        let manifest: Value = serde_json::from_str(sample.manifest)?;
        let site = manifest["site"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("a sample manifest names no site"))?;
        let Some(images) = manifest["images"].as_object() else {
            continue;
        };
        for (slot, entry) in images {
            let Some(file) = entry["file"].as_str() else {
                continue;
            };
            let Some((_, bytes)) = sample.files.iter().find(|(name, _)| *name == file) else {
                anyhow::bail!("{site}: {file} is in the manifest but not embedded");
            };
            if seed_one(pool, blobs, site, slot, entry, bytes).await? {
                added += 1;
            }
        }
    }
    Ok(added)
}

async fn seed_one(
    pool: &SqlitePool,
    blobs: &Arc<dyn Blobs>,
    site: &str,
    slot: &str,
    entry: &Value,
    bytes: &'static [u8],
) -> anyhow::Result<bool> {
    let digest = blobs.put(Bytes::from_static(bytes)).await?;
    let (same,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM media_images WHERE site_id = ? AND slot = ? AND digest = ?",
    )
    .bind(site)
    .bind(slot)
    .bind(&digest)
    .fetch_one(pool)
    .await?;
    if same > 0 {
        return Ok(false);
    }
    let (owners,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM media_images \
         WHERE site_id = ? AND slot = ? AND status = 'approved' AND asked_by != 'sample'",
    )
    .bind(site)
    .bind(slot)
    .fetch_one(pool)
    .await?;
    let status = if owners > 0 { "replaced" } else { "approved" };
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
    let text = |key: &str| entry[key].as_str().unwrap_or_default().to_owned();
    let flag = |key: &str| entry["description"][key].as_bool().unwrap_or(true);
    let mut tx = pool.begin().await?;
    if status == "approved" {
        sqlx::query(
            "UPDATE media_images SET status = 'replaced' \
             WHERE site_id = ? AND slot = ? AND status = 'approved' AND asked_by = 'sample'",
        )
        .bind(site)
        .bind(slot)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO media_images (image_id, site_id, slot, kind, status, origin, digest, media_type, \
         width, height, bytes, alt, label, photographic, shows_person, shows_text, owner_check, brief, \
         prompt, model, seed, screen, asked_by, cost_micro_usd, created_at, decided_at, decided_by) \
         VALUES (?, ?, ?, ?, ?, 'generated', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'sample', ?, ?, ?, 'sample seed')",
    )
    .bind(format!("img_{}", uuid::Uuid::now_v7().simple()))
    .bind(site)
    .bind(slot)
    .bind(text("kind"))
    .bind(status)
    .bind(&digest)
    .bind(text("media_type"))
    .bind(entry["width"].as_i64())
    .bind(entry["height"].as_i64())
    .bind(bytes.len() as i64)
    .bind(entry["description"]["alt"].as_str().unwrap_or_default())
    .bind(entry["label"].as_str())
    .bind(flag("photographic"))
    .bind(flag("shows_person"))
    .bind(flag("shows_text"))
    .bind(entry["description"]["check"].as_str().unwrap_or_default())
    .bind(text("brief"))
    .bind(text("prompt"))
    .bind(text("model"))
    .bind(entry["seed"].as_i64())
    .bind(entry["verdict"].to_string())
    .bind(entry["cost_micro_usd"].as_i64().unwrap_or(0).max(0))
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}
