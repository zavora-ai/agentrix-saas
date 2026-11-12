//! One real PayPal sandbox capture, booked in both companies' books in Zavora ERA (hosted).
//!
//! ```text
//! deploy/saas/era/era.sh up && deploy/saas/era/era.sh seed
//! scripts/wcargo run -p saas-books --example book_paypal_capture -- [vault_id]
//! ```
//!
//! 1. **Agentrix's books:** an invoice to Kilele Bakery for the WebAgent Essential plan, monthly,
//!    posted.
//! 2. **Kilele's books:** the bill for that invoice, approved and posted.
//! 3. **PayPal sandbox,** through agentrix-effects' client on Agentrix's app: a merchant-initiated
//!    Orders v2 capture on Kilele's saved wallet (Vault) for the invoice total. PayPal completes
//!    it in one call; `PayPal-Request-Id` comes from the ERA invoice id, so a repeat is the same
//!    capture.
//! 4. **Both books post from the one capture id,** through saas-core's posting rules: Agentrix's
//!    receipt clears the invoice and books PayPal's fee; Kilele's payment clears the bill.
//!
//! Keys come from `AGENTRIX_PAYPAL_FILE` (default `~/.config/agentrix-saas/paypal-sandbox.json`);
//! anything but the sandbox is refused. The wallet is the first argument, else
//! `SAAS_KILELE_VAULT_ID`, else the one slice C saved (`5an08729le1870910`). Prints PayPal's and
//! ERA's ids, never a secret.

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_effects::paypal::{self, PayPalClient, PayPalConfig};
use anyhow::{Context, bail};
use chrono::NaiveDate;
use reqwest::Method;
use saas_books::{EraBooks, EraConfig};
use saas_core::books::*;
use saas_core::paypal::PayPalRef;
use saas_core::{Books, CompanyId, Counterparty, Currency, Money, PayPalPosting, post_paypal};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PLAN: &str = "WebAgent Essential plan, monthly";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let vault_id = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SAAS_KILELE_VAULT_ID").ok())
        .unwrap_or_else(|| "5an08729le1870910".into());
    let keys = std::env::var_os("AGENTRIX_PAYPAL_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".config/agentrix-saas/paypal-sandbox.json")
        });
    let config = PayPalConfig::from_file(&keys)?.context("no PayPal sandbox keys")?;
    let client = Arc::new(PayPalClient::new(config, reqwest::Client::new()));
    if client.environment().is_live() {
        bail!("this example runs on PayPal's sandbox only");
    }

    let agentrix = EraBooks::connect(EraConfig::from_env(CompanyId::agentrix())?).await?;
    let kilele = EraBooks::connect(EraConfig::from_env(CompanyId::kilele())?).await?;
    println!(
        "books: Agentrix via {}, Kilele via {}",
        agentrix.via(),
        kilele.via()
    );
    let issued: NaiveDate = "2026-11-10".parse()?;
    let due: NaiveDate = "2026-11-24".parse()?;

    // 1. Agentrix invoices Kilele.
    let customer = agentrix
        .ensure_customer(&Counterparty::usd("Kilele Bakery", None), None)
        .await?;
    let plan = agentrix
        .products()
        .await?
        .into_iter()
        .find(|p| p.name == PLAN)
        .context("seeded plan")?;
    let draft = InvoiceDraft {
        customer: customer.id.clone(),
        issue_date: issued,
        due_date: due,
        currency: Currency::USD,
        lines: vec![LineDraft {
            product: Some(plan.id.clone()),
            description: plan.name.clone(),
            quantity: 1,
            unit_price: plan.price,
        }],
        reference: None,
        memo: Some("Kilele Bakery's WebAgent plan".into()),
    };
    let invoice = agentrix
        .post_invoice(&agentrix.draft_invoice(&draft).await?.id)
        .await?;
    let number = invoice.number.clone().unwrap_or_default();
    println!(
        "Agentrix invoice {number} ({}) for {}: {:?}",
        invoice.id, invoice.total, invoice.status
    );

    // 2. Kilele records the bill.
    let vendor = kilele
        .ensure_vendor(&Counterparty::usd("Agentrix Ltd", None))
        .await?;
    let bill = kilele
        .draft_bill(&BillDraft {
            vendor: vendor.id.clone(),
            vendor_reference: Some(number.clone()),
            issue_date: issued,
            due_date: due,
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: PLAN.into(),
                quantity: 1,
                unit_price: invoice.total,
            }],
            memo: Some(format!("Agentrix Ltd invoice {number}")),
        })
        .await?;
    let bill = kilele.post_bill(&bill.id).await?;
    println!(
        "Kilele bill {} ({}) for {}: {:?}",
        bill.number.clone().unwrap_or_default(),
        bill.id,
        bill.total,
        bill.status
    );

    // 3. Agentrix's app captures on Kilele's saved wallet.
    let request_id =
        hex(Sha256::digest(format!("agentrix-saas/era/{}", invoice.id).as_bytes()).as_slice());
    let body = json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "reference_id": invoice.id.0,
            "invoice_id": format!("agentrix-saas/era/{number}/{}", &invoice.id.0[..8]),
            "custom_id": invoice.id.0,
            "description": format!("Agentrix SaaS {PLAN} ({number})"),
            "amount": paypal::amount(invoice.total.currency.code(), invoice.total.minor),
        }],
        "payment_source": {"paypal": {"vault_id": vault_id}},
    });
    let mut order = client
        .request(
            Method::POST,
            "/v2/checkout/orders",
            Some(&body),
            Some(&request_id),
        )
        .await?;
    if matches!(
        order["status"].as_str(),
        Some("APPROVED" | "CREATED" | "SAVED")
    ) {
        let id = order["id"].as_str().unwrap_or_default().to_owned();
        let capture_req = format!("{request_id}-capture");
        order = client
            .request(
                Method::POST,
                &format!("/v2/checkout/orders/{id}/capture"),
                None,
                Some(&capture_req),
            )
            .await?;
    }
    let capture = &order["purchase_units"][0]["payments"]["captures"][0];
    if order["status"].as_str() != Some("COMPLETED")
        || capture["status"].as_str() != Some("COMPLETED")
    {
        bail!(
            "PayPal answered order {} {}, capture {}",
            order["id"],
            order["status"],
            capture["status"]
        );
    }
    let breakdown = &capture["seller_receivable_breakdown"];
    let gross = amount(&breakdown["gross_amount"]).or_else(|_| amount(&capture["amount"]))?;
    let fee = amount(&breakdown["paypal_fee"]).unwrap_or(Money::zero(gross.currency));
    let capture_id = capture["id"].as_str().context("capture id")?.to_owned();
    let date: NaiveDate = capture["create_time"]
        .as_str()
        .and_then(|t| t.get(..10))
        .unwrap_or("2026-11-10")
        .parse()?;
    println!(
        "PayPal sandbox: order {} capture {capture_id} COMPLETED, gross {gross}, PayPal fee {fee}",
        order["id"].as_str().unwrap_or_default()
    );

    // 4. Both books post from the one capture id.
    let paypal_ref = PayPalRef::Capture(capture_id.parse()?);
    let received = post_paypal(
        &agentrix,
        &PayPalPosting::Received {
            paypal: paypal_ref.clone(),
            customer: customer.id.clone(),
            gross,
            fee,
            date,
            invoice: Some(invoice.id.clone()),
        },
    )
    .await?;
    let paid = post_paypal(
        &kilele,
        &PayPalPosting::PaidOut {
            paypal: paypal_ref.clone(),
            vendor: vendor.id.clone(),
            amount: gross,
            fee: Money::zero(gross.currency),
            date,
            bill: Some(bill.id.clone()),
        },
    )
    .await?;
    let invoice = agentrix.invoice(&invoice.id).await?;
    let bill = kilele.bill(&bill.id).await?;
    println!("posted from {paypal_ref}:");
    println!(
        "  Agentrix: receipt {:?}, fee journal {:?}, invoice {number} now {:?} (owes {})",
        received.payment, received.fee_journal, invoice.status, invoice.balance_due
    );
    println!(
        "  Kilele:   payment {:?}, bill now {:?} (owes {})",
        paid.payment, bill.status, bill.balance_due
    );
    for (name, books) in [("Agentrix", &agentrix), ("Kilele", &kilele)] {
        let clearing = books.account(AccountRole::PayPalClearing).await?;
        println!(
            "  {name} PayPal clearing (GL {}): {}",
            clearing.code,
            clearing.balance.unwrap_or(Money::zero(Currency::USD))
        );
    }
    let again = post_paypal(
        &agentrix,
        &PayPalPosting::Received {
            paypal: paypal_ref,
            customer: customer.id,
            gross,
            fee,
            date,
            invoice: Some(invoice.id),
        },
    )
    .await?;
    println!(
        "posting the same capture again: already posted = {}",
        again.already_posted
    );
    Ok(())
}

fn amount(v: &Value) -> anyhow::Result<Money> {
    let currency = Currency::new(v["currency_code"].as_str().context("currency_code")?)?;
    Ok(Money::parse_decimal(
        v["value"].as_str().context("value")?,
        currency,
    )?)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
