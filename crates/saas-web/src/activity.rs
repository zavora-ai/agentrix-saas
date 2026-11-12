//! The live activity page for operators: what the agents did, newest first, streamed, from the
//! real records only.
//!
//! Each [`Source`] reads one kind of record and returns what is new since its last read:
//! [`AuditSource`] the signed audit logs (adk-rust's `AuditLogStore`), [`WalletSource`] the
//! wallets (their entries, ERA's numbers from the three-way match, and every reconciliation),
//! [`GatewaySource`] the model gateway's usage in `control.db` (its `usage` table). [`Activity`] merges them into
//! rows; [`router`] serves `/activity` (the page), `/activity/rows.json` and
//! `/activity/stream` (server-sent events) behind an [`OperatorGate`]. A source that isn't
//! connected is named on the page as not connected; nothing is made up to fill it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adk_core::audit_log::AuditLogStore;
use async_trait::async_trait;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::{DateTime, FixedOffset, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::apps::Wallets;
use crate::feed::FeedSource;
use crate::pages;
use saas_core::PayPalRef;
use saas_core::wallet::{
    Authority, MatchState, SyncState, WalletEntryKind, WalletSource as Recorded,
};

/// How a policy decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Allowed inside the policy (a mandate, a payer's approval).
    Allowed,
    /// Held for the owner.
    Held,
    /// Denied.
    Denied,
    /// The owner approved it.
    Approved,
    /// The owner refused it.
    Rejected,
}

impl Decision {
    fn refusal(self) -> bool {
        matches!(self, Decision::Denied | Decision::Rejected)
    }
}

/// One thing an agent did, as one record says it.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    /// Unique across sources (`audit:kilele:412`).
    pub id: String,
    /// `audit`, `wallet` or `gateway`.
    pub source: &'static str,
    /// When.
    pub at: DateTime<Utc>,
    /// `agentrix`, `kilele`.
    pub company: String,
    /// The agent's role.
    pub agent: Option<String>,
    /// What woke it: a schedule, a webhook, the owner, a message.
    pub woke_by: Option<String>,
    /// One line: what it did.
    pub what: String,
    /// The model, when a model was called.
    pub model: Option<String>,
    /// Tokens in.
    pub tokens_in: Option<i64>,
    /// Tokens out.
    pub tokens_out: Option<i64>,
    /// Cost in millionths of a dollar.
    pub cost_micro_usd: Option<i64>,
    /// The tool called.
    pub tool: Option<String>,
    /// The policy's decision.
    pub decision: Option<Decision>,
    /// Why it was denied or held, or on what authority it was allowed.
    pub reason: Option<String>,
    /// The currency moved.
    pub currency: Option<String>,
    /// The amount moved, signed: money out is negative.
    pub amount_minor: Option<i64>,
    /// The PayPal object.
    pub paypal_id: Option<String>,
    /// ERA's payment and journal numbers.
    pub era: Option<String>,
    /// The wallet checkpoint covering it: `matches` or `drift`.
    pub checkpoint: Option<String>,
}

impl Row {
    fn new(
        source: &'static str,
        id: String,
        at: DateTime<Utc>,
        company: &str,
        what: String,
    ) -> Self {
        Self {
            id,
            source,
            at,
            company: company.to_owned(),
            agent: None,
            woke_by: None,
            what,
            model: None,
            tokens_in: None,
            tokens_out: None,
            cost_micro_usd: None,
            tool: None,
            decision: None,
            reason: None,
            currency: None,
            amount_minor: None,
            paypal_id: None,
            era: None,
            checkpoint: None,
        }
    }

    fn moves_money(&self) -> bool {
        self.amount_minor.is_some() || self.paypal_id.is_some()
    }
}

/// East Africa Time, which the page speaks.
fn eat() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"))
}

/// Midnight today in EAT, as UTC.
fn today_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let local = now
        .with_timezone(&eat())
        .date_naive()
        .and_time(NaiveTime::MIN);
    eat()
        .from_local_datetime(&local)
        .single()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(now)
}

fn money(minor: i64, currency: &str) -> String {
    crate::feed::money(minor, currency)
}

fn cost_words(micro: i64) -> String {
    if micro < 10_000 {
        format!("${:.4}", micro as f64 / 1_000_000.0)
    } else {
        format!("${:.2}", micro as f64 / 1_000_000.0)
    }
}

