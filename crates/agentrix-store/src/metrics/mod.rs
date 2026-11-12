//! The read side behind every number the owner console and the control plane show.
//!
//! [`Metrics`] is the trait W3 (operator console) and W5 (owner console) call. It is read-only.
//! [`SqliteMetrics`] implements it over the first release's `control.db` (the Model Gateway's `usage` and
//! `budgets` live there too) and over W8's visit counts. When the per-site stores land, a
//! second implementation reads them; callers don't change.
//!
//! Rules every method keeps:
//! - **Recorded activity only.** Each value is computed from rows; nothing is typed in.
//! - **Honest gaps.** A number with no source yet comes back as [`Reading::NoSource`] with the
//!   words to show and the workstream that will build the source (see [`crate::catalogue`]).
//! - **Fleet numbers leave samples and fixtures out** (03-data.md §5.2 rule 6). Lists by site
//!   carry each site's [`Origin`] so a screen can show them apart.
//! - **Small cells stay hidden** for visit counts ([`crate::words::Shown`]).

mod health_score;
mod sqlite;
#[cfg(test)]
mod tests;

use agentrix_types::SiteId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

pub use health_score::{Band, Signal, SiteHealth};
pub use sqlite::SqliteMetrics;

use crate::time::{Span, Window};
use crate::visits::VisitReport;
use crate::words::MicroUsd;

/// What a store read can fail with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The database refused or failed the read.
    #[error("reading control.db: {0}")]
    Db(#[from] sqlx::Error),
}

/// A metric read.
pub type Result<T> = std::result::Result<T, Error>;

/// A number that may not have a source yet.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Reading<T> {
    /// Computed from recorded rows.
    Value {
        /// The value.
        value: T,
    },
    /// Nothing produces this number yet. Show `words`; never a made-up figure.
    NoSource {
        /// The catalogue id ([`crate::catalogue::Metric::id`]).
        metric: &'static str,
        /// What to show instead, in plain words.
        words: &'static str,
        /// The workstream building the source.
        owner: &'static str,
    },
}

impl<T> Reading<T> {
    /// The value, if there is a source.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value { value } => Some(value),
            Self::NoSource { .. } => None,
        }
    }
}

/// Which sites a number covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every real site: samples and fixtures left out.
    Fleet,
    /// Every site, samples and Zavora's fixture included: for work the platform did (receipts
    /// issued, Moves run), never for business numbers.
    Everything,
    /// One site.
    Site(SiteId),
}

/// Where a site came from (03-data.md §1.2). Derived until W3 adds `sites.origin`: a fixture
/// row has `fixture = 1`; a sample's host label ends in `-sample` (how-we-work.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A real business.
    Real,
    /// A sample site marked Sample (Kilele Bakery, Classy Perfumes, Tana Valley Times).
    Sample,
    /// A gauntlet or test fixture.
    Fixture,
}

impl Origin {
    /// The origin of a `sites` row.
    pub fn of(domain: &str, fixture: bool) -> Self {
        if fixture {
            Self::Fixture
        } else if domain
            .split('.')
            .next()
            .is_some_and(|label| label.ends_with("-sample"))
        {
            Self::Sample
        } else {
            Self::Real
        }
    }
}

/// Where a site is in its life, derived from the first release's rows until W3 adds a lifecycle column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiteState {
    /// Assessed, nobody has claimed it.
    Assessed,
    /// Claimed by an owner; no Move yet.
    Claimed,
    /// A Move is running.
    Moving,
    /// The Move is done and waits for the owner's go-live.
    AwaitingOwner,
    /// A signed release is live.
    Live,
    /// Rolled back to the old site.
    RolledBack,
    /// The Move failed or was stopped.
    Stopped,
}

impl SiteState {
    /// Plain words for a list.
    pub fn words(self) -> &'static str {
        match self {
            Self::Assessed => "Assessed",
            Self::Claimed => "Claimed",
            Self::Moving => "Moving",
            Self::AwaitingOwner => "Waiting for the owner",
            Self::Live => "Live",
            Self::RolledBack => "Back on the old site",
            Self::Stopped => "Move stopped",
        }
    }
}

/// One site in the registry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SiteSummary {
    /// The site.
    pub site_id: SiteId,
    /// Its primary domain.
    pub domain: String,
    /// Its name, if Assess found one.
    pub name: String,
    /// Real, sample or fixture.
    pub origin: Origin,
    /// Where it is.
    pub state: SiteState,
    /// Whether an owner has claimed it.
    pub claimed: bool,
    /// The live release, if any.
    pub live_seq: Option<i64>,
    /// When the first release went live.
    pub live_since: Option<DateTime<Utc>>,
    /// Minutes from UTC (the first release's fixed offset).
    pub utc_offset_minutes: i32,
    /// When the site was first assessed.
    pub created_at: DateTime<Utc>,
}

