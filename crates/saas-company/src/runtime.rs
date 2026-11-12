//! One company, running: its kernel, its flows and the PayPal events that wake them.
//!
//! [`Runtime::on_paypal_event`] takes an event `agentrix-effects` has verified (PayPal's own
//! signature check) and:
//!
//! 1. posts it to the books with saas-core's posting rules, once per PayPal id: a capture is a
//!    receipt applied to its invoice, a confirmed payout item a bill payment;
//! 2. wakes the invoice-to-cash flow of the invoice it paid (the flow's thread is the invoice's
//!    reference in the books), which closes it and remembers how the customer paid;
//! 3. wakes the agent the event concerns, through the kernel (a failed subscription payment,
//!    a dispute, a payout that didn't go through).

use std::path::PathBuf;
use std::sync::Arc;

use adk_core::Clock;
use saas_core::books::{CustomerId, DocumentFilter, InvoiceId};
use saas_core::paypal::{PayPalCaptureId, PayPalPayoutItemId, PayPalSaleId};
use saas_core::{Currency, Money, PayPalPosting, PayPalRef, Posted, post_paypal};
use serde_json::Value;

use crate::company::Company;
use crate::flows::{FlowStatus, Flows};
use crate::stores::CompanyStores;
use crate::supervisor::Supervisor;

/// The sales sites take through Agentrix's own checkout. Their PayPal events reach Agentrix's
/// runtime, because the sites use Agentrix's PayPal app, but they belong to the selling site,
/// not to an invoice in Agentrix's books.
#[async_trait::async_trait]
pub trait SiteSales: Send + Sync {
    /// When `event` is about a capture or a refund of a site's sale, keeps it on that sale as
    /// PayPal's confirmation (once per event id) and says which sale; `None` otherwise.
    async fn confirm(&self, event: &Value) -> anyhow::Result<Option<String>>;
}

/// What one PayPal event did.
#[derive(Debug, Clone, Default)]
pub struct Handled {
    /// The site's sale it confirmed, when it was about one (nothing is posted then).
    pub site_sale: Option<String>,
    /// What the books posted (`already_posted` when an earlier delivery did).
    pub posted: Option<Posted>,
    /// The flow it woke, and where that flow stands now.
    pub flow: Option<(String, FlowStatus)>,
    /// The kernel process it started for an agent.
    pub process: Option<String>,
}

/// One company, running.
pub struct Runtime {
    /// The company.
    pub company: Arc<Company>,
    /// Its kernel.
    pub supervisor: Arc<Supervisor>,
    /// Its flows.
    pub flows: Arc<Flows>,
    /// The sites' own sales, when a node sets them ([`Runtime::set_site_sales`]).
    site_sales: std::sync::OnceLock<Arc<dyn SiteSales>>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("company", &self.company.config.slug)
            .finish_non_exhaustive()
    }
}

fn money(amount: &Value, value_key: &str, currency_key: &str) -> Option<Money> {
    let currency = Currency::new(amount[currency_key].as_str()?).ok()?;
    Money::parse_decimal(amount[value_key].as_str()?, currency).ok()
}

fn date_of(resource: &Value, today: chrono::NaiveDate) -> chrono::NaiveDate {
    ["create_time", "update_time", "time_processed"]
        .iter()
        .find_map(|k| resource[*k].as_str())
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.date_naive())
        .unwrap_or(today)
}

/// The books' reference for a PayPal subscription's recurring invoice.
pub fn subscription_reference(subscription: &str) -> String {
    format!("paypal-sub:{subscription}")
}

/// The invoice a capture names: our id in `custom_id`, or saas-settle's PayPal invoice id
/// (`agentrix-saas/<invoice>/<cart mandate>`), or the invoice id itself.
pub fn invoice_named(resource: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(custom) = resource["custom_id"].as_str() {
        out.push(custom.to_owned());
    }
    if let Some(id) = resource["invoice_id"].as_str() {
        match id.strip_prefix("agentrix-saas/") {
            Some(rest) => out.extend(rest.split('/').next().map(str::to_owned)),
            None => out.push(id.to_owned()),
        }
    }
    out
}

