//! The Kilele Bakery bundle loads and every figure the journeys show comes out of its rows.

use std::path::PathBuf;

use agentrix_records::orders::{AgentProof, Contact, Placement};
use agentrix_records::pricing::CakeChoice;
use agentrix_records::seed::Loaded;
use agentrix_records::{Records, money, time};

fn bundle() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples/kilele-bakery")
}

// Sat 7 Nov 2026, 09:42 EAT: the anchor the journeys are drawn at.
const NOW: i64 = 1_794_033_720;

async fn loaded() -> Records {
    let r = Records::open_in_memory().await.unwrap();
    let first = r.load_bundle(&bundle(), None, false, NOW).await.unwrap();
    assert!(matches!(first, Loaded::Loaded { .. }));
    assert!(matches!(
        r.load_bundle(&bundle(), None, false, NOW).await.unwrap(),
        Loaded::Current { .. }
    ));
    r
}

#[tokio::test]
async fn orders_add_up_from_their_lines() {
    let r = loaded().await;
    let site = r.site("kilele-bakery-sample").await.unwrap().unwrap();
    assert!(site.is_sample());
    let kb1042 = r.order(&site, "KB-1042").await.unwrap().unwrap();
    assert_eq!(money::ksh(kb1042.order.total_minor), "KSh 2,550");
    assert_eq!(money::usd(kb1042.order.usd_cents()), "USD 19.77");
    assert_eq!(kb1042.refunds[0].amount_minor, 135_000);
    let kb1043 = r.order(&site, "kb-1043").await.unwrap().unwrap();
    assert_eq!(money::ksh(kb1043.order.subtotal_minor), "KSh 5,250");
    // Collected, so no delivery fee (facts.md: KSh 5,250, USD 40.70).
    assert_eq!(kb1043.order.fulfilment, "collection");
    assert_eq!(money::ksh(kb1043.order.total_minor), "KSh 5,250");
    assert_eq!(money::usd(kb1043.order.usd_cents()), "USD 40.70");
    assert_eq!(kb1043.order.state, "paid");
    assert_eq!(kb1043.events.len(), 3);
}

#[tokio::test]
async fn stock_is_the_plan_less_holds_and_orders() {
    let r = loaded().await;
    let site = r.site("kilele-bakery-sample").await.unwrap().unwrap();
    let stock = r.stock(&site, NOW).await.unwrap();
    assert_eq!(
        stock["sourdough"].left,
        Some(6),
        "8 baked, 2 held as H-0193"
    );
    assert_eq!(stock["red-velvet"].left, Some(12));
    assert_eq!(stock["cinnamon-roll"].state, "out");
    assert_eq!(
        time::fmt_clock(stock["cinnamon-roll"].back_at.unwrap(), 180),
        "06:30"
    );
    assert_eq!(stock["croissant"].state, "batch");
    assert_eq!(
        time::fmt_clock(stock["croissant"].next_batch_at.unwrap(), 180),
        "10:00"
    );
}

#[tokio::test]
async fn the_cake_is_priced_from_rules_and_held_for_the_owner() {
    let r = loaded().await;
    let site = r.site("kilele-bakery-sample").await.unwrap().unwrap();
    let mut choice = CakeChoice {
        size: Some("size.2".into()),
        sponge: Some("sponge.lemon".into()),
        colour: Some("colour.forest".into()),
        finish: Some("finish.ombre".into()),
        deco: Some("deco.berries".into()),
        message: "Happy 7th Amina".into(),
        date: Some("2026-11-14".into()),
        note: "No nuts".into(),
        idea: None,
    };
    let q = r.quote_cake(&site, &choice, NOW).await.unwrap();
    assert!(q.complete, "{:?}", q.flags);
    assert_eq!(money::ksh(q.total_minor), "KSh 5,250");
    assert_eq!(money::usd(q.usd_cents), "USD 40.70");
    assert_eq!(q.price_list, "v14");

    // Gluten-free on 3 tiers is refused by the rules, with words.
    choice.size = Some("size.3".into());
    choice.sponge = Some("sponge.gf".into());
    let bad = r.quote_cake(&site, &choice, NOW).await.unwrap();
    assert!(!bad.complete);
    assert!(bad.flags.iter().any(|f| {
        f.level == "error"
            && f.text
                .contains("Gluten-free sponge is only for 1 tier and 2 tiers")
    }));

    // Tomorrow is too soon for any size.
    choice.size = Some("size.1".into());
    choice.sponge = Some("sponge.vanilla".into());
    choice.date = Some("2026-11-08".into());
    assert!(!r.quote_cake(&site, &choice, NOW).await.unwrap().complete);

    choice.date = Some("2026-11-14".into());
    let q = r.quote_cake(&site, &choice, NOW).await.unwrap();
    let contact = Contact {
        name: "Test".into(),
        email: Some("t@example.com".into()),
        ..Default::default()
    };
    let proof = AgentProof {
        id: "wba:https://assistant.example#key-1".into(),
        vouched_by: "https://assistant.example".into(),
        method: "web-bot-auth".into(),
    };
    let how = Placement {
        channel: "agent door".into(),
        fulfilment: "collection".into(),
        agent: Some("wba:https://assistant.example#key-1 · verified by web-bot-auth".into()),
        proof: Some(proof.clone()),
        ..Default::default()
    };
    let order = r
        .request_custom(&site, &choice, &q, &contact, &how, NOW)
        .await
        .unwrap();
    assert_eq!(
        order.order.order_id, "KB-1052",
        "numbering carries on after the seeded KB-1051"
    );
    assert_eq!(order.order.state, "held");
    // The hold keeps who proved itself at the door, typed, for the owner's card.
    assert_eq!(order.order.proven_agent(), Some(proof.clone()));
    let read = r.order(&site, "KB-1052").await.unwrap().unwrap();
    assert_eq!(read.order.proven_agent(), Some(proof));
    let approved = r
        .decide(&site, "KB-1052", true, "Wanjiru", None, NOW + 60)
        .await
        .unwrap();
    assert_eq!(approved.order.state, "approved");
}

#[tokio::test]
async fn a_cart_becomes_an_order_and_takes_stock() {
    let r = loaded().await;
    let site = r.site("kilele-bakery-sample").await.unwrap().unwrap();
    r.add_to_cart(&site.site, "c1", "sourdough", 2, false)
        .await
        .unwrap();
    r.add_to_cart(&site.site, "c1", "weekend-bake-box", 1, false)
        .await
        .unwrap();
    // Today's 10:00–12:00 collection window (Saturday).
    let slots = r
        .slots(&site, "collection", NOW, NOW + 86_400)
        .await
        .unwrap();
    let slot = slots.iter().find(|s| !s.full).unwrap().slot_id.clone();
    let how = Placement {
        channel: "site".into(),
        fulfilment: "collection".into(),
        slot_id: Some(slot),
        ..Default::default()
    };
    let contact = Contact {
        name: "Test".into(),
        ..Default::default()
    };
    let view = r
        .place_order(&site, "c1", &contact, &how, NOW)
        .await
        .unwrap();
    assert_eq!(money::ksh(view.order.total_minor), "KSh 2,250");
    assert_eq!(view.order.state, "awaiting_payment");
    let stock = r.stock(&site, NOW).await.unwrap();
    assert_eq!(stock["sourdough"].left, Some(4));
    // The preorder window that the mission counts: 11 of 12 taken.
    let later = r
        .slots(&site, "collection", NOW + 40 * 86_400, NOW + 45 * 86_400)
        .await
        .unwrap();
    assert!(later.iter().any(|s| s.taken == 11 && s.capacity == 12));
}
