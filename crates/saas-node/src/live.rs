//! The live run (v3, "transform everything to live"): the companies' standing schedules, what
//! their desks reach outside the books (the wallet, the owner), and the owner's alerts.
//!
//! - [`NodeOutside`]: a company's wallet checkpoint (saas-wallet) and the owner's channel, for
//!   the desk's `wallet_checkpoint` and `brief_owner` tools.
//! - [`OwnerAlerts`]: the platform's own channels to the owner: Telegram through
//!   agentrix-channels' Bot API client (`SAAS_ALERT_TELEGRAM_TOKEN`,
//!   `SAAS_ALERT_TELEGRAM_CHAT`), email through agentrix-messages' Resend transport
//!   (`RESEND_API_KEY` and the rest of its settings, to `SAAS_ALERT_EMAIL`). The same text twice
//!   within ten minutes is sent once.
//! - [`follow_alerts`]: reads each company's signed audit log as it grows and alerts the owner on
//!   each call held for their passkey, each refused money call and each failed tool call.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adk_core::audit_log::{AuditDecision, AuditRecord, AuditRecordOutcome};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use saas_company::desk::Outside;
use saas_core::CompanyId;
use serde_json::{Value, json};

use crate::config::EnvFile;

/// The owner's channels.
pub struct OwnerAlerts {
    telegram: Option<(agentrix_channels::telegram::Client, i64)>,
    email: Option<(agentrix_messages::email::ResendTransport, String)>,
    recent: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl std::fmt::Debug for OwnerAlerts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerAlerts")
            .field("telegram", &self.telegram.is_some())
            .field("email", &self.email.is_some())
            .finish()
    }
}

impl OwnerAlerts {
    /// From the node's settings; a channel without its settings is off.
    pub fn from_env(env: &EnvFile) -> Self {
        let http = reqwest::Client::new();
        let telegram = match (
            env.get("SAAS_ALERT_TELEGRAM_TOKEN"),
            env.get("SAAS_ALERT_TELEGRAM_CHAT")
                .and_then(|c| c.parse::<i64>().ok()),
        ) {
            (Some(token), Some(chat)) => Some((
                agentrix_channels::telegram::Client::new(
                    http.clone(),
                    agentrix_channels::config::Token::new(token),
                ),
                chat,
            )),
            _ => None,
        };
        let email = match (
            agentrix_messages::email::EmailConfig::from_vars(|k| env.get(k)),
            env.get("SAAS_ALERT_EMAIL"),
        ) {
            (Some(cfg), Some(to)) => Some((
                agentrix_messages::email::ResendTransport::new(http, cfg),
                to,
            )),
            _ => None,
        };
        tracing::info!(
            telegram = telegram.is_some(),
            email = email.is_some(),
            "the owner's alert channels"
        );
        Self {
            telegram,
            email,
            recent: Mutex::new(HashMap::new()),
        }
    }

