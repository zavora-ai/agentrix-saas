//! Platform billing in USD (docs/08-control-plane.md §4.2).
//!
//! - **Plans are data**: [`catalogue`] reads `data/plans.json` (01 §6.1's hypothesis prices),
//!   including the discount on a plan paid 12 months up front ([`Catalogue::annual_discount_percent`]),
//!   the one number every page and the agent door quote it from.
//! - **Paid monthly or for 12 months** ([`Period`]): an annual plan is charged once, its
//!   [`Plan::annual_cents`], and renews 12 months later. The AI included still resets monthly.
//! - **Subscriptions** are PayPal Subscriptions run by W6's Effects Gateway. When it creates one it
//!   gives us the provider reference ([`Billing::start`]); each verified provider event comes back
//!   through [`Billing::on_event`]. Money never moves in this crate.
//! - **Usage** is rated against each plan's included AI budget ([`Billing::usage`]). Use above it
//!   is billed only when the owner raised their cap on a decision card; none has, so none is.
//! - **Invoices** are issued by an invoice run that `op.finance` approves ([`Billing::draft_run`],
//!   [`Billing::issue_run`]); a provider payment marks one paid.
//! - **The Money page** reads [`Billing::money`]: MRR from active subscriptions, collections from
//!   payments, outstanding invoices, reminders by the dunning policy, inference cost.
//!
//! Tables: `agentrix-node/src/db/migrations/0310_billing.sql` and `0350_billing_period.sql`
//! (W3's range).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::{DateTime, Datelike, Months, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

// ---------- The plan catalogue ----------

/// The plan catalogue (`plans.toml` in the design; JSON here, so no new dependency).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalogue {
    /// Bumped whenever a price or an allowance changes. Subscriptions keep the version they
    /// started on.
    pub version: u32,
    /// Always USD for platform billing (James, 8 Nov).
    pub currency: String,
    /// Where the numbers come from.
    pub source: String,
    /// The discount on a site plan paid 12 months up front, in percent ("a 12-month plan comes
    /// with a 20% discount"). The only copy of the number: the home page, the price list, the
    /// agent door and the plan page all read it here.
    pub annual_discount_percent: i64,
    /// The plans, cheapest first.
    pub plans: Vec<Plan>,
}

/// One plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// `essential`, `business`, `commerce`, `agency`.
    pub id: String,
    /// Shown to people.
    pub name: String,
    /// `site` (an owner's plan) or `wholesale` (per client site, for agencies).
    pub kind: String,
    /// Price per interval, US cents.
    pub price_cents: i64,
    /// `month`.
    pub interval: String,
    /// The AI budget included each month, micro-USD.
    pub included_ai_micro_usd: i64,
    /// Wholesale only: the fewest sites billed.
    #[serde(default)]
    pub minimum_sites: Option<u32>,
    /// The WebAgents on the plan.
    pub webagents: Vec<String>,
    /// Plain-words lines for plan cards.
    pub includes: Vec<String>,
    /// The AI included in a subscription's first 30 days instead of the monthly allowance (the
    /// launch offer), micro-USD; 0 when there's no offer.
    #[serde(default)]
    pub first_month_ai_micro_usd: i64,
    /// WebAgents announced for the plan that aren't built yet: shown as coming, never as included.
    #[serde(default)]
    pub webagents_soon: Vec<PlannedWebAgent>,
}

/// A WebAgent a plan will include once it's built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedWebAgent {
    /// "Marketing".
    pub name: String,
    /// What it will do, in plain words.
    pub role: String,
}

/// How often a plan is paid: each month, or 12 months up front at the catalogue's discount.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    /// Charged every month.
    #[default]
    Monthly,
    /// Charged once for 12 months, less [`Catalogue::annual_discount_percent`].
    Annual,
}

impl Period {
    /// `monthly` or `annual`, as stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Period::Monthly => "monthly",
            Period::Annual => "annual",
        }
    }

    /// From its stored name; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "monthly" => Some(Period::Monthly),
            "annual" => Some(Period::Annual),
            _ => None,
        }
    }

    /// Months one charge pays for: 1 or 12.
    pub fn months(self) -> u32 {
        match self {
            Period::Monthly => 1,
            Period::Annual => 12,
        }
    }

    /// The next charge after a charge at `at`: one month or 12 months later, on the same day
    /// (the month's last day when it's shorter).
    pub fn renewal(self, at: DateTime<Utc>) -> DateTime<Utc> {
        at.checked_add_months(Months::new(self.months()))
            .unwrap_or(at)
    }
}

/// The price of 12 months paid up front: 12 times `monthly_cents`, less `discount_percent`,
/// rounded to the nearest cent.
pub fn annual_cents(monthly_cents: i64, discount_percent: i64) -> i64 {
    let hundredths = monthly_cents * 12 * (100 - discount_percent.clamp(0, 100));
    (hundredths + 50).div_euclid(100)
}

