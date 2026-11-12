//! The Chief WebAgent's brief for Today (R10.1, task S2-04).
//!
//! The brief is grounded in receipts. Every line is composed here, in plain words, from the
//! site's receipts of the last day, and links the receipt it rests on. The Model Gateway's
//! `brief` purpose writes only the one-sentence headline, from counts this module computes.
//! No text from a visitor or from the old site reaches the model, so the headline cannot be
//! steered by what an attacker planted. When the gateway is down or the budget refuses, the
//! headline falls back to plain words composed here.
//!
//! Wiring:
//! - [`ChiefBrief::refresh`] for one site, for example after each Move step.
//! - [`ChiefBrief::spawn`] for a timer over every site with recent receipts.
//! - It writes the `briefs` table (`body` = [`agentrix_types::Brief`]), which the console's
//!   `ConsoleReads::brief` reads.
//! - A refresh whose lines and spend match the latest brief writes nothing and calls no model.
//! - Questions visitors passed to the owner (R8.3) add one line with their count, linking no
//!   receipt; their words stay on Today and never reach the model.
//! - The headline's prompt carries the rules the owner taught the Chief WebAgent
//!   (`agentrix_kernel::taught`), quoted, with the guard that no rule widens what it may do.
//!   They are the owner's words, read fresh for each brief, so a removed rule drops out.
//! - The spend line is this month's AI spend against the monthly AI budget, in the site's
//!   currency (`agentrix_store::budget`), the figure the console's meter shows. The gateway's
//!   daily limit is an internal guard and never the owner's number.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use agentrix_kernel::{Action, Effect};
use agentrix_types::{Brief, BriefLine, Digest, ReceiptKind, SiteId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::db::receipts::{ReceiptStore, Stored};
use crate::db::time;

/// How far back a brief looks.
pub const WINDOW_HOURS: i64 = 24;

/// The most lines a brief shows.
pub const MAX_LINES: usize = 12;

/// The most receipts a brief reads.
const READ_MAX: u32 = 5_000;

/// Writes the headline from the brief's counts.
#[async_trait]
pub trait HeadlineModel: Send + Sync {
    /// One plain sentence for the owner, from `facts` (counts and fixed phrases only), under
    /// `system` ([`system_prompt`]).
    async fn headline(&self, site: &SiteId, system: &str, facts: &Value) -> anyhow::Result<String>;
}

/// The Model Gateway's `brief` purpose (design §4.3), at `AGENTRIX_MODELS_URL`.
pub struct GatewayHeadline {
    base: String,
    http: reqwest::Client,
}

impl GatewayHeadline {
    /// A client for the gateway at `base`, such as `http://127.0.0.1:8090`.
    pub fn new(base: impl Into<String>, http: reqwest::Client) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            http,
        }
    }
}

const SYSTEM: &str = "You are the Chief WebAgent of a small business website. Write ONE short, \
calm sentence (at most 25 words) for the owner that sums up the last day from the JSON facts \
you are given. Use only those facts. Do not invent numbers, promise anything, or give advice. \
Plain words, no lists, no links, no quotes, no emoji.";

/// The headline's instructions, then `taught`: the rules and grants the owner gave the Chief
/// WebAgent ([`agentrix_kernel::taught::Taught::section`]), with the guard after the rules.
pub fn system_prompt(taught: &str) -> String {
    if taught.is_empty() {
        SYSTEM.to_owned()
    } else {
        format!("{SYSTEM}\n\n{taught}")
    }
}