    /// Sends `text` about `company` to the owner (`kind`: `brief`, `alert`). Returns where it
    /// went.
    pub async fn send(&self, company: &str, kind: &str, text: &str) -> Value {
        let now = Utc::now();
        {
            let mut recent = self.recent.lock().unwrap_or_else(|p| p.into_inner());
            recent.retain(|_, at| now - *at < chrono::Duration::minutes(10));
            if recent.insert(format!("{company}|{text}"), now).is_some() {
                return json!({ "sent": [], "skipped": "the same text was sent in the last ten minutes" });
            }
        }
        let title = match kind {
            "brief" => format!("{company}: the Chief's brief"),
            _ => format!("{company}: needs your attention"),
        };
        let mut sent = Vec::new();
        let mut not_sent = Vec::new();
        if let Some((client, chat)) = &self.telegram {
            let html = format!("<b>{}</b>\n{}", escape_html(&title), escape_html(text));
            match client.send_message(*chat, &html, &Vec::new()).await {
                Ok(_) => sent.push("telegram"),
                Err(e) => not_sent.push(format!("telegram: {e:?}")),
            }
        } else {
            not_sent.push("telegram: not configured".to_owned());
        }
        if let Some((transport, to)) = &self.email {
            use agentrix_messages::Transport;
            let envelope = agentrix_messages::Envelope {
                outbox_id: format!("saas-{}", uuid::Uuid::new_v4()),
                attempt: 1,
                message: format!("saas.{kind}"),
                site_id: None,
                channel: agentrix_messages::Channel::Email,
                to: to.clone(),
                to_name: None,
                from_name: "Agentrix".into(),
                subject: title.clone(),
                text: text.to_owned(),
                html: None,
                headers: Vec::new(),
                push: None,
                chat: None,
            };
            match transport.deliver(&envelope).await {
                agentrix_messages::Delivery::Sent { .. } => sent.push("email"),
                other => not_sent.push(format!("email: {other:?}")),
            }
        } else {
            not_sent.push("email: not configured".to_owned());
        }
        tracing::info!(company, kind, ?sent, ?not_sent, "told the owner");
        json!({ "sent": sent, "not_sent": not_sent })
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What a company's desk reaches outside the books.
pub struct NodeOutside {
    /// The company.
    pub company: CompanyId,
    /// Its wallet record, when it has one.
    pub wallets: Option<Arc<saas_wallet::Wallets>>,
    /// The owner's channels.
    pub owner: Arc<OwnerAlerts>,
    /// Node's `control.db`, where the model gateway meters (model spend).
    pub control: Option<sqlx::SqlitePool>,
    /// The company's books.
    pub books: Arc<dyn saas_core::Books>,
    /// The node's settings (the usage fee, payroll).
    pub env: crate::config::EnvFile,
    /// The company's signed audit log, where what Payables posts in the books is recorded with
    /// the books' numbers.
    pub audit: Option<Arc<saas_company::audit::SignedAudit>>,
    /// The company's slug (`kilele`), as its agents' ids name it.
    pub slug: String,
}

impl NodeOutside {
    /// Records a bill Payables posted (`tool` posted it) in the company's signed audit log, with
    /// the books' numbers (`era_document`, and `era_journal` when the books show the journal
    /// entry it made), so the operators' activity page shows it.
    async fn record_posted(&self, tool: &str, bill: &saas_core::books::Bill) {
        let Some(audit) = &self.audit else {
            return;
        };
        let document = bill.number.clone().unwrap_or_else(|| bill.id.to_string());
        let mut journal = None;
        for reference in [Some(document.clone()), bill.vendor_reference.clone()]
            .into_iter()
            .flatten()
        {
            let filter = saas_core::books::JournalFilter {
                reference: Some(reference),
                ..Default::default()
            };
            if let Ok(found) = self.books.journal(&filter).await
                && let Some(entry) = found.into_iter().next()
            {
                journal = Some(entry.number.unwrap_or_else(|| entry.id.to_string()));
                break;
            }
        }
        let actor = adk_core::audit_log::AuditActor {
            agent: saas_company::identity::agent_id_for(
                &self.slug,
                saas_company::identity::PAYABLES,
            )
            .ok(),
            ..Default::default()
        };
        let mut draft = adk_core::audit_log::AuditRecordDraft::new(
            adk_core::audit_log::AuditRecordKind::Other("books".into()),
            "posted a bill",
        )
        .actor(actor)
        .outcome(AuditRecordOutcome::Succeeded)
        .ext("tool", json!(tool))
        .ext("era_document", json!(document))
        .ext("amount_minor", json!(bill.total.minor))
        .ext("currency", json!(bill.total.currency.code()));
        if let Some(journal) = journal {
            draft = draft.ext("era_journal", json!(journal));
        }
        if let Err(e) = audit.record(draft).await {
            tracing::warn!(company = %self.company, error = %e, "a posted bill didn't reach the signed audit log");
        }
    }

    /// Face's site id for the company's wallet: `kilele`, `agentrix`.
    fn site(&self) -> &'static str {
        if self.company == CompanyId::kilele() {
            "kilele"
        } else {
            "agentrix"
        }
    }
}

#[async_trait]
impl Outside for NodeOutside {
    async fn wallet_checkpoint(&self) -> anyhow::Result<Value> {
        if self.wallets.is_none() {
            anyhow::bail!("{} keeps no wallet record on this node", self.company);
        }
        // The model is told the failure without its detail; the journal keeps it.
        let reconciled = crate::hourly::reconcile(self.site())
            .await
            .inspect_err(|e| tracing::warn!(company = %self.company, error = %format!("{e:#}"), "the wallet checkpoint failed"))?;
        Ok(json!({
            "matches": reconciled.matches(),
            "checkpoint": serde_json::to_value(reconciled.checkpoint.summary())?,
            "drift": serde_json::to_value(reconciled.drift())?,
            "drifted_entries": reconciled.drifted.len(),
        }))
    }

