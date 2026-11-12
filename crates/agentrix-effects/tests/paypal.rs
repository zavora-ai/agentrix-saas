//! The gateway against a small in-process PayPal that behaves like the sandbox: OAuth, Orders
//! v2 with `PayPal-Request-Id`, capture, refunds, Subscriptions and the webhook signature check.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use agentrix_effects::{
    CaptureState, EffectError, Effects, Outcome, PaymentRequest, PlanPrice, SubscriptionStart,
};
use agentrix_host::testing::MemoryReceipts;
use agentrix_types::{Money, SiteId};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

#[derive(Default)]
struct Mock {
    orders: HashMap<String, (String, Value, bool)>, // request id → (order id, amount, approved)
    by_id: HashMap<String, String>,                 // order id → request id
    captures: usize,
    refunds: HashMap<String, String>,       // request id → refund id
    subscriptions: HashMap<String, String>, // id → status
    subscription_plans: HashMap<String, String>, // id → PayPal plan id, once revised
    revisions: Vec<(String, Value)>,        // (subscription id, body)
    plans: usize,
    plan_bodies: Vec<Value>,
}

type Shared = Arc<Mutex<Mock>>;

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("paypal-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none")
        .to_owned()
}

async fn token() -> Json<Value> {
    Json(json!({ "access_token": "A21AA-test", "expires_in": 32400 }))
}

async fn create_order(
    State(m): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let key = request_id(&headers);
    let mut m = m.lock().unwrap();
    let n = m.orders.len() + 1;
    let amount = body["purchase_units"][0]["amount"].clone();
    let (id, _, _) = m
        .orders
        .entry(key.clone())
        .or_insert((format!("ORDER-{n}"), amount, false))
        .clone();
    m.by_id.insert(id.clone(), key);
    Json(json!({
        "id": id, "status": "PAYER_ACTION_REQUIRED",
        "links": [{ "rel": "payer-action", "href": format!("https://www.sandbox.paypal.com/checkoutnow?token={id}") }],
    }))
}

fn order_body(id: &str, amount: &Value, captured: bool) -> Value {
    if captured {
        json!({ "id": id, "status": "COMPLETED", "purchase_units": [{ "payments": { "captures": [{ "id": format!("CAP-{id}"), "status": "COMPLETED", "amount": amount }] } }] })
    } else {
        json!({ "id": id, "status": "APPROVED" })
    }
}

async fn capture(State(m): State<Shared>, Path(id): Path<String>) -> Response {
    let mut m = m.lock().unwrap();
    let key = m.by_id.get(&id).cloned().unwrap_or_default();
    let Some((_, amount, approved)) = m.orders.get(&key).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !approved {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "name": "UNPROCESSABLE_ENTITY", "message": "The requested action could not be performed.", "details": [{ "issue": "ORDER_NOT_APPROVED" }] }))).into_response();
    }
    m.captures += 1;
    Json(order_body(&id, &amount, true)).into_response()
}

