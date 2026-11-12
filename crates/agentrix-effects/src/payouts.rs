//! Partner payouts through PayPal Payouts (`POST /v1/payments/payouts`): money leaving the
//! company, so each one runs only with the approval of its exact fingerprint (the founder's
//! mandate, or an owner's passkey), and once. Ported from Zavora's `steward-paypal`
//! (Apache-2.0, same authors; see NOTICE).
//!
//! - **Once.** The fingerprint's hex is both the batch's `sender_batch_id` and the request's
//!   `PayPal-Request-Id`. PayPal answers a repeated request id with the first result, and
//!   refuses a `sender_batch_id` it has seen in the last 30 days, linking the first batch. So
//!   one approval pays at most once, even if this node's journal is lost.
//! - **Three outcomes.** Executed (PayPal took the batch), not executed (PayPal refused it, or
//!   the batch or its item failed), or unknown (a timeout or a PayPal 5xx).
//! - **Unknown is read back, never retried blind.** [`Effects::reconcile_payout`] reads the
//!   batch with `GET /v1/payments/payouts/{id}`. When PayPal never told us the id (the answer
//!   was lost), the only key PayPal knows the batch by is its `sender_batch_id`: we ask with the
//!   identical request and request id, PayPal answers with the first batch or a link to it, and
//!   we read that batch back. If the first request never reached PayPal, this one is the
//!   payout; there is never a second.

use agentrix_types::{EffectRequest, Money, ReceiptKind, SiteId};
use chrono::{Duration, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::payments::hex_of;
use crate::paypal::{self, PayPalClient, PayPalError};
use crate::{EffectError, Effects, Outcome, journal, now, words};

/// The tool a payout's [`EffectRequest`] names.
pub const PAYOUT_TOOL: &str = "payout.send";

/// What an agent (Finance) asks to pay a partner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayoutRequest {
    /// The company paying (its site).
    pub site: SiteId,
    /// The agent asking, for example `finance`.
    pub agent: String,
    /// The payee's PayPal email.
    pub receiver: String,
    /// How much.
    pub amount: Money,
    /// What it is for. PayPal shows it to the payee.
    pub note: String,
}

/// What a payout did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayoutOutcome {
    /// `po_…` (also PayPal's `sender_item_id`).
    pub payout_id: String,
    /// Executed, not executed, or unknown (U27). Unknown: call [`Effects::reconcile_payout`].
    pub outcome: Outcome,
    /// The amount.
    pub amount: Money,
    /// The payee's PayPal email.
    pub receiver: String,
    /// PayPal's `payout_batch_id`, once PayPal said it.
    pub batch_id: Option<String>,
    /// PayPal's batch status: `PENDING`, `PROCESSING`, `SUCCESS`, `DENIED`, `CANCELED`.
    pub batch_status: Option<String>,
    /// PayPal's `payout_item_id`, once the batch was read back.
    pub item_id: Option<String>,
    /// PayPal's item status: `SUCCESS`, `UNCLAIMED`, `PENDING`, `FAILED`, `RETURNED` and so on.
    pub item_status: Option<String>,
    /// The `payout.sent` receipt.
    pub receipt: Option<String>,
    /// Words for the owner.
    pub message: String,
}

/// A payout as stored.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Payout {
    /// `po_…`.
    pub id: String,
    /// The company paying.
    pub site_id: String,
    /// The approved effect's fingerprint (`sha256:…`); its hex is the `sender_batch_id`.
    pub fingerprint: String,
    /// The payee's PayPal email.
    pub receiver: String,
    /// ISO 4217.
    pub currency: String,
    /// The amount.
    pub amount_minor: i64,
    /// What it is for.
    pub note: String,
    /// started | executed | not_executed | unknown.
    pub state: String,
    /// PayPal's `payout_batch_id`.
    pub batch_id: Option<String>,
    /// PayPal's batch status.
    pub batch_status: Option<String>,
    /// PayPal's `payout_item_id`.
    pub item_id: Option<String>,
    /// PayPal's item status.
    pub item_status: Option<String>,
    /// Who approved it (the approval's account).
    pub approved_by: Option<String>,
    /// The `payout.sent` receipt.
    pub receipt: Option<String>,
    /// PayPal's sandbox.
    pub sandbox: bool,
    /// Created.
    pub created_at: i64,
    /// Updated.
    pub updated_at: i64,
}

