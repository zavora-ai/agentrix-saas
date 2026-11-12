//! Replay: read the old site into an inventory (R4).
//!
//! Two paths, both ending in an [`OldSite`]:
//! - **Replay** (R4.1): a `replay` room rebuilds WordPress from the dump or WXR with an
//!   internal-only network and returns `inventory.json` (adk-site's [`SiteInventory`]).
//! - **Fallback** (R4.4): when this deployment has no replay room, or the room fails, the
//!   Move reads the WXR export with adk-site's `Wxr` source, the uploads and code from the
//!   owner's `wp-content.tar.gz`, and the public pages the Assess crawl already read. The
//!   Move screen says so.
//!
//! Nothing here runs the old site's code: PHP, scripts and theme files are read as text for
//! the detectors and never copied (R5.5).

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use adk_core::{ActingContext, SubjectKind};
use adk_site::migrate::http::HttpClient;
use adk_site::migrate::sources::Wxr;
use adk_site::migrate::{InventoryMedia, InventoryUrl, SiteInventory, SiteSource, SourceContext};
use agentrix_types::{Digest, SiteId};
use anyhow::{Context as _, bail};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::ports::Blobs;

/// What the owner gave the Move, by digest in the CAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveInputs {
    /// The site's domain, for example `kilele-bakery.example`.
    pub domain: String,
    /// The old site's public origin, for example `https://kilele-bakery.example`.
    pub origin: String,
    /// The business name, when known.
    #[serde(default)]
    pub site_name: Option<String>,
    /// The WordPress export (WXR).
    #[serde(default)]
    pub wxr: Option<Digest>,
    /// A database dump, for the replay room.
    #[serde(default)]
    pub dump: Option<Digest>,
    /// `wp-content.tar.gz`: uploads, themes and plugins.
    #[serde(default)]
    pub wp_content: Option<Digest>,
    /// The Assess crawl's [`CrawlBundle`], when there is one.
    #[serde(default)]
    pub crawl: Option<Digest>,
    /// Zavora's fixture site (R3.3); every screen labels it.
    #[serde(default)]
    pub fixture: bool,
    /// The look the owner chose before the Move (the concept stage, `agentrix-concepts`): the
    /// rebuild uses its colours and fonts, through [`crate::theme::Tokens::chosen`], instead of
    /// the old theme's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<crate::theme::Tokens>,
}

/// The pages a crawl read, as the Assess pipeline stores them (one JSON blob in the CAS).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlBundle {
    /// Every page read.
    pub pages: Vec<CrawlPage>,
}

/// One crawled page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlPage {
    /// The absolute URL.
    pub url: String,
    /// The HTTP status.
    pub status: u16,
    /// The response's media type.
    #[serde(default)]
    pub content_type: Option<String>,
    /// The body, in the CAS.
    #[serde(default)]
    pub body: Option<Digest>,
    /// A sealed screenshot of the page (PNG), in the CAS.
    #[serde(default)]
    pub screenshot: Option<Digest>,
}

/// A file of the old site's code: themes, plugins, must-use plugins. Read as text for the
/// detectors, never copied, never run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeFile {
    /// Its path under `wp-content`, for example `mu-plugins/cloak.php`.
    pub path: String,
    /// Its bytes, in the CAS.
    pub digest: Digest,
}

