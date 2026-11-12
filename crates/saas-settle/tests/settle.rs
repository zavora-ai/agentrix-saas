//! Settlement between two AI-run companies, end to end over their agent doors, with PayPal
//! mocked by wiremock and both companies' books in saas-core's `FakeBooks`.

mod common;

use std::sync::Arc;

use adk_auth::web_bot_auth::{ComponentId, Ed25519SigningKey, HttpSignatureSigner, KeyDirectory};
use async_trait::async_trait;
use chrono::{Duration, NaiveDate};
use common::*;
use saas_settle::door::{CONTENT_DIGEST, content_digest};
use saas_settle::keys::{PAYABLES, RECEIVABLES};
use saas_settle::mandate::InstalmentBounds;
use saas_settle::protocol::Instalment;
use saas_settle::{
    DoorAnswer, InstalmentPlan, Invoice, Negotiator, PayError, Route, SealKind, SettleMessage,
    SettlementReceipt,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn a2a_body(message: &SettleMessage) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "1",
        "method": "SendMessage",
        "params": {"message": {
            "messageId": uuid::Uuid::now_v7().to_string(),
            "role": "ROLE_USER",
            "parts": [{"data": serde_json::to_value(message).unwrap()}],
        }},
    }))
    .unwrap()
}

/// Posts `body` to Agentrix's door, signed (or not) as `signer` would.
async fn post(
    world: &World,
    body: Vec<u8>,
    signer: Option<&HttpSignatureSigner>,
    digest_of: Option<&[u8]>,
) -> (u16, Value) {
    let http = reqwest::Client::new();
    let mut request = http
        .post(world.agentrix_door.join("a2a").unwrap())
        .header("content-type", "application/json")
        .header(CONTENT_DIGEST, content_digest(digest_of.unwrap_or(&body)))
        .body(body)
        .build()
        .unwrap();
    if let Some(signer) = signer {
        signer.sign_reqwest(&mut request).unwrap();
    }
    let response = http.execute(request).await.unwrap();
    (
        response.status().as_u16(),
        response.json().await.unwrap_or(Value::Null),
    )
}

fn signer(key: Arc<Ed25519SigningKey>, origin: &str) -> HttpSignatureSigner {
    HttpSignatureSigner::builder(key, origin)
        .components(vec![
            ComponentId::new("@authority"),
            ComponentId::new("@method"),
            ComponentId::new("@path"),
            ComponentId::new("@query"),
            ComponentId::new(CONTENT_DIGEST),
        ])
        .build()
        .unwrap()
}

async fn offer(world: &World, invoice_id: &str) -> DoorAnswer {
    world
        .payee
        .send_offer(invoice_id)
        .await
        .unwrap()
        .answer
        .expect("Kilele's door answers the offer")
}

