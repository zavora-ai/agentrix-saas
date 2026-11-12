//! View models: one per [`crate::Page`], with display-ready strings.
//!
//! `agentrix-node` fills these from the database and `agentrix-types`. Templates never format
//! numbers or dates and never print an `Option` directly; every field here is used by a
//! template, and the header comment of each template lists what it reads.
//!
//! Field names are the template contract, so fields are documented by their names and by
//! the templates; types and states are documented here.
#![allow(missing_docs)]

use serde::Serialize;

// ---------- Shared pieces ----------

/// A WebAgent's mark (ADR-031): a squircle in the agent's colour with a role glyph or monogram.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentMark {
    pub name: String,
    /// One or two letters, or a glyph character.
    pub glyph: String,
    /// A CSS colour that keeps 3:1 against the page (the site's colour on public pages).
    pub color: String,
    pub state: MarkState,
    /// The owner's words for the state, e.g. "Checking today's stock". Empty for idle.
    pub state_words: String,
}

/// The presence grammar shared by the site and the console (07 §3.5).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MarkState {
    #[default]
    Idle,
    Listening,
    Working,
    Answering,
    Waiting,
    Resting,
    Paused,
}

/// A label and a value, for definition lists.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

/// A link to a card that needs the owner.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CardLink {
    pub title: String,
    pub href: String,
    pub asked_by: AgentMark,
}

/// The sealed room's receipt, ticking (R1.3). Values are cumulative.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Tick {
    pub hosts: u32,
    pub requests: u32,
    /// "1.4 MB".
    pub bytes_words: String,
    pub refusals: u32,
}

/// A frame from the sealed browser.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Frame {
    pub kind: FrameKind,
    /// A node path such as `/frames/{digest}`, never a data URL.
    pub src: String,
    pub page_url: String,
}

/// Whether a frame is live or a sealed screenshot; the label on screen says which (honesty rule).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FrameKind {
    Live,
    #[default]
    SealedScreenshot,
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Urgent,
    Warn,
    #[default]
    Info,
}

/// One piece of evidence behind a finding or a quarantined item.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Evidence {
    pub label: String,
    pub excerpt: Option<String>,
    pub receipt: Option<String>,
}

/// A finding in plain words, with its evidence (R1.5).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub title: String,
    pub plain: String,
    pub evidence: Vec<Evidence>,
}

/// A receipt in a list.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReceiptRow {
    pub digest: String,
    pub digest_short: String,
    pub title: String,
    pub kind_words: String,
    pub at_words: String,
    pub verified: bool,
}

// ---------- The console chrome ----------

/// The site the console is showing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SiteRef {
    pub id: String,
    pub host: String,
    pub name: String,
    /// "In setup", "Moving", "Live since 7 Nov".
    pub state_words: String,
    /// True for Zavora's fixture site; the pages label it everywhere (R3.3).
    pub fixture: bool,
    /// Where visitors reach the site once a release is live (its platform address).
    pub live_url: Option<String>,
}

/// Which console tab is open.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NavTab {
    #[default]
    Today,
    Decisions,
    Move,
    Trust,
    Webagents,
}

/// The console navigation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Nav {
    pub active: NavTab,
    pub decisions_waiting: u32,
}

/// The signed-in owner.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OwnerRef {
    pub name: String,
}

/// What every console page carries. Flattened into each console view.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Chrome {
    pub site: SiteRef,
    pub nav: Nav,
    pub owner: OwnerRef,
}

// ---------- Public pages ----------

/// [`crate::Page::Start`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct StartView {
    pub error: Option<String>,
    pub url: Option<String>,
    /// A link that starts an assessment of Zavora's fixture, when one is configured.
    pub fixture_url: Option<String>,
}

/// Where an assessment is.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AssessState {
    #[default]
    Running,
    Done,
    Unreachable,
    Refused,
}

/// [`crate::Page::Assess`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct AssessView {
    pub id: String,
    pub url: String,
    pub host: String,
    pub fixture: bool,
    pub state: AssessState,
    /// "14 of 50 pages read".
    pub progress_words: String,
    pub started_words: String,
    pub tick: Tick,
    pub frame: Option<Frame>,
    /// Present when `state` is `Done`.
    pub report: Option<Report>,
    /// Present when `state` is `Unreachable` or `Refused` (R1.6).
    pub unreachable: Option<Unreachable>,
    /// Where "Move this site" leads: sign-in, then the claim (R1.7).
    pub move_href: String,
}

