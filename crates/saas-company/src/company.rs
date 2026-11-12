//! One company, assembled: its desk, its governance and its staff.
//!
//! [`Company::assemble`] takes the company's configuration ([`CompanyConfig`]: who it is, its
//! policy file, what it sells) and its parts ([`Parts`]: the books, PayPal, settlement, the
//! models, the clock) and builds the five agents on the governed path. The kernel
//! ([`crate::supervisor`]) runs it; tests and judge mode drive it directly.

use std::collections::BTreeMap;
use std::sync::Arc;

use adk_core::{ApprovalScope, Clock, Llm};
use adk_session::SessionService;
use saas_core::{Books, CompanyId};

use crate::audit::SignedAudit;
use crate::desk::{Desk, Outbox, PlanOffer, agentrix_catalogue};
use crate::governed::Governance;
use crate::identity::{ROLES, StaffCredentials, StaffIdentity};
use crate::mandate::MandateSource;
use crate::memory::CounterpartyMemory;
use crate::policy::{CompanyPolicy, PolicyFile};
use crate::rails::{Rails, Settlement};
use crate::screen::{InboundKind, Screened, screen_inbound};
use crate::staff::{Member, TurnReport};
use crate::stores::CompanyStores;

/// Who a company is.
#[derive(Debug, Clone)]
pub struct CompanyConfig {
    /// `agentrix`, `kilele`.
    pub slug: String,
    /// "Agentrix", "Kilele Bakery".
    pub name: String,
    /// The founder's email: sponsor of every agent, the one who approves.
    pub founder: String,
    /// The policy file.
    pub policy: PolicyFile,
    /// What it sells.
    pub catalogue: Vec<PlanOffer>,
}

impl CompanyConfig {
    /// Agentrix, which sells its own WebAgent plans.
    pub fn agentrix(founder: &str) -> Self {
        Self {
            slug: "agentrix".into(),
            name: "Agentrix".into(),
            founder: founder.into(),
            policy: PolicyFile::agentrix(),
            catalogue: agentrix_catalogue(),
        }
    }

    /// Kilele Bakery, a customer with AI staff of its own.
    pub fn kilele(founder: &str) -> Self {
        Self {
            slug: "kilele".into(),
            name: "Kilele Bakery".into(),
            founder: founder.into(),
            policy: PolicyFile::kilele(),
            catalogue: Vec::new(),
        }
    }
}

/// The model each role runs on.
#[derive(Clone)]
pub struct ModelMix {
    by_role: BTreeMap<String, Arc<dyn Llm>>,
}

impl std::fmt::Debug for ModelMix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.by_role.iter().map(|(r, m)| (r, m.name())))
            .finish()
    }
}

impl ModelMix {
    /// Every role on `model`.
    pub fn all(model: Arc<dyn Llm>) -> Self {
        Self {
            by_role: ROLES
                .iter()
                .map(|r| ((*r).to_owned(), model.clone()))
                .collect(),
        }
    }

    /// `role` on `model` instead.
    pub fn with(mut self, role: &str, model: Arc<dyn Llm>) -> Self {
        self.by_role.insert(role.to_owned(), model);
        self
    }

    /// The model of `role`.
    pub fn model(&self, role: &str) -> anyhow::Result<Arc<dyn Llm>> {
        self.by_role
            .get(role)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no model for {role}"))
    }
}

/// What a company runs on.
pub struct Parts {
    /// Its books.
    pub books: Arc<dyn Books>,
    /// PayPal.
    pub rails: Arc<dyn Rails>,
    /// Agent-to-agent settlement.
    pub settlement: Arc<dyn Settlement>,
    /// Where reminders go.
    pub outbox: Arc<dyn Outbox>,
    /// The clock.
    pub clock: Arc<dyn Clock>,
    /// Scoped credentials.
    pub creds: StaffCredentials,
    /// What it remembers.
    pub memory: CounterpartyMemory,
    /// The signed audit log.
    pub audit: Arc<SignedAudit>,
    /// The models.
    pub models: ModelMix,
}

/// One company, assembled.
pub struct Company {
    /// Who.
    pub config: CompanyConfig,
    /// Its operations.
    pub desk: Arc<Desk>,
    /// Its governed path.
    pub governance: Arc<Governance>,
    staff: BTreeMap<String, Member>,
    sessions: Arc<dyn SessionService>,
    access: std::sync::OnceLock<Arc<crate::access::AgentAccess>>,
    signed: std::sync::RwLock<Option<crate::mandate::SignedTerms>>,
}

impl std::fmt::Debug for Company {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Company")
            .field("slug", &self.config.slug)
            .finish_non_exhaustive()
    }
}

impl Company {
    /// Builds the company: desk, governance and the five agents.
    pub fn assemble(config: CompanyConfig, parts: Parts) -> anyhow::Result<Self> {
        let approvals: Arc<dyn adk_core::ApprovalStore> =
            Arc::new(adk_core::InMemoryApprovalStore::new());
        let ledger: Arc<dyn adk_core::ActionLedger> =
            Arc::new(adk_core::InMemoryActionLedger::new());
        Self::build(
            config,
            parts,
            approvals,
            ledger,
            Member::memory_sessions(),
            None,
        )
    }

