//! Orders and Customers (gap 10 of 04-ui-parity): the pages render for the owner, refuse
//! everyone else, say why they're empty, and list what the platform's tables hold. The tables are
//! seeded with the same columns their owners' migrations make (node's `decisions` and
//! `receipts`, W6's payments, customer accounts and consents, the first release's passed
//! questions); the cards are real `DecisionCard`s, serialised as node stores them.

use std::sync::Arc;

use agentrix_shell::sample::{SITE, SamplePort};
use agentrix_shell::{Shell, ShellState, router, store::ShellStore};
use agentrix_types::{CardKind, DecisionCard, DecisionId, EffectRequest, SiteId};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chrono::{Duration, SecondsFormat, Utc};
use serde_json::json;
use sqlx::SqlitePool;
use tower::ServiceExt;

const SCHEMA: &str = "
CREATE TABLE decisions (
    decision_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, kind TEXT NOT NULL, card TEXT NOT NULL,
    fingerprint TEXT NOT NULL, nonce TEXT, state TEXT NOT NULL, method TEXT, signer TEXT,
    assertion TEXT, receipt TEXT, created_at TEXT NOT NULL, decided_at TEXT);
CREATE TABLE receipts (
    digest TEXT PRIMARY KEY, receipt_id TEXT NOT NULL UNIQUE, site_id TEXT NOT NULL, kind TEXT NOT NULL,
    subject_type TEXT NOT NULL, subject_id TEXT NOT NULL, issuer TEXT NOT NULL, started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL, issued_at TEXT NOT NULL, body TEXT NOT NULL);
CREATE TABLE customer_payments (
    id TEXT PRIMARY KEY, site_id TEXT NOT NULL, order_id TEXT NOT NULL, customer TEXT,
    fingerprint TEXT NOT NULL UNIQUE, provider TEXT NOT NULL, provider_ref TEXT UNIQUE, approve_url TEXT,
    currency TEXT NOT NULL, amount_minor INTEGER NOT NULL, state TEXT NOT NULL, capture_id TEXT,
    captured_minor INTEGER, refunded_minor INTEGER NOT NULL DEFAULT 0, sandbox INTEGER NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE refunds (
    id TEXT PRIMARY KEY, payment_id TEXT NOT NULL, site_id TEXT NOT NULL, fingerprint TEXT NOT NULL UNIQUE,
    amount_minor INTEGER NOT NULL, currency TEXT NOT NULL, state TEXT NOT NULL, provider_ref TEXT,
    approved_by TEXT, receipt TEXT, created_at INTEGER NOT NULL);
CREATE TABLE visitor_accounts (
    id TEXT PRIMARY KEY, site_id TEXT NOT NULL, email TEXT NOT NULL COLLATE NOCASE, name TEXT NOT NULL,
    phone TEXT, user_handle TEXT NOT NULL, email_verified_at INTEGER, created_at INTEGER NOT NULL,
    UNIQUE (site_id, email));
CREATE TABLE consent_receipts (
    id TEXT PRIMARY KEY, site_id TEXT NOT NULL, account_id TEXT NOT NULL, purpose TEXT NOT NULL,
    body TEXT NOT NULL, digest TEXT NOT NULL, receipt TEXT, withdrawn_receipt TEXT,
    given_at INTEGER NOT NULL, withdrawn_at INTEGER);
CREATE TABLE passed_questions (
    question_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, release_seq INTEGER, question TEXT NOT NULL,
    contact TEXT, passed_at TEXT NOT NULL);
";

async fn make(port: SamplePort, seed: bool) -> (Router, SqlitePool) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    let store = ShellStore::open(pool.clone()).await.expect("schema");
    if seed {
        sqlx::raw_sql(SCHEMA).execute(&pool).await.expect("tables");
    }
    let state = ShellState {
        shell: Arc::new(Shell::new().expect("templates")),
        port: Arc::new(port),
        store,
        apps: Vec::new(),
        lanes: Vec::new(),
        freeze_enabled: false,
    };
    (router(state), pool)
}