#[tokio::test]
async fn the_happy_path_posts_to_both_books_from_one_paypal_id() {
    let world = world(Options::default()).await;
    let invoice = world.invoice_id.clone();

    let DoorAnswer::InvoiceRecorded { decision, .. } = offer(&world, &invoice).await else {
        panic!("the offer is recorded");
    };
    assert_eq!(decision, "will_settle");
    assert_eq!(balances(&world, &invoice).await, (120_000, 120_000));

    let settled = world.payer.settle(AGENTRIX, &invoice).await.unwrap();
    let receipt: SettlementReceipt = settled.receipt.read(SealKind::SettlementReceipt).unwrap();
    assert_eq!(receipt.route, Route::Vault);
    assert_eq!(receipt.amount, usd(120_000));
    assert_eq!(receipt.paypal.key(), "paypal:capture:CAP-1");
    assert_eq!(receipt.paypal.parent_id.as_deref(), Some("ORDER-1"));
    assert_eq!(receipt.invoice_open_after, usd(0));
    assert_eq!(settled.posted.paypal_key, "paypal:capture:CAP-1");

    // One PayPal capture; both books cleared, each by one payment citing that capture.
    assert_eq!(
        world.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(balances(&world, &invoice).await, (0, 0));
    assert_eq!(
        payments(&world.agentrix_books, "paypal:capture:CAP-1").await,
        1
    );
    assert_eq!(
        payments(&world.kilele_books, "paypal:capture:CAP-1").await,
        1
    );

    // The capture asked for exactly the invoice, on the saved wallet, unique per invoice.
    let requests = world.paypal.received_requests().await.unwrap();
    let order = requests
        .iter()
        .find(|r| r.url.path() == "/v2/checkout/orders")
        .unwrap();
    let body: Value = order.body_json().unwrap();
    assert_eq!(body["purchase_units"][0]["amount"]["value"], "1200.00");
    assert_eq!(
        body["purchase_units"][0]["invoice_id"],
        format!("agentrix-saas/{invoice}/{}", receipt.cart_mandate_id)
    );

    // The receipt is sealed by Agentrix's Receivables agent and checks against its directory.
    let key = settled
        .receipt
        .verify(SealKind::SettlementReceipt, world.directories.as_ref())
        .await
        .unwrap();
    assert_eq!(key.kid.as_deref(), Some("agent:receivables"));

    // Kilele's door kept the A2A task: the payer can read it back.
    let client = world.kilele_client(world.kilele.agent(PAYABLES).unwrap());
    let task = client.task(&settled.task_id).await.unwrap();
    assert!(matches!(task.answer, Some(DoorAnswer::Settled { .. })));
}

#[tokio::test]
async fn unsigned_and_wrong_key_requests_are_refused_before_any_paypal_call() {
    let world = world(Options::default()).await;
    let invoice = world.invoice_id.clone();
    let due = world.today() + Duration::days(14);
    let message = SettleMessage::Settle {
        intent: world.intent.clone(),
        cart: cart(&world, &invoice, 120_000, due),
        payout: None,
    };
    let body = a2a_body(&message);

    // No signature.
    let (status, answer) = post(&world, body.clone(), None, None).await;
    assert_eq!(status, 401);
    assert_eq!(answer["error"]["data"]["code"], "signature_required");

    // Signed with a key Kilele never published, naming Kilele's directory.
    let stranger = Arc::new(Ed25519SigningKey::generate().unwrap());
    let (status, answer) = post(
        &world,
        body.clone(),
        Some(&signer(stranger.clone(), KILELE)),
        None,
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(answer["error"]["data"]["code"], "signature_invalid");

    // A company that publishes its key, but isn't a counterparty.
    world
        .directories
        .insert(
            "https://stranger.example",
            KeyDirectory::from_keys([stranger.as_ref()]),
            None,
        )
        .await;
    let (status, answer) = post(
        &world,
        body.clone(),
        Some(&signer(stranger, "https://stranger.example")),
        None,
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(answer["error"]["data"]["code"], "agent_not_trusted");

    // Kilele's agent signed, but the body was changed after signing.
    let payables = signer(world.kilele.agent(PAYABLES).unwrap(), KILELE);
    let mut edited = body.clone();
    let at = edited.len() - 3;
    edited[at] = b' ';
    let (status, answer) = post(&world, edited, Some(&payables), Some(&body)).await;
    assert_eq!(status, 401);
    assert_eq!(answer["error"]["data"]["code"], "content_digest_mismatch");

    // A published Kilele key, but not the Payables agent the owner named: the door lets the
    // request in, the mandate check refuses it.
    let client = world.kilele_client(world.kilele.owner.clone());
    let reply = client.send(&message).await.unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Rejected);
    assert!(reply.refusal.unwrap().contains("not by the Payables agent"));

    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 0);
    assert_eq!(balances(&world, &invoice).await.0, 120_000);
}

#[tokio::test]
async fn a_replayed_mandate_is_refused() {
    let world = world(Options::default()).await;
    let invoice = world.invoice_id.clone();
    let due = world.today() + Duration::days(14);
    let sealed_cart = cart(&world, &invoice, 120_000, due);
    let message = SettleMessage::Settle {
        intent: world.intent.clone(),
        cart: sealed_cart.clone(),
        payout: None,
    };
    let client = world.kilele_client(world.kilele.agent(PAYABLES).unwrap());
    let first = client.send(&message).await.unwrap();
    assert!(matches!(first.answer, Some(DoorAnswer::Settled { .. })));

    // The same cart mandate again, in a fresh signed request.
    let again = client.send(&message).await.unwrap();
    assert_eq!(again.state, a2a_protocol_types::TaskState::Rejected);
    assert!(again.refusal.unwrap().contains("already used"));

    // The same signed request again, byte for byte: refused at the door.
    let body = a2a_body(&message);
    let payables = signer(world.kilele.agent(PAYABLES).unwrap(), KILELE);
    let http = reqwest::Client::new();
    let mut request = http
        .post(world.agentrix_door.join("a2a").unwrap())
        .header(CONTENT_DIGEST, content_digest(&body))
        .body(body)
        .build()
        .unwrap();
    payables.sign_reqwest(&mut request).unwrap();
    let replay = request.try_clone().unwrap();
    let _ = http.execute(request).await.unwrap();
    let response = http.execute(replay).await.unwrap();
    assert_eq!(response.status().as_u16(), 401);
    let answer: Value = response.json().await.unwrap();
    assert_eq!(answer["error"]["data"]["code"], "replayed_request");

    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 1);
}

