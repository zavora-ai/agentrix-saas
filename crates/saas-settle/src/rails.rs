//! How the money moves on PayPal, once per cart mandate.
//!
//! - **Vault** (primary): the payer's owner saves its PayPal wallet once, approving a setup
//!   token PayPal issued to the payee's app (`POST /v3/vault/setup-tokens`, then
//!   `/v3/vault/payment-tokens`). Each settlement is then a merchant-initiated Orders v2
//!   order on that `vault_id` (`POST /v2/checkout/orders`, intent `CAPTURE`), made by the
//!   payee only with the payer agent's signed cart mandate. agentrix-effects has no Vault call
//!   yet, so this goes through its PayPal client ([`PayPalClient::request`]).
//! - **Payout** (fallback): the payer's own PayPal app sends a payout to the payee's PayPal
//!   email, its note carrying the invoice id, through [`Effects::payout`].
//!
//! Either way the PayPal idempotency key is the cart mandate's digest: PayPal runs a repeated
//! request once and answers with the first result, so a cart mandate never pays twice even if
//! this node loses its answer.

use std::sync::Arc;
use std::time::Duration;

use agentrix_effects::paypal::{self, PayPalClient, PayPalError};
use agentrix_effects::{EffectError, Effects, Outcome, PAYOUT_TOOL};
use agentrix_types::{EffectRequest, Money, SiteId};
use reqwest::Method;
use serde_json::{Value, json};

use crate::mandate::CartMandate;
use crate::protocol::{PayPalSettlement, PayoutEvidence};

/// Why the money didn't (or may not have) moved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RailError {
    /// No PayPal keys for this rail.
    #[error("PayPal isn't switched on for this company. Nothing was paid.")]
    NotConfigured,
    /// The payer has no saved wallet with the payee.
    #[error("{0} hasn't saved its PayPal with us yet. Nothing was paid.")]
    NoWallet(String),
    /// PayPal refused; nothing moved.
    #[error("PayPal didn't take the payment: {0}. Nothing was paid.")]
    NotExecuted(String),
    /// We can't tell yet; ask again with the same cart mandate (never a new one).
    #[error(
        "PayPal hasn't confirmed the payment yet ({0}). It will never be paid twice; ask again in a minute."
    )]
    Unknown(String),
}

impl From<PayPalError> for RailError {
    fn from(e: PayPalError) -> Self {
        if e.outcome_unknown() {
            Self::Unknown(e.to_string())
        } else {
            Self::NotExecuted(e.to_string())
        }
    }
}

/// The hex of a `sha256:…` digest: the PayPal request id.
pub fn hex_of(digest: &str) -> &str {
    digest.strip_prefix("sha256:").unwrap_or(digest)
}

/// What the payer approves to save its wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSetup {
    /// PayPal's setup token.
    pub setup_token: String,
    /// Where the payer's owner approves, signed in to PayPal.
    pub approve_url: Option<String>,
    /// PayPal's status: `PAYER_ACTION_REQUIRED`, then `APPROVED`.
    pub status: String,
    /// PayPal's customer id for the payer.
    pub customer_id: Option<String>,
}

/// A saved wallet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultedWallet {
    /// PayPal's payment token id: the `vault_id` orders are made on.
    pub vault_id: String,
    /// PayPal's customer id.
    pub customer_id: Option<String>,
    /// PayPal's payment-token response, as received: where a binding's ids come from.
    pub response: Value,
}

/// The payee's Vault rail, over its own PayPal app.
#[derive(Clone)]
pub struct VaultRail {
    client: Arc<PayPalClient>,
}

impl std::fmt::Debug for VaultRail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultRail")
            .field("environment", self.client.environment())
            .finish()
    }
}

impl VaultRail {
    /// A rail over `client` (the payee's app, with `vault/payment-tokens` readwrite).
    pub fn new(client: Arc<PayPalClient>) -> Self {
        Self { client }
    }

    /// Whether this is PayPal's sandbox.
    pub fn sandbox(&self) -> bool {
        !self.client.environment().is_live()
    }

    /// Asks PayPal for a setup token the payer's owner approves once, signed in as the payer.
    /// `request_id` makes it idempotent.
    pub async fn start_setup(
        &self,
        brand: &str,
        description: &str,
        return_url: &str,
        cancel_url: &str,
        request_id: &str,
    ) -> Result<VaultSetup, RailError> {
        let body = json!({"payment_source": {"paypal": {
            "description": description,
            "usage_pattern": "IMMEDIATE",
            "usage_type": "MERCHANT",
            "customer_type": "CONSUMER",
            "experience_context": {
                "brand_name": brand,
                "shipping_preference": "NO_SHIPPING",
                "vault_instruction": "ON_PAYER_APPROVAL",
                "return_url": return_url,
                "cancel_url": cancel_url,
            },
        }}});
        let answer = self
            .client
            .request(
                Method::POST,
                "/v3/vault/setup-tokens",
                Some(&body),
                Some(request_id),
            )
            .await?;
        Ok(setup_of(&answer))
    }

