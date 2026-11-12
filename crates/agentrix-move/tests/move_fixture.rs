//! S1-06 acceptance on Zavora's 5-page infected fixture (`fixtures/wordpress-infected/exports/dev`),
//! with in-memory ports: no sealed rooms and no model calls (D10a).
//!
//! Done when: the fixture reaches a candidate release with parity >= 0.95 and the infection
//! quarantined; every step is persisted and a crashed Move resumes (N9).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use agentrix_move::candidate::load_manifest;
use agentrix_move::pipeline::Proved;
use agentrix_move::store::{CardState, ReleaseState};
use agentrix_move::testing::{FakeGateway, FakeRooms, MemoryBlobs, MemoryEvents, MemoryReceipts};
use agentrix_move::{MoveInputs, MoveState, MoveStore, Mover};
use agentrix_types::ui::{StepName, StepState};
use agentrix_types::{QuarantineEntry, ReceiptKind, SiteId};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const EXPORT: &[u8] = include_bytes!("../../../fixtures/wordpress-infected/exports/dev/export.xml");
const WP_CONTENT: &[u8] =
    include_bytes!("../../../fixtures/wordpress-infected/exports/dev/wp-content.tar.gz");

/// control.db as `db::open` builds it: auth and claim schemas, then the migrations.
async fn control_db() -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for sql in [
        include_str!("../../agentrix-node/src/auth/schema.sql"),
        include_str!("../../agentrix-node/src/claim/schema.sql"),
        include_str!("../../agentrix-node/src/db/migrations/0001_control.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO sites (site_id, domain, name, old_origin, fixture, created_at) \
         VALUES ('site_kilele', 'kilele-bakery.example', 'Kilele Bakery', 'https://kilele-bakery.example', 1, '2026-11-07T20:00:00.000000Z')",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

struct World {
    pool: SqlitePool,
    blobs: Arc<MemoryBlobs>,
    receipts: Arc<MemoryReceipts>,
    events: Arc<MemoryEvents>,
    site: SiteId,
    inputs: MoveInputs,
}

impl World {
    async fn new() -> Self {
        let pool = control_db().await;
        let blobs = Arc::new(MemoryBlobs::default());
        let site = SiteId::new("site_kilele");
        let inputs = MoveInputs {
            domain: "kilele-bakery.example".into(),
            origin: "https://kilele-bakery.example".into(),
            site_name: Some("Kilele Bakery".into()),
            wxr: Some(blobs.insert(&site, EXPORT)),
            dump: None,
            wp_content: Some(blobs.insert(&site, WP_CONTENT)),
            crawl: None,
            fixture: true,
            look: None,
        };
        Self {
            pool,
            blobs,
            receipts: Arc::new(MemoryReceipts::default()),
            events: Arc::new(MemoryEvents::default()),
            site,
            inputs,
        }
    }

    fn mover(&self, rooms: Arc<FakeRooms>, gateway: Arc<FakeGateway>) -> Mover {
        Mover {
            store: MoveStore::new(self.pool.clone()),
            blobs: self.blobs.clone(),
            receipts: self.receipts.clone(),
            events: self.events.clone(),
            rooms,
            gateway,
            policy: Default::default(),
        }
    }

    fn rooms(&self) -> Arc<FakeRooms> {
        Arc::new(FakeRooms::new(self.blobs.clone()))
    }
}

#[tokio::test]
async fn the_infected_fixture_reaches_a_candidate_with_the_infection_left_behind() {
    let world = World::new().await;
    let rooms = world.rooms();
    let gateway = Arc::new(FakeGateway {
        fail_first: ["/about/".to_owned()].into(),
        ..Default::default()
    });
    let mover = world.mover(rooms.clone(), gateway.clone());

    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    let state = mover.run(&move_id).await.unwrap();
    assert_eq!(
        state,
        MoveState::AwaitingOwner,
        "the Mover stops at the go-live card (R7)"
    );

    // Every step persisted, in order, each with a receipt (R6.5, N9).
    let store = MoveStore::new(world.pool.clone());
    let done: Vec<StepName> = store
        .steps(&move_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|s| matches!(s.state, StepState::Done | StepState::Waiting))
        .map(|s| s.step)
        .collect();
    assert_eq!(
        done,
        [
            StepName::Assess,
            StepName::Claim,
            StepName::Replay,
            StepName::Clean,
            StepName::Rebuild,
            StepName::Prove,
            StepName::GoLive
        ]
    );
    assert_eq!(world.receipts.of_kind(ReceiptKind::MoveStep).len(), 5);

    // The infection is quarantined: attacks 3-7 by the gauntlet's detector ids (R5.2).
    let clean = world
        .receipts
        .of_kind(ReceiptKind::MoveStep)
        .into_iter()
        .find(|r| {
            r.body
                .detail
                .as_ref()
                .and_then(|d| d.get("step"))
                .and_then(|s| s.as_str())
                == Some("clean")
        })
        .unwrap();
    let entries: Vec<QuarantineEntry> =
        serde_json::from_value(clean.body.detail.as_ref().unwrap()["quarantined"].clone()).unwrap();
    let detectors: BTreeSet<String> = entries.iter().filter_map(|e| e.detector.clone()).collect();
    for expected in [
        "hidden-instructions",
        "spam-links",
        "cloaking",
        "executable-upload",
        "obfuscated-script",
    ] {
        assert!(
            detectors.contains(expected),
            "{expected} missing from {detectors:?}\n{entries:#?}"
        );
    }
    assert!(
        entries.iter().any(|e| e.location.contains("cache.php")),
        "the webshell is named"
    );
    let held = store.quarantine(&world.site).await.unwrap();
    assert_eq!(held.len(), entries.len());

    // Parity and the candidate (R6.4).
    let resume = store
        .resume_of(&move_id, StepName::Prove)
        .await
        .unwrap()
        .unwrap();
    let proved_digest = resume["proved"].as_str().unwrap().parse().unwrap();
    let proved: Proved = serde_json::from_slice(
        &agentrix_move::ports::Blobs::get(world.blobs.as_ref(), &world.site, &proved_digest)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        proved.parity.site >= 0.95,
        "site parity {}",
        proved.parity.site
    );
    assert!(proved.parity.rendered_in_room);
    assert_eq!(proved.parity.pages.len(), 5);
    let candidate = proved.candidate.expect("a candidate release");
    let release = store
        .release(&world.site, candidate.seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(release.state, ReleaseState::Candidate);
    assert!(
        release.checkpoint.is_none(),
        "unsigned until the owner approves"
    );

    // Nothing infected reaches the release (R5.5).
    let manifest = load_manifest(
        world.blobs.as_ref(),
        &world.site,
        &candidate.manifest_digest,
    )
    .await
    .unwrap();
    let paths: Vec<&str> = manifest.files.keys().map(String::as_str).collect();
    for page in [
        "index.html",
        "about/index.html",
        "custom-cakes/index.html",
        "contact/index.html",
        "sourdough-starter-turns-five/index.html",
        "index.md",
        "assets/site.css",
    ] {
        assert!(paths.contains(&page), "{page} missing from {paths:?}");
    }
    assert!(
        !paths
            .iter()
            .any(|p| p.ends_with(".php") || p.ends_with("photo.jpg")),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"wp-content/uploads/2026/10/sourdough-loaf.png"),
        "{paths:?}"
    );
    for (path, entry) in &manifest.files {
        let digest = entry.sha256.as_str().parse().unwrap();
        let bytes = agentrix_move::ports::Blobs::get(world.blobs.as_ref(), &world.site, &digest)
            .await
            .unwrap()
            .unwrap();
        if !(path.ends_with(".html") || path.ends_with(".md")) {
            continue;
        }
        let text = String::from_utf8(bytes).unwrap();
        for bad in [
            "kilele-support.test",
            "kilele-assets.test",
            "cheap-casino.test",
            "pills-online.test",
            "cdn-stats.test",
            "_0x4f",
            "cache.php",
            "admin password",
        ] {
            assert!(!text.contains(bad), "{path} carries {bad}");
        }
        // Only the platform's scripts: the Ask island and W8's visit counter, both same-origin.
        let scripts: Vec<&str> = text
            .match_indices("<script")
            .map(|(i, _)| &text[i..text[i..].find('>').map_or(text.len(), |e| i + e)])
            .collect();
        let platform: &[&str] = if path.ends_with(".html") {
            &["/_agentrix/ask.js", "/_agx/visit.js"]
        } else {
            &[]
        };
        assert_eq!(
            scripts.len(),
            platform.len(),
            "{path}: only the platform's scripts: {scripts:?}"
        );
        for (tag, src) in scripts.iter().zip(platform) {
            assert!(tag.contains(&format!("src=\"{src}\"")), "{path}: {tag}");
        }
    }

    // The builder's refused first try was fed back (R6.3).
    let feedback = gateway
        .feedback
        .lock()
        .unwrap()
        .get("/about/")
        .cloned()
        .unwrap_or_default();
    assert!(
        feedback.iter().any(|f| f.contains("<script>")),
        "{feedback:?}"
    );

    // Attack 9: the old theme's build hook ran only in a room, was killed, and the build was retried.
    let builds: Vec<_> = rooms
        .runs
        .lock()
        .unwrap()
        .iter()
        .filter(|(j, _)| j.kind == agentrix_types::JobKind::Build)
        .map(|(_, killed)| *killed)
        .collect();
    assert_eq!(builds.len(), 2);
    assert!(builds[0].is_some() && builds[1].is_none());

    // The go-live card waits, bound to the release and charter v1 (R7.1, R7.2).
    let cards = store.cards(&world.site).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].state, CardState::Waiting);
    assert!(cards[0].card.fingerprint_matches().unwrap());
    assert_eq!(
        cards[0].card.effect.args["release"],
        candidate.manifest_digest.to_string()
    );
    assert!(store.charter(&world.site, 1).await.unwrap().is_some());

    let names = world.events.names();
    for name in ["step", "page", "quarantined", "parity", "card", "done"] {
        assert!(
            names.iter().any(|n| n == name),
            "no {name} event in {names:?}"
        );
    }
    assert!(
        names.iter().any(|n| n == "error"),
        "the retries are shown as errors that retry"
    );
}

