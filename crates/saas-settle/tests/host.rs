//! Settlement as the node runs it: built once with `SettleHost`, served through the pay host's
//! and Kilele's routers, and driven by the calls each company's kernel makes.

mod common;

use std::sync::Arc;

use adk_auth::web_bot_auth::{InMemoryKeyDirectorySource, KeyDirectorySource};
use chrono::{Duration, Utc};
use common::*;
use saas_core::fake::FakeBooks;
use saas_core::{Books, CompanyId};
use saas_settle::events::SettleEvent;
use saas_settle::host::{
    CompanySetup, SettleHost, SettleHostConfig, VAULT_APPROVED_PATH, VAULT_START_PATH,
    company_keys, demo_collections_terms, demo_intent_terms,
};
use saas_settle::keys::{PAYABLES, RECEIVABLES};
use saas_settle::mandate::{PASSKEY_METHOD, seal_owner_approved};
use saas_settle::payee::Clock;
use saas_settle::{BoundsNegotiator, Route, SealKind};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

struct Node {
    host: SettleHost,
    paypal: wiremock::MockServer,
    orders: Arc<Orders>,
    pay: Url,
    kilele_door: Url,
    agentrix_books: Arc<FakeBooks>,
    kilele_books: Arc<FakeBooks>,
    invoice: String,
}

