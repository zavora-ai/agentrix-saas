//! Billing against node's real schema files.

use std::path::PathBuf;

use chrono::{Duration, TimeZone, Utc};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use super::*;

async fn pool() -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agentrix-node/src");
    let mut files = vec![src.join("auth/schema.sql"), src.join("claim/schema.sql")];
    let mut migrations: Vec<PathBuf> = std::fs::read_dir(src.join("db/migrations"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    migrations.sort();
    files.extend(migrations);
    for f in files {
        let sql = std::fs::read_to_string(&f).unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    }
    pool
}

async fn site(pool: &SqlitePool, id: &str, domain: &str) {
    sqlx::query("INSERT INTO sites (site_id, domain, old_origin, created_at) VALUES (?, ?, ?, '2026-11-01T00:00:00.000000Z')")
        .bind(id)
        .bind(domain)
        .bind(format!("https://{domain}"))
        .execute(pool)
        .await
        .unwrap();
}

fn event(
    provider_ref: &str,
    kind: EventKind,
    at: DateTime<Utc>,
    payment: Option<&str>,
) -> SubscriptionEvent {
    SubscriptionEvent {
        provider_ref: provider_ref.into(),
        kind,
        amount_cents: payment.map(|_| 7900),
        provider_payment_id: payment.map(str::to_owned),
        at,
        period_end: Some(at + Duration::days(30)),
        receipt: Some("sha256:ab".into()),
    }
}

#[test]
fn the_catalogue_is_the_concepts_plans_in_usd() {
    let c = catalogue();
    assert_eq!(c.currency, "USD");
    let names: Vec<(&str, i64, i64)> = c
        .plans
        .iter()
        .map(|p| (p.name.as_str(), p.price_cents, p.included_ai_micro_usd))
        .collect();
    assert_eq!(
        names[..3],
        [
            ("Essential", 2900, 5_000_000),
            ("Business", 7900, 15_000_000),
            ("Commerce", 14_900, 30_000_000)
        ]
    );
    assert_eq!(plan("agency").unwrap().minimum_sites, Some(10));
}

#[tokio::test]
async fn a_subscription_follows_paypals_events_and_payments_land_on_invoices() {
    let pool = pool().await;
    site(&pool, "site_a", "bakery.example").await;
    let billing = Billing::new(pool.clone());
    let sub = billing
        .start("site_a", "business", "I-BW452GLLEP1G", Some("sha256:start"))
        .await
        .unwrap();
    assert_eq!(sub.state, "pending_approval");
    assert!(matches!(
        billing.start("site_a", "essential", "I-OTHER", None).await,
        Err(Error::AlreadySubscribed)
    ));

    let t0 = Utc.with_ymd_and_hms(2026, 11, 8, 9, 0, 0).unwrap();
    assert_eq!(
        billing
            .on_event(&event("I-BW452GLLEP1G", EventKind::Activated, t0, None))
            .await
            .unwrap(),
        Applied::Changed("active".into())
    );
    billing.went_live("site_a", t0).await.unwrap();
    let paid = event(
        "I-BW452GLLEP1G",
        EventKind::PaymentCompleted,
        t0 + Duration::minutes(5),
        Some("PAY-1"),
    );
    assert_eq!(
        billing.on_event(&paid).await.unwrap(),
        Applied::Changed("active".into())
    );
    assert_eq!(
        billing.on_event(&paid).await.unwrap(),
        Applied::Duplicate,
        "a repeated webhook changes nothing"
    );

    let m = billing.money(t0 + Duration::hours(1)).await.unwrap();
    assert_eq!(
        (m.mrr_cents, m.collected_cents, m.payments, m.first_attempt),
        (7900, 7900, 1, 1)
    );
    assert_eq!(m.mrr_by_plan[0].plan, "Business");
    let (number, state): (String, String) = sqlx::query_as("SELECT number, state FROM invoices")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (number.as_str(), state.as_str()),
        ("INV-2026-11-0001", "paid")
    );
    // The month's run has nothing left to issue for this site.
    assert!(
        billing
            .draft_run("2026-11")
            .await
            .unwrap()
            .drafts
            .is_empty()
    );
}

