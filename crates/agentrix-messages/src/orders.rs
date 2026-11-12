//! Order and digest messages from the facts W2's sample sites already hold, so a flow sends one
//! message with one call:
//!
//! ```no_run
//! # async fn demo(messages: agentrix_messages::Messages, site: agentrix_messages::orders::SiteFacts, order: agentrix_messages::orders::OrderFacts) -> Result<(), agentrix_messages::SendError> {
//! use agentrix_messages::orders::{OrderEvent, customer, owner_new_order};
//! messages.send(customer(&site, &order, &OrderEvent::Received, "njeri@example.com")).await?;
//! let (id, ctx, why, key) = owner_new_order(&site, &order, "https://agentrix.website");
//! messages.send_to_site_owner(&site.id, &id, ctx, &why, Some(key)).await?;
//! # Ok(()) }
//! ```
//!
//! Amounts arrive written ("KSh 2,550", "USD 19.77"); the templates print them as given. The
//! dedupe key is the order id plus the event, so a retried step sends nothing twice.

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::send::{Outgoing, Recipient};

/// The site, as its messages need it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteFacts {
    /// The site's id.
    pub id: String,
    /// "Kilele Bakery".
    pub name: String,
    /// "kilele-bakery-sample.agentrix.website".
    pub host: String,
    /// A `-sample` site: every email carries the Sample band.
    pub sample: bool,
    /// The owner's first name, as customers know them ("Wanjiru").
    pub owner_first: String,
    /// The site's accent colour for the email button (`#3b2416`), if it has one.
    #[serde(default)]
    pub accent: Option<String>,
}

/// One line of an order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    /// "Country sourdough".
    pub name: String,
    /// 2.
    pub qty: u32,
    /// "KSh 900", the line's total.
    pub amount: String,
}

/// An order, as W2's `OrderView` carries it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderFacts {
    /// "KB-1042".
    pub order_id: String,
    /// "Njeri Wambui".
    pub customer_name: String,
    /// `catalogue` or `custom`.
    pub kind: String,
    /// What was ordered.
    pub lines: Vec<Line>,
    /// "KSh 300", when delivered.
    #[serde(default)]
    pub delivery: Option<String>,
    /// "KSh 2,550".
    pub total: String,
    /// "USD 19.77", what PayPal charges.
    #[serde(default)]
    pub usd: Option<String>,
    /// "1 USD = KSh 129.00".
    #[serde(default)]
    pub rate: Option<String>,
    /// `delivery` or `collection`.
    pub fulfilment: String,
    /// "Sat 14 Nov, 10:00–12:00".
    pub slot: String,
    /// "Amina, at home in Kilimani".
    #[serde(default)]
    pub recipient: Option<String>,
    /// The customer's note, or the custom design in words.
    #[serde(default)]
    pub note: Option<String>,
    /// The order page on the site's own host.
    pub order_url: String,
    /// The pay page, once a payment is requested.
    #[serde(default)]
    pub pay_url: Option<String>,
    /// The receipt page, once paid.
    #[serde(default)]
    pub receipt_url: Option<String>,
}

