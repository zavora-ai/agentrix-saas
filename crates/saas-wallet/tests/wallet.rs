//! Wallets against a mocked PayPal and FakeBooks: entries only from PayPal's own evidence, posted
//! to the books once, reconciled, matched three ways, and tamper-evident.

use std::sync::Arc;

use adk_auth::signed_audit::Ed25519Signer;
use adk_core::audit_log::InMemoryAuditLogStore;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use reqwest::header::HeaderMap;
use saas_core::books::{Books, DocumentFilter, DocumentStatus};
use saas_core::fake::FakeBooks;
use saas_core::paypal::PayPalRef;
use saas_core::wallet::*;
use saas_core::{AgentId, CompanyId, Money};
use saas_wallet::{Context, WalletError, Wallets, key_name, verify_webhook};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn cashier() -> AgentId {
    AgentId::new(CompanyId::kilele(), "cashier").unwrap()
}

fn capture(status: &str) -> Value {
    json!({
        "id": "CAP1", "status": status,
        "amount": {"currency_code": "USD", "value": "29.00"},
        "seller_receivable_breakdown": {
            "gross_amount": {"currency_code": "USD", "value": "29.00"},
            "paypal_fee": {"currency_code": "USD", "value": "1.43"},
            "net_amount": {"currency_code": "USD", "value": "27.57"},
        },
        "invoice_id": "kilele/order-17",
        "create_time": "2026-11-10T15:59:20Z",
        "links": [{"rel": "up", "href": "https://api.sandbox.paypal.com/v2/checkout/orders/ORD1", "method": "GET"}],
    })
}

