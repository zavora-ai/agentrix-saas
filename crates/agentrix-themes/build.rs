//! Embeds every file in `static/` and stamps a content version on them, so pages can cache
//! `/_s/*?v=…` for good.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write as _;
use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new("static");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut paths: Vec<_> = fs::read_dir(dir)
        .expect("static/ is readable")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let mut hasher = DefaultHasher::new();
    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("assets.rs");
    let mut table = fs::File::create(&out).expect("writing assets.rs");
    writeln!(
        table,
        "/// Every embedded asset: name, bytes.\npub static ASSETS: &[(&str, &[u8])] = &["
    )
    .unwrap();
    for path in &paths {
        println!("cargo:rerun-if-changed={}", path.display());
        path.hash(&mut hasher);
        fs::read(path).expect("asset is readable").hash(&mut hasher);
        let name = path.file_name().unwrap().to_string_lossy();
        let abs = fs::canonicalize(path).unwrap();
        writeln!(
            table,
            "    ({name:?}, include_bytes!({:?})),",
            abs.display().to_string()
        )
        .unwrap();
    }
    writeln!(table, "];").unwrap();
    println!(
        "cargo:rustc-env=AGENTRIX_THEMES_VERSION={:012x}",
        hasher.finish() & 0xffff_ffff_ffff
    );
}
