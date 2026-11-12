//! The pages against a fake store holding real `agentrix-types` values: receipts signed with a
//! generated key, a go-live card with its fingerprint, Move events. Set `AGENTRIX_PREVIEW_DIR`
//! to also write each page there for a look in a browser.

use std::sync::Arc;

use agentrix_console::Console;
use agentrix_types::ui::{
    CardEvent, FrameEvent, FrameKind, PageEvent, PageStatus, ParityEvent, ParityState,
    QuarantinedEvent, StepEvent, StepName, StepState, TickEvent,
};
use agentrix_types::{
    Approval, ApprovalMethod, CardKind, DecisionCard, DecisionId, Digest, EffectRequest, KeySet,
    MoveId, QuarantineId, Receipt, ReceiptBody, ReceiptKind, Risk, Signer, SiteId, Subject,
};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use tower::ServiceExt;

use super::*;

struct Fake {
    signed_in: bool,
    keys: KeySet,
    receipts: Vec<Receipt>,
    go_live: DecisionCard,
    approved: bool,
    now: DateTime<Utc>,
}

fn site_id() -> SiteId {
    SiteId::new("site_k1")
}

impl Fake {
    fn new(signed_in: bool, approved: bool) -> Self {
        let now = Utc::now();
        let signer = Signer::generate();
        let receipts = [
            ReceiptKind::JobFetch,
            ReceiptKind::JobReplay,
            ReceiptKind::MoveStep,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, kind)| {
            let start = now - Duration::minutes(30 - i as i64 * 5);
            let mut body = ReceiptBody::new(
                kind,
                site_id(),
                Subject::Move(MoveId::new("mv_1")),
                start,
                start + Duration::seconds(108),
            );
            let title = [
                "Read 14 pages in a sealed room",
                "Replayed your WordPress, sealed",
                "Checked the replay against your site",
            ][i];
            body.detail = Some(serde_json::json!({ "title": title }));
            body.destroyed = Some(true);
            Receipt::sign(body, &signer).expect("signs")
        })
        .collect();
        let effect = EffectRequest {
            site: site_id(),
            webagent: "Mover".into(),
            tool: "go_live".into(),
            args: serde_json::json!({ "release": "v1", "charter": "v1" }),
            amount: None,
            expiry: now + Duration::hours(24),
        };
        let mut go_live = DecisionCard::new(
            DecisionId::new("dc_1"),
            CardKind::GoLive,
            "Go live: kilele-bakery.example",
            "Signed release v1 starts serving kilele-bakery.example: 14 pages, 97% match.\nThe 4 items found in your old site stay behind.\nYour old site stays as it is, as a fallback, for 30 days.",
            effect,
            now - Duration::minutes(5),
        )
        .expect("card");
        go_live.risk = Some(Risk {
            model: "systemone".into(),
            score: 0.4,
            confidence: Some(0.8),
            summary: "Your whole site's address changes; undo is one tap for 30 days.".into(),
        });
        Self {
            signed_in,
            keys: KeySet {
                keys: vec![signer.published_key()],
            },
            receipts,
            go_live,
            approved,
            now,
        }
    }
    fn row(&self) -> DecisionRow {
        DecisionRow {
            card: self.go_live.clone(),
            state: if self.approved {
                DecisionState::Approved
            } else {
                DecisionState::Waiting
            },
            approval: self.approved.then(|| Approval {
                decision_id: self.go_live.decision_id.clone(),
                fingerprint: self.go_live.fingerprint,
                method: ApprovalMethod::StepUp,
                approver: "acct_1".into(),
                at: self.now,
            }),
            receipt: self
                .approved
                .then(|| self.receipts[2].digest().expect("digest")),
        }
    }
}

