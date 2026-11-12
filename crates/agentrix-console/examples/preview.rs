//! Writes every sample page to a folder, with `/static/` beside it, for checking in a browser:
//! `cargo run -p agentrix-console --example preview -- <dir>`, then serve `<dir>` over HTTP.

#[path = "../tests/common/mod.rs"]
mod common;

use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        env::args()
            .nth(1)
            .unwrap_or_else(|| "target/console-preview".into()),
    );
    fs::create_dir_all(out.join("static"))?;
    let console = agentrix_console::Console::new()?;
    let mut index = String::from("<!doctype html><meta charset=utf-8><title>Previews</title><ul>");
    for (name, page, view) in common::all() {
        fs::write(
            out.join(format!("{name}.html")),
            console.render(page, &view)?,
        )?;
        index.push_str(&format!("<li><a href=\"{name}.html\">{name}</a></li>"));
    }
    fs::write(out.join("index.html"), index + "</ul>")?;
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
    for entry in fs::read_dir(here)? {
        let path = entry?.path();
        fs::copy(
            &path,
            out.join("static").join(path.file_name().expect("a file")),
        )?;
    }
    println!("wrote {}", out.display());
    Ok(())
}
