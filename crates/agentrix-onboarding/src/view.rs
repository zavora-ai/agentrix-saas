//! View models for the onboarding pages, with display-ready strings.
//!
//! Node already fills `agentrix_console::view::StartView` and `AssessView` from Assess's store
//! and live state. The pages here are built from those (`From`), so wiring them in changes no
//! data path: only which templates render. Fields are the template contract; each template's
//! header comment lists what it reads.
#![allow(missing_docs)]

use agentrix_console::view as console;
use serde::Serialize;

pub use console::{AssessState, Counts, Evidence, Frame, FrameKind, Severity, Tick, Unreachable};

// ---------- Start (O1.1) ----------

/// [`crate::OnboardingPage::Start`]: the address bar on its own, at `GET /assess`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StartPage {
    /// What was wrong with the address, in words.
    pub error: Option<String>,
    /// What was typed, shown back so it can be fixed.
    pub url: Option<String>,
    /// Starts an assessment of Zavora's fixture, when one is configured.
    pub fixture_url: Option<String>,
    /// A finished sample site to open instead, when one is configured.
    pub sample_site: Option<SampleLink>,
    /// An owner is signed in (for example sent here by the console while they have no site).
    pub signed_in: bool,
}

/// A link to a sample site, which says "Sample" wherever it is shown.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SampleLink {
    /// "Kilele Bakery".
    pub name: String,
    pub href: String,
}

impl From<console::StartView> for StartPage {
    fn from(v: console::StartView) -> Self {
        Self {
            error: v.error,
            url: v.url,
            fixture_url: v.fixture_url,
            sample_site: None,
            signed_in: false,
        }
    }
}

impl StartPage {
    /// The same page with a sample site to offer.
    pub fn with_sample(mut self, sample: Option<SampleLink>) -> Self {
        self.sample_site = sample;
        self
    }
}

// ---------- Assess (O1.2 to O1.4, O1.8) ----------

/// [`crate::OnboardingPage::Assess`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct AssessPage {
    pub id: String,
    pub host: String,
    pub fixture: bool,
    pub state: AssessState,
    /// "Step 1 of 3" while reading, "Step 2 of 3" with the report; empty otherwise.
    pub step_words: String,
    /// "14 of 50 pages read".
    pub progress_words: String,
    pub started_words: String,
    pub tick: Tick,
    pub frame: Option<Frame>,
    /// Present when `state` is `Done`.
    pub report: Option<ReportCard>,
    /// Present when `state` is `Unreachable` or `Refused` (R1.6).
    pub unreachable: Option<Unreachable>,
    /// "Keep this site: create your account": create the account, then the claim.
    pub keep_href: String,
    /// "Sign in", then the claim.
    pub signin_href: String,
    /// This report's own address, for "Share report".
    pub report_path: String,
    /// The run's event stream (design §4.3a).
    pub events_path: String,
}

/// The health report in the concept's shape: one grid of findings, then what looks fine.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReportCard {
    pub summary_words: String,
    pub counts: Counts,
    /// "4 findings", "1 finding", "No findings".
    pub total_words: String,
    /// "14 pages read".
    pub pages_words: String,
    pub receipt: Option<String>,
    /// Urgent first, then worth fixing, then good to know; report order within each.
    pub findings: Vec<FindingCard>,
    /// Areas with no findings, in the report's own words.
    pub fine: Vec<FineLine>,
}

/// One finding card (concept/assess.html `.finding`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct FindingCard {
    pub id: String,
    pub severity: Severity,
    /// The chip's class: `chip-risk`, `chip-wait` or empty.
    pub chip: String,
    /// "Urgent", "Worth fixing", "Good to know".
    pub severity_words: String,
    /// "Security", "Speed", "Broken links", "AI assistants".
    pub area: String,
    pub title: String,
    pub plain: String,
    pub evidence: Vec<Evidence>,
}

/// An area of the report with nothing to fix.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FineLine {
    pub area: String,
    /// "No broken links among the 14 pages we read."
    pub words: String,
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The short name of a report section, for chips.
fn area(title: &str) -> String {
    match title {
        "Can AI assistants use your site?" => "AI assistants".into(),
        other => other.trim_end_matches('?').to_owned(),
    }
}

