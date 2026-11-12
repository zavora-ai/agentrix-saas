//! Signed releases: load, verify, and keep the last good one (R8.1, design §4.5).
//!
//! A release is live for a host when `site_pointer` names it. The edge serves it only after:
//! 1. the site log's checkpoint verifies against one of the site's release keys
//!    (`verify_checkpoint`, key name `{domain}/.well-known/awp/log`);
//! 2. the log includes this release's manifest digest at its sequence number
//!    (`verify_release_inclusion`, from the log's tiles);
//! 3. the manifest blob in CAS has that digest (`manifest_digest`, the JCS form);
//! 4. every file it serves hashes to the digest its manifest entry names.
//!
//! IF a newer release fails any check, THEN the edge keeps serving the last good release for
//! that host and logs why. This is the same check `awp verify` runs from published files.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;

use adk_site::log::{TileFetch, TileProofs, verify_checkpoint, verify_release_inclusion};
use adk_site::release::{file_digest, manifest_digest};
use async_trait::async_trait;
use awp_types::{Digest, FileEntry, Redirect, ReleaseManifest};
use bytes::Bytes;
use tokio::sync::{OnceCell, RwLock};

use super::ask::AskIndex;

/// The live release a host points at, as `site_pointer` and the `releases` row record it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePointer {
    /// The site's id.
    pub site_id: String,
    /// The site's canonical domain: the checkpoint origin is `{domain}/.well-known/awp/log`.
    pub domain: String,
    /// The release's sequence number.
    pub seq: u64,
    /// The manifest digest, `sha256:<64 lowercase hex>`.
    pub manifest_digest: String,
    /// The signed checkpoint note stored with the release.
    pub checkpoint: String,
    /// The site's release verifier keys (`<name>+<hash>+<base64 key>`), from its AWP keys.
    pub release_keys: Vec<String>,
}

/// Where the edge reads releases from: the site pointer, the releases table, the site log's
/// tiles and the content-addressed store. The data and move lanes provide the implementation.
#[async_trait]
pub trait ReleaseSource: Send + Sync {
    /// The live pointer for a host (lowercase, no port), or `None` when the host serves no site.
    async fn live(&self, host: &str) -> anyhow::Result<Option<LivePointer>>;
    /// A blob from the content-addressed store by digest (`sha256:<hex>`).
    async fn blob(&self, digest: &str) -> anyhow::Result<Option<Bytes>>;
    /// A file of the site's log by path relative to the log prefix (tiles and entry bundles).
    async fn log_file(&self, site_id: &str, path: &str) -> anyhow::Result<Option<Bytes>>;
}

/// Reads a site's log tiles through a [`ReleaseSource`].
struct LogTiles<'a> {
    source: &'a dyn ReleaseSource,
    site_id: &'a str,
}

#[async_trait]
impl TileFetch for LogTiles<'_> {
    async fn fetch_tile(&self, path: &str) -> adk_site::Result<Option<Bytes>> {
        self.source
            .log_file(self.site_id, path)
            .await
            .map_err(|e| adk_site::SiteError::Log(format!("log tile {path}: {e}")))
    }
}

fn parse_all<T: FromStr>(keys: &[String]) -> Vec<T> {
    keys.iter().filter_map(|k| k.parse().ok()).collect()
}

/// A release that passed every check, with its Ask index built on first use.
pub struct LoadedRelease {
    /// The pointer it was loaded from.
    pub pointer: LivePointer,
    /// The verified manifest.
    pub manifest: ReleaseManifest,
    pub(super) index: OnceCell<Arc<AskIndex>>,
}

impl LoadedRelease {
    /// The manifest entry for a request path, trying `path`, `path/index.html` and
    /// `path.html` in that order. `/` is `index.html`.
    pub fn resolve(&self, path: &str) -> Option<(&str, &FileEntry)> {
        let trimmed = path.trim_start_matches('/');
        let candidates: Vec<String> = if trimmed.is_empty() {
            vec!["index.html".into()]
        } else if trimmed.ends_with('/') {
            vec![format!("{trimmed}index.html")]
        } else {
            vec![
                trimmed.into(),
                format!("{trimmed}/index.html"),
                format!("{trimmed}.html"),
            ]
        };
        candidates
            .into_iter()
            .find_map(|c| self.manifest.files.get_key_value(c.as_str()))
            .map(|(k, v)| (k.as_str(), v))
    }

    /// The Markdown twin of an HTML file: `about/index.html` → `about/index.md`.
    pub fn twin_of(&self, html_path: &str) -> Option<(&str, &FileEntry)> {
        let stem = html_path.strip_suffix(".html")?;
        self.manifest
            .files
            .get_key_value(format!("{stem}.md").as_str())
            .map(|(k, v)| (k.as_str(), v))
    }

