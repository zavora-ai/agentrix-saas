//! agentrix.website's own team (site [`SITE`]): four front desks on the home page (Your site,
//! How it works, Samples, Prices), and the Guardian, the Editor and Sales in the back office
//! with the Chief, who briefs James.
//!
//! Its marks move from agentrix.website's own records, never from a guess:
//! - the Your site desk works while Assess checks a site (`assessments`, state `running`);
//! - the Guardian works while the gauntlet runs (`gauntlet_runs` with no end yet);
//! - Sales works while its follow-up emails go out (`message_outbox`: queued, or sent in the
//!   last minute);
//! - the Editor waits, and the Chief shows the amber dot, while a card the Editor raised waits
//!   for James (`decision_cards_v2`, a newsletter draft for example).
//!
//! Those are other lanes' tables with no events to listen to, so while a stream is open on the
//! site, [`watch`] reads them every few seconds; with nobody watching, nothing runs. A frame goes
//! out only when something changed, so an idle team stays still.
//!
//! The Chief's brief ([`write_brief`]) is composed from the same records in plain words, with no
//! model call, into the `briefs` table, where W13's channels send it to James on Telegram when
//! his quiet hours end, as they do for Kilele. That needs a `sites` row for [`SITE`] owned by
//! James's account; until one exists, the brief isn't stored and says so in the log.

use std::time::Duration;

use agentrix_marks::{Audience, MarkState, WordsContext, words};
use agentrix_types::records::{Brief, BriefLine};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::presence::{Presence, PresenceFrame};
use crate::store::Rosters;
use crate::{looks, roster};

/// agentrix.website's site id, the one W11's stills already use.
pub const SITE: &str = "agentrix-home";

/// The front desk that fronts Assess.
pub const ASSESS_DESK: &str = "desk-your-site";

/// The messages that are Sales's follow-ups: Agentrix writing to someone who checked their
/// site or joined.
pub const SALES_MESSAGES: [&str; 5] = [
    "owner.claim_reminder",
    "owner.move_update",
    "owner.live",
    "owner.plan_started",
    "owner.plan_changed",
];

/// How often the watcher reads while someone watches.
const EVERY: Duration = Duration::from_secs(4);
/// How often the brief is brought up to date.
const BRIEF_EVERY: Duration = Duration::from_secs(30 * 60);
/// A follow-up sent this recently still shows Sales at work.
const JUST_SENT_SECS: i64 = 60;
/// A run or check still `running` after this long is stuck, not working.
const STUCK_MINUTES: i64 = 20;

const GAUNTLET: &str = "Running the gauntlet";

/// Records the team the first time: its look, who is on duty and the four desks. A team
/// already recorded is left as it is, so James's own changes stay. Returns whether it seeded.
pub async fn seed(rosters: &Rosters) -> anyhow::Result<bool> {
    if rosters.has_team(SITE).await? {
        return Ok(false);
    }
    let (on_duty, desks) = roster::home_team();
    rosters
        .set_team(SITE, &looks::agentrix(), &on_duty, &desks)
        .await?;
    Ok(true)
}

/// What agentrix.website's records say is happening now.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Now {
    /// Sites Assess is checking.
    pub checking: i64,
    /// Whether a gauntlet run is under way.
    pub gauntlet: bool,
    /// Follow-up emails queued, or sent in the last minute.
    pub sending: i64,
    /// Cards waiting for James: (card id, the WebAgent that raised it).
    pub cards: Vec<(String, String)>,
}