#[async_trait]
impl HeadlineModel for GatewayHeadline {
    async fn headline(&self, site: &SiteId, system: &str, facts: &Value) -> anyhow::Result<String> {
        let response: Value = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .header("x-agentrix-site", site.as_str())
            .header("x-agentrix-purpose", "brief")
            .timeout(Duration::from_secs(45))
            .json(&json!({
                "stream": false,
                "temperature": 0.2,
                "max_tokens": 120,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": facts.to_string()},
                ],
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let text = response
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("the gateway's reply has no content"))?;
        Ok(text.to_owned())
    }
}

/// Composes and stores the Chief WebAgent's brief.
pub struct ChiefBrief {
    pool: SqlitePool,
    receipts: Arc<ReceiptStore>,
    model: Arc<dyn HeadlineModel>,
    guards: crate::guards::Guards,
    records: Option<SqlitePool>,
}

impl ChiefBrief {
    /// A brief writer over the node's stores and a headline model.
    pub fn new(
        pool: SqlitePool,
        receipts: Arc<ReceiptStore>,
        model: Arc<dyn HeadlineModel>,
    ) -> Self {
        Self {
            pool,
            receipts,
            model,
            guards: crate::guards::Guards::default(),
            records: None,
        }
    }

    /// Also reads W2's `records.db` at `path` (read-only), for the posted rate that shows the
    /// AI budget in the site's currency. Without it, amounts stay in US dollars.
    #[must_use]
    pub fn with_records_at(mut self, path: &std::path::Path) -> Self {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .create_if_missing(false);
        self.records = Some(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(2)
                .connect_lazy_with(options),
        );
        self
    }

    /// Checks the platform's guards (`crate::guards`) before writing.
    #[must_use]
    pub fn with_guards(mut self, guards: crate::guards::Guards) -> Self {
        self.guards = guards;
        self
    }

    /// Composes `site`'s brief from the last day's receipts and stores it, unless it says the
    /// same as the latest brief. Returns the brief now current.
    pub async fn refresh(&self, site: &SiteId) -> anyhow::Result<Brief> {
        // While the owner's Freeze is in force the Chief writes nothing (W5, 2b.1).
        if self.guards.frozen(site.as_str()).await {
            return self.latest_or_empty(site).await;
        }
        let now = Utc::now();
        let since = now - chrono::Duration::hours(WINDOW_HOURS);
        let stored = self.receipts.list_since(site, since, READ_MAX).await?;
        let mut digest = compose(&stored);
        // Questions visitors passed to the owner (R8.3): a count only, never their words.
        let passed = crate::db::questions::QuestionStore::new(self.pool.clone())
            .count(site, Some(since))
            .await?;
        if passed > 0 {
            digest.lines.insert(
                0,
                BriefLine {
                    text: format!(
                        "{} the site doesn't answer. Reply on your Back office page.",
                        plural(
                            passed as usize,
                            "visitor passed you a question",
                            "visitors passed you questions"
                        )
                    ),
                    receipt: None,
                },
            );
            digest.lines.truncate(MAX_LINES);
            digest.facts["questions_passed_to_owner"] = json!(passed);
        }
        let spend = self.spend_words(site, now).await?;

        if let Some(latest) = self.latest(site).await?
            && latest.lines == digest.lines
            && latest.spend_words.as_deref() == Some(spend.as_str())
        {
            return Ok(latest);
        }

        // The Chief writes a new brief only while its dial lets it: at Watch it writes nothing
        // and calls no model (W1, handover-wiring 2a.2).
        let writing = Action::new("coordinator", "morning-brief", Effect::Draft);
        if self.guards.refusal(site.as_str(), &writing).await.is_some() {
            return self.latest_or_empty(site).await;
        }

        // The Chief's living mark shows it writing while the brief is written (W1, 2.4).
        let presence = agentrix_kernel::presence::global();
        presence.on_brief(site.as_str(), true);
        let written = self.write(site, digest, spend, now).await;
        presence.on_brief(site.as_str(), false);
        written
    }

    /// "AI spend this month: KSh 38 of KSh 3,870.": this month against the monthly AI budget,
    /// in the site's currency.
    async fn spend_words(&self, site: &SiteId, now: DateTime<Utc>) -> anyhow::Result<String> {
        let m = agentrix_store::budget::this_month(
            &self.pool,
            self.records.as_ref(),
            site.as_str(),
            "",
            now,
        )
        .await?;
        Ok(format!("AI spend this month: {}.", m.of_cap()))
    }