fn rank(s: Severity) -> u8 {
    match s {
        Severity::Urgent => 0,
        Severity::Warn => 1,
        Severity::Info => 2,
    }
}

fn card(section: &str, f: console::Finding) -> FindingCard {
    let (chip, words) = match f.severity {
        Severity::Urgent => ("chip-risk", "Urgent"),
        Severity::Warn => ("chip-wait", "Worth fixing"),
        Severity::Info => ("", "Good to know"),
    };
    FindingCard {
        id: f.id,
        severity: f.severity,
        chip: chip.into(),
        severity_words: words.into(),
        area: area(section),
        title: f.title,
        plain: f.plain,
        evidence: f.evidence,
    }
}

impl From<console::Report> for ReportCard {
    fn from(r: console::Report) -> Self {
        let total = r.counts.urgent + r.counts.warn + r.counts.info;
        let mut findings = Vec::new();
        let mut fine = Vec::new();
        for s in r.sections {
            if s.findings.is_empty() {
                fine.push(FineLine {
                    area: s.title,
                    words: s.empty_words,
                });
            } else {
                findings.extend(s.findings.into_iter().map(|f| card(&s.title, f)));
            }
        }
        findings.sort_by_key(|f| rank(f.severity));
        Self {
            summary_words: r.summary_words,
            counts: r.counts,
            total_words: if total == 0 {
                "No findings".into()
            } else {
                plural(total, "finding", "findings")
            },
            pages_words: plural(r.pages_read, "page read", "pages read"),
            receipt: r.receipt,
            findings,
            fine,
        }
    }
}

impl From<console::AssessView> for AssessPage {
    fn from(v: console::AssessView) -> Self {
        let step_words = match v.state {
            AssessState::Running => "Step 1 of 3",
            AssessState::Done => "Step 2 of 3",
            AssessState::Unreachable | AssessState::Refused => "",
        };
        // The first release sends "Move this site" to sign-in, then the claim. A new owner creates the
        // account instead, passkey first (O1.8 → O2.1), and both carry the same `next`.
        let next = v
            .move_href
            .split_once("next=")
            .map(|(_, n)| n.to_owned())
            .unwrap_or_else(|| crate::routes::DEFAULT_NEXT.to_owned());
        let keep_href = format!(
            "/account/new?next={}&host={}",
            encode(&next),
            encode(&v.host)
        );
        let signin_href = format!("/account/signin?next={}", encode(&next));
        Self {
            report_path: format!("/assess/{}", v.id),
            events_path: format!("/assess/{}/events", v.id),
            id: v.id,
            host: v.host,
            fixture: v.fixture,
            state: v.state,
            step_words: step_words.into(),
            progress_words: v.progress_words,
            started_words: v.started_words,
            tick: v.tick,
            frame: v.frame,
            report: v.report.map(ReportCard::from),
            unreachable: v.unreachable,
            keep_href,
            signin_href,
        }
    }
}

// ---------- Claim the domain (O4.1) ----------

/// [`crate::OnboardingPage::Claim`]: prove the domain is yours, as `owner-domain.html` draws it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClaimPage {
    pub site_id: String,
    pub host: String,
    pub fixture: bool,
    pub method: ClaimMethod,
    /// The TXT record's name as most registrars want it, e.g. `_agentrix`.
    pub dns_short: String,
    /// The full name, e.g. `_agentrix.kilele-bakery.example`.
    pub dns_name: String,
    pub dns_value: String,
    pub file_name: String,
    pub file_url: String,
    /// What the file holds, exactly.
    pub token: String,
    /// A `data:` link that downloads the file, so nobody has to make it by hand.
    pub file_download: String,
    pub state: ClaimState,
    /// "Checked 10:02 UTC from the sandbox".
    pub checked_words: Option<String>,
    /// What the last check saw, in words.
    pub said: Option<String>,
    /// Where "Check now" posts.
    pub verify_action: String,
    /// Where "Next" leads once proved.
    pub next_href: String,
}

