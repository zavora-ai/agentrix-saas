//! The governed path, assembled once per company, for agents and graphs alike.
//!
//! [`Governance`] holds what every money call goes through:
//!
//! 1. the `GovernanceControl` freeze (the founder's kill switch);
//! 2. the company's deny-by-default policy ([`crate::policy::CompanyPolicy`], from its policy
//!    file, limits from the signed mandate, facts from the books and PayPal);
//! 3. the tool guardrails ([`crate::screen::tool_guardrails`]);
//! 4. approvals bound to the call's fingerprint (agent, tool, canonical arguments) in an
//!    `ApprovalStore`: a held call runs only on the decision for exactly that call, and any edit
//!    is a different fingerprint, so it is held again;
//! 5. the `ActionLedger`, so an effect runs once, across retries and crashes;
//! 6. the signed audit log, which records every decision (allowed, denied, held, approved,
//!    executed).
//!
//! Agents get all of it through [`Governance::run_config`]: `LlmAgent` runs
//! `adk_core::authorize_tool_call` and the ledger around every tool. Graph nodes, which call
//! tools without a model, get the same path through [`Governance::call`].

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use adk_core::audit_log::{
    AuditActor, AuditDecision, AuditRecordDraft, AuditRecordKind, AuditRecordOutcome,
};
use adk_core::{
    ActionLedger, ActionOutcome, ActionRecord, ApprovalScope, ApprovalStore, GovernanceControl,
    GovernedCall, InMemoryActionLedger, InMemoryApprovalStore, PendingApproval, RunConfig,
    SubjectRef, Tool, ToolApproval, ToolAuthorization, ToolCallScreen, ToolConfirmationRequest,
    ToolGate, json_digest,
};
use adk_guardrail::{ToolCallDecision, ToolGuardrailSet};
use agentrix_decisions::MandateTerms;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::audit::SignedAudit;
use crate::desk::{CallCtx, Desk};
use crate::identity::StaffIdentity;
use crate::policy::CompanyPolicy;
use crate::screen::tool_guardrails;
use crate::tools::DeskTool;

/// One company's governance.
pub struct Governance {
    /// The company.
    pub company: String,
    /// The founder, as approvals and the freeze name them.
    pub founder: String,
    /// The policy.
    pub policy: Arc<CompanyPolicy>,
    /// Decisions on held calls.
    pub approvals: Arc<dyn ApprovalStore>,
    /// Exactly-once effects.
    pub ledger: Arc<dyn ActionLedger>,
    /// The kill switch.
    pub control: GovernanceControl,
    /// The signed audit log.
    pub audit: Arc<SignedAudit>,
    scopes: Mutex<BTreeSet<(String, String, String)>>,
    approved: Mutex<Vec<Held>>,
}

/// The business action a money call is about: the bill, the capture, the invoice. Two calls on
/// the same action with different arguments are an edit of one another.
pub fn action_key(tool: &str, args: &Value) -> Option<String> {
    let field = match tool {
        "pay_bill" | "settle_invoice" => "bill_id",
        "refund_payment" => "capture_id",
        "propose_debt_plan" | "issue_invoice" | "send_reminder" => "invoice_id",
        "change_vendor_payee" => "vendor_id",
        "collect_instalment" => "plan_id",
        _ => return None,
    };
    args.get(field)
        .and_then(Value::as_str)
        .map(|v| format!("{tool}:{v}"))
}

impl std::fmt::Debug for Governance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Governance")
            .field("company", &self.company)
            .field("frozen", &self.control.is_frozen())
            .finish_non_exhaustive()
    }
}

/// A held call, as the founder sees it.
#[derive(Debug, Clone)]
pub struct Held {
    /// Where it was held: (app, user, session).
    pub scope: (String, String, String),
    /// What the approval binds to.
    pub fingerprint: String,
    /// The tool.
    pub tool: String,
    /// Its arguments, exactly as they would run.
    pub args: Value,
    /// The agent.
    pub agent: Option<String>,
    /// The request exactly as it was held.
    pub request: ToolConfirmationRequest,
}

impl Held {
    /// What the founder's console shows.
    pub fn view(&self) -> Value {
        json!({
            "session": self.scope.2, "fingerprint": json_digest(&json!(self.fingerprint)),
            "tool": self.tool, "args": self.args, "agent": self.agent,
        })
    }