/// Sites by state, for the fleet's headline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FleetCounts {
    /// Real sites live.
    pub live: i64,
    /// Real sites with a Move running.
    pub moving: i64,
    /// Real sites waiting for the owner's go-live.
    pub awaiting_owner: i64,
    /// Real sites claimed, no Move yet.
    pub claimed: i64,
    /// Real sites assessed and not claimed.
    pub assessed: i64,
    /// Real sites rolled back or stopped.
    pub stopped: i64,
    /// Sample sites (not in the numbers above).
    pub samples: i64,
    /// Fixtures (not in the numbers above).
    pub fixtures: i64,
    /// Real sites that went live in the window asked for.
    pub went_live_in_window: i64,
}

/// One origin's counts for the operator's fleet tiles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OriginCounts {
    /// Sites in the registry.
    pub sites: i64,
    /// Sites an owner has claimed.
    pub claimed: i64,
    /// Sites serving a signed release now.
    pub live: i64,
    /// Sites whose first release went live inside the window.
    pub went_live_in_window: i64,
    /// Moves still in flight (claimed through waiting for the owner).
    pub moves_in_flight: i64,
    /// Moves waiting for the owner's go-live card.
    pub moves_waiting_owner: i64,
    /// Moves that ended live.
    pub moves_live: i64,
}

/// Fleet counts split by origin, so a screen labels samples and the fixture instead of
/// hiding or mixing them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FleetByOrigin {
    /// The window for `went_live_in_window`.
    pub span: Span,
    /// Real businesses.
    pub real: OriginCounts,
    /// Sample sites.
    pub sample: OriginCounts,
    /// Zavora's fixture.
    pub fixture: OriginCounts,
}

/// Quantiles of a duration, nearest rank.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Quantiles {
    /// How many durations.
    pub n: i64,
    /// The median.
    pub p50_ms: Option<i64>,
    /// The 90th percentile.
    pub p90_ms: Option<i64>,
    /// The 95th percentile.
    pub p95_ms: Option<i64>,
}

/// The Migration Factory strip: in-flight Moves by the step they are on, and how long Moves
/// take to reach the owner's go-live card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MoveFactory {
    /// The window for `to_go_live`.
    pub span: Span,
    /// In-flight Moves per step, in pipeline order, every step listed (0 when none).
    pub at_step: Vec<(String, i64)>,
    /// From a Move's start to its go-live card, for Moves started in the window.
    pub to_go_live: Quantiles,
}

/// One site's waiting decisions, as a count only (operators never see card contents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteWaiting {
    /// The site.
    pub site_id: SiteId,
    /// Its domain.
    pub domain: String,
    /// Real, sample or fixture.
    pub origin: Origin,
    /// Cards waiting.
    pub count: i64,
}

/// One line of a spend breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpendLine {
    /// Purpose, provider or model.
    pub key: String,
    /// For a purpose, its words from `usage_purposes` ("Answering visitors"); otherwise none.
    pub label: Option<String>,
    /// Estimated cost.
    pub cost: MicroUsd,
    /// Calls that reached a provider or a recording.
    pub calls: i64,
}

/// A budget and how much of it is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LimitUse {
    /// The purpose it covers, or `*` for every purpose.
    pub purpose: String,
    /// `day`, `month` or `total`.
    pub period: String,
    /// The limit.
    pub limit: MicroUsd,
    /// Spent in the current period.
    pub spent: MicroUsd,
    /// The gateway's own hard daily limit per site (`AGENTRIX_SITE_DAILY_CAP_USD`), rather
    /// than a budget row.
    pub hard: bool,
}

impl LimitUse {
    /// Whole percent used, capped at 100.
    pub fn percent(&self) -> i64 {
        if self.limit.0 <= 0 {
            return 100;
        }
        ((self.spent.0 as i128 * 100) / self.limit.0 as i128).clamp(0, 100) as i64
    }

    /// "$0.38 of $5.00 today".
    pub fn words(&self) -> String {
        let window = match self.period.as_str() {
            "day" => " today",
            "month" => " this month",
            _ => "",
        };
        format!("{} of {}{window}", self.spent.usd(), self.limit.usd())
    }
}

