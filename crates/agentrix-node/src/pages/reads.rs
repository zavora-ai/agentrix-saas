//! What the console pages read: the port between `pages/` and the stores.
//!
//! The handlers never touch the database. [`super::store::SqliteReads`] implements
//! [`ConsoleReads`] over the control database, and `app.rs` fills its
//! [`super::store::Delegates`] (the session, the approval ceremony, frames from the CAS). Rows
//! reuse `agentrix-types` wherever a type exists (receipts, cards, approvals, UI events, the
//! stored charter, brief and gauntlet shapes), so a page and the live stream it continues are
//! built from the same values.

use agentrix_types::ui::{
    CardEvent, FrameEvent, PageEvent, ParityEvent, QuarantinedEvent, StepEvent, TickEvent,
};
use agentrix_types::{
    Approval, DecisionCard, DecisionId, Digest, KeySet, MoveId, QuarantineId, Receipt, SiteId,
};
pub use agentrix_types::{Brief, BriefLine, GauntletAttack, JobLevel, Level};
use async_trait::async_trait;
use axum::http::HeaderMap;
use bytes::Bytes;
use chrono::{DateTime, Utc};

/// Reads fail with a plain error; pages show a generic "try again" and log the detail.
pub type ReadResult<T> = anyhow::Result<T>;

/// The signed-in owner, from the session cookie (`auth/`).
#[derive(Debug, Clone)]
pub struct OwnerSession {
    /// The account id.
    pub account: String,
    /// The owner's first name, for greetings.
    pub name: String,
    /// The site this session acts on.
    pub site: SiteId,
    /// Whether the account has a passkey; without one, approvals use an authenticator code.
    pub has_passkey: bool,
}

/// Where a site is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteStage {
    /// Assessed and claimed (or claiming); no Move yet.
    Setup,
    /// A Move is running or waiting for the go-live decision.
    Moving,
    /// Serving a signed release.
    Live,
}

/// A site.
#[derive(Debug, Clone)]
pub struct SiteRow {
    /// The site's id.
    pub id: SiteId,
    /// Its public host, e.g. `kilele-bakery.example`.
    pub host: String,
    /// The business name.
    pub name: String,
    /// True for Zavora's fixture site (R3.3): every page labels it.
    pub fixture: bool,
    /// Where it is.
    pub stage: SiteStage,
    /// When it went live, if it has.
    pub live_since: Option<DateTime<Utc>>,
    /// The owner's UTC offset in minutes, for times in plain words (Nairobi is +180).
    pub utc_offset_minutes: i32,
}

/// Where a Move is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStage {
    /// Steps are running.
    Running,
    /// The go-live card waits for the owner.
    Waiting,
    /// The owner approved and the release is live.
    Live,
    /// Stopped by a failure or by the owner.
    Stopped,
}

/// The latest Move of a site: the same values a `snapshot` UI event carries, plus the lists.
#[derive(Debug, Clone)]
pub struct MoveRow {
    /// The Move's id.
    pub id: MoveId,
    /// Where it is.
    pub stage: MoveStage,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// Each step's latest event, in order (R6.5).
    pub steps: Vec<StepEvent>,
    /// The latest page count, while pages are read or rebuilt.
    pub pages: Option<PageEvent>,
    /// The current room's receipt, cumulative.
    pub tick: TickEvent,
    /// The latest frame from the sealed browser.
    pub last_frame: Option<FrameEvent>,
    /// The site-wide parity score (`page` is `None`).
    pub parity_site: Option<ParityEvent>,
    /// Each page's latest parity event.
    pub parity_pages: Vec<ParityEvent>,
    /// Items left behind so far.
    pub quarantined: Vec<QuarantinedEvent>,
    /// The go-live card, once raised.
    pub go_live: Option<CardEvent>,
}

/// A card's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionState {
    /// Waiting for the owner.
    Waiting,
    /// Approved; `approval` and `receipt` are set.
    Approved,
    /// Declined; `receipt` is set.
    Declined,
    /// Expired unanswered (recorded as expired, never as rejected).
    Expired,
}

/// A decision card and what happened to it.
#[derive(Debug, Clone)]
pub struct DecisionRow {
    /// The card, with its effect and fingerprint.
    pub card: DecisionCard,
    /// Its state.
    pub state: DecisionState,
    /// How it was approved, when it was.
    pub approval: Option<Approval>,
    /// The decision's receipt, once decided.
    pub receipt: Option<Digest>,
}

/// The latest finished gauntlet run, computed from receipts only (R13.2).
#[derive(Debug, Clone)]
pub struct GauntletRow {
    /// When it ran.
    pub run_at: DateTime<Utc>,
    /// The `gauntlet.run` receipt.
    pub receipt: Digest,
    /// Each attack, as stored in `gauntlet_runs.detail`.
    pub attacks: Vec<GauntletAttack>,
}

/// A question a visitor passed to the owner from Ask (R8.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassedRow {
    /// The question, as data.
    pub question: String,
    /// How to reach the visitor, if they left one.
    pub contact: Option<String>,
    /// When it was passed.
    pub at: DateTime<Utc>,
}

/// An export the owner uploaded for the Move (R4.1): the newest of each name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadInfo {
    /// `export.xml`, `dump.sql.gz` or `wp-content.tar.gz`.
    pub name: String,
    /// Bytes.
    pub size: u64,
    /// When it was uploaded.
    pub at: DateTime<Utc>,
}

/// The meter's numbers (R11.3): what the gateway recorded in `usage` this month, and the
/// site's monthly AI budget and currency.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeterRow {
    /// Since the 1st of this month, UTC.
    pub month: crate::db::usage::Spend,
    /// The monthly cap and the currency it is shown in.
    pub budget: agentrix_shell::budget::AiBudget,
}