/// A payout batch as PayPal reports it (`GET /v1/payments/payouts/{id}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayoutBatch {
    /// PayPal's `payout_batch_id`.
    pub batch_id: String,
    /// Ours: the approved effect's fingerprint hex.
    pub sender_batch_id: Option<String>,
    /// `PENDING`, `PROCESSING`, `SUCCESS`, `DENIED`, `CANCELED`.
    pub status: String,
    /// The items (one per payout here). Empty in PayPal's answer to the POST.
    pub items: Vec<PayoutItem>,
}

/// One payout item as PayPal reports it (`GET /v1/payments/payouts-item/{id}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayoutItem {
    /// PayPal's `payout_item_id`.
    pub item_id: String,
    /// PayPal's `payout_batch_id`.
    pub batch_id: Option<String>,
    /// PayPal's transaction, once paid.
    pub transaction_id: Option<String>,
    /// `SUCCESS`, `UNCLAIMED`, `PENDING`, `ONHOLD`, `FAILED`, `BLOCKED`, `RETURNED`,
    /// `REFUNDED`, `REVERSED`, `CANCELED`, `DENIED`.
    pub status: String,
    /// The payee.
    pub receiver: Option<String>,
    /// The amount.
    pub amount: Option<Money>,
    /// Ours: `po_…`.
    pub sender_item_id: Option<String>,
    /// PayPal's error name, when the item failed.
    pub error: Option<String>,
}

