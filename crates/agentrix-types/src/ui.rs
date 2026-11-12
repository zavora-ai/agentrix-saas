//! UI events for SSE (design §4.3a, schema approved 7 Nov).
//!
//! `/assess/{id}/events` and `/moves/{id}/events` send each [`UiEnvelope`] as one SSE message:
//! `id:` is `seq`, `event:` is the variant's kebab-case name, and `data:` is the payload's
//! JSON. Every payload carries `at`. Node persists `(seq, name, data)` and replays from
//! `Last-Event-ID`, or sends a `snapshot`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::decision::CardKind;
use crate::digest::Digest;
use crate::error::Result;
use crate::finding::{Category, Finding, Severity};
use crate::ids::{DecisionId, FindingId, JobId, QuarantineId};

/// The longest a step summary may be, in characters.
pub const SUMMARY_MAX: usize = 140;

/// A Move or assessment step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepName {
    /// The free assessment.
    Assess,
    /// Proving ownership.
    Claim,
    /// Reading the old site inside a sealed room.
    Replay,
    /// Sanitise, detect and quarantine.
    Clean,
    /// Rebuilding the pages.
    Rebuild,
    /// Rendering and checking parity.
    Prove,
    /// Waiting for, and doing, go-live.
    #[serde(rename = "golive")]
    GoLive,
}

/// Where a step is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    /// Not started.
    Pending,
    /// Working.
    Running,
    /// Waiting on the owner.
    Waiting,
    /// Finished.
    Done,
    /// Stopped with an error.
    Failed,
}

/// `step`: a Move or assessment step changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepEvent {
    /// Which step.
    pub step: StepName,
    /// Where it is.
    pub state: StepState,
    /// Time since the step started.
    pub elapsed_ms: u64,
    /// Plain words, at most [`SUMMARY_MAX`] characters.
    pub summary: String,
    /// The step's receipt, once there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Digest>,
    /// When.
    pub at: DateTime<Utc>,
}

impl StepEvent {
    /// A step event, with the summary clipped to [`SUMMARY_MAX`] characters.
    pub fn new(
        step: StepName,
        state: StepState,
        elapsed_ms: u64,
        summary: &str,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            step,
            state,
            elapsed_ms,
            summary: clip(summary, SUMMARY_MAX),
            receipt: None,
            at,
        }
    }
}

/// `receipt-tick`: cumulative totals for the live receipt, never deltas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickEvent {
    /// The job these totals come from, if one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobId>,
    /// Hosts contacted.
    pub hosts: u64,
    /// Requests allowed.
    pub requests: u64,
    /// Bytes received.
    pub bytes: u64,
    /// Requests refused.
    pub refusals: u64,
    /// When.
    pub at: DateTime<Utc>,
}

/// Whether a frame is live or a sealed screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameKind {
    /// A live frame; only these may be labelled "Live".
    Live,
    /// A screenshot from a sealed room, about one a second.
    SealedScreenshot,
}

/// `frame`: a screenshot to show. The stream carries no data URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameEvent {
    /// Live or sealed screenshot.
    pub kind: FrameKind,
    /// The page shown.
    pub page_url: String,
    /// The frame's digest.
    pub digest: Digest,
    /// Where the browser fetches it: `/frames/{digest}`.
    pub src: String,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
    /// When.
    pub at: DateTime<Utc>,
}

impl FrameEvent {
    /// A frame event, with `src` derived from the digest.
    pub fn new(
        kind: FrameKind,
        page_url: impl Into<String>,
        digest: Digest,
        w: u32,
        h: u32,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            kind,
            page_url: page_url.into(),
            src: format!("/frames/{digest}"),
            digest,
            w,
            h,
            at,
        }
    }
}

/// What happened to one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageStatus {
    /// Read.
    Read,
    /// Refused by policy.
    Refused,
    /// Failed to load.
    Failed,
}

/// `page`: progress through the site's pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageEvent {
    /// This page's number, from 1.
    pub n: u32,
    /// Pages in all.
    pub of: u32,
    /// The page.
    pub url: String,
    /// What happened.
    pub status: PageStatus,
    /// Why, when refused or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When.
    pub at: DateTime<Utc>,
}

/// `finding`: a detector raised a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingEvent {
    /// The finding.
    pub id: FindingId,
    /// How much it matters.
    pub severity: Severity,
    /// What it is about.
    pub category: Category,
    /// Plain words.
    pub title: String,
    /// Where it was found.
    #[serde(rename = "where")]
    pub location: String,
    /// When.
    pub at: DateTime<Utc>,
}

impl From<&Finding> for FindingEvent {
    fn from(f: &Finding) -> Self {
        Self {
            id: f.id.clone(),
            severity: f.severity,
            category: f.category,
            title: f.title.clone(),
            location: f.location.clone(),
            at: f.at,
        }
    }
}