async fn get(app: &Router, path: &str) -> (StatusCode, String, Option<String>) {
    let res = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    let body =
        String::from_utf8(to_bytes(res.into_body(), 1 << 22).await.unwrap().to_vec()).unwrap();
    (status, body, location)
}

fn t(at: chrono::DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Holds an order the way node's `HeldOrders::raise` does; returns its id.
async fn hold(
    pool: &SqlitePool,
    site: &str,
    ago_hours: i64,
    name: &str,
    email: &str,
    what: &str,
    state: &str,
) -> String {
    let now = Utc::now();
    let created = now - Duration::hours(ago_hours);
    let input = json!({ "description": what, "date": "2026-11-14", "name": name, "email": email });
    let effect = EffectRequest {
        site: SiteId::new(site),
        webagent: "Concierge".into(),
        tool: "door.request_custom_order".into(),
        args: json!({ "capability": "request_custom_order", "input": input, "idempotency_key": format!("k-{name}") }),
        amount: None,
        expiry: created + Duration::hours(48),
    };
    let card = DecisionCard::new(
        DecisionId::generate(),
        CardKind::OrderHeld,
        format!("Order from an AI customer: {what}"),
        "An AI customer's agent asked.",
        effect,
        created,
    )
    .unwrap();
    let id = card.decision_id.as_str().to_owned();
    let decided = state != "waiting";
    sqlx::query(
        "INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, method, signer, assertion, receipt, created_at, decided_at) \
         VALUES (?, ?, 'order-held', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(site)
    .bind(serde_json::to_string(&card).unwrap())
    .bind(card.fingerprint.to_string())
    .bind(state)
    .bind((state == "approved").then_some("passkey"))
    .bind((state == "approved").then_some("acc_owner"))
    .bind((state == "approved").then_some("{}"))
    .bind((state == "approved").then(|| format!("sha256:{}", "ab".repeat(32))))
    .bind(t(created))
    .bind(decided.then(|| t(created + Duration::minutes(30))))
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn seed(pool: &SqlitePool) -> (String, String, String) {
    let waiting = hold(
        pool,
        SITE,
        2,
        "Njeri Kamau",
        "njeri@example.com",
        "A 2-tier forest cake for Amina, no nuts",
        "waiting",
    )
    .await;
    let paid = hold(
        pool,
        SITE,
        30,
        "Otieno Ouma",
        "Otieno@Example.com",
        "40 breakfast boxes for the office",
        "approved",
    )
    .await;
    let declined = hold(
        pool,
        SITE,
        50,
        "Njeri Kamau",
        "njeri@example.com",
        "Same-day wedding cake",
        "declined",
    )
    .await;
    // Another site's order never shows here.
    hold(
        pool,
        "site_other",
        1,
        "Someone Else",
        "else@example.com",
        "Not this site's order",
        "waiting",
    )
    .await;
    let now = Utc::now().timestamp();
    sqlx::query("INSERT INTO customer_payments (id, site_id, order_id, fingerprint, provider, currency, amount_minor, state, captured_minor, sandbox, created_at, updated_at) VALUES ('pay_1', ?, ?, 'fp1', 'paypal', 'KES', 840000, 'captured', 840000, 1, ?, ?)")
        .bind(SITE).bind(&paid).bind(now - 3600).bind(now - 3600)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO visitor_accounts (id, site_id, email, name, phone, user_handle, email_verified_at, created_at) VALUES ('vac_njeri', ?, 'njeri@example.com', 'Njeri Kamau', '+254700000001', 'u1', ?, ?)")
        .bind(SITE).bind(now - 86_400).bind(now - 86_400 * 3)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO consent_receipts (id, site_id, account_id, purpose, body, digest, receipt, given_at) VALUES ('cr_1', ?, 'vac_njeri', 'order-updates', '{}', 'sha256:x', ?, ?)")
        .bind(SITE).bind(format!("sha256:{}", "cd".repeat(32))).bind(now - 86_400)
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO passed_questions (question_id, site_id, question, contact, passed_at) VALUES ('q1', ?, 'Do you deliver to Karen?', 'mary.k@example.com', ?), ('q2', ?, 'Anonymous question', NULL, ?)")
        .bind(SITE).bind(t(Utc::now() - Duration::hours(5)))
        .bind(SITE).bind(t(Utc::now() - Duration::hours(6)))
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, started_at, ended_at, issued_at, body) VALUES (?, 'r1', ?, 'door.order-held', 'call', ?, 'k', ?, ?, ?, '{}')")
        .bind(format!("sha256:{}", "ef".repeat(32))).bind(SITE).bind(&waiting)
        .bind(t(Utc::now())).bind(t(Utc::now())).bind(t(Utc::now() - Duration::hours(2)))
        .execute(pool).await.unwrap();
    (waiting, paid, declined)
}

#[tokio::test]
async fn empty_sites_say_why_and_what_comes_next() {
    let (app, _) = make(SamplePort::default(), false).await;
    let base = format!("/console/{SITE}");
    let (s, html, _) = get(&app, &format!("{base}/orders")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "No orders yet",
        "They appear here when a customer&#x27;s AI agent orders through your site&#x27;s agent door.",
        "class=\"os",
    ] {
        assert!(html.contains(want), "orders lacks {want:?}");
    }
    let (s, html, _) = get(&app, &format!("{base}/customers")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    assert!(html.contains("No customers yet"), "{html}");
    // Unknown ids are not found, not blank pages.
    let (s, _, _) = get(&app, &format!("{base}/orders/dec_nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, &format!("{base}/customers/c_nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_the_owner_sees_them() {
    let (app, pool) = make(
        SamplePort {
            signed_in: false,
            ..SamplePort::default()
        },
        true,
    )
    .await;
    let (waiting, _, _) = seed(&pool).await;
    for path in [
        format!("/console/{SITE}/orders"),
        format!("/console/{SITE}/orders/{waiting}"),
        format!("/console/{SITE}/customers"),
        format!("/console/{SITE}/customers/vac_njeri"),
    ] {
        let (s, html, to) = get(&app, &path).await;
        assert!(s.is_redirection(), "{path}: {s}");
        assert!(to.unwrap_or_default().contains("signin"), "{path}");
        assert!(!html.contains("Njeri"), "{path} leaked a customer");
    }
    // Signed in, but not this site's owner.
    let (app, pool) = make(SamplePort::default(), true).await;
    seed(&pool).await;
    for path in [
        "/console/site_other/orders",
        "/console/site_other/customers",
    ] {
        let (s, html, _) = get(&app, path).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
        assert!(!html.contains("Someone Else"), "{path} leaked");
    }
}

#[tokio::test]
async fn orders_list_filter_and_open_from_seeded_rows() {
    let (app, pool) = make(SamplePort::default(), true).await;
    let (waiting, paid, declined) = seed(&pool).await;
    let base = format!("/console/{SITE}");

    let (s, html, _) = get(&app, &format!("{base}/orders")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "3 orders",
        "Njeri Kamau",
        "Otieno Ouma",
        "A 2-tier forest cake for Amina, no nuts",
        "Needs you",
        "Accepted",
        "Declined",
        "KSh 8,400 · Paid",
        "Sat 14 Nov",
        "A customer&#x27;s AI agent",
    ] {
        assert!(html.contains(want), "orders lacks {want:?}");
    }
    assert!(
        !html.contains("Not this site"),
        "another site's order showed"
    );
    assert!(!html.contains("No orders yet"));

    let (_, html, _) = get(&app, &format!("{base}/orders?state=new")).await;
    assert!(html.contains("Njeri Kamau") && !html.contains("Otieno Ouma"));
    let (_, html, _) = get(&app, &format!("{base}/orders?state=done")).await;
    assert!(html.contains("Otieno Ouma") && !html.contains("forest cake"));
    let (_, html, _) = get(&app, &format!("{base}/orders?state=refunded")).await;
    assert!(html.contains("No refunds on any order."), "{html}");
    let (_, html, _) = get(&app, &format!("{base}/orders?state=closed")).await;
    assert!(html.contains("Same-day wedding cake") && !html.contains("forest cake"));

    // A waiting order opens its decision card; nothing on the page changes it.
    let (s, html, _) = get(&app, &format!("{base}/orders/{waiting}")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "Order from Njeri Kamau",
        "Approve or decline",
        &format!("{base}/decisions/{waiting}"),
        "Nothing is charged either way",
        "No payment on this order",
        "Order held for you",
        "njeri@example.com",
        "/customers/vac_njeri",
    ] {
        assert!(html.contains(want), "waiting order lacks {want:?}");
    }
    assert!(
        !html.contains(&format!("action=\"{base}/orders")),
        "the order page must not post anything"
    );

    let (_, html, _) = get(&app, &format!("{base}/orders/{paid}")).await;
    for want in [
        "pay_1",
        "KSh 8,400",
        "Paid",
        "Test payment, no money moved",
        "You approved it with your passkey",
        "Refunds through PayPal aren't switched on",
        "K-",
    ] {
        assert!(html.contains(want), "paid order lacks {want:?}");
    }
    let (_, html, _) = get(&app, &format!("{base}/orders/{declined}")).await;
    assert!(html.contains("You declined it"), "{html}");

    // The dock carries both apps, and the Orders badge counts what waits.
    let (_, html, _) = get(&app, &base).await;
    assert!(html.contains(&format!("href=\"{base}/orders\"")));
    assert!(html.contains(&format!("href=\"{base}/customers\"")));
    assert!(
        html.contains("Orders<span class=\"badge\" aria-label=\"1 waiting\">1</span>"),
        "the Orders badge is missing"
    );
}

#[tokio::test]
async fn customers_list_search_and_open_from_seeded_rows() {
    let (app, pool) = make(SamplePort::default(), true).await;
    seed(&pool).await;
    let base = format!("/console/{SITE}");

    let (s, html, _) = get(&app, &format!("{base}/customers")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "3 people your site served",
        "1 with an account",
        "Njeri Kamau",
        "Otieno Ouma",
        "otieno@example.com",
        "mary.k@example.com",
    ] {
        assert!(html.contains(want), "customers lacks {want:?}");
    }
    assert!(!html.contains("Someone Else"));
    for link in html.split("href=\"").skip(1) {
        let link = link.split('"').next().unwrap();
        assert!(
            !link.contains('@') && !link.contains("%40"),
            "an email address is in a link: {link}"
        );
    }

    let (_, html, _) = get(&app, &format!("{base}/customers?q=otie")).await;
    assert!(html.contains("Otieno Ouma") && !html.contains("Njeri Kamau"));
    let (_, html, _) = get(&app, &format!("{base}/customers?q=zzz")).await;
    assert!(html.contains("Nobody matches"), "{html}");

    let (s, html, _) = get(&app, &format!("{base}/customers/vac_njeri")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "Njeri Kamau",
        "+254700000001",
        "email confirmed",
        "A 2-tier forest cake for Amina, no nuts",
        "Same-day wedding cake",
        "Sent by their AI agent",
        "Order updates",
        "/account/data",
        "you can't do either for them here",
    ] {
        assert!(html.contains(want), "customer lacks {want:?}");
    }

    // Someone known only by the contact they left with a question.
    let (_, list, _) = get(&app, &format!("{base}/customers?q=mary")).await;
    let href = list
        .split("href=\"")
        .find_map(|s| {
            s.starts_with(&format!("{base}/customers/c_"))
                .then(|| s.split('"').next().unwrap().to_owned())
        })
        .expect("a link to Mary");
    let (s, html, _) = get(&app, &href).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "Do you deliver to Karen?",
        "Passed to you from Ask",
        "No account on your site",
        "No orders from",
    ] {
        assert!(html.contains(want), "Mary's page lacks {want:?}");
    }
}