/// The health report (R1.5).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub summary_words: String,
    pub counts: Counts,
    pub sections: Vec<ReportSection>,
    pub pages_read: u32,
    pub receipt: Option<String>,
}

/// Findings by severity.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub urgent: u32,
    pub warn: u32,
    pub info: u32,
}

/// A report section: security, speed, broken links, AI-readiness.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReportSection {
    pub title: String,
    /// Shown when the section has no findings, e.g. "No broken links on the 14 pages we read."
    pub empty_words: String,
    pub findings: Vec<Finding>,
}

/// Why an assessment couldn't finish, and what to do (R1.6). Never invented findings.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Unreachable {
    pub said: String,
    pub next_step: String,
}

/// [`crate::Page::SignIn`] and [`crate::Page::Register`]. The steps after the first (the
/// authenticator code, enrolment) are driven by `static/auth.js` on control-plane's JSON
/// endpoints (S1-07).
#[derive(Debug, Clone, Default, Serialize)]
pub struct AuthView {
    /// Where to go after signing in, e.g. `/os` or the page that asked.
    pub next: String,
    /// Whether passkeys are offered on this deployment.
    pub passkeys: bool,
}

// ---------- Claim and the Move ----------

/// How the owner proves control (R3.1).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClaimMethod {
    #[default]
    Dns,
    File,
}

/// Where a claim is.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ClaimState {
    #[default]
    Pending,
    Checking,
    Verified,
    Failed,
}

/// The claim's details.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Claim {
    pub method: ClaimMethod,
    pub dns_name: String,
    pub dns_value: String,
    pub file_name: String,
    pub file_url: String,
    pub token: String,
    pub state: ClaimState,
    /// "Checked 10:02 from the sandbox".
    pub checked_words: Option<String>,
    /// What the last check saw, in words. Set when `state` is `Failed`.
    pub said: Option<String>,
}

/// [`crate::Page::Claim`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClaimView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub claim: Claim,
}

/// Where the Move is.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MoveState {
    #[default]
    None,
    Running,
    Waiting,
    Live,
    Stopped,
}

/// The Move's steps (R6.5).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StepKey {
    #[default]
    Assess,
    Claim,
    Replay,
    Clean,
    Rebuild,
    Prove,
    Golive,
}

/// A step's state.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    #[default]
    Pending,
    Running,
    Waiting,
    Done,
    Failed,
}

/// The Move itself.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MoveRun {
    pub id: String,
    pub state: MoveState,
    pub started_words: String,
    pub progress_words: Option<String>,
}

/// One step of the Move.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Step {
    pub key: StepKey,
    pub name: String,
    pub state: StepState,
    pub summary: String,
    /// "1 m 48 s"; empty while pending.
    pub took_words: String,
    pub receipt: Option<String>,
}

/// A page's parity state.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ParityState {
    #[default]
    Pass,
    Retry,
    Flagged,
}

/// Parity, site-wide and per page (R6.2).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Parity {
    /// "97%".
    pub score_words: String,
    /// 0–100, for the bar.
    pub score_pct: u8,
    pub pages: Vec<ParityPage>,
}

/// One page's parity.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ParityPage {
    pub url: String,
    pub state: ParityState,
    pub score_words: String,
    /// The vision check's one-line verdict.
    pub verdict: String,
}

/// A quarantined item in the Move summary.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuarantineLine {
    pub title: String,
    pub reason: String,
    #[serde(rename = "where")]
    pub location: String,
}

/// What was left behind, in brief.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuarantineSummary {
    pub count: u32,
    pub items: Vec<QuarantineLine>,
}

/// [`crate::Page::Move`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct MoveView {
    #[serde(flatten)]
    pub chrome: Chrome,
    #[serde(rename = "move")]
    pub run: MoveRun,
    pub steps: Vec<Step>,
    pub tick: Tick,
    pub frame: Option<Frame>,
    pub parity: Option<Parity>,
    pub quarantine: QuarantineSummary,
    /// The go-live card, once raised.
    pub card: Option<CardLink>,
    /// What this Move's model calls have cost (S2-11): "Models cost $0.31 on this Move across 22 calls."
    pub spend_words: Option<String>,
    /// The exports the owner uploaded for the Move (R4.1), newest of each name.
    pub uploads: Vec<UploadRow>,
}

