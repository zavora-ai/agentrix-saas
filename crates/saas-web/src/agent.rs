//! The Treasurer: the adk-rust agent the board's swapped-in AG Studio harness streams from.
//!
//! adk-server serves it under `/board/agent` on the money host. The harness
//! (`web/saas-board/src/harness.ts`) posts each message to `/board/agent/api/run_sse` with
//! `uiProtocol: "ag_ui"`, and adk-server's runtime turns the agent's events into AG-UI events
//! (`TEXT_MESSAGE_*`, `TOOL_CALL_*`, `RUN_*`), which is the protocol AG Studio's chat panel is
//! built on.
//!
//! **Judge mode.** This Treasurer answers from the feed by rules, with no model: every number it
//! says is read from the same [`Feed`] the board draws, and each answer shows the read as a tool
//! call. It costs nothing to run, so judges can use it freely. Slice B's Chief replaces it with
//! a model-backed agent behind the same route.

use std::sync::Arc;

use adk_core::{Agent, Content, Event, EventStream, InvocationContext, Part};
use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::feed::{AGENTRIX, Feed, FeedSource, money};

/// The agent's name, which is adk-server's app name: `/api/run_sse` takes `appName: "treasurer"`.
pub const APP: &str = "treasurer";

/// What day it is for the feed.
pub type Today = Arc<dyn Fn() -> NaiveDate + Send + Sync>;

/// The Treasurer.
pub struct Treasurer {
    feed: Arc<dyn FeedSource>,
    today: Today,
}

impl Treasurer {
    /// A Treasurer reading `feed` as of `today()`.
    pub fn new(feed: Arc<dyn FeedSource>, today: Today) -> Self {
        Self { feed, today }
    }
}

/// One answer: the read it made (shown as a tool call) and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The tool the read is shown as, e.g. `books.ageing`.
    pub tool: &'static str,
    /// Its arguments.
    pub args: Value,
    /// What the read returned (a summary of the feed rows it used).
    pub result: Value,
    /// The reply, in plain words.
    pub text: String,
}

/// Something the agent asks the board to do. The board's harness runs it in the browser
/// against AG Studio's API (a client-side tool): `board.filter` and `board.page`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardAction {
    /// `board.filter` or `board.page`.
    pub name: &'static str,
    /// Its arguments.
    pub args: Value,
    /// What the reply says about it.
    pub words: String,
}

/// The board actions `question` asks for: show one counterparty, show everything, or open a page.
pub fn board_actions(feed: &Feed, question: &str) -> Vec<BoardAction> {
    let q = question.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| q.contains(w));
    let mut out = Vec::new();
    if has(&[
        "show everything",
        "show all",
        "clear the filter",
        "clear filter",
        "remove the filter",
        "reset the board",
    ]) {
        out.push(BoardAction {
            name: "board.filter",
            args: json!({"counterparty": null}),
            words: "Showing every counterparty on the board again.".into(),
        });
    } else if has(&[
        "show only",
        "only show",
        "filter",
        "focus on",
        "just show",
        "zoom in on",
    ]) {
        let mut names: Vec<&str> = feed
            .receivables
            .iter()
            .map(|r| r.customer.as_str())
            .chain(feed.payables.iter().map(|p| p.vendor.as_str()))
            .collect();
        names.sort_unstable();
        names.dedup();
        let hit = names
            .iter()
            .find(|n| q.contains(&n.to_lowercase()))
            .or_else(|| {
                names.iter().find(|n| {
                    n.split_whitespace()
                        .next()
                        .is_some_and(|first| first.len() > 3 && q.contains(&first.to_lowercase()))
                })
            });
        if let Some(name) = hit {
            out.push(BoardAction {
                name: "board.filter",
                args: json!({"counterparty": name}),
                words: format!("Showing only {name} on the board."),
            });
        }
    }
    if has(&[
        "agents page",
        "agent spend page",
        "spend page",
        "open agents",
    ]) {
        out.push(BoardAction {
            name: "board.page",
            args: json!({"page": "agents"}),
            words: "Opened the Agents page.".into(),
        });
    } else if has(&["treasury page", "back to treasury", "open treasury"]) {
        out.push(BoardAction {
            name: "board.page",
            args: json!({"page": "treasury"}),
            words: "Opened the Treasury page.".into(),
        });
    }
    out
}