    /// The Chief WebAgent's rules and grants from the owner, read fresh, for the headline's
    /// prompt.
    async fn taught(&self, site: &SiteId, now: DateTime<Utc>) -> String {
        let offset = agentrix_kernel::taught::site_offset(&self.pool, site.as_str())
            .await
            .unwrap_or(0);
        agentrix_kernel::taught::Taught::for_prompt(
            &self.pool,
            site.as_str(),
            "coordinator",
            offset,
            now,
        )
        .await
        .section()
    }

    /// Writes the headline (the only model call) and stores the brief.
    async fn write(
        &self,
        site: &SiteId,
        digest: Composed,
        spend: String,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Brief> {
        let system = system_prompt(&self.taught(site, now).await);
        let headline = match self.model.headline(site, &system, &digest.facts).await {
            Ok(text) => clean_headline(&text).unwrap_or_else(|| digest.fallback.clone()),
            Err(e) => {
                tracing::warn!(site = %site.as_str(), error = %e, "brief headline fell back to plain words");
                digest.fallback.clone()
            }
        };
        let brief = Brief {
            headline: Some(headline),
            lines: digest.lines,
            spend_words: Some(spend),
        };
        sqlx::query("INSERT OR REPLACE INTO briefs (site_id, written_at, body) VALUES (?, ?, ?)")
            .bind(site.as_str())
            .bind(time(now))
            .bind(serde_json::to_string(&brief)?)
            .execute(&self.pool)
            .await?;
        Ok(brief)
    }

    /// Refreshes every site with receipts in the last day. Errors are logged per site.
    pub async fn refresh_all(&self) -> anyhow::Result<usize> {
        let since = Utc::now() - chrono::Duration::hours(WINDOW_HOURS);
        let sites: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT site_id FROM receipts WHERE issued_at >= ?")
                .bind(time(since))
                .fetch_all(&self.pool)
                .await?;
        for (site,) in &sites {
            if let Err(e) = self.refresh(&SiteId::new(site.clone())).await {
                tracing::error!(site = %site, error = %e, "the brief could not be refreshed");
            }
        }
        Ok(sites.len())
    }

    /// Refreshes every recent site now and then every `period`, for as long as the node runs.
    pub fn spawn(self: Arc<Self>, period: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(period);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(e) = self.refresh_all().await {
                    tracing::error!(error = %e, "the brief timer failed");
                }
            }
        })
    }

    /// The brief already written, or an empty one, when nothing new may be written.
    async fn latest_or_empty(&self, site: &SiteId) -> anyhow::Result<Brief> {
        Ok(self.latest(site).await?.unwrap_or(Brief {
            headline: None,
            lines: Vec::new(),
            spend_words: None,
        }))
    }

    async fn latest(&self, site: &SiteId) -> anyhow::Result<Option<Brief>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT body FROM briefs WHERE site_id = ? ORDER BY written_at DESC LIMIT 1",
        )
        .bind(site.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(body,)| serde_json::from_str(&body).ok()))
    }
}

/// The brief's lines, the model's facts and a plain-words headline, from receipts.
#[derive(Debug, Clone, PartialEq)]
pub struct Composed {
    /// Lines, newest first, each linking its receipt.
    pub lines: Vec<BriefLine>,
    /// Counts and fixed phrases for the headline model. Never visitor or site text.
    pub facts: Value,
    /// The headline when the model is unavailable.
    pub fallback: String,
}

/// A line and when the receipt it rests on ended, for ordering.
struct Dated {
    at: DateTime<Utc>,
    line: BriefLine,
}

