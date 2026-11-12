//! The resident supervisor: one adk-kernel `Kernel` per company, hosting its five agents as
//! targets. Agents wake on events; nothing polls.
//!
//! Wake sources:
//!
//! - **Due dates.** Each instalment of an agreed debt plan is a process that sleeps until its
//!   date (`Wake::At`) and then runs Receivables on it. A plan schedules its instalments the
//!   moment it is agreed ([`crate::desk::Desk::subscribe_plans`]).
//! - **PayPal webhook events** that `agentrix-effects` verified and applied
//!   ([`Supervisor::paypal_event`]): a failed subscription payment wakes Receivables, a dispute
//!   or refund wakes Support, a payout's failure wakes Payables.
//! - **ERA changes** ([`Supervisor::books_changed`]): a posted bill wakes Payables, an overdue
//!   invoice Receivables, a won deal Revenue.
//! - **Founder messages** ([`Supervisor::founder_message`]): the Chief, on the interactive queue.
//!
//! The kernel shares the company's `GovernanceControl`, so the founder's freeze stops the kernel
//! and every agent at once, and its audit trail is the company's signed log. Its event bus
//! ([`Supervisor::events`]) is what the owner console streams.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use adk_core::{ApprovalScope, Clock, InMemoryRunStore, NewRun};
use adk_kernel::control::ControlPlaneConfig;
use adk_kernel::{
    EventSubscriber, InMemoryProcessStore, Kernel, KernelConfig, Process, ProcessKind, ProcessSpec,
    QueueClass, Wake, WakeSet,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::company::Company;
use crate::desk::DebtPlan;
use crate::identity::{CHIEF, PAYABLES, RECEIVABLES, REVENUE, ROLES, SUPPORT};
use crate::policy::dollars;
use crate::screen::redact;

/// When on its due date an instalment is collected: 06:00 UTC, 09:00 in Nairobi.
pub fn due_at(date: NaiveDate) -> DateTime<Utc> {
    date.and_hms_opt(6, 0, 0)
        .expect("06:00 is a valid time")
        .and_utc()
}

/// A change in the books another system told the company about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum BooksChange {
    /// A bill was posted.
    BillPosted {
        /// The bill.
        bill_id: String,
    },
    /// An invoice went past due.
    InvoiceOverdue {
        /// The invoice.
        invoice_id: String,
    },
    /// A deal was won in the CRM.
    DealWon {
        /// The opportunity.
        opportunity_id: String,
    },
}

/// One company's resident supervisor.
pub struct Supervisor {
    company: Arc<Company>,
    kernel: Kernel,
    app: String,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervisor")
            .field("app", &self.app)
            .finish_non_exhaustive()
    }
}

impl Supervisor {
    /// Starts the kernel for `company`, on `clock` (a manual one in tests), keeping its files
    /// under `home`.
    pub async fn start(
        company: Arc<Company>,
        clock: Arc<dyn Clock>,
        home: PathBuf,
    ) -> anyhow::Result<Arc<Self>> {
        let runs: Arc<dyn adk_core::DurableRunStore> =
            Arc::new(InMemoryRunStore::new().with_clock(clock.clone()));
        Self::start_on(
            company,
            clock,
            home,
            runs,
            Arc::new(InMemoryProcessStore::new()),
            None,
        )
        .await
    }

    /// Starts the kernel over durable run and process stores (SQLite on the node), so a
    /// sleeping instalment wakes on its date after a restart.
    pub async fn start_on(
        company: Arc<Company>,
        clock: Arc<dyn Clock>,
        home: PathBuf,
        runs: Arc<dyn adk_core::DurableRunStore>,
        processes: Arc<dyn adk_kernel::ProcessStore>,
        bundle: Option<&adk_kernel::stores::KernelStores>,
    ) -> anyhow::Result<Arc<Self>> {
        let app = company
            .member(CHIEF)?
            .identity
            .app_name()
            .as_str()
            .to_owned();
        let worker = adk_server::background::DurableWorkerConfig::default()
            .with_worker_id(format!("{}-kernel", company.config.slug))
            .with_max_concurrent_runs(4);
        let config = KernelConfig::default()
            .with_app_name(app.clone())
            .with_home(home)
            .with_clock(clock.clone())
            .with_worker(worker)
            .with_scheduler_tick(Duration::from_millis(50))
            .with_governance(company.governance.control.clone())
            .with_control(ControlPlaneConfig::default().without_socket());
        let mut builder = Kernel::builder().config(config);
        // The kernel's own stores on the company's database; the run and process stores set
        // below are the same tables and win over the bundle's.
        if let Some(bundle) = bundle {
            builder = builder.stores(bundle);
        }
        let mut builder = builder
            .run_store(runs)
            .process_store(processes)
            .session_service(company.sessions())
            .audit(company.governance.audit.trail());
        for role in ROLES {
            builder = builder.agent(role, company.member(role)?.runner.clone());
        }
        let kernel = builder.build()?;
        kernel.start().await?;
        let supervisor = Arc::new(Self {
            company,
            kernel,
            app,
        });
        // Each plan agreed from now on schedules its instalments as it is agreed.
        let mut plans = supervisor.company.desk.subscribe_plans();
        let weak = Arc::downgrade(&supervisor);
        tokio::spawn(async move {
            while let Ok(plan) = plans.recv().await {
                let Some(supervisor) = weak.upgrade() else {
                    break;
                };
                if let Err(e) = supervisor.schedule_plan(&plan).await {
                    tracing::error!(plan = %plan.plan_id, error = %e, "couldn't schedule the plan's instalments");
                }
            }
        });
        Ok(supervisor)
    }

