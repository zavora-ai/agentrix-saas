//! Each test agent's identity: its own key directory and `did:web`, on its own host
//! (`test-agent-<slug>.<platform host>`), every page and document labelled "Test customer agent".

use std::sync::Arc;

use adk_auth::web_bot_auth::{ComponentId, Ed25519SigningKey, HttpSignatureSigner};
use saas_settle::door::CONTENT_DIGEST;
use saas_settle::keys::CompanyKeys;
use serde_json::{Value, json};

use crate::config::{Config, LABEL};

/// The key a test agent signs its requests with.
pub const SHOPPER: &str = "shopper";

/// One test customer agent.
#[derive(Clone)]
pub struct Agent {
    /// `amani`.
    pub slug: String,
    /// `Amani`.
    pub name: String,
    /// What it shops for, for the model.
    pub persona: String,
    /// `test-agent-amani.paypal.agentrix.website`.
    pub host: String,
    /// `https://test-agent-amani.paypal.agentrix.website`.
    pub origin: String,
    /// Its keys (owner and `shopper`), kept in the data directory.
    pub keys: CompanyKeys,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("slug", &self.slug)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl Agent {
    /// The agents, their keys loaded or made (mode 0600) under `config.data_dir`.
    pub fn all(config: &Config) -> anyhow::Result<Vec<Self>> {
        crate::config::AGENTS
            .iter()
            .map(|(slug, name, persona)| {
                let host = config.agent_host(slug);
                let origin = format!("{}://{host}", config.scheme);
                let keys = CompanyKeys::load_or_create(
                    &origin,
                    &[SHOPPER],
                    &config.data_dir.join("keys").join(slug),
                )?;
                Ok(Self {
                    slug: (*slug).into(),
                    name: (*name).into(),
                    persona: (*persona).into(),
                    host,
                    origin,
                    keys,
                })
            })
            .collect()
    }

    /// "Amani (Test customer agent)".
    pub fn display(&self) -> String {
        format!("{} ({LABEL})", self.name)
    }

    /// `did:web:test-agent-amani.paypal.agentrix.website`.
    pub fn did(&self) -> String {
        saas_badge::did::site_did(&self.host)
    }

    /// The `User-Agent` it sends: labelled, with where to read about it.
    pub fn user_agent(&self) -> String {
        format!(
            "AgentrixTestCustomerAgent/{} ({LABEL} {}; +{}/)",
            env!("CARGO_PKG_VERSION"),
            self.name,
            self.origin
        )
    }

    /// Signs as this agent (Web Bot Auth, RFC 9421): `Signature-Agent` names its own key
    /// directory.
    pub fn signer(&self) -> anyhow::Result<Arc<HttpSignatureSigner>> {
        self.signer_with(self.keys.agent(SHOPPER)?)
    }

    /// Signs with `key` while naming this agent's directory: the wrong-key impostor.
    pub fn signer_with(
        &self,
        key: Arc<Ed25519SigningKey>,
    ) -> anyhow::Result<Arc<HttpSignatureSigner>> {
        Ok(Arc::new(
            HttpSignatureSigner::builder(key, self.origin.clone())
                .components(vec![
                    ComponentId::new("@authority"),
                    ComponentId::new("@method"),
                    ComponentId::new("@path"),
                    ComponentId::new("@query"),
                    ComponentId::new(CONTENT_DIGEST),
                ])
                .build()?,
        ))
    }

    /// Its DID document: the key it signs with, and where its key directory is.
    pub fn did_document(&self) -> anyhow::Result<Value> {
        let did = self.did();
        let method = format!("{did}#{SHOPPER}");
        let key = self.keys.agent(SHOPPER)?;
        Ok(json!({
            "@context": saas_badge::did::CONTEXTS,
            "id": did,
            "alsoKnownAs": [format!("urn:agentrix:test-customer-agent:{}", self.slug)],
            "verificationMethod": [saas_badge::did::verification_method(&method, &did, key.public_key())],
            "authentication": [method],
            "assertionMethod": [method],
            "service": [
                {"id": format!("{did}#keys"), "type": "HttpMessageSignaturesDirectory",
                 "serviceEndpoint": format!("{}/.well-known/http-message-signatures-directory", self.origin)},
                {"id": format!("{did}#about"), "type": "LinkedDomains", "serviceEndpoint": format!("{}/", self.origin)},
            ],
            "label": LABEL,
            "description": format!("{}: a scripted test shopper Agentrix runs against its sample sites, paying with sandbox PayPal only. Not a person and not a real customer.", self.display()),
        }))
    }
}
