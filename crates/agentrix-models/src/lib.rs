//! The Model Gateway as a library, so one process can run it: `saas-node` (Agentrix SaaS)
//! serves it on a loopback port of its own. The `agentrix-models` binary is the same gateway on
//! its own (see `main.rs`); both start it through [`router`].
//!
//! - [`router`]: routing, purposes, budgets and metering exactly as the binary has them.
//! - [`init_control_db`]: for a node without agentrix-node, creates the control database the
//!   gateway meters into (agentrix-node's `0001_control.sql`, the schema its own tests use) and
//!   registers the sites that call it. It never touches a database that already exists.

// The gateway's modules were the binary's own; they are documented at module level.
#![allow(missing_docs)]

pub mod cache;
pub mod claude;
pub mod gateway;
pub mod meter;
pub mod route;
pub mod select;
pub mod upstream;

use std::path::{Path, PathBuf};

/// How to start the gateway.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The control database (agentrix-node's, or one [`init_control_db`] made).
    pub db: PathBuf,
    /// Vultr Serverless Inference's base URL.
    pub base_url: String,
    /// `live`, `record` or `replay` (design D10a).
    pub mode: String,
    /// Where recordings live; this crate's `recordings/` by default.
    pub recordings: Option<PathBuf>,
    /// The hard model-spend limit per site per day, in US dollars.
    pub daily_cap_usd: f64,
    /// The limit for every site together per UTC day, in US dollars. 0 refuses every call.
    pub fleet_daily_cap_usd: f64,
    /// The limit for the `-sample` sites together per UTC day, in US dollars.
    pub samples_daily_cap_usd: f64,
    /// A dotenv file to read provider keys from when the environment lacks them.
    pub env_file: Option<PathBuf>,
    /// The routing, when the caller read it from somewhere other than the process environment
    /// (`route::Routing::from_lookup`); `None` reads `AGENTRIX_PROVIDER` and the rest from it.
    pub routing: Option<route::Routing>,
}

/// The gateway's router, ready to serve on loopback.
pub async fn router(settings: &Settings) -> anyhow::Result<axum::Router> {
    for (name, usd) in [
        ("AGENTRIX_SITE_DAILY_CAP_USD", settings.daily_cap_usd),
        ("AGENTRIX_FLEET_DAILY_CAP_USD", settings.fleet_daily_cap_usd),
        (
            "AGENTRIX_SAMPLES_DAILY_CAP_USD",
            settings.samples_daily_cap_usd,
        ),
    ] {
        anyhow::ensure!(
            usd.is_finite() && usd >= 0.0,
            "{name} must be a dollar amount, not {usd}"
        );
    }
    tracing::info!(
        site = settings.daily_cap_usd,
        samples = settings.samples_daily_cap_usd,
        platform = settings.fleet_daily_cap_usd,
        "daily model spend limits in US dollars"
    );
    let mode = cache::Mode::parse(&settings.mode)?;
    let routing = match &settings.routing {
        Some(routing) => routing.clone(),
        None => route::Routing::from_env()?,
    };
    for (purpose, choice) in routing.table() {
        tracing::info!(
            purpose = purpose.as_str(),
            provider = choice.provider.as_str(),
            model = %choice.model,
            tier = choice.tier.map_or("specialist", route::Tier::as_str),
            "route"
        );
    }
    let (vultr, claude) = match mode {
        cache::Mode::Replay => (None, None),
        cache::Mode::Live | cache::Mode::Record => {
            let env_file = settings.env_file.as_deref();
            let vultr =
                upstream::Vultr::new(&settings.base_url, upstream::ApiKey::load(env_file)?)?;
            let claude = if routing.uses(route::Provider::Anthropic) {
                Some(claude::Claude::new(upstream::read_key(
                    "ANTHROPIC_API_KEY",
                    env_file,
                )?))
            } else {
                None
            };
            (Some(vultr), claude)
        }
    };
    let recordings = cache::Recordings::new(
        settings
            .recordings
            .clone()
            .unwrap_or_else(cache::Recordings::default_dir),
    );
    let pool = meter::open(&settings.db).await.map_err(|e| {
        anyhow::anyhow!(
            "cannot open {} (agentrix-node or init_control_db creates it): {e}",
            settings.db.display()
        )
    })?;
    let gateway = gateway::Gateway::new(gateway::Config {
        vultr,
        claude,
        pool,
        routing,
        mode,
        recordings,
        daily_cap_micro_usd: (settings.daily_cap_usd * 1e6).round() as i64,
        shared_caps: meter::SharedCaps {
            platform_micro_usd: (settings.fleet_daily_cap_usd * 1e6).round() as i64,
            samples_micro_usd: (settings.samples_daily_cap_usd * 1e6).round() as i64,
        },
    })?;
    tracing::info!(mode = mode.as_str(), "model gateway ready");
    Ok(gateway::router(gateway))
}

/// Makes the control database at `path` when there is none (agentrix-node's control schema,
/// with the `accounts` table its foreign keys name), and registers `sites` (`(site_id, domain)`)
/// so the gateway knows its callers. An existing database only gets the missing sites.
pub async fn init_control_db(path: &Path, sites: &[(&str, &str)]) -> anyhow::Result<()> {
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let fresh = !path.exists();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await?;
    if fresh {
        sqlx::raw_sql("CREATE TABLE accounts (id TEXT PRIMARY KEY);")
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!(
            "../../agentrix-node/src/db/migrations/0001_control.sql"
        ))
        .execute(&pool)
        .await?;
    }
    let now = meter::stamp(chrono::Utc::now());
    for (site, domain) in sites {
        sqlx::query(
            "INSERT OR IGNORE INTO sites (site_id, domain, name, old_origin, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(domain)
        .bind(site)
        .bind(format!("https://{domain}"))
        .bind(&now)
        .execute(&pool)
        .await?;
    }
    pool.close().await;
    Ok(())
}
