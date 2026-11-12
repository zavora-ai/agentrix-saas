//! Each company's keys and the key directory that publishes them.
//!
//! A company publishes one Web Bot Auth key directory at its agent door's
//! `/.well-known/http-message-signatures-directory`: on the demo node
//! `https://pay.agentrix.website/.well-known/…` for Agentrix and
//! `https://kilele.money.agentrix.website/.well-known/…` for Kilele Bakery. Each key's `kid`
//! names its role:
//!
//! - `owner`: seals the owner's mandates, and only after the owner approved the mandate's
//!   fingerprint with a passkey ([`crate::mandate`]). No agent signs with it.
//! - `agent:receivables`, `agent:payables`: the agents' own keys. They sign their A2A requests
//!   (RFC 9421) and what they sign for the company (invoices, cart mandates, receipts).
//!
//! The counterparty fetches the directory once and checks every signature against it, so a
//! cart mandate signed by the Payables agent can never pass for the owner's intent mandate.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use adk_auth::web_bot_auth::{
    DIRECTORY_MEDIA_TYPE, DIRECTORY_WELL_KNOWN_PATH, Ed25519SigningKey, FetchedDirectory,
    KeyDirectory, KeyDirectorySource, WebBotAuthError,
};
use async_trait::async_trait;
use axum::Router;
use axum::http::header;
use axum::routing::get;

/// The `kid` of the key that seals the owner's mandates.
pub const OWNER_KID: &str = "owner";
/// The Receivables agent (bills and collects).
pub const RECEIVABLES: &str = "receivables";
/// The Payables agent (checks bills against the owner's mandate and pays them).
pub const PAYABLES: &str = "payables";

/// The `kid` an agent's key is published under: `agent:<name>`.
pub fn agent_kid(agent: &str) -> String {
    format!("agent:{agent}")
}

/// One company's signing keys. `Debug` never shows a private key.
#[derive(Clone)]
pub struct CompanyKeys {
    /// The key directory origin, for example `https://pay.agentrix.website`.
    pub origin: String,
    /// Seals the owner's mandates after a passkey approval.
    pub owner: Arc<Ed25519SigningKey>,
    /// The agents' keys by name ([`RECEIVABLES`], [`PAYABLES`]).
    pub agents: BTreeMap<String, Arc<Ed25519SigningKey>>,
}

impl std::fmt::Debug for CompanyKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanyKeys")
            .field("origin", &self.origin)
            .field("owner", &self.owner.thumbprint())
            .field(
                "agents",
                &self
                    .agents
                    .iter()
                    .map(|(name, key)| (name.as_str(), key.thumbprint()))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl CompanyKeys {
    /// Fresh keys for `origin`: an owner key and one key for each of `agents`.
    pub fn generate(origin: &str, agents: &[&str]) -> anyhow::Result<Self> {
        let mut map = BTreeMap::new();
        for agent in agents {
            map.insert(
                (*agent).to_owned(),
                Arc::new(Ed25519SigningKey::generate()?),
            );
        }
        Ok(Self {
            origin: origin.trim_end_matches('/').to_owned(),
            owner: Arc::new(Ed25519SigningKey::generate()?),
            agents: map,
        })
    }

    /// Keys kept as private JWK files in `dir` (`owner.jwk`, `<agent>.jwk`), each mode 0600,
    /// created on first use. Keep `dir` outside the repository.
    pub fn load_or_create(origin: &str, agents: &[&str], dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let load = |name: &str| -> anyhow::Result<Arc<Ed25519SigningKey>> {
            let path = dir.join(format!("{name}.jwk"));
            if path.exists() {
                let jwk = std::fs::read_to_string(&path)?;
                return Ok(Arc::new(Ed25519SigningKey::from_jwk(jwk.trim())?));
            }
            let key = Ed25519SigningKey::generate()?;
            write_private(&path, key.to_private_jwk().as_bytes())?;
            Ok(Arc::new(key))
        };
        let mut map = BTreeMap::new();
        for agent in agents {
            map.insert((*agent).to_owned(), load(agent)?);
        }
        Ok(Self {
            origin: origin.trim_end_matches('/').to_owned(),
            owner: load(OWNER_KID)?,
            agents: map,
        })
    }

    /// The agent's key.
    pub fn agent(&self, name: &str) -> anyhow::Result<Arc<Ed25519SigningKey>> {
        self.agents
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{} has no {name} agent key", self.origin))
    }

    /// The published directory: every key, its `kid` naming its role.
    pub fn directory(&self) -> KeyDirectory {
        let mut directory = KeyDirectory::from_keys(
            std::iter::once(self.owner.as_ref()).chain(self.agents.values().map(AsRef::as_ref)),
        );
        for key in &mut directory.keys {
            let Ok(thumbprint) = key.thumbprint() else {
                continue;
            };
            if thumbprint == self.owner.thumbprint() {
                key.kid = Some(OWNER_KID.into());
            } else if let Some((name, _)) = self
                .agents
                .iter()
                .find(|(_, k)| k.thumbprint() == thumbprint)
            {
                key.kid = Some(agent_kid(name));
            }
        }
        directory
    }

    /// Serves [`CompanyKeys::directory`] at `/.well-known/http-message-signatures-directory`.
    pub fn directory_router(&self) -> Router {
        let json = self.directory().to_json();
        Router::new().route(
            DIRECTORY_WELL_KNOWN_PATH,
            get(move || {
                let json = json.clone();
                async move {
                    (
                        [
                            (header::CONTENT_TYPE, DIRECTORY_MEDIA_TYPE),
                            (header::CACHE_CONTROL, "max-age=300"),
                        ],
                        json,
                    )
                }
            }),
        )
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)?;
        Ok(())
    }
}

