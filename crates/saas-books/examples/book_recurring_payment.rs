//! A recurring invoice in Agentrix's books (Zavora ERA, hosted), paid through PayPal's sandbox,
//! and the payment booked against the invoice that schedule raised.
//!
//! ```text
//! scripts/wcargo run -p saas-books --example book_recurring_payment -- vault [vault_id]
//! scripts/wcargo run -p saas-books --example book_recurring_payment -- subscribe ["Customer name"]
//! scripts/wcargo run -p saas-books --example book_recurring_payment -- subscription I-XXXXXXXXXXXX
//! ```
//!
//! - **`vault`** (no buyer step): Kilele Bakery's monthly WebAgent plan as an ERA recurring
//!   invoice. ERA's own scheduler raises this month's invoice (hourly, and once at boot: run
//!   `deploy/saas/era/era.sh restart` locally if it hasn't yet), Agentrix's app collects it with
//!   a merchant-initiated capture on Kilele's saved wallet, and the capture books against that
//!   invoice.
//! - **`subscribe`**: a real PayPal subscription to the WebAgent Essential plan (USD 29.00 a
//!   month), with an ERA recurring invoice carrying the subscription's reference. Prints the
//!   approval link: a sandbox buyer approves it, and PayPal takes the first month then.
//! - **`subscription <id>`**: reads the subscription's completed payments from PayPal and books
//!   each against the invoice its recurring schedule raised, once per PayPal sale id.
//!
//! Every PayPal object made here is named "Agentrix SaaS …". Sandbox only; no secret is printed.

mod common;

use anyhow::{Context, bail};
use chrono::{NaiveDate, Utc};
use reqwest::Method;
use saas_books::{EraBooks, EraConfig};
use saas_core::books::*;
use saas_core::paypal::{PayPalRef, PayPalSubscriptionId};
use saas_core::{
    Books, CompanyId, Counterparty, Currency, Money, PayPalPosting, post_paypal,
    recurring_invoice_for,
};
use serde_json::json;

const PLAN: &str = "WebAgent Essential plan, monthly";
const VAULT_REFERENCE: &str = "agentrix-saas/recurring/kilele-bakery/webagent-essential-monthly";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let books = EraBooks::connect(EraConfig::from_env(CompanyId::agentrix())?).await?;
    println!("Agentrix books via {}", books.via());
    let today = Utc::now().date_naive();
    match mode.as_str() {
        "vault" => vault(&books, common::kilele_vault(args.next()), today).await,
        "subscribe" => subscribe(&books, args.next(), today).await,
        "subscription" => {
            subscription(
                &books,
                args.next().context("the subscription id, I-…")?.parse()?,
                today,
            )
            .await
        }
        _ => bail!("modes: vault [vault_id] | subscribe [customer name] | subscription <I-…>"),
    }
}

async fn plan_line(books: &EraBooks) -> anyhow::Result<LineDraft> {
    let plan = books
        .products()
        .await?
        .into_iter()
        .find(|p| p.name == PLAN)
        .context("seeded plan (era.sh seed)")?;
    Ok(LineDraft {
        product: Some(plan.id),
        description: plan.name,
        quantity: 1,
        unit_price: plan.price,
    })
}

async fn schedule(
    books: &EraBooks,
    customer: &CustomerId,
    reference: &str,
    start: NaiveDate,
) -> anyhow::Result<RecurringInvoice> {
    let r = books
        .create_recurring_invoice(&RecurringInvoiceDraft {
            customer: customer.clone(),
            currency: Currency::USD,
            lines: vec![plan_line(books).await?],
            frequency: Frequency::Monthly,
            start_date: start,
            end_date: None,
            due_in_days: 0,
            reference: Some(reference.to_owned()),
        })
        .await?;
    println!(
        "ERA recurring invoice {} ({:?}, {} a cycle, next {:?}), reference {reference}",
        r.id, r.frequency, r.amount, r.next_date
    );
    Ok(r)
}