/// A row as the page shows it.
fn row_view(r: &Row, paypal_link: &str, now: DateTime<Utc>) -> Value {
    let local = r.at.with_timezone(&eat());
    let when = if r.at >= today_start(now) {
        local.format("%H:%M:%S").to_string()
    } else {
        local.format("%d %b %H:%M").to_string()
    };
    let usage = match (&r.model, r.tokens_in, r.tokens_out, r.cost_micro_usd) {
        (None, None, None, None) => None,
        (m, i, o, c) => Some(
            [
                m.clone(),
                match (i, o) {
                    (Some(i), Some(o)) => Some(format!("{i} in · {o} out")),
                    _ => None,
                },
                c.map(cost_words),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · "),
        ),
    };
    json!({
        "id": r.id,
        "anchor": anchor(&r.id),
        "source": r.source,
        "at": r.at,
        "when": when,
        "company": r.company,
        "agent": r.agent,
        "woke_by": r.woke_by,
        "what": r.what,
        "usage": usage,
        "tool": r.tool,
        "decision": r.decision,
        "reason": r.reason,
        "amount": match (r.amount_minor, &r.currency) {
            (Some(m), Some(c)) => Some(money(m, c)),
            _ => None,
        },
        "paypal_id": r.paypal_id,
        "paypal_href": r.paypal_id.as_ref().map(|id| paypal_link.replace("{id}", id)),
        "era": r.era,
        "checkpoint": r.checkpoint,
        "money": r.moves_money(),
        "refusal": r.decision.is_some_and(Decision::refusal),
    })
}

/// A row's id as an HTML anchor: `audit:kilele-money:69` → `row-audit-kilele-money-69`.
fn anchor(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("row-{safe}")
}

/// A source of rows.
#[async_trait]
pub trait Source: Send + Sync {
    /// Its name on the page.
    fn name(&self) -> String;
    /// What is new since the last read, in any order.
    async fn poll(&self) -> anyhow::Result<Vec<Row>>;
    /// Model spend since `t`, in millionths of a dollar, when this source meters it.
    async fn spend_since(&self, _t: DateTime<Utc>) -> Option<i64> {
        None
    }
    /// The latest reconciliation with PayPal: (words, `matches`|`drift`, when).
    async fn reconciliation(&self) -> Option<(String, String, DateTime<Utc>)> {
        None
    }
}

// ---- the signed audit logs ----------------------------------------------------------------

/// The signed audit logs (adk-rust's `AuditLogStore`): every tool call, approval, refusal and
/// payment a company's agents made, as the log records it. Facts the record carries in its
/// `ext` are read by name: `woke_by`, `reason`, `paypal_id`, `era_journal`, `era_payment`,
/// `amount_minor`, `currency`, `model`, `tokens_in`, `tokens_out`, `cost_micro_usd`, `tool`.
pub struct AuditSource {
    logs: AuditLogs,
    next: Mutex<HashMap<String, u64>>,
}

/// (company, log id, the store it lives in).
pub type AuditLog = (String, String, Arc<dyn AuditLogStore>);

/// A function that names the logs to read, each time.
pub type AuditLogsFn = Arc<dyn Fn() -> Vec<AuditLog> + Send + Sync>;

enum AuditLogs {
    Fixed(Arc<dyn AuditLogStore>, HashMap<String, String>),
    Each(AuditLogsFn),
}

/// How far back the first read goes, per log.
const FIRST_READ: u64 = 300;

impl AuditSource {
    /// `logs` maps a log id to its company (`agentrix`, `kilele`); empty reads every log, each
    /// named by its id.
    pub fn new(store: Arc<dyn AuditLogStore>, logs: HashMap<String, String>) -> Self {
        Self {
            logs: AuditLogs::Fixed(store, logs),
            next: Mutex::new(HashMap::new()),
        }
    }

    /// The logs `logs` returns at each read (saas-node's `platform::audit_logs`, empty until
    /// the money layer has started): each company's log in its own store.
    pub fn each(logs: impl Fn() -> Vec<AuditLog> + Send + Sync + 'static) -> Self {
        Self {
            logs: AuditLogs::Each(Arc::new(logs)),
            next: Mutex::new(HashMap::new()),
        }
    }

    async fn logs(&self) -> anyhow::Result<Vec<AuditLog>> {
        Ok(match &self.logs {
            AuditLogs::Fixed(store, map) if map.is_empty() => store
                .logs()
                .await?
                .into_iter()
                .map(|l| (l.clone(), l, Arc::clone(store)))
                .collect(),
            AuditLogs::Fixed(store, map) => map
                .iter()
                .map(|(l, c)| (c.clone(), l.clone(), Arc::clone(store)))
                .collect(),
            AuditLogs::Each(f) => f(),
        })
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(t) if !t.is_empty() => Some(t.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k)
        .and_then(|x| x.as_i64().or_else(|| x.as_str()?.parse().ok()))
}

/// `kilele-bakery/payables` or `urn:adk:agent:kilele-bakery/payables` → `payables`.
fn role_of(agent: &str) -> String {
    agent.rsplit(['/', ':']).next().unwrap_or(agent).to_owned()
}

/// One audit record (its canonical JSON) as a row.
pub fn audit_row(company: &str, log: &str, seq: u64, record: &Value) -> Row {
    let at = record
        .get("ts")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let kind = s(record, "kind").unwrap_or_default();
    let action = s(record, "action").unwrap_or_default();
    let ext = record.get("ext").cloned().unwrap_or(Value::Null);
    let what = match kind.as_str() {
        "tool_call" | "" => action.replace('_', " "),
        k => format!("{}: {}", k.replace('_', " "), action.replace('_', " ")),
    };
    let mut row = Row::new("audit", format!("audit:{log}:{seq}"), at, company, what);
    row.agent = record
        .get("actor")
        .and_then(|a| a.get("agent"))
        .and_then(Value::as_str)
        .or_else(|| ext.get("agent").and_then(Value::as_str))
        .map(role_of);
    row.decision = match record.get("decision").and_then(Value::as_str) {
        Some("allowed") => Some(Decision::Allowed),
        Some("denied") => Some(Decision::Denied),
        Some("approval_required") => Some(Decision::Held),
        Some("approved") => Some(Decision::Approved),
        Some("rejected") => Some(Decision::Rejected),
        _ => None,
    };
    row.reason = s(&ext, "reason").or_else(|| {
        row.decision
            .filter(|d| d.refusal() || *d == Decision::Held)
            .and(s(record, "policyRef").map(|p| format!("policy {p}")))
    });
    row.tool = s(&ext, "tool").or_else(|| (kind == "tool_call").then(|| action.clone()));
    row.woke_by = s(&ext, "woke_by");
    row.model = s(&ext, "model");
    row.tokens_in = i(&ext, "tokens_in");
    row.tokens_out = i(&ext, "tokens_out");
    row.cost_micro_usd = i(&ext, "cost_micro_usd");
    row.paypal_id = s(&ext, "paypal_id");
    row.amount_minor = i(&ext, "amount_minor");
    row.currency = s(&ext, "currency").or_else(|| row.amount_minor.map(|_| "USD".to_owned()));
    let era: Vec<String> = ["era_document", "era_payment", "era_journal"]
        .iter()
        .filter_map(|k| s(&ext, k))
        .collect();
    row.era = (!era.is_empty()).then(|| era.join(" · "));
    // What woke a governed tool call: its session (`cron-reconcile` is the reconcile schedule).
    if row.woke_by.is_none() {
        row.woke_by = s(&ext, "sessionId").map(|id| session_words(&id));
    }
    if row.decision == Some(Decision::Allowed)
        && row.reason.is_none()
        && s(&ext, "effect").as_deref() == Some("read_only")
    {
        row.reason = Some("read only".into());
    }
    // The records saas-node writes, each in words.
    match action.as_str() {
        "kernel.started" => row.what = "kernel started".into(),
        "kernel.process.spawned" => {
            let kind = s(&ext, "kind").unwrap_or_default();
            row.what = if kind == "scheduled" {
                "woke on its schedule".into()
            } else if kind.is_empty() {
                "process started".into()
            } else {
                format!("process started ({kind})")
            };
            row.woke_by = row
                .woke_by
                .take()
                .or_else(|| ext.get("wake").and_then(wake_words));
        }
        "kernel.process.suspended" => {
            row.what = match s(&ext, "reason").as_deref() {
                Some("asleep") | None => "went back to sleep".into(),
                Some(r) => format!("suspended: {r}"),
            };
        }
        "governance.freeze" => row.what = "froze every money tool".into(),
        "governance.unfreeze" => row.what = "unfroze the money tools".into(),
        "model.call" => {
            row.what = format!(
                "model call: {}",
                s(&ext, "purpose")
                    .unwrap_or_default()
                    .replace(['.', '-'], " ")
            );
        }
        "a2a.refused" => {
            row.what = format!(
                "door refused {}",
                s(&ext, "requester").unwrap_or_else(|| "an agent".into())
            );
            row.decision = Some(Decision::Denied);
            row.reason = s(&ext, "reason").or_else(|| s(&ext, "code")).map(|c| {
                if ext.get("nothing_moved").and_then(Value::as_bool) == Some(true) {
                    format!("{c}; nothing moved")
                } else {
                    c
                }
            });
            row.woke_by = row.woke_by.take().or_else(|| Some("agent door".into()));
        }
        "agent.revoke" | "agent.restore" => {
            let who = s(&ext, "agent").map(|a| role_of(&a)).unwrap_or_default();
            let verb = if action == "agent.revoke" {
                "revoked"
            } else {
                "restored"
            };
            row.what = match i(&ext, "tokens_dropped") {
                Some(n) => format!("{verb} {who} ({n} tokens dropped)"),
                None => format!("{verb} {who}"),
            };
            row.reason = row.reason.take().or_else(|| s(&ext, "why"));
            row.woke_by = row.woke_by.take().or_else(|| s(&ext, "how"));
        }
        _ => {}
    }
    if let Some(outcome) = s(record, "outcome").filter(|o| o == "failed" || o == "outcome_unknown")
    {
        row.what = format!("{} ({})", row.what, outcome.replace('_', " "));
    }
    row
}

#[async_trait]
impl Source for AuditSource {
    fn name(&self) -> String {
        "Signed audit logs".into()
    }

    async fn poll(&self) -> anyhow::Result<Vec<Row>> {
        let logs = self.logs().await?;
        if logs.is_empty() {
            anyhow::bail!("no audit logs yet: the money layer hasn't started");
        }
        let mut rows = Vec::new();
        for (company, log, store) in logs {
            let size = store.size(&log).await?;
            let from = self
                .next
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&log)
                .copied()
                .unwrap_or_else(|| size.saturating_sub(FIRST_READ));
            let mut at = from;
            while at < size {
                let to = (at + 200).min(size);
                for entry in store.read(&log, at..to).await? {
                    let Some(text) = entry.record else { continue };
                    let Ok(record) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    rows.push(audit_row(&company, &log, entry.seq, &record));
                }
                at = to;
            }
            self.next
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(log, size);
        }
        Ok(rows)
    }
}

/// A session id in words: `cron-reconcile` → "schedule: reconcile".
fn session_words(id: &str) -> String {
    if let Some(rest) = id.strip_prefix("cron-") {
        format!("schedule: {}", rest.replace('-', " "))
    } else if id.starts_with("story-") {
        "the story's run".into()
    } else {
        format!("session {id}")
    }
}

/// A kernel wake condition in words: `{"anyOf":[{"cron":{"expr":"0 30 */3 * * *"}}]}` →
/// "schedule: every 3 hours at :30".
fn wake_words(wake: &Value) -> Option<String> {
    let exprs: Vec<String> = wake
        .get("anyOf")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|c| c.get("cron")?.get("expr")?.as_str().map(cron_words))
        .collect();
    (!exprs.is_empty()).then(|| format!("schedule: {}", exprs.join(" or ")))
}

