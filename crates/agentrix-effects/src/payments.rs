//! Customer payments through PayPal Orders v2: create (the customer approves on PayPal's page),
//! capture once, reconcile what's unknown, cancel, and refund only with the owner's approval.

use agentrix_types::tenancy::VisitorAccountId;
use agentrix_types::{EffectRequest, Money, ReceiptKind, SiteId};
use chrono::{Duration, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::paypal::{self, PayPalError};
use crate::{EffectError, Effects, Outcome, fingerprint, journal, now, words};

/// What a site's checkout asks for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequest {
    /// The site.
    pub site: SiteId,
    /// The name on PayPal's page, for example "Kilele Bakery (Sample)".
    pub site_name: String,
    /// The order on the site (W2's id).
    pub order_id: String,
    /// The amount to charge, in the currency PayPal will charge (USD on the samples, at the
    /// order's pinned rate).
    pub amount: Money,
    /// One line PayPal shows, for example "Kilele Bakery order K-1042 (Sample)".
    pub description: String,
    /// The customer, when signed in.
    pub customer: Option<VisitorAccountId>,
    /// Where PayPal sends the customer after approving (`?token=<PayPal order id>` is added).
    pub return_url: String,
    /// Where PayPal sends the customer who cancels.
    pub cancel_url: String,
}

/// A payment that waits for the customer on PayPal's page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PaymentStarted {
    /// `pay_…`.
    pub payment_id: String,
    /// PayPal's order id (the `token` PayPal adds to the return URL).
    pub provider_ref: String,
    /// PayPal's page for the customer.
    pub approve_url: String,
    /// The effect's fingerprint.
    pub fingerprint: String,
}

/// Where a capture ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureState {
    /// The money is captured.
    Captured,
    /// PayPal is still deciding (a review, an e-cheque); a webhook settles it.
    Pending,
    /// PayPal declined it; nothing was charged.
    Declined,
    /// The customer hasn't approved on PayPal, or cancelled.
    Cancelled,
}

/// What a capture did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureOutcome {
    /// Where it ended.
    pub state: CaptureState,
    /// Executed, not executed, or unknown (U27).
    pub outcome: Outcome,
    /// The amount captured.
    pub captured: Option<Money>,
    /// PayPal's capture id.
    pub provider_capture_id: Option<String>,
    /// The `payment.captured` receipt.
    pub receipt: Option<String>,
    /// Words for the customer.
    pub message: String,
}

/// A payment as stored.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Payment {
    /// `pay_…`.
    pub id: String,
    /// The site.
    pub site_id: String,
    /// The site's order.
    pub order_id: String,
    /// The customer, when signed in.
    pub customer: Option<String>,
    /// The effect's fingerprint.
    pub fingerprint: String,
    /// paypal.
    pub provider: String,
    /// PayPal's order id.
    pub provider_ref: Option<String>,
    /// PayPal's page.
    pub approve_url: Option<String>,
    /// ISO 4217.
    pub currency: String,
    /// Asked for.
    pub amount_minor: i64,
    /// created | captured | pending | declined | cancelled | refunded | partly_refunded.
    pub state: String,
    /// PayPal's capture id.
    pub capture_id: Option<String>,
    /// Captured.
    pub captured_minor: Option<i64>,
    /// Refunded so far.
    pub refunded_minor: i64,
    /// PayPal's sandbox.
    pub sandbox: bool,
    /// Created.
    pub created_at: i64,
    /// Updated.
    pub updated_at: i64,
}

/// What a refund did.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefundOutcome {
    /// `rf_…`.
    pub refund_id: String,
    /// Executed, not executed, or unknown.
    pub outcome: Outcome,
    /// The amount.
    pub amount: Money,
    /// PayPal's refund id.
    pub provider_ref: Option<String>,
    /// The `payment.refunded` receipt.
    pub receipt: Option<String>,
}

