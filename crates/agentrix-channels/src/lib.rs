//! Chat channels for owners and customers (W13, `specs/platform/13-mobile-and-channels.md`).
//!
//! - **Telegram** ([`bot`]): one bot per site from a credential file only node can read
//!   ([`config`]). An owner links their chat from Settings → Channels ([`app`]) with a one-time
//!   deep link; the chat then gets the morning brief, "a decision is waiting" and questions the
//!   desks pass on ([`watch`]), all through W9's catalogue and outbox ([`transport`]). From the
//!   chat the owner can ask, decline, pause and STOP; approving always opens the card in the
//!   console through `/d/{card}` ([`routes`]), because chat never approves (ADR-014, 07 principle
//!   9). Any other chat is a customer and gets the site's own Ask ([`customer`]).
//! - **What a chat offers** ([`rules`]), from W5's one rule table.
//! - **The owner app's API** ([`mobile`]): `/api/m/v1/…`, phones paired from Settings → Channels.
//!
//! Locally node long-polls `getUpdates`; on a deploy `AGENTRIX_TELEGRAM_MODE=webhook` points
//! Telegram at `POST /channels/telegram/{site}/webhook`.

pub mod app;
pub mod bot;
pub mod chief;
pub mod config;
pub mod customer;
pub mod mobile;
pub mod routes;
pub mod rules;
pub mod service;
pub mod store;
pub mod telegram;
pub mod transport;
pub mod watch;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use agentrix_host::{Mounted, NodeContext};
use futures::future::BoxFuture;

pub use service::Channels;

/// Runs this crate's migrations (W13's range, 1300–1399) with their own table.
pub fn migrate(pool: &sqlx::SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_channels");
        m.run(pool)
            .await
            .map_err(|e| anyhow::anyhow!("agentrix-channels migrations: {e}"))?;
        Ok(())
    })
}

static SHARED: OnceLock<Channels> = OnceLock::new();

/// The process's channels service: [`mount`] and [`console_app`] share one, so the Channels page
/// sees the bot `getMe` named. Reads the credential file once.
pub fn shared(ctx: &NodeContext) -> Channels {
    SHARED
        .get_or_init(|| {
            let config = match config::path(&ctx.data_dir) {
                None => None,
                Some(p) => match config::load(&p) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("{e}");
                        None
                    }
                },
            };
            if config.is_none() {
                tracing::info!("Telegram is off: no bot file (AGENTRIX_TELEGRAM_FILE)");
            }
            Channels::new(ctx, config.as_ref())
        })
        .clone()
}

/// The channels an owner of `site` can connect today, by W4's ids (`ChannelOffer`): `telegram`
/// when node has a bot for the site. Email is always on and isn't "connected"; device, WhatsApp,
/// Slack, Discord and Matrix aren't ready yet.
pub fn connectable(ctx: &NodeContext, site: &str) -> Vec<&'static str> {
    let ch = shared(ctx);
    let mut out = Vec::new();
    if ch.bot_for(site).is_some() {
        out.push("telegram");
    }
    out
}

/// Settings → Channels for W5's shell: node passes it with the other apps.
pub fn console_app(ctx: &NodeContext) -> Arc<dyn agentrix_shell::apps::ConsoleApp> {
    Arc::new(app::ChannelsApp::new(shared(ctx)))
}

/// [`mount_with`] and no effect runners: chat declines still work; approving from the owner app
/// is refused until node passes its runners.
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    mount_with(ctx, Vec::new())
}

