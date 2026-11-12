//! Telegram end to end against a fake Bot API and a fake site Ask: the owner links with a one-time
//! deep link, gets the brief and a decision alert through W9's outbox, declines one card in chat,
//! can't approve another from chat (it opens `/d/{card}`), freezes with STOP and unlinks with
//! `/stop`; a customer gets the site's own Ask answer. The token never reaches the outbox or a
//! receipt.

use std::sync::{Arc, Mutex};

use agentrix_channels::app::ChannelsApp;
use agentrix_channels::config::{BotConfig, Token};
use agentrix_channels::service::{Channels, Settings};
use agentrix_channels::telegram::{Client, Update};
use agentrix_channels::watch::Watcher;
use agentrix_decisions::{ApprovalKind, CardState, NewCard, RaisedBy, Risk, RiskLevel};
use agentrix_host::testing::{FixedOwner, MemoryReceipts, context};
use agentrix_shell::apps::{AppContext, AppPage, ConsoleApp};
use agentrix_types::{EffectRequest, SiteId};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::post;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

const SITE: &str = "kilele-bakery-sample";
const TOKEN: &str = "424242:SECRET-TOKEN-never-printed";
const OWNER_CHAT: i64 = 555;
const CUSTOMER_CHAT: i64 = 777;

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<(String, Value)>>,
    asked: Mutex<Vec<(String, Value)>>,
}

impl Fake {
    fn sent_to(&self, chat: i64) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, b)| {
                (m == "sendMessage" || m == "editMessageText") && b["chat_id"] == json!(chat)
            })
            .map(|(_, b)| b.clone())
            .collect()
    }
    fn last_text(&self, chat: i64) -> String {
        self.sent_to(chat)
            .last()
            .map(|b| b["text"].as_str().unwrap_or_default().to_owned())
            .unwrap_or_default()
    }
}

async fn bot_api(
    State(fake): State<Arc<Fake>>,
    Path((_bot, method)): Path<(String, String)>,
    body: axum::Json<Value>,
) -> axum::Json<Value> {
    let n = {
        let mut calls = fake.calls.lock().unwrap();
        calls.push((method.clone(), body.0.clone()));
        calls.len()
    };
    axum::Json(match method.as_str() {
        "getMe" => {
            json!({ "ok": true, "result": { "id": 1, "username": "KileleTestBot", "first_name": "Kilele Bakery (Sample)" } })
        }
        "sendMessage" => json!({ "ok": true, "result": { "message_id": 1000 + n } }),
        "editMessageText" => json!({ "ok": true, "result": { "message_id": 1 } }),
        _ => json!({ "ok": true, "result": true }),
    })
}

async fn site_ask(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
    body: axum::Json<Value>,
) -> axum::response::Response {
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_owned();
    fake.asked.lock().unwrap().push((host, body.0.clone()));
    let answer = json!({
        "desk": { "id": "cakes", "name": "Cakes desk" },
        "handover": null,
        "phrases": ["Yes. A **2-tier cake** serves 10 to 12 people [1]."],
        "sources": [{ "n": 1, "label": "Cakes · 2 tiers", "href": "/cakes#two-tier", "lock": false }],
        "actions": [{ "label": "See cakes", "primary": true, "kind": "link", "href": "/cakes" }],
        "meta": "Answered from this site's pages",
        "waiting": false
    });
    let sse = format!(
        "event: progress\ndata: {{\"label\":\"Searching this site\"}}\n\nevent: answer\ndata: {answer}\n\nevent: done\ndata: {{}}\n\n"
    );
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        sse,
    )
        .into_response_compat()
}

trait Compat {
    fn into_response_compat(self) -> axum::response::Response;
}
impl<T: axum::response::IntoResponse> Compat for T {
    fn into_response_compat(self) -> axum::response::Response {
        self.into_response()
    }
}

fn update(id: i64, chat: i64, text: &str) -> Update {
    serde_json::from_value(json!({
        "update_id": id,
        "message": { "message_id": id, "chat": { "id": chat, "type": "private" }, "from": { "id": chat, "is_bot": false, "first_name": "James", "username": "jk" }, "text": text }
    }))
    .unwrap()
}