pub use console::{ClaimMethod, ClaimState};

/// Collapses a doubled full stop ("server.. Nothing") left where two sentences were joined.
fn tidy_stops(words: &str) -> String {
    let mut out = String::with_capacity(words.len());
    for c in words.chars() {
        if c == '.' && out.ends_with('.') && !out.ends_with("..") {
            continue;
        }
        out.push(c);
    }
    out
}

/// Percent-encodes everything but unreserved characters and `=`, for a `data:` URL.
fn data_url_text(text: &str) -> String {
    let mut out = String::from("data:text/plain;charset=utf-8,");
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'=') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl From<console::ClaimView> for ClaimPage {
    fn from(v: console::ClaimView) -> Self {
        let host = v.chrome.site.host;
        let c = v.claim;
        let suffix = format!(".{host}");
        let dns_short = c
            .dns_name
            .strip_suffix(&suffix)
            .unwrap_or(&c.dns_name)
            .to_owned();
        Self {
            verify_action: format!("/sites/{}/claim/verify", v.chrome.site.id),
            next_href: "/console/move".into(),
            site_id: v.chrome.site.id,
            fixture: v.chrome.site.fixture,
            method: c.method,
            dns_short,
            dns_name: c.dns_name,
            dns_value: c.dns_value,
            file_name: c.file_name,
            file_url: c.file_url,
            file_download: data_url_text(&format!("{}\n", c.token)),
            token: c.token,
            state: c.state,
            checked_words: c.checked_words,
            // The claim joins what the sandbox said with its own sentence; don't show "..".
            said: c.said.map(|w| tidy_stops(&w)),
            host,
        }
    }
}

// ---------- Account (O2.1, O2.3, recovery) ----------

/// Which account page.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AccountMode {
    /// Create the account: passkey first, then an email code, then password + authenticator.
    #[default]
    New,
    /// Sign in.
    Signin,
    /// Lost the passkey: a recovery code, or an email code with a hold.
    Recover,
}

/// [`crate::OnboardingPage::Account`]: `concept/journeys/owner-account.html` for the new
/// account; sign-in and recovery drawn in the same language (01b O2.1, O2.3, §4).
#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountPage {
    pub mode: AccountMode,
    /// Where to go once signed in: a path on this host, already checked.
    pub next: String,
    /// The site being kept, when the owner came from an assessment ("Keep kilele-bakery.example").
    pub host: Option<String>,
    /// `/account/signin?next=…` and the others, with `next` carried along.
    pub new_href: String,
    pub signin_href: String,
    pub recover_href: String,
    /// Someone is already signed in in this browser: the page says so instead of offering a
    /// form that would act on that account.
    pub signed_in: bool,
}

impl AccountPage {
    /// The page for `mode`, sending the owner to `next` afterwards. `next` must already be a
    /// safe local path (see [`crate::routes::safe_next`]).
    pub fn new(mode: AccountMode, next: &str, host: Option<String>) -> Self {
        let q = format!("?next={}", encode(next));
        let host_q = host
            .as_deref()
            .map(|h| format!("&host={}", encode(h)))
            .unwrap_or_default();
        Self {
            mode,
            next: next.to_owned(),
            new_href: format!("/account/new{q}{host_q}"),
            signin_href: format!("/account/signin{q}"),
            recover_href: format!("/account/recover{q}"),
            host,
            signed_in: false,
        }
    }
}

/// Percent-encodes a query value.
pub(crate) fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------- Setup: the Chief WebAgent introduces itself (O2.4) ----------

/// [`crate::OnboardingPage::Setup`]: the first page after the account, before the claim.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SetupPage {
    /// The site being set up, when known.
    pub host: Option<String>,
    /// What comes next, in order, each with where it leads.
    pub steps: Vec<SetupStep>,
    /// What the Chief WebAgent says comes next, in its own sentence.
    pub chief_words: String,
    /// The first step that isn't done.
    pub next_href: String,
    pub next_words: String,
}

