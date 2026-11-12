//! Platform-host routes: Telegram's webhook. The `/d/{card}` short link chats carry is W5's
//! (`agentrix-shell`), built with `agentrix_decisions::surface::short_link`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;

use crate::service::Channels;
use crate::telegram::Update;

/// The routes.
pub fn router(channels: Channels) -> Router {
    Router::new()
        .route("/channels/telegram/{site}/webhook", post(webhook))
        .with_state(channels)
}

/// Constant-time comparison for the webhook secret.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// `POST /channels/telegram/{site}/webhook`: Telegram's call, checked against the secret derived
/// from the bot's token, handled in the background so Telegram gets its 200 at once.
async fn webhook(
    State(channels): State<Channels>,
    Path(site): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(bot) = channels.bot_for(&site).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let given = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !same(given, &bot.cfg.token.webhook_secret()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(update) = serde_json::from_slice::<Update>(&body) else {
        return StatusCode::OK.into_response();
    };
    // Once per update id: the cursor only moves forward.
    let seen = channels.store().cursor(&site).await.unwrap_or(0);
    if update.update_id < seen {
        return StatusCode::OK.into_response();
    }
    let _ = channels
        .store()
        .set_cursor(&site, update.update_id + 1, chrono::Utc::now())
        .await;
    let ch = channels.clone();
    let bot: Arc<_> = bot;
    tokio::spawn(async move { crate::bot::handle(&ch, &bot, update).await });
    StatusCode::OK.into_response()
}
