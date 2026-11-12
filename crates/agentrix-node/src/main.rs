//! `agentrix-node`: the single-node server (design §3, §4.1).
//!
//! Edge, control and kernel in one process: public pages, Assess, sign-in, the console, sites
//! served by `Host`, Ask over SSE, the agent door, the Move orchestrator and the broker client.

use clap::Parser as _;

use agentrix_node::{app, state::Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let config = Config::parse();
    let app = app::build(&config).await?;

    let public = tokio::net::TcpListener::bind(config.listen).await?;
    let internal = tokio::net::TcpListener::bind(config.internal_listen).await?;
    anyhow::ensure!(
        config.internal_listen.ip().is_loopback(),
        "the internal listener must be on loopback"
    );
    tracing::info!(listen = %config.listen, internal = %config.internal_listen, origin = %config.origin, "agentrix-node up");

    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("shutting down");
    };
    tokio::select! {
        r = axum::serve(public, app.public) => r?,
        r = axum::serve(internal, app.internal) => r?,
        () = shutdown => {}
    }
    Ok(())
}
