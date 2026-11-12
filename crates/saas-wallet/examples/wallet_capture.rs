//! A real PayPal sandbox capture into Agentrix's Cashier wallet and its books in Zavora ERA
//! (hosted), with the three-way match and a signed checkpoint reconciled against PayPal.
//!
//! ```text
//! deploy/saas/era/era.sh up
//! scripts/wcargo run -p saas-wallet --example wallet_capture -- vault 19.00 [vault_id]
//! scripts/wcargo run -p saas-wallet --example wallet_capture -- record <capture id>
//! scripts/wcargo run -p saas-wallet --example wallet_capture -- checkpoint
//! scripts/wcargo run -p saas-wallet --example wallet_capture -- feed
//! ```
//!
//! - `vault`: Agentrix's app captures the amount on Kilele's saved wallet (merchant-initiated
//!   Orders v2, no buyer step), then the wallet records it by reading it back and posts it to ERA.
//! - `record`: reads an existing capture back into the wallet (and links or posts it in ERA).
//! - `checkpoint`: reads every object back, records the reconciliation, signs a checkpoint.
//! - `feed`: prints the board's feed (`saas_core::wallet::WalletFeed`) as JSON.
//!
//! The wallet lives in `~/.config/agentrix-saas/wallet` (SQLite, key 0600). These runs are made
//! by an operator, so their authority says so. Sandbox only; no secret is printed.

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_effects::paypal::{self, PayPalClient, PayPalConfig};
use anyhow::{Context as _, bail};
use reqwest::Method;
use saas_books::{EraBooks, EraConfig};
use saas_core::books::Books;
use saas_core::paypal::PayPalRef;
use saas_core::wallet::{Authority, WalletCounterparty};
use saas_core::{AgentId, CompanyId, Currency, Money};
use saas_wallet::{Context, Wallets};
use serde_json::json;
use sha2::{Digest, Sha256};

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let keys = std::env::var_os("AGENTRIX_PAYPAL_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/agentrix-saas/paypal-sandbox.json"));
    let config = PayPalConfig::from_file(&keys)?.context("no PayPal sandbox keys")?;
    let client = Arc::new(PayPalClient::new(config, reqwest::Client::new()));
    if client.environment().is_live() {
        bail!("this example runs on PayPal's sandbox only");
    }
    let books: Arc<dyn Books> =
        Arc::new(EraBooks::connect(EraConfig::from_env(CompanyId::agentrix())?).await?);
    let dir = std::env::var_os("SAAS_WALLET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/agentrix-saas/wallet"));
    let wallets =
        Wallets::sqlite(CompanyId::agentrix(), &dir, client.clone(), books.clone()).await?;
    let cashier = AgentId::new(CompanyId::agentrix(), "cashier")?;
    let receivables = AgentId::new(CompanyId::agentrix(), AgentId::RECEIVABLES)?;

    let capture = match mode.as_str() {
        "vault" => {
            let amount = Money::parse_decimal(
                &args.next().unwrap_or_else(|| "19.00".into()),
                Currency::USD,
            )?;
            let vault_id = args
                .next()
                .or_else(|| std::env::var("SAAS_KILELE_VAULT_ID").ok())
                .unwrap_or_else(|| "5an08729le1870910".into());
            let tag = format!("{:x}", chrono::Utc::now().timestamp_millis());
            let body = json!({
                "intent": "CAPTURE",
                "purchase_units": [{
                    "reference_id": format!("wallet-{tag}"),
                    "invoice_id": format!("agentrix-saas/wallet/{tag}"),
                    "custom_id": format!("wallet-{tag}"),
                    "description": "Agentrix SaaS WebAgent plan (wallet run)",
                    "amount": paypal::amount(amount.currency.code(), amount.minor),
                }],
                "payment_source": {"paypal": {"vault_id": vault_id}},
            });
            let request_id = hex::encode(Sha256::digest(
                format!("agentrix-saas/wallet/{tag}").as_bytes(),
            ));
            let order = client
                .request(
                    Method::POST,
                    "/v2/checkout/orders",
                    Some(&body),
                    Some(&request_id),
                )
                .await?;
            let cap = &order["purchase_units"][0]["payments"]["captures"][0];
            let id = cap["id"].as_str().context("PayPal returned no capture")?;
            println!(
                "PayPal sandbox: order {} {}, capture {id} {}",
                order["id"].as_str().unwrap_or(""),
                order["status"].as_str().unwrap_or(""),
                cap["status"].as_str().unwrap_or("")
            );
            Some((id.to_owned(), request_id))
        }
        "record" => {
            let id = args.next().context("the capture id")?;
            let request_id = hex::encode(Sha256::digest(format!("record/{id}").as_bytes()));
            Some((id, request_id))
        }
        "checkpoint" => None,
        "feed" => {
            // The board's feed, as JSON, without reading PayPal again.
            println!(
                "{}",
                serde_json::to_string_pretty(&wallets.feed(&cashier).await?)?
            );
            return Ok(());
        }
        _ => bail!("modes: vault <amount> [vault_id] | record <capture id> | checkpoint | feed"),
    };

    if let Some((id, request_id)) = capture {
        let ctx = Context {
            acting: receivables.clone(),
            authority: Authority::Approval {
                fingerprint: format!("sha256:{request_id}"),
                approved_by: "operator (wallet_capture example)".into(),
            },
            cause: None,
            counterparty: Some(WalletCounterparty {
                name: "Kilele Bakery".into(),
                email: None,
                company: Some(CompanyId::kilele()),
            }),
        };
        let recorded = wallets
            .record(&cashier, &PayPalRef::Capture(id.parse()?), &ctx)
            .await?;
        println!("wallet {cashier} (already recorded: {}):", recorded.already);
        for e in &recorded.entries {
            println!(
                "  #{} {:?} {} {} [{}] from {:?}",
                e.seq, e.kind, e.amount, e.paypal, e.paypal_status, e.source
            );
        }
        let link = &recorded.books;
        if let Some(p) = &link.payment {
            let pay = books.payment(p).await?;
            println!(
                "  ERA: receipt {} ({p}), journal {:?}, fee journal {:?}",
                pay.number.unwrap_or_default(),
                link.journal,
                link.fee_journal
            );
        }
    }

    let cp = wallets.checkpoint(&cashier).await?;
    println!(
        "checkpoint: {} records in the signed log, root {}, wallet net {}",
        cp.size, cp.root_hash, cp.totals.net
    );
    println!("  {}", cp.summary().words);
    for d in &cp.reconciliation.drift {
        println!("  drift: {} {}", d.paypal, d.what);
    }
    for m in wallets.matches(&cashier).await? {
        println!(
            "  match #{} {:?} {} {} PayPal {:?} ERA journal {:?}: {:?}",
            m.seq, m.kind, m.amount, m.paypal, m.paypal_status, m.books.journal, m.state
        );
    }
    let report = wallets.verify(&cashier).await?;
    println!(
        "signed log verifies: {} ({} records, {} checkpoints)",
        report.is_valid(),
        report.records,
        report.checkpoints_verified
    );
    println!("verifier key: {}", wallets.verifier_key());
    Ok(())
}