    async fn day_report(&self) -> anyhow::Result<Value> {
        // The owner's day is Nairobi's.
        let nairobi = chrono::FixedOffset::east_opt(3 * 3600).expect("three hours east is a valid offset");
        let now = Utc::now().with_timezone(&nairobi);
        let today = now.date_naive();
        let since = today
            .and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(nairobi).single())
            .map_or(0, |t| t.timestamp());
        let usd = |cents: i64| format!("USD {}.{:02}", cents / 100, cents % 100);
        let plural = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let name = if self.company == CompanyId::kilele() {
            "Kilele Bakery"
        } else {
            "Agentrix"
        };

        // Costs: the bills dated today, by who they are owed to.
        let vendors: HashMap<String, String> = self
            .books
            .vendors()
            .await
            .map(|v| v.into_iter().map(|v| (v.id.to_string(), v.counterparty.name)).collect())
            .unwrap_or_default();
        let mut costs: Vec<(String, i64)> = Vec::new();
        for bill in self.books.bills(&saas_core::books::DocumentFilter::default()).await? {
            if bill.issue_date != today {
                continue;
            }
            let who = vendors
                .get(&bill.vendor.to_string())
                .cloned()
                .unwrap_or_else(|| "other".to_owned());
            match costs.iter_mut().find(|(n, _)| *n == who) {
                Some((_, cents)) => *cents += bill.total.minor,
                None => costs.push((who, bill.total.minor)),
            }
        }
        costs.sort_by_key(|(_, cents)| std::cmp::Reverse(*cents));
        let cost_total: i64 = costs.iter().map(|(_, c)| c).sum();
        let cost_words = if costs.is_empty() {
            "none today.".to_owned()
        } else {
            format!(
                "{} ({}).",
                usd(cost_total),
                costs.iter().map(|(n, c)| format!("{n} {}", usd(*c))).collect::<Vec<_>>().join(", ")
            )
        };

