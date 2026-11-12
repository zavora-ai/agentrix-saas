//! Draws a plan of pictures through the Model Gateway and writes them with a manifest, for
//! the sample sites and the home page. Re-running only draws slots the manifest lacks.
//!
//! ```text
//! agentrix-media --gateway http://127.0.0.1:19101 --plan crates/agentrix-media/samples/kilele-bakery/plan.json
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agentrix_media::client::Gateway;
use agentrix_media::pipeline::{self, Ask, Outcome};
use agentrix_media::policy::Kind;
use anyhow::Context as _;
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(about = "Draw a plan of pictures through the Model Gateway")]
struct Args {
    /// The Model Gateway (loopback).
    #[arg(long, default_value = "http://127.0.0.1:8090")]
    gateway: String,
    /// The plan: site, look and the pictures to draw.
    #[arg(long)]
    plan: PathBuf,
    /// Draw these slots again even if the manifest has them.
    #[arg(long = "again")]
    again: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Plan {
    site: String,
    #[serde(default)]
    look: String,
    #[serde(default)]
    sample: bool,
    #[serde(default = "fast")]
    tier: String,
    images: Vec<Planned>,
}

fn fast() -> String {
    "fast".into()
}

#[derive(Debug, Clone, Deserialize)]
struct Planned {
    slot: String,
    kind: Kind,
    brief: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    seed: Option<i64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    site: String,
    images: BTreeMap<String, Value>,
    #[serde(default)]
    blocked: Vec<Value>,
}

fn file_name(slot: &str, media_type: &str) -> String {
    let stem: String = slot
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ext = match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        _ => "webp",
    };
    format!("{stem}.{ext}")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let plan: Plan = serde_json::from_slice(
        &std::fs::read(&args.plan).with_context(|| format!("reading {}", args.plan.display()))?,
    )?;
    let dir = args.plan.parent().unwrap_or(Path::new(".")).to_path_buf();
    let manifest_path = dir.join("manifest.json");
    let mut manifest: Manifest = match std::fs::read(&manifest_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(_) => Manifest {
            site: plan.site.clone(),
            ..Manifest::default()
        },
    };
    let tier: &'static str = if plan.tier == "standard" {
        "standard"
    } else {
        "fast"
    };
    let gateway = Gateway::new(reqwest::Client::new(), &args.gateway, &plan.site);
    let mut spent = 0i64;
    for item in &plan.images {
        if manifest.images.contains_key(&item.slot) && !args.again.contains(&item.slot) {
            continue;
        }
        let ask = Ask {
            kind: item.kind,
            brief: item.brief.clone(),
            look: plan.look.clone(),
            context: item.context.clone(),
            n: 1,
            seed: item.seed,
            tier,
            sample: plan.sample,
        };
        let started = std::time::Instant::now();
        let outcomes = match pipeline::make(&gateway, &ask).await {
            Ok(outcomes) => outcomes,
            Err(err) => {
                eprintln!("{}: not drawn: {err}", item.slot);
                continue;
            }
        };
        for outcome in outcomes {
            match outcome {
                Outcome::Made(made) => {
                    let name = file_name(&item.slot, &made.media_type);
                    std::fs::write(dir.join(&name), &made.bytes)?;
                    let digest = agentrix_types::Digest::of(&made.bytes).to_string();
                    spent += made.cost_micro_usd;
                    eprintln!(
                        "{}: {} ({} KB, {:.1} s) alt: {}",
                        item.slot,
                        name,
                        made.bytes.len() / 1024,
                        started.elapsed().as_secs_f64(),
                        made.description.alt
                    );
                    let mut entry = serde_json::to_value(&*made)?;
                    entry["slot"] = json!(item.slot);
                    entry["kind"] = json!(item.kind);
                    entry["brief"] = json!(item.brief);
                    entry["file"] = json!(name);
                    entry["digest"] = json!(digest);
                    entry["bytes"] = json!(made.bytes.len());
                    manifest.images.insert(item.slot.clone(), entry);
                }
                Outcome::Blocked(blocked) => {
                    eprintln!(
                        "{}: blocked by the safety check: {:?}",
                        item.slot, blocked.verdict.categories
                    );
                    let mut entry = serde_json::to_value(&blocked)?;
                    entry["slot"] = json!(item.slot);
                    manifest.blocked.push(entry);
                }
            }
        }
        let mut text = serde_json::to_vec_pretty(&manifest)?;
        text.push(b'\n');
        std::fs::write(&manifest_path, text)?;
    }
    eprintln!("drawing cost ${:.4}", spent as f64 / 1e6);
    Ok(())
}
