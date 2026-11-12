//! PayPal Payouts against a wiremock PayPal: paid once per approved fingerprint, a duplicate
//! `sender_batch_id` answered with the first batch, and a timeout settled by reading back.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentrix_effects::{EffectError, Effects, Outcome, PAYOUT_TOOL, PayoutRequest};
use agentrix_types::{EffectRequest, Money};
use common::{gateway, gateway_with, kinds, paypal, request_id, requests, site};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn usd(minor: i64) -> Money {
    Money {
        currency: "USD".into(),
        minor,
    }
}

fn ask(effects: &Effects, minor: i64) -> EffectRequest {
    effects
        .payout_request(PayoutRequest {
            site: site(),
            agent: "finance".into(),
            receiver: "partner@example.com".into(),
            amount: usd(minor),
            note: "October referral share".into(),
        })
        .unwrap()
}

fn approval(effect: &EffectRequest) -> Value {
    let fp = effect.fingerprint().unwrap().to_string();
    json!({ "method": "mandate", "account": "mandate_founder_1", "fingerprint": fp.trim_start_matches("sha256:") })
}

fn hex(effect: &EffectRequest) -> String {
    effect
        .fingerprint()
        .unwrap()
        .to_string()
        .trim_start_matches("sha256:")
        .to_owned()
}

/// PayPal's batch document, as `GET /v1/payments/payouts/{id}` returns it.
fn batch(id: &str, status: &str, item_status: &str, sender_item_id: &str) -> Value {
    json!({
        "batch_header": {
            "payout_batch_id": id, "batch_status": status,
            "sender_batch_header": { "sender_batch_id": "x", "email_subject": "You have a payment" },
        },
        "items": [{
            "payout_item_id": format!("ITEM-{id}"), "transaction_id": format!("TX-{id}"),
            "transaction_status": item_status, "payout_batch_id": id,
            "payout_item": {
                "recipient_type": "EMAIL", "receiver": "partner@example.com",
                "amount": { "value": "20.00", "currency": "USD" }, "sender_item_id": sender_item_id,
            },
        }],
    })
}

/// PayPal's Payouts endpoint: the first POST makes batch `BATCH-1` (answered after `delay`); any
/// later POST with the same `sender_batch_id` is refused, linking the first batch, as PayPal
/// does for 30 days.
struct Payouts {
    posts: AtomicUsize,
    delay: Duration,
}

impl Respond for Payouts {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = request.body_json().unwrap();
        if self.posts.fetch_add(1, Ordering::SeqCst) == 0 {
            return ResponseTemplate::new(201)
                .set_body_json(json!({
                    "batch_header": {
                        "payout_batch_id": "BATCH-1", "batch_status": "PENDING",
                        "sender_batch_header": body["sender_batch_header"],
                    },
                    "links": [{ "href": "https://api-m.sandbox.paypal.com/v1/payments/payouts/BATCH-1", "rel": "self", "method": "GET" }],
                }))
                .set_delay(self.delay);
        }
        ResponseTemplate::new(400).set_body_json(json!({
            "name": "USER_BUSINESS_ERROR", "message": "User business error.", "debug_id": "6a2b3c4d5e6f7",
            "information_link": "https://developer.paypal.com/docs/api/payments.payouts-batch/#errors",
            "details": [{
                "field": "SENDER_BATCH_ID", "location": "body",
                "issue": "Batch with given sender_batch_id already exists",
                "link": [{ "href": "https://api-m.sandbox.paypal.com/v1/payments/payouts/BATCH-1", "rel": "self", "method": "GET", "encType": "application/json" }],
            }],
            "links": [],
        }))
    }
}

async fn mount_payouts(server: &MockServer, delay: Duration) {
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts"))
        .respond_with(Payouts {
            posts: AtomicUsize::new(0),
            delay,
        })
        .mount(server)
        .await;
}

async fn mount_batch(server: &MockServer, status: &str, item_status: &str, sender_item_id: &str) {
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts/BATCH-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch(
            "BATCH-1",
            status,
            item_status,
            sender_item_id,
        )))
        .mount(server)
        .await;
}