        // A company with a site: what people and agents did there today.
        let site = crate::samples::joined()
            .into_iter()
            .find(|(_, c)| *c == self.company)
            .map(|(s, _)| s);
        let activity = match site {
            Some(site) => agentrix_samples::held::day_activity(site, since)
                .await
                .inspect_err(|e| tracing::debug!(error = %format!("{e:#}"), "no site activity for the day report"))
                .ok()
                .flatten()
                .map(|(a, currency)| (site, a, currency)),
            None => None,
        };
        let mut lines = vec![format!("{name}, {}", today.format("%A %-d %B %Y"))];
        let mut figures = json!({ "costs_usd_cents": cost_total });
        if let Some((site, a, currency)) = activity {
            let visits: i64 = match &self.control {
                Some(control) => sqlx::query_scalar("SELECT COALESCE(SUM(visits), 0) FROM visit_counts WHERE site_id = ? AND day = ?")
                    .bind(site)
                    .bind(today.to_string())
                    .fetch_one(control)
                    .await
                    .unwrap_or(0),
                None => 0,
            };
            let local = |minor: i64| agentrix_records::money::fmt(&currency, minor);
            lines.push("People".into());
            lines.push(format!(
                "- Engagements: {} to the site{}.",
                plural(visits, "visit", "visits"),
                if a.questions_passed > 0 { format!("; {} passed to you", plural(a.questions_passed, "question", "questions")) } else { String::new() }
            ));
            lines.push(if a.people_orders > 0 {
                format!("- Orders: {} placed, {}.", a.people_orders, local(a.people_orders_minor))
            } else {
                "- Orders: none today.".into()
            });
            lines.push(if a.people_refunds > 0 {
                format!("- Refunds: {}, {}.", a.people_refunds, local(a.people_refunds_minor))
            } else {
                "- Refunds: none.".into()
            });
            lines.push("Agents".into());
            lines.push(if a.agent_requests > 0 || a.agents > 0 {
                format!(
                    "- Engagements: {} served; {} at the agent door, {} with a verified signature.",
                    plural(a.agents, "agent", "agents"),
                    plural(a.agent_requests, "request", "requests"),
                    a.agent_requests_verified
                )
            } else {
                "- Engagements: none today.".into()
            });
            lines.push(if a.agent_orders > 0 {
                format!("- Orders: {} placed and paid, {}.", a.agent_orders, local(a.agent_orders_minor))
            } else {
                "- Orders: none today.".into()
            });
            lines.push(if a.agent_refunds > 0 {
                format!("- Complaints and refunds: {}, {}.", plural(a.agent_refunds, "refund", "refunds"), local(a.agent_refunds_minor))
            } else {
                "- Complaints and refunds: none.".into()
            });
            lines.push("Today".into());
            lines.push(format!(
                "- Sales: {} from {} through PayPal.",
                usd(a.payments_usd_cents),
                plural(a.payments, "payment", "payments")
            ));
            lines.push(format!("- Costs: {cost_words}"));
            figures["site"] = serde_json::to_value(&a)?;
            figures["visits"] = json!(visits);
            figures["sales_usd_cents"] = json!(a.payments_usd_cents);
        } else {
            // A company without a shop site: its customers are other companies' agents.
            let invoices: Vec<_> = self
                .books
                .invoices(&saas_core::books::DocumentFilter::default())
                .await?
                .into_iter()
                .filter(|i| i.issue_date == today)
                .collect();
            let settled: Vec<_> = invoices.iter().filter(|i| i.balance_due.minor == 0).collect();
            let sales: i64 = settled.iter().map(|i| i.total.minor).sum();
            lines.push("People".into());
            lines.push("- Engagements, orders and refunds: none today; this company sells to other companies' agents.".into());
            lines.push("Agents".into());
            lines.push(format!(
                "- Engagements: {} offered at customers' agent doors.",
                plural(invoices.len() as i64, "invoice", "invoices")
            ));
            lines.push(if settled.is_empty() {
                "- Orders: none settled today.".into()
            } else {
                format!("- Orders: {} settled agent to agent, {}.", plural(settled.len() as i64, "invoice", "invoices"), usd(sales))
            });
            lines.push("- Complaints and refunds: none.".into());
            lines.push("Today".into());
            lines.push(format!("- Sales: {} through PayPal.", usd(sales)));
            lines.push(format!("- Costs: {cost_words}"));
            figures["invoices"] = json!(invoices.len());
            figures["sales_usd_cents"] = json!(sales);
        }
        Ok(json!({ "date": today.to_string(), "lines": lines, "figures": figures }))
    }

    async fn bill_model_spend(&self) -> anyhow::Result<Value> {
        let control = self
            .control
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this node has no model gateway meter"))?;
        let now = Utc::now();
        let spend = crate::hourly::bill_model_spend(
            control,
            self.books.as_ref(),
            &self.company,
            now - chrono::Duration::hours(24),
            now,
            &crate::hourly::payees_from_env(),
        )
        .await?;
        for posted in spend.bills.iter().filter(|b| !b.already) {
            self.record_posted("bill_model_spend", &posted.bill).await;
        }
        Ok(serde_json::to_value(spend)?)
    }