impl PayoutBatch {
    /// Reads PayPal's batch document; `None` without a `payout_batch_id`.
    pub fn from_paypal(body: &Value) -> Option<Self> {
        let header = &body["batch_header"];
        Some(Self {
            batch_id: header["payout_batch_id"].as_str()?.to_owned(),
            sender_batch_id: header["sender_batch_header"]["sender_batch_id"]
                .as_str()
                .map(str::to_owned),
            status: header["batch_status"]
                .as_str()
                .unwrap_or("PENDING")
                .to_owned(),
            items: body["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(PayoutItem::from_paypal)
                .collect(),
        })
    }
}

impl PayoutItem {
    /// Reads PayPal's item document; `None` without a `payout_item_id`.
    pub fn from_paypal(body: &Value) -> Option<Self> {
        let item = &body["payout_item"];
        Some(Self {
            item_id: body["payout_item_id"].as_str()?.to_owned(),
            batch_id: body["payout_batch_id"].as_str().map(str::to_owned),
            transaction_id: body["transaction_id"].as_str().map(str::to_owned),
            status: body["transaction_status"]
                .as_str()
                .unwrap_or("PENDING")
                .to_owned(),
            receiver: item["receiver"].as_str().map(str::to_owned),
            amount: payout_money(&item["amount"]),
            sender_item_id: item["sender_item_id"].as_str().map(str::to_owned),
            error: body["errors"]["name"]
                .as_str()
                .or(body["errors"]["message"].as_str())
                .map(str::to_owned),
        })
    }
}

/// Payouts write money `{value, currency}` (Orders write `{currency_code, value}`).
fn payout_amount(money: &Money) -> Value {
    let a = paypal::amount(&money.currency, money.minor);
    json!({ "value": a["value"], "currency": a["currency_code"] })
}

fn payout_money(amount: &Value) -> Option<Money> {
    let (currency, minor) = paypal::minor_of(
        &json!({ "currency_code": amount["currency"], "value": amount["value"] }),
    )?;
    Some(Money { currency, minor })
}

fn check_receiver(receiver: &str) -> Result<(), EffectError> {
    let ok = receiver.len() <= 127
        && !receiver.contains(char::is_whitespace)
        && receiver.split_once('@').is_some_and(|(user, host)| {
            !user.is_empty()
                && host.contains('.')
                && !host.starts_with('.')
                && !host.ends_with('.')
                && !host.contains('@')
        });
    if ok {
        Ok(())
    } else {
        Err(EffectError::BadRequest(format!(
            "'{receiver}' isn't an email address PayPal can pay."
        )))
    }
}

fn check_amount(amount: &Money) -> Result<(), EffectError> {
    if amount.minor <= 0 || amount.currency.len() != 3 {
        return Err(EffectError::BadRequest(
            "A payout must be more than zero, with a 3-letter currency.".into(),
        ));
    }
    Ok(())
}

/// `j***@example.com`: enough for the owner to recognise the payee in a receipt, without
/// putting a partner's address in the signed log.
fn masked(receiver: &str) -> String {
    match receiver.split_once('@') {
        Some((user, host)) => format!("{}***@{host}", user.chars().next().unwrap_or('*')),
        None => "***".into(),
    }
}

/// The batch a refusal links to: PayPal refuses a `sender_batch_id` it has already seen and
/// links the first batch (`…/v1/payments/payouts/{payout_batch_id}`).
fn first_batch(e: &PayPalError) -> Option<String> {
    let PayPalError::Api { status, links, .. } = e else {
        return None;
    };
    if !(400..500).contains(status) {
        return None;
    }
    links.iter().find_map(|href| {
        let (_, rest) = href.split_once("/v1/payments/payouts/")?;
        let id = rest.split(['/', '?', '#']).next()?;
        (!id.is_empty()).then(|| id.to_owned())
    })
}

/// What a batch and its item say about the money.
fn verdict(batch_status: &str, item_status: Option<&str>) -> Outcome {
    match (batch_status, item_status) {
        ("DENIED" | "CANCELED", _)
        | (
            _,
            Some(
                "FAILED" | "BLOCKED" | "DENIED" | "RETURNED" | "REFUNDED" | "REVERSED" | "CANCELED",
            ),
        ) => Outcome::NotExecuted,
        _ => Outcome::Executed,
    }
}

fn message(payout: &Payout, outcome: Outcome, refusal: Option<&str>) -> String {
    let amount = words(&Money {
        currency: payout.currency.clone(),
        minor: payout.amount_minor,
    });
    let to = &payout.receiver;
    match outcome {
        Outcome::Executed => match payout.item_status.as_deref() {
            Some("SUCCESS") => format!("Paid {amount} to {to} with PayPal."),
            Some("UNCLAIMED") => format!(
                "Sent {amount} to {to} with PayPal. They haven't claimed it yet; PayPal returns it after 30 days if they don't."
            ),
            _ => format!("Sent {amount} to {to} with PayPal. PayPal is still processing it."),
        },
        Outcome::NotExecuted => match payout.item_status.as_deref() {
            Some("RETURNED" | "REFUNDED" | "REVERSED" | "CANCELED") => {
                format!("The payout of {amount} to {to} came back to the account.")
            }
            _ => match refusal {
                Some(why) => {
                    format!("PayPal didn't pay {amount} to {to} ({why}). Nothing left the account.")
                }
                None => format!("PayPal didn't pay {amount} to {to}. Nothing left the account."),
            },
        },
        Outcome::Unknown => {
            "We couldn't confirm the payout with PayPal yet. It will never be paid twice; check again in a minute.".into()
        }
    }
}

fn stored(payout: &Payout) -> PayoutOutcome {
    let outcome = match payout.state.as_str() {
        "executed" => Outcome::Executed,
        "not_executed" => Outcome::NotExecuted,
        _ => Outcome::Unknown,
    };
    PayoutOutcome {
        payout_id: payout.id.clone(),
        outcome,
        amount: Money {
            currency: payout.currency.clone(),
            minor: payout.amount_minor,
        },
        receiver: payout.receiver.clone(),
        batch_id: payout.batch_id.clone(),
        batch_status: payout.batch_status.clone(),
        item_id: payout.item_id.clone(),
        item_status: payout.item_status.clone(),
        receipt: payout.receipt.clone(),
        message: message(payout, outcome, None),
    }
}

impl Effects {
    /// The effect an agent puts before the founder's mandate (or an owner's card). The approval
    /// of its fingerprint, and only that, lets [`Effects::payout`] run.
    pub fn payout_request(&self, request: PayoutRequest) -> Result<EffectRequest, EffectError> {
        let receiver = request.receiver.trim().to_owned();
        check_receiver(&receiver)?;
        check_amount(&request.amount)?;
        let note = request.note.trim();
        if note.is_empty() {
            return Err(EffectError::BadRequest(
                "A payout needs a note saying what it is for.".into(),
            ));
        }
        Ok(EffectRequest {
            site: request.site,
            webagent: request.agent,
            tool: PAYOUT_TOOL.into(),
            args: json!({
                "receiver": receiver,
                "note": note.chars().take(1000).collect::<String>(),
                "nonce": Uuid::new_v4().simple().to_string(),
            }),
            amount: Some(request.amount),
            expiry: Utc::now() + Duration::days(7),
        })
    }