fn tap(id: i64, chat: i64, data: &str, text: &str) -> Update {
    serde_json::from_value(json!({
        "update_id": id,
        "callback_query": { "id": format!("cb{id}"), "from": { "id": chat, "is_bot": false, "first_name": "James" }, "data": data,
          "message": { "message_id": 9000 + id, "chat": { "id": chat, "type": "private" }, "text": text } }
    }))
    .unwrap()
}

fn new_card(title: &str, key: &str) -> NewCard {
    NewCard {
        kind: "webagent.add".into(),
        title: title.into(),
        summary: "One line".into(),
        effects: vec!["The Sales WebAgent goes on duty".into()],
        effect: EffectRequest {
            site: SiteId::new(SITE),
            webagent: "chief".into(),
            tool: "webagent.add".into(),
            args: json!({ "webagent": "sales", "key": key }),
            amount: None,
            expiry: Utc::now() + Duration::hours(8),
        },
        raised_by: RaisedBy {
            webagent: "chief".into(),
            job: None,
        },
        risk: Risk {
            level: RiskLevel::Medium,
            reasons: vec!["Adds a WebAgent".into()],
        },
        approval: ApprovalKind::Reviewed,
        back: None,
        dedupe_key: Some(key.into()),
    }
}

#[tokio::test]
async fn telegram_end_to_end() {
    // A fake Bot API and a fake site Ask on one local server.
    let fake = Arc::new(Fake::default());
    let app = Router::new()
        .route("/{bot}/{method}", post(bot_api))
        .route("/ask", post(site_ask))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // Node's context with Kilele's site owned by James.
    let owner = Uuid::now_v7();
    let mut ctx = context().await.unwrap();
    let receipts = Arc::new(MemoryReceipts::default());
    ctx.receipts = receipts.clone();
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    sqlx::raw_sql(
        "CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL, display_name TEXT NOT NULL);
         CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', utc_offset_minutes INTEGER NOT NULL DEFAULT 0, owner_account_id TEXT, created_at TEXT NOT NULL DEFAULT '');
         CREATE TABLE briefs (site_id TEXT NOT NULL, written_at TEXT NOT NULL, body TEXT NOT NULL);",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO accounts VALUES (?, 'james@example.com', 'James Karanja')")
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sites (site_id, domain, name, utc_offset_minutes, owner_account_id) VALUES (?, 'kilele-bakery-sample.localhost', 'Kilele Bakery', 180, ?)")
        .bind(SITE)
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO briefs VALUES (?, ?, ?)")
        .bind(SITE)
        .bind(Utc::now().to_rfc3339())
        .bind(json!({ "headline": "Overnight: 41 visitors, 9 questions answered.", "lines": [{ "text": "The Guardian blocked 212 probes at 01:30.", "receipt": null }] }).to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    agentrix_decisions::migrate(&ctx.pool).await.unwrap();
    agentrix_messages::migrate(&ctx.pool).await.unwrap();
    agentrix_shell::migrate(&ctx.pool).await.unwrap();
    // Quiet hours default to 22:00–06:00 and would hold the brief when this runs at night, so
    // put them six hours away from now in the site's time (UTC+3).
    let quiet_start = (Utc::now() + chrono::Duration::minutes(180 + 360)).time();
    let quiet_end = quiet_start + chrono::Duration::minutes(1);
    sqlx::query("INSERT INTO shell_quiet_hours (site_id, start_hm, end_hm, updated_at, updated_by) VALUES (?, ?, ?, ?, 'test')")
        .bind(SITE)
        .bind(quiet_start.format("%H:%M").to_string())
        .bind(quiet_end.format("%H:%M").to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(&ctx.pool)
        .await
        .unwrap();
    agentrix_channels::migrate(&ctx.pool).await.unwrap();

    let client = Client::with_base(
        ctx.http.clone(),
        Token::new(TOKEN),
        &format!("http://{addr}"),
    );
    let bot = Channels::bot(
        BotConfig {
            site: SITE.into(),
            host: None,
            token: Token::new(TOKEN),
            api: None,
        },
        client,
    );
    bot.set_me("KileleTestBot".into(), "Kilele Bakery (Sample)".into());
    let node_settings = Settings {
        loopback: Some(format!("http://{addr}")),
        freeze_enforced: false,
    };
    let channels = Channels::with_bots(
        &ctx,
        vec![bot.clone()],
        agentrix_messages::from_context(&ctx),
        node_settings.clone(),
    );
    assert!(agentrix_messages::register_chat_transport(
        agentrix_channels::transport::TelegramTransport::new(channels.clone())
    ));

    // 1. Settings → Channels makes a one-time deep link.
    let settings = ChannelsApp::new(channels.clone());
    let actx = AppContext {
        site_id: SITE.into(),
        site_name: "Kilele Bakery".into(),
        host: "kilele-bakery-sample.localhost".into(),
        sample: true,
        account: owner.to_string(),
        owner_name: "James".into(),
        utc_offset_minutes: 180,
        base: format!("/console/{SITE}/apps/channels"),
        console: format!("/console/{SITE}"),
    };
    settings
        .post(
            &actx,
            "telegram/link",
            &HeaderMap::new(),
            Default::default(),
        )
        .await
        .unwrap();
    let AppPage::Field { html, .. } = settings.page(&actx, "", "").await.unwrap() else {
        panic!("no page")
    };
    let start = html
        .find("https://t.me/KileleTestBot?start=")
        .expect("a deep link");
    let code: String = html[start + "https://t.me/KileleTestBot?start=".len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    assert!(code.len() >= 20);

    // A wrong code links nothing.
    agentrix_channels::bot::handle(&channels, &bot, update(1, OWNER_CHAT, "/start not-a-code"))
        .await;
    assert!(
        fake.last_text(OWNER_CHAT)
            .contains("expired or was already used")
    );

    // 2. /start <code> links the chat, writes channel.changed and sends the latest brief.
    agentrix_channels::bot::handle(
        &channels,
        &bot,
        update(2, OWNER_CHAT, &format!("/start {code}")),
    )
    .await;
    let link = channels
        .store()
        .link_for_chat("telegram", SITE, &OWNER_CHAT.to_string())
        .await
        .unwrap()
        .expect("linked");
    assert_eq!(link.account_id, owner.to_string());
    let texts: Vec<String> = fake
        .sent_to(OWNER_CHAT)
        .iter()
        .map(|b| b["text"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(texts.iter().any(|t| t.contains("Linked")), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("Overnight: 41 visitors")),
        "{texts:?}"
    );
    // The same code can't link twice.
    agentrix_channels::bot::handle(&channels, &bot, update(3, 999, &format!("/start {code}")))
        .await;
    assert!(
        channels
            .store()
            .link_for_chat("telegram", SITE, "999")
            .await
            .unwrap()
            .is_none()
    );

    // 3. A new card → one alert through the outbox, with Open (moved into the text: localhost) and Decline.
    let d = channels.decisions();
    let first = d.raise(new_card("Put Sales on duty", "k1")).await.unwrap();
    let watcher = Watcher::default();
    watcher.tick(&channels).await;
    watcher.tick(&channels).await;
    let alerts: Vec<Value> = fake
        .sent_to(OWNER_CHAT)
        .into_iter()
        .filter(|b| {
            b["text"]
                .as_str()
                .unwrap_or_default()
                .contains("Put Sales on duty")
        })
        .collect();
    assert_eq!(alerts.len(), 1, "one alert per card");
    let alert = &alerts[0];
    assert!(
        alert["text"]
            .as_str()
            .unwrap()
            .contains(&format!("http://localhost:8080/d/{}", first.id))
    );
    assert_eq!(
        alert["reply_markup"]["inline_keyboard"][0][0]["callback_data"],
        json!(format!("dec:{}", first.id))
    );
    assert!(!alert.to_string().to_lowercase().contains("\"approve\""));
    let (status, provider): (String, Option<String>) = sqlx::query_as(
        "SELECT status, provider FROM message_outbox WHERE message = 'owner.decision_waiting'",
    )
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), provider.as_deref()),
        ("sent", Some("telegram"))
    );

    // 4. Decline in chat → W5's contract records it as a chat decline.
    agentrix_channels::bot::handle(
        &channels,
        &bot,
        tap(
            4,
            OWNER_CHAT,
            &format!("dec:{}", first.id),
            "Put Sales on duty",
        ),
    )
    .await;
    let row = d.get(SITE, &first.id).await.unwrap().unwrap();
    assert_eq!(row.state, CardState::Declined);
    assert_eq!(row.method.as_deref(), Some("chat"));
    assert!(
        fake.last_text(OWNER_CHAT)
            .contains("Declined. Nothing changed.")
    );

    // 5. "approve" never approves: it points at the console.
    let second = d
        .raise(new_card("Put Reputation on duty", "k2"))
        .await
        .unwrap();
    agentrix_channels::bot::handle(&channels, &bot, update(5, OWNER_CHAT, "approve")).await;
    assert!(fake.last_text(OWNER_CHAT).contains("never in chat"));
    assert!(
        fake.last_text(OWNER_CHAT)
            .contains(&format!("/d/{}", second.id))
    );
    assert_eq!(
        d.get(SITE, &second.id).await.unwrap().unwrap().state,
        CardState::Waiting
    );

    // 7. A customer gets the site's own Ask answer, asked on the site's host.
    agentrix_channels::bot::handle(
        &channels,
        &bot,
        update(7, CUSTOMER_CHAT, "Do you have cakes for 10 people?"),
    )
    .await;
    let reply = fake.last_text(CUSTOMER_CHAT);
    assert!(reply.contains("Cakes desk"), "{reply}");
    assert!(reply.contains("<b>2-tier cake</b>"), "{reply}");
    assert!(reply.contains("Answered from this site's pages"));
    let asked = fake.asked.lock().unwrap().clone();
    assert_eq!(asked[0].0, "kilele-bakery-sample.localhost");
    assert_eq!(asked[0].1["q"], json!("Do you have cakes for 10 people?"));

    // 8. STOP: honest while Freeze isn't enforced, then a real Freeze and its alert.
    agentrix_channels::bot::handle(&channels, &bot, update(8, OWNER_CHAT, "STOP")).await;
    assert!(fake.last_text(OWNER_CHAT).contains("nothing was stopped"));
    let enforced = Channels::with_bots(
        &ctx,
        vec![bot.clone()],
        agentrix_messages::from_context(&ctx),
        Settings {
            freeze_enforced: true,
            ..node_settings
        },
    );
    agentrix_channels::bot::handle(&enforced, &bot, update(9, OWNER_CHAT, "stop")).await;
    assert!(
        fake.last_text(OWNER_CHAT)
            .contains("Every WebAgent is frozen")
    );
    let f = channels
        .shell()
        .freeze(SITE)
        .await
        .unwrap()
        .expect("frozen");
    assert_eq!(f.how, "chat");
    watcher.tick(&channels).await;
    assert!(
        fake.sent_to(OWNER_CHAT)
            .iter()
            .any(|b| b["text"].as_str().unwrap_or_default().contains("is frozen"))
    );

    // 9. /stop unlinks, with a receipt for the link and the unlink.
    agentrix_channels::bot::handle(&channels, &bot, update(10, OWNER_CHAT, "/stop")).await;
    assert!(
        channels
            .store()
            .link_for_chat("telegram", SITE, &OWNER_CHAT.to_string())
            .await
            .unwrap()
            .is_none()
    );
    let kinds: Vec<String> = receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str().to_owned())
        .collect();
    assert_eq!(
        kinds.iter().filter(|k| *k == "channel.changed").count(),
        2,
        "{kinds:?}"
    );

    // The token is nowhere a person or a log could read it.
    let outbox: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT body_text, payload FROM message_outbox")
            .fetch_all(&ctx.pool)
            .await
            .unwrap();
    for (body, payload) in outbox {
        assert!(!body.contains("SECRET") && !payload.unwrap_or_default().contains("SECRET"));
    }
    for (json, _) in receipts.issued.lock().unwrap().iter() {
        assert!(!json.contains("SECRET"));
    }
}
