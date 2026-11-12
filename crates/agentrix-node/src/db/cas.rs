//! The content-addressed store on disk (design D4; 09 §3.3).
//!
//! Layout: `<root>/sha256/<aa>/<bb>/<64 hex>`, where `aa` and `bb` are the first two byte
//! pairs of the hex. Writes go to `<root>/tmp/` and are renamed into place after `fsync`, so a
//! reader never sees a partial blob. Reads re-hash and refuse a blob whose bytes no longer
//! match its name. With [`Cas::with_remote`] (Object Storage, R14.4), a blob missing on disk is
//! fetched from the remote, checked against its digest and kept; `crate::objects` mirrors the
//! store there and sets `location` to `local+object`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentrix_types::{Digest, SiteId};
use sqlx::SqlitePool;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Where the store can fetch a blob it no longer holds on disk (Object Storage).
#[async_trait::async_trait]
pub trait RemoteBlobs: Send + Sync + 'static {
    /// The bytes stored remotely under `digest`, unverified, or `None`.
    async fn fetch(&self, digest: &Digest) -> anyhow::Result<Option<Vec<u8>>>;
}

/// Blobs by digest under one root directory.
#[derive(Clone)]
pub struct Cas {
    root: PathBuf,
    remote: Option<Arc<dyn RemoteBlobs>>,
}

impl std::fmt::Debug for Cas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cas")
            .field("root", &self.root)
            .field("remote", &self.remote.is_some())
            .finish()
    }
}