    /// Pays, once, exactly what was approved. `approval` is the proof for this effect's
    /// fingerprint: it carries `fingerprint` in hex and the approver's `account` (an owner, or
    /// the founder's mandate). A second call returns the first outcome; one whose outcome was
    /// unknown is read back first.
    pub async fn payout(
        &self,
        effect: &EffectRequest,
        approval: &Value,
    ) -> Result<PayoutOutcome, EffectError> {
        if effect.tool != PAYOUT_TOOL {
            return Err(EffectError::BadRequest("Not a payout.".into()));
        }
        let fp = effect
            .fingerprint()
            .map_err(|e| EffectError::Internal(e.to_string()))?
            .to_string();
        if approval["fingerprint"].as_str() != Some(hex_of(&fp)) {
            return Err(EffectError::NotApproved);
        }
        if let Some(existing) = self.payout_by_fingerprint(&fp).await? {
            return if matches!(existing.state.as_str(), "started" | "unknown") {
                self.settle_payout(existing).await
            } else {
                Ok(stored(&existing))
            };
        }
        if effect.is_expired(Utc::now()) {
            return Err(EffectError::NotApproved);
        }
        let amount = effect
            .amount
            .clone()
            .ok_or_else(|| EffectError::BadRequest("A payout needs an amount.".into()))?;
        check_amount(&amount)?;
        let receiver = effect.args["receiver"].as_str().unwrap_or_default();
        check_receiver(receiver)?;
        let note = effect.args["note"].as_str().unwrap_or_default();
        let client = self.client()?;
        let payout_id = format!("po_{}", Uuid::now_v7().simple());
        if journal::claim(
            self.pool(),
            &fp,
            effect.site.as_str(),
            PAYOUT_TOOL,
            &payout_id,
        )
        .await?
        .is_some()
        {
            return Err(EffectError::Unknown("payout already running".into()));
        }
        let approver = approval["account"].as_str().map(str::to_owned).or_else(|| {
            approval["account"]
                .as_object()
                .map(|_| approval["account"].to_string())
        });
        let at = now();
        // Written before PayPal is asked, so a crash mid-call leaves a row to settle.
        sqlx::query(
            "INSERT INTO payouts (id, site_id, fingerprint, receiver, currency, amount_minor, note, state, approved_by, sandbox, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'started', ?, ?, ?, ?)",
        )
        .bind(&payout_id)
        .bind(effect.site.as_str())
        .bind(&fp)
        .bind(receiver)
        .bind(&amount.currency)
        .bind(amount.minor)
        .bind(note)
        .bind(&approver)
        .bind(self.sandbox())
        .bind(at)
        .bind(at)
        .execute(self.pool())
        .await?;
        let payout = self
            .payout_by_fingerprint(&fp)
            .await?
            .ok_or_else(|| EffectError::Internal("payout row vanished".into()))?;
        let answer = submit(client, &payout).await;
        self.record_payout(payout, answer).await
    }

    /// Reads a payout back from PayPal and records what it says (after an unknown outcome, on
    /// a webhook, or to follow an item from PENDING to SUCCESS).
    pub async fn reconcile_payout(
        &self,
        site: &SiteId,
        payout_id: &str,
    ) -> Result<PayoutOutcome, EffectError> {
        let payout = self
            .stored_payout(site, payout_id)
            .await?
            .ok_or(EffectError::NotFound)?;
        self.settle_payout(payout).await
    }

    async fn settle_payout(&self, payout: Payout) -> Result<PayoutOutcome, EffectError> {
        let client = self.client()?;
        match payout.batch_id.clone() {
            Some(batch_id) => {
                let batch = client
                    .request(
                        Method::GET,
                        &format!("/v1/payments/payouts/{batch_id}"),
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| {
                        if e.outcome_unknown() {
                            EffectError::Unknown(e.to_string())
                        } else {
                            EffectError::NotExecuted(e.to_string())
                        }
                    })?;
                self.record_payout(payout, Ok(batch)).await
            }
            // PayPal never said the batch id: ask for the batch by its sender_batch_id.
            None if matches!(payout.state.as_str(), "started" | "unknown") => {
                let answer = submit(client, &payout).await;
                self.record_payout(payout, answer).await
            }
            None => Ok(stored(&payout)),
        }
    }