#[tokio::test]
async fn failed_payments_put_a_site_in_reminders_by_the_policy() {
    let pool = pool().await;
    site(&pool, "site_b", "grill.example").await;
    let billing = Billing::new(pool.clone());
    billing
        .start("site_b", "essential", "I-LATE", None)
        .await
        .unwrap();
    let t0 = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap();
    billing
        .on_event(&event("I-LATE", EventKind::Activated, t0, None))
        .await
        .unwrap();
    billing
        .on_event(&event(
            "I-LATE",
            EventKind::PaymentFailed,
            t0 + Duration::hours(1),
            None,
        ))
        .await
        .unwrap();
    let m = billing.money(t0 + Duration::days(15)).await.unwrap();
    assert_eq!(m.reminders.len(), 1);
    assert_eq!(
        (m.reminders[0].step, m.reminders[0].needs_person),
        ("day 14", true)
    );
    assert_eq!(m.by_state.get("past_due"), Some(&1));
    // Recovered on the second try: out of reminders, and the payment counts as not first-attempt.
    let mut late = event(
        "I-LATE",
        EventKind::PaymentCompleted,
        t0 + Duration::days(15),
        Some("PAY-2"),
    );
    late.amount_cents = Some(2900);
    billing.on_event(&late).await.unwrap();
    let m = billing.money(t0 + Duration::days(15)).await.unwrap();
    assert!(m.reminders.is_empty());
    assert_eq!((m.payments, m.first_attempt), (1, 0));
}

#[tokio::test]
async fn an_invoice_run_issues_only_what_was_approved() {
    let pool = pool().await;
    site(&pool, "site_c", "clinic.example").await;
    site(&pool, "site_d", "shop.example").await;
    let billing = Billing::new(pool.clone());
    let t0 = Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap();
    for (s, plan_id, r) in [
        ("site_c", "commerce", "I-C"),
        ("site_d", "essential", "I-D"),
    ] {
        billing.start(s, plan_id, r, None).await.unwrap();
        billing
            .on_event(&event(r, EventKind::Activated, t0, None))
            .await
            .unwrap();
    }
    // Only sites that went live are billed.
    billing.went_live("site_c", t0).await.unwrap();
    let run = billing.draft_run("2026-11").await.unwrap();
    assert_eq!((run.drafts.len(), run.total_cents), (1, 14_900));
    // Something changes after the card was shown: the approval no longer matches.
    billing.went_live("site_d", t0).await.unwrap();
    assert!(matches!(
        billing.issue_run(&run, "acct", "passkey").await,
        Err(Error::RunChanged)
    ));
    let run = billing.draft_run("2026-11").await.unwrap();
    assert_eq!(billing.issue_run(&run, "acct", "passkey").await.unwrap(), 2);
    let m = billing.money(t0).await.unwrap();
    assert_eq!((m.outstanding_invoices, m.outstanding_cents), (2, 17_800));
    assert!(billing.draft_run("2026-11").await.unwrap().issued);
    assert!(matches!(
        billing.issue_run(&run, "acct", "passkey").await,
        Err(Error::RunChanged)
    ));
}

#[tokio::test]
async fn the_providers_side_comes_in_from_the_effects_gateways_tables() {
    let pool = pool().await;
    site(&pool, "site_e", "dental.example").await;
    // W6's table, as agentrix-effects creates it (0610_w6_payments.sql).
    sqlx::raw_sql(
        "CREATE TABLE plan_subscriptions (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, plan_key TEXT NOT NULL, provider_ref TEXT UNIQUE, approve_url TEXT, \
         state TEXT NOT NULL, currency TEXT NOT NULL, amount_minor INTEGER NOT NULL, period_end INTEGER, last_event TEXT, sandbox INTEGER NOT NULL, \
         created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
         CREATE TABLE plan_payments (provider_ref TEXT NOT NULL, provider_payment_id TEXT PRIMARY KEY, amount_minor INTEGER NOT NULL, paid_at INTEGER NOT NULL);",
    )
    .execute(&pool)
    .await
    .unwrap();
    let billing = Billing::new(pool.clone());
    assert_eq!(billing.sync().await.unwrap(), 0, "nothing to bring in yet");
    let now = Utc::now().timestamp();
    sqlx::query("INSERT INTO plan_subscriptions VALUES ('sub_w6', 'site_e', 'commerce@1', 'I-W6', 'https://paypal.example/approve', 'active', 'USD', 14900, ?, 'BILLING.SUBSCRIPTION.ACTIVATED', 1, ?, ?)")
        .bind(now + 30 * 86_400)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(billing.sync().await.unwrap(), 1);
    assert_eq!(
        billing.sync().await.unwrap(),
        0,
        "unchanged rows change nothing"
    );
    let sub = billing.for_site("site_e").await.unwrap().unwrap();
    assert_eq!(
        (
            sub.subscription_id.as_str(),
            sub.plan_id.as_str(),
            sub.state.as_str(),
            sub.price_cents
        ),
        ("sub_w6", "commerce", "active", 14_900)
    );
    sqlx::query("INSERT INTO plan_payments VALUES ('I-W6', 'SALE-1', 14900, ?)")
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    billing.sync().await.unwrap();
    billing.sync().await.unwrap();
    let m = billing.money(Utc::now()).await.unwrap();
    assert_eq!(
        (m.mrr_cents, m.collected_cents, m.payments),
        (14_900, 14_900, 1)
    );
}

