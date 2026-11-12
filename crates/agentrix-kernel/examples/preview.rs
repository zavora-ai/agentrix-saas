//! A local preview of a sample site's team with live presence, for checking the marks and the
//! stream in a browser:
//!
//! ```text
//! CARGO_TARGET_DIR=… cargo run -p agentrix-kernel --example preview -- kilele
//! open http://localhost:18100/
//! ```
//!
//! It runs on an in-memory database with a fake signed-in owner. `POST /dev/ask/{desk}` plays
//! one Ask turn's frames through the hub, as the first release's `/ask` sends them, and
//! `POST /dev/card/{on|off}` raises and decides a held-order card. Development only: none of
//! this is mounted in node.

use std::sync::Arc;
use std::time::Duration;

use agentrix_host::testing::{FixedOwner, context};
use agentrix_kernel::{Presence, Rosters, migrate};
use agentrix_marks::MarkState;
use agentrix_types::CardKind;
use axum::extract::{Path, State};
use axum::response::Html;
use axum::routing::{get, post};
use serde_json::json;

#[derive(Clone)]
struct App {
    rosters: Rosters,
    hub: Presence,
    site: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let slug = std::env::args().nth(1).unwrap_or_else(|| "kilele".into());
    let mut ctx = context().await?;
    migrate(&ctx.pool).await?;
    let owner = uuid::Uuid::new_v4();
    sqlx::query("CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT, name TEXT, owner_account_id TEXT)")
        .execute(&ctx.pool)
        .await?;
    let site = format!("{slug}-sample");
    sqlx::query("INSERT INTO sites VALUES (?, ?, '', ?)")
        .bind(&site)
        .bind(format!("{site}.agentrix.website"))
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await?;
    let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
    rosters.seed_sample(&site, &slug).await?;
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    ctx.origin = "http://localhost:18100".parse()?;
    let mounted = agentrix_kernel::mount(&ctx)?;
    let app = App {
        rosters,
        hub: agentrix_kernel::presence::global().clone(),
        site,
    };
    let router = axum::Router::new()
        .route("/", get(page))
        .route("/dev/ask/{desk}", post(ask))
        .route("/dev/card/{on}", post(card))
        .route("/dev.js", get(dev_js))
        .route("/workshop", get(workshop_page))
        .route("/dev.css", get(dev_css))
        .route("/teams", get(teams_page))
        .route("/states", get(states_page))
        .route("/dev/voice.wav", get(voice_wav))
        .route("/dev-states.js", get(dev_states_js))
        .with_state(app)
        .merge(mounted.platform);
    // A spoken sample answer for the speaking state, made once with macOS's local `say`. Where
    // there is no `say`, the page says the voice sample isn't available.
    let _ = std::process::Command::new("say")
        .args(["-o"])
        .arg(voice_path())
        .args([
            "--file-format=WAVE",
            "--data-format=LEI16@22050",
            SAMPLE_ANSWER,
        ])
        .status();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18100").await?;
    println!("preview on http://localhost:18100/");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn page(State(app): State<App>) -> Html<String> {
    let r = app.rosters.roster(&app.site).await.unwrap().unwrap();
    let mut marks: Vec<(String, String, String)> = Vec::new();
    for d in r.desks() {
        let svg = r
            .mark_svg(&d.id, 40.0, MarkState::Idle, &format!("p{}", marks.len()))
            .unwrap();
        marks.push((d.id.clone(), d.name.clone(), svg));
    }
    for m in r
        .on_duty()
        .filter(|m| m.spec.id != "coordinator" && m.spec.id != "concierge")
    {
        let svg = r
            .mark_svg(
                &m.spec.id,
                40.0,
                MarkState::Asleep,
                &format!("p{}", marks.len()),
            )
            .unwrap();
        marks.push((m.spec.id.clone(), m.spec.name.clone(), svg));
    }
    let rows: String = marks
        .iter()
        .map(|(id, name, svg)| {
            format!(
                r#"<li><wa-mark webagent="{id}">{svg}</wa-mark><div><b>{name}</b> <span class="ai">AI</span><br><span class="words" role="status" data-presence-words="{id}"></span></div><button type="button" data-ask="{id}">Play an Ask turn</button></li>"#
            )
        })
        .collect();
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{name} team</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="agentrix-presence" content="/api/webagents/{site}/presence">
<link rel="stylesheet" href="/_agentrix/marks.css">
<script type="module" src="/_agentrix/marks.js"></script>
<script type="module" src="/dev.js"></script>
<style>body{{font:15px/1.5 system-ui;margin:24px;background:{bg};color:{ink};--wa-ground:{bg}}}
ul{{list-style:none;padding:0;display:grid;gap:10px}}li{{display:flex;gap:12px;align-items:center}}
.ai{{font:700 10px/1.4 system-ui;border:1px solid #ccc;border-radius:999px;padding:1px 6px}}.words{{font-size:13px;opacity:.8}}
button{{margin-left:auto}}</style></head><body>
<h1><wa-mark team webagent="coordinator">{team}</wa-mark> {name}</h1>
<p class="words" data-presence-words="coordinator"></p>
<p><button type="button" data-card="on">Hold an order for the owner</button> <button type="button" data-card="off">Owner decides it</button></p>
<ul>{rows}</ul></body></html>"#,
        name = r.look.name,
        site = r.site,
        bg = r.look.bg,
        ink = r.look.ink,
        team = r.team_svg(48.0, MarkState::Asleep, &[]),
    ))
}

async fn ask(State(app): State<App>, Path(desk): Path<String>) -> &'static str {
    let (hub, site) = (app.hub.clone(), app.site.clone());
    tokio::spawn(async move {
        hub.on_ask(
            &site,
            &desk,
            "awp.presence",
            &json!({"state": "working", "label": "Reading this site"}),
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
        hub.on_ask(&site, &desk, "awp.presence", &json!({"state": "answering"}));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        hub.on_ask(&site, &desk, "done", &json!({}));
    });
    "ok"
}

async fn card(State(app): State<App>, Path(on): Path<String>) -> &'static str {
    app.hub
        .on_decision(&app.site, "d-preview", CardKind::OrderHeld, on == "on");
    "ok"
}

