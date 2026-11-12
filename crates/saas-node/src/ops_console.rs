//! Agentrix's real agents in the operator console on the ops host (`ops.<platform host>`).
//!
//! The console's pages are agentrix-operator's. [`crate::platform::with_activity`] sits in front
//! of them and adds these, for signed-in operators only:
//!
//! - `GET /activity/agents.json`: each of Agentrix's agents as it is now. That is one of:
//!   working (the kernel has it running, or it is answering an operator); waiting on a decision
//!   the owner holds; asleep until its next scheduled run; idle until an event wakes it; or
//!   switched off, or frozen. With each, the last thing the agent did, from the company's signed
//!   audit log.
//! - `POST /activity/ask`: the operator's question to Agentrix's Chief, with the Chief's own tools
//!   on the governed path, streamed back as server-sent events: each tool it calls, then its
//!   answer. Each operator has a session of their own and one question at a time.
//! - `GET /activity/console.js`: the console's script that shows both, in the dock's agent marks
//!   and the askline.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};

use adk_core::audit_log::{AuditDecision, AuditRecord, AuditRecordKind, AuditRecordOutcome};
use async_trait::async_trait;
use axum::Json;
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, FixedOffset, Utc};
use saas_company::identity::{CHIEF, PAYABLES, RECEIVABLES, REVENUE, ROLES, SUPPORT};
use saas_company::runtime::Runtime;
use saas_company::staff::TurnStep;
use serde_json::{Value, json};

/// Who is signed in to the operator console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorId {
    /// A stable id (the account's).
    pub id: String,
    /// The name the console greets.
    pub name: String,
}

/// The operator console's sign-in, as agentrix-node knows it.
#[async_trait]
pub trait Operators: Send + Sync {
    /// The operator these headers are signed in as, if any.
    async fn operator(&self, headers: &HeaderMap) -> Option<OperatorId>;
}

/// The longest question the askline takes, in characters.
const MOST_CHARS: usize = 2_000;

/// How many of the newest audit records are read for each agent's last action.
const RECENT: u64 = 600;

/// Nairobi, where the operators read the times.
fn eat(at: DateTime<Utc>) -> String {
    let nairobi = FixedOffset::east_opt(3 * 3600).expect("UTC+3 is a valid offset");
    at.with_timezone(&nairobi).format("%H:%M EAT").to_string()
}

/// The questions being answered now: session → the operator asking.
fn asking() -> &'static Mutex<BTreeMap<String, String>> {
    static ASKING: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    ASKING.get_or_init(Default::default)
}

/// The Chief's session for `operator`: `ops-<id>`, in characters a session id takes.
fn session_for(operator: &OperatorId) -> String {
    let id: String = operator
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    format!("ops-{id}")
}

fn agent_name(role: &str) -> &'static str {
    match role {
        CHIEF => "Chief",
        REVENUE => "Revenue",
        RECEIVABLES => "Receivables",
        PAYABLES => "Payables",
        SUPPORT => "Support",
        _ => "Agent",
    }
}

/// A session in words: `cron-payables` is the payables run.
fn session_words(session: &str) -> String {
    if let Some(name) = session.strip_prefix("cron-") {
        format!("the {} run", name.replace('-', " "))
    } else if session.starts_with("ops-") {
        "an operator's question".into()
    } else if let Some(rest) = session.strip_prefix("a2a-") {
        format!("an invoice at the agent door ({rest})")
    } else {
        session.replace('-', " ")
    }
}

/// The role an audit record's agent is, when it is one of the company's.
fn role_of(record: &AuditRecord) -> Option<&'static str> {
    let agent = record.actor.agent.as_ref()?.to_string();
    let last = agent
        .rsplit(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_owned();
    ROLES.iter().copied().find(|r| *r == last)
}

/// A record in a few words: "ap_bills_due, done", "pay_bill, held for the owner".
fn record_words(record: &AuditRecord) -> String {
    let how = match (record.decision, record.outcome) {
        (Some(AuditDecision::ApprovalRequired), _) => "held for the owner",
        (Some(AuditDecision::Denied | AuditDecision::Rejected), _) => "refused",
        (_, Some(AuditRecordOutcome::Failed)) => "failed",
        (_, Some(AuditRecordOutcome::Succeeded)) => "done",
        (Some(AuditDecision::Approved), _) => "approved by the owner",
        // A read-only call's record is its authorization: it ran.
        (Some(AuditDecision::Allowed), _) => "done",
        _ => "asked",
    };
    let what = if record.action == "model.call" {
        "thought (a model call)".to_owned()
    } else {
        record.action.replace('_', " ")
    };
    format!("{what}, {how}")
}

