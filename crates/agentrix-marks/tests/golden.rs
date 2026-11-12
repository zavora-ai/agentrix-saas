//! Golden tests: one generator, two builds (ADR-031).
//!
//! `tests/golden/cases.json` lists marks and team marks across the three sample sites, every
//! family, the size breaks (20, 28, 40, 64, 140), monograms, escaping and extreme seeds.
//! - `rust_matches_pinned_digests` pins each SVG's SHA-256 in `tests/golden/digests.json`, so a
//!   change to the generator is a deliberate, reviewed change (`UPDATE_GOLDEN=1` rewrites it).
//! - `javascript_matches_rust` renders the same cases with `assets/marks.js` under Node and
//!   compares every case byte for byte. Without Node it says so and passes; CI runs it where
//!   Node exists, and `node tests/render.mjs --check` checks the JS build on its own.

use std::path::PathBuf;
use std::process::Command;

use agentrix_marks::{MarkSpec, MarkState, Render, TeamMark, render, render_team};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Case {
    Mark {
        name: String,
        spec: MarkSpec,
        size: f64,
        state: MarkState,
        font: String,
        uid: String,
    },
    Team {
        name: String,
        team: TeamMark,
        size: f64,
        state: MarkState,
        busy: Vec<String>,
        font: String,
    },
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn rendered() -> Vec<(String, String)> {
    let cases: Vec<Case> =
        serde_json::from_str(&std::fs::read_to_string(dir().join("golden/cases.json")).unwrap())
            .unwrap();
    cases
        .into_iter()
        .map(|c| match c {
            Case::Mark {
                name,
                spec,
                size,
                state,
                font,
                uid,
            } => {
                spec.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
                let svg = render(
                    &spec,
                    &Render {
                        size,
                        state,
                        font: &font,
                        uid: &uid,
                    },
                );
                (name, svg)
            }
            Case::Team {
                name,
                team,
                size,
                state,
                busy,
                font,
            } => {
                let busy: Vec<&str> = busy.iter().map(String::as_str).collect();
                (name, render_team(&team, size, state, &busy, &font))
            }
        })
        .collect()
}

fn sha(s: &str) -> String {
    hex(&Sha256::digest(s.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn rust_matches_pinned_digests() {
    let got: serde_json::Map<String, serde_json::Value> = rendered()
        .into_iter()
        .map(|(n, svg)| (n, serde_json::Value::String(sha(&svg))))
        .collect();
    let path = dir().join("golden/digests.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let text = serde_json::to_string_pretty(&got).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
        return;
    }
    let pinned: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        pinned.len(),
        got.len(),
        "case count changed; rerun with UPDATE_GOLDEN=1"
    );
    for (name, digest) in &got {
        assert_eq!(
            pinned.get(name),
            Some(digest),
            "{name} changed; if that's intended, rerun with UPDATE_GOLDEN=1 and review the SVG"
        );
    }
}

#[test]
fn javascript_matches_rust() {
    let node = Command::new("node")
        .arg(dir().join("render.mjs"))
        .current_dir(dir())
        .output();
    let out = match node {
        Ok(out) => out,
        Err(e) => {
            eprintln!(
                "node isn't available ({e}); run `node tests/render.mjs --check` where it is"
            );
            return;
        }
    };
    assert!(
        out.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    #[derive(Deserialize)]
    struct Js {
        name: String,
        svg: String,
    }
    let js: Vec<Js> = serde_json::from_slice(&out.stdout).unwrap();
    let rust = rendered();
    assert_eq!(js.len(), rust.len());
    for (j, (name, svg)) in js.iter().zip(&rust) {
        assert_eq!(&j.name, name);
        if &j.svg != svg {
            let at = j
                .svg
                .bytes()
                .zip(svg.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(j.svg.len().min(svg.len()));
            let lo = at.saturating_sub(60);
            panic!(
                "{name}: JS and Rust differ at byte {at}\n  js:   …{}\n  rust: …{}",
                &j.svg[lo..(at + 60).min(j.svg.len())],
                &svg[lo..(at + 60).min(svg.len())]
            );
        }
    }
}

#[test]
fn marks_keep_the_platform_rules() {
    for (name, svg) in rendered() {
        // Never a face or a photo: the mark is paths, circles for dots and text for monograms.
        assert!(!svg.contains("<image"), "{name}");
        assert!(!svg.contains("<script"), "{name}");
        assert!(
            !svg.contains("style="),
            "{name}: no inline style, so a strict CSP holds"
        );
        assert!(svg.contains(r#"aria-hidden="true""#), "{name}");
        // The silhouette is the squircle path: the halo and body are paths, not circles.
        assert!(svg.contains(r#"class="wa-halo" d="M"#), "{name}");
    }
}

#[test]
fn small_marks_carry_no_signature() {
    for (name, svg) in rendered() {
        if name.ends_with("-20") {
            assert!(!svg.contains("wa-sig"), "{name}");
        }
    }
}
