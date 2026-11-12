//! Releases and evidence on Vultr Object Storage (R14.4, S3-04; integration lane).
//!
//! The CAS on the control VM stays the source of truth. [`ObjectMirror`] copies every blob in
//! it to a bucket: release files and manifests, frames, evidence and uploads. It also copies
//! every signed receipt, byte for byte as `GET /receipts/{digest}` serves it. A copied blob's
//! `artifacts.location` becomes `local+object`. [`Cas::with_remote`] makes reads fall back to
//! the bucket when a blob is missing on disk. The bytes are checked against their digest
//! before they are kept or served, so a release keeps serving even if the control VM loses a
//! file.
//!
//! Objects are written at `cas/sha256/<hex>` and `receipts/<hex>.json`, path-style on Vultr's
//! S3 endpoint and signed with AWS Signature V4. Hosting adds no new trust: a blob is named by
//! its digest, and a receipt carries its own signature.
//!
//! Configuration is a JSON file named by `AGENTRIX_OBJECTS_FILE`:
//! `{endpoint, bucket, region?, access_key, secret_key}`. On control it is a systemd credential,
//! so the keys never sit in node's environment. Without the file, nothing is mirrored and reads
//! stay local.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agentrix_types::Digest;
use anyhow::{Context as _, bail};
use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;

use crate::db::cas::{Cas, RemoteBlobs};

/// The mirror's bookkeeping, created by node itself (idempotent).
pub const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS object_mirror (
    key       TEXT PRIMARY KEY,
    digest    TEXT NOT NULL,
    mirrored_at TEXT NOT NULL
);";

/// `AGENTRIX_OBJECTS_FILE`'s contents.
#[derive(Clone, Deserialize)]
pub struct ObjectsConfig {
    /// `https://ams1.vultrobjects.com`.
    pub endpoint: String,
    /// The bucket.
    pub bucket: String,
    /// The SigV4 region; Vultr accepts `us-east-1`.
    #[serde(default = "default_region")]
    pub region: String,
    /// S3 access key.
    pub access_key: String,
    /// S3 secret key.
    pub secret_key: String,
}

fn default_region() -> String {
    "us-east-1".into()
}

impl std::fmt::Debug for ObjectsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectsConfig")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

impl ObjectsConfig {
    /// Reads the file named by `AGENTRIX_OBJECTS_FILE`, if set.
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(path) = std::env::var_os("AGENTRIX_OBJECTS_FILE") else {
            return Ok(None);
        };
        Self::read(Path::new(&path))
    }

    /// Reads a config file. An empty file means Object Storage is not configured: deploy.sh
    /// writes an empty root-only objects.json when no bucket is set up (systemd 255 rejects an
    /// empty `SetCredential=` fallback), so a fresh deployment without a bucket still starts
    /// (R14.3).
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        Self::read(path)?.with_context(|| format!("{} is empty", path.display()))
    }

    fn read(path: &Path) -> anyhow::Result<Option<Self>> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if text.trim().is_empty() {
            return Ok(None);
        }
        serde_json::from_str(&text)
            .map(Some)
            .with_context(|| format!("parsing {}", path.display()))
    }
}

/// A bucket on an S3-compatible endpoint.
#[derive(Debug, Clone)]
pub struct ObjectStore {
    http: reqwest::Client,
    config: Arc<ObjectsConfig>,
    host: String,
}

