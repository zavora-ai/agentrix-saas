//! The owner's AI budget (`specs/platform/facts.md`, "AI budget"): a monthly cap, set and shown
//! in the site's currency. The Model Gateway's daily limit per site is an internal guard against
//! a runaway day. It is never the owner's number, so nothing the owner reads shows it: not the
//! console, the Chief WebAgent's answers, the morning brief, Telegram or the owner app.
//!
//! - **The cap**, the first of: the site's own monthly budget for every purpose (the `budgets`
//!   row the gateway enforces), the AI budget the owner chose with their plan (W4's
//!   `onboarding_plan_choices`), the AI included with the plan they pay for (W3's
//!   `subscriptions`, through [`agentrix_billing::Plan::included_ai`]), the plan on the site's
//!   registry row (`sites.plan`). With none of them there is no cap, and pages say so.
//! - **The currency**: the site's own (W2's `rec_sites` for the samples, else `sites.currency`),
//!   at a posted rate: the site's live price list, else the newest live price list in that
//!   currency. Without a posted rate, amounts stay in US dollars, the unit the gateway meters
//!   in, rather than converting at a guessed rate.
//! - **The month's spend** ([`this_month`]): the metric catalogue's count for this month
//!   ([`crate::metrics::Metrics::spend`]), the same calls the console's meter counts.
//!
//! The console's meter (agentrix-shell's `budget::meter`) draws from here, and so do the Chief
//! WebAgent's `spend` tool, the morning brief and the channels' brief.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::metrics::Metrics as _;
use crate::words::MicroUsd;

/// How a site's money is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Currency {
    /// ISO code: `KES`, `USD`.
    pub code: String,
    /// Units of it per US dollar, from a posted rate; 1 for US dollars.
    pub per_usd: f64,
}

impl Default for Currency {
    fn default() -> Self {
        Self {
            code: "USD".into(),
            per_usd: 1.0,
        }
    }
}

impl Currency {
    fn converts(&self) -> bool {
        self.code != "USD" && self.per_usd.is_finite() && self.per_usd > 0.0
    }

    /// An amount as owners read budgets: "KSh 3,870", "KSh 38", "less than KSh 1", "$0.22".
    pub fn show(&self, amount: MicroUsd) -> String {
        if !self.converts() {
            return amount.usd();
        }
        let units = amount.0 as f64 / 1e6 * self.per_usd;
        if amount.0 > 0 && units < 0.5 {
            let one = MicroUsd((1e6 / self.per_usd).ceil() as i64);
            return format!("less than {}", one.local(&self.code, self.per_usd));
        }
        amount.local(&self.code, self.per_usd)
    }

    /// The same amount as a number in [`Currency::code`], for an API: whole units as
    /// [`Currency::show`] rounds them (38 for "KSh 38"), cents for US dollars, and cents for an
    /// amount under half a unit, so spend never reads as nothing.
    pub fn amount(&self, amount: MicroUsd) -> f64 {
        let units = if self.converts() {
            amount.0 as f64 / 1e6 * self.per_usd
        } else {
            amount.0 as f64 / 1e6
        };
        if !self.converts() || (amount.0 > 0 && units < 0.5) {
            let cents = (units * 100.0)
                .round()
                .max(if amount.0 > 0 { 1.0 } else { 0.0 });
            return cents / 100.0;
        }
        units.round()
    }
}

/// Where a site's monthly cap comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapFrom {
    /// The site's monthly budget, which the gateway enforces.
    Budget,
    /// The AI budget the owner chose with their plan.
    Chosen,
    /// The AI included with a plan, by its name ("Commerce").
    Plan(String),
}

/// A site's AI budget: its monthly cap, if it has one, and how its money is shown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AiBudget {
    /// The monthly cap and where it comes from.
    pub cap: Option<(MicroUsd, CapFrom)>,
    /// The site's currency.
    pub currency: Currency,
}

impl AiBudget {
    /// An amount in the site's currency.
    pub fn show(&self, amount: MicroUsd) -> String {
        self.currency.show(amount)
    }

    /// "KSh 38 of KSh 3,870", or "KSh 38" without a cap.
    pub fn of_cap(&self, spent: MicroUsd) -> String {
        match &self.cap {
            Some((cap, _)) => format!("{} of {}", self.show(spent), self.show(*cap)),
            None => self.show(spent),
        }
    }

