//! Turns [`Facts`] and the catalogue into the words and numbers the templates print.
//!
//! Every number on the page passes through here from a fact. A missing fact becomes `None`,
//! and the template says what will appear and when.

use chrono::{DateTime, FixedOffset, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::facts::{Facts, LiveMove};
use crate::marks::{Marks, TeamPicture};

/// East Africa Time, which Zavora and the submission use.
fn eat() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).expect("+03:00 is a valid offset")
}

/// "Sat 7 Nov 2026".
pub fn date(t: DateTime<Utc>) -> String {
    t.with_timezone(&eat()).format("%a %-d %b %Y").to_string()
}

/// "Sat 7 Nov, 23:58 EAT".
pub fn date_time(t: DateTime<Utc>) -> String {
    t.with_timezone(&eat())
        .format("%a %-d %b, %H:%M EAT")
        .to_string()
}

/// "1,204".
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "12.4 s", "2 min 8 s".
pub fn seconds(s: f64) -> String {
    if s < 60.0 {
        format!("{s:.1} s")
    } else {
        let whole = s.round() as u64;
        let (m, rest) = (whole / 60, whole % 60);
        if rest == 0 {
            format!("{m} min")
        } else {
            format!("{m} min {rest} s")
        }
    }
}

/// "$0.37" from millionths of a dollar.
pub fn usd(micro: i64) -> String {
    format!("${:.2}", micro as f64 / 1_000_000.0)
}

/// The receipt page for a digest.
fn receipt_href(digest: &str) -> String {
    format!("/receipts/{digest}")
}

fn step_label(step: &str) -> &'static str {
    match step {
        "assess" => "Assess",
        "claim" => "Claim",
        "replay" => "Replay",
        "clean" => "Clean",
        "rebuild" => "Rebuild",
        "prove" => "Prove",
        "golive" => "Go live",
        _ => "Step",
    }
}

fn provider_name(p: &str) -> String {
    match p {
        "vultr" => "Vultr Serverless Inference".into(),
        "anthropic" => "Anthropic's Claude".into(),
        other => other.to_owned(),
    }
}

/// The parity a prove summary states, as "98.8%": "Parity 98.8%; release 3 is ready for you".
fn parity_of(summary: &str) -> Option<String> {
    let rest = summary.split("Parity ").nth(1)?;
    let number: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    (!number.is_empty() && rest[number.len()..].starts_with('%')).then(|| format!("{number}%"))
}

/// One tile in the live strip.
#[derive(Debug, Serialize)]
pub struct Stat {
    /// The number, or `None` when it can't be read now.
    pub value: Option<String>,
    /// What it counts.
    pub label: &'static str,
    /// Where it comes from, or what will appear and when.
    pub note: String,
    /// A receipt or page that backs it.
    pub href: Option<String>,
}

/// The gauntlet card.
#[derive(Debug, Serialize)]
pub struct GauntletView {
    /// "10/10", when a run has finished.
    pub score: Option<String>,
    /// When the run finished.
    pub when: Option<String>,
    /// The run's receipt.
    pub href: Option<String>,
    /// Each attack and how it ended, from the run's detail.
    pub attacks: Vec<AttackView>,
}

/// One attack.
#[derive(Debug, Serialize)]
pub struct AttackView {
    /// Its short name.
    pub name: String,
    /// Whether the platform held.
    pub held: bool,
    /// Its receipt.
    pub href: Option<String>,
}

/// The first live Move.
#[derive(Debug, Serialize)]
pub struct MoveView {
    /// The site that moved.
    pub site_name: String,
    /// "Sat 7 Nov 2026".
    pub live_on: String,
    /// The steps that ran in sealed rooms, timed.
    pub steps: Vec<StepView>,
    /// The sealed steps' time together.
    pub sealed_total: Option<String>,
    /// Model cost of the whole Move.
    pub cost: Option<String>,
    /// Who served its model calls, in words.
    pub providers: Option<String>,
}

/// One timed step.
#[derive(Debug, Serialize)]
pub struct StepView {
    /// "Replay".
    pub label: &'static str,
    /// "12.4 s".
    pub time: String,
    /// The step's own summary.
    pub summary: String,
    /// Its receipt.
    pub href: Option<String>,
}

/// The go-live card and its proof.
#[derive(Debug, Serialize)]
pub struct GoLiveView {
    /// The site the card is for, when a Move has gone live.
    pub site_name: Option<String>,
    /// "98.8%, target 95%" or the rule.
    pub parity: String,
    /// "3 items, with reasons" or the rule.
    pub left_behind: String,
    /// "41.2 s", measured from approval to live.
    pub to_live: Option<String>,
    /// "a passkey" or "an authenticator code".
    pub approved_with: Option<&'static str>,
}