/// The newest tool call of each agent, else its newest model call, from the newest records of
/// the company's signed audit log (read from its store; nothing is written).
async fn last_actions(rt: &Runtime) -> BTreeMap<&'static str, (DateTime<Utc>, String)> {
    let audit = &rt.company.governance.audit;
    let store = audit.store();
    let mut out: BTreeMap<&'static str, (DateTime<Utc>, String)> = BTreeMap::new();
    let Ok(size) = store.size(audit.log_id()).await else {
        return out;
    };
    let Ok(entries) = store
        .read(audit.log_id(), size.saturating_sub(RECENT)..size)
        .await
    else {
        return out;
    };
    let mut model: BTreeMap<&'static str, (DateTime<Utc>, String)> = BTreeMap::new();
    for record in entries
        .iter()
        .filter_map(|e| e.record.as_deref())
        .filter_map(|t| serde_json::from_str::<AuditRecord>(t).ok())
    {
        let Some(role) = role_of(&record) else {
            continue;
        };
        let row = (record.ts, record_words(&record));
        match record.kind {
            AuditRecordKind::ToolCall => {
                out.insert(role, row);
            }
            _ if record.action == "model.call" => {
                model.insert(role, row);
            }
            _ => {}
        }
    }
    for (role, row) in model {
        out.entry(role).or_insert(row);
    }
    out
}

/// Agentrix's agents now, for the console's dock.
pub async fn agents(node: &crate::Node) -> Value {
    let rt = &node.agentrix;
    let now = Utc::now();
    let company = &rt.company;
    let frozen = company.governance.control.is_frozen();
    let held = company.governance.held().await.unwrap_or_default();
    let kernel = rt.supervisor.agents_now(now).await;
    let last = last_actions(rt).await;
    let asking: Vec<String> = asking()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .cloned()
        .collect();
    let mut rows = Vec::new();
    for role in ROLES {
        let now_role = kernel
            .as_ref()
            .ok()
            .and_then(|k| k.iter().find(|a| a.role == role).cloned());
        let waiting: Vec<&str> = held
            .iter()
            .filter(|h| {
                h.agent
                    .as_deref()
                    .is_some_and(|a| a.ends_with(&format!("/{role}")))
            })
            .map(|h| h.tool.as_str())
            .collect();
        let revoked = company.access().is_some_and(|a| a.is_revoked(role));
        let answering = role == CHIEF && !asking.is_empty();
        let working = now_role
            .as_ref()
            .map(|a| a.working.clone())
            .unwrap_or_default();
        let (state, mark, words) = if revoked {
            (
                "revoked",
                "resting",
                "Switched off by the owner; its tokens no longer work".to_owned(),
            )
        } else if frozen {
            (
                "frozen",
                "frozen",
                "Frozen by the owner: nothing that moves money runs".to_owned(),
            )
        } else if answering {
            (
                "working",
                "working",
                format!("Answering {}'s question", asking.join(" and ")),
            )
        } else if let Some(session) = working.first() {
            (
                "working",
                "working",
                format!("Working on {}", session_words(session)),
            )
        } else if !waiting.is_empty() {
            (
                "waiting",
                "waiting",
                format!(
                    "Waiting for the owner's passkey on {}",
                    waiting
                        .iter()
                        .map(|t| t.replace('_', " "))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        } else if let Some((at, session)) = now_role.as_ref().and_then(|a| a.next.clone()) {
            (
                "sleeping",
                "idle",
                format!("Asleep until {}, for {}", eat(at), session_words(&session)),
            )
        } else {
            (
                "idle",
                "idle",
                "Idle: it wakes when an event concerns it".to_owned(),
            )
        };
        let next = now_role.as_ref().and_then(|a| a.next.clone());
        rows.push(json!({
            "role": role,
            "name": agent_name(role),
            "state": state,
            "mark": mark,
            "words": words,
            "next": next.map(|(at, session)| json!({ "at": at, "at_words": eat(at), "for": session_words(&session) })),
            "last": last.get(role).map(|(at, what)| json!({ "at": at, "at_words": eat(*at), "what": what })),
        }));
    }
    json!({
        "company": company.config.name,
        "frozen": frozen,
        "kernel": kernel.is_ok(),
        "chief": { "ready": !frozen && !company.access().is_some_and(|a| a.is_revoked(CHIEF)) },
        "agents": rows,
    })
}

/// Whether a POST came from a page on this host (its `Origin`, when the browser sent one).
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    origin
        .split_once("://")
        .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(host))
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

/// The question in `body` (JSON `{"question": …}`, or a form's `question=`).
fn question_of(body: &[u8]) -> Option<String> {
    let text = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v["question"].as_str().map(str::to_owned))
        .or_else(|| {
            url::form_urlencoded::parse(body)
                .find(|(k, _)| k == "question")
                .map(|(_, v)| v.into_owned())
        })?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(MOST_CHARS).collect())
}

