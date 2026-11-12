//! The story, posted into saas-core's `FakeBooks` through the `Books` contract, one book per
//! company: the same calls slice A's Zavora ERA books answer on the node.
//!
//! - Every customer invoice is drafted and posted; money received posts with saas-core's PayPal
//!   posting rules (`PayPalPosting::Received`), so it lands in PayPal clearing and clears the
//!   receivable, as a verified PayPal webhook would.
//! - Every bill is drafted and posted; money paid posts as `PayPalPosting::PaidOut`.
//! - Kilele records each Agentrix invoice as a vendor bill (as slice C's payer does), paid by
//!   the *same* PayPal ids Agentrix received: both ledgers post from one PayPal transaction.
//! - Agentrix's subscriptions are recurring invoices, so MRR comes from the books too.
//!
//! The books number their own invoices; [`Seeded::numbers`] says which number each of the
//! story's invoices got, so the story's plans and settlements name the books' numbers.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
use chrono::{Days, NaiveDate};
use saas_core::books::{BillDraft, Frequency, InvoiceDraft, LineDraft, RecurringInvoiceDraft};
use saas_core::fake::FakeBooks;
use saas_core::paypal::{PayPalCaptureId, PayPalPayoutItemId};
use saas_core::{
    Books, CompanyId, Counterparty, Currency, Money, PayPalPosting, PayPalRef, post_paypal,
};

use crate::demo::{Numbers, story};
use crate::feed::{AGENTRIX, KILELE};

/// Both companies' books, seeded with the story.
pub struct Seeded {
    /// Agentrix's books.
    pub agentrix: Arc<FakeBooks>,
    /// Kilele Bakery's books.
    pub kilele: Arc<FakeBooks>,
    /// The books' numbers for the story's invoices.
    pub numbers: Numbers,
}

fn before(d: NaiveDate, days: u64) -> NaiveDate {
    d.checked_sub_days(Days::new(days)).unwrap_or(d)
}

fn line(description: &str, cents: i64) -> Vec<LineDraft> {
    vec![LineDraft {
        product: None,
        description: description.to_owned(),
        quantity: 1,
        unit_price: Money::usd(cents),
    }]
}

/// An Agentrix invoice Kilele owes, for Kilele's own bill.
struct Owed {
    /// Agentrix's invoice number in the books.
    number: String,
    description: String,
    issued: NaiveDate,
    due: NaiveDate,
    total: i64,
    /// When, by which PayPal capture id, and how much was paid.
    payment: Option<(NaiveDate, String, i64)>,
}

/// A demo PayPal id for money the story moved without naming one.
fn demo_id(prefix: &str, number: &str) -> String {
    let tail: String = number
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("DEMO-{prefix}{tail}")
}