/// Which provider serves model calls.
#[derive(Debug, Serialize)]
pub struct RoutingView {
    /// "Vultr Serverless Inference".
    pub provider: String,
    /// Whether that's Vultr.
    pub on_vultr: bool,
    /// When the receipt was issued.
    pub since: String,
    /// The receipt.
    pub href: String,
    /// Each purpose's model and who serves it.
    pub routes: Vec<RouteView>,
}

/// One purpose's route.
#[derive(Debug, Serialize)]
pub struct RouteView {
    /// "build".
    pub purpose: String,
    /// The model.
    pub model: String,
    /// Whether Vultr serves it.
    pub on_vultr: bool,
}

/// The phone's brief.
#[derive(Debug, Serialize)]
pub struct BriefView {
    /// The site.
    pub site_name: String,
    /// When it was written.
    pub when: String,
    /// The opening line.
    pub headline: Option<String>,
    /// Its first lines.
    pub lines: Vec<String>,
}

/// A sample site, from `data/samples.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sample {
    /// `kilele`, `classy`, `tana`: its colours and marks image.
    pub id: String,
    /// The invented business.
    pub name: String,
    /// What it is and where.
    pub tag: String,
    /// Its host's first label: `kilele-bakery-sample`.
    pub slug: String,
    /// What a visitor can do there.
    pub line: String,
    /// One thing a visitor can check on the live sample (screen 4).
    pub try_this: String,
    /// A walkthrough: what to open, ask and try, in order.
    #[serde(default)]
    pub walk: Vec<String>,
}

/// A sample on the page.
#[derive(Debug, Serialize)]
pub struct SampleView {
    /// From the catalogue.
    #[serde(flatten)]
    pub sample: Sample,
    /// Its host: `kilele-bakery-sample.agentrix.website`.
    pub host: String,
    /// Its address, while it answers.
    pub url: Option<String>,
    /// Its WebAgents' living marks.
    pub team: Option<TeamPicture>,
    /// Its hero picture from W11's library, labelled; the card shows a screenshot without one.
    pub picture: Option<crate::facts::Picture>,
}

/// A site plan's card, from W3's catalogue (`agentrix_billing::catalogue()`).
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    /// Stable id.
    pub id: String,
    /// "Essential".
    pub name: String,
    /// "$29".
    pub price: String,
    /// The same, in cents, for "Plans start at".
    #[serde(skip)]
    pub cents: i64,
    /// "month".
    pub interval: String,
    /// What it includes, in the catalogue's words.
    pub points: Vec<String>,
    /// The WebAgents on it, each with its one line from W1's roster.
    pub webagents: Vec<PlanAgent>,
    /// The same, with the coming ones, in groups: runs the team, customers, revenue, marketing,
    /// operations.
    pub groups: Vec<PlanGroup>,
    /// The launch offer: "USD 50 of AI in your first month".
    pub first_month: Option<String>,
}

/// One WebAgent a plan includes.
#[derive(Debug, Clone, Serialize)]
pub struct PlanAgent {
    /// "Concierge".
    pub name: String,
    /// What it looks after, as the roster says it: "Answers visitors on your site".
    pub role: String,
    /// Announced, not built yet.
    pub soon: bool,
}

/// A plan's WebAgents of one kind.
#[derive(Debug, Clone, Serialize)]
pub struct PlanGroup {
    /// "Revenue".
    pub name: &'static str,
    /// Its WebAgents.
    pub agents: Vec<PlanAgent>,
}

/// The group a WebAgent belongs to on the plan cards.
fn group_of(name: &str) -> &'static str {
    match name {
        "Chief" => "Runs the team",
        "Concierge" => "Customers",
        "Sales" | "Cashier" | "Reputation" | "Loyalty" => "Revenue",
        "Editor" | "Marketing" | "Advertising" | "Promotions" => "Marketing",
        _ => "Operations",
    }
}

/// A plan's WebAgents, built and coming, in the cards' groups.
fn plan_groups(built: &[PlanAgent], soon: &[agentrix_billing::PlannedWebAgent]) -> Vec<PlanGroup> {
    let all: Vec<PlanAgent> = built
        .iter()
        .cloned()
        .chain(soon.iter().map(|s| PlanAgent {
            name: s.name.clone(),
            role: s.role.clone(),
            soon: true,
        }))
        .collect();
    [
        "Runs the team",
        "Customers",
        "Revenue",
        "Marketing",
        "Operations",
    ]
    .into_iter()
    .filter_map(|g| {
        let agents: Vec<PlanAgent> = all
            .iter()
            .filter(|a| group_of(&a.name) == g)
            .cloned()
            .collect();
        (!agents.is_empty()).then_some(PlanGroup { name: g, agents })
    })
    .collect()
}

