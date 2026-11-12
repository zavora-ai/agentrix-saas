//! Consent receipts (A-08, 06 §4.10, 03 §1.5): each optional use of a customer's details is a
//! separate choice, off until they pick it, and each "on" leaves a receipt they can keep.
//! Account memory needs "I'm 18 or over" first (01a D7); the receipt records it.

use agentrix_types::tenancy::{SiteHost, VisitorAccountId};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// One optional use of a customer's details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Purpose {
    /// Order updates on WhatsApp: confirmed, out for delivery, delivered.
    OrderUpdates,
    /// Account memory: "Since your last visit" and saved designs.
    AccountMemory,
    /// Offers by email.
    Offers,
}

impl Purpose {
    /// Every purpose, in the order the page shows them.
    pub const ALL: [Purpose; 3] = [
        Purpose::OrderUpdates,
        Purpose::AccountMemory,
        Purpose::Offers,
    ];

    /// The stored key.
    pub fn key(self) -> &'static str {
        match self {
            Self::OrderUpdates => "order-updates",
            Self::AccountMemory => "account-memory",
            Self::Offers => "offers",
        }
    }

    /// From the stored key.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Whether turning it on needs the 18+ confirmation (01a D7).
    pub fn needs_adult(self) -> bool {
        matches!(self, Self::AccountMemory)
    }
}

/// The words for one purpose on one site.
#[derive(Debug, Clone, Serialize)]
pub struct PurposeWords {
    /// Stored key.
    pub key: &'static str,
    /// The switch's label.
    pub title: String,
    /// The line under it.
    pub line: String,
    /// The receipt's heading after "Consent receipt · ".
    pub short: &'static str,
    /// Receipt: purpose.
    pub purpose: String,
    /// Receipt: what.
    pub what: String,
    /// Receipt: shared with.
    pub shared_with: String,
    /// Receipt: kept for.
    pub kept_for: String,
    /// Receipt: how to withdraw.
    pub to_withdraw: String,
    /// Needs "I'm 18 or over".
    pub needs_adult: bool,
}

/// The words for `purpose` on `site`. `phone` is the customer's phone, if they gave one;
/// `offers_line` is the site's own line for offers ("New bakes and seasonal offers.").
pub fn words(
    purpose: Purpose,
    site: &SiteHost,
    phone: Option<&str>,
    offers_line: Option<&str>,
) -> PurposeWords {
    let name = &site.name;
    match purpose {
        Purpose::OrderUpdates => PurposeWords {
            key: purpose.key(),
            title: "Order updates on WhatsApp".into(),
            line: match phone {
                Some(phone) => format!("Confirmed, out for delivery, delivered. Sent to {phone}."),
                None => "Confirmed, out for delivery, delivered. Add a phone number in your details first.".into(),
            },
            short: "WhatsApp updates",
            purpose: "Order updates on WhatsApp: confirmed, out for delivery, delivered".into(),
            what: "Your phone number, your order numbers and their status. No offers.".into(),
            shared_with: "WhatsApp, only to deliver these messages".into(),
            kept_for: "Until you turn it off".into(),
            to_withdraw: "Account → Choices, one tap".into(),
            needs_adult: false,
        },
        Purpose::AccountMemory => PurposeWords {
            key: purpose.key(),
            title: "Remember me and my designs".into(),
            line: "Account memory: \"Since your last visit\" and saved designs, until you delete them.".into(),
            short: "Account memory",
            purpose: "\"Since your last visit\" and your saved designs".into(),
            what: "Designs and preferences you save, item by item".into(),
            shared_with: format!("No one. Only {name}'s desks read it, while you're signed in."),
            kept_for: "Until you delete an item, or turn this off".into(),
            to_withdraw: "Account → Choices, one tap".into(),
            needs_adult: true,
        },
        Purpose::Offers => PurposeWords {
            key: purpose.key(),
            title: "Offers by email".into(),
            line: offers_line.map_or_else(|| format!("New products and seasonal offers from {name}."), str::to_owned),
            short: "Offers by email",
            purpose: offers_line
                .map(|l| l.trim_end_matches('.').to_owned())
                .unwrap_or_else(|| format!("New products and seasonal offers from {name}")),
            what: "Your email address".into(),
            shared_with: "No one".into(),
            kept_for: "Until you turn it off".into(),
            to_withdraw: "Account → Choices, one tap, or the link in any offer".into(),
            needs_adult: false,
        },
    }
}

