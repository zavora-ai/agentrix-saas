//! The node's settings: the repository's `.env` (read line by line, never sourced), with the
//! process environment taking precedence, and the command line on top.
//!
//! A value that starts with `#` is empty (an unset key with a comment). No value is ever logged;
//! [`Config::summary`] says only which settings are present.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agentrix_effects::paypal::{Environment, PayPalConfig};
use saas_web::hosts::Hosts;

/// `KEY=VALUE` pairs from an env file.
#[derive(Debug, Clone, Default)]
pub struct EnvFile {
    values: BTreeMap<String, String>,
}

impl EnvFile {
    /// Parses `text`: `KEY=VALUE` lines; blank lines and `#` comments are skipped; quotes around
    /// a value are dropped; a value starting with `#` is empty.
    pub fn parse(text: &str) -> Self {
        let mut values = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let mut value = value.trim();
            if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                value = &value[1..value.len() - 1];
            }
            if value.starts_with('#') {
                value = "";
            }
            values.insert(key.trim().to_owned(), value.to_owned());
        }
        Self { values }
    }

    /// Reads `path`; a missing file is empty.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// `key`, from the process environment first, then the file; empty is none.
    pub fn get(&self, key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty() && !v.starts_with('#'))
            .or_else(|| self.values.get(key).cloned())
            .filter(|v| !v.is_empty())
    }
}

/// Which books the companies keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BooksMode {
    /// saas-core's in-memory FakeBooks, seeded with the story.
    Fake,
    /// Zavora ERA (hosted), through saas-books' EraBooks.
    Era,
}

/// Where ERA's per-company logins are: `SAAS_ERA_TENANTS_FILE`, or
/// `~/.config/agentrix-saas/era/tenants.json` (what `deploy/saas/era` writes).
pub fn era_tenants_file(env: &EnvFile) -> PathBuf {
    env.get("SAAS_ERA_TENANTS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config/agentrix-saas/era/tenants.json")
        })
}

impl BooksMode {
    /// ERA when it is configured (its tenants file is there), else the fake books.
    pub fn detect(env: &EnvFile) -> Self {
        if era_tenants_file(env).is_file() {
            Self::Era
        } else {
            Self::Fake
        }
    }
}

/// One company's PayPal app.
#[derive(Clone)]
pub struct PayPalApp {
    /// What agentrix-effects talks to PayPal with.
    pub config: PayPalConfig,
    /// The app's merchant (receiver) email.
    pub merchant_email: Option<String>,
}

impl std::fmt::Debug for PayPalApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayPalApp")
            .field("environment", &self.config.environment)
            .field("webhook", &self.config.webhook_id.is_some())
            .finish_non_exhaustive()
    }
}

/// The node's settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// The four public hosts.
    pub hosts: Hosts,
    /// The platform host the money layer's names sit under (`paypal.agentrix.website`), when the
    /// node knows it: agentrix-node's, or `SAAS_PLATFORM_HOST`. Wallet keys are named for it.
    pub platform_host: Option<String>,
    /// Where the node keeps its databases and keys (outside the repository).
    pub data_dir: PathBuf,
    /// Which books.
    pub books: BooksMode,
    /// Judge mode: scripted models, in-memory stores, the attack demos at `/judge/`.
    pub judge: bool,
    /// Money is faked in memory (no PayPal app configured, or `--fake-money`): no PayPal call,
    /// no agent door.
    pub fake_money: bool,
    /// Kilele's cash on hand, in cents, when the story makes it short (an invoice over it asks
    /// for instalments). `None`: no limit.
    pub kilele_cash_minor: Option<i64>,
    /// Agentrix's PayPal app (its webhooks come to `/paypal/webhook`).
    pub agentrix_paypal: Option<PayPalApp>,
    /// Kilele Bakery's PayPal app.
    pub kilele_paypal: Option<PayPalApp>,
    /// A Model Gateway already running elsewhere (`AGENTRIX_MODELS_URL`); `None` runs
    /// agentrix-models inside this process, on a loopback port of its own.
    pub models_url: Option<String>,
    /// The settings file the node read (provider keys for the embedded gateway).
    pub env_file: Option<PathBuf>,
    /// Everything the settings file holds, for the embedded gateway's routing.
    pub env: EnvFile,
    /// Zavora ERA's URL, forwarded on the books host.
    pub era_url: Option<String>,
    /// ERA's API, for the companies' books (`SAAS_ERA_URL`, default `http://127.0.0.1:18180`).
    pub era_api: String,
    /// ERA's per-company logins.
    pub era_tenants: PathBuf,
    /// The owner's console token.
    pub owner_token: Option<String>,
    /// Anyone may press the console's buttons (local demo, judging day).
    pub console_open: bool,
    /// AG Studio's licence key, for the board.
    pub board_licence: Option<String>,
    /// The board's built files.
    pub board_dist: Option<PathBuf>,
    /// The founders' emails.
    pub agentrix_founder: String,
    /// Kilele Bakery's founder.
    pub kilele_founder: String,
}

fn paypal_env(env: &EnvFile) -> Environment {
    match env.get("SAAS_PAYPAL_BASE_URL") {
        Some(url) => Environment::Custom(url),
        None => match env.get("PAYPAL_ENV").as_deref() {
            Some("live") => Environment::Live,
            _ => Environment::Sandbox,
        },
    }
}