#[async_trait]
impl ConsoleReads for Fake {
    async fn session(&self, _: &HeaderMap) -> ReadResult<Option<OwnerSession>> {
        Ok(self.signed_in.then(|| OwnerSession {
            account: "acct_1".into(),
            name: "Wanjiru".into(),
            site: site_id(),
            has_passkey: true,
        }))
    }
    async fn site(&self, id: &SiteId) -> ReadResult<Option<SiteRow>> {
        Ok((id == &site_id()).then(|| SiteRow {
            id: site_id(),
            host: "kilele-bakery.example".into(),
            name: "Kilele Bakery".into(),
            fixture: true,
            stage: SiteStage::Moving,
            live_since: None,
            utc_offset_minutes: 180,
        }))
    }
    async fn latest_move(&self, _: &SiteId) -> ReadResult<Option<MoveRow>> {
        let at = self.now;
        let steps = [
            StepName::Assess,
            StepName::Claim,
            StepName::Replay,
            StepName::Clean,
            StepName::Rebuild,
            StepName::Prove,
            StepName::GoLive,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let state = if i < 5 {
                StepState::Done
            } else if i == 5 {
                StepState::Running
            } else {
                StepState::Pending
            };
            let mut e = StepEvent::new(
                s,
                state,
                if state == StepState::Pending {
                    0
                } else {
                    108_000
                },
                if i <= 5 { "Done in a sealed room" } else { "" },
                at,
            );
            if i < 5 {
                e.receipt = Some(self.receipts[0].digest().expect("digest"));
            }
            e
        })
        .collect();
        Ok(Some(MoveRow {
            id: MoveId::new("mv_1"),
            stage: MoveStage::Running,
            started_at: at - Duration::minutes(20),
            steps,
            pages: Some(PageEvent {
                n: 12,
                of: 14,
                url: "/order".into(),
                status: PageStatus::Read,
                reason: None,
                at,
            }),
            tick: TickEvent {
                job: None,
                hosts: 1,
                requests: 38,
                bytes: 2_200_000,
                refusals: 3,
                at,
            },
            last_frame: Some(FrameEvent::new(
                FrameKind::SealedScreenshot,
                "https://kilele-bakery.example/cakes",
                Digest::of(b"frame"),
                390,
                844,
                at,
            )),
            parity_site: Some(ParityEvent {
                page: None,
                score: 0.96,
                verdict: String::new(),
                state: ParityState::Pass,
                attempt: 1,
                at,
            }),
            parity_pages: vec![ParityEvent {
                page: Some("/order".into()),
                score: 0.81,
                verdict: "The order form lost its date field.".into(),
                state: ParityState::Retry,
                attempt: 2,
                at,
            }],
            quarantined: vec![QuarantinedEvent {
                item_id: QuarantineId::new("q1"),
                title: "A hidden script that sends phone visitors to a casino site".into(),
                reason: "Redirects visitors".into(),
                location: "theme footer".into(),
                at,
            }],
            go_live: Some(CardEvent {
                decision_id: DecisionId::new("dc_1"),
                kind: CardKind::GoLive,
                title: "Go live: kilele-bakery.example".into(),
                href: "/console/decisions/dc_1".into(),
                at,
            }),
        }))
    }
    async fn decisions(&self, _: &SiteId) -> ReadResult<Vec<DecisionRow>> {
        Ok(vec![self.row()])
    }
    async fn decision(&self, _: &SiteId, id: &DecisionId) -> ReadResult<Option<DecisionRow>> {
        Ok((id == &self.go_live.decision_id).then(|| self.row()))
    }
    async fn begin_approval(
        &self,
        _: &OwnerSession,
        card: &DecisionCard,
    ) -> ReadResult<Option<String>> {
        Ok(Some(serde_json::json!({ "publicKey": { "challenge": card.fingerprint.hex(), "allowCredentials": [] } }).to_string()))
    }
    async fn levels(&self, _: &SiteId) -> ReadResult<Vec<JobLevel>> {
        Ok(vec![
            JobLevel {
                webagent: "Editor".into(),
                role: "Keeps every page true".into(),
                job: "Keep pages true".into(),
                level: Level::Suggest,
                ceiling: Level::WithinBudget,
                ceiling_reason: String::new(),
            },
            JobLevel {
                webagent: "Guardian".into(),
                role: "Watches the site and can roll it back".into(),
                job: "Protect the site".into(),
                level: Level::UndoOnly,
                ceiling: Level::UndoOnly,
                ceiling_reason: "Only rolls back; anything more asks you".into(),
            },
            JobLevel {
                webagent: "Concierge".into(),
                role: "Answers your customers".into(),
                job: "Reply to customers".into(),
                level: Level::AskFirst,
                ceiling: Level::AskFirst,
                ceiling_reason: "Messages customers, so it always asks first".into(),
            },
        ])
    }
    async fn receipts(&self, _: &SiteId, limit: usize) -> ReadResult<Vec<Receipt>> {
        Ok(self.receipts.iter().rev().take(limit).cloned().collect())
    }
    async fn receipt(&self, digest: &Digest) -> ReadResult<Option<Receipt>> {
        Ok(self
            .receipts
            .iter()
            .find(|r| r.digest().ok().as_ref() == Some(digest))
            .cloned())
    }
    async fn gauntlet(&self, _: &SiteId) -> ReadResult<Option<GauntletRow>> {
        let run = self.receipts[0].digest().expect("digest");
        Ok(Some(GauntletRow {
            run_at: self.now - Duration::hours(1),
            receipt: run,
            attacks: vec![
                GauntletAttack {
                    name: "Casino redirect in the theme".into(),
                    plain: "Left behind by Clean; never reached the new site.".into(),
                    held: true,
                    receipt: Some(run),
                },
                GauntletAttack {
                    name: "Prompt injection in a page".into(),
                    plain: "Read as text, never as an instruction.".into(),
                    held: true,
                    receipt: None,
                },
            ],
        }))
    }
    async fn brief(&self, _: &SiteId) -> ReadResult<Option<BriefRow>> {
        let read = self.receipts[0].digest().expect("digest");
        Ok(Some(BriefRow {
            written_at: self.now - Duration::hours(2),
            brief: Brief {
                headline: Some("A quiet night: your site stayed up and nothing needed you.".into()),
                lines: vec![
                    BriefLine {
                        text: "The Mover read your old site in a sealed room.".into(),
                        receipt: Some(read),
                    },
                    BriefLine {
                        text: "It checked the rebuilt pages against the old ones.".into(),
                        receipt: Some(read),
                    },
                ],
                spend_words: None,
            },
        }))
    }
    async fn quarantine(&self, _: &SiteId) -> ReadResult<Vec<QuarantineRow>> {
        Ok(vec![QuarantineRow {
            id: QuarantineId::new("q1"),
            title: "A hidden script that sends phone visitors to a casino site".into(),
            plain: "When someone arrives from a search on a phone, this sends them somewhere else."
                .into(),
            location: "wp-content/themes/kilele/footer.php, line 88".into(),
            evidence: vec![Digest::of(b"footer.php")],
            stage: QuarantineStage::LeftBehind,
            decision: None,
        }])
    }
    /// Three blobs: this owner's frame, another owner's, and one from a free Assess (no owner).
    async fn frame(
        &self,
        owner_site: Option<&SiteId>,
        digest: &Digest,
    ) -> ReadResult<Option<FrameBytes>> {
        let ours = digest == &Digest::of(b"frame") && owner_site == Some(&site_id());
        let assess = digest == &Digest::of(b"assess");
        Ok((ours || assess).then(|| FrameBytes {
            bytes: Bytes::from_static(b"RIFF....WEBP"),
            content_type: "image/webp".into(),
        }))
    }
    async fn keys(&self) -> ReadResult<KeySet> {
        Ok(self.keys.clone())
    }
}

