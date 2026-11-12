//! The money host's two desks: Sales and Support.
//!
//! The same contract as agentrix.website's desks (`agentrix-home::desks`): answer only from
//! what this site holds, cite it, and offer a person when nothing here answers. [`RulesDesk`]
//! answers from the price list (agentrix-billing's catalogue) and the support notes below,
//! with no model. Slice B's Revenue agent replaces it by implementing [`Desk`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One desk.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DeskInfo {
    /// `sales` or `support`.
    pub id: &'static str,
    /// "Sales".
    pub name: &'static str,
    /// The agent behind it.
    pub agent: &'static str,
    /// Its job.
    pub job: &'static str,
    /// Three questions to start with.
    pub starters: [&'static str; 3],
}

/// The two desks, in the page's order.
pub const DESKS: [DeskInfo; 2] = [
    DeskInfo {
        id: "sales",
        name: "Sales",
        agent: "Revenue",
        job: "Helps you pick a WebAgent plan and subscribe with PayPal.",
        starters: [
            "Which plan fits a small bakery?",
            "What does the Business plan include?",
            "How much is a year paid up front?",
        ],
    },
    DeskInfo {
        id: "support",
        name: "Support",
        agent: "Revenue",
        job: "Handles billing, cancellations and refunds for existing customers.",
        starters: [
            "How do I cancel my plan?",
            "Can I get a refund?",
            "I can't pay all of my invoice this month",
        ],
    },
];

/// A visitor's question.
#[derive(Debug, Clone, Deserialize)]
pub struct Ask {
    /// What they asked.
    pub q: String,
}

/// A source an answer cites.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Source {
    /// What it is.
    pub title: String,
    /// Where it is on this site.
    pub href: String,
}

/// A desk's answer.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Reply {
    /// The desk that answered.
    pub desk: String,
    /// The answer.
    pub text: String,
    /// Where each fact came from.
    pub sources: Vec<Source>,
    /// True when a model wrote the answer (the page then shows the AI disclosure).
    pub ai: bool,
    /// A next step, when there is one: `{label, href}`.
    pub action: Option<Source>,
}

/// A desk that answers visitors.
#[async_trait]
pub trait Desk: Send + Sync {
    /// Answers `q` at desk `desk` (`sales` or `support`).
    async fn reply(&self, desk: &str, q: &str) -> Reply;
}

/// Answers from the price list and the support notes, by rules.
pub struct RulesDesk {
    /// The agent door's host (`Hosts::pay`), which answers link to.
    door: String,
}

impl RulesDesk {
    /// Rules answers that link to the agent door at `door` (its host).
    pub fn new(door: &str) -> Self {
        Self {
            door: door.to_owned(),
        }
    }
}

impl Default for RulesDesk {
    fn default() -> Self {
        Self::new(&crate::hosts::Hosts::default().pay)
    }
}

fn src(title: &str, href: &str) -> Source {
    Source {
        title: title.into(),
        href: href.into(),
    }
}

/// "Web hosting included" → "web hosting included", for the middle of a sentence; names stay.
fn lower(item: &str) -> String {
    let mut chars = item.chars();
    match (chars.next(), item.chars().nth(1)) {
        (Some(first), Some(second)) if !second.is_uppercase() => {
            first.to_lowercase().chain(chars).collect()
        }
        _ => item.to_owned(),
    }
}

