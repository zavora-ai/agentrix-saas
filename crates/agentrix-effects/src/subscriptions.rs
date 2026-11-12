//! Plans in USD through PayPal Subscriptions (James: "PayPal only", "USD for plans"). W3 owns
//! the plan catalogue and the billing row; this keeps PayPal's side: one product and plan per
//! platform plan, version and period, and each subscription's state as PayPal reports it. A plan
//! paid 12 months up front is its own PayPal plan billed once a year for the 12-month total.

use agentrix_types::{Money, ReceiptKind, SiteId};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::paypal;
use crate::{EffectError, Effects, Outcome, fingerprint, journal, now};

/// A platform plan's price, from W3's catalogue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanPrice {
    /// For example `business`.
    pub plan_id: String,
    /// The catalogue version (prices change by version, never in place).
    pub version: u32,
    /// For example "Agentrix Business".
    pub name: String,
    /// Monthly price in USD.
    pub monthly: Money,
    /// Paid 12 months up front: the total PayPal charges once a year (W3's catalogue: 12 times
    /// the monthly price less its annual discount). `None` charges `monthly` every month.
    #[serde(default)]
    pub annual: Option<Money>,
}

impl PlanPrice {
    /// `<plan_id>@<version>`, or `<plan_id>@<version>:annual` for 12 months up front (W3's
    /// billing reads the period back from it).
    fn key(&self) -> String {
        match self.annual {
            Some(_) => format!("{}@{}:annual", self.plan_id, self.version),
            None => format!("{}@{}", self.plan_id, self.version),
        }
    }

    /// What one charge is: the monthly price, or the 12-month total.
    pub fn charge(&self) -> &Money {
        self.annual.as_ref().unwrap_or(&self.monthly)
    }
}

/// An owner starting a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionStart {
    /// The site the plan is for.
    pub site: SiteId,
    /// The plan.
    pub plan: PlanPrice,
    /// The owner's email, to prefill PayPal's page.
    pub subscriber_email: Option<String>,
    /// Where PayPal sends the owner after approving.
    pub return_url: String,
    /// Where PayPal sends the owner who cancels.
    pub cancel_url: String,
    /// The first charge's day (PayPal's `start_time`, in the future), for example go-live day.
    /// `None` charges when the owner approves.
    #[serde(default)]
    pub starts_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A subscription waiting for the owner on PayPal's page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionStarted {
    /// `sub_…`.
    pub subscription_id: String,
    /// PayPal's `I-…`.
    pub provider_ref: String,
    /// PayPal's page.
    pub approve_url: String,
    /// The effect's fingerprint.
    pub fingerprint: String,
}

/// A subscription as PayPal reports it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PlanSubscription {
    /// `sub_…`.
    pub id: String,
    /// The site.
    pub site_id: String,
    /// `<plan_id>@<version>`.
    pub plan_key: String,
    /// PayPal's `I-…`.
    pub provider_ref: Option<String>,
    /// PayPal's page while pending.
    pub approve_url: Option<String>,
    /// pending_approval | active | suspended | cancelled | expired | past_due.
    pub state: String,
    /// USD.
    pub currency: String,
    /// What one charge is: the monthly price, or the 12-month total on an annual plan.
    pub amount_minor: i64,
    /// End of the paid period, when PayPal says.
    pub period_end: Option<i64>,
    /// The last PayPal event applied.
    pub last_event: Option<String>,
    /// PayPal's sandbox.
    pub sandbox: bool,
    /// Created.
    pub created_at: i64,
    /// Updated.
    pub updated_at: i64,
}

#[derive(Serialize)]
struct StartKey<'a> {
    kind: &'static str,
    site: &'a SiteId,
    plan: String,
    amount: &'a Money,
}