/// One step of setting up.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SetupStep {
    pub n: u32,
    pub title: String,
    pub words: String,
    pub done: bool,
}

// ---------- The Move (O5.1 to O5.5) and go-live (O6.1, O6.2) ----------

pub use console::{MoveState, StepKey, StepState};

/// [`crate::OnboardingPage::Move`]: `concept/move.html` on the Move's real steps, rooms,
/// parity and left-behind items.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MovePage {
    pub site_id: String,
    pub host: String,
    pub fixture: bool,
    /// Where the new site answers once live (its platform address).
    pub live_url: Option<String>,
    pub move_id: String,
    pub state: MoveState,
    /// The h1: "Moving kilele-bakery.example", "kilele-bakery.example is live" …
    pub title: String,
    /// The state chip: its class and words.
    pub state_chip: Chip,
    pub started_words: String,
    pub progress_words: Option<String>,
    /// The eight stops: Read, Claim, Replay, Clean, Rebuild, Prove, Go live, Watch.
    pub rail: Vec<RailStop>,
    /// One card per step that runs in a sealed room.
    pub rooms: Vec<RoomCard>,
    pub tick: console::Tick,
    pub frame: Option<console::Frame>,
    pub parity: Option<ParityCard>,
    pub quarantine: console::QuarantineSummary,
    /// The go-live card, once the Mover raises it.
    pub card: Option<CardRef>,
    pub spend_words: Option<String>,
    pub uploads: Vec<console::UploadRow>,
    /// Once live, where the pictures drafted for pages that had none wait for the owner (W11's
    /// Pictures app), when any wait. Node sets it; `None` hides the line.
    pub pictures_href: Option<String>,
    /// The look the owner chose before the Move ("Choose your new look"), which the rebuild
    /// uses. Node sets it; `None` hides the line.
    pub look: Option<LookRef>,
}

/// The look chosen before the Move: its name, its direction in a sentence and where to change it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LookRef {
    pub name: String,
    pub sentence: String,
    pub href: String,
}

/// A chip: its class and words.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Chip {
    pub class: String,
    pub words: String,
}

/// One stop on the rail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RailStop {
    /// The step's key (`assess` … `golive`), or `watch`; live updates find the stop by it.
    pub key: String,
    pub name: String,
    /// "" (done), "run", "wait", "fail" or "todo".
    pub class: String,
    pub words: String,
    /// Where the stop's detail is on the page.
    pub anchor: String,
}

/// A sealed room the Move used, from its step.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RoomCard {
    pub key: String,
    pub title: String,
    pub what: String,
    pub network: String,
    pub summary: String,
    pub state: StepState,
    pub state_words: String,
    pub took_words: String,
    pub receipt: Option<String>,
    pub receipt_short: Option<String>,
}

/// Parity, as the concept's "Prove it matches" draws it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ParityCard {
    pub score_words: String,
    pub score_pct: u8,
    /// At or above the 95% bar.
    pub ok: bool,
    pub pages_words: String,
    pub pages: Vec<ParityRow>,
}

/// One page's parity.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ParityRow {
    pub url: String,
    pub chip: String,
    pub score_words: String,
    pub verdict: String,
    pub flagged: bool,
}

/// The go-live card's title and where to open it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CardRef {
    pub title: String,
    pub href: String,
}

/// The step's name on the wire (`StepEvent.step`), as the live island receives it.
fn step_key(k: StepKey) -> &'static str {
    match k {
        StepKey::Assess => "assess",
        StepKey::Claim => "claim",
        StepKey::Replay => "replay",
        StepKey::Clean => "clean",
        StepKey::Rebuild => "rebuild",
        StepKey::Prove => "prove",
        StepKey::Golive => "golive",
    }
}

fn step_words(s: StepState) -> &'static str {
    match s {
        StepState::Pending => "Not started",
        StepState::Running => "Now",
        StepState::Waiting => "Your decision",
        StepState::Done => "Done",
        StepState::Failed => "Stopped",
    }
}

fn rail_class(s: StepState) -> &'static str {
    match s {
        StepState::Done => "",
        StepState::Running => "run",
        StepState::Waiting => "wait",
        StepState::Failed => "fail",
        StepState::Pending => "todo",
    }
}

