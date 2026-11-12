//! The desk: one company's operations behind every tool, and the facts the policy reads.
//!
//! A tool call that the governed path authorized lands here with the acting agent and its
//! idempotency key ([`CallCtx`]). The desk takes a credential scoped to the agent and the
//! resource it needs ([`crate::identity::StaffCredentials`]), does the work on the books
//! (Zavora ERA through saas-core's `Books`) and the money rails (PayPal through
//! [`crate::rails::Rails`], agent-to-agent through [`crate::rails::Settlement`]), and posts every
//! PayPal movement to the books through saas-core's posting rules, once per PayPal id.
//!
//! [`Desk`] is also the policy's [`Facts`]: before deciding a call, the policy learns from the
//! books and PayPal (never from the model) whether the payee is the one on file, how much is left
//! on a bill or a capture, and what has been paid out today.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use adk_core::{ActingContext, Clock};
use async_trait::async_trait;
use chrono::NaiveDate;
use saas_core::books::{
    BillId, CustomerId, DocumentFilter, Frequency, InvoiceDraft, InvoiceId, LineDraft,
    OpportunityId, RecurringInvoiceDraft, TicketDraft, TicketPriority, VendorId,
};
use saas_core::paypal::{PayPalCaptureId, PayPalPayoutItemId, PayPalRefundId};
use saas_core::{Books, CompanyId, Counterparty, Currency, Money, PayPalPosting, PayPalRef};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::identity::{Resource, StaffCredentials};
use crate::memory::{Behaviour, CounterpartyMemory, Instalment, suggest_plan};
use crate::policy::{Facts, amount_usd, dollars, to_minor};
use crate::rails::{Rails, Settlement};
use crate::screen::redact;

/// A plan the company sells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOffer {
    /// Catalogue id: `essential`, `business`, `commerce`, `agency`.
    pub plan_id: String,
    /// "Agentrix Business".
    pub name: String,
    /// The monthly price, in minor units.
    pub monthly_minor: i64,
    /// The yearly price (paid up front), in minor units.
    pub annual_minor: i64,
}

/// Agentrix's WebAgent plans.
pub fn agentrix_catalogue() -> Vec<PlanOffer> {
    let plan = |id: &str, name: &str, monthly: i64| PlanOffer {
        plan_id: id.into(),
        name: format!("Agentrix {name}"),
        monthly_minor: monthly,
        annual_minor: monthly * 10,
    };
    vec![
        plan("essential", "Essential", 1_900),
        plan("business", "Business", 4_900),
        plan("commerce", "Commerce", 9_900),
        plan("agency", "Agency", 24_900),
    ]
}

/// Where reminders go: email (slice D/E), or a recording outbox in tests and judge mode.
#[async_trait]
pub trait Outbox: Send + Sync {
    /// Sends one message.
    async fn send(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<()>;
}

/// An outbox that only records what it was asked to send.
#[derive(Debug, Default)]
pub struct RecordingOutbox {
    sent: Mutex<Vec<(String, String, String)>>,
}

impl RecordingOutbox {
    /// Everything sent: (to, subject, body).
    pub fn sent(&self) -> Vec<(String, String, String)> {
        self.sent.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[async_trait]
impl Outbox for RecordingOutbox {
    async fn send(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<()> {
        self.sent.lock().unwrap_or_else(|p| p.into_inner()).push((
            to.to_owned(),
            subject.to_owned(),
            body.to_owned(),
        ));
        Ok(())
    }
}

/// A debt plan the company agreed with a customer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebtPlan {
    /// `plan-<invoice>`.
    pub plan_id: String,
    /// The customer.
    pub customer: String,
    /// The invoice it pays.
    pub invoice: String,
    /// ISO 4217.
    pub currency: String,
    /// The instalments, in order.
    pub instalments: Vec<Instalment>,
    /// Which are collected.
    pub collected: Vec<bool>,
}

/// What a tool call carries into the desk: who is acting, and the call's idempotency key (the
/// action ledger's key, so a retried call is the same PayPal request).
#[derive(Debug, Clone)]
pub struct CallCtx {
    /// The acting agent.
    pub acting: ActingContext,
    /// The call's idempotency key.
    pub key: String,
}

/// One company's operations.
pub struct Desk {
    /// The company.
    pub company: CompanyId,
    /// Its currency.
    pub currency: Currency,
    /// Its books.
    pub books: Arc<dyn Books>,
    /// PayPal.
    pub rails: Arc<dyn Rails>,
    /// Agent-to-agent settlement.
    pub settlement: Arc<dyn Settlement>,
    /// What it remembers about counterparties.
    pub memory: CounterpartyMemory,
    /// Scoped credentials.
    pub creds: StaffCredentials,
    /// Reminders.
    pub outbox: Arc<dyn Outbox>,
    /// The clock (a manual one in tests).
    pub clock: Arc<dyn Clock>,
    /// What it sells.
    pub catalogue: Vec<PlanOffer>,
    plans: Mutex<BTreeMap<String, DebtPlan>>,
    plan_book: std::sync::OnceLock<Arc<dyn PlanBook>>,
    outside: std::sync::OnceLock<Arc<dyn Outside>>,
    failed_payments: Mutex<Vec<String>>,
    plan_events: tokio::sync::broadcast::Sender<DebtPlan>,
}

/// One agent-run customer's platform usage for a window: the invoice that bills it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageCharge {
    /// The customer, by its id in the books.
    pub customer_id: String,
    /// Its name.
    pub customer: String,
    /// The invoice's reference. It names the window, so the next charge starts where this one
    /// ends and a window is invoiced once.
    pub reference: String,
    /// The window's start (inclusive).
    pub from: chrono::DateTime<chrono::Utc>,
    /// Its end (exclusive).
    pub until: chrono::DateTime<chrono::Utc>,
    /// What was used, line by line.
    pub lines: Vec<UsageLine>,
}

/// One line of a [`UsageCharge`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLine {
    /// What it is.
    pub description: String,
    /// Its amount, in minor units.
    pub amount_minor: i64,
}

impl UsageCharge {
    /// The invoice's total, in minor units.
    pub fn total_minor(&self) -> i64 {
        self.lines.iter().map(|l| l.amount_minor).sum()
    }
}

/// What the desk reaches outside the books: the business's wallet record (saas-wallet) and the
/// owner (the platform's channels). The node provides it.
#[async_trait::async_trait]
pub trait Outside: Send + Sync {
    /// Reconciles the wallet against PayPal and signs a checkpoint; what it found.
    async fn wallet_checkpoint(&self) -> anyhow::Result<Value>;
    /// Tells the owner `text` (`kind`: `brief`, `alert`).
    async fn tell_owner(&self, kind: &str, text: &str) -> anyhow::Result<Value>;
    /// Posts the vendor bills for the model spend the gateway metered since the last bill.
    async fn bill_model_spend(&self) -> anyhow::Result<Value> {
        anyhow::bail!("this node doesn't bill model spend")
    }
    /// The wallet record as last reconciled, without reading PayPal again.
    async fn wallet_status(&self) -> anyhow::Result<Value> {
        anyhow::bail!("this business keeps no wallet record on this node")
    }
    /// What each agent-run customer owes for its platform usage since its last usage invoice
    /// (charges that come to nothing are left out).
    async fn usage_due(&self) -> anyhow::Result<Vec<UsageCharge>> {
        anyhow::bail!("this node doesn't meter its customers' usage")
    }
    /// Posts the company's payroll bill for the pay period that just ended, once per period;
    /// what it posted (or found already posted).
    async fn post_payroll(&self) -> anyhow::Result<Value> {
        anyhow::bail!("this node runs no payroll")
    }
    /// Today at the company, for the Chief's brief: what people did, what agents did, and the
    /// day's sales and costs, with the brief's lines already written from those figures
    /// (`lines`).
    async fn day_report(&self) -> anyhow::Result<Value> {
        anyhow::bail!("this node keeps no day report")
    }
}

/// Where agreed debt plans are kept, so a restarted node still knows what each instalment's
/// wake is for. Agreements, not a ledger: the money is in the books.
#[async_trait]
pub trait PlanBook: Send + Sync {
    /// Saves `plan` (replacing the plan of the same id).
    async fn save(&self, plan: &DebtPlan) -> anyhow::Result<()>;
    /// Every plan saved.
    async fn all(&self) -> anyhow::Result<Vec<DebtPlan>>;
}

impl std::fmt::Debug for Desk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Desk")
            .field("company", &self.company)
            .finish_non_exhaustive()
    }
}

