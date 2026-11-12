//! Each test agent's saved sandbox buyer (PayPal Vault, with the sample sites' PayPal app):
//! approved once by a person signed in to a PayPal sandbox buyer account, then used for every
//! order with no one present (`confirm-payment-source` with the `vault_id`, then the site's own
//! return URL captures). Sandbox only.

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use reqwest::Method;
use saas_settle::rails::VaultRail;
use serde_json::json;

use crate::config::Config;
use crate::identity::Agent;

/// The sample sites' PayPal app, sandbox only.
#[derive(Clone)]
pub struct Buyers {
    client: Arc<PayPalClient>,
    vault: VaultRail,
    dir: PathBuf,
}

impl std::fmt::Debug for Buyers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Buyers").finish_non_exhaustive()
    }
}

impl Buyers {
    /// `None` unless a sandbox PayPal app is configured.
    pub fn new(config: &Config, http: reqwest::Client) -> Option<Self> {
        let (client_id, client_secret) = config.paypal.clone()?;
        let client = Arc::new(PayPalClient::new(
            PayPalConfig {
                client_id,
                client_secret,
                webhook_id: None,
                environment: Environment::Sandbox,
            },
            http,
        ));
        Some(Self {
            vault: VaultRail::new(client.clone()),
            client,
            dir: config.data_dir.join("buyers"),
        })
    }

    fn file(&self, agent: &Agent) -> PathBuf {
        self.dir.join(format!("{}.json", agent.slug))
    }

    /// The agent's saved buyer, if one was approved.
    pub fn vault_id(&self, agent: &Agent) -> Option<String> {
        let bytes = std::fs::read(self.file(agent)).ok()?;
        let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        v["vault_id"].as_str().map(str::to_owned)
    }

    /// Starts saving a buyer for `agent`: PayPal's approval page, for a person signed in to a
    /// sandbox buyer account.
    pub async fn start(&self, agent: &Agent) -> anyhow::Result<String> {
        let setup = self
            .vault
            .start_setup(
                &format!("{} (sandbox)", agent.display()),
                &format!(
                    "Agentrix SaaS {}: pays sample-site orders in sandbox",
                    agent.display()
                ),
                &format!("{}/paypal/returned", agent.origin),
                &format!("{}/paypal/cancelled", agent.origin),
                &format!(
                    "agentrix-saas-test-agent-{}-{}",
                    agent.slug,
                    uuid::Uuid::now_v7().simple()
                ),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        setup
            .approve_url
            .ok_or_else(|| anyhow::anyhow!("PayPal gave no approval link"))
    }

    /// Finishes saving after PayPal's approval: the buyer becomes the agent's.
    pub async fn finish(&self, agent: &Agent, setup_token: &str) -> anyhow::Result<String> {
        let saved = self
            .vault
            .complete_setup(setup_token)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(
            self.file(agent),
            serde_json::to_vec(
                &json!({"vault_id": saved.vault_id, "customer_id": saved.customer_id, "saved_at": chrono::Utc::now()}),
            )?,
        )?;
        Ok(saved.vault_id)
    }

    /// Pays a PayPal order the site created, as the agent's saved buyer: the order is then
    /// APPROVED for the site to capture.
    pub async fn confirm(&self, order_id: &str, vault_id: &str) -> anyhow::Result<String> {
        let answer = self
            .client
            .request(
                Method::POST,
                &format!("/v2/checkout/orders/{order_id}/confirm-payment-source"),
                Some(&json!({"payment_source": {"paypal": {"vault_id": vault_id}}})),
                Some(&format!("agentrix-saas-test-agent-confirm-{order_id}")),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(answer["status"].as_str().unwrap_or_default().to_owned())
    }
}