    /// The approval scope.
    pub fn approval_scope(&self) -> ApprovalScope {
        ApprovalScope::new(&self.scope.0, &self.scope.1, &self.scope.2)
    }
}

/// What a governed call from a graph came to.
#[derive(Debug, Clone, PartialEq)]
pub enum CallOutcome {
    /// It ran; the tool's result.
    Done(Value),
    /// It ran before (the ledger has it); nothing ran again.
    AlreadyDone,
    /// It was refused, and why.
    Refused(String),
    /// It waits for the founder.
    Held {
        /// The request; its fingerprint is what an approval binds to.
        request: ToolConfirmationRequest,
        /// Why.
        reason: String,
    },
}

/// The tool guardrails as the governed path's screen.
struct GuardrailScreen(ToolGuardrailSet);

#[async_trait]
impl ToolCallScreen for GuardrailScreen {
    async fn screen(&self, tool_name: &str, args: &Value) -> Result<Value, String> {
        match self.0.evaluate(tool_name, args).await {
            ToolCallDecision::Allow { args } => Ok(args),
            ToolCallDecision::Deny { reason, .. } => {
                Err(format!("Tool '{tool_name}' was not run: {reason}"))
            }
        }
    }
}

impl Governance {
    /// Governance for `company` in memory: tests and judge mode. Durable stores come with
    /// [`Governance::with_stores`].
    pub fn in_memory(
        company: &str,
        founder: &str,
        policy: Arc<CompanyPolicy>,
        audit: Arc<SignedAudit>,
    ) -> Self {
        Self::with_stores(
            company,
            founder,
            policy,
            audit,
            Arc::new(InMemoryApprovalStore::new()),
            Arc::new(InMemoryActionLedger::new()),
        )
    }

    /// Governance over these stores (SQLite on the node: adk-session's `SqliteApprovalStore`
    /// and `SqliteActionLedger`).
    pub fn with_stores(
        company: &str,
        founder: &str,
        policy: Arc<CompanyPolicy>,
        audit: Arc<SignedAudit>,
        approvals: Arc<dyn ApprovalStore>,
        ledger: Arc<dyn ActionLedger>,
    ) -> Self {
        Self::with_state(company, founder, policy, audit, approvals, ledger, None)
    }

    /// [`Governance::with_stores`], with the freeze kept in `state` (mirrored, so it outlives
    /// a restart) when there is one.
    pub fn with_state(
        company: &str,
        founder: &str,
        policy: Arc<CompanyPolicy>,
        audit: Arc<SignedAudit>,
        approvals: Arc<dyn ApprovalStore>,
        ledger: Arc<dyn ActionLedger>,
        state: Option<Arc<dyn adk_core::GovernanceStateStore>>,
    ) -> Self {
        let control = match state {
            Some(state) => adk_kernel::KernelConfig::mirrored_governance(state),
            None => GovernanceControl::new(),
        };
        control.attach_audit(audit.trail());
        Self {
            company: company.to_owned(),
            founder: founder.to_owned(),
            policy,
            approvals,
            ledger,
            control,
            audit,
            scopes: Mutex::new(BTreeSet::new()),
            approved: Mutex::new(Vec::new()),
        }
    }

    /// The run config every agent of the company runs under.
    pub fn run_config(&self, staff: &StaffIdentity) -> RunConfig {
        RunConfig::builder()
            .tool_policy(self.policy.clone())
            .approval_store(self.approvals.clone())
            .action_ledger(self.ledger.clone())
            .governance(self.control.clone())
            .audit(self.audit.trail())
            .agent_principal(staff.agent_name(), staff.principal.clone())
            .build()
    }

    /// Notes a scope calls may be held in, so the founder's list finds them.
    pub fn watch_scope(&self, scope: &ApprovalScope) {
        self.scopes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((
                scope.app_name.clone(),
                scope.user_id.clone(),
                scope.session_id.clone(),
            ));
    }

    /// The founder's subject.
    pub fn founder_subject(&self) -> SubjectRef {
        SubjectRef::human(&self.founder)
    }

    fn founder_actor(&self) -> AuditActor {
        AuditActor::subject(self.founder_subject())
    }