    async fn wallet_status(&self) -> anyhow::Result<Value> {
        let wallets = self.wallets.as_ref().ok_or_else(|| {
            anyhow::anyhow!("{} keeps no wallet record on this node", self.company)
        })?;
        let cashier = crate::samples::cashier(&self.company)?;
        Ok(match wallets.latest(&cashier).await? {
            Some(latest) => serde_json::to_value(latest)?,
            None => json!({ "reconciled": false, "why": "the wallet hasn't been reconciled yet" }),
        })
    }

    async fn usage_due(&self) -> anyhow::Result<Vec<saas_company::desk::UsageCharge>> {
        // Only the platform company has agent-run customers whose usage it meters.
        if self.company != CompanyId::agentrix() {
            return Ok(Vec::new());
        }
        let control = self
            .control
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("this node has no model gateway meter"))?;
        crate::billing::usage_due(
            control,
            self.books.as_ref(),
            crate::billing::usage_fee(&self.env),
            Utc::now(),
        )
        .await
    }

    async fn post_payroll(&self) -> anyhow::Result<Value> {
        let payroll =
            crate::billing::post_payroll(self.books.as_ref(), &self.env, &self.company, Utc::now())
                .await
                .inspect_err(|e| tracing::warn!(company = %self.company, error = %format!("{e:#}"), "payroll wasn't posted"))?;
        Ok(match payroll {
            Some(p) => {
                tracing::info!(company = %self.company, bill = %p.bill_id, total = %p.total, already = p.already, "payroll posted");
                if !p.already
                    && let Ok(bill) = self
                        .books
                        .bill(&saas_core::books::BillId::from(p.bill_id.as_str()))
                        .await
                {
                    self.record_posted("post_payroll", &bill).await;
                }
                serde_json::to_value(p)?
            }
            None => json!({ "posted": false, "why": "no wages are set for this company" }),
        })
    }

    async fn tell_owner(&self, kind: &str, text: &str) -> anyhow::Result<Value> {
        Ok(self.owner.send(self.company.as_str(), kind, text).await)
    }
}

/// Whether `record` is worth the owner's attention, and the words for it.
pub fn alert_for(record: &AuditRecord, money_tools: &[&str]) -> Option<String> {
    let agent = record
        .actor
        .agent
        .as_ref()
        .map(|a| a.to_string())
        .unwrap_or_else(|| "an agent".into());
    match (record.decision, record.outcome) {
        (Some(AuditDecision::ApprovalRequired), _) => Some(format!(
            "{agent} asked for {} and it waits for your passkey in Agentrix OS.",
            record.action
        )),
        (Some(AuditDecision::Denied), _) if money_tools.contains(&record.action.as_str()) => {
            Some(format!(
                "{agent} asked for {} and it was refused (policy {}).",
                record.action,
                record.policy_ref.as_deref().unwrap_or("rules")
            ))
        }
        (_, Some(AuditRecordOutcome::Failed)) => {
            Some(format!("{} by {agent} failed.", record.action))
        }
        _ => None,
    }
}

/// Reads `rt`'s signed audit log every `every`, from where it stands now, and alerts the owner
/// on each record [`alert_for`] picks.
pub async fn follow_alerts(
    rt: Arc<saas_company::runtime::Runtime>,
    owner: Arc<OwnerAlerts>,
    every: Duration,
) {
    let money: Vec<&str> = saas_company::tools::TOOLS
        .iter()
        .filter(|t| t.money)
        .map(|t| t.name)
        .collect();
    let audit = rt.company.governance.audit.clone();
    let mut seen = audit.records().await.map(|r| r.len() as u64).unwrap_or(0);
    let company = rt.company.config.slug.clone();
    loop {
        tokio::time::sleep(every).await;
        let Ok(records) = audit.records().await else {
            continue;
        };
        for record in records.iter().filter(|r| r.seq >= seen) {
            if let Some(text) = alert_for(record, &money) {
                owner.send(&company, "alert", &text).await;
            }
        }
        seen = records.len() as u64;
    }
}
