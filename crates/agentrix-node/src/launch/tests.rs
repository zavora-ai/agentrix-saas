//! S2-01: approve → live, verified the way the edge verifies; rollback restores the previous
//! release; stale and tampered releases are refused. Real control.db, CAS, receipts store and
//! site log; fake rooms and gateway (no model calls).

use std::sync::Arc;
use std::time::Duration;

use agentrix_move::store::{CardState, ReleaseState};
use agentrix_move::testing::{FakeGateway, FakeRooms};
use agentrix_move::{MoveInputs, MoveState, MoveStore, Mover};
use agentrix_types::{DecisionCard, Digest, ReceiptBody, ReceiptKind, Signer, SiteId, Subject};
use chrono::Utc;

use super::ports::{MoveEvents, NodeBlobs, NodeReceipts};
use super::*;
use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;

const EXPORT: &[u8] =
    include_bytes!("../../../../fixtures/wordpress-infected/exports/dev/export.xml");
const WP_CONTENT: &[u8] =
    include_bytes!("../../../../fixtures/wordpress-infected/exports/dev/wp-content.tar.gz");
const HOST: &str = "kilele-bakery.example";
const OWNER: &str = "01930000-0000-7000-8000-000000000001";

pub(crate) struct World {
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) cas: Arc<Cas>,
    pub(crate) receipts: Arc<ReceiptStore>,
    pub(crate) site: SiteId,
    pub(crate) mover: Mover,
    pub(crate) gate: LaunchGate,
    _dir: tempfile::TempDir,
}

impl World {
    pub(crate) async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open_in_memory().await.unwrap();
        let cas = Arc::new(Cas::open(dir.path().join("cas")).await.unwrap());
        let receipts = Arc::new(ReceiptStore::new(
            pool.clone(),
            Arc::new(Signer::generate()),
        ));
        let site =
            crate::db::sites::create_or_get(&pool, HOST, "https://kilele-bakery.example", true)
                .await
                .unwrap();
        sqlx::query("INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, 'wanjiru@kilele.example', 'Wanjiru', 0)")
            .bind(OWNER)
            .execute(&pool)
            .await
            .unwrap();
        crate::db::sites::set_owner(&pool, &site, OWNER)
            .await
            .unwrap();
        let blobs = NodeBlobs {
            pool: pool.clone(),
            cas: cas.clone(),
        };
        let events = Arc::new(MoveEvents::new(pool.clone()));
        let mover = Mover {
            store: MoveStore::new(pool.clone()),
            blobs: Arc::new(blobs.clone()),
            receipts: Arc::new(NodeReceipts(receipts.clone())),
            events: events.clone(),
            rooms: Arc::new(FakeRooms::new(Arc::new(blobs))),
            gateway: Arc::new(FakeGateway::default()),
            policy: Default::default(),
        };
        let gate = LaunchGate {
            store: MoveStore::new(pool.clone()),
            cas: cas.clone(),
            receipts: receipts.clone(),
            events,
            logs: Arc::new(SiteLogs::new(ReleaseKeys::new([9; 32]))),
            probes: (0, Duration::ZERO),
            after_live: None,
        };
        Self {
            pool,
            cas,
            receipts,
            site,
            mover,
            gate,
            _dir: dir,
        }
    }

    /// Runs a Move to its go-live card and returns the card.
    pub(crate) async fn card(&self) -> DecisionCard {
        let inputs = self.inputs().await;
        let id = self
            .mover
            .start(&self.site, &inputs, None, None)
            .await
            .unwrap();
        assert_eq!(self.mover.run(&id).await.unwrap(), MoveState::AwaitingOwner);
        let store = MoveStore::new(self.pool.clone());
        store
            .cards(&self.site)
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.state == CardState::Waiting)
            .unwrap()
            .card
    }

    /// The fixture's export and wp-content, stored for the site.
    pub(crate) async fn inputs(&self) -> MoveInputs {
        let put = |bytes: &'static [u8]| {
            let blobs = NodeBlobs {
                pool: self.pool.clone(),
                cas: self.cas.clone(),
            };
            let site = self.site.clone();
            async move {
                agentrix_move::ports::Blobs::put(
                    &blobs,
                    &site,
                    "upload",
                    "application/octet-stream",
                    bytes,
                )
                .await
                .unwrap()
            }
        };
        MoveInputs {
            domain: HOST.into(),
            origin: "https://kilele-bakery.example".into(),
            site_name: Some("Kilele Bakery".into()),
            wxr: Some(put(EXPORT).await),
            dump: None,
            wp_content: Some(put(WP_CONTENT).await),
            crawl: None,
            fixture: true,
            look: None,
        }
    }

    /// Records the owner's approval as decisions/ does, and returns its receipt.
    pub(crate) async fn approve(&self, card: &DecisionCard) -> Digest {
        let now = Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionApproved,
            self.site.clone(),
            Subject::Decision(card.decision_id.clone()),
            now,
            now,
        );
        body.detail =
            Some(serde_json::json!({ "fingerprint": card.fingerprint, "method": "passkey" }));
        let receipt = self.receipts.issue(body).await.unwrap().digest;
        let store = MoveStore::new(self.pool.clone());
        assert!(
            store
                .approve(
                    &card.decision_id,
                    "passkey",
                    OWNER,
                    &serde_json::json!({}),
                    &receipt,
                    now
                )
                .await
                .unwrap()
        );
        receipt
    }
}

