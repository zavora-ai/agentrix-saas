//! The PayPal edge cases for the ecosystem run against a wiremock PayPal: webhook events listed
//! and one resent to our own webhook (its second delivery counted, nothing changed), an
//! unclaimed payout item of the company's own payout cancelled, and PayPal's sandbox error
//! simulation carried by Orders v2 and Payments v2 calls only, recorded as such.

mod common;

use std::sync::Arc;

use agentrix_effects::paypal::{
    Environment, MOCK_RESPONSE_HEADER, MockResponse, PayPalClient, PayPalConfig,
};
use agentrix_effects::{
    CaptureState, EffectError, Effects, PaymentRequest, SANDBOX_ERROR_SIMULATION,
    WebhookEventsQuery,
};
use agentrix_host::testing::MemoryReceipts;
use agentrix_types::{Money, SiteId};
use common::{gateway, mount_signature_check, paypal, requests, site, webhook_headers};
use reqwest::Method;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn event(id: &str, kind: &str) -> Value {
    json!({
        "id": id, "event_type": kind, "resource_type": "checkout-order",
        "summary": "An order has been approved by buyer", "create_time": "2026-11-11T15:00:00Z",
        "resource": { "id": "ORDER-UNKNOWN" },
    })
}

#[tokio::test]
async fn listed_events_say_whether_our_webhook_received_them() {
    let server = paypal().await;
    Mock::given(method("GET"))
        .and(path("/v1/notifications/webhooks-events"))
        .and(query_param("page_size", "20"))
        .and(query_param("event_type", "CHECKOUT.ORDER.APPROVED"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "events": [event("WH-AAA", "CHECKOUT.ORDER.APPROVED"), event("WH-BBB", "CHECKOUT.ORDER.APPROVED")],
            "count": 2,
        })))
        .mount(&server)
        .await;
    mount_signature_check(&server).await;
    let (effects, _) = gateway(&server).await;
    // Our webhook got the first one.
    effects
        .handle_webhook(
            &webhook_headers("good"),
            &serde_json::to_vec(&event("WH-AAA", "CHECKOUT.ORDER.APPROVED")).unwrap(),
        )
        .await
        .unwrap();
    let events = effects
        .list_webhook_events(&WebhookEventsQuery {
            page_size: Some(20),
            event_type: Some("CHECKOUT.ORDER.APPROVED".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id, "WH-AAA");
    assert!(events[0].received);
    assert_eq!(events[0].resource_id.as_deref(), Some("ORDER-UNKNOWN"));
    assert!(!events[1].received, "the one we missed");
}

#[tokio::test]
async fn a_resend_goes_to_our_own_webhook_and_its_second_delivery_changes_nothing() {
    let server = paypal().await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/webhooks-events/WH-AAA/resend"))
        .and(body_json(json!({ "webhook_ids": ["WH-1"] })))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(event("WH-AAA", "CHECKOUT.ORDER.APPROVED")),
        )
        .expect(1)
        .mount(&server)
        .await;
    mount_signature_check(&server).await;
    let (effects, _) = gateway(&server).await;
    let headers = webhook_headers("good");
    let first = event("WH-AAA", "CHECKOUT.ORDER.APPROVED");
    effects.handle_webhook(&headers, &serde_json::to_vec(&first).unwrap()).await.unwrap();

    let resent = effects.resend_webhook_event("WH-AAA").await.unwrap();
    assert_eq!(resent.id, "WH-AAA");
    assert!(resent.received);
    // PayPal delivers it again: handled once, the second counted and nothing applied.
    assert!(!effects.handle_webhook(&headers, &serde_json::to_vec(&first).unwrap()).await.unwrap());
    let (duplicates,): (i64,) =
        sqlx::query_as("SELECT duplicates FROM paypal_events WHERE id = 'WH-AAA'")
            .fetch_one(effects.pool_for_tests())
            .await
            .unwrap();
    assert_eq!(duplicates, 1);

    let kept = effects.edge_cases(10).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].kind, "webhook.resend");
    assert_eq!(kept[0].subject, "WH-AAA");
    assert_eq!(kept[0].outcome, "executed");
    assert!(kept[0].detail.contains("WH-1"));
}

