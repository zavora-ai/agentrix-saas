//! Who is acting: every company and every agent has a typed identity, and every agent reaches
//! the books and PayPal with a credential scoped to it, which no model ever sees.
//!
//! - **Agents** are `adk_core` principals: `urn:adk:agent:<company>/<role>`, tenant the company,
//!   class the role, sponsored by the founder ([`StaffIdentity`]). The governed path, the policy
//!   file and every signed audit record name the agent by that id, so one company's policy never
//!   admits another company's agent.
//! - **Sessions and runs** are `adk_core::identity` values: the app is the company
//!   (`agentrix-saas/<company>`), the user is the agent or the founder, the session is the work
//!   item (an invoice, a bill, a counterparty).
//! - **Credentials** come from an `adk_core::CredentialBroker` ([`StaffCredentials`]): a tool
//!   asks for a token for the agent, the resource (the company's books, PayPal, the agent door)
//!   and the scope it needs (`paypal:payout`), and gets a token bound to that audience. In
//!   tests and judge mode it is adk-auth's `SecretCredentialBroker` over the company's secrets;
//!   on the node it is adk-auth's RFC 8693 `TokenExchangeBroker` against the company's token
//!   endpoint ([`crate::sts`]). Either way the secret stays in the tool's code path: it is never
//!   a tool argument, a tool result or a model message.

use std::collections::BTreeMap;
use std::sync::Arc;

use adk_core::{
    ActingContext, AdkError, AgentId, AgentPrincipal, AppName, CredentialBroker, ScopedToken,
    SubjectRef, Url, UserId,
};
use async_trait::async_trait;

/// The Chief: runs the company, answers the founder.
pub const CHIEF: &str = "chief";
/// Revenue: plans, the pipeline, subscriptions and recurring invoices.
pub const REVENUE: &str = "revenue";
/// Receivables: invoices, collections and debt plans.
pub const RECEIVABLES: &str = "receivables";
/// Payables: bills, and paying them.
pub const PAYABLES: &str = "payables";
/// Support: cancellations and refunds.
pub const SUPPORT: &str = "support";

/// Every role on a company's staff.
pub const ROLES: [&str; 5] = [CHIEF, REVENUE, RECEIVABLES, PAYABLES, SUPPORT];

/// The agent id of `role` at `company`: `urn:adk:agent:<company>/<role>`.
pub fn agent_id_for(company: &str, role: &str) -> anyhow::Result<AgentId> {
    let slug = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    anyhow::ensure!(slug(company), "company {company:?} isn't a slug");
    anyhow::ensure!(slug(role), "role {role:?} isn't a slug");
    Ok(AgentId::new(format!("urn:adk:agent:{company}/{role}"))?)
}

/// The adk-rust app name of `company`'s runs.
pub fn app_name(company: &str) -> AppName {
    AppName::new_unchecked(format!("agentrix-saas/{company}"))
}

/// One agent on one company's staff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaffIdentity {
    /// `agentrix`, `kilele`.
    pub company: String,
    /// `payables`.
    pub role: String,
    /// The principal the governed path, the policy and the audit log name.
    pub principal: AgentPrincipal,
}

impl StaffIdentity {
    /// `role` at `company`, sponsored by `founder` (the owner's email).
    pub fn new(company: &str, role: &str, founder: &str) -> anyhow::Result<Self> {
        let principal = AgentPrincipal::new(agent_id_for(company, role)?)
            .with_tenant(company)
            .with_class(role)
            .with_sponsor(SubjectRef::human(founder));
        Ok(Self {
            company: company.to_owned(),
            role: role.to_owned(),
            principal,
        })
    }

    /// The agent's id.
    pub fn id(&self) -> &AgentId {
        &self.principal.id
    }

    /// The agent's name inside a run: the role.
    pub fn agent_name(&self) -> &str {
        &self.role
    }

    /// The app its runs belong to.
    pub fn app_name(&self) -> AppName {
        app_name(&self.company)
    }

    /// The user its own runs are under: `agent:<company>/<role>`.
    pub fn user_id(&self) -> UserId {
        UserId::new_unchecked(format!("agent:{}/{}", self.company, self.role))
    }

    /// Acting on its own account.
    pub fn acting(&self) -> ActingContext {
        ActingContext::new(self.principal.clone())
    }
}