/// The plan's WebAgents with their roster lines; a name the roster doesn't know is left out.
fn plan_agents(names: &[String]) -> Vec<PlanAgent> {
    let Some(look) = agentrix_kernel::looks::sample("kilele") else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(|n| {
            // The catalogue says "Chief"; the roster's id for it is `coordinator`.
            let id = match n.as_str() {
                "Chief" => "coordinator".to_owned(),
                other => other.to_lowercase(),
            };
            let spec = agentrix_kernel::roster::template(&id, &look)?;
            Some(PlanAgent {
                name: n.clone(),
                role: spec.role_words,
                soon: false,
            })
        })
        .collect()
}

/// The agencies' price per client site, from the same catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct Wholesale {
    /// "$15".
    pub price: String,
    /// "month".
    pub interval: String,
    /// The fewest client sites billed.
    pub minimum_sites: Option<u32>,
}

/// "$29", or "$29.50" when there are cents.
fn dollars(cents: i64) -> String {
    let (whole, rest) = (cents / 100, cents % 100);
    let whole = count(u64::try_from(whole).unwrap_or(0));
    if rest == 0 {
        format!("${whole}")
    } else {
        format!("${whole}.{rest:02}")
    }
}

/// The site plans as cards and the agencies' wholesale price, in the catalogue's order.
pub fn plans(catalogue: &agentrix_billing::Catalogue) -> (Vec<Plan>, Option<Wholesale>) {
    let cards = catalogue
        .plans
        .iter()
        .filter(|p| p.kind == "site")
        .map(|p| Plan {
            id: p.id.clone(),
            name: p.name.clone(),
            price: dollars(p.price_cents),
            cents: p.price_cents,
            interval: p.interval.clone(),
            points: p.includes.clone(),
            webagents: plan_agents(&p.webagents),
            groups: plan_groups(&plan_agents(&p.webagents), &p.webagents_soon),
            first_month: (p.first_month_ai_micro_usd > p.included_ai_micro_usd).then(|| {
                format!(
                    "USD {} of AI in your first month",
                    count(u64::try_from(p.first_month_ai_micro_usd / 1_000_000).unwrap_or(0))
                )
            }),
        })
        .collect();
    let agency = catalogue
        .plans
        .iter()
        .find(|p| p.kind == "wholesale")
        .map(|p| Wholesale {
            price: dollars(p.price_cents),
            interval: p.interval.clone(),
            minimum_sites: p.minimum_sites,
        });
    (cards, agency)
}

/// Links that exist only once James publishes them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Links {
    /// The submission video.
    pub video: Option<String>,
    /// The public repository.
    pub code: Option<String>,
}

/// Everything `home.html` prints.
#[derive(Debug, Serialize)]
pub struct HomeView {
    /// `https://agentrix.website`, for canonical and Open Graph URLs.
    pub origin: String,
    /// The static assets' version stamp.
    pub v: &'static str,
    /// The UI kit's version stamp, for its fonts.
    pub ui_v: &'static str,
    /// Where the paste box posts (W4's Assess).
    pub assess_action: String,
    /// "Try it on Zavora's fixture", when the fixture is configured.
    pub fixture_href: Option<String>,
    /// The live strip.
    pub stats: Vec<Stat>,
    /// When the facts were read.
    pub read_at: Option<String>,
    /// The gauntlet card.
    pub gauntlet: GauntletView,
    /// The first live Move.
    pub first_move: Option<MoveView>,
    /// The go-live card.
    pub golive: GoLiveView,
    /// Model routing now.
    pub routing: Option<RoutingView>,
    /// The phone's brief.
    pub brief: Option<BriefView>,
    /// The three samples.
    pub samples: Vec<SampleView>,
    /// Plans in USD.
    pub plans: Vec<Plan>,
    /// The agencies' price per client site.
    pub agency: Option<Wholesale>,
    /// Video and code.
    pub links: Links,
    /// The hero's cut and the film, when they're on node's disk.
    pub film: crate::film::FilmView,
    /// "Rather talk to a person?", when James has published a number and hours.
    pub contact: Option<Contact>,
    /// Which page: `home`, `how`, `samples` or `prices` (the header marks its desk).
    pub page: String,
    /// The judges' page, on the money host under the platform's (`money.<platform>/judges`).
    pub judges_url: String,
    /// Its Verify box: paste any agent's DID, see every check.
    pub verify_url: String,
    /// Screen 2: agentrix.website's own team, each with a state read from its own records.
    pub team: Vec<TeamCard>,
    /// Screen 6: "USD 29", the lowest plan in W3's catalogue at render time.
    pub from_price: Option<String>,
    /// The discount on a plan paid 12 months up front, percent, from W3's catalogue
    /// (`agentrix_billing::catalogue().annual_discount_percent`): "a 12-month plan comes with
    /// a 20% discount".
    pub annual_discount_percent: i64,
    /// Screen 5: how many of the last self-attack run's attacks were stopped, and of how many.
    pub held: Option<(u64, u64)>,
    /// Whether the desks answer with a model (the header and screen 2 then say AI).
    pub desks_ai: bool,
    /// The four front desks, for their pages' ask boxes.
    pub front_desks: Vec<crate::desks::Desk>,
    /// Screen 7's newsletter block: the last issue sent, and a draft waiting for James.
    pub issues: crate::newsletter::Issues,
}