    /// Builds the company on `stores` (SQLite on the node): approvals, the action ledger, the
    /// signed audit log and the agents' sessions survive a restart. `parts.audit` is replaced
    /// by the stores' log.
    pub fn assemble_on(
        config: CompanyConfig,
        mut parts: Parts,
        stores: &CompanyStores,
    ) -> anyhow::Result<Self> {
        parts.audit = stores.audit.clone();
        Self::build(
            config,
            parts,
            stores.approvals.clone(),
            stores.ledger.clone(),
            stores.sessions.clone(),
            stores.kernel.as_ref().map(|k| k.governance.clone()),
        )
    }

    fn build(
        config: CompanyConfig,
        parts: Parts,
        approvals: Arc<dyn adk_core::ApprovalStore>,
        ledger: Arc<dyn adk_core::ActionLedger>,
        sessions: Arc<dyn SessionService>,
        governance_state: Option<Arc<dyn adk_core::GovernanceStateStore>>,
    ) -> anyhow::Result<Self> {
        let company = CompanyId::new(config.slug.clone())?;
        let desk = Arc::new(Desk::new(
            company,
            parts.books,
            parts.rails,
            parts.settlement,
            parts.memory,
            parts.creds,
            parts.outbox,
            parts.clock,
            config.catalogue.clone(),
        ));
        let policy = Arc::new(CompanyPolicy::new(config.policy.clone(), desk.clone()));
        let governance = Arc::new(Governance::with_state(
            &config.slug,
            &config.founder,
            policy,
            parts.audit,
            approvals,
            ledger,
            governance_state,
        ));
        let mut staff = BTreeMap::new();
        for role in ROLES {
            let identity = StaffIdentity::new(&config.slug, role, &config.founder)?;
            let member = Member::new(
                identity,
                parts.models.model(role)?,
                &desk,
                &governance,
                sessions.clone(),
            )?;
            staff.insert(role.to_owned(), member);
        }
        Ok(Self {
            config,
            desk,
            governance,
            staff,
            sessions,
            access: std::sync::OnceLock::new(),
            signed: std::sync::RwLock::new(None),
        })
    }

    /// Gives the company its per-agent switch (once; the node does it at boot).
    pub fn attach_access(&self, access: Arc<crate::access::AgentAccess>) {
        let _ = self.access.set(access);
    }

    /// The per-agent switch: revoke one agent, restore it with a passkey.
    pub fn access(&self) -> Option<&Arc<crate::access::AgentAccess>> {
        self.access.get()
    }

    /// Puts the founder's signed mandate from `source` in force (or none, when there is none
    /// or it lapsed: then every money tool is denied). Returns whether one is in force.
    pub async fn load_mandate(&self, source: &dyn MandateSource) -> anyhow::Result<bool> {
        let signed = source.signed(&self.config.slug).await?;
        let now = self.desk.clock.now();
        self.governance
            .set_mandate(signed.as_ref().map(|s| s.terms.clone()), now);
        let in_force = self.governance.policy.mandate().is_some();
        *self.signed.write().unwrap_or_else(|p| p.into_inner()) = signed.filter(|_| in_force);
        Ok(in_force)
    }

    /// The signed mandate in force, with how it was signed.
    pub fn signed_mandate(&self) -> Option<crate::mandate::SignedTerms> {
        self.signed
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The session service every member's runs are in.
    pub fn sessions(&self) -> Arc<dyn SessionService> {
        self.sessions.clone()
    }

    /// The member of staff in `role`.
    pub fn member(&self, role: &str) -> anyhow::Result<&Member> {
        self.staff
            .get(role)
            .ok_or_else(|| anyhow::anyhow!("{} has no {role}", self.config.slug))
    }

    /// `role` takes a turn in `session` on `text`.
    pub async fn ask(&self, role: &str, session: &str, text: &str) -> anyhow::Result<TurnReport> {
        self.ask_observed(role, session, text, None).await
    }

    /// [`Company::ask`], telling `steps` each step of the turn as it happens.
    pub async fn ask_observed(
        &self,
        role: &str,
        session: &str,
        text: &str,
        steps: Option<&tokio::sync::mpsc::UnboundedSender<crate::staff::TurnStep>>,
    ) -> anyhow::Result<TurnReport> {
        let member = self.member(role)?;
        self.governance.watch_scope(&ApprovalScope::new(
            member.identity.app_name().as_str(),
            member.user(),
            session,
        ));
        let report = member.turn_observed(session, text, steps).await?;
        for held in &report.held {
            self.governance.supersede(held).await?;
        }
        Ok(report)
    }

    /// `role` resumes `session` after the founder decided what it held.
    pub async fn resume(&self, role: &str, session: &str) -> anyhow::Result<TurnReport> {
        let report = self.member(role)?.resume(session).await?;
        for held in &report.held {
            self.governance.supersede(held).await?;
        }
        Ok(report)
    }

    /// Screens inbound text, then gives it to `role` in `session`, unless the screen blocked
    /// it. Returns the screening and, when it reached the agent, the turn.
    pub async fn inbound(
        &self,
        role: &str,
        session: &str,
        kind: InboundKind,
        from: &str,
        text: &str,
    ) -> anyhow::Result<(Screened, Option<TurnReport>)> {
        let screened = screen_inbound(kind, from, text).await;
        if screened.blocked {
            tracing::warn!(
                company = %self.config.slug,
                from = %screened.from,
                "inbound text blocked: it instructs the AI"
            );
            return Ok((screened, None));
        }
        let turn = self.ask(role, session, &screened.for_model).await?;
        Ok((screened, Some(turn)))
    }
}
