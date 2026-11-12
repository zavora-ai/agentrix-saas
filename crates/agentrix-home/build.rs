//! Embeds the templates and stamps a content version on the static assets.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::{
    fs,
    path::{Path, PathBuf},
};

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

/// A small, safe CSS minifier: drops comments and collapses whitespace, and removes the spaces
/// next to `{ } ; , >` and after `:` inside declarations. The page's HTML and CSS must stay under
/// 60 KB (02 §9).
fn minify_css(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    let mut last_space = false;
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            for d in chars.by_ref() {
                if prev == '*' && d == '/' {
                    break;
                }
                prev = d;
            }
            continue;
        }
        if c.is_whitespace() {
            last_space = true;
            continue;
        }
        let tight = |ch: char| matches!(ch, '{' | '}' | ';' | ',' | '>');
        if last_space
            && !out.is_empty()
            && !tight(c)
            && !out.ends_with(tight)
            && !out.ends_with(':')
        {
            out.push(' ');
        }
        last_space = false;
        if c == '}' && out.ends_with(';') {
            out.pop();
        }
        out.push(c);
    }
    out
}

fn main() {
    minijinja_embed::embed_templates!("templates");

    let css = fs::read_to_string("static/home.css").expect("static/home.css is readable");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out.join("home.min.css"), minify_css(&css)).expect("OUT_DIR is writable");

    // The asset version changes whenever a static file changes, so pages can cache
    // `/_home/*?v=…` as immutable.
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
        "cargo:rustc-env=AGENTRIX_HOME_ASSETS_VERSION={:012x}",
        hasher.finish() & 0xffff_ffff_ffff
    );
}