/// A plan's inclusions as one phrase: "web hosting included, bookings and forms and $5 of AI".
fn list(items: &[String]) -> String {
    let words: Vec<String> = items
        .iter()
        .filter(|i| !i.starts_with("Everything in"))
        .map(|i| lower(i))
        .collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

fn usd(cents: i64) -> String {
    crate::feed::money(cents, "USD")
}

#[async_trait]
impl Desk for RulesDesk {
    async fn reply(&self, desk: &str, q: &str) -> Reply {
        let q = q.to_lowercase();
        let has = |w: &[&str]| w.iter().any(|x| q.contains(x));
        let cat = agentrix_billing::catalogue();
        let plan = |id: &str| cat.plans.iter().find(|p| p.id == id);
        let prices = src("Plans and prices", "/plans");
        let reply = |text: String, sources: Vec<Source>, action: Option<Source>| Reply {
            desk: desk.to_owned(),
            text,
            sources,
            ai: false,
            action,
        };

        if desk == "support" {
            if has(&["cancel", "stop", "end my"]) {
                return reply(
                    "Yes, any time. Support stops your PayPal subscription so you are not charged again, and ERA closes the recurring invoice. Tell me the email on your plan and Support starts it; nothing changes until you confirm.".into(),
                    vec![src("Billing terms", "/plans#terms")],
                    None,
                );
            }
            if has(&["refund", "money back", "charged twice", "double"]) {
                return reply(
                    "Refunds go back through PayPal to the account that paid, and ERA posts each one against the original PayPal capture, so your books and ours match. Whether a charge is refunded is James's call: Support asks him with the facts and tells you the answer.".into(),
                    vec![src("Billing terms", "/plans#terms")],
                    None,
                );
            }
            if has(&[
                "can't pay",
                "cannot pay",
                "instal",
                "split",
                "short",
                "later",
                "late",
            ]) {
                return reply(
                    "Yes: Receivables can agree an instalment plan with you, up to 3 instalments over 90 days, inside the limits our founder signed. If your business runs its own Agentrix agents, your Payables agent and ours agree it between them and each owner's mandate bounds it. Each instalment is collected on its due date through PayPal.".into(),
                    vec![src("Billing terms", "/plans#terms")],
                    Some(src("Talk to Receivables", "/#desks")),
                );
            }
            return reply(
                "Support handles billing, cancellations, refunds and instalment plans. Nothing on this site answers that yet, so I'll pass it to a person: James reads every message.".into(),
                vec![],
                Some(src("Email support", "mailto:hello@agentrix.website")),
            );
        }

        // Sales.
        if has(&["year", "annual", "up front", "upfront", "12 month"]) {
            let lines: Vec<String> = cat
                .plans
                .iter()
                .filter(|p| p.kind == "site")
                .map(|p| {
                    format!(
                        "{} {} a year (instead of {})",
                        p.name,
                        usd(p.annual_cents()),
                        usd(p.price_cents * 12)
                    )
                })
                .collect();
            return reply(
                format!(
                    "Paid 12 months up front, each plan is {}% off: {}.",
                    cat.annual_discount_percent,
                    lines.join("; ")
                ),
                vec![prices],
                Some(src("See the plans", "/plans")),
            );
        }
        for p in cat.plans.iter() {
            if q.contains(&p.id) || q.contains(&p.name.to_lowercase()) {
                return reply(
                    format!(
                        "{} is {} a month ({} a year paid up front). It includes {}. Your WebAgents: {}.",
                        p.name,
                        usd(p.price_cents),
                        usd(p.annual_cents()),
                        list(&p.includes),
                        p.webagents.join(", ")
                    ),
                    vec![prices],
                    Some(src(
                        &format!("Subscribe to {} with PayPal", p.name),
                        &format!("/subscribe?plan={}", p.id),
                    )),
                );
            }
        }
        if has(&[
            "bakery",
            "small",
            "shop",
            "one person",
            "fit",
            "which plan",
            "cheapest",
        ]) {
            if let (Some(e), Some(b)) = (plan("essential"), plan("business")) {
                return reply(
                    format!(
                        "For a small shop, start with Essential at {} a month: {}. If you take bookings or orders on WhatsApp, Business at {} adds {}.",
                        usd(e.price_cents),
                        list(&e.includes),
                        usd(b.price_cents),
                        list(
                            &b.includes
                                .iter()
                                .filter(|i| !e.includes.contains(i))
                                .cloned()
                                .collect::<Vec<_>>()
                        )
                    ),
                    vec![prices],
                    Some(src(
                        "Subscribe to Essential with PayPal",
                        "/subscribe?plan=essential",
                    )),
                );
            }
        }
        if has(&[
            "pay",
            "paypal",
            "card",
            "how do i subscribe",
            "sign up",
            "subscribe",
        ]) {
            return reply(
                "You subscribe with PayPal: pick a plan, approve it on PayPal, and your first invoice and receipt appear in your account. Your agents can also pay us themselves over our agent door, inside a mandate you sign.".into(),
                vec![prices, src("The agent door", &format!("https://{}/", self.door))],
                Some(src("See the plans", "/plans")),
            );
        }
        let names: Vec<String> = cat
            .plans
            .iter()
            .filter(|p| p.kind == "site")
            .map(|p| format!("{} {}", p.name, usd(p.price_cents)))
            .collect();
        reply(
            format!(
                "Agentrix sells WebAgent plans by the month: {}. Ask me what any plan includes, or which one fits your business.",
                names.join(", ")
            ),
            vec![prices],
            Some(src("See the plans", "/plans")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sales_quotes_the_price_list() {
        let r = RulesDesk::default()
            .reply("sales", "What does the Business plan include?")
            .await;
        assert!(r.text.contains("Business is $79.00 a month"), "{}", r.text);
        assert_eq!(r.sources[0].href, "/plans");
        assert!(!r.ai);
        let r = RulesDesk::default()
            .reply("sales", "how much for a year up front?")
            .await;
        assert!(r.text.contains("20% off"), "{}", r.text);
    }

    #[tokio::test]
    async fn a_small_shop_gets_essential_in_plain_words() {
        let r = RulesDesk::default()
            .reply("sales", "Which plan fits a small bakery?")
            .await;
        assert!(
            r.text.starts_with(
                "For a small shop, start with Essential at $29.00 a month: web hosting included"
            ),
            "{}",
            r.text
        );
        assert!(!r.text.contains("Everything in"), "{}", r.text);
    }

    #[tokio::test]
    async fn support_offers_instalments_and_a_person() {
        let r = RulesDesk::default()
            .reply("support", "I can't pay all of it this month")
            .await;
        assert!(r.text.contains("instalment plan"), "{}", r.text);
        let r = RulesDesk::default()
            .reply("support", "what's the weather")
            .await;
        assert!(r.action.unwrap().href.starts_with("mailto:"));
    }
}