/// Reads [`Now`] from node's database. A table that isn't there reads as nothing happening.
pub async fn read(pool: &SqlitePool, now: DateTime<Utc>) -> anyhow::Result<Now> {
    let stuck = stamp(now - chrono::Duration::minutes(STUCK_MINUTES));
    let checking = count(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM assessments WHERE state = 'running' AND created_at >= ?",
        )
        .bind(&stuck)
        .fetch_one(pool)
        .await,
    )?;
    let gauntlet = count(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM gauntlet_runs WHERE ended_at IS NULL AND started_at >= ?",
        )
        .bind(&stuck)
        .fetch_one(pool)
        .await,
    )? > 0;
    let sending = count(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_outbox WHERE message IN (SELECT value FROM json_each(?)) \
             AND (status = 'queued' OR (status = 'sent' AND sent_at >= ?))",
        )
        .bind(json!(SALES_MESSAGES).to_string())
        .bind(stamp(now - chrono::Duration::seconds(JUST_SENT_SECS)))
        .fetch_one(pool)
        .await,
    )?;
    Ok(Now {
        checking,
        gauntlet,
        sending,
        cards: cards(pool).await?,
    })
}

/// The cards waiting for James on agentrix.website, with who raised each.
async fn cards(pool: &SqlitePool) -> anyhow::Result<Vec<(String, String)>> {
    let rows: Vec<(String, Option<String>)> = match sqlx::query_as(
        "SELECT id, json_extract(card, '$.raised_by.webagent') FROM decision_cards_v2 \
         WHERE site_id = ? AND state = 'waiting' ORDER BY created_at",
    )
    .bind(SITE)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) if missing(&e) => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    Ok(rows
        .into_iter()
        .map(|(id, who)| (id, who.unwrap_or_else(|| "coordinator".to_string())))
        .collect())
}

/// Shows [`Now`] on the team's marks. Only what changed sends a frame.
pub fn show(hub: &Presence, now: &Now) {
    let checking = match now.checking {
        0 => None,
        1 => Some("Checking a site".to_string()),
        n => Some(format!("Checking {n} sites")),
    };
    put(hub, ASSESS_DESK, checking.as_deref(), MarkState::Idle);
    put(
        hub,
        "guardian",
        now.gauntlet.then_some(GAUNTLET),
        MarkState::Asleep,
    );
    let sending = match now.sending {
        0 => None,
        1 => Some("Sending a follow-up email".to_string()),
        n => Some(format!("Sending {n} follow-up emails")),
    };
    put(hub, "sales", sending.as_deref(), MarkState::Asleep);
    hub.sync_cards(SITE, &now.cards, MarkState::Asleep);
}

/// Sets a member working with `label`, or back to `rest` when the work this module showed has
/// ended. Never touches a state something else set (an Ask turn, a card).
fn put(hub: &Presence, id: &str, label: Option<&str>, rest: MarkState) {
    match label {
        Some(label) => {
            hub.publish(SITE, id, MarkState::Working, Some(label), None);
        }
        None => {
            let shown_here = hub.current(SITE, id).is_some_and(|f| {
                f.state == MarkState::Working && f.label.as_deref().is_some_and(is_ours)
            });
            if shown_here {
                hub.publish(SITE, id, rest, None, None);
            }
        }
    }
}

/// Whether a label is one this module shows: the only labels a visitor may read.
pub fn is_ours(label: &str) -> bool {
    label.starts_with("Checking ") || label == GAUNTLET || label.starts_with("Sending ")
}

/// Reads the records and shows them.
pub async fn refresh(hub: &Presence, pool: &SqlitePool) -> anyhow::Result<()> {
    let now = read(pool, Utc::now()).await?;
    show(hub, &now);
    Ok(())
}

/// Keeps the marks current while a stream is open on [`SITE`]: reads the records every few
/// seconds, and stops once the last stream closes. One watcher per hub; calling it again while
/// one runs does nothing.
pub fn watch(hub: &Presence, pool: &SqlitePool) {
    if !hub.claim_watch(SITE) {
        return;
    }
    let (hub, pool) = (hub.clone(), pool.clone());
    tokio::spawn(async move {
        loop {
            if let Err(e) = refresh(&hub, &pool).await {
                tracing::warn!(error = %format!("{e:#}"), "agentrix.website's records couldn't be read");
            }
            tokio::time::sleep(EVERY).await;
            if !hub.watched(SITE) {
                hub.release_watch(SITE);
                // A stream that opened just as this one let go would find no watcher.
                if hub.watched(SITE) && hub.claim_watch(SITE) {
                    continue;
                }
                break;
            }
        }
    });
}

