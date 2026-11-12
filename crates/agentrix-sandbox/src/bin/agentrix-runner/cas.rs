//! The runner's content-addressed store: input blobs from control and artifacts from rooms.
//!
//! Layout: `<root>/sha256/<hex>`. Writes go to a temporary file in `<root>/tmp` and are renamed
//! into place only after the digest is checked, so a reader never sees a partial blob.

use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::spec::Digest;

/// A directory of blobs named by their SHA-256.
#[derive(Debug, Clone)]
pub struct Cas {
    root: PathBuf,
}

impl Cas {
    /// Opens (and creates) a store at `root`.
    pub async fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("sha256")).await?;
        fs::create_dir_all(root.join("tmp")).await?;
        Ok(Self { root })
    }

    /// Where `digest` lives, whether or not it is present.
    pub fn path(&self, digest: &Digest) -> PathBuf {
        self.root.join("sha256").join(digest.hex())
    }

    /// Whether `digest` is present.
    pub async fn has(&self, digest: &Digest) -> bool {
        fs::try_exists(self.path(digest)).await.unwrap_or(false)
    }

    fn temp_path(&self) -> PathBuf {
        let n: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        self.root
            .join("tmp")
            .join(format!("{n:x}-{:x}", std::process::id()))
    }

    /// Stores `bytes` and returns their digest.
    pub async fn put_bytes(&self, bytes: &[u8]) -> io::Result<Digest> {
        let digest = Digest::of(bytes);
        if !self.has(&digest).await {
            let tmp = self.temp_path();
            fs::write(&tmp, bytes).await?;
            fs::rename(&tmp, self.path(&digest)).await?;
        }
        Ok(digest)
    }

    /// Starts a streamed write that must match `expected` (for `PUT /blobs/{digest}`).
    pub async fn writer(&self, expected: Digest, limit: u64) -> io::Result<BlobWriter> {
        let tmp = self.temp_path();
        let file = fs::File::create(&tmp).await?;
        Ok(BlobWriter {
            cas: self.clone(),
            tmp,
            file,
            hasher: Sha256::new(),
            written: 0,
            limit,
            expected,
        })
    }

    /// Copies the regular file at `src` into the store and returns its digest and size.
    /// The caller has checked that `src` is a regular file and not a link.
    pub async fn put_file(&self, src: &Path) -> io::Result<(Digest, u64)> {
        let mut input = fs::File::open(src).await?;
        let tmp = self.temp_path();
        let mut out = fs::File::create(&tmp).await?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = input.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).await?;
            total += n as u64;
        }
        out.flush().await?;
        drop(out);
        let digest = Digest::from_bytes(hasher.finalize().into());
        if self.has(&digest).await {
            fs::remove_file(&tmp).await?;
        } else {
            fs::rename(&tmp, self.path(&digest)).await?;
        }
        Ok((digest, total))
    }
}

/// A blob being uploaded; it lands in the store only if its digest matches.
#[derive(Debug)]
pub struct BlobWriter {
    cas: Cas,
    tmp: PathBuf,
    file: fs::File,
    hasher: Sha256,
    written: u64,
    limit: u64,
    expected: Digest,
}

/// Why an upload was refused.
#[derive(Debug, thiserror::Error)]
pub enum PutError {
    /// The bytes do not hash to the digest in the URL.
    #[error("the body's sha256 is {actual}, not {expected}")]
    Mismatch {
        /// The digest in the URL.
        expected: Digest,
        /// The digest of the body.
        actual: Digest,
    },
    /// The body is larger than the limit.
    #[error("the body is larger than {0} bytes")]
    TooLarge(u64),
    /// Disk trouble.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl BlobWriter {
    /// Appends a chunk.
    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), PutError> {
        self.written += chunk.len() as u64;
        if self.written > self.limit {
            return Err(PutError::TooLarge(self.limit));
        }
        self.hasher.update(chunk);
        self.file.write_all(chunk).await?;
        Ok(())
    }

    /// Checks the digest and moves the blob into place.
    pub async fn finish(mut self) -> Result<Digest, PutError> {
        self.file.flush().await?;
        let actual = Digest::from_bytes(self.hasher.clone().finalize().into());
        if actual != self.expected {
            let _ = fs::remove_file(&self.tmp).await;
            return Err(PutError::Mismatch {
                expected: self.expected,
                actual,
            });
        }
        if self.cas.has(&actual).await {
            fs::remove_file(&self.tmp).await?;
        } else {
            fs::rename(&self.tmp, self.cas.path(&actual)).await?;
        }
        Ok(actual)
    }

    /// Throws the partial upload away.
    pub async fn abort(self) {
        let _ = fs::remove_file(&self.tmp).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uploads_land_only_when_the_digest_matches() {
        let dir = tempfile::tempdir().unwrap();
        let cas = Cas::open(dir.path()).await.unwrap();
        let good = Digest::of(b"inventory");
        let mut w = cas.writer(good, 1024).await.unwrap();
        w.write(b"inventory").await.unwrap();
        assert_eq!(w.finish().await.unwrap(), good);
        assert!(cas.has(&good).await);

        let claimed = Digest::of(b"something else");
        let mut w = cas.writer(claimed, 1024).await.unwrap();
        w.write(b"inventory!").await.unwrap();
        assert!(matches!(w.finish().await, Err(PutError::Mismatch { .. })));
        assert!(!cas.has(&claimed).await);

        let mut w = cas.writer(good, 4).await.unwrap();
        assert!(matches!(
            w.write(b"too long").await,
            Err(PutError::TooLarge(4))
        ));
        w.abort().await;
    }

    #[tokio::test]
    async fn put_file_hashes_and_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        let cas = Cas::open(dir.path().join("cas")).await.unwrap();
        let f = dir.path().join("a.txt");
        tokio::fs::write(&f, b"page").await.unwrap();
        let (d1, n) = cas.put_file(&f).await.unwrap();
        let (d2, _) = cas.put_file(&f).await.unwrap();
        assert_eq!((d1, n), (Digest::of(b"page"), 4));
        assert_eq!(d1, d2);
    }
}
