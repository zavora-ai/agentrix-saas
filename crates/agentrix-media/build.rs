//! Embeds the sample pictures (`samples/<site>/manifest.json` and the files it names), so
//! node seeds them into the content store at startup without redrawing anything.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let root =
        Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("samples");
    println!("cargo:rerun-if-changed=samples");
    let mut out = String::from(
        "/// The sample sites' pictures, embedded at build time.\npub static SAMPLES: &[SampleSite] = &[\n",
    );
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join("manifest.json").is_file())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    for dir in dirs {
        let manifest_path = dir.join("manifest.json");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
        let text = std::fs::read_to_string(&manifest_path).expect("manifest");
        let manifest: serde_json::Value = serde_json::from_str(&text).expect("manifest is JSON");
        let mut files: Vec<String> = manifest["images"]
            .as_object()
            .map(|images| {
                images
                    .values()
                    .filter_map(|i| i["file"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        writeln!(
            out,
            "    SampleSite {{ manifest: include_str!({:?}), files: &[",
            manifest_path.display().to_string()
        )
        .unwrap();
        for file in files {
            let path = dir.join(&file);
            assert!(
                path.is_file(),
                "{} names {file}, which is missing",
                manifest_path.display()
            );
            writeln!(
                out,
                "        ({file:?}, include_bytes!({:?})),",
                path.display().to_string()
            )
            .unwrap();
        }
        out.push_str("    ] },\n");
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").expect("out dir")).join("samples.rs");
    std::fs::write(dest, out).expect("write samples.rs");
}
