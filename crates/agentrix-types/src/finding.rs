//! Findings from the detectors (R1, R5; design §5 `findings`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::digest::Digest;
use crate::ids::{FindingId, SiteId};

/// How much a finding matters, in the console's words (design §4.3a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Act now.
    Urgent,
    /// Worth fixing.
    Warn,
    /// For information.
    Info,
}

/// What a finding is about (design §4.3a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Malware, spam, injected scripts, exposed secrets.
    Security,
    /// Page weight and load time.
    Speed,
    /// Broken or hijacked links.
    Links,
    /// How well AI agents can read and act on the site.
    AiReadiness,
}

/// One finding: a detector's verdict on one item, with evidence by digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The finding.
    pub id: FindingId,
    /// The site.
    pub site: SiteId,
    /// The detector that raised it.
    pub detector: String,
    /// How much it matters.
    pub severity: Severity,
    /// What it is about.
    pub category: Category,
    /// Plain words, for the owner.
    pub title: String,
    /// Plain words: why it matters and what the Move does about it.
    #[serde(default)]
    pub explanation: String,
    /// Where it was found: a page URL, a file or a database item.
    #[serde(rename = "where")]
    pub location: String,
    /// Evidence in the CAS.
    #[serde(default)]
    pub evidence: Vec<Digest>,
    /// When it was raised.
    pub at: DateTime<Utc>,
}
