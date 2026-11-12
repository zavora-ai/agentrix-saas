//! PayPal Subscriptions against a wiremock PayPal: plans made once, then a subscription's life:
//! start, revise, suspend, activate and cancel.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agentrix_effects::{EffectError, PlanPrice, SubscriptionStart};
use agentrix_types::Money;
use common::{gateway, kinds, paypal, request_id, requests, site};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn usd(minor: i64) -> Money {
    Money {
        currency: "USD".into(),
        minor,
    }
}

fn starter() -> PlanPrice {
    PlanPrice {
        plan_id: "saas-starter".into(),
        version: 1,
        name: "Agentrix SaaS Starter".into(),
        monthly: usd(1900),
        annual: None,
    }
}

/// Answers each create with the next id: `PROD-1`, `PROD-2`… or `P-1`, `P-2`….
struct Counter(&'static str, AtomicUsize);

impl Respond for Counter {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.1.fetch_add(1, Ordering::SeqCst) + 1;
        ResponseTemplate::new(201).set_body_json(json!({ "id": format!("{}-{n}", self.0) }))
    }
}

async fn mount_catalogue(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/catalogs/products"))
        .respond_with(Counter("PROD", AtomicUsize::new(0)))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/plans"))
        .respond_with(Counter("P", AtomicUsize::new(0)))
        .mount(server)
        .await;
}

/// PayPal's side of subscription `I-1`: its status, which a test moves on as the owner acts on
/// PayPal's pages.
#[derive(Clone)]
struct Status(Arc<Mutex<&'static str>>);

impl Status {
    fn set(&self, status: &'static str) {
        *self.0.lock().unwrap() = status;
    }
}

impl Respond for Status {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "id": "I-1", "status": *self.0.lock().unwrap(), "plan_id": "P-1",
            "billing_info": { "next_billing_time": "2026-12-10T10:00:00Z" },
        }))
    }
}

/// The catalogue, subscription `I-1` reporting `status`, and its revise, suspend, activate and
/// cancel calls.
async fn mount_subscription(server: &MockServer, status: &'static str) -> Status {
    mount_catalogue(server).await;
    let state = Status(Arc::new(Mutex::new(status)));
    Mock::given(method("GET"))
        .and(path("/v1/billing/subscriptions/I-1"))
        .respond_with(state.clone())
        .mount(server)
        .await;
    for action in ["revise", "suspend", "activate", "cancel"] {
        let answer = if action == "revise" {
            ResponseTemplate::new(200).set_body_json(json!({
                "plan_id": "P-2",
                "links": [{ "rel": "approve", "href": "https://www.sandbox.paypal.com/webapps/billing/subscriptions/update?ba_token=BA-2" }],
            }))
        } else {
            ResponseTemplate::new(204)
        };
        Mock::given(method("POST"))
            .and(path(format!("/v1/billing/subscriptions/I-1/{action}")))
            .respond_with(answer)
            .with_priority(3)
            .mount(server)
            .await;
    }
    state
}

#[tokio::test]
async fn a_plan_is_made_once_and_its_price_changes_only_by_version() {
    let server = paypal().await;
    mount_catalogue(&server).await;
    let (effects, _) = gateway(&server).await;

    let first = effects.ensure_plan(&starter()).await.unwrap();
    let again = effects.ensure_plan(&starter()).await.unwrap();
    assert_eq!((first.as_str(), again.as_str()), ("P-1", "P-1"));
    let products = requests(&server, "POST", "/v1/catalogs/products").await;
    let plans = requests(&server, "POST", "/v1/billing/plans").await;
    assert_eq!((products.len(), plans.len()), (1, 1), "made once");

    // Keyed by plan and version, so another node (or a lost database) asking again gets the
    // same product and plan back from PayPal, not a second one.
    assert_eq!(
        request_id(&products[0]).as_deref(),
        Some("product-saas-starter@1")
    );
    assert_eq!(
        request_id(&plans[0]).as_deref(),
        Some("plan-saas-starter@1")
    );
    let product: Value = products[0].body_json().unwrap();
    assert_eq!(product["name"], "Agentrix SaaS Starter");
    assert_eq!(product["type"], "SERVICE");
    let plan: Value = plans[0].body_json().unwrap();
    assert_eq!(plan["product_id"], "PROD-1");
    assert_eq!(plan["status"], "ACTIVE");
    let cycle = &plan["billing_cycles"][0];
    assert_eq!(cycle["frequency"]["interval_unit"], "MONTH");
    assert_eq!(
        cycle["pricing_scheme"]["fixed_price"],
        json!({ "currency_code": "USD", "value": "19.00" })
    );

    let (elsewhere, _) = gateway(&server).await;
    elsewhere.ensure_plan(&starter()).await.unwrap();
    let plans = requests(&server, "POST", "/v1/billing/plans").await;
    assert_eq!(
        request_id(&plans[1]),
        request_id(&plans[0]),
        "the same request id from a fresh database"
    );

    let dearer = PlanPrice {
        monthly: usd(2900),
        ..starter()
    };
    assert!(matches!(
        effects.ensure_plan(&dearer).await,
        Err(EffectError::BadRequest(_))
    ));
    let next = PlanPrice {
        version: 2,
        ..dearer
    };
    assert_eq!(effects.ensure_plan(&next).await.unwrap(), "P-3");
    let free = PlanPrice {
        plan_id: "free".into(),
        monthly: usd(0),
        ..starter()
    };
    assert!(effects.ensure_plan(&free).await.is_err(), "above zero");
}