fn short_receipt(r: &str) -> String {
    let hex = r.split_once(':').map_or(r, |(_, h)| h);
    if hex.len() > 12 {
        format!("{}…{}", &hex[..4], &hex[hex.len() - 4..])
    } else {
        hex.to_owned()
    }
}

impl From<console::MoveView> for MovePage {
    fn from(v: console::MoveView) -> Self {
        let site = v.chrome.site;
        let host = site.host.clone();
        let state = v.run.state;
        let title = match state {
            MoveState::None => format!("Ready to move {host}"),
            MoveState::Running => format!("Moving {host}"),
            MoveState::Waiting => format!("{host} is ready to go live"),
            MoveState::Live => format!("{host} is live"),
            MoveState::Stopped => "The Move stopped".to_owned(),
        };
        let state_chip = match state {
            MoveState::None => ("", "Not started"),
            MoveState::Running => ("chip-info", "Moving"),
            MoveState::Waiting => ("chip-wait", "Waiting for you: go live"),
            MoveState::Live => ("chip-ok", "Live"),
            MoveState::Stopped => ("chip-risk", "Stopped"),
        };
        let mut rail: Vec<RailStop> = v
            .steps
            .iter()
            .map(|s| {
                let (name, anchor) = match s.key {
                    StepKey::Assess => ("Read", "#rooms"),
                    StepKey::Claim => ("Claim", "#rail"),
                    StepKey::Replay => ("Replay", "#rooms"),
                    StepKey::Clean => ("Clean", "#left-behind"),
                    StepKey::Rebuild => ("Rebuild", "#rooms"),
                    StepKey::Prove => ("Prove", "#prove"),
                    StepKey::Golive => ("Go live", "#golive"),
                };
                let words = match s.state {
                    StepState::Done if !s.took_words.is_empty() => s.took_words.clone(),
                    other => step_words(other).to_owned(),
                };
                RailStop {
                    key: step_key(s.key).into(),
                    name: name.into(),
                    class: rail_class(s.state).into(),
                    words,
                    anchor: anchor.into(),
                }
            })
            .collect();
        rail.push(RailStop {
            key: "watch".into(),
            name: "Watch".into(),
            class: if state == MoveState::Live {
                "run"
            } else {
                "todo"
            }
            .into(),
            words: "30 days".into(),
            anchor: "#golive".into(),
        });
        let rooms = v
            .steps
            .iter()
            .filter_map(|s| {
                let (title, what, network) = match s.key {
                    StepKey::Assess => (
                        "Read room",
                        "Reads your public pages the way a visitor's browser would.",
                        format!("{host} only"),
                    ),
                    StepKey::Replay => (
                        "Replay room",
                        "Your WordPress, rebuilt from its own export, so your content comes out without your password.",
                        "None".to_owned(),
                    ),
                    StepKey::Rebuild => (
                        "Build room",
                        "Your new pages and look, built from what was kept.",
                        "None".to_owned(),
                    ),
                    StepKey::Prove => (
                        "Render room",
                        "Photographs old and new pages so a vision check can compare them.",
                        "None".to_owned(),
                    ),
                    _ => return None,
                };
                Some(RoomCard {
                    key: step_key(s.key).into(),
                    title: title.into(),
                    what: what.into(),
                    network,
                    summary: s.summary.clone(),
                    state: s.state,
                    state_words: step_words(s.state).into(),
                    took_words: s.took_words.clone(),
                    receipt_short: s.receipt.as_deref().map(short_receipt),
                    receipt: s.receipt.clone(),
                })
            })
            .collect();
        let parity = v.parity.map(|p| ParityCard {
            ok: p.score_pct >= 95,
            pages_words: match p.pages.len() {
                1 => "1 page compared".into(),
                n => format!("{n} pages compared"),
            },
            pages: p
                .pages
                .into_iter()
                .map(|pg| ParityRow {
                    chip: match pg.state {
                        console::ParityState::Pass => "chip-ok",
                        console::ParityState::Retry => "chip-wait",
                        console::ParityState::Flagged => "chip-risk",
                    }
                    .into(),
                    flagged: pg.state == console::ParityState::Flagged,
                    url: pg.url,
                    score_words: pg.score_words,
                    verdict: pg.verdict,
                })
                .collect(),
            score_words: p.score_words,
            score_pct: p.score_pct,
        });
        Self {
            site_id: site.id,
            host,
            fixture: site.fixture,
            live_url: site.live_url,
            move_id: v.run.id,
            state,
            title,
            state_chip: Chip {
                class: state_chip.0.into(),
                words: state_chip.1.into(),
            },
            started_words: v.run.started_words,
            progress_words: v.run.progress_words,
            rail,
            rooms,
            tick: v.tick,
            frame: v.frame,
            parity,
            quarantine: v.quarantine,
            card: v.card.map(|c| CardRef {
                title: c.title,
                href: c.href,
            }),
            spend_words: v.spend_words,
            uploads: v.uploads,
            pictures_href: None,
            look: None,
        }
    }
}

