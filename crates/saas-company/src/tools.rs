//! The tools each role's agent may call, as adk-rust `Tool`s over the [`Desk`].
//!
//! A tool never decides whether it may run: `LlmAgent` puts every call through the governed path
//! (`adk_core::authorize_tool_call`: freeze, the company policy, the tool guardrails, approvals)
//! and the action ledger before `execute` is reached. A money tool declares
//! `ToolEffect::NonIdempotent`, so the ledger records it before it runs and answers a replay from
//! the record; its idempotency key is also the PayPal request's, so a retry is never a second
//! payment. Read tools are `ReadOnly`.
//!
//! The model sees the tool's result, never a credential: the desk takes the agent's scoped token
//! inside the call.

use std::sync::Arc;

use adk_core::{AdkError, Tool, ToolContext, ToolEffect};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::desk::{CallCtx, Desk};
use crate::identity::{CHIEF, PAYABLES, RECEIVABLES, REVENUE, SUPPORT};
use crate::screen::redact;

/// One tool's declaration.
#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    /// Its name.
    pub name: &'static str,
    /// What the model is told it does.
    pub description: &'static str,
    /// The roles that carry it.
    pub roles: &'static [&'static str],
    /// Whether it moves money or changes what someone owes.
    pub money: bool,
    /// Its parameters (JSON Schema), as text.
    pub schema: &'static str,
}

const NONE: &str = r#"{"type":"object","properties":{}}"#;