fn line(at: DateTime<Utc>, text: String, receipt: Digest) -> Dated {
    Dated {
        at,
        line: BriefLine {
            text,
            receipt: Some(receipt),
        },
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What a detector id means to the owner (attacks.json and the Move lane's ids).
fn detector_words(id: &str) -> &'static str {
    match id {
        "hidden-instructions" => "hidden instructions aimed at AI agents",
        "spam-links" => "hidden spam links",
        "cloaking" => "a plugin that showed search engines different pages",
        "executable-upload" => "a program disguised as an upload",
        "obfuscated-script" => "a disguised script in the footer",
        "calls-out" => "code that tried to call out to the internet",
        "mime-mismatch" => "a file pretending to be an image",
        "private-address" => "links into private addresses",
        "backdoor-code" => "backdoor code",
        "spam-page" => "spam pages",
        _ => "other risky content",
    }
}

/// Where a refused host pointed, in the owner's words, when it is notable.
fn refused_words(host: &str) -> Option<&'static str> {
    let h = host.trim_start_matches('[').to_ascii_lowercase();
    if h.starts_with("169.254.169.254") {
        Some("Vultr's private metadata address")
    } else if h.starts_with("127.") || h.starts_with("localhost") || h.starts_with("::1") {
        Some("the server's own internal address")
    } else {
        None
    }
}

fn detail_str<'a>(s: &'a Stored, key: &str) -> Option<&'a str> {
    s.receipt.body.detail.as_ref()?.get(key)?.as_str()
}

/// The platform-written title of a receipt, when it has one.
fn title(s: &Stored) -> Option<String> {
    s.receipt.body.title().map(str::to_owned)
}

fn truncate(text: &str, max: usize) -> String {
    let t: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        format!("{}…", t.trim_end())
    } else {
        t
    }
}