type HmacSha256 = Hmac<Sha256>;

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// AWS Signature V4 for one S3 request: returns the `Authorization` header value.
/// `headers` are the signed headers, lower-case names, without duplicates.
#[allow(clippy::too_many_arguments)]
pub fn sign_v4(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(&str, &str)],
    payload_hash: &str,
    amz_date: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
) -> String {
    let mut sorted: Vec<(&str, &str)> = headers.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let canonical_headers: String = sorted
        .iter()
        .map(|(k, v)| format!("{k}:{}\n", v.trim()))
        .collect();
    let signed_headers = sorted.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(";");
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let date = &amz_date[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

impl ObjectStore {
    /// A store for `config`.
    pub fn new(http: reqwest::Client, config: ObjectsConfig) -> anyhow::Result<Self> {
        let url = url::Url::parse(&config.endpoint).context("the Object Storage endpoint")?;
        let host = match (url.host_str(), url.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_owned(),
            _ => bail!("the Object Storage endpoint has no host"),
        };
        if config.bucket.is_empty()
            || !config
                .bucket
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            bail!("the bucket name must be lower-case letters, digits and hyphens");
        }
        Ok(Self {
            http,
            config: Arc::new(config),
            host,
        })
    }

    /// The bucket's name.
    pub fn bucket(&self) -> &str {
        &self.config.bucket
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> anyhow::Result<reqwest::Response> {
        let amz_date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let payload_hash = sha256_hex(&body);
        let mut headers: Vec<(&str, &str)> = vec![
            ("host", &self.host),
            ("x-amz-content-sha256", &payload_hash),
            ("x-amz-date", &amz_date),
        ];
        if let Some(ct) = content_type {
            headers.push(("content-type", ct));
        }
        let auth = sign_v4(
            method.as_str(),
            path,
            "",
            &headers,
            &payload_hash,
            &amz_date,
            &self.config.region,
            &self.config.access_key,
            &self.config.secret_key,
        );
        let url = format!("{}{path}", self.config.endpoint.trim_end_matches('/'));
        let mut req = self
            .http
            .request(method, url)
            .timeout(Duration::from_secs(60))
            // Always explicit: an empty body (a zero-byte blob) otherwise goes without one and
            // Vultr answers 411 Length Required.
            .header("content-length", body.len().to_string())
            .header("x-amz-content-sha256", &payload_hash)
            .header("x-amz-date", &amz_date)
            .header("authorization", auth);
        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }
        Ok(req.body(body).send().await?)
    }

    fn object_path(&self, key: &str) -> anyhow::Result<String> {
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_'))
            || key.contains("..")
        {
            bail!("unusable object key {key:?}");
        }
        Ok(format!("/{}/{key}", self.config.bucket))
    }

    /// Creates the bucket if it does not exist.
    pub async fn ensure_bucket(&self) -> anyhow::Result<()> {
        let path = format!("/{}", self.config.bucket);
        let head = self
            .send(reqwest::Method::HEAD, &path, Vec::new(), None)
            .await?;
        if head.status().is_success() {
            return Ok(());
        }
        let made = self
            .send(reqwest::Method::PUT, &path, Vec::new(), None)
            .await?;
        let status = made.status();
        if status.is_success() || status == reqwest::StatusCode::CONFLICT {
            return Ok(());
        }
        bail!(
            "creating bucket {}: {status} {}",
            self.config.bucket,
            made.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        )
    }

    /// Writes one object.
    pub async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> anyhow::Result<()> {
        let path = self.object_path(key)?;
        let res = self
            .send(reqwest::Method::PUT, &path, bytes, Some(content_type))
            .await?;
        let status = res.status();
        if !status.is_success() {
            bail!(
                "PUT {key}: {status} {}",
                res.text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(200)
                    .collect::<String>()
            );
        }
        Ok(())
    }

    /// Reads one object, or `None` if the bucket has no such key.
    pub async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let path = self.object_path(key)?;
        let res = self
            .send(reqwest::Method::GET, &path, Vec::new(), None)
            .await?;
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            bail!("GET {key}: {status}");
        }
        Ok(Some(res.bytes().await?.to_vec()))
    }
}

/// Where a CAS blob lives in the bucket.
pub fn blob_key(digest: &Digest) -> String {
    format!("cas/sha256/{}", digest.hex())
}

/// Where a receipt lives in the bucket.
pub fn receipt_key(digest: &Digest) -> String {
    format!("receipts/{}.json", digest.hex())
}

#[async_trait]
impl RemoteBlobs for ObjectStore {
    async fn fetch(&self, digest: &Digest) -> anyhow::Result<Option<Vec<u8>>> {
        self.get(&blob_key(digest)).await
    }
}

/// Copies the CAS and the receipts to the bucket.
pub struct ObjectMirror {
    pool: SqlitePool,
    cas: Arc<Cas>,
    store: ObjectStore,
}

impl ObjectMirror {
    /// A mirror of `cas` and the receipts in `pool` to `store`.
    pub fn new(pool: SqlitePool, cas: Arc<Cas>, store: ObjectStore) -> Self {
        Self { pool, cas, store }
    }