/// One of agentrix.website's WebAgents on screen 2.
#[derive(Debug, Serialize)]
pub struct TeamCard {
    /// "Your site".
    pub name: &'static str,
    /// Its role mark, `/_home/marks/role-{mark}.svg`.
    pub mark: &'static str,
    /// Front of house or back office.
    pub side: &'static str,
    /// Its one job.
    pub job: &'static str,
    /// What it's doing now, from records; "Idle" when it isn't doing anything.
    pub state: String,
    /// How it works today, so a rule is never called AI.
    pub runs_on: &'static str,
    /// The desk a visitor can ask in place (`/_home/peek/{desk}`), for the four that talk.
    pub desk: Option<&'static str>,
    /// One thing a visitor can do with it, words and a link: a desk's own page (its ask box,
    /// without JavaScript), or what to do about a back-office WebAgent's work.
    pub action: Option<(&'static str, &'static str)>,
}

/// What the keep step has recorded, for the Sales WebAgent's state.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeptCounts {
    /// Reports people asked to keep, in the last 30 days.
    pub asked: u64,
    /// Of those, sent through W9's outbox.
    pub sent: u64,
}

fn team(
    facts: &Facts,
    gauntlet: &GauntletView,
    kept: KeptCounts,
    desks_ai: bool,
    answered: &std::collections::BTreeMap<String, crate::desks::Answered>,
    (issues, readers): (&crate::newsletter::Issues, u64),
    chief_brief: Option<DateTime<Utc>>,
) -> Vec<TeamCard> {
    let editor = if issues.waiting {
        "This week's fix is waiting for James's OK.".to_owned()
    } else if let Some(t) = &issues.last_sent {
        format!("Last fix sent: {t}.")
    } else if readers > 0 {
        format!(
            "Drafts on Thursdays for {} {}.",
            count(readers),
            if readers == 1 { "reader" } else { "readers" }
        )
    } else {
        "Starts when the first reader confirms.".to_owned()
    };
    let held = held(facts);
    // A desk's state: how many questions it answered today, from its own counts.
    let today = |id: &str| match answered.get(id).map_or(0, |a| a.all) {
        0 => "Idle.".to_owned(),
        1 => "Answered 1 question today.".to_owned(),
        n => format!("Answered {} questions today.", count(n)),
    };
    let runs = |rules: &'static str| {
        if desks_ai {
            "AI · this site's pages"
        } else {
            rules
        }
    };
    let your_site = match facts.reports_written {
        Some(0) | None => "No sites checked yet.".to_owned(),
        Some(1) => "1 report written so far.".to_owned(),
        Some(n) => format!("{} reports written so far.", count(n)),
    };
    let guardian = match (&gauntlet.when, held) {
        (Some(when), Some((h, of))) => format!("Last self-attack {when}: {h} of {of} stopped."),
        _ => "The Guardian hasn't attacked this site yet.".to_owned(),
    };
    let sales = match kept {
        KeptCounts { asked: 0, .. } => "Idle. Nobody has asked for a report yet.".to_owned(),
        KeptCounts { asked, sent: 0 } => {
            format!("{asked} asked for. None sent yet.")
        }
        KeptCounts { asked, sent } => format!("{sent} of {asked} reports sent."),
    };
    vec![
        TeamCard {
            name: "Your site",
            mark: "mover",
            side: "front",
            job: "Checks any site for free, from public pages only.",
            state: match answered.get("your-site").map_or(0, |a| a.all) {
                0 => your_site,
                _ => format!("{your_site} {}", today("your-site")),
            },
            runs_on: if desks_ai {
                "Rules · AI answers"
            } else {
                "Rules"
            },
            desk: Some("your-site"),
            action: Some(("Ask Your site", "#check")),
        },
        TeamCard {
            name: "How it works",
            mark: "concierge",
            side: "front",
            job: "Shows how the move works, step by step.",
            state: today("how-it-works"),
            runs_on: runs("This site's pages"),
            desk: Some("how-it-works"),
            action: Some(("Ask How it works", "/how-it-works")),
        },
        TeamCard {
            name: "Samples",
            mark: "sales",
            side: "front",
            job: "Opens the three sample sites and the films.",
            state: today("samples"),
            runs_on: runs("This site's pages"),
            desk: Some("samples"),
            action: Some(("Ask Samples", "/samples")),
        },
        TeamCard {
            name: "Prices",
            mark: "cashier",
            side: "front",
            job: "Shows the price list. Never guesses a price.",
            state: today("prices"),
            runs_on: runs("The price list"),
            desk: Some("prices"),
            action: Some(("Ask Prices", "/prices")),
        },
        TeamCard {
            name: "Guardian",
            mark: "guardian",
            side: "back",
            job: "Attacks our own system after every update.",
            state: guardian,
            runs_on: "Rules",
            desk: None,
            action: Some(("See the status page", "/status")),
        },
        TeamCard {
            name: "Sales",
            mark: "sales",
            side: "back",
            job: "Sends reports people ask for. Messages nobody else.",
            state: sales,
            runs_on: "Email",
            desk: None,
            action: Some(("Check your site and keep the report", "#check")),
        },
        TeamCard {
            name: "Editor",
            mark: "editor",
            side: "back",
            job: "Drafts one website fix a week. James checks each one.",
            state: editor,
            runs_on: if desks_ai {
                "AI · James approves"
            } else {
                "James approves"
            },
            desk: None,
            action: Some(("Get the weekly fix", "#start")),
        },
        TeamCard {
            name: "Chief",
            mark: "coordinator",
            side: "back",
            job: "Briefs James on this site's day. Holds what needs a person.",
            state: match chief_brief {
                Some(t) => format!("Wrote James's brief {}.", date_time(t)),
                None => "Not set up for this site yet.".into(),
            },
            runs_on: if chief_brief.is_some() {
                "This site's records"
            } else {
                "Not running yet"
            },
            desk: None,
            action: None,
        },
    ]
}