#[tokio::test]
async fn approve_goes_live_as_the_edge_verifies_and_rollback_restores_the_previous_release() {
    let world = World::new().await;
    let store = MoveStore::new(world.pool.clone());

    // Release 1.
    let card1 = world.card().await;
    let receipt1 = world.approve(&card1).await;
    let live1 = world.gate.go_live(&card1, &receipt1, OWNER).await.unwrap();
    assert_eq!((live1.seq, live1.host.as_str()), (1, HOST));
    assert!(
        live1.took_ms < 60_000,
        "live within 60 s (R7.3, N4): {} ms",
        live1.took_ms
    );
    world.gate.probe(HOST, 1).await.unwrap();

    let edge = ReleaseCache::new(Arc::new(world.gate.releases()));
    let served = edge.current(HOST).await.unwrap().unwrap();
    assert_eq!(served.pointer.seq, 1);
    let (_, entry) = served.resolve("/about/").unwrap();
    let about = String::from_utf8(edge.file(entry).await.unwrap().unwrap().to_vec()).unwrap();
    assert!(about.contains("Kilele"), "{about}");
    assert!(!about.contains("kilele-support.test"));

    let signed = store.release(&world.site, 1).await.unwrap().unwrap();
    assert_eq!(signed.state, ReleaseState::Live);
    assert!(
        signed
            .checkpoint
            .as_deref()
            .unwrap()
            .contains("kilele-bakery.example/.well-known/awp/log")
    );
    let receipt = world.receipts.get(&live1.receipt).await.unwrap().unwrap();
    let body = receipt.verify_with(world.receipts.keys()).unwrap();
    assert_eq!(body.kind, ReceiptKind::ReleaseSigned);
    assert_eq!(body.inputs, vec![live1.manifest, receipt1]);
    assert_eq!(
        body.detail.as_ref().unwrap()["fingerprint"],
        card1.fingerprint.to_string()
    );
    let (_, _, signer) = store.charter(&world.site, 1).await.unwrap().unwrap();
    assert_eq!(
        signer.as_deref(),
        Some(OWNER),
        "charter v1 signed on the same card (R7.5)"
    );
    let mv = store.latest_move(&world.site).await.unwrap().unwrap();
    assert_eq!(mv.state, MoveState::Live);

    // Release 2, then undo back to release 1 in one step (R7.4).
    let card2 = world.card().await;
    let receipt2 = world.approve(&card2).await;
    let live2 = world.gate.go_live(&card2, &receipt2, OWNER).await.unwrap();
    assert_eq!(live2.seq, 2);
    assert_eq!(edge.current(HOST).await.unwrap().unwrap().pointer.seq, 2);

    let undo = world
        .gate
        .roll_back(&world.site, RollbackTo::Previous, OWNER)
        .await
        .unwrap();
    assert_eq!(undo.to, Some(1));
    let fresh = ReleaseCache::new(Arc::new(world.gate.releases()));
    let back = fresh.current(HOST).await.unwrap().unwrap();
    assert_eq!(
        back.pointer.seq, 1,
        "release 1 verifies under the newer checkpoint"
    );
    assert_eq!(
        store.release(&world.site, 2).await.unwrap().unwrap().state,
        ReleaseState::RolledBack
    );
    let rollback = world.receipts.get(&undo.receipt).await.unwrap().unwrap();
    assert_eq!(rollback.body.kind, ReceiptKind::ReleaseRollback);
    assert_eq!(rollback.body.outputs, vec![live1.manifest]);

    // A card for an old candidate can't be replayed.
    assert!(world.gate.go_live(&card2, &receipt2, OWNER).await.is_err());

    // Back to the old site: the edge serves nothing for the host.
    let off = world
        .gate
        .roll_back(&world.site, RollbackTo::OldSite, OWNER)
        .await
        .unwrap();
    assert_eq!(off.to, None);
    assert!(world.gate.releases().live(HOST).await.unwrap().is_none());
}

