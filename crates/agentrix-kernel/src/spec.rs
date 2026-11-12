//! `WebAgentSpec`: one WebAgent as data (05 §2.1, "The roster as data").
//!
//! The console's WebAgent card, the Store listing (can, never, wakes, jobs, budget) and the
//! dial all render from a spec, never from hand-written copy (07 §3.4). Levels are the first release's
//! [`Level`], the owner's words for `awp_types::AutonomyLevel` (05 §2.3), so the charters
//! owners already signed overlay a spec without translation.

use agentrix_marks::MarkSpec;
use serde::{Deserialize, Serialize};

pub use agentrix_types::Level;

/// A dial position in the owner's words (05 §2.3; the console's ladder).
pub fn level_words(level: Level) -> &'static str {
    match level {
        Level::Watch => "Watch",
        Level::Suggest => "Suggest",
        Level::AskFirst => "Ask me first",
        Level::DoAndTell => "Do it and tell me",
        Level::WithinBudget => "Handle it within budget",
        Level::UndoOnly => "Undo only",
    }
}

/// The current spec version.
pub const WEBAGENT_SPEC_VERSION: u32 = 1;

/// One WebAgent of one site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebAgentSpec {
    /// The spec version, `1`.
    pub spec: u32,
    /// The principal suffix: `urn:adk:agent:{site}/{id}` (05 §3.1). Never changes.
    pub id: String,
    /// The owner-facing name: "Chief WebAgent", "Concierge". Never a human name.
    pub name: String,
    /// One line under the name: what it looks after.
    pub role_words: String,
    /// Its duty, with limits (01 §4.2). "Keep every page true", not "maximise sales".
    pub mandate: String,
    /// What it never does without the owner, in plain words.
    pub never: Vec<String>,
    /// Public desks read untrusted text; the back office never does (ADR-011).
    pub partition: Partition,
    /// The `adk-website` role it is built from (05 §2.1), until the roster is data upstream (U1).
    pub base_role: String,
    /// How the owner controls it.
    pub dial: DialKind,
    /// Its jobs: one dial each.
    pub jobs: Vec<JobSpec>,
    /// What wakes it.
    pub wakes: Vec<Wake>,
    /// Its share of the site's monthly AI budget. Above it, it sleeps until the owner raises it.
    pub budget: Budget,
    /// Effects Gateway connections it needs (`paypal`, `email`), never their credentials.
    pub connections: Vec<String>,
    /// Its look.
    pub mark: MarkSpec,
    /// The Concierge's desks; empty for every other WebAgent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub desks: Vec<DeskSpec>,
}

impl WebAgentSpec {
    /// Its principal on `site`.
    pub fn principal(&self, site: &str) -> String {
        format!("urn:adk:agent:{site}/{}", self.id)
    }

    /// A job by id.
    pub fn job(&self, id: &str) -> Option<&JobSpec> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Everything it can do, with the effect each carries, for the Store's "Can" list.
    pub fn can(&self) -> impl Iterator<Item = &Capability> {
        self.jobs.iter().flat_map(|j| j.can.iter())
    }
}

/// Which side of the wall a WebAgent works on (ADR-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Partition {
    /// Reads visitors, email and other agents; holds no privileged tool.
    Public,
    /// Acts for the owner; reads only typed signals.
    BackOffice,
}

/// How the owner controls a WebAgent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DialKind {
    /// One ladder per job.
    Ladder,
    /// Public desks: the owner's fixed rules, never a dial (05 §2.3, A-I2).
    FixedRules,
    /// The Guardian: undo and the read-only switch only (A-I11).
    UndoOnly,
}

/// What the most consequential tool of a job does (`EffectClass`, `adk-kernel/src/drivers.rs`).
/// It sets the job's ceiling (ADR-012, amendment 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Reads only.
    Read,
    /// Drafts, notes and messages to the owner (`write_local`).
    Draft,
    /// Changes to the site's own existing pages that a signed release reverts.
    SiteEdit,
    /// Rollback to a signed release and the edge's read-only switch.
    Undo,
    /// A new page on the site. Consequential (ADR-014), so it always asks.
    NewPage,
    /// Posts and replies off the site.
    PublishPublic,
    /// Messages to customers or third parties.
    SendExternal,
    /// Meetings and bookings with others.
    ScheduleWithOthers,
    /// Moving money: orders, payments, refunds, renewals.
    Financial,
    /// Deletions.
    Delete,
    /// Anything the catalogue doesn't class.
    Other,
}