/// An export uploaded for the Move: `export.xml`, `dump.sql.gz` or `wp-content.tar.gz`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UploadRow {
    pub name: String,
    /// "28 KB".
    pub size_words: String,
    /// "22:10" or "Yesterday".
    pub when_words: String,
}

// ---------- Today, Decisions, the go-live card ----------

/// A line of the brief.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BriefLine {
    pub text: String,
    pub href: Option<String>,
}

/// The Chief WebAgent's brief, composed only from receipts (design §4.4).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Brief {
    pub written_words: String,
    pub events_count: u32,
    pub lines: Vec<BriefLine>,
    pub spend_words: Option<String>,
}

/// [`crate::Page::Today`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct TodayView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub greeting: String,
    /// The Chief WebAgent's team mark.
    pub chief: AgentMark,
    pub brief: Option<Brief>,
    /// Shown when there is no brief yet, e.g. "Your first brief comes at 07:00 tomorrow."
    pub first_brief_words: String,
    pub needs_you: Vec<CardLink>,
    pub recent: Vec<ReceiptRow>,
    /// Questions visitors passed to the owner from Ask (R8.3), newest first.
    pub passed: Vec<PassedQuestion>,
    /// How many questions visitors have passed in all.
    pub passed_total: u32,
    /// Model spend this month against the site's monthly AI budget (R11.3), once any model
    /// call was made.
    pub meter: Option<Meter>,
}

/// The model-spend meter (R11.3): this month against the monthly AI budget, in the site's
/// currency, and where it went. The gateway's daily limit is an internal guard and never shown
/// (`specs/platform/facts.md`, "AI budget").
#[derive(Debug, Clone, Default, Serialize)]
pub struct Meter {
    /// Spent this month: "KSh 38".
    pub spent: String,
    /// "KSh 38 of KSh 3,870 this month", or "KSh 38 this month" before a monthly budget is set.
    pub cap_words: String,
    /// This month's share of the monthly budget, 0–100; `None` without one.
    pub cap_pct: Option<u32>,
    /// What the budget is: "KSh 3,870 a month of AI is included with your Commerce plan.", or
    /// that none is set.
    pub cap_note: String,
    /// "312 AI calls this month, 40 of them replayed at no cost".
    pub calls_words: String,
    /// This month by job, most expensive first.
    pub by_purpose: Vec<MeterLine>,
    /// This month by provider (Anthropic, Vultr), most expensive first.
    pub by_provider: Vec<MeterLine>,
    /// "3 AI calls were stopped by a spending limit this month", when any were.
    pub refused_words: Option<String>,
}

/// One line of the meter.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MeterLine {
    pub label: String,
    pub amount: String,
}

/// A question a visitor passed to the owner from Ask, because the site didn't say (R8.3).
#[derive(Debug, Clone, Default, Serialize)]
pub struct PassedQuestion {
    /// The question, as the visitor wrote it (plain text; the template escapes it).
    pub question: String,
    /// When, in the owner's words, for example "09:41" or "Yesterday".
    pub when_words: String,
    /// How to reach the visitor, if they left one.
    pub contact: Option<String>,
}

/// How risky a card is.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Risk {
    #[default]
    Low,
    Medium,
    High,
}

/// A card in the list.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CardSummary {
    pub id: String,
    pub title: String,
    pub asked_by: AgentMark,
    pub risk: Risk,
    pub risk_words: String,
    pub created_words: String,
    pub summary: String,
    /// For decided cards: "Approved with your passkey".
    pub outcome_words: String,
}

/// [`crate::Page::Decisions`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct DecisionsView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub waiting: Vec<CardSummary>,
    pub decided: Vec<CardSummary>,
}

/// What a card decides.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CardKind {
    #[default]
    GoLive,
    QuarantineRestore,
    OrderHeld,
}

/// A card's state.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CardState {
    #[default]
    Waiting,
    Approved,
    Declined,
    Expired,
}

/// One position on a job's dial.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Position {
    /// "Watch", "Suggest", "Ask me first", "Do it and tell me", "Handle it within budget".
    pub name: String,
    pub current: bool,
    /// Above the job's ceiling (ADR-012 amendment 1).
    pub locked: bool,
    /// Why it's locked, in the owner's words. Empty unless `locked`.
    pub reason: String,
}