#[derive(Serialize)]
struct CreateKey<'a> {
    kind: &'static str,
    site: &'a SiteId,
    order_id: &'a str,
    amount: &'a Money,
}

pub(crate) fn hex_of(fingerprint: &str) -> &str {
    fingerprint.strip_prefix("sha256:").unwrap_or(fingerprint)
}

fn check_url(url: &str) -> Result<(), EffectError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| EffectError::BadRequest(format!("{url} isn't a full address")))?;
    let local = parsed
        .host_str()
        .is_some_and(|h| h == "localhost" || h.ends_with(".localhost"));
    if parsed.scheme() == "https" || (parsed.scheme() == "http" && local) {
        Ok(())
    } else {
        Err(EffectError::BadRequest(
            "PayPal sends customers back over https only.".into(),
        ))
    }
}

fn not_executed_words(e: &PayPalError) -> String {
    if e.has_issue("PAYEE_ACCOUNT_RESTRICTED") || e.has_issue("PAYEE_ACCOUNT_LOCKED_OR_CLOSED") {
        return "PayPal can't take payments for this shop right now. Nothing was charged.".into();
    }
    if e.has_issue("ORDER_NOT_APPROVED") {
        return "You haven't approved the payment on PayPal yet. Nothing was charged.".into();
    }
    if e.has_issue("INSTRUMENT_DECLINED") {
        return "PayPal declined that payment method. Nothing was charged. Try again on PayPal with another one.".into();
    }
    "PayPal refused the payment. Nothing was charged.".into()
}

impl Effects {
    /// Starts a payment: a PayPal order for exactly this amount, idempotent by site + order +
    /// amount. The customer approves on `approve_url`; then the site calls [`Effects::capture`].
    pub async fn create_payment(
        &self,
        request: PaymentRequest,
    ) -> Result<PaymentStarted, EffectError> {
        if request.amount.minor <= 0 || request.amount.currency.len() != 3 {
            return Err(EffectError::BadRequest(
                "The amount must be more than zero, with a 3-letter currency.".into(),
            ));
        }
        if request.order_id.trim().is_empty() || request.order_id.len() > 127 {
            return Err(EffectError::BadRequest(
                "The order needs an id of at most 127 characters.".into(),
            ));
        }
        check_url(&request.return_url)?;
        check_url(&request.cancel_url)?;
        let fp = fingerprint(&CreateKey {
            kind: "payment.create",
            site: &request.site,
            order_id: &request.order_id,
            amount: &request.amount,
        })?
        .to_string();
        if let Some(existing) = self.payment_by_fingerprint(&fp).await? {
            if let (Some(provider_ref), Some(approve_url)) =
                (existing.provider_ref.clone(), existing.approve_url.clone())
            {
                return Ok(PaymentStarted {
                    payment_id: existing.id,
                    provider_ref,
                    approve_url,
                    fingerprint: fp,
                });
            }
        }
        let client = self.client()?;
        let payment_id = format!("pay_{}", Uuid::now_v7().simple());
        if let Some(row) = journal::claim(
            self.pool(),
            &fp,
            request.site.as_str(),
            "payment.create",
            &payment_id,
        )
        .await?
        {
            // Another call is creating it right now; PayPal's request id makes it one order.
            return Err(EffectError::Unknown(format!(
                "payment.create {} already {}",
                row.fingerprint, row.state
            )));
        }
        let body = json!({
            "intent": "CAPTURE",
            "purchase_units": [{
                "reference_id": request.order_id,
                "custom_id": payment_id,
                "description": request.description.chars().take(127).collect::<String>(),
                "amount": paypal::amount(&request.amount.currency, request.amount.minor),
            }],
            "payment_source": { "paypal": { "experience_context": {
                "brand_name": request.site_name.chars().take(127).collect::<String>(),
                "user_action": "PAY_NOW",
                "shipping_preference": "NO_SHIPPING",
                "return_url": request.return_url,
                "cancel_url": request.cancel_url,
            }}},
        });
        let order = match self
            .paypal_call(
                client,
                &request.site,
                Method::POST,
                "/v2/checkout/orders",
                Some(&body),
                Some(hex_of(&fp)),
            )
            .await
        {
            Ok(order) => order,
            Err(e) => {
                tracing::warn!(error = %e, "effects: PayPal order not created");
                // Creating an order moves no money, and PayPal's request id makes a retry the
                // same order, so the fingerprint is free to try again.
                journal::release(self.pool(), &fp).await?;
                return Err(if e.outcome_unknown() {
                    EffectError::Unknown(e.to_string())
                } else {
                    EffectError::NotExecuted(not_executed_words(&e))
                });
            }
        };
        let provider_ref = order["id"]
            .as_str()
            .ok_or_else(|| EffectError::Internal("PayPal order without id".into()))?
            .to_owned();
        let approve_url = paypal::link(&order, &["payer-action", "approve"])
            .ok_or_else(|| EffectError::Internal("PayPal order without an approve link".into()))?;
        let at = now();
        sqlx::query(
            "INSERT INTO customer_payments (id, site_id, order_id, customer, fingerprint, provider, provider_ref, approve_url, currency, amount_minor, state, sandbox, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'paypal', ?, ?, ?, ?, 'created', ?, ?, ?)",
        )
        .bind(&payment_id)
        .bind(request.site.as_str())
        .bind(&request.order_id)
        .bind(request.customer.as_ref().map(|c| c.as_str()))
        .bind(&fp)
        .bind(&provider_ref)
        .bind(&approve_url)
        .bind(&request.amount.currency)
        .bind(request.amount.minor)
        .bind(self.sandbox())
        .bind(at)
        .bind(at)
        .execute(self.pool())
        .await?;
        let detail = json!({
            "payment": payment_id, "order": request.order_id, "provider": "paypal", "provider_ref": provider_ref,
            "amount": request.amount, "sandbox": self.sandbox(),
        });
        let receipt = self
            .receipt(ReceiptKind::PaymentStarted, &request.site, detail.clone())
            .await;
        journal::finish(
            self.pool(),
            &fp,
            Outcome::Executed,
            Some(&provider_ref),
            &detail,
            receipt.as_deref(),
        )
        .await?;
        Ok(PaymentStarted {
            payment_id,
            provider_ref,
            approve_url,
            fingerprint: fp,
        })
    }