/// `quarantined`: an item was set aside.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantinedEvent {
    /// The item.
    pub item_id: QuarantineId,
    /// Plain words.
    pub title: String,
    /// Why.
    pub reason: String,
    /// Where it was found.
    #[serde(rename = "where")]
    pub location: String,
    /// When.
    pub at: DateTime<Utc>,
}

/// Where a parity check stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParityState {
    /// Met the bar.
    Pass,
    /// Rebuilding and trying again.
    Retry,
    /// Out of tries; flagged for the owner.
    Flagged,
}

/// `parity`: a parity verdict for one page, or the whole site.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityEvent {
    /// The page, or `null` for the whole site.
    pub page: Option<String>,
    /// Score from 0 to 1.
    pub score: f64,
    /// The model's verdict, in plain words.
    pub verdict: String,
    /// Where the check stands.
    pub state: ParityState,
    /// Which try this is, from 1.
    pub attempt: u32,
    /// When.
    pub at: DateTime<Utc>,
}

/// `card`: a decision card was raised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardEvent {
    /// The decision.
    pub decision_id: DecisionId,
    /// Which card.
    pub kind: CardKind,
    /// Plain words.
    pub title: String,
    /// Where to open it.
    pub href: String,
    /// When.
    pub at: DateTime<Utc>,
}

/// How a stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DoneOutcome {
    /// The assessment report is ready.
    ReportReady,
    /// The site could not be reached.
    Unreachable,
    /// A candidate release is ready for the owner.
    CandidateReady,
    /// The site is live.
    Live,
    /// The owner or the system stopped it.
    Stopped,
}

/// `done`: the stream's work finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneEvent {
    /// How it ended.
    pub outcome: DoneOutcome,
    /// Where to go next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    /// When.
    pub at: DateTime<Utc>,
}

/// `error`: something went wrong, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// The step it happened in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<StepName>,
    /// Plain words, never a trace.
    pub message: String,
    /// Whether it is being retried.
    pub retrying: bool,
    /// When.
    pub at: DateTime<Utc>,
}

/// `snapshot`: current state, for a client that connects without a usable `Last-Event-ID`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEvent {
    /// Every step's latest event.
    pub steps: Vec<StepEvent>,
    /// The latest totals.
    pub tick: TickEvent,
    /// The latest frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame: Option<FrameEvent>,
    /// The latest parity verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parity: Option<ParityEvent>,
    /// When.
    pub at: DateTime<Utc>,
}

/// One UI event, written `{"event": "<kebab-case name>", "data": {…}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "kebab-case")]
pub enum UiEvent {
    /// A step changed.
    Step(StepEvent),
    /// Live receipt totals.
    ReceiptTick(TickEvent),
    /// A screenshot.
    Frame(FrameEvent),
    /// Page progress.
    Page(PageEvent),
    /// A finding.
    Finding(FindingEvent),
    /// An item was quarantined.
    Quarantined(QuarantinedEvent),
    /// A parity verdict.
    Parity(ParityEvent),
    /// A decision card.
    Card(CardEvent),
    /// The work finished.
    Done(DoneEvent),
    /// Something went wrong.
    Error(ErrorEvent),
    /// Current state.
    Snapshot(SnapshotEvent),
}

impl UiEvent {
    /// The SSE `event:` name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Step(_) => "step",
            Self::ReceiptTick(_) => "receipt-tick",
            Self::Frame(_) => "frame",
            Self::Page(_) => "page",
            Self::Finding(_) => "finding",
            Self::Quarantined(_) => "quarantined",
            Self::Parity(_) => "parity",
            Self::Card(_) => "card",
            Self::Done(_) => "done",
            Self::Error(_) => "error",
            Self::Snapshot(_) => "snapshot",
        }
    }

    /// The SSE `data:` JSON: the payload alone, on one line.
    pub fn data_json(&self) -> Result<String> {
        let mut value = serde_json::to_value(self)?;
        Ok(serde_json::to_string(&value["data"].take())?)
    }

    /// Rebuilds an event from a persisted name and payload, for `Last-Event-ID` replay.
    pub fn from_parts(name: &str, data: &str) -> Result<Self> {
        let data: serde_json::Value = serde_json::from_str(data)?;
        Ok(serde_json::from_value(
            serde_json::json!({ "event": name, "data": data }),
        )?)
    }
}

/// A UI event with its sequence number, monotonic per assessment or Move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiEnvelope {
    /// The SSE `id:`.
    pub seq: u64,
    /// The event.
    #[serde(flatten)]
    pub event: UiEvent,
}

impl UiEnvelope {
    /// The event as one SSE message, ending with the blank line.
    pub fn to_sse(&self) -> Result<String> {
        Ok(format!(
            "id: {}\nevent: {}\ndata: {}\n\n",
            self.seq,
            self.event.name(),
            self.event.data_json()?
        ))
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}
