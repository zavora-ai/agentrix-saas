//! In-memory receipts and blobs, for tests and for running without a database.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use agentrix_types::{Digest, Receipt, ReceiptBody, Signer, SiteId};
use async_trait::async_trait;

use super::{BlobStore, ReceiptIssuer};

/// Signs receipts with control's key and keeps them in memory.
#[derive(Debug, Clone)]
pub struct MemoryReceipts {
    signer: Arc<Signer>,
    stored: Arc<Mutex<Vec<(Digest, Receipt)>>>,
}

impl MemoryReceipts {
    /// A store that signs with `signer`.
    pub fn new(signer: Arc<Signer>) -> Self {
        Self {
            signer,
            stored: Arc::default(),
        }
    }

    /// Every receipt issued, in order.
    pub fn all(&self) -> Vec<(Digest, Receipt)> {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The receipt stored under `digest`.
    pub fn get(&self, digest: &Digest) -> Option<Receipt> {
        self.all()
            .into_iter()
            .find(|(d, _)| d == digest)
            .map(|(_, r)| r)
    }
}

#[async_trait]
impl ReceiptIssuer for MemoryReceipts {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Digest> {
        let receipt = Receipt::sign(body, &self.signer)?;
        let digest = receipt.digest()?;
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((digest, receipt));
        Ok(digest)
    }
}

/// Blobs by digest, with the per-site index.
#[derive(Debug, Clone, Default)]
pub struct MemoryBlobs {
    inner: Arc<Mutex<Blobs>>,
}

#[derive(Debug, Default)]
struct Blobs {
    bytes: HashMap<Digest, Vec<u8>>,
    index: HashSet<(SiteId, Digest)>,
    types: HashMap<(SiteId, Digest), (String, String)>,
}

impl MemoryBlobs {
    /// The `(kind, content_type)` `site` recorded for `digest`.
    pub fn kind_of(&self, site: &SiteId, digest: &Digest) -> Option<(String, String)> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .types
            .get(&(site.clone(), *digest))
            .cloned()
    }

    /// How many distinct blobs are stored.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .bytes
            .len()
    }

    /// Whether nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl BlobStore for MemoryBlobs {
    async fn put(
        &self,
        site: &SiteId,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest> {
        let digest = Digest::of(bytes);
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.bytes.entry(digest).or_insert_with(|| bytes.to_vec());
        inner.index.insert((site.clone(), digest));
        inner
            .types
            .entry((site.clone(), digest))
            .or_insert_with(|| (kind.to_owned(), content_type.to_owned()));
        Ok(digest)
    }

    async fn get(
        &self,
        site: &SiteId,
        digest: &Digest,
    ) -> anyhow::Result<Option<(Vec<u8>, String)>> {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !inner.index.contains(&(site.clone(), *digest)) {
            return Ok(None);
        }
        let ct = inner
            .types
            .get(&(site.clone(), *digest))
            .map(|(_, ct)| ct.clone())
            .unwrap_or_else(|| "application/octet-stream".into());
        Ok(inner.bytes.get(digest).map(|b| (b.clone(), ct)))
    }
}