/// The signed body of a consent receipt. Field order does not matter: it is signed as RFC 8785
/// canonical JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentReceipt {
    /// Always `agentrix.consent.v1`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `cr_…`.
    pub id: String,
    /// The site id.
    pub site: String,
    /// The site's host, which the receipt is "signed by" in the customer's words.
    pub host: String,
    /// The customer's account on the site.
    pub subject: VisitorAccountId,
    /// Purpose key.
    pub purpose: String,
    /// Purpose in words.
    pub purpose_text: String,
    /// What is used.
    pub what: String,
    /// Who else receives it.
    pub shared_with: String,
    /// How long it is kept.
    pub kept_for: String,
    /// How to withdraw.
    pub to_withdraw: String,
    /// When given, RFC 3339 UTC.
    pub given_at: String,
    /// "I'm 18 or over" was confirmed (account memory only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adult_confirmed: Option<bool>,
}

/// "Sat 7 Nov 2026, 18:40 Nairobi time". Sites in Horizon 1 keep Nairobi time; the site's own
/// zone comes with the attention settings (03 §1.8).
pub fn when(unix: i64) -> String {
    let eat = FixedOffset::east_opt(3 * 3600).expect("UTC+3 is a valid offset");
    let at: DateTime<FixedOffset> = Utc
        .timestamp_opt(unix, 0)
        .single()
        .unwrap_or_default()
        .with_timezone(&eat);
    format!(
        "{}, {} Nairobi time",
        at.format("%a %-d %b %Y"),
        at.format("%H:%M")
    )
}

/// "cr-4e1b…9a07": the short form a customer reads.
pub fn short_id(id: &str) -> String {
    let raw = id.trim_start_matches("cr_");
    if raw.len() <= 8 {
        return format!("cr-{raw}");
    }
    format!("cr-{}…{}", &raw[..4], &raw[raw.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::SiteId;

    fn kilele() -> SiteHost {
        SiteHost {
            site: SiteId::new("site_k"),
            host: "kilele-bakery-sample.agentrix.website".into(),
            origin: "https://kilele-bakery-sample.agentrix.website".into(),
            name: "Kilele Bakery".into(),
            sample: true,
        }
    }

    #[test]
    fn words_follow_the_concept() {
        let w = words(
            Purpose::OrderUpdates,
            &kilele(),
            Some("+44 7700 900418"),
            None,
        );
        assert_eq!(
            w.line,
            "Confirmed, out for delivery, delivered. Sent to +44 7700 900418."
        );
        let m = words(Purpose::AccountMemory, &kilele(), None, None);
        assert!(m.needs_adult);
        assert_eq!(
            m.shared_with,
            "No one. Only Kilele Bakery's desks read it, while you're signed in."
        );
        let o = words(
            Purpose::Offers,
            &kilele(),
            None,
            Some("New bakes and seasonal offers."),
        );
        assert_eq!(o.line, "New bakes and seasonal offers.");
        assert_eq!(o.purpose, "New bakes and seasonal offers");
        for p in Purpose::ALL {
            assert_eq!(Purpose::from_key(p.key()), Some(p));
        }
    }

    #[test]
    fn times_and_ids_read_plainly() {
        // 2026-11-07T15:40:00Z is 18:40 in Nairobi.
        assert_eq!(when(1_794_066_000), "Sat 7 Nov 2026, 18:40 Nairobi time");
        assert_eq!(short_id("cr_0194e1b0c0de9a07"), "cr-0194…9a07");
    }
}