impl Plan {
    /// 12 months paid up front: 12 times the monthly price less the catalogue's discount,
    /// rounded to the cent ($29 a month is $278.40 for 12 months at 20% off).
    pub fn annual_cents(&self) -> i64 {
        annual_cents(self.price_cents, catalogue().annual_discount_percent)
    }

    /// What 12 months paid up front saves against 12 monthly charges.
    pub fn annual_saving_cents(&self) -> i64 {
        self.price_cents * 12 - self.annual_cents()
    }

    /// What one charge is for `period`: the monthly price, or the 12-month total.
    pub fn charge_cents(&self, period: Period) -> i64 {
        match period {
            Period::Monthly => self.price_cents,
            Period::Annual => self.annual_cents(),
        }
    }

    /// The AI included for a subscription started at `started`, as of `now`: the first-month
    /// offer for its first 30 days, then the monthly allowance. The same for monthly and annual
    /// plans: a plan paid for 12 months still gets its AI allowance month by month, never 12
    /// months of it at once.
    pub fn included_ai(&self, started: Option<DateTime<Utc>>, now: DateTime<Utc>) -> i64 {
        match started {
            Some(at) if now - at < chrono::Duration::days(30) => self
                .first_month_ai_micro_usd
                .max(self.included_ai_micro_usd),
            _ => self.included_ai_micro_usd,
        }
    }
}

/// The catalogue compiled into this build.
pub fn catalogue() -> &'static Catalogue {
    static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        serde_json::from_str(include_str!("../data/plans.json")).expect("data/plans.json is valid")
    })
}

/// A plan by id.
pub fn plan(id: &str) -> Option<&'static Plan> {
    catalogue().plans.iter().find(|p| p.id == id)
}

// ---------- Errors and times ----------

/// Why a billing call failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The database refused.
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    /// No plan with that id.
    #[error("there's no plan called {0}")]
    UnknownPlan(String),
    /// No subscription with that provider reference.
    #[error("no subscription has the provider reference {0}")]
    UnknownSubscription(String),
    /// The site already has a live subscription.
    #[error("this site already has a subscription")]
    AlreadySubscribed,
    /// The run changed since it was shown, or was already issued.
    #[error("the invoice run changed since it was drafted; draft it again")]
    RunChanged,
}

fn time(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn month_start(at: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(at)
}

fn next_month(at: DateTime<Utc>) -> DateTime<Utc> {
    let (y, m) = if at.month() == 12 {
        (at.year() + 1, 1)
    } else {
        (at.year(), at.month() + 1)
    };
    Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
        .single()
        .unwrap_or(at)
}

/// `2026-11` for a time.
pub fn period_of(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}

// ---------- Subscriptions ----------

/// A subscription row.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Subscription {
    /// `sub_…`.
    pub subscription_id: String,
    /// The site.
    pub site_id: String,
    /// The plan.
    pub plan_id: String,
    /// The catalogue version it started on.
    pub plan_version: i64,
    /// What one charge is, cents: the monthly price, or the 12-month total for an annual plan.
    pub price_cents: i64,
    /// `monthly` or `annual` ([`Period`]).
    pub period: String,
    /// `paypal`.
    pub rail: String,
    /// PayPal's `I-…` id.
    pub provider_ref: Option<String>,
    /// `pending_approval`, `active`, `past_due`, `suspended`, `cancelled`, `expired`.
    pub state: String,
    /// The paid period ends.
    pub period_end: Option<String>,
    /// First failed payment not yet recovered.
    pub past_due_since: Option<String>,
    /// Go-live day, when the first charge happens.
    pub starts_at: Option<String>,
    /// Made.
    pub created_at: String,
    /// Last change.
    pub updated_at: String,
}

impl Subscription {
    /// How often it's charged; a row from before periods existed is monthly.
    pub fn period(&self) -> Period {
        Period::parse(&self.period).unwrap_or_default()
    }

    /// The next charge's day: the end of the paid period PayPal reported, or, before that, one
    /// period after the first charge.
    pub fn next_charge(&self) -> Option<DateTime<Utc>> {
        self.period_end.as_deref().and_then(parse).or_else(|| {
            self.starts_at
                .as_deref()
                .and_then(parse)
                .map(|at| self.period().renewal(at))
        })
    }
}