async fn paypal(capture_status: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(ok(
            json!({"access_token": "A21", "token_type": "Bearer", "expires_in": 32400}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/payments/captures/CAP1"))
        .respond_with(ok(capture(capture_status)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/checkout/orders/ORD1"))
        .respond_with(ok(json!({"id": "ORD1", "payer": {"name": {"given_name": "Amina", "surname": "Odhiambo"}, "email_address": "amina@example.com"}})))
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer) -> Arc<PayPalClient> {
    let config = PayPalConfig {
        client_id: "test-client".into(),
        client_secret: "test-secret".into(),
        webhook_id: Some("WH-TEST".into()),
        environment: Environment::Custom(server.uri()),
    };
    Arc::new(PayPalClient::new(config, reqwest::Client::new()))
}

fn wallets(server: &MockServer, books: Arc<FakeBooks>) -> Wallets {
    let signer = Ed25519Signer::from_seed(key_name(&CompanyId::kilele()), [7u8; 32]).unwrap();
    Wallets::new(
        CompanyId::kilele(),
        Arc::new(InMemoryAuditLogStore::new()),
        signer,
        client(server),
        books,
    )
}

fn ctx() -> Context {
    Context {
        acting: cashier(),
        authority: Authority::PayerApproved,
        cause: None,
        counterparty: None,
    }
}

#[tokio::test]
async fn a_capture_read_back_writes_entries_posts_once_and_matches_three_ways() {
    let server = paypal("COMPLETED").await;
    let books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let w = wallets(&server, books.clone());
    let cap = PayPalRef::Capture("CAP1".parse().unwrap());

    let first = w.record(&cashier(), &cap, &ctx()).await.unwrap();
    assert!(!first.already);
    let kinds: Vec<_> = first
        .entries
        .iter()
        .map(|e| (e.seq, e.kind, e.amount))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (0, WalletEntryKind::Capture, Money::usd(2_900)),
            (1, WalletEntryKind::Fee, Money::usd(-143))
        ]
    );
    let e = &first.entries[0];
    assert_eq!(
        e.counterparty.as_ref().map(|c| c.name.as_str()),
        Some("Amina Odhiambo"),
        "the payer, as PayPal names them"
    );
    assert!(
        matches!(&e.cause, Some(WalletCause::Order { order, reference }) if order.as_str() == "ORD1" && reference.as_deref() == Some("kilele/order-17"))
    );
    assert_eq!(
        e.source,
        WalletSource::ReadBack {
            call: "GET /v2/payments/captures/CAP1".into()
        }
    );

    // Books: a sales invoice for the order, the receipt into clearing, the fee journal.
    assert!(
        first.books.payment.is_some()
            && first.books.journal.is_some()
            && first.books.fee_journal.is_some()
    );
    let invoices = books
        .invoices(&DocumentFilter {
            reference: Some("paypal:order:ORD1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0].status, DocumentStatus::Paid);
    assert_eq!(
        books.balance("1010"),
        Money::usd(2_900 - 143),
        "clearing holds what PayPal holds"
    );

    let again = w.record(&cashier(), &cap, &ctx()).await.unwrap();
    assert!(again.already, "the same PayPal object records once");
    assert_eq!(w.entries(&cashier()).await.unwrap().len(), 2);
    assert_eq!(books.payments(&Default::default()).await.unwrap().len(), 1);

    let cp = w.checkpoint(&cashier()).await.unwrap();
    assert_eq!(cp.reconciliation.state, SyncState::Matches);
    assert_eq!(cp.reconciliation.objects, 1);
    assert_eq!(cp.totals.net, Money::usd(2_757));
    assert!(cp.summary().words.starts_with("Matches PayPal as of "));
    let rows = w.matches(&cashier()).await.unwrap();
    assert_eq!(
        rows.iter().map(|r| r.state).collect::<Vec<_>>(),
        vec![MatchState::Matched, MatchState::Matched]
    );
    assert_eq!(
        rows[1].books.journal, first.books.fee_journal,
        "the fee row points at the fee journal"
    );
    assert_eq!(
        (rows[0].paypal_amount, rows[0].books_amount),
        (Some(Money::usd(2_900)), Some(Money::usd(2_900)))
    );
    assert_eq!(
        (rows[1].paypal_amount, rows[1].books_amount),
        (Some(Money::usd(-143)), Some(Money::usd(-143)))
    );
    let r = w.latest_ref(&cashier()).await.unwrap().unwrap();
    assert_eq!(r.stream, "wallet:kilele-bakery", "one stream per site");
    assert_eq!(r.reconciled, Reconciled::Matches);
    assert!(r.root.starts_with("sha256:"));
    assert!(rows[0].books.payment_number.is_some() && rows[0].books.journal_number.is_some());
    assert_eq!(
        rows[1].books.journal_number,
        rows[1].books.fee_journal_number
    );
    let feed = w.feed(&cashier()).await.unwrap();
    assert_eq!(feed.history.len(), 1);
    assert_eq!(feed.entries.len(), 2);
    assert!(feed.checkpoint.is_some());
    assert!(w.verify(&cashier()).await.unwrap().is_valid());
}

#[tokio::test]
async fn a_flipped_byte_in_the_wallet_fails_verification() {
    let server = paypal("COMPLETED").await;
    let w = wallets(&server, Arc::new(FakeBooks::new(CompanyId::kilele())));
    w.record(
        &cashier(),
        &PayPalRef::Capture("CAP1".parse().unwrap()),
        &ctx(),
    )
    .await
    .unwrap();
    let mut export = w.export(&cashier()).await.unwrap();
    let key = w.verifier_key().parse().unwrap();
    assert!(export.verify(&[key]).is_valid());
    let text = export.entries[0].record.clone().unwrap();
    export.entries[0].record = Some(
        text.replacen("29.00", "92.00", 1)
            .replacen("2900", "9200", 1),
    );
    let key = w.verifier_key().parse().unwrap();
    assert!(
        !export.verify(&[key]).is_valid(),
        "a changed amount breaks the Merkle root"
    );
}

#[tokio::test]
async fn nothing_is_recorded_until_paypal_says_the_money_moved() {
    let server = paypal("PENDING").await;
    let w = wallets(&server, Arc::new(FakeBooks::new(CompanyId::kilele())));
    let err = w
        .record(
            &cashier(),
            &PayPalRef::Capture("CAP1".parse().unwrap()),
            &ctx(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, WalletError::NotCompleted(_, ref s) if s == "PENDING"),
        "{err:?}"
    );
    assert!(w.entries(&cashier()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_checkpoint_shows_drift_when_paypal_changes_its_story() {
    let server = paypal("COMPLETED").await;
    let w = wallets(&server, Arc::new(FakeBooks::new(CompanyId::kilele())));
    w.record(
        &cashier(),
        &PayPalRef::Capture("CAP1".parse().unwrap()),
        &ctx(),
    )
    .await
    .unwrap();
    // The hourly call, while PayPal agrees.
    let hour = w.reconcile().await.unwrap();
    assert!(hour.matches(), "{:?}", hour.drift());
    assert!(hour.drifted.is_empty());
    let recorded: Vec<u64> = w
        .entries(&cashier())
        .await
        .unwrap()
        .iter()
        .map(|e| e.seq)
        .collect();
    // PayPal changes its story: the capture was refunded after the wallet recorded it.
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(ok(
            json!({"access_token": "A21", "token_type": "Bearer", "expires_in": 32400}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/payments/captures/CAP1"))
        .respond_with(ok(capture("REFUNDED")))
        .mount(&server)
        .await;
    let cp = w.checkpoint(&cashier()).await.unwrap();
    assert_eq!(cp.reconciliation.state, SyncState::Drift);
    assert!(
        cp.reconciliation.drift[0].what.contains("REFUNDED"),
        "{:?}",
        cp.reconciliation.drift
    );
    assert!(cp.summary().words.starts_with("Drift from PayPal"));
    let rows = w.matches(&cashier()).await.unwrap();
    assert!(rows.iter().all(|r| r.state == MatchState::PayPalDiffers));
    // The hourly call names the entries that drifted, under a new signed checkpoint.
    let hour = w.reconcile().await.unwrap();
    assert!(!hour.matches());
    assert_eq!(hour.drift()[0].entries, recorded);
    assert_eq!(
        hour.drifted.iter().map(|e| e.seq).collect::<Vec<_>>(),
        recorded
    );
    assert!(hour.checkpoint.size > cp.size);
    assert!(w.verify(&cashier()).await.unwrap().is_valid());
}

#[tokio::test]
async fn only_a_webhook_paypal_verified_names_an_object_to_record() {
    let server = paypal("COMPLETED").await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .respond_with(ok(json!({"verification_status": "FAILURE"})))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .respond_with(ok(json!({"verification_status": "SUCCESS"})))
        .with_priority(2)
        .mount(&server)
        .await;
    let w = wallets(&server, Arc::new(FakeBooks::new(CompanyId::kilele())));
    let pp = client(&server);
    // The body claims a different amount: it is never read; the read-back is.
    let event = json!({
        "id": "WH-1TEST", "event_type": "PAYMENT.CAPTURE.COMPLETED",
        "resource": {"id": "CAP1", "amount": {"currency_code": "USD", "value": "1000.00"}},
    });
    assert!(
        matches!(
            verify_webhook(&pp, &HeaderMap::new(), &event).await,
            Err(WalletError::NotVerified)
        ),
        "a forged webhook is refused"
    );
    let verified = verify_webhook(&pp, &HeaderMap::new(), &event)
        .await
        .unwrap();
    let recorded = w
        .record_event(&cashier(), &verified, &ctx())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recorded.entries[0].amount,
        Money::usd(2_900),
        "the amount is PayPal's read-back, not the webhook body"
    );
    assert!(
        matches!(&recorded.entries[0].source, WalletSource::Webhook { event_type, .. } if event_type == "PAYMENT.CAPTURE.COMPLETED")
    );
    let other = json!({"id": "WH-2TEST", "event_type": "CHECKOUT.ORDER.APPROVED", "resource": {"id": "ORD1"}});
    let ignored = verify_webhook(&pp, &HeaderMap::new(), &other)
        .await
        .unwrap();
    assert!(
        w.record_event(&cashier(), &ignored, &ctx())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_board_reads_a_sites_cashier_feed_over_http() {
    use tower::ServiceExt;
    let server = paypal("COMPLETED").await;
    let w = std::sync::Arc::new(wallets(
        &server,
        Arc::new(FakeBooks::new(CompanyId::kilele())),
    ));
    w.record(
        &cashier(),
        &PayPalRef::Capture("CAP1".parse().unwrap()),
        &ctx(),
    )
    .await
    .unwrap();
    let app = saas_wallet::router([("kilele".to_owned(), w)].into_iter().collect());
    let get = |uri: &str| {
        axum::http::Request::get(uri)
            .body(axum::body::Body::empty())
            .unwrap()
    };
    let ok = app
        .clone()
        .oneshot(get("/wallet/kilele.json"))
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body = axum::body::to_bytes(ok.into_body(), 1 << 20).await.unwrap();
    let feed: WalletFeed = serde_json::from_slice(&body).unwrap();
    assert_eq!(feed.wallet, cashier());
    assert_eq!(feed.entries.len(), 2);
    assert_eq!(
        app.oneshot(get("/wallet/agentrix.json"))
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn one_signed_stream_per_site_and_each_agent_sees_what_it_did() {
    let server = paypal("COMPLETED").await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts-item/ITEM1"))
        .respond_with(ok(json!({
            "payout_item_id": "ITEM1", "transaction_status": "SUCCESS",
            "payout_item_fee": {"currency": "USD", "value": "0.25"},
            "payout_item": {"amount": {"currency": "USD", "value": "40.00"}, "receiver": "billing@vultr.example"},
            "time_processed": "2026-11-10T16:00:00Z",
        })))
        .mount(&server)
        .await;
    let w = wallets(&server, Arc::new(FakeBooks::new(CompanyId::kilele())));
    let receivables = AgentId::new(CompanyId::kilele(), AgentId::RECEIVABLES).unwrap();
    let payables = AgentId::new(CompanyId::kilele(), AgentId::PAYABLES).unwrap();
    let as_desk = |acting: &AgentId| Context {
        acting: acting.clone(),
        authority: Authority::PayerApproved,
        cause: None,
        counterparty: None,
    };

    // Each desk records through its own view; both land in the one site stream.
    w.record(
        &receivables,
        &PayPalRef::Capture("CAP1".parse().unwrap()),
        &as_desk(&receivables),
    )
    .await
    .unwrap();
    w.record(
        &payables,
        &PayPalRef::Payout("ITEM1".parse().unwrap()),
        &as_desk(&payables),
    )
    .await
    .unwrap();
    // The same object again, through another view: still once for the site.
    assert!(
        w.record(
            &payables,
            &PayPalRef::Capture("CAP1".parse().unwrap()),
            &as_desk(&payables)
        )
        .await
        .unwrap()
        .already
    );

    let site = w.entries(&cashier()).await.unwrap();
    assert_eq!(site.len(), 4, "capture, its fee, payout, its fee");
    assert!(
        site.iter().all(|e| e.wallet == cashier()),
        "the wallet is the site's, held by the Cashier"
    );
    assert!(
        w.entries(&receivables)
            .await
            .unwrap()
            .iter()
            .all(|e| e.acting == receivables)
    );
    assert_eq!(w.entries(&receivables).await.unwrap().len(), 2);
    assert_eq!(w.entries(&payables).await.unwrap().len(), 2);

    // Checkpoints and the match are the site's, whichever agent asks.
    let cp = w.checkpoint(&payables).await.unwrap();
    assert_eq!(cp.wallet, cashier());
    assert_eq!(cp.reconciliation.objects, 2);
    assert_eq!(cp.reference().stream, "wallet:kilele-bakery");
    assert_eq!(cp.totals.net, Money::usd(2_900 - 143 - 4_000 - 25));
    let feed = w.feed(&payables).await.unwrap();
    assert_eq!(
        (feed.wallet.clone(), feed.entries.len(), feed.matches.len()),
        (payables.clone(), 2, 2)
    );
    assert_eq!(
        feed.checkpoint.unwrap().size,
        cp.size,
        "the site's checkpoint in a desk's view"
    );
    assert!(w.verify(&receivables).await.unwrap().is_valid());
    assert_eq!(
        saas_wallet::log_id(&receivables),
        saas_wallet::log_id(&payables)
    );

    let other = AgentId::new(CompanyId::agentrix(), AgentId::CASHIER).unwrap();
    assert!(w.entries(&other).await.is_err() || w.entries(&other).await.unwrap().is_empty());
    assert!(
        w.record(
            &other,
            &PayPalRef::Capture("CAP1".parse().unwrap()),
            &as_desk(&other)
        )
        .await
        .is_err(),
        "another company's agent can't write here"
    );
}
