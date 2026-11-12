//! The money rails the company's agents act through: PayPal, by way of `agentrix-effects`.
//!
//! [`Rails`] is what the money tools call once the governed path has authorized them. It is a
//! port so tests and judge-mode demo takes run on [`FakeRails`] (PayPal faked in memory), and
//! the node runs on [`crate::effects_rails::EffectsRails`] (PayPal sandbox through
//! `agentrix-effects`). Every call carries an idempotency key derived from the governed call,
//! so a retried call is the same PayPal request, never a second one.
//!
//! [`Settlement`] is the agent-to-agent side slice C (`saas-settle`) owns: settling an invoice
//! another company sent over A2A, and collecting a debt-plan instalment. The company crate only
//! decides *whether* (on the governed path); slice C decides *how* (Vault capture or payout).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use adk_core::Clock;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// A capture as PayPal records it: who paid, and how much is left to refund.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecord {
    /// PayPal's capture id.
    pub capture_id: String,
    /// The payer's PayPal email, from PayPal's own record. A refund can go nowhere else.
    pub payer_email: Option<String>,
    /// ISO 4217.
    pub currency: String,
    /// Captured, in minor units.
    pub amount_minor: i64,
    /// Refunded so far, in minor units.
    pub refunded_minor: i64,
    /// PayPal's subscription `I-…`, when the capture was a subscription charge.
    pub subscription: Option<String>,
}

impl CaptureRecord {
    /// What can still be refunded.
    pub fn refundable_minor(&self) -> i64 {
        (self.amount_minor - self.refunded_minor).max(0)
    }
}

/// A payout PayPal took (or a fake one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutDone {
    /// PayPal's id for the money movement: the payout item's when PayPal has said it, else the
    /// batch's.
    pub paypal_id: String,
    /// PayPal's `payout_item_id`, once PayPal has said it. The books post a payout by its item
    /// (saas-core's posting rules); a payout still PENDING posts when PayPal confirms the item.
    pub item_id: Option<String>,
    /// PayPal's status.
    pub status: String,
    /// The receiver's PayPal email.
    pub receiver: String,
    /// Paid, in minor units.
    pub amount_minor: i64,
    /// ISO 4217.
    pub currency: String,
}

/// A refund PayPal took.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundDone {
    /// PayPal's refund id.
    pub refund_id: String,
    /// PayPal's status.
    pub status: String,
    /// Refunded, in minor units.
    pub amount_minor: i64,
}

/// A subscription waiting for the customer on PayPal's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionLink {
    /// PayPal's `I-…`.
    pub provider_ref: String,
    /// PayPal's approval page.
    pub approve_url: String,
}

/// Earning, giving back and paying out, on PayPal.
#[async_trait]
pub trait Rails: Send + Sync {
    /// "PayPal sandbox", or "PayPal (faked)": shown wherever results are.
    fn label(&self) -> String;
    /// Pays `receiver` `amount_minor` once per `key` (PayPal Payouts). A second call with the
    /// same key returns the first payout.
    async fn payout(
        &self,
        key: &str,
        receiver: &str,
        currency: &str,
        amount_minor: i64,
        note: &str,
    ) -> anyhow::Result<PayoutDone>;
    /// What has been paid out today (UTC), in minor units.
    async fn paid_out_today(&self, now: DateTime<Utc>) -> anyhow::Result<i64>;
    /// PayPal's record of a capture, or `None` when PayPal has none.
    async fn capture(&self, capture_id: &str) -> anyhow::Result<Option<CaptureRecord>>;
    /// Refunds part or all of a capture, to the payer, once per `key`.
    async fn refund(
        &self,
        key: &str,
        capture_id: &str,
        amount_minor: i64,
        note: &str,
    ) -> anyhow::Result<RefundDone>;
    /// Starts a subscription the customer approves on PayPal's page.
    async fn start_subscription(
        &self,
        key: &str,
        plan_id: &str,
        customer_email: &str,
    ) -> anyhow::Result<SubscriptionLink>;
    /// Cancels a subscription.
    async fn cancel_subscription(&self, provider_ref: &str, reason: &str) -> anyhow::Result<()>;
    /// Suspends a subscription after a failed payment.
    async fn suspend_subscription(&self, provider_ref: &str, reason: &str) -> anyhow::Result<()>;
}

/// A settled agent-to-agent invoice or instalment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settled {
    /// The one PayPal id both companies' books post from.
    pub paypal_id: String,
    /// Which PayPal object it names: `capture` (Vault route) or `payout` (a payout item).
    pub paypal_kind: String,
    /// Paid, in minor units.
    pub amount_minor: i64,
    /// ISO 4217.
    pub currency: String,
    /// When.
    pub settled_on: NaiveDate,
}