/// How long a customer has to pay a usage invoice.
const USAGE_DUE_DAYS: i64 = 7;

/// The PayPal object a settlement names, as the books key it: a capture (Vault route) or a
/// payout item (Payout route). Both companies post from it, once.
fn settled_ref(settled: &crate::rails::Settled) -> anyhow::Result<PayPalRef> {
    Ok(match settled.paypal_kind.as_str() {
        "payout" => PayPalRef::Payout(PayPalPayoutItemId::new(settled.paypal_id.clone())?),
        _ => PayPalRef::Capture(PayPalCaptureId::new(settled.paypal_id.clone())?),
    })
}

fn arg<'a>(args: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{key} is required"))
}

fn same_email(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

impl Desk {
    /// A desk over these parts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        company: CompanyId,
        books: Arc<dyn Books>,
        rails: Arc<dyn Rails>,
        settlement: Arc<dyn Settlement>,
        memory: CounterpartyMemory,
        creds: StaffCredentials,
        outbox: Arc<dyn Outbox>,
        clock: Arc<dyn Clock>,
        catalogue: Vec<PlanOffer>,
    ) -> Self {
        Self {
            company,
            currency: Currency::USD,
            books,
            rails,
            settlement,
            memory,
            creds,
            outbox,
            clock,
            catalogue,
            plans: Mutex::new(BTreeMap::new()),
            plan_book: std::sync::OnceLock::new(),
            outside: std::sync::OnceLock::new(),
            failed_payments: Mutex::new(Vec::new()),
            plan_events: tokio::sync::broadcast::channel(64).0,
        }
    }

    /// Today, by the desk's clock.
    pub fn today(&self) -> NaiveDate {
        self.clock.now().date_naive()
    }

    fn money(&self, minor: i64) -> Money {
        Money::new(minor, self.currency)
    }

    fn usd(&self, args: &Value) -> anyhow::Result<Money> {
        let a = amount_usd(args.get("amount_usd"))
            .ok_or_else(|| anyhow::anyhow!("amount_usd is required"))?;
        Ok(self.money(to_minor(a)))
    }

    /// A debt plan, by id.
    pub fn plan(&self, plan_id: &str) -> Option<DebtPlan> {
        self.plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(plan_id)
            .cloned()
    }

    /// Every debt plan.
    pub fn plans(&self) -> Vec<DebtPlan> {
        self.plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Reaches the wallet and the owner through `outside` (once).
    pub fn use_outside(&self, outside: Arc<dyn Outside>) {
        let _ = self.outside.set(outside);
    }

    fn outside(&self) -> anyhow::Result<&Arc<dyn Outside>> {
        self.outside
            .get()
            .ok_or_else(|| anyhow::anyhow!("this node has no wallet record or owner channel"))
    }

    /// Keeps plans in `book`, and loads the ones it already holds. Returns them.
    pub async fn use_plan_book(&self, book: Arc<dyn PlanBook>) -> anyhow::Result<Vec<DebtPlan>> {
        let saved = book.all().await?;
        {
            let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
            for plan in &saved {
                plans.insert(plan.plan_id.clone(), plan.clone());
            }
        }
        let _ = self.plan_book.set(book);
        Ok(saved)
    }

    async fn keep(&self, plan: &DebtPlan) -> anyhow::Result<()> {
        if let Some(book) = self.plan_book.get() {
            book.save(plan).await?;
        }
        Ok(())
    }

    /// Records a plan agreed elsewhere (another company's door started it) and announces it,
    /// so its instalments are scheduled. A plan already held is kept as it is.
    pub async fn adopt_plan(&self, plan: DebtPlan) -> anyhow::Result<DebtPlan> {
        if let Some(held) = self.plan(&plan.plan_id) {
            return Ok(held);
        }
        self.plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(plan.plan_id.clone(), plan.clone());
        self.keep(&plan).await?;
        let _ = self.plan_events.send(plan.clone());
        Ok(plan)
    }

    /// Every debt plan agreed from now on, as it is agreed: the supervisor schedules each
    /// instalment's due date from these.
    pub fn subscribe_plans(&self) -> tokio::sync::broadcast::Receiver<DebtPlan> {
        self.plan_events.subscribe()
    }

    /// PayPal reported a failed payment on `provider_ref` (a webhook).
    pub fn payment_failed(&self, provider_ref: &str) {
        self.failed_payments
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(provider_ref.to_owned());
    }

    async fn need(&self, ctx: &CallCtx, resource: Resource, scope: &str) -> anyhow::Result<()> {
        self.creds
            .token(&ctx.acting, resource, scope)
            .await
            .map_err(|e| anyhow::anyhow!("no credential for {scope}: {}", e.message))?;
        Ok(())
    }

    /// Runs tool `tool` for `ctx`. Only the governed path calls this, after it authorized the
    /// call; it does the work and returns what the model is told.
    pub async fn run(&self, tool: &str, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        match tool {
            "company_brief" => self.company_brief(ctx).await,
            "crm_pipeline" => self.crm_pipeline(ctx).await,
            "crm_mark_won" => self.crm_mark_won(ctx, args).await,
            "start_subscription" => self.start_subscription(ctx, args).await,
            "create_recurring_invoice" => self.create_recurring_invoice(ctx, args).await,
            "ar_ageing" => self.ar_ageing(ctx).await,
            "counterparty_history" => self.counterparty_history(ctx, args).await,
            "issue_invoice" => self.issue_invoice(ctx, args).await,
            "send_reminder" => self.send_reminder(ctx, args).await,
            "propose_debt_plan" => self.propose_debt_plan(ctx, args).await,
            "collect_instalment" => self.collect_instalment(ctx, args).await,
            "suspend_subscription" => self.suspend_subscription(ctx, args).await,
            "ap_bills_due" => self.ap_bills_due(ctx).await,
            "pay_bill" => self.pay_bill(ctx, args).await,
            "change_vendor_payee" => self.change_vendor_payee(ctx, args).await,
            "settle_invoice" => self.settle_invoice(ctx, args).await,
            "lookup_capture" => self.lookup_capture(ctx, args).await,
            "refund_payment" => self.refund_payment(ctx, args).await,
            "cancel_subscription" => self.cancel_subscription(ctx, args).await,
            "wallet_checkpoint" => self.outside()?.wallet_checkpoint().await,
            "bill_model_spend" => self.outside()?.bill_model_spend().await,
            "post_payroll" => self.outside()?.post_payroll().await,
            "wallet_status" => self.outside()?.wallet_status().await,
            "invoice_usage" => self.invoice_usage(ctx).await,
            "brief_owner" => {
                let text = arg(args, "text")?;
                self.outside()?.tell_owner("brief", text).await
            }
            other => anyhow::bail!("no tool {other}"),
        }
    }

    async fn company_brief(&self, ctx: &CallCtx) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:read").await?;
        let today = self.today();
        let ar = self.books.ar_ageing(today).await?;
        let ap = self.books.ap_ageing(today).await?;
        // The day's figures, where the node keeps them; the brief is written from these.
        let day = match self.outside() {
            Ok(outside) => outside
                .day_report()
                .await
                .inspect_err(|e| tracing::debug!(error = %format!("{e:#}"), "no day report"))
                .ok(),
            Err(_) => None,
        };
        Ok(json!({
            "today": day,
            "company": self.company.as_str(),
            "as_of": today,
            "owed_to_us": ar.totals.total()?.to_string(),
            "owed_to_us_over_30_days": ar.totals.days_31_60.checked_add(ar.totals.days_61_90)?.checked_add(ar.totals.over_90)?.to_string(),
            "we_owe": ap.totals.total()?.to_string(),
            "debt_plans": self.plans().len(),
            "paypal": self.rails.label(),
        }))
    }

    async fn crm_pipeline(&self, ctx: &CallCtx) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:read").await?;
        let leads = self.books.leads().await?;
        let opportunities = self.books.opportunities().await?;
        Ok(json!({
            "leads": leads.iter().map(|l| json!({ "id": l.id, "contact": redact(&l.contact_name), "company": l.company_name, "status": l.status })).collect::<Vec<_>>(),
            "opportunities": opportunities.iter().map(|o| json!({ "id": o.id, "name": o.name, "amount": o.amount.to_string(), "stage": o.stage })).collect::<Vec<_>>(),
        }))
    }

    async fn crm_mark_won(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:crm").await?;
        let opportunity = OpportunityId::from(arg(args, "opportunity_id")?);
        let name = arg(args, "customer_name")?;
        let email = args.get("customer_email").and_then(Value::as_str);
        let customer = self
            .books
            .ensure_customer(&Counterparty::usd(name, email), None)
            .await?;
        let won = self.books.win_opportunity(&opportunity).await?;
        Ok(json!({ "opportunity": won.id, "stage": won.stage, "customer_id": customer.id }))
    }

    fn offer(&self, plan_id: &str) -> Option<&PlanOffer> {
        self.catalogue.iter().find(|p| p.plan_id == plan_id)
    }

    async fn start_subscription(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::PayPal, "paypal:subscribe").await?;
        let plan_id = arg(args, "plan_id")?;
        let email = arg(args, "customer_email")?;
        let offer = self
            .offer(plan_id)
            .ok_or_else(|| anyhow::anyhow!("no plan {plan_id}"))?;
        let link = self
            .rails
            .start_subscription(&ctx.key, plan_id, email)
            .await?;
        Ok(json!({
            "plan": offer.name,
            "subscription": link.provider_ref,
            "approve_url": link.approve_url,
            "next": "the customer approves on PayPal's page; the receipt posts when PayPal's signed webhook checks out",
        }))
    }

    async fn create_recurring_invoice(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:invoice").await?;
        let customer = CustomerId::from(arg(args, "customer_id")?);
        let plan_id = arg(args, "plan_id")?;
        let offer = self
            .offer(plan_id)
            .ok_or_else(|| anyhow::anyhow!("no plan {plan_id}"))?;
        let (frequency, price) = match args.get("frequency").and_then(Value::as_str) {
            Some("yearly") => (Frequency::Yearly, offer.annual_minor),
            _ => (Frequency::Monthly, offer.monthly_minor),
        };
        let recurring = self
            .books
            .create_recurring_invoice(&RecurringInvoiceDraft {
                customer,
                currency: self.currency,
                lines: vec![LineDraft {
                    product: None,
                    description: offer.name.clone(),
                    quantity: 1,
                    unit_price: self.money(price),
                }],
                frequency,
                start_date: self.today(),
                end_date: None,
                due_in_days: 14,
                reference: Some(
                    args.get("reference")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("plan:{plan_id}:{}", ctx.key)),
                ),
            })
            .await?;
        Ok(
            json!({ "recurring_invoice": recurring.id, "amount": recurring.amount.to_string(), "next_date": recurring.next_date }),
        )
    }

    async fn ar_ageing(&self, ctx: &CallCtx) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:read").await?;
        let report = self.books.ar_ageing(self.today()).await?;
        Ok(json!({
            "as_of": report.as_of,
            "total": report.totals.total()?.to_string(),
            "rows": report.rows.iter().map(|r| json!({
                "party": r.party, "name": r.name,
                "current": r.buckets.current.to_string(), "days_1_30": r.buckets.days_1_30.to_string(),
                "days_31_60": r.buckets.days_31_60.to_string(), "days_61_90": r.buckets.days_61_90.to_string(),
                "over_90": r.buckets.over_90.to_string(),
            })).collect::<Vec<_>>(),
        }))
    }

    async fn counterparty_history(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:read").await?;
        let customer = arg(args, "customer_id")?;
        let profile = self.memory.profile(customer).await?;
        let recalled = self.memory.recall(customer, 10).await?;
        let mut out = json!({
            "customer_id": customer,
            "standing": profile.standing(),
            "profile": profile,
            "remembered": recalled,
        });
        if let Some(invoice) = args.get("invoice_id").and_then(Value::as_str) {
            let invoice = self.books.invoice(&InvoiceId::from(invoice)).await?;
            let mandate_hint = |key: &str, default: u32| {
                args.get(key)
                    .and_then(Value::as_u64)
                    .map(|v| v as u32)
                    .unwrap_or(default)
            };
            let plan = suggest_plan(
                profile.standing(),
                invoice.balance_due.minor,
                self.today(),
                mandate_hint("max_instalments", 4),
                mandate_hint("max_days", 120),
            );
            out["suggested_plan"] = json!({
                "invoice_id": invoice.id,
                "instalments": plan.iter().map(|i| json!({ "due_date": i.due_date, "amount_usd": dollars(i.amount_minor) })).collect::<Vec<_>>(),
            });
        }
        Ok(out)
    }

    async fn issue_invoice(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:invoice").await?;
        let customer = CustomerId::from(arg(args, "customer_id")?);
        let lines = args
            .get("lines")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("lines are required"))?
            .iter()
            .map(|l| {
                Ok(LineDraft {
                    product: None,
                    description: arg(l, "description")?.to_owned(),
                    quantity: l.get("quantity").and_then(Value::as_u64).unwrap_or(1) as u32,
                    unit_price: self.usd(l)?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let due_in = args
            .get("due_in_days")
            .and_then(Value::as_i64)
            .unwrap_or(14);
        let reference = args
            .get("reference")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| ctx.key.clone());
        let posted = self
            .post_invoice(customer, lines, due_in, reference, None)
            .await?;
        Ok(
            json!({ "invoice_id": posted.id, "number": posted.number, "total": posted.total.to_string(), "due_date": posted.due_date }),
        )
    }

    /// Drafts, submits and posts an invoice in the books.
    async fn post_invoice(
        &self,
        customer: CustomerId,
        lines: Vec<LineDraft>,
        due_in_days: i64,
        reference: String,
        memo: Option<String>,
    ) -> anyhow::Result<saas_core::books::Invoice> {
        let today = self.today();
        let draft = self
            .books
            .draft_invoice(&InvoiceDraft {
                customer,
                issue_date: today,
                due_date: today + chrono::Duration::days(due_in_days),
                currency: self.currency,
                lines,
                reference: Some(reference),
                memo,
            })
            .await?;
        self.books.submit_invoice(&draft.id).await?;
        Ok(self.books.post_invoice(&draft.id).await?)
    }

    /// Bills each agent-run customer for its platform usage since its last usage invoice
    /// ([`Outside::usage_due`]) and offers each invoice to that company's agent door, where its
    /// Payables settles it. A window is invoiced once: an invoice already in the books under the
    /// charge's reference is offered again rather than issued twice.
    async fn invoice_usage(&self, ctx: &CallCtx) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:invoice").await?;
        self.need(ctx, Resource::AgentDoor, "door:offer").await?;
        let mut out = Vec::new();
        for charge in self.outside()?.usage_due().await? {
            let found = self
                .books
                .invoices(&DocumentFilter {
                    reference: Some(charge.reference.clone()),
                    ..Default::default()
                })
                .await?
                .into_iter()
                .find(|i| i.status != saas_core::books::DocumentStatus::Void);
            let invoice = match found {
                Some(invoice) => invoice,
                None => {
                    let lines = charge
                        .lines
                        .iter()
                        .map(|l| LineDraft {
                            product: None,
                            description: l.description.clone(),
                            quantity: 1,
                            unit_price: self.money(l.amount_minor),
                        })
                        .collect();
                    let memo = format!(
                        "Platform usage of {} on Agentrix, {} to {} UTC, metered by the Agentrix \
                         model gateway.",
                        charge.customer,
                        charge.from.format("%Y-%m-%d %H:%M"),
                        charge.until.format("%Y-%m-%d %H:%M"),
                    );
                    self.post_invoice(
                        CustomerId::from(charge.customer_id.as_str()),
                        lines,
                        USAGE_DUE_DAYS,
                        charge.reference.clone(),
                        Some(memo),
                    )
                    .await?
                }
            };
            let offered = match self.settlement.offer_invoice(invoice.id.as_str()).await {
                Ok(decision) => json!(decision),
                Err(e) => json!(format!("not offered: {}", redact(&format!("{e:#}")))),
            };
            out.push(json!({
                "customer": charge.customer, "invoice_id": invoice.id, "number": invoice.number,
                "total": invoice.total.to_string(), "window": format!("{} to {} UTC",
                    charge.from.format("%Y-%m-%d %H:%M"), charge.until.format("%H:%M")),
                "offered": offered,
            }));
        }
        Ok(json!({ "invoices": out }))
    }

    async fn send_reminder(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:remind").await?;
        let invoice = self
            .books
            .invoice(&InvoiceId::from(arg(args, "invoice_id")?))
            .await?;
        let customer = self.books.customer(&invoice.customer).await?;
        let to = customer
            .counterparty
            .email
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the books have no email for {}", customer.id))?;
        let days = (self.today() - invoice.due_date).num_days();
        let note = args
            .get("note")
            .and_then(Value::as_str)
            .map(redact)
            .unwrap_or_default();
        let body = format!(
            "{} is {days} days past due; {} is still open. {note}",
            invoice
                .number
                .clone()
                .unwrap_or_else(|| invoice.id.to_string()),
            invoice.balance_due
        );
        self.outbox
            .send(&to, &format!("Reminder: {}", invoice.id), &body)
            .await?;
        self.memory
            .note(
                customer.id.as_str(),
                &format!("Reminded about {} at {days} days past due.", invoice.id),
                self.clock.now(),
            )
            .await?;
        Ok(json!({ "reminded": invoice.id, "days_past_due": days }))
    }

    fn parse_instalments(&self, args: &Value) -> anyhow::Result<Vec<Instalment>> {
        args.get("instalments")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("instalments are required"))?
            .iter()
            .map(|i| {
                Ok(Instalment {
                    due_date: arg(i, "due_date")?.parse()?,
                    amount_minor: self.usd(i)?.minor,
                })
            })
            .collect()
    }

    async fn propose_debt_plan(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:invoice").await?;
        let invoice = self
            .books
            .invoice(&InvoiceId::from(arg(args, "invoice_id")?))
            .await?;
        let mut instalments = self.parse_instalments(args)?;
        let plan_id = format!("plan-{}", invoice.id);
        // With an agent-run customer the plan is agreed between the two companies' agents,
        // inside both owners' mandates; a refusal leaves no plan.
        let customer = self.books.customer(&invoice.customer).await?;
        if let Some(counterparty) = &customer.counterparty.company {
            let agreed = self
                .settlement
                .agree_plan(
                    &ctx.key,
                    counterparty.as_str(),
                    &plan_id,
                    invoice.id.as_str(),
                    invoice.balance_due.currency.code(),
                    &instalments
                        .iter()
                        .map(|i| (i.due_date, i.amount_minor))
                        .collect::<Vec<_>>(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{counterparty} didn't agree the plan: {e}"))?;
            instalments = agreed
                .into_iter()
                .map(|(due_date, amount_minor)| Instalment {
                    due_date,
                    amount_minor,
                })
                .collect();
        }
        let plan = DebtPlan {
            plan_id,
            customer: invoice.customer.to_string(),
            invoice: invoice.id.to_string(),
            currency: self.currency.to_string(),
            collected: vec![false; instalments.len()],
            instalments,
        };
        for i in &plan.instalments {
            self.memory
                .remember(
                    &plan.customer,
                    &Behaviour::Promised {
                        invoice: plan.invoice.clone(),
                        amount_minor: i.amount_minor,
                        by: i.due_date,
                    },
                    self.clock.now(),
                )
                .await?;
        }
        self.plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(plan.plan_id.clone(), plan.clone());
        self.keep(&plan).await?;
        // No one listening (no supervisor) is fine: the plan is in the desk either way.
        let _ = self.plan_events.send(plan.clone());
        Ok(json!({
            "plan_id": plan.plan_id,
            "instalments": plan.instalments.iter().map(|i| json!({ "due_date": i.due_date, "amount_usd": dollars(i.amount_minor) })).collect::<Vec<_>>(),
            "next": "each instalment is collected on its due date",
        }))
    }

    async fn collect_instalment(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::AgentDoor, "door:collect").await?;
        let plan_id = arg(args, "plan_id")?;
        let n = args
            .get("instalment")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("instalment is required"))? as usize;
        let plan = self
            .plan(plan_id)
            .ok_or_else(|| anyhow::anyhow!("no plan {plan_id}"))?;
        let due = plan
            .instalments
            .get(n)
            .ok_or_else(|| anyhow::anyhow!("plan {plan_id} has no instalment {n}"))?
            .clone();
        let settled = self
            .settlement
            .collect_instalment(
                &ctx.key,
                &plan.customer,
                plan_id,
                n as u32,
                &plan.currency,
                due.amount_minor,
            )
            .await?;
        let posted = saas_core::post_paypal(
            self.books.as_ref(),
            &PayPalPosting::Received {
                paypal: settled_ref(&settled)?,
                customer: CustomerId::from(plan.customer.as_str()),
                gross: self.money(due.amount_minor),
                fee: self.money(0),
                date: settled.settled_on,
                invoice: Some(InvoiceId::from(plan.invoice.as_str())),
            },
        )
        .await?;
        let updated = {
            let mut plans = self.plans.lock().unwrap_or_else(|p| p.into_inner());
            match plans.get_mut(plan_id) {
                Some(p) => {
                    if let Some(c) = p.collected.get_mut(n) {
                        *c = true;
                    }
                    Some(p.clone())
                }
                None => None,
            }
        };
        if let Some(p) = updated {
            self.keep(&p).await?;
        }
        let late = (settled.settled_on - due.due_date).num_days();
        self.memory
            .remember(
                &plan.customer,
                &Behaviour::KeptPromise {
                    invoice: plan.invoice.clone(),
                },
                self.clock.now(),
            )
            .await?;
        Ok(json!({
            "collected": dollars(due.amount_minor), "instalment": n, "paypal_id": settled.paypal_id,
            "days_late": late, "posted": !posted.already_posted,
        }))
    }

    async fn suspend_subscription(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::PayPal, "paypal:subscribe").await?;
        let sub = arg(args, "subscription")?;
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("payment failed");
        self.rails.suspend_subscription(sub, reason).await?;
        Ok(json!({ "suspended": sub }))
    }

    async fn ap_bills_due(&self, ctx: &CallCtx) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:read").await?;
        let bills = self
            .books
            .bills(&DocumentFilter {
                party: None,
                open_only: true,
                reference: None,
            })
            .await?;
        let mut out = Vec::new();
        for b in bills {
            let vendor = self.books.vendor(&b.vendor).await?;
            // A bill from another agent-run company is settled at its agent door
            // (settle_invoice), never paid out to an email.
            let pay_with = if vendor.counterparty.company.is_some() {
                "settle_invoice"
            } else {
                "pay_bill"
            };
            out.push(json!({
                "bill_id": b.id, "vendor_id": vendor.id, "vendor": vendor.counterparty.name,
                "payee": vendor.counterparty.paypal_email, "due_date": b.due_date,
                "amount_usd": dollars(b.balance_due.minor), "reference": b.vendor_reference,
                "pay_with": pay_with,
            }));
        }
        Ok(json!({ "bills": out }))
    }

    async fn pay_bill(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::PayPal, "paypal:payout").await?;
        self.need(ctx, Resource::Books, "books:bill").await?;
        let bill = self
            .books
            .bill(&BillId::from(arg(args, "bill_id")?))
            .await?;
        let vendor = self.books.vendor(&bill.vendor).await?;
        let payee = arg(args, "payee")?;
        // The policy denied any other payee already; the desk pays only the one on file anyway.
        let on_file = vendor
            .counterparty
            .paypal_email
            .clone()
            .filter(|p| same_email(p, payee))
            .ok_or_else(|| anyhow::anyhow!("that payee isn't the one the books hold"))?;
        let amount = self.usd(args)?;
        let done = self
            .rails
            .payout(
                &ctx.key,
                &on_file,
                self.currency.code(),
                amount.minor,
                &format!(
                    "Bill {}",
                    bill.vendor_reference
                        .clone()
                        .unwrap_or_else(|| bill.id.to_string())
                ),
            )
            .await?;
        // The books post a payout by its item; one PayPal hasn't itemised yet posts when its
        // confirmation (PAYMENT.PAYOUTS-ITEM.SUCCEEDED) arrives, never twice.
        let posted = match &done.item_id {
            Some(item) => Some(
                saas_core::post_paypal(
                    self.books.as_ref(),
                    &PayPalPosting::PaidOut {
                        paypal: PayPalRef::Payout(PayPalPayoutItemId::new(item.clone())?),
                        vendor: vendor.id.clone(),
                        amount,
                        fee: self.money(0),
                        date: self.today(),
                        bill: Some(bill.id.clone()),
                    },
                )
                .await?,
            ),
            None => None,
        };
        Ok(json!({
            "paid": amount.to_string(), "to": vendor.counterparty.name, "paypal_id": done.paypal_id,
            "status": done.status, "posted": posted.is_some_and(|p| !p.already_posted),
        }))
    }

    async fn change_vendor_payee(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::Books, "books:vendor").await?;
        let vendor = self
            .books
            .vendor(&VendorId::from(arg(args, "vendor_id")?))
            .await?;
        let payee = arg(args, "payee")?;
        // saas-core has no vendor update yet: the approved change is recorded in ERA as a ticket
        // for the books to apply (follow-up for slice A).
        let ticket = self
            .books
            .open_ticket(&TicketDraft {
                customer: None,
                subject: format!("Payee change approved for {}", vendor.counterparty.name),
                description: format!(
                    "The founder approved paying {} at {} from now on.",
                    vendor.counterparty.name,
                    redact(payee)
                ),
                priority: TicketPriority::High,
            })
            .await?;
        Ok(json!({ "vendor": vendor.id, "ticket": ticket.id }))
    }

    async fn settle_invoice(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::AgentDoor, "door:settle").await?;
        let bill = self
            .books
            .bill(&BillId::from(arg(args, "bill_id")?))
            .await?;
        let vendor = self.books.vendor(&bill.vendor).await?;
        let counterparty = vendor
            .counterparty
            .company
            .clone()
            .ok_or_else(|| anyhow::anyhow!("{} isn't an agent-run company", vendor.id))?;
        let settled = self
            .settlement
            .settle_invoice(
                &ctx.key,
                counterparty.as_str(),
                bill.vendor_reference.as_deref().unwrap_or(bill.id.as_str()),
                bill.balance_due.currency.code(),
                bill.balance_due.minor,
            )
            .await?;
        let posted = saas_core::post_paypal(
            self.books.as_ref(),
            &PayPalPosting::PaidOut {
                paypal: settled_ref(&settled)?,
                vendor: vendor.id.clone(),
                amount: bill.balance_due,
                fee: self.money(0),
                date: settled.settled_on,
                bill: Some(bill.id.clone()),
            },
        )
        .await?;
        Ok(json!({
            "settled": bill.balance_due.to_string(), "with": counterparty.as_str(),
            "paypal_id": settled.paypal_id, "posted": !posted.already_posted,
        }))
    }

    async fn lookup_capture(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::PayPal, "paypal:refund").await?;
        let capture = self.rails.capture(arg(args, "capture_id")?).await?;
        Ok(match capture {
            Some(c) => json!({
                "capture_id": c.capture_id, "payer": c.payer_email.as_deref().map(redact),
                "amount_usd": dollars(c.amount_minor), "refundable_usd": dollars(c.refundable_minor()),
                "subscription": c.subscription,
            }),
            None => json!({ "capture_id": args["capture_id"], "found": false }),
        })
    }

    async fn refund_payment(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::PayPal, "paypal:refund").await?;
        let capture_id = arg(args, "capture_id")?;
        let customer = CustomerId::from(arg(args, "customer_id")?);
        let amount = self.usd(args)?;
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("refund")
            .to_owned();
        // PayPal refunds a capture to the account that paid it: there is no "to" to get wrong.
        let done = self
            .rails
            .refund(&ctx.key, capture_id, amount.minor, &reason)
            .await?;
        let posted = saas_core::post_paypal(
            self.books.as_ref(),
            &PayPalPosting::Refunded {
                paypal: PayPalRef::Refund(PayPalRefundId::new(done.refund_id.clone())?),
                customer,
                amount,
                fee_returned: self.money(0),
                date: self.today(),
                invoice: args
                    .get("invoice_id")
                    .and_then(Value::as_str)
                    .map(InvoiceId::from),
                reason: redact(&reason),
            },
        )
        .await?;
        Ok(json!({
            "refunded": amount.to_string(), "refund_id": done.refund_id, "to": "the original payer",
            "posted": !posted.already_posted,
        }))
    }

    async fn cancel_subscription(&self, ctx: &CallCtx, args: &Value) -> anyhow::Result<Value> {
        self.need(ctx, Resource::PayPal, "paypal:subscribe").await?;
        let sub = arg(args, "subscription")?;
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("the customer asked");
        self.rails.cancel_subscription(sub, &redact(reason)).await?;
        Ok(json!({ "cancelled": sub }))
    }

    async fn pay_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let Some(bill_id) = args.get("bill_id").and_then(Value::as_str) else {
            return Ok(json!({ "bill_open": false }));
        };
        let Ok(bill) = self.books.bill(&BillId::from(bill_id)).await else {
            return Ok(json!({ "bill_open": false }));
        };
        let vendor = self.books.vendor(&bill.vendor).await?;
        let of_record = vendor.counterparty.paypal_email.clone();
        let amount = amount_usd(args.get("amount_usd")).unwrap_or(0.0);
        let today = self.rails.paid_out_today(self.clock.now()).await?;
        let payee_on_file = match (args.get("payee").and_then(Value::as_str), &of_record) {
            (Some(p), Some(r)) => same_email(p, r),
            _ => false,
        };
        Ok(json!({
            "bill_open": bill.status.is_open() && bill.balance_due.is_positive(),
            "vendor_is_company": vendor.counterparty.company.is_some(),
            "vendor_id": vendor.id,
            "vendor_name": vendor.counterparty.name,
            "payee_of_record": of_record.as_deref().map(redact),
            "payee_on_file": payee_on_file,
            "within_bill": to_minor(amount) <= bill.balance_due.minor,
            "bill_due_usd": dollars(bill.balance_due.minor),
            "day_total_after_usd": dollars(today + to_minor(amount)),
        }))
    }

    async fn settle_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let Some(bill_id) = args.get("bill_id").and_then(Value::as_str) else {
            return Ok(json!({ "bill_matches": false }));
        };
        let Ok(bill) = self.books.bill(&BillId::from(bill_id)).await else {
            return Ok(json!({ "bill_matches": false }));
        };
        let vendor = self.books.vendor(&bill.vendor).await?;
        let today = self.rails.paid_out_today(self.clock.now()).await?;
        Ok(json!({
            "bill_matches": bill.status.is_open() && bill.balance_due.is_positive() && vendor.counterparty.company.is_some(),
            "amount_usd": dollars(bill.balance_due.minor),
            "day_total_after_usd": dollars(today + bill.balance_due.minor),
        }))
    }

    async fn refund_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let capture_id = args
            .get("capture_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(capture) = self.rails.capture(capture_id).await? else {
            return Ok(json!({ "capture_found": false }));
        };
        let amount = amount_usd(args.get("amount_usd")).unwrap_or(0.0);
        let to_original_payer = match args.get("refund_to").and_then(Value::as_str) {
            None => true,
            Some(to) => capture
                .payer_email
                .as_deref()
                .is_some_and(|p| same_email(p, to)),
        };
        Ok(json!({
            "capture_found": true,
            "original_payer": capture.payer_email.as_deref().map(redact),
            "to_original_payer": to_original_payer,
            "within_capture": to_minor(amount) <= capture.refundable_minor(),
            "refundable_usd": dollars(capture.refundable_minor()),
        }))
    }

    async fn invoice_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let Some(customer) = args.get("customer_id").and_then(Value::as_str) else {
            return Ok(json!({ "customer_known": false }));
        };
        let Ok(position) = self
            .books
            .credit_position(&CustomerId::from(customer))
            .await
        else {
            return Ok(json!({ "customer_known": false }));
        };
        let total: i64 = args
            .get("lines")
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .map(|l| {
                        to_minor(amount_usd(l.get("amount_usd")).unwrap_or(0.0))
                            * l.get("quantity").and_then(Value::as_i64).unwrap_or(1)
                    })
                    .sum()
            })
            .unwrap_or(0);
        Ok(json!({
            "customer_known": true,
            "within_credit_limit": position.allows(self.money(total)),
        }))
    }

    /// Whether each usage charge's customer is in the books, and whether every charge fits
    /// its customer's credit limit.
    async fn usage_facts(&self) -> anyhow::Result<Value> {
        let charges = self.outside()?.usage_due().await?;
        let (mut known, mut within, mut total) = (true, true, 0i64);
        for c in &charges {
            total += c.total_minor();
            match self
                .books
                .credit_position(&CustomerId::from(c.customer_id.as_str()))
                .await
            {
                Ok(position) => within &= position.allows(self.money(c.total_minor())),
                Err(_) => known = false,
            }
        }
        Ok(json!({
            "customers_known": known,
            "within_credit_limit": within,
            "charges": charges.len(),
            "total_usd": dollars(total),
        }))
    }

    async fn plan_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let Some(invoice) = args.get("invoice_id").and_then(Value::as_str) else {
            return Ok(json!({ "invoice_open": false }));
        };
        let Ok(invoice) = self.books.invoice(&InvoiceId::from(invoice)).await else {
            return Ok(json!({ "invoice_open": false }));
        };
        let instalments = self.parse_instalments(args).unwrap_or_default();
        let total: i64 = instalments.iter().map(|i| i.amount_minor).sum();
        let today = self.today();
        let days = instalments
            .iter()
            .map(|i| (i.due_date - today).num_days())
            .max()
            .unwrap_or(0);
        Ok(json!({
            "invoice_open": invoice.status.is_open() && invoice.balance_due.is_positive(),
            "sums_to_balance": !instalments.is_empty() && total == invoice.balance_due.minor,
            "instalment_count": instalments.len(),
            "plan_days": days.max(0),
        }))
    }

    async fn collect_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let plan = args
            .get("plan_id")
            .and_then(Value::as_str)
            .and_then(|p| self.plan(p));
        let n = args.get("instalment").and_then(Value::as_u64);
        let due = match (plan, n) {
            (Some(plan), Some(n)) => {
                let n = n as usize;
                plan.instalments
                    .get(n)
                    .is_some_and(|i| i.due_date <= self.today())
                    && plan.collected.get(n) == Some(&false)
            }
            _ => false,
        };
        Ok(json!({ "instalment_due": due }))
    }

    async fn reminder_facts(&self, args: &Value) -> anyhow::Result<Value> {
        let Some(invoice) = args.get("invoice_id").and_then(Value::as_str) else {
            return Ok(json!({ "invoice_overdue": false }));
        };
        let Ok(invoice) = self.books.invoice(&InvoiceId::from(invoice)).await else {
            return Ok(json!({ "invoice_overdue": false }));
        };
        Ok(json!({
            "invoice_overdue": invoice.status.is_open() && invoice.balance_due.is_positive() && invoice.due_date < self.today(),
        }))
    }
}

