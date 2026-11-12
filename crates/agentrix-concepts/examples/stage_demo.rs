//! A local demo of "Choose your new look" without spending on models: the crate's real routes,
//! run engine, sanitizer, store and SSE, with a scripted Zavora CLI that replays a real run's
//! output (W0's three DeepSeek designs and the GLM directions, 8 Nov) at a readable pace: each
//! designer writes its real page into index.html in its room a section at a time.
//!
//! ```sh
//! scripts/wcargo run -p agentrix-concepts --example stage_demo -- 18071
//! # then open http://127.0.0.1:18071/
//! ```
//!
//! `CONCEPTS_DEMO_DIR` holds `concept-1.html` to `concept-3.html` and `directions.json`;
//! `CONCEPTS_DEMO_IMG` the photos they use (`/img/*.webp`).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agentrix_concepts::runner::{AgentRunner, ZavoraRunner};
use agentrix_host::testing::FixedOwner;
use axum::Router;
use axum::extract::Path as UrlPath;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use bytes::Bytes;
use serde_json::json;

const DEMO_DIR: &str =
    "/Users/jameskaranja/Developer/hackathons/.agentrix-local-w0/concepts-demo/v2";
const IMG_DIR: &str = "/Users/jameskaranja/Developer/hackathons/.agentrix-local-w0/concepts/img";
const FACTS: &str =
    "/Users/jameskaranja/Developer/hackathons/.agentrix-local-w0/concepts/run-1/facts.json";
const SITE: &str = "site_demo";

fn event(kind: &str, data: serde_json::Value) -> String {
    json!({"schema_version": "zavora.headless.v1", "sequence": 0, "timestamp": "t", "type": kind,
           "command": "ask", "session_id": "demo", "data": data})
    .to_string()
}

