//! What the shell reads: the port between the shell's pages and the platform.
//!
//! `agentrix-node` implements [`ShellPort`] over the first release's console reads (`pages::ConsoleReads`
//! and `pages::map`), the approval ceremony (`auth::Approvals` through `decisions::Approver`)
//! and the receipts store. Wherever the first release already builds a view model, the port hands that
//! model back (`agentrix_console::view`), so a page in the shell shows exactly what the first release's
//! page showed, from the same reads. Values the shell needs that the first release never built are small
//! rows defined here.

use agentrix_console::view as cv;
use agentrix_types::{ApprovalMethod, Digest, ReceiptBody};
use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

/// Reads fail with a plain error; pages show a generic "try again" and log the detail.
pub type PortResult<T> = anyhow::Result<T>;

/// The site the owner is looking at.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SiteInfo {
    /// The site's id, as used in `/console/{site}`.
    pub id: String,
    /// The business name.
    pub name: String,
    /// Its public host.
    pub host: String,
    /// "Live since 09:41", "Moving", "In setup".
    pub state_words: String,
    /// The public address once it is live.
    pub live_url: Option<String>,
    /// Zavora's fixture site (R3.3): every page says so.
    pub fixture: bool,
    /// A sample site (`*-sample`): every page says so (voice.md rule 9).
    pub sample: bool,
    /// Whether the site serves a signed release.
    pub live: bool,
    /// The owner's UTC offset in minutes (Nairobi is +180).
    pub utc_offset_minutes: i32,
}

/// The signed-in owner of the site in the path.
#[derive(Debug, Clone, Default)]
pub struct Owner {
    /// The account id.
    pub account: String,
    /// First name, for greetings. May be empty.
    pub name: String,
    /// Whether the account has a passkey; without one, approvals use an authenticator code.
    pub has_passkey: bool,
    /// The site.
    pub site: SiteInfo,
}

/// Who is asking for a page.
#[derive(Debug, Clone)]
pub enum Access {
    /// The owner of the site.
    Owner(Owner),
    /// Not allowed here: go to this path instead (sign-in with `next`, or the owner's own site).
    Redirect(String),
    /// No such site for this person.
    NotFound,
}

/// A decision card as the lane and the list show it, from the first release's cards or card v2.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CardRow {
    /// The card's id.
    pub id: String,
    /// "go-live", "quarantine-restore", "order-held", or a v2 kind such as "site.unfreeze".
    pub kind: String,
    /// Plain words: what will happen.
    pub title: String,
    /// One line: what changes, for whom, how to undo.
    pub summary: String,
    /// The WebAgent that raised it.
    pub asked_by: cv::AgentMark,
    /// Low, medium or high.
    pub risk: cv::Risk,
    /// "Low risk: …" in the owner's words, never a bare score.
    pub risk_words: String,
    /// When it was raised.
    pub created_at: DateTime<Utc>,
    /// After this it can't be approved, and it is recorded as expired, never as declined.
    pub expires_at: DateTime<Utc>,
    /// Where it is.
    pub state: cv::CardState,
    /// For decided cards: "Approved with your passkey".
    pub outcome_words: String,
    /// The card page.
    pub href: String,
    /// The frozen approve contract (design §4.1) for this card.
    pub approve_url: String,
    /// Declines it.
    pub decline_url: String,
}

/// What a v2 card carries besides the first release's card view: the fields the owner may edit,
/// the owner's edit or decline reason once decided, and the raising WebAgent's standing rules.
#[derive(Debug, Clone, Default)]
pub struct CardExtras {
    /// The WebAgent that raised it (`cashier`).
    pub webagent: String,
    /// The fields the owner may change, with their values as raised: as the form holds them
    /// ("4500") and in the owner's words ("KSh 4,500").
    pub editable: Vec<(agentrix_decisions::EditableField, String, String)>,
    /// The owner's edit, once approved edited.
    pub edit: Option<agentrix_decisions::OwnerEdit>,
    /// The owner's reason, once declined with one.
    pub decline_reason: Option<String>,
    /// The raising WebAgent's standing rules.
    pub rules: Vec<agentrix_decisions::Rule>,
    /// "Always allow orders for Amani W., up to KSh 4,500, for 90 days, and tell me", when the
    /// card names a person and may get a standing grant.
    pub allow_person: Option<String>,
    /// The same for every card of its kind from this WebAgent.
    pub allow_kind: Option<String>,
    /// Why this card can't get a standing grant, in the owner's words.
    pub allow_refusal: Option<String>,
    /// The standing grant this card made or was approved under, while it holds.
    pub grant: Option<agentrix_decisions::Grant>,
    /// Whether a standing grant approved it (not the owner).
    pub by_grant: bool,
}