/// A frame as a visitor to agentrix.website sees it: the state a visitor's mark shows, the
/// words beside it, and the label only when it is one of this module's own (never a question
/// James asked or an address someone pasted).
pub fn for_visitor(frame: &PresenceFrame, owner: &str) -> Value {
    let label = frame.label.as_deref().filter(|l| is_ours(l));
    json!({
        "site": frame.site,
        "webagent": frame.webagent,
        "state": frame.state.for_visitor(),
        "label": label,
        "words": words(
            frame.state,
            Audience::Visitor,
            &WordsContext {
                label,
                owner: Some(owner),
                ..Default::default()
            },
        ),
        "seq": frame.seq,
        "at": frame.at,
    })
}

/// The Chief's brief for agentrix.website's last day, in plain words from the records: sites
/// Assess checked, the gauntlet's runs, Sales's follow-ups and what waits for James.
pub async fn compose(pool: &SqlitePool, now: DateTime<Utc>) -> anyhow::Result<Brief> {
    let since = stamp(now - chrono::Duration::hours(24));
    let checked: (i64, i64) = match sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(a.state = 'report-ready'), 0) FROM assessments a \
         LEFT JOIN sites s ON s.site_id = a.site_id \
         WHERE a.created_at >= ? AND COALESCE(s.fixture, 0) = 0",
    )
    .bind(&since)
    .fetch_one(pool)
    .await
    {
        Ok(row) => row,
        Err(e) if missing(&e) => (0, 0),
        Err(e) => return Err(e.into()),
    };
    let runs: Vec<(i64, i64, Option<String>)> = match sqlx::query_as(
        "SELECT score, out_of, receipt FROM gauntlet_runs WHERE ended_at >= ? ORDER BY ended_at DESC",
    )
    .bind(&since)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) if missing(&e) => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let sent = count(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM message_outbox WHERE message IN (SELECT value FROM json_each(?)) \
             AND status = 'sent' AND sent_at >= ?",
        )
        .bind(json!(SALES_MESSAGES).to_string())
        .bind(&since)
        .fetch_one(pool)
        .await,
    )?;
    let waiting = cards(pool).await?;

    let mut lines = Vec::new();
    let line = |text: String| BriefLine {
        text,
        receipt: None,
    };
    let drafts = waiting.iter().filter(|(_, who)| who == "editor").count();
    let others = waiting.len() - drafts;
    if drafts > 0 {
        lines.push(line(format!(
            "{} from the Editor {} waiting for your OK.",
            plural(drafts as i64, "draft", "drafts"),
            if drafts == 1 { "is" } else { "are" }
        )));
    }
    if others > 0 {
        lines.push(line(format!(
            "{} {} waiting for you.",
            plural(others as i64, "other decision", "other decisions"),
            if others == 1 { "is" } else { "are" }
        )));
    }
    let (checks, ready) = checked;
    if checks > 0 {
        let reports = match ready {
            0 => "no report is ready yet".to_string(),
            1 => "1 report is ready".to_string(),
            n => format!("{n} reports are ready"),
        };
        lines.push(line(format!(
            "Assess checked {} in the last day; {reports}.",
            plural(checks, "site", "sites")
        )));
    }
    if let Some((score, out_of, receipt)) = runs.first() {
        lines.push(BriefLine {
            text: format!(
                "The Guardian ran the gauntlet {}; the latest run contained {score} of {out_of} attacks.",
                match runs.len() {
                    1 => "once".to_string(),
                    2 => "twice".to_string(),
                    n => format!("{n} times"),
                }
            ),
            receipt: receipt
                .as_deref()
                .and_then(|r| serde_json::from_value(json!(r)).ok()),
        });
    }
    if sent > 0 {
        lines.push(line(format!(
            "Sales sent {}.",
            plural(sent, "follow-up email", "follow-up emails")
        )));
    }
    let headline = if lines.is_empty() {
        lines.push(line(
            "No site was checked and nothing went out in the last day.".to_string(),
        ));
        "A quiet day on agentrix.website."
    } else if waiting.is_empty() {
        "Here's agentrix.website's last day."
    } else {
        "Here's agentrix.website's last day, and what waits for you."
    };
    Ok(Brief {
        headline: Some(headline.to_string()),
        lines,
        spend_words: None,
    })
}