/// Every tool.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "company_brief",
        description: "The company's position today: what customers owe, what it owes, debt plans.",
        roles: &[CHIEF],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "crm_pipeline",
        description: "Leads and opportunities in the CRM.",
        roles: &[REVENUE],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "crm_mark_won",
        description: "Marks an opportunity won and makes sure the customer is in the books.",
        roles: &[REVENUE],
        money: false,
        schema: r#"{"type":"object","properties":{"opportunity_id":{"type":"string"},"customer_name":{"type":"string"},"customer_email":{"type":"string"}},"required":["opportunity_id","customer_name"]}"#,
    },
    ToolSpec {
        name: "start_subscription",
        description: "Starts a PayPal subscription to a catalogue plan; the customer approves on PayPal's page.",
        roles: &[REVENUE],
        money: true,
        schema: r#"{"type":"object","properties":{"plan_id":{"type":"string","enum":["essential","business","commerce","agency"]},"customer_email":{"type":"string"}},"required":["plan_id","customer_email"]}"#,
    },
    ToolSpec {
        name: "create_recurring_invoice",
        description: "Sets up a recurring invoice in the books for a customer's plan.",
        roles: &[REVENUE],
        money: true,
        schema: r#"{"type":"object","properties":{"customer_id":{"type":"string"},"plan_id":{"type":"string"},"frequency":{"type":"string","enum":["monthly","yearly"]},"reference":{"type":"string"}},"required":["customer_id","plan_id"]}"#,
    },
    ToolSpec {
        name: "ar_ageing",
        description: "What customers owe, by how far past due.",
        roles: &[RECEIVABLES, CHIEF],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "counterparty_history",
        description: "What the company remembers about a customer's payments and promises, with the instalment plan that history suggests for an invoice.",
        roles: &[RECEIVABLES],
        money: false,
        schema: r#"{"type":"object","properties":{"customer_id":{"type":"string"},"invoice_id":{"type":"string"},"max_instalments":{"type":"integer"},"max_days":{"type":"integer"}},"required":["customer_id"]}"#,
    },
    ToolSpec {
        name: "issue_invoice",
        description: "Issues and posts an invoice in the books.",
        roles: &[RECEIVABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"customer_id":{"type":"string"},"lines":{"type":"array","items":{"type":"object","properties":{"description":{"type":"string"},"amount_usd":{"type":"number"},"quantity":{"type":"integer"}},"required":["description","amount_usd"]}},"due_in_days":{"type":"integer"},"reference":{"type":"string"}},"required":["customer_id","lines"]}"#,
    },
    ToolSpec {
        name: "invoice_usage",
        description: "Invoices each agent-run customer company for its platform usage since its last usage invoice (its agents' model calls at the providers' prices, and the platform fee), and offers each invoice to that company's agent door, where its Payables settles it.",
        roles: &[RECEIVABLES],
        money: true,
        schema: NONE,
    },
    ToolSpec {
        name: "send_reminder",
        description: "Emails the customer a reminder about an overdue invoice.",
        roles: &[RECEIVABLES],
        money: false,
        schema: r#"{"type":"object","properties":{"invoice_id":{"type":"string"},"note":{"type":"string"}},"required":["invoice_id"]}"#,
    },
    ToolSpec {
        name: "propose_debt_plan",
        description: "Agrees an instalment plan for an open invoice. The instalments must add up to what is owed.",
        roles: &[RECEIVABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"invoice_id":{"type":"string"},"instalments":{"type":"array","items":{"type":"object","properties":{"due_date":{"type":"string","format":"date"},"amount_usd":{"type":"number"}},"required":["due_date","amount_usd"]}}},"required":["invoice_id","instalments"]}"#,
    },
    ToolSpec {
        name: "collect_instalment",
        description: "Collects a debt plan's instalment that is due, and posts it to the books.",
        roles: &[RECEIVABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"plan_id":{"type":"string"},"instalment":{"type":"integer","minimum":0}},"required":["plan_id","instalment"]}"#,
    },
    ToolSpec {
        name: "suspend_subscription",
        description: "Suspends a subscription after PayPal reported a failed payment.",
        roles: &[RECEIVABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"subscription":{"type":"string"},"reason":{"type":"string"}},"required":["subscription"]}"#,
    },
    ToolSpec {
        name: "ap_bills_due",
        description: "Open bills, with each vendor's payee on file.",
        roles: &[PAYABLES, CHIEF],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "pay_bill",
        description: "Pays a bill through PayPal Payouts to the payee the books hold for the vendor.",
        roles: &[PAYABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"bill_id":{"type":"string"},"amount_usd":{"type":"number"},"payee":{"type":"string"}},"required":["bill_id","amount_usd","payee"]}"#,
    },
    ToolSpec {
        name: "change_vendor_payee",
        description: "Asks to change the payee the books hold for a vendor. Always goes to the founder.",
        roles: &[PAYABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"vendor_id":{"type":"string"},"payee":{"type":"string"},"evidence":{"type":"string"}},"required":["vendor_id","payee"]}"#,
    },
    ToolSpec {
        name: "settle_invoice",
        description: "Settles an invoice another agent-run company sent (a bill in the books) on PayPal.",
        roles: &[PAYABLES],
        money: true,
        schema: r#"{"type":"object","properties":{"bill_id":{"type":"string"}},"required":["bill_id"]}"#,
    },
    ToolSpec {
        name: "lookup_capture",
        description: "PayPal's record of a payment: who paid, and how much can still be refunded.",
        roles: &[SUPPORT],
        money: false,
        schema: r#"{"type":"object","properties":{"capture_id":{"type":"string"}},"required":["capture_id"]}"#,
    },
    ToolSpec {
        name: "refund_payment",
        description: "Refunds part or all of a payment, to the person who paid it.",
        roles: &[SUPPORT],
        money: true,
        schema: r#"{"type":"object","properties":{"capture_id":{"type":"string"},"customer_id":{"type":"string"},"invoice_id":{"type":"string"},"amount_usd":{"type":"number"},"reason":{"type":"string"},"refund_to":{"type":"string"}},"required":["capture_id","customer_id","amount_usd"]}"#,
    },
    ToolSpec {
        name: "cancel_subscription",
        description: "Cancels a customer's subscription.",
        roles: &[SUPPORT],
        money: true,
        schema: r#"{"type":"object","properties":{"subscription":{"type":"string"},"reason":{"type":"string"}},"required":["subscription"]}"#,
    },
    ToolSpec {
        name: "wallet_checkpoint",
        description: "Reconciles the business's wallet record against PayPal: reads every entry back, records what matches and what drifted, and signs a checkpoint.",
        roles: &[PAYABLES],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "wallet_status",
        description: "The business's wallet record as last reconciled against PayPal: whether it matched, what drifted, the totals and when it was checked. Reads no PayPal.",
        roles: &[CHIEF],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "bill_model_spend",
        description: "Posts the vendor bills for the model spend the platform's gateway metered since the last bill (one per provider), for Payables to pay.",
        roles: &[PAYABLES],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "post_payroll",
        description: "Posts the payroll bill for the pay period that just ended (the staff's wages, paid to the payroll account on file), once per period, for Payables to pay.",
        roles: &[PAYABLES],
        money: false,
        schema: NONE,
    },
    ToolSpec {
        name: "brief_owner",
        description: "Sends the owner a short brief (Telegram and email): what customers owe, what the company owes, what is waiting for them.",
        roles: &[CHIEF],
        money: false,
        schema: r#"{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}"#,
    },
];