impl Runtime {
    /// Starts `company` on `stores`: its kernel (with the stores' runs and processes) and its
    /// flows (with the stores' checkpoints). The kernel keeps its own files under `home`.
    pub async fn start(
        company: Arc<Company>,
        stores: &CompanyStores,
        clock: Arc<dyn Clock>,
        home: PathBuf,
    ) -> anyhow::Result<Self> {
        let saved = company.desk.use_plan_book(stores.plans.clone()).await?;
        let supervisor = Supervisor::start_on(
            company.clone(),
            clock,
            home,
            stores.runs.clone(),
            stores.processes.clone(),
            stores.kernel.as_ref(),
        )
        .await?;
        // A restarted node wakes on every instalment still to collect (a repeated spawn finds
        // the process already sleeping).
        for plan in &saved {
            if let Err(e) = supervisor.schedule_plan(plan).await {
                tracing::error!(plan = %plan.plan_id, error = %e, "couldn't reschedule a plan");
            }
        }
        let flows = Arc::new(Flows::new(company.clone(), stores.checkpointer.clone())?);
        Ok(Self {
            company,
            supervisor,
            flows,
            site_sales: std::sync::OnceLock::new(),
        })
    }

    /// Tells this runtime which sales are sites' own (taken through its PayPal app): their
    /// capture and refund events are kept as those sales' confirmations and never posted to
    /// this company's books. The first call wins.
    pub fn set_site_sales(&self, sales: Arc<dyn SiteSales>) -> bool {
        self.site_sales.set(sales).is_ok()
    }

    async fn posting(&self, kind: &str, resource: &Value) -> anyhow::Result<Option<PayPalPosting>> {
        let books = &self.company.desk.books;
        let today = self.company.desk.today();
        match kind {
            "PAYMENT.CAPTURE.COMPLETED" => {
                let (Some(id), Some(gross)) = (
                    resource["id"].as_str(),
                    money(&resource["amount"], "value", "currency_code"),
                ) else {
                    return Ok(None);
                };
                let mut invoice = None;
                for named in invoice_named(resource) {
                    if let Ok(found) = books.invoice(&InvoiceId::from(named.as_str())).await {
                        invoice = Some(found);
                        break;
                    }
                    let by_reference = books
                        .invoices(&DocumentFilter {
                            party: None,
                            open_only: false,
                            reference: Some(named.clone()),
                        })
                        .await?;
                    if let Some(found) = by_reference.into_iter().next() {
                        invoice = Some(found);
                        break;
                    }
                }
                let Some(invoice) = invoice else {
                    tracing::warn!(capture = %id, "a capture names no invoice in the books; nothing posted");
                    return Ok(None);
                };
                let fee = money(
                    &resource["seller_receivable_breakdown"]["paypal_fee"],
                    "value",
                    "currency_code",
                )
                .unwrap_or(Money::zero(gross.currency));
                Ok(Some(PayPalPosting::Received {
                    paypal: PayPalRef::Capture(PayPalCaptureId::new(id)?),
                    customer: CustomerId::from(invoice.customer.as_str()),
                    gross,
                    fee,
                    date: date_of(resource, today),
                    invoice: Some(invoice.id),
                }))
            }
            "PAYMENT.SALE.COMPLETED" => {
                // A subscription's billing-cycle payment: its customer is the one whose
                // recurring invoice carries the subscription.
                let (Some(id), Some(sub), Some(gross)) = (
                    resource["id"].as_str(),
                    resource["billing_agreement_id"].as_str(),
                    money(&resource["amount"], "total", "currency"),
                ) else {
                    return Ok(None);
                };
                let reference = subscription_reference(sub);
                let Some(recurring) = books
                    .recurring_invoices()
                    .await?
                    .into_iter()
                    .find(|r| r.reference.as_deref() == Some(reference.as_str()))
                else {
                    tracing::warn!(sale = %id, subscription = %sub, "a subscription payment for no customer in the books; nothing posted");
                    return Ok(None);
                };
                let fee = money(&resource["transaction_fee"], "value", "currency")
                    .unwrap_or(Money::zero(gross.currency));
                Ok(Some(PayPalPosting::Received {
                    paypal: PayPalRef::SubscriptionSale(PayPalSaleId::new(id)?),
                    customer: recurring.customer,
                    gross,
                    fee,
                    date: date_of(resource, today),
                    invoice: None,
                }))
            }
            "PAYMENT.PAYOUTS-ITEM.SUCCEEDED" => {
                let item = &resource["payout_item"];
                let (Some(id), Some(receiver), Some(amount)) = (
                    resource["payout_item_id"].as_str(),
                    item["receiver"].as_str(),
                    money(&item["amount"], "value", "currency"),
                ) else {
                    return Ok(None);
                };
                let vendor = books.vendors().await?.into_iter().find(|v| {
                    v.counterparty
                        .paypal_email
                        .as_deref()
                        .is_some_and(|p| p.eq_ignore_ascii_case(receiver))
                });
                let Some(vendor) = vendor else {
                    tracing::warn!(item = %id, "a payout went to no vendor in the books; nothing posted");
                    return Ok(None);
                };
                let fee = money(&resource["payout_item_fee"], "value", "currency")
                    .unwrap_or(Money::zero(amount.currency));
                Ok(Some(PayPalPosting::PaidOut {
                    paypal: PayPalRef::Payout(PayPalPayoutItemId::new(id)?),
                    vendor: vendor.id,
                    amount,
                    fee,
                    date: date_of(resource, today),
                    bill: None,
                }))
            }
            _ => Ok(None),
        }
    }