/// Mounts the owner app's API and Telegram's webhook, registers the Telegram chat transport with
/// W9, and starts each bot (polling or webhook) and the watcher. `runners` are every crate's
/// effect runners (node's `effect_runners`), so an approval from the app runs the card's effect.
pub fn mount_with(
    ctx: &NodeContext,
    runners: Vec<Arc<dyn agentrix_decisions::EffectRunner>>,
) -> anyhow::Result<Mounted> {
    let channels = shared(ctx);
    channels.set_runners(runners);
    let mut mounted = Mounted::default()
        .platform(routes::router(channels.clone()))
        .platform(mobile::router(channels.clone()));
    if channels.bots().is_empty() {
        return Ok(mounted);
    }
    agentrix_messages::register_chat_transport(transport::TelegramTransport::new(channels.clone()));
    let webhook = std::env::var("AGENTRIX_TELEGRAM_MODE").as_deref() == Ok("webhook");
    for bot in channels.bots().iter().cloned() {
        let ch = channels.clone();
        mounted = mounted.task(
            "channels: a Telegram bot",
            Box::pin(async move { run_bot(ch, bot, webhook).await }),
        );
    }
    let ch = channels.clone();
    mounted = mounted.task(
        "channels: alerts, the morning brief and questions to linked chats",
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let watcher = watch::Watcher::default();
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                watcher.tick(&ch).await;
                n += 1;
                if n % 240 == 1
                    && let Err(e) = ch.store().tidy(chrono::Utc::now()).await
                {
                    tracing::warn!(error = %e, "couldn't tidy chat turns");
                }
            }
        }),
    );
    Ok(mounted)
}

/// Learns the bot's name, sets its menu, then polls (or sets the webhook) for as long as node runs.
async fn run_bot(channels: Channels, bot: Arc<service::Bot>, webhook: bool) {
    let site = bot.cfg.site.clone();
    let mut wait = Duration::from_secs(2);
    loop {
        match bot.client.get_me().await {
            Ok(me) => {
                tracing::info!(site = %site, bot = %me.username, "Telegram bot ready");
                bot.set_me(me.username, me.first_name);
                break;
            }
            Err(e) if matches!(e.code(), Some(401 | 404)) => {
                tracing::error!(site = %site, "Telegram refused the bot token; check the bot file. Telegram stays off for this site.");
                return;
            }
            Err(e) => {
                tracing::warn!(site = %site, error = %e, "Telegram not reachable yet");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(120));
            }
        }
    }
    if let Err(e) = bot.client.set_commands(bot::COMMANDS).await {
        tracing::warn!(site = %site, error = %e, "couldn't set the bot's menu");
    }
    if webhook {
        let url = format!("{}/channels/telegram/{site}/webhook", channels.origin());
        match bot
            .client
            .set_webhook(&url, &bot.cfg.token.webhook_secret())
            .await
        {
            Ok(_) => tracing::info!(site = %site, "Telegram webhook set"),
            Err(e) => {
                tracing::error!(site = %site, error = %e, "couldn't set the Telegram webhook")
            }
        }
        return;
    }
    if let Err(e) = bot.client.delete_webhook().await {
        tracing::warn!(site = %site, error = %e, "couldn't clear a webhook before polling");
    }
    poll(channels, bot).await;
}

async fn poll(channels: Channels, bot: Arc<service::Bot>) {
    let site = bot.cfg.site.clone();
    let mut warned_conflict = false;
    loop {
        let offset = channels.store().cursor(&site).await.unwrap_or(0);
        match bot.client.get_updates(offset, 25).await {
            Ok(updates) => {
                warned_conflict = false;
                for u in updates {
                    // The cursor moves before the update is handled, so a crash never repeats an
                    // action; a lost message is safer than a repeated one.
                    if let Err(e) = channels
                        .store()
                        .set_cursor(&site, u.update_id + 1, chrono::Utc::now())
                        .await
                    {
                        tracing::error!(error = %e, "couldn't move the Telegram cursor");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        break;
                    }
                    let ch = channels.clone();
                    let b = bot.clone();
                    tokio::spawn(async move { bot::handle(&ch, &b, u).await });
                }
            }
            Err(e) if e.code() == Some(409) => {
                if !warned_conflict {
                    tracing::warn!(site = %site, "another process is reading this bot's updates (or a webhook is set); waiting");
                    warned_conflict = true;
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Err(telegram::TgError::Api {
                retry_after: Some(s),
                ..
            }) => tokio::time::sleep(Duration::from_secs(s.max(1))).await,
            Err(e) if matches!(e.code(), Some(401 | 404)) => {
                tracing::error!(site = %site, "Telegram refused the bot token while polling; stopping");
                return;
            }
            Err(e) => {
                tracing::debug!(site = %site, error = %e, "poll failed; retrying");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