    async fn record_payout(
        &self,
        mut payout: Payout,
        answer: Result<Value, PayPalError>,
    ) -> Result<PayoutOutcome, EffectError> {
        let mut refusal = None;
        let outcome = match answer {
            Ok(body) => match PayoutBatch::from_paypal(&body) {
                Some(batch) => {
                    let item = batch
                        .items
                        .iter()
                        .find(|i| i.sender_item_id.as_deref() == Some(payout.id.as_str()))
                        .or(batch.items.first());
                    if let Some(item) = item {
                        payout.item_id = Some(item.item_id.clone());
                        payout.item_status = Some(item.status.clone());
                    }
                    let outcome = verdict(&batch.status, payout.item_status.as_deref());
                    payout.batch_id = Some(batch.batch_id);
                    payout.batch_status = Some(batch.status);
                    outcome
                }
                None => {
                    tracing::warn!(payout = %payout.id, "effects: PayPal answered a payout without a batch id");
                    Outcome::Unknown
                }
            },
            Err(e) if e.outcome_unknown() => {
                tracing::warn!(error = %e, payout = %payout.id, "effects: payout outcome unknown");
                Outcome::Unknown
            }
            Err(e) => {
                tracing::warn!(error = %e, payout = %payout.id, "effects: payout refused");
                if let PayPalError::Api { name, .. } = &e {
                    refusal = Some(name.clone());
                }
                Outcome::NotExecuted
            }
        };
        payout.state = outcome.as_str().to_owned();
        let site = SiteId::new(payout.site_id.clone());
        let detail = json!({
            "payout": payout.id, "receiver": masked(&payout.receiver),
            "amount": Money { currency: payout.currency.clone(), minor: payout.amount_minor },
            "batch": payout.batch_id, "batch_status": payout.batch_status,
            "item": payout.item_id, "item_status": payout.item_status,
            "approved_by": payout.approved_by, "fingerprint": payout.fingerprint, "sandbox": payout.sandbox,
        });
        if outcome == Outcome::Executed && payout.receipt.is_none() {
            payout.receipt = self
                .receipt(ReceiptKind::PayoutSent, &site, detail.clone())
                .await;
        }
        sqlx::query(
            "UPDATE payouts SET state = ?, batch_id = COALESCE(?, batch_id), batch_status = COALESCE(?, batch_status), item_id = COALESCE(?, item_id), item_status = COALESCE(?, item_status), receipt = COALESCE(?, receipt), updated_at = ? WHERE id = ?",
        )
        .bind(&payout.state)
        .bind(&payout.batch_id)
        .bind(&payout.batch_status)
        .bind(&payout.item_id)
        .bind(&payout.item_status)
        .bind(&payout.receipt)
        .bind(now())
        .bind(&payout.id)
        .execute(self.pool())
        .await?;
        journal::finish(
            self.pool(),
            &payout.fingerprint,
            outcome,
            payout.batch_id.as_deref(),
            &detail,
            payout.receipt.as_deref(),
        )
        .await?;
        let mut done = stored(&payout);
        done.message = message(&payout, outcome, refusal.as_deref());
        Ok(done)
    }

    /// A batch as PayPal reports it now (`GET /v1/payments/payouts/{batch_id}`). Reads only.
    pub async fn payout_batch(&self, batch_id: &str) -> Result<PayoutBatch, EffectError> {
        let body = self
            .client()?
            .request(
                Method::GET,
                &format!("/v1/payments/payouts/{batch_id}"),
                None,
                None,
            )
            .await
            .map_err(read_error)?;
        PayoutBatch::from_paypal(&body)
            .ok_or_else(|| EffectError::Internal("PayPal batch without id".into()))
    }

    /// One item as PayPal reports it now (`GET /v1/payments/payouts-item/{item_id}`). Reads
    /// only.
    pub async fn payout_item(&self, item_id: &str) -> Result<PayoutItem, EffectError> {
        let body = self
            .client()?
            .request(
                Method::GET,
                &format!("/v1/payments/payouts-item/{item_id}"),
                None,
                None,
            )
            .await
            .map_err(read_error)?;
        PayoutItem::from_paypal(&body)
            .ok_or_else(|| EffectError::Internal("PayPal payout item without id".into()))
    }