    /// "KSh 38 of KSh 3,870 this month", or "KSh 38 this month" without a cap.
    pub fn month_words(&self, spent: MicroUsd) -> String {
        format!("{} this month", self.of_cap(spent))
    }

    /// This month's share of the cap, 0–100, rounded up so any spend shows; `None` without a cap.
    pub fn percent(&self, spent: MicroUsd) -> Option<u32> {
        let (cap, _) = self.cap.as_ref()?;
        if cap.0 <= 0 {
            return Some(100);
        }
        let pct = (spent.0.max(0) as i128 * 100 + cap.0 as i128 - 1) / cap.0 as i128;
        Some(u32::try_from(pct.min(100)).unwrap_or(100))
    }

    /// What the cap is, in the owner's words.
    pub fn cap_note(&self) -> String {
        match &self.cap {
            Some((cap, CapFrom::Budget)) => format!(
                "Your monthly AI budget is {}. AI calls stop for the rest of the month once it's spent.",
                self.show(*cap)
            ),
            Some((cap, CapFrom::Chosen)) => format!(
                "{} a month is the AI budget you chose with your plan.",
                self.show(*cap)
            ),
            Some((cap, CapFrom::Plan(name))) => format!(
                "{} a month of AI is included with your {name} plan.",
                self.show(*cap)
            ),
            None => "No monthly AI budget is set for this site yet.".into(),
        }
    }
}

/// This month's AI spend against the site's monthly cap ([`this_month`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MonthSpend {
    /// The cap and the currency.
    pub budget: AiBudget,
    /// Spent this month.
    pub spent: MicroUsd,
    /// AI calls this month.
    pub calls: i64,
}

impl MonthSpend {
    /// "KSh 38 of KSh 3,870 this month".
    pub fn words(&self) -> String {
        self.budget.month_words(self.spent)
    }

    /// "KSh 38 of KSh 3,870".
    pub fn of_cap(&self) -> String {
        self.budget.of_cap(self.spent)
    }

    /// Spent, as a number in the site's currency ([`Currency::amount`]).
    pub fn used(&self) -> f64 {
        self.budget.currency.amount(self.spent)
    }

    /// The monthly cap as a number in the site's currency; `None` without one.
    pub fn cap(&self) -> Option<f64> {
        self.budget
            .cap
            .as_ref()
            .map(|(cap, _)| self.budget.currency.amount(*cap))
    }
}

/// `site`'s AI spend this month, from the metric catalogue, against its monthly cap in its
/// currency ([`read`]). A database the gateway hasn't metered into yet has spent nothing.
///
/// # Errors
///
/// When a table that exists can't be read.
pub async fn this_month(
    control: &SqlitePool,
    records: Option<&SqlitePool>,
    site: &str,
    host: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<MonthSpend> {
    let metrics = crate::metrics::SqliteMetrics::new(control.clone()).with_clock(move || now);
    let scope = crate::metrics::Scope::Site(agentrix_types::SiteId::new(site));
    let (spent, calls) = match metrics.spend(&scope, crate::time::Window::ThisMonth).await {
        Ok(month) => (month.cost, month.calls),
        Err(crate::metrics::Error::Db(e)) if missing(&e) => (MicroUsd(0), 0),
        Err(e) => return Err(e.into()),
    };
    Ok(MonthSpend {
        budget: read(control, records, site, host, now).await?,
        spent,
        calls,
    })
}

/// A table, column or database file that isn't there (yet): nothing to read, not an error.
fn missing(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(d) => {
            let m = d.message();
            m.contains("no such table")
                || m.contains("no such column")
                || m.contains("unable to open")
        }
        sqlx::Error::Io(_) => true,
        _ => false,
    }
}