    /// The kernel.
    pub fn kernel(&self) -> &Kernel {
        &self.kernel
    }

    /// The company.
    pub fn company(&self) -> &Arc<Company> {
        &self.company
    }

    /// The kernel's event bus, from now on.
    pub fn events(&self) -> EventSubscriber {
        self.kernel.events()
    }

    /// A process, by id.
    pub async fn process(&self, pid: &str) -> anyhow::Result<Option<Process>> {
        Ok(self.kernel.processes().get(pid).await?)
    }

    /// Stops the kernel.
    pub async fn stop(&self) -> anyhow::Result<()> {
        Ok(self.kernel.stop().await?)
    }

    #[allow(clippy::too_many_arguments)]
    async fn spawn(
        &self,
        role: &str,
        session: &str,
        text: &str,
        kind: ProcessKind,
        queue: QueueClass,
        wake: Option<WakeSet>,
        key: &str,
    ) -> anyhow::Result<String> {
        let member = self.company.member(role)?;
        let user = member.user();
        self.company
            .governance
            .watch_scope(&ApprovalScope::new(&self.app, &user, session));
        let run = NewRun::new(
            queue.as_str(),
            &self.app,
            &user,
            role,
            json!({ "text": text }),
        )
        .with_session_id(session)
        .with_idempotency_key(key);
        let mut spec = ProcessSpec::new(kind)
            .with_label("company", self.company.config.slug.clone())
            .with_label("role", role);
        if let Some(wake) = wake {
            spec = spec.with_wake(wake);
        }
        let process = self.kernel.processes().spawn(run, spec).await?;
        Ok(process.run.run_id)
    }

    /// Schedules every uncollected instalment of `plan` on its due date. Returns the sleeping
    /// processes' ids.
    pub async fn schedule_plan(&self, plan: &DebtPlan) -> anyhow::Result<Vec<String>> {
        let mut pids = Vec::new();
        for (n, instalment) in plan.instalments.iter().enumerate() {
            if plan.collected.get(n) == Some(&true) {
                continue;
            }
            let text = format!(
                "Instalment {n} of {} ({} {:.2}) from {} is due today, {}. Collect it with \
                 collect_instalment (plan_id {}, instalment {n}).",
                plan.plan_id,
                plan.currency,
                dollars(instalment.amount_minor),
                plan.customer,
                instalment.due_date,
                plan.plan_id,
            );
            let pid = self
                .spawn(
                    RECEIVABLES,
                    &format!("collect-{}-{n}", plan.plan_id),
                    &text,
                    ProcessKind::Scheduled,
                    QueueClass::Background,
                    Some(WakeSet::any_of([Wake::At(due_at(instalment.due_date))])),
                    &format!("collect:{}:{n}", plan.plan_id),
                )
                .await?;
            pids.push(pid);
        }
        Ok(pids)
    }

    /// A PayPal event `agentrix-effects` verified and applied. Wakes the agent it concerns;
    /// `None` when it concerns none. Each event wakes at most once (its id is the key).
    pub async fn paypal_event(&self, event: &Value) -> anyhow::Result<Option<String>> {
        let id = event["id"].as_str().unwrap_or_default();
        let kind = event["event_type"].as_str().unwrap_or_default();
        let resource = &event["resource"];
        let resource_id = resource["id"].as_str().unwrap_or_default();
        let (role, text) = match kind {
            "BILLING.SUBSCRIPTION.PAYMENT.FAILED" => {
                self.company.desk.payment_failed(resource_id);
                (
                    RECEIVABLES,
                    format!(
                        "PayPal reports a failed payment on subscription {resource_id}. Decide \
                         whether to suspend it, as the founder's mandate allows."
                    ),
                )
            }
            "CUSTOMER.DISPUTE.CREATED" | "PAYMENT.CAPTURE.REVERSED" => (
                SUPPORT,
                format!("PayPal reports {kind} on {resource_id}. Look at the payment and answer."),
            ),
            k if k.starts_with("PAYMENT.PAYOUTS-ITEM.")
                && !k.ends_with("SUCCEEDED")
                && !k.ends_with("CLAIMED") =>
            {
                (
                    PAYABLES,
                    format!(
                        "PayPal reports {kind} for payout item {resource_id}. Tell the founder \
                         which bill it was and what happens next."
                    ),
                )
            }
            _ => return Ok(None),
        };
        let pid = self
            .spawn(
                role,
                &format!("paypal-{id}"),
                &redact(&text),
                ProcessKind::Triggered {
                    source: "paypal".into(),
                },
                QueueClass::Background,
                None,
                &format!("paypal:{id}"),
            )
            .await?;
        Ok(Some(pid))
    }