/// A six-field cron (seconds first) in words, for the shapes the companies use; otherwise the
/// expression itself.
fn cron_words(expr: &str) -> String {
    let f: Vec<&str> = expr.split_whitespace().collect();
    let [sec, min, hour, dom, mon, dow] = f.as_slice() else {
        return format!("cron {expr}");
    };
    if *sec != "0"
        || *dom != "*"
        || *mon != "*"
        || *dow != "*"
        || !min.bytes().all(|b| b.is_ascii_digit())
    {
        return format!("cron {expr}");
    }
    let at = format!(":{min:0>2}");
    match *hour {
        "*" => format!("every hour at {at}"),
        h if h.starts_with("*/") => format!("every {} hours at {at}", &h[2..]),
        h if h.bytes().all(|b| b.is_ascii_digit()) => format!("daily at {h:0>2}{at} UTC"),
        _ => format!("cron {expr}"),
    }
}

// ---- the wallets --------------------------------------------------------------------------

/// The wallets: each PayPal move an agent made, with ERA's numbers from the three-way match,
/// and every reconciliation with PayPal.
pub struct WalletSource {
    wallets: Arc<dyn Wallets>,
    sites: Vec<(String, String)>,
    seen: Mutex<HashSet<String>>,
    latest: Mutex<Option<(String, String, DateTime<Utc>)>>,
}

impl WalletSource {
    /// `sites`: (site id the wallets know, company it shows as).
    pub fn new(wallets: Arc<dyn Wallets>, sites: Vec<(String, String)>) -> Self {
        Self {
            wallets,
            sites,
            seen: Mutex::new(HashSet::new()),
            latest: Mutex::new(None),
        }
    }
}

fn paypal_id(r: &PayPalRef) -> String {
    match r {
        PayPalRef::Capture(id) => id.to_string(),
        PayPalRef::SubscriptionSale(id) => id.to_string(),
        PayPalRef::Refund(id) => id.to_string(),
        PayPalRef::Payout(id) => id.to_string(),
    }
}

fn state_name(s: SyncState) -> String {
    match s {
        SyncState::Matches => "matches".into(),
        SyncState::Drift => "drift".into(),
    }
}

#[async_trait]
impl Source for WalletSource {
    fn name(&self) -> String {
        "Wallets and ERA postings".into()
    }

