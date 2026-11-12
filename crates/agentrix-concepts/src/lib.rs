//! The concept stage: between the health report and the Move, agents design three different
//! versions of the owner's new Agentrix Website, in parallel and live on screen, and the owner
//! chooses one.
//!
//! - Only an owner whose claim on the site is verified sees it (generation costs money).
//! - "Design three looks" starts a run ([`run::Engine`]): one planner call (`glm-5.3`) reads the
//!   site's facts from its health report ([`facts`]) and writes three clearly different
//!   directions; then three Zavora CLI agents (`deepseek-v4.1-flash-1017`) write one complete home
//!   page each, in parallel, with the samples' example page and `kit.css` in their rooms
//!   ([`prompts`], [`runner`]).
//! - The page follows the run over SSE ([`routes`]): three columns, what each agent is doing, and
//!   previews that fill in as the pages arrive. Every page is cleaned ([`sanitize`]) and shown in
//!   `<iframe sandbox>` with a strict CSP.
//! - "Choose this look" records the choice ([`store::chosen`]); the Move page shows it and the
//!   Move's rebuild takes its colours and fonts from it ([`look`]).
//!
//! Designers write `index.html` in their own rooms with `fs_write` (the one tool approved), so
//! the page grows on screen; each finished page is checked for the fundamentals ([`check`]) and
//! sent back to fix, and the owner can tell any designer what to change. The page also shows,
//! from this run's records, how the stage meets Vultr Agent Rush's Challenge 1 ([`evidence`]).
//!
//! Settings come from node's environment: `AGENTRIX_ZAVORA_BIN` (default
//! `/opt/agentrix/bin/zavora-cli`) and `VULTR_INFERENCE_API_KEY`. Without either, the stage
//! hides itself with a plain message and the owner goes straight on to the Move.

pub mod check;
pub mod evidence;
pub mod facts;
pub mod look;
pub mod prompts;
pub mod receipts;
pub mod routes;
pub mod run;
pub mod runner;
pub mod sanitize;
pub mod store;
pub mod stream;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use store::{chosen, migrate};

/// The default Zavora CLI binary on a node.
pub const DEFAULT_ZAVORA_BIN: &str = "/opt/agentrix/bin/zavora-cli";
/// The planner's model.
pub const DEFAULT_PLANNER_MODEL: &str = "glm-5.3";
/// The designers' model (James prefers DeepSeek for code).
pub const DEFAULT_WORKER_MODEL: &str = "deepseek-v4.1-flash-1017";

/// Seconds before a fix or an owner's change is stopped (a first build gets
/// [`run::BUILD_SECS`]).
pub const AGENT_SECS: u64 = 300;

/// The concept stage's settings.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Zavora CLI.
    pub zavora_bin: PathBuf,
    /// The Vultr Inference key, if set.
    pub api_key: Option<String>,
    /// The planner's model.
    pub planner_model: String,
    /// The designers' model.
    pub worker_model: String,
    /// The longest one agent may run.
    pub timeout: Duration,
}

impl Settings {
    /// Reads `AGENTRIX_ZAVORA_BIN`, `VULTR_INFERENCE_API_KEY`, and optionally
    /// `AGENTRIX_CONCEPTS_PLANNER_MODEL` and `AGENTRIX_CONCEPTS_WORKER_MODEL`.
    pub fn from_env() -> Self {
        let var = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        Self {
            zavora_bin: var("AGENTRIX_ZAVORA_BIN")
                .unwrap_or_else(|| DEFAULT_ZAVORA_BIN.into())
                .into(),
            api_key: var("VULTR_INFERENCE_API_KEY"),
            planner_model: var("AGENTRIX_CONCEPTS_PLANNER_MODEL")
                .unwrap_or_else(|| DEFAULT_PLANNER_MODEL.into()),
            worker_model: var("AGENTRIX_CONCEPTS_WORKER_MODEL")
                .unwrap_or_else(|| DEFAULT_WORKER_MODEL.into()),
            timeout: Duration::from_secs(AGENT_SECS),
        }
    }

    /// Why the stage is off, in words for the owner; `None` when it's ready.
    pub fn missing(&self) -> Option<&'static str> {
        if self.api_key.is_none() {
            return Some(
                "Designing new looks isn't set up yet: Agentrix has no key for the design models.",
            );
        }
        if !self.zavora_bin.is_file() {
            return Some("Designing new looks isn't set up yet: the designers aren't installed.");
        }
        None
    }
}

/// Mounts the concept stage on the platform host with node's settings ([`Settings::from_env`]).
pub fn mount(ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    let settings = Settings::from_env();
    let runner: Option<Arc<dyn runner::AgentRunner>> = match (settings.missing(), &settings.api_key)
    {
        (None, Some(key)) => Some(Arc::new(runner::ZavoraRunner {
            bin: settings.zavora_bin.clone(),
            api_key: key.clone(),
            rooms: rooms_dir(ctx),
            timeout: settings.timeout,
            max_prompt_chars: prompts::MAX_PROMPT_CHARS,
        })),
        _ => None,
    };
    mount_with(ctx, &settings, runner)
}

/// Mounts with a given runner (tests pass a fake Zavora); `None` hides the stage.
pub fn mount_with(
    ctx: &agentrix_host::NodeContext,
    settings: &Settings,
    runner: Option<Arc<dyn runner::AgentRunner>>,
) -> anyhow::Result<agentrix_host::Mounted> {
    let pool = ctx.pool.clone();
    let off = runner.is_none().then(|| {
        settings
            .missing()
            .unwrap_or("Designing new looks isn't set up yet.")
    });
    let engine = runner.map(|runner| run::Engine {
        pool: ctx.pool.clone(),
        blobs: ctx.blobs.clone(),
        receipts: ctx.receipts.clone(),
        rooms: rooms_dir(ctx),
        runner,
        models: run::Models {
            planner: settings.planner_model.clone(),
            worker: settings.worker_model.clone(),
        },
        hub: Arc::new(run::Hub::default()),
    });
    let limits = evidence::Limits {
        build_secs: run::BUILD_SECS,
        agent_secs: settings.timeout.as_secs(),
        max_prompt_chars: prompts::MAX_PROMPT_CHARS,
    };
    let sweeper = engine.clone();
    let routes = routes::Routes::new(ctx, engine, off, limits)?;
    Ok(agentrix_host::Mounted::default()
        .platform(routes::router(routes))
        .task(
            "concepts: close interrupted runs and delete rooms after 24 hours",
            Box::pin(async move {
                match store::fail_unfinished(&pool).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(n, "concepts: marked interrupted runs as stopped"),
                    Err(e) => tracing::warn!(error = %e, "concepts: closing interrupted runs"),
                }
                let Some(engine) = sweeper else {
                    return;
                };
                loop {
                    match store::runs_with_rooms(&pool, None, chrono::Duration::hours(24)).await {
                        Ok(runs) => {
                            for run in runs {
                                if let Err(e) = engine.delete_rooms(&run).await {
                                    tracing::warn!(error = %e, "concepts: deleting old rooms");
                                }
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "concepts: finding old rooms"),
                    }
                    tokio::time::sleep(Duration::from_secs(600)).await;
                }
            }),
        ))
}

/// Where designers' rooms live: `data_dir/agentrix-concepts/rooms`.
fn rooms_dir(ctx: &agentrix_host::NodeContext) -> PathBuf {
    ctx.data_dir.join("agentrix-concepts").join("rooms")
}
