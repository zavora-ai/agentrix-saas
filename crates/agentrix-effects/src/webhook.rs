//! `POST /effects/paypal/webhook` on the platform host. Only events PayPal's own signature
//! check accepts (`/v1/notifications/verify-webhook-signature` with our webhook id) are applied,
//! each once (PayPal retries; its event id dedupes). Webhooks only settle what we already
//! started: they never create a payment, a subscription or a payout.

use agentrix_types::SiteId;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::Method;
use serde::Serialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::{Effects, now};

/// The webhook route.
pub fn router(effects: Effects) -> Router {
    Router::new()
        .route("/effects/paypal/webhook", post(webhook))
        .with_state(effects)
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

async fn webhook(State(effects): State<Effects>, headers: HeaderMap, body: Bytes) -> Response {
    match effects.handle_webhook(&headers, &body).await {
        Ok(applied) => Json(json!({ "applied": applied })).into_response(),
        Err(WebhookError::NotJson) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "message": "Not JSON." })),
        )
            .into_response(),
        Err(WebhookError::NotVerified) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Signature not verified." })),
        )
            .into_response(),
        Err(WebhookError::Off) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "message": "Payments aren't switched on." })),
        )
            .into_response(),
        Err(WebhookError::Retry(detail)) => {
            tracing::warn!(%detail, "effects: webhook not applied; PayPal will retry");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "message": "Try again later." })),
            )
                .into_response()
        }
    }
}

/// PayPal's headers on every delivery it signs.
const SIGNATURE_HEADERS: [&str; 5] = [
    "paypal-auth-algo",
    "paypal-cert-url",
    "paypal-transmission-id",
    "paypal-transmission-sig",
    "paypal-transmission-time",
];

/// The body of `/v1/notifications/verify-webhook-signature`. `webhook_event` is the delivery's
/// own bytes: PayPal checks its signature over the event as it sent it, so a copy re-serialized
/// with its keys in another order fails.
#[derive(Serialize)]
struct VerifyRequest<'a> {
    auth_algo: Option<String>,
    cert_url: Option<String>,
    transmission_id: Option<String>,
    transmission_sig: Option<String>,
    transmission_time: Option<String>,
    webhook_id: &'a str,
    webhook_event: &'a RawValue,
}

/// Why a webhook wasn't applied.
#[derive(Debug)]
pub enum WebhookError {
    /// The body isn't a JSON event.
    NotJson,
    /// PayPal's check refused the signature, or no webhook id is set.
    NotVerified,
    /// No PayPal keys.
    Off,
    /// Ours or PayPal's fault; PayPal retries.
    Retry(String),
}