impl Effects {
    /// PayPal's plan id for a platform plan and version, made once (product, then plan).
    pub async fn ensure_plan(&self, plan: &PlanPrice) -> Result<String, EffectError> {
        let charge = plan.charge();
        if [&plan.monthly, charge]
            .iter()
            .any(|m| m.currency != "USD" || m.minor <= 0)
        {
            return Err(EffectError::BadRequest(
                "Plans are priced in USD, above zero.".into(),
            ));
        }
        let key = plan.key();
        let existing: Option<(String, i64)> = sqlx::query_as(
            "SELECT paypal_plan_id, amount_minor FROM paypal_plans WHERE plan_key = ?",
        )
        .bind(&key)
        .fetch_optional(self.pool())
        .await?;
        if let Some((id, minor)) = existing {
            if minor != charge.minor {
                return Err(EffectError::BadRequest(format!(
                    "{key} already has another price; prices change by version."
                )));
            }
            return Ok(id);
        }
        let client = self.client()?;
        let (name, description, unit) = match plan.annual {
            Some(_) => (
                format!("{}, 12 months", plan.name),
                format!(
                    "{} plan for one website, paid 12 months at a time",
                    plan.name
                ),
                "YEAR",
            ),
            None => (
                plan.name.clone(),
                format!("{} plan for one website", plan.name),
                "MONTH",
            ),
        };
        let product = client
            .request(
                Method::POST,
                "/v1/catalogs/products",
                Some(&json!({ "name": name, "type": "SERVICE", "category": "SOFTWARE", "description": description })),
                Some(&format!("product-{key}")),
            )
            .await
            .map_err(|e| EffectError::NotExecuted(format!("PayPal didn't take the plan: {e}")))?;
        let product_id = product["id"]
            .as_str()
            .ok_or_else(|| EffectError::Internal("PayPal product without id".into()))?
            .to_owned();
        let body = json!({
            "product_id": product_id,
            "name": name,
            "status": "ACTIVE",
            "billing_cycles": [{
                "frequency": { "interval_unit": unit, "interval_count": 1 },
                "tenure_type": "REGULAR", "sequence": 1, "total_cycles": 0,
                "pricing_scheme": { "fixed_price": paypal::amount("USD", charge.minor) },
            }],
            "payment_preferences": { "auto_bill_outstanding": true, "payment_failure_threshold": 3 },
        });
        let created = client
            .request(
                Method::POST,
                "/v1/billing/plans",
                Some(&body),
                Some(&format!("plan-{key}")),
            )
            .await
            .map_err(|e| EffectError::NotExecuted(format!("PayPal didn't take the plan: {e}")))?;
        let paypal_plan_id = created["id"]
            .as_str()
            .ok_or_else(|| EffectError::Internal("PayPal plan without id".into()))?
            .to_owned();
        sqlx::query("INSERT OR IGNORE INTO paypal_plans (plan_key, product_id, paypal_plan_id, currency, amount_minor, created_at) VALUES (?, ?, ?, 'USD', ?, ?)")
            .bind(&key)
            .bind(&product_id)
            .bind(&paypal_plan_id)
            .bind(charge.minor)
            .bind(now())
            .execute(self.pool())
            .await?;
        Ok(paypal_plan_id)
    }