#[tokio::test]
async fn a_subscription_starts_once_then_revises_suspends_activates_and_cancels() {
    let server = paypal().await;
    let paypal_side = mount_subscription(&server, "APPROVAL_PENDING").await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/subscriptions"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "I-1", "status": "APPROVAL_PENDING",
            "links": [{ "rel": "approve", "href": "https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token=BA-1" }],
        })))
        .expect(1)
        .named("one subscription")
        .mount(&server)
        .await;
    let (effects, receipts) = gateway(&server).await;
    let start = SubscriptionStart {
        site: site(),
        plan: starter(),
        subscriber_email: Some("founder@example.com".into()),
        return_url: "https://saas.agentrix.website/billing/done".into(),
        cancel_url: "https://saas.agentrix.website/billing".into(),
        starts_at: Some(chrono::Utc::now() + chrono::Duration::days(3)),
    };

    // Start: one PayPal subscription however often the owner taps.
    let started = effects.start_subscription(start.clone()).await.unwrap();
    let same = effects.start_subscription(start).await.unwrap();
    assert_eq!(started, same);
    assert_eq!(started.provider_ref, "I-1");
    assert!(started.approve_url.contains("ba_token=BA-1"));
    let created = requests(&server, "POST", "/v1/billing/subscriptions").await;
    assert_eq!(
        request_id(&created[0]).as_deref(),
        Some(started.fingerprint.trim_start_matches("sha256:"))
    );
    let body: Value = created[0].body_json().unwrap();
    assert_eq!(body["plan_id"], "P-1");
    assert_eq!(body["custom_id"], started.subscription_id);
    assert_eq!(body["subscriber"]["email_address"], "founder@example.com");
    assert!(body["start_time"].as_str().unwrap().ends_with('Z'));
    let id = started.subscription_id.clone();

    // Nothing can be paused or switched before the owner approves.
    assert!(matches!(
        effects.suspend_subscription(&site(), &id, "late").await,
        Err(EffectError::BadRequest(_))
    ));

    // The owner approved on PayPal's page.
    paypal_side.set("ACTIVE");
    let live = effects.refresh_subscription("I-1").await.unwrap();
    assert_eq!(live.state, "active");

    // Revise: the owner agrees to the new plan on PayPal's page.
    let annual = PlanPrice {
        annual: Some(usd(18_240)),
        ..starter()
    };
    let approve = effects
        .revise_subscription(
            &site(),
            &id,
            &annual,
            "https://saas.agentrix.website/billing/period/done",
            "https://saas.agentrix.website/billing",
        )
        .await
        .unwrap();
    assert!(approve.contains("ba_token=BA-2"));
    let revised = requests(&server, "POST", "/v1/billing/subscriptions/I-1/revise").await;
    assert_eq!(revised.len(), 1);
    let revise: Value = revised[0].body_json().unwrap();
    assert_eq!(
        revise["plan_id"], "P-2",
        "the 12-month plan, made for the switch"
    );
    assert_eq!(
        revise["application_context"]["return_url"],
        "https://saas.agentrix.website/billing/period/done"
    );

    // Suspend after a failed payment: once.
    let paused = effects
        .suspend_subscription(&site(), &id, "The October payment failed")
        .await
        .unwrap();
    assert_eq!(paused.state, "suspended");
    let again = effects
        .suspend_subscription(&site(), &id, "The October payment failed")
        .await
        .unwrap();
    assert_eq!(again.state, "suspended");
    let suspends = requests(&server, "POST", "/v1/billing/subscriptions/I-1/suspend").await;
    assert_eq!(suspends.len(), 1, "suspended at PayPal once");
    let reason: Value = suspends[0].body_json().unwrap();
    assert_eq!(reason, json!({ "reason": "The October payment failed" }));

    // Activate once paid: once.
    let back = effects
        .activate_subscription(&site(), &id, "Paid in full")
        .await
        .unwrap();
    assert_eq!(back.state, "active");
    effects
        .activate_subscription(&site(), &id, "Paid in full")
        .await
        .unwrap();
    let activations = requests(&server, "POST", "/v1/billing/subscriptions/I-1/activate").await;
    assert_eq!(activations.len(), 1, "activated at PayPal once");
    assert_eq!(
        activations[0].body_json::<Value>().unwrap(),
        json!({ "reason": "Paid in full" })
    );

    // Cancel: once, and then nothing else moves it.
    let gone = effects
        .cancel_subscription(&site(), &id, "The founder closed the company")
        .await
        .unwrap();
    assert_eq!(gone.state, "cancelled");
    effects
        .cancel_subscription(&site(), &id, "again")
        .await
        .unwrap();
    assert_eq!(
        requests(&server, "POST", "/v1/billing/subscriptions/I-1/cancel")
            .await
            .len(),
        1
    );
    assert!(matches!(
        effects.activate_subscription(&site(), &id, "undo").await,
        Err(EffectError::BadRequest(_))
    ));
    assert!(matches!(
        effects.suspend_subscription(&site(), "sub_nope", "x").await,
        Err(EffectError::NotFound)
    ));

    // Started, activated (read back), revise asked, suspended, activated, cancelled.
    assert_eq!(kinds(&receipts), ["subscription.changed"; 6]);
    let states: Vec<String> = common::details(&receipts)
        .iter()
        .map(|d| d["state"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        states,
        [
            "pending_approval",
            "active",
            "waiting_for_owner",
            "suspended",
            "active",
            "cancelled"
        ]
    );
}