    /// The redirect for a request path, if the manifest has one.
    pub fn redirect(&self, path: &str) -> Option<&Redirect> {
        self.manifest.redirects.iter().find(|r| r.from == path)
    }

    /// Every file of the release.
    pub fn files(&self) -> &BTreeMap<String, FileEntry> {
        &self.manifest.files
    }
}

/// Loads and verifies releases, and keeps the last good one per host.
pub struct ReleaseCache {
    source: Arc<dyn ReleaseSource>,
    last_good: RwLock<HashMap<String, Arc<LoadedRelease>>>,
}

impl ReleaseCache {
    /// A cache over a source.
    pub fn new(source: Arc<dyn ReleaseSource>) -> Self {
        Self {
            source,
            last_good: RwLock::new(HashMap::new()),
        }
    }

    /// The source behind the cache (live pointers, for the Host dispatcher and the door).
    pub fn source(&self) -> &Arc<dyn ReleaseSource> {
        &self.source
    }

    /// The release to serve for a host: the live one if it verifies, else the last good one.
    ///
    /// `Ok(None)` means the host serves no site, or no release has ever verified for it.
    ///
    /// # Errors
    ///
    /// When the release source itself fails.
    pub async fn current(&self, host: &str) -> anyhow::Result<Option<Arc<LoadedRelease>>> {
        let Some(pointer) = self.source.live(host).await? else {
            return Ok(None);
        };
        if let Some(cached) = self.last_good.read().await.get(host)
            && cached.pointer == pointer
        {
            return Ok(Some(cached.clone()));
        }
        match self.load(&pointer).await {
            Ok(release) => {
                let release = Arc::new(release);
                self.last_good
                    .write()
                    .await
                    .insert(host.to_owned(), release.clone());
                Ok(Some(release))
            }
            Err(refused) => {
                tracing::warn!(host, seq = pointer.seq, %refused, "refusing release; keeping the last good one");
                Ok(self.last_good.read().await.get(host).cloned())
            }
        }
    }

    async fn load(&self, pointer: &LivePointer) -> anyhow::Result<LoadedRelease> {
        let digest = Digest::from_str(&pointer.manifest_digest)
            .map_err(|e| anyhow::anyhow!("manifest digest: {e}"))?;
        let checkpoint = verify_checkpoint(
            &pointer.checkpoint,
            &pointer.domain,
            parse_all(&pointer.release_keys).as_slice(),
        )?;
        if checkpoint.size < pointer.seq {
            anyhow::bail!(
                "the checkpoint covers {} releases, not {}",
                checkpoint.size,
                pointer.seq
            );
        }
        let tiles = LogTiles {
            source: self.source.as_ref(),
            site_id: &pointer.site_id,
        };
        let proofs = TileProofs::new(&tiles, checkpoint.size);
        let entry = proofs.entry(pointer.seq - 1).await?;
        let proof = proofs.inclusion_proof(pointer.seq - 1).await?;
        verify_release_inclusion(&checkpoint, &entry, &proof, pointer.seq, &digest)?;

        let blob = self
            .source
            .blob(&pointer.manifest_digest)
            .await?
            .ok_or_else(|| anyhow::anyhow!("manifest {} is missing", pointer.manifest_digest))?;
        let manifest: ReleaseManifest = serde_json::from_slice(&blob)?;
        let (computed, _) = manifest_digest(&manifest)?;
        if computed != digest {
            anyhow::bail!("the manifest does not match its digest");
        }
        if manifest.seq != pointer.seq {
            anyhow::bail!(
                "the pointer says release {}, the manifest says {}",
                pointer.seq,
                manifest.seq
            );
        }
        if !manifest.is_snapshot() {
            anyhow::bail!(
                "release {} is a delta; the edge serves snapshot releases",
                manifest.seq
            );
        }
        Ok(LoadedRelease {
            pointer: pointer.clone(),
            manifest,
            index: OnceCell::new(),
        })
    }

    /// A file's bytes, checked against its manifest digest. `None` when missing or tampered.
    ///
    /// # Errors
    ///
    /// When the store itself fails.
    pub async fn file(&self, entry: &FileEntry) -> anyhow::Result<Option<Bytes>> {
        let Some(bytes) = self.source.blob(entry.sha256.as_str()).await? else {
            return Ok(None);
        };
        if file_digest(&bytes) != entry.sha256 {
            tracing::warn!(digest = %entry.sha256.as_str(), "a stored file does not match its digest; not serving it");
            return Ok(None);
        }
        Ok(Some(bytes))
    }
}
