//! One transport per channel: email (Resend, or nothing yet), push (later) and chat (whatever
//! a channel crate registers, Telegram first for Kilele).
//!
//! W13 registers its chat transport once at mount with [`register_chat_transport`]. A
//! [`crate::Messages`] built before that (node mounts W13 last) still finds it, because
//! [`ChannelRouter::new`] looks the registration up at each delivery. Without one, chat stays
//! [`Undelivered`] and messages are kept as "Not sent".

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::catalogue::Channel;
use crate::send::{Delivery, Envelope, Readiness, Transport, Undelivered};

static CHAT: OnceLock<Arc<dyn Transport>> = OnceLock::new();

/// Registers the process's chat transport. The first registration wins; returns false if one
/// was already registered.
pub fn register_chat_transport(transport: Arc<dyn Transport>) -> bool {
    CHAT.set(transport).is_ok()
}

/// The registered chat transport, if any.
pub fn chat_transport() -> Option<Arc<dyn Transport>> {
    CHAT.get().cloned()
}

/// Sends each envelope to its channel's transport.
pub struct ChannelRouter {
    /// Email.
    pub email: Arc<dyn Transport>,
    /// Phone notifications.
    pub push: Arc<dyn Transport>,
    /// Chat (WhatsApp, Telegram).
    pub chat: Arc<dyn Transport>,
}

impl ChannelRouter {
    /// `email` for email, nothing for push yet, and for chat whatever transport is registered
    /// when a message goes (or nothing).
    pub fn new(email: Arc<dyn Transport>) -> Self {
        Self {
            email,
            push: Arc::new(Undelivered),
            chat: Arc::new(Registered),
        }
    }

    fn pick(&self, channel: Channel) -> &Arc<dyn Transport> {
        match channel {
            Channel::Email => &self.email,
            Channel::Push => &self.push,
            Channel::Chat => &self.chat,
        }
    }
}

/// The registered chat transport, looked up at each call, so the order crates mount in doesn't
/// matter.
struct Registered;

#[async_trait]
impl Transport for Registered {
    fn readiness(&self, channel: Channel) -> Readiness {
        match chat_transport() {
            Some(t) => t.readiness(channel),
            None => Undelivered.readiness(channel),
        }
    }

    async fn deliver(&self, envelope: &Envelope) -> Delivery {
        match chat_transport() {
            Some(t) => t.deliver(envelope).await,
            None => Undelivered.deliver(envelope).await,
        }
    }
}

#[async_trait]
impl Transport for ChannelRouter {
    fn readiness(&self, channel: Channel) -> Readiness {
        self.pick(channel).readiness(channel)
    }

    async fn deliver(&self, envelope: &Envelope) -> Delivery {
        self.pick(envelope.channel).deliver(envelope).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ChatOnly;

    #[async_trait]
    impl Transport for ChatOnly {
        fn readiness(&self, _channel: Channel) -> Readiness {
            Readiness::Ready
        }
        async fn deliver(&self, _envelope: &Envelope) -> Delivery {
            Delivery::Sent {
                provider: "Telegram".into(),
                provider_ref: "tg-1".into(),
            }
        }
    }

    #[test]
    fn each_channel_goes_to_its_own_transport() {
        let router = ChannelRouter {
            email: Arc::new(Undelivered),
            push: Arc::new(Undelivered),
            chat: Arc::new(ChatOnly),
        };
        assert_eq!(router.readiness(Channel::Chat), Readiness::Ready);
        assert!(matches!(
            router.readiness(Channel::Email),
            Readiness::NotReady(_)
        ));
        assert!(matches!(
            router.readiness(Channel::Push),
            Readiness::NotReady(_)
        ));
    }
}