impl Effects {
    /// Verifies and applies one PayPal delivery, `body` as PayPal sent it: PayPal checks the
    /// signature over the event's own bytes, so they go to its check unchanged. `Ok(false)` for
    /// an event already applied or not one we act on. A delivery PayPal doesn't verify, or one
    /// without PayPal's signature headers, is logged with its transmission id.
    pub async fn handle_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<bool, WebhookError> {
        let client = self.inner.client.as_deref().ok_or(WebhookError::Off)?;
        let raw: &RawValue = serde_json::from_slice(body).map_err(|_| WebhookError::NotJson)?;
        let event: Value = serde_json::from_str(raw.get()).map_err(|_| WebhookError::NotJson)?;
        let transmission_id = header(headers, "paypal-transmission-id");
        let missing: Vec<&str> = SIGNATURE_HEADERS
            .iter()
            .copied()
            .filter(|h| header(headers, h).is_none_or(|v| v.is_empty()))
            .collect();
        if !missing.is_empty() {
            tracing::warn!(
                ?missing,
                transmission_id = transmission_id.as_deref().unwrap_or("-"),
                event_id = event["id"].as_str().unwrap_or("-"),
                "effects: a webhook delivery without PayPal's signature headers; not verified"
            );
            return Err(WebhookError::NotVerified);
        }
        let Some(webhook_id) = client.webhook_id() else {
            tracing::warn!(
                transmission_id = transmission_id.as_deref().unwrap_or("-"),
                "effects: a PayPal webhook arrived but no webhook id is set here; not verified"
            );
            return Err(WebhookError::NotVerified);
        };
        let check = VerifyRequest {
            auth_algo: header(headers, "paypal-auth-algo"),
            cert_url: header(headers, "paypal-cert-url"),
            transmission_id: transmission_id.clone(),
            transmission_sig: header(headers, "paypal-transmission-sig"),
            transmission_time: header(headers, "paypal-transmission-time"),
            webhook_id,
            webhook_event: raw,
        };
        let verdict = client
            .request_serialized(
                Method::POST,
                "/v1/notifications/verify-webhook-signature",
                &check,
                None,
            )
            .await
            .map_err(|e| WebhookError::Retry(e.to_string()))?;
        if verdict["verification_status"].as_str() != Some("SUCCESS") {
            tracing::warn!(
                transmission_id = transmission_id.as_deref().unwrap_or("-"),
                verification_status = verdict["verification_status"].as_str().unwrap_or("none"),
                event_id = event["id"].as_str().unwrap_or("-"),
                event_type = event["event_type"].as_str().unwrap_or("-"),
                "effects: PayPal didn't verify a webhook delivery; not applied"
            );
            return Err(WebhookError::NotVerified);
        }
        let id = event["id"].as_str().unwrap_or_default().to_owned();
        let kind = event["event_type"].as_str().unwrap_or_default().to_owned();
        let resource = &event["resource"];
        let resource_id = resource["id"].as_str().unwrap_or_default().to_owned();
        if id.is_empty() {
            return Ok(false);
        }
        let fresh = sqlx::query("INSERT OR IGNORE INTO paypal_events (id, kind, resource, received_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(&kind)
            .bind(&resource_id)
            .bind(now())
            .execute(self.pool())
            .await
            .map_err(|e| WebhookError::Retry(e.to_string()))?
            .rows_affected()
            == 1;
        if !fresh {
            // Delivered again (PayPal's retry, or a resend): counted, and nothing changes.
            sqlx::query(
                "UPDATE paypal_events SET duplicates = duplicates + 1, last_duplicate_at = ? WHERE id = ?",
            )
            .bind(now())
            .bind(&id)
            .execute(self.pool())
            .await
            .map_err(|e| WebhookError::Retry(e.to_string()))?;
            return Ok(false);
        }
        let applied = self.apply_event(&kind, resource).await;
        if let Err(e) = &applied {
            // Let PayPal retry: forget the event so the retry is applied.
            let _ = sqlx::query("DELETE FROM paypal_events WHERE id = ?")
                .bind(&id)
                .execute(self.pool())
                .await;
            return Err(WebhookError::Retry(e.to_string()));
        }
        applied.map_err(|e| WebhookError::Retry(e.to_string()))
    }

    /// Records what PayPal collected for a plan (a v1 sale: `{id, amount: {total, currency},
    /// create_time}`), once per sale id, for W3's Money page.
    async fn record_plan_payment(
        &self,
        provider_ref: &str,
        sale: &Value,
    ) -> Result<(), crate::EffectError> {
        let Some(sale_id) = sale["id"].as_str() else {
            return Ok(());
        };
        let amount = json!({ "currency_code": sale["amount"]["currency"], "value": sale["amount"]["total"] });
        let Some((_, minor)) = crate::paypal::minor_of(&amount) else {
            return Ok(());
        };
        let paid_at = sale["create_time"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map_or_else(now, |t| t.timestamp());
        sqlx::query("INSERT OR IGNORE INTO plan_payments (provider_ref, provider_payment_id, amount_minor, paid_at) VALUES (?, ?, ?, ?)")
            .bind(provider_ref)
            .bind(sale_id)
            .bind(minor)
            .bind(paid_at)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn apply_event(&self, kind: &str, resource: &Value) -> Result<bool, crate::EffectError> {
        match kind {
            // A capture settled (after PENDING) or the order was approved and captured elsewhere.
            "PAYMENT.CAPTURE.COMPLETED" | "PAYMENT.CAPTURE.DENIED" | "CHECKOUT.ORDER.APPROVED" => {
                let order_id = resource["supplementary_data"]["related_ids"]["order_id"]
                    .as_str()
                    .or_else(|| {
                        (kind == "CHECKOUT.ORDER.APPROVED")
                            .then(|| resource["id"].as_str())
                            .flatten()
                    });
                let Some(order_id) = order_id else {
                    return Ok(false);
                };
                let row: Option<(String,)> =
                    sqlx::query_as("SELECT site_id FROM customer_payments WHERE provider_ref = ?")
                        .bind(order_id)
                        .fetch_optional(self.pool())
                        .await?;
                let Some((site,)) = row else { return Ok(false) };
                self.reconcile(&SiteId::new(site), order_id).await?;
                Ok(true)
            }
            k if k.starts_with("BILLING.SUBSCRIPTION.") => {
                let Some(provider_ref) = resource["id"].as_str() else {
                    return Ok(false);
                };
                if self.subscription_by_ref(provider_ref).await?.is_none() {
                    return Ok(false);
                }
                self.refresh_subscription(provider_ref).await?;
                Ok(true)
            }
            // A payout batch or item moved on (PENDING to SUCCESS, UNCLAIMED, RETURNED…): read
            // the batch back.
            k if k.starts_with("PAYMENT.PAYOUTSBATCH.")
                || k.starts_with("PAYMENT.PAYOUTS-ITEM.") =>
            {
                let batch_id = resource["batch_header"]["payout_batch_id"]
                    .as_str()
                    .or(resource["payout_batch_id"].as_str());
                let Some(batch_id) = batch_id else {
                    return Ok(false);
                };
                self.settle_payout_batch(batch_id).await
            }
            "PAYMENT.SALE.COMPLETED" | "PAYMENT.SALE.DENIED" => {
                let Some(provider_ref) = resource["billing_agreement_id"].as_str() else {
                    return Ok(false);
                };
                let Some(current) = self.subscription_by_ref(provider_ref).await? else {
                    return Ok(false);
                };
                if kind == "PAYMENT.SALE.COMPLETED" {
                    self.record_plan_payment(provider_ref, resource).await?;
                }
                let state = if kind == "PAYMENT.SALE.COMPLETED" {
                    "active"
                } else {
                    "past_due"
                };
                self.set_subscription_state(&current, state, None, kind)
                    .await?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
