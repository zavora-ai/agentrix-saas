//! Subscribing to a plan with PayPal, on the money host.
//!
//! 1. `GET /subscribe?plan=business&period=month`: the plan, and "Continue to PayPal".
//! 2. `POST /subscribe`: [`Checkout::start`] makes the PayPal subscription; the visitor goes to
//!    PayPal's approval page (`approve_url`).
//! 3. PayPal sends them back to `GET /subscribe/return?subscription_id=I-…`. The page asks
//!    `GET /subscribe/status?subscription_id=I-…` every two seconds until the plan is active
//!    *and* the receipt is booked: the signed PayPal webhook, verified on the agent door, posts
//!    it to the books.
//! 4. `GET /subscribe/cancelled`: they cancelled on PayPal; nothing was charged.
//!
//! **For `saas-node`.** Implement [`Checkout`] with agentrix-effects: `start` is
//! `Effects::start_subscription` (the plan's `PlanPrice` from agentrix-billing's catalogue,
//! `return_url` and `cancel_url` from [`Start`]); `status` is `Effects::refresh_subscription` by
//! PayPal's id, plus the books' receipt for it once the webhook has posted one. Until then
//! [`NotConnected`] says so on the page and nothing is charged.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Monthly, or 12 months up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// Charged every month.
    Month,
    /// Charged once a year, for the 12-month total.
    Year,
}

impl Period {
    /// `month` or `year`.
    pub fn as_str(self) -> &'static str {
        match self {
            Period::Month => "month",
            Period::Year => "year",
        }
    }

    /// From the query or form value; anything else is monthly.
    pub fn parse(v: Option<&str>) -> Self {
        if v == Some("year") {
            Period::Year
        } else {
            Period::Month
        }
    }
}

/// A visitor starting a plan.
#[derive(Debug, Clone, Serialize)]
pub struct Start {
    /// The plan's id in agentrix-billing's catalogue (`essential`, `business`, `commerce`).
    pub plan: String,
    /// How it is paid.
    pub period: Period,
    /// The visitor's email, to prefill PayPal's page.
    pub email: Option<String>,
    /// Where PayPal sends them after approving.
    pub return_url: String,
    /// Where PayPal sends them if they cancel.
    pub cancel_url: String,
}

/// A subscription waiting for the visitor on PayPal's page.
#[derive(Debug, Clone, Serialize)]
pub struct Started {
    /// PayPal's subscription id, `I-…`.
    pub subscription_id: String,
    /// PayPal's approval page.
    pub approve_url: String,
}

/// The receipt the books hold for the first charge.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Receipt {
    /// The PayPal id the receipt was posted from.
    pub paypal_id: String,
    /// The books' invoice it paid, when there is one.
    pub invoice_number: Option<String>,
    /// ISO 4217.
    pub currency: String,
    /// Amount.
    pub amount_minor: i64,
}

/// Where a subscription has got to.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Status {
    /// PayPal's id, `I-…`.
    pub subscription_id: String,
    /// The plan's name.
    pub plan: String,
    /// As PayPal reports it: `pending_approval`, `active`, `suspended`, `cancelled`, `expired`,
    /// `past_due`.
    pub state: String,
    /// The booked receipt, once the signed webhook has posted it.
    pub receipt: Option<Receipt>,
}

/// Why checkout didn't go ahead. The words are safe to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckoutError {
    /// No PayPal on this node.
    #[error("PayPal checkout isn't connected on this node yet. Nothing was charged.")]
    NotConnected,
    /// No such plan, or no such subscription.
    #[error("Nothing here by that name.")]
    NotFound,
    /// PayPal or the books said no.
    #[error("{0}")]
    Refused(String),
}

/// PayPal Subscriptions behind the subscribe pages.
#[async_trait]
pub trait Checkout: Send + Sync {
    /// Makes the subscription and returns PayPal's approval page.
    async fn start(&self, start: Start) -> Result<Started, CheckoutError>;
    /// Reads it back (PayPal's state, and the books' receipt once the webhook has posted it).
    async fn status(&self, subscription_id: &str) -> Result<Status, CheckoutError>;
    /// False when there is no PayPal behind it: the page says so instead of offering a button.
    fn connected(&self) -> bool {
        true
    }
}

/// The default: no PayPal here.
pub struct NotConnected;

#[async_trait]
impl Checkout for NotConnected {
    async fn start(&self, _start: Start) -> Result<Started, CheckoutError> {
        Err(CheckoutError::NotConnected)
    }

    async fn status(&self, _subscription_id: &str) -> Result<Status, CheckoutError> {
        Err(CheckoutError::NotConnected)
    }

    fn connected(&self) -> bool {
        false
    }
}

/// True once the plan is live: PayPal says active and the books hold the receipt.
pub fn is_done(s: &Status) -> bool {
    s.state == "active" && s.receipt.is_some()
}

/// The page's words for a status.
pub fn words(s: &Status) -> String {
    match (s.state.as_str(), &s.receipt) {
        ("active", Some(r)) => format!(
            "Active. The first charge is booked from PayPal {}{}.",
            r.paypal_id,
            r.invoice_number.as_ref().map(|n| format!(" against {n}")).unwrap_or_default()
        ),
        ("active", None) => "PayPal says your plan is active. Waiting for PayPal's signed notice to book the receipt…".into(),
        ("pending_approval", _) => "Waiting for PayPal to confirm your approval…".into(),
        ("cancelled", _) => "This subscription was cancelled. Nothing more will be charged.".into(),
        ("suspended", _) => "This subscription is paused. Nothing is charged while it is.".into(),
        ("past_due", _) => "PayPal couldn't take the last payment. Support will be in touch.".into(),
        (other, _) => format!("PayPal says: {other}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str, receipt: bool) -> Status {
        Status {
            subscription_id: "I-TEST".into(),
            plan: "Business".into(),
            state: state.into(),
            receipt: receipt.then(|| Receipt {
                paypal_id: "8AB12345CD6789012".into(),
                invoice_number: Some("INV-0042".into()),
                currency: "USD".into(),
                amount_minor: 7_900,
            }),
        }
    }

    #[test]
    fn done_only_once_the_receipt_is_booked() {
        assert!(!is_done(&status("pending_approval", false)));
        assert!(!is_done(&status("active", false)));
        assert!(is_done(&status("active", true)));
        assert!(
            words(&status("active", true))
                .contains("booked from PayPal 8AB12345CD6789012 against INV-0042")
        );
        assert!(words(&status("active", false)).contains("Waiting for PayPal's signed notice"));
    }

    #[test]
    fn periods_read_from_the_form() {
        assert_eq!(Period::parse(Some("year")), Period::Year);
        assert_eq!(Period::parse(Some("weekly")), Period::Month);
        assert_eq!(Period::parse(None).as_str(), "month");
    }
}