async fn dev_js() -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        r#"document.addEventListener("click", e => {
  const b = e.target.closest("button"); if (!b) return;
  if (b.dataset.ask) fetch("/dev/ask/" + b.dataset.ask, { method: "POST" });
  if (b.dataset.card) fetch("/dev/card/" + b.dataset.card, { method: "POST" });
});"#,
    )
}

async fn workshop_page(State(app): State<App>) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Workshop</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/dev.css">
<link rel="stylesheet" href="/_agentrix/marks.css">
<link rel="stylesheet" href="/_agentrix/workshop.css">
<script type="module" src="/_agentrix/marks.js"></script>
<script type="module" src="/_agentrix/workshop.js"></script>
</head><body><main class="page"><h1>Appearance</h1>
<wa-workshop site="{site}" webagent="editor"></wa-workshop></main></body></html>"#,
        site = app.site
    ))
}

/// A few of agentrix-ui's tokens and its button and chip classes, so the preview looks like the
/// console without depending on the kit.
async fn dev_css() -> ([(axum::http::HeaderName, &'static str); 1], &'static str) {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css")],
        r#":root{--paper:#fff;--line:#e3e2ec;--line-strong:#cfcedd;--ink:#23223a;--ink-2:#4a4960;--muted:#656479;--indigo:#38356d;--indigo-50:#f6f6fb;--coral:#de4060;--ok:#11734a;--ok-bg:#e6f5ee;--wait:#8a5300;--wait-bg:#fff4dd;--font-text:"DM Sans",system-ui,sans-serif;--font-display:Manrope,system-ui,sans-serif;--font-mono:ui-monospace,Menlo,monospace;--r:12px;--r-lg:18px}
body{margin:0;background:#f6f6fa;font:15px/1.5 var(--font-text);color:var(--ink)}.page{max-width:1180px;margin:0 auto;padding:24px 16px}
.btn{display:inline-flex;align-items:center;justify-content:center;min-height:44px;padding:0 16px;border-radius:12px;border:1px solid var(--line-strong);background:var(--paper);color:var(--ink);font:700 14px/1 var(--font-text);cursor:pointer}
.btn-primary{background:var(--indigo);border-color:var(--indigo);color:#fff}.btn-sm{min-height:36px;padding:0 12px;font-size:13px;border-radius:10px}.btn[disabled]{opacity:.5;cursor:not-allowed}
.chip{display:inline-flex;align-items:center;gap:6px;padding:4px 10px;border-radius:999px;font:600 12px/1.3 var(--font-text)}.chip-ok{background:var(--ok-bg);color:var(--ok)}.chip-wait{background:var(--wait-bg);color:var(--wait)}"#,
    )
}

/// The three sample teams side by side, as concept/identity/marks.html's first section draws them.
async fn teams_page() -> Html<String> {
    use agentrix_kernel::roster;
    let mut cards = String::new();
    let mut uid = 0;
    for slug in agentrix_kernel::looks::SAMPLES {
        let r = roster::sample(slug, slug).expect("sample");
        let mut rows = String::new();
        let mut ids: Vec<(String, String, String)> = r
            .desks()
            .iter()
            .filter(|d| d.id.starts_with("desk-"))
            .map(|d| (d.id.clone(), d.name.clone(), d.mandate.clone()))
            .collect();
        ids.extend(
            r.on_duty()
                .filter(|m| m.spec.id != "coordinator" && m.spec.id != "concierge")
                .map(|m| {
                    (
                        m.spec.id.clone(),
                        m.spec.name.clone(),
                        m.spec.role_words.clone(),
                    )
                }),
        );
        for (id, name, what) in ids {
            uid += 1;
            let svg = r
                .mark_svg(&id, 34.0, MarkState::Idle, &format!("t{uid}"))
                .unwrap_or_default();
            rows.push_str(&format!(
                r#"<li class="member"><wa-mark webagent="{id}" state="idle">{svg}</wa-mark><div><div class="who">{name} <span class="ai">AI</span></div><div class="what">{what}</div></div></li>"#
            ));
        }
        cards.push_str(&format!(
            r#"<article class="site" data-bg="{bg}" style="--sbg:{bg};--sink:{ink};--sfont:{font};--wa-ground:{bg}"><div class="site-name">{name}</div><div class="site-tag">Sample · family {family}, corners {n}</div>
<div class="chief-row"><wa-mark team webagent="coordinator">{team}</wa-mark><div><b>Chief WebAgent</b><span>Its dots are the team; each pulses while that WebAgent works</span></div></div><ul class="members">{rows}</ul></article>"#,
            bg = r.look.bg,
            ink = r.look.ink,
            font = r.look.font.replace('"', "'"),
            name = r.look.name,
            family = r.look.family.label(),
            n = r.look.squareness,
            team = r.team_svg(48.0, MarkState::Idle, &[]),
        ));
    }
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Three teams</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Young+Serif&family=Italiana&family=Libre+Caslon+Text:wght@400;700&display=swap">
<link rel="stylesheet" href="/dev.css"><link rel="stylesheet" href="/_agentrix/marks.css">
<script type="module" src="/_agentrix/marks.js"></script>
<style>.grid{{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}}@media(max-width:980px){{.grid{{grid-template-columns:1fr}}}}
.site{{background:var(--sbg);color:var(--sink);border:1px solid rgba(0,0,0,.08);border-radius:16px;padding:18px;display:grid;gap:14px;align-content:start}}
.site-name{{font-family:var(--sfont);font-size:1.5rem;line-height:1.1}}.site-tag{{font-size:12px;opacity:.75}}
.chief-row{{display:flex;align-items:center;gap:12px;padding-bottom:12px;border-bottom:1px solid rgba(0,0,0,.08)}}.chief-row b{{display:block;font-size:14px}}.chief-row span{{display:block;font-size:12.5px;opacity:.75}}
.members{{list-style:none;margin:0;padding:0;display:grid;gap:8px}}.member{{display:grid;grid-template-columns:auto minmax(0,1fr);align-items:center;gap:10px}}
.who{{font:700 14px/1.2 var(--font-text)}}.what{{font-size:12.5px;opacity:.75}}.ai{{display:inline-flex;padding:1px 6px;border-radius:999px;border:1px solid currentColor;font:700 10px/1.4 var(--font-text);opacity:.7}}</style>
</head><body><main class="page"><h1>Three sites, three teams</h1><p>Rendered by agentrix-marks from each site's roster (agentrix-kernel), one family per site, one seed per WebAgent.</p><div class="grid">{cards}</div></main></body></html>"#
    ))
}

/// One mark in all ten states, with the owner's and the visitor's words (06 §4.3).
async fn states_page(State(app): State<App>) -> Html<String> {
    use agentrix_marks::{Audience, WordsContext, words};
    let r = app.rosters.roster(&app.site).await.unwrap().unwrap();
    let desk = r
        .desks()
        .iter()
        .find(|d| d.id == "desk-cakes")
        .cloned()
        .unwrap();
    let cx = WordsContext {
        owner: Some(&r.look.owner),
        label: Some("Checking today's stock"),
        ..Default::default()
    };
    let mut tiles = String::new();
    for (i, st) in MarkState::ALL.iter().enumerate() {
        let svg = agentrix_marks::render(
            &desk.mark,
            &agentrix_marks::Render {
                size: 56.0,
                state: *st,
                font: &r.look.font,
                uid: &format!("s{i}"),
            },
        );
        let owner = words(*st, Audience::Owner, &cx);
        let visitor = words(*st, Audience::Visitor, &cx);
        tiles.push_str(&format!(
            r#"<div class="st"><div class="mk"><wa-mark webagent="cakes-{i}" state="{key}">{svg}</wa-mark></div><h3>{name}</h3><p><b>Owner:</b> {owner}</p><p><b>Visitor:</b> {visitor}</p></div>"#,
            key = st.key(),
            name = st.name(),
            owner = if owner.is_empty() { "Mark only".to_string() } else { owner },
            visitor = if visitor.is_empty() { "Mark only".to_string() } else { visitor },
        ));
    }
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Ten states</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<link rel="stylesheet" href="/dev.css"><link rel="stylesheet" href="/_agentrix/marks.css">
<script type="module" src="/_agentrix/marks.js"></script>
<style>.grid{{display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:12px}}@media(max-width:980px){{.grid{{grid-template-columns:repeat(2,minmax(0,1fr))}}}}
.st{{background:{bg};color:{ink};border:1px solid rgba(0,0,0,.08);border-radius:14px;padding:14px;display:grid;gap:6px;align-content:start;--wa-ground:{bg}}}.mk{{display:grid;place-items:center;height:84px}}.st h3{{margin:0;font-size:14px}}.st p{{margin:0;font-size:12.5px}}
.try{{background:{bg};color:{ink};border:1px solid rgba(0,0,0,.08);border-radius:16px;padding:18px;display:grid;grid-template-columns:auto minmax(0,1fr);gap:20px;align-items:center;margin-bottom:16px;--wa-ground:{bg}}}
.try .say{{display:grid;gap:8px;min-width:0}}.try .now{{font:600 13px/1.3 var(--font-mono);opacity:.75}}.try .log{{min-height:3.2em;font-size:15px}}.try .row{{display:flex;gap:8px;flex-wrap:wrap}}.try .note{{font-size:12px;opacity:.7;margin:0}}</style>
<script type="module" src="/dev-states.js"></script>
</head><body><main class="page"><h1>Ten states, one grammar</h1><p>The Cakes desk's mark, drawn by the same generator in every state. Motion settles after 5 seconds; speaking follows the voice.</p>
<section class="try" aria-label="Try answering and speaking"><wa-mark webagent="try" state="idle">{big}</wa-mark>
<div class="say"><div class="now" role="status" data-now>Idle</div><div class="log" role="log" aria-live="polite" data-log></div>
<div class="row"><button type="button" class="btn btn-sm" data-stream>Stream an answer</button><button type="button" class="btn btn-sm" data-speak>Speak the answer</button></div>
<p class="note">Sample answer. Answering holds a steady ring while the text streams. Speaking's ring follows the level of the voice actually playing, measured in the browser; mute it and the ring stills.</p>
<audio data-voice src="/dev/voice.wav" preload="auto"></audio></div></section>
<div class="grid">{tiles}</div></main></body></html>"#,
        bg = r.look.bg,
        ink = r.look.ink,
        big = agentrix_marks::render(
            &desk.mark,
            &agentrix_marks::Render {
                size: 96.0,
                state: MarkState::Idle,
                font: &r.look.font,
                uid: "try",
            },
        ),
    ))
}