#[tokio::test]
async fn an_approved_payout_is_paid_once() {
    let server = paypal().await;
    mount_payouts(&server, Duration::ZERO).await;
    let (effects, receipts) = gateway(&server).await;
    let effect = ask(&effects, 2000);
    assert_eq!(effect.tool, PAYOUT_TOOL);
    assert_eq!(effect.webagent, "finance");

    // Not approved, or approved for something else: nothing reaches PayPal.
    let forged = json!({ "account": "mandate_founder_1", "fingerprint": "00".repeat(32) });
    assert!(matches!(
        effects.payout(&effect, &forged).await,
        Err(EffectError::NotApproved)
    ));
    let other = ask(&effects, 2000);
    assert!(matches!(
        effects.payout(&effect, &approval(&other)).await,
        Err(EffectError::NotApproved)
    ));
    assert!(
        requests(&server, "POST", "/v1/payments/payouts")
            .await
            .is_empty()
    );

    let paid = effects.payout(&effect, &approval(&effect)).await.unwrap();
    assert_eq!(paid.outcome, Outcome::Executed);
    assert_eq!(paid.batch_id.as_deref(), Some("BATCH-1"));
    assert_eq!(paid.batch_status.as_deref(), Some("PENDING"));
    assert_eq!(paid.amount, usd(2000));
    assert!(paid.receipt.is_some());
    assert_eq!(
        paid.message,
        "Sent USD 20.00 to partner@example.com with PayPal. PayPal is still processing it."
    );

    // What PayPal was sent: the fingerprint as the sender_batch_id and the request id.
    let posts = requests(&server, "POST", "/v1/payments/payouts").await;
    assert_eq!(posts.len(), 1);
    assert_eq!(request_id(&posts[0]), Some(hex(&effect)));
    let body: Value = posts[0].body_json().unwrap();
    assert_eq!(body["sender_batch_header"]["sender_batch_id"], hex(&effect));
    let item = &body["items"][0];
    assert_eq!(item["recipient_type"], "EMAIL");
    assert_eq!(item["receiver"], "partner@example.com");
    assert_eq!(
        item["amount"],
        json!({ "value": "20.00", "currency": "USD" })
    );
    assert_eq!(item["note"], "October referral share");
    assert_eq!(item["sender_item_id"], paid.payout_id);

    // The same approved effect again: the first outcome, and no second POST.
    let again = effects.payout(&effect, &approval(&effect)).await.unwrap();
    assert_eq!(again, paid);
    assert_eq!(
        requests(&server, "POST", "/v1/payments/payouts")
            .await
            .len(),
        1
    );

    // PayPal finishes the batch: reading back follows it to SUCCESS, with GET only.
    mount_batch(&server, "SUCCESS", "SUCCESS", &paid.payout_id).await;
    let done = effects
        .reconcile_payout(&site(), &paid.payout_id)
        .await
        .unwrap();
    assert_eq!(done.outcome, Outcome::Executed);
    assert_eq!(done.item_id.as_deref(), Some("ITEM-BATCH-1"));
    assert_eq!(done.item_status.as_deref(), Some("SUCCESS"));
    assert_eq!(done.receipt, paid.receipt, "one receipt per payout");
    assert_eq!(
        done.message,
        "Paid USD 20.00 to partner@example.com with PayPal."
    );
    assert_eq!(
        requests(&server, "POST", "/v1/payments/payouts")
            .await
            .len(),
        1
    );

    // The batch and the item read on their own.
    let read = effects.payout_batch("BATCH-1").await.unwrap();
    assert_eq!((read.status.as_str(), read.items.len()), ("SUCCESS", 1));
    assert_eq!(
        read.items[0].sender_item_id.as_deref(),
        Some(paid.payout_id.as_str())
    );
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts-item/ITEM-BATCH-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            batch("BATCH-1", "SUCCESS", "SUCCESS", &paid.payout_id)["items"][0].clone(),
        ))
        .mount(&server)
        .await;
    let item = effects.payout_item("ITEM-BATCH-1").await.unwrap();
    assert_eq!(item.status, "SUCCESS");
    assert_eq!(item.transaction_id.as_deref(), Some("TX-BATCH-1"));
    assert_eq!(item.amount, Some(usd(2000)));

    // One payout.sent receipt, with the approver and the payee masked.
    assert_eq!(kinds(&receipts), ["payout.sent"]);
    let detail = &common::details(&receipts)[0];
    assert_eq!(detail["receiver"], "p***@example.com");
    assert_eq!(detail["approved_by"], "mandate_founder_1");
    assert_eq!(detail["batch"], "BATCH-1");
    let stored = effects.payouts_for_site(&site()).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        (stored[0].state.as_str(), stored[0].batch_status.as_deref()),
        ("executed", Some("SUCCESS"))
    );
}

