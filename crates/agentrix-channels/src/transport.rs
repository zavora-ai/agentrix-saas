//! W9's chat transport for Telegram: a catalogue message recorded in the outbox, addressed to
//! `telegram:<chat id>`, goes out through the site's bot with the buttons the rule table allows.
//! Telegram's `message_id` becomes the outbox row's provider reference.

use std::sync::Arc;

use agentrix_messages::{Channel, Delivery, Envelope, Readiness, Transport};
use async_trait::async_trait;
use chrono::Utc;

use crate::bot::{html_for, keyboard_for, send_raw};
use crate::service::{Channels, TELEGRAM};

/// Delivers [`Channel::Chat`] envelopes addressed to Telegram chats.
pub struct TelegramTransport {
    channels: Channels,
}

impl TelegramTransport {
    /// Over the channels service.
    pub fn new(channels: Channels) -> Arc<Self> {
        Arc::new(Self { channels })
    }
}

#[async_trait]
impl Transport for TelegramTransport {
    fn readiness(&self, channel: Channel) -> Readiness {
        match channel {
            Channel::Chat if !self.channels.bots().is_empty() => Readiness::Ready,
            Channel::Chat => Readiness::NotReady("No chat is connected yet".into()),
            _ => Readiness::NotReady("Telegram carries chat messages only".into()),
        }
    }

    async fn deliver(&self, envelope: &Envelope) -> Delivery {
        let Some(chat) = envelope
            .to
            .strip_prefix("telegram:")
            .and_then(|c| c.parse::<i64>().ok())
        else {
            return Delivery::NotSent {
                reason: "WhatsApp isn't connected yet".into(),
            };
        };
        let bot = envelope
            .site_id
            .as_deref()
            .and_then(|s| self.channels.bot_for(s))
            .or_else(|| self.channels.bots().first());
        let Some(bot) = bot else {
            return Delivery::NotSent {
                reason: "No Telegram bot is set up for this site".into(),
            };
        };
        let (html, keyboard) = match &envelope.chat {
            Some(c) => (html_for(&c.text), keyboard_for(c.link.as_ref(), &c.replies)),
            None => (html_for(&envelope.text), Vec::new()),
        };
        match send_raw(bot, chat, &html, &keyboard).await {
            Ok(id) => Delivery::Sent {
                provider: TELEGRAM.into(),
                provider_ref: id.to_string(),
            },
            Err(e) if e.code() == Some(403) => {
                // The person blocked the bot: the link ends, as if they had sent /stop.
                if let Ok(Some(link)) = self
                    .channels
                    .store()
                    .link_for_chat(TELEGRAM, &bot.cfg.site, &chat.to_string())
                    .await
                    && self
                        .channels
                        .store()
                        .unlink(&link.id, "blocked", Utc::now())
                        .await
                        .unwrap_or(false)
                {
                    self.channels
                        .receipt_channel(&link.site_id, &link.account_id, "unlinked", "blocked")
                        .await;
                }
                Delivery::Failed {
                    reason: "You blocked the bot in Telegram, so the chat is unlinked".into(),
                }
            }
            Err(e) => Delivery::Failed {
                reason: match e {
                    crate::telegram::TgError::Api {
                        retry_after: Some(s),
                        ..
                    } => {
                        format!("Telegram asked us to wait {s} seconds")
                    }
                    crate::telegram::TgError::Api { description, .. } => {
                        format!("Telegram refused it: {description}")
                    }
                    crate::telegram::TgError::Network { .. } => {
                        "Telegram couldn't be reached".into()
                    }
                },
            },
        }
    }
}