/// Agent-to-agent settlement (slice C).
#[async_trait]
pub trait Settlement: Send + Sync {
    /// Pays an invoice another company sent, once per `key`.
    async fn settle_invoice(
        &self,
        key: &str,
        counterparty: &str,
        invoice_ref: &str,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<Settled>;
    /// Agrees an instalment plan for `invoice_ref` with another agent-run company, once per
    /// `plan_id`; returns the instalments agreed (due date, minor units). The default has no one
    /// to agree with and takes the terms as proposed.
    async fn agree_plan(
        &self,
        key: &str,
        counterparty: &str,
        plan_id: &str,
        invoice_ref: &str,
        currency: &str,
        instalments: &[(NaiveDate, i64)],
    ) -> anyhow::Result<Vec<(NaiveDate, i64)>> {
        let _ = (key, counterparty, plan_id, invoice_ref, currency);
        Ok(instalments.to_vec())
    }
    /// Collects one instalment of a debt plan from the customer, once per `key`.
    async fn collect_instalment(
        &self,
        key: &str,
        counterparty: &str,
        plan_id: &str,
        instalment: u32,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<Settled>;
    /// Offers invoice `invoice_ref` (its books id or number) to the customer company's agent
    /// door, which records it as a bill and wakes that company's Payables to settle it; returns
    /// the door's decision (`will_settle`, or why not). The default has no door to offer it to.
    async fn offer_invoice(&self, invoice_ref: &str) -> anyhow::Result<String> {
        anyhow::bail!("no agent door to offer {invoice_ref} to")
    }
}

#[derive(Debug, Default)]
struct FakeState {
    payouts: BTreeMap<String, (PayoutDone, DateTime<Utc>)>,
    captures: BTreeMap<String, CaptureRecord>,
    refunds: BTreeMap<String, RefundDone>,
    subscriptions: BTreeMap<String, (String, String)>,
    cancelled: Vec<String>,
    suspended: Vec<String>,
    settled: BTreeMap<String, Settled>,
    calls: Vec<String>,
}

/// PayPal faked in memory, for tests and judge-mode demo takes. It answers the way PayPal does
/// where it matters: a refund above what is left on a capture is refused, and a payout repeated
/// with the same key is the first payout, not a second.
#[derive(Default)]
pub struct FakeRails {
    state: Mutex<FakeState>,
    clock: Option<Arc<dyn Clock>>,
}

impl std::fmt::Debug for FakeRails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeRails").finish_non_exhaustive()
    }
}

impl FakeRails {
    /// No captures, nothing paid.
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Dates payouts by `clock` (a manual clock in tests) instead of the wall clock.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    fn now(&self) -> DateTime<Utc> {
        self.clock.as_ref().map_or_else(Utc::now, |c| c.now())
    }

    /// Adds a capture PayPal would know about.
    pub fn with_capture(self, capture: CaptureRecord) -> Self {
        self.state()
            .captures
            .insert(capture.capture_id.clone(), capture);
        self
    }

    /// Every payout made, oldest key first.
    pub fn payouts(&self) -> Vec<PayoutDone> {
        self.state()
            .payouts
            .values()
            .map(|(p, _)| p.clone())
            .collect()
    }

    /// Every settlement made.
    pub fn settlements(&self) -> Vec<Settled> {
        self.state().settled.values().cloned().collect()
    }

    /// Every money call that reached PayPal, in order (`payout:<key>`, `refund:<key>`, ...).
    /// A repeated key reaches PayPal again, as it would, and PayPal answers with the first.
    pub fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }
}

#[async_trait]
impl Rails for FakeRails {
    fn label(&self) -> String {
        "PayPal (faked)".into()
    }

    async fn payout(
        &self,
        key: &str,
        receiver: &str,
        currency: &str,
        amount_minor: i64,
        _note: &str,
    ) -> anyhow::Result<PayoutDone> {
        let mut s = self.state();
        s.calls.push(format!("payout:{key}"));
        if let Some((done, _)) = s.payouts.get(key) {
            return Ok(done.clone());
        }
        anyhow::ensure!(amount_minor > 0, "a payout must be more than zero");
        let n = s.payouts.len() + 1;
        let done = PayoutDone {
            paypal_id: format!("FAKEBATCH{n:04}"),
            item_id: Some(format!("FAKEBATCH{n:04}")),
            status: "SUCCESS".into(),
            receiver: receiver.to_owned(),
            amount_minor,
            currency: currency.to_owned(),
        };
        let at = self.now();
        s.payouts.insert(key.to_owned(), (done.clone(), at));
        Ok(done)
    }

    async fn paid_out_today(&self, now: DateTime<Utc>) -> anyhow::Result<i64> {
        let today = now.date_naive();
        Ok(self
            .state()
            .payouts
            .values()
            .filter(|(_, at)| at.date_naive() == today)
            .map(|(p, _)| p.amount_minor)
            .sum())
    }