/// The launch offer: a plan's first-month AI for a subscription's first 30 days, then the
/// monthly allowance.
#[test]
fn the_first_month_ai_lasts_thirty_days() {
    let now = Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
    let business = plan("business").unwrap();
    assert_eq!(business.first_month_ai_micro_usd, 150_000_000);
    let started = |days: i64| Some(now - chrono::Duration::days(days));
    assert_eq!(business.included_ai(started(0), now), 150_000_000);
    assert_eq!(business.included_ai(started(29), now), 150_000_000);
    assert_eq!(business.included_ai(started(30), now), 15_000_000);
    assert_eq!(business.included_ai(None, now), 15_000_000);
    for (id, first) in [("essential", 50), ("business", 150), ("commerce", 250)] {
        let p = plan(id).unwrap();
        assert_eq!(p.first_month_ai_micro_usd, first * 1_000_000, "{id}");
        assert_eq!(p.includes[0], "Web hosting included", "{id}");
    }
}

/// 12 months up front: 12 times the monthly price less the catalogue's one discount, to the cent.
#[test]
fn twelve_months_cost_twelve_monthly_prices_less_the_discount() {
    assert_eq!(catalogue().annual_discount_percent, 20);
    for (id, monthly, year, saved) in [
        ("essential", 2900, 27_840, 6960),
        ("business", 7900, 75_840, 18_960),
        ("commerce", 14_900, 143_040, 35_760),
    ] {
        let p = plan(id).unwrap();
        assert_eq!(p.charge_cents(Period::Monthly), monthly, "{id}");
        assert_eq!(p.annual_cents(), year, "{id}");
        assert_eq!(p.charge_cents(Period::Annual), year, "{id}");
        assert_eq!(p.annual_saving_cents(), saved, "{id}");
    }
    // Rounded to the nearest cent when the discount doesn't divide evenly.
    assert_eq!(annual_cents(999, 15), 10_190); // 101.898 dollars
    assert_eq!(annual_cents(2900, 0), 34_800);
    assert_eq!(Period::parse("annual"), Some(Period::Annual));
    assert_eq!(Period::parse("monthly"), Some(Period::Monthly));
    assert_eq!(Period::parse("weekly"), None);
}

#[test]
fn a_renewal_is_one_month_or_twelve_later() {
    let at = Utc.with_ymd_and_hms(2026, 11, 8, 9, 0, 0).unwrap();
    assert_eq!(
        Period::Monthly.renewal(at),
        Utc.with_ymd_and_hms(2026, 12, 8, 9, 0, 0).unwrap()
    );
    assert_eq!(
        Period::Annual.renewal(at),
        Utc.with_ymd_and_hms(2027, 11, 8, 9, 0, 0).unwrap()
    );
    // The month's last day when the next month is shorter.
    let end = Utc.with_ymd_and_hms(2027, 1, 31, 9, 0, 0).unwrap();
    assert_eq!(
        Period::Monthly.renewal(end),
        Utc.with_ymd_and_hms(2027, 2, 28, 9, 0, 0).unwrap()
    );
    assert_eq!(
        plan_key_parts("commerce@1"),
        ("commerce", 1, Period::Monthly)
    );
    assert_eq!(
        plan_key_parts("commerce@2:annual"),
        ("commerce", 2, Period::Annual)
    );
}

