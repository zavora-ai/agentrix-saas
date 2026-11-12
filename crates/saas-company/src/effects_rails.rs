//! The PayPal rails over `agentrix-effects` (feature `effects`): the node's money, sandbox only.
//!
//! - **Payouts** go through `Effects::payout`, which pays each effect's fingerprint once and
//!   settles an unknown outcome by reading PayPal back. The effect is built from the governed
//!   call's idempotency key (its nonce is the key's digest, its expiry fixed), so a retried or
//!   resumed call is the same effect, the same `sender_batch_id`, and never a second payout. Its
//!   approval proof is the governed path's: the call was authorized by the policy, the mandate
//!   or the founder before it reached here.
//! - **Subscriptions** start through `Effects::start_subscription`, one effects site per
//!   customer so each customer's subscription is its own; cancel and suspend, capture reads and
//!   refunds use PayPal's REST API directly, with the call's key as `PayPal-Request-Id`.

use std::sync::Arc;

use agentrix_effects::paypal::PayPalClient;
use agentrix_effects::{Effects, Outcome, PayoutRequest, PlanPrice, SubscriptionStart};
use agentrix_types::{Money, SiteId};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::desk::PlanOffer;
use crate::rails::{CaptureRecord, PayoutDone, Rails, RefundDone, SubscriptionLink};

/// The digest of a governed call's key, as PayPal request ids and effect nonces carry it.
pub fn key_digest(key: &str) -> String {
    hex::encode(&Sha256::digest(key.as_bytes())[..16])
}

fn cents(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    let (whole, frac) = text.split_once('.').unwrap_or((text, "0"));
    let frac = format!("{frac:0<2}");
    Some(whole.parse::<i64>().ok()? * 100 + frac[..2].parse::<i64>().ok()?)
}

fn decimal(minor: i64) -> String {
    format!("{}.{:02}", minor / 100, minor % 100)
}

/// PayPal through `agentrix-effects`.
pub struct EffectsRails {
    effects: Effects,
    client: Arc<PayPalClient>,
    site: SiteId,
    agent: String,
    catalogue: Vec<PlanOffer>,
    return_url: String,
}

impl std::fmt::Debug for EffectsRails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectsRails")
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

impl EffectsRails {
    /// Over `effects` and the same PayPal `client`, for the company's effects `site`.
    pub fn new(
        effects: Effects,
        client: Arc<PayPalClient>,
        site: SiteId,
        catalogue: Vec<PlanOffer>,
        return_url: &str,
    ) -> Self {
        Self {
            effects,
            client,
            site,
            agent: "payables".into(),
            catalogue,
            return_url: return_url.to_owned(),
        }
    }

    fn customer_site(&self, email: &str) -> SiteId {
        SiteId::new(format!(
            "{}-c{}",
            self.site.as_str(),
            &key_digest(email)[..12]
        ))
    }
}

/// When a governed payout effect lapses: never on its own. The governed path, not the effect,
/// decides when it may run; a fixed expiry keeps the effect, and so its fingerprint, the same on
/// every retry.
fn governed_expiry() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 12, 31, 0, 0, 0)
        .single()
        .expect("a valid date")
}

#[async_trait]
impl Rails for EffectsRails {
    fn label(&self) -> String {
        if self.effects.sandbox() {
            "PayPal sandbox".into()
        } else {
            "PayPal".into()
        }
    }

    async fn payout(
        &self,
        key: &str,
        receiver: &str,
        currency: &str,
        amount_minor: i64,
        note: &str,
    ) -> anyhow::Result<PayoutDone> {
        let mut effect = self.effects.payout_request(PayoutRequest {
            site: self.site.clone(),
            agent: self.agent.clone(),
            receiver: receiver.to_owned(),
            amount: Money {
                currency: currency.to_owned(),
                minor: amount_minor,
            },
            note: note.to_owned(),
        })?;
        effect.args["nonce"] = json!(key_digest(key));
        effect.expiry = governed_expiry();
        let fingerprint = effect.fingerprint()?.to_string();
        let approval = json!({
            "method": "governed",
            "account": format!("governed:{}", key_digest(key)),
            "fingerprint": fingerprint.trim_start_matches("sha256:"),
        });
        let out = self.effects.payout(&effect, &approval).await?;
        match out.outcome {
            Outcome::Executed => Ok(PayoutDone {
                paypal_id: out
                    .item_id
                    .clone()
                    .or(out.batch_id.clone())
                    .unwrap_or(out.payout_id.clone()),
                item_id: out.item_id.clone(),
                status: out
                    .item_status
                    .or(out.batch_status)
                    .unwrap_or_else(|| "PENDING".into()),
                receiver: out.receiver,
                amount_minor: out.amount.minor,
                currency: out.amount.currency,
            }),
            Outcome::NotExecuted => anyhow::bail!("PayPal didn't pay: {}", out.message),
            Outcome::Unknown => anyhow::bail!(
                "PayPal hasn't confirmed payout {} yet; it will be read back, never sent twice",
                out.payout_id
            ),
        }
    }

