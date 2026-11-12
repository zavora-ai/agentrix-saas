//! Real PayPal (sandbox) for the sample sites: agentrix-samples' `Payments` over the Effects
//! Gateway. The pay page sends the customer to PayPal's approval page; the return URL captures
//! once by PayPal's order id; each payment is a row in `customer_payments`, which
//! `saas_node::samples::follow` turns into wallet entries and ERA journals. Node registers it
//! once, with an `Effects` over the node's own pool:
//!
//! ```ignore
//! agentrix_samples::set_payments(Arc::new(saas_settle::SamplePayPal::new(
//!     agentrix_effects::Effects::new(ctx.pool.clone(), Some(agentrix_paypal_client), ctx.receipts.clone()),
//! )));
//! ```

use agentrix_effects::{CaptureState, Effects};
use agentrix_samples::ports::{Capture, PaymentRequest, PaymentStart, Payments};
use agentrix_types::tenancy::VisitorAccountId;
use agentrix_types::{Money, SiteId};
use async_trait::async_trait;

/// The sample sites' payments through the Effects Gateway.
#[derive(Clone)]
pub struct SamplePayPal {
    effects: Effects,
}

impl std::fmt::Debug for SamplePayPal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamplePayPal").finish_non_exhaustive()
    }
}

impl SamplePayPal {
    /// Payments through `effects` (with a PayPal client; without one, every payment says so).
    pub fn new(effects: Effects) -> Self {
        Self { effects }
    }
}

/// "Kilele Bakery (Sample)" from "Kilele Bakery order KB-1042 (Sample)".
fn site_name(description: &str) -> String {
    match description.split_once(" order ") {
        Some((name, rest)) if rest.ends_with("(Sample)") => format!("{name} (Sample)"),
        Some((name, _)) => name.to_owned(),
        None => description.to_owned(),
    }
}

#[async_trait]
impl Payments for SamplePayPal {
    async fn create(&self, req: PaymentRequest) -> anyhow::Result<PaymentStart> {
        let started = self
            .effects
            .create_payment(agentrix_effects::PaymentRequest {
                site: SiteId::new(req.site.clone()),
                site_name: site_name(&req.description),
                order_id: req.order_id.clone(),
                amount: Money {
                    currency: "USD".into(),
                    minor: req.amount_usd_cents,
                },
                description: req.description.clone(),
                customer: req.customer.clone().map(VisitorAccountId::new),
                return_url: req.return_url.clone(),
                cancel_url: req.cancel_url.clone(),
            })
            .await;
        Ok(match started {
            Ok(s) => PaymentStart::Redirect {
                payment_id: s.payment_id,
                provider_ref: s.provider_ref,
                approve_url: s.approve_url,
            },
            Err(e) => {
                tracing::warn!(site = %req.site, order = %req.order_id, error = %e, "samples: PayPal didn't start the payment");
                PaymentStart::NotExecuted {
                    words: format!("PayPal didn't start the payment: {e} Nothing was charged."),
                }
            }
        })
    }

    async fn capture(&self, site: &str, provider_ref: &str) -> anyhow::Result<Capture> {
        let out = self
            .effects
            .capture(&SiteId::new(site.to_owned()), provider_ref)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Capture {
            state: match out.state {
                CaptureState::Captured => "captured",
                CaptureState::Pending => "pending",
                CaptureState::Declined => "declined",
                CaptureState::Cancelled => "cancelled",
            }
            .into(),
            provider_capture_id: out.provider_capture_id,
            receipt: out.receipt,
        })
    }

    fn is_stand_in(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_site_name_comes_from_the_description() {
        assert_eq!(
            super::site_name("Kilele Bakery order KB-1042 (Sample)"),
            "Kilele Bakery (Sample)"
        );
        assert_eq!(
            super::site_name("Tana Valley Times subscription"),
            "Tana Valley Times subscription"
        );
    }
}