    async fn poll(&self) -> anyhow::Result<Vec<Row>> {
        let mut rows = Vec::new();
        let mut errors = Vec::new();
        for (site, company) in &self.sites {
            let feed = match self.wallets.wallet(site).await {
                Ok(f) => f,
                Err(e) => {
                    errors.push(format!("{site}: {e}"));
                    continue;
                }
            };
            if let Some(c) = &feed.checkpoint {
                let mut latest = self.latest.lock().unwrap_or_else(|e| e.into_inner());
                if latest.as_ref().is_none_or(|(_, _, at)| c.checked_at > *at) {
                    *latest = Some((c.words.clone(), state_name(c.state), c.checked_at));
                }
            }
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            for e in &feed.entries {
                let id = format!("wallet:{site}:{}", e.seq);
                if !seen.insert(id.clone()) {
                    continue;
                }
                let kind = serde_json::to_value(e.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let with = e
                    .counterparty
                    .as_ref()
                    .map(|c| {
                        let dir = if e.amount.minor < 0 { "to" } else { "from" };
                        format!(" {dir} {}", c.name)
                    })
                    .unwrap_or_default();
                let mut row = Row::new("wallet", id, e.at, company, format!("{kind}{with}"));
                row.agent = Some(e.acting.name().to_owned());
                row.woke_by = Some(match &e.source {
                    Recorded::Webhook { event_type, .. } => format!("PayPal webhook {event_type}"),
                    Recorded::ReadBack { call } => format!("read back: {call}"),
                });
                (row.decision, row.reason) = match &e.authority {
                    Authority::Mandate { .. } => {
                        (Some(Decision::Allowed), Some("inside the mandate".into()))
                    }
                    Authority::Approval { approved_by, .. } => (
                        Some(Decision::Approved),
                        Some(format!("approved by {approved_by}")),
                    ),
                    Authority::PayerApproved => (
                        Some(Decision::Allowed),
                        Some("the payer approved it at PayPal".into()),
                    ),
                };
                row.currency = Some(e.amount.currency.code().to_owned());
                row.amount_minor = Some(e.amount.minor);
                row.paypal_id = Some(paypal_id(&e.paypal));
                if e.kind == WalletEntryKind::Fee {
                    row.tool = None;
                }
                if let Some(m) = feed.matches.iter().find(|m| m.seq == e.seq) {
                    let b = &m.books;
                    let journal = b
                        .journal_number
                        .clone()
                        .or_else(|| b.journal.as_ref().map(|j| j.0.clone()));
                    let payment = b
                        .payment_number
                        .clone()
                        .or_else(|| b.payment.as_ref().map(|p| p.0.clone()));
                    row.era = match (payment, journal) {
                        (Some(p), Some(j)) => Some(format!("{p} · {j}")),
                        (p, j) => p.or(j),
                    };
                    if m.state != MatchState::Matched {
                        let state = serde_json::to_value(m.state)
                            .ok()
                            .and_then(|v| v.as_str().map(|s| s.replace('_', " ")))
                            .unwrap_or_default();
                        row.reason = Some(format!("three-way match: {state}"));
                    }
                }
                row.checkpoint = feed
                    .checkpoint
                    .as_ref()
                    .filter(|c| c.size > e.seq)
                    .map(|c| state_name(c.state));
                rows.push(row);
            }
            for c in &feed.history {
                let id = format!("wallet:{site}:checkpoint:{}", c.size);
                if !seen.insert(id.clone()) {
                    continue;
                }
                let mut row = Row::new(
                    "wallet",
                    id,
                    c.checked_at,
                    company,
                    format!("Reconciled with PayPal: {} ({} entries)", c.words, c.size),
                );
                row.agent = Some(feed.wallet.name().to_owned());
                row.checkpoint = Some(state_name(c.state));
                rows.push(row);
            }
        }
        if rows.is_empty() && !errors.is_empty() {
            anyhow::bail!("{}", errors.join("; "));
        }
        Ok(rows)
    }

    async fn reconciliation(&self) -> Option<(String, String, DateTime<Utc>)> {
        self.latest
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Each site's own wallet seam, read as one (`site` → its wallets).
pub struct PerSite(pub HashMap<String, Arc<dyn Wallets>>);

#[async_trait]
impl Wallets for PerSite {
    async fn wallet(
        &self,
        site: &str,
    ) -> Result<saas_core::wallet::WalletFeed, crate::apps::AppError> {
        match self.0.get(site) {
            Some(w) => w.wallet(site).await,
            None => Err(crate::apps::AppError::NotFound),
        }
    }
}

// ---- the model gateway -------------------------------------------------------------------

/// The model gateway's usage, from `control.db`'s `usage` table (agentrix-store's 0801 builds
/// it as `usage_open` and renames it `usage`): each model call, its model,
/// tokens and cost, by site, WebAgent and job.
pub struct GatewaySource {
    pool: sqlx::SqlitePool,
    sites: HashMap<String, String>,
    last: Mutex<Option<i64>>,
    spend_only: bool,
}

impl GatewaySource {
    /// `sites` maps control.db's `site_id` to the company it shows as; empty shows every site
    /// by its id.
    pub fn new(pool: sqlx::SqlitePool, sites: HashMap<String, String>) -> Self {
        Self {
            pool,
            sites,
            last: Mutex::new(None),
            spend_only: false,
        }
    }

    /// Only today's spend, no rows: for a node whose audit log already records each model
    /// call (`model.call`), so a call isn't listed twice.
    pub fn spend_only(mut self) -> Self {
        self.spend_only = true;
        self
    }

    fn company(&self, site: &str) -> Option<String> {
        if self.sites.is_empty() {
            Some(site.to_owned())
        } else {
            self.sites.get(site).cloned()
        }
    }
}

type UsageRow = (
    i64,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn parse_at(t: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(t)
        .map(|t| t.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").map(|n| n.and_utc())
        })
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f").map(|n| n.and_utc())
        })
        .unwrap_or_else(|_| Utc::now())
}

#[async_trait]
impl Source for GatewaySource {
    fn name(&self) -> String {
        if self.spend_only {
            "Model gateway spend (control.db)".into()
        } else {
            "Model gateway usage (control.db)".into()
        }
    }