async fn vault(books: &EraBooks, vault_id: String, today: NaiveDate) -> anyhow::Result<()> {
    let client = common::sandbox_client()?;
    let kilele = books
        .ensure_customer(&Counterparty::usd("Kilele Bakery", None), None)
        .await?;
    schedule(books, &kilele.id, VAULT_REFERENCE, today).await?;
    let Some(invoice) = recurring_invoice_for(books, VAULT_REFERENCE, today).await? else {
        println!(
            "ERA's scheduler hasn't raised this cycle's invoice yet. It runs hourly and at boot: run"
        );
        println!("`deploy/saas/era/era.sh restart` (locally) and this again.");
        std::process::exit(2);
    };
    let invoice = books.invoice(&invoice).await?;
    let number = invoice.number.clone().unwrap_or_default();
    println!(
        "this cycle: invoice {number} ({}) issued {}, {:?}, owes {}",
        invoice.id, invoice.issue_date, invoice.status, invoice.balance_due
    );
    let captured = common::vault_capture(
        &client,
        &vault_id,
        &invoice.id.0,
        &format!("agentrix-saas/era/{number}/{}", &invoice.id.0[..8]),
        &format!("Agentrix SaaS {PLAN} ({number})"),
        invoice.balance_due,
    )
    .await?;
    println!(
        "PayPal sandbox: order {} capture {} COMPLETED, gross {}, PayPal fee {}",
        captured.order_id, captured.capture_id, captured.gross, captured.fee
    );
    let posted = post_paypal(
        books,
        &PayPalPosting::Received {
            paypal: PayPalRef::Capture(captured.capture_id.parse()?),
            customer: kilele.id.clone(),
            gross: captured.gross,
            fee: captured.fee,
            date: captured.date,
            invoice: Some(invoice.id.clone()),
        },
    )
    .await?;
    report(books, &posted, &invoice.id).await
}

async fn subscribe(books: &EraBooks, name: Option<String>, today: NaiveDate) -> anyhow::Result<()> {
    let client = common::sandbox_client()?;
    let name = name.unwrap_or_else(|| "Wanjiru Designs".into());
    let email = common::env_value("PAYPAL_BUYER_EMAIL_1");
    let customer = books
        .ensure_customer(&Counterparty::usd(name.clone(), email.as_deref()), None)
        .await?;

    let product = client
        .request(
            Method::POST,
            "/v1/catalogs/products",
            Some(&json!({"name": "Agentrix SaaS WebAgent Essential", "type": "SERVICE", "category": "SOFTWARE"})),
            Some("agentrix-saas-product-webagent-essential-v1"),
        )
        .await?;
    let product_id = product["id"].as_str().context("product id")?;
    let plan = client
        .request(
            Method::POST,
            "/v1/billing/plans",
            Some(&json!({
                "product_id": product_id,
                "name": "Agentrix SaaS WebAgent Essential plan, monthly",
                "billing_cycles": [{
                    "frequency": {"interval_unit": "MONTH", "interval_count": 1},
                    "tenure_type": "REGULAR",
                    "sequence": 1,
                    "total_cycles": 0,
                    "pricing_scheme": {"fixed_price": {"value": "29.00", "currency_code": "USD"}},
                }],
                "payment_preferences": {"auto_bill_outstanding": true, "payment_failure_threshold": 1},
            })),
            Some("agentrix-saas-plan-webagent-essential-monthly-v1"),
        )
        .await?;
    let plan_id = plan["id"].as_str().context("plan id")?;
    let request = format!(
        "agentrix-saas-subscription-{}-{}",
        customer.id,
        Utc::now().timestamp()
    );
    let sub = client
        .request(
            Method::POST,
            "/v1/billing/subscriptions",
            Some(&json!({
                "plan_id": plan_id,
                "custom_id": customer.id.0,
                "application_context": {
                    "brand_name": "Agentrix",
                    "user_action": "SUBSCRIBE_NOW",
                    "shipping_preference": "NO_SHIPPING",
                    "return_url": "https://money.agentrix.website/plans",
                    "cancel_url": "https://money.agentrix.website/plans",
                },
            })),
            Some(&request),
        )
        .await?;
    let sub_id: PayPalSubscriptionId = sub["id"].as_str().context("subscription id")?.parse()?;
    let approve = sub["links"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|l| l["rel"] == "approve")
        .and_then(|l| l["href"].as_str())
        .unwrap_or("");
    println!(
        "PayPal sandbox: product {product_id}, plan {plan_id}, subscription {sub_id} {}",
        sub["status"].as_str().unwrap_or("")
    );
    schedule(books, &customer.id, &sub_id.reference(), today).await?;
    println!(
        "ERA customer {} ({})",
        customer.counterparty.name, customer.id
    );
    println!("approve as a sandbox buyer: {approve}");
    println!(
        "then: scripts/wcargo run -p saas-books --example book_recurring_payment -- subscription {sub_id}"
    );
    Ok(())
}