#[tokio::test]
async fn a_duplicate_sender_batch_id_returns_the_original_batch() {
    // Two nodes (or one that lost its database) pay the same approved effect: PayPal makes one
    // batch, refuses the second request's sender_batch_id and links the first; both read it.
    let server = paypal().await;
    mount_payouts(&server, Duration::ZERO).await;
    let (first, _) = gateway(&server).await;
    let (second, second_receipts) = gateway(&server).await;
    let effect = ask(&first, 2000);

    let paid = first.payout(&effect, &approval(&effect)).await.unwrap();
    mount_batch(&server, "SUCCESS", "SUCCESS", &paid.payout_id).await;
    let replay = second.payout(&effect, &approval(&effect)).await.unwrap();
    assert_eq!(replay.outcome, Outcome::Executed);
    assert_eq!(
        replay.batch_id.as_deref(),
        Some("BATCH-1"),
        "the first batch"
    );
    assert_eq!(replay.batch_status.as_deref(), Some("SUCCESS"));
    assert_eq!(replay.item_status.as_deref(), Some("SUCCESS"));
    let posts = requests(&server, "POST", "/v1/payments/payouts").await;
    assert_eq!(posts.len(), 2);
    assert_eq!(request_id(&posts[0]), request_id(&posts[1]));
    assert_eq!(
        posts[1].body_json::<Value>().unwrap()["sender_batch_header"]["sender_batch_id"],
        hex(&effect)
    );
    assert_eq!(
        requests(&server, "GET", "/v1/payments/payouts/BATCH-1")
            .await
            .len(),
        1,
        "the second node read the first batch"
    );
    assert_eq!(kinds(&second_receipts), ["payout.sent"]);
}

#[tokio::test]
async fn a_timeout_is_unknown_until_reading_back_settles_it() {
    let server = paypal().await;
    // PayPal makes the batch but answers after our client has given up.
    mount_payouts(&server, Duration::from_secs(2)).await;
    let (effects, receipts) = gateway_with(&server, Some(Duration::from_millis(300))).await;
    let effect = ask(&effects, 2000);

    let lost = effects.payout(&effect, &approval(&effect)).await.unwrap();
    assert_eq!(lost.outcome, Outcome::Unknown);
    assert_eq!(lost.batch_id, None);
    assert_eq!(lost.receipt, None);
    assert_eq!(
        lost.message,
        "We couldn't confirm the payout with PayPal yet. It will never be paid twice; check again in a minute."
    );
    assert!(kinds(&receipts).is_empty());
    let stored = effects
        .stored_payout(&site(), &lost.payout_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, "unknown");

    // Reading back: PayPal knows the batch only by its sender_batch_id, so we ask with the
    // identical request; PayPal links the first batch and we read it with GET.
    mount_batch(&server, "SUCCESS", "SUCCESS", &lost.payout_id).await;
    let settled = effects
        .reconcile_payout(&site(), &lost.payout_id)
        .await
        .unwrap();
    assert_eq!(settled.outcome, Outcome::Executed);
    assert_eq!(settled.batch_id.as_deref(), Some("BATCH-1"));
    assert_eq!(settled.item_status.as_deref(), Some("SUCCESS"));
    assert!(settled.receipt.is_some());
    let posts = requests(&server, "POST", "/v1/payments/payouts").await;
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0].body, posts[1].body,
        "the identical request, never a new one"
    );
    assert_eq!(request_id(&posts[0]), request_id(&posts[1]));
    assert_eq!(kinds(&receipts), ["payout.sent"]);

    // From now on reading back is GET only, and the approved effect returns the same outcome.
    let again = effects.payout(&effect, &approval(&effect)).await.unwrap();
    assert_eq!(again.batch_id.as_deref(), Some("BATCH-1"));
    effects
        .reconcile_payout(&site(), &lost.payout_id)
        .await
        .unwrap();
    assert_eq!(
        requests(&server, "POST", "/v1/payments/payouts")
            .await
            .len(),
        2
    );
}