fn state(fake: Fake) -> PagesState {
    PagesState {
        console: Arc::new(Console::new().expect("templates")),
        reads: Arc::new(fake),
        onboarding: None,
        concepts: None,
    }
}

fn app(fake: Fake) -> Router {
    router(state(fake))
}

/// The first release's deprecated owner pages, rendered by their kept handlers.
fn first_release(fake: Fake) -> Router {
    first_release_router(state(fake))
}

async fn get(app: &Router, path: &str, accept: Option<&str>) -> (StatusCode, HeaderMap, String) {
    let mut req = Request::get(path);
    if let Some(a) = accept {
        req = req.header(header::ACCEPT, a);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.expect("body");
    (
        parts.status,
        parts.headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn preview(name: &str, html: &str) {
    if let Ok(dir) = std::env::var("AGENTRIX_PREVIEW_DIR") {
        std::fs::create_dir_all(&dir).expect("preview dir");
        std::fs::write(format!("{dir}/node-{name}.html"), html).expect("preview");
    }
}

#[tokio::test]
async fn first_release_pages_go_to_their_page_in_agentrix_os() {
    let app = app(Fake::new(false, false));
    for (from, to) in [
        ("/console", "/os"),
        ("/console/today", "/os"),
        ("/console/first-today", "/os"),
        ("/console/decisions", "/os/decisions"),
        ("/console/decisions/dc_1", "/d/dc_1"),
        ("/console/trust", "/os/trust"),
        ("/console/webagents", "/os/webagents"),
        ("/console/quarantine", "/os/quarantine"),
    ] {
        let (status, headers, _) = get(&app, from, None).await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT, "{from}");
        assert_eq!(headers[header::LOCATION], to, "{from}");
    }
    assert_eq!(
        get(&app, "/console/decisions/..%2Fsettings", None).await.0,
        StatusCode::NOT_FOUND,
        "only a plain card id is passed on"
    );
}

#[tokio::test]
async fn every_console_page_renders_from_the_store() {
    let old = first_release(Fake::new(true, false));
    let app = app(Fake::new(true, false));
    for (name, path) in [
        ("today", "/console/first-today"),
        ("decisions", "/console/decisions"),
        ("decision", "/console/decisions/dc_1"),
        ("move", "/console/move"),
        ("trust", "/console/trust"),
        ("webagents", "/console/webagents"),
        ("quarantine", "/console/quarantine"),
    ] {
        let app = if path == "/console/move" { &app } else { &old };
        let (status, _, html) = get(app, path, None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {html}");
        assert!(
            html.contains("Kilele Bakery"),
            "{path}: the site's name is in the header"
        );
        preview(name, &html);
    }
}

#[tokio::test]
async fn signed_out_goes_to_sign_in_and_comes_back() {
    let app = first_release(Fake::new(false, false));
    let (status, headers, _) = get(&app, "/console/trust", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers[header::LOCATION],
        "/auth/signin?next=/console/trust"
    );
}

#[tokio::test]
async fn the_go_live_card_carries_its_fingerprint_options_and_locked_positions() {
    let fake = Fake::new(true, false);
    let fp = fake.go_live.fingerprint.to_string();
    let app = first_release(fake);
    let (_, _, html) = get(&app, "/console/decisions/dc_1", None).await;
    assert!(
        html.contains(&fp),
        "the card shows the fingerprint it signs"
    );
    assert!(
        html.contains("data-options="),
        "the ceremony's options ride on the page (D9)"
    );
    assert!(
        html.contains("Messages customers, so it always asks first"),
        "positions above the ceiling are locked"
    );
    assert!(html.contains("Undo only"));
    assert!(
        html.contains("action=\"/decisions/dc_1/approve\""),
        "the code form posts to the approve route"
    );
}

#[tokio::test]
async fn a_decided_card_names_the_method_and_links_its_receipt() {
    let fake = Fake::new(true, true);
    let receipt = fake.receipts[2].digest().unwrap().to_string();
    let app = first_release(fake);
    let (_, _, html) = get(&app, "/console/decisions/dc_1", None).await;
    assert!(html.contains("Approved with a code from your authenticator app"));
    assert!(html.contains(&format!("/receipts/{receipt}")));
    assert!(
        !html.contains("data-options="),
        "no ceremony for a decided card"
    );
}

#[tokio::test]
async fn receipts_verify_and_open_as_json_or_a_page_for_anyone() {
    let fake = Fake::new(false, false);
    let digest = fake.receipts[0].digest().unwrap().to_string();
    let app = app(fake);
    let (status, headers, json) = get(
        &app,
        &format!("/receipts/{digest}"),
        Some("application/json"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    let parsed: Receipt = serde_json::from_str(&json).expect("a signed receipt");
    assert_eq!(
        parsed.digest().unwrap().to_string(),
        digest,
        "the JSON is the receipt the digest names"
    );
    let (status, _, html) = get(&app, &format!("/receipts/{digest}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Signature checked against our published key"));
    preview("receipt", &html);
}

#[tokio::test]
async fn a_tampered_receipt_says_so() {
    let mut fake = Fake::new(true, false);
    fake.receipts[1].body.destroyed = Some(false);
    let app = first_release(fake);
    let (_, _, html) = get(&app, "/console/trust", None).await;
    assert!(
        html.contains("Not verified"),
        "a receipt whose body changed fails verification"
    );
}

#[tokio::test]
async fn frames_come_from_the_cas_and_unknown_things_are_404() {
    let app = app(Fake::new(true, false));
    let (status, headers, _) = get(&app, &format!("/frames/{}", Digest::of(b"frame")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/webp");
    assert!(
        headers[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .starts_with("private"),
        "never in a shared cache"
    );
    assert_eq!(
        get(&app, &format!("/frames/{}", Digest::of(b"other")), None)
            .await
            .0,
        StatusCode::NOT_FOUND,
        "another owner's frame"
    );
    assert_eq!(
        get(&app, "/frames/not-a-digest", None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(
            &first_release(Fake::new(true, false)),
            "/console/decisions/nope",
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn frames_are_the_owners_or_a_free_assess() {
    let app = app(Fake::new(false, false));
    assert_eq!(
        get(&app, &format!("/frames/{}", Digest::of(b"frame")), None)
            .await
            .0,
        StatusCode::NOT_FOUND,
        "an owner's frame needs their session"
    );
    assert_eq!(
        get(&app, &format!("/frames/{}", Digest::of(b"assess")), None)
            .await
            .0,
        StatusCode::OK,
        "Assess frames are public (design §4.1)"
    );
}

#[tokio::test]
async fn the_brief_and_the_gauntlet_read_their_stored_shapes() {
    let app = first_release(Fake::new(true, false));
    let (_, _, today) = get(&app, "/console/first-today", None).await;
    assert!(
        today.contains("A quiet night: your site stayed up"),
        "the headline opens the brief"
    );
    assert!(
        today.contains("Written from 1 recorded event."),
        "two lines citing one receipt are one event"
    );
    let (_, _, trust) = get(&app, "/console/trust", None).await;
    assert!(trust.contains("2 / 2"));
    assert_eq!(
        trust.matches(">receipt</a>").count(),
        1,
        "an attack without a receipt gets no link"
    );
}