/// A verified provider event, as W6's Effects Gateway reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionEvent {
    /// PayPal's subscription id.
    pub provider_ref: String,
    /// What happened.
    pub kind: EventKind,
    /// For a payment: the amount, US cents.
    #[serde(default)]
    pub amount_cents: Option<i64>,
    /// For a payment: PayPal's payment id.
    #[serde(default)]
    pub provider_payment_id: Option<String>,
    /// When it happened.
    pub at: DateTime<Utc>,
    /// The end of the period now paid for.
    #[serde(default)]
    pub period_end: Option<DateTime<Utc>>,
    /// The Effects Gateway's receipt digest for the event.
    #[serde(default)]
    pub receipt: Option<String>,
}

/// The provider events W6 forwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The owner approved on PayPal's page.
    Activated,
    /// A period was paid.
    PaymentCompleted,
    /// A payment failed.
    PaymentFailed,
    /// PayPal suspended it.
    Suspended,
    /// Cancelled.
    Cancelled,
    /// It ran its course.
    Expired,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            EventKind::Activated => "activated",
            EventKind::PaymentCompleted => "payment_completed",
            EventKind::PaymentFailed => "payment_failed",
            EventKind::Suspended => "suspended",
            EventKind::Cancelled => "cancelled",
            EventKind::Expired => "expired",
        }
    }
}

/// What an event did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// It changed the subscription; the new state.
    Changed(String),
    /// Seen before; nothing changed.
    Duplicate,
}

macro_rules! sub_select {
    ($rest:literal) => {
        concat!(
            "SELECT subscription_id, site_id, plan_id, plan_version, price_cents, period, rail, provider_ref, state, period_end, past_due_since, starts_at, created_at, updated_at FROM subscriptions ",
            $rest
        )
    };
}

/// Billing over `control.db`.
#[derive(Debug, Clone)]
pub struct Billing {
    pool: SqlitePool,
}

impl Billing {
    /// Billing over a pool migrated through 0310.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Records a monthly subscription W6's Effects Gateway created on PayPal for `site` on
    /// `plan_id`, waiting for the owner to approve it on PayPal's page.
    pub async fn start(
        &self,
        site: &str,
        plan_id: &str,
        provider_ref: &str,
        receipt: Option<&str>,
    ) -> Result<Subscription, Error> {
        self.start_for(site, plan_id, Period::Monthly, provider_ref, receipt)
            .await
    }

    /// [`Billing::start`] for a plan paid monthly or for 12 months: the row keeps the period
    /// and what one charge is ([`Plan::charge_cents`]).
    pub async fn start_for(
        &self,
        site: &str,
        plan_id: &str,
        period: Period,
        provider_ref: &str,
        receipt: Option<&str>,
    ) -> Result<Subscription, Error> {
        let plan = plan(plan_id).ok_or_else(|| Error::UnknownPlan(plan_id.to_owned()))?;
        if self
            .for_site(site)
            .await?
            .is_some_and(|s| ["pending_approval", "active", "past_due"].contains(&s.state.as_str()))
        {
            return Err(Error::AlreadySubscribed);
        }
        let now = time(Utc::now());
        let id = format!("sub_{}", Uuid::now_v7().simple());
        sqlx::query(
            "INSERT INTO subscriptions (subscription_id, site_id, plan_id, plan_version, price_cents, period, provider_ref, state, receipt, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'pending_approval', ?, ?, ?)",
        )
        .bind(&id)
        .bind(site)
        .bind(&plan.id)
        .bind(i64::from(catalogue().version))
        .bind(plan.charge_cents(period))
        .bind(period.as_str())
        .bind(provider_ref)
        .bind(receipt)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(self.by_ref(provider_ref).await?.expect("just inserted"))
    }

