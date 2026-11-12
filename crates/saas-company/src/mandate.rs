//! Where the founder's mandate comes from.
//!
//! - [`DecisionsMandates`]: the mandate card in agentrix-decisions, signed with the founder's
//!   passkey ([`agentrix_decisions::Decisions::signed_mandate`]). A card approved any other way
//!   is not a signature and is ignored.
//! - [`LocalMandates`]: terms the operator's own key file approves, for local sandbox runs and
//!   judge mode where no phone is at hand. Marked `local-owner-key` wherever it shows (the feed,
//!   the audit log), never as a passkey, as saas-settle marks its sandbox mandates.
//!
//! [`crate::company::Company::load_mandate`] puts the newest signed terms in force; with none,
//! every money tool is denied.

use std::collections::BTreeMap;
use std::sync::Arc;

use agentrix_decisions::{Decisions, MandateTerms};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How a mandate was signed: with the founder's passkey.
pub const PASSKEY: &str = "passkey";
/// How a mandate was signed: with the operator's key file (local sandbox runs, judge mode).
pub const LOCAL_OWNER_KEY: &str = "local-owner-key";

/// Signed terms, with how and where they were signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedTerms {
    /// The terms.
    pub terms: MandateTerms,
    /// [`PASSKEY`] or [`LOCAL_OWNER_KEY`].
    pub signed_with: String,
    /// The card id, or the file it came from.
    pub reference: String,
    /// When.
    pub signed_at: DateTime<Utc>,
}

/// Where a company's mandate comes from.
#[async_trait]
pub trait MandateSource: Send + Sync {
    /// The newest signed mandate of `company`, if any.
    async fn signed(&self, company: &str) -> anyhow::Result<Option<SignedTerms>>;
}

/// The passkey-signed mandate cards in agentrix-decisions.
pub struct DecisionsMandates {
    decisions: Arc<Decisions>,
    sites: BTreeMap<String, String>,
}

impl std::fmt::Debug for DecisionsMandates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionsMandates")
            .field("sites", &self.sites)
            .finish_non_exhaustive()
    }
}

impl DecisionsMandates {
    /// Over `decisions`; `sites` maps a company to the decisions site its cards are on.
    pub fn new(decisions: Arc<Decisions>, sites: BTreeMap<String, String>) -> Self {
        Self { decisions, sites }
    }

    /// The decisions site of `company` (`saas-<company>` unless mapped).
    pub fn site(&self, company: &str) -> String {
        self.sites
            .get(company)
            .cloned()
            .unwrap_or_else(|| format!("saas-{company}"))
    }

    /// The decisions.
    pub fn decisions(&self) -> &Arc<Decisions> {
        &self.decisions
    }
}

#[async_trait]
impl MandateSource for DecisionsMandates {
    async fn signed(&self, company: &str) -> anyhow::Result<Option<SignedTerms>> {
        Ok(self
            .decisions
            .signed_mandate(&self.site(company))
            .await?
            .map(|m| SignedTerms {
                terms: m.terms,
                signed_with: PASSKEY.into(),
                reference: m.card_id,
                signed_at: m.signed_at,
            }))
    }
}

/// Mandates the operator's key file approves: local sandbox runs and judge mode.
#[derive(Debug, Clone, Default)]
pub struct LocalMandates {
    by_company: BTreeMap<String, SignedTerms>,
}

impl LocalMandates {
    /// None yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// `terms` for `company`, approved by `operator` from `reference` (the file).
    pub fn with(mut self, company: &str, terms: MandateTerms, reference: &str) -> Self {
        self.by_company.insert(
            company.to_owned(),
            SignedTerms {
                terms,
                signed_with: LOCAL_OWNER_KEY.into(),
                reference: reference.to_owned(),
                signed_at: Utc::now(),
            },
        );
        self
    }
}

#[async_trait]
impl MandateSource for LocalMandates {
    async fn signed(&self, company: &str) -> anyhow::Result<Option<SignedTerms>> {
        Ok(self.by_company.get(company).cloned())
    }
}