/// Where a Move or another long job is, as the "at work" card shows it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkCard {
    /// Who is working.
    pub who: cv::AgentMark,
    /// "The Move · kilele-bakery.example".
    pub title: String,
    /// The current step in the tool's own words.
    pub state_words: String,
    /// "working", "waiting" or "done", for the state line.
    pub state: String,
    /// The steps, each "done", "now" or "todo".
    pub steps: Vec<Step>,
    /// Where to watch it.
    pub href: String,
}

/// One step of a [`WorkCard`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct Step {
    /// What it does.
    pub text: String,
    /// "done", "now" or "todo".
    pub state: String,
}

/// Something recorded: one receipt, for the night's timeline and "while you were away".
#[derive(Debug, Clone, Serialize)]
pub struct Happened {
    /// When it finished.
    pub at: DateTime<Utc>,
    /// The receipt's title, in plain words.
    pub title: String,
    /// The receipt kind, such as `decision.approved`.
    pub kind: String,
    /// The receipt's digest.
    pub digest: String,
    /// Whether its signature checks against the published key.
    pub verified: bool,
    /// True when the receipt records a failure or a refusal.
    pub failed: bool,
}

/// A signed release of the site, for the Website page (01b C23).
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseRow {
    /// The release number: v1 is the go-live.
    pub seq: i64,
    /// `live`, `superseded`, `rolled-back` or `candidate`.
    pub state: String,
    /// Whether the site serves it now.
    pub live: bool,
    /// When it was signed, if it was.
    pub signed_at: Option<DateTime<Utc>>,
    /// How it was approved: `passkey`, `totp`, or `None` (a Guardian rollback, a candidate).
    pub method: Option<String>,
    /// The card that approved it, if one did.
    pub decision: Option<String>,
}

/// How the owner tried to approve (the frozen approve contract's two methods).
#[derive(Debug, Clone, PartialEq)]
pub enum Attempt {
    /// A webauthn-rs `PublicKeyCredential`, as JSON.
    Passkey(Value),
    /// Six digits from the owner's authenticator app.
    Totp(String),
}

/// A verified approval.
#[derive(Debug, Clone, PartialEq)]
pub struct Proof {
    /// How.
    pub method: ApprovalMethod,
    /// The evidence for the receipt.
    pub evidence: Value,
}