#[tokio::test]
async fn an_invoice_over_the_mandate_is_refused() {
    let world = world(Options {
        invoice: 200_000,
        ..Options::default()
    })
    .await;
    let invoice = world.invoice_id.clone();

    // Kilele's Payables agent records the bill but holds it for the owner.
    let DoorAnswer::InvoiceRecorded { decision, note, .. } = offer(&world, &invoice).await else {
        panic!("recorded");
    };
    assert_eq!(decision, "held");
    assert!(note.contains("per-invoice cap"), "{note}");

    // It won't sign a cart mandate for it…
    let refused = world.payer.settle(AGENTRIX, &invoice).await.unwrap_err();
    assert!(matches!(refused, PayError::Mandate(_)), "{refused}");

    // …and if a cart mandate for it arrives anyway, Agentrix refuses before PayPal.
    let due = world.today() + Duration::days(14);
    let client = world.kilele_client(world.kilele.agent(PAYABLES).unwrap());
    let reply = client
        .send(&SettleMessage::Settle {
            intent: world.intent.clone(),
            cart: cart(&world, &invoice, 200_000, due),
            payout: None,
        })
        .await
        .unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Rejected);
    assert!(reply.refusal.unwrap().contains("per-invoice cap"));

    // A cart mandate for less than the invoice doesn't get round it.
    let reply = client
        .send(&SettleMessage::Settle {
            intent: world.intent.clone(),
            cart: cart(&world, &invoice, 100_000, due),
            payout: None,
        })
        .await
        .unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Rejected);

    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 0);
    assert_eq!(balances(&world, &invoice).await, (200_000, 200_000));
}

#[tokio::test]
async fn the_monthly_cap_holds_across_invoices() {
    let world = world(Options::default()).await;
    let first = world.invoice_id.clone();
    offer(&world, &first).await;
    world.payer.settle(AGENTRIX, &first).await.unwrap();
    // $1,200 paid; another $1,000 would pass $2,000 for the month.
    let second = world.invoice(100_000, "Commerce add-on, annual").await;
    offer(&world, &second).await;
    let due = world.today() + Duration::days(14);
    let reply = world
        .kilele_client(world.kilele.agent(PAYABLES).unwrap())
        .send(&SettleMessage::Settle {
            intent: world.intent.clone(),
            cart: cart(&world, &second, 100_000, due),
            payout: None,
        })
        .await
        .unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Rejected);
    assert!(reply.refusal.unwrap().contains("monthly cap"));
    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 1);
}

/// Refuses to counter.
struct NoCounter;

#[async_trait]
impl Negotiator for NoCounter {
    async fn counter(
        &self,
        _: &InstalmentPlan,
        _: &Invoice,
        _: &InstalmentBounds,
        _: &InstalmentBounds,
        _: NaiveDate,
    ) -> Option<(InstalmentPlan, String)> {
        None
    }
}

/// A negotiating model that drafts a counter outside the owners' bounds.
struct Overreaching;