    /// Captures an approved payment, once. A second call returns the first outcome.
    pub async fn capture(
        &self,
        site: &SiteId,
        provider_ref: &str,
    ) -> Result<CaptureOutcome, EffectError> {
        let payment = self
            .payment_by_ref(site, provider_ref)
            .await?
            .ok_or(EffectError::NotFound)?;
        if let Some(done) = settled(&payment) {
            return Ok(done);
        }
        let client = self.client()?;
        let path = format!("/v2/checkout/orders/{provider_ref}/capture");
        let key = format!("capture-{}", payment.id);
        match self
            .paypal_call(client, site, Method::POST, &path, None, Some(&key))
            .await
        {
            Ok(order) => self.apply_order(&payment, &order).await,
            Err(e) if e.has_issue("ORDER_ALREADY_CAPTURED") => {
                self.reconcile(site, provider_ref).await
            }
            Err(e) if e.outcome_unknown() => {
                tracing::warn!(error = %e, payment = %payment.id, "effects: capture outcome unknown");
                Ok(CaptureOutcome {
                    state: CaptureState::Pending,
                    outcome: Outcome::Unknown,
                    captured: None,
                    provider_capture_id: None,
                    receipt: None,
                    message: EffectError::Unknown(String::new()).to_string(),
                })
            }
            Err(e) => {
                let declined = e.has_issue("INSTRUMENT_DECLINED");
                if declined {
                    self.set_state(&payment.id, "declined").await?;
                }
                Ok(CaptureOutcome {
                    state: if declined {
                        CaptureState::Declined
                    } else {
                        CaptureState::Cancelled
                    },
                    outcome: Outcome::NotExecuted,
                    captured: None,
                    provider_capture_id: None,
                    receipt: None,
                    message: not_executed_words(&e),
                })
            }
        }
    }