/// A job's dial: the starting level on the go-live card, read-only on WebAgents.
#[derive(Debug, Clone, Default, Serialize)]
pub struct JobLevels {
    pub webagent: String,
    pub job: String,
    /// The current position, in plain words.
    pub level_words: String,
    /// True when the job may act without asking (listed in plain words, R7.5).
    pub above_ask_first: bool,
    pub positions: Vec<Position>,
}

/// How a decided card was decided.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Decided {
    pub outcome_words: String,
    /// "Your passkey on this iPhone" or "A code from your authenticator app".
    pub method_words: String,
    pub at_words: String,
    pub receipt: String,
}

/// A decision card, including the go-live card (R7).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Card {
    pub id: String,
    pub kind: CardKind,
    pub title: String,
    pub asked_by: AgentMark,
    pub risk: Risk,
    pub risk_words: String,
    pub why_words: String,
    /// "If you approve, exactly this happens", rendered from the canonical arguments.
    pub effects: Vec<String>,
    /// Parity, left behind, old site kept, how to undo.
    pub facts: Vec<Fact>,
    /// Starting autonomy for charter v1 (R7.5). Empty for other kinds.
    pub levels: Vec<JobLevels>,
    pub fingerprint: String,
    pub fingerprint_short: String,
    /// The canonical arguments behind the fingerprint.
    pub fields: Vec<Fact>,
    pub state: CardState,
    pub decided: Option<Decided>,
    /// The approval ceremony's options as JSON (challenge = SHA-256(fingerprint ‖ nonce); D9).
    pub passkey_options: Option<String>,
    pub has_passkey: bool,
    pub approve_url: String,
    pub decline_url: String,
}

/// [`crate::Page::Decision`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct DecisionView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub card: Card,
}

// ---------- Trust, receipts, WebAgents, quarantine ----------

/// One gauntlet attack.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Attack {
    pub name: String,
    pub plain: String,
    pub held: bool,
    /// The receipt that proves it, when the run recorded one.
    pub receipt: Option<String>,
}

/// The gauntlet score, computed from receipts only (R13.2).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Gauntlet {
    pub held: u32,
    pub of: u32,
    /// "10 / 10".
    pub score_words: String,
    /// "at 14:02 today".
    pub run_words: String,
    pub receipt: String,
    pub attacks: Vec<Attack>,
}

/// [`crate::Page::Trust`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct TrustView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub gauntlet: Option<Gauntlet>,
    pub receipts: Vec<ReceiptRow>,
    pub key_id: String,
}

/// One receipt, opened.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReceiptDetail {
    pub digest: String,
    pub title: String,
    pub kind: String,
    pub kind_words: String,
    pub at_words: String,
    pub issuer: String,
    pub subject: String,
    pub facts: Vec<Fact>,
    pub outputs: Vec<String>,
    pub verified: bool,
    /// "Signature checked against key ab12… at 10:31" or why it failed.
    pub verified_words: String,
    pub key_id: String,
    /// The signed receipt, pretty-printed.
    pub json: String,
}

/// [`crate::Page::Receipt`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReceiptView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub receipt: ReceiptDetail,
}

/// A WebAgent and its jobs.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentCard {
    pub mark: AgentMark,
    pub name: String,
    pub role_words: String,
    pub jobs: Vec<JobLevels>,
}

/// [`crate::Page::WebAgents`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct WebAgentsView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub agents: Vec<AgentCard>,
}

/// A quarantined item's state.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QuarantineState {
    #[default]
    LeftBehind,
    RestoreAsked,
    Restored,
}

/// One item left behind.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuarantineItem {
    pub id: String,
    pub title: String,
    pub plain: String,
    #[serde(rename = "where")]
    pub location: String,
    pub evidence: Vec<Evidence>,
    pub state: QuarantineState,
    /// The restore card, once asked.
    pub decision_href: Option<String>,
    /// Where the restore form posts (raises a card; S2-06).
    pub restore_url: String,
}

/// [`crate::Page::Quarantine`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuarantineView {
    #[serde(flatten)]
    pub chrome: Chrome,
    pub items: Vec<QuarantineItem>,
}
