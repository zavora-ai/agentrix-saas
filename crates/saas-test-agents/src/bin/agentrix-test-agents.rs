//! `agentrix-test-agents`: serves the test customer agents' hosts and runs their visits.
//! `agentrix-test-agents once <agent> [site]` makes one visit now and exits (a smoke test); the
//! site is a sample site's host label, such as `tana-valley-times-sample`, or picked by the
//! agent's weights when left out.

use std::sync::Arc;

use saas_test_agents::activity::Log;
use saas_test_agents::brain::Brain;
use saas_test_agents::config::Config;
use saas_test_agents::identity::Agent;
use saas_test_agents::pay::Buyers;
use saas_test_agents::runner::{self, Shared};
use saas_test_agents::shops;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.data_dir)?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let agents = Agent::all(&config)?;
    let shared = Arc::new(Shared {
        brain: Brain::new(&config, http.clone()),
        log: Log::new(config.data_dir.join("activity.jsonl")),
        buyers: Buyers::new(&config, http.clone()),
        started: chrono::Utc::now(),
        http,
        config,
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("once") {
        let slug = args.get(1).cloned().unwrap_or_else(|| "amani".into());
        let agent = agents
            .iter()
            .find(|a| a.slug == slug)
            .ok_or_else(|| anyhow::anyhow!("no agent {slug}"))?;
        match args.get(2) {
            Some(site) => {
                let shop = shops::by_slug(site).ok_or_else(|| {
                    anyhow::anyhow!(
                        "no sample site {site} (one of: {})",
                        shops::SHOPS.map(|s| s.slug).join(", ")
                    )
                })?;
                runner::visit_shop(&shared, agent, shop).await?;
            }
            None => runner::visit(&shared, agent).await?,
        }
        for a in shared.log.recent(20).into_iter().rev() {
            println!(
                "{} {} {} {}",
                a.kind,
                if a.ok { "ok" } else { "NOT OK" },
                a.site,
                a.detail
            );
        }
        return Ok(());
    }
    let app = saas_test_agents::server::app(shared.clone(), &agents);
    let listener = tokio::net::TcpListener::bind(&shared.config.listen).await?;
    tracing::info!(listen = %shared.config.listen, agents = agents.len(), "test customer agents serving");
    for agent in agents {
        tokio::spawn(runner::run(shared.clone(), agent));
    }
    axum::serve(listener, app).await?;
    Ok(())
}