    /// Reads the order back from PayPal and records what it says (after an unknown outcome,
    /// or when a webhook arrives).
    pub async fn reconcile(
        &self,
        site: &SiteId,
        provider_ref: &str,
    ) -> Result<CaptureOutcome, EffectError> {
        let payment = self
            .payment_by_ref(site, provider_ref)
            .await?
            .ok_or(EffectError::NotFound)?;
        if let Some(done) = settled(&payment) {
            return Ok(done);
        }
        let order = self
            .paypal_call(
                self.client()?,
                site,
                Method::GET,
                &format!("/v2/checkout/orders/{provider_ref}"),
                None,
                None,
            )
            .await
            .map_err(|e| {
                if e.outcome_unknown() {
                    EffectError::Unknown(e.to_string())
                } else {
                    EffectError::NotExecuted(not_executed_words(&e))
                }
            })?;
        self.apply_order(&payment, &order).await
    }

    /// The customer came back through the cancel URL: nothing was charged.
    pub async fn cancel(&self, site: &SiteId, provider_ref: &str) -> Result<(), EffectError> {
        let payment = self
            .payment_by_ref(site, provider_ref)
            .await?
            .ok_or(EffectError::NotFound)?;
        if payment.state == "created" {
            self.set_state(&payment.id, "cancelled").await?;
        }
        Ok(())
    }

    async fn apply_order(
        &self,
        payment: &Payment,
        order: &Value,
    ) -> Result<CaptureOutcome, EffectError> {
        let capture = order["purchase_units"][0]["payments"]["captures"][0].clone();
        let status = capture["status"]
            .as_str()
            .or(order["status"].as_str())
            .unwrap_or("");
        match status {
            "COMPLETED" => {
                let capture_id = capture["id"].as_str().map(str::to_owned);
                let (currency, minor) = paypal::minor_of(&capture["amount"])
                    .unwrap_or((payment.currency.clone(), payment.amount_minor));
                if minor != payment.amount_minor || currency != payment.currency {
                    tracing::error!(payment = %payment.id, minor, expected = payment.amount_minor, "effects: captured amount differs from the order");
                }
                let done = sqlx::query(
                    "UPDATE customer_payments SET state = 'captured', capture_id = ?, captured_minor = ?, updated_at = ? WHERE id = ? AND state IN ('created', 'pending')",
                )
                .bind(&capture_id)
                .bind(minor)
                .bind(now())
                .bind(&payment.id)
                .execute(self.pool())
                .await?;
                let captured = Money { currency, minor };
                let receipt = if done.rows_affected() == 1 {
                    let site = SiteId::new(payment.site_id.clone());
                    let detail = json!({
                        "payment": payment.id, "order": payment.order_id, "provider": "paypal", "provider_ref": payment.provider_ref,
                        "capture": capture_id, "amount": captured, "sandbox": payment.sandbox,
                    });
                    let receipt = self
                        .receipt(ReceiptKind::PaymentCaptured, &site, detail.clone())
                        .await;
                    let fp = format!("{}#capture", payment.fingerprint);
                    if journal::claim(
                        self.pool(),
                        &fp,
                        &payment.site_id,
                        "payment.capture",
                        &payment.id,
                    )
                    .await?
                    .is_none()
                    {
                        journal::finish(
                            self.pool(),
                            &fp,
                            Outcome::Executed,
                            capture_id.as_deref(),
                            &detail,
                            receipt.as_deref(),
                        )
                        .await?;
                    }
                    receipt
                } else {
                    journal::get(self.pool(), &format!("{}#capture", payment.fingerprint))
                        .await?
                        .and_then(|r| r.receipt)
                };
                Ok(CaptureOutcome {
                    state: CaptureState::Captured,
                    outcome: Outcome::Executed,
                    message: format!("Paid {} with PayPal.", words(&captured)),
                    captured: Some(captured),
                    provider_capture_id: capture_id,
                    receipt,
                })
            }
            "PENDING" => {
                self.set_state(&payment.id, "pending").await?;
                Ok(CaptureOutcome {
                    state: CaptureState::Pending,
                    outcome: Outcome::Unknown,
                    captured: None,
                    provider_capture_id: capture["id"].as_str().map(str::to_owned),
                    receipt: None,
                    message: "PayPal is still checking this payment. We'll confirm the order as soon as it clears.".into(),
                })
            }
            "DECLINED" | "FAILED" => {
                self.set_state(&payment.id, "declined").await?;
                Ok(CaptureOutcome {
                    state: CaptureState::Declined,
                    outcome: Outcome::NotExecuted,
                    captured: None,
                    provider_capture_id: None,
                    receipt: None,
                    message: "PayPal declined that payment. Nothing was charged.".into(),
                })
            }
            "VOIDED" => {
                self.set_state(&payment.id, "cancelled").await?;
                Ok(cancelled(
                    "This payment was cancelled on PayPal. Nothing was charged.",
                ))
            }
            _ => Ok(cancelled(
                "You haven't approved the payment on PayPal yet. Nothing was charged.",
            )),
        }
    }