    async fn poll(&self) -> anyhow::Result<Vec<Row>> {
        if self.spend_only {
            // Still a read, so a broken control.db shows as not connected.
            sqlx::query("SELECT 1 FROM usage LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;
            return Ok(Vec::new());
        }
        let last = *self.last.lock().unwrap_or_else(|e| e.into_inner());
        let found: Vec<UsageRow> = match last {
            Some(id) => {
                sqlx::query_as(
                    "SELECT usage_id, site_id, purpose, model, tokens_in, tokens_out, cost_micro_usd, outcome, at, webagent, job FROM usage WHERE usage_id > ? ORDER BY usage_id LIMIT 500",
                )
                .bind(id)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query_as(
                    "SELECT usage_id, site_id, purpose, model, tokens_in, tokens_out, cost_micro_usd, outcome, at, webagent, job FROM usage ORDER BY usage_id DESC LIMIT 300",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        let mut rows = Vec::new();
        let mut max = last;
        for (id, site, purpose, model, tin, tout, cost, outcome, at, webagent, job) in found {
            max = Some(max.map_or(id, |m: i64| m.max(id)));
            let Some(company) = self.company(&site) else {
                continue;
            };
            let mut row = Row::new(
                "gateway",
                format!("gateway:{id}"),
                parse_at(&at),
                &company,
                format!("model call: {}", purpose.replace(['.', '-'], " ")),
            );
            row.agent = webagent;
            row.woke_by = job.map(|j| format!("job {j}"));
            row.model = Some(model);
            row.tokens_in = Some(tin);
            row.tokens_out = Some(tout);
            row.cost_micro_usd = Some(cost);
            match outcome.as_str() {
                "over-budget" => {
                    row.decision = Some(Decision::Denied);
                    row.reason = Some("over the model budget".into());
                }
                "error" => row.what = format!("{} (failed)", row.what),
                _ => {}
            }
            rows.push(row);
        }
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = max;
        Ok(rows)
    }

    async fn spend_since(&self, t: DateTime<Utc>) -> Option<i64> {
        let sites: Vec<String> = self.sites.keys().cloned().collect();
        // The gateway writes `at` as RFC 3339 with microseconds and `Z`; text compares in order.
        let since = t.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        if sites.is_empty() {
            let (n,): (i64,) =
                sqlx::query_as("SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE at >= ?")
                    .bind(since)
                    .fetch_one(&self.pool)
                    .await
                    .ok()?;
            return Some(n);
        }
        let mut total = 0;
        for site in &sites {
            let (n,): (i64,) = sqlx::query_as(
                "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE at >= ? AND site_id = ?",
            )
            .bind(&since)
            .bind(site)
            .fetch_one(&self.pool)
            .await
            .ok()?;
            total += n;
        }
        Some(total)
    }
}

// ---- the test customer agents' visits -------------------------------------------------------

/// The test customer agents' visits to the sample sites, from the service's own log
/// (`<SAAS_DATA_DIR>/test-agents/activity.jsonl`: one JSON object a line with `at`, `agent`,
/// `kind`, `ok`, `site`, `detail`). Each line is read once.
pub struct VisitsSource {
    path: std::path::PathBuf,
    offset: Mutex<Option<u64>>,
}

/// How many lines the first read keeps.
const FIRST_LINES: usize = 300;

impl VisitsSource {
    /// The log at `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: Mutex::new(None),
        }
    }
}

/// `https://kilele-bakery-sample.paypal.agentrix.website` → `kilele-bakery-sample`.
fn site_label(site: &str) -> String {
    let host = site
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(['/', ':'])
        .next()
        .unwrap_or(site);
    host.split('.').next().unwrap_or(host).to_owned()
}

/// One line of the visits log as a row; `None` for a line that isn't one.
pub fn visit_row(offset: u64, line: &str) -> Option<Row> {
    let v: Value = serde_json::from_str(line).ok()?;
    let at = parse_at(v.get("at")?.as_str()?);
    let site = s(&v, "site")
        .map(|x| site_label(&x))
        .unwrap_or_else(|| "a sample site".into());
    let kind = s(&v, "kind").unwrap_or_else(|| "visit".into());
    let detail = s(&v, "detail").unwrap_or_default();
    let mut row = Row::new(
        "visits",
        format!("visit:{offset}"),
        at,
        &site,
        if detail.is_empty() {
            kind.clone()
        } else {
            format!("{kind}: {detail}")
        },
    );
    row.agent = s(&v, "agent");
    row.woke_by = Some("test customer agent".into());
    if v.get("ok").and_then(Value::as_bool) == Some(false) {
        row.what = format!("{} (not ok)", row.what);
    }
    Some(row)
}

#[async_trait]
impl Source for VisitsSource {
    fn name(&self) -> String {
        "Test customer agents' visits".into()
    }

    async fn poll(&self) -> anyhow::Result<Vec<Row>> {
        let text = match tokio::fs::read(&self.path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                anyhow::bail!("no visits logged yet")
            }
            Err(e) => return Err(e.into()),
        };
        let len = text.len() as u64;
        let (from, first) = {
            let mut offset = self.offset.lock().unwrap_or_else(|e| e.into_inner());
            // A shorter file was rotated: start again.
            let (from, first) = match *offset {
                Some(o) if o <= len => (o as usize, false),
                _ => (0, true),
            };
            // Only whole lines: one still being written waits for the next read.
            let end = text[from..]
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(from, |p| from + p + 1);
            *offset = Some(end as u64);
            (from..end, first)
        };
        let mut rows = Vec::new();
        let mut at = from.start;
        for line in text[from].split(|b| *b == b'\n') {
            let start = at as u64;
            at += line.len() + 1;
            let Ok(line) = std::str::from_utf8(line) else {
                continue;
            };
            if let Some(r) = visit_row(start, line.trim()) {
                rows.push(r);
            }
        }
        if first && rows.len() > FIRST_LINES {
            rows.drain(..rows.len() - FIRST_LINES);
        }
        Ok(rows)
    }
}

// ---- merging them -------------------------------------------------------------------------

/// The model spend cap the totals compare against.
#[derive(Debug, Clone, Serialize)]
pub struct SpendCap {
    /// Millionths of a dollar.
    pub micro_usd: i64,
    /// `day` or `month`.
    pub per: String,
}

#[derive(Debug, Clone, Serialize)]
struct SourceStatus {
    name: String,
    connected: bool,
    note: Option<String>,
}

/// The rows kept in memory (newest kept).
const KEEP: usize = 5000;

struct Inner {
    sources: Vec<Arc<dyn Source>>,
    missing: Vec<String>,
    feed: Option<(Arc<dyn FeedSource>, crate::agent::Today)>,
    cap: Option<SpendCap>,
    paypal_link: String,
    rows: Mutex<Vec<(u64, Row)>>,
    status: Mutex<Vec<SourceStatus>>,
    n: AtomicU64,
    polled: Mutex<Option<Instant>>,
    polling: tokio::sync::Mutex<()>,
}

/// Every source merged, newest first.
#[derive(Clone)]
pub struct Activity {
    inner: Arc<Inner>,
}

impl Default for Activity {
    fn default() -> Self {
        Self::new()
    }
}

/// What a filter keeps.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Filter {
    /// One company.
    pub company: Option<String>,
    /// One agent (role).
    pub agent: Option<String>,
    /// Only rows that move money.
    pub money: Option<String>,
    /// Only refusals.
    pub refusals: Option<String>,
}

impl Filter {
    fn on(v: &Option<String>) -> bool {
        matches!(v.as_deref(), Some("1" | "on" | "true"))
    }

