//! PayPal edge cases for the ecosystem run (`specs/saas/paypal-edge-cases.md`), added to the
//! gateway:
//!
//! - [`Effects::list_webhook_events`]: what PayPal sent (`GET
//!   /v1/notifications/webhooks-events`), each marked with whether our webhook received it, so
//!   the hourly reconcile can find an event we missed.
//! - [`Effects::resend_webhook_event`]: PayPal delivers an event again (`POST
//!   /v1/notifications/webhooks-events/{id}/resend`), to our own webhook id only, never to another
//!   webhook in the shared app. The second delivery is counted as a duplicate and changes nothing.
//! - [`Effects::cancel_unclaimed_payout_item`]: a payout to an address with no PayPal account
//!   stays `UNCLAIMED`; cancelling it (`POST /v1/payments/payouts-item/{id}/cancel`) returns the
//!   money to the company. Only an unclaimed item of that company's own payout.
//! - [`Effects::simulating`]: PayPal's sandbox error simulation on Orders v2 and Payments v2.
//!
//! Each is kept in `paypal_edge_cases` ([`Effects::edge_cases`]).

use agentrix_types::SiteId;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::paypal::{MockResponse, PayPalClient, PayPalError, SANDBOX_ERROR_SIMULATION};
use crate::payouts::read_error;
use crate::{EffectError, Effects, Outcome, PayoutItem, now};

/// One event PayPal sent, as `GET /v1/notifications/webhooks-events` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// PayPal's event id (`WH-…`).
    pub id: String,
    /// For example `PAYMENT.CAPTURE.COMPLETED`.
    pub event_type: String,
    /// For example `capture`.
    pub resource_type: Option<String>,
    /// The id of what the event is about.
    pub resource_id: Option<String>,
    /// PayPal's one-line summary.
    pub summary: Option<String>,
    /// PayPal's delivery status, when it says one.
    pub status: Option<String>,
    /// When PayPal made the event (ISO 8601).
    pub create_time: Option<String>,
    /// Whether our webhook received and verified it (it is in `paypal_events`).
    pub received: bool,
    /// How many times it came again after the first, and changed nothing.
    pub duplicates: i64,
}

impl WebhookEvent {
    /// Reads PayPal's event document; `None` without an `id`.
    pub fn from_paypal(body: &Value) -> Option<Self> {
        let text = |v: &Value| v.as_str().map(str::to_owned);
        Some(Self {
            id: body["id"].as_str()?.to_owned(),
            event_type: body["event_type"].as_str().unwrap_or_default().to_owned(),
            resource_type: text(&body["resource_type"]),
            resource_id: text(&body["resource"]["id"]),
            summary: text(&body["summary"]),
            status: text(&body["status"]),
            create_time: text(&body["create_time"]),
            received: false,
            duplicates: 0,
        })
    }
}

/// Which events to list. Times are ISO 8601, as PayPal takes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebhookEventsQuery {
    /// How many, 1 to 300 (50 when unset).
    pub page_size: Option<u32>,
    /// Made at or after.
    pub start_time: Option<String>,
    /// Made before.
    pub end_time: Option<String>,
    /// Only this kind, such as `PAYMENT.CAPTURE.COMPLETED`.
    pub event_type: Option<String>,
    /// Only events about this PayPal transaction.
    pub transaction_id: Option<String>,
}

/// One edge case the run made happen, as `paypal_edge_cases` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct EdgeCase {
    /// `edge_…`.
    pub id: String,
    /// `webhook.resend`, `payout_item.cancel` or `simulation`.
    pub kind: String,
    /// The company concerned, when there is one.
    pub site_id: Option<String>,
    /// The event id, the payout item id, or the call's path.
    pub subject: String,
    /// "sandbox error simulation" for a simulated call.
    pub label: Option<String>,
    /// `executed`, `not_executed` or `unknown`.
    pub outcome: String,
    /// JSON: what was asked and what PayPal answered.
    pub detail: String,
    /// Always, in Horizon 1.
    pub sandbox: bool,
    /// Unix seconds.
    pub created_at: i64,
}

/// A PayPal id we put in a path: letters, digits, `-` and `_`, at most 64.
fn check_id(id: &str, what: &str) -> Result<(), EffectError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(EffectError::BadRequest(format!(
            "'{id}' isn't a PayPal {what} id."
        )))
    }
}