/// The newest passed questions and how many there are in all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassedRows {
    /// How many in all.
    pub total: u32,
    /// The newest, newest first.
    pub recent: Vec<PassedRow>,
}

/// The Chief WebAgent's latest brief (S2-04), composed from receipts only.
#[derive(Debug, Clone)]
pub struct BriefRow {
    /// When it was written.
    pub written_at: DateTime<Utc>,
    /// The brief, as stored in `briefs.body`.
    pub brief: Brief,
}

/// Where a quarantined item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineStage {
    /// Kept out of the new site.
    LeftBehind,
    /// The owner asked to restore it; the card waits for a signature.
    RestoreAsked,
    /// Restored by a signed decision.
    Restored,
}

/// An item left behind.
#[derive(Debug, Clone)]
pub struct QuarantineRow {
    /// Its id.
    pub id: QuarantineId,
    /// What it is, in plain words.
    pub title: String,
    /// Why it was left behind.
    pub plain: String,
    /// Where it was found.
    pub location: String,
    /// Evidence in the CAS.
    pub evidence: Vec<Digest>,
    /// Where it is.
    pub stage: QuarantineStage,
    /// The restore card, once asked.
    pub decision: Option<DecisionId>,
}

/// A frame's bytes from the CAS.
#[derive(Debug, Clone)]
pub struct FrameBytes {
    /// The image.
    pub bytes: Bytes,
    /// Its media type, e.g. `image/webp`.
    pub content_type: String,
}

/// Everything the console pages read.
#[async_trait]
pub trait ConsoleReads: Send + Sync + 'static {
    /// The signed-in owner, or `None` (pages then redirect to sign-in).
    async fn session(&self, headers: &HeaderMap) -> ReadResult<Option<OwnerSession>>;
    /// Where someone who is signed in but owns no site yet goes instead (their claim, or
    /// Assess). `None` when not signed in: pages then redirect to sign-in.
    async fn landing(&self, _headers: &HeaderMap) -> ReadResult<Option<String>> {
        Ok(None)
    }
    /// A site.
    async fn site(&self, id: &SiteId) -> ReadResult<Option<SiteRow>>;
    /// The site's latest Move, if one has started.
    async fn latest_move(&self, site: &SiteId) -> ReadResult<Option<MoveRow>>;
    /// What a Move's model calls have cost, in words (S2-11). `None` when unknown.
    async fn move_spend(&self, _move_id: &MoveId) -> ReadResult<Option<String>> {
        Ok(None)
    }
    /// The site's cards, newest first.
    async fn decisions(&self, site: &SiteId) -> ReadResult<Vec<DecisionRow>>;
    /// One card of the site.
    async fn decision(&self, site: &SiteId, id: &DecisionId) -> ReadResult<Option<DecisionRow>>;
    /// Starts the approval ceremony for `card` (design D9: challenge = SHA-256(fingerprint ‖
    /// nonce)) and returns its options as JSON, or `None` when the owner has no passkey.
    /// Implemented by `decisions/`, which keeps the ceremony state.
    async fn begin_approval(
        &self,
        session: &OwnerSession,
        card: &DecisionCard,
    ) -> ReadResult<Option<String>>;
    /// The signed charter's levels and ceilings, per job, in charter order.
    async fn levels(&self, site: &SiteId) -> ReadResult<Vec<JobLevel>>;
    /// The site's receipts, newest first.
    async fn receipts(&self, site: &SiteId, limit: usize) -> ReadResult<Vec<Receipt>>;
    /// A receipt by its content address.
    async fn receipt(&self, digest: &Digest) -> ReadResult<Option<Receipt>>;
    /// The latest finished gauntlet run against this deployment.
    async fn gauntlet(&self, site: &SiteId) -> ReadResult<Option<GauntletRow>>;
    /// The latest brief.
    async fn brief(&self, site: &SiteId) -> ReadResult<Option<BriefRow>>;
    /// Questions visitors passed to the owner from Ask (R8.3): the newest and the total.
    async fn passed_questions(&self, _site: &SiteId) -> ReadResult<PassedRows> {
        Ok(PassedRows::default())
    }
    /// The exports uploaded for the site's Move, newest of each name (R4.1).
    async fn uploads(&self, _site: &SiteId) -> ReadResult<Vec<UploadInfo>> {
        Ok(Vec::new())
    }
    /// Model spend this month against the monthly AI budget (R11.3). `None` when no model call
    /// was ever made for the site.
    async fn meter(&self, _site: &SiteId) -> ReadResult<Option<MeterRow>> {
        Ok(None)
    }
    /// Pictures the Move's fill drafted for pages that had none, waiting for the owner in W11's
    /// Pictures (handover-wiring 1.3).
    async fn pictures_waiting(&self, _site: &SiteId) -> ReadResult<i64> {
        Ok(0)
    }
    /// Items left behind.
    async fn quarantine(&self, site: &SiteId) -> ReadResult<Vec<QuarantineRow>>;
    /// A frame from the CAS, if the viewer may see it (design §4.1: "Owner, public during
    /// Assess"). `owner_site` is the signed-in owner's site, or `None` for anyone else.
    /// An owner sees their own site's frames; anyone sees frames of a site with no owner yet,
    /// which is a free Assess. Never another owner's frames, even by digest (the site wall,
    /// design §7). `None` for anything else.
    async fn frame(
        &self,
        owner_site: Option<&SiteId>,
        digest: &Digest,
    ) -> ReadResult<Option<FrameBytes>>;
    /// Control's published keys, to verify receipts.
    async fn keys(&self) -> ReadResult<KeySet>;
}
