//! The feed from the node: saas-core's [`Books`], one per company (Zavora ERA on the node,
//! `FakeBooks` in tests), plus [`OffBooks`], what the books don't hold.
//!
//! **For `saas-node`.** Build a [`BooksFeed`] from each company's `Arc<dyn Books>` and one
//! [`OffBooks`] that reads the kernels (staff, mandates, decisions, agent spend, the audit
//! check) and slice C's settlement store (instalment plans, agent-to-agent settlements), then
//! hand it to [`crate::Face::with_feed`]. The books answer receivables, ageing, payables, PayPal
//! cash and MRR; nothing on the board or the consoles is typed in twice.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use saas_core::books::{
    AccountRole, DocumentFilter, DocumentRef, DocumentStatus, Frequency, PaymentFilter,
};
use saas_core::{Books, PayPalRef};

use crate::feed::{
    AGENTRIX, AgeingRow, AgentSpend, Audit, CashBalance, Company, DebtPlan, Decision, Feed,
    FeedSource, Mandate, MrrPoint, Payable, Receivable, Settlement, VERSION, bucket,
};

/// What the books don't hold, as of one day. Every list covers both companies.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Each company, its owner, its staff (with their states) and whether it is frozen.
    pub companies: Vec<Company>,
    /// The mandates owners signed (AP2-style intent mandates).
    pub mandates: Vec<Mandate>,
    /// Decisions waiting on an owner, and recent ones.
    pub decisions: Vec<Decision>,
    /// Instalment plans agreed between the companies' agents. `invoice_number` is the books'.
    pub debt_plans: Vec<DebtPlan>,
    /// Agent-to-agent settlements, newest first.
    pub settlements: Vec<Settlement>,
    /// What each agent spent, per day.
    pub agent_spend: Vec<AgentSpend>,
    /// The signed audit log's last offline check.
    pub audit: Audit,
    /// Cash on rails the books don't hold (M-Pesa); PayPal comes from the books.
    pub other_cash: Vec<CashBalance>,
}

/// The source `saas-node` implements from the kernels and the settlement store.
#[async_trait]
pub trait OffBooks: Send + Sync {
    /// The facts as of `as_of`.
    async fn facts(&self, as_of: NaiveDate) -> anyhow::Result<Facts>;
}

/// One company's books.
#[derive(Clone)]
pub struct CompanyBooks {
    /// `agentrix` or `kilele`.
    pub company: String,
    /// Its books.
    pub books: Arc<dyn Books>,
    /// `sandbox` or `live`: what the PayPal clearing balance stands for.
    pub paypal_mode: String,
}

/// The feed with its money rows read from the books.
pub struct BooksFeed {
    companies: Vec<CompanyBooks>,
    rest: Arc<dyn OffBooks>,
    source: String,
}

impl BooksFeed {
    /// Reads `companies`' books, and everything else from `rest`. `source` names the books
    /// in the feed (`"books"` for Zavora ERA, `"fakebooks"` for saas-core's in-memory books).
    pub fn new(companies: Vec<CompanyBooks>, rest: Arc<dyn OffBooks>, source: &str) -> Self {
        Self {
            companies,
            rest,
            source: source.to_owned(),
        }
    }
}

fn status_of(s: DocumentStatus) -> &'static str {
    match s {
        DocumentStatus::Paid => "paid",
        DocumentStatus::PartiallyPaid => "partial",
        _ => "open",
    }
}

/// MRR of one schedule: what it brings in a month.
fn monthly(amount: i64, f: &Frequency) -> i64 {
    match f {
        Frequency::Weekly => (amount * 52 + 6) / 12,
        Frequency::Monthly => amount,
        Frequency::Quarterly => (amount + 1) / 3,
        Frequency::Yearly => (amount + 6) / 12,
    }
}

