//! claim/'s [`ClaimProbe`], answered from the sandbox (R3.1): DNS TXT over DNS-over-HTTPS and
//! the proof file, each a `fetch` job with its own receipt.

use agentrix_types::SiteId;
use async_trait::async_trait;
use sqlx::SqlitePool;
use url::Url;

use super::Broker;
use crate::claim::{ClaimProbe, FileSeen, ProbeError};

/// The broker as claim/'s probe.
#[derive(Debug, Clone)]
pub struct BrokerProbe {
    broker: Broker,
    pool: SqlitePool,
}

impl BrokerProbe {
    /// A probe that runs jobs through `broker` and files their receipts under the site of the
    /// domain being claimed (looked up, or created, in `pool`'s `sites`).
    pub fn new(broker: Broker, pool: SqlitePool) -> Self {
        Self { broker, pool }
    }

    async fn site(&self, domain: &str) -> Result<SiteId, ProbeError> {
        let domain = domain.trim_end_matches('.').to_ascii_lowercase();
        crate::db::sites::create_or_get(&self.pool, &domain, &format!("https://{domain}"), false)
            .await
            .map_err(|_| ProbeError("we couldn't look up that site just now".into()))
    }
}

#[async_trait]
impl ClaimProbe for BrokerProbe {
    async fn txt(&self, name: &str) -> Result<Vec<String>, ProbeError> {
        let domain = name
            .trim_end_matches('.')
            .strip_prefix(&format!("{}.", crate::claim::TXT_LABEL))
            .unwrap_or(name);
        let site = self.site(domain).await?;
        self.broker.probe_txt(&site, name).await.map_err(ProbeError)
    }

    async fn file(&self, url: &Url) -> Result<FileSeen, ProbeError> {
        let host = url
            .host_str()
            .ok_or_else(|| ProbeError("the proof address has no site in it".into()))?;
        let site = self.site(host).await?;
        let (status, body) = self
            .broker
            .probe_file(&site, url)
            .await
            .map_err(ProbeError)?;
        Ok(FileSeen { status, body })
    }
}