/// Writes the Chief's brief for agentrix.website unless it says the same as the latest one.
/// Returns whether it wrote one.
pub async fn write_brief(hub: &Presence, pool: &SqlitePool) -> anyhow::Result<bool> {
    let brief = compose(pool, Utc::now()).await?;
    let latest: Option<(String,)> = match sqlx::query_as(
        "SELECT body FROM briefs WHERE site_id = ? ORDER BY written_at DESC LIMIT 1",
    )
    .bind(SITE)
    .fetch_optional(pool)
    .await
    {
        Ok(row) => row,
        Err(e) if missing(&e) => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    if latest
        .and_then(|(b,)| serde_json::from_str::<Brief>(&b).ok())
        .is_some_and(|b| b.lines == brief.lines && b.headline == brief.headline)
    {
        return Ok(false);
    }
    hub.on_brief(SITE, true);
    let written = sqlx::query("INSERT INTO briefs (site_id, written_at, body) VALUES (?, ?, ?)")
        .bind(SITE)
        .bind(stamp(Utc::now()))
        .bind(serde_json::to_string(&brief)?)
        .execute(pool)
        .await;
    hub.on_brief(SITE, false);
    written?;
    Ok(true)
}

/// Where agentrix.website answers: the platform's own host and origin.
#[derive(Debug, Clone)]
pub struct Platform {
    /// `agentrix.website`.
    pub host: String,
    /// `https://agentrix.website/`.
    pub origin: String,
}

/// Adds agentrix.website to control.db's `sites` if it isn't there, so the Model Gateway
/// serves its desks (it refuses sites it doesn't know) and its Chief's brief can be stored.
/// An existing row, with any owner set on it, stays as it is.
///
/// # Errors
///
/// On a database error, or when node's `sites` table isn't there.
pub async fn register(pool: &SqlitePool, platform: &Platform) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, fixture, created_at) \
         VALUES (?, ?, 'agentrix.website', ?, 180, 0, ?) ON CONFLICT (site_id) DO NOTHING",
    )
    .bind(SITE)
    .bind(&platform.host)
    .bind(&platform.origin)
    .bind(stamp(Utc::now()))
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// The team's background work, spawned once by [`crate::mount`]: make agentrix.website a site
/// the platform knows, record the team the first time, then keep the brief current every half
/// hour (a few reads, no model call).
pub async fn run(rosters: Rosters, hub: Presence, platform: Platform) {
    match register(rosters.pool(), &platform).await {
        Ok(true) => tracing::info!(site = SITE, "agentrix.website added to sites"),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "agentrix.website not added to sites");
        }
    }
    match seed(&rosters).await {
        Ok(true) => tracing::info!(site = SITE, "agentrix.website's team recorded"),
        Ok(false) => {}
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "agentrix.website's team not recorded");
        }
    }
    loop {
        if let Err(e) = write_brief(&hub, rosters.pool()).await {
            tracing::info!(error = %format!("{e:#}"), site = SITE, "the brief wasn't stored (is there a sites row for agentrix-home?)");
        }
        tokio::time::sleep(BRIEF_EVERY).await;
    }
}

