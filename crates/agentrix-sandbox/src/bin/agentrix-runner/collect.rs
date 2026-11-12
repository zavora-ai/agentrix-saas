//! Reading what a room left in `/out`.
//!
//! The room owned `/out` and could have planted links to anything on the host. Collection runs
//! only after the room is destroyed, so nothing changes under it, and it never follows a link:
//! every entry is checked with `symlink_metadata`, and only regular files and directories count.

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::cas::Cas;
use crate::spec::ArtifactRef;

/// Deepest directory nesting walked.
pub const MAX_DEPTH: usize = 16;
/// Most files collected from one room.
pub const MAX_FILES: usize = 10_000;

/// A file found under `/out`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its path on the host.
    pub path: PathBuf,
    /// Its path relative to `/out`, with `/` separators.
    pub name: String,
    /// Its size.
    pub bytes: u64,
}

/// Lists the regular files under `root`, skipping links, devices, sockets and FIFOs, and
/// stopping at [`MAX_DEPTH`] and [`MAX_FILES`]. Returns the files and how many entries were
/// skipped.
pub fn walk(root: &Path) -> io::Result<(Vec<Found>, usize)> {
    let mut found = Vec::new();
    let mut skipped = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let meta = std::fs::symlink_metadata(&dir)?;
        if !meta.file_type().is_dir() {
            skipped += 1;
            continue;
        }
        let mut entries: Vec<_> = std::fs::read_dir(&dir)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            let ft = meta.file_type();
            if ft.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    stack.push((path, depth + 1));
                } else {
                    skipped += 1;
                }
            } else if ft.is_file() {
                if found.len() >= MAX_FILES {
                    skipped += 1;
                    continue;
                }
                let Some(name) = relative_name(root, &path) else {
                    skipped += 1;
                    continue;
                };
                found.push(Found {
                    path,
                    name,
                    bytes: meta.len(),
                });
            } else {
                skipped += 1;
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((found, skipped))
}

/// Total size of the regular files under `root`, for the `/out` cap while the room runs.
/// Links are not followed, so a link to a large host file costs nothing.
pub fn size(root: &Path) -> u64 {
    walk(root).map_or(0, |(files, _)| files.iter().map(|f| f.bytes).sum())
}

fn relative_name(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str()?.to_string()),
            _ => return None,
        }
    }
    let name = parts.join("/");
    (!name.is_empty() && name.len() <= 512).then_some(name)
}

/// Moves every file under `root` into the store. Call only after the room is destroyed.
pub async fn collect(root: &Path, cas: &Cas) -> io::Result<(Vec<ArtifactRef>, usize)> {
    let root = root.to_path_buf();
    let (files, skipped) = tokio::task::spawn_blocking(move || walk(&root))
        .await
        .map_err(io::Error::other)??;
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        // Checked again just before reading: still a regular file, not a link.
        let meta = tokio::fs::symlink_metadata(&f.path).await?;
        if !meta.file_type().is_file() {
            continue;
        }
        let (digest, bytes) = cas.put_file(&f.path).await?;
        out.push(ArtifactRef {
            digest,
            name: f.name,
            bytes,
        });
    }
    Ok((out, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn links_are_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(out.join("pages")).unwrap();
        std::fs::write(out.join("pages/index.html"), b"<h1>Kilele</h1>").unwrap();
        let secret = dir.path().join("host-secret");
        std::fs::write(&secret, b"do not copy").unwrap();
        std::os::unix::fs::symlink(&secret, out.join("stolen.txt")).unwrap();
        std::os::unix::fs::symlink(dir.path(), out.join("escape")).unwrap();

        let (files, skipped) = walk(&out).unwrap();
        assert_eq!(
            files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["pages/index.html"]
        );
        assert_eq!(skipped, 2);
        assert_eq!(size(&out), 15);

        let cas = Cas::open(dir.path().join("cas")).await.unwrap();
        let (arts, _) = collect(&out, &cas).await.unwrap();
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].name, "pages/index.html");
    }

    #[test]
    fn depth_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = dir.path().to_path_buf();
        for i in 0..(MAX_DEPTH + 3) {
            p = p.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("deep.txt"), b"x").unwrap();
        let (files, skipped) = walk(dir.path()).unwrap();
        assert!(files.is_empty());
        assert_eq!(skipped, 1);
    }
}