fn paypal_text(r: PayPalRef) -> String {
    match r {
        PayPalRef::Capture(id) => id.to_string(),
        PayPalRef::SubscriptionSale(id) => id.to_string(),
        PayPalRef::Refund(id) => id.to_string(),
        PayPalRef::Payout(id) => id.to_string(),
    }
}

#[async_trait]
impl FeedSource for BooksFeed {
    async fn feed(&self, as_of: NaiveDate) -> anyhow::Result<Feed> {
        let facts = self.rest.facts(as_of).await?;
        let mut receivables = Vec::new();
        let mut ageing_rows = Vec::new();
        let mut payables = Vec::new();
        let mut cash = Vec::new();
        let mut mrr = Vec::new();

        for cb in &self.companies {
            let books = cb.books.as_ref();
            let company = cb.company.as_str();
            let customers = books.customers().await?;
            let vendors = books.vendors().await?;
            let payments = books.payments(&PaymentFilter::default()).await?;
            // The PayPal id that last paid a document, from the payment's reference.
            let paypal_for = |doc: &DocumentRef| {
                payments
                    .iter()
                    .rev()
                    .filter(|p| p.applied.iter().any(|(d, _)| d == doc))
                    .find_map(|p| p.reference.as_deref().and_then(PayPalRef::from_key))
                    .map(paypal_text)
            };

            for inv in books.invoices(&DocumentFilter::default()).await? {
                if matches!(inv.status, DocumentStatus::Draft | DocumentStatus::Void) {
                    continue;
                }
                let customer = customers
                    .iter()
                    .find(|c| c.id == inv.customer)
                    .map_or_else(|| inv.customer.0.clone(), |c| c.counterparty.name.clone());
                let number = inv.number.clone().unwrap_or_else(|| inv.id.0.clone());
                let open = inv.balance_due.minor;
                let (name, late) = bucket(inv.due_date, as_of);
                let on_plan = open > 0
                    && facts
                        .debt_plans
                        .iter()
                        .any(|p| p.creditor == company && p.invoice_number == number);
                let status = if on_plan {
                    "on_plan"
                } else {
                    status_of(inv.status)
                };
                receivables.push(Receivable {
                    company: company.to_owned(),
                    invoice_id: inv.id.0.clone(),
                    number,
                    customer,
                    description: inv
                        .lines
                        .first()
                        .map(|l| l.description.clone())
                        .unwrap_or_default(),
                    issued: inv.issue_date,
                    due: inv.due_date,
                    currency: inv.total.currency.code().to_owned(),
                    amount_minor: inv.total.minor,
                    open_minor: open,
                    days_overdue: if open > 0 { late } else { 0 },
                    bucket: if open > 0 { name } else { "paid" }.to_owned(),
                    status: status.to_owned(),
                    paypal_id: paypal_for(&DocumentRef::Invoice(inv.id.clone())),
                });
            }

            let report = books.ar_ageing(as_of).await?;
            for row in report.rows {
                let limit = match &row.party {
                    DocumentRef::Customer(id) => customers
                        .iter()
                        .find(|c| &c.id == id)
                        .and_then(|c| c.credit_limit)
                        .map(|m| m.minor),
                    _ => None,
                };
                let b = &row.buckets;
                let total = b.current.minor
                    + b.days_1_30.minor
                    + b.days_31_60.minor
                    + b.days_61_90.minor
                    + b.over_90.minor;
                if total == 0 {
                    continue;
                }
                ageing_rows.push(AgeingRow {
                    company: company.to_owned(),
                    customer: row.name,
                    currency: b.current.currency.code().to_owned(),
                    current_minor: b.current.minor,
                    d1_30_minor: b.days_1_30.minor,
                    d31_60_minor: b.days_31_60.minor,
                    d61_90_minor: b.days_61_90.minor,
                    over90_minor: b.over_90.minor,
                    total_minor: total,
                    credit_limit_minor: limit,
                });
            }

            // The per-payment limit of this company's Payables mandate: a bill above it waits
            // for the owner's passkey.
            let per_payment = facts
                .mandates
                .iter()
                .find(|m| m.company == company && m.agent == "payables")
                .map(|m| m.per_payment_minor);
            for bill in books.bills(&DocumentFilter::default()).await? {
                if matches!(bill.status, DocumentStatus::Draft | DocumentStatus::Void) {
                    continue;
                }
                let vendor = vendors
                    .iter()
                    .find(|v| v.id == bill.vendor)
                    .map_or_else(|| bill.vendor.0.clone(), |v| v.counterparty.name.clone());
                let number = bill
                    .vendor_reference
                    .clone()
                    .or_else(|| bill.number.clone())
                    .unwrap_or_else(|| bill.id.0.clone());
                let open = bill.balance_due.minor;
                let on_plan = open > 0
                    && facts
                        .debt_plans
                        .iter()
                        .any(|p| p.debtor == company && p.invoice_number == number);
                // An open decision about this bill means the owner was asked: it is held.
                let asked = open > 0
                    && facts.decisions.iter().any(|d| {
                        d.company == company && d.state == "open" && d.body.contains(&number)
                    });
                let over = asked || per_payment.is_some_and(|limit| open > limit);
                let status = if on_plan {
                    "on plan"
                } else if asked {
                    "held"
                } else {
                    status_of(bill.status)
                };
                payables.push(Payable {
                    company: company.to_owned(),
                    bill_id: bill.id.0.clone(),
                    number,
                    vendor,
                    description: String::new(),
                    due: bill.due_date,
                    currency: bill.total.currency.code().to_owned(),
                    amount_minor: bill.total.minor,
                    open_minor: open,
                    days_to_due: (bill.due_date - as_of).num_days(),
                    rail: "paypal_payouts".into(),
                    mandate: if over && !on_plan { "over" } else { "within" }.into(),
                    status: status.into(),
                });
            }

            let clearing = books.account(AccountRole::PayPalClearing).await?;
            if let Some(balance) = clearing.balance {
                cash.push(CashBalance {
                    company: company.to_owned(),
                    rail: "PayPal".into(),
                    account: clearing.name.clone(),
                    currency: balance.currency.code().to_owned(),
                    balance_minor: balance.minor,
                    mode: cb.paypal_mode.clone(),
                });
            }

            if company == AGENTRIX {
                let schedules: Vec<_> = books
                    .recurring_invoices()
                    .await?
                    .into_iter()
                    .filter(|r| r.active && r.end_date.is_none_or(|end| end >= as_of))
                    .collect();
                let mut payers: Vec<&str> =
                    schedules.iter().map(|r| r.customer.0.as_str()).collect();
                payers.sort_unstable();
                payers.dedup();
                mrr.push(MrrPoint {
                    month: format!("{:04}-{:02}", as_of.year(), as_of.month()),
                    currency: "USD".into(),
                    mrr_minor: schedules
                        .iter()
                        .map(|r| monthly(r.amount.minor, &r.frequency))
                        .sum(),
                    new_minor: 0,
                    churn_minor: 0,
                    customers: u32::try_from(payers.len()).unwrap_or(u32::MAX),
                });
            }
        }
        cash.extend(facts.other_cash);

        Ok(Feed {
            version: VERSION,
            as_of,
            source: self.source.clone(),
            companies: facts.companies,
            receivables,
            ageing: ageing_rows,
            payables,
            debt_plans: facts.debt_plans,
            cash,
            mrr,
            agent_spend: facts.agent_spend,
            mandates: facts.mandates,
            decisions: facts.decisions,
            settlements: facts.settlements,
            audit: facts.audit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_count_by_the_month() {
        assert_eq!(monthly(7_900, &Frequency::Monthly), 7_900);
        assert_eq!(monthly(75_840, &Frequency::Yearly), 6_320);
        assert_eq!(monthly(3_000, &Frequency::Quarterly), 1_000);
        assert_eq!(monthly(1_200, &Frequency::Weekly), 5_200);
    }
}
