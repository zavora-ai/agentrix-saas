//! A sample site's sale reaches Agentrix's runtime through Agentrix's PayPal app: its capture
//! and refund webhooks are kept as that sale's confirmation and never posted to Agentrix's books.
//! A capture that is no site's still goes the books' way.

use std::sync::Arc;

use adk_core::{Clock, SystemClock};
use saas_node::Node;
use saas_node::config::{BooksMode, Config, EnvFile};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sites_capture_and_refund_confirm_its_sale_and_post_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!("SAAS_DATA_DIR={}\n", dir.path().display()));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let node = Node::start(Config::from_env(&env, BooksMode::Fake, true), clock)
        .await
        .unwrap();

    let control = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    agentrix_effects::migrate(&control).await.unwrap();
    sqlx::query(
        "INSERT INTO customer_payments (id, site_id, order_id, fingerprint, provider, provider_ref, currency, amount_minor, state, capture_id, captured_minor, sandbox, created_at, updated_at)
         VALUES ('pay_1', 'kilele-bakery-sample', 'KB-1055', 'sha256:1', 'paypal', 'ORDER-1', 'USD', 171, 'captured', '8F698678SM3689049', 171, 1, 1, 1)",
    )
    .execute(&control)
    .await
    .unwrap();
    assert!(
        node.agentrix
            .set_site_sales(Arc::new(saas_node::samples::SiteSales::new(control)))
    );

    let capture = json!({
        "id": "WH-SITE-1", "event_type": "PAYMENT.CAPTURE.COMPLETED",
        "resource": { "id": "8F698678SM3689049", "status": "COMPLETED",
                      "amount": { "currency_code": "USD", "value": "1.71" } },
    });
    let handled = node.agentrix.on_paypal_event(&capture).await.unwrap();
    assert_eq!(
        handled.site_sale.as_deref(),
        Some("capture 8F698678SM3689049 confirms kilele-bakery-sample order KB-1055")
    );
    assert!(handled.posted.is_none(), "a site's sale is never posted again");

    let refund = json!({
        "id": "WH-SITE-2", "event_type": "PAYMENT.CAPTURE.REFUNDED",
        "resource": { "id": "1JU08902781691411", "status": "COMPLETED",
                      "amount": { "currency_code": "USD", "value": "1.71" },
                      "links": [{ "rel": "up", "href": "https://api.sandbox.paypal.com/v2/payments/captures/8F698678SM3689049" }] },
    });
    let handled = node.agentrix.on_paypal_event(&refund).await.unwrap();
    assert!(handled.site_sale.unwrap().starts_with("refund 1JU08902781691411"));

    // No site's capture: the books' way, as before (no invoice here, so nothing posts).
    let other = json!({
        "id": "WH-OTHER", "event_type": "PAYMENT.CAPTURE.COMPLETED",
        "resource": { "id": "NOT-A-SITE-SALE", "status": "COMPLETED",
                      "amount": { "currency_code": "USD", "value": "5.00" } },
    });
    let handled = node.agentrix.on_paypal_event(&other).await.unwrap();
    assert!(handled.site_sale.is_none());
    assert!(handled.posted.is_none());
}