fn words_of(content: &Content) -> String {
    content
        .parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Answers `question` from `feed`, by rules. Public so tests and the console can call it.
pub fn answer(feed: &Feed, question: &str) -> Answer {
    let q = question.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| q.contains(w));
    let as_of = feed.as_of.to_string();
    let usd = |m: i64| money(m, "USD");

    if has(&["instal", "kilele", "debt", "plan"]) && !has(&["plans page", "pricing"]) {
        let rows: Vec<Value> = feed
            .debt_plans
            .iter()
            .map(|p| {
                json!({"plan": p.id, "invoice": p.invoice_number, "debtor": p.debtor, "principal": usd(p.principal_minor),
                       "paid": usd(p.paid_minor), "status": p.status,
                       "next": p.instalments.iter().find(|i| i.status != "paid").map(|i| json!({"n": i.n, "due": i.due, "amount": usd(i.amount_minor)}))})
            })
            .collect();
        let text = match feed.debt_plans.first() {
            Some(p) => {
                let paid = p.instalments.iter().filter(|i| i.status == "paid").count();
                let next = p.instalments.iter().find(|i| i.status != "paid");
                let debtor = feed
                    .company(&p.debtor)
                    .map_or(p.debtor.as_str(), |c| c.name.as_str());
                let creditor = feed
                    .company(&p.creditor)
                    .map_or(p.creditor.as_str(), |c| c.name.as_str());
                let mut t = format!(
                    "{debtor} owes {creditor} {} on {} under an instalment plan both agents agreed on {}, inside both owners' mandates. {paid} of {} instalments are paid ({} so far).",
                    usd(p.principal_minor - p.paid_minor),
                    p.invoice_number,
                    p.agreed,
                    p.instalments.len(),
                    usd(p.paid_minor),
                );
                if let Some(n) = next {
                    t.push_str(&format!(
                        " Next: instalment {} of {} on {}. Receivables collects it on the day; the kernel wakes on the date, nothing polls.",
                        n.n,
                        usd(n.amount_minor),
                        n.due
                    ));
                }
                t
            }
            None => "There are no instalment plans open.".into(),
        };
        return Answer {
            tool: "settle.debt_plans",
            args: json!({"asOf": as_of}),
            result: json!({"plans": rows}),
            text,
        };
    }

    if has(&[
        "overdue",
        "late",
        "ageing",
        "aging",
        "owe",
        "receivable",
        "collect",
        "chase",
        "ar ",
    ]) || q.trim() == "ar"
    {
        let open: Vec<_> = feed
            .receivables
            .iter()
            .filter(|r| r.open_minor > 0 && r.company == AGENTRIX)
            .collect();
        let total: i64 = open.iter().map(|r| r.open_minor).sum();
        let late: Vec<_> = {
            let mut l: Vec<_> = open.iter().filter(|r| r.days_overdue > 0).collect();
            l.sort_by_key(|r| -r.days_overdue);
            l
        };
        let late_total: i64 = late.iter().map(|r| r.open_minor).sum();
        let mut text = format!(
            "Customers owe Agentrix {} across {} open invoices; {} of it is past due.",
            usd(total),
            open.len(),
            usd(late_total)
        );
        for r in late.iter().take(3) {
            let note = if r.status == "on_plan" {
                " (on an instalment plan)"
            } else {
                ""
            };
            text.push_str(&format!(
                " {} owes {} on {}, {} days late{note}.",
                r.customer,
                usd(r.open_minor),
                r.number,
                r.days_overdue
            ));
        }
        text.push_str(" The ageing widget shows every bucket.");
        let rows: Vec<Value> = feed
            .ageing
            .iter()
            .filter(|a| a.company == AGENTRIX)
            .map(|a| json!({"customer": a.customer, "current": usd(a.current_minor), "1-30": usd(a.d1_30_minor), "31-60": usd(a.d31_60_minor), "61-90": usd(a.d61_90_minor), "90+": usd(a.over90_minor)}))
            .collect();
        return Answer {
            tool: "books.ageing",
            args: json!({"company": AGENTRIX, "asOf": as_of}),
            result: json!({"rows": rows}),
            text,
        };
    }

    if has(&[
        "bill",
        "payable",
        "vendor",
        "pay out",
        "payout",
        "owe them",
        "due this week",
    ]) {
        let due: Vec<_> = feed
            .payables
            .iter()
            .filter(|p| p.open_minor > 0 && p.company == AGENTRIX)
            .collect();
        let week: i64 = due
            .iter()
            .filter(|p| p.days_to_due <= 7)
            .map(|p| p.open_minor)
            .sum();
        let held: Vec<_> = due.iter().filter(|p| p.mandate == "over").collect();
        let mut text = format!(
            "Agentrix has {} to pay in the next 7 days across {} bills, all through PayPal Payouts.",
            usd(week),
            due.iter().filter(|p| p.days_to_due <= 7).count()
        );
        for p in &held {
            text.push_str(&format!(
                " {} for {} ({}) is over the founder's per-payment limit, so Payables is holding it for James's passkey.",
                usd(p.open_minor),
                p.vendor,
                p.number
            ));
        }
        let within: i64 = due
            .iter()
            .filter(|p| p.mandate == "within")
            .map(|p| p.open_minor)
            .sum();
        text.push_str(&format!(
            " The rest ({}) is inside the mandate and goes out on its due date.",
            usd(within)
        ));
        let rows: Vec<Value> = due
            .iter()
            .map(|p| json!({"bill": p.number, "vendor": p.vendor, "due": p.due, "amount": usd(p.open_minor), "mandate": p.mandate, "status": p.status}))
            .collect();
        return Answer {
            tool: "books.payables",
            args: json!({"company": AGENTRIX, "withinDays": 7, "asOf": as_of}),
            result: json!({"rows": rows}),
            text,
        };
    }

    if has(&[
        "cash", "balance", "paypal", "m-pesa", "mpesa", "bank", "runway",
    ]) {
        let rows: Vec<Value> = feed
            .cash
            .iter()
            .map(|c| json!({"company": c.company, "rail": c.rail, "account": c.account, "balance": money(c.balance_minor, &c.currency), "mode": c.mode}))
            .collect();
        let parts: Vec<String> = feed
            .cash
            .iter()
            .filter(|c| c.company == AGENTRIX)
            .map(|c| {
                format!(
                    "{} on {} ({})",
                    money(c.balance_minor, &c.currency),
                    c.rail,
                    c.mode
                )
            })
            .collect();
        let text = format!(
            "Agentrix holds {}. Every PayPal event posts to ERA through the PayPal clearing account, so this matches the books.",
            parts.join(" and ")
        );
        return Answer {
            tool: "books.cash",
            args: json!({"asOf": as_of}),
            result: json!({"rows": rows}),
            text,
        };
    }

    if has(&["mrr", "revenue", "growth", "subscri", "churn", "customers"]) {
        let rows: Vec<Value> = feed
            .mrr
            .iter()
            .map(|m| json!({"month": m.month, "mrr": usd(m.mrr_minor), "new": usd(m.new_minor), "churn": usd(m.churn_minor), "customers": m.customers}))
            .collect();
        let text = match (feed.mrr.first(), feed.mrr.last()) {
            (Some(a), Some(b)) => format!(
                "MRR is {} from {} paying customers in {}, up from {} in {}. This month added {} new and lost {} to churn.",
                usd(b.mrr_minor),
                b.customers,
                b.month,
                usd(a.mrr_minor),
                a.month,
                usd(b.new_minor),
                usd(b.churn_minor)
            ),
            _ => "No MRR yet.".into(),
        };
        return Answer {
            tool: "books.mrr",
            args: json!({"months": feed.mrr.len()}),
            result: json!({"rows": rows}),
            text,
        };
    }

    if has(&["spend", "cost", "token", "model", "meter"]) {
        let mut by_agent: Vec<(String, i64, u32)> = Vec::new();
        for s in &feed.agent_spend {
            let key = format!("{}/{}", s.company, s.agent);
            match by_agent.iter_mut().find(|(k, _, _)| *k == key) {
                Some(row) => {
                    row.1 += s.model_minor;
                    row.2 += s.tool_calls;
                }
                None => by_agent.push((key, s.model_minor, s.tool_calls)),
            }
        }
        let total: i64 = by_agent.iter().map(|r| r.1).sum();
        by_agent.sort_by_key(|r| -r.1);
        let top = by_agent
            .first()
            .map(|r| format!(" {} spent the most ({}).", r.0, usd(r.1)))
            .unwrap_or_default();
        let text = format!(
            "Both companies' agents spent {} on models over the last 7 days, posted to ERA as each company's own expense.{top}",
            usd(total)
        );
        let rows: Vec<Value> = by_agent
            .iter()
            .map(|(k, m, c)| json!({"agent": k, "model": usd(*m), "toolCalls": c}))
            .collect();
        return Answer {
            tool: "meter.agent_spend",
            args: json!({"days": 7}),
            result: json!({"rows": rows}),
            text,
        };
    }

    if has(&[
        "mandate",
        "limit",
        "authority",
        "approve",
        "decision",
        "passkey",
        "freeze",
        "frozen",
    ]) {
        let open: Vec<_> = feed
            .decisions
            .iter()
            .filter(|d| d.state == "open")
            .collect();
        let mut text = String::new();
        for m in &feed.mandates {
            text.push_str(&format!(
                "{}'s mandate for {} ({}): up to {} a payment and {} a day, {} used today, signed with a {} and good until {}. ",
                m.owner, m.agent, m.company, usd(m.per_payment_minor), usd(m.daily_minor), usd(m.used_today_minor), m.signed_with, m.expires
            ));
        }
        text.push_str(&format!("{} decisions wait for an owner.", open.len()));
        for d in &open {
            text.push_str(&format!(" {}: {}", d.title, d.why));
        }
        let rows: Vec<Value> = feed.mandates.iter().map(|m| json!({"id": m.id, "agent": m.agent, "perPayment": usd(m.per_payment_minor), "daily": usd(m.daily_minor), "usedToday": usd(m.used_today_minor), "status": m.status})).collect();
        return Answer {
            tool: "mandates.list",
            args: json!({"asOf": as_of}),
            result: json!({"mandates": rows, "openDecisions": open.len()}),
            text,
        };
    }

    if has(&["audit", "verify", "proof", "tamper", "log"]) {
        let a = &feed.audit;
        let text = if a.verified {
            format!(
                "The signed audit log has {} entries and its checkpoint verifies offline with `{}` ({}). Flip one byte and the same check fails.",
                a.entries, a.tool, a.checked
            )
        } else {
            format!(
                "The audit log did not verify at its last check ({}). Treat the books as unproven until it does.",
                a.checked
            )
        };
        return Answer {
            tool: "audit.verify",
            args: json!({}),
            result: json!({"verified": a.verified, "entries": a.entries, "checkpoint": a.checkpoint}),
            text,
        };
    }

    // The briefing.
    let cash: i64 = feed
        .cash
        .iter()
        .filter(|c| c.company == AGENTRIX && c.currency == "USD")
        .map(|c| c.balance_minor)
        .sum();
    let ar: i64 = feed
        .receivables
        .iter()
        .filter(|r| r.company == AGENTRIX)
        .map(|r| r.open_minor)
        .sum();
    let ap: i64 = feed
        .payables
        .iter()
        .filter(|p| p.company == AGENTRIX && p.days_to_due <= 7)
        .map(|p| p.open_minor)
        .sum();
    let mrr = feed.mrr.last().map_or(0, |m| m.mrr_minor);
    let open = feed
        .decisions
        .iter()
        .filter(|d| d.state == "open" && d.company == AGENTRIX)
        .count();
    let text = format!(
        "Today's position: {} in PayPal, {} owed to Agentrix, {} of bills due in 7 days, MRR {}. {} decision{} waiting for James. Ask me about overdue invoices, the Kilele instalment plan, bills, cash, MRR, agent spend, mandates or the audit log.",
        usd(cash),
        usd(ar),
        usd(ap),
        usd(mrr),
        open,
        if open == 1 { " is" } else { "s are" }
    );
    Answer {
        tool: "treasury.briefing",
        args: json!({"asOf": as_of}),
        result: json!({"cash": usd(cash), "receivables": usd(ar), "payablesDue7d": usd(ap), "mrr": usd(mrr), "openDecisions": open}),
        text,
    }
}