#[tokio::test]
async fn the_rebuild_uses_the_look_chosen_before_the_move() {
    use agentrix_move::pipeline::BuiltSite;
    use agentrix_move::theme::Tokens;
    let mut world = World::new().await;
    let look = Tokens {
        bg: "#fffbf2".into(),
        soft: "#eceee2".into(),
        ink: "#1a1a1a".into(),
        ink2: "#5f5e5b".into(),
        line: "#ddd9d2".into(),
        accent: "#14532d".into(),
        on_accent: "#ffffff".into(),
        display: "'Archivo Black', system-ui, sans-serif".into(),
        text: "'DM Sans', system-ui, sans-serif".into(),
    };
    world.inputs.look = Some(look.clone());
    let mover = world.mover(world.rooms(), Arc::new(FakeGateway::default()));
    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    assert_eq!(mover.run(&move_id).await.unwrap(), MoveState::AwaitingOwner);

    let store = MoveStore::new(world.pool.clone());
    let resume = store
        .resume_of(&move_id, StepName::Rebuild)
        .await
        .unwrap()
        .unwrap();
    let digest = resume["built"].as_str().unwrap().parse().unwrap();
    let built: BuiltSite = serde_json::from_slice(
        &agentrix_move::ports::Blobs::get(world.blobs.as_ref(), &world.site, &digest)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(built.tokens, Tokens::chosen(&look));
    assert_eq!(built.tokens.accent, "#14532d");
    assert_eq!(built.tokens.display, "Archivo Black, system-ui, sans-serif");
    assert!(
        built
            .legacy_theme
            .as_deref()
            .is_some_and(|w| w.contains("new look you chose")),
        "{:?}",
        built.legacy_theme
    );
}

#[tokio::test]
async fn a_move_that_stops_resumes_from_its_last_finished_step() {
    let world = World::new().await;
    let broken = Arc::new(FakeGateway {
        broken_screen: true,
        ..Default::default()
    });
    let mover = world.mover(world.rooms(), broken);
    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    let err = mover.run(&move_id).await.unwrap_err();
    assert!(err.to_string().contains("safety screen"), "{err}");

    let store = MoveStore::new(world.pool.clone());
    assert_eq!(
        store.move_record(&move_id).await.unwrap().unwrap().state,
        MoveState::Replayed
    );

    let fixed = Arc::new(FakeGateway::default());
    let mover = world.mover(world.rooms(), fixed.clone());
    assert_eq!(mover.run(&move_id).await.unwrap(), MoveState::AwaitingOwner);
    let steps = store.steps(&move_id).await.unwrap();
    let count = |step, state| {
        steps
            .iter()
            .filter(|s| s.step == step && s.state == state)
            .count()
    };
    assert_eq!(
        count(StepName::Replay, StepState::Done),
        1,
        "replay is not run twice"
    );
    assert_eq!(count(StepName::Clean, StepState::Failed), 1);
    assert_eq!(count(StepName::Clean, StepState::Done), 1);
    assert!(fixed.screens.load(Ordering::SeqCst) > 0);
}

#[tokio::test]
async fn pages_that_lose_text_are_flagged_and_no_candidate_is_raised() {
    let world = World::new().await;
    let gateway = Arc::new(FakeGateway {
        always_short: ["/about/".to_owned()].into(),
        ..Default::default()
    });
    let mover = world.mover(world.rooms(), gateway.clone());
    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    assert_eq!(
        mover.run(&move_id).await.unwrap(),
        MoveState::Rebuilt,
        "waits for the owner (R6.4)"
    );
    let store = MoveStore::new(world.pool.clone());
    assert!(store.cards(&world.site).await.unwrap().is_empty());
    assert!(store.releases(&world.site).await.unwrap().is_empty());
    let attempts = gateway
        .feedback
        .lock()
        .unwrap()
        .get("/about/")
        .map_or(0, Vec::len);
    assert_eq!(attempts, 2, "tries 2 and 3 each got the parity shortfall");
    // Accepting before Prove has asked is refused in plain words (the button can appear early).
    let early = world.mover(world.rooms(), Arc::new(FakeGateway::default()));
    let other = early
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    store
        .set_state(&other, MoveState::Rebuilt, chrono::Utc::now())
        .await
        .unwrap();
    let refused = early
        .accept_pages(&other, &["/about/".to_owned()])
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("still being checked"),
        "{refused}"
    );
    // Running again does not rebuild: the Move waits.
    assert_eq!(mover.run(&move_id).await.unwrap(), MoveState::Rebuilt);
    // Accepting the page does not lift site parity to the bar, so still no candidate.
    let state = mover
        .accept_pages(&move_id, &["/about/".to_owned()])
        .await
        .unwrap();
    assert_eq!(state, MoveState::Rebuilt);
    assert!(store.cards(&world.site).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_sealed_replay_feeds_its_pages_and_crawler_differences_to_the_clean_step() {
    let world = World::new().await;
    let mut rooms = FakeRooms::new(world.blobs.clone());
    rooms.kinds.insert(agentrix_types::JobKind::Replay);
    rooms.replay_pages = vec![
        (
            "/".to_owned(),
            "<html><body><main><h1>Kilele Bakery</h1><p>Fresh bread.</p></main></body></html>".to_owned(),
            "<html><body><main><h1>Kilele Bakery</h1><p>Fresh bread.</p><a href=\"https://cheap-casino.test/\">casino</a></main></body></html>".to_owned(),
        ),
        (
            "/about/".to_owned(),
            "<html><body><main><h1>About</h1></main></body></html>".to_owned(),
            "<html><body><main><h1>About</h1></main></body></html>".to_owned(),
        ),
    ];
    let rooms = Arc::new(rooms);
    let mover = world.mover(rooms.clone(), Arc::new(FakeGateway::default()));
    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    assert_eq!(mover.run(&move_id).await.unwrap(), MoveState::AwaitingOwner);

    let replay = rooms
        .runs
        .lock()
        .unwrap()
        .iter()
        .find(|(j, _)| j.kind == agentrix_types::JobKind::Replay)
        .map(|(j, _)| j.args.clone())
        .expect("a replay job ran");
    assert_eq!(replay["public_origin"], "https://kilele-bakery.example");
    assert_eq!(
        replay["wxr"],
        world.inputs.wxr.unwrap().hex(),
        "bare hex, as room-replay reads it"
    );
    assert_eq!(replay["wp_content"], world.inputs.wp_content.unwrap().hex());

    let store = MoveStore::new(world.pool.clone());
    let replay_step = store
        .steps(&move_id)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.step == StepName::Replay && s.state == StepState::Done)
        .unwrap();
    assert!(
        replay_step.summary.contains("in a sealed replay"),
        "{}",
        replay_step.summary
    );
    let held = store.quarantine(&world.site).await.unwrap();
    assert!(
        held.iter()
            .any(|q| q.title == "Different pages for search engines" && q.location == "/"),
        "{held:#?}"
    );
}

#[tokio::test]
async fn a_failed_render_room_falls_back_to_text_parity_and_says_so() {
    let world = World::new().await;
    let mut rooms = FakeRooms::new(world.blobs.clone());
    rooms.fail_render = true;
    let mover = world.mover(Arc::new(rooms), Arc::new(FakeGateway::default()));
    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    assert_eq!(mover.run(&move_id).await.unwrap(), MoveState::AwaitingOwner);
    let store = MoveStore::new(world.pool.clone());
    let prove = store
        .steps(&move_id)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.step == StepName::Prove && s.state == StepState::Done)
        .unwrap();
    assert!(
        prove.summary.contains("text only, not rendered"),
        "{}",
        prove.summary
    );
}