    /// Starts a subscription; the owner approves it on PayPal's page. Idempotent per site, plan,
    /// period and price. An annual plan's first charge is the 12-month total, once.
    pub async fn start_subscription(
        &self,
        start: SubscriptionStart,
    ) -> Result<SubscriptionStarted, EffectError> {
        let fp = fingerprint(&StartKey {
            kind: "subscription.create",
            site: &start.site,
            plan: start.plan.key(),
            amount: start.plan.charge(),
        })?
        .to_string();
        let existing: Option<PlanSubscription> = sqlx::query_as(
            "SELECT s.* FROM plan_subscriptions s JOIN effects e ON e.subject = s.id WHERE e.fingerprint = ? AND s.state = 'pending_approval'",
        )
        .bind(&fp)
        .fetch_optional(self.pool())
        .await?;
        if let Some(s) = existing {
            if let (Some(provider_ref), Some(approve_url)) = (s.provider_ref, s.approve_url) {
                return Ok(SubscriptionStarted {
                    subscription_id: s.id,
                    provider_ref,
                    approve_url,
                    fingerprint: fp,
                });
            }
        }
        let paypal_plan_id = self.ensure_plan(&start.plan).await?;
        let client = self.client()?;
        let subscription_id = format!("sub_{}", Uuid::now_v7().simple());
        journal::release(self.pool(), &fp).await?;
        if journal::claim(
            self.pool(),
            &fp,
            start.site.as_str(),
            "subscription.create",
            &subscription_id,
        )
        .await?
        .is_some()
        {
            return Err(EffectError::Unknown("subscription already starting".into()));
        }
        let mut body = json!({
            "plan_id": paypal_plan_id,
            "custom_id": subscription_id,
            "application_context": {
                "brand_name": "Agentrix",
                "user_action": "SUBSCRIBE_NOW",
                "shipping_preference": "NO_SHIPPING",
                "return_url": start.return_url,
                "cancel_url": start.cancel_url,
            },
        });
        if let Some(starts_at) = start.starts_at {
            if starts_at <= chrono::Utc::now() {
                return Err(EffectError::BadRequest(
                    "A plan's first charge can't be in the past.".into(),
                ));
            }
            body["start_time"] =
                json!(starts_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        }
        if let Some(email) = &start.subscriber_email {
            body["subscriber"] = json!({ "email_address": email });
        }
        let created = match client
            .request(
                Method::POST,
                "/v1/billing/subscriptions",
                Some(&body),
                Some(fp.trim_start_matches("sha256:")),
            )
            .await
        {
            Ok(created) => created,
            Err(e) => {
                journal::release(self.pool(), &fp).await?;
                return Err(if e.outcome_unknown() {
                    EffectError::Unknown(e.to_string())
                } else {
                    EffectError::NotExecuted(format!("PayPal didn't start the plan: {e}"))
                });
            }
        };
        let provider_ref = created["id"]
            .as_str()
            .ok_or_else(|| EffectError::Internal("PayPal subscription without id".into()))?
            .to_owned();
        let approve_url =
            paypal::link(&created, &["approve", "payer-action"]).ok_or_else(|| {
                EffectError::Internal("PayPal subscription without an approve link".into())
            })?;
        let at = now();
        sqlx::query(
            "INSERT INTO plan_subscriptions (id, site_id, plan_key, provider_ref, approve_url, state, currency, amount_minor, sandbox, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'pending_approval', 'USD', ?, ?, ?, ?)",
        )
        .bind(&subscription_id)
        .bind(start.site.as_str())
        .bind(start.plan.key())
        .bind(&provider_ref)
        .bind(&approve_url)
        .bind(start.plan.charge().minor)
        .bind(self.sandbox())
        .bind(at)
        .bind(at)
        .execute(self.pool())
        .await?;
        let detail = json!({ "subscription": subscription_id, "plan": start.plan.key(), "provider_ref": provider_ref, "state": "pending_approval", "amount": start.plan.charge(), "every": if start.plan.annual.is_some() { "12 months" } else { "month" }, "sandbox": self.sandbox() });
        let receipt = self
            .receipt(
                ReceiptKind::SubscriptionChanged,
                &start.site,
                detail.clone(),
            )
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
        Ok(SubscriptionStarted {
            subscription_id,
            provider_ref,
            approve_url,
            fingerprint: fp,
        })
    }

    /// Reads a subscription back from PayPal and records its state (after the owner returns
    /// from PayPal's page, or on a webhook).
    pub async fn refresh_subscription(
        &self,
        provider_ref: &str,
    ) -> Result<PlanSubscription, EffectError> {
        let current = self
            .subscription_by_ref(provider_ref)
            .await?
            .ok_or(EffectError::NotFound)?;
        let paypal = self
            .client()?
            .request(
                Method::GET,
                &format!("/v1/billing/subscriptions/{provider_ref}"),
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
        let state = match paypal["status"].as_str().unwrap_or("") {
            "ACTIVE" => "active",
            "SUSPENDED" => "suspended",
            "CANCELLED" => "cancelled",
            "EXPIRED" => "expired",
            _ => "pending_approval",
        };
        let period_end = paypal["billing_info"]["next_billing_time"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp());
        // A plan or period the owner switched to (revise_subscription) shows as PayPal's plan
        // id once they agreed: the key and the amount follow it, so W3's billing does too.
        if let Some(paypal_plan) = paypal["plan_id"].as_str() {
            let known: Option<(String, i64)> = sqlx::query_as(
                "SELECT plan_key, amount_minor FROM paypal_plans WHERE paypal_plan_id = ?",
            )
            .bind(paypal_plan)
            .fetch_optional(self.pool())
            .await?;
            if let Some((key, amount)) = known
                && (key != current.plan_key || amount != current.amount_minor)
            {
                sqlx::query("UPDATE plan_subscriptions SET plan_key = ?, amount_minor = ?, updated_at = ? WHERE id = ?")
                    .bind(&key)
                    .bind(amount)
                    .bind(now())
                    .bind(&current.id)
                    .execute(self.pool())
                    .await?;
            }
        }
        self.set_subscription_state(&current, state, period_end, "refresh")
            .await
    }

    /// Asks PayPal to move a live subscription to `plan` (another plan, or the same plan paid
    /// monthly or for 12 months) from its next billing cycle. The owner agrees on PayPal's page:
    /// this returns that page. Nothing changes until they do; then [`Effects::refresh_subscription`]
    /// reads the new plan back.
    pub async fn revise_subscription(
        &self,
        site: &SiteId,
        subscription_id: &str,
        plan: &PlanPrice,
        return_url: &str,
        cancel_url: &str,
    ) -> Result<String, EffectError> {
        let current: PlanSubscription =
            sqlx::query_as("SELECT * FROM plan_subscriptions WHERE site_id = ? AND id = ?")
                .bind(site.as_str())
                .bind(subscription_id)
                .fetch_optional(self.pool())
                .await?
                .ok_or(EffectError::NotFound)?;
        if current.state != "active" {
            return Err(EffectError::BadRequest(
                "Only an active plan can be switched; this one isn't active.".into(),
            ));
        }
        let provider_ref = current.provider_ref.clone().ok_or(EffectError::NotFound)?;
        let paypal_plan_id = self.ensure_plan(plan).await?;
        let revised = self
            .client()?
            .request(
                Method::POST,
                &format!("/v1/billing/subscriptions/{provider_ref}/revise"),
                Some(&json!({
                    "plan_id": paypal_plan_id,
                    "application_context": {
                        "brand_name": "Agentrix",
                        "shipping_preference": "NO_SHIPPING",
                        "return_url": return_url,
                        "cancel_url": cancel_url,
                    },
                })),
                None,
            )
            .await
            .map_err(|e| {
                if e.outcome_unknown() {
                    EffectError::Unknown(e.to_string())
                } else {
                    EffectError::NotExecuted(format!("PayPal didn't take the switch: {e}"))
                }
            })?;
        let approve = paypal::link(&revised, &["approve", "payer-action"]).ok_or_else(|| {
            EffectError::Internal("PayPal's revise came back without an approve link".into())
        })?;
        let detail = json!({ "subscription": current.id, "provider_ref": provider_ref, "from": current.plan_key, "to": plan.key(), "amount": plan.charge(), "state": "waiting_for_owner", "sandbox": current.sandbox });
        self.receipt(ReceiptKind::SubscriptionChanged, site, detail)
            .await;
        Ok(approve)
    }

    pub(crate) async fn set_subscription_state(
        &self,
        current: &PlanSubscription,
        state: &str,
        period_end: Option<i64>,
        event: &str,
    ) -> Result<PlanSubscription, EffectError> {
        sqlx::query("UPDATE plan_subscriptions SET state = ?, period_end = COALESCE(?, period_end), last_event = ?, updated_at = ? WHERE id = ?")
            .bind(state)
            .bind(period_end)
            .bind(event)
            .bind(now())
            .bind(&current.id)
            .execute(self.pool())
            .await?;
        if state != current.state {
            let site = SiteId::new(current.site_id.clone());
            let detail = json!({ "subscription": current.id, "plan": current.plan_key, "provider_ref": current.provider_ref, "from": current.state, "state": state, "event": event, "sandbox": current.sandbox });
            self.receipt(ReceiptKind::SubscriptionChanged, &site, detail)
                .await;
        }
        self.subscription_by_ref(current.provider_ref.as_deref().unwrap_or_default())
            .await?
            .ok_or(EffectError::NotFound)
    }

    /// Cancels a subscription at PayPal (an owner decision W3 or W5 raises and approves first).
    pub async fn cancel_subscription(
        &self,
        site: &SiteId,
        subscription_id: &str,
        reason: &str,
    ) -> Result<PlanSubscription, EffectError> {
        let current = self.site_subscription(site, subscription_id).await?;
        if matches!(current.state.as_str(), "cancelled" | "expired") {
            return Ok(current);
        }
        self.subscription_action(&current, "cancel", reason).await?;
        self.set_subscription_state(&current, "cancelled", None, "cancel")
            .await
    }

    /// Suspends a live subscription at PayPal: no more charges until it is activated again.
    /// Finance does this after a failed payment. Suspending one already suspended changes
    /// nothing.
    pub async fn suspend_subscription(
        &self,
        site: &SiteId,
        subscription_id: &str,
        reason: &str,
    ) -> Result<PlanSubscription, EffectError> {
        let current = self.site_subscription(site, subscription_id).await?;
        match current.state.as_str() {
            "suspended" => return Ok(current),
            "active" | "past_due" => {}
            _ => {
                return Err(EffectError::BadRequest(
                    "Only an active plan can be paused; this one isn't active.".into(),
                ));
            }
        }
        self.subscription_action(&current, "suspend", reason)
            .await?;
        self.set_subscription_state(&current, "suspended", None, "suspend")
            .await
    }

    /// Activates a suspended subscription at PayPal again: charges resume on its billing
    /// cycle. Activating one already active changes nothing.
    pub async fn activate_subscription(
        &self,
        site: &SiteId,
        subscription_id: &str,
        reason: &str,
    ) -> Result<PlanSubscription, EffectError> {
        let current = self.site_subscription(site, subscription_id).await?;
        match current.state.as_str() {
            "active" => return Ok(current),
            "suspended" => {}
            _ => {
                return Err(EffectError::BadRequest(
                    "Only a paused plan can be started again; this one isn't paused.".into(),
                ));
            }
        }
        self.subscription_action(&current, "activate", reason)
            .await?;
        self.set_subscription_state(&current, "active", None, "activate")
            .await
    }

    async fn site_subscription(
        &self,
        site: &SiteId,
        subscription_id: &str,
    ) -> Result<PlanSubscription, EffectError> {
        sqlx::query_as("SELECT * FROM plan_subscriptions WHERE site_id = ? AND id = ?")
            .bind(site.as_str())
            .bind(subscription_id)
            .fetch_optional(self.pool())
            .await?
            .ok_or(EffectError::NotFound)
    }

    /// `POST /v1/billing/subscriptions/{id}/{cancel|suspend|activate}` with a reason (PayPal
    /// answers 204).
    async fn subscription_action(
        &self,
        current: &PlanSubscription,
        action: &str,
        reason: &str,
    ) -> Result<(), EffectError> {
        let provider_ref = current
            .provider_ref
            .as_deref()
            .ok_or(EffectError::NotFound)?;
        self.client()?
            .request(
                Method::POST,
                &format!("/v1/billing/subscriptions/{provider_ref}/{action}"),
                Some(&json!({ "reason": reason.chars().take(128).collect::<String>() })),
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
        Ok(())
    }

    pub(crate) async fn subscription_by_ref(
        &self,
        provider_ref: &str,
    ) -> Result<Option<PlanSubscription>, sqlx::Error> {
        sqlx::query_as("SELECT * FROM plan_subscriptions WHERE provider_ref = ?")
            .bind(provider_ref)
            .fetch_optional(self.pool())
            .await
    }

    /// A site's subscriptions, newest first (for W3's Money pages and W5's billing card).
    pub async fn subscriptions_for_site(
        &self,
        site: &SiteId,
    ) -> Result<Vec<PlanSubscription>, EffectError> {
        Ok(sqlx::query_as(
            "SELECT * FROM plan_subscriptions WHERE site_id = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(site.as_str())
        .fetch_all(self.pool())
        .await?)
    }
}