/// Stopped and in all, from the last self-attack run's own record.
fn held(facts: &Facts) -> Option<(u64, u64)> {
    facts
        .gauntlet
        .as_ref()
        .map(|g| (u64::from(g.score), u64::from(g.out_of)))
}

/// A sample's address on this platform: the platform origin's scheme and port with the
/// sample's host, so a local node (`http://localhost:18050`) links `http://kilele-bakery-sample.localhost:18050/`.
pub fn site_url(origin: &str, host: &str) -> String {
    let (scheme, rest) = origin.split_once("://").unwrap_or(("https", origin));
    let authority = rest.split('/').next().unwrap_or(rest);
    let port = authority
        .rsplit_once(':')
        .filter(|(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        .map(|(_, p)| p)
        .filter(|p| !matches!((scheme, *p), ("https", "443") | ("http", "80")));
    match port {
        Some(p) => format!("{scheme}://{host}:{p}/"),
        None => format!("{scheme}://{host}/"),
    }
}

/// A person to talk to on WhatsApp, with the hours James has published.
#[derive(Debug, Clone, Serialize)]
pub struct Contact {
    /// `https://wa.me/254…?text=…`, with a first line already written.
    pub href: String,
    /// "6 a.m. to 11 p.m. EAT": the published hours, written the site's one way.
    pub hours: String,
    /// The hour (EAT, 0–23) replies start and stop, read from `hours` when it says so.
    #[serde(skip)]
    pub opens: Option<(u32, u32)>,
    /// Whether it's within those hours now (set when the page is drawn).
    pub open_now: bool,
    /// "We reply from 6 a.m. EAT.", for outside those hours.
    pub reply_from: Option<String>,
}

/// "6am", "6 a.m." or "6 AM" → 6; "11pm" or "11 p.m." → 23; "12am" → 0.
fn hour(word: &str) -> Option<u32> {
    let w: String = word
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .collect::<String>()
        .to_ascii_lowercase();
    let (n, pm) = if let Some(n) = w.strip_suffix("am") {
        (n, false)
    } else {
        (w.strip_suffix("pm")?, true)
    };
    let h: u32 = n.trim().parse().ok()?;
    (1..=12)
        .contains(&h)
        .then(|| (h % 12) + if pm { 12 } else { 0 })
}

/// "11 p.m. EAT" or "11pm EAT" → (23, "EAT"): the hour, then what follows it.
fn hour_then(text: &str) -> Option<(u32, String)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    (1..=words.len().min(2)).find_map(|n| Some((hour(&words[..n].concat())?, words[n..].join(" "))))
}

/// 6 → "6 a.m.", 23 → "11 p.m.", 0 → "12 a.m.": the one way the site writes an hour,
/// as the films do.
fn clock(h: u32) -> String {
    match h {
        0 => "12 a.m.".into(),
        1..=11 => format!("{h} a.m."),
        12 => "12 p.m.".into(),
        _ => format!("{} p.m.", h - 12),
    }
}

impl Contact {
    /// From `AGENTRIX_HOME_WHATSAPP` (the number, digits with an optional +) and
    /// `AGENTRIX_HOME_WHATSAPP_HOURS` ("6 a.m. to 11 p.m. EAT", or "6am to 11pm EAT"); none
    /// unless both are set.
    pub fn from_env() -> Option<Self> {
        let number = std::env::var("AGENTRIX_HOME_WHATSAPP").ok()?;
        let hours = std::env::var("AGENTRIX_HOME_WHATSAPP_HOURS").ok()?;
        Self::new(&number, &hours)
    }

    /// A contact for `number`, or none if it isn't a phone number or the hours are blank.
    pub fn new(number: &str, hours: &str) -> Option<Self> {
        let digits: String = number.chars().filter(char::is_ascii_digit).collect();
        let hours = hours.trim();
        if !(9..=15).contains(&digits.len()) || hours.is_empty() {
            return None;
        }
        let read = hours.split_once(" to ").and_then(|(a, b)| {
            let (close, rest) = hour_then(b)?;
            Some((hour(a)?, close, rest))
        });
        let opens = read.as_ref().map(|(o, c, _)| (*o, *c));
        // Hours the parser reads are written back the site's one way; anything else as published.
        let shown = match &read {
            Some((o, c, rest)) => format!(
                "{} to {} {}",
                clock(*o),
                clock(*c),
                if rest.is_empty() { "EAT" } else { rest }
            ),
            None => hours.to_owned(),
        };
        Some(Self {
            href: format!(
                "https://wa.me/{digits}?text=Hi%2C%20I%27d%20like%20to%20talk%20about%20my%20website."
            ),
            hours: shown,
            reply_from: opens.map(|(o, _)| format!("We reply from {} EAT.", clock(o))),
            opens,
            open_now: true,
        })
    }

    /// The contact as it reads at `now`: within the published hours or not (EAT).
    pub fn at(&self, now: DateTime<Utc>) -> Self {
        let mut c = self.clone();
        if let Some((open, close)) = self.opens {
            let h = (now + chrono::Duration::hours(3)).hour();
            c.open_now = if open <= close {
                h >= open && h < close
            } else {
                h >= open || h < close
            };
        }
        c
    }
}

/// What the page needs besides the facts.
pub struct Inputs<'a> {
    /// The platform's origin.
    pub origin: &'a str,
    /// Where the paste box posts.
    pub assess_action: &'a str,
    /// The fixture link, if configured.
    pub fixture_href: Option<&'a str>,
    /// The samples.
    pub samples: &'a [Sample],
    /// The plans.
    pub plans: &'a [Plan],
    /// The agencies' price.
    pub agency: Option<&'a Wholesale>,
    /// Video and code.
    pub links: &'a Links,
    /// The living marks.
    pub marks: &'a Marks,
    /// The platform's host, under which the samples live.
    pub platform_host: &'a str,
    /// The sample hosts that answered the last check.
    pub samples_open: &'a [String],
    /// The hero's cut and the film.
    pub film: crate::film::FilmView,
    /// The person to talk to, if published.
    pub contact: Option<&'a Contact>,
    /// What the keep step has recorded.
    pub kept: KeptCounts,
    /// Whether the desks have a model to write with (only then do they say AI).
    pub desks_ai: bool,
    /// Questions each desk answered today.
    pub answered: std::collections::BTreeMap<String, crate::desks::Answered>,
    /// The weekly fix: the last issue sent, and whether a draft waits for James.
    pub issues: crate::newsletter::Issues,
    /// Readers who confirmed the weekly fix.
    pub readers: u64,
    /// When the Chief last wrote James's brief for agentrix.website (W1's `briefs` row).
    pub chief_brief: Option<DateTime<Utc>>,
}