/// What an agent's credential is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Resource {
    /// The company's books (Zavora ERA, through mcp-erp and its REST API).
    Books,
    /// PayPal's REST API.
    PayPal,
    /// The agent door other companies are reached through (A2A).
    AgentDoor,
}

/// The resource indicators (RFC 8707) of one company's resources.
#[derive(Debug, Clone)]
pub struct Resources {
    books: Url,
    paypal: Url,
    door: Url,
}

impl Resources {
    /// The hosted defaults for `company`.
    pub fn hosted(company: &str) -> Self {
        let url = |s: String| Url::parse(&s).expect("a valid resource url");
        Self {
            books: url(format!("https://books.agentrix.website/{company}/")),
            paypal: url("https://api-m.sandbox.paypal.com/".into()),
            door: url("https://pay.agentrix.website/a2a/".into()),
        }
    }

    /// The url of `resource`.
    pub fn url(&self, resource: Resource) -> &Url {
        match resource {
            Resource::Books => &self.books,
            Resource::PayPal => &self.paypal,
            Resource::AgentDoor => &self.door,
        }
    }
}

/// The scopes each role is granted, per resource. Nothing else is ever issued to it.
pub fn granted_scopes(role: &str) -> BTreeMap<Resource, Vec<&'static str>> {
    let mut out = BTreeMap::new();
    let mut grant = |r: Resource, scopes: &[&'static str]| {
        out.insert(r, scopes.to_vec());
    };
    match role {
        CHIEF => grant(Resource::Books, &["books:read"]),
        REVENUE => {
            grant(
                Resource::Books,
                &["books:read", "books:crm", "books:invoice"],
            );
            grant(Resource::PayPal, &["paypal:subscribe"]);
        }
        RECEIVABLES => {
            grant(
                Resource::Books,
                &["books:read", "books:invoice", "books:remind"],
            );
            grant(Resource::PayPal, &["paypal:subscribe"]);
            grant(Resource::AgentDoor, &["door:collect", "door:offer"]);
        }
        PAYABLES => {
            grant(
                Resource::Books,
                &["books:read", "books:bill", "books:vendor"],
            );
            grant(Resource::PayPal, &["paypal:payout"]);
            grant(Resource::AgentDoor, &["door:settle"]);
        }
        SUPPORT => {
            grant(Resource::Books, &["books:read", "books:refund"]);
            grant(Resource::PayPal, &["paypal:refund", "paypal:subscribe"]);
        }
        _ => {}
    }
    out
}

/// A company's credential broker and its resources: what a tool asks before it touches the
/// books or PayPal.
#[derive(Clone)]
pub struct StaffCredentials {
    broker: Arc<dyn CredentialBroker>,
    resources: Resources,
    check: Option<Arc<dyn TokenCheck>>,
}

/// Where a token is checked before each use: the company's token service, which answers no
/// for a token it didn't issue, one that lapsed, or one of an agent the owner revoked.
pub trait TokenCheck: Send + Sync {
    /// Whether the owner switched `agent` off.
    fn revoked(&self, agent: &str) -> bool;
    /// Whether the service still vouches for `token`, held by `agent`.
    fn holds(&self, agent: &str, token: &str) -> bool;
}

impl std::fmt::Debug for StaffCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaffCredentials")
            .field("resources", &self.resources)
            .finish_non_exhaustive()
    }
}

impl StaffCredentials {
    /// Over `broker`.
    pub fn new(broker: Arc<dyn CredentialBroker>, resources: Resources) -> Self {
        Self {
            broker,
            resources,
            check: None,
        }
    }

    /// Checks every token at `check` before it is used (the token service: a revoked agent's
    /// tokens stop at the next use).
    pub fn checked_by(mut self, check: Arc<dyn TokenCheck>) -> Self {
        self.check = Some(check);
        self
    }

    /// Secrets held in memory, bound per agent and resource with [`granted_scopes`]: tests and
    /// judge mode. `secrets` maps a resource to the secret's value for this company.
    pub fn in_memory(
        company: &str,
        founder: &str,
        secrets: BTreeMap<Resource, String>,
    ) -> anyhow::Result<Self> {
        let resources = Resources::hosted(company);
        let mut names = BTreeMap::new();
        for (resource, value) in &secrets {
            names.insert(format!("{company}/{resource:?}"), value.clone());
        }
        let mut broker = adk_auth::SecretCredentialBroker::new(Arc::new(MemorySecrets(names)));
        for role in ROLES {
            let staff = StaffIdentity::new(company, role, founder)?;
            for (resource, scopes) in granted_scopes(role) {
                if !secrets.contains_key(&resource) {
                    continue;
                }
                broker = broker.bind(
                    staff.id().clone(),
                    resources.url(resource).clone(),
                    format!("{company}/{resource:?}"),
                    scopes.iter().map(|s| (*s).to_owned()).collect(),
                );
            }
        }
        Ok(Self::new(Arc::new(broker), resources))
    }