// ---------- Plan and pay (O3.1, O3.2) ----------

/// A plan as the catalogue gives it (W3's `agentrix_billing::Plan`, field for field), so the
/// page reads prices rather than typing them.
#[derive(Debug, Clone, Default)]
pub struct PlanInput {
    pub id: String,
    pub name: String,
    /// `site` plans are shown to owners; `wholesale` ones never.
    pub kind: String,
    /// US cents a month.
    pub price_cents: i64,
    /// US cents for 12 months paid up front (W3's `Plan::annual_cents`: 12 months less the
    /// catalogue's discount); 0 when the plan isn't offered that way.
    pub annual_cents: i64,
    pub included_ai_micro_usd: i64,
    pub webagents: Vec<String>,
    pub includes: Vec<String>,
}

/// One plan card (owner.css `.o-plan`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlanCard {
    pub id: String,
    pub name: String,
    /// "$29".
    pub price_words: String,
    /// "$278.40": 12 months paid up front; empty when not offered.
    pub annual_words: String,
    /// "$348": 12 monthly charges, what the 12-month price is set against.
    pub year_words: String,
    /// "$69.60": what paying for 12 months up front saves.
    pub saving_words: String,
    /// Whole dollars of AI included a month, for the budget slider.
    pub included_usd: i64,
    pub includes: Vec<String>,
    /// "Chief, Concierge, Editor, Guardian and Caretaker".
    pub webagents_words: String,
}

/// [`crate::OnboardingPage::Plan`]: `concept/journeys/owner-plan.html`, PayPal only, in USD.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlanPage {
    pub site_id: String,
    pub host: Option<String>,
    pub plans: Vec<PlanCard>,
    /// The plan pressed when the page opens.
    pub chosen: String,
    /// `monthly` or `annual`: how often the owner pays, checked when the page opens.
    pub period: String,
    /// Whether 12 months up front is offered (the catalogue prices it).
    pub annual: bool,
    /// The catalogue's discount on 12 months up front, percent (W3's `annual_discount_percent`).
    pub discount_percent: i64,
    /// Where PayPal sends the owner back to.
    pub return_path: String,
    /// Where to go once the subscription is approved: the claim.
    pub next: String,
}

fn dollars(cents: i64) -> String {
    usd(cents)
}

/// "$29", "$278.40", "$1,430.40": US dollars with thousands separators, cents only when there
/// are some.
pub fn usd(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let cents = cents.abs();
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if cents % 100 == 0 {
        format!("{sign}${grouped}")
    } else {
        format!("{sign}${grouped}.{:02}", cents % 100)
    }
}

fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl PlanPage {
    /// The page for `site`, from the catalogue's plans: owners see only `site` plans, cheapest
    /// first, with Commerce pressed when it's there (the concept's suggestion for a shop).
    pub fn new(site_id: &str, host: Option<String>, plans: &[PlanInput], next: &str) -> Self {
        let mut cards: Vec<(i64, PlanCard)> = plans
            .iter()
            .filter(|p| p.kind == "site")
            .map(|p| {
                (
                    p.price_cents,
                    PlanCard {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        price_words: dollars(p.price_cents),
                        annual_words: if p.annual_cents > 0 {
                            usd(p.annual_cents)
                        } else {
                            String::new()
                        },
                        year_words: usd(p.price_cents * 12),
                        saving_words: usd((p.price_cents * 12 - p.annual_cents).max(0)),
                        included_usd: p.included_ai_micro_usd / 1_000_000,
                        includes: p.includes.clone(),
                        webagents_words: and_list(&p.webagents),
                    },
                )
            })
            .collect();
        cards.sort_by_key(|(c, _)| *c);
        let plans: Vec<PlanCard> = cards.into_iter().map(|(_, c)| c).collect();
        let chosen = plans
            .iter()
            .find(|p| p.id == "commerce")
            .or(plans.last())
            .map(|p| p.id.clone())
            .unwrap_or_default();
        Self {
            site_id: site_id.to_owned(),
            host,
            annual: plans.iter().any(|p| !p.annual_words.is_empty()),
            plans,
            chosen,
            period: agentrix_billing::Period::Monthly.as_str().to_owned(),
            discount_percent: agentrix_billing::catalogue().annual_discount_percent,
            return_path: format!(
                "/setup/plan/return?site={}&next={}",
                encode(site_id),
                encode(next)
            ),
            next: next.to_owned(),
        }
    }
}

// ---------- Pay: the PayPal subscription, approved just before go-live (O3.2) ----------

/// Where the owner's plan payment stands.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PayState {
    /// No plan chosen yet.
    #[default]
    NoPlan,
    /// A plan is chosen; PayPal isn't approved yet.
    Ready,
    /// Waiting for the owner on PayPal's page.
    Pending,
    /// Approved on PayPal.
    Active,
    /// PayPal isn't switched on for this deployment.
    Off,
}

/// [`crate::OnboardingPage::Pay`]: the plan's PayPal subscription, in owner-plan.html's look.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PayPage {
    pub site_id: String,
    pub state: PayState,
    /// "Commerce".
    pub plan_name: String,
    /// "$149".
    pub price_words: String,
    /// What one charge is, with how often: "$149 a month" or "$1,430.40 for 12 months".
    pub charge_words: String,
    /// `monthly` or `annual`.
    pub period: String,
    /// "$30".
    pub budget_words: String,
    /// What PayPal or the gateway said, in words, when something didn't work.
    pub said: Option<String>,
    /// PayPal's page, while pending.
    pub approve_url: Option<String>,
    /// Choose (or change) the plan.
    pub plan_href: String,
    /// Back to where the owner came from: the Move.
    pub next: String,
}

// ---------- Billing and ownership (O9.1 to O9.4) ----------

/// [`crate::OnboardingPage::Billing`]: `concept/journeys/owner-billing.html` on W3's billing
/// and W6's Effects Gateway.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BillingPage {
    pub site_id: String,
    pub state_chip: Chip,
    /// "Commerce · $149 a month · PayPal subscription"; empty with no plan.
    pub summary: String,
    pub has_plan: bool,
    /// "Monthly" or "12 months, 20% off".
    pub period_words: String,
    /// "Mon 7 Dec 2026, $149": the next charge's day and amount, once PayPal has charged.
    pub next_charge: Option<String>,
    /// Paying the other way: what it costs and when it starts.
    pub switch: Option<PeriodSwitch>,
    /// "$3.40" of "$30" this period, when the gateway has counted it.
    pub spent_words: Option<String>,
    pub budget_words: Option<String>,
    /// Rounded to the meter's 5% steps.
    pub spent_pct: u8,
    pub plans: Vec<PlanCard>,
    pub current_plan: String,
    /// The cancel decision: what the owner's passkey or code signs.
    pub cancel: Option<CancelCard>,
    /// After a cancel: what happened, in words.
    pub said: Option<String>,
    pub plan_href: String,
}