/// What happened to the order.
#[derive(Debug, Clone)]
pub enum OrderEvent {
    /// Paid (a catalogue order), or a custom order paid after approval.
    Received,
    /// A custom request waits for the owner.
    RequestHeld {
        /// "2 hours".
        answer_within: String,
        /// When the request closes unanswered, "Thu 19 Nov, 20:00".
        expires: String,
    },
    /// The owner approved; a payment request is ready at `pay_url`.
    RequestApproved {
        /// "Fri 20 Nov, 12:00".
        pay_by: String,
    },
    /// The owner declined.
    RequestDeclined {
        /// The owner's own words, if they gave any.
        reason: Option<String>,
    },
    /// The owner suggests a change instead.
    ChangeSuggested {
        /// The owner's words.
        text: String,
        /// The new price, written, if it changes.
        new_total: Option<String>,
        /// "Thu 19 Nov, 20:00".
        answer_by: String,
    },
    /// The request closed unanswered.
    RequestLapsed {
        /// "Thu 19 Nov, 20:00".
        expired_at: String,
    },
    /// Paid; the signed receipt is ready at `receipt_url`.
    Receipt {
        /// "Thu 12 Nov, 14:06 (Nairobi time)".
        paid_at: String,
        /// "R-1042".
        receipt_id: String,
    },
    /// The order moved on.
    Status {
        /// "Baking", "Out for delivery", "Ready to collect", "Delivered".
        label: String,
        /// Who moved it: "Wanjiru", "The Orders desk".
        by: String,
        /// "05:40".
        at: String,
        /// A line for the customer, if any.
        note: Option<String>,
    },
    /// A change the customer asked for was answered.
    ChangeAnswered {
        /// Yes or no.
        approved: bool,
        /// What they asked.
        asked: String,
        /// What the order now says (when approved).
        result: String,
        /// The owner's words (when declined).
        reason: Option<String>,
    },
    /// Money went back.
    Refunded {
        /// "R-1042-1".
        refund_id: String,
        /// "USD 10.47".
        amount: String,
        /// "KSh 1,350 at 1 USD = KSh 129.00".
        local: Option<String>,
        /// "The Cashier" (under rule g-3) or "Wanjiru".
        by: String,
        /// "the bake box".
        what: String,
    },
    /// A refund is above the rule and waits for the owner.
    RefundHeld {
        /// "KSh 5,250".
        amount: String,
        /// "the lemon sponge cake".
        what: String,
        /// "Sun 15 Nov, 20:00".
        decide_by: String,
    },
}

impl OrderEvent {
    /// The event's part of the dedupe key: each status step and each refund is its own event.
    fn key(&self) -> String {
        let slug = |s: &str| {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>()
        };
        match self {
            Self::Received => "received".into(),
            Self::RequestHeld { .. } => "held".into(),
            Self::RequestApproved { .. } => "approved".into(),
            Self::RequestDeclined { .. } => "declined".into(),
            Self::ChangeSuggested { .. } => "change-suggested".into(),
            Self::RequestLapsed { .. } => "lapsed".into(),
            Self::Receipt { .. } => "receipt".into(),
            Self::Status { label, .. } => format!("status-{}", slug(label)),
            Self::ChangeAnswered { asked, .. } => format!("change-answered-{}", slug(asked)),
            Self::Refunded { refund_id, .. } => format!("refunded-{}", slug(refund_id)),
            Self::RefundHeld { amount, .. } => format!("refund-held-{}", slug(amount)),
        }
    }
}

fn site_json(site: &SiteFacts) -> Json {
    json!({
        "name": site.name,
        "host": site.host,
        "sample": site.sample,
        "owner": site.owner_first,
        "accent": site.accent,
        "url": format!("https://{}", site.host),
    })
}

fn first_name(full: &str) -> String {
    full.split_whitespace().next().unwrap_or(full).to_string()
}

fn fulfilment(order: &OrderFacts) -> Json {
    let (label, when) = if order.fulfilment == "delivery" {
        let to = order
            .recipient
            .as_deref()
            .map(|r| format!(", for {r}"))
            .unwrap_or_default();
        ("Delivery", format!("{}{to}", order.slot))
    } else {
        ("Collection", order.slot.clone())
    };
    json!({ "label": label, "when": when })
}

fn lines_json(order: &OrderFacts) -> Vec<Json> {
    let mut lines: Vec<Json> = order
        .lines
        .iter()
        .map(|l| json!({ "qty": l.qty, "name": l.name, "total": l.amount }))
        .collect();
    if let Some(d) = &order.delivery {
        lines.push(json!({ "qty": 1, "name": "Delivery", "total": d }));
    }
    lines
}