#[async_trait]
impl Negotiator for Overreaching {
    async fn counter(
        &self,
        proposal: &InstalmentPlan,
        _: &Invoice,
        _: &InstalmentBounds,
        _: &InstalmentBounds,
        _: NaiveDate,
    ) -> Option<(InstalmentPlan, String)> {
        Some((
            InstalmentPlan {
                plan_id: "plan_overreach".into(),
                ..proposal.clone()
            },
            "Sure, take as long as you like.".into(),
        ))
    }
}

fn instalments(today: NaiveDate, parts: &[(i64, i64)]) -> Vec<Instalment> {
    parts
        .iter()
        .enumerate()
        .map(|(i, (days, minor))| Instalment {
            index: i as u32 + 1,
            due_on: today + Duration::days(*days),
            amount: usd(*minor),
        })
        .collect()
}

#[tokio::test]
async fn an_instalment_plan_outside_the_bounds_is_refused() {
    for negotiator in [
        Arc::new(NoCounter) as Arc<dyn Negotiator>,
        Arc::new(Overreaching),
    ] {
        let world = world(Options {
            negotiator,
            cash_available: Some(usd(30_000)),
            ..Options::default()
        })
        .await;
        let invoice = world.invoice_id.clone();
        let DoorAnswer::InvoiceRecorded { decision, .. } = offer(&world, &invoice).await else {
            panic!("recorded");
        };
        assert_eq!(decision, "instalments");
        // Six monthly instalments: over both owners' counts and spans.
        let today = world.today();
        let six = instalments(
            today,
            &[
                (14, 20_000),
                (44, 20_000),
                (74, 20_000),
                (104, 20_000),
                (134, 20_000),
                (164, 20_000),
            ],
        );
        let refused = world
            .payer
            .propose_instalments(AGENTRIX, &invoice, six, "Short this quarter")
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("outside"),
            "the refusal says which mandate: {refused}"
        );
    }

    // A plan the payee never offered, sealed by the payer itself, isn't agreed either.
    let world = world(Options::default()).await;
    let invoice = world.invoice_id.clone();
    offer(&world, &invoice).await;
    let today = world.today();
    let plan = InstalmentPlan {
        plan_id: "plan_self_made".into(),
        invoice_id: invoice.clone(),
        payer: KILELE.into(),
        payee: AGENTRIX.into(),
        total: usd(120_000),
        instalments: instalments(today, &[(14, 60_000), (44, 60_000)]),
        note: "ours".into(),
    };
    let self_sealed = saas_settle::Sealed::seal(
        SealKind::InstalmentPlan,
        &plan,
        KILELE,
        &world.kilele.agent(PAYABLES).unwrap(),
    )
    .unwrap();
    let refused = world
        .payer
        .accept_plan(AGENTRIX, &self_sealed)
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("wasn't offered by us"),
        "{refused}"
    );
    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 0);
}