/// Model spend over a window, for one site or the fleet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpendReport {
    /// The window.
    pub span: Span,
    /// Estimated cost (catalogue prices; replayed calls cost nothing).
    pub cost: MicroUsd,
    /// Calls that reached a provider or a recording.
    pub calls: i64,
    /// Calls a provider answered.
    pub ok: i64,
    /// Calls answered from recordings.
    pub replayed: i64,
    /// Calls a budget refused.
    pub refused: i64,
    /// Calls that ended in an error.
    pub errors: i64,
    /// Tokens in and out.
    pub tokens_in: i64,
    /// Tokens out, reasoning included.
    pub tokens_out: i64,
    /// By purpose, most expensive first.
    pub by_purpose: Vec<SpendLine>,
    /// By provider, most expensive first.
    pub by_provider: Vec<SpendLine>,
    /// By model, most expensive first.
    pub by_model: Vec<SpendLine>,
    /// By family (`chat`, `search`, `image`, `voice`; `other` for a purpose not in
    /// `usage_purposes`), most expensive first.
    pub by_family: Vec<SpendLine>,
    /// By WebAgent (`coordinator`, `cashier`…), for calls that named one
    /// (`x-agentrix-webagent`), most expensive first.
    pub by_webagent: Vec<SpendLine>,
    /// By job (`owner-questions`, `payments`…), for calls that named one, most expensive first.
    pub by_job: Vec<SpendLine>,
    /// Quantities of calls not counted in tokens: (`image`, 12), (`audio_second`, 240),
    /// (`character`, 5400).
    pub units: Vec<(String, i64)>,
    /// The site's budgets and the gateway's daily cap, each with its current period's spend.
    /// Empty for the fleet.
    pub limits: Vec<LimitUse>,
}

impl SpendReport {
    /// The daily limit that binds first for this site: the smallest `*` day limit, whether the
    /// site's own budget or the gateway's cap. Its `spent` is counted in the UTC day, as the
    /// gateway counts it. `None` for the fleet.
    pub fn day_limit(&self) -> Option<&LimitUse> {
        self.limits
            .iter()
            .filter(|l| l.purpose == "*" && l.period == "day")
            .min_by_key(|l| l.limit)
    }

    /// Plain words: "Models cost $0.42 today across 31 calls, 4 of them replayed at no cost."
    pub fn words(&self) -> String {
        if self.calls == 0 && self.refused == 0 {
            return format!("No model calls {}.", self.span.words);
        }
        let mut line = format!(
            "Models cost {} {} across {}",
            self.cost.usd(),
            self.span.words,
            crate::words::plural(self.calls, "call", "calls")
        );
        if self.replayed > 0 {
            line.push_str(&format!(", {} of them replayed at no cost", self.replayed));
        }
        if self.refused > 0 {
            line.push_str(&format!("; {} stopped by the budget", self.refused));
        }
        line.push('.');
        line
    }
}

/// One site's spend, for the operator's list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SiteSpend {
    /// The site.
    pub site_id: SiteId,
    /// Its domain.
    pub domain: String,
    /// Real, sample or fixture (sample spend is billed to the operator tenant).
    pub origin: Origin,
    /// Estimated cost in the window.
    pub cost: MicroUsd,
    /// Calls in the window.
    pub calls: i64,
    /// Calls a budget refused in the window.
    pub refused: i64,
    /// Calls answered from recordings in the window.
    pub replayed: i64,
    /// Calls that ended in an error in the window.
    pub errors: i64,
    /// Spent today (UTC) against the gateway's daily cap.
    pub today: LimitUse,
    /// The site's budget rows, each with its spend in its own current period.
    pub budgets: Vec<LimitUse>,
}

/// Filters for a list of receipts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceiptQuery {
    /// Only these kinds (`job.fetch`, `decision.approved` …); empty for all.
    pub kinds: Vec<String>,
    /// Only receipts issued before this stored time (the last row's `issued_at`), for paging.
    pub before: Option<String>,
    /// At most this many (1 to 200; default 50).
    pub limit: Option<u32>,
    /// Leave titles out: operators see kinds, subjects, digests and times only.
    pub content_free: bool,
    /// Only receipts whose work ended at or after this time. With either `ended_*` bound the
    /// list is ordered by `ended_at`, newest first.
    pub ended_from: Option<DateTime<Utc>>,
    /// Only receipts whose work ended before this time.
    pub ended_before: Option<DateTime<Utc>>,
}