fn outcome_of(e: &PayPalError) -> Outcome {
    if e.outcome_unknown() {
        Outcome::Unknown
    } else {
        Outcome::NotExecuted
    }
}

impl Effects {
    /// The webhook events PayPal sent, newest first as PayPal lists them, each marked with
    /// whether our webhook received it. Reads only.
    pub async fn list_webhook_events(
        &self,
        query: &WebhookEventsQuery,
    ) -> Result<Vec<WebhookEvent>, EffectError> {
        let client = self.client()?;
        let mut q = url::form_urlencoded::Serializer::new(String::new());
        q.append_pair(
            "page_size",
            &query.page_size.unwrap_or(50).clamp(1, 300).to_string(),
        );
        for (name, value) in [
            ("start_time", &query.start_time),
            ("end_time", &query.end_time),
            ("event_type", &query.event_type),
            ("transaction_id", &query.transaction_id),
        ] {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                q.append_pair(name, value);
            }
        }
        let body = client
            .request(
                Method::GET,
                &format!("/v1/notifications/webhooks-events?{}", q.finish()),
                None,
                None,
            )
            .await
            .map_err(read_error)?;
        let mut events: Vec<WebhookEvent> = body["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(WebhookEvent::from_paypal)
            .collect();
        for event in &mut events {
            self.mark_received(event).await?;
        }
        Ok(events)
    }

    async fn mark_received(&self, event: &mut WebhookEvent) -> Result<(), sqlx::Error> {
        let seen: Option<(i64,)> =
            sqlx::query_as("SELECT duplicates FROM paypal_events WHERE id = ?")
                .bind(&event.id)
                .fetch_optional(self.pool())
                .await?;
        event.received = seen.is_some();
        event.duplicates = seen.map_or(0, |(n,)| n);
        Ok(())
    }

    /// Asks PayPal to deliver `event_id` again, to our own webhook only (the configured webhook
    /// id; nothing is sent without one). Our handler counts the second delivery as a duplicate
    /// and changes nothing.
    pub async fn resend_webhook_event(&self, event_id: &str) -> Result<WebhookEvent, EffectError> {
        check_id(event_id, "webhook event")?;
        let client = self.client()?;
        let webhook_id = client
            .webhook_id()
            .ok_or_else(|| {
                EffectError::BadRequest(
                    "No webhook id is set here, so there is no webhook of ours to resend to. Nothing was sent."
                        .into(),
                )
            })?
            .to_owned();
        let path = format!("/v1/notifications/webhooks-events/{event_id}/resend");
        let answer = client
            .request(
                Method::POST,
                &path,
                Some(&json!({ "webhook_ids": [webhook_id] })),
                None,
            )
            .await;
        match answer {
            Ok(body) => {
                let mut event = WebhookEvent::from_paypal(&body).unwrap_or_else(|| WebhookEvent {
                    id: event_id.to_owned(),
                    event_type: String::new(),
                    resource_type: None,
                    resource_id: None,
                    summary: None,
                    status: None,
                    create_time: None,
                    received: false,
                    duplicates: 0,
                });
                self.mark_received(&mut event).await?;
                self.record_edge_case(
                    "webhook.resend",
                    None,
                    event_id,
                    None,
                    Outcome::Executed,
                    &json!({ "webhook_id": webhook_id, "event_type": event.event_type, "status": event.status }),
                )
                .await?;
                Ok(event)
            }
            Err(e) => {
                self.record_edge_case(
                    "webhook.resend",
                    None,
                    event_id,
                    None,
                    outcome_of(&e),
                    &json!({ "webhook_id": webhook_id, "error": e.to_string() }),
                )
                .await?;
                Err(read_error(e))
            }
        }
    }

