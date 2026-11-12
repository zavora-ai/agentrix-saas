//! Orders and Customers inside the shell (gap 10 of specs/platform/04-ui-parity.md §3): the
//! owner's own pages over [`crate::commerce`], for every site the owner owns. Owner only, like
//! every shell page. Nothing here changes an order: approving or declining one opens its
//! decision card, which goes through the frozen approve contract.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Uri};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;

use crate::commerce::{Customer, Order, OrderState, Payment, Refund};
use crate::port::Owner;
use crate::routes::{PageError, PageResult, base, chrome, owner, render};
use crate::schedule;
use crate::view::*;
use crate::{ShellPage, ShellState};

/// The list's filters: id, label.
const FILTERS: [(&str, &str); 6] = [
    ("all", "All"),
    ("new", "New"),
    ("waiting", "Waiting"),
    ("done", "Done"),
    ("refunded", "Refunded"),
    ("closed", "Declined or expired"),
];

#[derive(Debug, Default, Deserialize)]
pub(crate) struct OrdersQuery {
    state: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CustomersQuery {
    q: Option<String>,
}

fn when(o: &Owner, at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    schedule::when(at, now, o.site.utc_offset_minutes)
}

/// "Sat 14 Nov" for a date sent as `YYYY-MM-DD`; anything else as sent.
fn day(text: &str) -> String {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map(|d| d.format("%a %-d %b").to_string())
        .unwrap_or_else(|_| text.to_owned())
}

/// The first line of what they asked for, at most 90 characters.
fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 90 {
        let cut: String = line.chars().take(89).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

fn label(state: OrderState) -> &'static str {
    match state {
        OrderState::New => "Needs you",
        OrderState::Waiting => "Waiting for payment",
        OrderState::Done => "Accepted",
        OrderState::Refunded => "Refunded",
        OrderState::Declined => "Declined",
        OrderState::Expired => "Expired",
    }
}

fn approved(state: OrderState) -> bool {
    matches!(
        state,
        OrderState::Waiting | OrderState::Done | OrderState::Refunded
    )
}

fn payment_words(state: &str) -> (&'static str, &'static str) {
    match state {
        "created" => ("Not paid yet", "info"),
        "pending" => ("Pending at PayPal", "info"),
        "captured" => ("Paid", "ok"),
        "declined" => ("Declined by PayPal", "risk"),
        "cancelled" => ("Cancelled", ""),
        "refunded" => ("Refunded", "risk"),
        "partly_refunded" => ("Partly refunded", "risk"),
        _ => ("Unknown", ""),
    }
}

fn provider_words(p: &str) -> String {
    match p {
        "paypal" => "PayPal".into(),
        other => other.to_owned(),
    }
}

/// "Paid KSh 4,500", "No payment".
fn money_line(order: &Order) -> String {
    match order.payments.last() {
        None => "No payment".into(),
        Some(p) => format!(
            "{} · {}",
            crate::commerce::money(&p.currency, p.amount_minor),
            payment_words(&p.state).0
        ),
    }
}

fn order_href(o: &Owner, order: &Order) -> String {
    format!("{}/orders/{}", base(o), order.id)
}

fn customer_href(o: &Owner, key: &str) -> String {
    format!("{}/customers/{key}", base(o))
}

fn short(digest: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    if hex.len() > 8 {
        format!("{}…{}", &hex[..4], &hex[hex.len() - 4..])
    } else {
        hex.to_owned()
    }
}

/// What one of an order's receipts records, in the owner's words. A kind with no words here
/// uses the console's words for it, and one without any reads "Something recorded about this
/// order": the stored name (`payment.captured`) never reaches the page.
fn receipt_words(kind: &str) -> String {
    let words = match kind {
        "door.order-held" => "Order held for you",
        "decision.approved" => "You approved the order",
        "decision.declined" => "You declined the order",
        "decision" => "Your decision",
        "payment.started" => "Payment started",
        "payment.captured" => "Payment confirmed",
        "payment.refunded" => "Refund sent",
        "consent.given" => "The customer gave consent",
        "consent.withdrawn" => "The customer withdrew consent",
        other => {
            return agentrix_types::ReceiptKind::parse(other)
                .map(crate::map::kind_words)
                .filter(|w| *w != other)
                .unwrap_or("Something recorded about this order")
                .to_owned();
        }
    };
    words.to_owned()
}

/// `GET /console/{site}/orders?state=`: every order, newest first, by state.
pub(crate) async fn orders(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    Query(q): Query<OrdersQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let all = state.store.commerce().orders(&o.site.id, now).await?;
    let pick = q
        .state
        .as_deref()
        .filter(|s| FILTERS.iter().any(|(id, _)| id == s))
        .unwrap_or("all");
    let shown = |order: &Order, id: &str| id == "all" || order.state.filter() == id;
    let tabs = FILTERS
        .iter()
        .map(|(id, label)| FilterTab {
            label: (*label).to_owned(),
            href: if *id == "all" {
                format!("{}/orders", base(&o))
            } else {
                format!("{}/orders?state={id}", base(&o))
            },
            count: all.iter().filter(|x| shown(x, id)).count(),
            current: *id == pick,
        })
        .collect();
    let rows = all
        .iter()
        .filter(|x| shown(x, pick))
        .map(|x| OrderRow {
            href: order_href(&o, x),
            number: if approved(x.state) {
                x.reference.clone()
            } else {
                String::new()
            },
            name: x.name.clone(),
            what: one_line(&x.description),
            wanted: day(&x.wanted),
            placed: when(&o, x.created_at, now),
            source: x.source.words().to_owned(),
            label: label(x.state).to_owned(),
            tone: x.state.tone().to_owned(),
            money: money_line(x),
        })
        .collect();
    let empty = match pick {
        "new" => {
            "No orders wait for you. An order from a customer's AI agent waits here until you approve or decline it."
        }
        "waiting" => "No orders are waiting for payment.",
        "done" => "No accepted orders yet. Orders you approve show here.",
        "refunded" => "No refunds on any order.",
        "closed" => "No declined or expired orders.",
        _ => "No orders yet.",
    };
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "orders", &cards).await?;
    render(
        &state,
        ShellPage::Orders,
        &OrdersView {
            chrome,
            tabs,
            rows,
            total: all.len(),
            empty: empty.to_owned(),
            empty_title: "No orders yet".into(),
            empty_body:
                "They appear here when a customer's AI agent orders through your site's agent door."
                    .into(),
        },
    )
}