/// Times as node stores them: RFC 3339 UTC with microseconds, so text order is time order.
fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn plural(n: i64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn missing(e: &sqlx::Error) -> bool {
    e.to_string().contains("no such table")
}

fn count(r: Result<i64, sqlx::Error>) -> anyhow::Result<i64> {
    match r {
        Ok(n) => Ok(n),
        Err(e) if missing(&e) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// node's tables, as the first release, W9 and W5 create them (only the columns read here).
    async fn node() -> (SqlitePool, Rosters) {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        agentrix_decisions::migrate(&ctx.pool).await.unwrap();
        for sql in [
            "CREATE TABLE IF NOT EXISTS sites (site_id TEXT PRIMARY KEY, fixture INTEGER NOT NULL DEFAULT 0)",
            "CREATE TABLE assessments (assessment_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL)",
            "CREATE TABLE gauntlet_runs (run_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, score INTEGER NOT NULL, out_of INTEGER NOT NULL, receipt TEXT, started_at TEXT NOT NULL, ended_at TEXT)",
            "CREATE TABLE message_outbox (id TEXT PRIMARY KEY, message TEXT NOT NULL, status TEXT NOT NULL, sent_at TEXT)",
            "CREATE TABLE IF NOT EXISTS briefs (site_id TEXT NOT NULL, written_at TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (site_id, written_at))",
        ] {
            sqlx::query(sql).execute(&ctx.pool).await.unwrap();
        }
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        (ctx.pool.clone(), rosters)
    }

    async fn exec(pool: &SqlitePool, sql: &'static str, binds: &[&str]) {
        let mut q = sqlx::query(sql);
        for b in binds {
            q = q.bind(*b);
        }
        q.execute(pool).await.unwrap();
    }

    /// Without a `sites` row the Model Gateway refused agentrix.website's desks (403
    /// unknown_site) and the brief couldn't be stored.
    #[tokio::test]
    async fn agentrix_website_is_a_site_the_platform_knows() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        // The first release's `sites` columns the row fills in.
        sqlx::query(
            "CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', \
             old_origin TEXT NOT NULL, utc_offset_minutes INTEGER NOT NULL DEFAULT 0, owner_account_id TEXT, \
             fixture INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)",
        )
        .execute(&ctx.pool)
        .await
        .unwrap();
        let platform = Platform {
            host: "agentrix.website".into(),
            origin: "https://agentrix.website/".into(),
        };
        assert!(register(&ctx.pool, &platform).await.unwrap());
        // Once: an owner set on it later stays.
        sqlx::query("UPDATE sites SET owner_account_id = 'acct-james' WHERE site_id = ?")
            .bind(SITE)
            .execute(&ctx.pool)
            .await
            .unwrap();
        assert!(!register(&ctx.pool, &platform).await.unwrap());
        let row: (String, String, i64, i64, Option<String>) = sqlx::query_as(
            "SELECT domain, name, utc_offset_minutes, fixture, owner_account_id FROM sites WHERE site_id = ?",
        )
        .bind(SITE)
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
        assert_eq!(
            row,
            (
                "agentrix.website".into(),
                "agentrix.website".into(),
                180,
                0,
                Some("acct-james".into())
            )
        );
    }

    async fn card(pool: &SqlitePool, id: &str, who: &str, state: &str) {
        let card = json!({ "raised_by": { "webagent": who } }).to_string();
        exec(
            pool,
            "INSERT OR REPLACE INTO decision_cards_v2 (id, site_id, kind, card, fingerprint, state, created_at) VALUES (?, ?, 'post', ?, 'f', ?, ?)",
            &[id, SITE, &card, state, &stamp(Utc::now())],
        )
        .await;
    }

    #[tokio::test]
    async fn the_team_is_seeded_once_and_james_changes_stay() {
        let (_, rosters) = node().await;
        assert!(seed(&rosters).await.unwrap());
        let r = rosters.roster(SITE).await.unwrap().unwrap();
        assert_eq!(r.look.key, "agentrix");
        let desks: Vec<_> = r.desks().iter().map(|d| d.id.clone()).collect();
        assert_eq!(
            desks,
            [
                "concierge",
                "agent-desk",
                "desk-your-site",
                "desk-how-it-works",
                "desk-samples",
                "desk-prices"
            ],
            "the front desk and the agent door, then the four section desks"
        );
        let your_site = r.desks().iter().find(|d| d.id == ASSESS_DESK).unwrap();
        assert_eq!(your_site.mark.color.fill, "#2f55c4");
        let on_duty: Vec<_> = r.on_duty().map(|m| m.spec.id.clone()).collect();
        for id in ["coordinator", "concierge", "guardian", "editor", "sales"] {
            assert!(on_duty.contains(&id.to_string()), "{id} on duty");
        }
        assert!(!on_duty.contains(&"cashier".to_string()));
        for m in r.on_duty() {
            assert!(!m.spec.jobs.is_empty(), "{} has a job", m.spec.id);
        }
        rosters
            .set_team(SITE, &looks::agentrix(), &["concierge", "guardian"], &[])
            .await
            .unwrap();
        assert!(!seed(&rosters).await.unwrap(), "seeding again is a no-op");
        let r = rosters.roster(SITE).await.unwrap().unwrap();
        assert!(
            !r.on_duty().any(|m| m.spec.id == "sales"),
            "James's change stays"
        );
    }

    #[tokio::test]
    async fn marks_move_from_the_records_and_an_idle_team_stays_still() {
        let (pool, _) = node().await;
        let hub = Presence::new();
        let (_, mut rx) = hub.subscribe(SITE);
        refresh(&hub, &pool).await.unwrap();
        assert!(rx.try_recv().is_err(), "nothing happening sends nothing");

        let now = stamp(Utc::now());
        exec(
            &pool,
            "INSERT INTO assessments VALUES ('a1', 'x', 'running', ?)",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO assessments VALUES ('a2', 'y', 'running', ?)",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO assessments VALUES ('old', 'z', 'running', '2026-01-01T00:00:00.000000Z')",
            &[],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO gauntlet_runs VALUES ('g1', 'f', 0, 12, NULL, ?, NULL)",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO message_outbox VALUES ('m1', 'owner.claim_reminder', 'queued', NULL)",
            &[],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO message_outbox VALUES ('m2', 'owner.morning_brief', 'queued', NULL)",
            &[],
        )
        .await;
        card(&pool, "c1", "editor", "waiting").await;
        refresh(&hub, &pool).await.unwrap();

        let state = |id: &str| hub.current(SITE, id).map(|f| (f.state, f.label));
        assert_eq!(
            state(ASSESS_DESK),
            Some((MarkState::Working, Some("Checking 2 sites".into()))),
            "a check stuck for hours isn't shown as work"
        );
        assert_eq!(
            state("guardian"),
            Some((MarkState::Working, Some(GAUNTLET.into())))
        );
        assert_eq!(
            state("sales"),
            Some((MarkState::Working, Some("Sending a follow-up email".into()))),
            "only Sales's own messages"
        );
        assert_eq!(state("editor"), Some((MarkState::Waiting, None)));
        assert_eq!(state("coordinator"), Some((MarkState::Waiting, None)));

        let before: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        refresh(&hub, &pool).await.unwrap();
        assert!(
            rx.try_recv().is_err(),
            "the same records send nothing again"
        );
        assert_eq!(before.len(), 5);

        exec(&pool, "UPDATE assessments SET state = 'report-ready'", &[]).await;
        exec(
            &pool,
            "UPDATE gauntlet_runs SET ended_at = ?, score = 12",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "UPDATE message_outbox SET status = 'sent', sent_at = '2026-01-01T00:00:00.000000Z'",
            &[],
        )
        .await;
        card(&pool, "c1", "editor", "approved").await;
        refresh(&hub, &pool).await.unwrap();
        assert_eq!(state(ASSESS_DESK), Some((MarkState::Idle, None)));
        assert_eq!(state("guardian"), Some((MarkState::Asleep, None)));
        assert_eq!(state("sales"), Some((MarkState::Asleep, None)));
        assert_eq!(state("editor"), Some((MarkState::Asleep, None)));
        assert_eq!(state("coordinator"), Some((MarkState::Asleep, None)));
    }

    #[tokio::test]
    async fn an_ask_turn_on_a_desk_is_left_alone() {
        let (pool, _) = node().await;
        let hub = Presence::new();
        hub.publish(
            SITE,
            ASSESS_DESK,
            MarkState::Working,
            Some("Reading this site"),
            None,
        );
        refresh(&hub, &pool).await.unwrap();
        assert_eq!(
            hub.current(SITE, ASSESS_DESK).unwrap().label.as_deref(),
            Some("Reading this site")
        );
    }

    #[tokio::test]
    async fn the_watcher_runs_only_while_someone_watches() {
        let (pool, _) = node().await;
        let hub = Presence::new();
        let (_, rx) = hub.subscribe(SITE);
        watch(&hub, &pool);
        assert!(!hub.claim_watch(SITE), "one watcher at a time");
        drop(rx);
        tokio::time::sleep(EVERY + Duration::from_millis(300)).await;
        assert!(hub.claim_watch(SITE), "it let go once nobody watched");
    }

    #[test]
    fn visitors_see_only_this_modules_labels_and_never_console_states() {
        let frame = |state, label: Option<&str>| PresenceFrame {
            site: SITE.into(),
            webagent: "guardian".into(),
            state,
            label: label.map(str::to_string),
            level: None,
            words: String::new(),
            seq: 1,
            at: String::new(),
        };
        let v = for_visitor(&frame(MarkState::Working, Some(GAUNTLET)), "James");
        assert_eq!(v["words"], GAUNTLET);
        let v = for_visitor(
            &frame(MarkState::Working, Some("Reading the catalogue")),
            "James",
        );
        assert!(v["label"].is_null());
        assert_eq!(v["words"], "");
        let v = for_visitor(&frame(MarkState::Waiting, None), "James");
        assert_eq!(v["words"], "Waiting for James's OK");
        let v = for_visitor(&frame(MarkState::Asleep, None), "James");
        assert_eq!(v["state"], "idle");
    }

    #[tokio::test]
    async fn the_brief_says_the_day_in_plain_words_and_is_written_once() {
        let (pool, _) = node().await;
        let hub = Presence::new();
        let now = stamp(Utc::now());
        exec(&pool, "INSERT INTO sites VALUES ('fixture', 1)", &[]).await;
        exec(
            &pool,
            "INSERT INTO assessments VALUES ('a1', 'x', 'report-ready', ?)",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO assessments VALUES ('a2', 'y', 'unreachable', ?)",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO assessments VALUES ('a3', 'fixture', 'report-ready', ?)",
            &[&now],
        )
        .await;
        let digest = format!("sha256:{}", "a".repeat(64));
        exec(
            &pool,
            "INSERT INTO gauntlet_runs VALUES ('g1', 'f', 12, 12, ?, ?, ?)",
            &[&digest, &now, &now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO message_outbox VALUES ('m1', 'owner.claim_reminder', 'sent', ?)",
            &[&now],
        )
        .await;
        exec(
            &pool,
            "INSERT INTO message_outbox VALUES ('m2', 'owner.plan_started', 'sent', ?)",
            &[&now],
        )
        .await;
        card(&pool, "c1", "editor", "waiting").await;

        let b = compose(&pool, Utc::now()).await.unwrap();
        let text: Vec<&str> = b.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            text,
            [
                "1 draft from the Editor is waiting for your OK.",
                "Assess checked 2 sites in the last day; 1 report is ready.",
                "The Guardian ran the gauntlet once; the latest run contained 12 of 12 attacks.",
                "Sales sent 2 follow-up emails.",
            ]
        );
        assert!(
            b.lines[2].receipt.is_some(),
            "the gauntlet line rests on its receipt"
        );
        assert_eq!(
            b.headline.as_deref(),
            Some("Here's agentrix.website's last day, and what waits for you.")
        );

        assert!(write_brief(&hub, &pool).await.unwrap());
        assert!(
            !write_brief(&hub, &pool).await.unwrap(),
            "the same day isn't written twice"
        );
        let (body,): (String,) = sqlx::query_as("SELECT body FROM briefs WHERE site_id = ?")
            .bind(SITE)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(body.contains("Assess checked 2 sites"));
    }

    #[tokio::test]
    async fn a_quiet_day_says_so() {
        let (pool, _) = node().await;
        let b = compose(&pool, Utc::now()).await.unwrap();
        assert_eq!(
            b.headline.as_deref(),
            Some("A quiet day on agentrix.website.")
        );
        assert_eq!(b.lines.len(), 1);
    }
}