#[tokio::test]
async fn a_tampered_release_fails_its_probe_and_the_pointer_goes_straight_back() {
    let world = World::new().await;
    let card = world.card().await;
    let args = agentrix_move::golive::GoLiveArgs::of(&card).unwrap();
    let manifest = agentrix_move::candidate::load_manifest(
        &CasBlobs(world.cas.clone()),
        &world.site,
        &args.release,
    )
    .await
    .unwrap();
    let index = manifest.files.get("index.html").unwrap();
    let digest: Digest = index.sha256.as_str().parse().unwrap();
    std::fs::write(world.cas.path_for(&digest), b"<h1>defaced</h1>").unwrap();

    let receipt = world.approve(&card).await;
    let err = world
        .gate
        .go_live(&card, &receipt, OWNER)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("switched your site straight back"), "{err}");
    assert!(
        MoveStore::new(world.pool.clone())
            .pointer(&world.site)
            .await
            .unwrap()
            .is_none()
    );
    let rollbacks = world
        .receipts
        .list(&world.site, Some(ReceiptKind::ReleaseRollback), 10)
        .await
        .unwrap();
    assert_eq!(rollbacks.len(), 1);
}

#[tokio::test]
async fn a_changed_card_or_charter_is_refused() {
    let world = World::new().await;
    let mut card = world.card().await;
    let receipt = world.approve(&card).await;
    card.effect.args["seq"] = serde_json::json!(7);
    let err = world
        .gate
        .go_live(&card, &receipt, OWNER)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("changed"), "{err}");
    assert!(world.gate.releases().live(HOST).await.unwrap().is_none());
}

#[tokio::test]
async fn a_live_site_is_also_served_at_its_platform_address_and_nowhere_else() {
    let world = World::new().await;
    let card = world.card().await;
    let receipt = world.approve(&card).await;
    world.gate.go_live(&card, &receipt, OWNER).await.unwrap();
    let releases = world
        .gate
        .releases()
        .with_platform_host(Some("agentrix.website"));
    assert_eq!(site_alias(HOST), "kilele-bakery-example");
    let aliased = releases
        .live("kilele-bakery-example.agentrix.website")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(aliased.domain, HOST, "the checkpoint stays the site's own");
    let edge = ReleaseCache::new(Arc::new(releases.clone()));
    assert!(
        edge.current("kilele-bakery-example.agentrix.website")
            .await
            .unwrap()
            .is_some()
    );
    for other in [
        "kilele-bakery-example.evil.example",
        "x.kilele-bakery-example.agentrix.website",
        "agentrix.website",
        "nope.agentrix.website",
    ] {
        assert!(releases.live(other).await.unwrap().is_none(), "{other}");
    }
    assert_eq!(
        public_address(HOST, Some("agentrix.website")),
        "https://kilele-bakery-example.agentrix.website/"
    );
}

