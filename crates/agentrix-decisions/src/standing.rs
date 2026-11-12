//! What the owner teaches a WebAgent while deciding a card (04-ui-parity §3 item 7;
//! concept/console.html's "Decline and teach" and "Allow, and tell me").
//!
//! - A **rule** comes from a decline: a reason in the owner's words, kept as a standing
//!   instruction for the WebAgent and job that raised the card ("Never take same-day orders").
//!   Rules are listed on the WebAgent's page, shown on its later cards, carried in every prompt
//!   it answers under (read through [`crate::taught`]), and the owner can remove one at any time.
//! - A **grant** comes from an approval: "always allow this for this person (or this kind), and
//!   tell me". It is signed with the same passkey or code as the approval, and it is capped at
//!   the card's own amount. Later cards that match are approved under it, each with its own
//!   receipt and a line in Today. Money leaving the business, publishing, changes to what
//!   WebAgents may do, and high-risk cards never get one ([`grant_refusal`]).

use agentrix_types::{Digest, EffectRequest, Money, canonical_json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{DecisionCardV2, RiskLevel};

/// The longest reason or rule, in characters.
pub const MAX_REASON: usize = 280;

/// How long a grant lasts before it needs the owner again.
pub const GRANT_DAYS: i64 = 90;

/// A standing instruction the owner gave a WebAgent when declining one of its cards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// `rule_…`.
    pub id: String,
    /// The site.
    pub site: String,
    /// The WebAgent it binds (`cashier`).
    pub webagent: String,
    /// The job, when the card named one.
    pub job: Option<String>,
    /// The kind of card it came from (`order.confirm`).
    pub kind: String,
    /// The rule, in the owner's words.
    pub words: String,
    /// The card it came from.
    pub from_card: String,
    /// The decline's receipt, which records it.
    pub receipt: String,
    /// When the owner made it.
    pub created_at: DateTime<Utc>,
}

/// Who a grant covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantScope {
    /// This kind of card, for the person on this card only.
    Person,
    /// Every card of this kind from this WebAgent.
    Kind,
}

impl GrantScope {
    /// From `person` or `kind`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "person" => Some(Self::Person),
            "kind" => Some(Self::Kind),
            _ => None,
        }
    }
}

/// What a grant allows: everything the approval signs besides the card itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantTerms {
    /// Person or kind.
    pub scope: GrantScope,
    /// The WebAgent whose cards it covers.
    pub webagent: String,
    /// Its job, when the card named one.
    pub job: Option<String>,
    /// The card kind.
    pub kind: String,
    /// The tool that runs.
    pub tool: String,
    /// For [`GrantScope::Person`]: the person, as the card names them.
    pub person: Option<String>,
    /// The most a matching card may carry: the card's own amount. `None` when the card had no
    /// amount, and then a later card with an amount never matches.
    pub limit: Option<Money>,
    /// How many days it lasts.
    pub days: i64,
    /// In the owner's words: "orders for Amani W.".
    pub words: String,
}

impl GrantTerms {
    /// What an approval that also makes this grant signs: the effect's fingerprint and the
    /// terms, together.
    pub fn fingerprint(&self, effect: &Digest) -> anyhow::Result<Digest> {
        let v = json!({ "v": 1, "approve": effect.to_string(), "grant": self });
        Ok(Digest::of(&canonical_json(&v)?))
    }

    /// "Always allow orders for Amani W., up to KSh 4,500, for 90 days, and tell me".
    pub fn sentence(&self) -> String {
        let limit = self
            .limit
            .as_ref()
            .map(|m| format!(", up to {}", crate::edit::money_words(&m.currency, m.minor)))
            .unwrap_or_default();
        format!(
            "Always allow {}{limit}, for {} days, and tell me",
            self.words, self.days
        )
    }
}

/// A standing grant the owner signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// `grant_…`.
    pub id: String,
    /// The site.
    pub site: String,
    /// What it allows.
    pub terms: GrantTerms,
    /// The card it came from.
    pub from_card: String,
    /// The approval's receipt, which records it.
    pub receipt: String,
    /// When the owner signed it.
    pub created_at: DateTime<Utc>,
    /// When it stops.
    pub expires_at: DateTime<Utc>,
    /// Cards approved under it so far.
    pub uses: i64,
}

