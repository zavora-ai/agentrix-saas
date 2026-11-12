//! Embeds the templates and stamps a content version on the static assets.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// Every file under `dir`, recursively.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display())) {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    minijinja_embed::embed_templates!("templates");

    // The asset version changes whenever a static file changes, so pages can cache
    // `/static/*?v=…` as immutable.
    let mut paths = Vec::new();
    files(Path::new("static"), &mut paths);
    paths.sort();
    let mut hasher = DefaultHasher::new();
    for path in &paths {
        println!("cargo:rerun-if-changed={}", path.display());
        path.hash(&mut hasher);
        fs::read(path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
            .hash(&mut hasher);
    }
    println!(
        "cargo:rustc-env=AGENTRIX_ONBOARDING_ASSETS_VERSION={:012x}",
        hasher.finish() & 0xffff_ffff_ffff
    );
}