    async fn capture(&self, capture_id: &str) -> anyhow::Result<Option<CaptureRecord>> {
        Ok(self.state().captures.get(capture_id).cloned())
    }

    async fn refund(
        &self,
        key: &str,
        capture_id: &str,
        amount_minor: i64,
        _note: &str,
    ) -> anyhow::Result<RefundDone> {
        let mut s = self.state();
        s.calls.push(format!("refund:{key}"));
        if let Some(done) = s.refunds.get(key) {
            return Ok(done.clone());
        }
        let capture = s
            .captures
            .get_mut(capture_id)
            .ok_or_else(|| anyhow::anyhow!("PayPal has no capture {capture_id}"))?;
        anyhow::ensure!(
            amount_minor > 0 && amount_minor <= capture.refundable_minor(),
            "PayPal refused: the refund is more than is left on the capture"
        );
        capture.refunded_minor += amount_minor;
        let n = s.refunds.len() + 1;
        let done = RefundDone {
            refund_id: format!("FAKEREFUND{n:04}"),
            status: "COMPLETED".into(),
            amount_minor,
        };
        s.refunds.insert(key.to_owned(), done.clone());
        Ok(done)
    }

    async fn start_subscription(
        &self,
        key: &str,
        plan_id: &str,
        customer_email: &str,
    ) -> anyhow::Result<SubscriptionLink> {
        let mut s = self.state();
        s.calls.push(format!("subscribe:{key}"));
        let n = s.subscriptions.len() + 1;
        let provider_ref = s
            .subscriptions
            .get(key)
            .map(|(r, _)| r.clone())
            .unwrap_or_else(|| format!("I-FAKE{n:04}"));
        s.subscriptions.insert(
            key.to_owned(),
            (provider_ref.clone(), format!("{plan_id}:{customer_email}")),
        );
        Ok(SubscriptionLink {
            approve_url: format!(
                "https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token={provider_ref}"
            ),
            provider_ref,
        })
    }

    async fn cancel_subscription(&self, provider_ref: &str, _reason: &str) -> anyhow::Result<()> {
        let mut s = self.state();
        s.calls.push(format!("cancel:{provider_ref}"));
        s.cancelled.push(provider_ref.to_owned());
        Ok(())
    }

    async fn suspend_subscription(&self, provider_ref: &str, _reason: &str) -> anyhow::Result<()> {
        let mut s = self.state();
        s.calls.push(format!("suspend:{provider_ref}"));
        s.suspended.push(provider_ref.to_owned());
        Ok(())
    }
}

#[async_trait]
impl Settlement for FakeRails {
    async fn settle_invoice(
        &self,
        key: &str,
        _counterparty: &str,
        _invoice_ref: &str,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<Settled> {
        let now = self.now();
        let mut s = self.state();
        s.calls.push(format!("settle:{key}"));
        let n = s.settled.len() + 1;
        let settled = s
            .settled
            .entry(key.to_owned())
            .or_insert_with(|| Settled {
                paypal_id: format!("FAKECAPTURE{n:04}"),
                paypal_kind: "capture".into(),
                amount_minor,
                currency: currency.to_owned(),
                settled_on: now.date_naive(),
            })
            .clone();
        Ok(settled)
    }

    async fn collect_instalment(
        &self,
        key: &str,
        counterparty: &str,
        plan_id: &str,
        _instalment: u32,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<Settled> {
        self.settle_invoice(key, counterparty, plan_id, currency, amount_minor)
            .await
    }

    async fn offer_invoice(&self, invoice_ref: &str) -> anyhow::Result<String> {
        self.state().calls.push(format!("offer:{invoice_ref}"));
        Ok("will_settle".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_repeated_payout_key_is_the_first_payout() {
        let rails = FakeRails::new();
        let a = rails
            .payout("k1", "v@example.com", "USD", 1_000, "bill")
            .await
            .unwrap();
        let b = rails
            .payout("k1", "v@example.com", "USD", 1_000, "bill")
            .await
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(rails.payouts().len(), 1);
        assert_eq!(rails.paid_out_today(Utc::now()).await.unwrap(), 1_000);
    }

    #[tokio::test]
    async fn a_refund_never_exceeds_the_capture() {
        let rails = FakeRails::new().with_capture(CaptureRecord {
            capture_id: "C1".into(),
            payer_email: Some("ana@example.com".into()),
            currency: "USD".into(),
            amount_minor: 2_000,
            refunded_minor: 0,
            subscription: None,
        });
        assert!(rails.refund("r1", "C1", 2_500, "too much").await.is_err());
        assert!(rails.refund("r2", "C1", 2_000, "all").await.is_ok());
        assert!(rails.refund("r3", "C1", 1, "more").await.is_err());
    }
}