#[tokio::test]
async fn an_agreed_plan_is_countered_inside_both_mandates_and_collected_on_due_dates() {
    let world = world(Options {
        cash_available: Some(usd(30_000)),
        ..Options::default()
    })
    .await;
    let invoice = world.invoice_id.clone();
    offer(&world, &invoice).await;
    let today = world.today();
    // Four instalments over 80 days: inside Kilele's owner's bounds, outside Agentrix's (3, 60).
    let four = instalments(
        today,
        &[(14, 30_000), (34, 30_000), (64, 30_000), (94, 30_000)],
    );
    let (plan, sealed, countered, reason) = world
        .payer
        .propose_instalments(
            AGENTRIX,
            &invoice,
            four,
            "Short until the holiday orders pay",
        )
        .await
        .unwrap();
    assert!(countered, "{reason}");
    assert_eq!(plan.instalments.len(), 3);
    assert!(
        plan.instalments[0].amount.minor >= 30_000,
        "the first is at least 25%"
    );
    assert_eq!(
        plan.instalments.last().unwrap().due_on,
        today + Duration::days(14 + 60)
    );

    let (agreement, _) = world.payer.accept_plan(AGENTRIX, &sealed).await.unwrap();
    assert_eq!(agreement.cart_digests.len(), 3);

    // Nothing is due yet.
    assert!(world.payee.collect_due().await.unwrap().is_empty());
    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 0);

    // On the first due date, Receivables collects instalment 1 with Kilele's pre-signed cart
    // mandate; Kilele's door takes the receipt and its books post the same PayPal id.
    world.advance(14);
    let collected = world.payee.collect_due().await.unwrap();
    assert_eq!(collected.len(), 1);
    let receipt: SettlementReceipt = collected[0]
        .2
        .as_ref()
        .unwrap()
        .read(SealKind::SettlementReceipt)
        .unwrap();
    let first = plan.instalments[0].amount.minor;
    assert_eq!(receipt.amount.minor, first);
    assert_eq!(receipt.instalment.as_ref().unwrap().index, 1);
    let key = receipt.paypal.key();
    assert_eq!(payments(&world.agentrix_books, &key).await, 1);
    assert_eq!(payments(&world.kilele_books, &key).await, 1);
    assert_eq!(
        balances(&world, &invoice).await,
        (120_000 - first, 120_000 - first)
    );

    // Collecting again the same day takes nothing more.
    assert!(world.payee.collect_due().await.unwrap().is_empty());

    // The rest on their dates.
    for (step, expected_open) in [(30, plan.instalments[2].amount.minor), (30, 0)] {
        world.advance(step);
        let collected = world.payee.collect_due().await.unwrap();
        assert_eq!(collected.len(), 1);
        assert!(collected[0].2.is_ok(), "{:?}", collected[0].2);
        assert_eq!(
            balances(&world, &invoice).await,
            (expected_open, expected_open)
        );
    }
    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 3);
}

#[tokio::test]
async fn the_payout_fallback_settles_and_posts_both_books_from_the_payout_item() {
    let world = world(Options {
        route: Route::Payout,
        ..Options::default()
    })
    .await;
    let item = json!({
        "payout_item_id": "ITEM-1", "transaction_id": "TX-1",
        "transaction_status": "SUCCESS", "payout_batch_id": "BATCH-1",
        "payout_item": {
            "recipient_type": "EMAIL", "receiver": AGENTRIX_PAYPAL,
            "amount": { "value": "1200.00", "currency": "USD" }, "sender_item_id": "po_1",
        },
    });
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "batch_header": {"payout_batch_id": "BATCH-1", "batch_status": "PENDING"},
        })))
        .mount(&world.paypal)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts/BATCH-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batch_header": {"payout_batch_id": "BATCH-1", "batch_status": "SUCCESS"},
            "items": [item.clone()],
        })))
        .mount(&world.paypal)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts-item/ITEM-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(item))
        .mount(&world.paypal)
        .await;

    let invoice = world.invoice_id.clone();
    offer(&world, &invoice).await;
    let settled = world.payer.settle(AGENTRIX, &invoice).await.unwrap();
    assert_eq!(settled.terms.route, Route::Payout);
    assert_eq!(settled.terms.paypal.key(), "paypal:payout:ITEM-1");
    assert_eq!(settled.terms.paypal.transaction_id.as_deref(), Some("TX-1"));
    assert_eq!(balances(&world, &invoice).await, (0, 0));
    assert_eq!(
        payments(&world.agentrix_books, "paypal:payout:ITEM-1").await,
        1
    );
    assert_eq!(
        payments(&world.kilele_books, "paypal:payout:ITEM-1").await,
        1
    );

    // The payout carried the invoice id to Agentrix's PayPal email, once.
    let requests = world.paypal.received_requests().await.unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|r| r.url.path() == "/v1/payments/payouts" && r.method.as_str() == "POST")
        .collect();
    assert_eq!(posts.len(), 1);
    let body: Value = posts[0].body_json().unwrap();
    assert_eq!(body["items"][0]["receiver"], AGENTRIX_PAYPAL);
    assert!(
        body["items"][0]["note"]
            .as_str()
            .unwrap()
            .contains(&invoice)
    );
    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 0);

    // Asked again, the Payables agent refuses before any payout: the money would leave first.
    let again = world.payer.settle(AGENTRIX, &invoice).await.unwrap_err();
    assert!(again.to_string().contains("already settled"), "{again}");
    let posts = world
        .paypal
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/payments/payouts" && r.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 1);
}

