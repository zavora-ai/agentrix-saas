//! Shared by the wiremock tests: a mock PayPal that hands out OAuth tokens, and a gateway over
//! a fresh database pointed at it.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use agentrix_effects::Effects;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use agentrix_host::testing::MemoryReceipts;
use agentrix_types::SiteId;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// A mock PayPal that answers the OAuth token call; tests mount the rest.
pub async fn paypal() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "access_token": "A21AA-test", "expires_in": 32400 })),
        )
        .mount(&server)
        .await;
    server
}

/// A gateway over a fresh migrated database, talking to `server` with webhook id `WH-1`.
pub async fn gateway(server: &MockServer) -> (Effects, Arc<MemoryReceipts>) {
    gateway_with(server, None).await
}

/// The same with a per-request timeout.
pub async fn gateway_with(
    server: &MockServer,
    timeout: Option<Duration>,
) -> (Effects, Arc<MemoryReceipts>) {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    let receipts = Arc::new(MemoryReceipts::default());
    let mut client = PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: Some("WH-1".into()),
            environment: Environment::Custom(server.uri()),
        },
        reqwest::Client::new(),
    );
    if let Some(timeout) = timeout {
        client = client.with_timeout(timeout);
    }
    (
        Effects::new(ctx.pool.clone(), Some(Arc::new(client)), receipts.clone()),
        receipts,
    )
}

/// A gateway with no PayPal keys.
pub async fn gateway_off() -> Effects {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    Effects::new(ctx.pool.clone(), None, Arc::new(MemoryReceipts::default()))
}

pub fn site() -> SiteId {
    SiteId::new("site_agentrix_saas")
}

/// The kinds of the receipts signed so far, in order.
pub fn kinds(receipts: &MemoryReceipts) -> Vec<String> {
    receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str().to_owned())
        .collect()
}

/// The details of the receipts signed so far, in order.
pub fn details(receipts: &MemoryReceipts) -> Vec<Value> {
    receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.detail.clone().unwrap_or(Value::Null))
        .collect()
}

/// The requests the mock received for `method` and `path`, in order.
pub async fn requests(server: &MockServer, verb: &str, at: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == at)
        .collect()
}

/// A request's `PayPal-Request-Id`.
pub fn request_id(request: &Request) -> Option<String> {
    request
        .headers
        .get("paypal-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// PayPal's signature check: SUCCESS for `transmission_sig: good` with webhook `WH-1`, FAILURE
/// for anything else.
pub async fn mount_signature_check(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .and(wiremock::matchers::body_partial_json(
            json!({ "transmission_sig": "good", "webhook_id": "WH-1" }),
        ))
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

/// The headers PayPal sends with a webhook, signed `sig`.
pub fn webhook_headers(sig: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    for (name, value) in [
        ("paypal-auth-algo", "SHA256withRSA"),
        (
            "paypal-cert-url",
            "https://api.sandbox.paypal.com/v1/notifications/certs/CERT-360caa42-fca2a594-a5cafa77",
        ),
        (
            "paypal-transmission-id",
            "69cd13f0-d67a-11e5-baa3-778b53f4ae55",
        ),
        ("paypal-transmission-time", "2026-11-10T09:00:00Z"),
        ("paypal-transmission-sig", sig),
    ] {
        headers.insert(name, value.parse().unwrap());
    }
    headers
}
