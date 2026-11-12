//! The subscribe pages' checkout, on PayPal Subscriptions through agentrix-effects.
//!
//! Each visitor is their own effects site, so two customers on the same plan get two
//! subscriptions. The receipt is the books': once PayPal's signed webhook says the subscription
//! is active, Revenue sets up its recurring invoice, and each billing cycle's payment posts
//! against it ([`saas_company::runtime::Runtime::subscription_activated`]).

use std::sync::Arc;

use agentrix_effects::{Effects, PlanPrice, SubscriptionStart};
use agentrix_types::{Money, SiteId};
use async_trait::async_trait;
use saas_company::desk::PlanOffer;
use saas_company::effects_rails::key_digest;
use saas_company::runtime::{Runtime, subscription_reference};
use saas_core::books::{PaymentDirection, PaymentFilter};
use saas_web::checkout::{Checkout, CheckoutError, Period, Receipt, Start, Started, Status};

/// The effects site of a visitor who subscribes with `email` (or anonymously).
pub fn subscriber_site(email: Option<&str>) -> SiteId {
    let who = email
        .map(|e| e.trim().to_ascii_lowercase())
        .unwrap_or_default();
    SiteId::new(format!("saas-agentrix-c{}", &key_digest(&who)[..12]))
}

/// The plan id and period of an effects plan key (`business@1`, `business@1:annual`).
pub fn plan_of_key(key: &str) -> (String, bool) {
    let (plan, annual) = match key.strip_suffix(":annual") {
        Some(p) => (p, true),
        None => (key, false),
    };
    (plan.split('@').next().unwrap_or(plan).to_owned(), annual)
}

/// PayPal checkout over agentrix-effects.
pub struct EffectsCheckout {
    effects: Effects,
    catalogue: Vec<PlanOffer>,
    agentrix: Arc<Runtime>,
}

impl EffectsCheckout {
    /// Over Agentrix's `effects`, selling `catalogue`.
    pub fn new(effects: Effects, catalogue: Vec<PlanOffer>, agentrix: Arc<Runtime>) -> Self {
        Self {
            effects,
            catalogue,
            agentrix,
        }
    }
}

fn refused(e: impl std::fmt::Display) -> CheckoutError {
    CheckoutError::Refused(e.to_string())
}

#[async_trait]
impl Checkout for EffectsCheckout {
    async fn start(&self, start: Start) -> Result<Started, CheckoutError> {
        let offer = self
            .catalogue
            .iter()
            .find(|p| p.plan_id == start.plan)
            .ok_or(CheckoutError::NotFound)?;
        let usd = |minor| Money {
            currency: "USD".into(),
            minor,
        };
        let started = self
            .effects
            .start_subscription(SubscriptionStart {
                site: subscriber_site(start.email.as_deref()),
                plan: PlanPrice {
                    plan_id: offer.plan_id.clone(),
                    version: 1,
                    name: offer.name.clone(),
                    monthly: usd(offer.monthly_minor),
                    annual: matches!(start.period, Period::Year).then(|| usd(offer.annual_minor)),
                },
                subscriber_email: start.email.clone(),
                return_url: start.return_url,
                cancel_url: start.cancel_url,
                starts_at: None,
            })
            .await
            .map_err(refused)?;
        Ok(Started {
            subscription_id: started.provider_ref,
            approve_url: started.approve_url,
        })
    }

    async fn status(&self, subscription_id: &str) -> Result<Status, CheckoutError> {
        let sub = self
            .effects
            .refresh_subscription(subscription_id)
            .await
            .map_err(|_| CheckoutError::NotFound)?;
        let (plan, _) = plan_of_key(&sub.plan_key);
        let name = self
            .catalogue
            .iter()
            .find(|p| p.plan_id == plan)
            .map(|p| p.name.clone())
            .unwrap_or(plan);
        let books = &self.agentrix.company.desk.books;
        let reference = subscription_reference(subscription_id);
        let mut receipt = None;
        if let Some(recurring) = books
            .recurring_invoices()
            .await
            .map_err(refused)?
            .into_iter()
            .find(|r| r.reference.as_deref() == Some(reference.as_str()))
        {
            let payments = books
                .payments(&PaymentFilter {
                    reference: None,
                    direction: Some(PaymentDirection::Received),
                })
                .await
                .map_err(refused)?;
            receipt = payments
                .into_iter()
                .filter(|p| {
                    matches!(&p.party, saas_core::books::DocumentRef::Customer(c) if *c == recurring.customer)
                })
                .find(|p| {
                    p.reference
                        .as_deref()
                        .is_some_and(|r| r.starts_with("paypal:sale:"))
                })
                .map(|p| Receipt {
                    paypal_id: p
                        .reference
                        .clone()
                        .unwrap_or_default()
                        .trim_start_matches("paypal:sale:")
                        .to_owned(),
                    invoice_number: None,
                    currency: p.amount.currency.to_string(),
                    amount_minor: p.amount.minor,
                });
        }
        Ok(Status {
            subscription_id: subscription_id.to_owned(),
            plan: name,
            state: sub.state,
            receipt,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_keys_read_back() {
        assert_eq!(plan_of_key("business@1"), ("business".into(), false));
        assert_eq!(plan_of_key("agency@3:annual"), ("agency".into(), true));
        assert_ne!(
            subscriber_site(Some("a@x.example")),
            subscriber_site(Some("b@x.example"))
        );
    }
}