/// The scripted Zavora CLI and its replays. Each designer writes its real page into
/// `index.html` in its room a section at a time, as fs_write would, with the tool events a real
/// run sends (including the refusals W0 saw: execute_bash and file_edit).
fn fake_zavora(dir: &Path, demo: &Path) -> anyhow::Result<PathBuf> {
    let directions: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(demo.join("directions.json"))?)?;
    std::fs::write(
        dir.join("planner.jsonl"),
        [
            event("init", json!({"capabilities": ["message"]})),
            event("agent", json!({"name": "assistant"})),
            event(
                "result",
                json!({"success": true, "provider": "vultr", "model": "glm-5.3", "stats": {"duration_ms": 13000},
                       "response": directions.to_string()}),
            ),
        ]
        .join("\n"),
    )?;
    let write = |id: usize| {
        (
            event(
                "tool_use",
                json!({"call_id": format!("w{id}"), "name": "fs_write", "detail": "{\"path\":\"index.html\"}"}),
            ),
            event(
                "tool_result",
                json!({"call_id": format!("w{id}"), "name": "fs_write", "success": true, "detail": "written"}),
            ),
        )
    };
    let mut cases = String::new();
    for (i, d) in directions.as_array().into_iter().flatten().enumerate() {
        let n = i + 1;
        let html = std::fs::read_to_string(demo.join(format!("concept-{n}.html")))?;
        let part_dir = dir.join(format!("designer-{n}"));
        std::fs::create_dir_all(&part_dir)?;
        // The head and styles first, then each section of the body.
        let body_at = html.find("<body").unwrap_or(html.len() / 3);
        let mut parts = vec![html[..body_at].to_owned()];
        let rest = &html[body_at..];
        let mut cut = 0;
        for (k, _) in rest
            .match_indices("<section")
            .chain(rest.match_indices("<footer"))
        {
            if k > cut {
                parts.push(rest[cut..k].to_owned());
                cut = k;
            }
        }
        parts.push(rest[cut..].to_owned());
        parts.retain(|p| !p.is_empty());
        let mut sh = String::from("#!/bin/sh\n");
        let emit = |line: String, sh: &mut String| {
            std::fs::write(part_dir.join(format!("e{}.json", sh.len())), &line).ok();
            sh.push_str(&format!(
                "cat '{}/e{}.json'; echo\n",
                part_dir.display(),
                sh.len()
            ));
        };
        emit(event("init", json!({"capabilities": ["message"]})), &mut sh);
        emit(event("agent", json!({"name": "assistant"})), &mut sh);
        emit(
            event(
                "tool_use",
                json!({"call_id": "r1", "name": "fs_read", "detail": "{\"path\":\"kit.css\"}"}),
            ),
            &mut sh,
        );
        sh.push_str("sleep 1\n");
        emit(
            event(
                "tool_result",
                json!({"call_id": "r1", "name": "fs_read", "success": true, "detail": "read"}),
            ),
            &mut sh,
        );
        emit(
            event(
                "tool_use",
                json!({"call_id": "x1", "name": "execute_bash", "detail": "{}"}),
            ),
            &mut sh,
        );
        emit(
            event(
                "tool_result",
                json!({"call_id": "x1", "name": "execute_bash", "success": false, "detail": "requires approval in headless mode"}),
            ),
            &mut sh,
        );
        for (k, part) in parts.iter().enumerate() {
            let file = part_dir.join(format!("part-{k}.html"));
            std::fs::write(&file, part)?;
            let (use_, done) = write(k);
            emit(use_, &mut sh);
            sh.push_str(&format!(
                "sleep 0.{}\ncat '{}' {} index.html\n",
                6 + i,
                file.display(),
                if k == 0 { ">" } else { ">>" }
            ));
            emit(done, &mut sh);
            if k == 2 && i == 1 {
                emit(
                    event(
                        "tool_use",
                        json!({"call_id": "e1", "name": "file_edit", "detail": "{}"}),
                    ),
                    &mut sh,
                );
                emit(
                    event(
                        "tool_result",
                        json!({"call_id": "e1", "name": "file_edit", "success": false, "detail": "requires approval in headless mode"}),
                    ),
                    &mut sh,
                );
            }
        }
        emit(
            event(
                "message",
                json!({"role": "assistant", "author": "assistant", "delta": "index.html is complete."}),
            ),
            &mut sh,
        );
        emit(
            event(
                "result",
                json!({"success": true, "provider": "vultr", "model": "deepseek-v4.1-flash-1017", "stats": {"duration_ms": 69000 + 3000 * i}, "response": "index.html is complete."}),
            ),
            &mut sh,
        );
        let script = dir.join(format!("designer-{n}.sh"));
        std::fs::write(&script, sh)?;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        let name = d["name"].as_str().unwrap_or_default().replace('\'', "");
        cases.push_str(&format!(
            "  *'\"name\":\"{name}\"'*) exec '{}' ;;\n",
            script.display()
        ));
    }
    let fix = [
        event("init", json!({"capabilities": ["message"]})),
        event("tool_use", json!({"call_id": "f1", "name": "fs_write", "detail": "{\"mode\":\"patch\"}"})),
        event("tool_result", json!({"call_id": "f1", "name": "fs_write", "success": true, "detail": "patched"})),
        event("result", json!({"success": true, "provider": "vultr", "model": "deepseek-v4.1-flash-1017", "stats": {"duration_ms": 9000}, "response": "Done."})),
    ]
    .join("\n");
    std::fs::write(dir.join("fix.jsonl"), fix)?;
    let script = dir.join("zavora-cli");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
# Replays a real Zavora run as stream-json, writing index.html as the designer would.
case "$*" in
  *"design director"*) sleep 3; cat '{dir}/planner.jsonl'; exit 0 ;;
  *"The owner asked for this change"*)
     sleep 3
     /usr/bin/perl -0pi -e 's#</head>#<style>h1{{color:\#9a3412 !important}}</style></head>#' index.html
     cat '{dir}/fix.jsonl'; exit 0 ;;
  *"An automatic check found these missing"*) sleep 2; cat '{dir}/fix.jsonl'; exit 0 ;;
{cases}  *) echo '{{"type":"error","data":{{"fatal":true,"message":"unknown direction"}}}}'; exit 1 ;;
esac
"#,
            dir = dir.display()
        ),
    )?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    Ok(script)
}