    /// A payout by id on a site.
    pub async fn stored_payout(
        &self,
        site: &SiteId,
        payout_id: &str,
    ) -> Result<Option<Payout>, EffectError> {
        Ok(
            sqlx::query_as("SELECT * FROM payouts WHERE site_id = ? AND id = ?")
                .bind(site.as_str())
                .bind(payout_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    /// A site's payouts, newest first.
    pub async fn payouts_for_site(&self, site: &SiteId) -> Result<Vec<Payout>, EffectError> {
        Ok(sqlx::query_as(
            "SELECT * FROM payouts WHERE site_id = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(site.as_str())
        .fetch_all(self.pool())
        .await?)
    }

    async fn payout_by_fingerprint(&self, fp: &str) -> Result<Option<Payout>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM payouts WHERE fingerprint = ?")
            .bind(fp)
            .fetch_optional(self.pool())
            .await
    }

    pub(crate) async fn payout_by_batch(
        &self,
        batch_id: &str,
    ) -> Result<Option<Payout>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM payouts WHERE batch_id = ?")
            .bind(batch_id)
            .fetch_optional(self.pool())
            .await
    }

    /// Reads a payout back for a webhook about its batch.
    pub(crate) async fn settle_payout_batch(&self, batch_id: &str) -> Result<bool, EffectError> {
        match self.payout_by_batch(batch_id).await? {
            Some(payout) => {
                self.settle_payout(payout).await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

pub(crate) fn read_error(e: PayPalError) -> EffectError {
    if e.outcome_unknown() {
        EffectError::Unknown(e.to_string())
    } else if matches!(e, PayPalError::Api { status: 404, .. }) {
        EffectError::NotFound
    } else {
        EffectError::NotExecuted(e.to_string())
    }
}

/// `POST /v1/payments/payouts` for one stored payout; a refusal that links the first batch
/// with this `sender_batch_id` reads that batch instead.
async fn submit(client: &PayPalClient, payout: &Payout) -> Result<Value, PayPalError> {
    let key = hex_of(&payout.fingerprint);
    let money = Money {
        currency: payout.currency.clone(),
        minor: payout.amount_minor,
    };
    let body = json!({
        "sender_batch_header": {
            "sender_batch_id": key,
            "email_subject": "You have a payment",
            "email_message": payout.note.chars().take(255).collect::<String>(),
        },
        "items": [{
            "recipient_type": "EMAIL",
            "receiver": payout.receiver,
            "amount": payout_amount(&money),
            "note": payout.note.chars().take(4000).collect::<String>(),
            "sender_item_id": payout.id,
        }],
    });
    match client
        .request(Method::POST, "/v1/payments/payouts", Some(&body), Some(key))
        .await
    {
        Ok(batch) => Ok(batch),
        Err(e) => match first_batch(&e) {
            Some(batch_id) => {
                tracing::info!(payout = %payout.id, %batch_id, "effects: PayPal already has this batch; reading it");
                client
                    .request(
                        Method::GET,
                        &format!("/v1/payments/payouts/{batch_id}"),
                        None,
                        None,
                    )
                    .await
            }
            None => Err(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_as_outcomes() {
        assert_eq!(verdict("PENDING", None), Outcome::Executed);
        assert_eq!(verdict("SUCCESS", Some("SUCCESS")), Outcome::Executed);
        assert_eq!(verdict("SUCCESS", Some("UNCLAIMED")), Outcome::Executed);
        assert_eq!(verdict("DENIED", None), Outcome::NotExecuted);
        assert_eq!(verdict("SUCCESS", Some("FAILED")), Outcome::NotExecuted);
        assert_eq!(verdict("SUCCESS", Some("RETURNED")), Outcome::NotExecuted);
    }

    #[test]
    fn receivers_and_links() {
        assert!(check_receiver("partner@example.com").is_ok());
        for bad in [
            "",
            "no-at",
            "@example.com",
            "a@b",
            "a b@example.com",
            "a@.com",
        ] {
            assert!(check_receiver(bad).is_err(), "{bad}");
        }
        assert_eq!(masked("wanjiru@example.com"), "w***@example.com");
        let e = PayPalError::Api {
            status: 400,
            name: "USER_BUSINESS_ERROR".into(),
            message: "User business error.".into(),
            issues: vec![],
            debug_id: None,
            links: vec!["https://api.sandbox.paypal.com/v1/payments/payouts/5UXD2E8A7EBQJ".into()],
        };
        assert_eq!(first_batch(&e).as_deref(), Some("5UXD2E8A7EBQJ"));
        assert_eq!(
            payout_amount(&Money {
                currency: "USD".into(),
                minor: 2050
            }),
            json!({ "value": "20.50", "currency": "USD" })
        );
    }
}
