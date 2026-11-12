//! The sample sites' real payments: agentrix-samples' `Payments` over the Effects Gateway, with
//! PayPal mocked. The pay page gets PayPal's approval page; the return URL captures once.

use std::sync::Arc;

use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use agentrix_samples::ports::{PaymentRequest, PaymentStart, Payments};
use saas_settle::SamplePayPal;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn a_sample_order_goes_to_paypal_and_captures_once() {
    let paypal = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "A21AA-test", "expires_in": 32400})),
        )
        .mount(&paypal)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/checkout/orders"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "ORDER1", "status": "PAYER_ACTION_REQUIRED",
            "links": [{"rel": "payer-action", "href": "https://www.sandbox.paypal.com/checkoutnow?token=ORDER1"}],
        })))
        .mount(&paypal)
        .await;
    Mock::given(method("POST"))
        .and(path("/v2/checkout/orders/ORDER1/capture"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "ORDER1", "status": "COMPLETED",
            "purchase_units": [{"payments": {"captures": [{"id": "CAP1", "status": "COMPLETED", "amount": {"currency_code": "USD", "value": "12.34"}}]}}],
        })))
        .expect(1)
        .mount(&paypal)
        .await;
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    let client = Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: None,
            environment: Environment::Custom(paypal.uri()),
        },
        reqwest::Client::new(),
    ));
    let payments = SamplePayPal::new(agentrix_effects::Effects::new(
        ctx.pool.clone(),
        Some(client),
        Arc::new(agentrix_host::testing::MemoryReceipts::default()),
    ));
    assert!(!payments.is_stand_in());
    let started = payments
        .create(PaymentRequest {
            site: "kilele-bakery-sample".into(),
            order_id: "KB-1042".into(),
            amount_usd_cents: 1234,
            description: "Kilele Bakery order KB-1042 (Sample)".into(),
            customer: Some("vac_test".into()),
            return_url: "https://kilele-bakery-sample.paypal.agentrix.example/orders/KB-1042/paid".into(),
            cancel_url: "https://kilele-bakery-sample.paypal.agentrix.example/orders/KB-1042/pay?cancelled=1".into(),
        })
        .await
        .unwrap();
    let PaymentStart::Redirect {
        provider_ref,
        approve_url,
        ..
    } = started
    else {
        panic!("expected PayPal's approval page, got {started:?}");
    };
    assert_eq!(provider_ref, "ORDER1");
    assert!(approve_url.contains("token=ORDER1"));
    let captured = payments
        .capture("kilele-bakery-sample", "ORDER1")
        .await
        .unwrap();
    assert_eq!(captured.state, "captured");
    assert_eq!(captured.provider_capture_id.as_deref(), Some("CAP1"));
    // Once: the second capture answers from the record.
    let again = payments
        .capture("kilele-bakery-sample", "ORDER1")
        .await
        .unwrap();
    assert_eq!(again.state, "captured");
    // saas_node::samples::follow reads it from customer_payments.
    let (capture_id,): (Option<String>,) =
        sqlx::query_as("SELECT capture_id FROM customer_payments WHERE provider_ref = 'ORDER1'")
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(capture_id.as_deref(), Some("CAP1"));
}
