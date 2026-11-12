//! PayPal's confirmation of a site's sale is kept once per event id, on the order it confirms,
//! with one timeline line.

use std::path::PathBuf;

use agentrix_records::Records;
use agentrix_records::confirmations::PayPalConfirmation;

const NOW: i64 = 1_794_033_720;

#[tokio::test]
async fn a_paypal_event_confirms_its_sale_once() {
    let r = Records::open_in_memory().await.unwrap();
    r.load_bundle(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples/kilele-bakery"),
        None,
        false,
        NOW,
    )
    .await
    .unwrap();
    let site = r.site("kilele-bakery-sample").await.unwrap().unwrap();
    let c = PayPalConfirmation {
        event_id: "WH-8F698678".into(),
        order_id: "KB-1042".into(),
        event_type: "PAYMENT.CAPTURE.COMPLETED".into(),
        kind: "capture".into(),
        object_id: "8F698678SM3689049".into(),
        status: "COMPLETED".into(),
        confirmed_at: NOW + 60,
    };
    assert!(r.confirm_paypal(&site.site, &c).await.unwrap());
    // PayPal delivers it again: nothing changes.
    assert!(!r.confirm_paypal(&site.site, &c).await.unwrap());
    assert_eq!(
        r.paypal_confirmations(&site.site, "KB-1042").await.unwrap(),
        vec![c.clone()]
    );
    let order = r.order(&site, "KB-1042").await.unwrap().unwrap();
    let lines: Vec<_> = order
        .events
        .iter()
        .filter(|e| e.step == "paypal_confirmed")
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].actor, "PayPal");
    assert!(
        lines[0]
            .note
            .as_deref()
            .unwrap()
            .contains("8F698678SM3689049")
    );

    // A refund on a sale the site no longer has an order for is still kept, without a line.
    let refund = PayPalConfirmation {
        event_id: "WH-REF-1".into(),
        order_id: "KB-9999".into(),
        event_type: "PAYMENT.CAPTURE.REFUNDED".into(),
        kind: "refund".into(),
        object_id: "1JU08902781691411".into(),
        status: "COMPLETED".into(),
        confirmed_at: NOW + 120,
    };
    assert!(r.confirm_paypal(&site.site, &refund).await.unwrap());
    assert_eq!(
        r.paypal_confirmations(&site.site, "KB-9999")
            .await
            .unwrap()
            .len(),
        1
    );
}