/// Switching between monthly and 12 months up front (applies at the next renewal).
#[derive(Debug, Clone, Default, Serialize)]
pub struct PeriodSwitch {
    /// `annual` or `monthly`: what the button switches to.
    pub to: String,
    /// "Pay for 12 months at a time: $1,430.40 once a year instead of $1,788, so you save $357.60."
    pub words: String,
    /// "Switch to 12 months".
    pub button: String,
    /// On a live plan whose switch isn't agreed at PayPal yet: the period to post to open
    /// PayPal's page again.
    pub agree_to: Option<String>,
    /// "From your next renewal, Mon 7 Dec 2026, you pay $1,430.40 for 12 months.", once asked.
    pub pending: Option<String>,
}

/// The cancel card (owner-billing.html's "Cancel" decision).
#[derive(Debug, Clone, Default, Serialize)]
pub struct CancelCard {
    pub title: String,
    /// What the approval signs, shown as text.
    pub sign_words: String,
    /// The fingerprint, hex.
    pub fingerprint: String,
    /// `navigator.credentials.get` options as JSON, or none (then a code).
    pub passkey_options: Option<String>,
}

// ---------- Where should your WebAgents reach you? (after go-live) ----------

/// Which channels W13's Channels app can connect today. Node fills this; a channel that isn't
/// ready shows "Not available yet" and no button. Email is always on and isn't listed here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelOffer {
    /// Notifications on this device (web push: "a decision is waiting", nothing more).
    pub device: bool,
    pub telegram: bool,
    pub whatsapp: bool,
    pub slack: bool,
    pub discord: bool,
    pub matrix: bool,
}

/// One channel in the step.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReachChannel {
    /// `device`, `email`, `telegram`, `whatsapp`, `slack`, `discord`, `matrix`; also the
    /// Channels app's `?connect=` value.
    pub id: String,
    pub name: String,
    /// One line on what it does there.
    pub words: String,
    /// `on` (email), `connect` or `soon`.
    pub state: String,
    /// `you` or `team`.
    pub group: String,
}

impl ChannelOffer {
    /// The offer from the channel ids the Channels app can connect for a site
    /// (`agentrix_channels::connectable`); unknown ids are ignored.
    pub fn from_ready(ids: &[String]) -> Self {
        let has = |id: &str| ids.iter().any(|i| i == id);
        Self {
            device: has("device"),
            telegram: has("telegram"),
            whatsapp: has("whatsapp"),
            slack: has("slack"),
            discord: has("discord"),
            matrix: has("matrix"),
        }
    }

    /// The step's list, in order: for you, then for your team.
    pub fn channels(self) -> Vec<ReachChannel> {
        let c =
            |id: &str, name: &str, words: &str, ready: Option<bool>, group: &str| ReachChannel {
                id: id.into(),
                name: name.into(),
                words: words.into(),
                state: match ready {
                    None => "on",
                    Some(true) => "connect",
                    Some(false) => "soon",
                }
                .into(),
                group: group.into(),
            };
        vec![
            c(
                "device",
                "This device",
                "A notification that a decision is waiting. Nothing else is in it.",
                Some(self.device),
                "you",
            ),
            c(
                "email",
                "Email",
                "Your morning brief and receipts. Always on, whatever else you add.",
                None,
                "you",
            ),
            c(
                "telegram",
                "Telegram",
                "Your morning brief, decisions waiting and questions your desks pass on. Approving opens the console.",
                Some(self.telegram),
                "you",
            ),
            c(
                "whatsapp",
                "WhatsApp",
                "Your morning brief, decisions waiting and questions your desks pass on. Approving opens the console.",
                Some(self.whatsapp),
                "you",
            ),
            c(
                "slack",
                "Slack",
                "Your morning brief and decisions waiting, posted in your team's channel.",
                Some(self.slack),
                "team",
            ),
            c(
                "discord",
                "Discord",
                "Your morning brief and decisions waiting, posted in your team's server.",
                Some(self.discord),
                "team",
            ),
            c(
                "matrix",
                "Matrix",
                "Your morning brief and decisions waiting, posted in your team's room.",
                Some(self.matrix),
                "team",
            ),
        ]
    }
}