/// Composes the brief from receipts (any order).
pub fn compose(receipts: &[Stored]) -> Composed {
    let mut out: Vec<Dated> = Vec::new();
    let latest = |pred: &dyn Fn(&Stored) -> bool| -> Option<&Stored> {
        receipts
            .iter()
            .filter(|s| pred(s))
            .max_by_key(|s| s.receipt.body.ended_at)
    };
    let kind_is = |k: ReceiptKind| move |s: &Stored| s.receipt.body.kind == k;
    let count = |pred: &dyn Fn(&Stored) -> bool| receipts.iter().filter(|s| pred(s)).count();

    // The gauntlet: its latest run's own title ("Containment gauntlet: n/10 contained").
    let gauntlet = latest(&kind_is(ReceiptKind::GauntletRun));
    let mut gauntlet_score = None;
    if let Some(s) = gauntlet {
        let d = s.receipt.body.detail.as_ref();
        let score = d.and_then(|d| d.get("score")).and_then(Value::as_u64);
        let of = d.and_then(|d| d.get("out_of")).and_then(Value::as_u64);
        gauntlet_score = score.zip(of);
        let text = match gauntlet_score {
            Some((n, of)) => format!("{n} of {of} test attacks stopped."),
            None => "Test attacks ran against your site.".to_owned(),
        };
        out.push(line(s.receipt.body.ended_at, text, s.digest));
    }

    // Sealed-room jobs.
    let is_job = |s: &Stored| {
        matches!(
            s.receipt.body.kind,
            ReceiptKind::JobFetch
                | ReceiptKind::JobReplay
                | ReceiptKind::JobRender
                | ReceiptKind::JobBuild
        )
    };
    let jobs = count(&is_job);
    let destroyed = count(&|s: &Stored| is_job(s) && s.receipt.body.destroyed == Some(true));
    let fetches = count(&kind_is(ReceiptKind::JobFetch));
    if let Some(s) = latest(&kind_is(ReceiptKind::JobFetch)) {
        out.push(line(
            s.receipt.body.ended_at,
            format!(
                "Read the old site's public pages in {}.",
                plural(fetches, "sealed room", "sealed rooms")
            ),
            s.digest,
        ));
    }
    if let Some(s) = latest(&kind_is(ReceiptKind::JobReplay)) {
        out.push(line(
            s.receipt.body.ended_at,
            "Replayed the old WordPress site in a sealed room with no way out to the internet."
                .to_owned(),
            s.digest,
        ));
    }
    let builds = count(&|s: &Stored| {
        matches!(
            s.receipt.body.kind,
            ReceiptKind::JobBuild | ReceiptKind::JobRender
        ) && detail_str(s, "killed").is_none()
    });
    if let Some(s) = latest(&|s: &Stored| {
        matches!(
            s.receipt.body.kind,
            ReceiptKind::JobBuild | ReceiptKind::JobRender
        ) && detail_str(s, "killed").is_none()
    }) {
        out.push(line(
            s.receipt.body.ended_at,
            format!(
                "Built and checked the new pages in {}.",
                plural(builds, "sealed room", "sealed rooms")
            ),
            s.digest,
        ));
    }

    // Refusals by the egress guard.
    let refusals: Vec<&str> = receipts
        .iter()
        .filter(|s| is_job(s))
        .filter_map(|s| s.receipt.body.observed.as_ref())
        .flat_map(|o| o.refusals.iter().map(|r| r.host.as_str()))
        .collect();
    let notable: Vec<&'static str> = {
        let mut v: Vec<&'static str> = refusals.iter().filter_map(|h| refused_words(h)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    if let Some(s) = latest(&|s: &Stored| {
        is_job(s)
            && s.receipt
                .body
                .observed
                .as_ref()
                .is_some_and(|o| !o.refusals.is_empty())
    }) {
        let mut text = format!(
            "Blocked {} to reach addresses off limits to a sealed room",
            plural(refusals.len(), "attempt", "attempts")
        );
        if !notable.is_empty() {
            text.push_str(&format!(", including {}", notable.join(" and ")));
        }
        text.push('.');
        out.push(line(s.receipt.body.ended_at, text, s.digest));
    }

    // Jobs stopped at their caps.
    let killed: Vec<&Stored> = receipts
        .iter()
        .filter(|s| is_job(s) && detail_str(s, "killed").is_some())
        .collect();
    if let Some(s) = killed.iter().max_by_key(|s| s.receipt.body.ended_at) {
        let cap = match detail_str(s, "killed") {
            Some("memory") => "memory",
            Some("pids") => "process",
            Some("deadline") => "time",
            Some("out_cap") => "output",
            _ => "resource",
        };
        let gone = if s.receipt.body.destroyed == Some(true) {
            " Its room was destroyed."
        } else {
            ""
        };
        let text = if killed.len() == 1 {
            format!("Stopped a job that hit its {cap} cap.{gone}")
        } else {
            format!(
                "Stopped {} jobs at their caps, the latest at its {cap} cap.{gone}",
                killed.len()
            )
        };
        out.push(line(s.receipt.body.ended_at, text, s.digest));
    }

    // What the clean step set aside.
    let mut quarantined_n = 0usize;
    if let Some(s) = latest(&|s: &Stored| {
        s.receipt
            .body
            .detail
            .as_ref()
            .and_then(|d| d.get("quarantined"))
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    }) {
        let items = s
            .receipt
            .body
            .detail
            .as_ref()
            .and_then(|d| d.get("quarantined"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        quarantined_n = items.len();
        let mut kinds: Vec<&'static str> = items
            .iter()
            .map(|i| detector_words(i.get("detector").and_then(Value::as_str).unwrap_or("")))
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        out.push(line(
            s.receipt.body.ended_at,
            format!(
                "The Mover left behind {} from the old site: {}.",
                plural(quarantined_n, "risky item", "risky items"),
                kinds.join(", ")
            ),
            s.digest,
        ));
    }

    // Other Move steps, latest per step, by their own titles.
    let mut steps: BTreeMap<String, &Stored> = BTreeMap::new();
    for s in receipts
        .iter()
        .filter(|s| s.receipt.body.kind == ReceiptKind::MoveStep)
    {
        let step = detail_str(s, "step").unwrap_or("move").to_owned();
        if step == "clean" && quarantined_n > 0 {
            continue;
        }
        let keep = steps
            .get(&step)
            .is_none_or(|prev| prev.receipt.body.ended_at <= s.receipt.body.ended_at);
        if keep {
            steps.insert(step, s);
        }
    }
    for (step, s) in steps {
        let text = title(s).unwrap_or_else(|| format!("The Move finished its {step} step."));
        out.push(line(s.receipt.body.ended_at, text, s.digest));
    }

    // The Concierge's refusals: the attempt is shown to the owner, quoted briefly.
    let refusals_n = count(&kind_is(ReceiptKind::ConciergeRefusal));
    if let Some(s) = latest(&kind_is(ReceiptKind::ConciergeRefusal)) {
        let excerpt = s
            .receipt
            .body
            .detail
            .as_ref()
            .and_then(|d| d.pointer("/observed/excerpt").or_else(|| d.get("excerpt")))
            .and_then(Value::as_str)
            .map(|e| truncate(e, 60));
        let mut text = format!(
            "The Concierge refused {} it isn't allowed to grant",
            plural(refusals_n, "request", "requests")
        );
        if let Some(e) = excerpt {
            text.push_str(&format!(", such as \u{201c}{e}\u{201d}"));
        }
        text.push('.');
        out.push(line(s.receipt.body.ended_at, text, s.digest));
    }

    // Owner-facing events, one line each, by title or plain template.
    let mut approvals = 0usize;
    let mut releases = 0usize;
    let mut held = 0usize;
    for s in receipts {
        let fixed = match s.receipt.body.kind {
            ReceiptKind::ClaimVerified => Some("You proved you own the site."),
            ReceiptKind::DecisionApproved => {
                approvals += 1;
                Some("You approved a decision.")
            }
            ReceiptKind::DecisionDeclined => Some("You declined a decision."),
            ReceiptKind::ReleaseSigned => {
                releases += 1;
                Some("A new signed release of your site went live.")
            }
            ReceiptKind::ReleaseRollback => {
                Some("Your site was rolled back to its previous release.")
            }
            ReceiptKind::QuarantineRestored => {
                Some("You brought back an item that was left behind.")
            }
            ReceiptKind::DoorOrderHeld => {
                held += 1;
                Some("A customer's AI agent asked for an order; it is waiting for your approval.")
            }
            _ => None,
        };
        if let Some(text) = fixed {
            let text = title(s).unwrap_or_else(|| text.to_owned());
            out.push(line(s.receipt.body.ended_at, text, s.digest));
        }
    }

    out.sort_by_key(|d| std::cmp::Reverse(d.at));
    out.truncate(MAX_LINES);

    let facts = json!({
        "window": "the last 24 hours",
        "sealed_room_jobs": jobs,
        "rooms_destroyed": destroyed,
        "old_site_read_jobs": fetches,
        "blocked_attempts": refusals.len(),
        "blocked_notable": notable,
        "jobs_stopped_at_caps": killed.len(),
        "items_quarantined": quarantined_n,
        "concierge_refusals": refusals_n,
        "approvals": approvals,
        "releases_live": releases,
        "orders_waiting_for_owner": held,
        "gauntlet": gauntlet_score.map(|(n, of)| format!("{n}/{of} contained")),
    });
    let fallback = if let Some((n, of)) = gauntlet_score {
        format!("Your WebAgents stopped {n} of {of} test attacks.")
    } else if quarantined_n > 0 {
        format!(
            "The Mover read and cleaned your old site and left {} behind.",
            plural(quarantined_n, "risky item", "risky items")
        )
    } else if out.is_empty() {
        "A quiet day: nothing needed you.".to_owned()
    } else {
        "Here is what your WebAgents did in the last day.".to_owned()
    };
    Composed {
        lines: out.into_iter().map(|d| d.line).collect(),
        facts,
        fallback,
    }
}

/// One line of plain text, or nothing if the model wrote something a headline must not hold.
fn clean_headline(text: &str) -> Option<String> {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let one =
        one.trim_matches(|c: char| c == '"' || c == '\'' || c == '\u{201c}' || c == '\u{201d}');
    let bad = one.is_empty()
        || one.chars().count() > 200
        || one.contains("://")
        || one.contains('<')
        || one.contains('@');
    (!bad).then(|| one.to_owned())
}

#[cfg(test)]
mod tests;