/// A receipt as a list shows it. The signed body stays behind `GET /receipts/{digest}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceiptRow {
    /// The content address.
    pub digest: String,
    /// The receipt id.
    pub receipt_id: String,
    /// What it records.
    pub kind: String,
    /// What it is about: `job`, `move`, `decision`, `site` …
    pub subject_type: String,
    /// The subject's id.
    pub subject_id: String,
    /// The key that signed it.
    pub issuer: String,
    /// When it was issued (stored text; also the paging cursor).
    pub issued_at: String,
    /// When the work it records ended (stored RFC 3339 UTC text).
    pub ended_at: String,
    /// Whether it records something that went wrong: `detail.state` or `detail.outcome` is
    /// `failed`, `error` or `stopped` (the console's rule).
    pub failed: bool,
    /// Its one-line title, when the issuer gave one.
    pub title: Option<String>,
}

/// Receipt counts, for "412 entries since go-live" and the Trust tab's filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceiptCounts {
    /// Receipts in the window.
    pub total: i64,
    /// Receipts since the site's first release went live (site scope only).
    pub since_go_live: Option<i64>,
    /// By kind, most first.
    pub by_kind: Vec<(String, i64)>,
    /// The newest receipt's issue time.
    pub newest: Option<String>,
}

/// One step of a Move, with its times.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepTimes {
    /// `assess`, `claim`, `replay`, `clean`, `rebuild`, `prove`, `golive`.
    pub step: String,
    /// Its latest state: `pending`, `running`, `waiting`, `done`, `failed`.
    pub state: String,
    /// Its latest summary in plain words.
    pub summary: String,
    /// When it first started.
    pub started_at: String,
    /// When it ended, if it has.
    pub ended_at: Option<String>,
    /// How long it took, or has taken so far.
    pub millis: Option<i64>,
    /// The receipt of its last row, if any.
    pub receipt: Option<String>,
}

/// A Move and its steps.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MoveSummary {
    /// The Move.
    pub move_id: String,
    /// The site.
    pub site_id: SiteId,
    /// The site's domain.
    pub domain: String,
    /// The site's origin.
    pub origin: Origin,
    /// The Move's state.
    pub state: String,
    /// Whether it is still in flight.
    pub in_flight: bool,
    /// Started.
    pub created_at: String,
    /// Last changed.
    pub updated_at: String,
    /// Steps in the order they started.
    pub steps: Vec<StepTimes>,
    /// The step it is on (the latest one not done), if any.
    pub current_step: Option<String>,
    /// Model spend so far.
    pub cost: MicroUsd,
}

/// Filters for a list of Moves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveQuery {
    /// One site's Moves only.
    pub site: Option<SiteId>,
    /// Only Moves still in flight.
    pub in_flight: bool,
    /// At most this many, newest first (1 to 200; default 50).
    pub limit: Option<u32>,
}

/// How long one step of the Move takes across finished steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepTime {
    /// The step.
    pub step: String,
    /// Finished runs of the step in the window.
    pub runs: i64,
    /// The median.
    pub p50_ms: i64,
    /// The 95th percentile (the slowest run when there are fewer than 20).
    pub p95_ms: i64,
}

/// Sealed-room jobs (the `jobs` table).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RoomStats {
    /// Running now.
    pub running: i64,
    /// Queued now.
    pub queued: i64,
    /// Finished in the window.
    pub done: i64,
    /// Failed in the window.
    pub failed: i64,
    /// Killed in the window (a limit was hit).
    pub killed: i64,
    /// Per kind (`fetch`, `replay`, `render`, `build`): finished runs and times.
    pub by_kind: Vec<RoomKind>,
}

/// One kind of sealed room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoomKind {
    /// `fetch`, `replay`, `render` or `build`.
    pub kind: String,
    /// Jobs started in the window.
    pub jobs: i64,
    /// Failed or killed in the window.
    pub not_done: i64,
    /// Median run time of finished jobs.
    pub p50_ms: Option<i64>,
    /// 95th percentile run time.
    pub p95_ms: Option<i64>,
    /// Finished jobs the run times are over.
    pub runs: i64,
    /// Jobs that started, which the queue waits are over.
    pub waits: i64,
    /// Median wait from queued to started.
    pub wait_p50_ms: Option<i64>,
    /// 95th percentile wait from queued to started.
    pub wait_p95_ms: Option<i64>,
}

/// Sealed-room time for one site: what W12's cost-to-serve rule splits the sandbox by.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomMinutes {
    /// The site.
    pub site_id: SiteId,
    /// Its domain.
    pub domain: String,
    /// Real, sample or fixture.
    pub origin: Origin,
    /// Jobs that ran in the window.
    pub jobs: i64,
    /// Room time inside the window (a job still running counts up to now), in milliseconds.
    pub millis: i64,
}

impl RoomMinutes {
    /// Whole minutes, rounded up, as a bill counts them.
    pub fn minutes(&self) -> i64 {
        (self.millis + 59_999) / 60_000
    }
}

