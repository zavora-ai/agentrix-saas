//! Embeds the templates and the sample bundles (`fixtures/samples/<site>/`).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    minijinja_embed::embed_templates!("templates");

    let root = Path::new("../../fixtures/samples");
    let mut table =
        fs::File::create(Path::new(&env::var("OUT_DIR").unwrap()).join("bundles.rs")).unwrap();
    writeln!(table, "/// Embedded sample bundles: folder, path inside it, contents.\npub static BUNDLES: &[(&str, &str, &str)] = &[").unwrap();
    for (_, folder) in [
        ("", "kilele-bakery"),
        ("", "classy-perfumes"),
        ("", "tana-valley-times"),
    ] {
        let base = root.join(folder);
        let mut files = Vec::new();
        walk(&base, &mut files);
        files.sort();
        for file in files {
            let rel = file
                .strip_prefix(&base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if !(rel == "manifest.json" || rel.ends_with(".jsonl")) {
                continue;
            }
            let abs = fs::canonicalize(&file).unwrap();
            println!("cargo:rerun-if-changed={}", abs.display());
            writeln!(
                table,
                "    ({folder:?}, {rel:?}, include_str!({:?})),",
                abs.display().to_string()
            )
            .unwrap();
        }
    }
    writeln!(table, "];").unwrap();
}
