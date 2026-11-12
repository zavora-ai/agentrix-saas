//! What PayPal itself says about one object: the only source of a wallet entry's facts.
//!
//! Every read here is a GET on PayPal's API through agentrix-effects' client, and the webhook
//! check is PayPal's own `verify-webhook-signature`. Nothing a request or a model sends is
//! taken as a fact; a webhook only names the object to read back.

use agentrix_effects::paypal::PayPalClient;
use chrono::{DateTime, Utc};
use reqwest::Method;
use reqwest::header::HeaderMap;
use saas_core::paypal::{PayPalRef, PayPalWebhookEventId};
use saas_core::wallet::{WalletCause, WalletCounterparty, WalletEntryKind};
use saas_core::{Currency, Money};
use serde_json::{Value, json};

use crate::WalletError;

/// One PayPal object, as read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// The object.
    pub paypal: PayPalRef,
    /// What it is.
    pub kind: WalletEntryKind,
    /// PayPal's status, for example `COMPLETED`.
    pub status: String,
    /// The amount that moved, as a positive amount.
    pub gross: Money,
    /// PayPal's fee on it (charged), or for a refund the fee PayPal gave back; positive or zero.
    pub fee: Money,
    /// When PayPal says it happened.
    pub at: DateTime<Utc>,
    /// Who, as PayPal names them.
    pub counterparty: Option<WalletCounterparty>,
    /// What caused it, from the fields we set on the PayPal object (the order, its invoice id,
    /// the subscription).
    pub cause: Option<WalletCause>,
    /// The call it was read with.
    pub call: String,
}

impl Observed {
    /// Whether PayPal counts this money as moved.
    pub fn completed(&self) -> bool {
        matches!(
            self.status.as_str(),
            "COMPLETED" | "completed" | "SUCCESS" | "PARTIALLY_REFUNDED" | "REFUNDED"
        )
    }
}

/// A webhook PayPal's own check accepted. Only [`verify_webhook`] makes one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEvent {
    /// The event.
    pub id: PayPalWebhookEventId,
    /// Its type, for example `PAYMENT.CAPTURE.COMPLETED`.
    pub event_type: String,
    /// The object to read back, when the event is one a wallet records.
    pub object: Option<PayPalRef>,
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// Asks PayPal whether a webhook is genuine (`/v1/notifications/verify-webhook-signature`, with
/// our webhook id), and names the object it is about.
pub async fn verify_webhook(
    client: &PayPalClient,
    headers: &HeaderMap,
    event: &Value,
) -> Result<VerifiedEvent, WalletError> {
    let webhook_id = client
        .webhook_id()
        .ok_or(WalletError::NotVerified)?
        .to_owned();
    let check = json!({
        "auth_algo": header(headers, "paypal-auth-algo"),
        "cert_url": header(headers, "paypal-cert-url"),
        "transmission_id": header(headers, "paypal-transmission-id"),
        "transmission_sig": header(headers, "paypal-transmission-sig"),
        "transmission_time": header(headers, "paypal-transmission-time"),
        "webhook_id": webhook_id,
        "webhook_event": event,
    });
    let verdict = client
        .request(
            Method::POST,
            "/v1/notifications/verify-webhook-signature",
            Some(&check),
            None,
        )
        .await
        .map_err(|e| WalletError::PayPal(e.to_string()))?;
    if verdict["verification_status"].as_str() != Some("SUCCESS") {
        return Err(WalletError::NotVerified);
    }
    let id: PayPalWebhookEventId = event["id"]
        .as_str()
        .unwrap_or_default()
        .parse()
        .map_err(|_| WalletError::NotVerified)?;
    let event_type = event["event_type"].as_str().unwrap_or_default().to_owned();
    let r = &event["resource"];
    let object = match event_type.as_str() {
        "PAYMENT.CAPTURE.COMPLETED" => r["id"]
            .as_str()
            .and_then(|i| i.parse().ok())
            .map(PayPalRef::Capture),
        "PAYMENT.CAPTURE.REFUNDED" | "PAYMENT.REFUND.COMPLETED" => r["id"]
            .as_str()
            .and_then(|i| i.parse().ok())
            .map(PayPalRef::Refund),
        "PAYMENT.SALE.COMPLETED" => r["id"]
            .as_str()
            .and_then(|i| i.parse().ok())
            .map(PayPalRef::SubscriptionSale),
        "PAYMENT.PAYOUTS-ITEM.SUCCEEDED" => r["payout_item_id"]
            .as_str()
            .and_then(|i| i.parse().ok())
            .map(PayPalRef::Payout),
        _ => None,
    };
    Ok(VerifiedEvent {
        id,
        event_type,
        object,
    })
}

fn money(v: &Value) -> Result<Money, WalletError> {
    let code = v["currency_code"]
        .as_str()
        .or(v["currency"].as_str())
        .ok_or_else(|| WalletError::PayPal(format!("no currency in {v}")))?;
    let value = v["value"]
        .as_str()
        .or(v["total"].as_str())
        .ok_or_else(|| WalletError::PayPal(format!("no amount in {v}")))?;
    let currency = Currency::new(code).map_err(|e| WalletError::PayPal(e.to_string()))?;
    Money::parse_decimal(value, currency).map_err(|e| WalletError::PayPal(e.to_string()))
}