    /// A subscription PayPal says is active: Revenue makes sure the customer is in the books
    /// and sets up the plan's recurring invoice, on the governed path, keyed by the
    /// subscription, so each billing cycle's payment posts against it.
    pub async fn subscription_activated(
        &self,
        subscription: &str,
        email: &str,
        name: &str,
        plan_id: &str,
        yearly: bool,
    ) -> anyhow::Result<crate::governed::CallOutcome> {
        let books = &self.company.desk.books;
        let customer = books
            .ensure_customer(&saas_core::Counterparty::usd(name, Some(email)), None)
            .await?;
        let revenue = self.company.member(crate::identity::REVENUE)?;
        let session = format!("subscription-{subscription}");
        let scope = adk_core::ApprovalScope::new(
            revenue.identity.app_name().as_str(),
            revenue.user(),
            &session,
        );
        self.company
            .governance
            .call(
                &self.company.desk,
                &revenue.identity,
                &scope,
                &session,
                "create_recurring_invoice",
                &format!("{session}/recurring"),
                serde_json::json!({
                    "customer_id": customer.id, "plan_id": plan_id,
                    "frequency": if yearly { "yearly" } else { "monthly" },
                    "reference": subscription_reference(subscription),
                }),
                None,
            )
            .await
    }

    /// Handles one verified PayPal event: posts it, wakes its flow, wakes its agent.
    pub async fn on_paypal_event(&self, event: &Value) -> anyhow::Result<Handled> {
        let kind = event["event_type"].as_str().unwrap_or_default();
        let resource = &event["resource"];
        let mut handled = Handled::default();
        // A site's own sale is confirmed on that sale, before any invoice is looked for.
        if matches!(kind, "PAYMENT.CAPTURE.COMPLETED" | "PAYMENT.CAPTURE.REFUNDED")
            && let Some(sales) = self.site_sales.get()
            && let Some(sale) = sales.confirm(event).await?
        {
            handled.site_sale = Some(sale);
            return Ok(handled);
        }
        if let Some(posting) = self.posting(kind, resource).await? {
            let posted = post_paypal(self.company.desk.books.as_ref(), &posting).await?;
            if let PayPalPosting::Received {
                invoice: Some(invoice),
                ..
            } = &posting
            {
                let invoice = self.company.desk.books.invoice(invoice).await?;
                if let Some(thread) = invoice.reference.filter(|r| r.starts_with("i2c-")) {
                    let status = self.flows.wake_thread(&thread, "paid").await?;
                    handled.flow = Some((thread, status));
                }
            }
            handled.posted = Some(posted);
        }
        handled.process = self.supervisor.paypal_event(event).await?;
        Ok(handled)
    }
}
