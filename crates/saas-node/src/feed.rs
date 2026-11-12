//! What the books don't hold, for saas-web's [`saas_web::books_feed::BooksFeed`]: each
//! company's staff and whether it is frozen, the founders' mandates, the calls waiting for
//! them, the debt plans, the agent-to-agent settlements, and the signed audit logs' check.
//! The books answer the rest (receivables, ageing, payables, PayPal cash, MRR).
//!
//! Only what the node knows goes in; model spend per agent stays empty until it is metered here.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use saas_company::identity::{CHIEF, PAYABLES, RECEIVABLES, REVENUE, ROLES, SUPPORT};
use saas_company::runtime::Runtime;
use saas_web::books_feed::{Facts, OffBooks};
use saas_web::feed::{
    self, Agent, Audit, Company, DebtPlan, Decision, Instalment, Mandate, Settlement,
};
use saas_web::hosts::Hosts;

/// Agent-to-agent settlements as saas-settle reported them, newest first.
#[derive(Debug, Default)]
pub struct SettlementLog {
    rows: Mutex<Vec<Settlement>>,
    refusals: Mutex<std::collections::BTreeSet<String>>,
}

impl SettlementLog {
    /// Records a door's refusal as a `refused` row, once per refusal id; whether it was new.
    pub fn record_refusal(&self, refusal_id: &str, row: Settlement) -> bool {
        if !self
            .refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(refusal_id.to_owned())
        {
            return false;
        }
        self.rows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(0, row);
        true
    }

    /// Records one (the same PayPal id once).
    pub fn record(&self, row: Settlement) {
        let mut rows = self.rows.lock().unwrap_or_else(|p| p.into_inner());
        if rows
            .iter()
            .any(|r| r.status != "refused" && r.paypal_id == row.paypal_id)
        {
            return;
        }
        rows.insert(0, row);
    }

