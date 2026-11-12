//! Settings, from the environment (the unit's `EnvironmentFile`). Every host follows from the
//! platform host; nothing is built in.

use std::path::PathBuf;

use chrono::Duration;

/// The three test customer agents: slug, name, and what they shop for.
pub const AGENTS: [(&str, &str, &str); 3] = [
    (
        "amani",
        "Amani",
        "a home cook in Nairobi who buys bread and pastries for the weekend",
    ),
    (
        "baraka",
        "Baraka",
        "buys gifts for family occasions: a perfume, now and then a celebration cake",
    ),
    (
        "chiku",
        "Chiku",
        "reads the local news and orders small treats for the office",
    ),
];

/// The label every place a test agent shows carries.
pub const LABEL: &str = "Test customer agent";

/// The sample sites the agents visit, as host slugs under the platform host.
pub const SITES: [&str; 3] = [
    "kilele-bakery-sample",
    "classy-perfumes-sample",
    "tana-valley-times-sample",
];

/// The settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// `paypal.agentrix.website`; `localhost:18182` on a laptop.
    pub platform_host: String,
    /// `https`, or `http` for loopback.
    pub scheme: String,
    /// Where this process listens (Caddy proxies the agents' hosts here).
    pub listen: String,
    /// Keys, state and the activity log; outside the repository.
    pub data_dir: PathBuf,
    /// The model endpoint (OpenAI-compatible), its key and the fast model.
    pub model_base: Option<String>,
    /// The model key (never logged).
    pub model_key: Option<String>,
    /// The model name.
    pub model: String,
    /// Each agent's model budget per UTC day, in US dollars.
    pub daily_model_usd: f64,
    /// The price used to count spend, per million tokens.
    pub usd_per_mtok: f64,
    /// Minutes between an agent's visits.
    pub every_min: (u64, u64),
    /// How often a visit is an impostor's.
    pub impostor_rate: f64,
    /// How long the agents run before they stop by themselves.
    pub run_for: Duration,
    /// Present: paused. The operator console (or anyone on the VM) creates it to stop them.
    pub off_file: PathBuf,
    /// Bearer token for `/control/*` on the control host; unset turns those routes off.
    pub control_token: Option<String>,
    /// The PayPal app the sample sites take payments with (sandbox only).
    pub paypal: Option<(String, String)>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && !v.starts_with('#'))
}

impl Config {
    /// From the environment.
    pub fn from_env() -> anyhow::Result<Self> {
        let platform_host = var("TEST_AGENTS_PLATFORM_HOST")
            .or_else(|| var("AGENTRIX_PLATFORM_HOST"))
            .unwrap_or_else(|| "paypal.agentrix.website".into());
        let loopback =
            platform_host.starts_with("localhost") || platform_host.contains(".localhost");
        let data_dir = var("TEST_AGENTS_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                    .join(".config/agentrix-saas/test-agents")
            });
        let paypal = match (
            var("PAYPAL_ENV").as_deref(),
            var("PAYPAL_CLIENT_ID"),
            var("PAYPAL_CLIENT_SECRET"),
        ) {
            (Some("sandbox"), Some(id), Some(secret)) => Some((id, secret)),
            _ => None,
        };
        let num =
            |name: &str, default: f64| var(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        Ok(Self {
            scheme: var("TEST_AGENTS_SCHEME").unwrap_or_else(|| {
                if loopback {
                    "http".into()
                } else {
                    "https".into()
                }
            }),
            listen: var("TEST_AGENTS_LISTEN").unwrap_or_else(|| "127.0.0.1:18190".into()),
            off_file: var("TEST_AGENTS_OFF_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("off")),
            model_base: var("VULTR_INFERENCE_BASE_URL"),
            model_key: var("VULTR_INFERENCE_API_KEY"),
            model: var("TEST_AGENTS_MODEL")
                .or_else(|| var("AGENTRIX_TIER_FAST_VULTR"))
                .unwrap_or_else(|| "llama-3.1-8b-instruct".into()),
            daily_model_usd: num("TEST_AGENTS_DAILY_MODEL_USD", 0.25),
            usd_per_mtok: num("TEST_AGENTS_USD_PER_MTOK", 0.5),
            every_min: (
                num("TEST_AGENTS_MIN_MINUTES", 20.0) as u64,
                num("TEST_AGENTS_MAX_MINUTES", 60.0) as u64,
            ),
            impostor_rate: num("TEST_AGENTS_IMPOSTOR_RATE", 0.02),
            run_for: Duration::minutes((num("TEST_AGENTS_HOURS", 12.0) * 60.0) as i64),
            control_token: var("TEST_AGENTS_CONTROL_TOKEN"),
            paypal,
            platform_host,
            data_dir,
        })
    }

    /// An origin on the platform: `https://<label>.<platform host>`.
    pub fn origin(&self, label: &str) -> String {
        format!("{}://{label}.{}", self.scheme, self.platform_host)
    }

    /// A test agent's host: `test-agent-<slug>.<platform host>`.
    pub fn agent_host(&self, slug: &str) -> String {
        format!("test-agent-{slug}.{}", self.platform_host)
    }

    /// The control host: `test-agents.<platform host>`.
    pub fn control_host(&self) -> String {
        format!("test-agents.{}", self.platform_host)
    }

    /// The sample sites' origins.
    pub fn sites(&self) -> Vec<String> {
        SITES.iter().map(|s| self.origin(s)).collect()
    }

    /// Whether the agents are paused.
    pub fn is_off(&self) -> bool {
        self.off_file.exists()
    }
}
