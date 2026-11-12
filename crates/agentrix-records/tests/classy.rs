//! The Classy Perfumes bundle loads and the figures its pages show come out of its rows.

use std::path::PathBuf;

use agentrix_records::seed::Loaded;
use agentrix_records::{Records, money};

fn bundle() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples/classy-perfumes")
}

// Sat 7 Nov 2026, 09:42 EAT.
const NOW: i64 = 1_794_033_720;

async fn loaded() -> Records {
    let r = Records::open_in_memory().await.unwrap();
    let first = r.load_bundle(&bundle(), None, false, NOW).await.unwrap();
    assert!(matches!(first, Loaded::Loaded { .. }));
    r
}

#[tokio::test]
async fn the_catalogue_is_big_enough_to_rank() {
    let r = loaded().await;
    let site = r.site("classy-perfumes-sample").await.unwrap().unwrap();
    assert!(site.is_sample());
    assert_eq!(site.archetype, "shop");
    let all = r.products(&site.site, None).await.unwrap();
    let scents: Vec<_> = all.iter().filter(|p| p.section != "sample").collect();
    assert!(scents.len() >= 24, "{} scents", scents.len());
    let families: std::collections::HashSet<_> =
        scents.iter().filter_map(|p| p.family.clone()).collect();
    assert_eq!(families.len(), 5);
    // Perfumes come in three sizes; gift sets say what's in the box ("2 × 30 ml").
    let sizes: std::collections::HashSet<_> = scents
        .iter()
        .filter(|p| p.section == "fragrance")
        .filter_map(|p| p.size.clone())
        .collect();
    assert_eq!(sizes.len(), 3);
    let duo = r
        .product(&site.site, "citrus-travel-duo")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(duo.size.as_deref(), Some("2 × 30 ml"));
    assert!(
        scents
            .iter()
            .all(|p| (400_000..=900_000).contains(&p.price_minor))
    );
    let gift = r
        .product(&site.site, "vanilla-gift-set")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(money::ksh(gift.price_minor), "KSh 4,500");
    assert_eq!(gift.attrs()["gift_ready"], true);
    assert_eq!(
        r.product(&site.site, "sample-2ml")
            .await
            .unwrap()
            .unwrap()
            .price_minor,
        0
    );
}

#[tokio::test]
async fn stock_counts_and_orders_add_up() {
    let r = loaded().await;
    let site = r.site("classy-perfumes-sample").await.unwrap().unwrap();
    let stock = r.stock(&site, NOW).await.unwrap();
    assert_eq!(stock["citrus-dawn"].left, Some(3));
    assert_eq!(stock["amber-noir"].left, Some(14));
    assert_eq!(stock["tuberose-night"].state, "out");
    assert_eq!(stock["amber-noir"].as_of, NOW - 40 * 60);
    let zones = r.zones(&site.site).await.unwrap();
    assert_eq!(money::ksh(zones[0].fee_minor), "KSh 300");
    assert_eq!(money::ksh(zones[1].fee_minor), "KSh 650");
    let cp2008 = r.order(&site, "CP-2008").await.unwrap().unwrap();
    assert_eq!(
        money::ksh(cp2008.order.total_minor),
        "KSh 5,250",
        "4,600 plus the 650 courier fee"
    );
    assert_eq!(cp2008.refunds[0].amount_minor, 230_000);
    assert_eq!(cp2008.refunds[0].state, "waiting");
    let list = r.live_price_list(&site.site).await.unwrap().unwrap();
    assert_eq!(list.version, "v31");
    assert_eq!(money::rate_line(list.usd_rate_minor), "1 USD = KSh 129.00");
}

#[tokio::test]
async fn order_timelines_run_in_order_and_same_day_beats_the_cutoff() {
    let r = loaded().await;
    let site = r.site("classy-perfumes-sample").await.unwrap().unwrap();
    let rank = |step: &str| match step {
        "requested" => 0,
        "paid" => 1,
        "out" => 2,
        "delivered" => 3,
        other => panic!("unexpected step {other}"),
    };
    for n in 2001..=2015 {
        let id = format!("CP-{n}");
        let v = r.order(&site, &id).await.unwrap().unwrap();
        let steps: Vec<i64> = v.events.iter().map(|e| rank(&e.step)).collect();
        assert!(
            steps.windows(2).all(|w| w[0] < w[1]),
            "{id} steps {steps:?}"
        );
        assert!(
            v.events.windows(2).all(|w| w[0].at < w[1].at),
            "{id}: each step comes after the one before"
        );
        let placed = &v.events[0];
        let delivered = v.events.iter().find(|e| e.step == "delivered");
        if let Some(d) = delivered {
            let same_day = agentrix_records::time::day_of(placed.at, site.utc_offset_min)
                == agentrix_records::time::day_of(d.at, site.utc_offset_min);
            if same_day {
                let clock = agentrix_records::time::fmt_clock(placed.at, site.utc_offset_min);
                assert!(
                    clock.as_str() < "14:00",
                    "{id} placed at {clock}, after the cut-off"
                );
            }
        }
    }
}