fn summary(order: &OrderFacts) -> String {
    let what = order
        .lines
        .iter()
        .map(|l| {
            if l.qty > 1 {
                format!("{} × {}", l.qty, l.name)
            } else {
                l.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match order.note.as_deref().filter(|n| !n.trim().is_empty()) {
        Some(note) => format!("{what}: {note}"),
        None => what,
    }
}

fn request_json(order: &OrderFacts) -> serde_json::Map<String, Json> {
    let mut m = serde_json::Map::new();
    m.insert("number".into(), json!(order.order_id));
    m.insert("summary".into(), json!(summary(order)));
    m.insert("quote".into(), json!(order.total));
    m.insert("currency".into(), json!("KES"));
    m.insert("when".into(), json!(fulfilment(order)["when"]));
    m.insert("url".into(), json!(order.order_url));
    m
}

/// The message to the customer for `event`, ready for [`crate::Messages::send`].
pub fn customer(site: &SiteFacts, order: &OrderFacts, event: &OrderEvent, email: &str) -> Outgoing {
    let first = first_name(&order.customer_name);
    let mut ctx = json!({ "site": site_json(site), "person": { "name": first } });
    let charged = order
        .usd
        .as_ref()
        .map(|usd| json!({ "amount": usd, "currency": "USD", "method": "PayPal" }));
    let (message, reason) = match event {
        OrderEvent::Received => {
            ctx["order"] = json!({
                "number": order.order_id, "currency": "KES", "total": order.total,
                "lines": lines_json(order), "charged": charged, "fulfilment": fulfilment(order),
                "cutoff": null, "url": order.order_url,
            });
            (
                "customer.order_received",
                format!("{first} paid for order {}", order.order_id),
            )
        }
        OrderEvent::RequestHeld {
            answer_within,
            expires,
        } => {
            let mut r = request_json(order);
            r.insert("answer_within".into(), json!(answer_within));
            r.insert("expires".into(), json!(expires));
            ctx["request"] = Json::Object(r);
            (
                "customer.request_held",
                format!(
                    "{first}'s request {} waits for {}'s OK",
                    order.order_id, site.owner_first
                ),
            )
        }
        OrderEvent::RequestApproved { pay_by } => {
            ctx["request"] = Json::Object(request_json(order));
            ctx["payment"] = json!({
                "amount": order.total, "currency": "KES",
                "charge": order.usd.clone().unwrap_or_else(|| order.total.clone()),
                "charge_currency": if order.usd.is_some() { "USD" } else { "KES" },
                "rate": order.rate, "expires": pay_by,
                "url": order.pay_url.clone().unwrap_or_else(|| order.order_url.clone()),
            });
            (
                "customer.request_approved",
                format!(
                    "{} approved request {}; it's ready to pay",
                    site.owner_first, order.order_id
                ),
            )
        }
        OrderEvent::RequestDeclined { reason } => {
            let mut r = request_json(order);
            r.insert("reason".into(), json!(reason));
            ctx["request"] = Json::Object(r);
            (
                "customer.request_declined",
                format!("{} declined request {}", site.owner_first, order.order_id),
            )
        }
        OrderEvent::ChangeSuggested {
            text,
            new_total,
            answer_by,
        } => {
            ctx["request"] = Json::Object(request_json(order));
            ctx["change"] =
                json!({ "text": text, "quote": new_total, "when": null, "expires": answer_by });
            (
                "customer.request_change_suggested",
                format!(
                    "{} suggested a change to {}",
                    site.owner_first, order.order_id
                ),
            )
        }
        OrderEvent::RequestLapsed { expired_at } => {
            let mut r = request_json(order);
            r.insert("expired_at".into(), json!(expired_at));
            ctx["request"] = Json::Object(r);
            (
                "customer.request_lapsed",
                format!("Request {} closed without an answer", order.order_id),
            )
        }
        OrderEvent::Receipt {
            paid_at,
            receipt_id,
        } => {
            ctx["order"] = json!({
                "number": order.order_id, "currency": "KES", "total": order.total,
                "lines": lines_json(order),
            });
            ctx["payment"] = json!({
                "charge": order.usd.clone().unwrap_or_else(|| order.total.clone()),
                "charge_currency": "USD", "method": "PayPal", "rate": order.rate,
                "paid_at": paid_at, "receipt_id": receipt_id,
                "receipt_url": order.receipt_url.clone().unwrap_or_else(|| format!("{}/receipt", order.order_url)),
            });
            (
                "customer.payment_receipt",
                format!("PayPal confirmed the payment for {}", order.order_id),
            )
        }
        OrderEvent::Status {
            label,
            by,
            at,
            note,
        } => {
            ctx["order"] = json!({ "number": order.order_id, "url": order.order_url, "steps": [] });
            ctx["status"] = json!({ "label": label, "by": by, "at": at, "note": note });
            (
                "customer.order_status",
                format!("{by} moved {} to {label}", order.order_id),
            )
        }
        OrderEvent::ChangeAnswered {
            approved,
            asked,
            result,
            reason,
        } => {
            ctx["order"] = json!({ "number": order.order_id, "url": order.order_url });
            ctx["change"] = json!({
                "approved": approved, "asked": asked, "result": result,
                "price_difference": null, "reason": reason,
            });
            (
                "customer.change_answered",
                format!(
                    "{} answered a change to {}",
                    site.owner_first, order.order_id
                ),
            )
        }
        OrderEvent::Refunded {
            refund_id,
            amount,
            local,
            by,
            what,
        } => {
            ctx["order"] = json!({ "number": order.order_id, "url": order.order_url });
            ctx["refund"] = json!({
                "amount": amount, "currency": "USD", "local": local, "method": "PayPal",
                "by": by, "what": what, "receipt_id": refund_id,
            });
            (
                "customer.refund_issued",
                format!("{by} refunded {amount} on {}", order.order_id),
            )
        }
        OrderEvent::RefundHeld {
            amount,
            what,
            decide_by,
        } => {
            ctx["order"] = json!({ "number": order.order_id, "url": order.order_url });
            ctx["refund"] = json!({ "amount": amount, "currency": "KES", "what": what, "decide_by": decide_by });
            (
                "customer.refund_held",
                format!(
                    "A refund on {} waits for {}",
                    order.order_id, site.owner_first
                ),
            )
        }
    };
    Outgoing {
        message: message.to_string(),
        site_id: Some(site.id.clone()),
        to: Recipient::email(email, Some(&first)),
        context: ctx,
        reason,
        dedupe_key: Some(format!("{}:{}:{}", site.id, order.order_id, event.key())),
    }
}

/// The owner's "new order" message (email, push and WhatsApp), for
/// [`crate::Messages::send_to_site_owner`]: returns the message id, context, reason and key.
/// `platform_origin` is the console's origin (`ctx.origin`, e.g. `https://agentrix.website`).
///
/// "Open the order" goes to the order in W2's back office (`/samples/{site}/orders/{id}`,
/// agentrix-samples), the only page that shows an order to its owner: the console has no
/// Orders app yet, so `/console/{site}/apps/orders` is a 404.
pub fn owner_new_order(
    site: &SiteFacts,
    order: &OrderFacts,
    platform_origin: &str,
) -> (String, Json, String, String) {
    let origin = platform_origin.trim_end_matches('/');
    let ctx = json!({
        "site": site_json(site),
        "console_url": format!("{origin}/console/{}", site.id),
        "order": {
            "number": order.order_id, "customer": first_name(&order.customer_name),
            "currency": "KES", "total": order.total,
            "charged": order.usd.as_ref().map(|usd| json!({ "amount": usd, "currency": "USD", "method": "PayPal" })),
            "lines": lines_json(order), "fulfilment": fulfilment(order),
            "url": format!("{origin}/samples/{}/orders/{}", site.id, order.order_id),
        },
    });
    (
        "owner.new_order".to_string(),
        ctx,
        format!(
            "{} paid for order {}",
            first_name(&order.customer_name),
            order.order_id
        ),
        format!("{}:{}:owner-new-order", site.id, order.order_id),
    )
}

/// One story in the Friday digest, as W2's news records carry it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    /// The headline.
    pub headline: String,
    /// One or two lines.
    pub standfirst: String,
    /// The story's address on the site's host.
    pub url: String,
    /// "Transport".
    pub section: String,
    /// "Grace Wairimu".
    pub byline: String,
    /// "Thu 12 Nov".
    #[serde(default)]
    pub published: Option<String>,
    /// Whether it was corrected.
    #[serde(default)]
    pub corrected: bool,
    /// When or what, for the corrected line ("Thu 12 Nov, 16:20").
    #[serde(default)]
    pub correction_note: Option<String>,
}

/// The digest's double opt-in email.
pub fn digest_confirm(site: &SiteFacts, email: &str, topics: &str, confirm_url: &str) -> Outgoing {
    Outgoing {
        message: "digest.confirm".into(),
        site_id: Some(site.id.clone()),
        to: Recipient::email(email, None),
        context: json!({
            "site": site_json(site),
            "digest": { "name": "your Friday five", "schedule": "Every Friday at 07:00", "topics": topics },
            "confirm_url": confirm_url, "forget_after": "7 days",
        }),
        reason: format!("Someone asked for the Friday five at {}", site.host),
        dedupe_key: None,
    }
}

/// One subscriber's Friday digest.
pub fn digest_weekly(
    site: &SiteFacts,
    email: &str,
    topics: &str,
    date: &str,
    stories: &[Story],
    stop_url: &str,
) -> Outgoing {
    let stories: Vec<Json> = stories
        .iter()
        .map(|s| {
            json!({
                "headline": s.headline, "summary": s.standfirst, "url": s.url,
                "section": s.section, "byline": s.byline,
                "published": s.published.clone().unwrap_or_else(|| "This week".into()),
                "corrected": if s.corrected { Some(s.correction_note.clone().unwrap_or_else(|| "since it was published".into())) } else { None },
            })
        })
        .collect();
    Outgoing {
        message: "digest.weekly".into(),
        site_id: Some(site.id.clone()),
        to: Recipient::email(email, None),
        context: json!({
            "site": site_json(site),
            "digest": { "topics": topics },
            "issue": { "date": date, "stories": stories, "locked": null },
            "unsubscribe_url": stop_url,
        }),
        reason: format!("The Friday five for {date}"),
        dedupe_key: Some(format!("{}:digest:{date}:{email}", site.id)),
    }
}

/// The note that the digest stopped and the topics are gone.
pub fn digest_stopped(site: &SiteFacts, email: &str, resubscribe_url: Option<&str>) -> Outgoing {
    Outgoing {
        message: "digest.stopped".into(),
        site_id: Some(site.id.clone()),
        to: Recipient::email(email, None),
        context: json!({
            "site": site_json(site),
            "digest": { "name": "your Friday five" },
            "resubscribe_url": resubscribe_url,
        }),
        reason: "A subscriber stopped the digest".into(),
        dedupe_key: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render;

    fn kilele() -> SiteFacts {
        SiteFacts {
            id: "site_kilele_sample".into(),
            name: "Kilele Bakery".into(),
            host: "kilele-bakery-sample.agentrix.website".into(),
            sample: true,
            owner_first: "Wanjiru".into(),
            accent: Some("#3b2416".into()),
        }
    }

    fn kb1042() -> OrderFacts {
        OrderFacts {
            order_id: "KB-1042".into(),
            customer_name: "Njeri Wambui".into(),
            kind: "catalogue".into(),
            lines: vec![
                Line {
                    name: "Country sourdough".into(),
                    qty: 2,
                    amount: "KSh 900".into(),
                },
                Line {
                    name: "Weekend bake box".into(),
                    qty: 1,
                    amount: "KSh 1,350".into(),
                },
            ],
            delivery: Some("KSh 300".into()),
            total: "KSh 2,550".into(),
            usd: Some("USD 19.77".into()),
            rate: Some("1 USD = KSh 129.00".into()),
            fulfilment: "delivery".into(),
            slot: "Sat 14 Nov, 10:00–12:00".into(),
            recipient: Some("Njeri, in Kilimani".into()),
            note: None,
            order_url: "https://kilele-bakery-sample.agentrix.website/orders/KB-1042".into(),
            pay_url: Some(
                "https://kilele-bakery-sample.agentrix.website/orders/KB-1042/pay".into(),
            ),
            receipt_url: Some(
                "https://kilele-bakery-sample.agentrix.website/orders/KB-1042/receipt".into(),
            ),
        }
    }

    #[test]
    fn every_order_event_renders_from_w2s_facts() {
        let events = vec![
            OrderEvent::Received,
            OrderEvent::RequestHeld {
                answer_within: "2 hours".into(),
                expires: "Thu 19 Nov, 20:00".into(),
            },
            OrderEvent::RequestApproved {
                pay_by: "Fri 20 Nov, 12:00".into(),
            },
            OrderEvent::RequestDeclined {
                reason: Some("We're full that Saturday.".into()),
            },
            OrderEvent::RequestDeclined { reason: None },
            OrderEvent::ChangeSuggested {
                text: "A 1-tier sponge serves 15.".into(),
                new_total: Some("KSh 3,800".into()),
                answer_by: "Thu 19 Nov, 20:00".into(),
            },
            OrderEvent::RequestLapsed {
                expired_at: "Thu 19 Nov, 20:00".into(),
            },
            OrderEvent::Receipt {
                paid_at: "Thu 12 Nov, 14:06 (Nairobi time)".into(),
                receipt_id: "R-1042".into(),
            },
            OrderEvent::Status {
                label: "Baking".into(),
                by: "Wanjiru".into(),
                at: "05:40".into(),
                note: None,
            },
            OrderEvent::ChangeAnswered {
                approved: false,
                asked: "Deliver at 12:00".into(),
                result: String::new(),
                reason: Some("The van is full then.".into()),
            },
            OrderEvent::Refunded {
                refund_id: "R-1042-1".into(),
                amount: "USD 10.47".into(),
                local: Some("KSh 1,350 at 1 USD = KSh 129.00".into()),
                by: "The Cashier".into(),
                what: "the bake box".into(),
            },
            OrderEvent::RefundHeld {
                amount: "KSh 5,250".into(),
                what: "the lemon sponge cake".into(),
                decide_by: "Sun 15 Nov, 20:00".into(),
            },
        ];
        for event in events {
            let out = customer(&kilele(), &kb1042(), &event, "njeri@example.com");
            let r = render::render(&out.message, &out.context)
                .unwrap_or_else(|e| panic!("{event:?}: {e}"));
            let email = r.email.expect("email");
            assert!(
                email.text.contains("KB-1042"),
                "{}: no order number",
                out.message
            );
            assert!(
                email.text.starts_with("Sample:"),
                "{}: no sample band",
                out.message
            );
            let problems = crate::lint::check(&email.text);
            assert!(problems.is_empty(), "{}: {problems:?}", out.message);
        }
        let paid = customer(
            &kilele(),
            &kb1042(),
            &OrderEvent::Received,
            "njeri@example.com",
        );
        let text = render::render(&paid.message, &paid.context)
            .unwrap()
            .email
            .unwrap()
            .text;
        assert!(text.contains("Total: KSh 2,550"));
        assert!(text.contains("Paid: USD 19.77 with PayPal"));
        assert!(text.contains("Delivery: Sat 14 Nov, 10:00–12:00, for Njeri, in Kilimani"));
        assert_eq!(
            paid.dedupe_key.as_deref(),
            Some("site_kilele_sample:KB-1042:received")
        );
    }

    #[test]
    fn the_owner_hears_about_a_new_order() {
        let (id, ctx, _, key) = owner_new_order(&kilele(), &kb1042(), "https://agentrix.website");
        let r = render::render(&id, &ctx).unwrap();
        assert_eq!(r.push.unwrap().body, "A new order came in.");
        assert!(r.email.unwrap().subject.contains("KB-1042: KSh 2,550"));
        assert!(key.ends_with("owner-new-order"));
    }

    #[test]
    fn the_digest_renders_from_stories() {
        let tana = SiteFacts {
            id: "site_tana_sample".into(),
            name: "Tana Valley Times".into(),
            host: "tana-valley-times-sample.agentrix.website".into(),
            sample: true,
            owner_first: "Nduku".into(),
            accent: Some("#b3261e".into()),
        };
        let story = Story {
            headline: "Council approves the northern rail extension, 31 to 12".into(),
            standfirst: "Construction tenders open in January.".into(),
            url: "https://tana-valley-times-sample.agentrix.website/2026/11/rail-extension".into(),
            section: "Transport".into(),
            byline: "Grace Wairimu".into(),
            published: None,
            corrected: true,
            correction_note: Some("Thu 12 Nov, 16:20".into()),
        };
        let stop = "https://tana-valley-times-sample.agentrix.website/digest/stop?token=t1";
        for out in [
            digest_confirm(
                &tana,
                "reader@example.com",
                "transport",
                "https://tana-valley-times-sample.agentrix.website/digest/confirm?token=t1",
            ),
            digest_weekly(
                &tana,
                "reader@example.com",
                "transport",
                "Fri 13 Nov",
                &[story.clone(), story],
                stop,
            ),
            digest_stopped(&tana, "reader@example.com", None),
        ] {
            let r = render::render(&out.message, &out.context)
                .unwrap_or_else(|e| panic!("{}: {e}", out.message));
            assert!(r.email.is_some());
        }
    }
}