#[tokio::test]
async fn nothing_is_resent_without_our_own_webhook_id_or_with_a_strange_event_id() {
    let server = paypal().await;
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    let client = PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: None,
            environment: Environment::Custom(server.uri()),
        },
        reqwest::Client::new(),
    );
    let effects = Effects::new(
        ctx.pool.clone(),
        Some(Arc::new(client)),
        Arc::new(MemoryReceipts::default()),
    );
    let refused = effects.resend_webhook_event("WH-AAA").await.unwrap_err();
    assert!(matches!(refused, EffectError::BadRequest(_)), "{refused:?}");

    let (effects, _) = gateway(&server).await;
    for strange in ["", "WH-AAA/../../webhooks/WH-2", "WH AAA"] {
        assert!(matches!(
            effects.resend_webhook_event(strange).await,
            Err(EffectError::BadRequest(_))
        ));
    }
    let sent = server.received_requests().await.unwrap_or_default();
    assert!(sent.iter().all(|r| !r.url.path().contains("/resend")));
}

/// A payout of `site` whose PayPal item is `item`.
async fn stored_payout(effects: &Effects, site: &SiteId, item: &str) {
    sqlx::query(
        "INSERT INTO payouts (id, site_id, fingerprint, receiver, currency, amount_minor, note, state, batch_id, batch_status, item_id, item_status, sandbox, created_at, updated_at)
         VALUES (?, ?, ?, 'no-account@example.com', 'USD', 4000, 'November wages', 'executed', ?, 'SUCCESS', ?, 'UNCLAIMED', 1, 1, 1)",
    )
    .bind(format!("po_{item}"))
    .bind(site.as_str())
    .bind(format!("sha256:{item}"))
    .bind(format!("BATCH-{item}"))
    .bind(item)
    .execute(effects.pool_for_tests())
    .await
    .unwrap();
}

fn item(id: &str, status: &str) -> Value {
    json!({
        "payout_item_id": id, "transaction_status": status, "payout_batch_id": format!("BATCH-{id}"),
        "payout_item": {
            "recipient_type": "EMAIL", "receiver": "no-account@example.com",
            "amount": { "value": "40.00", "currency": "USD" }, "sender_item_id": format!("po_{id}"),
        },
    })
}

#[tokio::test]
async fn an_unclaimed_item_of_our_own_payout_is_cancelled_and_the_money_returns() {
    let server = paypal().await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts-item/ITEMU1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(item("ITEMU1", "UNCLAIMED")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts-item/ITEMU1/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(item("ITEMU1", "RETURNED")))
        .expect(1)
        .mount(&server)
        .await;
    let (effects, _) = gateway(&server).await;
    stored_payout(&effects, &site(), "ITEMU1").await;

    // Another company can't cancel it, and PayPal isn't asked.
    let other = SiteId::new("site_someone_else");
    assert!(matches!(
        effects.cancel_unclaimed_payout_item(&other, "ITEMU1").await,
        Err(EffectError::NotFound)
    ));
    assert!(
        requests(&server, "POST", "/v1/payments/payouts-item/ITEMU1/cancel")
            .await
            .is_empty()
    );

    let cancelled = effects
        .cancel_unclaimed_payout_item(&site(), "ITEMU1")
        .await
        .unwrap();
    assert_eq!(cancelled.status, "RETURNED");
    let (status,): (String,) =
        sqlx::query_as("SELECT item_status FROM payouts WHERE item_id = 'ITEMU1'")
            .fetch_one(effects.pool_for_tests())
            .await
            .unwrap();
    assert_eq!(status, "RETURNED");
    let kept = effects.edge_cases(10).await.unwrap();
    assert_eq!(kept[0].kind, "payout_item.cancel");
    assert_eq!(kept[0].site_id.as_deref(), Some(site().as_str()));
    assert!(kept[0].detail.contains("UNCLAIMED") && kept[0].detail.contains("RETURNED"));
}

#[tokio::test]
async fn a_claimed_item_is_not_cancelled() {
    let server = paypal().await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts-item/ITEMS1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(item("ITEMS1", "SUCCESS")))
        .mount(&server)
        .await;
    let (effects, _) = gateway(&server).await;
    stored_payout(&effects, &site(), "ITEMS1").await;
    let refused = effects
        .cancel_unclaimed_payout_item(&site(), "ITEMS1")
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("SUCCESS"), "{refused}");
    assert!(
        requests(&server, "POST", "/v1/payments/payouts-item/ITEMS1/cancel")
            .await
            .is_empty()
    );
    assert!(effects.edge_cases(10).await.unwrap().is_empty());
}

const DECLINED: &str = r#"{"mock_application_codes":"INSTRUMENT_DECLINED"}"#;

