//! The money flows as graphs: a held approval or a crash resumes where it stopped.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adk_core::Clock;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use saas_company::flows::{FlowStatus, Flows};
use saas_company::rails::{
    CaptureRecord, FakeRails, PayoutDone, Rails, RefundDone, SubscriptionLink,
};
use saas_core::books::{BillId, InvoiceId};
use saas_core::paypal::PayPalCaptureId;
use saas_core::{Books, Money, PayPalPosting, PayPalRef, post_paypal};
use serde_json::json;

fn db(dir: &tempfile::TempDir) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("flows.db").display()
    )
}

#[tokio::test]
async fn a_held_payment_waits_in_its_checkpoint_and_resumes_after_a_restart() {
    let h = common::agentrix(6).await;
    let dir = tempfile::tempdir().unwrap();
    let bill = h.seeded.anthropic_bill.to_string();

    // USD 900 is over the mandate: the flow stops at `pay`, holding the call.
    let flows = Flows::sqlite(h.company.clone(), &db(&dir)).await.unwrap();
    let status = flows.pay_bill(&bill).await.unwrap();
    let FlowStatus::Waiting { reason, data, .. } = status else {
        panic!("held, not {status:?}");
    };
    assert!(reason.contains("held for the founder"), "{reason}");
    assert_eq!(data.unwrap()["args"]["amount_usd"], json!(900.0));
    assert!(h.rails.payouts().is_empty());
    drop(flows);

    // The process restarts: new graphs over the same checkpoint database. The founder approves.
    let held = h.company.governance.held().await.unwrap();
    assert_eq!(held.len(), 1);
    h.company.governance.approve(&held[0]).await.unwrap();
    let flows = Flows::sqlite(h.company.clone(), &db(&dir)).await.unwrap();
    let status = flows.resume_bill(&bill).await.unwrap();
    let state = status.state().expect("done").clone();
    assert_eq!(state["result"]["paypal_id"], json!("FAKEBATCH0001"));
    assert_eq!(h.rails.payouts().len(), 1);
    let paid = h.books.bill(&BillId::from(bill.as_str())).await.unwrap();
    assert!(
        !paid.balance_due.is_positive(),
        "the bill is paid in the books"
    );

    // Running the flow again pays nothing more.
    let again = flows.resume_bill(&bill).await.unwrap();
    assert!(again.is_done());
    assert_eq!(h.rails.payouts().len(), 1);
}

/// PayPal that takes the first payout and then never answers: the process dies mid-call.
struct DiesMidPayout {
    inner: Arc<FakeRails>,
    died: AtomicBool,
}

#[async_trait]
impl Rails for DiesMidPayout {
    fn label(&self) -> String {
        self.inner.label()
    }
    async fn payout(
        &self,
        key: &str,
        receiver: &str,
        currency: &str,
        amount_minor: i64,
        note: &str,
    ) -> anyhow::Result<PayoutDone> {
        let done = self
            .inner
            .payout(key, receiver, currency, amount_minor, note)
            .await?;
        if !self.died.swap(true, Ordering::SeqCst) {
            // The money moved; the answer never comes back.
            std::future::pending::<()>().await;
        }
        Ok(done)
    }
    async fn paid_out_today(&self, now: DateTime<Utc>) -> anyhow::Result<i64> {
        self.inner.paid_out_today(now).await
    }
    async fn capture(&self, capture_id: &str) -> anyhow::Result<Option<CaptureRecord>> {
        self.inner.capture(capture_id).await
    }
    async fn refund(
        &self,
        key: &str,
        capture_id: &str,
        amount_minor: i64,
        note: &str,
    ) -> anyhow::Result<RefundDone> {
        self.inner.refund(key, capture_id, amount_minor, note).await
    }
    async fn start_subscription(
        &self,
        key: &str,
        plan_id: &str,
        customer_email: &str,
    ) -> anyhow::Result<SubscriptionLink> {
        self.inner
            .start_subscription(key, plan_id, customer_email)
            .await
    }
    async fn cancel_subscription(&self, provider_ref: &str, reason: &str) -> anyhow::Result<()> {
        self.inner.cancel_subscription(provider_ref, reason).await
    }
    async fn suspend_subscription(&self, provider_ref: &str, reason: &str) -> anyhow::Result<()> {
        self.inner.suspend_subscription(provider_ref, reason).await
    }
}

