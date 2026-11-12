//! The company's two money flows as adk-graph graphs, checkpointed after every step, so a held
//! approval or a crash resumes where it stopped.
//!
//! - **Procure-to-pay** (one thread per bill): `check` reads the bill and the payee the books
//!   hold → `pay` pays it on the governed path. A payment over the mandate is held: the graph
//!   interrupts with the held call, and resumes when the founder has decided. A crash mid-payment
//!   resumes at `pay`, where the action ledger and PayPal's idempotency key read the first
//!   payment back instead of paying twice.
//! - **Invoice-to-cash** (one thread per invoice): `issue` issues it on the governed path →
//!   `await` waits for the cash (PayPal) or the due date (the kernel) → `remind` on the due date
//!   → `close` remembers how the customer paid, which the next debt plan reads.
//!
//! Graph calls run on the same governed path as agents ([`Governance::call`]); there is no
//! model in them.

use std::collections::HashMap;
use std::sync::Arc;

use adk_core::ApprovalScope;
use adk_graph::SqliteCheckpointer;
use adk_graph::{
    Checkpointer, CompiledGraph, END, ExecutionConfig, GraphError, Interrupt, MemoryCheckpointer,
    NodeContext, NodeOutput, START, State, StateGraph,
};
use saas_core::books::{BillId, InvoiceId};
use serde_json::{Value, json};

use crate::company::Company;
use crate::governed::CallOutcome;
use crate::identity::{PAYABLES, RECEIVABLES};
use crate::memory::Behaviour;
use crate::policy::dollars;

/// Where a flow stands after a step.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowStatus {
    /// It finished; its final state.
    Done(State),
    /// It is waiting (for the founder, the cash, or a date) at `node`.
    Waiting {
        /// The thread.
        thread: String,
        /// The node it will resume at.
        node: String,
        /// Why, in plain words.
        reason: String,
        /// What it is waiting on (a held call's fingerprint, an invoice).
        data: Option<Value>,
    },
}

impl FlowStatus {
    /// Whether the flow finished.
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done(_))
    }

    /// The final state, when done.
    pub fn state(&self) -> Option<&State> {
        match self {
            Self::Done(s) => Some(s),
            Self::Waiting { .. } => None,
        }
    }
}

fn graph_err(e: impl std::fmt::Display) -> GraphError {
    GraphError::NodeExecutionFailed {
        node: "flow".into(),
        message: e.to_string(),
    }
}

