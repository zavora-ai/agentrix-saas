//! S3-10 step 1, the bake-off: which models serve `build` and `vision-parity` on Vultr, measured
//! against Claude on the same pages (design D10b). Live model calls, so it is ignored by default.
//!
//! Run it with the provider keys in the environment (never printed), a built
//! `agentrix-models`, and Google Chrome for the screenshots:
//!
//! ```text
//! BAKEOFF_DIR=/tmp/bakeoff BAKEOFF_GATEWAY=target/debug/agentrix-models \
//!   cargo test -p agentrix-move --test bakeoff -- --ignored --nocapture
//! ```
//!
//! Per candidate it starts a local gateway with `AGENTRIX_ROUTE_<PURPOSE>` set, rebuilds the
//! fixture's clean pages through the pipeline's own `build_page` (page check, three tries,
//! feedback), and reads cost from the gateway's usage rows. Vision candidates judge the same
//! screenshots, plus one deliberately wrong pairing per page, which a good judge scores low.
//! Results go to `$BAKEOFF_DIR/results.json`. A budget guard stops at about $2.50.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agentrix_move::clean::{CleanContext, CleanSite, clean};
use agentrix_move::gateway::HttpGateway;
use agentrix_move::inventory::{MoveInputs, read_fallback};
use agentrix_move::parity::{text_parity, words_of_markdown};
use agentrix_move::ports::{CallContext, Gateway, VisionRequest};
use agentrix_move::rebuild::{Builder, build_page};
use agentrix_move::testing::{FakeGateway, MemoryBlobs};
use agentrix_move::theme::{self, PageView, SiteView, Tokens};
use agentrix_types::{MoveId, SiteId};
use futures::StreamExt as _;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

const EXPORT: &[u8] = include_bytes!("../../../fixtures/wordpress-infected/exports/dev/export.xml");
const WP_CONTENT: &[u8] =
    include_bytes!("../../../fixtures/wordpress-infected/exports/dev/wp-content.tar.gz");
const SITE: &str = "site_bakeoff";
const CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const BUDGET_MICRO_USD: i64 = 2_500_000;

const BUILD: &[(&str, &str)] = &[
    ("vultr", "deepseek-v4.1-flash-1017"),
    ("vultr", "glm-5.3"),
    ("vultr", "qwen3.8-27b"),
    ("anthropic", "claude-haiku-5-5"),
    ("anthropic", "claude-sonnet-5-5"),
];
const VISION: &[(&str, &str)] = &[
    ("vultr", "deepseek-v4.1-flash-1017"),
    ("vultr", "qwen3.8-27b"),
    ("anthropic", "claude-haiku-5-5"),
];

