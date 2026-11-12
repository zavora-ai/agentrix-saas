//! Who is acting: companies and their agents, as adk-rust identities.
//!
//! A company is an adk-rust *app* ([`AppName`]): each company's sessions, memory and runs are
//! scoped to it. An agent is a named member of one company's staff; to the runtime it is the
//! *user* `agent:<company>/<name>` ([`UserId`]), so every action it takes is attributed to it.

use std::fmt;
use std::str::FromStr;

use adk_core::identity::{AppName, IdentityError, UserId};
use serde::{Deserialize, Serialize};

/// Why a company or agent id was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// adk-rust's own identity checks failed (empty, too long, null byte).
    #[error(transparent)]
    Identity(#[from] IdentityError),
    /// Ids here are lowercase slugs: `a-z`, `0-9` and `-`, starting with a letter, at most 64 bytes.
    #[error(
        "{kind} must be a lowercase slug (a-z, 0-9, '-', starting with a letter, at most 64 bytes): {value:?}"
    )]
    NotASlug {
        /// Which id.
        kind: &'static str,
        /// The value refused.
        value: String,
    },
    /// An agent id is written `<company>/<agent>`.
    #[error("an agent id is written <company>/<agent>: {0:?}")]
    AgentForm(String),
}

fn slug(value: &str, kind: &'static str) -> Result<(), IdError> {
    let ok = !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(IdError::NotASlug {
            kind,
            value: value.to_owned(),
        })
    }
}

/// One company the node runs, for example `agentrix` or `kilele-bakery`.
///
/// It is the adk-rust app name for that company's agents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CompanyId(AppName);

impl CompanyId {
    /// Agentrix Ltd, the company that sells WebAgent plans.
    pub fn agentrix() -> Self {
        Self(AppName::new_unchecked("agentrix"))
    }

    /// Kilele Bakery, the customer that runs its own Agentrix staff.
    pub fn kilele() -> Self {
        Self(AppName::new_unchecked("kilele-bakery"))
    }

    /// A company id from a slug.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        slug(&value, "CompanyId")?;
        Ok(Self(AppName::new(value)?))
    }

    /// The slug.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The adk-rust app this company's agents run under.
    pub fn app_name(&self) -> &AppName {
        &self.0
    }
}

impl fmt::Display for CompanyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CompanyId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for CompanyId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<CompanyId> for String {
    fn from(id: CompanyId) -> Self {
        id.as_str().to_owned()
    }
}

/// One agent on one company's staff, written `<company>/<name>`, for example
/// `agentrix/receivables`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentId {
    company: CompanyId,
    name: String,
}

impl AgentId {
    /// The Cashier: holds the company's wallet record. Receivables and Payables are its two desks
    /// and keep their own ids; a wallet entry's acting agent is the desk's.
    pub const CASHIER: &'static str = "cashier";
    /// The Chief: runs the company, talks to the founder.
    pub const CHIEF: &'static str = "chief";
    /// Revenue: plans, pricing and the sales pipeline.
    pub const REVENUE: &'static str = "revenue";
    /// Receivables: invoices, collections and debt plans.
    pub const RECEIVABLES: &'static str = "receivables";
    /// Payables: bills and paying them.
    pub const PAYABLES: &'static str = "payables";
    /// The public sales desk.
    pub const SALES_DESK: &'static str = "sales-desk";
    /// The public support desk.
    pub const SUPPORT_DESK: &'static str = "support-desk";

    /// An agent of `company` named `name` (a lowercase slug).
    pub fn new(company: CompanyId, name: impl Into<String>) -> Result<Self, IdError> {
        let name = name.into();
        slug(&name, "agent name")?;
        Ok(Self { company, name })
    }

    /// The company the agent works for.
    pub fn company(&self) -> &CompanyId {
        &self.company
    }

    /// The agent's name within its company, for example `receivables`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The adk-rust user this agent acts as: `agent:<company>/<name>`.
    pub fn user_id(&self) -> UserId {
        UserId::new_unchecked(format!("agent:{self}"))
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.company, self.name)
    }
}

impl FromStr for AgentId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (company, name) = s
            .split_once('/')
            .ok_or_else(|| IdError::AgentForm(s.to_owned()))?;
        Self::new(CompanyId::new(company)?, name)
    }
}

impl TryFrom<String> for AgentId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<AgentId> for String {
    fn from(id: AgentId) -> Self {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_and_agent_ids_round_trip() {
        let a = AgentId::new(CompanyId::agentrix(), AgentId::RECEIVABLES).unwrap();
        assert_eq!(a.to_string(), "agentrix/receivables");
        assert_eq!(a.user_id().as_str(), "agent:agentrix/receivables");
        assert_eq!(a.company().app_name().as_str(), "agentrix");
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, "\"agentrix/receivables\"");
        assert_eq!(serde_json::from_str::<AgentId>(&json).unwrap(), a);
        assert_eq!(
            "kilele-bakery/payables"
                .parse::<AgentId>()
                .unwrap()
                .company(),
            &CompanyId::kilele()
        );
    }

    #[test]
    fn ids_must_be_slugs() {
        assert!(CompanyId::new("Agentrix").is_err());
        assert!(CompanyId::new("").is_err());
        assert!(CompanyId::new("9lives").is_err());
        assert!("agentrix".parse::<AgentId>().is_err());
        assert!("agentrix/Chief".parse::<AgentId>().is_err());
        assert!(serde_json::from_str::<CompanyId>("\"has space\"").is_err());
    }
}