fn payment_line(o: &Owner, p: &Payment, now: DateTime<Utc>) -> PaymentLine {
    let (label, tone) = payment_words(&p.state);
    PaymentLine {
        id: p.id.clone(),
        provider: provider_words(&p.provider),
        amount: crate::commerce::money(&p.currency, p.amount_minor),
        label: label.to_owned(),
        tone: tone.to_owned(),
        sandbox: p.sandbox,
        at: when(o, p.created_at, now),
    }
}

fn refund_line(o: &Owner, r: &Refund, now: DateTime<Utc>) -> RefundLine {
    let (label, tone) = match r.state.as_str() {
        "executed" => ("Sent back", "ok"),
        "not_executed" => ("Not sent", "risk"),
        _ => ("Not confirmed by PayPal yet", "wait"),
    };
    RefundLine {
        id: r.id.clone(),
        amount: crate::commerce::money(&r.currency, r.amount_minor),
        label: label.to_owned(),
        tone: tone.to_owned(),
        at: when(o, r.created_at, now),
        receipt: r.receipt.clone(),
    }
}

/// `GET /console/{site}/orders/{id}`: one order.
pub(crate) async fn order(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let commerce = state.store.commerce();
    let Some(x) = commerce.order(&o.site.id, &id, now).await? else {
        return Err(PageError::NotFound);
    };
    let customers = commerce.customers(&o.site.id, now).await?;
    let mine = customers.iter().find(|c| c.contact == x.contact());
    let name = if x.name.is_empty() {
        "The customer".to_owned()
    } else {
        x.name.clone()
    };
    let decided_at = x.decided_at.map(|t| when(&o, t, now)).unwrap_or_default();
    let decided_words = match x.state {
        OrderState::New => String::new(),
        OrderState::Declined => format!("You declined it {decided_at}."),
        OrderState::Expired => format!(
            "Nobody answered before it expired{}. It was never approved.",
            x.expires_at
                .map(|t| format!(" {}", when(&o, t, now)))
                .unwrap_or_default()
        ),
        _ => format!(
            "You approved it {}{decided_at}.",
            match x.method.as_deref() {
                Some("passkey") => "with your passkey, ",
                Some("totp") => "with a code from your authenticator app, ",
                _ => "",
            }
        ),
    };
    let captured = x.payments.iter().any(|p| p.state == "captured");
    let next_words = match x.state {
        OrderState::New => format!(
            "{name}'s AI agent is waiting for your answer. If you approve, it's told the order is accepted, with reference {}. If you decline, it's told no. Nothing is charged either way.",
            x.reference
        ),
        OrderState::Waiting => format!("Waiting for {name} to pay."),
        OrderState::Done if captured => "Paid. Nothing more to do here.".to_owned(),
        OrderState::Done => format!(
            "Accepted. {name}'s AI agent was given reference {}. The site doesn't track making or delivering it yet, so nothing more shows here.",
            x.reference
        ),
        OrderState::Refunded => "Refunded. Nothing more to do here.".to_owned(),
        OrderState::Declined => "Declined. Nothing more to do on this order.".to_owned(),
        OrderState::Expired => {
            "Expired without an answer. The customer's agent sees that it wasn't accepted."
                .to_owned()
        }
    };
    let receipts = commerce
        .receipts(&o.site.id, &x)
        .await?
        .into_iter()
        .map(|r| OrderReceiptLine {
            href: format!("/receipts/{}", r.digest),
            words: receipt_words(&r.kind),
            at: when(&o, r.at, now),
            short: short(&r.digest),
        })
        .collect();
    let detail = OrderDetail {
        id: x.id.clone(),
        reference: x.reference.clone(),
        reference_given: approved(x.state),
        filter: x.state.filter().to_owned(),
        label: label(x.state).to_owned(),
        tone: x.state.tone().to_owned(),
        name: x.name.clone(),
        email: x.email.clone(),
        what: x.description.clone(),
        wanted: day(&x.wanted),
        placed: when(&o, x.created_at, now),
        source: x.source.words().to_owned(),
        source_words: "Sent through your site's agent door by the customer's AI agent. The agent door takes no payment.".to_owned(),
        decision_href: format!("{}/decisions/{}", base(&o), x.id),
        waiting: x.state == OrderState::New,
        decided_words,
        expires_words: match (x.state, x.expires_at) {
            (OrderState::New, Some(t)) => format!("Waits for you until {}", when(&o, t, now)),
            _ => String::new(),
        },
        next_words,
        payments: x.payments.iter().map(|p| payment_line(&o, p, now)).collect(),
        refunds: x.refunds.iter().map(|r| refund_line(&o, r, now)).collect(),
        receipts,
        customer_href: mine.map(|c| customer_href(&o, &c.key)).unwrap_or_default(),
        other_orders: mine.map_or(0, |c| c.orders.len().saturating_sub(1)),
    };
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "orders", &cards).await?;
    render(&state, ShellPage::Order, &OrderView { chrome, o: detail })
}

