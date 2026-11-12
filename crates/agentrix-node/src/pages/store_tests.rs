//! [`SqliteReads`] against the real schema (`db::open_in_memory`: control-plane's tables and
//! migration 0001), with rows written the way each lane writes them, then the pages over it.
//! Set `AGENTRIX_PREVIEW_DIR` to also write each page there.

use std::sync::Arc;

use agentrix_console::Console;
use agentrix_types::ui::{
    CardEvent, FrameEvent, FrameKind, ParityEvent, ParityState, QuarantinedEvent, StepEvent,
    StepName, StepState, TickEvent, UiEvent,
};
use agentrix_types::{
    ApprovalMethod, Brief, BriefLine, CardKind, Charter, DecisionCard, DecisionId, Digest,
    EffectRequest, GauntletAttack, JobLevel, KeySet, Level, QuarantineId, Receipt, ReceiptBody,
    ReceiptKind, Signer, SiteId, Subject,
};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::SqlitePool;
use tower::ServiceExt;

use super::*;
use crate::db;

fn k1() -> SiteId {
    SiteId::new("site_k1")
}

fn t(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 7, 6, 0, 0).unwrap() + Duration::minutes(minutes)
}

fn at(minutes: i64) -> String {
    db::time(t(minutes))
}

/// What the database can't answer: a signed-in owner of site_k1 without a passkey.
struct Outside;

#[async_trait]
impl Delegates for Outside {
    async fn session(&self, _: &HeaderMap) -> ReadResult<Option<OwnerSession>> {
        Ok(Some(OwnerSession {
            account: "acct_1".into(),
            name: "Wanjiru".into(),
            site: k1(),
            has_passkey: false,
        }))
    }
    async fn begin_approval(
        &self,
        _: &OwnerSession,
        _: &DecisionCard,
    ) -> ReadResult<Option<String>> {
        Ok(None)
    }
    async fn frame(&self, _: Option<&SiteId>, _: &Digest) -> ReadResult<Option<FrameBytes>> {
        Ok(None)
    }
}

struct Seeded {
    pool: SqlitePool,
    keys: KeySet,
    receipts: Vec<Digest>,
    go_live: DecisionCard,
}

async fn exec(pool: &SqlitePool, sql: &'static str, binds: &[Option<String>]) {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(b.clone());
    }
    q.execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

fn s(v: &str) -> Option<String> {
    Some(v.to_owned())
}

fn card(id: &str, kind: CardKind, title: &str, minutes: i64) -> DecisionCard {
    let effect = EffectRequest {
        site: k1(),
        webagent: "Mover".into(),
        tool: if kind == CardKind::GoLive {
            "go_live"
        } else {
            "quarantine_restore"
        }
        .into(),
        args: serde_json::json!({ "release": "v1" }),
        amount: None,
        expiry: t(minutes) + Duration::hours(24),
    };
    DecisionCard::new(
        DecisionId::new(id),
        kind,
        title,
        "What happens if you approve.",
        effect,
        t(minutes),
    )
    .expect("card")
}