#[tokio::test]
async fn a_payout_to_someone_else_is_not_a_settlement() {
    let world = world(Options::default()).await;
    let invoice = world.invoice_id.clone();
    let due = world.today() + Duration::days(14);
    let reply = world
        .kilele_client(world.kilele.agent(PAYABLES).unwrap())
        .send(&SettleMessage::Settle {
            intent: world.intent.clone(),
            cart: cart(&world, &invoice, 120_000, due),
            payout: Some(saas_settle::protocol::PayoutEvidence {
                batch_id: "BATCH-X".into(),
                item_id: "ITEM-X".into(),
                transaction_id: None,
                status: "SUCCESS".into(),
                amount: usd(120_000),
                receiver: "someone-else@example.com".into(),
                fee: None,
            }),
        })
        .await
        .unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Failed);
    assert!(reply.refusal.unwrap().contains("not to our PayPal account"));
    assert_eq!(balances(&world, &invoice).await.0, 120_000);
}

#[tokio::test]
async fn kilele_refuses_an_invoice_not_sealed_by_its_sender() {
    let world = world(Options::default()).await;
    let (invoice, _) = world.payee.offer(&world.invoice_id).await.unwrap();
    // Sealed by a key Agentrix never published.
    let forged = saas_settle::Sealed::seal(
        SealKind::Invoice,
        &invoice,
        AGENTRIX,
        &Ed25519SigningKey::generate().unwrap(),
    )
    .unwrap();
    let client = saas_settle::DoorClient::new(
        reqwest::Client::new(),
        world.kilele_door.clone(),
        world.agentrix.agent(RECEIVABLES).unwrap(),
        AGENTRIX,
    )
    .unwrap();
    let reply = client
        .send(&SettleMessage::InvoiceOffer { invoice: forged })
        .await
        .unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Rejected);
    assert_eq!(
        saas_settle::era::bill_of(world.kilele_books.as_ref(), &world.invoice_id)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn an_unconfirmed_payment_blocks_any_other_until_the_same_mandate_settles_it() {
    let world = world(Options::default()).await;
    world
        .orders
        .fail_first
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let invoice = world.invoice_id.clone();
    let due = world.today() + Duration::days(14);
    let client = world.kilele_client(world.kilele.agent(PAYABLES).unwrap());
    let first_cart = cart(&world, &invoice, 120_000, due);
    let first = SettleMessage::Settle {
        intent: world.intent.clone(),
        cart: first_cart,
        payout: None,
    };

    // PayPal answers 503: we can't tell whether the capture happened.
    let unknown = client.send(&first).await.unwrap();
    assert_eq!(unknown.state, a2a_protocol_types::TaskState::Failed);
    assert!(unknown.refusal.unwrap().contains("never be paid twice"));

    // Another cart mandate for the same invoice is refused while that one is unconfirmed.
    let other = client
        .send(&SettleMessage::Settle {
            intent: world.intent.clone(),
            cart: cart(&world, &invoice, 120_000, due),
            payout: None,
        })
        .await
        .unwrap();
    assert_eq!(other.state, a2a_protocol_types::TaskState::Rejected);
    assert!(other.refusal.unwrap().contains("still being confirmed"));

    // The same cart mandate again settles it, under the same PayPal request id: one capture.
    let settled = client.send(&first).await.unwrap();
    assert!(
        matches!(settled.answer, Some(DoorAnswer::Settled { .. })),
        "{:?}",
        settled.refusal
    );
    assert_eq!(
        world.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(balances(&world, &invoice).await.0, 0);
}

#[tokio::test]
async fn money_that_moved_counts_even_when_the_books_lag() {
    let world = world(Options {
        payee_post_fails_once: true,
        ..Options::default()
    })
    .await;
    let invoice = world.invoice_id.clone();
    offer(&world, &invoice).await;
    let settled = world.payer.settle(AGENTRIX, &invoice).await.unwrap();
    // Kilele's books posted; Agentrix's didn't yet.
    assert_eq!(balances(&world, &invoice).await, (120_000, 0));

    // A second cart mandate for the invoice doesn't capture again.
    let due = world.today() + Duration::days(14);
    let again = world
        .kilele_client(world.kilele.agent(PAYABLES).unwrap())
        .send(&SettleMessage::Settle {
            intent: world.intent.clone(),
            cart: cart(&world, &invoice, 120_000, due),
            payout: None,
        })
        .await
        .unwrap();
    assert_eq!(again.state, a2a_protocol_types::TaskState::Rejected);
    assert_eq!(
        world.orders.posts.load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    // Reposting catches the books up, from the same capture id.
    world
        .payee
        .repost(&settled.terms.cart_mandate_id)
        .await
        .unwrap();
    assert_eq!(balances(&world, &invoice).await, (0, 0));
    assert_eq!(
        payments(&world.agentrix_books, "paypal:capture:CAP-1").await,
        1
    );
}

/// A cloud's agent token beside the signature: one that checks names the requester's principal
/// in what the door seals; one that doesn't is refused with a sealed receipt, before any
/// PayPal call. The signature is still required either way.
#[tokio::test]
async fn an_agent_token_names_the_requester_and_a_bad_one_is_refused() {
    use adk_core::principal::{AgentId, AgentPrincipal};
    let world = world(Options::default()).await;
    let invoice = world.invoice_id.clone();
    let due = world.today() + Duration::days(14);
    let message = SettleMessage::Settle {
        intent: world.intent.clone(),
        cart: cart(&world, &invoice, 120_000, due),
        payout: None,
    };
    let spiffe = "spiffe://kilele.example/agent/payables";
    world.tokens.accept(
        "entra-token",
        AGENTRIX,
        AgentPrincipal::new(AgentId::new(spiffe).unwrap()).with_class("payables-blueprint"),
        "https://login.microsoftonline.com/tenant/v2.0",
        "entra",
    );
    world.tokens.accept(
        "other-door",
        "https://elsewhere.example",
        AgentPrincipal::new(AgentId::new(spiffe).unwrap()),
        "https://iss.example",
        "google",
    );

    // A token the door can't check: refused and sealed, nothing done.
    let client = world.kilele_client(world.kilele.agent(PAYABLES).unwrap());
    for bad in ["forged-token", "other-door"] {
        client.set_token(Some(bad.into()));
        match client.send(&message).await {
            Err(saas_settle::door::ClientError::Refused {
                status,
                code,
                receipt,
                ..
            }) => {
                assert_eq!(status, 401);
                assert_eq!(code, "token_refused");
                let receipt = receipt.expect("a sealed refusal receipt");
                assert_eq!(receipt.body["requester"], client.id());
                assert!(receipt.body["principal"].is_null());
            }
            other => panic!("expected a token refusal, got {other:?}"),
        }
    }

    // A token that checks, on a request the mandate check refuses (the owner's key, not the
    // Payables agent): the sealed refusal names who the requester is in its cloud.
    let owner = world.kilele_client(world.kilele.owner.clone());
    owner.set_token(Some("entra-token".into()));
    let reply = owner.send(&message).await.unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Rejected);
    let receipt = reply.refusal_receipt.expect("a sealed refusal receipt");
    assert_eq!(receipt.body["principal"]["principal"]["id"], spiffe);
    assert_eq!(
        receipt.body["principal"]["principal"]["class"],
        "payables-blueprint"
    );
    assert_eq!(receipt.body["principal"]["profile"], "entra");
    assert_eq!(
        receipt.body["principal"]["token_digest"],
        saas_settle::tokens::token_digest("entra-token")
    );
    assert!(!receipt.body.to_string().contains("\"entra-token\""));

    // No token: the door works as before.
    let plain = world.kilele_client(world.kilele.agent(PAYABLES).unwrap());
    let reply = plain.send(&message).await.unwrap();
    assert_eq!(reply.state, a2a_protocol_types::TaskState::Completed);
    assert_eq!(calls(&world.paypal, "/v2/checkout/orders").await, 1);
}