impl Grant {
    /// Whether `card` falls inside this grant at `now`.
    pub fn covers(&self, card: &DecisionCardV2, now: DateTime<Utc>) -> bool {
        let t = &self.terms;
        now < self.expires_at
            && self.site == card.site.as_str()
            && t.webagent == card.raised_by.webagent
            && t.job == card.raised_by.job
            && t.kind == card.kind
            && t.tool == card.effect.tool
            && grant_refusal(card).is_none()
            && match t.scope {
                GrantScope::Kind => true,
                GrantScope::Person => {
                    t.person.is_some() && person(&card.effect).as_deref() == t.person.as_deref()
                }
            }
            && match (&t.limit, &card.effect.amount) {
                (_, None) => true,
                (Some(l), Some(a)) => l.currency == a.currency && a.minor <= l.minor,
                (None, Some(_)) => false,
            }
    }
}

/// The arguments a card names its person by, in order.
pub const PERSON_KEYS: [&str; 4] = ["customer", "person", "recipient", "contact"];

/// The person a card is for, when its effect names one.
pub fn person(effect: &EffectRequest) -> Option<String> {
    PERSON_KEYS.iter().find_map(|k| {
        effect
            .args
            .get(*k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}

/// A card kind in the owner's words, plural: "orders", "replies to reviews".
pub fn kind_words(kind: &str) -> String {
    match kind {
        "order.confirm" | "order-held" | "order" => "orders".into(),
        "reply.review" | "review.reply" => "replies to reviews".into(),
        "reply.message" | "message.reply" => "replies to messages".into(),
        "booking.change" => "booking changes".into(),
        other => format!("{} cards", other.replace(['.', '-', '_'], " ")),
    }
}

fn words_of(card: &DecisionCardV2) -> Vec<String> {
    format!("{} {}", card.kind, card.effect.tool)
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Why `card` can't get a standing grant, in the owner's words; `None` when it can.
pub fn grant_refusal(card: &DecisionCardV2) -> Option<&'static str> {
    let w = words_of(card);
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    if card.risk.level == RiskLevel::High {
        Some("High-risk cards always ask you, so this one can't be allowed for good.")
    } else if has(&[
        "refund", "refunds", "payout", "payouts", "transfer", "withdraw", "spend", "price",
        "prices", "charge", "pay", "budget",
    ]) {
        Some("Money leaving your business always asks you, so this one can't be allowed for good.")
    } else if has(&[
        "publish",
        "release",
        "live",
        "unfreeze",
        "freeze",
        "newsletter",
        "domain",
        "dns",
    ]) {
        Some("Publishing and going live always ask you, so this one can't be allowed for good.")
    } else if has(&[
        "dial", "webagent", "grant", "mission", "team", "role", "access", "rule",
    ]) {
        Some(
            "Changes to what your WebAgents may do always ask you, so this one can't be allowed for good.",
        )
    } else {
        None
    }
}

/// The terms a grant from `card` (as approved, so edited if the owner edited it) would carry.
/// `Err` is plain words for the owner.
pub fn terms(
    card: &DecisionCardV2,
    effect: &EffectRequest,
    scope: GrantScope,
) -> Result<GrantTerms, String> {
    if let Some(no) = grant_refusal(card) {
        return Err(no.into());
    }
    let who = person(effect);
    let words = match (scope, &who) {
        (GrantScope::Person, Some(p)) => format!("{} for {p}", kind_words(&card.kind)),
        (GrantScope::Person, None) => {
            return Err(
                "This card doesn't name a person, so it can only be allowed for its kind.".into(),
            );
        }
        (GrantScope::Kind, _) => kind_words(&card.kind),
    };
    Ok(GrantTerms {
        scope,
        webagent: card.raised_by.webagent.clone(),
        job: card.raised_by.job.clone(),
        kind: card.kind.clone(),
        tool: effect.tool.clone(),
        person: if scope == GrantScope::Person {
            who
        } else {
            None
        },
        limit: effect.amount.clone(),
        days: GRANT_DAYS,
        words,
    })
}

/// A reason or rule as the owner typed it, trimmed; `Err` when it is too long.
pub fn reason(typed: &str) -> Result<Option<String>, String> {
    let t = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        Ok(None)
    } else if t.chars().count() > MAX_REASON {
        Err(format!(
            "Keep the reason short: at most {MAX_REASON} characters."
        ))
    } else {
        Ok(Some(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApprovalKind, RaisedBy, Risk};
    use agentrix_types::SiteId;

    fn card(
        kind: &str,
        tool: &str,
        level: RiskLevel,
        customer: &str,
        minor: i64,
    ) -> DecisionCardV2 {
        let effect = EffectRequest {
            site: SiteId::new("s1"),
            webagent: "cashier".into(),
            tool: tool.into(),
            args: json!({ "customer": customer }),
            amount: Some(Money {
                currency: "KES".into(),
                minor,
            }),
            expiry: Utc::now() + chrono::Duration::hours(1),
        };
        DecisionCardV2 {
            v: 2,
            id: "dec2_x".into(),
            site: SiteId::new("s1"),
            kind: kind.into(),
            title: "t".into(),
            summary: String::new(),
            effects: Vec::new(),
            fingerprint: effect.fingerprint().unwrap(),
            effect,
            raised_by: RaisedBy {
                webagent: "cashier".into(),
                job: Some("orders".into()),
            },
            risk: Risk {
                level,
                reasons: Vec::new(),
            },
            approval: ApprovalKind::Reviewed,
            created_at: Utc::now(),
            back: None,
        }
    }

    #[test]
    fn money_out_publishing_and_high_risk_never_get_grants() {
        let ok = card(
            "order.confirm",
            "order.confirm",
            RiskLevel::Low,
            "Amani W.",
            450_000,
        );
        assert_eq!(grant_refusal(&ok), None);
        for (kind, tool, level) in [
            ("order.confirm", "order.confirm", RiskLevel::High),
            ("refund", "payment.refund", RiskLevel::Low),
            ("release.publish", "release.publish", RiskLevel::Low),
            ("dial.raise", "dial.raise", RiskLevel::Low),
            ("newsletter", "newsletter.send", RiskLevel::Low),
        ] {
            let c = card(kind, tool, level, "Amani W.", 1);
            assert!(grant_refusal(&c).is_some(), "{kind} {tool} {level:?}");
            assert!(terms(&c, &c.effect, GrantScope::Kind).is_err());
        }
    }

    #[test]
    fn a_grant_covers_the_same_person_up_to_its_limit() {
        let first = card(
            "order.confirm",
            "order.confirm",
            RiskLevel::Low,
            "Amani W.",
            450_000,
        );
        let t = terms(&first, &first.effect, GrantScope::Person).unwrap();
        assert_eq!(
            t.sentence(),
            "Always allow orders for Amani W., up to KSh 4,500, for 90 days, and tell me"
        );
        let g = Grant {
            id: "grant_1".into(),
            site: "s1".into(),
            terms: t,
            from_card: first.id.clone(),
            receipt: "r".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(GRANT_DAYS),
            uses: 0,
        };
        let now = Utc::now();
        let same = card(
            "order.confirm",
            "order.confirm",
            RiskLevel::Low,
            "Amani W.",
            400_000,
        );
        assert!(g.covers(&same, now));
        let over = card(
            "order.confirm",
            "order.confirm",
            RiskLevel::Low,
            "Amani W.",
            500_000,
        );
        assert!(!g.covers(&over, now), "over the limit");
        let other = card(
            "order.confirm",
            "order.confirm",
            RiskLevel::Low,
            "Njeri",
            100_000,
        );
        assert!(!g.covers(&other, now), "another person");
        let risky = card(
            "order.confirm",
            "order.confirm",
            RiskLevel::High,
            "Amani W.",
            100_000,
        );
        assert!(!g.covers(&risky, now), "high risk");
        assert!(
            !g.covers(&same, now + chrono::Duration::days(GRANT_DAYS + 1)),
            "ended"
        );
    }
}