fn s(ctx: &NodeContext, key: &str) -> Option<String> {
    ctx.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn scope(company: &Company, role: &str, thread: &str) -> Result<ApprovalScope, GraphError> {
    let member = company.member(role).map_err(graph_err)?;
    Ok(ApprovalScope::new(
        member.identity.app_name().as_str(),
        member.user(),
        thread,
    ))
}

/// A governed call from a node: Done → the result, Held → an interrupt with the held call.
async fn governed(
    company: &Company,
    role: &str,
    thread: &str,
    tool: &str,
    call: &str,
    args: Value,
) -> Result<Result<Value, NodeOutput>, GraphError> {
    let member = company.member(role).map_err(graph_err)?;
    let outcome = company
        .governance
        .call(
            &company.desk,
            &member.identity,
            &scope(company, role, thread)?,
            thread,
            tool,
            &format!("{thread}/{call}"),
            args,
            None,
        )
        .await
        .map_err(graph_err)?;
    Ok(match outcome {
        CallOutcome::Done(v) => Ok(v),
        CallOutcome::AlreadyDone => Ok(json!({ "already_done": true })),
        CallOutcome::Refused(reason) => Err(NodeOutput::new()
            .with_update("refused", json!(reason))
            .with_goto([END])),
        CallOutcome::Held { request, reason } => Err(NodeOutput::interrupt_with_data(
            &format!("held for the founder: {reason}"),
            json!({ "tool": request.tool_name, "args": request.args, "fingerprint": request.fingerprint() }),
        )),
    })
}

fn procure_to_pay(company: Arc<Company>) -> Result<StateGraph, GraphError> {
    let check_co = company.clone();
    let pay_co = company;
    Ok(StateGraph::with_channels(&[
        "bill_id",
        "amount_usd",
        "payee",
        "vendor",
        "result",
        "refused",
    ])
    .add_node_fn("check", move |ctx: NodeContext| {
        let company = check_co.clone();
        async move {
            if ctx.get("payee").is_some() {
                return Ok(NodeOutput::new());
            }
            let bill_id = s(&ctx, "bill_id").ok_or_else(|| graph_err("no bill_id"))?;
            let books = &company.desk.books;
            let bill = books
                .bill(&BillId::from(bill_id.as_str()))
                .await
                .map_err(graph_err)?;
            if !bill.status.is_open() || !bill.balance_due.is_positive() {
                return Ok(NodeOutput::new()
                    .with_update("refused", json!("the bill isn't open"))
                    .with_goto([END]));
            }
            let vendor = books.vendor(&bill.vendor).await.map_err(graph_err)?;
            let Some(payee) = vendor.counterparty.paypal_email.clone() else {
                return Ok(NodeOutput::new()
                    .with_update("refused", json!("the books hold no payee for the vendor"))
                    .with_goto([END]));
            };
            Ok(NodeOutput::new()
                .with_update("amount_usd", json!(dollars(bill.balance_due.minor)))
                .with_update("payee", json!(payee))
                .with_update("vendor", json!(vendor.counterparty.name)))
        }
    })
    .add_node_fn("pay", move |ctx: NodeContext| {
        let company = pay_co.clone();
        async move {
            let thread = ctx.config.thread_id.clone();
            let args = json!({
                "bill_id": ctx.get("bill_id"), "amount_usd": ctx.get("amount_usd"),
                "payee": ctx.get("payee"),
            });
            match governed(&company, PAYABLES, &thread, "pay_bill", "pay", args).await? {
                Ok(result) => Ok(NodeOutput::new().with_update("result", result)),
                Err(stop) => Ok(stop),
            }
        }
    })
    .add_edge(START, "check")
    .add_edge("check", "pay")
    .add_edge("pay", END))
}

fn invoice_to_cash(company: Arc<Company>) -> Result<StateGraph, GraphError> {
    let issue_co = company.clone();
    let await_co = company.clone();
    let remind_co = company.clone();
    let close_co = company;
    Ok(StateGraph::with_channels(&[
        "customer_id",
        "lines",
        "due_in_days",
        "invoice_id",
        "due_date",
        "reminded_for",
        "outcome",
        "refused",
        "next",
    ])
    .add_node_fn("issue", move |ctx: NodeContext| {
        let company = issue_co.clone();
        async move {
            if ctx.get("invoice_id").is_some() {
                return Ok(NodeOutput::new());
            }
            let thread = ctx.config.thread_id.clone();
            let args = json!({
                "customer_id": ctx.get("customer_id"), "lines": ctx.get("lines"),
                "due_in_days": ctx.get("due_in_days").cloned().unwrap_or(json!(14)),
                "reference": thread,
            });
            match governed(&company, RECEIVABLES, &thread, "issue_invoice", "issue", args).await? {
                Ok(issued) => Ok(NodeOutput::new()
                    .with_update("invoice_id", issued["invoice_id"].clone())
                    .with_update("due_date", issued["due_date"].clone())),
                Err(stop) => Ok(stop),
            }
        }
    })
    .add_node_fn("await", move |ctx: NodeContext| {
        let company = await_co.clone();
        async move {
            let invoice_id = s(&ctx, "invoice_id").ok_or_else(|| graph_err("no invoice"))?;
            let invoice = company
                .desk
                .books
                .invoice(&InvoiceId::from(invoice_id.as_str()))
                .await
                .map_err(graph_err)?;
            if !invoice.balance_due.is_positive() {
                return Ok(NodeOutput::new().with_update("next", json!("close")));
            }
            let woke = ctx.resume_value().and_then(Value::as_str).map(str::to_owned);
            if let Some(due) = woke.as_deref().and_then(|w| w.strip_prefix("due:"))
                && s(&ctx, "reminded_for").as_deref() != Some(due)
                && invoice.due_date < company.desk.today()
            {
                return Ok(NodeOutput::new()
                    .with_update("next", json!("remind"))
                    .with_update("reminded_for", json!(due)));
            }
            Ok(NodeOutput::interrupt_with_data(
                "waiting for the cash or the due date",
                json!({ "invoice_id": invoice_id, "due_date": invoice.due_date, "owed": invoice.balance_due.to_string() }),
            ))
        }
    })
    .add_node_fn("remind", move |ctx: NodeContext| {
        let company = remind_co.clone();
        async move {
            let thread = ctx.config.thread_id.clone();
            let due = s(&ctx, "reminded_for").unwrap_or_default();
            let args = json!({ "invoice_id": ctx.get("invoice_id") });
            match governed(
                &company,
                RECEIVABLES,
                &thread,
                "send_reminder",
                &format!("remind-{due}"),
                args,
            )
            .await?
            {
                Ok(_) | Err(_) => Ok(NodeOutput::new().with_update("next", json!("await"))),
            }
        }
    })
    .add_node_fn("close", move |ctx: NodeContext| {
        let company = close_co.clone();
        async move {
            let invoice_id = s(&ctx, "invoice_id").ok_or_else(|| graph_err("no invoice"))?;
            let invoice = company
                .desk
                .books
                .invoice(&InvoiceId::from(invoice_id.as_str()))
                .await
                .map_err(graph_err)?;
            let late = (company.desk.today() - invoice.due_date).num_days();
            company
                .desk
                .memory
                .remember(
                    invoice.customer.as_str(),
                    &Behaviour::Paid {
                        invoice: invoice_id.clone(),
                        days_late: late,
                    },
                    company.desk.clock.now(),
                )
                .await
                .map_err(graph_err)?;
            Ok(NodeOutput::new().with_update("outcome", json!({ "paid": invoice_id, "days_late": late })))
        }
    })
    .add_edge(START, "issue")
    .add_edge("issue", "await")
    .add_conditional_edges(
        "await",
        |state: &State| {
            state
                .get("next")
                .and_then(Value::as_str)
                .unwrap_or("close")
                .to_owned()
        },
        [("remind", "remind"), ("close", "close")],
    )
    .add_edge("remind", "await")
    .add_edge("close", END))
}

/// One company's flows over one checkpointer.
pub struct Flows {
    p2p: CompiledGraph,
    i2c: CompiledGraph,
}

impl std::fmt::Debug for Flows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flows").finish_non_exhaustive()
    }
}

