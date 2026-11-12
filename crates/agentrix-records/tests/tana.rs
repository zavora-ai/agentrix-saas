//! The Tana Valley Times bundle loads, and the figures its pages show come out of its rows.

use std::collections::HashSet;
use std::path::PathBuf;

use agentrix_records::seed::Loaded;
use agentrix_records::{Records, time};

fn bundle() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples/tana-valley-times")
}

// Sun 8 Nov 2026, 07:30 EAT: the day the bundle is drawn for.
const NOW: i64 = 1_794_112_200;
const SITE: &str = "tana-valley-times-sample";

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
async fn the_paper_has_three_years_of_stories_in_five_sections() {
    let r = loaded().await;
    let site = r.site(SITE).await.unwrap().unwrap();
    assert!(site.is_sample());
    assert_eq!(site.archetype, "news");
    assert_eq!(site.fact("/owner/first"), "Nduku");
    let stories = r.stories(SITE, None, 500).await.unwrap();
    assert_eq!(
        stories.len(),
        63,
        "60 from the archive and three from this week"
    );
    let sections: HashSet<&str> = stories.iter().map(|s| s.section.as_str()).collect();
    assert_eq!(sections.len(), 5);
    assert!(stories.iter().filter(|s| s.locked()).count() >= 5);
    assert!(
        stories
            .iter()
            .all(|s| (2..=6).contains(&s.paragraphs().len()))
    );
    let oldest = stories.last().unwrap().published_at;
    assert!(
        NOW - oldest > 2 * 365 * 86_400,
        "the archive spans more than two years"
    );
    let authors = r.authors(SITE).await.unwrap();
    assert_eq!(authors.len(), 4);
    assert!(
        stories
            .iter()
            .all(|s| authors.iter().any(|a| a.author_id == s.author_id))
    );
}

#[tokio::test]
async fn the_rail_story_carries_its_correction_and_a_scheduled_step() {
    let r = loaded().await;
    let slug = "council-approves-first-phase-of-the-northern-rail-extension";
    let lead = r.story(SITE, slug).await.unwrap().unwrap();
    assert_eq!(
        time::fmt_day_clock(lead.published_at, 180),
        "Thu 5 Nov, 11:04"
    );
    let corrections = r.corrections(SITE, Some(slug)).await.unwrap();
    assert_eq!(corrections.len(), 1);
    assert_eq!(corrections[0].what, "KSh 41bn, not 48bn");
    assert_eq!(time::fmt_clock(corrections[0].at, 180), "16:20");
    let steps = r.timeline(SITE, "rail").await.unwrap();
    assert!(steps.iter().any(|s| s.kind == "correction"));
    let last = steps.last().unwrap();
    assert_eq!(last.kind, "scheduled");
    assert!(last.at > NOW);
    let all = r.corrections(SITE, None).await.unwrap();
    let mut corrected_sections = HashSet::new();
    for c in &all {
        let s = r.story(SITE, &c.story_slug).await.unwrap().unwrap();
        corrected_sections.insert(s.section);
    }
    assert_eq!(
        corrected_sections.len(),
        5,
        "at least one correction per section"
    );
}

#[tokio::test]
async fn readers_subscribers_and_the_digest_add_up() {
    let r = loaded().await;
    assert_eq!(r.customers(SITE).await.unwrap().len(), 40);
    let subs = r.subscribers(SITE).await.unwrap();
    assert_eq!(subs.len(), 12);
    for s in subs.iter().filter(|s| s.source.contains("payment")) {
        let until = s.until_at.expect("a payment has an end date");
        assert!(
            until - s.since <= 31 * 86_400,
            "{}: one payment buys one month",
            s.subscriber_id
        );
    }
    assert!(r.is_subscriber(SITE, "cus-halima", NOW).await.unwrap());
    assert!(!r.is_subscriber(SITE, "cus-baraka", NOW).await.unwrap());
    let digest = r.digest_subs(SITE).await.unwrap();
    assert_eq!(digest.iter().filter(|d| d.state == "confirmed").count(), 25);

    let (_, token) = r
        .start_digest(SITE, "Baraka@Example.com", &["Transport".into()], NOW)
        .await
        .unwrap();
    let confirmed = r
        .digest_by_token(SITE, &token, true, NOW)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(confirmed.state, "confirmed");
    assert_eq!(confirmed.email, "baraka@example.com");
    let stopped = r
        .digest_by_token(SITE, &token, false, NOW)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.state, "stopped");
    assert_eq!(stopped.topics, "[]", "stopping deletes the topics");

    r.grant_subscription(
        SITE,
        "cus-baraka",
        "Supporter",
        "sample payment",
        Some(NOW + 30 * 86_400),
        NOW,
    )
    .await
    .unwrap();
    assert!(r.is_subscriber(SITE, "cus-baraka", NOW).await.unwrap());
}

#[tokio::test]
async fn each_reader_gets_the_friday_five_once_and_can_stop_it_from_any_edition() {
    let r = loaded().await;
    let site = "tana-valley-times-sample";
    let due = r.digest_due(site, NOW).await.unwrap();
    assert!(!due.is_empty(), "confirmed readers are due an edition");
    let (sub, stop) = due[0].clone();
    // The stop token is stable from week to week.
    let again = r.digest_due(site, NOW).await.unwrap();
    assert_eq!(
        again
            .iter()
            .find(|(s, _)| s.sub_id == sub.sub_id)
            .unwrap()
            .1,
        stop
    );
    // Once recorded, the reader isn't due again for that edition.
    r.mark_digest_sent(site, &sub.sub_id, NOW + 60)
        .await
        .unwrap();
    let after = r.digest_due(site, NOW).await.unwrap();
    assert!(!after.iter().any(|(s, _)| s.sub_id == sub.sub_id));
    // The stop link in any weekly email stops it and deletes the topics.
    let stopped = r
        .digest_by_token(site, &stop, false, NOW + 120)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.state, "stopped");
    assert_eq!(stopped.topics, "[]");
}