impl Cas {
    /// Opens the store at `root`, creating its directories.
    pub async fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("sha256")).await?;
        fs::create_dir_all(root.join("tmp")).await?;
        Ok(Self { root, remote: None })
    }

    /// Reads fall back to `remote` for blobs missing on disk (R14.4).
    #[must_use]
    pub fn with_remote(mut self, remote: Arc<dyn RemoteBlobs>) -> Self {
        self.remote = Some(remote);
        self
    }

    /// Every blob on disk, in no particular order.
    pub async fn list(&self) -> io::Result<Vec<Digest>> {
        let mut out = Vec::new();
        let mut first = fs::read_dir(self.root.join("sha256")).await?;
        while let Some(a) = first.next_entry().await? {
            if !a.file_type().await?.is_dir() {
                continue;
            }
            let mut second = fs::read_dir(a.path()).await?;
            while let Some(b) = second.next_entry().await? {
                if !b.file_type().await?.is_dir() {
                    continue;
                }
                let mut files = fs::read_dir(b.path()).await?;
                while let Some(f) = files.next_entry().await? {
                    if let Some(name) = f.file_name().to_str()
                        && let Ok(d) = format!("{}{name}", Digest::PREFIX).parse::<Digest>()
                    {
                        out.push(d);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The store's root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the blob named `digest` lives.
    pub fn path_for(&self, digest: &Digest) -> PathBuf {
        let hex = digest.hex();
        self.root
            .join("sha256")
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(hex)
    }

    /// Whether the store holds `digest`.
    pub async fn contains(&self, digest: &Digest) -> bool {
        fs::try_exists(self.path_for(digest)).await.unwrap_or(false)
    }

    /// Stores `bytes` and returns their digest. Storing the same bytes twice is a no-op.
    pub async fn put(&self, bytes: &[u8]) -> io::Result<Digest> {
        let digest = Digest::of(bytes);
        let path = self.path_for(&digest);
        if fs::try_exists(&path).await? {
            return Ok(digest);
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).await?;
        }
        let tmp = self
            .root
            .join("tmp")
            .join(uuid::Uuid::now_v7().simple().to_string());
        let mut file = fs::File::create(&tmp).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        if let Err(err) = fs::rename(&tmp, &path).await {
            let _ = fs::remove_file(&tmp).await;
            return Err(err);
        }
        Ok(digest)
    }

    /// Stores `bytes` only if they hash to `expected`, as when a client uploads by digest.
    pub async fn put_expecting(&self, bytes: &[u8], expected: &Digest) -> io::Result<()> {
        let actual = Digest::of(bytes);
        if &actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected {expected}, got {actual}"),
            ));
        }
        self.put(bytes).await.map(|_| ())
    }

    /// The bytes named `digest`, or `None` if neither the disk nor the remote holds them. A
    /// blob whose bytes don't match its name is an `InvalidData` error, never returned.
    pub async fn get(&self, digest: &Digest) -> io::Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.get_local(digest).await? {
            return Ok(Some(bytes));
        }
        let Some(remote) = &self.remote else {
            return Ok(None);
        };
        let fetched = remote
            .fetch(digest)
            .await
            .map_err(|e| io::Error::other(format!("Object Storage: {e:#}")))?;
        let Some(bytes) = fetched else {
            return Ok(None);
        };
        if &Digest::of(&bytes) != digest {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the remote copy of {digest} does not match its name"),
            ));
        }
        tracing::warn!(%digest, "blob missing on disk; restored from Object Storage");
        self.put(&bytes).await?;
        Ok(Some(bytes))
    }

    /// The bytes named `digest` on disk only.
    pub async fn get_local(&self, digest: &Digest) -> io::Result<Option<Vec<u8>>> {
        let bytes = match fs::read(self.path_for(digest)).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        if &Digest::of(&bytes) != digest {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("blob {digest} is corrupt"),
            ));
        }
        Ok(Some(bytes))
    }

    /// Records in the `artifacts` index that `site` holds `digest`: its kind (for example
    /// `page`, `frame`, `evidence`, `manifest`), its content type and its size. Recording the
    /// same blob for the same site again changes nothing.
    pub async fn record(
        &self,
        pool: &SqlitePool,
        site: &SiteId,
        digest: &Digest,
        kind: &str,
        content_type: &str,
        size: u64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO artifacts (site_id, digest, kind, content_type, size, location, created_at) \
             VALUES (?, ?, ?, ?, ?, 'local', ?) ON CONFLICT (site_id, digest) DO NOTHING",
        )
        .bind(site.as_str())
        .bind(digest.to_string())
        .bind(kind)
        .bind(content_type)
        .bind(i64::try_from(size).unwrap_or(i64::MAX))
        .bind(super::now())
        .execute(pool)
        .await?;
        Ok(())
    }

    /// The bytes and content type of a blob that `site` holds, for serving (for example
    /// `GET /frames/{digest}`). `None` if the site has no such blob, so one site cannot read
    /// another's by guessing a digest.
    pub async fn get_for_site(
        &self,
        pool: &SqlitePool,
        site: &SiteId,
        digest: &Digest,
    ) -> anyhow::Result<Option<(Vec<u8>, String)>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT content_type FROM artifacts WHERE site_id = ? AND digest = ?")
                .bind(site.as_str())
                .bind(digest.to_string())
                .fetch_optional(pool)
                .await?;
        let Some((content_type,)) = row else {
            return Ok(None);
        };
        Ok(self.get(digest).await?.map(|bytes| (bytes, content_type)))
    }
}