/// Everything the shell reads and the two things it asks the platform to do (verify an
/// approval, sign a receipt).
#[async_trait]
pub trait ShellPort: Send + Sync + 'static {
    /// Who is asking for `site` (`None`: whichever site the person owns). `back` is the path
    /// to return to after signing in.
    async fn access(
        &self,
        headers: &HeaderMap,
        site: Option<&str>,
        back: &str,
    ) -> PortResult<Access>;

    /// The first release's Today: the brief, questions visitors passed, model spend, recent receipts.
    async fn today(&self, owner: &Owner) -> PortResult<cv::TodayView>;
    /// Every card of the site, newest first.
    async fn cards(&self, owner: &Owner) -> PortResult<Vec<CardRow>>;
    /// One card, with a fresh passkey ceremony when `ceremony` is true and it is waiting.
    async fn card(&self, owner: &Owner, id: &str, ceremony: bool) -> PortResult<Option<cv::Card>>;
    /// What a v2 card carries besides its view ([`CardExtras`]); `None` for the first release's
    /// cards and where card v2 isn't wired.
    async fn card_extras(&self, _owner: &Owner, _id: &str) -> PortResult<Option<CardExtras>> {
        Ok(None)
    }
    /// The site's standing rules, newest first; `webagent` narrows them to one WebAgent.
    async fn rules(
        &self,
        _owner: &Owner,
        _webagent: Option<&str>,
    ) -> PortResult<Vec<agentrix_decisions::Rule>> {
        Ok(Vec::new())
    }
    /// The site's standing grants that the owner hasn't stopped, newest first.
    async fn grants(
        &self,
        _owner: &Owner,
        _webagent: Option<&str>,
    ) -> PortResult<Vec<agentrix_decisions::Grant>> {
        Ok(Vec::new())
    }
    /// Cards approved under a standing grant since `since`, newest first: Today tells the owner.
    async fn granted(
        &self,
        _owner: &Owner,
        _since: DateTime<Utc>,
    ) -> PortResult<Vec<agentrix_decisions::CardRow>> {
        Ok(Vec::new())
    }
    /// The site's WebAgents and their jobs' levels.
    async fn webagents(&self, owner: &Owner) -> PortResult<cv::WebAgentsView>;
    /// Receipts and the gauntlet score.
    async fn trust(&self, owner: &Owner) -> PortResult<cv::TrustView>;
    /// Items left behind by the Move.
    async fn quarantine(&self, owner: &Owner) -> PortResult<cv::QuarantineView>;
    /// The site's WebAgents from W1's roster (marks, mandates, jobs, who is on duty), or `None`
    /// when the kernel has nothing for the site; pages then fall back to the charter's levels.
    async fn roster(&self, _owner: &Owner) -> PortResult<Option<agentrix_kernel::Roster>> {
        Ok(None)
    }
    /// Raises a card v2 for the owner to decide (the Store's "Add"); returns its id.
    async fn raise(
        &self,
        _owner: &Owner,
        _card: agentrix_decisions::NewCard,
    ) -> PortResult<String> {
        anyhow::bail!("raising cards isn't available here")
    }
    /// The Chief WebAgent's draft of a mission contract from the owner's words (07 §3.5), or
    /// `None` where no model is wired, so the owner fills the contract in. `Err` when the model
    /// was asked and didn't answer. The page checks the draft ([`crate::missions::settle`]).
    async fn draft_mission(
        &self,
        _owner: &Owner,
        _ask: &crate::missions::DraftAsk,
    ) -> PortResult<Option<crate::missions::RawDraft>> {
        Ok(None)
    }
    /// The site's releases, newest first.
    async fn releases(&self, _owner: &Owner) -> PortResult<Vec<ReleaseRow>> {
        Ok(Vec::new())
    }
    /// Model spend this month against the monthly AI budget, in the site's currency, for the
    /// budget pill on every page ([`crate::budget`]). `None` before the first model call.
    async fn meter(&self, owner: &Owner) -> PortResult<Option<cv::Meter>>;
    /// The latest gauntlet run, for the trust pill on every page.
    async fn gauntlet(&self, owner: &Owner) -> PortResult<Option<cv::Gauntlet>>;
    /// The Move, while one is running or waiting.
    async fn work(&self, _owner: &Owner) -> PortResult<Vec<WorkCard>> {
        Ok(Vec::new())
    }
    /// Receipts that finished in `[from, to)`, oldest first.
    async fn happened(
        &self,
        owner: &Owner,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> PortResult<Vec<Happened>>;
    /// Model spend since `since` in the site's currency ("KSh 54"), `None` if nothing.
    async fn spend_since(
        &self,
        _owner: &Owner,
        _since: DateTime<Utc>,
    ) -> PortResult<Option<String>> {
        Ok(None)
    }

    /// Passkey options bound to `fingerprint` (challenge = SHA-256(fingerprint ‖ nonce); D9),
    /// as JSON, or `None` when the owner has no passkey.
    async fn passkey_options(
        &self,
        owner: &Owner,
        fingerprint: &Digest,
    ) -> PortResult<Option<String>>;
    /// Checks an approval over `fingerprint`. `Err` carries a plain-words reason for the owner.
    async fn verify(
        &self,
        owner: &Owner,
        fingerprint: &Digest,
        attempt: Attempt,
    ) -> Result<Proof, String>;
    /// Signs and stores a receipt; returns its digest.
    async fn issue(&self, body: ReceiptBody, title: String) -> PortResult<Digest>;
}