/// A gauntlet run's score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GauntletScore {
    /// The site.
    pub site_id: SiteId,
    /// Its domain.
    pub domain: String,
    /// Attacks stopped.
    pub score: i64,
    /// Attacks tried.
    pub out_of: i64,
    /// When the run started.
    pub started_at: String,
    /// When it ended.
    pub ended_at: Option<String>,
    /// The run's receipt.
    pub receipt: Option<String>,
    /// Whether the site is live now.
    pub live: bool,
}

/// A decision card waiting for the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingCard {
    /// The decision.
    pub decision_id: String,
    /// The site.
    pub site_id: SiteId,
    /// `go-live`, `quarantine-restore`, `order-held` (more once W5's card store lands).
    pub kind: String,
    /// The card's title.
    pub title: String,
    /// When it was raised.
    pub created_at: String,
}

/// Decisions waiting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DecisionsWaiting {
    /// How many.
    pub count: i64,
    /// When the oldest was raised.
    pub oldest: Option<DateTime<Utc>>,
    /// By kind, most first.
    pub by_kind: Vec<(String, i64)>,
    /// The cards, oldest first (at most 50).
    pub cards: Vec<WaitingCard>,
}

/// The read side the consoles call. Every method is read-only.
#[async_trait]
pub trait Metrics: Send + Sync {
    /// Every site in the registry, newest first.
    async fn sites(&self) -> Result<Vec<SiteSummary>>;

    /// One site.
    async fn site(&self, site: &SiteId) -> Result<Option<SiteSummary>>;

    /// Real sites by state; samples and fixtures counted apart.
    async fn fleet(&self, window: Window) -> Result<FleetCounts>;

    /// Fleet counts split into real, sample and fixture.
    async fn fleet_by_origin(&self, window: Window) -> Result<FleetByOrigin>;

    /// In-flight Moves by step, and time to the go-live card for Moves started in `window`.
    async fn move_factory(&self, window: Window) -> Result<MoveFactory>;

    /// Gauntlet runs that started in `window`, every site.
    async fn gauntlet_runs(&self, window: Window) -> Result<i64>;

    /// Decisions waiting per site, counts only, most first.
    async fn waiting_by_site(&self) -> Result<Vec<SiteWaiting>>;

    /// Model spend over `window`. For a site, also its budgets and the gateway's daily cap.
    async fn spend(&self, scope: &Scope, window: Window) -> Result<SpendReport>;

    /// Spend per site over `window`, most expensive first, every origin included.
    async fn spend_by_site(&self, window: Window) -> Result<Vec<SiteSpend>>;

    /// A site's receipts, newest first.
    async fn receipts(&self, site: &SiteId, query: &ReceiptQuery) -> Result<Vec<ReceiptRow>>;

    /// Receipt counts over `window`.
    async fn receipt_counts(&self, scope: &Scope, window: Window) -> Result<ReceiptCounts>;

    /// Moves with their steps and times, newest first.
    async fn moves(&self, query: &MoveQuery) -> Result<Vec<MoveSummary>>;

    /// How long each step of the Move takes, over finished steps that started in `window`.
    async fn step_times(&self, window: Window) -> Result<Vec<StepTime>>;

    /// Sealed rooms now and over `window`.
    async fn rooms(&self, scope: &Scope, window: Window) -> Result<RoomStats>;

    /// Sealed-room time per site over `window`, clipped to the window, most first.
    async fn room_minutes(&self, scope: &Scope, window: Window) -> Result<Vec<RoomMinutes>>;

    /// The latest gauntlet score per site (one site, or every site).
    async fn gauntlet(&self, site: Option<&SiteId>) -> Result<Vec<GauntletScore>>;

    /// Decision cards waiting for an owner.
    async fn decisions_waiting(&self, scope: &Scope) -> Result<DecisionsWaiting>;

    /// Assess reports finished (assessments in `report-ready`) over `window`, every site.
    async fn reports_written(&self, window: Window) -> Result<i64>;

    /// Questions visitors passed to the owner from Ask over `window`.
    async fn questions_passed(&self, site: &SiteId, window: Window) -> Result<i64>;

    /// A site's health score from 08 §4.1's content-free signals (operators and partners
    /// only; never shown to the owner as a grade). `None` if the site isn't known.
    async fn site_health(&self, site: &SiteId) -> Result<Option<SiteHealth>>;

    /// Health for every live site, lowest score first (no score last).
    async fn health_by_site(&self) -> Result<Vec<SiteHealth>>;

    /// Visit counts over `window`, small cells hidden.
    async fn visits(&self, site: &SiteId, window: Window) -> Result<Reading<VisitReport>>;
}