/// The fixture bakery's facts as one page the "health check" read.
fn fixture_page() -> String {
    let facts: serde_json::Value = std::fs::read_to_string(FACTS)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut body = String::new();
    for p in facts["pages"].as_array().into_iter().flatten() {
        body.push_str(&format!(
            "<h2>{}</h2><p>{}</p>",
            p["title"].as_str().unwrap_or_default(),
            p["text"].as_str().unwrap_or_default()
        ));
    }
    format!(
        "<html><head><title>{}</title><meta name=\"description\" content=\"{}\"></head><body>{body}</body></html>",
        facts["name"].as_str().unwrap_or("Kilele Bakery"),
        facts["tagline"].as_str().unwrap_or_default()
    )
}

async fn image(UrlPath(name): UrlPath<String>) -> Response {
    let dir = std::env::var("CONCEPTS_DEMO_IMG").unwrap_or_else(|_| IMG_DIR.into());
    if !name.ends_with(".webp")
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '-' || c == '.')
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(Path::new(&dir).join(&name)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/webp")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(18071);
    let demo =
        PathBuf::from(std::env::var("CONCEPTS_DEMO_DIR").unwrap_or_else(|_| DEMO_DIR.into()));
    let scratch = tempfile::tempdir()?;
    let owner = uuid::Uuid::now_v7();
    let mut ctx = agentrix_host::testing::context().await?;
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    ctx.origin = format!("http://127.0.0.1:{port}").parse()?;
    ctx.platform_host = "127.0.0.1".into();
    for sql in [
        "CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', owner_account_id TEXT)",
        "CREATE TABLE assessments (assessment_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, state TEXT NOT NULL, report TEXT, created_at TEXT NOT NULL)",
    ] {
        sqlx::query(sql).execute(&ctx.pool).await?;
    }
    agentrix_concepts::migrate(&ctx.pool).await?;
    let digest = ctx.blobs.put(Bytes::from(fixture_page())).await?;
    sqlx::query("INSERT INTO sites VALUES (?1, 'fixture.agentrix.website', 'Kilele Bakery', ?2)")
        .bind(SITE)
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await?;
    sqlx::query("INSERT INTO assessments VALUES ('asmt_demo', ?1, 'report-ready', ?2, '2026-11-08T19:00:00Z')")
        .bind(SITE)
        .bind(json!({"pages": [{"url": "https://fixture.agentrix.website/", "digest": digest}]}).to_string())
        .execute(&ctx.pool)
        .await?;
    let mut settings = agentrix_concepts::Settings::from_env();
    settings.zavora_bin = fake_zavora(scratch.path(), &demo)?;
    settings.api_key = Some("demo".into());
    let runner: Arc<dyn AgentRunner> = Arc::new(ZavoraRunner {
        bin: settings.zavora_bin.clone(),
        api_key: "demo".into(),
        rooms: scratch.path().join("rooms"),
        timeout: Duration::from_secs(120),
        max_prompt_chars: 200_000,
    });
    let mounted = agentrix_concepts::mount_with(&ctx, &settings, Some(runner))?;
    let app = Router::new()
        .route(
            "/",
            get(|| async { Redirect::to("/sites/site_demo/concepts") }),
        )
        .route("/img/{name}", get(image))
        .merge(agentrix_ui::assets())
        .merge(agentrix_onboarding::assets())
        .merge(mounted.platform);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("concept stage demo on http://127.0.0.1:{port}/");
    axum::serve(listener, app).await?;
    Ok(())
}