async fn subscription(
    books: &EraBooks,
    sub_id: PayPalSubscriptionId,
    today: NaiveDate,
) -> anyhow::Result<()> {
    let client = common::sandbox_client()?;
    let sub = client
        .request(
            Method::GET,
            &format!("/v1/billing/subscriptions/{sub_id}"),
            None,
            None,
        )
        .await?;
    println!(
        "PayPal sandbox: subscription {sub_id} {}",
        sub["status"].as_str().unwrap_or("?")
    );
    if matches!(
        sub["status"].as_str(),
        Some("APPROVAL_PENDING" | "APPROVED")
    ) {
        let approve = sub["links"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["rel"] == "approve")
            .and_then(|l| l["href"].as_str());
        println!(
            "not active yet, so no payment: a sandbox buyer approves it here, and PayPal takes the first month then:"
        );
        println!("{}", approve.unwrap_or("(PayPal gave no approval link)"));
        return Ok(());
    }
    let reference = sub_id.reference();
    let schedules = books.recurring_invoices().await?;
    let customer = match schedules
        .iter()
        .find(|r| r.reference.as_deref() == Some(reference.as_str()))
    {
        Some(r) => r.customer.clone(),
        None => {
            // A subscription started elsewhere (the sales desk): its subscriber becomes the customer.
            let s = &sub["subscriber"];
            let name = format!(
                "{} {}",
                s["name"]["given_name"].as_str().unwrap_or(""),
                s["name"]["surname"].as_str().unwrap_or("")
            );
            let who = Counterparty::usd(name.trim(), s["email_address"].as_str());
            let c = books.ensure_customer(&who, None).await?;
            let start = sub["start_time"]
                .as_str()
                .and_then(|t| t.get(..10))
                .map(str::parse)
                .transpose()?
                .unwrap_or(today);
            schedule(books, &c.id, &reference, start).await?;
            c.id
        }
    };
    let since = sub["start_time"]
        .as_str()
        .and_then(|t| t.get(..10))
        .unwrap_or("2026-11-01");
    let window = format!("start_time={since}T00:00:00Z&end_time={}T23:59:59Z", today);
    let tx = client
        .request(
            Method::GET,
            &format!("/v1/billing/subscriptions/{sub_id}/transactions?{window}"),
            None,
            None,
        )
        .await?;
    let mut booked = 0;
    for t in tx["transactions"].as_array().into_iter().flatten() {
        if t["status"].as_str() != Some("COMPLETED") {
            continue;
        }
        let sale = t["id"].as_str().context("sale id")?;
        let gross = common::amount(&t["amount_with_breakdown"]["gross_amount"])?;
        let fee = common::amount(&t["amount_with_breakdown"]["fee_amount"])
            .unwrap_or(Money::zero(gross.currency));
        let date: NaiveDate = t["time"]
            .as_str()
            .and_then(|x| x.get(..10))
            .context("time")?
            .parse()?;
        let invoice = recurring_invoice_for(books, &reference, date).await?;
        println!(
            "sale {sale} on {date}: gross {gross}, PayPal fee {fee}, pays invoice {invoice:?}"
        );
        let posted = post_paypal(
            books,
            &PayPalPosting::Received {
                paypal: PayPalRef::SubscriptionSale(sale.parse()?),
                customer: customer.clone(),
                gross,
                fee,
                date,
                invoice: invoice.clone(),
            },
        )
        .await?;
        match &invoice {
            Some(id) => report(books, &posted, id).await?,
            None => println!(
                "  no open invoice of this subscription yet: receipt {:?} waits unapplied on the customer",
                posted.payment
            ),
        }
        booked += 1;
    }
    if booked == 0 {
        println!(
            "no completed payment yet: approve the subscription as a sandbox buyer first (PayPal takes the first month then)"
        );
    }
    Ok(())
}

async fn report(
    books: &EraBooks,
    posted: &saas_core::Posted,
    invoice: &InvoiceId,
) -> anyhow::Result<()> {
    let inv = books.invoice(invoice).await?;
    println!("posted from {}:", posted.paypal);
    println!(
        "  receipt {:?}, fee journal {:?}, already posted {}",
        posted.payment, posted.fee_journal, posted.already_posted
    );
    println!(
        "  invoice {} ({}) now {:?}, owes {}",
        inv.number.unwrap_or_default(),
        inv.id,
        inv.status,
        inv.balance_due
    );
    Ok(())
}
