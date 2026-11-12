//! The candidate release (design §4.5): every file in the CAS under its digest, an
//! `awp_types::ReleaseManifest` snapshot, and the manifest's JCS bytes in the CAS under
//! `adk_site::release::manifest_digest`. Nothing here signs or publishes: only the owner's
//! approval does that, through the LaunchGate (R7).

use std::collections::{BTreeMap, BTreeSet};

use adk_site::release::{content_type, file_digest, manifest_digest};
use adk_site::scan::scan_file;
use agentrix_types::{Digest, SiteId};
use anyhow::{Context as _, bail};
use awp_types::{FileEntry, GeneratorRef, Redirect, ReleaseManifest, ReleaseSource};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ports::Blobs;

/// A staged candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// Its release number: the site log's size plus one.
    pub seq: u64,
    /// The manifest's digest, which the go-live card's fingerprint covers.
    pub manifest_digest: Digest,
    /// Pages in the release.
    pub pages: usize,
    /// Files in the release.
    pub files: usize,
}

/// Converts an awp digest to the platform's.
pub fn to_digest(d: &awp_types::Digest) -> anyhow::Result<Digest> {
    d.as_str()
        .parse()
        .map_err(|e| anyhow::anyhow!("digest {}: {e}", d.as_str()))
}

/// Converts the platform's digest to awp's.
pub fn to_awp(d: &Digest) -> awp_types::Digest {
    awp_types::Digest::from_sha256_bytes(*d.as_bytes())
}

/// Stages a snapshot release of `files` (path without a leading `/`, bytes). Refuses the
/// whole release when any file is server code (A-I1), as adk-site's release builder does.
#[allow(clippy::too_many_arguments)]
pub async fn stage(
    blobs: &dyn Blobs,
    site: &SiteId,
    seq: u64,
    previous: Option<Digest>,
    files: BTreeMap<String, Vec<u8>>,
    redirects: Vec<(String, String)>,
    pages: usize,
    now: DateTime<Utc>,
) -> anyhow::Result<Candidate> {
    let mut entries = BTreeMap::new();
    for (path, bytes) in &files {
        let found = scan_file(path, bytes);
        if !found.is_empty() {
            bail!("{path} is server code; the release was refused");
        }
        let kind = content_type(path);
        let stored = blobs.put(site, "release-file", kind, bytes).await?;
        let digest = file_digest(bytes);
        if digest.as_str() != stored.to_string() {
            bail!("the store named {path} {stored}, not {}", digest.as_str());
        }
        entries.insert(
            path.clone(),
            FileEntry {
                sha256: digest,
                size: bytes.len() as u64,
                content_type: kind.to_owned(),
            },
        );
    }
    let mut seen = BTreeSet::new();
    let redirects = redirects
        .into_iter()
        .filter(|(from, to)| from != to && seen.insert(from.clone()))
        .map(|(from, to)| Redirect {
            from,
            to,
            status: 301,
        })
        .collect();
    let manifest = ReleaseManifest {
        seq,
        previous: previous.as_ref().map(to_awp),
        base: None,
        created_at: now,
        files: entries,
        removed: BTreeSet::new(),
        redirects,
        source: ReleaseSource::Migration,
        generator: GeneratorRef {
            name: "agentrix-move".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        charter_version: 1,
    };
    let (digest, canonical) =
        manifest_digest(&manifest).map_err(|e| anyhow::anyhow!("manifest digest: {e}"))?;
    let stored = blobs
        .put(site, "manifest", "application/json", &canonical)
        .await?;
    let digest = to_digest(&digest)?;
    if stored != digest {
        bail!("the manifest was stored as {stored}, not {digest}");
    }
    Ok(Candidate {
        seq,
        manifest_digest: digest,
        pages,
        files: files.len(),
    })
}

/// Reads a staged manifest back from the CAS and checks it against its digest.
pub async fn load_manifest(
    blobs: &dyn Blobs,
    site: &SiteId,
    digest: &Digest,
) -> anyhow::Result<ReleaseManifest> {
    let bytes = blobs
        .get(site, digest)
        .await?
        .with_context(|| format!("manifest {digest} is not stored"))?;
    let manifest: ReleaseManifest = serde_json::from_slice(&bytes)?;
    let (computed, _) =
        manifest_digest(&manifest).map_err(|e| anyhow::anyhow!("manifest digest: {e}"))?;
    if to_digest(&computed)? != *digest {
        bail!("manifest {digest} does not match its digest");
    }
    Ok(manifest)
}