#[tokio::test]
async fn a_paypal_5xx_is_unknown_and_a_refusal_is_not_executed() {
    let server = paypal().await;
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "name": "INSUFFICIENT_FUNDS", "message": "Sender does not have sufficient funds.",
            "debug_id": "8f7e6d5c4b3a2",
        })))
        .with_priority(2)
        .mount(&server)
        .await;
    let (effects, receipts) = gateway(&server).await;

    let effect = ask(&effects, 2000);
    let unknown = effects.payout(&effect, &approval(&effect)).await.unwrap();
    assert_eq!(unknown.outcome, Outcome::Unknown);

    let refused_effect = ask(&effects, 900_000);
    let refused = effects
        .payout(&refused_effect, &approval(&refused_effect))
        .await
        .unwrap();
    assert_eq!(refused.outcome, Outcome::NotExecuted);
    assert_eq!(
        refused.message,
        "PayPal didn't pay USD 9000.00 to partner@example.com (INSUFFICIENT_FUNDS). Nothing left the account."
    );
    // Not executed is final for this approval: no second request.
    let again = effects
        .payout(&refused_effect, &approval(&refused_effect))
        .await
        .unwrap();
    assert_eq!(again.outcome, Outcome::NotExecuted);
    assert_eq!(
        requests(&server, "POST", "/v1/payments/payouts")
            .await
            .len(),
        2
    );
    assert!(kinds(&receipts).is_empty());
}

#[tokio::test]
async fn a_payout_item_webhook_reads_the_batch_back() {
    let server = paypal().await;
    common::mount_signature_check(&server).await;
    mount_payouts(&server, Duration::ZERO).await;
    let (effects, _) = gateway(&server).await;
    let effect = ask(&effects, 2000);
    let paid = effects.payout(&effect, &approval(&effect)).await.unwrap();
    mount_batch(&server, "SUCCESS", "UNCLAIMED", &paid.payout_id).await;

    let event = json!({
        "id": "WH-PO-1", "event_type": "PAYMENT.PAYOUTS-ITEM.UNCLAIMED",
        "resource": { "payout_item_id": "ITEM-BATCH-1", "payout_batch_id": "BATCH-1", "transaction_status": "UNCLAIMED" },
    });
    let headers = common::webhook_headers("good");
    assert!(
        effects
            .handle_webhook(&headers, &serde_json::to_vec(&event).unwrap())
            .await
            .unwrap()
    );
    assert!(
        !effects.handle_webhook(&headers, &serde_json::to_vec(&event).unwrap()).await.unwrap(),
        "once"
    );
    let now = effects
        .stored_payout(&site(), &paid.payout_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (now.state.as_str(), now.item_status.as_deref()),
        ("executed", Some("UNCLAIMED"))
    );
    let forged = json!({
        "id": "WH-PO-2", "event_type": "PAYMENT.PAYOUTSBATCH.DENIED",
        "resource": { "batch_header": { "payout_batch_id": "BATCH-1", "batch_status": "DENIED" } },
    });
    assert!(
        effects
            .handle_webhook(&common::webhook_headers("forged"), &serde_json::to_vec(&forged).unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn requests_are_checked_and_payouts_need_keys() {
    let server = paypal().await;
    let (effects, _) = gateway(&server).await;
    let bad = |receiver: &str, minor: i64, note: &str| {
        effects
            .payout_request(PayoutRequest {
                site: site(),
                agent: "finance".into(),
                receiver: receiver.into(),
                amount: usd(minor),
                note: note.into(),
            })
            .is_err()
    };
    assert!(bad("not-an-email", 2000, "x"));
    assert!(bad("partner@example.com", 0, "x"));
    assert!(bad("partner@example.com", 2000, "  "));
    assert!(!bad(" partner@example.com ", 2000, "x"), "trimmed");

    let mut expired = ask(&effects, 2000);
    expired.expiry = chrono::Utc::now() - chrono::Duration::minutes(1);
    assert!(matches!(
        effects.payout(&expired, &approval(&expired)).await,
        Err(EffectError::NotApproved)
    ));

    let off = common::gateway_off().await;
    let effect = ask(&off, 2000);
    assert!(matches!(
        off.payout(&effect, &approval(&effect)).await,
        Err(EffectError::NotConfigured)
    ));
}