/// Takes `session` off the asking list when the answer is over, however it ends.
struct Asked(String);

impl Drop for Asked {
    fn drop(&mut self) {
        asking()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.0);
    }
}

/// `POST /activity/ask`: `operator`'s question to Agentrix's Chief, answered as a stream.
pub async fn ask(node: Arc<crate::Node>, operator: OperatorId, request: Request) -> Response {
    let headers = request.headers().clone();
    if !same_origin(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "Ask from the console on this host. Nothing was asked.",
        );
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), 64 * 1024).await else {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "That question is too long. Nothing was asked.",
        );
    };
    let Some(question) = question_of(&body) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "Type a question for the Chief first.",
        );
    };
    let session = session_for(&operator);
    {
        let mut asking = asking().lock().unwrap_or_else(|p| p.into_inner());
        if asking.contains_key(&session) {
            return refuse(
                StatusCode::CONFLICT,
                "The Chief is still answering your last question.",
            );
        }
        asking.insert(session.clone(), operator.name.clone());
    }
    let asked = Asked(session.clone());
    let (out_tx, out_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let text = format!(
        "{} (an Agentrix operator, signed in to the operator console) asks: {question}",
        operator.name
    );
    tracing::info!(operator = %operator.name, session = %session, "an operator asked Agentrix's Chief");
    tokio::spawn(async move {
        let _asked = asked;
        let step = |step: TurnStep| {
            Event::default()
                .event("step")
                .data(serde_json::to_string(&step).unwrap_or_default())
        };
        let (steps_tx, mut steps_rx) = tokio::sync::mpsc::unbounded_channel::<TurnStep>();
        // The turn runs to its end even if the operator closes the page: it is on the governed
        // path, and what it did is in the company's signed log either way.
        let turn = node
            .agentrix
            .company
            .ask_observed(CHIEF, &session, &text, Some(&steps_tx));
        tokio::pin!(turn);
        let result = loop {
            tokio::select! {
                result = &mut turn => break result,
                Some(s) = steps_rx.recv() => {
                    let _ = out_tx.send(step(s));
                }
            }
        };
        while let Ok(s) = steps_rx.try_recv() {
            let _ = out_tx.send(step(s));
        }
        let done = match result {
            Ok(report) => json!({
                "text": report.text,
                "tools": report.calls.iter().map(|(t, _)| t).collect::<Vec<_>>(),
                "held": report.held.iter().map(|h| &h.tool_name).collect::<Vec<_>>(),
                "refused": report.refused,
                "ended": report.ended,
            }),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "the Chief couldn't answer an operator");
                json!({ "error": "The Chief couldn't answer just now. Nothing was done. Try again in a minute." })
            }
        };
        let _ = out_tx.send(Event::default().event("done").data(done.to_string()));
    });
    let stream = futures::stream::unfold(out_rx, |mut rx| async move {
        rx.recv().await.map(|e| (Ok::<_, Infallible>(e), rx))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// The console's script.
pub fn script() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/ops-console.js"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_and_words() {
        let op = OperatorId {
            id: "2f1c-AB/../x".into(),
            name: "James".into(),
        };
        assert_eq!(session_for(&op), "ops-2f1c-ABx");
        assert_eq!(session_words("cron-payables"), "the payables run");
        assert_eq!(
            session_words("cron-usage-invoices"),
            "the usage invoices run"
        );
        assert_eq!(
            eat(DateTime::parse_from_rfc3339("2026-11-11T15:45:00Z")
                .unwrap()
                .with_timezone(&Utc)),
            "18:45 EAT"
        );
    }

    #[test]
    fn a_question_comes_as_json_or_a_form() {
        assert_eq!(
            question_of(br#"{"question":"  How much cash?  "}"#).as_deref(),
            Some("How much cash?")
        );
        assert_eq!(
            question_of(b"question=What+is+due%3F").as_deref(),
            Some("What is due?")
        );
        assert_eq!(question_of(br#"{"question":"   "}"#), None);
    }

    #[test]
    fn a_post_from_another_site_is_refused() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "ops.paypal.agentrix.website".parse().unwrap());
        assert!(same_origin(&h));
        h.insert(
            header::ORIGIN,
            "https://ops.paypal.agentrix.website".parse().unwrap(),
        );
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!same_origin(&h));
    }
}
