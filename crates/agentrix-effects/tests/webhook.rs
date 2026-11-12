//! `POST /effects/paypal/webhook` against a wiremock PayPal: only events PayPal's signature
//! check accepts are applied, each once.

mod common;

use agentrix_effects::{Effects, PlanPrice, SubscriptionStart};
use agentrix_types::Money;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use common::{gateway, mount_signature_check, paypal, requests, site, webhook_headers};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Posts `event` to the webhook route with `headers`; returns the status and the JSON answer.
async fn post(effects: &Effects, headers: &HeaderMap, event: &[u8]) -> (StatusCode, Value) {
    let mut request = Request::post("/effects/paypal/webhook")
        .header("content-type", "application/json")
        .body(Body::from(event.to_vec()))
        .unwrap();
    request.headers_mut().extend(headers.clone());
    let response = effects.router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A subscription waiting for approval (`I-1`), with PayPal reporting it `status`.
async fn pending_subscription(server: &MockServer, effects: &Effects, status: &str) -> String {
    Mock::given(method("POST"))
        .and(path("/v1/catalogs/products"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "PROD-1" })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/plans"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "P-1" })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/subscriptions"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "I-1", "status": "APPROVAL_PENDING",
            "links": [{ "rel": "approve", "href": "https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token=BA-1" }],
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/billing/subscriptions/I-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "I-1", "status": status, "plan_id": "P-1",
            "billing_info": { "next_billing_time": "2026-12-10T10:00:00Z" },
        })))
        .mount(server)
        .await;
    effects
        .start_subscription(SubscriptionStart {
            site: site(),
            plan: PlanPrice {
                plan_id: "saas-starter".into(),
                version: 1,
                name: "Agentrix SaaS Starter".into(),
                monthly: Money {
                    currency: "USD".into(),
                    minor: 1900,
                },
                annual: None,
            },
            subscriber_email: None,
            return_url: "https://saas.agentrix.website/billing/done".into(),
            cancel_url: "https://saas.agentrix.website/billing".into(),
            starts_at: None,
        })
        .await
        .unwrap()
        .provider_ref
}

fn activated(id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": id, "event_type": "BILLING.SUBSCRIPTION.ACTIVATED", "resource_type": "subscription",
        "resource": { "id": "I-1", "status": "ACTIVE" },
    }))
    .unwrap()
}

async fn state(effects: &Effects) -> String {
    effects.subscriptions_for_site(&site()).await.unwrap()[0]
        .state
        .clone()
}

#[tokio::test]
async fn a_signed_event_is_applied_and_its_signature_checked_with_paypal() {
    let server = paypal().await;
    mount_signature_check(&server).await;
    let (effects, _) = gateway(&server).await;
    let provider_ref = pending_subscription(&server, &effects, "ACTIVE").await;
    assert_eq!(provider_ref, "I-1");

    let (status, body) = post(&effects, &webhook_headers("good"), &activated("WH-EVT-1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "applied": true }));
    assert_eq!(state(&effects).await, "active", "read back from PayPal");

    // PayPal was asked to check exactly what arrived: its five headers, our webhook id and
    // the event itself.
    let checks = requests(
        &server,
        "POST",
        "/v1/notifications/verify-webhook-signature",
    )
    .await;
    assert_eq!(checks.len(), 1);
    let check: Value = checks[0].body_json().unwrap();
    assert_eq!(check["webhook_id"], "WH-1");
    assert_eq!(check["auth_algo"], "SHA256withRSA");
    assert_eq!(
        check["transmission_id"],
        "69cd13f0-d67a-11e5-baa3-778b53f4ae55"
    );
    assert_eq!(check["transmission_time"], "2026-11-10T09:00:00Z");
    assert!(
        check["cert_url"]
            .as_str()
            .unwrap()
            .starts_with("https://api.sandbox.paypal.com/")
    );
    assert_eq!(check["webhook_event"]["id"], "WH-EVT-1");
}

#[tokio::test]
async fn a_forged_event_changes_nothing() {
    let server = paypal().await;
    mount_signature_check(&server).await;
    let (effects, _) = gateway(&server).await;
    pending_subscription(&server, &effects, "ACTIVE").await;

    let (status, body) = post(&effects, &webhook_headers("forged"), &activated("WH-EVT-1")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["message"], "Signature not verified.");
    let (status, _) = post(&effects, &HeaderMap::new(), &activated("WH-EVT-1")).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no signature headers at all"
    );
    assert_eq!(state(&effects).await, "pending_approval");
    assert!(
        requests(&server, "GET", "/v1/billing/subscriptions/I-1")
            .await
            .is_empty(),
        "a forged event never reaches PayPal's subscription"
    );

    // Refused events aren't remembered: the genuine one with the same id still applies.
    let (status, body) = post(&effects, &webhook_headers("good"), &activated("WH-EVT-1")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "applied": true })));
    assert_eq!(state(&effects).await, "active");

    let (status, _) = post(&effects, &webhook_headers("good"), b"not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_same_event_twice_applies_once() {
    let server = paypal().await;
    mount_signature_check(&server).await;
    let (effects, receipts) = gateway(&server).await;
    pending_subscription(&server, &effects, "ACTIVE").await;

    let first = post(&effects, &webhook_headers("good"), &activated("WH-EVT-7")).await;
    let again = post(&effects, &webhook_headers("good"), &activated("WH-EVT-7")).await;
    assert_eq!(first, (StatusCode::OK, json!({ "applied": true })));
    assert_eq!(
        again,
        (StatusCode::OK, json!({ "applied": false })),
        "PayPal's retry of the same event id"
    );
    assert_eq!(
        requests(&server, "GET", "/v1/billing/subscriptions/I-1")
            .await
            .len(),
        1,
        "read back from PayPal once"
    );
    assert_eq!(
        common::kinds(&receipts),
        ["subscription.changed", "subscription.changed"],
        "started, then activated once"
    );
    let events: Vec<(String, String)> =
        sqlx::query_as("SELECT id, kind FROM paypal_events ORDER BY id")
            .fetch_all(effects.pool_for_tests())
            .await
            .unwrap();
    assert_eq!(
        events,
        [(
            "WH-EVT-7".to_owned(),
            "BILLING.SUBSCRIPTION.ACTIVATED".to_owned()
        )]
    );
}