    async fn set_state(&self, payment: &str, state: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE customer_payments SET state = ?, updated_at = ? WHERE id = ?")
            .bind(state)
            .bind(now())
            .bind(payment)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn payment_by_fingerprint(&self, fp: &str) -> Result<Option<Payment>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM customer_payments WHERE fingerprint = ?")
            .bind(fp)
            .fetch_optional(self.pool())
            .await
    }

    async fn payment_by_ref(
        &self,
        site: &SiteId,
        provider_ref: &str,
    ) -> Result<Option<Payment>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM customer_payments WHERE site_id = ? AND provider_ref = ?")
            .bind(site.as_str())
            .bind(provider_ref)
            .fetch_optional(self.pool())
            .await
    }

    /// A payment by id on a site.
    pub async fn payment(
        &self,
        site: &SiteId,
        payment_id: &str,
    ) -> Result<Option<Payment>, EffectError> {
        Ok(
            sqlx::query_as("SELECT * FROM customer_payments WHERE site_id = ? AND id = ?")
                .bind(site.as_str())
                .bind(payment_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    /// Every payment for an order on a site, newest first.
    pub async fn payments_for_order(
        &self,
        site: &SiteId,
        order_id: &str,
    ) -> Result<Vec<Payment>, EffectError> {
        Ok(sqlx::query_as("SELECT * FROM customer_payments WHERE site_id = ? AND order_id = ? ORDER BY created_at DESC, id DESC")
            .bind(site.as_str())
            .bind(order_id)
            .fetch_all(self.pool())
            .await?)
    }

    // ---- refunds --------------------------------------------------------------------------

    /// The effect W5 puts on the owner's refund card. The approval of its fingerprint, and only
    /// that, lets [`Effects::refund`] run.
    pub fn refund_request(
        &self,
        payment: &Payment,
        amount: Money,
        reason: &str,
    ) -> Result<EffectRequest, EffectError> {
        let capture_id = payment.capture_id.clone().ok_or_else(|| {
            EffectError::BadRequest("Only a captured payment can be refunded.".into())
        })?;
        let left = payment.captured_minor.unwrap_or(0) - payment.refunded_minor;
        if amount.currency != payment.currency || amount.minor <= 0 || amount.minor > left {
            return Err(EffectError::BadRequest(format!(
                "A refund here can be at most {}.",
                words(&Money {
                    currency: payment.currency.clone(),
                    minor: left
                })
            )));
        }
        Ok(EffectRequest {
            site: SiteId::new(payment.site_id.clone()),
            webagent: "cashier".into(),
            tool: "payment.refund".into(),
            args: json!({
                "payment_id": payment.id, "order_id": payment.order_id, "capture_id": capture_id,
                "reason": reason.chars().take(200).collect::<String>(), "nonce": Uuid::new_v4().simple().to_string(),
            }),
            amount: Some(amount),
            expiry: Utc::now() + Duration::days(7),
        })
    }

    /// Refunds, once, exactly what the owner approved. `approval` is the proof the frozen
    /// approvals contract returned for this effect's fingerprint (it carries `fingerprint` in
    /// hex and the approver's `account`).
    pub async fn refund(
        &self,
        effect: &EffectRequest,
        approval: &Value,
    ) -> Result<RefundOutcome, EffectError> {
        if effect.tool != "payment.refund" {
            return Err(EffectError::BadRequest("Not a refund.".into()));
        }
        if effect.is_expired(Utc::now()) {
            return Err(EffectError::NotApproved);
        }
        let fp = effect
            .fingerprint()
            .map_err(|e| EffectError::Internal(e.to_string()))?
            .to_string();
        if approval["fingerprint"].as_str() != Some(hex_of(&fp)) {
            return Err(EffectError::NotApproved);
        }
        let amount = effect
            .amount
            .clone()
            .ok_or_else(|| EffectError::BadRequest("A refund needs an amount.".into()))?;
        let payment_id = effect.args["payment_id"].as_str().unwrap_or_default();
        let payment = self
            .payment(&effect.site, payment_id)
            .await?
            .ok_or(EffectError::NotFound)?;
        if let Some(done) = self.refund_by_fingerprint(&fp, &payment).await? {
            return Ok(done);
        }
        let left = payment.captured_minor.unwrap_or(0) - payment.refunded_minor;
        if amount.currency != payment.currency || amount.minor > left {
            return Err(EffectError::BadRequest(format!(
                "A refund here can be at most {}.",
                words(&Money {
                    currency: payment.currency.clone(),
                    minor: left
                })
            )));
        }
        let capture_id = payment.capture_id.clone().ok_or_else(|| {
            EffectError::BadRequest("Only a captured payment can be refunded.".into())
        })?;
        let client = self.client()?;
        let refund_id = format!("rf_{}", Uuid::now_v7().simple());
        if journal::claim(
            self.pool(),
            &fp,
            &payment.site_id,
            "payment.refund",
            &refund_id,
        )
        .await?
        .is_some()
        {
            return Err(EffectError::Unknown("refund already running".into()));
        }
        let body = json!({
            "amount": paypal::amount(&amount.currency, amount.minor),
            "note_to_payer": effect.args["reason"].as_str().unwrap_or("Refund").chars().take(255).collect::<String>(),
        });
        let path = format!("/v2/payments/captures/{capture_id}/refund");
        let approver = approval["account"].as_str().map(str::to_owned).or_else(|| {
            approval["account"]
                .as_object()
                .map(|_| approval["account"].to_string())
        });
        let (outcome, provider_ref) = match self
            .paypal_call(
                client,
                &effect.site,
                Method::POST,
                &path,
                Some(&body),
                Some(hex_of(&fp)),
            )
            .await
        {
            Ok(refund) => {
                let status = refund["status"].as_str().unwrap_or("");
                let outcome = if matches!(status, "COMPLETED" | "PENDING") {
                    Outcome::Executed
                } else {
                    Outcome::NotExecuted
                };
                (outcome, refund["id"].as_str().map(str::to_owned))
            }
            Err(e) if e.outcome_unknown() => (Outcome::Unknown, None),
            Err(e) => {
                tracing::warn!(error = %e, "effects: refund refused");
                (Outcome::NotExecuted, None)
            }
        };
        let detail = json!({
            "payment": payment.id, "order": payment.order_id, "refund": refund_id, "provider_ref": provider_ref,
            "amount": amount, "approved_by": approver, "fingerprint": fp, "sandbox": payment.sandbox,
        });
        let receipt = if outcome == Outcome::Executed {
            self.receipt(ReceiptKind::PaymentRefunded, &effect.site, detail.clone())
                .await
        } else {
            None
        };
        if outcome == Outcome::Executed {
            let refunded = payment.refunded_minor + amount.minor;
            let state = if Some(refunded) >= payment.captured_minor {
                "refunded"
            } else {
                "partly_refunded"
            };
            sqlx::query("UPDATE customer_payments SET refunded_minor = ?, state = ?, updated_at = ? WHERE id = ?")
                .bind(refunded)
                .bind(state)
                .bind(now())
                .bind(&payment.id)
                .execute(self.pool())
                .await?;
        }
        sqlx::query(
            "INSERT INTO refunds (id, payment_id, site_id, fingerprint, amount_minor, currency, state, provider_ref, approved_by, receipt, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&refund_id)
        .bind(&payment.id)
        .bind(&payment.site_id)
        .bind(&fp)
        .bind(amount.minor)
        .bind(&amount.currency)
        .bind(outcome.as_str())
        .bind(&provider_ref)
        .bind(&approver)
        .bind(&receipt)
        .bind(now())
        .execute(self.pool())
        .await?;
        journal::finish(
            self.pool(),
            &fp,
            outcome,
            provider_ref.as_deref(),
            &detail,
            receipt.as_deref(),
        )
        .await?;
        Ok(RefundOutcome {
            refund_id,
            outcome,
            amount,
            provider_ref,
            receipt,
        })
    }

    async fn refund_by_fingerprint(
        &self,
        fp: &str,
        payment: &Payment,
    ) -> Result<Option<RefundOutcome>, EffectError> {
        let row: Option<RefundRow> = sqlx::query_as(
            "SELECT id, amount_minor, currency, state, provider_ref, receipt FROM refunds WHERE fingerprint = ? AND payment_id = ?",
        )
        .bind(fp)
        .bind(&payment.id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|r| RefundOutcome {
            refund_id: r.id,
            outcome: match r.state.as_str() {
                "executed" => Outcome::Executed,
                "unknown" => Outcome::Unknown,
                _ => Outcome::NotExecuted,
            },
            amount: Money {
                currency: r.currency,
                minor: r.amount_minor,
            },
            provider_ref: r.provider_ref,
            receipt: r.receipt,
        }))
    }
}

/// A stored refund.
#[derive(sqlx::FromRow)]
struct RefundRow {
    id: String,
    amount_minor: i64,
    currency: String,
    state: String,
    provider_ref: Option<String>,
    receipt: Option<String>,
}

fn cancelled(message: &str) -> CaptureOutcome {
    CaptureOutcome {
        state: CaptureState::Cancelled,
        outcome: Outcome::NotExecuted,
        captured: None,
        provider_capture_id: None,
        receipt: None,
        message: message.into(),
    }
}

/// The stored outcome of a payment that already ended.
fn settled(payment: &Payment) -> Option<CaptureOutcome> {
    match payment.state.as_str() {
        "captured" | "refunded" | "partly_refunded" => {
            let captured = Money {
                currency: payment.currency.clone(),
                minor: payment.captured_minor.unwrap_or(payment.amount_minor),
            };
            Some(CaptureOutcome {
                state: CaptureState::Captured,
                outcome: Outcome::Executed,
                message: format!("Paid {} with PayPal.", words(&captured)),
                captured: Some(captured),
                provider_capture_id: payment.capture_id.clone(),
                receipt: None,
            })
        }
        "declined" => Some(CaptureOutcome {
            state: CaptureState::Declined,
            outcome: Outcome::NotExecuted,
            captured: None,
            provider_capture_id: None,
            receipt: None,
            message: "PayPal declined that payment. Nothing was charged.".into(),
        }),
        "cancelled" => Some(cancelled(
            "This payment was cancelled. Nothing was charged.",
        )),
        _ => None,
    }
}