    /// Every settlement, newest first.
    pub fn rows(&self) -> Vec<Settlement> {
        self.rows.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// The node's off-books facts.
pub struct NodeOffBooks {
    hosts: Hosts,
    companies: Vec<(Arc<Runtime>, String)>,
    settlements: Arc<SettlementLog>,
}

impl NodeOffBooks {
    /// Over `companies` (each with its owner's name as cards say it).
    pub fn new(
        hosts: Hosts,
        companies: Vec<(Arc<Runtime>, String)>,
        settlements: Arc<SettlementLog>,
    ) -> Self {
        Self {
            hosts,
            companies,
            settlements,
        }
    }
}

fn agent_words(role: &str) -> (&'static str, &'static str) {
    match role {
        CHIEF => (
            "Chief",
            "Runs the company inside the founder's mandate and answers them.",
        ),
        REVENUE => (
            "Revenue",
            "Sells plans: the CRM, PayPal Subscriptions and recurring invoices.",
        ),
        RECEIVABLES => (
            "Receivables",
            "Invoices customers, collects by ageing, agrees instalment plans.",
        ),
        PAYABLES => (
            "Payables",
            "Pays bills through PayPal inside the founder's mandate; payee changes go to the founder.",
        ),
        SUPPORT => (
            "Support",
            "Cancellations and refunds, only ever to the person who paid.",
        ),
        _ => ("Agent", ""),
    }
}

async fn one(
    rt: &Runtime,
    owner: &str,
    hosts: &Hosts,
    as_of: NaiveDate,
    f: &mut Facts,
) -> anyhow::Result<bool> {
    let company = &rt.company;
    let slug = company.config.slug.clone();
    let frozen = company.governance.control.is_frozen();
    let held = company.governance.held().await?;

    let mut agents = Vec::new();
    for role in ROLES {
        let (name, job) = agent_words(role);
        let waiting = held
            .iter()
            .filter(|h| {
                h.agent
                    .as_deref()
                    .is_some_and(|a| a.ends_with(&format!("/{role}")))
            })
            .count();
        let revoked = company.access().is_some_and(|a| a.is_revoked(role));
        let (state, doing) = if revoked {
            (
                "revoked",
                "Switched off by the founder; its tokens no longer work".to_owned(),
            )
        } else if frozen {
            ("frozen", "Frozen by the founder".to_owned())
        } else if waiting > 0 {
            ("waiting", format!("{waiting} waiting for the founder"))
        } else {
            ("idle", "Waiting for its next event".to_owned())
        };
        agents.push(Agent {
            id: role.into(),
            name: name.into(),
            job: job.into(),
            state: state.into(),
            doing,
            key_id: None,
        });
    }
    f.companies.push(Company {
        id: slug.clone(),
        name: company.config.name.clone(),
        host: if slug == feed::KILELE {
            hosts.kilele.clone()
        } else {
            hosts.money.clone()
        },
        owner: owner.into(),
        agents,
        frozen,
    });

    if let Some(m) = company.signed_mandate() {
        let used = company
            .desk
            .rails
            .paid_out_today(company.desk.clock.now())
            .await
            .unwrap_or(0);
        f.mandates.push(Mandate {
            id: m.reference.clone(),
            company: slug.clone(),
            owner: owner.into(),
            agent: PAYABLES.into(),
            scope: "Pay bills, refunds, instalment plans".into(),
            currency: m.terms.currency.clone(),
            per_payment_minor: m.terms.payout_max_each_minor,
            daily_minor: m.terms.payout_max_per_day_minor,
            used_today_minor: used,
            payees: m.terms.payees.clone(),
            signed_with: m.signed_with.clone(),
            signed: m.signed_at.date_naive(),
            expires: m.terms.valid_until.date_naive(),
            status: if m.terms.in_force(company.desk.clock.now()) {
                "in force"
            } else {
                "lapsed"
            }
            .into(),
        });
    }

    for h in &held {
        let view = h.view();
        let fingerprint = view["fingerprint"].as_str().unwrap_or_default().to_owned();
        f.decisions.push(Decision {
            id: fingerprint.clone(),
            company: slug.clone(),
            agent: h
                .agent
                .as_deref()
                .and_then(|a| a.rsplit('/').next())
                .unwrap_or_default()
                .to_owned(),
            title: format!("{} is waiting for you", h.tool.replace('_', " ")),
            body: h.args.to_string(),
            effects: vec![format!("If you approve, {} runs exactly as shown", h.tool)],
            why: "It is outside the mandate you signed".into(),
            currency: "USD".into(),
            amount_minor: h.args["amount_usd"]
                .as_f64()
                .map(|a| (a * 100.0).round() as i64),
            // saas-web's console decides only `open` decisions.
            state: "open".into(),
            fingerprint,
            requested_by: None,
        });
    }
    // Custom orders an agent asked for at the company's sample site's door.
    match crate::door_cards::cards(&slug).await {
        Ok(cards) => f.decisions.extend(cards),
        Err(e) => tracing::debug!(company = %slug, error = %e, "no door cards"),
    }

    for p in company.desk.plans() {
        let paid: i64 = p
            .instalments
            .iter()
            .zip(&p.collected)
            .filter(|(_, c)| **c)
            .map(|(i, _)| i.amount_minor)
            .sum();
        let principal: i64 = p.instalments.iter().map(|i| i.amount_minor).sum();
        let debtor = company
            .desk
            .books
            .customer(&saas_core::books::CustomerId::from(p.customer.as_str()))
            .await
            .map(|c| c.counterparty.name)
            .unwrap_or_else(|_| p.customer.clone());
        f.debt_plans.push(DebtPlan {
            id: p.plan_id.clone(),
            creditor: slug.clone(),
            debtor,
            invoice_number: p.invoice.clone(),
            currency: p.currency.clone(),
            principal_minor: principal,
            paid_minor: paid,
            status: if paid >= principal { "paid" } else { "running" }.into(),
            agreed: p.instalments.first().map(|i| i.due_date).unwrap_or(as_of),
            mandate_ids: Vec::new(),
            instalments: p
                .instalments
                .iter()
                .zip(&p.collected)
                .enumerate()
                .map(|(n, (i, c))| Instalment {
                    n: n as u32 + 1,
                    due: i.due_date,
                    amount_minor: i.amount_minor,
                    status: if *c {
                        "paid"
                    } else if i.due_date < as_of {
                        "late"
                    } else {
                        "scheduled"
                    }
                    .into(),
                    paypal_id: None,
                })
                .collect(),
        });
    }

    let report = company.governance.audit.verify().await?;
    f.audit.entries += report.records;
    Ok(report.is_valid())
}

#[async_trait]
impl OffBooks for NodeOffBooks {
    async fn facts(&self, as_of: NaiveDate) -> anyhow::Result<Facts> {
        let mut f = Facts {
            audit: Audit {
                verified: true,
                entries: 0,
                checkpoint: String::new(),
                checked: Utc::now().to_rfc3339(),
                tool: "adk-rust audit verify".into(),
            },
            ..Facts::default()
        };
        let mut verified = true;
        let mut logs = Vec::new();
        for (rt, owner) in &self.companies {
            verified &= one(rt, owner, &self.hosts, as_of, &mut f).await?;
            logs.push(rt.company.governance.audit.log_id().to_owned());
        }
        f.audit.verified = verified;
        f.audit.checkpoint = logs.join(", ");
        f.settlements = self.settlements.rows();
        Ok(f)
    }
}