/// A runner whose first call fails in transit, as a stale pooled connection does.
struct FlakyRunner {
    inner: crate::broker::fake::FakeRunner,
    failed: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl crate::broker::RunnerApi for FlakyRunner {
    async fn put_blob(
        &self,
        digest: &Digest,
        bytes: Bytes,
    ) -> Result<(), crate::broker::BrokerError> {
        if !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::broker::BrokerError::Unreachable(
                "connection closed before message completed".into(),
            ));
        }
        self.inner.put_blob(digest, bytes).await
    }
    async fn post_job(
        &self,
        signed: &agentrix_types::SignedJobSpec,
    ) -> Result<crate::broker::EventStream, crate::broker::BrokerError> {
        self.inner.post_job(signed).await
    }
    async fn get_artifact(&self, digest: &Digest) -> Result<Bytes, crate::broker::BrokerError> {
        self.inner.get_artifact(digest).await
    }
}

#[tokio::test]
async fn a_room_whose_first_call_fails_in_transit_is_tried_once_more() {
    use agentrix_move::ports::Rooms as _;
    let world = World::new().await;
    let signer = Arc::new(Signer::from_secret_bytes(&[3; 32]));
    let room: crate::broker::fake::Room = Arc::new(|_, _| crate::broker::fake::RoomRun {
        events: vec![],
        frames: vec![],
        out: Default::default(),
        exit_code: 0,
    });
    let runner = FlakyRunner {
        inner: crate::broker::fake::FakeRunner::new(signer.verifying_key(), room),
        failed: std::sync::atomic::AtomicBool::new(false),
    };
    let image = |name: &str| {
        agentrix_types::ImageRef::parse(&format!("{name}@{}", Digest::of(name.as_bytes()))).unwrap()
    };
    let broker = crate::broker::Broker::new(
        Arc::new(runner),
        signer.clone(),
        Arc::new(crate::broker::memory::MemoryReceipts::new(signer)),
        Arc::new(crate::broker::memory::MemoryBlobs::default()),
        crate::broker::Images {
            fetch: Some(image("room-fetch")),
            replay: Some(image("room-replay")),
            render: Some(image("room-render")),
            build: Some(image("room-build")),
        },
    );
    let blobs = NodeBlobs {
        pool: world.pool.clone(),
        cas: world.cas.clone(),
    };
    let input = agentrix_move::ports::Blobs::put(&blobs, &world.site, "upload", "text/xml", EXPORT)
        .await
        .unwrap();
    let rooms = super::ports::BrokerRooms { broker, blobs };
    let run = rooms
        .run(
            &world.site,
            &agentrix_types::MoveId::new("move_flaky"),
            agentrix_move::ports::RoomJob {
                kind: agentrix_types::JobKind::Replay,
                inputs: vec![input],
                args: serde_json::json!({}),
            },
        )
        .await
        .expect("the second try reaches the runner");
    assert!(run.succeeded());
}

/// Each site handed over and its pages.
type Handed = Arc<std::sync::Mutex<Vec<(SiteId, Vec<agentrix_media::fill::Page>)>>>;

/// What go-live hands over, recorded once the test lets it through.
struct Recorder {
    release: Arc<tokio::sync::Notify>,
    seen: Handed,
}