impl Flows {
    /// The flows of `company`, checkpointed in `checkpointer`.
    pub fn new(company: Arc<Company>, checkpointer: Arc<dyn Checkpointer>) -> anyhow::Result<Self> {
        let p2p = procure_to_pay(company.clone())?
            .compile()?
            .with_checkpointer_arc(checkpointer.clone());
        let i2c = invoice_to_cash(company)?
            .compile()?
            .with_checkpointer_arc(checkpointer);
        Ok(Self { p2p, i2c })
    }

    /// Checkpointed in memory: tests that don't restart.
    pub fn in_memory(company: Arc<Company>) -> anyhow::Result<Self> {
        Self::new(company, Arc::new(MemoryCheckpointer::new()))
    }

    /// Checkpointed in the SQLite database at `url` (`sqlite:/var/lib/agentrix-saas/flows.db`),
    /// so a restarted process resumes every flow where it stopped.
    pub async fn sqlite(company: Arc<Company>, url: &str) -> anyhow::Result<Self> {
        let checkpointer = SqliteCheckpointer::new(url).await?;
        Self::new(company, Arc::new(checkpointer))
    }

    async fn run(
        graph: &CompiledGraph,
        input: State,
        config: ExecutionConfig,
    ) -> anyhow::Result<FlowStatus> {
        match graph.invoke(input, config).await {
            Ok(state) => Ok(FlowStatus::Done(state)),
            Err(GraphError::Interrupted(paused)) => {
                let (node, reason, data) = match &paused.interrupt {
                    Interrupt::Dynamic { message, data } => {
                        (String::new(), message.clone(), data.clone())
                    }
                    Interrupt::Before(n) | Interrupt::After(n) => (n.clone(), String::new(), None),
                };
                Ok(FlowStatus::Waiting {
                    thread: paused.thread_id.clone(),
                    node,
                    reason,
                    data,
                })
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The thread of `bill_id`'s procure-to-pay flow.
    pub fn bill_thread(bill_id: &str) -> String {
        format!("p2p-{bill_id}")
    }

    /// The thread of an invoice-to-cash flow started under `reference`.
    pub fn invoice_thread(reference: &str) -> String {
        format!("i2c-{reference}")
    }

    /// Starts (or continues) paying `bill_id`.
    pub async fn pay_bill(&self, bill_id: &str) -> anyhow::Result<FlowStatus> {
        let input = HashMap::from([("bill_id".to_owned(), json!(bill_id))]);
        Self::run(
            &self.p2p,
            input,
            ExecutionConfig::new(&Self::bill_thread(bill_id)),
        )
        .await
    }

    /// Resumes `bill_id`'s flow after the founder decided, or after a crash.
    pub async fn resume_bill(&self, bill_id: &str) -> anyhow::Result<FlowStatus> {
        Self::run(
            &self.p2p,
            State::new(),
            ExecutionConfig::new(&Self::bill_thread(bill_id))
                .with_resume_value("pay", json!("decided")),
        )
        .await
    }

    /// Starts invoicing `customer_id` for `lines` under `reference`.
    pub async fn invoice(
        &self,
        reference: &str,
        customer_id: &str,
        lines: Value,
        due_in_days: i64,
    ) -> anyhow::Result<FlowStatus> {
        let input = HashMap::from([
            ("customer_id".to_owned(), json!(customer_id)),
            ("lines".to_owned(), lines),
            ("due_in_days".to_owned(), json!(due_in_days)),
        ]);
        Self::run(
            &self.i2c,
            input,
            ExecutionConfig::new(&Self::invoice_thread(reference)),
        )
        .await
    }

    /// Wakes an invoice's flow: `"paid"` when PayPal's money arrived, `"due:<date>"` on a due
    /// date.
    pub async fn wake_invoice(&self, reference: &str, why: &str) -> anyhow::Result<FlowStatus> {
        Self::run(
            &self.i2c,
            State::new(),
            ExecutionConfig::new(&Self::invoice_thread(reference))
                .with_resume_value("await", json!(why)),
        )
        .await
    }

    /// Wakes the flow on `thread` (`i2c-…` or `p2p-…`) with `why`: what an event knows when it
    /// only has the thread (an invoice's reference in the books).
    pub async fn wake_thread(&self, thread: &str, why: &str) -> anyhow::Result<FlowStatus> {
        let (graph, node) = if thread.starts_with("p2p-") {
            (&self.p2p, "pay")
        } else {
            (&self.i2c, "await")
        };
        Self::run(
            graph,
            State::new(),
            ExecutionConfig::new(thread).with_resume_value(node, json!(why)),
        )
        .await
    }

    /// The saved state of a thread.
    pub async fn state(&self, thread: &str) -> anyhow::Result<Option<State>> {
        let graph = if thread.starts_with("p2p-") {
            &self.p2p
        } else {
            &self.i2c
        };
        Ok(graph.get_state(thread).await?)
    }
}