fn app(env: &EnvFile, prefix: &str, webhook: Option<String>) -> Option<PayPalApp> {
    let client_id = env.get(&format!("{prefix}CLIENT_ID"))?;
    let client_secret = env.get(&format!("{prefix}CLIENT_SECRET"))?;
    Some(PayPalApp {
        config: PayPalConfig {
            client_id,
            client_secret,
            webhook_id: webhook,
            environment: paypal_env(env),
        },
        merchant_email: env.get(&format!("{prefix}MERCHANT_EMAIL")),
    })
}

/// The default data directory: `~/.local/share/agentrix-saas` (`/var/lib/agentrix-saas` on
/// the VM, through `SAAS_DATA_DIR`).
pub fn default_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/agentrix-saas")
}

impl Config {
    /// The settings in `env`.
    pub fn from_env(env: &EnvFile, books: BooksMode, judge: bool) -> Self {
        let d = Hosts::default();
        let host = |key: &str, default: String| {
            env.get(key)
                .map(|v| v.to_ascii_lowercase())
                .unwrap_or(default)
        };
        let paypal_live = env.get("PAYPAL_ENV").as_deref() == Some("live");
        if paypal_live {
            tracing::warn!("PAYPAL_ENV=live is ignored: Agentrix SaaS runs on PayPal's sandbox");
        }
        let mut agentrix_paypal = app(env, "PAYPAL_", env.get("PAYPAL_WEBHOOK_ID"));
        let mut kilele_paypal = app(env, "KILELE_PAYPAL_", None);
        for a in [&mut agentrix_paypal, &mut kilele_paypal]
            .into_iter()
            .flatten()
        {
            if matches!(a.config.environment, Environment::Live) {
                a.config.environment = Environment::Sandbox;
            }
        }
        Self {
            hosts: Hosts {
                money: host("SAAS_HOST", d.money),
                kilele: host("SAAS_KILELE_HOST", d.kilele),
                pay: host("SAAS_AGENT_DOOR_HOST", d.pay),
                books: host("SAAS_BOOKS_HOST", d.books),
            },
            platform_host: env
                .get("SAAS_PLATFORM_HOST")
                .map(|v| v.to_ascii_lowercase()),
            data_dir: env
                .get("SAAS_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(default_data_dir),
            books,
            judge,
            fake_money: agentrix_paypal.is_none(),
            kilele_cash_minor: env
                .get("SAAS_KILELE_CASH_USD")
                .and_then(|v| v.parse::<f64>().ok())
                .map(|usd| (usd * 100.0).round() as i64),
            agentrix_paypal,
            kilele_paypal,
            models_url: env.get("AGENTRIX_MODELS_URL"),
            env_file: None,
            env: env.clone(),
            era_url: env.get("SAAS_ERA_URL"),
            era_api: env
                .get("SAAS_ERA_URL")
                .unwrap_or_else(|| "http://127.0.0.1:18180".into()),
            era_tenants: era_tenants_file(env),
            owner_token: env.get("SAAS_OWNER_TOKEN"),
            console_open: judge || env.get("SAAS_CONSOLE_OPEN").as_deref() == Some("1"),
            board_licence: env.get("AG_STUDIO_LICENSE_KEY"),
            board_dist: env.get("SAAS_BOARD_DIST").map(PathBuf::from),
            agentrix_founder: env
                .get("SAAS_AGENTRIX_FOUNDER")
                .unwrap_or_else(|| "founder@agentrix.website".into()),
            kilele_founder: env
                .get("SAAS_KILELE_FOUNDER")
                .unwrap_or_else(|| "owner@kilele.example".into()),
        }
    }

    /// Which settings are present, without any value.
    pub fn summary(&self) -> String {
        format!(
            "books={:?} judge={} paypal(agentrix)={} paypal(kilele)={} webhook={} era={} data_dir={}",
            self.books,
            self.judge,
            self.agentrix_paypal.is_some(),
            self.kilele_paypal.is_some(),
            self.agentrix_paypal
                .as_ref()
                .is_some_and(|a| a.config.webhook_id.is_some()),
            self.era_url.is_some(),
            self.data_dir.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_env_file_is_read_line_by_line() {
        let env = EnvFile::parse(
            "# a comment\nPAYPAL_CLIENT_ID=abc\nPAYPAL_CLIENT_SECRET=\"s e c\"\nPAYPAL_WEBHOOK_ID=# not set yet\nexport SAAS_HOST=Money.Example\nbroken line\n",
        );
        assert_eq!(env.get("PAYPAL_CLIENT_ID").as_deref(), Some("abc"));
        assert_eq!(env.get("PAYPAL_CLIENT_SECRET").as_deref(), Some("s e c"));
        assert_eq!(env.get("PAYPAL_WEBHOOK_ID"), None);
        let config = Config::from_env(&env, BooksMode::Fake, false);
        assert_eq!(config.hosts.money, "money.example");
        let app = config.agentrix_paypal.clone().expect("Agentrix app");
        assert!(app.config.webhook_id.is_none());
        assert!(config.kilele_paypal.is_none());
        assert!(!config.summary().contains("abc"));
    }
}
