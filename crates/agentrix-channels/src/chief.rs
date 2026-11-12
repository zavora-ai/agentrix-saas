//! The owner's free text in chat is a question for the Chief WebAgent. It goes to W1's
//! `agentrix_kernel::chief::answer`, the one Chief every surface uses: real inference through the
//! Model Gateway, read tools over the site's records (catalogue and prices, the site's own
//! knowledge, orders, stock, questions, decisions, spend, receipts, the team), the owner's `brief`
//! and `site` commands, and other WebAgents it can ask. Every tool is checked against its dial
//! first, nothing it calls changes anything, and it can't approve: approvals stay in the console.

use agentrix_kernel::chief::{self, Surface};

use crate::service::Channels;

/// The Chief's answer to `said`, in plain words for a chat.
pub async fn answer(channels: &Channels, site: &str, said: &str) -> String {
    chief::answer(channels.ctx(), site, said, Surface::Chat)
        .await
        .text
}