/// Key directories on this machine, for running both companies' doors locally: origins whose
/// host is `localhost`, a `*.localhost` name (RFC 6761: always loopback) or `127.0.0.1`, fetched
/// over plain HTTP from 127.0.0.1 on the origin's port. Anything else is refused; on the demo
/// node, use adk-auth's `HttpKeyDirectorySource` (https and public addresses only).
#[derive(Debug, Default)]
pub struct LoopbackDirectorySource;

impl LoopbackDirectorySource {
    /// The source.
    pub fn new() -> Self {
        Self
    }
}

/// `(host, port)` when `origin` is a loopback origin.
pub fn loopback_host(origin: &str) -> Option<(String, u16)> {
    let url = url::Url::parse(origin).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    let loopback = host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1";
    loopback.then_some((host, port))
}

/// A reqwest client that reaches `*.localhost` names on 127.0.0.1, as browsers do.
pub fn loopback_client(origins: &[&str]) -> reqwest::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10));
    for origin in origins {
        if let Some((host, port)) = loopback_host(origin) {
            builder = builder.resolve(&host, SocketAddr::from(([127, 0, 0, 1], port)));
        }
    }
    builder.build()
}

#[async_trait]
impl KeyDirectorySource for LoopbackDirectorySource {
    async fn fetch(&self, origin: &str) -> Result<FetchedDirectory, WebBotAuthError> {
        let unavailable = |reason: String| WebBotAuthError::DirectoryUnavailable {
            origin: origin.to_owned(),
            reason,
        };
        if loopback_host(origin).is_none() {
            return Err(WebBotAuthError::EgressRefused {
                origin: origin.to_owned(),
                reason: "only loopback origins are fetched locally".into(),
            });
        }
        let client = loopback_client(&[origin]).map_err(|e| unavailable(e.to_string()))?;
        let url = format!(
            "{}{}",
            origin.trim_end_matches('/'),
            DIRECTORY_WELL_KNOWN_PATH
        );
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(unavailable(format!("HTTP {}", response.status())));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        let directory = KeyDirectory::parse(&bytes, 64).map_err(|reason| {
            WebBotAuthError::MalformedDirectory {
                origin: origin.to_owned(),
                reason,
            }
        })?;
        Ok(FetchedDirectory {
            directory,
            max_age: Some(Duration::from_secs(60)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_names_each_role() {
        let keys = CompanyKeys::generate("https://kilele.example/", &[PAYABLES]).unwrap();
        assert_eq!(keys.origin, "https://kilele.example");
        let directory = keys.directory();
        let owner = directory.find(keys.owner.thumbprint()).unwrap();
        assert_eq!(owner.kid.as_deref(), Some(OWNER_KID));
        let payables = keys.agent(PAYABLES).unwrap();
        let agent = directory.find(payables.thumbprint()).unwrap();
        assert_eq!(agent.kid.as_deref(), Some("agent:payables"));
        let parsed = KeyDirectory::parse(directory.to_json().as_bytes(), 64).unwrap();
        assert_eq!(
            parsed.find(keys.owner.thumbprint()).unwrap().kid.as_deref(),
            Some(OWNER_KID),
            "the role survives publishing"
        );
        assert!(!format!("{keys:?}").contains("\"d\""));
    }

    #[test]
    fn only_loopback_origins_are_local() {
        assert!(loopback_host("http://kilele.localhost:18182").is_some());
        assert!(loopback_host("http://127.0.0.1:18182").is_some());
        assert!(loopback_host("https://pay.agentrix.website").is_none());
        assert!(loopback_host("http://10.0.0.5:18182").is_none());
        assert!(loopback_host("http://localhost.evil.example").is_none());
    }

    #[test]
    fn keys_persist_private_and_reload() {
        let dir = std::env::temp_dir().join(format!("saas-settle-keys-{}", uuid::Uuid::new_v4()));
        let a = CompanyKeys::load_or_create("https://a.example", &[RECEIVABLES], &dir).unwrap();
        let b = CompanyKeys::load_or_create("https://a.example", &[RECEIVABLES], &dir).unwrap();
        assert_eq!(a.owner.thumbprint(), b.owner.thumbprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("owner.jwk"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
