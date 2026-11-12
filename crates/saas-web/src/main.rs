//! `saas-web`: Agentrix SaaS's face on one port, routing by `Host`.
//!
//! ```sh
//! scripts/wcargo run -p saas-web          # http://money.localhost:18183, kilele.money.localhost,
//!                                         # pay.localhost, books.localhost
//! ```
//!
//! Environment (all optional): `SAAS_WEB_ADDR` (default `127.0.0.1:18183`), `SAAS_HOST`,
//! `SAAS_KILELE_HOST`, `SAAS_AGENT_DOOR_HOST`, `SAAS_BOOKS_HOST`, `SAAS_BOARD_DIST`,
//! `SAAS_ERA_URL`, `SAAS_OWNER_TOKEN`, `SAAS_CONSOLE_OPEN=1`, `AG_STUDIO_LICENSE_KEY`,
//! `SAAS_FEED` (`fakebooks`, the default, or `demo`), `SAAS_PASSKEYS_FILE` (owners' passkeys), `SAAS_PROOFS_DIR` (what /proofs reads),
//! `SAAS_WALLET_URL` (books' wallet feed, `GET {url}/wallet/{site}.json`; without it the wallet
//! page reads the books and says it isn't reconciled), `SAAS_RUN_DAY_URL` (company's story
//! runner, `POST {url}/judge/run-day`: turns on judge mode's "Run the day").

use std::path::PathBuf;

use saas_web::hosts::Hosts;
use saas_web::{Access, Face, default_board_dist};

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && !v.starts_with('#'))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "saas_web=info,tower_http=info".into()),
        )
        .init();

    let addr = var("SAAS_WEB_ADDR").unwrap_or_else(|| "127.0.0.1:18183".into());
    let hosts = Hosts::from_env();
    let board = var("SAAS_BOARD_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(default_board_dist);
    // SAAS_FEED: "fakebooks" (default) reads saas-core's books, seeded with the story;
    // "demo" reads the story directly.
    let base = match var("SAAS_FEED").as_deref() {
        Some("demo") => Face::demo(hosts.clone()),
        _ => Face::fakebooks(hosts.clone(), saas_web::today_in_nairobi()).await?,
    };
    let face = base
        .with_board_dist(Some(board))
        .with_era(var("SAAS_ERA_URL"))
        .with_board_licence(var("AG_STUDIO_LICENSE_KEY"))
        .with_proofs_dir(Some(
            var("SAAS_PROOFS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(saas_web::default_proofs_dir),
        ))
        .with_access(Access {
            owner_token: var("SAAS_OWNER_TOKEN"),
            open: var("SAAS_CONSOLE_OPEN").as_deref() == Some("1"),
        });
    let face = match var("SAAS_PASSKEYS_FILE") {
        Some(file) => face.with_passkeys(std::sync::Arc::new(saas_web::passkeys::Passkeys::at(
            file.into(),
        )?)),
        None => face,
    };
    let face = match var("SAAS_WALLET_URL") {
        Some(url) => face.with_wallets(std::sync::Arc::new(saas_web::apps::http::HttpWallet::new(
            &url,
        ))),
        None => face,
    };
    let face = match var("SAAS_RUN_DAY_URL") {
        Some(url) => face.with_day_runner(Some(std::sync::Arc::new(
            saas_web::apps::http::HttpDay::new(&url),
        ))),
        None => face,
    };
    let app = face.router()?;

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let port = listener.local_addr()?.port();
    tracing::info!(
        "saas-web on {addr}: http://money.localhost:{port}/ · http://kilele.money.localhost:{port}/ · http://pay.localhost:{port}/ · http://books.localhost:{port}/ (production names: {}, {}, {}, {})",
        hosts.money,
        hosts.kilele,
        hosts.pay,
        hosts.books
    );
    axum::serve(listener, app).await?;
    Ok(())
}