const SEALED: [&str; 4] = ["replay", "clean", "rebuild", "prove"];

fn move_view(m: &LiveMove) -> MoveView {
    let sealed: Vec<_> = m
        .steps
        .iter()
        .filter(|s| SEALED.contains(&s.step.as_str()))
        .collect();
    let total: f64 = sealed.iter().map(|s| s.seconds).sum();
    MoveView {
        site_name: m.site_name.clone(),
        live_on: date(m.live_at),
        steps: sealed
            .iter()
            .map(|s| StepView {
                label: step_label(&s.step),
                time: seconds(s.seconds),
                summary: s.summary.clone(),
                href: s.receipt.as_deref().map(receipt_href),
            })
            .collect(),
        sealed_total: (!sealed.is_empty()).then(|| seconds(total)),
        cost: m.cost_micro_usd.map(usd),
        providers: (!m.providers.is_empty()).then(|| {
            m.providers
                .iter()
                .map(|p| provider_name(p))
                .collect::<Vec<_>>()
                .join(" and ")
        }),
    }
}

fn golive_view(m: Option<&LiveMove>) -> GoLiveView {
    let parity = m
        .and_then(|m| m.steps.iter().find(|s| s.step == "prove"))
        .and_then(|s| parity_of(&s.summary));
    GoLiveView {
        site_name: m.map(|m| m.site_name.clone()),
        parity: match parity {
            Some(p) => format!("{p}, target 95%"),
            None => "At least 95%, page by page".into(),
        },
        left_behind: match m.and_then(|m| m.left_behind) {
            Some(1) => "1 item, with its reason".into(),
            Some(n) => format!("{} items, with reasons", count(n)),
            None => "Every item, with its reason".into(),
        },
        to_live: m.and_then(|m| m.approval_to_live_seconds).map(seconds),
        approved_with: m.and_then(|m| m.approved_with.as_deref()).map(|w| match w {
            "passkey" => "a passkey",
            _ => "an authenticator code",
        }),
    }
}