/// The old site as read: what the Replay step persists and the Clean step reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OldSite {
    /// adk-site's inventory. Page bodies and media bytes are kept by digest beside it.
    pub inventory: SiteInventory,
    /// Page bodies by URL path.
    #[serde(default)]
    pub bodies: BTreeMap<String, Digest>,
    /// Upload bytes by site path, for example `/wp-content/uploads/2026/10/bread.jpg`.
    #[serde(default)]
    pub media: BTreeMap<String, Digest>,
    /// Theme and plugin files.
    #[serde(default)]
    pub code: Vec<CodeFile>,
    /// Old screenshots by URL path, from the crawl.
    #[serde(default)]
    pub screenshots: BTreeMap<String, Digest>,
    /// Themes with a build hook (`themes/<name>/build.sh`).
    #[serde(default)]
    pub theme_hooks: Vec<String>,
    /// `wp-content.tar.gz`, for the build room's legacy-theme step.
    #[serde(default)]
    pub wp_content: Option<Digest>,
    /// True when a sealed replay room read the site; false for the R4.4 fallback.
    pub replayed: bool,
    /// Pages the replay served differently to a search crawler than to a browser.
    #[serde(default)]
    pub cloaked: Vec<CloakedPage>,
    /// What was read and what was not, in plain words.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl OldSite {
    /// The inventory with page bodies and media bytes loaded back from the CAS, for the
    /// detectors.
    pub async fn hydrate(&self, blobs: &dyn Blobs, site: &SiteId) -> anyhow::Result<SiteInventory> {
        let mut inventory = self.inventory.clone();
        for url in &mut inventory.urls {
            if let Some(digest) = self.bodies.get(&url.path) {
                url.body = blobs.get(site, digest).await?.map(Bytes::from);
            }
        }
        for media in &mut inventory.media {
            if let Some(digest) = self.media.get(&media.path)
                && let Some(bytes) = blobs.get(site, digest).await?
            {
                *media = media.clone().with_bytes(Bytes::from(bytes));
            }
        }
        Ok(inventory)
    }

    /// Plain words for the Replay step's summary.
    pub fn summary(&self) -> String {
        let pages = self.inventory.items.len();
        let media = self.media.len();
        let how = if self.replayed {
            "in a sealed replay"
        } else {
            "from your export and public pages (no replay)"
        };
        format!("Read {pages} pages and {media} files {how}")
    }
}

/// Limits on the owner's archive, so a hostile archive cannot exhaust control.
const MAX_ARCHIVE_FILES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CODE_BYTES: u64 = 1024 * 1024;

/// One regular file from a tar archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TarFile {
    /// Its path, normalised: no leading `./`, no `..`, no leading `wp-content/`.
    pub path: String,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// Reads the regular files of a `.tar` or `.tar.gz` archive, within fixed limits. Links,
/// devices and paths that climb out (`..`) are skipped.
pub fn read_archive(bytes: &[u8]) -> anyhow::Result<Vec<TarFile>> {
    let tar = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes)
            .take(MAX_TOTAL_BYTES + 1)
            .read_to_end(&mut out)
            .context("the archive is not a readable .tar.gz")?;
        if out.len() as u64 > MAX_TOTAL_BYTES {
            bail!(
                "the archive is larger than {} MB",
                MAX_TOTAL_BYTES / 1024 / 1024
            );
        }
        out
    } else {
        bytes.to_vec()
    };
    let mut files = Vec::new();
    let mut at = 0usize;
    let mut long_name: Option<String> = None;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let size = octal(&header[124..136]).context("a tar header has a bad size")?;
        let kind = header[156];
        let data_start = at + 512;
        let data_end = data_start
            .checked_add(usize::try_from(size).unwrap_or(usize::MAX))
            .filter(|end| *end <= tar.len())
            .context("the archive is truncated")?;
        let data = &tar[data_start..data_end];
        at = data_start + (size as usize).div_ceil(512) * 512;
        match kind {
            b'L' => {
                long_name = Some(cstr(data));
                continue;
            }
            b'x' => {
                long_name = pax_path(data).or(long_name);
                continue;
            }
            b'0' | 0 => {}
            _ => {
                long_name = None;
                continue;
            }
        }
        let name = match long_name.take() {
            Some(name) => name,
            None => {
                let name = cstr(&header[0..100]);
                let prefix = if &header[257..262] == b"ustar" {
                    cstr(&header[345..500])
                } else {
                    String::new()
                };
                if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                }
            }
        };
        let Some(path) = normalise(&name) else {
            continue;
        };
        if size > MAX_FILE_BYTES {
            continue;
        }
        if files.len() >= MAX_ARCHIVE_FILES {
            bail!("the archive holds more than {MAX_ARCHIVE_FILES} files");
        }
        files.push(TarFile {
            path,
            bytes: data.to_vec(),
        });
    }
    Ok(files)
}

fn cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