fn optional<T>(r: Result<Option<T>, sqlx::Error>) -> anyhow::Result<Option<T>> {
    match r {
        Ok(v) => Ok(v),
        Err(e) if missing(&e) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Reads `site`'s AI budget from `control` (control.db) and, for the samples, `records` (W2's
/// records.db). `host` is the site's host, which W2's records key the samples by.
///
/// # Errors
///
/// When a table that exists can't be read.
pub async fn read(
    control: &SqlitePool,
    records: Option<&SqlitePool>,
    site: &str,
    host: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<AiBudget> {
    Ok(AiBudget {
        cap: cap(control, site, now).await?,
        currency: currency(control, records, site, host).await?,
    })
}

async fn cap(
    control: &SqlitePool,
    site: &str,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<(MicroUsd, CapFrom)>> {
    let own: Option<(i64,)> = optional(
        sqlx::query_as(
            "SELECT limit_micro_usd FROM budgets WHERE site_id = ? AND purpose = '*' AND period = 'month'",
        )
        .bind(site)
        .fetch_optional(control)
        .await,
    )?;
    if let Some((limit,)) = own {
        return Ok(Some((MicroUsd(limit), CapFrom::Budget)));
    }
    let chosen: Option<(i64,)> = optional(
        sqlx::query_as("SELECT ai_budget_usd FROM onboarding_plan_choices WHERE site_id = ?")
            .bind(site)
            .fetch_optional(control)
            .await,
    )?;
    if let Some((usd,)) = chosen.filter(|(usd,)| *usd > 0) {
        return Ok(Some((
            MicroUsd(usd.saturating_mul(1_000_000)),
            CapFrom::Chosen,
        )));
    }
    let paid: Option<(String, String)> = optional(
        sqlx::query_as(
            "SELECT plan_id, COALESCE(starts_at, created_at) FROM subscriptions \
             WHERE site_id = ? AND state IN ('pending_approval', 'active', 'past_due') \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(site)
        .fetch_optional(control)
        .await,
    )?;
    if let Some((id, started)) = paid
        && let Some(plan) = agentrix_billing::plan(&id)
    {
        let started = DateTime::parse_from_rfc3339(&started)
            .ok()
            .map(|t| t.with_timezone(&Utc));
        return Ok(Some((
            MicroUsd(plan.included_ai(started, now)),
            CapFrom::Plan(plan.name.clone()),
        )));
    }
    let registry: Option<(Option<String>,)> = optional(
        sqlx::query_as("SELECT plan FROM sites WHERE site_id = ?")
            .bind(site)
            .fetch_optional(control)
            .await,
    )?;
    Ok(registry
        .and_then(|(plan,)| plan)
        .and_then(|id| agentrix_billing::plan(&id))
        .map(|p| {
            (
                MicroUsd(p.included_ai_micro_usd),
                CapFrom::Plan(p.name.clone()),
            )
        }))
}

async fn currency(
    control: &SqlitePool,
    records: Option<&SqlitePool>,
    site: &str,
    host: &str,
) -> anyhow::Result<Currency> {
    // The samples keep their currency and price lists in W2's records.
    let mut code: Option<String> = None;
    let mut rate: Option<i64> = None;
    if let Some(records) = records {
        let row: Option<(String, String)> = optional(
            sqlx::query_as(
                "SELECT site, currency FROM rec_sites WHERE site = ? OR host = ? LIMIT 1",
            )
            .bind(site)
            .bind(host)
            .fetch_optional(records)
            .await,
        )?;
        if let Some((slug, c)) = row {
            rate = optional(
                sqlx::query_as::<_, (i64,)>(
                    "SELECT usd_rate_minor FROM rec_price_lists WHERE site = ? AND status = 'live' \
                     ORDER BY valid_from DESC LIMIT 1",
                )
                .bind(&slug)
                .fetch_optional(records)
                .await,
            )?
            .map(|(r,)| r);
            code = Some(c);
        }
    }
    if code.is_none() {
        let row: Option<(Option<String>,)> = optional(
            sqlx::query_as("SELECT currency FROM sites WHERE site_id = ?")
                .bind(site)
                .fetch_optional(control)
                .await,
        )?;
        code = row.and_then(|(c,)| c).filter(|c| !c.trim().is_empty());
    }
    let Some(code) = code.map(|c| c.trim().to_ascii_uppercase()) else {
        return Ok(Currency::default());
    };
    if code == "USD" {
        return Ok(Currency::default());
    }
    if rate.is_none()
        && let Some(records) = records
    {
        rate = optional(
            sqlx::query_as::<_, (i64,)>(
                "SELECT usd_rate_minor FROM rec_price_lists WHERE currency = ? AND status = 'live' \
                 ORDER BY valid_from DESC LIMIT 1",
            )
            .bind(&code)
            .fetch_optional(records)
            .await,
        )?
        .map(|(r,)| r);
    }
    Ok(match rate.filter(|r| *r > 0) {
        Some(minor) => Currency {
            code,
            per_usd: minor as f64 / 100.0,
        },
        None => Currency::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn kes() -> Currency {
        Currency {
            code: "KES".into(),
            per_usd: 129.0,
        }
    }

    #[test]
    fn money_shows_in_the_sites_currency() {
        let c = kes();
        assert_eq!(c.show(MicroUsd(30_000_000)), "KSh 3,870");
        assert_eq!(c.show(MicroUsd(294_000)), "KSh 38");
        assert_eq!(c.show(MicroUsd(0)), "KSh 0");
        assert_eq!(c.show(MicroUsd(1_000)), "less than KSh 1");
        assert_eq!(Currency::default().show(MicroUsd(220_000)), "$0.22");
    }

    #[test]
    fn the_cap_is_monthly_and_never_a_day() {
        let b = AiBudget {
            cap: Some((MicroUsd(30_000_000), CapFrom::Plan("Commerce".into()))),
            currency: kes(),
        };
        assert_eq!(
            b.month_words(MicroUsd(294_000)),
            "KSh 38 of KSh 3,870 this month"
        );
        assert_eq!(b.percent(MicroUsd(294_000)), Some(1));
        assert_eq!(b.percent(MicroUsd(40_000_000)), Some(100));
        assert_eq!(
            b.cap_note(),
            "KSh 3,870 a month of AI is included with your Commerce plan."
        );
        let none = AiBudget {
            cap: None,
            currency: kes(),
        };
        assert_eq!(none.month_words(MicroUsd(294_000)), "KSh 38 this month");
        assert_eq!(none.percent(MicroUsd(294_000)), None);
        for words in [b.month_words(MicroUsd(1)), b.cap_note(), none.cap_note()] {
            assert!(!words.contains("day"), "{words}");
        }
    }

    async fn pool() -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_cap_and_rate_come_from_records() {
        let control = pool().await;
        let records = pool().await;
        let now = Utc::now();
        // Nothing to read at all: no cap, in dollars.
        assert_eq!(
            read(&control, Some(&records), "k", "k.example", now)
                .await
                .unwrap(),
            AiBudget::default()
        );
        sqlx::raw_sql(
            "CREATE TABLE sites (site_id TEXT PRIMARY KEY, plan TEXT, currency TEXT);
             CREATE TABLE budgets (site_id TEXT, purpose TEXT, period TEXT, limit_micro_usd INTEGER);
             CREATE TABLE subscriptions (site_id TEXT, plan_id TEXT, state TEXT, starts_at TEXT, created_at TEXT);
             INSERT INTO sites VALUES ('k', 'commerce', NULL), ('t', NULL, 'KES');",
        )
        .execute(&control)
        .await
        .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE rec_sites (site TEXT, host TEXT, currency TEXT);
             CREATE TABLE rec_price_lists (site TEXT, currency TEXT, status TEXT, usd_rate_minor INTEGER, valid_from INTEGER);
             INSERT INTO rec_sites VALUES ('k', 'k.example', 'KES');
             INSERT INTO rec_price_lists VALUES ('k', 'KES', 'draft', 13500, 9), ('k', 'KES', 'live', 12900, 1);",
        )
        .execute(&records)
        .await
        .unwrap();
        let b = read(&control, Some(&records), "k", "k.example", now)
            .await
            .unwrap();
        assert_eq!(b.currency, kes(), "the live price list's rate");
        assert_eq!(
            b.cap,
            Some((MicroUsd(30_000_000), CapFrom::Plan("Commerce".into())))
        );
        assert_eq!(
            b.month_words(MicroUsd(294_000)),
            "KSh 38 of KSh 3,870 this month"
        );

        // A site without a price list of its own uses the posted rate for its currency; without
        // records there is no posted rate, so it stays in dollars.
        let t = read(&control, Some(&records), "t", "t.example", now)
            .await
            .unwrap();
        assert_eq!(t.currency, kes());
        assert_eq!(t.cap, None);
        let t = read(&control, None, "t", "t.example", now).await.unwrap();
        assert_eq!(t.currency, Currency::default());

        // A plan being paid for beats the registry's plan; a cancelled one doesn't count.
        sqlx::query("INSERT INTO subscriptions VALUES ('k', 'business', 'cancelled', NULL, ?)")
            .bind(now.to_rfc3339())
            .execute(&control)
            .await
            .unwrap();
        let b = read(&control, Some(&records), "k", "k.example", now)
            .await
            .unwrap();
        assert_eq!(b.cap.unwrap().1, CapFrom::Plan("Commerce".into()));
        sqlx::query("INSERT INTO subscriptions VALUES ('k', 'business', 'active', NULL, ?)")
            .bind((now - chrono::Duration::days(40)).to_rfc3339())
            .execute(&control)
            .await
            .unwrap();
        let b = read(&control, Some(&records), "k", "k.example", now)
            .await
            .unwrap();
        assert_eq!(
            b.cap,
            Some((MicroUsd(15_000_000), CapFrom::Plan("Business".into())))
        );

        // The site's own monthly budget, which the gateway enforces, beats both.
        sqlx::query(
            "INSERT INTO budgets VALUES ('k', 'build', 'month', 1000000), ('k', '*', 'month', 20000000)",
        )
        .execute(&control)
        .await
        .unwrap();
        let b = read(&control, Some(&records), "k", "k.example", now)
            .await
            .unwrap();
        assert_eq!(b.cap, Some((MicroUsd(20_000_000), CapFrom::Budget)));
        assert_eq!(b.month_words(MicroUsd(0)), "KSh 0 of KSh 2,580 this month");
    }
    #[test]
    fn amounts_for_an_api_read_as_the_words_do() {
        let c = kes();
        assert_eq!(c.amount(MicroUsd(30_000_000)), 3870.0);
        assert_eq!(c.amount(MicroUsd(294_000)), 38.0, "\"KSh 38\"");
        assert_eq!(c.amount(MicroUsd(0)), 0.0);
        assert_eq!(
            c.amount(MicroUsd(1_000)),
            0.13,
            "less than KSh 1 never reads as nothing"
        );
        assert_eq!(c.amount(MicroUsd(10)), 0.01);
        assert_eq!(Currency::default().amount(MicroUsd(215_000)), 0.22);
        assert_eq!(Currency::default().amount(MicroUsd(30_000_000)), 30.0);
    }

    #[tokio::test]
    async fn this_months_spend_against_the_monthly_cap() {
        let control = crate::testing::control_db().await;
        let now = DateTime::parse_from_rfc3339("2026-11-09T06:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Nothing metered yet: nothing spent, and no cap without a plan.
        let none = this_month(&control, None, "k", "k.example", now)
            .await
            .unwrap();
        assert_eq!(none.spent, MicroUsd(0));
        assert_eq!(none.words(), "$0.00 this month");
        assert_eq!(none.cap(), None);

        sqlx::raw_sql(
            "INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, created_at, plan, currency) \
             VALUES ('k', 'k.example', 'Kilele Bakery', 'https://k.example', 180, '2026-10-01T00:00:00Z', 'commerce', 'KES')",
        )
        .execute(&control)
        .await
        .unwrap();
        for (at, cost) in [
            // Last month's calls don't count; this month's do.
            ("2026-10-31T12:00:00.000000Z", 5_000_000),
            ("2026-11-02T08:00:00.000000Z", 200_000),
            ("2026-11-09T05:00:00.000000Z", 94_000),
        ] {
            sqlx::query(
                "INSERT INTO usage (site_id, purpose, provider, model, tokens_in, tokens_out, \
                 cost_micro_usd, outcome, at) VALUES ('k', 'concierge', 'vultr', 'm1', 10, 5, ?, 'ok', ?)",
            )
            .bind(cost)
            .bind(at)
            .execute(&control)
            .await
            .unwrap();
        }
        let records = pool().await;
        sqlx::raw_sql(
            "CREATE TABLE rec_sites (site TEXT, host TEXT, currency TEXT);
             CREATE TABLE rec_price_lists (site TEXT, currency TEXT, status TEXT, usd_rate_minor INTEGER, valid_from INTEGER);
             INSERT INTO rec_price_lists VALUES ('other', 'KES', 'live', 12900, 1);",
        )
        .execute(&records)
        .await
        .unwrap();
        let m = this_month(&control, Some(&records), "k", "k.example", now)
            .await
            .unwrap();
        assert_eq!((m.spent, m.calls), (MicroUsd(294_000), 2));
        assert_eq!(m.words(), "KSh 38 of KSh 3,870 this month");
        assert_eq!(m.of_cap(), "KSh 38 of KSh 3,870");
        assert_eq!((m.used(), m.cap()), (38.0, Some(3870.0)));
        assert!(!m.words().contains("day"));
    }
}
