//! The approve, decline, restore and rollback routes, and held orders, over a real
//! control.db, CAS, receipts store and site log. Approvals use a fake verifier; the auth lane
//! tests the real passkey and TOTP checks.

use std::sync::Arc;

use agentrix_move::MoveStore;
use agentrix_move::store::{CardState, QuarantineState};
use agentrix_types::{ApprovalMethod, Digest, ReceiptKind};
use awp_types::HeldCallState;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::*;
use crate::launch::ports::MoveEvents;
use crate::launch::tests::World;
use crate::site::release::ReleaseSource as _;

const OWNER: &str = "01930000-0000-7000-8000-000000000001";

struct Approve;

#[async_trait]
impl Approver for Approve {
    async fn options(
        &self,
        _account: &str,
        fingerprint: &Digest,
    ) -> anyhow::Result<Option<String>> {
        Ok(Some(
            json!({ "publicKey": { "challenge": fingerprint.hex() } }).to_string(),
        ))
    }
    async fn verify(
        &self,
        account: &str,
        _fingerprint: &Digest,
        attempt: Attempt,
    ) -> Result<Proof, String> {
        assert_eq!(account, OWNER);
        match attempt {
            Attempt::Passkey(c) if c["ok"] == true => Ok(Proof {
                method: ApprovalMethod::Passkey,
                evidence: json!({ "assertion": c }),
            }),
            Attempt::Totp(code) if code == "123456" => Ok(Proof {
                method: ApprovalMethod::StepUp,
                evidence: json!({ "totp_step": 1 }),
            }),
            _ => Err("That passkey didn't work. Try again, or use your password and code.".into()),
        }
    }
}

struct Header;

#[async_trait]
impl Sessions for Header {
    async fn account(&self, headers: &HeaderMap) -> Option<String> {
        headers
            .get("x-account")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }
}

fn state(world: &World) -> DecisionsState {
    DecisionsState {
        store: MoveStore::new(world.pool.clone()),
        receipts: world.receipts.clone(),
        gate: world.gate.clone(),
        approver: Arc::new(Approve),
        sessions: Arc::new(Header),
        events: Arc::new(MoveEvents::new(world.pool.clone())),
        guards: crate::guards::Guards::default(),
    }
}