/// The spec of tool `name`.
pub fn spec(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|t| t.name == name)
}

/// One tool over the desk.
pub struct DeskTool {
    spec: &'static ToolSpec,
    schema: Value,
    desk: Arc<Desk>,
}

impl std::fmt::Debug for DeskTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeskTool")
            .field("name", &self.spec.name)
            .finish()
    }
}

impl DeskTool {
    /// Tool `spec` over `desk`.
    pub fn new(spec: &'static ToolSpec, desk: Arc<Desk>) -> Self {
        let schema = serde_json::from_str(spec.schema).unwrap_or_else(|_| json!({}));
        Self { spec, schema, desk }
    }
}

/// The tools `role` carries, over `desk`.
pub fn tools_for(role: &str, desk: &Arc<Desk>) -> Vec<Arc<dyn Tool>> {
    TOOLS
        .iter()
        .filter(|t| t.roles.contains(&role))
        .map(|t| Arc::new(DeskTool::new(t, desk.clone())) as Arc<dyn Tool>)
        .collect()
}

#[async_trait]
impl Tool for DeskTool {
    fn name(&self) -> &str {
        self.spec.name
    }

    fn description(&self) -> &str {
        self.spec.description
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(self.schema.clone())
    }

    fn is_read_only(&self) -> bool {
        // Not money, but each writes: an email, the CRM, a bill in the books.
        let writes = matches!(
            self.spec.name,
            "send_reminder" | "crm_mark_won" | "bill_model_spend" | "post_payroll"
        );
        !self.spec.money && !writes
    }

    fn effect(&self) -> ToolEffect {
        if self.is_read_only() {
            ToolEffect::ReadOnly
        } else {
            ToolEffect::NonIdempotent
        }
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let call = CallCtx {
            acting: ctx.acting_context(),
            key: ctx.idempotency_key(),
        };
        self.desk
            .run(self.spec.name, &call, &args)
            .await
            .map_err(|e| AdkError::agent(redact(&format!("{}: {e}", self.spec.name))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::PolicyFile;

    #[test]
    fn every_schema_parses_and_every_money_tool_is_in_the_policy() {
        let policy = PolicyFile::agentrix();
        for t in TOOLS {
            let schema: Value = serde_json::from_str(t.schema).expect(t.name);
            assert_eq!(schema["type"], "object", "{}", t.name);
            if t.money {
                assert!(
                    policy.money_tools.iter().any(|m| m == t.name),
                    "{} moves money but the policy file doesn't list it",
                    t.name
                );
            }
            assert!(
                policy.rules.iter().any(|r| r.tool == t.name),
                "{} has no rule",
                t.name
            );
        }
    }
}