    /// Reads a setup token back.
    pub async fn setup(&self, setup_token: &str) -> Result<VaultSetup, RailError> {
        let answer = self
            .client
            .request(
                Method::GET,
                &format!("/v3/vault/setup-tokens/{setup_token}"),
                None,
                None,
            )
            .await?;
        Ok(setup_of(&answer))
    }

    /// Turns an approved setup token into a saved wallet.
    pub async fn complete_setup(&self, setup_token: &str) -> Result<VaultedWallet, RailError> {
        let body = json!({"payment_source": {"token": {"id": setup_token, "type": "SETUP_TOKEN"}}});
        let answer = self
            .client
            .request(
                Method::POST,
                "/v3/vault/payment-tokens",
                Some(&body),
                Some(&format!("agentrix-saas-vault-{setup_token}")),
            )
            .await?;
        let vault_id = answer["id"]
            .as_str()
            .ok_or_else(|| RailError::NotExecuted("PayPal returned no payment token".into()))?
            .to_owned();
        Ok(VaultedWallet {
            vault_id,
            customer_id: answer["customer"]["id"].as_str().map(str::to_owned),
            response: answer,
        })
    }

    /// Reads a saved payment method back from PayPal (`GET /v3/vault/payment-tokens/{id}`).
    pub async fn payment_token(&self, vault_id: &str) -> Result<VaultedWallet, RailError> {
        let answer = self
            .client
            .request(
                Method::GET,
                &format!("/v3/vault/payment-tokens/{vault_id}"),
                None,
                None,
            )
            .await?;
        let id = answer["id"]
            .as_str()
            .ok_or_else(|| RailError::NotExecuted("PayPal returned no payment token".into()))?
            .to_owned();
        Ok(VaultedWallet {
            vault_id: id,
            customer_id: answer["customer"]["id"].as_str().map(str::to_owned),
            response: answer,
        })
    }

    /// Captures `cart.amount` from the payer's saved wallet, merchant-initiated, once per cart
    /// mandate (`cart_digest` is the PayPal request id). `paypal_invoice_id` must be unique for
    /// the payee's PayPal account: PayPal refuses a second completed payment with the same one.
    pub async fn capture(
        &self,
        vault_id: &str,
        cart: &CartMandate,
        cart_digest: &str,
        paypal_invoice_id: &str,
        description: &str,
    ) -> Result<PayPalSettlement, RailError> {
        let request_id = hex_of(cart_digest);
        let body = json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "reference_id": cart.cart_mandate_id,
                "invoice_id": paypal_invoice_id,
                "custom_id": cart.cart_mandate_id,
                "description": description.chars().take(127).collect::<String>(),
                "amount": paypal::amount(&cart.amount.currency, cart.amount.minor),
            }],
            "payment_source": {"paypal": {"vault_id": vault_id}},
        });
        let order = self
            .client
            .request(
                Method::POST,
                "/v2/checkout/orders",
                Some(&body),
                Some(request_id),
            )
            .await?;
        let order = match order["status"].as_str() {
            Some("COMPLETED") => order,
            Some("APPROVED" | "CREATED" | "SAVED") => {
                let id = order["id"].as_str().unwrap_or_default();
                self.client
                    .request(
                        Method::POST,
                        &format!("/v2/checkout/orders/{id}/capture"),
                        None,
                        Some(&format!("{request_id}-capture")),
                    )
                    .await?
            }
            Some("PAYER_ACTION_REQUIRED") => {
                return Err(RailError::NotExecuted(
                    "PayPal wants the payer to approve this one on PayPal's page".into(),
                ));
            }
            other => {
                return Err(RailError::NotExecuted(format!(
                    "the order is {}",
                    other.unwrap_or("in an unknown state")
                )));
            }
        };
        settlement_of(&order, &cart.amount, self.sandbox())
    }

    /// Reads an order back (after an unknown outcome).
    pub async fn order(
        &self,
        order_id: &str,
        amount: &Money,
    ) -> Result<PayPalSettlement, RailError> {
        let order = self
            .client
            .request(
                Method::GET,
                &format!("/v2/checkout/orders/{order_id}"),
                None,
                None,
            )
            .await?;
        settlement_of(&order, amount, self.sandbox())
    }
}

fn setup_of(answer: &Value) -> VaultSetup {
    VaultSetup {
        setup_token: answer["id"].as_str().unwrap_or_default().to_owned(),
        approve_url: paypal::link(answer, &["approve"]),
        status: answer["status"].as_str().unwrap_or("UNKNOWN").to_owned(),
        customer_id: answer["customer"]["id"].as_str().map(str::to_owned),
    }
}