/// An annual plan's AI still comes month by month: the first-month offer for 30 days, then the
/// monthly allowance, never 12 months of it.
#[tokio::test]
async fn an_annual_plan_keeps_the_monthly_ai_allowance() {
    let pool = pool().await;
    site(&pool, "site_y", "yearly.example").await;
    let billing = Billing::new(pool.clone());
    let sub = billing
        .start_for("site_y", "business", Period::Annual, "I-YEAR", None)
        .await
        .unwrap();
    assert_eq!(sub.period(), Period::Annual);
    let at = Utc.with_ymd_and_hms(2026, 11, 8, 9, 0, 0).unwrap();
    let started = Some(at);
    let p = plan("business").unwrap();
    let day = |d: i64| at + Duration::days(d);
    assert_eq!(p.included_ai(started, day(10)), 150_000_000);
    assert_eq!(p.included_ai(started, day(45)), 15_000_000);
    assert_eq!(p.included_ai(started, day(300)), 15_000_000);
    let usage = billing.usage(Utc::now()).await.unwrap();
    let u = usage.iter().find(|u| u.site_id == "site_y").unwrap();
    assert_eq!(u.included_micro_usd, 150_000_000, "the first month's offer");
}

#[tokio::test]
async fn an_annual_plan_is_charged_once_renews_in_twelve_months_and_counts_a_twelfth_in_mrr() {
    let pool = pool().await;
    site(&pool, "site_a", "annual.example").await;
    let billing = Billing::new(pool.clone());
    let sub = billing
        .start_for("site_a", "commerce", Period::Annual, "I-ANNUAL", None)
        .await
        .unwrap();
    assert_eq!((sub.period.as_str(), sub.price_cents), ("annual", 143_040));
    let t0 = Utc.with_ymd_and_hms(2026, 11, 8, 9, 0, 0).unwrap();
    billing
        .on_event(&event("I-ANNUAL", EventKind::Activated, t0, None))
        .await
        .unwrap();
    billing.went_live("site_a", t0).await.unwrap();
    let mut paid = event(
        "I-ANNUAL",
        EventKind::PaymentCompleted,
        t0 + Duration::minutes(1),
        Some("PAY-Y1"),
    );
    paid.amount_cents = Some(143_040);
    paid.period_end = None;
    billing.on_event(&paid).await.unwrap();
    let sub = billing.for_site("site_a").await.unwrap().unwrap();
    assert_eq!(
        sub.next_charge().unwrap(),
        Period::Annual.renewal(paid.at),
        "the next charge is 12 months on"
    );
    let m = billing.money(t0 + Duration::hours(1)).await.unwrap();
    assert_eq!((m.mrr_cents, m.collected_cents), (11_920, 143_040));
    // Invoiced once: no run asks for it again until 12 months have passed.
    for month in ["2026-12", "2027-06", "2027-10"] {
        assert!(
            billing.draft_run(month).await.unwrap().drafts.is_empty(),
            "{month}"
        );
    }
    let due = billing.draft_run("2027-11").await.unwrap();
    assert_eq!((due.drafts.len(), due.total_cents), (1, 143_040));
    let (line,): (String,) = sqlx::query_as("SELECT description FROM invoice_lines")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(line, "Commerce plan, 12 months from 2026-11");
}

#[tokio::test]
async fn an_annual_subscription_comes_in_from_the_effects_gateway_with_its_period() {
    let pool = pool().await;
    site(&pool, "site_f", "annual-w6.example").await;
    sqlx::raw_sql(
        "CREATE TABLE plan_subscriptions (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, plan_key TEXT NOT NULL, provider_ref TEXT UNIQUE, approve_url TEXT, \
         state TEXT NOT NULL, currency TEXT NOT NULL, amount_minor INTEGER NOT NULL, period_end INTEGER, last_event TEXT, sandbox INTEGER NOT NULL, \
         created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);",
    )
    .execute(&pool)
    .await
    .unwrap();
    let now = Utc::now().timestamp();
    sqlx::query("INSERT INTO plan_subscriptions VALUES ('sub_y', 'site_f', 'essential@1:annual', 'I-Y', NULL, 'active', 'USD', 27840, ?, NULL, 1, ?, ?)")
        .bind(now + 365 * 86_400)
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
    let billing = Billing::new(pool.clone());
    assert_eq!(billing.sync().await.unwrap(), 1);
    let sub = billing.for_site("site_f").await.unwrap().unwrap();
    assert_eq!(
        (sub.plan_id.as_str(), sub.period(), sub.price_cents),
        ("essential", Period::Annual, 27_840)
    );
}