    /// Puts the founder's signed mandate in force (or none).
    pub fn set_mandate(&self, mandate: Option<MandateTerms>, now: chrono::DateTime<chrono::Utc>) {
        self.policy.set_mandate(mandate, now);
    }

    /// Freezes every money tool of the company, recorded as the founder's.
    pub async fn freeze(&self, reason: &str) -> anyhow::Result<()> {
        self.control
            .freeze_recorded(reason, self.founder_actor())
            .await?;
        Ok(())
    }

    /// Lifts the freeze, recorded as the founder's.
    pub async fn unfreeze(&self) -> anyhow::Result<()> {
        self.control.unfreeze_recorded(self.founder_actor()).await?;
        Ok(())
    }

    /// Every call waiting for the founder.
    pub async fn held(&self) -> anyhow::Result<Vec<Held>> {
        let scopes: Vec<_> = self
            .scopes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect();
        let mut out = Vec::new();
        for (app, user, session) in scopes {
            let scope = ApprovalScope::new(&app, &user, &session);
            for PendingApproval {
                fingerprint,
                request,
                ..
            } in self.approvals.pending(&scope).await?
            {
                out.push(Held {
                    scope: (app.clone(), user.clone(), session.clone()),
                    fingerprint,
                    tool: request.tool_name.clone(),
                    args: request.args.clone(),
                    agent: request.acting.as_ref().map(|a| a.agent.id.to_string()),
                    request,
                });
            }
        }
        Ok(out)
    }

    /// The founder approves `held` exactly as it is.
    pub async fn approve(&self, held: &Held) -> anyhow::Result<()> {
        self.decide(held, ToolApproval::approve(), "approved").await
    }

    /// The founder approves `held` with its arguments changed (a lower amount). The governed
    /// path checks the edited arguments against the policy again before anything runs, and an
    /// edit the policy still holds goes back to the founder as a new call.
    pub async fn approve_edited(&self, held: &Held, args: Value) -> anyhow::Result<()> {
        self.decide(
            held,
            ToolApproval::approve().with_edited_args(args),
            "approved_edited",
        )
        .await
    }

    /// The founder declines `held`.
    pub async fn decline(&self, held: &Held, why: &str) -> anyhow::Result<()> {
        self.decide(held, ToolApproval::deny().with_message(why), "declined")
            .await
    }

    /// An edit voids earlier approvals: when a call is held for an action (a bill, a capture)
    /// that the founder already approved with other arguments, the earlier approval is taken out
    /// of the store unused, so the call it was for is held again. Returns how many were voided.
    pub async fn supersede(&self, request: &ToolConfirmationRequest) -> anyhow::Result<usize> {
        let Some(key) = action_key(&request.tool_name, &request.args) else {
            return Ok(0);
        };
        let fingerprint = request.fingerprint();
        let stale: Vec<Held> = {
            let mut approved = self.approved.lock().unwrap_or_else(|p| p.into_inner());
            let (stale, keep): (Vec<_>, Vec<_>) = approved.drain(..).partition(|h| {
                action_key(&h.tool, &h.args).as_deref() == Some(key.as_str())
                    && h.fingerprint != fingerprint
            });
            *approved = keep;
            stale
        };
        let mut voided = 0;
        for h in stale {
            if self
                .approvals
                .take_approval(&h.approval_scope(), &h.fingerprint)
                .await?
                .is_some()
            {
                voided += 1;
                // The call it was for waits for the founder again.
                self.approvals
                    .record_pending(&h.approval_scope(), &h.request)
                    .await?;
                self.audit
                    .record(
                        AuditRecordDraft::new(AuditRecordKind::Approval, h.tool.clone())
                            .actor(self.founder_actor())
                            .args_digest(self.audit.digest(&h.args))
                            .decision(AuditDecision::Rejected)
                            .ext("how", json!("voided_by_edit"))
                            .ext("action", json!(key)),
                    )
                    .await?;
            }
        }
        Ok(voided)
    }

