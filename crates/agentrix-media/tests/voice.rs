//! Talk to your Chief WebAgent, against a fake Model Gateway: spoken decisions by a fixed
//! grammar, the brief read aloud, speech streamed, and live speech relayed both ways. No model
//! is called and no audio is kept.

use std::sync::Arc;

use agentrix_host::NodeContext;
use agentrix_host::testing::{FixedOwner, context};
use agentrix_media::apps::console_apps;
use agentrix_media::routes::MediaState;
use agentrix_media::voice::voice_router;
use agentrix_shell::apps::{AppContext, AppPage, AppReply};
use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use uuid::Uuid;

/// The Model Gateway as W1's Chief calls it: one answer, no tool calls.
async fn fake_chat(headers: HeaderMap, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(headers["x-agentrix-site"], "site_1");
    assert!(
        body["messages"].to_string().contains("croissants"),
        "the owner's words reach the Chief's model"
    );
    Json(json!({
        "choices": [{ "message": { "role": "assistant", "content": "Three orders came in overnight, KSh 8,400 in all." }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 400, "completion_tokens": 20 }
    }))
}

async fn fake_speech(headers: HeaderMap, Json(body): Json<Value>) -> Response {
    assert_eq!(headers["x-agentrix-purpose"], "voice.speak");
    assert_eq!(body["voice"], "aiden");
    (
        [("content-type", "text/event-stream")],
        "data: {\"type\":\"speech.audio.delta\",\"audio\":\"AAABAA==\"}\n\ndata: {\"type\":\"speech.audio.done\",\"usage\":{\"seconds\":1}}\n\n",
    )
        .into_response()
}

async fn fake_realtime(headers: HeaderMap, upgrade: WebSocketUpgrade) -> Response {
    assert_eq!(headers["x-agentrix-purpose"], "voice.live");
    assert_eq!(headers["x-agentrix-site"], "site_1");
    upgrade.on_upgrade(|mut socket| async move {
        let _ = socket.send(WsMessage::Text(json!({ "type": "session.updated" }).to_string().into())).await;
        let mut heard = 0;
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                WsMessage::Binary(pcm) => heard += pcm.len(),
                WsMessage::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_str()).unwrap();
                    assert_eq!(event["type"], "input_audio_buffer.commit", "the browser's words are translated");
                    assert!(heard > 0, "audio came first");
                    for out in [
                        json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "Deny" }),
                        json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": " Deny. " }),
                        json!({ "type": "something.internal", "secret": "never forwarded" }),
                    ] {
                        let _ = socket.send(WsMessage::Text(out.to_string().into())).await;
                    }
                }
                _ => {}
            }
        }
    })
}

/// Stands in for the Cashier's runner: releasing an order does nothing in the test.
struct ReleaseOrder;

#[async_trait::async_trait]
impl agentrix_decisions::EffectRunner for ReleaseOrder {
    fn tool(&self) -> &str {
        "order.release"
    }
    async fn run(
        &self,
        _card: &agentrix_decisions::DecisionCardV2,
        _receipt: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

struct Rig {
    ctx: NodeContext,
    url: String,
    owner: Uuid,
}

async fn rig() -> Rig {
    let gateway = Router::new()
        .route("/v1/chat/completions", post(fake_chat))
        .route("/v1/audio/speech", post(fake_speech))
        .route("/v1/realtime", get(fake_realtime));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });

    let owner = Uuid::new_v4();
    let mut ctx = context().await.unwrap();
    ctx.models_url = format!("http://{addr}");
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    sqlx::raw_sql(
        "CREATE TABLE sites (site_id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', owner_account_id TEXT);
         CREATE TABLE briefs (site_id TEXT NOT NULL, written_at TEXT NOT NULL, body TEXT NOT NULL);",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO sites VALUES ('site_1', 'Kilele Bakery', ?)")
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    for sql in [
        include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
        include_str!("../../agentrix-node/src/db/migrations/1201_voice_notes.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&ctx.pool).await.unwrap();
    }
    let app = voice_router(MediaState::new(&ctx).with_runners(vec![Arc::new(ReleaseOrder)]));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Rig {
        ctx,
        url: format!("http://{addr}"),
        owner,
    }
}

impl Rig {
    async fn turn(&self, said: &str, card: Option<Value>) -> Value {
        let response = reqwest::Client::new()
            .post(format!("{}/api/voice/site_1/turn", self.url))
            .json(&json!({ "said": said, "card": card, "waiting": 1 }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }

    async fn notes(&self) -> Vec<(String, Option<String>, String)> {
        sqlx::query_as("SELECT heard, intent, did FROM voice_notes ORDER BY note_id")
            .fetch_all(&self.ctx.pool)
            .await
            .unwrap()
    }
}

fn go_live() -> Value {
    json!({ "id": "card_1", "title": "Go live with release v14", "risk": "risk", "from": "Mover asks", "risk_words": "High risk" })
}

fn held_order() -> Value {
    json!({ "id": "card_2", "title": "Release the held order KB-1042", "risk": "wait", "from": "Cashier asks", "risk_words": "Medium" })
}

#[tokio::test]
async fn spoken_decisions_follow_the_voice_rules_and_leave_a_note() {
    let rig = rig().await;
    let deny = rig.turn("Deny.", Some(held_order())).await;
    assert_eq!(deny["intent"], "deny");
    assert_eq!(
        deny["action"],
        json!({ "do": "decline", "via": "form", "card": "card_2", "title": "Release the held order KB-1042", "meta": "Cashier asks · Medium", "after_ms": 3000 })
    );
    assert_eq!(
        deny["reply"],
        "Declining Release the held order KB-1042 in 3 seconds. Say cancel to keep it."
    );

    let high = rig.turn("Allow once", Some(go_live())).await;
    assert_eq!(high["action"]["do"], "open_sheet", "voice never approves");
    assert_eq!(
        high["reply"],
        "Go live with release v14 is high risk, so voice can't approve it. I've opened it on your screen."
    );
    let everyday = rig.turn("allow it once", Some(held_order())).await;
    assert_eq!(everyday["action"]["do"], "open_sheet");

    let stop = rig.turn("Stop everything", None).await;
    assert_eq!(
        stop["action"]["do"], "freeze",
        "stop acts at once, with no card needed"
    );
    let cancel = rig.turn("Cancel", None).await;
    assert_eq!(cancel["action"]["do"], "cancel");
    let nothing = rig.turn("Deny", None).await;
    assert_eq!(nothing["action"], Value::Null);
    assert_eq!(nothing["reply"], "Nothing is waiting for you to decide.");
    let next = rig.turn("What's next?", Some(held_order())).await;
    assert_eq!(
        next["reply"],
        "Cashier asks: Release the held order KB-1042. Medium. Say allow once, or deny."
    );

    let notes = rig.notes().await;
    assert_eq!(notes.len(), 7);
    assert_eq!(
        notes[0],
        (
            "Deny.".to_owned(),
            Some("deny".to_owned()),
            "decline after 3 s".to_owned()
        )
    );
    assert_eq!(notes[1].2, "sent to screen");
    assert_eq!(notes[3].2, "freeze");
}

#[tokio::test]
async fn questions_go_to_the_chief_and_the_brief_is_read_from_its_record() {
    let rig = rig().await;
    let none = rig.turn("Read me my brief", None).await;
    assert_eq!(
        none["reply"],
        "There's no brief yet. I write one each morning from what your WebAgents did."
    );
    sqlx::query("INSERT INTO briefs VALUES ('site_1', '2026-11-08T04:00:00.000000Z', ?)")
        .bind(json!({ "headline": "Good morning", "lines": [{ "text": "3 orders overnight, KSh 8,400" }, { "text": "Mover finished the replay." }], "spend_words": "Models cost $0.42 today" }).to_string())
        .execute(&rig.ctx.pool)
        .await
        .unwrap();
    let brief = rig.turn("What happened overnight?", None).await;
    assert_eq!(
        brief["reply"],
        "Good morning. 3 orders overnight, KSh 8,400. Mover finished the replay. Models cost $0.42 today."
    );
    let page: Value = reqwest::get(format!("{}/api/voice/site_1/brief", rig.url))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["written"], true);

    let asked = rig
        .turn("How did the croissants sell yesterday?", None)
        .await;
    assert_eq!(asked["intent"], "ask");
    assert_eq!(
        asked["reply"],
        "Three orders came in overnight, KSh 8,400 in all."
    );
}

#[tokio::test]
async fn speech_streams_from_the_gateway() {
    let rig = rig().await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/voice/site_1/speak", rig.url))
        .json(&json!({ "text": "Good morning." }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert!(response.text().await.unwrap().contains("speech.audio.done"));
}

#[tokio::test]
async fn live_speech_is_relayed_both_ways_and_only_from_the_console() {
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let rig = rig().await;
    let url = format!(
        "{}/api/voice/site_1/live",
        rig.url.replacen("http://", "ws://", 1)
    );

    let mut foreign = url.clone().into_client_request().unwrap();
    foreign
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    match tokio_tungstenite::connect_async(foreign).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), 403)
        }
        other => panic!(
            "a foreign page opened the microphone relay: {:?}",
            other.map(|_| ())
        ),
    }

    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", "http://localhost:8080".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let mut events = Vec::new();
    let ready: Value = match socket.next().await.unwrap().unwrap() {
        Message::Text(t) => serde_json::from_str(t.as_str()).unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(ready["type"], "ready");
    socket
        .send(Message::binary(vec![0u8; 3_200]))
        .await
        .unwrap();
    socket
        .send(Message::text(json!({ "type": "commit" }).to_string()))
        .await
        .unwrap();
    while let Some(Ok(Message::Text(t))) = socket.next().await {
        let event: Value = serde_json::from_str(t.as_str()).unwrap();
        let done = event["type"] == "final";
        events.push(event);
        if done {
            break;
        }
    }
    assert_eq!(
        events,
        vec![
            json!({ "type": "partial", "text": "Deny" }),
            json!({ "type": "final", "text": "Deny." })
        ]
    );
}

fn app_context(owner: Uuid) -> AppContext {
    AppContext {
        site_id: "site_1".into(),
        site_name: "Kilele Bakery".into(),
        host: "kilele.example".into(),
        sample: false,
        account: owner.to_string(),
        owner_name: "Wanjiru".into(),
        utc_offset_minutes: 180,
        base: "/console/site_1/apps/pictures".into(),
        console: "/console/site_1".into(),
    }
}

#[tokio::test]
async fn the_console_apps_render_and_decide() {
    let rig = rig().await;
    let apps = console_apps(&rig.ctx);
    let ids: Vec<String> = apps.iter().map(|a| a.spec().id).collect();
    assert_eq!(ids, ["pictures", "talk"]);
    let ctx = app_context(rig.owner);

    let AppPage::Field { html, .. } = apps[1].page(&ctx, "", "").await.unwrap() else {
        panic!()
    };
    assert!(html.contains(r#"data-api="/api/voice/site_1""#));
    assert!(html.contains("never the recording"));

    let AppPage::Field { html, .. } = apps[0].page(&ctx, "", "").await.unwrap() else {
        panic!()
    };
    assert!(html.contains("No pictures yet."));
    assert!(html.contains("Approved pictures go out with your next release, which you sign."));

    // A draft, kept as the pipeline keeps one.
    sqlx::raw_sql(
        "INSERT INTO media_images (image_id, site_id, slot, kind, status, origin, digest, media_type, bytes, alt, label, \
         photographic, shows_person, shows_text, owner_check, brief, model, asked_by, created_at) VALUES \
         ('img_1', 'site_1', 'product:sourdough', 'product', 'draft', 'generated', 'sha256:00', 'image/webp', 10, \
          'A round loaf <b>', 'Made with AI, not a photo', 1, 0, 0, 'Seven pieces, not six.', 'A loaf', 'z-image-turbo', 'owner', \
          '2026-11-08T05:00:00.000000Z');",
    )
    .execute(&rig.ctx.pool)
    .await
    .unwrap();
    assert_eq!(apps[0].badge(&ctx).await.unwrap(), 1);
    let AppPage::Field { html, .. } = apps[0].page(&ctx, "", "").await.unwrap() else {
        panic!()
    };
    assert!(html.contains("1 picture waits for you."));
    assert!(
        html.contains("A round loaf &lt;b&gt;"),
        "alt text is escaped"
    );
    assert!(html.contains("/console/site_1/apps/pictures/approve/img_1"));
    assert!(html.contains("Seven pieces, not six."));

    let reply = apps[0]
        .post(&ctx, "approve/img_1", &HeaderMap::new(), Bytes::new())
        .await
        .unwrap();
    let AppReply::Redirect(to) = reply else {
        panic!("{reply:?}")
    };
    assert!(to.starts_with("/console/site_1/apps/pictures?said=Approved"));
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM media_images WHERE image_id = 'img_1'")
            .fetch_one(&rig.ctx.pool)
            .await
            .unwrap();
    assert_eq!(status, "approved");

    let refused = apps[0]
        .post(
            &ctx,
            "draw",
            &HeaderMap::new(),
            Bytes::from("kind=product&slot=owner&brief=A+portrait+of+me"),
        )
        .await
        .unwrap();
    let AppReply::Redirect(to) = refused else {
        panic!()
    };
    assert!(
        to.contains("never+show+people") || to.contains("never%20show%20people"),
        "{to}"
    );
    let _ = StatusCode::OK;
}

fn new_card(
    title: &str,
    level: agentrix_decisions::RiskLevel,
    approval: agentrix_decisions::ApprovalKind,
) -> agentrix_decisions::NewCard {
    agentrix_decisions::NewCard {
        kind: "order.release".into(),
        title: title.into(),
        summary: "Amina's usual Saturday order".into(),
        effects: vec!["The order goes to the bake list".into()],
        effect: agentrix_types::effect::EffectRequest {
            site: agentrix_types::SiteId::new("site_1"),
            webagent: "cashier".into(),
            tool: "order.release".into(),
            args: json!({ "order": title }),
            amount: None,
            expiry: chrono::Utc::now() + chrono::Duration::hours(6),
        },
        raised_by: agentrix_decisions::RaisedBy {
            webagent: "cashier".into(),
            job: None,
        },
        risk: agentrix_decisions::Risk {
            level,
            reasons: vec!["Returning customer".into()],
        },
        approval,
        back: None,
        dedupe_key: None,
    }
}

#[tokio::test]
async fn everyday_cards_are_allowed_once_by_voice_and_serious_ones_go_to_the_screen() {
    use agentrix_decisions::{ApprovalKind, Decisions, RiskLevel};
    let rig = rig().await;
    agentrix_decisions::migrate(&rig.ctx.pool).await.unwrap();
    let decisions = Decisions::new(&rig.ctx);
    let serious = decisions
        .raise(new_card(
            "Refund KSh 2,300 to Achieng",
            RiskLevel::Medium,
            ApprovalKind::Reviewed,
        ))
        .await
        .unwrap();
    let everyday = decisions
        .raise(new_card(
            "Release order KB-1042",
            RiskLevel::Low,
            ApprovalKind::OneTap,
        ))
        .await
        .unwrap();

    // With no card on screen, voice takes the newest waiting card.
    let next = rig.turn("What's next?", None).await;
    assert_eq!(
        next["reply"],
        "Cashier asks: Release order KB-1042. Low risk: Returning customer. Say allow once, or deny."
    );
    let allow = rig.turn("Allow once", None).await;
    assert_eq!(allow["action"]["do"], "allow");
    assert_eq!(allow["action"]["via"], "decide");
    assert_eq!(allow["action"]["card"], everyday.id.as_str());

    // A low-risk one-tap card that moves money still needs the screen (W5's surface rules).
    let mut paid = new_card(
        "Refund KSh 400 to Amina",
        RiskLevel::Low,
        ApprovalKind::OneTap,
    );
    paid.effect.amount = Some(agentrix_types::effect::Money {
        minor: 40_000,
        currency: "KES".into(),
    });
    let paid = decisions.raise(paid).await.unwrap();
    let money = rig
        .turn(
            "Allow once",
            Some(json!({ "id": paid.id, "title": paid.title, "risk": "low" })),
        )
        .await;
    assert_eq!(
        money["action"]["do"], "open_sheet",
        "voice never allows money"
    );

    let card = json!({ "id": serious.id, "title": serious.title, "risk": "medium" });
    let screen = rig.turn("allow it once", Some(card.clone())).await;
    assert_eq!(
        screen["action"]["do"], "open_sheet",
        "a reviewed card needs the screen"
    );
    let deny = rig.turn("Deny", Some(card)).await;
    assert_eq!(deny["action"]["via"], "decide");

    let http = reqwest::Client::new();
    let decided = http
        .post(format!("{}/api/voice/site_1/decide", rig.url))
        .json(&json!({ "card": everyday.id, "allow": true, "heard": "Allow once." }))
        .send()
        .await
        .unwrap();
    assert_eq!(decided.status(), 200);
    let decided: Value = decided.json().await.unwrap();
    assert!(decided["receipt"].as_str().unwrap().starts_with("sha256:"));
    let refused = http
        .post(format!("{}/api/voice/site_1/decide", rig.url))
        .json(&json!({ "card": serious.id, "allow": true, "heard": "Allow once." }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        403,
        "decide_spoken refuses a reviewed card"
    );
    let declined = http
        .post(format!("{}/api/voice/site_1/decide", rig.url))
        .json(&json!({ "card": serious.id, "allow": false, "heard": "Deny." }))
        .send()
        .await
        .unwrap();
    assert_eq!(declined.status(), 200);
    let notes = rig.notes().await;
    let dids: Vec<&str> = notes.iter().map(|n| n.2.as_str()).collect();
    assert!(
        dids.ends_with(&["allowed once", "refused", "declined"]),
        "{dids:?}"
    );
}