    /// Cancels an unclaimed item of `site`'s own payout (a payout to an address with no PayPal
    /// account), so PayPal returns the money to the company. Refused before PayPal is asked when
    /// the item isn't `site`'s, or isn't `UNCLAIMED`.
    pub async fn cancel_unclaimed_payout_item(
        &self,
        site: &SiteId,
        item_id: &str,
    ) -> Result<PayoutItem, EffectError> {
        check_id(item_id, "payout item")?;
        let client = self.client()?;
        let ours: Option<(String,)> =
            sqlx::query_as("SELECT id FROM payouts WHERE site_id = ? AND item_id = ?")
                .bind(site.as_str())
                .bind(item_id)
                .fetch_optional(self.pool())
                .await?;
        let Some((payout_id,)) = ours else {
            return Err(EffectError::NotFound);
        };
        let item = self.payout_item(item_id).await?;
        if item.status != "UNCLAIMED" {
            return Err(EffectError::NotExecuted(format!(
                "PayPal cancels only an unclaimed payout, and this one is {}. Nothing changed.",
                item.status
            )));
        }
        let answer = client
            .request(
                Method::POST,
                &format!("/v1/payments/payouts-item/{item_id}/cancel"),
                None,
                Some(&format!("cancel-{item_id}")),
            )
            .await;
        let body = match answer {
            Ok(body) => body,
            Err(e) => {
                self.record_edge_case(
                    "payout_item.cancel",
                    Some(site),
                    item_id,
                    None,
                    outcome_of(&e),
                    &json!({ "payout": payout_id, "error": e.to_string() }),
                )
                .await?;
                return Err(read_error(e));
            }
        };
        let cancelled = match PayoutItem::from_paypal(&body) {
            Some(item) => item,
            None => self.payout_item(item_id).await?,
        };
        sqlx::query("UPDATE payouts SET item_status = ?, updated_at = ? WHERE id = ?")
            .bind(&cancelled.status)
            .bind(now())
            .bind(&payout_id)
            .execute(self.pool())
            .await?;
        self.record_edge_case(
            "payout_item.cancel",
            Some(site),
            item_id,
            None,
            Outcome::Executed,
            &json!({ "payout": payout_id, "was": item.status, "now": cancelled.status, "amount": cancelled.amount }),
        )
        .await?;
        Ok(cancelled)
    }

    /// The edge cases kept so far, newest first.
    pub async fn edge_cases(&self, limit: i64) -> Result<Vec<EdgeCase>, EffectError> {
        Ok(sqlx::query_as(
            "SELECT id, kind, site_id, subject, label, outcome, detail, sandbox, created_at FROM paypal_edge_cases ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(self.pool())
        .await?)
    }

    async fn record_edge_case(
        &self,
        kind: &str,
        site: Option<&SiteId>,
        subject: &str,
        label: Option<&str>,
        outcome: Outcome,
        detail: &Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO paypal_edge_cases (id, kind, site_id, subject, label, outcome, detail, sandbox, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("edge_{}", Uuid::now_v7().simple()))
        .bind(kind)
        .bind(site.map(SiteId::as_str))
        .bind(subject)
        .bind(label)
        .bind(outcome.as_str())
        .bind(detail.to_string())
        .bind(self.sandbox())
        .bind(now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// An Orders v2 or Payments v2 call for `site`, with this copy's sandbox error simulation
    /// when it carries one; a simulated call is kept as a "sandbox error simulation".
    pub(crate) async fn paypal_call(
        &self,
        client: &PayPalClient,
        site: &SiteId,
        method: Method,
        path: &str,
        body: Option<&Value>,
        idempotency_key: Option<&str>,
    ) -> Result<Value, PayPalError> {
        let mock = self
            .simulation
            .as_deref()
            .filter(|_| MockResponse::applies_to(path));
        let answer = client
            .request_simulating(method.clone(), path, body, idempotency_key, mock)
            .await;
        if let Some(mock) = mock {
            let (outcome, detail) = match &answer {
                Ok(v) => (
                    Outcome::Executed,
                    json!({ "method": method.as_str(), "code": mock.code(), "status": v["status"] }),
                ),
                Err(e) => (
                    outcome_of(e),
                    json!({ "method": method.as_str(), "code": mock.code(), "error": e.to_string() }),
                ),
            };
            if let Err(e) = self
                .record_edge_case(
                    "simulation",
                    Some(site),
                    path,
                    Some(SANDBOX_ERROR_SIMULATION),
                    outcome,
                    &detail,
                )
                .await
            {
                tracing::warn!(error = %e, "effects: sandbox error simulation not recorded");
            }
        }
        answer
    }
}