    /// The resource urls.
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// A token for `acting` to use `resource` with `scope`, checked against the resource's
    /// audience. The tool keeps it; it never goes to the model.
    pub async fn token(
        &self,
        acting: &ActingContext,
        resource: Resource,
        scope: &str,
    ) -> adk_core::Result<ScopedToken> {
        let url = self.resources.url(resource);
        let mut token = self
            .broker
            .token_for(acting, url, &[scope.to_owned()])
            .await?;
        if let Some(check) = &self.check {
            let agent = acting.agent.id.as_str();
            let forget = adk_core::RevocationScope::Agent(acting.agent.id.clone());
            if check.revoked(agent) {
                self.broker.revoke(&forget).await?;
                return Err(AdkError::agent(format!(
                    "the owner revoked {agent}; its tokens no longer work"
                )));
            }
            if !check.holds(agent, token.secret.expose()) {
                // A token cached from before a revoke and restore: ask again, once.
                self.broker.revoke(&forget).await?;
                token = self
                    .broker
                    .token_for(acting, url, &[scope.to_owned()])
                    .await?;
                if !check.holds(agent, token.secret.expose()) {
                    return Err(AdkError::agent(format!(
                        "the token service doesn't vouch for {agent}'s token"
                    )));
                }
            }
        }
        token.authorize_for(url)?;
        if !token.scopes.iter().any(|s| s == scope) {
            return Err(AdkError::agent(format!(
                "credential for {} doesn't carry {scope}",
                acting.agent.id
            )));
        }
        Ok(token)
    }
}

/// Secrets in memory, by name.
struct MemorySecrets(BTreeMap<String, String>);

#[async_trait]
impl adk_auth::secrets::SecretProvider for MemorySecrets {
    async fn get_secret(&self, name: &str) -> Result<String, AdkError> {
        self.0
            .get(name)
            .cloned()
            .ok_or_else(|| AdkError::agent(format!("no secret named {name}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> StaffCredentials {
        StaffCredentials::in_memory(
            "agentrix",
            "founder@agentrix.website",
            BTreeMap::from([
                (Resource::Books, "era-key".to_owned()),
                (Resource::PayPal, "paypal-key".to_owned()),
            ]),
        )
        .unwrap()
    }

    #[test]
    fn agent_ids_name_the_company_and_role() {
        let id = agent_id_for("kilele", "payables").unwrap();
        assert_eq!(id.as_str(), "urn:adk:agent:kilele/payables");
        assert!(agent_id_for("Kilele", "payables").is_err());
        let staff = StaffIdentity::new("agentrix", PAYABLES, "founder@agentrix.website").unwrap();
        assert_eq!(staff.principal.tenant.as_deref(), Some("agentrix"));
        assert_eq!(staff.user_id().as_str(), "agent:agentrix/payables");
    }

    #[tokio::test]
    async fn an_agent_gets_only_the_scopes_its_role_is_granted() {
        let creds = creds();
        let payables = StaffIdentity::new("agentrix", PAYABLES, "f@a").unwrap();
        let support = StaffIdentity::new("agentrix", SUPPORT, "f@a").unwrap();
        let token = creds
            .token(&payables.acting(), Resource::PayPal, "paypal:payout")
            .await
            .unwrap();
        assert_eq!(token.audience.host_str(), Some("api-m.sandbox.paypal.com"));
        assert!(
            creds
                .token(&support.acting(), Resource::PayPal, "paypal:payout")
                .await
                .is_err(),
            "support can't pay out"
        );
        let kilele = StaffIdentity::new("kilele", PAYABLES, "f@k").unwrap();
        assert!(
            creds
                .token(&kilele.acting(), Resource::PayPal, "paypal:payout")
                .await
                .is_err(),
            "another company's agent gets nothing from this broker"
        );
    }
}