    /// Mirrors now, then every `every`. Failures are logged and retried.
    pub fn spawn(self: Arc<Self>, every: Duration) {
        tokio::spawn(async move {
            let mut bucket_ready = false;
            loop {
                if !bucket_ready {
                    match self.store.ensure_bucket().await {
                        Ok(()) => bucket_ready = true,
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "Object Storage bucket not ready")
                        }
                    }
                }
                if bucket_ready {
                    match self.run_once().await {
                        Ok((0, 0)) => {}
                        Ok((blobs, receipts)) => tracing::info!(
                            blobs,
                            receipts,
                            bucket = self.store.bucket(),
                            "mirrored to Object Storage"
                        ),
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "Object Storage mirror pass failed")
                        }
                    }
                }
                tokio::time::sleep(every).await;
            }
        });
    }

    /// One pass: every blob and receipt not yet in the bucket. Returns (blobs, receipts) copied.
    pub async fn run_once(&self) -> anyhow::Result<(usize, usize)> {
        sqlx::raw_sql(SCHEMA_SQL).execute(&self.pool).await?;
        let done: HashSet<String> = sqlx::query_as::<_, (String,)>("SELECT key FROM object_mirror")
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(|(k,)| k)
            .collect();

        let mut blobs = 0;
        for digest in self.cas.list().await? {
            let key = blob_key(&digest);
            if done.contains(&key) {
                continue;
            }
            let Some(bytes) = self.cas.get_local(&digest).await? else {
                continue;
            };
            // One object that won't copy must not stop the rest, the receipts included.
            if let Err(e) = self
                .store
                .put(&key, bytes, "application/octet-stream")
                .await
            {
                tracing::warn!(error = %format!("{e:#}"), %key, "an object did not mirror; retrying next pass");
                continue;
            }
            self.mark(&key, &digest).await?;
            sqlx::query("UPDATE artifacts SET location = 'local+object' WHERE digest = ?")
                .bind(digest.to_string())
                .execute(&self.pool)
                .await?;
            blobs += 1;
        }

        let mut receipts = 0;
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT digest, body FROM receipts ORDER BY issued_at")
                .fetch_all(&self.pool)
                .await?;
        for (digest, body) in rows {
            let digest: Digest = digest.parse().context("a stored receipt digest")?;
            let key = receipt_key(&digest);
            if done.contains(&key) {
                continue;
            }
            if let Err(e) = self
                .store
                .put(&key, body.into_bytes(), "application/json")
                .await
            {
                tracing::warn!(error = %format!("{e:#}"), %key, "a receipt did not mirror; retrying next pass");
                continue;
            }
            self.mark(&key, &digest).await?;
            receipts += 1;
        }
        Ok((blobs, receipts))
    }

    async fn mark(&self, key: &str, digest: &Digest) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO object_mirror (key, digest, mirrored_at) VALUES (?, ?, ?)",
        )
        .bind(key)
        .bind(digest.to_string())
        .bind(crate::db::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Bytes;
    use axum::extract::{Path as AxPath, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::any;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// AWS's published SigV4 example for S3 (GET /test.txt with a Range header).
    #[test]
    fn signs_like_aws_example() {
        let auth = sign_v4(
            "GET",
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                (
                    "x-amz-content-sha256",
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                ),
                ("x-amz-date", "20130524T000000Z"),
            ],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "20130524T000000Z",
            "us-east-1",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        assert!(
            auth.ends_with(
                "Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
            ),
            "{auth}"
        );
        assert!(auth.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    /// Live check against the configured bucket: `AGENTRIX_OBJECTS_FILE=… cargo test -p
    /// agentrix-node objects::tests::live -- --ignored`.
    #[tokio::test]
    #[ignore = "needs AGENTRIX_OBJECTS_FILE and the network"]
    async fn live() {
        let cfg = ObjectsConfig::from_env()
            .unwrap()
            .expect("AGENTRIX_OBJECTS_FILE");
        let store = ObjectStore::new(reqwest::Client::new(), cfg).unwrap();
        store.ensure_bucket().await.unwrap();
        let bytes = format!("agentrix live check {}", Utc::now()).into_bytes();
        let digest = Digest::of(&bytes);
        store
            .put(&blob_key(&digest), bytes.clone(), "text/plain")
            .await
            .unwrap();
        assert_eq!(store.get(&blob_key(&digest)).await.unwrap(), Some(bytes));
        assert_eq!(store.get("cas/sha256/missing").await.unwrap(), None);
    }

    #[test]
    fn an_empty_credential_means_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("objects.json");
        std::fs::write(&empty, "").unwrap();
        assert!(
            ObjectsConfig::read(&empty).unwrap().is_none(),
            "a fresh deploy without a bucket starts"
        );
        std::fs::write(&empty, "{not json").unwrap();
        assert!(
            ObjectsConfig::read(&empty).is_err(),
            "a broken file is an error, not silently off"
        );
    }

    type Bucket = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    async fn fake_s3(
        State(bucket): State<Bucket>,
        method: axum::http::Method,
        AxPath(path): AxPath<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> (StatusCode, Vec<u8>) {
        let signed = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.starts_with("AWS4-HMAC-SHA256 Credential="));
        if !signed {
            return (StatusCode::FORBIDDEN, Vec::new());
        }
        let mut b = bucket.lock().unwrap();
        match method.as_str() {
            "PUT" if !headers.contains_key("content-length") => {
                (StatusCode::LENGTH_REQUIRED, Vec::new())
            }
            "PUT" => {
                b.insert(path, body.to_vec());
                (StatusCode::OK, Vec::new())
            }
            "GET" | "HEAD" => match b.get(&path) {
                Some(v) => (StatusCode::OK, v.clone()),
                None if !path.contains('/') => (StatusCode::NOT_FOUND, Vec::new()),
                None => (StatusCode::NOT_FOUND, Vec::new()),
            },
            _ => (StatusCode::METHOD_NOT_ALLOWED, Vec::new()),
        }
    }

    #[tokio::test]
    async fn mirrors_blobs_and_receipts_and_reads_fall_back_to_the_bucket() {
        let bucket: Bucket = Arc::default();
        let app = Router::new()
            .route("/{*path}", any(fake_s3))
            .with_state(bucket.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let store = ObjectStore::new(
            reqwest::Client::new(),
            ObjectsConfig {
                endpoint,
                bucket: "agx-test".into(),
                region: default_region(),
                access_key: "AK".into(),
                secret_key: "SK".into(),
            },
        )
        .unwrap();
        store.ensure_bucket().await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        let cas = Arc::new(Cas::open(dir.path().join("cas")).await.unwrap());
        let page = b"<h1>Kilele Bakery</h1>".to_vec();
        let digest = cas.put(&page).await.unwrap();
        cas.put(b"").await.unwrap(); // a zero-byte blob: Vultr wants Content-Length: 0 (411 otherwise)
        let pool = crate::db::open_in_memory().await.unwrap();

        let mirror = ObjectMirror::new(pool.clone(), cas.clone(), store.clone());
        assert_eq!(mirror.run_once().await.unwrap(), (2, 0));
        assert_eq!(mirror.run_once().await.unwrap(), (0, 0), "nothing twice");
        assert_eq!(
            bucket
                .lock()
                .unwrap()
                .get(&format!("agx-test/{}", blob_key(&digest))),
            Some(&page)
        );

        // The control VM loses the file: a CAS backed by the bucket still serves it, checked.
        std::fs::remove_file(cas.path_for(&digest)).unwrap();
        assert_eq!(
            cas.get(&digest).await.unwrap(),
            None,
            "plain CAS has nothing"
        );
        let backed = Cas::open(dir.path().join("cas"))
            .await
            .unwrap()
            .with_remote(Arc::new(store.clone()));
        assert_eq!(backed.get(&digest).await.unwrap(), Some(page.clone()));
        assert!(
            backed.path_for(&digest).exists(),
            "the blob is restored on disk"
        );

        // A tampered object in the bucket is refused, never served.
        let other = b"<h1>About</h1>".to_vec();
        let other_digest = Digest::of(&other);
        bucket.lock().unwrap().insert(
            format!("agx-test/{}", blob_key(&other_digest)),
            b"<h1>hacked</h1>".to_vec(),
        );
        assert!(
            backed.get(&other_digest).await.is_err(),
            "bytes that don't match their name are refused"
        );
    }
}