#[async_trait]
impl Facts for Desk {
    async fn facts(&self, tool: &str, args: &Value) -> Value {
        let facts = match tool {
            "pay_bill" => self.pay_facts(args).await,
            "settle_invoice" => self.settle_facts(args).await,
            "refund_payment" => self.refund_facts(args).await,
            "issue_invoice" => self.invoice_facts(args).await,
            "invoice_usage" => self.usage_facts().await,
            "propose_debt_plan" => self.plan_facts(args).await,
            "collect_instalment" => self.collect_facts(args).await,
            "send_reminder" => self.reminder_facts(args).await,
            "start_subscription" | "create_recurring_invoice" => {
                let plan = args
                    .get("plan_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Ok(json!({ "plan_listed": self.offer(plan).is_some() }))
            }
            "suspend_subscription" => {
                let sub = args
                    .get("subscription")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let failed = self
                    .failed_payments
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .iter()
                    .any(|s| s == sub);
                Ok(json!({ "payment_failed": failed }))
            }
            _ => Ok(json!({})),
        };
        // Facts that can't be read leave every fact-guarded rule unmatched; the money rules
        // that allow need facts, so an unreadable fact never lets money move.
        facts.unwrap_or_else(|e| json!({ "unreadable": e.to_string() }))
    }
}
