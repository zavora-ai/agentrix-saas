//! The weekly fix: agentrix.website's newsletter agent.
//!
//! - **Who gets it.** Only readers who ticked the weekly fix and then used the confirm link in
//!   W9's `newsletter.confirm` (`home_contacts.weekly_confirmed_at`), from the keep step or the
//!   sign-up on screen 7. Every issue carries a one-tap stop that deletes their details.
//! - **Who writes it.** The Editor (W1's roster for `agentrix-home`) drafts it on Thursdays
//!   from what the week's free checks found, counted across sites and never naming one: the
//!   Model Gateway (purpose `brief`, `x-agentrix-webagent: editor`, job `weekly-fix`) writes a
//!   title and at most five lines from numbered sources, and a number no source holds drops the
//!   draft. No model, no readers or no findings: no draft.
//! - **Who sends it.** Nobody until James says yes: the draft becomes a decision card on
//!   agentrix.website ("waiting for James's OK"); approving it runs [`NewsletterRunner`], which
//!   sends W9's `newsletter.issue` to each confirmed reader. Declining keeps it unsent.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc, Weekday};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::desks::DeskModel;
use crate::keep::{ReportSender, SOURCE, WEEKLY_WORDS};

/// The tool a draft's card runs when James approves it.
pub const TOOL: &str = "newsletter.send";
/// agentrix.website's site id.
const SITE: &str = crate::desks::SITE;

/// The newsletter's state for the page: the last issue sent, and a draft waiting for James.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Issues {
    /// The last issue's title, once one has gone out.
    pub last_sent: Option<String>,
    /// Whether a draft waits for James's OK.
    pub waiting: bool,
}

/// The newsletter agent.
#[derive(Clone)]
pub struct Newsletter {
    pool: Option<SqlitePool>,
    model: Option<Arc<dyn DeskModel>>,
    decisions: Option<agentrix_decisions::Decisions>,
    sender: Option<Arc<dyn ReportSender>>,
    origin: String,
}

fn eat(t: DateTime<Utc>) -> DateTime<chrono::FixedOffset> {
    t.with_timezone(&chrono::FixedOffset::east_opt(3 * 3600).expect("UTC+3 is a valid offset"))
}

