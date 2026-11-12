//! Serves the home page locally for side-by-side checks against the concepts.
//!
//! ```sh
//! # facts from a control.db (read-only), e.g. a copy of node's:
//! HOME_DB=/path/to/control.db cargo run -p agentrix-home --example home-preview
//! # no database: every fact unknown, so the honest empty states show
//! cargo run -p agentrix-home --example home-preview
//! # made-up facts, to check the filled-in layout (never wired into node):
//! HOME_DEMO=1 cargo run -p agentrix-home --example home-preview
//! ```
//!
//! Listens on 127.0.0.1:19000 (W10's preview range), or `HOME_LISTEN`.

use std::sync::Arc;

use std::collections::BTreeMap;

use agentrix_home::facts::{
    Attack, BriefSnapshot, GauntletRun, LiveMove, Picture, Route, Routing, StepTime,
};
use agentrix_home::{FactSource, Facts, Home, HomeConfig, SqliteFacts};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};

struct NoFacts;

#[async_trait]
impl FactSource for NoFacts {
    async fn read(&self) -> anyhow::Result<Facts> {
        Ok(Facts::default())
    }
}

/// Made-up numbers for checking the layout, with W11's real sample pictures. Preview only.
struct DemoFacts {
    pictures: BTreeMap<String, Picture>,
}