async fn mount_orders(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v2/checkout/orders"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "ORDER-1", "status": "PAYER_ACTION_REQUIRED",
            "links": [{ "rel": "payer-action", "href": "https://www.sandbox.paypal.com/checkoutnow?token=ORDER-1" }],
        })))
        .mount(server)
        .await;
    // The sandbox answers the simulation header with that error.
    Mock::given(method("POST"))
        .and(path("/v2/checkout/orders/ORDER-1/capture"))
        .and(header(MOCK_RESPONSE_HEADER, DECLINED))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "name": "UNPROCESSABLE_ENTITY", "message": "The instrument presented was either declined by the processor or bank.",
            "details": [{ "issue": "INSTRUMENT_DECLINED" }],
        })))
        .with_priority(1)
        .mount(server)
        .await;
}

fn payment() -> PaymentRequest {
    PaymentRequest {
        site: site(),
        site_name: "Kilele Bakery (Sample)".into(),
        order_id: "KB-1060".into(),
        amount: Money {
            currency: "USD".into(),
            minor: 1_200,
        },
        description: "Kilele Bakery order KB-1060 (Sample)".into(),
        customer: None,
        return_url: "https://kilele-bakery-sample.paypal.agentrix.website/orders/KB-1060/paid"
            .into(),
        cancel_url: "https://kilele-bakery-sample.paypal.agentrix.website/orders/KB-1060".into(),
    }
}

#[tokio::test]
async fn the_error_simulation_rides_on_orders_calls_only_and_is_recorded_as_such() {
    let server = paypal().await;
    mount_orders(&server).await;
    let (effects, _) = gateway(&server).await;
    let started = effects.create_payment(payment()).await.unwrap();
    // The plain gateway sends no simulation header.
    let created = requests(&server, "POST", "/v2/checkout/orders").await;
    assert!(created[0].headers.get(MOCK_RESPONSE_HEADER).is_none());

    let simulating = effects
        .simulating(MockResponse::new("INSTRUMENT_DECLINED").unwrap())
        .unwrap();
    assert_eq!(
        simulating.simulation().map(MockResponse::code),
        Some("INSTRUMENT_DECLINED")
    );
    let outcome = simulating
        .capture(&site(), &started.provider_ref)
        .await
        .unwrap();
    assert_eq!(outcome.state, CaptureState::Declined);
    let captures = requests(&server, "POST", "/v2/checkout/orders/ORDER-1/capture").await;
    assert_eq!(
        captures[0]
            .headers
            .get(MOCK_RESPONSE_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some(DECLINED)
    );

    let kept = effects.edge_cases(10).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].kind, "simulation");
    assert_eq!(kept[0].label.as_deref(), Some(SANDBOX_ERROR_SIMULATION));
    assert_eq!(kept[0].subject, "/v2/checkout/orders/ORDER-1/capture");
    assert_eq!(kept[0].outcome, "not_executed");
    assert!(kept[0].detail.contains("INSTRUMENT_DECLINED"));
}

#[tokio::test]
async fn the_simulation_is_refused_where_paypal_doesnt_document_it_and_on_live() {
    let server = paypal().await;
    let sandbox = PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: None,
            environment: Environment::Custom(server.uri()),
        },
        reqwest::Client::new(),
    );
    let mock = MockResponse::new("INSUFFICIENT_FUNDS").unwrap();
    for not_documented in [
        "/v1/payments/payouts",
        "/v1/billing/subscriptions",
        "/v1/notifications/webhooks-events",
    ] {
        assert!(
            sandbox
                .request_simulating(Method::POST, not_documented, None, None, Some(&mock))
                .await
                .is_err()
        );
    }
    assert!(MockResponse::applies_to("/v2/payments/captures/CAP-1/refund"));
    assert!(MockResponse::new("instrument declined").is_err());
    assert!(MockResponse::new("").is_err());
    let paths: Vec<String> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert!(
        paths.iter().all(|p| p == "/v1/oauth2/token"),
        "nothing but a token went out: {paths:?}"
    );

    // Never against live PayPal: refused before any request.
    let live = PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: None,
            environment: Environment::Live,
        },
        reqwest::Client::new(),
    );
    assert!(
        live.request_simulating(
            Method::POST,
            "/v2/checkout/orders/X/capture",
            None,
            None,
            Some(&mock)
        )
        .await
        .is_err()
    );
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    let effects = Effects::new(
        ctx.pool.clone(),
        Some(Arc::new(live)),
        Arc::new(MemoryReceipts::default()),
    );
    assert!(matches!(
        effects.simulating(mock),
        Err(EffectError::BadRequest(_))
    ));
}