    fn keeps(&self, r: &Row) -> bool {
        self.company
            .as_deref()
            .filter(|c| !c.is_empty())
            .is_none_or(|c| r.company == c)
            && self
                .agent
                .as_deref()
                .filter(|a| !a.is_empty())
                .is_none_or(|a| r.agent.as_deref() == Some(a))
            && (!Self::on(&self.money) || r.moves_money())
            && (!Self::on(&self.refusals) || r.decision.is_some_and(Decision::refusal))
    }

    fn query(&self) -> String {
        let mut q = Vec::new();
        for (k, v) in [("company", &self.company), ("agent", &self.agent)] {
            if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                q.push(format!("{k}={}", urlencode(v)));
            }
        }
        if Self::on(&self.money) {
            q.push("money=1".into());
        }
        if Self::on(&self.refusals) {
            q.push("refusals=1".into());
        }
        q.join("&")
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

impl Activity {
    /// No sources yet: every expected one shows as not connected.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                sources: Vec::new(),
                missing: Vec::new(),
                feed: None,
                cap: None,
                paypal_link: "https://www.sandbox.paypal.com/activity/payment/{id}".into(),
                rows: Mutex::new(Vec::new()),
                status: Mutex::new(Vec::new()),
                n: AtomicU64::new(0),
                polled: Mutex::new(None),
                polling: tokio::sync::Mutex::new(()),
            }),
        }
    }

    fn edit(mut self, f: impl FnOnce(&mut Inner)) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            f(inner);
        }
        self
    }

    /// Reads `source`.
    pub fn with_source(self, source: Arc<dyn Source>) -> Self {
        self.edit(|i| i.sources.push(source))
    }

    /// Names a source this node doesn't connect, so the page says so.
    pub fn not_connected(self, name: &str) -> Self {
        let name = name.to_owned();
        self.edit(|i| i.missing.push(name))
    }

    /// Counts the decisions waiting for owners from `feed`.
    pub fn with_feed(self, feed: Arc<dyn FeedSource>, today: crate::agent::Today) -> Self {
        self.edit(|i| i.feed = Some((feed, today)))
    }

    /// Compares model spend against `cap`.
    pub fn with_cap(self, cap: Option<SpendCap>) -> Self {
        self.edit(|i| i.cap = cap)
    }

    /// Links a PayPal id with `link` (`{id}` is replaced).
    pub fn with_paypal_link(self, link: &str) -> Self {
        let link = link.to_owned();
        self.edit(|i| i.paypal_link = link)
    }

    /// Reads every source, at most once every two seconds; new rows get the next numbers.
    pub async fn refresh(&self) {
        {
            let polled = self.inner.polled.lock().unwrap_or_else(|e| e.into_inner());
            if polled.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
                return;
            }
        }
        let Ok(_guard) = self.inner.polling.try_lock() else {
            return;
        };
        let mut status = Vec::new();
        let mut fresh = Vec::new();
        for source in &self.inner.sources {
            match source.poll().await {
                Ok(rows) => {
                    status.push(SourceStatus {
                        name: source.name(),
                        connected: true,
                        note: None,
                    });
                    fresh.extend(rows);
                }
                Err(e) => status.push(SourceStatus {
                    name: source.name(),
                    connected: false,
                    note: Some(format!("didn't answer: {e}")),
                }),
            }
        }
        for name in &self.inner.missing {
            status.push(SourceStatus {
                name: name.clone(),
                connected: false,
                note: Some("not connected on this node".into()),
            });
        }
        fresh.sort_by_key(|r| r.at);
        {
            let mut rows = self.inner.rows.lock().unwrap_or_else(|e| e.into_inner());
            for r in fresh {
                let n = self.inner.n.fetch_add(1, Ordering::Relaxed) + 1;
                rows.push((n, r));
            }
            if rows.len() > KEEP {
                let drop = rows.len() - KEEP;
                rows.drain(..drop);
            }
        }
        *self.inner.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
        *self.inner.polled.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }

    /// Rows `filter` keeps, newest first, at most `limit`.
    pub fn rows(&self, filter: &Filter, limit: usize) -> Vec<Row> {
        let rows = self.inner.rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<&(u64, Row)> = rows.iter().filter(|(_, r)| filter.keeps(r)).collect();
        out.sort_by(|a, b| b.1.at.cmp(&a.1.at).then(b.0.cmp(&a.0)));
        out.into_iter()
            .take(limit)
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Rows numbered after `n` that `filter` keeps, oldest first, and the highest number.
    fn after(&self, n: u64, filter: &Filter) -> (Vec<Row>, u64) {
        let rows = self.inner.rows.lock().unwrap_or_else(|e| e.into_inner());
        let top = rows.last().map_or(n, |(m, _)| *m).max(n);
        (
            rows.iter()
                .filter(|(m, r)| *m > n && filter.keeps(r))
                .map(|(_, r)| r.clone())
                .collect(),
            top,
        )
    }

    fn top(&self) -> u64 {
        self.inner
            .rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .map_or(0, |(n, _)| *n)
    }

    /// The strip at the top: money moved today, decisions waiting, refusals today, model
    /// spend today against the cap, the last reconciliation.
    pub async fn totals(&self, now: DateTime<Utc>) -> Value {
        let start = today_start(now);
        let (moved, refusals, metered) = {
            let rows = self.inner.rows.lock().unwrap_or_else(|e| e.into_inner());
            let mut moved: BTreeMap<String, (i64, i64)> = BTreeMap::new();
            let mut refusals = 0;
            let mut metered = 0i64;
            for (_, r) in rows.iter().filter(|(_, r)| r.at >= start) {
                if r.source == "wallet" {
                    if let (Some(m), Some(c)) = (r.amount_minor, &r.currency) {
                        let e = moved.entry(c.clone()).or_default();
                        if m >= 0 {
                            e.0 += m;
                        } else {
                            e.1 += -m;
                        }
                    }
                }
                if r.decision.is_some_and(Decision::refusal) {
                    refusals += 1;
                }
                if r.source != "gateway" {
                    metered += r.cost_micro_usd.unwrap_or(0);
                }
            }
            (moved, refusals, metered)
        };
        let has_wallet = self
            .inner
            .sources
            .iter()
            .any(|s| s.name().starts_with("Wallets"));
        let money: Vec<String> = moved
            .iter()
            .map(|(c, (i, o))| format!("{} in · {} out", money(*i, c), money(*o, c)))
            .collect();
        let mut spend = None;
        let mut reconciliation = None;
        for s in &self.inner.sources {
            if let Some(n) = s.spend_since(start).await {
                spend = Some(spend.unwrap_or(0) + n);
            }
            if let Some(r) = s.reconciliation().await {
                if reconciliation
                    .as_ref()
                    .is_none_or(|(_, _, at): &(String, String, DateTime<Utc>)| r.2 > *at)
                {
                    reconciliation = Some(r);
                }
            }
        }
        if spend.is_none() && metered > 0 {
            spend = Some(metered);
        }
        let held = match &self.inner.feed {
            Some((feed, today)) => feed
                .feed(today())
                .await
                .ok()
                .map(|f| f.decisions.iter().filter(|d| d.state == "open").count()),
            None => None,
        };
        json!({
            "money": if has_wallet { json!(if money.is_empty() { vec!["nothing moved yet today".to_owned()] } else { money }) } else { Value::Null },
            "held": held,
            "refusals": refusals,
            "spend": spend.map(cost_words),
            "cap": self.inner.cap.as_ref().map(|c| format!("{} a {}", cost_words(c.micro_usd), c.per)),
            "over_cap": match (spend, &self.inner.cap) {
                (Some(s), Some(c)) if c.per == "day" => s > c.micro_usd,
                _ => false,
            },
            "reconciliation": reconciliation.map(|(words, state, _)| json!({"words": words, "state": state})),
        })
    }

    /// The summary panel's HTML (see [`router`]'s `/activity/summary`): the totals and the
    /// newest ten rows, each linking to `/activity`. For a host page that renders it itself.
    pub async fn summary_html(&self) -> Option<String> {
        self.refresh().await;
        let now = Utc::now();
        let rows = self.rows(&Filter::default(), 10);
        let view: Vec<Value> = self
            .view(&rows, now)
            .into_iter()
            .map(|mut v| {
                let q = Filter {
                    company: v["company"].as_str().map(str::to_owned),
                    ..Filter::default()
                }
                .query();
                v["href"] = json!(format!(
                    "/activity?{q}#{}",
                    v["anchor"].as_str().unwrap_or("")
                ));
                v
            })
            .collect();
        pages::render_to_string(
            "saas/activity_summary.html",
            json!({
                "rows": view,
                "totals": self.totals(now).await,
                "sources": self.status(),
                "saas_v": pages::assets_version(),
            }),
        )
    }

    fn view(&self, rows: &[Row], now: DateTime<Utc>) -> Vec<Value> {
        rows.iter()
            .map(|r| row_view(r, &self.inner.paypal_link, now))
            .collect()
    }

    fn status(&self) -> Vec<SourceStatus> {
        self.inner
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The companies and agents seen so far, for the filters.
    fn choices(&self) -> (Vec<String>, Vec<String>) {
        let rows = self.inner.rows.lock().unwrap_or_else(|e| e.into_inner());
        let mut companies: Vec<String> = rows.iter().map(|(_, r)| r.company.clone()).collect();
        let mut agents: Vec<String> = rows.iter().filter_map(|(_, r)| r.agent.clone()).collect();
        companies.sort();
        companies.dedup();
        agents.sort();
        agents.dedup();
        (companies, agents)
    }
}

// ---- the page and the stream ---------------------------------------------------------------

/// Who may see the page: the node's signed-in operators.
#[async_trait]
pub trait OperatorGate: Send + Sync {
    /// Whether these request headers are a signed-in operator's.
    async fn allows(&self, headers: &HeaderMap) -> bool;
}

/// Anyone on a loopback or `.localhost` host, or carrying the owner's token: for a node run on
/// its own (saas-web's binary), never for the ops host.
pub struct LocalGate(pub crate::Access);

#[async_trait]
impl OperatorGate for LocalGate {
    async fn allows(&self, headers: &HeaderMap) -> bool {
        self.0.allows(headers)
    }
}

#[derive(Clone)]
struct Ctx {
    activity: Activity,
    gate: Arc<dyn OperatorGate>,
}

/// `/activity`, `/activity/rows.json` and `/activity/stream`, for operators only.
pub fn router(activity: Activity, gate: Arc<dyn OperatorGate>) -> Router {
    Router::new()
        .route("/activity", get(page))
        .route("/activity/rows.json", get(rows_json))
        .route("/activity/stream", get(stream))
        .route("/activity/summary", get(summary))
        .with_state(Ctx { activity, gate })
}

async fn page(State(ctx): State<Ctx>, Query(f): Query<Filter>, headers: HeaderMap) -> Response {
    if !ctx.gate.allows(&headers).await {
        return Redirect::to("/").into_response();
    }
    ctx.activity.refresh().await;
    let now = Utc::now();
    let rows = ctx.activity.rows(&f, 300);
    let (companies, agents) = ctx.activity.choices();
    let q = f.query();
    let view = json!({
        "rows": ctx.activity.view(&rows, now),
        "totals": ctx.activity.totals(now).await,
        "sources": ctx.activity.status(),
        "companies": companies,
        "agents": agents,
        "filter": {
            "company": f.company.clone().unwrap_or_default(),
            "agent": f.agent.clone().unwrap_or_default(),
            "money": Filter::on(&f.money),
            "refusals": Filter::on(&f.refusals),
        },
        "stream_url": if q.is_empty() { "/activity/stream".to_owned() } else { format!("/activity/stream?{q}") },
        "top": ctx.activity.top(),
    });
    let mut r = pages::render("saas/activity.html", view);
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    r
}

async fn rows_json(
    State(ctx): State<Ctx>,
    Query(f): Query<Filter>,
    headers: HeaderMap,
) -> Response {
    if !ctx.gate.allows(&headers).await {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"message": "Sign in as an operator."})),
        )
            .into_response();
    }
    ctx.activity.refresh().await;
    let now = Utc::now();
    let rows = ctx.activity.rows(&f, 500);
    (
        [(header::CACHE_CONTROL, "no-store")],
        axum::Json(json!({
            "rows": ctx.activity.view(&rows, now),
            "totals": ctx.activity.totals(now).await,
            "sources": ctx.activity.status(),
        })),
    )
        .into_response()
}