#[async_trait::async_trait]
impl super::after_live::AfterLive for Recorder {
    async fn went_live(
        &self,
        site: &SiteId,
        _at: chrono::DateTime<Utc>,
        pages: Vec<agentrix_media::fill::Page>,
    ) {
        self.release.notified().await;
        self.seen.lock().unwrap().push((site.clone(), pages));
    }
}

#[tokio::test]
async fn go_live_hands_the_new_pages_to_after_live_without_waiting_for_it() {
    let mut world = World::new().await;
    let release = Arc::new(tokio::sync::Notify::new());
    let seen: Handed = Arc::default();
    world.gate.after_live = Some(Arc::new(Recorder {
        release: release.clone(),
        seen: seen.clone(),
    }));
    let card = world.card().await;
    let receipt = world.approve(&card).await;
    let live = world.gate.go_live(&card, &receipt, OWNER).await.unwrap();
    assert_eq!(live.seq, 1);
    assert!(
        seen.lock().unwrap().is_empty(),
        "go-live returned while the after-live work still waited"
    );
    release.notify_one();
    let mut handed = None;
    for _ in 0..500 {
        if let Some(first) = seen.lock().unwrap().first().cloned() {
            handed = Some(first);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (site, pages) = handed.expect("the after-live work ran");
    assert_eq!(site, world.site);
    assert_eq!(pages[0].path, "/", "the home page comes first");
    assert!(pages.iter().all(|p| !p.title.is_empty()), "{pages:?}");
    assert!(pages.iter().any(|p| p.path == "/about/"), "{pages:?}");
}

#[tokio::test]
async fn after_live_on_the_node_records_go_live_day_for_billing() {
    let world = World::new().await;
    let billing = agentrix_billing::Billing::new(world.pool.clone());
    billing
        .start(world.site.as_str(), "essential", "I-TEST", None)
        .await
        .unwrap();
    let ctx = agentrix_host::NodeContext {
        pool: world.pool.clone(),
        // Nothing answers here, so the fill draws nothing and costs nothing.
        models_url: "http://127.0.0.1:9".into(),
        ..agentrix_host::testing::context().await.unwrap()
    };
    let at = Utc::now();
    let page = agentrix_media::fill::Page {
        path: "/".into(),
        title: "Kilele Bakery".into(),
        summary: "Sourdough and cakes.".into(),
        has_picture: false,
    };
    super::after_live::AfterLive::went_live(
        &super::after_live::NodeAfterLive::new(ctx),
        &world.site,
        at,
        vec![page],
    )
    .await;
    let sub = billing
        .for_site(world.site.as_str())
        .await
        .unwrap()
        .unwrap();
    assert!(sub.starts_at.is_some(), "{sub:?}");
    assert_eq!(
        agentrix_media::library::waiting(&world.pool, world.site.as_str(), Some("move"))
            .await
            .unwrap(),
        0,
        "no gateway, no drafts"
    );
}

#[tokio::test]
async fn move_events_move_the_movers_mark() {
    let world = World::new().await;
    let events = MoveEvents::new(world.pool.clone());
    let move_id = agentrix_types::MoveId::new("move_presence");
    let mark = || {
        agentrix_kernel::presence::global()
            .current(world.site.as_str(), "mover")
            .unwrap()
    };
    events
        .emit(
            &world.site,
            &move_id,
            UiEvent::Step(StepEvent::new(
                StepName::Replay,
                StepState::Running,
                0,
                "Replaying your WordPress",
                Utc::now(),
            )),
        )
        .await
        .unwrap();
    assert_eq!(mark().state.key(), "working");
    assert_eq!(mark().label.as_deref(), Some("Replaying your WordPress"));
    events
        .emit(
            &world.site,
            &move_id,
            UiEvent::Step(StepEvent::new(
                StepName::GoLive,
                StepState::Waiting,
                0,
                "Waiting for you",
                Utc::now(),
            )),
        )
        .await
        .unwrap();
    assert_eq!(mark().state.key(), "waiting");
}