#[tokio::test]
async fn a_suspend_paypal_refuses_or_never_answers_changes_nothing_here() {
    let server = paypal().await;
    mount_subscription(&server, "ACTIVE").await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/subscriptions"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "I-1", "status": "APPROVAL_PENDING",
            "links": [{ "rel": "approve", "href": "https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token=BA-1" }],
        })))
        .mount(&server)
        .await;
    let (effects, _) = gateway(&server).await;
    let started = effects
        .start_subscription(SubscriptionStart {
            site: site(),
            plan: starter(),
            subscriber_email: None,
            return_url: "https://saas.agentrix.website/billing/done".into(),
            cancel_url: "https://saas.agentrix.website/billing".into(),
            starts_at: None,
        })
        .await
        .unwrap();
    effects.refresh_subscription("I-1").await.unwrap();

    Mock::given(method("POST"))
        .and(path("/v1/billing/subscriptions/I-1/suspend"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "name": "UNPROCESSABLE_ENTITY", "message": "The requested action could not be performed.",
            "details": [{ "issue": "SUBSCRIPTION_STATUS_INVALID" }],
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/billing/subscriptions/I-1/suspend"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(2)
        .mount(&server)
        .await;
    let refused = effects
        .suspend_subscription(&site(), &started.subscription_id, "late")
        .await
        .unwrap_err();
    assert!(
        matches!(refused, EffectError::NotExecuted(_)),
        "{refused:?}"
    );
    let unknown = effects
        .suspend_subscription(&site(), &started.subscription_id, "late")
        .await
        .unwrap_err();
    assert!(matches!(unknown, EffectError::Unknown(_)), "{unknown:?}");
    assert_eq!(
        effects.subscriptions_for_site(&site()).await.unwrap()[0].state,
        "active",
        "still active until PayPal says otherwise"
    );
}
