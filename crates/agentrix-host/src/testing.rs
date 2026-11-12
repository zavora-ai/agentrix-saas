//! Fakes for module tests: an in-memory node without `agentrix-node`.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let ctx = agentrix_host::testing::context().await?;
//! # let _ = ctx; Ok(()) }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderMap;
use uuid::Uuid;

use agentrix_types::{Digest, ReceiptBody};

use crate::{
    Approvals, Blobs, NodeContext, Owner, OwnerAccount, OwnerAccounts, OwnerSessions, Receipts,
};

/// A [`NodeContext`] on an in-memory SQLite database and the fakes below, with origin
/// `http://localhost:8080`. Run your crate's migrations on `ctx.pool` before use.
pub async fn context() -> anyhow::Result<NodeContext> {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    Ok(NodeContext {
        pool,
        http: reqwest::Client::new(),
        data_dir: std::env::temp_dir().join(format!("agentrix-host-{}", Uuid::new_v4())),
        origin: "http://localhost:8080".parse()?,
        platform_host: "localhost".into(),
        models_url: "http://127.0.0.1:8090".into(),
        receipts: Arc::new(MemoryReceipts::default()),
        owners: Arc::new(FixedOwner::nobody()),
        approvals: Arc::new(AlwaysApprove),
        owner_accounts: Arc::new(MemoryOwnerAccounts::default()),
        blobs: Arc::new(MemoryBlobs::default()),
    })
}

/// Receipts kept in memory as JSON, keyed by the digest of that JSON. Unsigned: tests only.
#[derive(Default)]
pub struct MemoryReceipts {
    /// Everything issued, in order.
    pub issued: Mutex<Vec<(String, ReceiptBody)>>,
}

#[async_trait]
impl Receipts for MemoryReceipts {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<String> {
        let json = serde_json::to_vec(&body)?;
        let digest = Digest::of(&json).to_string();
        self.issued
            .lock()
            .expect("receipts lock")
            .push((digest.clone(), body));
        Ok(digest)
    }

    async fn get_json(&self, digest: &str) -> anyhow::Result<Option<String>> {
        let issued = self.issued.lock().expect("receipts lock");
        Ok(issued
            .iter()
            .find(|(d, _)| d == digest)
            .map(|(_, body)| serde_json::to_string(body))
            .transpose()?)
    }
}

/// The same owner on every request, or nobody.
pub struct FixedOwner(pub Option<Owner>);

impl FixedOwner {
    /// Nobody is signed in.
    pub fn nobody() -> Self {
        Self(None)
    }

    /// `account` is signed in with a passkey.
    pub fn passkey(account: Uuid) -> Self {
        Self(Some(Owner {
            account,
            method: "passkey".into(),
        }))
    }
}

#[async_trait]
impl OwnerSessions for FixedOwner {
    async fn owner(&self, _headers: &HeaderMap) -> anyhow::Result<Option<Owner>> {
        Ok(self.0.clone())
    }
}

/// Approves everything with an authenticator-code proof. Tests only: never wire it into node.
pub struct AlwaysApprove;

#[async_trait]
impl Approvals for AlwaysApprove {
    async fn passkey_options(
        &self,
        _account: Uuid,
        _fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        _request: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "method": "totp",
            "account": account,
            "fingerprint": fingerprint.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            "at": 0,
            "totp_step": 0,
        }))
    }
}

/// Owner accounts in memory. `start_session` returns a fixed test cookie.
#[derive(Default)]
pub struct MemoryOwnerAccounts {
    /// Every account, in creation order.
    pub accounts: Mutex<Vec<OwnerAccount>>,
    /// Every session started: `(account, method)`.
    pub sessions: Mutex<Vec<(Uuid, String)>>,
}

#[async_trait]
impl OwnerAccounts for MemoryOwnerAccounts {
    async fn by_email(&self, email: &str) -> anyhow::Result<Option<OwnerAccount>> {
        let email = email.trim().to_lowercase();
        let accounts = self.accounts.lock().expect("accounts lock");
        Ok(accounts.iter().find(|a| a.email == email).cloned())
    }

    async fn create(&self, email: &str, display_name: &str) -> anyhow::Result<Uuid> {
        let email = email.trim().to_lowercase();
        let mut accounts = self.accounts.lock().expect("accounts lock");
        anyhow::ensure!(
            accounts.iter().all(|a| a.email != email),
            "that email already has an account"
        );
        let account = Uuid::new_v4();
        accounts.push(OwnerAccount {
            account,
            email,
            display_name: display_name.to_owned(),
            has_passkey: false,
            has_password: false,
        });
        Ok(account)
    }

    async fn start_session(
        &self,
        account: Uuid,
        method: &str,
    ) -> anyhow::Result<http::HeaderValue> {
        self.sessions
            .lock()
            .expect("sessions lock")
            .push((account, method.to_owned()));
        Ok(http::HeaderValue::from_static(
            "agx_session=test; Path=/; HttpOnly; SameSite=Lax",
        ))
    }
}

/// Blobs in memory.
#[derive(Default)]
pub struct MemoryBlobs {
    blobs: Mutex<HashMap<String, Bytes>>,
}

#[async_trait]
impl Blobs for MemoryBlobs {
    async fn put(&self, bytes: Bytes) -> anyhow::Result<String> {
        let digest = Digest::of(&bytes).to_string();
        self.blobs
            .lock()
            .expect("blobs lock")
            .insert(digest.clone(), bytes);
        Ok(digest)
    }

    async fn get(&self, digest: &str) -> anyhow::Result<Option<Bytes>> {
        Ok(self.blobs.lock().expect("blobs lock").get(digest).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostMatch, Mounted};
    use axum::{Router, routing::get};

    #[tokio::test]
    async fn a_module_mounts_and_its_host_matches() -> anyhow::Result<()> {
        let ctx = context().await?;
        let ops = format!("ops.{}", ctx.platform_host);
        let mounted = Mounted::default()
            .platform(Router::new().route("/hello", get(|| async { "hi" })))
            .host(
                HostMatch::Exact(ops.clone()),
                Router::new().route("/", get(|| async { "ops" })),
            );
        assert_eq!(mounted.hosts.len(), 1);
        assert!(mounted.hosts[0].matcher.serves(&ops).await?);
        assert!(!mounted.hosts[0].matcher.serves("other.localhost").await?);
        Ok(())
    }

    #[tokio::test]
    async fn fakes_round_trip() -> anyhow::Result<()> {
        let ctx = context().await?;
        let digest = ctx.blobs.put(Bytes::from_static(b"cake")).await?;
        assert_eq!(
            ctx.blobs.get(&digest).await?,
            Some(Bytes::from_static(b"cake"))
        );
        assert_eq!(ctx.owners.owner(&HeaderMap::new()).await?, None);
        assert!(
            ctx.approvals
                .passkey_options(Uuid::nil(), [0; 32])
                .await?
                .is_none()
        );
        Ok(())
    }
}