fn octal(field: &[u8]) -> Option<u64> {
    let text = cstr(field);
    let text = text.trim();
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

fn pax_path(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    text.lines().find_map(|line| {
        let (_, record) = line.split_once(' ')?;
        record.strip_prefix("path=").map(str::to_owned)
    })
}

fn normalise(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    if parts.first() == Some(&"wp-content") {
        parts.remove(0);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn is_code(path: &str) -> bool {
    let top = path.split('/').next().unwrap_or_default();
    matches!(top, "themes" | "plugins" | "mu-plugins")
        || path.ends_with(".php")
        || path.ends_with(".phtml")
}

fn acting(domain: &str) -> ActingContext {
    ActingContext::derived(domain, "mover", "owner", SubjectKind::Human)
}

/// Reads the old site through the R4.4 fallback: the WXR export, the archive's uploads and
/// code, and the Assess crawl's pages.
pub async fn read_fallback(
    blobs: &dyn Blobs,
    site: &SiteId,
    inputs: &MoveInputs,
) -> anyhow::Result<OldSite> {
    let mut notes = vec![
        "Read from your export and the public pages, without a sealed replay of the old site."
            .to_owned(),
    ];
    let mut inventory = match &inputs.wxr {
        Some(digest) => {
            let bytes = blobs
                .get(site, digest)
                .await?
                .context("the WordPress export is missing from storage")?;
            let http = Arc::new(
                HttpClient::builder()
                    .build()
                    .map_err(|e| anyhow::anyhow!("crawler client: {e}"))?,
            );
            let ctx = SourceContext::new(acting(&inputs.domain), http);
            Wxr::from_bytes(bytes)
                .inventory(&ctx)
                .await
                .map_err(|e| anyhow::anyhow!("the WordPress export could not be read: {e}"))?
        }
        None => {
            notes.push("No WordPress export was given; only public pages were read.".into());
            SiteInventory::new(adk_site::migrate::SourceKind::Crawl, chrono::Utc::now())
        }
    };
    if inventory.origin.is_none() {
        inventory.origin = Some(inputs.origin.trim_end_matches('/').to_owned());
    }
    if let Some(name) = &inputs.site_name {
        inventory.site_name = Some(name.clone());
    }

    let mut old = OldSite {
        inventory,
        bodies: BTreeMap::new(),
        media: BTreeMap::new(),
        code: Vec::new(),
        screenshots: BTreeMap::new(),
        theme_hooks: Vec::new(),
        wp_content: inputs.wp_content,
        replayed: false,
        cloaked: Vec::new(),
        notes,
    };

    if let Some(digest) = &inputs.wp_content {
        let bytes = blobs
            .get(site, digest)
            .await?
            .context("wp-content.tar.gz is missing from storage")?;
        let files = read_archive(&bytes)?;
        let origin = old.inventory.origin.clone().unwrap_or_default();
        for file in files {
            if file.path.starts_with("uploads/") {
                let path = format!("/wp-content/{}", file.path);
                let digest = blobs
                    .put(site, "old-upload", "application/octet-stream", &file.bytes)
                    .await?;
                old.media.insert(path.clone(), digest);
                if !old.inventory.media.iter().any(|m| m.path == path) {
                    old.inventory
                        .media
                        .push(InventoryMedia::new(format!("{origin}{path}"), path));
                }
            } else if is_code(&file.path) && (file.bytes.len() as u64) <= MAX_CODE_BYTES {
                let digest = blobs
                    .put(site, "old-code", "text/plain", &file.bytes)
                    .await?;
                if let Some(theme) = file
                    .path
                    .strip_prefix("themes/")
                    .and_then(|rest| rest.strip_suffix("/build.sh"))
                    .filter(|theme| !theme.contains('/'))
                {
                    old.theme_hooks.push(theme.to_owned());
                }
                old.code.push(CodeFile {
                    path: file.path,
                    digest,
                });
            }
        }
    } else {
        old.notes
            .push("No wp-content archive was given; uploads were not checked.".into());
    }

    if let Some(digest) = &inputs.crawl {
        let bytes = blobs
            .get(site, digest)
            .await?
            .context("the crawl is missing from storage")?;
        let crawl: CrawlBundle =
            serde_json::from_slice(&bytes).context("the crawl bundle does not parse")?;
        merge_crawl(&mut old, &crawl);
    }
    Ok(old)
}

/// A page the replay served differently to a crawler: both versions are in the CAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloakedPage {
    /// The page.
    pub path: String,
    /// What a browser got.
    pub browser: Option<Digest>,
    /// What a crawler got.
    pub crawler: Option<Digest>,
}

/// One version of a replayed page, as room-replay's `recording.json` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayedFile {
    /// Its file under the room's `/out`.
    pub file: String,
    /// The HTTP status.
    #[serde(default)]
    pub status: u16,
}

/// One page of room-replay's `recording.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayedPage {
    /// Its path.
    pub path: String,
    /// Its public URL.
    pub public_url: String,
    /// As a browser saw it.
    pub browser: ReplayedFile,
    /// As a search crawler saw it.
    pub crawler: ReplayedFile,
    /// Whether the two differ.
    #[serde(default)]
    pub differs: bool,
}