    /// A change in the books. Wakes the agent it concerns.
    pub async fn books_changed(&self, change: &BooksChange) -> anyhow::Result<String> {
        let (role, session, text) = match change {
            BooksChange::BillPosted { bill_id } => (
                PAYABLES,
                format!("bill-{bill_id}"),
                format!("Bill {bill_id} was posted. Pay it if it is inside the mandate."),
            ),
            BooksChange::InvoiceOverdue { invoice_id } => (
                RECEIVABLES,
                format!("overdue-{invoice_id}"),
                format!(
                    "Invoice {invoice_id} is past due. Look at the customer's history and remind them."
                ),
            ),
            BooksChange::DealWon { opportunity_id } => (
                REVENUE,
                format!("deal-{opportunity_id}"),
                format!("Opportunity {opportunity_id} was won. Set the customer up."),
            ),
        };
        let key = format!("books:{}", serde_json::to_string(change)?);
        self.spawn(
            role,
            &session,
            &text,
            ProcessKind::Triggered {
                source: "books".into(),
            },
            QueueClass::Background,
            None,
            &key,
        )
        .await
    }

    /// Wakes `role` in `session` on `text` (an event another part of the node saw: an invoice
    /// offered over the agent door, a plan agreed). Each `key` wakes once.
    pub async fn wake(
        &self,
        role: &str,
        session: &str,
        text: &str,
        source: &str,
        key: &str,
    ) -> anyhow::Result<String> {
        self.spawn(
            role,
            session,
            &redact(text),
            ProcessKind::Triggered {
                source: source.into(),
            },
            QueueClass::Background,
            None,
            key,
        )
        .await
    }

    /// Puts `standing` on the kernel: each a process asleep on its cron wake that recurs after
    /// every run, kept in the kernel's SQLite stores. Spawning one again (a restart) finds the
    /// one already there. Returns their ids.
    pub async fn schedule_standing(&self, standing: &[Standing]) -> anyhow::Result<Vec<String>> {
        let mut pids = Vec::new();
        for s in standing {
            let pid = self
                .spawn(
                    s.role,
                    &format!("cron-{}", s.name),
                    &s.text,
                    ProcessKind::Scheduled,
                    QueueClass::Background,
                    Some(WakeSet::any_of([Wake::Cron {
                        expr: s.cron.to_owned(),
                        missed: adk_core::MissedFirePolicy::FireOnce,
                    }])),
                    &format!("cron:{}:{}", s.name, s.cron),
                )
                .await?;
            tracing::info!(company = %self.company.config.slug, schedule = s.name, cron = s.cron, role = s.role, "scheduled");
            pids.push(pid);
        }
        Ok(pids)
    }

    /// The founder writes to the Chief.
    pub async fn founder_message(&self, session: &str, text: &str) -> anyhow::Result<String> {
        let key = format!("founder:{session}:{}", uuid::Uuid::new_v4());
        self.spawn(
            CHIEF,
            session,
            text,
            ProcessKind::Interactive,
            QueueClass::Interactive,
            None,
            &key,
        )
        .await
    }
}

/// What one agent of the company is doing now, from the kernel's processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentNow {
    /// The role.
    pub role: String,
    /// The sessions it is working in now (its processes claimed or queued).
    pub working: Vec<String>,
    /// Its next scheduled wake: when, and the session it wakes in (`cron-payables`).
    pub next: Option<(DateTime<Utc>, String)>,
}

/// When `wake` next fires after `after`: its time, or its cron expression's next occurrence.
pub fn next_wake(wake: &WakeSet, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    use std::str::FromStr;
    wake.any_of
        .iter()
        .filter_map(|w| match w {
            Wake::At(at) => Some(*at),
            Wake::Cron { expr, .. } => cron::Schedule::from_str(expr).ok()?.after(&after).next(),
            _ => None,
        })
        .min()
}

