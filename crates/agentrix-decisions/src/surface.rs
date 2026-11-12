//! Where a card may be decided (07 §4.4; concept/console/phone.html, "the same decisions, with
//! fewer powers"). One rule table for every surface, so a card can't be approved somewhere with
//! weaker checks:
//!
//! | Surface  | May decide                                                 | Never                          |
//! |----------|------------------------------------------------------------|--------------------------------|
//! | App      | everything; money, publishing and high risk with a passkey | details in a notification      |
//! | WhatsApp | allow once or decline everyday cards; STOP freezes         | money, high risk, unfreeze     |
//! | Voice    | allow once or decline everyday cards; stop everything      | money, publishing, high risk   |
//! | Email    | nothing: the brief and the weekly review, with links       | approvals (easy to forge)      |
//!
//! Anything a surface can't decide goes out as a short link, [`short_link`], that opens the card
//! in the console after sign-in.

use serde::{Deserialize, Serialize};

use crate::{ApprovalKind, DecisionCardV2, RiskLevel};

/// A place the owner can see a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    /// The console, on a desk or a phone.
    App,
    /// WhatsApp replies.
    WhatsApp,
    /// Speaking to the Chief WebAgent.
    Voice,
    /// Email.
    Email,
}

impl Surface {
    /// From a channel name: `app`, `whatsapp` (or `chat`), `voice` (or `spoken`), `email`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "app" | "console" => Surface::App,
            "whatsapp" | "chat" => Surface::WhatsApp,
            "voice" | "spoken" => Surface::Voice,
            "email" => Surface::Email,
            _ => return None,
        })
    }
}

/// What a surface may do with one card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Powers {
    /// Approve with the card open and a passkey, or an authenticator code without one.
    pub approve: bool,
    /// "Allow once" in words. Never counts toward a promotion.
    pub allow_once: bool,
    /// Decline.
    pub decline: bool,
    /// Freeze every WebAgent at once ("STOP", "stop everything").
    pub freeze: bool,
}

/// An everyday card: low risk, one tap, no money, not publishing and not unfreezing.
pub fn everyday(card: &DecisionCardV2) -> bool {
    let tool = card.effect.tool.as_str();
    card.risk.level == RiskLevel::Low
        && card.approval == ApprovalKind::OneTap
        && card.effect.amount.is_none()
        && !tool.contains("publish")
        && !tool.contains("unfreeze")
        && !card.kind.contains("publish")
}

/// What `surface` may do with `card`.
pub fn powers(surface: Surface, card: &DecisionCardV2) -> Powers {
    match surface {
        Surface::App => Powers {
            approve: true,
            allow_once: false,
            decline: true,
            freeze: true,
        },
        Surface::WhatsApp | Surface::Voice => Powers {
            approve: false,
            allow_once: everyday(card),
            decline: true,
            freeze: true,
        },
        Surface::Email => Powers {
            approve: false,
            allow_once: false,
            decline: false,
            freeze: false,
        },
    }
}

/// The short link a message carries for a card: `{origin}/d/{card}`. It opens the card in the
/// console after sign-in, where the passkey or code approves it.
pub fn short_link(origin: &str, card: &str) -> String {
    format!("{}/d/{card}", origin.trim_end_matches('/'))
}

/// One row of the table, in the owner's words.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Row {
    /// The surface.
    pub surface: &'static str,
    /// What it may decide.
    pub may: &'static str,
    /// What it never does.
    pub never: &'static str,
}

/// The table for the console to show. [`powers`] is what enforces it; the test below keeps the
/// two saying the same thing.
pub const TABLE: [Row; 4] = [
    Row {
        surface: "This console",
        may: "Everything. Money, publishing and high-risk cards need your passkey, or a code from your authenticator app",
        never: "Show a card's details in a notification",
    },
    Row {
        surface: "WhatsApp",
        may: "Decline any card. STOP freezes every WebAgent",
        never: "Approve anything or unfreeze: approving sends a link to open here",
    },
    Row {
        surface: "Voice",
        may: "Allow an everyday card once, or decline any card. \"Stop everything\" freezes every WebAgent",
        never: "Approve money, publishing or high risk",
    },
    Row {
        surface: "Email",
        may: "Nothing. The brief and the weekly review, with links",
        never: "Approvals: email is easy to forge and forward",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RaisedBy, Risk};
    use agentrix_types::{Digest, EffectRequest, Money, SiteId};
    use chrono::Utc;

    fn card(level: RiskLevel, approval: ApprovalKind, tool: &str, money: bool) -> DecisionCardV2 {
        DecisionCardV2 {
            v: 2,
            id: "dec2_x".into(),
            site: SiteId::new("s1"),
            kind: tool.into(),
            title: "t".into(),
            summary: String::new(),
            effects: Vec::new(),
            effect: EffectRequest {
                site: SiteId::new("s1"),
                webagent: "w".into(),
                tool: tool.into(),
                args: serde_json::json!({}),
                amount: money.then(|| Money {
                    currency: "KES".into(),
                    minor: 450_000,
                }),
                expiry: Utc::now(),
            },
            fingerprint: Digest::of(b"x"),
            raised_by: RaisedBy {
                webagent: "w".into(),
                job: None,
            },
            risk: Risk {
                level,
                reasons: Vec::new(),
            },
            approval,
            created_at: Utc::now(),
            back: None,
        }
    }

    #[test]
    fn fewer_powers_away_from_the_console() {
        let everyday = card(RiskLevel::Low, ApprovalKind::OneTap, "review.reply", false);
        let money = card(RiskLevel::Low, ApprovalKind::OneTap, "order.confirm", true);
        let publish = card(
            RiskLevel::Low,
            ApprovalKind::OneTap,
            "release.publish",
            false,
        );
        let high = card(RiskLevel::High, ApprovalKind::Reviewed, "dial.raise", false);
        for c in [&everyday, &money, &publish, &high] {
            let app = powers(Surface::App, c);
            assert!(app.approve && app.decline && !app.allow_once);
            let email = powers(Surface::Email, c);
            assert!(!email.approve && !email.allow_once && !email.decline && !email.freeze);
            for s in [Surface::WhatsApp, Surface::Voice] {
                let p = powers(s, c);
                assert!(!p.approve && p.decline && p.freeze, "{s:?}");
            }
        }
        assert!(powers(Surface::WhatsApp, &everyday).allow_once);
        assert!(powers(Surface::Voice, &everyday).allow_once);
        for c in [&money, &publish, &high] {
            assert!(!powers(Surface::WhatsApp, c).allow_once);
            assert!(!powers(Surface::Voice, c).allow_once);
        }
        assert!(TABLE[3].may.starts_with("Nothing"));
        assert!(TABLE[1].never.contains("Approve anything"));
        assert_eq!(
            short_link("https://agentrix.website/", "dec2_1"),
            "https://agentrix.website/d/dec2_1"
        );
        assert_eq!(Surface::parse("chat"), Some(Surface::WhatsApp));
        assert_eq!(Surface::parse("spoken"), Some(Surface::Voice));
    }
}