/// The ISO week a moment falls in, in Nairobi: `2026-W45`.
pub fn week_of(t: DateTime<Utc>) -> String {
    let w = eat(t).iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

impl Newsletter {
    /// The agent on `pool`, drafting with `model`, asking James through `decisions` and sending
    /// through `sender`.
    pub fn new(
        pool: Option<SqlitePool>,
        model: Option<Arc<dyn DeskModel>>,
        decisions: Option<agentrix_decisions::Decisions>,
        sender: Option<Arc<dyn ReportSender>>,
        origin: &str,
    ) -> Self {
        Self {
            pool,
            model,
            decisions,
            sender,
            origin: origin.trim_end_matches('/').to_owned(),
        }
    }

    /// Confirmed readers.
    pub async fn readers(&self) -> u64 {
        let Some(pool) = &self.pool else { return 0 };
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(DISTINCT contact) FROM home_contacts WHERE weekly = 1 AND weekly_confirmed_at IS NOT NULL",
        )
        .fetch_one(pool)
        .await
        .map_or(0, |n| u64::try_from(n).unwrap_or(0))
    }

    /// The last issue sent, and whether a draft waits.
    pub async fn issues(&self) -> Issues {
        let Some(pool) = &self.pool else {
            return Issues::default();
        };
        let last_sent: Option<String> = sqlx::query_scalar(
            "SELECT title FROM home_issues WHERE state = 'sent' ORDER BY sent_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let waiting: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM home_issues WHERE state = 'waiting' LIMIT 1")
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        Issues {
            last_sent,
            waiting: waiting.is_some(),
        }
    }

    /// What the week's free checks found, counted across sites and naming none.
    async fn patterns(&self, pool: &SqlitePool, now: DateTime<Utc>) -> Vec<String> {
        let since = (now - Duration::days(7)).to_rfc3339();
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT f.title, f.category, COUNT(DISTINCT f.assessment_id) AS n FROM findings f \
             JOIN assessments a ON a.assessment_id = f.assessment_id \
             WHERE a.created_at >= ? GROUP BY f.title, f.category ORDER BY n DESC LIMIT 8",
        )
        .bind(since)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        rows.into_iter()
            .map(|(title, category, n)| {
                format!("{n} of this week's free checks found: {title} ({category}).")
            })
            .collect()
    }

    /// Drafts this week's fix and asks James, once a week, on Thursdays from 15:00 in Nairobi.
    /// Returns whether a draft was raised.
    pub async fn draft_week(&self, now: DateTime<Utc>) -> anyhow::Result<bool> {
        let (Some(pool), Some(model), Some(decisions)) = (&self.pool, &self.model, &self.decisions)
        else {
            return Ok(false);
        };
        let week = week_of(now);
        let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM home_issues WHERE week = ?")
            .bind(&week)
            .fetch_optional(pool)
            .await?;
        let readers = self.readers().await;
        if exists.is_some() || readers == 0 {
            return Ok(false);
        }
        let found = self.patterns(pool, now).await;
        if found.is_empty() {
            return Ok(false);
        }
        let sources: String = found
            .iter()
            .enumerate()
            .map(|(i, f)| format!("[{}] {f}\n", i + 1))
            .collect();
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title", "lines"],
            "properties": {
                "title": { "type": "string" },
                "lines": { "type": "array", "items": { "type": "string" }, "maxItems": 5 },
            },
        });
        let system = "You are the Editor on agentrix.website, an AI WebAgent. Draft this week's website fix: one thing a \
             small business owner can fix on their own website this week, chosen from what free checks found most. \
             Write a short title in plain words and at most five short lines, each one step the owner can do themselves. \
             Use only the numbered sources for what was found; never name a site or a business, and never state a \
             number the sources don't hold. James at Zavora checks every issue before it goes.";
        let user = format!("Numbered sources:\n{sources}");
        let text = model.reply("editor", system, &user, &schema).await?;
        let body = text
            .find('{')
            .zip(text.rfind('}'))
            .map_or(text.as_str(), |(a, b)| &text[a..=b.max(a)]);
        let v: Value = serde_json::from_str(body)?;
        let title = v["title"]
            .as_str()
            .unwrap_or("")
            .trim()
            .chars()
            .take(120)
            .collect::<String>();
        let lines: Vec<String> = v["lines"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|l| l.trim().chars().take(200).collect())
                    .filter(|l: &String| !l.is_empty())
                    .take(5)
                    .collect()
            })
            .unwrap_or_default();
        if title.is_empty() || lines.is_empty() {
            anyhow::bail!("the Editor's draft had no title or lines");
        }
        let digits = |s: &str| -> Vec<String> {
            s.split(|c: char| !c.is_ascii_digit())
                .filter(|d| !d.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let known: std::collections::HashSet<String> = digits(&sources).into_iter().collect();
        if let Some(x) = lines
            .iter()
            .chain(std::iter::once(&title))
            .flat_map(|l| digits(l))
            .find(|d| !known.contains(d))
        {
            anyhow::bail!("the Editor's draft states {x}, which no source holds");
        }
        let id = format!("issue_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO home_issues (id, week, title, lines, state, readers, created_at) VALUES (?, ?, ?, ?, 'waiting', ?, ?)",
        )
        .bind(&id)
        .bind(&week)
        .bind(&title)
        .bind(serde_json::to_string(&lines)?)
        .bind(i64::try_from(readers).unwrap_or(i64::MAX))
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
        let card = decisions
            .raise(agentrix_decisions::NewCard {
                kind: "newsletter".into(),
                title: format!(
                    "Send this week's website fix to {readers} {}?",
                    if readers == 1 { "reader" } else { "readers" }
                ),
                summary: title.clone(),
                effects: vec![
                    format!(
                        "Sends “{title}” by email to the {readers} {} who confirmed the weekly fix",
                        if readers == 1 { "reader" } else { "readers" }
                    ),
                    "Each email has a one-tap stop that deletes the reader's details".into(),
                    "Nothing is sent if you decline".into(),
                ],
                effect: agentrix_types::EffectRequest {
                    site: agentrix_types::SiteId::new(SITE),
                    webagent: "editor".into(),
                    tool: TOOL.into(),
                    args: json!({ "issue": id, "title": title, "lines": lines }),
                    amount: None,
                    expiry: now + Duration::days(3),
                },
                raised_by: agentrix_decisions::RaisedBy {
                    webagent: "editor".into(),
                    job: Some("weekly-fix".into()),
                },
                risk: agentrix_decisions::Risk {
                    level: agentrix_decisions::RiskLevel::Medium,
                    reasons: vec![
                        "Goes to people outside the business".into(),
                        "Can't be unsent".into(),
                    ],
                },
                approval: agentrix_decisions::ApprovalKind::Reviewed,
                back: None,
                dedupe_key: Some(format!("weekly-fix:{week}")),
            })
            .await?;
        sqlx::query("UPDATE home_issues SET card_id = ? WHERE id = ?")
            .bind(&card.id)
            .bind(&id)
            .execute(pool)
            .await?;
        Ok(true)
    }

    /// Sends an approved issue to each confirmed reader. Returns how many it went to.
    pub async fn send(&self, issue: &str, now: DateTime<Utc>) -> anyhow::Result<u64> {
        let (Some(pool), Some(sender)) = (&self.pool, &self.sender) else {
            anyhow::bail!("the newsletter can't send from here");
        };
        let row: Option<(String, String, String)> =
            sqlx::query_as("SELECT title, lines, state FROM home_issues WHERE id = ?")
                .bind(issue)
                .fetch_optional(pool)
                .await?;
        let Some((title, lines, state)) = row else {
            anyhow::bail!("there's no such issue");
        };
        if state == "sent" {
            return Ok(0);
        }
        let lines: Vec<String> = serde_json::from_str(&lines)?;
        let readers: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT contact, token, weekly_confirmed_at FROM home_contacts \
             WHERE weekly = 1 AND weekly_confirmed_at IS NOT NULL AND token IS NOT NULL GROUP BY contact",
        )
        .fetch_all(pool)
        .await?;
        let mut sent = 0;
        for (email, token, confirmed_at) in readers {
            let given = DateTime::parse_from_rfc3339(&confirmed_at)
                .map(|t| {
                    eat(t.with_timezone(&Utc))
                        .format("%a %-d %b, %H:%M")
                        .to_string()
                })
                .unwrap_or(confirmed_at);
            let context = json!({
                "person": { "name": null },
                "issue": {
                    "title": title,
                    "lines": lines,
                    "link": { "label": "Check your own site for free", "url": format!("{}/assess", self.origin) },
                },
                "unsubscribe_url": format!("{}/_home/fix/stop?token={token}", self.origin),
                "consent": { "words": WEEKLY_WORDS, "given_at": given, "source": SOURCE },
            });
            // One reader's email refused doesn't stop the issue for the others.
            match sender
                .send(
                    crate::keep::ISSUE_MESSAGE,
                    &email,
                    context,
                    &format!("{issue}:{token}"),
                )
                .await
            {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), issue, "home: a reader's weekly fix")
                }
            }
        }
        sqlx::query("UPDATE home_issues SET state = 'sent', sent_at = ?, readers = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(sent)
            .bind(issue)
            .execute(pool)
            .await?;
        Ok(u64::try_from(sent).unwrap_or(0))
    }

    /// Marks a declined draft, so it is never sent.
    pub async fn declined(&self, issue: &str) -> anyhow::Result<()> {
        if let Some(pool) = &self.pool {
            sqlx::query(
                "UPDATE home_issues SET state = 'declined' WHERE id = ? AND state = 'waiting'",
            )
            .bind(issue)
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    /// Drafts on Thursdays from 15:00 in Nairobi, checking every half hour.
    pub fn task(self) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(1800));
            loop {
                every.tick().await;
                let now = Utc::now();
                let local = eat(now);
                if local.weekday() != Weekday::Thu || local.hour() < 15 {
                    continue;
                }
                match self.draft_week(now).await {
                    Ok(true) => {
                        tracing::info!("home: the Editor drafted this week's fix for James")
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!(error = %format!("{e:#}"), "home: the Editor's draft"),
                }
            }
        })
    }
}

/// Runs an approved weekly fix: sends it to each confirmed reader.
pub struct NewsletterRunner(pub Newsletter);

#[async_trait]
impl agentrix_decisions::EffectRunner for NewsletterRunner {
    fn tool(&self) -> &str {
        TOOL
    }

    async fn run(
        &self,
        card: &agentrix_decisions::DecisionCardV2,
        _receipt: &str,
    ) -> anyhow::Result<Option<String>> {
        let issue = card.effect.args["issue"].as_str().unwrap_or_default();
        let n = self.0.send(issue, Utc::now()).await?;
        tracing::info!(
            issue,
            readers = n,
            "home: James approved this week's fix; sent"
        );
        Ok(None)
    }

    async fn declined(
        &self,
        card: &agentrix_decisions::DecisionCardV2,
        _receipt: &str,
    ) -> anyhow::Result<()> {
        self.0
            .declined(card.effect.args["issue"].as_str().unwrap_or_default())
            .await
    }
}