impl Effect {
    /// The highest level a job holding this effect may reach without a signed standing rule.
    pub fn ceiling(self) -> Level {
        match self {
            Effect::Read | Effect::Draft | Effect::SiteEdit => Level::WithinBudget,
            Effect::Undo => Level::UndoOnly,
            Effect::NewPage
            | Effect::PublishPublic
            | Effect::SendExternal
            | Effect::ScheduleWithOthers
            | Effect::Financial
            | Effect::Delete
            | Effect::Other => Level::AskFirst,
        }
    }

    /// The tag the Store shows beside a capability.
    pub fn tag(self) -> &'static str {
        match self {
            Effect::Read => "read",
            Effect::Draft => "draft",
            Effect::SiteEdit => "site edit",
            Effect::Undo => "undo",
            Effect::NewPage => "new page",
            Effect::PublishPublic => "publish_public",
            Effect::SendExternal => "send_external",
            Effect::ScheduleWithOthers => "schedule",
            Effect::Financial => "money",
            Effect::Delete => "delete",
            Effect::Other => "other",
        }
    }

    /// Why the ceiling is where it is, in the owner's words. Empty when it can rise to the top.
    pub fn ceiling_reason(self) -> &'static str {
        match self {
            Effect::Read | Effect::Draft | Effect::SiteEdit => "",
            Effect::Undo => {
                "It can only roll back to your last signed release or switch the site to read-only."
            }
            Effect::NewPage => "A new page is public, so every one asks you first.",
            Effect::PublishPublic => {
                "Posting in public can't be taken back, so it always asks you first."
            }
            Effect::SendExternal => {
                "Messages to other people can't be unsent, so it always asks you first."
            }
            Effect::ScheduleWithOthers => {
                "Bookings involve other people's time, so it always asks you first."
            }
            Effect::Financial => "Money stays at Ask me first unless you sign a rule with a limit.",
            Effect::Delete => "Deleting can't be undone, so it always asks you first.",
            Effect::Other => "This kind of work isn't classed yet, so it always asks you first.",
        }
    }
}

/// One job: a unit of authority with its own dial (05 §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    /// The job id: `adk-website`'s (`nurture`, `freshness`) or the platform's (`morning-brief`).
    pub id: String,
    /// What the owner reads: "Follow up leads".
    pub name: String,
    /// Its most consequential effect.
    pub effect: Effect,
    /// Where the dial starts (01 §4.2; charter v1).
    pub start: Level,
    /// The highest the dial may go: the effect's ceiling, or lower.
    pub ceiling: Level,
    /// Why the ceiling is there, in the owner's words. Empty when there is none.
    pub ceiling_reason: String,
    /// One line under the dial.
    pub note: String,
    /// What it can do in this job.
    pub can: Vec<Capability>,
}

/// One thing a WebAgent can do, for the Store's "Can" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Plain words: "Draft follow-ups and quotes".
    pub words: String,
    /// What it carries.
    pub effect: Effect,
}

/// What wakes a WebAgent (05 §3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wake {
    /// The kind of wake.
    pub kind: WakeKind,
    /// The signal, job or schedule key, e.g. `lead_captured` or `morning-brief`.
    pub key: String,
    /// What the owner reads: "Every morning before 07:00".
    pub words: String,
}

/// Kinds of wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeKind {
    /// A typed signal from the public desks.
    Signal,
    /// A schedule, in the site's time zone.
    Schedule,
    /// Someone at the site: a visitor, a customer or another agent.
    Visitor,
    /// The owner, from a paired device.
    Owner,
    /// An order, payment or provider event.
    Event,
}

/// A WebAgent's budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// Percent of the site's monthly AI budget. The amount comes from the site's plan, never
    /// from here, so no figure is invented.
    pub share_percent: u8,
}

/// One of the Concierge's desks: the front desk, the agent door, the inbox, or a section desk
/// for a menu item the owner turned into a desk (05 §4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeskSpec {
    /// `concierge`, `agent-desk`, `inbox` or `desk-{slug}`. Fixed at creation.
    pub id: String,
    /// The menu label: "Cakes".
    pub name: String,
    /// What it answers, from the site's own pages.
    pub mandate: String,
    /// The page the menu item opens, for section desks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Its look; a section desk has its own seed and colour.
    pub mark: MarkSpec,
}