    async fn paid_out_today(&self, now: DateTime<Utc>) -> anyhow::Result<i64> {
        let today = now.date_naive();
        Ok(self
            .effects
            .payouts_for_site(&self.site)
            .await?
            .iter()
            .filter(|p| !matches!(p.state.as_str(), "not_executed" | "failed"))
            .filter(|p| {
                DateTime::from_timestamp(p.created_at, 0).is_some_and(|t| t.date_naive() == today)
            })
            .map(|p| p.amount_minor)
            .sum())
    }

    async fn capture(&self, capture_id: &str) -> anyhow::Result<Option<CaptureRecord>> {
        let path = format!("/v2/payments/captures/{capture_id}");
        let Ok(c) = self.client.request(Method::GET, &path, None, None).await else {
            return Ok(None);
        };
        let amount = cents(&c["amount"]["value"]).unwrap_or(0);
        let refunded = if c["status"] == "REFUNDED" { amount } else { 0 };
        Ok(Some(CaptureRecord {
            capture_id: capture_id.to_owned(),
            payer_email: None,
            currency: c["amount"]["currency_code"]
                .as_str()
                .unwrap_or("USD")
                .to_owned(),
            amount_minor: amount,
            refunded_minor: refunded,
            subscription: c["supplementary_data"]["related_ids"]["subscription_id"]
                .as_str()
                .map(str::to_owned),
        }))
    }

    async fn refund(
        &self,
        key: &str,
        capture_id: &str,
        amount_minor: i64,
        note: &str,
    ) -> anyhow::Result<RefundDone> {
        let capture = self
            .capture(capture_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("PayPal has no capture {capture_id}"))?;
        let body = json!({
            "amount": { "value": decimal(amount_minor), "currency_code": capture.currency },
            "note_to_payer": note.chars().take(255).collect::<String>(),
        });
        let r = self
            .client
            .request(
                Method::POST,
                &format!("/v2/payments/captures/{capture_id}/refund"),
                Some(&body),
                Some(&key_digest(key)),
            )
            .await?;
        Ok(RefundDone {
            refund_id: r["id"].as_str().unwrap_or_default().to_owned(),
            status: r["status"].as_str().unwrap_or_default().to_owned(),
            amount_minor,
        })
    }

    async fn start_subscription(
        &self,
        _key: &str,
        plan_id: &str,
        customer_email: &str,
    ) -> anyhow::Result<SubscriptionLink> {
        let offer = self
            .catalogue
            .iter()
            .find(|p| p.plan_id == plan_id)
            .ok_or_else(|| anyhow::anyhow!("no plan {plan_id}"))?;
        let usd = |minor| Money {
            currency: "USD".into(),
            minor,
        };
        let started = self
            .effects
            .start_subscription(SubscriptionStart {
                site: self.customer_site(customer_email),
                plan: PlanPrice {
                    plan_id: offer.plan_id.clone(),
                    version: 1,
                    name: offer.name.clone(),
                    monthly: usd(offer.monthly_minor),
                    annual: None,
                },
                subscriber_email: Some(customer_email.to_owned()),
                return_url: self.return_url.clone(),
                cancel_url: self.return_url.clone(),
                starts_at: None,
            })
            .await?;
        Ok(SubscriptionLink {
            provider_ref: started.provider_ref,
            approve_url: started.approve_url,
        })
    }

    async fn cancel_subscription(&self, provider_ref: &str, reason: &str) -> anyhow::Result<()> {
        self.client
            .request(
                Method::POST,
                &format!("/v1/billing/subscriptions/{provider_ref}/cancel"),
                Some(&json!({ "reason": reason.chars().take(127).collect::<String>() })),
                None,
            )
            .await?;
        Ok(())
    }

    async fn suspend_subscription(&self, provider_ref: &str, reason: &str) -> anyhow::Result<()> {
        self.client
            .request(
                Method::POST,
                &format!("/v1/billing/subscriptions/{provider_ref}/suspend"),
                Some(&json!({ "reason": reason.chars().take(127).collect::<String>() })),
                None,
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_convert_both_ways() {
        assert_eq!(cents(&json!("20.5")), Some(2_050));
        assert_eq!(cents(&json!("120.00")), Some(12_000));
        assert_eq!(decimal(2_050), "20.50");
        assert_eq!(key_digest("k").len(), 32);
    }
}