/// Adds the replay's rendered pages to the inventory (as the crawl's are) and records the
/// pages served differently to crawlers. `artifact` maps a room file to its digest. Returns
/// how many pages were read.
pub fn merge_replay(
    old: &mut OldSite,
    pages: &[ReplayedPage],
    artifact: impl Fn(&str) -> Option<Digest>,
) -> usize {
    let mut bundle = CrawlBundle::default();
    for page in pages {
        let body = artifact(&page.browser.file);
        if page.browser.status == 200 && body.is_some() {
            bundle.pages.push(CrawlPage {
                url: page.public_url.clone(),
                status: page.browser.status,
                content_type: Some("text/html".into()),
                body,
                screenshot: None,
            });
        }
        if page.differs {
            old.cloaked.push(CloakedPage {
                path: page.path.clone(),
                browser: body,
                crawler: artifact(&page.crawler.file),
            });
        }
    }
    let read = bundle.pages.len();
    merge_crawl(old, &bundle);
    read
}

/// Adds the crawl's pages to the inventory as URLs with bodies, linked to the export's
/// items by path, and keeps their screenshots for parity.
pub fn merge_crawl(old: &mut OldSite, crawl: &CrawlBundle) {
    for page in &crawl.pages {
        let Ok(url) = url::Url::parse(&page.url) else {
            continue;
        };
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        };
        let item = old
            .inventory
            .items
            .iter()
            .find(|i| i.path == path)
            .map(|i| i.key.clone());
        if let Some(body) = page.body {
            old.bodies.insert(path.clone(), body);
        }
        if let Some(shot) = page.screenshot {
            old.screenshots.insert(path.clone(), shot);
        }
        if old.inventory.urls.iter().any(|u| u.path == path) {
            continue;
        }
        old.inventory.urls.push(InventoryUrl {
            url: page.url.clone(),
            path,
            status: page.status,
            redirect_to: None,
            content_type: page.content_type.clone(),
            sha256: None,
            item,
            page: None,
            inbound_links: 0,
            body: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_entry(name: &str, body: &[u8], kind: u8) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let size = format!("{:011o}\0", body.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[156] = kind;
        header[257..262].copy_from_slice(b"ustar");
        let mut out = header.to_vec();
        out.extend_from_slice(body);
        out.resize(512 + body.len().div_ceil(512) * 512, 0);
        out
    }

    #[test]
    fn reads_regular_files_and_refuses_paths_that_climb_out() {
        let mut tar = Vec::new();
        tar.extend(tar_entry("wp-content/uploads/a.txt", b"hello", b'0'));
        tar.extend(tar_entry(
            "./wp-content/mu-plugins/cloak.php",
            b"<?php",
            b'0',
        ));
        tar.extend(tar_entry("../../etc/passwd", b"root", b'0'));
        tar.extend(tar_entry("wp-content/uploads/link", b"", b'2'));
        tar.extend([0u8; 1024]);
        let files = read_archive(&tar).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["uploads/a.txt", "mu-plugins/cloak.php"]);
        assert_eq!(files[0].bytes, b"hello");

        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        assert_eq!(read_archive(&gz.finish().unwrap()).unwrap().len(), 2);
    }
}