    /// The site's newest subscription.
    pub async fn for_site(&self, site: &str) -> Result<Option<Subscription>, Error> {
        Ok(sqlx::query_as(sub_select!(
            "WHERE site_id = ? ORDER BY created_at DESC LIMIT 1"
        ))
        .bind(site)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn by_ref(&self, provider_ref: &str) -> Result<Option<Subscription>, Error> {
        Ok(sqlx::query_as(sub_select!("WHERE provider_ref = ?"))
            .bind(provider_ref)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Every subscription, newest first.
    pub async fn all(&self) -> Result<Vec<Subscription>, Error> {
        Ok(
            sqlx::query_as(sub_select!("ORDER BY created_at DESC LIMIT 500"))
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Brings in the provider's side: W6's Effects Gateway keeps PayPal's view of each plan
    /// subscription in `plan_subscriptions` (and payments in `plan_payments`, once it records
    /// them). Rows are matched by subscription id; state, price and period follow the provider.
    /// A site whose release is live counts as gone live from its first signed release, so the
    /// invoice run never waits on wiring. Tables that don't exist yet are skipped. Returns how
    /// many subscriptions it brought in or changed.
    pub async fn sync(&self) -> Result<usize, Error> {
        let mut changed = 0;
        if table_exists(&self.pool, "plan_subscriptions").await? {
            type Row = (
                String,
                String,
                String,
                Option<String>,
                String,
                i64,
                Option<i64>,
                i64,
                i64,
            );
            let rows: Vec<Row> = sqlx::query_as(
                "SELECT id, site_id, plan_key, provider_ref, state, amount_minor, period_end, created_at, updated_at FROM plan_subscriptions WHERE currency = 'USD'",
            )
            .fetch_all(&self.pool)
            .await?;
            for (id, site, plan_key, provider_ref, state, amount, period_end, created, updated) in
                rows
            {
                let (plan_id, version, period) = plan_key_parts(&plan_key);
                let unix = |s: i64| Utc.timestamp_opt(s, 0).single().map(time);
                let state = match state.as_str() {
                    "pending_approval" | "active" | "past_due" | "suspended" | "cancelled"
                    | "expired" => state,
                    _ => continue,
                };
                let done = sqlx::query(
                    "INSERT INTO subscriptions (subscription_id, site_id, plan_id, plan_version, price_cents, period, provider_ref, state, period_end, created_at, updated_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                     ON CONFLICT (subscription_id) DO UPDATE SET state = excluded.state, price_cents = excluded.price_cents, period = excluded.period, \
                       provider_ref = excluded.provider_ref, period_end = excluded.period_end, updated_at = excluded.updated_at, \
                       past_due_since = CASE WHEN excluded.state = 'past_due' THEN COALESCE(subscriptions.past_due_since, excluded.updated_at) ELSE NULL END \
                     WHERE subscriptions.state <> excluded.state OR subscriptions.price_cents <> excluded.price_cents OR subscriptions.period <> excluded.period \
                       OR COALESCE(subscriptions.period_end, '') <> COALESCE(excluded.period_end, '') OR subscriptions.provider_ref IS NOT excluded.provider_ref",
                )
                .bind(&id)
                .bind(&site)
                .bind(plan_id)
                .bind(version)
                .bind(amount)
                .bind(period.as_str())
                .bind(&provider_ref)
                .bind(&state)
                .bind(period_end.and_then(unix))
                .bind(unix(created).unwrap_or_else(|| time(Utc::now())))
                .bind(unix(updated).unwrap_or_else(|| time(Utc::now())))
                .execute(&self.pool)
                .await?;
                changed += usize::try_from(done.rows_affected()).unwrap_or(0);
            }
        }
        if table_exists(&self.pool, "plan_payments").await? {
            type Pay = (String, String, i64, i64);
            let rows: Vec<Pay> = sqlx::query_as("SELECT provider_ref, provider_payment_id, amount_minor, paid_at FROM plan_payments")
                .fetch_all(&self.pool)
                .await?;
            for (provider_ref, payment_id, amount, at) in rows {
                let Some(at) = Utc.timestamp_opt(at, 0).single() else {
                    continue;
                };
                let ev = SubscriptionEvent {
                    provider_ref,
                    kind: EventKind::PaymentCompleted,
                    amount_cents: Some(amount),
                    provider_payment_id: Some(payment_id),
                    at,
                    period_end: None,
                    receipt: None,
                };
                if let Ok(Applied::Changed(_)) = self.on_event(&ev).await {
                    changed += 1;
                }
            }
        }
        // Go-live from the records: the first signed release of a site the edge serves.
        sqlx::query(
            "UPDATE subscriptions SET starts_at = (SELECT MIN(r.signed_at) FROM releases r WHERE r.site_id = subscriptions.site_id AND r.signed_at IS NOT NULL) \
             WHERE starts_at IS NULL AND site_id IN (SELECT site_id FROM site_pointer)",
        )
        .execute(&self.pool)
        .await?;
        Ok(changed)
    }

    /// Records go-live day for the site's subscription: the first charge is then.
    pub async fn went_live(&self, site: &str, at: DateTime<Utc>) -> Result<(), Error> {
        sqlx::query("UPDATE subscriptions SET starts_at = COALESCE(starts_at, ?), updated_at = ? WHERE site_id = ? AND state IN ('pending_approval', 'active')")
            .bind(time(at))
            .bind(time(Utc::now()))
            .bind(site)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Applies a verified provider event. Repeats are recognised and change nothing.
    pub async fn on_event(&self, ev: &SubscriptionEvent) -> Result<Applied, Error> {
        let sub = self
            .by_ref(&ev.provider_ref)
            .await?
            .ok_or_else(|| Error::UnknownSubscription(ev.provider_ref.clone()))?;
        let inserted = sqlx::query(
            "INSERT INTO subscription_events (subscription_id, provider_ref, kind, amount_cents, provider_payment_id, at, period_end, receipt) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(&sub.subscription_id)
        .bind(&ev.provider_ref)
        .bind(ev.kind.as_str())
        .bind(ev.amount_cents)
        .bind(&ev.provider_payment_id)
        .bind(time(ev.at))
        .bind(ev.period_end.map(time))
        .bind(&ev.receipt)
        .execute(&self.pool)
        .await?;
        if inserted.rows_affected() == 0 {
            return Ok(Applied::Duplicate);
        }
        // A payment the provider sent without its period end pays for one period: a month, or
        // 12 months on an annual plan.
        let period_end = ev.period_end.or_else(|| {
            (ev.kind == EventKind::PaymentCompleted).then(|| sub.period().renewal(ev.at))
        });
        let now = time(Utc::now());
        let state = match ev.kind {
            EventKind::Activated => "active",
            EventKind::PaymentCompleted => {
                self.record_payment(&sub, ev).await?;
                "active"
            }
            EventKind::PaymentFailed => "past_due",
            EventKind::Suspended => "suspended",
            EventKind::Cancelled => "cancelled",
            EventKind::Expired => "expired",
        };
        let past_due = match ev.kind {
            EventKind::PaymentFailed => {
                Some(sub.past_due_since.clone().unwrap_or_else(|| time(ev.at)))
            }
            EventKind::PaymentCompleted | EventKind::Activated => None,
            _ => sub.past_due_since.clone(),
        };
        sqlx::query("UPDATE subscriptions SET state = ?, past_due_since = ?, period_end = COALESCE(?, period_end), updated_at = ? WHERE subscription_id = ?")
            .bind(state)
            .bind(past_due)
            .bind(period_end.map(time))
            .bind(&now)
            .bind(&sub.subscription_id)
            .execute(&self.pool)
            .await?;
        Ok(Applied::Changed(state.to_owned()))
    }

    /// A payment: marks the oldest unpaid invoice paid, or records the period's invoice as paid
    /// (PayPal charges on its own schedule, sometimes before the month's run).
    async fn record_payment(
        &self,
        sub: &Subscription,
        ev: &SubscriptionEvent,
    ) -> Result<(), Error> {
        let amount = ev.amount_cents.unwrap_or(sub.price_cents);
        let payment_ref = ev
            .provider_payment_id
            .clone()
            .unwrap_or_else(|| format!("{}:{}", ev.provider_ref, time(ev.at)));
        let unpaid: Option<(String,)> =
            sqlx::query_as("SELECT invoice_id FROM invoices WHERE subscription_id = ? AND state = 'issued' ORDER BY issued_at LIMIT 1")
                .bind(&sub.subscription_id)
                .fetch_optional(&self.pool)
                .await?;
        let invoice_id = match unpaid {
            Some((id,)) => {
                sqlx::query("UPDATE invoices SET state = 'paid', paid_at = ? WHERE invoice_id = ?")
                    .bind(time(ev.at))
                    .bind(&id)
                    .execute(&self.pool)
                    .await?;
                id
            }
            None => {
                let period = period_of(ev.at);
                let existing: Option<(String,)> = sqlx::query_as(
                    "SELECT invoice_id FROM invoices WHERE subscription_id = ? AND period = ?",
                )
                .bind(&sub.subscription_id)
                .bind(&period)
                .fetch_optional(&self.pool)
                .await?;
                match existing {
                    Some((id,)) => id,
                    None => {
                        let id = self
                            .insert_invoice(sub, &period, amount, None, ev.at)
                            .await?;
                        sqlx::query(
                            "UPDATE invoices SET state = 'paid', paid_at = ? WHERE invoice_id = ?",
                        )
                        .bind(time(ev.at))
                        .bind(&id)
                        .execute(&self.pool)
                        .await?;
                        id
                    }
                }
            }
        };
        let (failed_before,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM subscription_events WHERE subscription_id = ? AND kind = 'payment_failed' AND at >= COALESCE(?, '')",
        )
        .bind(&sub.subscription_id)
        .bind(&sub.past_due_since)
        .fetch_one(&self.pool)
        .await?;
        let attempts = if sub.past_due_since.is_some() {
            failed_before + 1
        } else {
            1
        };
        sqlx::query(
            "INSERT INTO payments (payment_id, subscription_id, invoice_id, provider_payment_id, amount_cents, attempts, receipt, at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (provider_payment_id) DO NOTHING",
        )
        .bind(format!("pay_{}", Uuid::now_v7().simple()))
        .bind(&sub.subscription_id)
        .bind(&invoice_id)
        .bind(payment_ref)
        .bind(amount)
        .bind(attempts.max(1))
        .bind(&ev.receipt)
        .bind(time(ev.at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn next_number(&self, period: &str) -> Result<String, Error> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM invoices WHERE period = ?")
            .bind(period)
            .fetch_one(&self.pool)
            .await?;
        Ok(format!("INV-{period}-{:04}", n + 1))
    }

    async fn insert_invoice(
        &self,
        sub: &Subscription,
        period: &str,
        amount: i64,
        run: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<String, Error> {
        let id = format!("inv_{}", Uuid::now_v7().simple());
        let number = self.next_number(period).await?;
        sqlx::query(
            "INSERT INTO invoices (invoice_id, number, site_id, subscription_id, period, total_cents, state, run_id, issued_at) VALUES (?, ?, ?, ?, ?, ?, 'issued', ?, ?)",
        )
        .bind(&id)
        .bind(&number)
        .bind(&sub.site_id)
        .bind(&sub.subscription_id)
        .bind(period)
        .bind(amount)
        .bind(run)
        .bind(time(at))
        .execute(&self.pool)
        .await?;
        let name = plan(&sub.plan_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| sub.plan_id.clone());
        sqlx::query("INSERT INTO invoice_lines (invoice_id, line, kind, description, amount_cents) VALUES (?, 1, 'plan', ?, ?)")
            .bind(&id)
            .bind(match sub.period() {
                Period::Monthly => format!("{name} plan, {period}"),
                Period::Annual => format!("{name} plan, 12 months from {period}"),
            })
            .bind(amount)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }
}

/// W6's plan key, `<plan_id>@<version>` for a monthly plan or `<plan_id>@<version>:annual` for
/// one paid 12 months at a time: the plan, its catalogue version and the period.
pub fn plan_key_parts(key: &str) -> (&str, i64, Period) {
    let (plan_id, rest) = key.split_once('@').unwrap_or((key, "1"));
    let (version, period) = match rest.split_once(':') {
        Some((v, p)) => (v, Period::parse(p).unwrap_or_default()),
        None => (rest, Period::Monthly),
    };
    (plan_id, version.parse().unwrap_or(1), period)
}

async fn table_exists(pool: &SqlitePool, name: &str) -> Result<bool, sqlx::Error> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(name)
            .fetch_one(pool)
            .await?;
    Ok(n > 0)
}

// ---------- The invoice run ----------

/// One invoice the run would issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    /// The subscription.
    pub subscription_id: String,
    /// The site.
    pub site_id: String,
    /// The plan's name.
    pub plan: String,
    /// The plan line, cents.
    pub plan_cents: i64,
    /// AI use above the included budget that the owner approved, cents. Always 0 until owners
    /// can raise their cap on a card.
    pub overage_cents: i64,
    /// Total, cents.
    pub total_cents: i64,
}

/// A drafted run for one period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    /// `2026-11`.
    pub period: String,
    /// The invoices it would issue.
    pub drafts: Vec<Draft>,
    /// Sum, cents.
    pub total_cents: i64,
    /// SHA-256 over the drafts: what op.finance approves.
    pub digest: String,
    /// Already issued for this period.
    pub issued: bool,
}

impl Billing {
    /// Drafts the run for `period` from active and past-due subscriptions that have gone live and
    /// have no invoice for it yet. An annual plan is invoiced once in 12 months: it's left out
    /// while its last invoice is less than 12 months old.
    pub async fn draft_run(&self, period: &str) -> Result<Run, Error> {
        let year_before = months_before(period, 12).unwrap_or_default();
        let (issued,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM invoice_runs WHERE period = ?")
                .bind(period)
                .fetch_one(&self.pool)
                .await?;
        let subs: Vec<Subscription> = sqlx::query_as(sub_select!(
            "WHERE state IN ('active', 'past_due') AND starts_at IS NOT NULL \
             AND subscription_id NOT IN (SELECT subscription_id FROM invoices WHERE period = ? AND subscription_id IS NOT NULL) \
             AND NOT (subscriptions.period = 'annual' AND subscription_id IN (SELECT i.subscription_id FROM invoices i WHERE i.period > ? AND i.subscription_id IS NOT NULL)) ORDER BY site_id"
        ))
        .bind(period)
        .bind(&year_before)
        .fetch_all(&self.pool)
        .await?;
        let drafts: Vec<Draft> = subs
            .into_iter()
            .map(|s| Draft {
                plan: plan(&s.plan_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| s.plan_id.clone()),
                plan_cents: s.price_cents,
                overage_cents: 0,
                total_cents: s.price_cents,
                subscription_id: s.subscription_id,
                site_id: s.site_id,
            })
            .collect();
        let total_cents = drafts.iter().map(|d| d.total_cents).sum();
        let canonical = serde_json::to_string(&(period, &drafts)).unwrap_or_default();
        let digest = format!("sha256:{}", sha256_hex(canonical.as_bytes()));
        Ok(Run {
            period: period.to_owned(),
            drafts,
            total_cents,
            digest,
            issued: issued > 0,
        })
    }

    /// Issues `run` after its approval. Refuses if the run no longer matches what was approved.
    pub async fn issue_run(
        &self,
        run: &Run,
        approved_by: &str,
        method: &str,
    ) -> Result<usize, Error> {
        let fresh = self.draft_run(&run.period).await?;
        if fresh.digest != run.digest || fresh.issued {
            return Err(Error::RunChanged);
        }
        let run_id = format!("run_{}", Uuid::now_v7().simple());
        let now = Utc::now();
        sqlx::query("INSERT INTO invoice_runs (run_id, period, digest, invoices, total_cents, approved_by, method, approved_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&run_id)
            .bind(&run.period)
            .bind(&run.digest)
            .bind(run.drafts.len() as i64)
            .bind(run.total_cents)
            .bind(approved_by)
            .bind(method)
            .bind(time(now))
            .execute(&self.pool)
            .await?;
        for d in &run.drafts {
            let sub: Subscription = sqlx::query_as(sub_select!("WHERE subscription_id = ?"))
                .bind(&d.subscription_id)
                .fetch_one(&self.pool)
                .await?;
            self.insert_invoice(&sub, &run.period, d.total_cents, Some(&run_id), now)
                .await?;
        }
        Ok(run.drafts.len())
    }
}

/// `2025-11` for `months_before("2026-11", 12)`.
fn months_before(period: &str, months: u32) -> Option<String> {
    let (y, m) = period.split_once('-')?;
    let at = Utc
        .with_ymd_and_hms(y.parse().ok()?, m.parse().ok()?, 1, 0, 0, 0)
        .single()?
        .checked_sub_months(Months::new(months))?;
    Some(period_of(at))
}

/// SHA-256 as hex, without pulling a hash crate into every caller.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

// ---------- The Money page ----------

/// MRR for one plan.
#[derive(Debug, Clone, Serialize)]
pub struct PlanMrr {
    /// The plan's name.
    pub plan: String,
    /// Subscriptions.
    pub count: i64,
    /// Monthly, cents.
    pub cents: i64,
}

/// A subscription in reminders (the dunning policy, 08 §4.2).
#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    /// The site.
    pub site_id: String,
    /// Its address.
    pub domain: String,
    /// Days since the first failed payment.
    pub days: i64,
    /// `day 0`, `day 3`, `day 7`, `day 14`, `day 30`: the policy's step.
    pub step: &'static str,
    /// What the policy does at that step.
    pub needs_person: bool,
}

/// `(site_id, domain, plan_id, state, spent)` from the usage query.
type UsageRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

/// One site's AI use against its plan.
#[derive(Debug, Clone, Serialize)]
pub struct SiteUsage {
    /// The site.
    pub site_id: String,
    /// Its address.
    pub domain: String,
    /// The plan's name, or none.
    pub plan: Option<String>,
    /// Subscription state.
    pub state: Option<String>,
    /// Included this month, micro-USD.
    pub included_micro_usd: i64,
    /// Spent this month, micro-USD (gateway estimates).
    pub spent_micro_usd: i64,
}

/// Everything the Money page counts for one month.
#[derive(Debug, Clone, Serialize)]
pub struct Money {
    /// `2026-11`.
    pub period: String,
    /// `November`.
    pub month: String,
    /// Subscriptions by state.
    pub by_state: BTreeMap<String, i64>,
    /// Monthly recurring revenue from active and past-due subscriptions, cents. An annual plan
    /// counts a twelfth of its 12-month charge.
    pub mrr_cents: i64,
    /// By plan.
    pub mrr_by_plan: Vec<PlanMrr>,
    /// Payments received this month, cents.
    pub collected_cents: i64,
    /// How many.
    pub payments: i64,
    /// Of those, paid on the first attempt.
    pub first_attempt: i64,
    /// Issued and unpaid, cents.
    pub outstanding_cents: i64,
    /// How many invoices.
    pub outstanding_invoices: i64,
    /// Invoices issued this month.
    pub invoices_this_month: i64,
    /// Subscriptions in reminders.
    pub reminders: Vec<Reminder>,
    /// Model spend this month across every site, micro-USD (gateway estimates).
    pub inference_micro_usd: i64,
}

impl Billing {
    /// The Money page's numbers for `now`'s month.
    pub async fn money(&self, now: DateTime<Utc>) -> Result<Money, Error> {
        let from = time(month_start(now));
        let to = time(next_month(now));
        let period = period_of(now);
        let states: Vec<(String, i64)> =
            sqlx::query_as("SELECT state, COUNT(*) FROM subscriptions GROUP BY state")
                .fetch_all(&self.pool)
                .await?;
        let by_plan: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT plan_id, COUNT(*), COALESCE(SUM(CASE WHEN period = 'annual' THEN (price_cents + 6) / 12 ELSE price_cents END), 0) FROM subscriptions WHERE state IN ('active', 'past_due') GROUP BY plan_id ORDER BY 3 DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let (collected, payments, first): (i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(amount_cents), 0), COUNT(*), COALESCE(SUM(attempts = 1), 0) FROM payments WHERE at >= ? AND at < ?",
        )
        .bind(&from)
        .bind(&to)
        .fetch_one(&self.pool)
        .await?;
        let (outstanding, outstanding_n): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(total_cents), 0), COUNT(*) FROM invoices WHERE state = 'issued'",
        )
        .fetch_one(&self.pool)
        .await?;
        let (issued_month,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM invoices WHERE period = ?")
                .bind(&period)
                .fetch_one(&self.pool)
                .await?;
        let (inference,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE at >= ? AND at < ?",
        )
        .bind(&from)
        .bind(&to)
        .fetch_one(&self.pool)
        .await?;
        let late: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT s.site_id, COALESCE(x.domain, s.site_id), s.past_due_since FROM subscriptions s LEFT JOIN sites x ON x.site_id = s.site_id \
             WHERE s.state = 'past_due' AND s.past_due_since IS NOT NULL ORDER BY s.past_due_since",
        )
        .fetch_all(&self.pool)
        .await?;
        let reminders = late
            .into_iter()
            .filter_map(|(site_id, domain, since)| {
                let days = (now - parse(&since)?).num_days().max(0);
                let (step, needs_person) = match days {
                    0..=2 => ("day 0", false),
                    3..=6 => ("day 3", false),
                    7..=13 => ("day 7", false),
                    14..=29 => ("day 14", true),
                    _ => ("day 30", true),
                };
                Some(Reminder {
                    site_id,
                    domain,
                    days,
                    step,
                    needs_person,
                })
            })
            .collect();
        Ok(Money {
            month: now.format("%B").to_string(),
            period,
            by_state: {
                let mut all: BTreeMap<String, i64> = [
                    "pending_approval",
                    "active",
                    "past_due",
                    "suspended",
                    "cancelled",
                    "expired",
                ]
                .into_iter()
                .map(|s| (s.to_owned(), 0))
                .collect();
                all.extend(states);
                all
            },
            mrr_cents: by_plan.iter().map(|(_, _, c)| c).sum(),
            mrr_by_plan: by_plan
                .into_iter()
                .map(|(id, count, cents)| PlanMrr {
                    plan: plan(&id).map(|p| p.name.clone()).unwrap_or(id),
                    count,
                    cents,
                })
                .collect(),
            collected_cents: collected,
            payments,
            first_attempt: first,
            outstanding_cents: outstanding,
            outstanding_invoices: outstanding_n,
            invoices_this_month: issued_month,
            reminders,
            inference_micro_usd: inference,
        })
    }

    /// AI use this month for every claimed or subscribed site, against its plan's allowance.
    pub async fn usage(&self, now: DateTime<Utc>) -> Result<Vec<SiteUsage>, Error> {
        let from = time(month_start(now));
        let rows: Vec<UsageRow> = sqlx::query_as(
            "SELECT x.site_id, x.domain, \
             (SELECT plan_id FROM subscriptions s WHERE s.site_id = x.site_id ORDER BY created_at DESC LIMIT 1), \
             (SELECT state FROM subscriptions s WHERE s.site_id = x.site_id ORDER BY created_at DESC LIMIT 1), \
             (SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage u WHERE u.site_id = x.site_id AND u.at >= ?), \
             (SELECT created_at FROM subscriptions s WHERE s.site_id = x.site_id ORDER BY created_at DESC LIMIT 1) \
             FROM sites x WHERE x.owner_account_id IS NOT NULL OR x.site_id IN (SELECT site_id FROM subscriptions) ORDER BY 5 DESC LIMIT 200",
        )
        .bind(&from)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(site_id, domain, plan_id, state, spent, started)| {
                let p = plan_id.as_deref().and_then(plan);
                let started = started
                    .as_deref()
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| t.with_timezone(&Utc));
                SiteUsage {
                    site_id,
                    domain,
                    plan: p.map(|p| p.name.clone()),
                    state,
                    included_micro_usd: p.map_or(0, |p| p.included_ai(started, now)),
                    spent_micro_usd: spent,
                }
            })
            .collect())
    }

    /// Issued runs, newest first: `(period, invoices, total cents, approved_by, method, at)`.
    pub async fn runs(&self) -> Result<Vec<(String, i64, i64, String, String, String)>, Error> {
        Ok(sqlx::query_as("SELECT period, invoices, total_cents, approved_by, method, approved_at FROM invoice_runs ORDER BY approved_at DESC LIMIT 12")
            .fetch_all(&self.pool)
            .await?)
    }
}

#[cfg(test)]
mod tests;
