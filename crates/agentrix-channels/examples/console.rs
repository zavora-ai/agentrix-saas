//! Settings → Channels inside W5's shell, on the shell's sample bakery, with a stand-in Bot API,
//! for checking it in a browser:
//! `scripts/wcargo run -p agentrix-channels --example console -- 19302`, then open
//! <http://localhost:19302/console/site_kilele_sample/apps/channels>.
//!
//! Tap Link Telegram, then open `/_preview/open-link` to play the owner tapping Start in
//! Telegram; reload Channels to see the link. `/_preview/sent` lists what the bot would have sent.
//! Nothing here talks to Telegram or a real node.

use std::sync::{Arc, Mutex};

use agentrix_channels::config::{BotConfig, Token};
use agentrix_channels::service::{Channels, Settings};
use agentrix_channels::telegram::{Client, Update};
use agentrix_host::testing::{FixedOwner, context};
use agentrix_shell::{Shell, ShellState, sample::SamplePort, store::ShellStore};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

const OWNER: &str = "0190f5c2-0000-7000-8000-000000000001";
const TOKEN: &str = "100:preview-token";

type Sent = Arc<Mutex<Vec<(String, Value)>>>;

async fn bot_api(
    State(sent): State<Sent>,
    Path((_bot, method)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let n = {
        let mut s = sent.lock().unwrap();
        s.push((method.clone(), body));
        s.len()
    };
    Json(match method.as_str() {
        "getMe" => {
            json!({ "ok": true, "result": { "id": 1, "username": "KileleBakerySampleBot", "first_name": "Kilele Bakery (Sample)" } })
        }
        "sendMessage" => json!({ "ok": true, "result": { "message_id": n } }),
        _ => json!({ "ok": true, "result": true }),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(19302);
    let site = agentrix_shell::sample::SITE;

    // A stand-in Bot API.
    let sent: Sent = Arc::default();
    let api = Router::new()
        .route("/{bot}/{method}", post(bot_api))
        .with_state(sent.clone());
    let api_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let api_addr = api_listener.local_addr()?;
    tokio::spawn(async move { axum::serve(api_listener, api).await });

    let account: uuid::Uuid = OWNER.parse()?;
    let mut ctx = context().await?;
    ctx.owners = Arc::new(FixedOwner::passkey(account));
    ctx.origin = format!("http://localhost:{port}").parse()?;
    sqlx::raw_sql(
        "CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL, display_name TEXT NOT NULL);
         CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', utc_offset_minutes INTEGER NOT NULL DEFAULT 0, owner_account_id TEXT, created_at TEXT NOT NULL DEFAULT '');
         CREATE TABLE briefs (site_id TEXT NOT NULL, written_at TEXT NOT NULL, body TEXT NOT NULL);",
    )
    .execute(&ctx.pool)
    .await?;
    sqlx::query("INSERT INTO accounts VALUES (?, 'wanjiru@example.com', 'Wanjiru Kamau')")
        .bind(OWNER)
        .execute(&ctx.pool)
        .await?;
    sqlx::query("INSERT INTO sites (site_id, domain, name, utc_offset_minutes, owner_account_id) VALUES (?, 'kilele-bakery-sample.agentrix.website', 'Kilele Bakery', 180, ?)")
        .bind(site)
        .bind(OWNER)
        .execute(&ctx.pool)
        .await?;
    agentrix_decisions::migrate(&ctx.pool).await?;
    agentrix_messages::migrate(&ctx.pool).await?;
    agentrix_channels::migrate(&ctx.pool).await?;
    let shell_store = ShellStore::open(ctx.pool.clone()).await?;

    let bot = Channels::bot(
        BotConfig {
            site: site.into(),
            host: Some("kilele-bakery-sample.agentrix.website".into()),
            token: Token::new(TOKEN),
            api: None,
        },
        Client::with_base(
            ctx.http.clone(),
            Token::new(TOKEN),
            &format!("http://{api_addr}"),
        ),
    );
    let me = bot.client.get_me().await?;
    bot.set_me(me.username, me.first_name);
    let channels = Channels::with_bots(
        &ctx,
        vec![bot.clone()],
        agentrix_messages::from_context(&ctx),
        Settings::default(),
    );
    agentrix_messages::register_chat_transport(
        agentrix_channels::transport::TelegramTransport::new(channels.clone()),
    );
    let app = Arc::new(agentrix_channels::app::ChannelsApp::new(channels.clone()));

    let shell = agentrix_shell::router(ShellState {
        shell: Arc::new(Shell::new()?),
        port: Arc::new(SamplePort::default()),
        store: shell_store,
        apps: vec![app],
        lanes: Vec::new(),
        freeze_enabled: false,
    });

    // Play the owner tapping Start on the newest deep link the bot sent to the page.
    let (ch, b) = (channels.clone(), bot.clone());
    let preview = Router::new()
        .route(
            "/_preview/open-link",
            get(move || {
                let (ch, b) = (ch.clone(), b.clone());
                async move {
                    // The page keeps the code in memory only; read it back from the page itself.
                    let page = reqwest::get(format!(
                        "{}/console/{}/apps/channels",
                        ch.origin(),
                        agentrix_shell::sample::SITE
                    ))
                    .await
                    .ok();
                    let html = match page {
                        Some(r) => r.text().await.unwrap_or_default(),
                        None => String::new(),
                    };
                    let marker = "https://t.me/KileleBakerySampleBot?start=";
                    let Some(at) = html.find(marker) else {
                        return "Tap Link Telegram on the Channels page first.".into_response();
                    };
                    let code: String = html[at + marker.len()..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                        .collect();
                    let update: Update = serde_json::from_value(json!({
                        "update_id": 1,
                        "message": { "message_id": 1, "chat": { "id": 555, "type": "private" }, "from": { "id": 555, "is_bot": false, "first_name": "Wanjiru", "username": "wanjiru_k" }, "text": format!("/start {code}") }
                    }))
                    .expect("an update");
                    agentrix_channels::bot::handle(&ch, &b, update).await;
                    Redirect::to(&format!("/console/{}/apps/channels", agentrix_shell::sample::SITE)).into_response()
                }
            }),
        )
        .route(
            "/_preview/sent",
            get(move || {
                let sent = sent.clone();
                async move { Json(json!(sent.lock().unwrap().clone())) }
            }),
        );

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("http://localhost:{port}/console/{site}/apps/channels");
    axum::serve(listener, shell.merge(preview).merge(agentrix_ui::assets())).await?;
    Ok(())
}