/// The panel the operator console embeds: the totals strip, the newest ten rows (each linking
/// to its place on `/activity`), and "Open full activity". `activity-summary.js` fetches it
/// again every few seconds.
async fn summary(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    if !ctx.gate.allows(&headers).await {
        return (StatusCode::UNAUTHORIZED, "Sign in as an operator.").into_response();
    }
    match ctx.activity.summary_html().await {
        Some(html) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            html,
        )
            .into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "The activity summary failed to render.",
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct StreamQuery {
    #[serde(flatten)]
    filter: Filter,
    after: Option<u64>,
}

async fn stream(
    State(ctx): State<Ctx>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
) -> Response {
    if !ctx.gate.allows(&headers).await {
        return (StatusCode::UNAUTHORIZED, "Sign in as an operator.").into_response();
    }
    let activity = ctx.activity.clone();
    let filter = q.filter;
    let start = q.after.unwrap_or_else(|| activity.top());
    let events = futures::stream::unfold(
        (activity, filter, start, true),
        |(a, f, n, first)| async move {
            if !first {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            a.refresh().await;
            let now = Utc::now();
            let (rows, top) = a.after(n, &f);
            let mut out: Vec<Result<Event, Infallible>> = a
                .view(&rows, now)
                .into_iter()
                .map(|v| Ok(Event::default().event("row").data(v.to_string())))
                .collect();
            out.push(Ok(Event::default()
                .event("totals")
                .data(a.totals(now).await.to_string())));
            out.push(Ok(Event::default()
                .event("sources")
                .data(json!(a.status()).to_string())));
            Some((futures::stream::iter(out), (a, f, top, false)))
        },
    );
    use futures::StreamExt;
    Sse::new(events.flatten())
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_audit_record_reads_as_a_row() {
        let record = json!({
            "v": 1, "logId": "kilele", "seq": 7, "ts": "2026-11-11T05:02:03Z",
            "actor": {"agent": "kilele-bakery/payables"},
            "kind": "tool_call", "action": "pay_bill", "decision": "denied", "outcome": "not_run",
            "policyRef": "mandate:kilele/payables",
            "ext": {"woke_by": "schedule 06:00 bills", "reason": "Unga Millers isn't a payee in the mandate",
                    "amount_minor": 42000, "currency": "USD", "model": "qwen3.8-27b", "tokens_in": 812, "tokens_out": 96, "cost_micro_usd": 410}
        });
        let r = audit_row("kilele", "kilele", 7, &record);
        assert_eq!(r.id, "audit:kilele:7");
        assert_eq!(r.agent.as_deref(), Some("payables"));
        assert_eq!(r.decision, Some(Decision::Denied));
        assert_eq!(r.tool.as_deref(), Some("pay_bill"));
        assert_eq!(
            r.reason.as_deref(),
            Some("Unga Millers isn't a payee in the mandate")
        );
        assert!(r.moves_money());
        let v = row_view(
            &r,
            "https://p/{id}",
            "2026-11-11T06:00:00Z".parse().unwrap(),
        );
        assert_eq!(v["when"], "08:02:03", "EAT");
        assert_eq!(v["usage"], "qwen3.8-27b · 812 in · 96 out · $0.0004");
        assert_eq!(v["amount"], "$420.00");

        // Held without a reason in ext: the policy names it.
        let held = json!({"ts": "2026-11-11T05:02:03Z", "kind": "approval", "action": "pay_bill", "decision": "approval_required", "policyRef": "step-up:new-payee"});
        let r = audit_row("kilele", "kilele", 8, &held);
        assert_eq!(
            (r.decision, r.reason.as_deref()),
            (Some(Decision::Held), Some("policy step-up:new-payee"))
        );
        assert_eq!(r.what, "approval: pay bill");
    }

    #[test]
    fn filters_keep_what_they_say() {
        let mut a = Row::new("audit", "a".into(), Utc::now(), "kilele", "x".into());
        a.agent = Some("payables".into());
        a.decision = Some(Decision::Denied);
        let b = Row::new("gateway", "b".into(), Utc::now(), "agentrix", "y".into());
        let f = |c: &str, ag: &str, m: &str, r: &str| Filter {
            company: Some(c.into()),
            agent: Some(ag.into()),
            money: Some(m.into()),
            refusals: Some(r.into()),
        };
        assert!(f("kilele", "payables", "", "1").keeps(&a));
        assert!(!f("kilele", "", "", "1").keeps(&b));
        assert!(!f("", "", "1", "").keeps(&a), "no money on it");
        assert!(f("", "", "", "").keeps(&b));
        assert_eq!(
            f("kilele", "", "1", "on").query(),
            "company=kilele&money=1&refusals=1"
        );
    }

    #[test]
    fn the_day_starts_at_midnight_in_nairobi() {
        let now: DateTime<Utc> = "2026-11-11T05:00:00Z".parse().unwrap();
        assert_eq!(
            today_start(now),
            "2026-11-10T21:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }
}
