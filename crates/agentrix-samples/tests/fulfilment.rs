//! After the sale: the shop's agents move each real paid order on in its slot, each step a
//! record with its time and agent; delivery orders wait at ready for the courier; and only the
//! right party may make each change. One test: the samples register one app per process.

use std::path::Path;

use agentrix_samples::Config;
use agentrix_samples::fulfilment::{Actor, Change, pass_now, transition_on};
use agentrix_samples::held;

const K: &str = "kilele-bakery-sample";

async fn mounted() -> std::path::PathBuf {
    let ctx = agentrix_host::testing::context().await.expect("context");
    agentrix_identity::migrate(&ctx.pool)
        .await
        .expect("identity migrations");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data_dir =
        std::env::temp_dir().join(format!("agentrix-samples-fulfil-{}", rand::random::<u64>()));
    let config = Config {
        data_dir: data_dir.clone(),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: Some(root.join("crates/agentrix-samples/templates")),
        reseed: true,
    };
    agentrix_samples::mount_with(&ctx, config).expect("mount");
    data_dir
}

async fn state(id: &str) -> String {
    held::order(K, id).await.unwrap().unwrap().order.state
}

#[tokio::test]
async fn the_shop_moves_real_orders_on_and_only_the_right_party_makes_each_change() {
    let data = mounted().await;
    // The records open on first use: this pass seeds them, and moves nothing (all samples).
    let now = agentrix_records::time::now();
    assert!(
        pass_now(now).await.unwrap().is_empty(),
        "the bundle's sample orders don't move"
    );

    // Three real orders: KB-1001 for collection, KB-1002 and KB-1004 for delivery.
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", data.join("records.db").display()))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE rec_orders SET sample = 0 WHERE site = ? AND order_id IN ('KB-1001', 'KB-1002', 'KB-1004')",
    )
    .bind(K)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE rec_orders SET fulfilment = 'delivery' WHERE site = ? AND order_id IN ('KB-1002', 'KB-1004')",
    )
    .bind(K)
    .execute(&pool)
    .await
    .unwrap();

    // Well past their slots (the bundle's are weeks ahead), each pass takes one step per order.
    let later = now + 120 * 86_400;
    for _ in 0..4 {
        pass_now(later).await.unwrap();
    }
    let collected = held::order(K, "KB-1001").await.unwrap().unwrap();
    assert_eq!(collected.order.state, "collected");
    let steps: Vec<(&str, &str)> = collected
        .events
        .iter()
        .map(|e| (e.step.as_str(), e.actor.as_str()))
        .filter(|(s, _)| ["baking", "ready", "collected"].contains(s))
        .collect();
    assert_eq!(
        steps.iter().map(|s| s.0).collect::<Vec<_>>(),
        ["baking", "ready", "collected"]
    );
    assert!(
        steps
            .iter()
            .all(|(_, by)| !by.is_empty() && *by != "Wanjiru"),
        "the shop's agent, not the owner: {steps:?}"
    );
    // A delivery order waits at ready for the courier; a sample order is as the bundle has it.
    assert_eq!(state("KB-1002").await, "ready");
    assert_eq!(state("KB-1003").await, "paid");

    let customer = Actor::Customer {
        agent_id: "wba:https://test-agent-chiku.example#k1".into(),
        vouched_by: "https://test-agent-chiku.example".into(),
        method: "web-bot-auth".into(),
    };
    let shop = Actor::Shop {
        agent: "the Orders desk".into(),
    };
    // The customer's agent can't move the kitchen on; the shop can't confirm a delivery.
    let e = transition_on(K, "KB-1003", Change::Baking, &customer)
        .await
        .unwrap_err();
    assert!(format!("{e:?}").contains("Only the shop"), "{e:?}");
    let e = transition_on(K, "KB-1002", Change::ConfirmDelivered, &shop)
        .await
        .unwrap_err();
    assert!(format!("{e:?}").contains("Only the customer"), "{e:?}");
    // Confirming a delivery that isn't out says why.
    let e = transition_on(K, "KB-1002", Change::ConfirmDelivered, &customer)
        .await
        .unwrap_err();
    assert!(format!("{e:?}").contains("isn't out for delivery"), "{e:?}");

    // A held custom order can be cancelled by the customer's agent, with its reason kept.
    let v = transition_on(
        K,
        "KB-1044",
        Change::Cancel {
            note: "Plans changed.".into(),
        },
        &customer,
    )
    .await
    .unwrap();
    assert_eq!(v.order.state, "cancelled");
    assert!(
        v.events
            .iter()
            .any(|e| e.note.as_deref() == Some("Plans changed."))
    );

    // Booked with the courier: ready → out, the tracking number on the order; then the
    // customer's agent confirms the delivery.
    let v = transition_on(
        K,
        "KB-1002",
        Change::Booked {
            courier: "courier.example".into(),
            tracking: "TRK-1".into(),
        },
        &shop,
    )
    .await
    .unwrap();
    assert_eq!(
        (v.order.state.as_str(), v.order.tracking.as_deref()),
        ("out", Some("TRK-1"))
    );
    let v = transition_on(K, "KB-1002", Change::ConfirmDelivered, &customer)
        .await
        .unwrap();
    assert_eq!(v.order.state, "delivered");
    assert!(v.events.last().unwrap().actor.contains("vouched for by"));
    // KB-1004 is still waiting at ready.
    assert_eq!(state("KB-1004").await, "ready");
}
