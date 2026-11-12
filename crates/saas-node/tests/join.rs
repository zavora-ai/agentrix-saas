//! The join (v3): a Kilele Bakery sample-site sale, as node's checkout leaves it in
//! `control.db`, lands once in Kilele's Cashier wallet and once in Kilele's books.

use std::sync::Arc;

use adk_auth::signed_audit::Ed25519Signer;
use adk_core::audit_log::InMemoryAuditLogStore;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use saas_core::CompanyId;
use saas_core::books::{Books, PaymentFilter};
use saas_core::fake::FakeBooks;
use saas_core::paypal::PayPalRef;
use saas_node::samples;
use saas_wallet::{Wallets, key_name};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn paypal() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "A21", "token_type": "Bearer", "expires_in": 32400}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/payments/captures/CAPCAKE1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "CAPCAKE1", "status": "COMPLETED",
            "amount": {"currency_code": "USD", "value": "29.00"},
            "seller_receivable_breakdown": {
                "gross_amount": {"currency_code": "USD", "value": "29.00"},
                "paypal_fee": {"currency_code": "USD", "value": "1.43"},
                "net_amount": {"currency_code": "USD", "value": "27.57"},
            },
            "create_time": "2026-11-10T15:59:20Z",
            "links": [{"rel": "up", "href": "https://api.sandbox.paypal.com/v2/checkout/orders/ORDCAKE1", "method": "GET"}],
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/checkout/orders/ORDCAKE1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "ORDCAKE1",
            "payer": {"name": {"given_name": "Amina", "surname": "Odhiambo"}, "email_address": "amina@example.com"},
        })))
        .mount(&server)
        .await;
    server
}

async fn control() -> sqlx::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    // The columns node's checkout writes (agentrix-effects' 0610 migration).
    sqlx::raw_sql(
        "CREATE TABLE customer_payments (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, order_id TEXT NOT NULL, provider TEXT NOT NULL, provider_ref TEXT, state TEXT NOT NULL, capture_id TEXT, created_at INTEGER NOT NULL);
         INSERT INTO customer_payments VALUES ('pay_1', 'kilele-bakery-sample', 'order-17', 'paypal', 'ORDCAKE1', 'captured', 'CAPCAKE1', 1);
         INSERT INTO customer_payments VALUES ('pay_2', 'kilele-bakery-sample', 'order-18', 'paypal', 'ORDX', 'created', NULL, 2);
         INSERT INTO customer_payments VALUES ('pay_3', 'classy-perfumes-sample', 'order-1', 'paypal', 'ORDY', 'captured', 'CAPOTHER', 3);",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

#[tokio::test]
async fn a_sample_site_sale_lands_once_in_its_company_s_wallet_and_books() {
    let server = paypal().await;
    let books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let wallets = Wallets::new(
        CompanyId::kilele(),
        Arc::new(InMemoryAuditLogStore::new()),
        Ed25519Signer::from_seed(key_name(&CompanyId::kilele()), [9u8; 32]).unwrap(),
        Arc::new(PayPalClient::new(
            PayPalConfig {
                client_id: "id".into(),
                client_secret: "secret".into(),
                webhook_id: None,
                environment: Environment::Custom(server.uri()),
            },
            reqwest::Client::new(),
        )),
        books.clone(),
    );
    let pool = control().await;

    // Only Kilele's captured sale; the open order and the other site's sale aren't Kilele's.
    let sales = samples::captured(&pool, "kilele-bakery-sample")
        .await
        .unwrap();
    assert_eq!(sales.len(), 1);
    assert_eq!(sales[0].capture, "CAPCAKE1");

    assert!(samples::record(&wallets, &sales[0]).await.unwrap(), "new");
    assert!(
        !samples::record(&wallets, &sales[0]).await.unwrap(),
        "only once"
    );

    let cashier = samples::cashier(&CompanyId::kilele()).unwrap();
    let entries = wallets.entries(&cashier).await.unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.paypal == PayPalRef::Capture("CAPCAKE1".parse().unwrap())),
        "{entries:?}"
    );
    let key = PayPalRef::Capture("CAPCAKE1".parse().unwrap()).key();
    let posted = books
        .payments(&PaymentFilter {
            reference: Some(key),
            direction: None,
        })
        .await
        .unwrap();
    assert_eq!(posted.len(), 1, "one receipt in Kilele's books");
    assert!(wallets.verify(&cashier).await.unwrap().is_valid());
}

/// Kilele's hourly reconciliation (its Payables' wallet_checkpoint) checkpoints Kilele's own
/// wallet, even before its first sale, signed under the key named for the platform host. Its
/// desk once named Kilele by its slug (`kilele`), which reconciled Agentrix's wallet instead.
#[tokio::test]
async fn kileles_hourly_checkpoint_is_on_its_own_wallet_under_the_platform_hosts_key() {
    use saas_company::desk::Outside;
    let server = paypal().await;
    let dir = tempfile::tempdir().unwrap();
    let wallets_dir = dir.path().join("wallets");
    let kilele = CompanyId::kilele();
    saas_node::wallet_key::name_for_host(&wallets_dir, &kilele, "paypal.agentrix.website").unwrap();
    let client = Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: None,
            environment: Environment::Custom(server.uri()),
        },
        reqwest::Client::new(),
    ));
    let books: Arc<dyn Books> = Arc::new(FakeBooks::new(kilele.clone()));
    let wallets = Arc::new(
        Wallets::sqlite(kilele.clone(), &wallets_dir, client, books.clone())
            .await
            .unwrap(),
    );
    saas_node::os::shared().add_wallets("kilele", wallets.clone());
    let env = saas_node::config::EnvFile::default();
    let outside = saas_node::live::NodeOutside {
        company: kilele.clone(),
        wallets: Some(wallets.clone()),
        owner: Arc::new(saas_node::live::OwnerAlerts::from_env(&env)),
        control: None,
        books,
        env,
        audit: None,
        slug: "kilele".into(),
    };
    let done = outside.wallet_checkpoint().await.unwrap();
    assert_eq!(done["matches"], true, "{done}");

    let cashier = samples::cashier(&kilele).unwrap();
    let history = wallets.reconciliations(&cashier).await.unwrap();
    assert_eq!(history.len(), 1, "one checkpoint on Kilele's wallet");
    assert!(
        history[0]
            .note
            .contains("wallet.kilele-bakery.paypal.agentrix.website"),
        "{}",
        history[0].note
    );
    assert!(wallets.verify(&cashier).await.unwrap().is_valid());
}
