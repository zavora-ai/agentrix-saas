//! What a chat may offer on a card, from the one rule table (ADR-014, docs/05 §7, docs/07 §4.1).
//!
//! The table is W5's: [`agentrix_decisions::surface`] (`powers`, `TABLE`). Telegram follows its
//! WhatsApp row, with one stricter switch on top: [`CHAT_ALLOW_ONCE`] is off, because James's
//! brief for Telegram says chat approves nothing. So a chat offers Open (the `/d/{card}` link)
//! and Decline, and never Approve. W5's `decide_spoken` enforces the same limits on the server.

use agentrix_decisions::DecisionCardV2;
use agentrix_decisions::surface::{self, Surface};
use serde::Serialize;

/// What a chat can say about a card, in the order buttons show them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// Open the card in the console (`/d/{card}`), where a passkey or code approves it.
    Open,
    /// Allow an everyday card once; never counts toward promotion.
    AllowOnce,
    /// Decline; nothing changes.
    Decline,
}

/// Whether chat may allow an everyday card once. W5's table allows it for WhatsApp; it's off for
/// the submission (chat approves nothing), so it comes on everywhere at once when James says so.
pub const CHAT_ALLOW_ONCE: bool = false;

/// What W5's table lets a chat do with a v2 card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatPowers {
    /// Allow once (before [`CHAT_ALLOW_ONCE`]).
    pub allow_once: bool,
    /// Decline.
    pub decline: bool,
}

impl ChatPowers {
    /// From W5's table, for a v2 card.
    pub fn of(card: &DecisionCardV2) -> Self {
        let p = surface::powers(Surface::WhatsApp, card);
        debug_assert!(!p.approve, "W5's table never lets chat approve");
        Self {
            allow_once: p.allow_once,
            decline: p.decline,
        }
    }
}

/// The answers a chat offers on a card with `powers`.
pub fn chat_answers(powers: ChatPowers) -> Vec<Answer> {
    let mut a = vec![Answer::Open];
    if powers.allow_once && CHAT_ALLOW_ONCE {
        a.push(Answer::AllowOnce);
    }
    if powers.decline {
        a.push(Answer::Decline);
    }
    a
}

/// Whether chat may freeze every WebAgent (W5's table: every surface but email).
pub fn chat_may_freeze() -> bool {
    surface::TABLE.iter().any(|r| r.surface == "WhatsApp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_decisions::{ApprovalKind, RaisedBy, Risk, RiskLevel};
    use agentrix_types::{Digest, EffectRequest, Money, SiteId};

    fn card(level: RiskLevel, approval: ApprovalKind, money: bool) -> DecisionCardV2 {
        DecisionCardV2 {
            v: 2,
            id: "dec2_x".into(),
            site: SiteId::new("s"),
            kind: "order.confirm".into(),
            title: "t".into(),
            summary: String::new(),
            effects: Vec::new(),
            effect: EffectRequest {
                site: SiteId::new("s"),
                webagent: "cashier".into(),
                tool: "order.confirm".into(),
                args: serde_json::json!({}),
                amount: money.then(|| Money {
                    currency: "KES".into(),
                    minor: 450_000,
                }),
                expiry: chrono::Utc::now(),
            },
            fingerprint: Digest::of(b"x"),
            raised_by: RaisedBy {
                webagent: "cashier".into(),
                job: None,
            },
            risk: Risk {
                level,
                reasons: Vec::new(),
            },
            approval,
            created_at: chrono::Utc::now(),
            back: None,
        }
    }

    #[test]
    fn chat_offers_open_and_decline_and_never_approves() {
        for c in [
            card(RiskLevel::Low, ApprovalKind::OneTap, false),
            card(RiskLevel::Low, ApprovalKind::OneTap, true),
            card(RiskLevel::High, ApprovalKind::Reviewed, false),
        ] {
            let a = chat_answers(ChatPowers::of(&c));
            assert_eq!(a, vec![Answer::Open, Answer::Decline], "{:?}", c.risk);
        }
    }

    #[test]
    fn money_is_never_allowed_once_even_with_the_switch() {
        let money = ChatPowers::of(&card(RiskLevel::Low, ApprovalKind::OneTap, true));
        assert!(!money.allow_once);
        let everyday = ChatPowers::of(&card(RiskLevel::Low, ApprovalKind::OneTap, false));
        assert!(
            everyday.allow_once,
            "W5's table allows it; our switch keeps it off"
        );
        assert!(chat_may_freeze());
    }
}
