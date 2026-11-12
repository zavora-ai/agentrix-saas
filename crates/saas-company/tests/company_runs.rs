//! The company running on its own: the kernel wakes Receivables on a due date, and what the
//! company remembers about a customer shapes the plan it proposes.

mod common;

use std::sync::Arc;
use std::time::Duration;

use adk_core::audit_log::AuditDecision;
use adk_core::{Clock, LlmRequest, RunState, SteerOutcome};
use adk_kernel::ProcessFilter;
use chrono::TimeZone;
use saas_company::identity::{PAYABLES, RECEIVABLES};
use saas_company::memory::Behaviour;
use saas_company::rails::{CaptureRecord, PayoutDone, Rails, RefundDone, SubscriptionLink};
use saas_company::screen::InboundKind;
use saas_company::scripted::{Turn, last_tool_result, last_user_text};
use saas_company::supervisor::Supervisor;
use saas_core::books::{InvoiceId, PaymentFilter};
use saas_core::paypal::PayPalCaptureId;
use saas_core::{Books, Money, PayPalPosting, PayPalRef, post_paypal};
use serde_json::json;

/// Polls `probe` every 25 ms until it holds, for at most `limit`.
async fn eventually<F, Fut>(what: &str, limit: Duration, mut probe: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if probe().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

/// Whether the newest message in `req` is a fresh user message (not a tool result).
fn fresh(req: &LlmRequest) -> bool {
    req.contents
        .last()
        .is_some_and(|c| c.role == "user" && c.parts.iter().any(|p| p.text().is_some()))
}

/// The word after `label` in `text`, trimmed of punctuation.
fn word_after(text: &str, label: &str) -> Option<String> {
    let rest = &text[text.find(label)? + label.len()..];
    let word = rest.split_whitespace().next()?;
    Some(
        word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
            .to_owned(),
    )
}

/// Receivables, scripted to collect whatever instalment its wake-up names.
fn collector(req: &LlmRequest) -> Turn {
    if !fresh(req) {
        return Turn::Text("Collected.".into());
    }
    let text = last_user_text(req);
    match (
        word_after(&text, "plan_id"),
        word_after(&text, ", instalment"),
    ) {
        (Some(plan), Some(n)) => Turn::Calls(vec![(
            "collect_instalment".into(),
            json!({ "plan_id": plan, "instalment": n.parse::<u64>().unwrap_or(0) }),
        )]),
        _ => Turn::Text("Nothing to collect.".into()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_collections_run_fires_on_its_due_date() {
    let h = common::agentrix(4).await;
    let home = tempfile::tempdir().unwrap();
    let supervisor = Supervisor::start(
        h.company.clone(),
        Arc::new(h.clock.clone()) as Arc<dyn Clock>,
        home.path().into(),
    )
    .await
    .unwrap();
    let invoice = h.seeded.kilele_invoice.to_string();

    // Receivables agrees a two-part plan with Kilele (inside the mandate: 2 of 4 instalments,
    // 32 of 120 days); each instalment then sleeps in the kernel until its date.
    h.model(RECEIVABLES).extend([
        Turn::Calls(vec![(
            "propose_debt_plan".into(),
            json!({ "invoice_id": invoice, "instalments": [
                { "due_date": "2026-11-12", "amount_usd": 600.0 },
                { "due_date": "2026-12-12", "amount_usd": 600.0 },
            ] }),
        )]),
        Turn::Text("Agreed: two instalments.".into()),
    ]);
    for _ in 0..8 {
        h.model(RECEIVABLES)
            .extend([Turn::Dynamic(Arc::new(collector))]);
    }
    let agreed = h
        .company
        .ask(
            RECEIVABLES,
            "kilele-plan",
            "Kilele asks to pay INV-KIL-1 in two parts.",
        )
        .await
        .unwrap();
    assert!(agreed.refused.is_empty(), "{agreed:?}");
    let plan = h.company.desk.plans().pop().expect("a plan");

    let waiting = || async {
        supervisor
            .kernel()
            .processes()
            .ps(ProcessFilter::default().states([RunState::Waiting]))
            .await
            .unwrap()
            .len()
    };
    eventually(
        "both instalments asleep",
        Duration::from_secs(5),
        || async { waiting().await == 2 },
    )
    .await;

    // Not due yet: nothing runs, whatever the wall clock does.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(h.rails.settlements().is_empty());
    assert_eq!(waiting().await, 2);

    // 12 November, 09:00 in Nairobi: the first instalment's process wakes and collects.
    h.clock
        .set(chrono::Utc.with_ymd_and_hms(2026, 11, 12, 6, 0, 1).unwrap());
    eventually("the first collection", Duration::from_secs(15), || async {
        h.rails.settlements().len() == 1
    })
    .await;
    assert_eq!(h.rails.settlements()[0].amount_minor, 60_000);
    eventually("the plan to record it", Duration::from_secs(5), || async {
        h.company
            .desk
            .plan(&plan.plan_id)
            .is_some_and(|p| p.collected == vec![true, false])
    })
    .await;
    let inv = h
        .books
        .invoice(&InvoiceId::from(invoice.as_str()))
        .await
        .unwrap();
    assert_eq!(inv.balance_due.minor, 60_000, "the receipt is in the books");
    assert_eq!(waiting().await, 1, "December's instalment still sleeps");
    let allowed = h
        .audit
        .last("collect_instalment", AuditDecision::Allowed)
        .await
        .unwrap();
    assert!(allowed.is_some(), "collected on the governed path");

    // 12 December: the second.
    h.clock
        .set(chrono::Utc.with_ymd_and_hms(2026, 12, 12, 6, 0, 1).unwrap());
    eventually("the second collection", Duration::from_secs(15), || async {
        h.rails.settlements().len() == 2
    })
    .await;
    supervisor.stop().await.unwrap();
}

/// Receivables, scripted to read the customer's history and propose the plan it suggests.
fn planner(req: &LlmRequest) -> Turn {
    if fresh(req) {
        let text = last_user_text(req);
        return match (word_after(&text, "customer"), word_after(&text, "invoice")) {
            (Some(c), Some(i)) => Turn::Calls(vec![(
                "counterparty_history".into(),
                json!({ "customer_id": c, "invoice_id": i, "max_instalments": 4, "max_days": 120 }),
            )]),
            _ => Turn::Text("Which customer?".into()),
        };
    }
    if last_tool_result(req, "propose_debt_plan").is_some() {
        return Turn::Text("Plan agreed.".into());
    }
    match last_tool_result(req, "counterparty_history") {
        Some(history) => Turn::Calls(vec![(
            "propose_debt_plan".into(),
            history["suggested_plan"].clone(),
        )]),
        None => Turn::Text("No history.".into()),
    }
}

#[tokio::test]
async fn memory_changes_the_debt_plan_the_agent_proposes() {
    let h = common::agentrix(5).await;
    let memory = &h.company.desk.memory;
    let now = h.clock.now();
    let kilele = h.seeded.kilele.to_string();
    let acme = h.seeded.acme.to_string();
    // Kilele pays on time and keeps its promises; Acme pays late and broke a promise.
    for invoice in ["INV-K-7", "INV-K-8"] {
        memory
            .remember(
                &kilele,
                &Behaviour::Paid {
                    invoice: invoice.into(),
                    days_late: 0,
                },
                now,
            )
            .await
            .unwrap();
    }
    memory
        .remember(
            &kilele,
            &Behaviour::KeptPromise {
                invoice: "INV-K-8".into(),
            },
            now,
        )
        .await
        .unwrap();
    for (invoice, days_late) in [("INV-A-3", 45), ("INV-A-4", 30)] {
        memory
            .remember(
                &acme,
                &Behaviour::Paid {
                    invoice: invoice.into(),
                    days_late,
                },
                now,
            )
            .await
            .unwrap();
    }
    memory
        .remember(
            &acme,
            &Behaviour::BrokePromise {
                invoice: "INV-A-4".into(),
            },
            now,
        )
        .await
        .unwrap();

    // The same balance, the same agent and the same script; only the memory differs.
    for _ in 0..8 {
        h.model(RECEIVABLES)
            .extend([Turn::Dynamic(Arc::new(planner))]);
    }
    for (customer, invoice, session) in [
        (&kilele, h.seeded.kilele_invoice.to_string(), "plan-kilele"),
        (&acme, h.seeded.acme_invoice.to_string(), "plan-acme"),
    ] {
        let turn = h
            .company
            .ask(
                RECEIVABLES,
                session,
                &format!("The customer {customer} asks to pay invoice {invoice} over time."),
            )
            .await
            .unwrap();
        assert!(turn.refused.is_empty() && turn.held.is_empty(), "{turn:?}");
        let history = turn.result("counterparty_history").expect("history read");
        assert!(
            history["remembered"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
        );
    }
    let plan = |customer: &str| {
        h.company
            .desk
            .plans()
            .into_iter()
            .find(|p| p.customer == customer)
            .expect("a plan")
    };
    let (reliable, risky) = (plan(&kilele), plan(&acme));
    assert_eq!(reliable.instalments.len(), 2, "{reliable:?}");
    assert_eq!(risky.instalments.len(), 4, "{risky:?}");
    assert!(
        risky.instalments[0].amount_minor > reliable.instalments[0].amount_minor,
        "a risky payer pays more up front"
    );
    let total = |p: &saas_company::desk::DebtPlan| {
        p.instalments.iter().map(|i| i.amount_minor).sum::<i64>()
    };
    assert_eq!(total(&reliable), 120_000);
    assert_eq!(total(&risky), 120_000);
    // Both plans went through the governed path, inside the mandate.
    let records = h.audit.records().await.unwrap();
    let allowed = records
        .iter()
        .filter(|r| r.action == "propose_debt_plan" && r.decision == Some(AuditDecision::Allowed))
        .count();
    assert_eq!(allowed, 2);
}

/// PayPal that holds a payout until the test lets it go, and says when it has one.
struct Gate {
    inner: Arc<saas_company::rails::FakeRails>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl Rails for Gate {
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
        self.entered.notify_one();
        self.release.notified().await;
        self.inner
            .payout(key, receiver, currency, amount_minor, note)
            .await
    }
    async fn paid_out_today(&self, now: chrono::DateTime<chrono::Utc>) -> anyhow::Result<i64> {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_founder_steers_a_running_agent() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (e, r) = (entered.clone(), release.clone());
    let h = common::agentrix_wrapped(10, move |inner| {
        Arc::new(Gate {
            inner,
            entered: e,
            release: r,
        })
    })
    .await;
    let bill = h.seeded.vultr_bill.to_string();
    h.model(PAYABLES).extend([
        Turn::Calls(vec![(
            "pay_bill".into(),
            json!({ "bill_id": bill, "amount_usd": 120.0, "payee": "billing@vultr.example" }),
        )]),
        Turn::Dynamic(Arc::new(|req: &LlmRequest| {
            if last_user_text(req).contains("Wanjiru") {
                Turn::Text("Paid Vultr; Wanjiru Designs is next, as you asked.".into())
            } else {
                Turn::Text("Paid Vultr.".into())
            }
        })),
    ]);
    let company = h.company.clone();
    let running = tokio::spawn(async move {
        company
            .ask(PAYABLES, "ap-steer", "Pay the Vultr bill.")
            .await
    });

    // Mid-payment, the founder steers the running turn.
    entered.notified().await;
    let receipt = h
        .company
        .member(PAYABLES)
        .unwrap()
        .steer("ap-steer", "After Vultr, look at the Wanjiru Designs bill.")
        .await
        .unwrap();
    release.notify_one();
    assert!(matches!(
        receipt.outcome().await.unwrap(),
        SteerOutcome::Delivered { .. }
    ));
    let turn = running.await.unwrap().unwrap();
    assert!(turn.text.contains("Wanjiru"), "{turn:?}");
    assert_eq!(h.rails.payouts().len(), 1);
}

#[tokio::test]
async fn kilele_settles_agentrix_s_invoice_and_both_books_post_from_one_paypal_id() {
    let h = common::agentrix(11).await;
    let k = common::kilele(11, h.clock.clone(), h.rails.clone()).await;
    let bill = k.bill.to_string();

    // Agentrix's invoice reaches Kilele's Payables over the agent door; it settles it inside
    // the mandate Kilele's owner signed.
    k.payables.extend([
        Turn::Calls(vec![("settle_invoice".into(), json!({ "bill_id": bill }))]),
        Turn::Text("Settled Agentrix's invoice.".into()),
    ]);
    let (_, turn) = k
        .company
        .inbound(
            PAYABLES,
            "a2a-INV-KIL-1",
            InboundKind::AgentMessage,
            "agentrix/receivables",
            "Agentrix invoice INV-KIL-1 for USD 1,200 (Business plan, annual) is due.",
        )
        .await
        .unwrap();
    let turn = turn.expect("reached Kilele's Payables");
    let settled = turn.result("settle_invoice").expect("settled").clone();
    let paypal_id = settled["paypal_id"].as_str().unwrap().to_owned();
    let paid = k.books.bill(&k.bill).await.unwrap();
    assert!(!paid.balance_due.is_positive(), "Kilele's payable cleared");

    // Agentrix posts its receivable from the same PayPal id.
    let capture = PayPalRef::Capture(PayPalCaptureId::new(paypal_id.clone()).unwrap());
    post_paypal(
        h.books.as_ref(),
        &PayPalPosting::Received {
            paypal: capture.clone(),
            customer: h.seeded.kilele.clone(),
            gross: Money::usd(120_000),
            fee: Money::usd(0),
            date: h.clock.now().date_naive(),
            invoice: Some(h.seeded.kilele_invoice.clone()),
        },
    )
    .await
    .unwrap();
    let invoice = h.books.invoice(&h.seeded.kilele_invoice).await.unwrap();
    assert!(
        !invoice.balance_due.is_positive(),
        "Agentrix's receivable cleared"
    );
    for books in [&k.books, &h.books] {
        let payments = books
            .payments(&PaymentFilter {
                reference: Some(capture.key()),
                direction: None,
            })
            .await
            .unwrap();
        assert_eq!(payments.len(), 1, "one PayPal id, one posting in each book");
    }

    // The decision is in Kilele's own signed log, by Kilele's own agent.
    let allowed = k
        .audit
        .last("settle_invoice", AuditDecision::Allowed)
        .await
        .unwrap()
        .expect("allowed inside Kilele's mandate");
    assert_eq!(
        allowed.actor.agent.as_ref().map(|a| a.as_str()),
        Some("urn:adk:agent:kilele/payables")
    );
    assert!(
        h.audit
            .last("settle_invoice", AuditDecision::Allowed)
            .await
            .unwrap()
            .is_none(),
        "Agentrix's log holds none of Kilele's decisions"
    );
}