/// The capture inside a completed order, checked against the amount the cart mandate allows.
fn settlement_of(
    order: &Value,
    amount: &Money,
    sandbox: bool,
) -> Result<PayPalSettlement, RailError> {
    let capture = &order["purchase_units"][0]["payments"]["captures"][0];
    let Some(capture_id) = capture["id"].as_str() else {
        return Err(RailError::NotExecuted(format!(
            "the order {} has no capture",
            order["id"].as_str().unwrap_or("?")
        )));
    };
    let status = capture["status"].as_str().unwrap_or("UNKNOWN").to_owned();
    match status.as_str() {
        "COMPLETED" => {}
        "PENDING" => {
            return Err(RailError::Unknown(format!(
                "capture {capture_id} is pending at PayPal"
            )));
        }
        other => {
            return Err(RailError::NotExecuted(format!(
                "capture {capture_id} is {other}"
            )));
        }
    }
    let captured = paypal::minor_of(&capture["amount"]);
    if captured.as_ref().map(|(c, m)| (c.as_str(), *m))
        != Some((amount.currency.as_str(), amount.minor))
    {
        return Err(RailError::NotExecuted(format!(
            "capture {capture_id} is for {:?}, not the mandated amount",
            captured
        )));
    }
    let fee = paypal::minor_of(&capture["seller_receivable_breakdown"]["paypal_fee"])
        .map(|(currency, minor)| Money { currency, minor });
    Ok(PayPalSettlement {
        kind: "capture".into(),
        id: capture_id.to_owned(),
        parent_id: order["id"].as_str().map(str::to_owned),
        transaction_id: Some(capture_id.to_owned()),
        status,
        fee,
        sandbox,
    })
}

/// The payer's Payout rail, over its own PayPal app ([`Effects`] with the payer's keys).
#[derive(Clone, Debug)]
pub struct PayoutRail {
    effects: Effects,
    site: SiteId,
    wait: Duration,
}

impl PayoutRail {
    /// A rail paying from `site`'s PayPal through `effects`, waiting up to `wait` for the
    /// payout item to succeed.
    pub fn new(effects: Effects, site: SiteId, wait: Duration) -> Self {
        Self {
            effects,
            site,
            wait,
        }
    }

    /// The payout effect for a cart mandate: the same mandate always gives the same effect,
    /// so its fingerprint (PayPal's `sender_batch_id`) pays once.
    pub fn effect(&self, cart: &CartMandate, receiver: &str) -> EffectRequest {
        EffectRequest {
            site: self.site.clone(),
            webagent: crate::keys::PAYABLES.into(),
            tool: PAYOUT_TOOL.into(),
            args: json!({
                "receiver": receiver,
                "note": format!(
                    "Agentrix SaaS settlement of invoice {} (cart mandate {})",
                    cart.invoice_id, cart.cart_mandate_id
                ),
                "nonce": cart.cart_mandate_id,
            }),
            amount: Some(cart.amount.clone()),
            expiry: cart.expires_at,
        }
    }

    /// Pays the cart mandate to `receiver` (the payee's PayPal email), once, and waits for the
    /// item to succeed.
    pub async fn pay(
        &self,
        cart: &CartMandate,
        receiver: &str,
    ) -> Result<PayoutEvidence, RailError> {
        let effect = self.effect(cart, receiver);
        let fp = effect
            .fingerprint()
            .map_err(|e| RailError::NotExecuted(e.to_string()))?
            .to_string();
        let approval = json!({
            "fingerprint": hex_of(&fp),
            "account": format!("cart-mandate:{}", cart.cart_mandate_id),
        });
        let mut outcome = self
            .effects
            .payout(&effect, &approval)
            .await
            .map_err(effect_error)?;
        let deadline = tokio::time::Instant::now() + self.wait;
        loop {
            match (outcome.outcome, outcome.item_status.as_deref()) {
                (Outcome::NotExecuted, _) => return Err(RailError::NotExecuted(outcome.message)),
                (Outcome::Executed, Some("SUCCESS")) => break,
                (Outcome::Executed, Some("UNCLAIMED")) => {
                    return Err(RailError::NotExecuted(format!(
                        "{receiver} has no PayPal account to receive it (the payout is unclaimed)"
                    )));
                }
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RailError::Unknown(outcome.message));
            }
            tokio::time::sleep(Duration::from_secs(3).min(self.wait)).await;
            outcome = self
                .effects
                .reconcile_payout(&self.site, &outcome.payout_id)
                .await
                .map_err(effect_error)?;
        }
        let item_id = outcome
            .item_id
            .clone()
            .ok_or_else(|| RailError::Unknown("PayPal hasn't named the payout item".into()))?;
        let item = self
            .effects
            .payout_item(&item_id)
            .await
            .map_err(effect_error)?;
        Ok(PayoutEvidence {
            batch_id: outcome.batch_id.clone().unwrap_or_default(),
            item_id,
            transaction_id: item.transaction_id,
            status: "SUCCESS".into(),
            amount: outcome.amount,
            receiver: receiver.to_owned(),
            fee: None,
        })
    }
}

fn effect_error(e: EffectError) -> RailError {
    match e {
        EffectError::NotConfigured => RailError::NotConfigured,
        EffectError::Unknown(m) => RailError::Unknown(m),
        other => RailError::NotExecuted(other.to_string()),
    }
}