impl Supervisor {
    /// What each agent is doing at `now`: the processes it is working in, and when it wakes
    /// next.
    pub async fn agents_now(&self, now: DateTime<Utc>) -> anyhow::Result<Vec<AgentNow>> {
        use adk_core::RunState;
        let live = self
            .kernel
            .processes()
            .ps(adk_kernel::ProcessFilter::default().states([
                RunState::Queued,
                RunState::Running,
                RunState::Waiting,
            ]))
            .await?;
        Ok(ROLES
            .iter()
            .map(|role| {
                let mine = live.iter().filter(|p| p.run.target == *role);
                let session = |p: &Process| p.run.session_id.clone().unwrap_or_default();
                let working = mine
                    .clone()
                    .filter(|p| matches!(p.run.state, RunState::Queued | RunState::Running))
                    .map(session)
                    .collect();
                let next = mine
                    .filter(|p| p.run.state == RunState::Waiting)
                    .filter_map(|p| {
                        let at = next_wake(p.meta.wake.as_ref()?, now)?;
                        Some((at, session(p)))
                    })
                    .min();
                AgentNow {
                    role: (*role).to_owned(),
                    working,
                    next,
                }
            })
            .collect())
    }
}

/// A standing schedule: `role` is woken with `text` at each occurrence of `cron` (six-field,
/// UTC: seconds first).
#[derive(Debug, Clone)]
pub struct Standing {
    /// Its name, for the process and the logs.
    pub name: &'static str,
    /// The agent woken.
    pub role: &'static str,
    /// Six-field UTC cron.
    pub cron: &'static str,
    /// What the agent is told.
    pub text: String,
}

/// The live run's schedules (spec v3, "transform everything to live"), in UTC. Every company
/// runs collections and the Chief's brief to the owner every 3 hours, reconciles its wallet
/// against PayPal hourly, posts its payroll every 6 hours, and pays what is due every 3 hours
/// inside the founder's mandate (over it, a payment waits for the founder's passkey). The
/// platform company (`platform`: Agentrix) also bills its model spend and invoices its
/// agent-run customers for their usage every 3 hours; each customer's Payables settles that
/// invoice when it arrives at its agent door.
pub fn live_schedules(platform: bool) -> Vec<Standing> {
    let mut out = vec![
        Standing {
            name: "collections",
            role: RECEIVABLES,
            cron: "0 0 */3 * * *",
            text: "Scheduled collections run (every 3 hours). Read the receivables ageing with \
                   ar_ageing. For each invoice past due, send a reminder with send_reminder. \
                   Say what you did in one line."
                .into(),
        },
        Standing {
            name: "brief",
            role: CHIEF,
            cron: "0 30 */3 * * *",
            text: "Scheduled brief (every 3 hours). Read the company's position with \
                   company_brief, then send the owner a short brief with brief_owner: what \
                   customers owe and how much is late, what the company owes, debt plans \
                   running, and anything waiting for the owner's decision."
                .into(),
        },
        Standing {
            name: "reconcile",
            role: PAYABLES,
            cron: "0 15 * * * *",
            text: "Scheduled reconciliation (hourly). Reconcile the wallet record against \
                   PayPal with wallet_checkpoint, and say in one line whether it matches."
                .into(),
        },
        Standing {
            name: "payroll",
            role: PAYABLES,
            cron: "0 35 */6 * * *",
            text: "Scheduled payroll (every 6 hours). Post the payroll bill for the pay period \
                   that just ended with post_payroll, and say in one line what was posted. The \
                   payables run pays it."
                .into(),
        },
    ];
    if platform {
        out.push(Standing {
            name: "usage-invoices",
            role: RECEIVABLES,
            cron: "0 20 */3 * * *",
            text: "Scheduled usage billing (every 3 hours). Invoice each agent-run customer \
                   company for its platform usage with invoice_usage; each invoice is offered \
                   to that company's agent door, where its Payables settles it. Say in one line \
                   what was invoiced and what each door answered."
                .into(),
        });
        out.push(Standing {
            name: "spend-bill",
            role: PAYABLES,
            cron: "0 40 */3 * * *",
            text: "Scheduled model-spend billing (every 3 hours, before the payables run). \
                   Post the vendor bills for what the platform's model gateway metered with \
                   bill_model_spend, and say in one line what was billed."
                .into(),
        });
    }
    out.push(Standing {
        name: "payables",
        role: PAYABLES,
        cron: "0 45 */3 * * *",
        text: "Scheduled payables run (every 3 hours). List the bills due with ap_bills_due. \
               Pay each bill whose pay_with is pay_bill with pay_bill, to the payee the books \
               hold; settle each bill whose pay_with is settle_invoice with settle_invoice. A \
               payment over the founder's mandate waits for their passkey; say which."
            .into(),
    });
    out
}