async fn get_order(State(m): State<Shared>, Path(id): Path<String>) -> Response {
    let m = m.lock().unwrap();
    let key = m.by_id.get(&id).cloned().unwrap_or_default();
    match m.orders.get(&key) {
        Some((_, amount, approved)) => {
            Json(order_body(&id, amount, *approved && m.captures > 0)).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn refund(
    State(m): State<Shared>,
    headers: HeaderMap,
    Path(_capture): Path<String>,
) -> Json<Value> {
    let mut m = m.lock().unwrap();
    let n = m.refunds.len() + 1;
    let id = m
        .refunds
        .entry(request_id(&headers))
        .or_insert(format!("REFUND-{n}"))
        .clone();
    Json(json!({ "id": id, "status": "COMPLETED" }))
}

async fn product() -> Json<Value> {
    Json(json!({ "id": "PROD-1" }))
}

async fn plan(State(m): State<Shared>, Json(body): Json<Value>) -> Json<Value> {
    let mut m = m.lock().unwrap();
    m.plans += 1;
    m.plan_bodies.push(body);
    Json(json!({ "id": format!("P-{}", m.plans) }))
}

async fn create_subscription(State(m): State<Shared>) -> Json<Value> {
    let mut m = m.lock().unwrap();
    let id = format!("I-{}", m.subscriptions.len() + 1);
    m.subscriptions
        .insert(id.clone(), "APPROVAL_PENDING".into());
    Json(
        json!({ "id": id, "status": "APPROVAL_PENDING", "links": [{ "rel": "approve", "href": format!("https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token={id}") }] }),
    )
}

async fn get_subscription(State(m): State<Shared>, Path(id): Path<String>) -> Json<Value> {
    let m = m.lock().unwrap();
    let status = m.subscriptions.get(&id).cloned().unwrap_or_default();
    let plan = m
        .subscription_plans
        .get(&id)
        .cloned()
        .unwrap_or_else(|| "P-1".into());
    Json(
        json!({ "id": id, "status": status, "plan_id": plan, "billing_info": { "next_billing_time": "2026-12-08T10:00:00Z" } }),
    )
}

async fn revise(
    State(m): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    m.lock().unwrap().revisions.push((id.clone(), body.clone()));
    Json(
        json!({ "plan_id": body["plan_id"], "links": [{ "rel": "approve", "href": format!("https://www.sandbox.paypal.com/webapps/billing/subscriptions/update?ba_token={id}") }] }),
    )
}

/// The five headers PayPal sends with every delivery, signed `sig`.
fn paypal_headers(sig: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("paypal-auth-algo", "SHA256withRSA"),
        (
            "paypal-cert-url",
            "https://api.sandbox.paypal.com/v1/notifications/certs/CERT-1",
        ),
        ("paypal-transmission-id", "69cd13f0-d67a-11e5-baa3-778b53f4ae55"),
        ("paypal-transmission-time", "2026-11-10T09:00:00Z"),
        ("paypal-transmission-sig", sig),
    ] {
        headers.insert(name, value.parse().unwrap());
    }
    headers
}

async fn verify(Json(body): Json<Value>) -> Json<Value> {
    let ok = body["transmission_sig"] == "good" && body["webhook_id"] == "WH-1";
    Json(json!({ "verification_status": if ok { "SUCCESS" } else { "FAILURE" } }))
}

async fn paypal() -> (String, Shared) {
    let state: Shared = Arc::default();
    let app = Router::new()
        .route("/v1/oauth2/token", post(token))
        .route("/v2/checkout/orders", post(create_order))
        .route("/v2/checkout/orders/{id}", get(get_order))
        .route("/v2/checkout/orders/{id}/capture", post(capture))
        .route("/v2/payments/captures/{id}/refund", post(refund))
        .route("/v1/catalogs/products", post(product))
        .route("/v1/billing/plans", post(plan))
        .route("/v1/billing/subscriptions", post(create_subscription))
        .route("/v1/billing/subscriptions/{id}", get(get_subscription))
        .route("/v1/billing/subscriptions/{id}/revise", post(revise))
        .route("/v1/notifications/verify-webhook-signature", post(verify))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, state)
}

async fn gateway(base: Option<&str>) -> (Effects, Arc<MemoryReceipts>) {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    let receipts = Arc::new(MemoryReceipts::default());
    let client = base.map(|b| {
        Arc::new(PayPalClient::new(
            PayPalConfig {
                client_id: "id".into(),
                client_secret: "secret".into(),
                webhook_id: Some("WH-1".into()),
                environment: Environment::Custom(b.into()),
            },
            reqwest::Client::new(),
        ))
    });
    (
        Effects::new(ctx.pool.clone(), client, receipts.clone()),
        receipts,
    )
}

fn kilele() -> SiteId {
    SiteId::new("site_kilele_sample")
}

fn order(order_id: &str, minor: i64) -> PaymentRequest {
    PaymentRequest {
        site: kilele(),
        site_name: "Kilele Bakery (Sample)".into(),
        order_id: order_id.into(),
        amount: Money {
            currency: "USD".into(),
            minor,
        },
        description: format!("Kilele Bakery order {order_id} (Sample)"),
        customer: None,
        return_url: "https://kilele-bakery-sample.agentrix.website/orders/K-1042/paid".into(),
        cancel_url: "https://kilele-bakery-sample.agentrix.website/orders/K-1042".into(),
    }
}

fn kinds(receipts: &MemoryReceipts) -> Vec<String> {
    receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn without_keys_it_says_so_and_charges_nothing() {
    let (effects, _) = gateway(None).await;
    assert!(!effects.enabled());
    let e = effects
        .create_payment(order("K-1042", 1850))
        .await
        .unwrap_err();
    assert!(matches!(e, EffectError::NotConfigured));
    assert_eq!(
        e.to_string(),
        "Payments aren't switched on for this sample yet. Nothing was charged."
    );
}

#[tokio::test]
async fn one_order_per_request_captured_once() {
    let (base, mock) = paypal().await;
    let (effects, receipts) = gateway(Some(&base)).await;
    let first = effects.create_payment(order("K-1042", 1850)).await.unwrap();
    let again = effects.create_payment(order("K-1042", 1850)).await.unwrap();
    assert_eq!(first, again, "a double tap is one payment");
    assert_eq!(mock.lock().unwrap().orders.len(), 1, "and one PayPal order");
    assert!(first.approve_url.contains(&first.provider_ref));
    let other = effects.create_payment(order("K-1042", 1900)).await.unwrap();
    assert_ne!(
        other.payment_id, first.payment_id,
        "a different amount is a different payment"
    );

    // Back from PayPal before approving: nothing charged.
    let early = effects
        .capture(&kilele(), &first.provider_ref)
        .await
        .unwrap();
    assert_eq!(
        (early.state, early.outcome),
        (CaptureState::Cancelled, Outcome::NotExecuted)
    );
    assert_eq!(
        early.message,
        "You haven't approved the payment on PayPal yet. Nothing was charged."
    );

    // Approved on PayPal's page, then captured once.
    let key = mock.lock().unwrap().by_id[&first.provider_ref].clone();
    mock.lock().unwrap().orders.get_mut(&key).unwrap().2 = true;
    let paid = effects
        .capture(&kilele(), &first.provider_ref)
        .await
        .unwrap();
    assert_eq!(
        (paid.state, paid.outcome),
        (CaptureState::Captured, Outcome::Executed)
    );
    assert_eq!(
        paid.captured,
        Some(Money {
            currency: "USD".into(),
            minor: 1850
        })
    );
    assert_eq!(paid.message, "Paid USD 18.50 with PayPal.");
    let twice = effects
        .capture(&kilele(), &first.provider_ref)
        .await
        .unwrap();
    assert_eq!(twice.state, CaptureState::Captured);
    assert_eq!(mock.lock().unwrap().captures, 1, "captured once");
    assert_eq!(
        kinds(&receipts),
        ["payment.started", "payment.started", "payment.captured"]
    );

    // Another site can't capture or read it.
    assert!(matches!(
        effects
            .capture(&SiteId::new("site_classy_sample"), &first.provider_ref)
            .await,
        Err(EffectError::NotFound)
    ));
    // PayPal only sends people back over https (or to localhost in development).
    let mut bad = order("K-2", 100);
    bad.return_url = "http://kilele.example/paid".into();
    assert!(matches!(
        effects.create_payment(bad).await,
        Err(EffectError::BadRequest(_))
    ));
}

#[tokio::test]
async fn refunds_need_the_owners_approval_of_that_exact_effect() {
    let (base, mock) = paypal().await;
    let (effects, receipts) = gateway(Some(&base)).await;
    let started = effects.create_payment(order("K-1042", 1850)).await.unwrap();
    let key = mock.lock().unwrap().by_id[&started.provider_ref].clone();
    mock.lock().unwrap().orders.get_mut(&key).unwrap().2 = true;
    effects
        .capture(&kilele(), &started.provider_ref)
        .await
        .unwrap();
    let payment = effects
        .payment(&kilele(), &started.payment_id)
        .await
        .unwrap()
        .unwrap();

    assert!(
        effects
            .refund_request(
                &payment,
                Money {
                    currency: "USD".into(),
                    minor: 5000
                },
                "too much"
            )
            .is_err(),
        "never above what was captured"
    );
    let effect = effects
        .refund_request(
            &payment,
            Money {
                currency: "USD".into(),
                minor: 600,
            },
            "One loaf was missing",
        )
        .unwrap();
    assert_eq!(effect.tool, "payment.refund");
    let fp = effect.fingerprint().unwrap().to_string();
    let hex = fp.trim_start_matches("sha256:");

    let forged =
        json!({ "method": "passkey", "account": "owner-1", "fingerprint": "00".repeat(32) });
    assert!(matches!(
        effects.refund(&effect, &forged).await,
        Err(EffectError::NotApproved)
    ));
    assert!(
        mock.lock().unwrap().refunds.is_empty(),
        "nothing sent to PayPal"
    );

    let approved = json!({ "method": "passkey", "account": "owner-1", "fingerprint": hex });
    let done = effects.refund(&effect, &approved).await.unwrap();
    assert_eq!(done.outcome, Outcome::Executed);
    let again = effects.refund(&effect, &approved).await.unwrap();
    assert_eq!(
        again.refund_id, done.refund_id,
        "the same approval refunds once"
    );
    assert_eq!(mock.lock().unwrap().refunds.len(), 1);
    let payment = effects
        .payment(&kilele(), &started.payment_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (payment.state.as_str(), payment.refunded_minor),
        ("partly_refunded", 600)
    );
    assert_eq!(
        kinds(&receipts).last().map(String::as_str),
        Some("payment.refunded")
    );
}

#[tokio::test]
async fn plans_subscribe_in_usd_and_webhooks_must_be_signed() {
    let (base, mock) = paypal().await;
    let (effects, receipts) = gateway(Some(&base)).await;
    let plan = PlanPrice {
        plan_id: "business".into(),
        version: 1,
        name: "Agentrix Business".into(),
        monthly: Money {
            currency: "USD".into(),
            minor: 4900,
        },
        annual: None,
    };
    let start = SubscriptionStart {
        site: kilele(),
        plan: plan.clone(),
        subscriber_email: Some("wanjiru@example.com".into()),
        return_url: "https://agentrix.website/start/plan/done".into(),
        cancel_url: "https://agentrix.website/start/plan".into(),
        starts_at: Some(chrono::Utc::now() + chrono::Duration::days(3)),
    };
    let started = effects.start_subscription(start.clone()).await.unwrap();
    assert!(started.approve_url.contains(&started.provider_ref));
    let same = effects.start_subscription(start).await.unwrap();
    assert_eq!(
        same.subscription_id, started.subscription_id,
        "one pending subscription per site and plan"
    );
    assert_eq!(
        mock.lock().unwrap().plans,
        1,
        "the PayPal plan is made once"
    );
    let kes = PlanPrice {
        monthly: Money {
            currency: "KES".into(),
            minor: 650_000,
        },
        ..plan.clone()
    };
    assert!(effects.ensure_plan(&kes).await.is_err(), "plans are in USD");

    // 12 months up front: its own PayPal plan, billed once a year for the 12-month total.
    let annual = PlanPrice {
        annual: Some(Money {
            currency: "USD".into(),
            minor: 47_040,
        }),
        ..plan.clone()
    };
    let yearly = effects
        .start_subscription(SubscriptionStart {
            site: SiteId::new("annual-site"),
            plan: annual,
            subscriber_email: None,
            return_url: "https://agentrix.website/setup/pay/return".into(),
            cancel_url: "https://agentrix.website/setup/pay".into(),
            starts_at: None,
        })
        .await
        .unwrap();
    {
        let m = mock.lock().unwrap();
        assert_eq!(m.plans, 2, "a second PayPal plan for 12 months");
        let cycle = &m.plan_bodies[1]["billing_cycles"][0];
        assert_eq!(cycle["frequency"]["interval_unit"], "YEAR");
        assert_eq!(cycle["frequency"]["interval_count"], 1);
        assert_eq!(cycle["pricing_scheme"]["fixed_price"]["value"], "470.40");
        assert_eq!(
            cycle["pricing_scheme"]["fixed_price"]["currency_code"],
            "USD"
        );
    }
    let row = &effects
        .subscriptions_for_site(&SiteId::new("annual-site"))
        .await
        .unwrap()[0];
    assert_eq!(
        (
            row.plan_key.as_str(),
            row.amount_minor,
            row.provider_ref.as_deref()
        ),
        (
            "business@1:annual",
            47_040,
            Some(yearly.provider_ref.as_str())
        )
    );

    // A webhook PayPal didn't sign changes nothing.
    let event = json!({ "id": "WH-EVT-1", "event_type": "BILLING.SUBSCRIPTION.ACTIVATED", "resource": { "id": started.provider_ref } });
    let forged = paypal_headers("forged");
    assert!(
        effects
            .handle_webhook(&forged, &serde_json::to_vec(&event).unwrap())
            .await
            .is_err()
    );
    assert_eq!(
        effects.subscriptions_for_site(&kilele()).await.unwrap()[0].state,
        "pending_approval"
    );

    // Signed: applied once.
    mock.lock()
        .unwrap()
        .subscriptions
        .insert(started.provider_ref.clone(), "ACTIVE".into());
    let signed = paypal_headers("good");
    assert!(
        effects
            .handle_webhook(&signed, &serde_json::to_vec(&event).unwrap())
            .await
            .unwrap()
    );
    assert!(
        !effects.handle_webhook(&signed, &serde_json::to_vec(&event).unwrap()).await.unwrap(),
        "the same event twice applies once"
    );
    let subs = effects.subscriptions_for_site(&kilele()).await.unwrap();
    assert_eq!(subs[0].state, "active");
    assert!(subs[0].period_end.is_some());
    // Started, the annual one started, activated.
    assert_eq!(
        kinds(&receipts),
        [
            "subscription.changed",
            "subscription.changed",
            "subscription.changed"
        ]
    );

    // Switching the live plan to 12 months: PayPal's revise to the 12-month plan, agreed on
    // PayPal's page; once PayPal reports the new plan, the key and the amount follow.
    let annual_plan = PlanPrice {
        annual: Some(Money {
            currency: "USD".into(),
            minor: 47_040,
        }),
        ..plan.clone()
    };
    let approve = effects
        .revise_subscription(
            &kilele(),
            &started.subscription_id,
            &annual_plan,
            "https://agentrix.website/billing/period/return",
            "https://agentrix.website/billing",
        )
        .await
        .unwrap();
    assert!(approve.contains(&started.provider_ref));
    {
        let m = mock.lock().unwrap();
        assert_eq!(m.revisions.len(), 1);
        assert_eq!(m.revisions[0].0, started.provider_ref);
        assert_eq!(
            m.revisions[0].1["plan_id"], "P-2",
            "the 12-month plan made earlier"
        );
    }
    let before = effects
        .refresh_subscription(&started.provider_ref)
        .await
        .unwrap();
    assert_eq!(
        before.plan_key, "business@1",
        "nothing changes until the owner agrees"
    );
    mock.lock()
        .unwrap()
        .subscription_plans
        .insert(started.provider_ref.clone(), "P-2".into());
    let after = effects
        .refresh_subscription(&started.provider_ref)
        .await
        .unwrap();
    assert_eq!(
        (after.plan_key.as_str(), after.amount_minor),
        ("business@1:annual", 47_040)
    );

    // The first charge: recorded once for W3's Money page, however often PayPal retries.
    let sale = json!({
        "id": "WH-EVT-2", "event_type": "PAYMENT.SALE.COMPLETED",
        "resource": { "id": "SALE-9", "billing_agreement_id": started.provider_ref, "amount": { "total": "49.00", "currency": "USD" }, "create_time": "2026-11-11T09:00:00Z" },
    });
    assert!(effects.handle_webhook(&signed, &serde_json::to_vec(&sale).unwrap()).await.unwrap());
    let mut retry = sale.clone();
    retry["id"] = json!("WH-EVT-3");
    assert!(effects.handle_webhook(&signed, &serde_json::to_vec(&retry).unwrap()).await.unwrap());
    let payments: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT provider_ref, provider_payment_id, amount_minor, paid_at FROM plan_payments",
    )
    .fetch_all(effects.pool_for_tests())
    .await
    .unwrap();
    assert_eq!(
        payments,
        vec![(
            started.provider_ref.clone(),
            "SALE-9".to_owned(),
            4900,
            1_794_387_600
        )]
    );
}