/// Posts the story at `as_of` into a fresh pair of books.
pub async fn seed(as_of: NaiveDate) -> anyhow::Result<Seeded> {
    let story = story(as_of);
    let agentrix = Arc::new(FakeBooks::new(CompanyId::agentrix()));
    let kilele = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let books_of = |company: &str| -> &Arc<FakeBooks> {
        if company == KILELE {
            &kilele
        } else {
            &agentrix
        }
    };
    let defaults = Numbers::default();

    // When and through which PayPal id each paid invoice was paid.
    let plan_paid: Vec<(NaiveDate, String)> = story
        .debt_plans
        .iter()
        .flat_map(|p| p.instalments.iter())
        .filter(|i| i.status == "paid")
        .filter_map(|i| i.paypal_id.clone().map(|id| (i.due, id)))
        .collect();

    let mut receivables = story.receivables.clone();
    receivables.sort_by(|a, b| a.issued.cmp(&b.issued).then(a.number.cmp(&b.number)));
    let mut numbers: HashMap<String, String> = HashMap::new();
    let mut kilele_owes: Vec<Owed> = Vec::new();

    for r in &receivables {
        let books = books_of(&r.company);
        let limit = story
            .ageing
            .iter()
            .find(|a| a.company == r.company && a.customer == r.customer)
            .and_then(|a| a.credit_limit_minor)
            .map(Money::usd);
        let mut who = Counterparty::usd(&r.customer, None);
        if r.customer == "Kilele Bakery" {
            who.company = Some(CompanyId::kilele());
        }
        let customer = books
            .ensure_customer(&who, limit)
            .await
            .context("a customer")?;
        let draft = books
            .draft_invoice(&InvoiceDraft {
                customer: customer.id.clone(),
                issue_date: r.issued,
                due_date: r.due,
                currency: Currency::USD,
                lines: line(&r.description, r.amount_minor),
                reference: Some(format!("story:{}", r.number)),
                memo: None,
            })
            .await
            .context("drafting an invoice")?;
        let inv = books
            .post_invoice(&draft.id)
            .await
            .context("posting an invoice")?;
        let number = inv.number.clone().unwrap_or_else(|| inv.id.0.clone());
        numbers.insert(r.number.clone(), number.clone());

        let paid = r.amount_minor - r.open_minor;
        let mut payment = None;
        if paid > 0 {
            let (date, capture) = match (&r.paypal_id, r.status.as_str()) {
                (Some(id), _) => (r.due, id.clone()),
                (None, _) => plan_paid
                    .first()
                    .cloned()
                    .unwrap_or_else(|| (r.due, demo_id("C", &r.number))),
            };
            post_paypal(
                books.as_ref(),
                &PayPalPosting::Received {
                    paypal: PayPalRef::Capture(PayPalCaptureId::new(capture.clone())?),
                    customer: customer.id.clone(),
                    gross: Money::usd(paid),
                    fee: Money::usd(0),
                    date,
                    invoice: Some(inv.id.clone()),
                },
            )
            .await
            .context("posting a PayPal receipt")?;
            payment = Some((date, capture, paid));
        }
        if r.company == AGENTRIX && r.customer == "Kilele Bakery" {
            kilele_owes.push(Owed {
                number,
                description: r.description.clone(),
                issued: r.issued,
                due: r.due,
                total: r.amount_minor,
                payment,
            });
        }
    }

    // Kilele's bills from Agentrix: the whole invoice, paid by the same PayPal ids.
    let agentrix_vendor = {
        let mut who = Counterparty::usd("Agentrix", None);
        who.company = Some(CompanyId::agentrix());
        kilele
            .ensure_vendor(&who)
            .await
            .context("Agentrix as Kilele's vendor")?
    };
    for Owed {
        number,
        description,
        issued,
        due,
        total,
        payment,
    } in &kilele_owes
    {
        let bill = kilele
            .draft_bill(&BillDraft {
                vendor: agentrix_vendor.id.clone(),
                vendor_reference: Some(number.clone()),
                issue_date: *issued,
                due_date: *due,
                currency: Currency::USD,
                lines: line(description, *total),
                memo: None,
            })
            .await
            .context("Kilele's bill")?;
        let bill = kilele
            .post_bill(&bill.id)
            .await
            .context("posting Kilele's bill")?;
        if let Some((date, capture, paid)) = payment {
            post_paypal(
                kilele.as_ref(),
                &PayPalPosting::PaidOut {
                    paypal: PayPalRef::Capture(PayPalCaptureId::new(capture.clone())?),
                    vendor: agentrix_vendor.id.clone(),
                    amount: Money::usd(*paid),
                    fee: Money::usd(0),
                    date: *date,
                    bill: Some(bill.id.clone()),
                },
            )
            .await
            .context("Kilele paying Agentrix")?;
        }
    }

    // Every other bill, as the story has it.
    for p in story
        .payables
        .iter()
        .filter(|p| !(p.company == KILELE && p.vendor == "Agentrix"))
    {
        let books = books_of(&p.company);
        let vendor = books
            .ensure_vendor(&Counterparty::usd(&p.vendor, None))
            .await
            .context("a vendor")?;
        let bill = books
            .draft_bill(&BillDraft {
                vendor: vendor.id.clone(),
                vendor_reference: Some(p.number.clone()),
                issue_date: before(p.due, 30),
                due_date: p.due,
                currency: Currency::USD,
                lines: line(&p.description, p.amount_minor),
                memo: None,
            })
            .await
            .context("drafting a bill")?;
        let bill = books.post_bill(&bill.id).await.context("posting a bill")?;
        let paid = p.amount_minor - p.open_minor;
        if paid > 0 {
            post_paypal(
                books.as_ref(),
                &PayPalPosting::PaidOut {
                    paypal: PayPalRef::Payout(PayPalPayoutItemId::new(demo_id("P", &p.number))?),
                    vendor: vendor.id.clone(),
                    amount: Money::usd(paid),
                    fee: Money::usd(0),
                    date: p.due,
                    bill: Some(bill.id.clone()),
                },
            )
            .await
            .context("paying a bill")?;
        }
    }

    // Agentrix's subscriptions: each customer's plan, from its latest invoice.
    let mut subscribed: Vec<(&str, &str, i64)> = Vec::new();
    for r in receivables.iter().rev().filter(|r| r.company == AGENTRIX) {
        if subscribed.iter().any(|(c, _, _)| *c == r.customer) {
            continue;
        }
        subscribed.push((r.customer.as_str(), r.description.as_str(), r.amount_minor));
    }
    for (customer_name, description, cents) in subscribed {
        let customer = agentrix
            .customers()
            .await?
            .into_iter()
            .find(|c| c.counterparty.name == customer_name)
            .context("a subscribed customer")?;
        let yearly = description.contains("annual");
        agentrix
            .create_recurring_invoice(&RecurringInvoiceDraft {
                customer: customer.id,
                currency: Currency::USD,
                lines: line(description.split(',').next().unwrap_or(description), cents),
                frequency: if yearly {
                    Frequency::Yearly
                } else {
                    Frequency::Monthly
                },
                start_date: as_of,
                end_date: None,
                due_in_days: 30,
                reference: Some(format!("story:subscription:{customer_name}")),
            })
            .await
            .context("a subscription")?;
    }

    let renamed = |story_number: &str| {
        numbers
            .get(story_number)
            .cloned()
            .unwrap_or_else(|| story_number.to_owned())
    };
    Ok(Seeded {
        agentrix,
        kilele,
        numbers: Numbers {
            kilele_annual: renamed(&defaults.kilele_annual),
            kilele_october: renamed(&defaults.kilele_october),
        },
    })
}
