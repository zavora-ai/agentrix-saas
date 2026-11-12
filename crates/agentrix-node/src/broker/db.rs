//! The broker's storage seams over the data lane's `db/`: receipts through
//! [`ReceiptStore`](crate::db::receipts::ReceiptStore) and blobs through
//! [`Cas`](crate::db::cas::Cas) and its per-site `artifacts` index.

use agentrix_types::{Digest, ReceiptBody, SiteId};
use async_trait::async_trait;
use sqlx::SqlitePool;

use super::{BlobStore, ReceiptIssuer};
use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;

#[async_trait]
impl ReceiptIssuer for ReceiptStore {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Digest> {
        Ok(ReceiptStore::issue(self, body).await?.digest)
    }
}

/// Control's CAS with its `artifacts` index.
#[derive(Debug, Clone)]
pub struct DbBlobs {
    cas: Cas,
    pool: SqlitePool,
}

impl DbBlobs {
    /// Blobs in `cas`, indexed in `pool`.
    pub fn new(cas: Cas, pool: SqlitePool) -> Self {
        Self { cas, pool }
    }
}

#[async_trait]
impl BlobStore for DbBlobs {
    async fn put(
        &self,
        site: &SiteId,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest> {
        let digest = self.cas.put(bytes).await?;
        self.cas
            .record(
                &self.pool,
                site,
                &digest,
                kind,
                content_type,
                bytes.len() as u64,
            )
            .await?;
        Ok(digest)
    }

    async fn get(
        &self,
        site: &SiteId,
        digest: &Digest,
    ) -> anyhow::Result<Option<(Vec<u8>, String)>> {
        self.cas.get_for_site(&self.pool, site, digest).await
    }
}
