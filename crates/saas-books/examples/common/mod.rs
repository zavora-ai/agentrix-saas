//! The PayPal sandbox side the examples share, through agentrix-effects' client on Agentrix SaaS's
//! app. Sandbox only; never prints a secret.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_effects::paypal::{self, PayPalClient, PayPalConfig};
use anyhow::{Context, bail};
use chrono::NaiveDate;
use reqwest::Method;
use saas_core::{Currency, Money};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Agentrix SaaS's sandbox app: `AGENTRIX_PAYPAL_FILE`, default
/// `~/.config/agentrix-saas/paypal-sandbox.json`. Refuses anything but the sandbox.
pub fn sandbox_client() -> anyhow::Result<Arc<PayPalClient>> {
    let keys = std::env::var_os("AGENTRIX_PAYPAL_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/agentrix-saas/paypal-sandbox.json"));
    let config = PayPalConfig::from_file(&keys)?.context("no PayPal sandbox keys")?;
    let client = Arc::new(PayPalClient::new(config, reqwest::Client::new()));
    if client.environment().is_live() {
        bail!("these examples run on PayPal's sandbox only");
    }
    Ok(client)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// The wallet Kilele saved for Agentrix's app: the first argument after the mode, else
/// `SAAS_KILELE_VAULT_ID`, else the one slice C saved.
pub fn kilele_vault(arg: Option<String>) -> String {
    arg.or_else(|| std::env::var("SAAS_KILELE_VAULT_ID").ok())
        .unwrap_or_else(|| "5an08729le1870910".into())
}

/// One value from the repo's `.env` (`SAAS_ENV_FILE` overrides), read line by line.
pub fn env_value(key: &str) -> Option<String> {
    let path = std::env::var_os("SAAS_ENV_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("Developer/hackathons/agentrix-saas/.env"));
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        let v = l
            .strip_prefix(key)?
            .strip_prefix('=')?
            .trim()
            .trim_matches(['"', '\'']);
        (!v.is_empty() && !v.starts_with('#')).then(|| v.to_owned())
    })
}

pub fn amount(v: &Value) -> anyhow::Result<Money> {
    let currency = Currency::new(v["currency_code"].as_str().context("currency_code")?)?;
    Ok(Money::parse_decimal(
        v["value"].as_str().context("value")?,
        currency,
    )?)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A completed capture, as the books need it.
pub struct Captured {
    pub order_id: String,
    pub capture_id: String,
    pub gross: Money,
    pub fee: Money,
    pub date: NaiveDate,
}

/// A merchant-initiated Orders v2 capture on a saved wallet: COMPLETED in one call, no buyer
/// step. `key` (the ERA invoice id) makes it idempotent: the same key is the same capture.
pub async fn vault_capture(
    client: &PayPalClient,
    vault_id: &str,
    key: &str,
    paypal_invoice_id: &str,
    description: &str,
    total: Money,
) -> anyhow::Result<Captured> {
    let request_id = hex(Sha256::digest(format!("agentrix-saas/era/{key}").as_bytes()).as_slice());
    let body = json!({
        "intent": "CAPTURE",
        "purchase_units": [{
            "reference_id": key,
            "invoice_id": paypal_invoice_id,
            "custom_id": key,
            "description": description.chars().take(127).collect::<String>(),
            "amount": paypal::amount(total.currency.code(), total.minor),
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
    Ok(Captured {
        order_id: order["id"].as_str().unwrap_or_default().to_owned(),
        capture_id: capture["id"].as_str().context("capture id")?.to_owned(),
        fee: amount(&breakdown["paypal_fee"]).unwrap_or(Money::zero(gross.currency)),
        gross,
        date: capture["create_time"]
            .as_str()
            .and_then(|t| t.get(..10))
            .unwrap_or("2026-11-10")
            .parse()?,
    })
}