/// The 50-page set deployed on control (`exports/live`, origin fixture.agentrix.website):
/// what James's Start moves.
#[tokio::test]
async fn the_live_fifty_page_fixture_reaches_a_candidate_with_the_infection_left_behind() {
    let world = World::new().await;
    let site = world.site.clone();
    let inputs = MoveInputs {
        domain: "fixture.agentrix.website".into(),
        origin: "https://fixture.agentrix.website".into(),
        site_name: None,
        wxr: Some(world.blobs.insert(
            &site,
            include_bytes!("../../../fixtures/wordpress-infected/exports/live/export.xml"),
        )),
        dump: None,
        wp_content: Some(world.blobs.insert(
            &site,
            include_bytes!("../../../fixtures/wordpress-infected/exports/live/wp-content.tar.gz"),
        )),
        crawl: None,
        fixture: true,
        look: None,
    };
    // A runner that takes only 32 inputs a room: the pages go in batches.
    let mut rooms = FakeRooms::new(world.blobs.clone());
    rooms.max_inputs = 32;
    let mut mover = world.mover(Arc::new(rooms), Arc::new(FakeGateway::default()));
    mover.policy.room_inputs = 32;
    let started = std::time::Instant::now();
    let move_id = mover.start(&site, &inputs, None, None).await.unwrap();
    let state = mover.run(&move_id).await.unwrap();
    println!("50-page Move with fakes: {:?}", started.elapsed());
    if state != MoveState::AwaitingOwner {
        let store = MoveStore::new(world.pool.clone());
        for s in store.steps(&move_id).await.unwrap() {
            println!("STEP {:?} {:?} {}", s.step, s.state, s.summary);
        }
        let resume = store
            .resume_of(&move_id, StepName::Prove)
            .await
            .unwrap()
            .unwrap();
        let digest = resume["proved"].as_str().unwrap().parse().unwrap();
        let proved: Proved = serde_json::from_slice(
            &agentrix_move::ports::Blobs::get(world.blobs.as_ref(), &site, &digest)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        for p in proved.parity.pages.iter().filter(|p| p.score < 0.95) {
            println!(
                "LOW {} score {:.3} text {:.3} {:?} {}",
                p.path, p.score, p.text, p.state, p.verdict
            );
        }
    }
    assert_eq!(state, MoveState::AwaitingOwner);

    let store = MoveStore::new(world.pool.clone());
    let resume = store
        .resume_of(&move_id, StepName::Prove)
        .await
        .unwrap()
        .unwrap();
    let digest = resume["proved"].as_str().unwrap().parse().unwrap();
    let proved: Proved = serde_json::from_slice(
        &agentrix_move::ports::Blobs::get(world.blobs.as_ref(), &site, &digest)
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        proved.parity.pages.len(),
        50,
        "every page and post is rebuilt"
    );
    assert!(
        proved.parity.rendered_in_room,
        "every batch fits the runner's input limit: {:?}",
        proved.parity.render_note
    );
    assert!(
        proved.parity.render_receipts.len() >= 2,
        "50 pages and the shared assets take more than one room of 32"
    );
    assert!(
        proved.parity.site >= 0.95,
        "site parity {}",
        proved.parity.site
    );
    let candidate = proved.candidate.expect("a candidate release");
    let manifest = load_manifest(world.blobs.as_ref(), &site, &candidate.manifest_digest)
        .await
        .unwrap();
    assert!(manifest.files.contains_key("index.html"));
    for (path, entry) in &manifest.files {
        if !(path.ends_with(".html") || path.ends_with(".md")) {
            continue;
        }
        let digest = entry.sha256.as_str().parse().unwrap();
        let text = String::from_utf8(
            agentrix_move::ports::Blobs::get(world.blobs.as_ref(), &site, &digest)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        for bad in [
            "kilele-support.test",
            "kilele-assets.test",
            "cheap-casino.test",
            "pills-online.test",
            "cdn-stats.test",
            "_0x4f",
            "cache.php",
        ] {
            assert!(!text.contains(bad), "{path} carries {bad}");
        }
    }
    let clean = world
        .receipts
        .of_kind(ReceiptKind::MoveStep)
        .into_iter()
        .find(|r| {
            r.body
                .detail
                .as_ref()
                .and_then(|d| d.get("step"))
                .and_then(|s| s.as_str())
                == Some("clean")
        })
        .unwrap();
    let entries: Vec<QuarantineEntry> =
        serde_json::from_value(clean.body.detail.as_ref().unwrap()["quarantined"].clone()).unwrap();
    let detectors: BTreeSet<String> = entries.iter().filter_map(|e| e.detector.clone()).collect();
    for expected in [
        "hidden-instructions",
        "spam-links",
        "cloaking",
        "executable-upload",
        "obfuscated-script",
    ] {
        assert!(
            detectors.contains(expected),
            "{expected} missing from {detectors:?}"
        );
    }
    let footer = entries
        .iter()
        .filter(|e| e.detector.as_deref() == Some("obfuscated-script"))
        .count();
    assert!(
        footer <= 3,
        "the theme's footer script is one item, not one per page: {footer}"
    );
}

#[tokio::test]
async fn two_runs_of_one_move_never_run_a_step_twice() {
    let world = World::new().await;
    let mover = world.mover(world.rooms(), Arc::new(FakeGateway::default()));
    let move_id = mover
        .start(&world.site, &world.inputs, None, None)
        .await
        .unwrap();
    let (a, b) = tokio::join!(mover.run(&move_id), mover.run(&move_id));
    a.unwrap();
    b.unwrap();
    let store = MoveStore::new(world.pool.clone());
    assert_eq!(
        store.move_record(&move_id).await.unwrap().unwrap().state,
        MoveState::AwaitingOwner
    );
    let steps = store.steps(&move_id).await.unwrap();
    for step in [
        StepName::Replay,
        StepName::Clean,
        StepName::Rebuild,
        StepName::Prove,
    ] {
        assert_eq!(
            steps
                .iter()
                .filter(|s| s.step == step && s.state == StepState::Done)
                .count(),
            1,
            "{step:?}"
        );
    }
    assert_eq!(
        store.cards(&world.site).await.unwrap().len(),
        1,
        "one go-live card"
    );
}
