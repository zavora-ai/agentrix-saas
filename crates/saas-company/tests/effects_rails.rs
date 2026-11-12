//! The agents drive `agentrix-effects` on the governed path, with PayPal faked over HTTP.

#![cfg(feature = "effects")]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentrix_effects::Effects;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use agentrix_host::testing::MemoryReceipts;
use agentrix_types::SiteId;
use saas_company::desk::agentrix_catalogue;
use saas_company::effects_rails::EffectsRails;
use saas_company::identity::PAYABLES;
use saas_company::rails::Rails;
use saas_company::scripted::Turn;
use saas_core::Books;
use saas_core::books::BillId;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// PayPal's Payouts endpoint: each new `sender_batch_id` makes a batch (`BATCH-<n>`); a repeat
/// is refused, as PayPal does.
struct Payouts(Arc<AtomicUsize>, std::sync::Mutex<Vec<Value>>);

impl Respond for Payouts {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = request.body_json().unwrap();
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let id = body["sender_batch_header"]["sender_batch_id"].clone();
        let mut seen = self.1.lock().unwrap();
        if !seen.contains(&id) {
            seen.push(id);
            return ResponseTemplate::new(201).set_body_json(json!({
                "batch_header": { "payout_batch_id": format!("BATCH-{n}"), "batch_status": "PENDING",
                    "sender_batch_header": body["sender_batch_header"] },
            }));
        }
        ResponseTemplate::new(400).set_body_json(json!({ "name": "USER_BUSINESS_ERROR",
            "details": [{ "issue": "Batch with given sender_batch_id already exists" }] }))
    }
}

async fn paypal(posts: Arc<AtomicUsize>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "access_token": "A21AA-test", "expires_in": 32400 })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts"))
        .respond_with(Payouts(posts, std::sync::Mutex::new(Vec::new())))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts/BATCH-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "batch_header": { "payout_batch_id": "BATCH-1", "batch_status": "SUCCESS" },
            "items": [{ "payout_item_id": "ITEM-BATCH-1", "transaction_status": "SUCCESS",
                "payout_batch_id": "BATCH-1",
                "payout_item": { "receiver": "billing@vultr.example",
                    "amount": { "value": "120.00", "currency": "USD" } } }],
        })))
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn payables_pays_a_bill_through_agentrix_effects_once() {
    let posts = Arc::new(AtomicUsize::new(0));
    let server = paypal(posts.clone()).await;
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    let client = Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: Some("WH-1".into()),
            environment: Environment::Custom(server.uri()),
        },
        reqwest::Client::new(),
    ));
    let effects = Effects::new(
        ctx.pool.clone(),
        Some(client.clone()),
        Arc::new(MemoryReceipts::default()),
    );
    let rails = Arc::new(EffectsRails::new(
        effects.clone(),
        client,
        SiteId::new("saas-agentrix"),
        agentrix_catalogue(),
        "https://money.agentrix.website/thanks",
    ));
    let rails_for_desk: Arc<dyn Rails> = rails.clone();
    let h = common::agentrix_wrapped(9, move |_| rails_for_desk).await;
    let bill = h.seeded.vultr_bill.to_string();

    h.model(PAYABLES).extend([
        Turn::Calls(vec![(
            "pay_bill".into(),
            json!({ "bill_id": bill, "amount_usd": 120.0, "payee": "billing@vultr.example" }),
        )]),
        Turn::Text("Paid Vultr.".into()),
    ]);
    let turn = h
        .company
        .ask(PAYABLES, "ap-effects", "Pay the Vultr bill.")
        .await
        .unwrap();
    let paid = turn.result("pay_bill").expect("paid");
    // PayPal took the batch; its item is confirmed later, and the books post it then.
    assert_eq!(paid["paypal_id"], json!("BATCH-1"), "{turn:?}");
    assert_eq!(paid["status"], json!("PENDING"));
    assert_eq!(paid["posted"], json!(false));
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    let stored = effects
        .payouts_for_site(&SiteId::new("saas-agentrix"))
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].amount_minor, 12_000);
    let bill = h.books.bill(&BillId::from(bill.as_str())).await.unwrap();
    assert!(
        bill.balance_due.is_positive(),
        "not posted before PayPal confirms the item"
    );

    // The same governed call again (a retry, a resume): the same effect, no second payout.
    let key = "agentrix-saas/agentrix/agent:agentrix/payables/ap-effects/retry";
    let first = rails
        .payout(key, "billing@vultr.example", "USD", 5_000, "Retry check")
        .await;
    let after_first = posts.load(Ordering::SeqCst);
    let again = rails
        .payout(key, "billing@vultr.example", "USD", 5_000, "Retry check")
        .await;
    assert_eq!(
        posts.load(Ordering::SeqCst),
        after_first,
        "the retry never reached PayPal"
    );
    let (first, again) = (first.unwrap(), again.unwrap());
    assert_eq!(
        first.paypal_id, again.paypal_id,
        "a repeated key is the first payout"
    );
    assert_eq!(again.amount_minor, 5_000);
    assert_eq!(
        effects
            .payouts_for_site(&SiteId::new("saas-agentrix"))
            .await
            .unwrap()
            .len(),
        2
    );
}