async fn control_db(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();
    let fresh: Option<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE name = 'sites'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    if fresh.is_none() {
        for sql in [
            include_str!("../../agentrix-node/src/auth/schema.sql"),
            include_str!("../../agentrix-node/src/claim/schema.sql"),
            include_str!("../../agentrix-node/src/db/migrations/0001_control.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO sites (site_id, domain, name, old_origin, fixture, created_at) \
             VALUES (?, 'kilele-bakery.example', 'Kilele Bakery', 'https://kilele-bakery.example', 1, '2026-11-08T00:00:00.000000Z')",
        )
        .bind(SITE)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool
}

async fn new_move(pool: &SqlitePool, label: &str) -> MoveId {
    let id = MoveId::new(format!("move_bakeoff_{}", label.replace([':', '.'], "_")));
    sqlx::query(
        "INSERT OR IGNORE INTO moves (move_id, site_id, state, created_at, updated_at) \
         VALUES (?, ?, 'rebuilt', '2026-11-08T00:00:00.000000Z', '2026-11-08T00:00:00.000000Z')",
    )
    .bind(id.as_str())
    .bind(SITE)
    .execute(pool)
    .await
    .unwrap();
    id
}

struct Gw(Child);

impl Drop for Gw {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn gateway(
    bin: &Path,
    db: &Path,
    port: u16,
    provider: &str,
    var: &str,
    route: &str,
    log: &Path,
) -> Gw {
    let child = Command::new(bin)
        .env("AGENTRIX_MODELS_MODE", "live")
        .env("AGENTRIX_PROVIDER", provider)
        .env("AGENTRIX_CONTROL_DB", db)
        .env("AGENTRIX_MODELS_LISTEN", format!("127.0.0.1:{port}"))
        .env("AGENTRIX_SITE_DAILY_CAP_USD", "3.0")
        .env(var, route)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log).unwrap())
        .spawn()
        .expect("start agentrix-models");
    let gw = Gw(child);
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return gw;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    drop(gw);
    panic!("the gateway on {port} did not start; see {}", log.display());
}

type UsageRow = (
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

async fn usage(pool: &SqlitePool, move_id: &MoveId) -> Value {
    let row: UsageRow = sqlx::query_as(
        "SELECT COUNT(*), SUM(tokens_in), SUM(tokens_out), SUM(reasoning_tokens), SUM(cost_micro_usd), \
         group_concat(DISTINCT outcome), group_concat(DISTINCT model) FROM usage WHERE move_id = ?",
    )
    .bind(move_id.as_str())
    .fetch_one(pool)
    .await
    .unwrap();
    json!({
        "calls": row.0, "tokens_in": row.1, "tokens_out": row.2, "reasoning_tokens": row.3,
        "cost_usd": row.4.unwrap_or(0) as f64 / 1e6, "outcomes": row.5, "models": row.6,
    })
}

async fn spent(pool: &SqlitePool) -> i64 {
    let (total,): (Option<i64>,) =
        sqlx::query_as("SELECT SUM(cost_micro_usd) FROM usage WHERE site_id = ?")
            .bind(SITE)
            .fetch_one(pool)
            .await
            .unwrap();
    total.unwrap_or(0)
}

async fn clean_site(blobs: &MemoryBlobs) -> CleanSite {
    let site = SiteId::new(SITE);
    let inputs = MoveInputs {
        domain: "kilele-bakery.example".into(),
        origin: "https://kilele-bakery.example".into(),
        site_name: Some("Kilele Bakery".into()),
        wxr: Some(blobs.insert(&site, EXPORT)),
        dump: None,
        wp_content: Some(blobs.insert(&site, WP_CONTENT)),
        crawl: None,
        fixture: true,
        look: None,
    };
    let old = read_fallback(blobs, &site, &inputs).await.unwrap();
    let inventory = old.hydrate(blobs, &site).await.unwrap();
    let fake = FakeGateway::default();
    let ctx = CleanContext {
        site: &site,
        call: CallContext {
            site: site.clone(),
            move_id: MoveId::new("move_bakeoff_clean"),
            attempt: 1,
        },
        blobs,
        gateway: &fake,
        restored: BTreeSet::new(),
        now: chrono::Utc::now(),
    };
    clean(&ctx, &old, &inventory).await.unwrap()
}

#[tokio::test]
#[ignore = "live model calls; run by hand with keys in the environment"]
async fn bakeoff() {
    let dir = PathBuf::from(
        std::env::var("BAKEOFF_DIR").unwrap_or_else(|_| "/tmp/agentrix-bakeoff".into()),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let bin = PathBuf::from(
        std::env::var("BAKEOFF_GATEWAY").expect("BAKEOFF_GATEWAY: the agentrix-models binary"),
    );
    let db = dir.join("control.db");
    let pool = control_db(&db).await;
    let blobs = MemoryBlobs::default();
    let site = clean_site(&blobs).await;
    let paths: BTreeSet<String> = site.pages.iter().map(|p| p.path.clone()).collect();
    println!("clean pages: {:?}", paths);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .unwrap();
    let mut results = json!({ "pages": paths, "build": [], "vision": [] });
    let mut sample_bodies: Vec<(String, String)> = Vec::new();

    for (i, (provider, model)) in BUILD.iter().enumerate() {
        if spent(&pool).await > BUDGET_MICRO_USD {
            println!("budget reached; stopping before {model}");
            break;
        }
        let label = format!("build-{model}");
        let move_id = new_move(&pool, &label).await;
        let port = 18190 + i as u16;
        let _gw = gateway(
            &bin,
            &db,
            port,
            provider,
            "AGENTRIX_ROUTE_BUILD",
            &format!("{provider}:{model}"),
            &dir.join(format!("{label}.log")),
        )
        .await;
        let gw = Arc::new(HttpGateway::new(
            format!("http://127.0.0.1:{port}"),
            http.clone(),
        ));
        let started = Instant::now();
        let pages: Vec<Value> = futures::stream::iter(site.pages.iter().map(|page| {
            let (gw, move_id, paths, name) = (gw.clone(), move_id.clone(), &paths, &site.site_name);
            async move {
                let ctx = CallContext {
                    site: SiteId::new(SITE),
                    move_id,
                    attempt: 1,
                };
                let t = Instant::now();
                let mut why = Vec::new();
                let body = build_page(
                    gw.as_ref(),
                    &ctx,
                    name,
                    page,
                    paths,
                    1,
                    None,
                    &mut |attempt, problem| {
                        why.push(format!(
                            "try {attempt}: {}",
                            problem.chars().take(300).collect::<String>()
                        ))
                    },
                )
                .await;
                let text = text_parity(&page.source(), &body.body_html);
                json!({
                    "path": page.path,
                    "attempts": body.attempts,
                    "first_try": body.attempts == 1 && body.flagged.is_none(),
                    "flagged": body.flagged,
                    "retry_reasons": why,
                    "by_model": matches!(body.builder, Builder::Model { .. }),
                    "text_parity": (text * 1000.0).round() / 1000.0,
                    "seconds": t.elapsed().as_secs_f64().round(),
                    "body": body.body_html,
                })
            }
        }))
        .buffer_unordered(5)
        .collect()
        .await;
        let wall = started.elapsed().as_secs_f64().round();
        let cost = usage(&pool, &move_id).await;
        let n = pages.len().max(1) as f64;
        let first = pages.iter().filter(|p| p["first_try"] == true).count();
        let mean_text = pages
            .iter()
            .map(|p| p["text_parity"].as_f64().unwrap_or(0.0))
            .sum::<f64>()
            / n;
        let summary = json!({
            "provider": provider, "model": model, "pages": pages.len(), "first_try_pass": first,
            "retries": pages.iter().map(|p| p["attempts"].as_u64().unwrap_or(1) - 1).sum::<u64>(),
            "flagged": pages.iter().filter(|p| !p["flagged"].is_null()).count(),
            "mean_text_parity": (mean_text * 1000.0).round() / 1000.0,
            "wall_seconds": wall,
            "usage": cost,
            "cost_per_page_usd": cost["cost_usd"].as_f64().unwrap_or(0.0) / n,
        });
        println!("{summary}");
        if *model == "claude-sonnet-5-5" || sample_bodies.is_empty() {
            sample_bodies = pages
                .iter()
                .map(|p| {
                    (
                        p["path"].as_str().unwrap().to_owned(),
                        p["body"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
        }
        let mut entry = summary;
        entry["page_detail"] = Value::Array(
            pages
                .into_iter()
                .map(|mut p| {
                    p.as_object_mut().unwrap().remove("body");
                    p
                })
                .collect(),
        );
        results["build"].as_array_mut().unwrap().push(entry);
        std::fs::write(
            dir.join("results.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }

    // Screenshots of one build's pages, served over loopback like room-render, taken with Chrome.
    let site_dir = dir.join("site");
    let view = SiteView {
        name: site.site_name.clone(),
        domain: "kilele-bakery.example".into(),
        lang: site.language.clone(),
        tokens: Tokens::default(),
        nav: site.nav.clone(),
        contact_href: Some("/contact/".into()),
        fixture: true,
        counting: false,
        counter: None,
    };
    let mut shots = Vec::new();
    for page in &site.pages {
        let body = sample_bodies
            .iter()
            .find(|(p, _)| *p == page.path)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        let html = theme::render_page(
            &view,
            &PageView {
                title: &page.title,
                description: None,
                path: &page.path,
                md_href: String::new(),
                body_html: &body,
                updated: None,
            },
        )
        .unwrap();
        let file = theme::page_file(&page.path);
        std::fs::create_dir_all(site_dir.join(&file).parent().unwrap()).unwrap();
        std::fs::write(site_dir.join(&file), html).unwrap();
        shots.push((page.path.clone(), file, words_of_markdown(&page.source())));
    }
    std::fs::create_dir_all(site_dir.join("assets")).unwrap();
    std::fs::write(site_dir.join("assets/site.css"), theme::SITE_CSS).unwrap();
    for media in &site.media {
        if let Ok(Some(bytes)) =
            agentrix_move::ports::Blobs::get(&blobs, &SiteId::new(SITE), &media.digest).await
        {
            let target = site_dir.join(media.path.trim_start_matches('/'));
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
    }
    let server = Command::new("python3")
        .args([
            "-m",
            "http.server",
            "18299",
            "--bind",
            "127.0.0.1",
            "--directory",
        ])
        .arg(&site_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Gw)
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut pngs = Vec::new();
    for (path, file, words) in &shots {
        let png = dir.join(format!("{}.png", file.replace('/', "__")));
        let ok = Command::new(CHROME)
            // Headless Chrome on macOS lays pages out wider than a 390 px window, which clips the
            // screenshot; 500 px is its narrowest true layout (room-render's Playwright viewport
            // is a real 390 px).
            .args([
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--window-size=500,1000",
            ])
            .arg(format!("--screenshot={}", png.display()))
            .arg(format!("http://127.0.0.1:18299/{file}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok && let Ok(bytes) = std::fs::read(&png) {
            pngs.push((path.clone(), bytes, words.clone()));
        }
    }
    drop(server);
    println!("screenshots: {}", pngs.len());

    for (i, (provider, model)) in VISION.iter().enumerate() {
        if pngs.is_empty() || spent(&pool).await > BUDGET_MICRO_USD {
            println!("no screenshots or budget reached; stopping vision");
            break;
        }
        let label = format!("vision-{model}");
        let move_id = new_move(&pool, &label).await;
        let port = 18250 + i as u16;
        let _gw = gateway(
            &bin,
            &db,
            port,
            provider,
            "AGENTRIX_ROUTE_VISION_PARITY",
            &format!("{provider}:{model}"),
            &dir.join(format!("{label}.log")),
        )
        .await;
        let gw = HttpGateway::new(format!("http://127.0.0.1:{port}"), http.clone());
        let ctx = CallContext {
            site: SiteId::new(SITE),
            move_id: move_id.clone(),
            attempt: 1,
        };
        let mut judged = Vec::new();
        for (k, (path, png, words)) in pngs.iter().enumerate() {
            // The right text, then another page's text: a good judge separates them.
            let wrong = &pngs[(k + 1) % pngs.len()].2;
            for (pairing, expected) in [("match", words), ("mismatch", wrong)] {
                let t = Instant::now();
                let request = VisionRequest {
                    path: path.clone(),
                    new_png: png.clone(),
                    old_png: None,
                    expected_text: expected.clone(),
                };
                let verdict = gw.vision_parity(&ctx, &request).await;
                judged.push(match verdict {
                    Ok(v) => json!({ "path": path, "pairing": pairing, "score": v.score, "verdict": v.verdict, "seconds": t.elapsed().as_secs_f64().round() }),
                    Err(e) => json!({ "path": path, "pairing": pairing, "error": e.to_string(), "seconds": t.elapsed().as_secs_f64().round() }),
                });
            }
        }
        let mean = |pairing: &str| {
            let s: Vec<f64> = judged
                .iter()
                .filter(|j| j["pairing"] == pairing)
                .filter_map(|j| j["score"].as_f64())
                .collect();
            if s.is_empty() {
                None
            } else {
                Some((s.iter().sum::<f64>() / s.len() as f64 * 1000.0).round() / 1000.0)
            }
        };
        let cost = usage(&pool, &move_id).await;
        let summary = json!({
            "provider": provider, "model": model, "calls": judged.len(),
            "errors": judged.iter().filter(|j| j.get("error").is_some()).count(),
            "mean_match": mean("match"), "mean_mismatch": mean("mismatch"),
            "usage": cost,
            "cost_per_check_usd": cost["cost_usd"].as_f64().unwrap_or(0.0) / judged.len().max(1) as f64,
        });
        println!("{summary}");
        let mut entry = summary;
        entry["detail"] = Value::Array(judged);
        results["vision"].as_array_mut().unwrap().push(entry);
        std::fs::write(
            dir.join("results.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
    }
    println!("total spent: ${:.3}", spent(&pool).await as f64 / 1e6);
}