#[tokio::test]
async fn a_crash_mid_payout_resumes_and_pays_exactly_once() {
    let h = common::agentrix_wrapped(7, |inner| {
        Arc::new(DiesMidPayout {
            inner,
            died: AtomicBool::new(false),
        })
    })
    .await;
    let dir = tempfile::tempdir().unwrap();
    let bill = h.seeded.vultr_bill.to_string();

    // USD 120 is inside the mandate. PayPal pays, and the process dies before it hears back.
    let flows = Flows::sqlite(h.company.clone(), &db(&dir)).await.unwrap();
    let crashed = tokio::time::timeout(Duration::from_millis(500), flows.pay_bill(&bill)).await;
    assert!(crashed.is_err(), "the process died mid-payout");
    drop(flows);
    assert_eq!(h.rails.payouts().len(), 1, "the money moved once");

    // Restarted, the flow resumes at `pay`. The ledger shows the call begun and never finished,
    // so it is run again under the same key, and PayPal answers with the first payout.
    let flows = Flows::sqlite(h.company.clone(), &db(&dir)).await.unwrap();
    let status = flows.resume_bill(&bill).await.unwrap();
    assert!(status.is_done(), "{status:?}");
    assert_eq!(h.rails.payouts().len(), 1, "paid exactly once");
    let attempts: Vec<_> = h
        .rails
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("payout:"))
        .collect();
    assert_eq!(attempts.len(), 2, "{attempts:?}");
    assert_eq!(
        attempts[0], attempts[1],
        "the same PayPal request both times"
    );
    let paid = h.books.bill(&BillId::from(bill.as_str())).await.unwrap();
    assert!(!paid.balance_due.is_positive(), "posted to the books once");
}

#[tokio::test]
async fn an_invoice_is_issued_reminded_on_its_due_date_and_closed_when_paid() {
    let h = common::agentrix(8).await;
    let dir = tempfile::tempdir().unwrap();
    let flows = Flows::sqlite(h.company.clone(), &db(&dir)).await.unwrap();
    let customer = h.seeded.kilele.to_string();
    let lines = json!([{ "description": "Agentrix Business, annual", "amount_usd": 490.0 }]);

    // Issued on the governed path, then waiting for the cash.
    let status = flows
        .invoice("kil-annual", &customer, lines, 7)
        .await
        .unwrap();
    let FlowStatus::Waiting { data, .. } = status else {
        panic!("waiting for cash, not {status:?}");
    };
    let invoice = data.unwrap()["invoice_id"].as_str().unwrap().to_owned();

    // The due date passes; the kernel wakes the flow, which reminds once.
    h.clock.advance(Duration::from_secs(9 * 24 * 3600));
    let today = h.clock.now().date_naive();
    let status = flows
        .wake_invoice("kil-annual", &format!("due:{today}"))
        .await
        .unwrap();
    assert!(matches!(status, FlowStatus::Waiting { .. }), "{status:?}");
    assert_eq!(h.outbox.sent().len(), 1, "one reminder");
    flows
        .wake_invoice("kil-annual", &format!("due:{today}"))
        .await
        .unwrap();
    assert_eq!(h.outbox.sent().len(), 1, "not twice for the same date");

    // PayPal's money arrives and is posted; the flow closes and remembers how Kilele paid.
    post_paypal(
        h.books.as_ref(),
        &PayPalPosting::Received {
            paypal: PayPalRef::Capture(PayPalCaptureId::new("5TY05013RG002845M").unwrap()),
            customer: h.seeded.kilele.clone(),
            gross: Money::usd(49_000),
            fee: Money::usd(0),
            date: today,
            invoice: Some(InvoiceId::from(invoice.as_str())),
        },
    )
    .await
    .unwrap();
    let status = flows.wake_invoice("kil-annual", "paid").await.unwrap();
    let state = status.state().expect("closed").clone();
    assert_eq!(state["outcome"]["paid"], json!(invoice));
    let profile = h.company.desk.memory.profile(&customer).await.unwrap();
    assert_eq!(profile.payments, 1);
    assert_eq!(profile.late, 1, "{profile:?}");
}