/// `label` is the singular and the plural: "1 signed receipt issued", "2 signed receipts issued".
fn stat(value: Option<u64>, label: [&'static str; 2], note: &str, unknown: &str) -> Stat {
    Stat {
        note: if value.is_some() { note } else { unknown }.to_owned(),
        label: if value == Some(1) { label[0] } else { label[1] },
        value: value.map(count),
        href: None,
    }
}

/// Builds the page's view.
pub fn home(facts: &Facts, input: &Inputs<'_>) -> HomeView {
    let gauntlet_stat = match &facts.gauntlet {
        Some(g) => Stat {
            value: Some(format!("{}/{}", g.score, g.out_of)),
            label: "attacks contained on the last gauntlet run",
            note: format!("Run on {}. The score comes from its receipt.", date_time(g.finished_at)),
            href: g.receipt.as_deref().map(receipt_href),
        },
        None => Stat {
            value: None,
            label: "the gauntlet score",
            note: "The gauntlet runs ten attacks after every deploy. Its score appears here from the run's receipt.".into(),
            href: None,
        },
    };
    let stats = vec![
        gauntlet_stat,
        stat(
            facts.receipts_issued,
            ["signed receipt issued", "signed receipts issued"],
            "Every job, approval and change gets one. Each verifies in your browser.",
            "The receipt count can't be read right now.",
        ),
        stat(
            facts.sites_live,
            [
                "site serving a signed release",
                "sites serving a signed release",
            ],
            "Counted from the live release pointers. Samples aren't counted.",
            "The count of live sites can't be read right now.",
        ),
        stat(
            facts.reports_written,
            ["free health report written", "free health reports written"],
            "Each one read a site's public pages in sealed rooms.",
            "The count of health reports can't be read right now.",
        ),
    ];
    let gauntlet = match &facts.gauntlet {
        Some(g) => GauntletView {
            score: Some(format!("{}/{}", g.score, g.out_of)),
            when: Some(date_time(g.finished_at)),
            href: g.receipt.as_deref().map(receipt_href),
            attacks: g
                .attacks
                .iter()
                .map(|a| AttackView {
                    name: a.name.clone(),
                    held: a.held,
                    href: a.receipt.as_deref().map(receipt_href),
                })
                .collect(),
        },
        None => GauntletView {
            score: None,
            when: None,
            href: None,
            attacks: Vec::new(),
        },
    };
    let team_cards = team(
        facts,
        &gauntlet,
        input.kept,
        input.desks_ai,
        &input.answered,
        (&input.issues, input.readers),
        input.chief_brief,
    );
    HomeView {
        origin: input.origin.trim_end_matches('/').to_owned(),
        v: crate::ASSETS_VERSION,
        ui_v: agentrix_ui::VERSION,
        assess_action: input.assess_action.to_owned(),
        fixture_href: input.fixture_href.map(str::to_owned),
        stats,
        read_at: facts.read_at.map(date_time),
        gauntlet,
        first_move: facts.first_live_move.as_ref().map(move_view),
        golive: golive_view(facts.first_live_move.as_ref()),
        routing: facts.routing.as_ref().map(|r| RoutingView {
            provider: provider_name(&r.provider),
            on_vultr: r.provider == "vultr",
            since: date(r.since),
            href: receipt_href(&r.receipt),
            routes: r
                .routes
                .iter()
                .map(|route| RouteView {
                    purpose: route.purpose.clone(),
                    model: route.model.clone(),
                    on_vultr: route.provider.as_deref().unwrap_or(&r.provider) == "vultr",
                })
                .collect(),
        }),
        brief: facts.brief.as_ref().map(|b| BriefView {
            site_name: b.site_name.clone(),
            when: date_time(b.written_at),
            headline: b.headline.clone(),
            lines: b.lines.clone(),
        }),
        samples: input
            .samples
            .iter()
            .map(|s| {
                let host = format!("{}.{}", s.slug, input.platform_host);
                SampleView {
                    url: input
                        .samples_open
                        .iter()
                        .any(|d| d.eq_ignore_ascii_case(&host))
                        .then(|| site_url(input.origin, &host)),
                    host,
                    team: input.marks.team(&s.id).cloned(),
                    picture: facts.sample_pictures.get(&s.slug).cloned(),
                    sample: s.clone(),
                }
            })
            .collect(),
        plans: input.plans.to_vec(),
        agency: input.agency.cloned(),
        links: input.links.clone(),
        film: input.film.clone(),
        contact: input.contact.map(|c| c.at(Utc::now())),
        page: "home".into(),
        judges_url: format!(
            "{}judges",
            site_url(input.origin, &format!("money.{}", input.platform_host))
        ),
        verify_url: format!(
            "{}judges#verify",
            site_url(input.origin, &format!("money.{}", input.platform_host))
        ),
        team: team_cards,
        annual_discount_percent: agentrix_billing::catalogue().annual_discount_percent,
        from_price: input
            .plans
            .iter()
            .filter(|p| p.interval == "month")
            .map(|p| p.cents)
            .min()
            .map(|c| format!("USD {}", count(u64::try_from(c / 100).unwrap_or(0)))),
        held: held(facts),
        desks_ai: input.desks_ai,
        front_desks: crate::desks::DESKS.to_vec(),
        issues: input.issues.clone(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn samples_link_on_this_platform_s_scheme_and_port() {
        assert_eq!(
            super::site_url(
                "https://agentrix.website",
                "kilele-bakery-sample.agentrix.website"
            ),
            "https://kilele-bakery-sample.agentrix.website/"
        );
        assert_eq!(
            super::site_url("http://localhost:18050", "kilele-bakery-sample.localhost"),
            "http://kilele-bakery-sample.localhost:18050/"
        );
        assert_eq!(
            super::site_url("https://agentrix.website:443/", "x.agentrix.website"),
            "https://x.agentrix.website/"
        );
    }

    #[test]
    fn the_person_link_knows_its_hours() {
        use chrono::TimeZone;
        let c =
            super::Contact::new("+254 721 490 933", "6 a.m. to 11 p.m. EAT").expect("a contact");
        assert!(c.href.starts_with("https://wa.me/254721490933?text=Hi%2C"));
        assert_eq!(c.opens, Some((6, 23)));
        assert_eq!(c.hours, "6 a.m. to 11 p.m. EAT");
        assert_eq!(c.reply_from.as_deref(), Some("We reply from 6 a.m. EAT."));
        // The old way of writing them reads the same and is written the site's one way.
        let old = super::Contact::new("+254 721 490 933", "6am to 11pm EAT").expect("a contact");
        assert_eq!(old.opens, Some((6, 23)));
        assert_eq!(old.hours, "6 a.m. to 11 p.m. EAT");
        let noon =
            super::Contact::new("+254 721 490 933", "12 p.m. to 12 a.m.").expect("a contact");
        assert_eq!(noon.opens, Some((12, 0)));
        assert_eq!(noon.hours, "12 p.m. to 12 a.m. EAT");
        // Hours it can't read stay as published, with no "reply from".
        let free = super::Contact::new("+254 721 490 933", "most evenings").expect("a contact");
        assert_eq!(
            (free.opens, free.hours.as_str(), free.reply_from),
            (None, "most evenings", None)
        );
        // 10:00 EAT is 07:00 UTC; 02:00 EAT is 23:00 UTC the day before.
        assert!(
            c.at(Utc.with_ymd_and_hms(2026, 11, 8, 7, 0, 0).unwrap())
                .open_now
        );
        assert!(
            !c.at(Utc.with_ymd_and_hms(2026, 11, 7, 23, 0, 0).unwrap())
                .open_now
        );
        assert!(super::Contact::new("12", "6am to 11pm EAT").is_none());
    }

    use super::*;

    #[test]
    fn numbers_read_the_way_people_say_them() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1204), "1,204");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(seconds(12.43), "12.4 s");
        assert_eq!(seconds(128.0), "2 min 8 s");
        assert_eq!(seconds(120.2), "2 min");
        assert_eq!(usd(370_000), "$0.37");
    }

    #[test]
    fn plans_come_from_the_billing_catalogue() {
        assert_eq!(dollars(2900), "$29");
        assert_eq!(dollars(2950), "$29.50");
        assert_eq!(dollars(150_000), "$1,500");
        let (cards, agency) = plans(agentrix_billing::catalogue());
        assert!(!cards.is_empty());
        assert!(cards.iter().all(|c| c.price.starts_with('$')));
        assert!(agency.is_some_and(|a| a.price.starts_with('$')));
    }

    #[test]
    fn parity_comes_from_the_prove_summary() {
        assert_eq!(
            parity_of("Parity 98.8%; release 3 is ready for you").as_deref(),
            Some("98.8%")
        );
        assert_eq!(
            parity_of("Parity 91.0% is below 95%; the Move needs a person").as_deref(),
            Some("91.0%")
        );
        assert_eq!(parity_of("Rebuilt 50 pages"), None);
    }

    #[test]
    fn dates_are_in_east_africa_time() {
        let t: DateTime<Utc> = "2026-11-07T20:58:00Z".parse().unwrap();
        assert_eq!(date(t), "Sat 7 Nov 2026");
        assert_eq!(date_time(t), "Sat 7 Nov, 23:58 EAT");
    }
}
