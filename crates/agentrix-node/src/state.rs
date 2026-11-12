//! Configuration and the node's own keys (integration lane, design §2, D6).

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::Parser;
use url::Url;

/// `agentrix-node` configuration: flags, each with an environment fallback.
#[derive(Debug, Clone, Parser)]
#[command(version, about = "Agentrix single-node server")]
pub struct Config {
    /// Public listener (Caddy proxies to it).
    #[arg(long, env = "AGENTRIX_LISTEN", default_value = "127.0.0.1:8080")]
    pub listen: SocketAddr,
    /// Loopback-only listener for Caddy's on-demand TLS ask.
    #[arg(
        long,
        env = "AGENTRIX_INTERNAL_LISTEN",
        default_value = "127.0.0.1:8079"
    )]
    pub internal_listen: SocketAddr,
    /// Where the database, the content store and the node's keys live.
    #[arg(long, env = "AGENTRIX_DATA_DIR", default_value = "./var")]
    pub data_dir: PathBuf,
    /// The platform's origin as browsers see it, e.g. `https://agentrix.website`.
    #[arg(long, env = "AGENTRIX_ORIGIN", default_value = "http://127.0.0.1:8080")]
    pub origin: Url,
    /// The Model Gateway (`agentrix-models`), localhost only.
    #[arg(
        long,
        env = "AGENTRIX_MODELS_URL",
        default_value = "http://127.0.0.1:8090"
    )]
    pub models_url: String,
    /// The runner on the sandbox VM, e.g. `http://10.42.0.4:7443`. Unset: a fake runner.
    #[arg(long, env = "AGENTRIX_RUNNER")]
    pub runner: Option<String>,
    /// Zavora's own fixture domain, labelled as a fixture everywhere (R3.3).
    #[arg(long, env = "AGENTRIX_FIXTURE_DOMAIN")]
    pub fixture_domain: Option<String>,
    /// The gauntlet token (one line). Under systemd it comes in as a credential
    /// (`LoadCredential=`), so the root-only secrets directory stays closed to node.
    #[arg(long, env = "AGENTRIX_GAUNTLET_TOKEN_FILE")]
    pub gauntlet_token_file: Option<PathBuf>,
}

impl Config {
    /// The host the platform's own pages answer on.
    pub fn platform_host(&self) -> String {
        self.origin
            .host_str()
            .unwrap_or("localhost")
            .to_ascii_lowercase()
    }

    /// The database file.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("control.db")
    }

    /// The content-addressed store.
    pub fn cas_dir(&self) -> PathBuf {
        self.data_dir.join("cas")
    }

    /// Control's Ed25519 signing key (receipts, job specs).
    pub fn signing_key_path(&self) -> PathBuf {
        self.data_dir.join("keys").join("control.ed25519")
    }

    /// The sealing key for TOTP seeds (09 §2.3).
    pub fn seal_key_path(&self) -> PathBuf {
        self.data_dir.join("keys").join("seal.key")
    }
}

/// Reads a 32-byte key file, creating it with fresh random bytes (mode 600) if it is absent.
pub fn key_file(path: &Path) -> anyhow::Result<[u8; 32]> {
    if let Ok(bytes) = std::fs::read(path) {
        return bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("{} must hold exactly 32 bytes", path.display()));
    }
    let dir = path.parent().context("key path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let key: [u8; 32] = rand::random();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(&key)?;
    tracing::info!(path = %path.display(), "created a new key file");
    Ok(key)
}