/// Splits `text` into chunks of about `size` characters at word boundaries, for streaming.
fn chunks(text: &str, size: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split_inclusive(' ') {
        cur.push_str(word);
        if cur.len() >= size {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[async_trait]
impl Agent for Treasurer {
    fn name(&self) -> &str {
        APP
    }

    fn description(&self) -> &str {
        "Agentrix's treasury desk: receivables, payables, debt plans, cash, MRR, agent spend, mandates and the audit log, read from the books."
    }

    fn sub_agents(&self) -> &[Arc<dyn Agent>] {
        &[]
    }

    async fn run(&self, ctx: Arc<dyn InvocationContext>) -> adk_core::Result<EventStream> {
        let question = words_of(ctx.user_content());
        let invocation = ctx.invocation_id().to_owned();
        let feed = self
            .feed
            .feed((self.today)())
            .await
            .map_err(|e| adk_core::AdkError::agent(format!("reading the feed: {e}")))?;
        let mut a = answer(&feed, &question);
        let actions = board_actions(&feed, &question);
        if !actions.is_empty() {
            let lead: Vec<&str> = actions.iter().map(|x| x.words.as_str()).collect();
            a.text = format!("{} {}", lead.join(" "), a.text);
        }
        let call_id = format!("call-{}", uuid::Uuid::new_v4().simple());

        let mut events = Vec::new();
        let mut call = Event::new(&invocation);
        call.author = APP.into();
        call.llm_response.content = Some(Content {
            role: "model".into(),
            parts: vec![Part::FunctionCall {
                name: a.tool.into(),
                args: a.args.clone(),
                id: Some(call_id.clone()),
                thought_signature: None,
            }],
        });
        events.push(call);

        let mut result = Event::new(&invocation);
        result.author = APP.into();
        result.llm_response.content = Some(Content {
            role: "function".into(),
            parts: vec![Part::FunctionResponse {
                function_response: adk_core::FunctionResponseData::new(a.tool, a.result.clone()),
                id: Some(call_id),
                annotations: None,
            }],
        });
        events.push(result);

        // The board's own tools: no response here, the harness runs them in the browser.
        for action in &actions {
            let mut e = Event::new(&invocation);
            e.author = APP.into();
            e.llm_response.content = Some(Content {
                role: "model".into(),
                parts: vec![Part::FunctionCall {
                    name: action.name.into(),
                    args: action.args.clone(),
                    id: Some(format!("call-{}", uuid::Uuid::new_v4().simple())),
                    thought_signature: None,
                }],
            });
            events.push(e);
        }

        let message_id = uuid::Uuid::new_v4().to_string();
        for piece in chunks(&a.text, 48) {
            let mut e = Event::with_id(&message_id, &invocation);
            e.author = APP.into();
            e.llm_response.partial = true;
            e.llm_response.content = Some(Content {
                role: "model".into(),
                parts: vec![Part::Text { text: piece }],
            });
            events.push(e);
        }
        let mut done = Event::with_id(&message_id, &invocation);
        done.author = APP.into();
        done.llm_response.turn_complete = true;
        // Marks this as the whole text of the chunks above, so adk-server's AG-UI stream sends
        // only what they didn't already carry (nothing) instead of the text a second time.
        done.llm_response.provider_metadata = Some(json!({"content_complete": true}));
        done.llm_response.content = Some(Content {
            role: "model".into(),
            parts: vec![Part::Text { text: a.text }],
        });
        events.push(done);

        Ok(Box::pin(futures::stream::iter(events.into_iter().map(Ok))))
    }
}

/// adk-server's app for the Treasurer: sessions in memory, the money host and the board's
/// origins allowed. Mount it with `Router::nest("/board/agent", …)`.
pub fn server(
    feed: Arc<dyn FeedSource>,
    today: Today,
    hosts: &crate::hosts::Hosts,
) -> axum::Router {
    let agent: Arc<dyn Agent> = Arc::new(Treasurer::new(feed, today));
    let config = adk_server::ServerConfig::new(
        Arc::new(adk_core::SingleAgentLoader::new(agent)),
        Arc::new(adk_session::InMemorySessionService::new()),
    )
    .with_allowed_hosts(vec![hosts.money.clone(), ".localhost".into()])
    .with_allowed_origins(hosts.board_origins());
    adk_server::create_app(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::story;

    fn feed() -> Feed {
        story("2026-11-10".parse().unwrap())
    }

    #[test]
    fn each_topic_reads_its_part_of_the_books() {
        let f = feed();
        for (q, tool) in [
            ("Which invoices are overdue?", "books.ageing"),
            (
                "How is the Kilele instalment plan going?",
                "settle.debt_plans",
            ),
            ("What bills are due this week?", "books.payables"),
            ("How much cash do we have on PayPal?", "books.cash"),
            ("Show me MRR growth", "books.mrr"),
            ("What did the agents spend on models?", "meter.agent_spend"),
            ("What is the founder's mandate?", "mandates.list"),
            ("Does the audit log verify?", "audit.verify"),
            ("Good morning", "treasury.briefing"),
        ] {
            assert_eq!(answer(&f, q).tool, tool, "{q}");
        }
    }

    #[test]
    fn numbers_come_from_the_feed() {
        let f = feed();
        let a = answer(&f, "how is kilele's debt plan?");
        assert!(a.text.contains("$505.60"), "{}", a.text);
        assert!(a.text.contains("instalment 2 of $252.80"), "{}", a.text);
        let a = answer(&f, "what's overdue?");
        assert!(
            a.text.contains("Kilele Bakery owes $505.60 on INV-1042"),
            "{}",
            a.text
        );
        let a = answer(&f, "bills due?");
        assert!(a.text.contains("$360.00 for Amina Yusuf"), "{}", a.text);
    }

    #[test]
    fn the_board_is_asked_to_filter_and_clear() {
        let f = feed();
        let a = board_actions(&f, "Show only Kilele Bakery on the board.");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "board.filter");
        assert_eq!(a[0].args["counterparty"], "Kilele Bakery");
        assert_eq!(
            board_actions(&f, "focus on tana")[0].args["counterparty"],
            "Tana Valley Times"
        );
        assert_eq!(
            board_actions(&f, "show everything")[0].args["counterparty"],
            Value::Null
        );
        assert_eq!(
            board_actions(&f, "open the agents page")[0].name,
            "board.page"
        );
        assert!(board_actions(&f, "what is overdue?").is_empty());
        assert!(board_actions(&f, "show only nobody-at-all").is_empty());
    }

    #[test]
    fn chunks_keep_every_character() {
        let text = "one two three four five six seven eight nine ten";
        assert_eq!(chunks(text, 10).concat(), text);
        assert!(chunks(text, 10).len() > 2);
    }
}
