//! Pictures and Talk inside W5's console shell, on the shell's sample bakery, for comparing
//! with `concept/console/live.html#voice` and adk-surface's Voice in a browser.
//!
//! `scripts/wcargo run -p agentrix-media --example console-preview`, then open
//! <http://localhost:19102/console/site_kilele_sample/apps/talk> and `…/apps/pictures`.
//! It calls the Model Gateway at `AGENTRIX_MODELS_URL` (default <http://127.0.0.1:19101>), so
//! run one locally (it needs `site_kilele_sample` in its control.db). Marked Sample throughout.

use std::sync::Arc;

use agentrix_host::testing::{FixedOwner, context};
use agentrix_media::{apps, routes, samples, voice};
use agentrix_shell::{Shell, ShellState, router, sample::SamplePort, store::ShellStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("AGENTRIX_MEDIA_PORT").unwrap_or_else(|_| "19102".into());
    let site = agentrix_shell::sample::SITE;
    let account: uuid::Uuid = "0190f5c2-0000-7000-8000-000000000001".parse()?;
    let mut ctx = context().await?;
    ctx.models_url =
        std::env::var("AGENTRIX_MODELS_URL").unwrap_or_else(|_| "http://127.0.0.1:19101".into());
    ctx.origin = format!("http://localhost:{port}").parse()?;
    ctx.owners = Arc::new(FixedOwner::passkey(account));
    sqlx::raw_sql(
        "CREATE TABLE sites (site_id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', owner_account_id TEXT);
         CREATE TABLE briefs (site_id TEXT NOT NULL, written_at TEXT NOT NULL, body TEXT NOT NULL);",
    )
    .execute(&ctx.pool)
    .await?;
    sqlx::query("INSERT INTO sites VALUES (?, 'Kilele Bakery', ?)")
        .bind(site)
        .bind(account.to_string())
        .execute(&ctx.pool)
        .await?;
    sqlx::query("INSERT INTO briefs VALUES (?, '2026-11-08T03:58:00.000000Z', ?)")
        .bind(site)
        .bind(serde_json::json!({
            "headline": "Good morning, Wanjiru. This is sample data",
            "lines": [{ "text": "3 orders overnight, KSh 8,400" }, { "text": "One go-live decision waits for you" }],
            "spend_words": "Models cost $0.42 yesterday"
        }).to_string())
        .execute(&ctx.pool)
        .await?;
    for sql in [
        include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
        include_str!("../../agentrix-node/src/db/migrations/1201_voice_notes.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&ctx.pool).await?;
    }
    samples::seed(&ctx.pool, &ctx.blobs).await?;
    // The shell's sample site borrows Kilele's sample pictures; two wait as drafts.
    sqlx::query("UPDATE media_images SET site_id = ? WHERE site_id = 'kilele-bakery-sample'")
        .bind(site)
        .execute(&ctx.pool)
        .await?;
    sqlx::raw_sql(
        "UPDATE media_images SET status = 'draft', asked_by = 'editor', decided_at = NULL, decided_by = NULL \
         WHERE slot IN ('product:mandazi', 'product:banana-bread');",
    )
    .execute(&ctx.pool)
    .await?;

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    let state = ShellState {
        shell: Arc::new(Shell::new()?),
        port: Arc::new(SamplePort {
            has_passkey: true,
            ..SamplePort::default()
        }),
        store: ShellStore::open(pool).await?,
        apps: apps::console_apps(&ctx),
        lanes: Vec::new(),
        freeze_enabled: true,
    };
    let media = routes::MediaState::new(&ctx);
    let app = router(state)
        .merge(agentrix_ui::assets())
        .merge(routes::console_router(media.clone()))
        .merge(routes::picture_router(media.clone()))
        .merge(voice::voice_router(media))
        .merge(voice::assets_router());
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    println!("Console preview on http://localhost:{port}/console/{site}/apps/talk");
    axum::serve(listener, app).await?;
    Ok(())
}