    async fn decide(&self, held: &Held, approval: ToolApproval, how: &str) -> anyhow::Result<()> {
        self.approvals
            .decide(&held.approval_scope(), &held.fingerprint, approval)
            .await?;
        if how != "declined" {
            self.approved
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(held.clone());
        }
        self.audit
            .record(
                AuditRecordDraft::new(AuditRecordKind::Approval, held.tool.clone())
                    .actor(self.founder_actor())
                    .args_digest(self.audit.digest(&held.args))
                    .decision(if how == "declined" {
                        AuditDecision::Rejected
                    } else {
                        AuditDecision::Approved
                    })
                    .ext("how", json!(how))
                    .ext("fingerprint", json!(json_digest(&json!(held.fingerprint)))),
            )
            .await?;
        Ok(())
    }

    /// One tool call from a graph node, on the governed path: the freeze, the policy, the
    /// guardrails, the approval for exactly this call, then the action ledger around the desk.
    ///
    /// `call_id` names the call within the flow (the same on a resume); `resumes` is the request
    /// it was held with, when the flow resumes after the founder decided.
    #[allow(clippy::too_many_arguments)]
    pub async fn call(
        &self,
        desk: &Arc<Desk>,
        staff: &StaffIdentity,
        scope: &ApprovalScope,
        invocation_id: &str,
        tool: &str,
        call_id: &str,
        args: Value,
        resumes: Option<ToolConfirmationRequest>,
    ) -> anyhow::Result<CallOutcome> {
        let spec = crate::tools::spec(tool).ok_or_else(|| anyhow::anyhow!("no tool {tool}"))?;
        let tool_impl = DeskTool::new(spec, desk.clone());
        self.watch_scope(scope);
        let config = self.run_config(staff);
        let screen = GuardrailScreen(tool_guardrails());
        let gate = ToolGate::new(&config, staff.agent_name(), invocation_id, scope.clone())
            .with_acting_context(staff.acting())
            .with_screen(&screen);
        let mut call = GovernedCall::for_tool(&tool_impl, call_id, args);
        if let Some(held) = resumes {
            call = call.resuming(held);
        }
        let args = match adk_core::authorize_tool_call(&gate, call).await {
            Ok(ToolAuthorization::Execute { args, .. }) => args,
            Ok(ToolAuthorization::Refuse { reason, .. }) => {
                return Ok(CallOutcome::Refused(reason));
            }
            Ok(ToolAuthorization::Pending { request, reason }) => {
                return Ok(CallOutcome::Held { request, reason });
            }
            Err(e) => return Ok(CallOutcome::Refused(e.message)),
        };
        let key = format!(
            "{}/{}/{}/flow/{call_id}",
            scope.app_name, scope.user_id, scope.session_id
        );
        let ctx = CallCtx {
            acting: staff.acting(),
            key: key.clone(),
        };
        match self.ledger.get(&key).await? {
            Some(record) if record.is_completed() => return Ok(CallOutcome::AlreadyDone),
            // Begun and never finished (a crash mid-call): the rails and the books are
            // idempotent by this key, so running it again reads the first outcome back instead
            // of paying twice.
            Some(_) => {}
            None => {
                self.ledger
                    .begin(&ActionRecord::new(&key, tool, &args, tool_impl.effect()))
                    .await?;
            }
        }
        // The freeze is checked again between the ledger and the effect.
        if let Err(frozen) = config.check_governance() {
            self.ledger
                .complete(
                    &key,
                    ActionOutcome::Failed {
                        error: adk_core::not_run_text(&frozen),
                    },
                )
                .await?;
            return Ok(CallOutcome::Refused(frozen.message));
        }
        let result = desk.run(tool, &ctx, &args).await;
        let outcome = match &result {
            Ok(v) => ActionOutcome::Succeeded {
                result_digest: json_digest(v),
            },
            Err(e) => ActionOutcome::Failed {
                error: e.to_string(),
            },
        };
        self.ledger.complete(&key, outcome).await?;
        self.audit
            .record(
                AuditRecordDraft::new(AuditRecordKind::ToolCall, tool)
                    .actor(AuditActor::new(&staff.acting(), invocation_id))
                    .args_digest(self.audit.digest(&args))
                    .outcome(if result.is_ok() {
                        AuditRecordOutcome::Succeeded
                    } else {
                        AuditRecordOutcome::Failed
                    })
                    .ext("ledgerKey", json!(key)),
            )
            .await?;
        Ok(CallOutcome::Done(result?))
    }
}