async fn pool() -> sqlx::SqlitePool {
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

/// Kilele's owner allows `kilele_max_count` instalments; Agentrix's owner allows 3.
async fn node(kilele_max_count: u32) -> Node {
    let (paypal, orders) = paypal().await;
    let dir = std::env::temp_dir().join(format!("saas-settle-host-{}", Uuid::new_v4()));
    let pay_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let kilele_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pay = Url::parse(&format!("http://{}/", pay_listener.local_addr().unwrap())).unwrap();
    let kilele_door = Url::parse(&format!(
        "http://{}/",
        kilele_listener.local_addr().unwrap()
    ))
    .unwrap();
    let agentrix_books = Arc::new(FakeBooks::new(CompanyId::agentrix()));
    let kilele_books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let invoice = raise_invoice(
        &agentrix_books,
        120_000,
        "WebAgent Pro, annual plan",
        Utc::now().date_naive(),
    )
    .await;
    let agentrix = CompanySetup {
        company: CompanyId::agentrix(),
        name: "Agentrix SaaS".into(),
        origin: AGENTRIX.into(),
        door_url: pay.clone(),
        keys_dir: dir.join("agentrix"),
        pool: pool().await,
        books: agentrix_books.clone(),
        paypal_email: Some(AGENTRIX_PAYPAL.into()),
    };
    let kilele = CompanySetup {
        company: CompanyId::kilele(),
        name: "Kilele Bakery".into(),
        origin: KILELE.into(),
        door_url: kilele_door.clone(),
        keys_dir: dir.join("kilele"),
        pool: pool().await,
        books: kilele_books.clone(),
        paypal_email: None,
    };
    // The console seals each owner's mandate with the company's key files, after a passkey.
    let agentrix_keys = company_keys(&agentrix, RECEIVABLES).unwrap();
    let kilele_keys = company_keys(&kilele, PAYABLES).unwrap();
    let mut intent_terms = demo_intent_terms(&kilele_keys, AGENTRIX).unwrap();
    intent_terms.instalments.max_count = kilele_max_count;
    let intent = seal_owner_approved(
        SealKind::IntentMandate,
        &intent_terms,
        &TestPasskey,
        Uuid::new_v4(),
        json!({"method": "passkey", "credential": {}}),
        &kilele_keys,
    )
    .await
    .unwrap();
    let collections = seal_owner_approved(
        SealKind::CollectionsMandate,
        &demo_collections_terms(&agentrix_keys, KILELE),
        &TestPasskey,
        Uuid::new_v4(),
        json!({"method": "passkey", "credential": {}}),
        &agentrix_keys,
    )
    .await
    .unwrap();
    let directories = Arc::new(InMemoryKeyDirectorySource::new());
    directories
        .insert(AGENTRIX, agentrix_keys.directory(), None)
        .await;
    directories
        .insert(KILELE, kilele_keys.directory(), None)
        .await;
    let directories: Arc<dyn KeyDirectorySource> = directories;
    let clock: Clock = Arc::new(Utc::now);
    let host = SettleHost::build(SettleHostConfig {
        agentrix,
        kilele,
        directories,
        http: reqwest::Client::new(),
        agentrix_paypal: Some(client(&paypal)),
        kilele_payout: None,
        route: Route::Vault,
        intent,
        collections,
        owner_methods: vec![PASSKEY_METHOD.into()],
        negotiator: Arc::new(BoundsNegotiator),
        cash_available: None,
        clock,
    })
    .await
    .unwrap();
    let pay_router = host.pay_router();
    let kilele_router = host.kilele_router();
    tokio::spawn(async move { axum::serve(pay_listener, pay_router).await.unwrap() });
    tokio::spawn(async move { axum::serve(kilele_listener, kilele_router).await.unwrap() });
    Node {
        host,
        paypal,
        orders,
        pay,
        kilele_door,
        agentrix_books,
        kilele_books,
        invoice,
    }
}

async fn bill_open(books: &FakeBooks, invoice: &str) -> i64 {
    saas_settle::era::bill_of(books, invoice)
        .await
        .unwrap()
        .map(|(_, open)| open.minor)
        .unwrap()
}

#[tokio::test]
async fn the_routers_serve_each_company_s_door_and_keys() {
    let node = node(4).await;
    let http = reqwest::Client::new();
    let directory: Value = http
        .get(
            node.pay
                .join(".well-known/http-message-signatures-directory")
                .unwrap(),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kids: Vec<&str> = directory["keys"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|k| k["kid"].as_str())
        .collect();
    assert!(
        kids.contains(&"owner") && kids.contains(&"agent:receivables"),
        "{kids:?}"
    );
    let kilele: Value = http
        .get(
            node.kilele_door
                .join(".well-known/http-message-signatures-directory")
                .unwrap(),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(kilele.to_string().contains("agent:payables"));
    let card: Value = http
        .get(node.pay.join(".well-known/agent-card.json").unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        card["supportedInterfaces"][0]["url"],
        format!("{AGENTRIX}/a2a")
    );
    let unsigned = http
        .post(node.pay.join("a2a").unwrap())
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(unsigned.status().as_u16(), 401);
}

#[tokio::test]
async fn kilele_s_owner_saves_paypal_through_the_pay_host_s_pages() {
    let node = node(4).await;
    Mock::given(method("POST"))
        .and(path("/v3/vault/setup-tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "TOKEN1", "status": "PAYER_ACTION_REQUIRED", "customer": {"id": "CUST1"},
            "links": [{"rel": "approve", "href": "https://www.sandbox.paypal.com/agreements/approve?approval_session_id=TOKEN1"}],
        })))
        .mount(&node.paypal)
        .await;
    Mock::given(method("GET"))
        .and(path("/v3/vault/setup-tokens/TOKEN1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "TOKEN1", "status": "VAULTED", "customer": {"id": "CUST1"},
        })))
        .mount(&node.paypal)
        .await;
    Mock::given(method("POST"))
        .and(path("/v3/vault/payment-tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": VAULT_ID, "customer": {"id": "CUST1"}, "payment_source": {"paypal": {}},
        })))
        .mount(&node.paypal)
        .await;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // A return with no token from PayPal saves nothing.
    let bare = http
        .get(
            node.pay
                .join(VAULT_APPROVED_PATH.trim_start_matches('/'))
                .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status().as_u16(), 400);

    let start = http
        .get(
            node.pay
                .join(VAULT_START_PATH.trim_start_matches('/'))
                .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(start.status().as_u16(), 303);
    assert!(
        start.headers()["location"]
            .to_str()
            .unwrap()
            .contains("approval_session_id=TOKEN1")
    );
    let setup: Value = node
        .paypal
        .received_requests()
        .await
        .unwrap()
        .iter()
        .find(|r| r.url.path() == "/v3/vault/setup-tokens")
        .unwrap()
        .body_json()
        .unwrap();
    assert_eq!(
        setup["payment_source"]["paypal"]["experience_context"]["return_url"],
        format!("{AGENTRIX}{VAULT_APPROVED_PATH}")
    );

    let page = http
        .get(
            node.pay
                .join("settle/vault/approved?approval_token_id=TOKEN1&approval_session_id=TOKEN1")
                .unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(page.status().as_u16(), 200);
    let html = page.text().await.unwrap();
    assert!(
        html.contains("Kilele Bakery's PayPal is saved with Agentrix SaaS"),
        "{html}"
    );
    assert!(
        !html.contains(VAULT_ID),
        "the page shows only the end of the saved method"
    );

    // The saved wallet settles the invoice.
    node.host.offer_invoice(&node.invoice).await.unwrap();
    let settled = node
        .host
        .settle_invoice("gov-1", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap();
    assert_eq!(settled.paypal_kind, "capture");
}

#[tokio::test]
async fn the_kernel_offers_then_kilele_settles_once_per_key() {
    let node = node(4).await;
    node.host.payee().use_vault(KILELE, VAULT_ID).await.unwrap();
    let mut kilele_events = node.host.payer_events();
    let mut agentrix_events = node.host.payee_events();

    let offer = node.host.offer_invoice(&node.invoice).await.unwrap();
    assert_eq!(offer.decision, "will_settle");
    let SettleEvent::BillOffered {
        invoice_id,
        bill_id,
        amount,
        ..
    } = kilele_events.recv().await.unwrap()
    else {
        panic!("Kilele's kernel hears of the bill first");
    };
    assert_eq!(
        (invoice_id.as_str(), amount.minor),
        (node.invoice.as_str(), 120_000)
    );
    assert!(!bill_id.is_empty());

    // Kilele's Payables agent, on its governed path, settles with the call's key.
    let settled = node
        .host
        .settle_invoice("call-7", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap();
    assert_eq!(
        (settled.paypal_kind.as_str(), settled.paypal_id.as_str()),
        ("capture", "CAP-1")
    );
    let SettleEvent::Settled { receipt } = agentrix_events.recv().await.unwrap() else {
        panic!("Agentrix's kernel hears of the settlement");
    };
    assert_eq!(receipt.paypal.id, "CAP-1");

    // The same governed call again: the same settlement, no second capture.
    let again = node
        .host
        .settle_invoice("call-7", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap();
    assert_eq!(again.paypal_id, "CAP-1");
    assert_eq!(
        node.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        payments(&node.agentrix_books, "paypal:capture:CAP-1").await,
        1
    );
    assert_eq!(
        payments(&node.kilele_books, "paypal:capture:CAP-1").await,
        1
    );
    assert_eq!(bill_open(&node.kilele_books, &node.invoice).await, 0);

    // A new call for an invoice already settled pays nothing.
    let paid = node
        .host
        .settle_invoice("call-8", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap_err();
    assert!(paid.to_string().contains("already settled"), "{paid}");
    assert_eq!(
        node.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn agentrix_s_terms_are_agreed_inside_both_mandates_and_collected_on_the_kernel_s_wake() {
    // Kilele's owner allows at most 2 instalments; Agentrix's owner at most 3.
    let node = node(2).await;
    node.host.payee().use_vault(KILELE, VAULT_ID).await.unwrap();
    node.host.offer_invoice(&node.invoice).await.unwrap();
    let today = Utc::now().date_naive();
    let plan_id = format!("plan-{}", node.invoice);

    // Three instalments are inside Agentrix's owner's mandate, not Kilele's: refused.
    let refused = node
        .host
        .agree_plan(
            &plan_id,
            &node.invoice,
            &[
                (today, 40_000),
                (today + Duration::days(30), 40_000),
                (today + Duration::days(59), 40_000),
            ],
            "Three monthly instalments",
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("3 instalments"), "{refused}");
    assert!(node.host.uncollected().await.unwrap().is_empty());

    // Two are inside both: Kilele's Payables agent accepts with two cart mandates.
    let mut agentrix_events = node.host.payee_events();
    let (plan, _agreement) = node
        .host
        .agree_plan(
            &plan_id,
            &node.invoice,
            &[(today, 60_000), (today + Duration::days(30), 60_000)],
            "Half now, half in thirty days",
        )
        .await
        .unwrap();
    assert_eq!(plan.plan_id, plan_id);
    let SettleEvent::PlanAgreed {
        instalments,
        books_invoice_id,
        ..
    } = agentrix_events.recv().await.unwrap()
    else {
        panic!("the kernel hears of the plan, with its due dates");
    };
    assert_eq!(instalments.len(), 2);
    let books_id = node
        .agentrix_books
        .invoices(&saas_core::books::DocumentFilter::default())
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.number.as_deref() == Some(node.invoice.as_str()))
        .map(|i| i.id.0);
    assert_eq!(
        books_invoice_id, books_id,
        "the books id, for the kernel's desk"
    );
    let wake = node.host.uncollected().await.unwrap();
    assert_eq!(
        wake.iter().map(|r| r.due_on.clone()).collect::<Vec<_>>(),
        [today.to_string(), (today + Duration::days(30)).to_string()]
    );
    // Asked again, the same plan comes back; nothing new is agreed.
    let (same, _) = node
        .host
        .agree_plan(&plan_id, &node.invoice, &[(today, 1)], "again")
        .await
        .unwrap();
    assert_eq!(same, plan);

    // The kernel wakes on the first due date: instalment 0 is collected, once.
    let first = node
        .host
        .collect_instalment("wake-1", "kilele", &plan_id, 0, "USD", 60_000)
        .await
        .unwrap();
    let again = node
        .host
        .collect_instalment("wake-1", "kilele", &plan_id, 0, "USD", 60_000)
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(
        node.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        payments(
            &node.agentrix_books,
            &format!("paypal:capture:{}", first.paypal_id)
        )
        .await,
        1
    );
    assert_eq!(bill_open(&node.kilele_books, &node.invoice).await, 60_000);

    // The second isn't due yet: nothing moves, and it stays on the wake list.
    let early = node
        .host
        .collect_instalment("wake-2", "kilele", &plan_id, 1, "USD", 60_000)
        .await
        .unwrap_err();
    assert!(early.to_string().contains("isn't due until"), "{early}");
    assert_eq!(node.host.uncollected().await.unwrap().len(), 1);
    assert_eq!(
        node.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn every_attack_is_refused_with_a_signed_receipt_and_nothing_moves() {
    use saas_settle::attacks::Attack;
    let node = node(4).await;
    node.host.payee().use_vault(KILELE, VAULT_ID).await.unwrap();
    // One real settlement first (the replay needs a used cart mandate), then a fresh invoice
    // offered (the terms attack needs an open one).
    node.host.offer_invoice(&node.invoice).await.unwrap();
    node.host
        .settle_invoice("call-1", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap();
    let second = raise_invoice(
        &node.agentrix_books,
        90_000,
        "Commerce add-on",
        Utc::now().date_naive(),
    )
    .await;
    node.host.offer_invoice(&second).await.unwrap();
    let mut agentrix_events = node.host.payee_events();

    for (attack, code, refused_by) in [
        (Attack::UnsignedRequest, "signature_required", AGENTRIX),
        (Attack::WrongKeyAgent, "signature_invalid", AGENTRIX),
        (Attack::ReplayedMandate, "mandate_used", AGENTRIX),
        (Attack::InstalmentOutsideBounds, "plan_refused", KILELE),
    ] {
        let outcome = node.host.attack(attack).await.unwrap();
        assert!(outcome.refused, "{attack:?}: {outcome:?}");
        assert_eq!(outcome.code, code, "{attack:?}: {}", outcome.reason);
        assert!(!outcome.money_moved, "{attack:?}");
        let terms = outcome
            .receipt_terms
            .unwrap_or_else(|| panic!("{attack:?}: a refusal receipt that checks"));
        assert_eq!(terms.refused_by, refused_by, "{attack:?}");
        assert_eq!(terms.code, code);
        assert!(terms.nothing_moved);
        assert_eq!(outcome.receipt.unwrap().signer, refused_by);
    }
    assert_eq!(
        node.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(node.host.refusals(10).await.unwrap().len() >= 4);
    let mut refused = 0;
    while let Ok(event) = agentrix_events.try_recv() {
        if matches!(event, SettleEvent::Refused { .. }) {
            refused += 1;
        }
    }
    assert_eq!(
        refused, 3,
        "Agentrix's kernel hears of the refusals at its door"
    );
}

#[tokio::test]
async fn a_capture_paypal_confirms_by_webhook_finishes_the_settlement_once() {
    let node = node(4).await;
    node.host.payee().use_vault(KILELE, VAULT_ID).await.unwrap();
    node.host.offer_invoice(&node.invoice).await.unwrap();
    // PayPal's answer to the capture is lost: the settlement is unknown, nothing is posted.
    node.orders
        .fail_first
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let lost = node
        .host
        .settle_invoice("call-1", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap_err();
    assert!(lost.to_string().contains("never be paid twice"), "{lost}");
    let cart = node
        .host
        .payee()
        .store()
        .pending_for_invoice(&node.invoice)
        .await
        .unwrap()
        .expect("the capture is pending confirmation");
    assert_eq!(bill_open(&node.kilele_books, &node.invoice).await, 120_000);

    // PayPal's verified webhook says the capture completed: both books post it.
    let event = json!({
        "id": "WH-1", "event_type": "PAYMENT.CAPTURE.COMPLETED",
        "resource": {
            "id": "CAP-W1", "status": "COMPLETED", "custom_id": cart,
            "invoice_id": format!("agentrix-saas/{}/{cart}", node.invoice),
            "amount": {"currency_code": "USD", "value": "1200.00"},
            "seller_receivable_breakdown": {"paypal_fee": {"currency_code": "USD", "value": "42.10"}},
            "supplementary_data": {"related_ids": {"order_id": "ORDER-W1"}},
        },
    });
    let booked = node.host.on_paypal_event(&event).await.unwrap().unwrap();
    assert!(booked.completed_now);
    assert_eq!(booked.capture_id, "CAP-W1");
    assert_eq!(
        payments(&node.agentrix_books, "paypal:capture:CAP-W1").await,
        1
    );
    assert_eq!(
        payments(&node.kilele_books, "paypal:capture:CAP-W1").await,
        1
    );
    assert_eq!(bill_open(&node.kilele_books, &node.invoice).await, 0);

    // The same delivery again books nothing new; Kilele's retry gets the same settlement.
    let again = node.host.on_paypal_event(&event).await.unwrap().unwrap();
    assert!(!again.completed_now);
    assert_eq!(
        payments(&node.agentrix_books, "paypal:capture:CAP-W1").await,
        1
    );
    let retried = node
        .host
        .settle_invoice("call-1", "agentrix", &node.invoice, "USD", 120_000)
        .await
        .unwrap();
    assert_eq!(retried.paypal_id, "CAP-W1");
    assert_eq!(
        node.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    // Captures that aren't settlement's, and other events, are left to the node.
    let other = json!({"event_type": "PAYMENT.CAPTURE.COMPLETED", "resource": {"id": "CAP-X", "custom_id": "checkout-7"}});
    assert!(node.host.on_paypal_event(&other).await.unwrap().is_none());
    let sale = json!({"event_type": "PAYMENT.SALE.COMPLETED", "resource": {"id": "S-1"}});
    assert!(node.host.on_paypal_event(&sale).await.unwrap().is_none());
}

#[tokio::test]
async fn the_pay_host_lists_both_companies_keys() {
    use saas_settle::host::{AGENT_KEYS_PATH, is_settle_path};
    let node = node(4).await;
    let index: Value = reqwest::get(
        node.pay
            .join(AGENT_KEYS_PATH.trim_start_matches('/'))
            .unwrap(),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let companies = index["companies"].as_array().unwrap();
    assert_eq!(companies.len(), 2);
    for (entry, origin, agent) in [
        (&companies[0], AGENTRIX, "agent:receivables"),
        (&companies[1], KILELE, "agent:payables"),
    ] {
        assert_eq!(entry["origin"], origin);
        assert_eq!(
            entry["key_directory"],
            format!("{origin}/.well-known/http-message-signatures-directory")
        );
        let kids: Vec<&str> = entry["keys"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|k| k["kid"].as_str())
            .collect();
        assert!(kids.contains(&"owner") && kids.contains(&agent), "{kids:?}");
    }
    for path in [
        "/a2a",
        "/.well-known/http-message-signatures-directory",
        "/.well-known/agent-card.json",
        AGENT_KEYS_PATH,
        "/settle/vault/approved",
    ] {
        assert!(is_settle_path(path), "{path}");
    }
    assert!(!is_settle_path("/paypal/webhook"));
}

#[tokio::test]
async fn the_company_kernel_agrees_a_plan_by_books_invoice_id_once() {
    let node = node(4).await;
    node.host.offer_invoice(&node.invoice).await.unwrap();
    let books_id = node
        .agentrix_books
        .invoices(&saas_core::books::DocumentFilter::default())
        .await
        .unwrap()
        .into_iter()
        .find(|i| i.number.as_deref() == Some(node.invoice.as_str()))
        .unwrap()
        .id
        .0;
    let today = Utc::now().date_naive();
    let plan_id = format!("plan-{books_id}");
    let parts = [(today, 60_000), (today + Duration::days(30), 60_000)];
    let first = node
        .host
        .agree_debt_plan(
            "call-9",
            "kilele",
            &plan_id,
            &books_id,
            "USD",
            &parts,
            "Two halves",
        )
        .await
        .unwrap();
    assert_eq!(first.plan_id, plan_id);
    assert_eq!(first.instalments, parts);
    // A retried governed call: the same plan, no second agreement, no error.
    let again = node
        .host
        .agree_debt_plan(
            "call-9",
            "kilele",
            &plan_id,
            &books_id,
            "USD",
            &parts,
            "Two halves",
        )
        .await
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(node.host.uncollected().await.unwrap().len(), 2);
    // The wrong currency is refused before anything is sent.
    assert!(
        node.host
            .agree_debt_plan("call-10", "kilele", "plan-x", &books_id, "KES", &parts, "x")
            .await
            .is_err()
    );
}

/// The sample doors' checker from the host: Agentrix (which hosts the samples) seals their
/// refusals, and the kernel hears of them.
#[tokio::test]
async fn the_host_gives_the_sample_doors_a_checker_on_agentrix_s_keys() {
    use agentrix_samples::{AgentProof, DoorRefusal};
    let node = node(4).await;
    let mut kernel = node.host.payee().events().subscribe();
    let proof = node.host.sample_door_proof(
        Arc::new(adk_auth::web_bot_auth::InMemoryKeyDirectorySource::new()),
        None,
    );
    let receipt = proof
        .refusal_receipt(DoorRefusal {
            door: "https://kilele-bakery-sample.paypal.agentrix.example",
            site: "Kilele Bakery (sample)",
            tool: "checkout.create",
            code: "agent_identity_required",
            reason: "checkout.create needs a proven agent.",
            claimed: None,
            body: b"{}",
        })
        .await
        .expect("sealed");
    assert_eq!(
        receipt["signer"].as_str().unwrap(),
        node.host.agentrix_keys().origin
    );
    assert!(matches!(
        kernel.recv().await.unwrap(),
        saas_settle::events::SettleEvent::Refused { .. }
    ));
}