/// The sample answer the states page streams and speaks.
const SAMPLE_ANSWER: &str = "Yes. Sourdough comes out of the oven at ten, and the next batch is at two. Order before noon and Wanjiru can hold a loaf for you.";

fn voice_path() -> std::path::PathBuf {
    std::env::temp_dir().join("agentrix-w1-preview-voice.wav")
}

async fn voice_wav() -> axum::response::Response {
    use axum::response::IntoResponse;
    match std::fs::read(voice_path()) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "audio/wav")], bytes).into_response(),
        Err(_) => (
            axum::http::StatusCode::NOT_FOUND,
            "The voice sample isn't available on this machine.",
        )
            .into_response(),
    }
}

/// Streams the sample answer beside the mark (answering), or plays it aloud (speaking) with
/// marks.js's followVoice measuring the audio.
async fn dev_states_js() -> ([(axum::http::HeaderName, &'static str); 1], String) {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        format!(
            r#"import {{ setState, followVoice }} from "/_agentrix/marks.js";
const answer = {answer:?};
const svg = () => document.querySelector('wa-mark[webagent="try"] svg.wa');
const now = document.querySelector("[data-now]"), log = document.querySelector("[data-log]");
const audio = document.querySelector("[data-voice]");
let streaming = false;
document.querySelector("[data-stream]").addEventListener("click", async () => {{
  if (streaming) return;
  streaming = true; log.textContent = ""; setState(svg(), "answering"); now.textContent = "Answering";
  for (const word of answer.split(" ")) {{ log.textContent += (log.textContent ? " " : "") + word; await new Promise(r => setTimeout(r, 140)); }}
  setState(svg(), "idle"); now.textContent = "Idle"; streaming = false;
}});
let following = null;
document.querySelector("[data-speak]").addEventListener("click", () => {{
  if (!following) following = followVoice(svg(), audio);
  log.textContent = answer; now.textContent = "Speaking";
  audio.currentTime = 0; audio.play().catch(() => {{ now.textContent = "The voice sample isn't available on this machine."; }});
}});
audio.addEventListener("ended", () => {{ now.textContent = "Idle"; }});
"#,
            answer = SAMPLE_ANSWER
        ),
    )
}