fn display_name(c: &Customer) -> String {
    if c.name.trim().is_empty() {
        c.contact.clone()
    } else {
        c.name.clone()
    }
}

/// `GET /console/{site}/customers?q=`: everyone the site served, with search.
pub(crate) async fn customers(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    Query(q): Query<CustomersQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let all = state.store.commerce().customers(&o.site.id, now).await?;
    let typed: String = q.q.unwrap_or_default().trim().chars().take(100).collect();
    let needle = typed.to_lowercase();
    let rows = all
        .iter()
        .filter(|c| c.matches(&needle))
        .map(|c| CustomerRow {
            href: customer_href(&o, &c.key),
            name: display_name(c),
            contact: c.contact.clone(),
            has_account: c.account.is_some(),
            orders: c.orders.len(),
            questions: c.questions.len(),
            last: when(&o, c.last_at, now),
        })
        .collect();
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "customers", &cards).await?;
    render(
        &state,
        ShellPage::Customers,
        &CustomersView {
            chrome,
            q: typed,
            rows,
            total: all.len(),
            accounts: all.iter().filter(|c| c.account.is_some()).count(),
        },
    )
}

fn purpose_words(p: &str) -> String {
    match p {
        "order-updates" => "Order updates".into(),
        "account-memory" => "Account memory".into(),
        "offers" => "Offers by email".into(),
        other => other.to_owned(),
    }
}