#[async_trait]
impl FactSource for DemoFacts {
    async fn read(&self) -> anyhow::Result<Facts> {
        let at = |h, m| Utc.with_ymd_and_hms(2026, 11, 7, h, m, 0).unwrap();
        let attacks = [
            "Redirect to 169.254.169.254",
            "Link into 127.0.0.1",
            "Orders hidden for AI agents",
            "Hidden spam links",
            "Cloaking plugin",
            "Webshell in uploads",
            "Obfuscated miner script",
            "PHP phones home",
            "Fork bomb, 8 GB",
            "\"Give me 90% off\"",
        ];
        Ok(Facts {
            gauntlet: Some(GauntletRun {
                score: 10,
                out_of: 10,
                receipt: Some("sha256:demo".into()),
                finished_at: at(21, 10),
                attacks: attacks
                    .iter()
                    .map(|a| Attack {
                        name: (*a).into(),
                        plain: String::new(),
                        held: true,
                        receipt: None,
                    })
                    .collect(),
            }),
            first_live_move: Some(LiveMove {
                site_name: "Zavora's fixture".into(),
                steps: [
                    (
                        "replay",
                        12.4,
                        "Read 50 pages and 31 files in a sealed replay",
                    ),
                    ("clean", 37.5, "50 pages clean; 4 items left behind"),
                    ("rebuild", 26.6, "Rebuilt 50 pages"),
                    ("prove", 18.2, "Parity 98.8%; release 2 is ready for you"),
                ]
                .iter()
                .map(|(s, t, sum)| StepTime {
                    step: (*s).into(),
                    seconds: *t,
                    summary: (*sum).into(),
                    receipt: Some("sha256:demo".into()),
                })
                .collect(),
                live_at: at(21, 0),
                approval_to_live_seconds: Some(41.2),
                approved_with: Some("passkey".into()),
                left_behind: Some(4),
                cost_micro_usd: Some(370_000),
                providers: vec!["anthropic".into()],
            }),
            sites_live: Some(1),
            receipts_issued: Some(1204),
            reports_written: Some(17),
            routing: Some(Routing {
                provider: "vultr".into(),
                routes: vec![
                    Route {
                        purpose: "build".into(),
                        model: "qwen3.8-27b".into(),
                        provider: Some("vultr".into()),
                    },
                    Route {
                        purpose: "embed".into(),
                        model: "qwen3-embedding-4b".into(),
                        provider: Some("vultr".into()),
                    },
                ],
                since: at(22, 0),
                receipt: "sha256:demo".into(),
            }),
            brief: Some(BriefSnapshot {
                site_name: "Zavora's fixture".into(),
                written_at: at(4, 0),
                headline: Some("Your site moved overnight.".into()),
                lines: vec![
                    "The Guardian left 4 infected items behind, each with its reason.".into(),
                ],
            }),
            sample_pictures: self.pictures.clone(),
            read_at: Some(Utc::now()),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let listen = std::env::var("HOME_LISTEN").unwrap_or_else(|_| "127.0.0.1:19000".into());
    // W11's sample pictures, seeded from the files agentrix-media embeds (nothing is drawn),
    // and served under /media/ as node serves them.
    let ctx = agentrix_host::testing::context().await?;
    sqlx::raw_sql(include_str!(
        "../../agentrix-node/src/db/migrations/1200_media.sql"
    ))
    .execute(&ctx.pool)
    .await?;
    agentrix_media::samples::seed(&ctx.pool, &ctx.blobs).await?;
    let mut pictures = BTreeMap::new();
    for slug in [
        "kilele-bakery-sample",
        "classy-perfumes-sample",
        "tana-valley-times-sample",
    ] {
        if let Some(hero) = agentrix_media::shown(&ctx.pool, slug).await?.remove("hero") {
            pictures.insert(
                slug.to_owned(),
                Picture {
                    url: hero.url,
                    alt: hero.alt,
                    label: hero.label,
                    width: hero.width,
                    height: hero.height,
                },
            );
        }
    }
    let source: Arc<dyn FactSource> = if std::env::var_os("HOME_DEMO").is_some() {
        Arc::new(DemoFacts { pictures })
    } else if let Some(db) = std::env::var_os("HOME_DB") {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(db)
            .read_only(true);
        Arc::new(SqliteFacts::new(
            sqlx::SqlitePool::connect_with(options).await?,
        ))
    } else {
        Arc::new(NoFacts)
    };
    let mut config = HomeConfig::from_env(&format!("http://{listen}"));
    config.fixture_href = Some("/assess/fixture".into());
    // Preview only: keep reports in memory, with sending switched off, as on a node before W9's
    // `lead.report` exists.
    let kept = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    agentrix_home::migrate(&kept).await?;
    let home = Home::new(config, source)?
        .with_keep(agentrix_home::keep::Keep::new(Some(kept), None))
        // HOME_GATEWAY=http://127.0.0.1:8090 lets the desks answer with the real model.
        .with_desks(agentrix_home::desks::Desks::new(
            std::env::var("HOME_GATEWAY").ok().map(|url| {
                Arc::new(agentrix_home::desks::GatewayModel::new(
                    &url,
                    reqwest::Client::new(),
                )) as Arc<dyn agentrix_home::desks::DeskModel>
            }),
            None,
        ));
    if std::env::var_os("HOME_DEMO").is_some() {
        // Preview only: pretend the first sample answers, to check the card's open state.
        let first = home.sample_hosts().into_iter().take(1).collect();
        home.sample_probe().set_open(first);
    }
    let app = home
        .routes()
        .merge(home.front_door())
        .merge(agentrix_ui::assets())
        .merge(agentrix_media::public_router(&ctx))
        .merge(stand_in_assess());
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!("home page preview on http://{listen}/");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Preview only: a stand-in for node's Assess (`POST /assess` and its events), playing a short
/// made-up check in the shape of W4's real events (design §4.3a), so screen 1's in-place check
/// can be seen without a sandbox. `bad.example` gets Assess's own error page.
fn stand_in_assess() -> axum::Router {
    use axum::response::sse::{Event, Sse};
    use axum::response::{Html, IntoResponse, Redirect};
    use axum::routing::{get, post};
    use futures::stream::{self, StreamExt};
    use std::convert::Infallible;
    use std::time::Duration;

    #[derive(serde::Deserialize)]
    struct Start {
        url: String,
    }
    let start = |axum::Form(f): axum::Form<Start>| async move {
        if f.url.contains("bad.example") || !f.url.contains('.') {
            return (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                Html(r#"<p class="field-error" role="alert">That doesn't look like a website address.</p>"#),
            )
                .into_response();
        }
        Redirect::to("/assess/asmt_preview").into_response()
    };
    let events = || async {
        let at = "2026-11-08T10:00:00Z";
        let list: Vec<(u64, &str, serde_json::Value)> = vec![
            (
                300,
                "step",
                serde_json::json!({"step": "read", "state": "running", "elapsed_ms": 0, "summary": "Opening a sealed room to read your site.", "at": at}),
            ),
            (
                700,
                "page",
                serde_json::json!({"n": 1, "of": 12, "url": "https://kilele-bakery.example/", "status": "read", "at": at}),
            ),
            (
                700,
                "page",
                serde_json::json!({"n": 4, "of": 12, "url": "https://kilele-bakery.example/cakes/", "status": "read", "at": at}),
            ),
            (
                600,
                "finding",
                serde_json::json!({"id": "f1", "severity": "urgent", "category": "security", "title": "Hidden links you didn't put there", "where": "kilele-bakery.example/blog/summer-offers", "at": at}),
            ),
            (
                700,
                "page",
                serde_json::json!({"n": 12, "of": 12, "url": "https://kilele-bakery.example/contact/", "status": "read", "at": at}),
            ),
            (
                500,
                "finding",
                serde_json::json!({"id": "f2", "severity": "warn", "category": "security", "title": "An old WordPress with a public login page", "where": "kilele-bakery.example/wp-login.php", "at": at}),
            ),
            (
                500,
                "finding",
                serde_json::json!({"id": "f3", "severity": "info", "category": "ai-readiness", "title": "No structured data for your products", "where": "12 pages", "at": at}),
            ),
            (
                600,
                "step",
                serde_json::json!({"step": "report", "state": "done", "elapsed_ms": 4200, "summary": "Writing your report.", "at": at}),
            ),
            (
                400,
                "done",
                serde_json::json!({"outcome": "report-ready", "href": "/assess/asmt_preview", "at": at}),
            ),
        ];
        let s = stream::iter(list).then(|(wait, name, data)| async move {
            tokio::time::sleep(Duration::from_millis(wait)).await;
            Ok::<_, Infallible>(Event::default().event(name).data(data.to_string()))
        });
        Sse::new(s)
    };
    axum::Router::new()
        .route("/assess", post(start))
        .route(
            "/assess/asmt_preview",
            get(|| async { Html("<p>The full report (W4's page on node).</p>") }),
        )
        .route("/assess/asmt_preview/events", get(events))
}