async fn seed() -> Seeded {
    let pool = db::open_in_memory().await.expect("database");
    exec(
        &pool,
        "INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, ?, ?, 0)",
        &[
            s("acct_1"),
            s("wanjiru@kilele-bakery.example"),
            s("Wanjiru"),
        ],
    )
    .await;
    exec(
        &pool,
        "INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, ?, ?, 0)",
        &[s("acct_2"), s("owner@other.example"), s("Other")],
    )
    .await;
    for (id, domain, name, owner, fixture) in [
        (
            "site_k1",
            "kilele-bakery.example",
            "Kilele Bakery",
            s("acct_1"),
            "1",
        ),
        ("site_free", "free.example", "", None, "0"),
        (
            "site_other",
            "other.example",
            "Other Shop",
            s("acct_2"),
            "0",
        ),
    ] {
        exec(
            &pool,
            "INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, owner_account_id, fixture, created_at) VALUES (?, ?, ?, ?, 180, ?, ?, ?)",
            &[s(id), s(domain), s(name), s(&format!("https://{domain}")), owner, s(fixture), Some(at(0))],
        )
        .await;
    }
    // site_other is live: a signed release and the pointer to it.
    exec(&pool, "INSERT INTO releases (site_id, seq, manifest_digest, checkpoint, state, created_at, signed_at) VALUES ('site_other', 1, ?, 'checkpoint', 'live', ?, ?)", &[Some(Digest::of(b"manifest").to_string()), Some(at(0)), Some(at(5))]).await;
    exec(&pool, "INSERT INTO site_pointer (site_id, seq, manifest_digest, updated_at) VALUES ('site_other', 1, ?, ?)", &[Some(Digest::of(b"manifest").to_string()), Some(at(5))]).await;

    // site_k1's Move waits for the owner; its UI events as the coordinator writes them.
    exec(&pool, "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES ('mv_1', 'site_k1', 'awaiting-owner', ?, ?)", &[Some(at(10)), Some(at(60))]).await;
    let mut events = vec![
        UiEvent::Step(StepEvent::new(
            StepName::Replay,
            StepState::Running,
            0,
            "Reading your old site in a sealed room",
            t(11),
        )),
        UiEvent::Step(StepEvent::new(
            StepName::Replay,
            StepState::Done,
            90_000,
            "Read your old site in a sealed room",
            t(12),
        )),
        UiEvent::Step(StepEvent::new(
            StepName::Claim,
            StepState::Done,
            1_000,
            "You proved the site is yours",
            t(12),
        )),
        UiEvent::ReceiptTick(TickEvent {
            job: None,
            hosts: 1,
            requests: 30,
            bytes: 4096,
            refusals: 2,
            at: t(12),
        }),
        UiEvent::Frame(FrameEvent::new(
            FrameKind::SealedScreenshot,
            "https://kilele-bakery.example/cakes",
            Digest::of(b"frame-k1"),
            390,
            844,
            t(13),
        )),
        UiEvent::Parity(ParityEvent {
            page: Some("/cakes".into()),
            score: 0.7,
            verdict: "The menu moved".into(),
            state: ParityState::Retry,
            attempt: 1,
            at: t(14),
        }),
        UiEvent::Parity(ParityEvent {
            page: Some("/cakes".into()),
            score: 0.98,
            verdict: "Matches".into(),
            state: ParityState::Pass,
            attempt: 2,
            at: t(15),
        }),
        UiEvent::Parity(ParityEvent {
            page: None,
            score: 0.98,
            verdict: "Matches".into(),
            state: ParityState::Pass,
            attempt: 1,
            at: t(15),
        }),
    ];
    let left = QuarantinedEvent {
        item_id: QuarantineId::new("qtn_1"),
        title: "A hidden casino redirect".into(),
        reason: "Sends phone visitors elsewhere".into(),
        location: "footer.php".into(),
        at: t(13),
    };
    events.push(UiEvent::Quarantined(left.clone()));
    events.push(UiEvent::Quarantined(left));
    events.push(UiEvent::Card(CardEvent {
        decision_id: DecisionId::new("dc_1"),
        kind: CardKind::GoLive,
        title: "Go live".into(),
        href: "/console/decisions/dc_1".into(),
        at: t(16),
    }));
    let mut seq = 0;
    for e in &events {
        seq += 1;
        exec(&pool, "INSERT INTO ui_events (stream, seq, site_id, name, data, at) VALUES ('move:mv_1', ?, 'site_k1', ?, ?, ?)", &[Some(seq.to_string()), s(e.name()), Some(e.data_json().unwrap()), Some(at(16))]).await;
    }
    exec(&pool, "INSERT INTO ui_events (stream, seq, site_id, name, data, at) VALUES ('move:mv_1', 99, 'site_k1', 'nonsense', '{}', ?)", &[Some(at(16))]).await;

    // Receipts, through the same columns the receipts store writes.
    let signer = Signer::generate();
    let mut receipts = Vec::new();
    for (i, title) in [
        "Read your old site in a sealed room",
        "Approved with a code from your authenticator app",
    ]
    .into_iter()
    .enumerate()
    {
        let start = t(20 + i as i64);
        let mut body = ReceiptBody::new(
            ReceiptKind::JobReplay,
            k1(),
            Subject::Site(k1()),
            start,
            start + Duration::seconds(30),
        );
        body.detail = Some(serde_json::json!({ "title": title }));
        let r = Receipt::sign(body, &signer).expect("signs");
        let json = String::from_utf8(r.canonical_json().unwrap()).unwrap();
        let d = r.digest().unwrap();
        exec(
            &pool,
            "INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, started_at, ended_at, issued_at, body) VALUES (?, ?, 'site_k1', ?, 'site', 'site_k1', ?, ?, ?, ?, ?)",
            &[Some(d.to_string()), s(r.body.receipt_id.as_str()), s(r.body.kind.as_str()), s(r.body.issuer.as_str()), Some(db::time(r.body.started_at)), Some(db::time(r.body.ended_at)), Some(db::time(r.body.ended_at)), Some(json)],
        )
        .await;
        receipts.push(d);
    }

    // Cards: go-live waiting, a restore approved with a code, a restore waiting.
    let go_live = card(
        "dc_1",
        CardKind::GoLive,
        "Go live: kilele-bakery.example",
        16,
    );
    let restored = card(
        "dc_2",
        CardKind::QuarantineRestore,
        "Put back the contact form",
        17,
    );
    let asked = card(
        "dc_3",
        CardKind::QuarantineRestore,
        "Put back the casino redirect",
        18,
    );
    for (c, kind, state, method, signer, assertion, receipt, decided) in [
        (&go_live, "go-live", "waiting", None, None, None, None, None),
        (
            &restored,
            "quarantine-restore",
            "approved",
            s("totp"),
            s("acct_1"),
            s("{\"step_up\":true}"),
            Some(receipts[1].to_string()),
            Some(at(19)),
        ),
        (
            &asked,
            "quarantine-restore",
            "waiting",
            None,
            None,
            None,
            None,
            None,
        ),
    ] {
        exec(
            &pool,
            "INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, method, signer, assertion, receipt, created_at, decided_at) VALUES (?, 'site_k1', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[s(c.decision_id.as_str()), s(kind), Some(serde_json::to_string(c).unwrap()), Some(c.fingerprint.to_string()), s(state), method, signer, assertion, receipt, Some(db::time(c.created_at)), decided],
        )
        .await;
    }

    // Quarantine: asked to restore, left behind, restored, deleted.
    for (id, title, state, decision, minutes) in [
        ("qtn_1", "A hidden casino redirect", "held", s("dc_3"), 21),
        ("qtn_2", "An old admin account", "held", None, 22),
        ("qtn_3", "The contact form", "restored", s("dc_2"), 23),
        ("qtn_4", "A spam page", "deleted", None, 24),
    ] {
        exec(
            &pool,
            "INSERT INTO quarantine (item_id, site_id, move_id, title, reason, location, evidence, state, decision_id, created_at, updated_at) VALUES (?, 'site_k1', 'mv_1', ?, 'Plain reason', 'somewhere.php', ?, ?, ?, ?, ?)",
            &[s(id), s(title), Some(serde_json::to_string(&[Digest::of(id.as_bytes())]).unwrap()), s(state), decision, Some(at(minutes)), Some(at(minutes))],
        )
        .await;
    }

    // Charter v1 then v2: the pages read the latest.
    let job = |webagent: &str, job: &str, level, ceiling, reason: &str| JobLevel {
        webagent: webagent.into(),
        role: "Plain role".into(),
        job: job.into(),
        level,
        ceiling,
        ceiling_reason: reason.into(),
    };
    for (version, jobs) in [
        (
            1,
            vec![job(
                "Editor",
                "Keep pages true",
                Level::Watch,
                Level::Suggest,
                "",
            )],
        ),
        (
            2,
            vec![
                job(
                    "Editor",
                    "Keep pages true",
                    Level::Suggest,
                    Level::WithinBudget,
                    "",
                ),
                job(
                    "Concierge",
                    "Reply to customers",
                    Level::AskFirst,
                    Level::AskFirst,
                    "Messages customers, so it always asks first",
                ),
            ],
        ),
    ] {
        let body = serde_json::to_string(&Charter { jobs }).unwrap();
        exec(&pool, "INSERT INTO charters (site_id, version, body, digest, created_at) VALUES ('site_k1', ?, ?, ?, ?)", &[Some(version.to_string()), Some(body.clone()), Some(Digest::of(body.as_bytes()).to_string()), Some(at(version))]).await;
    }

    // Briefs: the latest wins.
    for (minutes, headline) in [
        (30, "Yesterday's brief"),
        (40, "A quiet night: nothing needed you."),
    ] {
        let brief = Brief {
            headline: Some(headline.into()),
            lines: vec![BriefLine {
                text: "Your site stayed up.".into(),
                receipt: Some(receipts[0]),
            }],
            spend_words: Some("Models cost $0.02 today.".into()),
        };
        exec(
            &pool,
            "INSERT INTO briefs (site_id, written_at, body) VALUES ('site_k1', ?, ?)",
            &[
                Some(at(minutes)),
                Some(serde_json::to_string(&brief).unwrap()),
            ],
        )
        .await;
    }

    // Gauntlet: a finished run, then one still going (no receipt yet).
    let attacks = vec![
        GauntletAttack {
            name: "Casino redirect in the theme".into(),
            plain: "Left behind by Clean.".into(),
            held: true,
            receipt: Some(receipts[0]),
        },
        GauntletAttack {
            name: "Prompt injection in a page".into(),
            plain: "Read as text.".into(),
            held: false,
            receipt: None,
        },
    ];
    exec(&pool, "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, receipt, detail, started_at, ended_at) VALUES ('gr_1', 'site_k1', 1, 2, ?, ?, ?, ?)", &[Some(receipts[0].to_string()), Some(serde_json::to_string(&attacks).unwrap()), Some(at(50)), Some(at(55))]).await;
    exec(&pool, "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, started_at) VALUES ('gr_2', 'site_k1', 0, 2, ?)", &[Some(at(70))]).await;

    Seeded {
        pool,
        keys: KeySet {
            keys: vec![signer.published_key()],
        },
        receipts,
        go_live,
    }
}