async fn post(
    state: &DecisionsState,
    uri: &str,
    account: Option<&str>,
    body: Body,
    json_body: bool,
) -> (StatusCode, String, Value) {
    let mut request = Request::post(uri);
    if let Some(a) = account {
        request = request.header("x-account", a);
    }
    request = request.header(
        header::CONTENT_TYPE,
        if json_body {
            "application/json"
        } else {
            "application/x-www-form-urlencoded"
        },
    );
    let response = router(state.clone())
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        location,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_passkey_approval_takes_the_site_live_and_is_on_the_receipt() {
    let world = World::new().await;
    let s = state(&world);
    let card = world.card().await;
    let uri = format!("/decisions/{}/approve", card.decision_id);
    let passkey = |ok: bool| {
        Body::from(json!({ "method": "passkey", "credential": { "ok": ok } }).to_string())
    };

    let (status, _, _) = post(&s, &uri, None, passkey(true), true).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = post(
        &s,
        &uri,
        Some("01930000-0000-7000-8000-00000000beef"),
        passkey(true),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) = post(&s, &uri, Some(OWNER), passkey(false), true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["message"].as_str().unwrap().contains("passkey"));
    assert_eq!(
        s.store
            .card(&card.decision_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        CardState::Waiting
    );

    let (status, _, body) = post(&s, &uri, Some(OWNER), passkey(true), true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["redirect"], "/console/move");
    assert!(
        s.gate
            .releases()
            .live("kilele-bakery.example")
            .await
            .unwrap()
            .is_some()
    );
    let approved = world
        .receipts
        .list(&world.site, Some(ReceiptKind::DecisionApproved), 5)
        .await
        .unwrap();
    assert_eq!(approved.len(), 1);
    let detail = approved[0].receipt.body.detail.clone().unwrap();
    assert_eq!(detail["method"], "passkey");
    assert_eq!(detail["fingerprint"], card.fingerprint.to_string());
    assert_eq!(
        world
            .receipts
            .list(&world.site, Some(ReceiptKind::ReleaseSigned), 5)
            .await
            .unwrap()
            .len(),
        1
    );

    let (status, _, _) = post(&s, &uri, Some(OWNER), passkey(true), true).await;
    assert_eq!(status, StatusCode::CONFLICT, "a card is approved once");

    let options = s
        .begin_approval(
            &OwnerSession {
                account: OWNER.into(),
                name: "Wanjiru".into(),
                site: world.site.clone(),
                has_passkey: true,
            },
            &card,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(options.contains(&card.fingerprint.hex()));

    // Undo in one step (R7.4).
    let (status, location, _) = post(
        &s,
        &format!("/sites/{}/rollback", world.site),
        Some(OWNER),
        Body::from("to=old-site"),
        false,
    )
    .await;
    assert_eq!(
        (status, location.as_str()),
        (StatusCode::SEE_OTHER, "/console/move")
    );
    assert!(
        s.gate
            .releases()
            .live("kilele-bakery.example")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_quarantine_restore_is_a_signed_decision_and_code_never_comes_back() {
    let world = World::new().await;
    let s = state(&world);
    world.card().await;
    let held = s.store.quarantine(&world.site).await.unwrap();
    let webshell = held
        .iter()
        .find(|q| q.location.ends_with("cache.php"))
        .unwrap();
    let spam = held
        .iter()
        .find(|q| q.title == "Spam links" || q.title == "Hidden links to other sites")
        .unwrap();

    let restore = |id: &QuarantineId| format!("/sites/{}/quarantine/{id}/restore", world.site);
    let (status, _, body) = post(
        &s,
        &restore(&webshell.id),
        Some(OWNER),
        Body::from("reason=it+is+my+cache"),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["message"].as_str().unwrap().contains("never"));
    let (status, _, _) = post(
        &s,
        &restore(&spam.id),
        Some(OWNER),
        Body::from("reason="),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a reason is required");

    let (status, location, _) = post(
        &s,
        &restore(&spam.id),
        Some(OWNER),
        Body::from("reason=That+is+our+partner+bakery"),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let id = location
        .strip_prefix(&format!("/console/{}/decisions/", world.site))
        .unwrap();
    let card = s.store.card(&DecisionId::new(id)).await.unwrap().unwrap();
    assert_eq!(
        (card.card.kind, card.state),
        (CardKind::QuarantineRestore, CardState::Waiting)
    );
    assert!(card.card.body.contains("partner bakery"));
    assert_eq!(
        s.store
            .quarantine_item(&world.site, &spam.id)
            .await
            .unwrap()
            .unwrap()
            .decision
            .as_ref()
            .map(|d| d.as_str()),
        Some(id)
    );

    let form = format!(
        "method=totp&code=123456&fingerprint={}",
        card.card.fingerprint.hex()
    );
    let (status, location, _) = post(
        &s,
        &format!("/decisions/{id}/approve"),
        Some(OWNER),
        Body::from(form),
        false,
    )
    .await;
    assert_eq!(
        (status, location),
        (
            StatusCode::SEE_OTHER,
            format!("/console/{}/quarantine", world.site)
        )
    );
    assert_eq!(
        s.store
            .quarantine_item(&world.site, &spam.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        QuarantineState::Restored
    );
    let restored = world
        .receipts
        .list(&world.site, Some(ReceiptKind::QuarantineRestored), 5)
        .await
        .unwrap();
    assert_eq!(restored.len(), 1);
    let approved = world
        .receipts
        .list(&world.site, Some(ReceiptKind::DecisionApproved), 5)
        .await
        .unwrap();
    assert_eq!(
        approved[0].receipt.body.detail.as_ref().unwrap()["method"],
        "totp"
    );
}

#[tokio::test]
async fn declining_go_live_stops_the_move_and_changes_nothing() {
    let world = World::new().await;
    let s = state(&world);
    let card = world.card().await;
    let (status, location, _) = post(
        &s,
        &format!("/decisions/{}/decline", card.decision_id),
        Some(OWNER),
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(
        (status, location),
        (
            StatusCode::SEE_OTHER,
            format!("/console/{}/decisions", world.site)
        )
    );
    assert_eq!(
        s.store
            .card(&card.decision_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        CardState::Declined
    );
    assert!(
        s.gate
            .releases()
            .live("kilele-bakery.example")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        s.store
            .latest_move(&world.site)
            .await
            .unwrap()
            .unwrap()
            .state,
        agentrix_move::MoveState::Stopped
    );
    assert_eq!(
        world
            .receipts
            .list(&world.site, Some(ReceiptKind::DecisionDeclined), 5)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn held_orders_are_idempotent_and_complete_when_the_owner_approves() {
    let world = World::new().await;
    let s = state(&world);
    let orders = HeldOrders {
        store: s.store.clone(),
    };
    let input = json!({ "description": "A two-tier birthday cake", "date": "2026-12-01", "name": "Amina", "email": "amina@example.com" });
    let site = world.site.as_str();
    let first = orders
        .raise(site, "request_custom_order", input.clone(), "key-1")
        .await
        .unwrap();
    let again = orders
        .raise(site, "request_custom_order", input.clone(), "key-1")
        .await
        .unwrap();
    assert_eq!(first.id, again.id, "same key, same call, one card");
    assert_eq!(first.state, HeldCallState::Pending);
    assert_eq!(first.status, format!("/awp/v2/calls/{}", first.id));
    assert_eq!(s.store.cards(&world.site).await.unwrap().len(), 1);
    assert!(
        orders
            .status("site_other", &first.id)
            .await
            .unwrap()
            .is_none()
    );

    let card = s
        .store
        .card(&DecisionId::new(first.id.clone()))
        .await
        .unwrap()
        .unwrap()
        .card;
    let form = format!("method=totp&code=123456&fingerprint={}", card.fingerprint);
    let (status, _, _) = post(
        &s,
        &format!("/decisions/{}/approve", first.id),
        Some(OWNER),
        Body::from(form),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let done = orders.status(site, &first.id).await.unwrap().unwrap();
    assert_eq!(done.state, HeldCallState::Completed);
    assert!(
        done.result.unwrap()["reference"]["id"]
            .as_str()
            .unwrap()
            .starts_with("K-")
    );

    let other = orders
        .raise(site, "request_custom_order", input, "key-2")
        .await
        .unwrap();
    let (status, _, _) = post(
        &s,
        &format!("/decisions/{}/decline", other.id),
        Some(OWNER),
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        orders.status(site, &other.id).await.unwrap().unwrap().state,
        HeldCallState::Rejected
    );
    // Personal details stay on the card, never on a receipt.
    for stored in world.receipts.list(&world.site, None, 50).await.unwrap() {
        let text = serde_json::to_string(&stored.receipt).unwrap();
        assert!(!text.contains("amina@example.com"), "{text}");
    }
}

#[tokio::test]
async fn the_owner_starts_a_move_from_the_fixture_exports_and_it_reaches_the_card() {
    let world = World::new().await;
    let blobs = crate::launch::ports::NodeBlobs {
        pool: world.pool.clone(),
        cas: world.cas.clone(),
    };
    let moves = moves::MovesState {
        mover: world.mover.clone(),
        blobs,
        sessions: Arc::new(Header),
        fixture_exports: Some(std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/wordpress-infected/exports/dev"
        ))),
        guards: crate::guards::Guards::default(),
    };
    let send =
        |uri: String, account: Option<&'static str>, method: &'static str, body: &'static [u8]| {
            let moves = moves.clone();
            async move {
                let mut request = Request::builder().method(method).uri(uri);
                if let Some(a) = account {
                    request = request.header("x-account", a);
                }
                let response = moves::router(moves)
                    .oneshot(request.body(Body::from(body)).unwrap())
                    .await
                    .unwrap();
                response.status()
            }
        };
    let site = world.site.clone();
    assert_eq!(
        send(format!("/sites/{site}/moves"), None, "POST", b"").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            format!("/sites/{site}/uploads/evil.php"),
            Some(OWNER),
            "PUT",
            b"<?php"
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(format!("/sites/{site}/moves"), Some(OWNER), "POST", b"").await,
        StatusCode::SEE_OTHER
    );

    let store = MoveStore::new(world.pool.clone());
    let mut state = None;
    for _ in 0..100 {
        state = store
            .latest_move(&world.site)
            .await
            .unwrap()
            .map(|m| m.state);
        if state == Some(agentrix_move::MoveState::AwaitingOwner) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(state, Some(agentrix_move::MoveState::AwaitingOwner));
    // A second start while a card waits starts a new Move only after this one ends.
    assert_eq!(
        send(format!("/sites/{site}/moves"), Some(OWNER), "POST", b"").await,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn the_move_page_streams_a_snapshot_then_replays_from_last_event_id() {
    use futures::StreamExt as _;
    let world = World::new().await;
    let card = world.card().await;
    let move_id = agentrix_move::golive::GoLiveArgs::of(&card)
        .unwrap()
        .move_id;
    let events = Arc::new(MoveEvents::new(world.pool.clone()));
    let watch = watch::router(watch::WatchState {
        store: MoveStore::new(world.pool.clone()),
        events: events.clone(),
        sessions: Arc::new(Header),
    });
    let get = |uri: String,
               account: Option<&'static str>,
               last: Option<&'static str>,
               json_accept: bool| {
        let watch = watch.clone();
        async move {
            let mut request = Request::get(uri);
            if let Some(a) = account {
                request = request.header("x-account", a);
            }
            if let Some(l) = last {
                request = request.header("last-event-id", l);
            }
            if json_accept {
                request = request.header(header::ACCEPT, "application/json");
            }
            watch
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    let uri = format!("/moves/{move_id}/events");
    assert_eq!(
        get(uri.clone(), None, None, false).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(
            uri.clone(),
            Some("01930000-0000-7000-8000-00000000beef"),
            None,
            false
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    // A fresh page gets a snapshot first, and the stream stays open.
    let fresh = get(uri.clone(), Some(OWNER), None, false).await;
    assert_eq!(fresh.status(), StatusCode::OK);
    let mut body = fresh.into_body().into_data_stream();
    let first = String::from_utf8(body.next().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(first.contains("event: snapshot"), "{first}");
    assert!(first.contains("\"golive\""), "{first}");

    // Reconnecting with Last-Event-ID 0 replays everything, in order, ending at `done`.
    let replay = get(uri, Some(OWNER), Some("0"), false).await;
    let bytes = axum::body::to_bytes(replay.into_body(), 1 << 22)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    for name in [
        "event: step",
        "event: quarantined",
        "event: parity",
        "event: card",
        "event: done",
    ] {
        assert!(text.contains(name), "{name} missing");
    }
    assert!(text.starts_with("id: 1\n"), "{}", &text[..80]);
    assert!(text.trim_end().ends_with('}') && text.contains("candidate-ready"));

    // GET /moves/{id}: JSON for the island, a redirect for a browser.
    let json_reply = get(format!("/moves/{move_id}"), Some(OWNER), None, true).await;
    assert_eq!(json_reply.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(
        &axum::body::to_bytes(json_reply.into_body(), 1 << 20)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v["state"], "awaiting-owner");
    assert_eq!(v["card"]["decision_id"], card.decision_id.to_string());
    let browser = get(format!("/moves/{move_id}"), Some(OWNER), None, false).await;
    assert_eq!(browser.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn accepting_pages_needs_the_owner_and_a_move_waiting_on_them() {
    let world = World::new().await;
    let card = world.card().await;
    let move_id = agentrix_move::golive::GoLiveArgs::of(&card)
        .unwrap()
        .move_id;
    let moves = moves::MovesState {
        mover: world.mover.clone(),
        blobs: crate::launch::ports::NodeBlobs {
            pool: world.pool.clone(),
            cas: world.cas.clone(),
        },
        sessions: Arc::new(Header),
        fixture_exports: None,
        guards: crate::guards::Guards::default(),
    };
    let send = |account: &'static str, body: &'static str| {
        let moves = moves.clone();
        let uri = format!("/moves/{move_id}/accept");
        async move {
            let request = Request::post(uri)
                .header("x-account", account)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap();
            moves::router(moves)
                .oneshot(request)
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(
        send("01930000-0000-7000-8000-00000000beef", "page=%2Fabout%2F").await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(send(OWNER, "").await, StatusCode::BAD_REQUEST);
    assert_eq!(
        send(OWNER, "page=%2Fabout%2F").await,
        StatusCode::CONFLICT,
        "this Move waits on its card, not on pages"
    );
}

#[tokio::test]
async fn pressing_start_again_resumes_a_move_that_stopped_on_a_fault() {
    let world = World::new().await;
    let inputs = world.inputs().await;
    // A Move that stopped at Clean because the gateway was down.
    let mut broken = world.mover.clone();
    broken.gateway = Arc::new(agentrix_move::testing::FakeGateway {
        broken_screen: true,
        ..Default::default()
    });
    let id = broken
        .start(&world.site, &inputs, None, None)
        .await
        .unwrap();
    assert!(broken.run(&id).await.is_err());
    let store = MoveStore::new(world.pool.clone());
    assert_eq!(
        store.move_record(&id).await.unwrap().unwrap().state,
        agentrix_move::MoveState::Replayed
    );

    let moves = moves::MovesState {
        mover: world.mover.clone(),
        blobs: crate::launch::ports::NodeBlobs {
            pool: world.pool.clone(),
            cas: world.cas.clone(),
        },
        sessions: Arc::new(Header),
        fixture_exports: None,
        guards: crate::guards::Guards::default(),
    };
    let request = Request::post(format!("/sites/{}/moves", world.site))
        .header("x-account", OWNER)
        .body(Body::empty())
        .unwrap();
    let status = moves::router(moves)
        .oneshot(request)
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::SEE_OTHER);
    let mut state = None;
    for _ in 0..100 {
        state = store.move_record(&id).await.unwrap().map(|m| m.state);
        if state == Some(agentrix_move::MoveState::AwaitingOwner) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        state,
        Some(agentrix_move::MoveState::AwaitingOwner),
        "the same Move resumed; no new one"
    );
    let replays = store
        .steps(&id)
        .await
        .unwrap()
        .into_iter()
        .filter(|s| {
            s.step == agentrix_types::ui::StepName::Replay
                && s.state == agentrix_types::ui::StepState::Done
        })
        .count();
    assert_eq!(replays, 1, "replay is not run again");
}

#[tokio::test]
async fn a_held_order_lights_the_agent_desk_until_the_owner_decides() {
    let world = World::new().await;
    let s = state(&world);
    let orders = HeldOrders {
        store: s.store.clone(),
    };
    let site = world.site.as_str();
    let mark = |who: &str| {
        agentrix_kernel::presence::global()
            .current(site, who)
            .map(|f| f.state.key())
    };
    let input = json!({ "description": "Two dozen croissants", "date": "2026-12-01", "name": "Amina", "email": "amina@example.com" });
    let held = orders
        .raise(site, "request_custom_order", input, "key-presence")
        .await
        .unwrap();
    assert_eq!(mark("agent-desk"), Some("waiting"));
    assert_eq!(
        mark("coordinator"),
        Some("waiting"),
        "the Chief shows it too"
    );
    let (status, _, _) = post(
        &s,
        &format!("/decisions/{}/decline", held.id),
        Some(OWNER),
        Body::empty(),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(mark("agent-desk"), Some("idle"));
    assert_eq!(mark("coordinator"), Some("asleep"));
}

#[tokio::test]
async fn while_frozen_nothing_goes_live_and_no_move_starts() {
    let world = World::new().await;
    let card = world.card().await;
    let mut s = state(&world);
    s.guards = crate::guards::testing::frozen(&world.pool, world.site.as_str()).await;
    let passkey =
        Body::from(json!({ "method": "passkey", "credential": { "ok": true } }).to_string());
    let (status, _, body) = post(
        &s,
        &format!("/decisions/{}/approve", card.decision_id),
        Some(OWNER),
        passkey,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["message"], FROZEN_GO_LIVE);
    assert_eq!(
        s.store
            .card(&card.decision_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        CardState::Waiting,
        "the card keeps waiting"
    );
    assert!(
        s.gate
            .releases()
            .live("kilele-bakery.example")
            .await
            .unwrap()
            .is_none()
    );

    let moves = moves::MovesState {
        mover: world.mover.clone(),
        blobs: crate::launch::ports::NodeBlobs {
            pool: world.pool.clone(),
            cas: world.cas.clone(),
        },
        sessions: Arc::new(Header),
        fixture_exports: None,
        guards: s.guards.clone(),
    };
    let before = s.store.latest_move(&world.site).await.unwrap().unwrap().id;
    let response = moves::router(moves)
        .oneshot(
            Request::post(format!("/sites/{}/moves", world.site))
                .header("x-account", OWNER)
                .header(header::ACCEPT, "application/json")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains("frozen"),
        "{bytes:?}"
    );
    assert_eq!(
        s.store.latest_move(&world.site).await.unwrap().unwrap().id,
        before,
        "no new Move"
    );
}