fn money_or_zero(v: &Value, currency: Currency) -> Result<Money, WalletError> {
    if v.is_null() {
        Ok(Money::zero(currency))
    } else {
        money(v)
    }
}

fn time(v: &Value) -> Result<DateTime<Utc>, WalletError> {
    v.as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| WalletError::PayPal(format!("no time in {v}")))
}

fn link(v: &Value, rel: &str) -> Option<String> {
    v["links"]
        .as_array()?
        .iter()
        .find(|l| l["rel"].as_str() == Some(rel))
        .and_then(|l| l["href"].as_str())
        .map(|h| h.rsplit('/').next().unwrap_or(h).to_owned())
}

async fn get(client: &PayPalClient, path: &str) -> Result<Value, WalletError> {
    client
        .request(Method::GET, path, None, None)
        .await
        .map_err(|e| WalletError::PayPal(e.to_string()))
}

/// Reads one object back from PayPal.
pub async fn read_back(client: &PayPalClient, object: &PayPalRef) -> Result<Observed, WalletError> {
    match object {
        PayPalRef::Capture(id) => {
            let call = format!("/v2/payments/captures/{id}");
            let c = get(client, &call).await?;
            let b = &c["seller_receivable_breakdown"];
            let gross = if b["gross_amount"].is_null() {
                money(&c["amount"])?
            } else {
                money(&b["gross_amount"])?
            };
            let order = link(&c, "up");
            // The payer is on the order, not the capture.
            let counterparty = match &order {
                Some(o) => {
                    let ord = get(client, &format!("/v2/checkout/orders/{o}"))
                        .await
                        .unwrap_or(Value::Null);
                    let p = &ord["payer"];
                    let name = format!(
                        "{} {}",
                        p["name"]["given_name"].as_str().unwrap_or(""),
                        p["name"]["surname"].as_str().unwrap_or("")
                    );
                    let email = p["email_address"].as_str().map(str::to_owned);
                    (!name.trim().is_empty() || email.is_some()).then(|| WalletCounterparty {
                        name: if name.trim().is_empty() {
                            email.clone().unwrap_or_default()
                        } else {
                            name.trim().to_owned()
                        },
                        email,
                        company: None,
                    })
                }
                None => None,
            };
            let cause = order
                .and_then(|o| o.parse().ok())
                .map(|order| WalletCause::Order {
                    order,
                    reference: c["invoice_id"]
                        .as_str()
                        .or(c["custom_id"].as_str())
                        .map(str::to_owned),
                });
            Ok(Observed {
                paypal: object.clone(),
                kind: WalletEntryKind::Capture,
                status: c["status"].as_str().unwrap_or_default().to_owned(),
                fee: money_or_zero(&b["paypal_fee"], gross.currency)?,
                gross,
                at: time(&c["create_time"])?,
                counterparty,
                cause,
                call: format!("GET {call}"),
            })
        }
        PayPalRef::Refund(id) => {
            let call = format!("/v2/payments/refunds/{id}");
            let r = get(client, &call).await?;
            let gross = money(&r["amount"])?;
            let b = &r["seller_payable_breakdown"];
            Ok(Observed {
                paypal: object.clone(),
                kind: WalletEntryKind::Refund,
                status: r["status"].as_str().unwrap_or_default().to_owned(),
                fee: money_or_zero(&b["paypal_fee"], gross.currency)?,
                gross,
                at: time(&r["create_time"])?,
                counterparty: None,
                cause: None,
                call: format!("GET {call}"),
            })
        }
        PayPalRef::SubscriptionSale(id) => {
            let call = format!("/v1/payments/sale/{id}");
            let s = get(client, &call).await?;
            let gross = money(&s["amount"])?;
            let cause = s["billing_agreement_id"]
                .as_str()
                .and_then(|b| b.parse().ok())
                .map(|subscription| WalletCause::Subscription { subscription });
            Ok(Observed {
                paypal: object.clone(),
                kind: WalletEntryKind::Subscription,
                status: s["state"].as_str().unwrap_or_default().to_owned(),
                fee: money_or_zero(&s["transaction_fee"], gross.currency)?,
                gross,
                at: time(&s["create_time"])?,
                counterparty: None,
                cause,
                call: format!("GET {call}"),
            })
        }
        PayPalRef::Payout(id) => {
            let call = format!("/v1/payments/payouts-item/{id}");
            let p = get(client, &call).await?;
            let item = &p["payout_item"];
            let gross = money(&item["amount"])?;
            let receiver = item["receiver"].as_str().map(str::to_owned);
            Ok(Observed {
                paypal: object.clone(),
                kind: WalletEntryKind::Payout,
                status: p["transaction_status"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                fee: money_or_zero(&p["payout_item_fee"], gross.currency)?,
                gross,
                at: time(&p["time_processed"])?,
                counterparty: receiver.map(|r| WalletCounterparty {
                    name: r.clone(),
                    email: Some(r),
                    company: None,
                }),
                cause: None,
                call: format!("GET {call}"),
            })
        }
    }
}