fn reads(seeded: &Seeded) -> SqliteReads<Outside> {
    SqliteReads::new(seeded.pool.clone(), seeded.keys.clone(), Outside)
}

#[tokio::test]
async fn sites_say_where_they_are() {
    let seeded = seed().await;
    let reads = reads(&seeded);
    let k1 = reads.site(&k1()).await.unwrap().expect("site");
    assert_eq!(
        (
            k1.name.as_str(),
            k1.host.as_str(),
            k1.fixture,
            k1.utc_offset_minutes
        ),
        ("Kilele Bakery", "kilele-bakery.example", true, 180)
    );
    assert_eq!(k1.stage, SiteStage::Moving);
    let free = reads
        .site(&SiteId::new("site_free"))
        .await
        .unwrap()
        .expect("site");
    assert_eq!(
        (free.name.as_str(), free.stage),
        ("free.example", SiteStage::Setup),
        "no name yet: the host stands in"
    );
    let live = reads
        .site(&SiteId::new("site_other"))
        .await
        .unwrap()
        .expect("site");
    assert_eq!((live.stage, live.live_since), (SiteStage::Live, Some(t(5))));
    assert!(
        reads
            .site(&SiteId::new("site_nope"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_move_is_its_ui_events_folded() {
    let seeded = seed().await;
    let m = reads(&seeded)
        .latest_move(&k1())
        .await
        .unwrap()
        .expect("move");
    assert_eq!(m.stage, MoveStage::Waiting);
    assert_eq!(
        m.steps
            .iter()
            .map(|s| (s.step, s.state))
            .collect::<Vec<_>>(),
        [
            (StepName::Claim, StepState::Done),
            (StepName::Replay, StepState::Done)
        ],
        "latest per step, in Move order"
    );
    assert_eq!((m.tick.requests, m.tick.refusals), (30, 2));
    assert_eq!(m.last_frame.expect("frame").digest, Digest::of(b"frame-k1"));
    assert_eq!(m.parity_site.expect("site parity").state, ParityState::Pass);
    assert_eq!(m.parity_pages.len(), 1, "one row per page");
    assert_eq!(m.parity_pages[0].attempt, 2);
    assert_eq!(m.quarantined.len(), 1, "the same item twice is one item");
    assert_eq!(
        m.go_live.expect("card").decision_id,
        DecisionId::new("dc_1")
    );
    assert!(
        reads(&seeded)
            .latest_move(&SiteId::new("site_free"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cards_carry_their_approval_and_stay_on_their_site() {
    let seeded = seed().await;
    let reads = reads(&seeded);
    let all = reads.decisions(&k1()).await.unwrap();
    assert_eq!(
        all.iter()
            .map(|d| d.card.decision_id.as_str())
            .collect::<Vec<_>>(),
        ["dc_3", "dc_2", "dc_1"],
        "newest first"
    );
    let approved = &all[1];
    assert_eq!(approved.state, DecisionState::Approved);
    let approval = approved.approval.as_ref().expect("approval");
    assert_eq!(
        (approval.method, approval.approver.as_str(), approval.at),
        (ApprovalMethod::StepUp, "acct_1", t(19))
    );
    assert_eq!(approved.receipt, Some(seeded.receipts[1]));
    let one = reads
        .decision(&k1(), &DecisionId::new("dc_1"))
        .await
        .unwrap()
        .expect("card");
    assert_eq!(
        (one.state, one.card.fingerprint),
        (DecisionState::Waiting, seeded.go_live.fingerprint)
    );
    assert!(
        reads
            .decision(&SiteId::new("site_other"), &DecisionId::new("dc_1"))
            .await
            .unwrap()
            .is_none(),
        "another site's card is not found"
    );
}

#[tokio::test]
async fn a_card_whose_fingerprint_was_changed_is_an_error() {
    let seeded = seed().await;
    exec(
        &seeded.pool,
        "UPDATE decisions SET fingerprint = ? WHERE decision_id = 'dc_1'",
        &[Some(Digest::of(b"something else").to_string())],
    )
    .await;
    assert!(
        reads(&seeded)
            .decision(&k1(), &DecisionId::new("dc_1"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn quarantine_says_where_each_item_is() {
    let seeded = seed().await;
    let rows = reads(&seeded).quarantine(&k1()).await.unwrap();
    assert_eq!(
        rows.iter()
            .map(|q| (q.id.as_str(), q.stage))
            .collect::<Vec<_>>(),
        [
            ("qtn_1", QuarantineStage::RestoreAsked),
            ("qtn_2", QuarantineStage::LeftBehind),
            ("qtn_3", QuarantineStage::Restored)
        ],
        "deleted items are gone"
    );
    assert_eq!(rows[0].evidence, [Digest::of(b"qtn_1")]);
}

#[tokio::test]
async fn receipts_charter_brief_and_gauntlet_read_their_stored_shapes() {
    let seeded = seed().await;
    let reads = reads(&seeded);
    let receipts = reads.receipts(&k1(), 10).await.unwrap();
    assert_eq!(
        receipts
            .iter()
            .map(|r| r.digest().unwrap())
            .collect::<Vec<_>>(),
        [seeded.receipts[1], seeded.receipts[0]],
        "newest first"
    );
    assert!(
        receipts[0]
            .verify_with(&reads.keys().await.unwrap())
            .is_ok()
    );
    assert_eq!(
        reads
            .receipt(&seeded.receipts[0])
            .await
            .unwrap()
            .expect("receipt")
            .body
            .title(),
        Some("Read your old site in a sealed room")
    );
    let levels = reads.levels(&k1()).await.unwrap();
    assert_eq!(
        levels.iter().map(|l| l.job.as_str()).collect::<Vec<_>>(),
        ["Keep pages true", "Reply to customers"],
        "charter v2"
    );
    let brief = reads.brief(&k1()).await.unwrap().expect("brief");
    assert_eq!(
        (brief.written_at, brief.brief.headline.as_deref()),
        (t(40), Some("A quiet night: nothing needed you."))
    );
    let run = reads.gauntlet(&k1()).await.unwrap().expect("run");
    assert_eq!(
        (run.run_at, run.receipt, run.attacks.len()),
        (t(55), seeded.receipts[0], 2),
        "the finished run, not the one still going"
    );
    assert!(
        reads
            .levels(&SiteId::new("site_free"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        reads
            .brief(&SiteId::new("site_free"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        reads
            .gauntlet(&SiteId::new("site_free"))
            .await
            .unwrap()
            .is_none()
    );
}

async fn page(app: &Router, path: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn every_console_page_renders_from_the_database() {
    let seeded = seed().await;
    let state = PagesState {
        console: Arc::new(Console::new().expect("templates")),
        reads: Arc::new(reads(&seeded)),
        onboarding: None,
        concepts: None,
    };
    let old = first_release_router(state.clone());
    let app = router(state);
    for (name, path, shows) in [
        (
            "today",
            "/console/first-today",
            "A quiet night: nothing needed you.",
        ),
        (
            "decisions",
            "/console/decisions",
            "Put back the casino redirect",
        ),
        (
            "decision",
            "/console/decisions/dc_1",
            "Go live: kilele-bakery.example",
        ),
        (
            "move",
            "/console/move",
            "Read your old site in a sealed room",
        ),
        ("trust", "/console/trust", "Prompt injection in a page"),
        (
            "webagents",
            "/console/webagents",
            "Messages customers, so it always asks first",
        ),
        ("quarantine", "/console/quarantine", "An old admin account"),
    ] {
        let on = if path == "/console/move" { &app } else { &old };
        let (status, html) = page(on, path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {html}");
        assert!(
            html.contains("Kilele Bakery"),
            "{path}: the site's name is in the header"
        );
        assert!(html.contains(shows), "{path}: shows {shows:?}");
        if let Ok(dir) = std::env::var("AGENTRIX_PREVIEW_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(format!("{dir}/db-{name}.html"), &html).unwrap();
        }
    }
    let (_, decision) = page(&old, "/console/decisions/dc_1").await;
    assert!(
        decision.contains(&seeded.go_live.fingerprint.to_string()),
        "the card shows the fingerprint it signs"
    );
    assert!(
        !decision.contains("Approve with your passkey"),
        "no passkey: the code is the way to approve"
    );
    let (status, receipt) = page(&app, &format!("/receipts/{}", seeded.receipts[0])).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        receipt.contains("Signature checked against our published key"),
        "a stored receipt verifies against the published key"
    );
}

#[tokio::test]
async fn today_shows_the_questions_visitors_passed() {
    use crate::site::QuestionSink as _;
    let seeded = seed().await;
    let store = db::questions::QuestionStore::new(seeded.pool.clone());
    for (minutes, question, contact) in [
        (20, "Do you bake sugar-free cakes?", Some("0700 000 000")),
        (40, "Open on Easter Monday?", None),
    ] {
        store
            .pass(crate::site::PassedQuestion {
                site: "site_k1".into(),
                release_seq: 1,
                question: question.into(),
                contact: contact.map(str::to_owned),
                at: t(minutes),
            })
            .await
            .unwrap();
    }
    let rows = reads(&seeded).passed_questions(&k1()).await.unwrap();
    assert_eq!(rows.total, 2);
    assert_eq!(
        rows.recent[0].question, "Open on Easter Monday?",
        "newest first"
    );

    let app = first_release_router(PagesState {
        console: Arc::new(Console::new().expect("templates")),
        reads: Arc::new(reads(&seeded)),
        onboarding: None,
        concepts: None,
    });
    let (status, html) = page(&app, "/console/first-today").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("Questions visitors passed to you"), "{html}");
    assert!(html.contains("Do you bake sugar-free cakes?"));
    assert!(html.contains("Reply to 0700 000 000"));
    assert!(html.contains("No way to reply was left"));
}

#[tokio::test]
async fn the_meter_reads_the_month_against_the_monthly_budget_never_a_day() {
    let seeded = seed().await;
    let r = reads(&seeded);
    assert_eq!(
        r.meter(&k1()).await.unwrap(),
        None,
        "no model call yet: no meter"
    );
    let now = crate::db::time(Utc::now());
    for (purpose, provider, cost, outcome) in [
        ("build", "anthropic", 210_000, "ok"),
        ("screen", "anthropic", 2_000, "ok"),
        ("embed", "vultr", 0, "replayed"),
        ("build", "anthropic", 0, "over-budget"),
    ] {
        sqlx::query(
            "INSERT INTO usage (site_id, purpose, provider, model, tokens_in, tokens_out, cost_micro_usd, outcome, at) \
             VALUES ('site_k1', ?, ?, 'm', 0, 0, ?, ?, ?)",
        )
        .bind(purpose)
        .bind(provider)
        .bind(cost)
        .bind(outcome)
        .bind(&now)
        .execute(&seeded.pool)
        .await
        .unwrap();
    }
    // A day budget is the gateway's guard; it is never the owner's number.
    sqlx::query("INSERT INTO budgets (site_id, purpose, period, limit_micro_usd) VALUES ('site_k1', '*', 'day', 1000000)")
        .execute(&seeded.pool)
        .await
        .unwrap();
    let m = r.meter(&k1()).await.unwrap().expect("a meter");
    assert_eq!(m.month.micro_usd, 212_000);
    assert_eq!(m.budget.cap, None, "no monthly budget yet");
    let v = crate::pages::map::meter(&m);
    assert_eq!(v.cap_words, "$0.21 this month");
    assert_eq!(v.cap_pct, None);
    assert_eq!(v.cap_note, "No monthly AI budget is set for this site yet.");
    assert_eq!(v.by_purpose[0].label, "Building pages");
    assert_eq!(v.by_provider[0].label, "Anthropic");
    assert_eq!(
        v.refused_words.as_deref(),
        Some("1 AI call was stopped by a spending limit this month")
    );

    // A monthly budget for every purpose is the cap the owner sees.
    sqlx::query("UPDATE budgets SET period = 'month', limit_micro_usd = 30000000 WHERE site_id = 'site_k1' AND purpose = '*'")
        .execute(&seeded.pool)
        .await
        .unwrap();
    let v = crate::pages::map::meter(&r.meter(&k1()).await.unwrap().unwrap());
    assert_eq!(v.cap_words, "$0.21 of $30.00 this month");
    assert_eq!(v.cap_pct, Some(1));
    for words in [&v.cap_words, &v.cap_note, &v.calls_words] {
        assert!(!words.contains("day"), "{words}");
    }
}

#[tokio::test]
async fn uploads_list_the_newest_of_each_export() {
    let seeded = seed().await;
    let r = reads(&seeded);
    for (digest, kind, size, t) in [
        ("sha256:aa", "upload:export.xml", 1000, 10),
        ("sha256:bb", "upload:export.xml", 2000, 20),
        ("sha256:cc", "upload:dump.sql.gz", 500, 15),
        ("sha256:dd", "page", 99, 30),
    ] {
        sqlx::query("INSERT INTO artifacts (site_id, digest, kind, content_type, size, location, created_at) VALUES ('site_k1', ?, ?, 'application/octet-stream', ?, 'local', ?)")
            .bind(digest)
            .bind(kind)
            .bind(size)
            .bind(at(t))
            .execute(&seeded.pool)
            .await
            .unwrap();
    }
    let ups = r.uploads(&k1()).await.unwrap();
    assert_eq!(ups.len(), 2, "pages are not uploads");
    assert_eq!(
        (ups[0].name.as_str(), ups[0].size),
        ("export.xml", 2000),
        "the newest export wins"
    );
    assert_eq!(ups[1].name, "dump.sql.gz");
}

#[tokio::test]
async fn pictures_waiting_counts_only_the_moves_drafts() {
    let seeded = seed().await;
    let r = reads(&seeded);
    assert_eq!(r.pictures_waiting(&k1()).await.unwrap(), 0);
    for (id, status, asked_by) in [
        ("img_1", "draft", "move"),
        ("img_2", "draft", "move"),
        ("img_3", "approved", "move"),
        ("img_4", "draft", "owner"),
    ] {
        sqlx::query(
            "INSERT INTO media_images (image_id, site_id, slot, kind, status, origin, digest, media_type, bytes, alt, photographic, shows_person, shows_text, asked_by, created_at) \
             VALUES (?, 'site_k1', 'page:home', 'fill', ?, 'generated', '', 'image/webp', 0, '', 0, 0, 0, ?, ?)",
        )
        .bind(id)
        .bind(status)
        .bind(asked_by)
        .bind(at(10))
        .execute(&seeded.pool)
        .await
        .unwrap();
    }
    assert_eq!(r.pictures_waiting(&k1()).await.unwrap(), 2);
}