/// `GET /console/{site}/customers/{key}`: one customer.
pub(crate) async fn customer(
    State(state): State<ShellState>,
    Path((site, key)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let Some(c) = state
        .store
        .commerce()
        .customer(&o.site.id, &key, now)
        .await?
    else {
        return Err(PageError::NotFound);
    };
    let mut conversations: Vec<(DateTime<Utc>, ConversationLine)> = c
        .questions
        .iter()
        .map(|q| {
            (
                q.at,
                ConversationLine {
                    at: when(&o, q.at, now),
                    how: "Passed to you from Ask".into(),
                    said: q.question.clone(),
                    href: None,
                },
            )
        })
        .chain(c.orders.iter().map(|x| {
            (
                x.created_at,
                ConversationLine {
                    at: when(&o, x.created_at, now),
                    how: "Sent by their AI agent".into(),
                    said: x.description.clone(),
                    href: Some(order_href(&o, x)),
                },
            )
        }))
        .collect();
    conversations.sort_by_key(|c| std::cmp::Reverse(c.0));
    let rights_url = match &o.site.live_url {
        Some(u) => format!("{}/account/data", u.trim_end_matches('/')),
        None => format!("https://{}/account/data", o.site.host),
    };
    let detail = CustomerDetail {
        name: display_name(&c),
        contact: c.contact.clone(),
        phone: c.phone.clone().unwrap_or_default(),
        account_words: c
            .account
            .as_ref()
            .map(|a| {
                format!(
                    "Since {}, {}",
                    when(&o, a.created_at, now),
                    if a.verified {
                        "email confirmed"
                    } else {
                        "email not confirmed yet"
                    }
                )
            })
            .unwrap_or_default(),
        has_account: c.account.is_some(),
        first: when(&o, c.first_at, now),
        last: when(&o, c.last_at, now),
        orders: c
            .orders
            .iter()
            .map(|x| CustomerOrderLine {
                href: order_href(&o, x),
                number: if approved(x.state) {
                    x.reference.clone()
                } else {
                    String::new()
                },
                what: one_line(&x.description),
                placed: when(&o, x.created_at, now),
                label: label(x.state).to_owned(),
                tone: x.state.tone().to_owned(),
            })
            .collect(),
        conversations: conversations.into_iter().map(|(_, l)| l).collect(),
        consents: c
            .consents
            .iter()
            .map(|k| ConsentLine {
                purpose: purpose_words(&k.purpose),
                given: when(&o, k.given_at, now),
                withdrawn: k
                    .withdrawn_at
                    .map(|t| format!("Withdrawn {}", when(&o, t, now)))
                    .unwrap_or_default(),
                receipt: k.receipt.clone(),
            })
            .collect(),
        rights_url,
    };
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "customers", &cards).await?;
    render(
        &state,
        ShellPage::Customer,
        &CustomerView { chrome, c: detail },
    )
}

#[cfg(test)]
mod tests {
    use super::receipt_words;

    #[test]
    fn an_orders_receipts_read_in_plain_words() {
        assert_eq!(receipt_words("payment.captured"), "Payment confirmed");
        assert_eq!(receipt_words("payment.refunded"), "Refund sent");
        assert_eq!(receipt_words("payment.started"), "Payment started");
        assert_eq!(receipt_words("door.order-held"), "Order held for you");
        assert_eq!(receipt_words("decision.approved"), "You approved the order");
        assert_eq!(receipt_words("decision.declined"), "You declined the order");
        assert_eq!(receipt_words("decision"), "Your decision");
        assert_eq!(receipt_words("release.signed"), "Release signed");
        for kind in ["order.shipped", "team.changed", "", "payment.captured.v2"] {
            assert_eq!(
                receipt_words(kind),
                "Something recorded about this order",
                "{kind}"
            );
        }
        for kind in agentrix_types::ReceiptKind::ALL {
            let words = receipt_words(kind.as_str());
            assert!(!words.contains('.'), "{kind:?} reads {words:?}");
        }
    }
}