#[tokio::test]
async fn when_paypal_cannot_check_it_paypal_retries_later() {
    let server = paypal().await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    mount_signature_check(&server).await;
    let (effects, _) = gateway(&server).await;
    pending_subscription(&server, &effects, "ACTIVE").await;

    let (status, _) = post(&effects, &webhook_headers("good"), &activated("WH-EVT-9")).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "not applied, not refused: PayPal sends it again"
    );
    assert_eq!(state(&effects).await, "pending_approval");
    let (status, body) = post(&effects, &webhook_headers("good"), &activated("WH-EVT-9")).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "applied": true })));
}

#[tokio::test]
async fn without_keys_the_route_says_payments_are_off() {
    let effects = common::gateway_off().await;
    let (status, _) = post(&effects, &webhook_headers("good"), &activated("WH-EVT-1")).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// A delivery as PayPal sends it: keys not in alphabetical order, and PayPal's own spacing.
const DELIVERY: &str = r#"{"id": "WH-ORDER-7", "event_type": "CHECKOUT.ORDER.APPROVED", "resource": {"status": "APPROVED", "id": "ORDER-UNKNOWN"}, "create_time": "2026-11-11T13:14:00Z"}"#;

/// PayPal's check as it really behaves: SUCCESS only when the event it is asked about is the
/// delivery's own bytes, keys in their order; a copy re-serialized any other way fails.
async fn mount_strict_check(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .and(body_string_contains(DELIVERY))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "verification_status": "SUCCESS" })),
        )
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "verification_status": "FAILURE" })),
        )
        .with_priority(2)
        .mount(server)
        .await;
}

#[tokio::test]
async fn paypal_checks_the_event_byte_for_byte_as_it_was_delivered() {
    let server = paypal().await;
    mount_strict_check(&server).await;
    let (effects, _) = gateway(&server).await;

    let (status, body) = post(&effects, &webhook_headers("good"), DELIVERY.as_bytes()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Verified and recorded; not an order of ours, so nothing to apply.
    assert_eq!(body, json!({ "applied": false }));
    let (recorded,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM paypal_events WHERE id = 'WH-ORDER-7'")
            .fetch_one(effects.pool_for_tests())
            .await
            .unwrap();
    assert_eq!(recorded, 1);

    // PayPal's retry of the same delivery, through the call node makes: verified again,
    // counted as a duplicate, nothing changed.
    assert!(
        !effects
            .handle_webhook(&webhook_headers("good"), DELIVERY.as_bytes())
            .await
            .unwrap()
    );
    let checks = requests(
        &server,
        "POST",
        "/v1/notifications/verify-webhook-signature",
    )
    .await;
    assert_eq!(checks.len(), 2);
    assert!(
        String::from_utf8_lossy(&checks[0].body).contains(DELIVERY),
        "the event went to PayPal as delivered"
    );
}

#[tokio::test]
async fn a_delivery_without_paypals_headers_is_refused_without_asking_paypal() {
    let server = paypal().await;
    mount_strict_check(&server).await;
    let (effects, _) = gateway(&server).await;
    let mut headers = webhook_headers("good");
    headers.remove("paypal-transmission-sig");
    let (status, _) = post(&effects, &headers, DELIVERY.as_bytes()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = post(&effects, &webhook_headers("good"), b"not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        requests(
            &server,
            "POST",
            "/v1/notifications/verify-webhook-signature"
        )
        .await
        .is_empty()
    );
}