impl Cas {
    /// A frame anyone may see: a `frame` blob of a site nobody has claimed yet, as during a
    /// free Assess (design §4.1, §7). Claimed sites' frames are served only through
    /// [`Cas::get_for_site`] to their owner.
    pub async fn get_public_frame(
        &self,
        pool: &SqlitePool,
        digest: &Digest,
    ) -> anyhow::Result<Option<(Vec<u8>, String)>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT a.content_type FROM artifacts a JOIN sites s ON s.site_id = a.site_id \
             WHERE a.digest = ? AND a.kind = 'frame' AND s.owner_account_id IS NULL LIMIT 1",
        )
        .bind(digest.to_string())
        .fetch_optional(pool)
        .await?;
        let Some((content_type,)) = row else {
            return Ok(None);
        };
        Ok(self.get(digest).await?.map(|bytes| (bytes, content_type)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_cas() -> (Cas, PathBuf) {
        let dir = std::env::temp_dir().join(format!("agentrix-cas-{}", uuid::Uuid::now_v7()));
        (Cas::open(&dir).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn put_and_get_by_digest() {
        let (cas, dir) = temp_cas().await;
        let digest = cas.put(b"<h1>Fresh bread</h1>").await.unwrap();
        assert_eq!(digest, Digest::of(b"<h1>Fresh bread</h1>"));
        assert!(cas.contains(&digest).await);
        assert_eq!(
            cas.get(&digest).await.unwrap().as_deref(),
            Some(&b"<h1>Fresh bread</h1>"[..])
        );
        assert_eq!(cas.put(b"<h1>Fresh bread</h1>").await.unwrap(), digest);
        let hex = digest.hex();
        assert!(cas.path_for(&digest).ends_with(format!(
            "sha256/{}/{}/{hex}",
            &hex[..2],
            &hex[2..4]
        )));
        assert_eq!(cas.get(&Digest::of(b"absent")).await.unwrap(), None);
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn refuses_wrong_uploads_and_corrupt_blobs() {
        let (cas, dir) = temp_cas().await;
        let wrong = cas.put_expecting(b"evil", &Digest::of(b"good")).await;
        assert_eq!(wrong.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(!cas.contains(&Digest::of(b"good")).await);

        let digest = cas.put(b"original").await.unwrap();
        fs::write(cas.path_for(&digest), b"tampered").await.unwrap();
        assert_eq!(
            cas.get(&digest).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn records_blobs_per_site() {
        let pool = super::super::open_in_memory().await.unwrap();
        sqlx::query("INSERT INTO sites (site_id, domain, old_origin, created_at) VALUES ('site_1', 'bakery.example', 'https://bakery.example', ?)")
            .bind(super::super::now())
            .execute(&pool)
            .await
            .unwrap();
        let (cas, dir) = temp_cas().await;
        let digest = cas.put(b"page").await.unwrap();
        let site = SiteId::new("site_1");
        cas.record(&pool, &site, &digest, "page", "text/html", 4)
            .await
            .unwrap();
        cas.record(&pool, &site, &digest, "page", "text/html", 4)
            .await
            .unwrap();
        let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM artifacts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let (bytes, content_type) = cas
            .get_for_site(&pool, &site, &digest)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (bytes.as_slice(), content_type.as_str()),
            (&b"page"[..], "text/html")
        );
        let other = SiteId::new("site_2");
        assert!(
            cas.get_for_site(&pool, &other, &digest)
                .await
                .unwrap()
                .is_none()
        );
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn frames_are_public_only_while_nobody_owns_the_site() {
        let pool = super::super::open_in_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO sites (site_id, domain, old_origin, created_at) \
             VALUES ('site_1', 'bakery.example', 'https://bakery.example', ?)",
        )
        .bind(super::super::now())
        .execute(&pool)
        .await
        .unwrap();
        let (cas, dir) = temp_cas().await;
        let site = SiteId::new("site_1");
        let frame = cas.put(b"png-bytes").await.unwrap();
        let page = cas.put(b"<html>").await.unwrap();
        cas.record(&pool, &site, &frame, "frame", "image/png", 9)
            .await
            .unwrap();
        cas.record(&pool, &site, &page, "page", "text/html", 6)
            .await
            .unwrap();
        assert!(cas.get_public_frame(&pool, &frame).await.unwrap().is_some());
        assert!(
            cas.get_public_frame(&pool, &page).await.unwrap().is_none(),
            "frames only"
        );

        sqlx::query(
            "INSERT INTO accounts (id, email, display_name, created_at) \
             VALUES ('acct_1', 'o@example.com', 'Owner', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        super::super::sites::set_owner(&pool, &site, "acct_1")
            .await
            .unwrap();
        assert!(
            cas.get_public_frame(&pool, &frame).await.unwrap().is_none(),
            "claimed: the owner only"
        );
        assert!(
            cas.get_for_site(&pool, &site, &frame)
                .await
                .unwrap()
                .is_some()
        );
        fs::remove_dir_all(dir).await.unwrap();
    }
}
