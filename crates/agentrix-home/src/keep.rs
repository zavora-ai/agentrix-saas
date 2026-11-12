//! "Keep this report": the one field after the free check (screen 1).
//!
//! A visitor who has seen their findings may leave an email address so the report reaches them.
//! The row keeps the exact words they agreed to, when, and on which page, because consent is
//! only as good as its record. The weekly fix is a separate, unticked choice and is never
//! bundled with the report. Rows are deleted 30 days after they're made, as the form says,
//! unless the weekly fix is later confirmed (slice d).
//!
//! The report goes out through W9's outbox as `lead.report`. Until W9's catalogue has that
//! message, nothing is sent and the visitor is told so, with the report's link to keep.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// The words under the field, stored with every row: what the visitor agreed to.
pub const REPORT_WORDS: &str = "We use your email to send this report. We keep it for 30 days.";
/// The words beside the weekly tick, stored only when it was ticked.
pub const WEEKLY_WORDS: &str = "Also send me one website fix a week. Stop any time.";
/// The catalogue message that carries the report (W9).
pub const REPORT_MESSAGE: &str = "lead.report";
/// The double opt-in for the weekly fix (W9).
pub const CONFIRM_MESSAGE: &str = "newsletter.confirm";
/// One week's fix, to a confirmed reader (W9).
pub const ISSUE_MESSAGE: &str = "newsletter.issue";
/// Where the words above were agreed to, as the message's consent record says.
pub const SOURCE: &str = "agentrix.website check";
/// The words beside screen 7's sign-up, stored with every reader who joins there.
pub const JOIN_WORDS: &str =
    "Send me the weekly fix. We use your email only for this. Stop any time.";

/// Runs this crate's migrations (W10's range, 1100–1199) with their own table. Node lists it in
/// its `MIGRATIONS`.
pub fn migrate(pool: &SqlitePool) -> futures::future::BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_home");
        m.run(pool)
            .await
            .map_err(|e| anyhow::anyhow!("agentrix-home migrations: {e}"))?;
        Ok(())
    })
}

/// Sends one of W9's messages, or says sending isn't switched on yet.
#[async_trait]
pub trait ReportSender: Send + Sync {
    /// `Ok(true)` once the email has gone to `email` (now, or the first time it was asked for);
    /// `Ok(false)` while email isn't switched on (or the catalogue doesn't have `message`); an
    /// error when it couldn't be sent, such as the provider refusing it.
    async fn send(
        &self,
        message: &str,
        email: &str,
        context: serde_json::Value,
        key: &str,
    ) -> anyhow::Result<bool>;
}

/// W9's outbox.
pub struct OutboxSender(pub agentrix_messages::Messages);

#[async_trait]
impl ReportSender for OutboxSender {
    async fn send(
        &self,
        message: &str,
        email: &str,
        context: serde_json::Value,
        key: &str,
    ) -> anyhow::Result<bool> {
        if agentrix_messages::catalogue::get(message).is_none() {
            return Ok(false);
        }
        let sent = self
            .0
            .send(agentrix_messages::send::Outgoing {
                message: message.into(),
                site_id: None,
                to: agentrix_messages::send::Recipient::email(email, None),
                context,
                reason: match message {
                    CONFIRM_MESSAGE => "A visitor ticked the weekly fix on agentrix.website; nothing more is sent until they confirm".into(),
                    ISSUE_MESSAGE => "A reader confirmed the weekly fix and James approved this issue".into(),
                    _ => "A visitor asked agentrix.website to send them their free check".into(),
                },
                dedupe_key: Some(format!("{message}:{key}")),
            })
            .await?;
        // The page tells the visitor to check their inbox, so only the email counts.
        let email_rows = || {
            sent.entries
                .iter()
                .filter(|e| e.channel == agentrix_messages::Channel::Email)
        };
        if email_rows().any(|e| e.status == agentrix_messages::Status::Sent) {
            return Ok(true);
        }
        if let Some(e) = email_rows().find(|e| e.status == agentrix_messages::Status::Failed) {
            anyhow::bail!(
                "{message} to a visitor wasn't sent: {}",
                e.status_reason
                    .as_deref()
                    .unwrap_or("the provider refused it")
            );
        }
        Ok(false)
    }
}

/// What a check found, from node's own records (`assessments`, `findings`).
#[derive(Debug, Clone, Default)]
pub struct CheckFacts {
    /// The site checked, as its host.
    pub site: String,
    /// When it finished.
    pub at: Option<DateTime<Utc>>,
    /// The report's summary in plain words.
    pub headline: String,
    /// Up to five findings, the most urgent first.
    pub findings: Vec<String>,
}

/// "Sun 8 Nov, 10:12" in Nairobi.
fn eat(t: DateTime<Utc>) -> String {
    let eat = chrono::FixedOffset::east_opt(3 * 3600).expect("UTC+3 is a valid offset");
    t.with_timezone(&eat).format("%a %-d %b, %H:%M").to_string()
}

/// Reads a check's facts; a check this node doesn't hold reads as empty.
pub async fn check_facts(pool: &SqlitePool, id: &str) -> CheckFacts {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT url, COALESCE(finished_at, created_at), report FROM assessments WHERE assessment_id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap_or_default();
    let Some((url, at, report)) = row else {
        return CheckFacts::default();
    };
    let findings: Vec<(String,)> = sqlx::query_as(
        "SELECT title FROM findings WHERE assessment_id = ? \
         ORDER BY CASE severity WHEN 'urgent' THEN 0 WHEN 'warn' THEN 1 ELSE 2 END, at LIMIT 5",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let site = url
        .split_once("://")
        .map_or(url.as_str(), |(_, r)| r)
        .trim_end_matches('/')
        .to_owned();
    let headline = report
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
        .and_then(|r| r["summary"].as_str().map(str::to_owned))
        .unwrap_or_default();
    CheckFacts {
        site,
        at: DateTime::parse_from_rfc3339(&at)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        headline,
        findings: findings.into_iter().map(|(t,)| t).collect(),
    }
}

/// What the form posts.
#[derive(Debug, Deserialize)]
pub struct KeepForm {
    /// Where to send the report.
    #[serde(default)]
    pub email: String,
    /// The weekly fix, ticked or not.
    #[serde(default)]
    pub weekly: Option<String>,
    /// Which check: `asmt_…`.
    #[serde(default)]
    pub assessment: String,
}

/// What the page shows after the form.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Kept {
    /// `sent`, `saved` (sending not switched on) or `invalid`.
    pub status: &'static str,
    /// One plain sentence for the visitor.
    pub message: String,
    /// The report's own page, to keep.
    pub report_url: String,
}

/// The keep step's store and sender.
#[derive(Clone)]
pub struct Keep {
    pool: Option<SqlitePool>,
    sender: Option<Arc<dyn ReportSender>>,
    origin: String,
}

fn valid_email(e: &str) -> bool {
    let e = e.trim();
    let Some((user, domain)) = e.split_once('@') else {
        return false;
    };
    e.len() <= 254
        && !user.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !e.chars().any(|c| c.is_whitespace() || c == '<' || c == '>')
        && !domain.contains('@')
}

fn valid_assessment(id: &str) -> bool {
    id.starts_with("asmt_")
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl Keep {
    /// The keep step on `pool`, sending through `sender`.
    pub fn new(pool: Option<SqlitePool>, sender: Option<Arc<dyn ReportSender>>) -> Self {
        Self {
            pool,
            sender,
            origin: "https://agentrix.website".into(),
        }
    }

    /// The same keep step, linking to `origin` in its messages.
    pub fn with_origin(mut self, origin: &str) -> Self {
        self.origin = origin.trim_end_matches('/').to_owned();
        self
    }

    /// Stores the request and sends the report if sending is on.
    pub async fn keep(&self, form: &KeepForm, source_page: &str, now: DateTime<Utc>) -> Kept {
        let report_url = format!("/assess/{}", form.assessment);
        let email = form.email.trim().to_ascii_lowercase();
        if !valid_assessment(&form.assessment) || !valid_email(&email) {
            return Kept {
                status: "invalid",
                message: "Please check the email address.".into(),
                report_url: if valid_assessment(&form.assessment) {
                    report_url
                } else {
                    "/".into()
                },
            };
        }
        let weekly = form.weekly.as_deref().is_some_and(|v| !v.is_empty());
        let Some(pool) = &self.pool else {
            return Kept {
                status: "saved",
                message: "We couldn't save that just now. Keep this link to your report.".into(),
                report_url,
            };
        };
        let id = format!("kept_{}", uuid::Uuid::new_v4().simple());
        let stored = sqlx::query(
            "INSERT INTO home_contacts (id, channel, contact, assessment_id, report_url, report_words, \
             weekly, weekly_words, source_page, created_at, delete_after, token) \
             VALUES (?, 'email', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (assessment_id, contact) DO UPDATE SET weekly = MAX(weekly, excluded.weekly), \
             weekly_words = COALESCE(excluded.weekly_words, weekly_words), token = COALESCE(token, excluded.token)",
        )
        .bind(&id)
        .bind(&email)
        .bind(&form.assessment)
        .bind(&report_url)
        .bind(REPORT_WORDS)
        .bind(weekly as i64)
        .bind(weekly.then_some(WEEKLY_WORDS))
        .bind(source_page)
        .bind(now.to_rfc3339())
        .bind((now + Duration::days(30)).to_rfc3339())
        .bind(uuid::Uuid::new_v4().simple().to_string())
        .execute(pool)
        .await;
        if let Err(e) = stored {
            tracing::error!(error = %e, "home: keeping a report");
            return Kept {
                status: "saved",
                message: "We couldn't save that just now. Keep this link to your report.".into(),
                report_url,
            };
        }
        let token: String = sqlx::query_scalar(
            "SELECT token FROM home_contacts WHERE assessment_id = ? AND contact = ?",
        )
        .bind(&form.assessment)
        .bind(&email)
        .fetch_one(pool)
        .await
        .unwrap_or_default();
        let facts = check_facts(pool, &form.assessment).await;
        let given_at = eat(now);
        let report = serde_json::json!({
            "person": { "name": null },
            "check": {
                "site": if facts.site.is_empty() { "your site".to_owned() } else { facts.site.clone() },
                "checked_at": eat(facts.at.unwrap_or(now)),
                "headline": if facts.headline.is_empty() { "Your free website check.".to_owned() } else { facts.headline.clone() },
                "findings": facts.findings,
                "report_url": format!("{}{report_url}", self.origin),
                "keep_url": format!("{}/start", self.origin),
                "expires": (now + Duration::days(30)).with_timezone(&chrono::FixedOffset::east_opt(3 * 3600).expect("UTC+3")).format("%a %-d %b").to_string(),
            },
            "consent": { "words": REPORT_WORDS, "given_at": given_at, "source": SOURCE },
        });
        let key = format!("{}:{email}", form.assessment);
        let send = |message: &'static str, context: serde_json::Value| {
            let sender = self.sender.clone();
            let email = email.clone();
            let key = key.clone();
            async move {
                match sender {
                    Some(s) => s
                        .send(message, &email, context, &key)
                        .await
                        .map_err(|e| {
                            tracing::error!(error = %format!("{e:#}"), message, "home: sending a kept report");
                        }),
                    None => Ok(false),
                }
            }
        };
        let outcome = send(REPORT_MESSAGE, report).await;
        let sent = outcome == Ok(true);
        if sent {
            let _ = sqlx::query(
                "UPDATE home_contacts SET report_status = 'sent' WHERE assessment_id = ? AND contact = ?",
            )
            .bind(&form.assessment)
            .bind(&email)
            .execute(pool)
            .await;
        }
        // The weekly fix is its own choice: a confirm link first, and nothing more until it's used.
        let confirm_sent = weekly
            && !token.is_empty()
            && send(
                CONFIRM_MESSAGE,
                serde_json::json!({
                    "person": { "name": null },
                    "newsletter": { "day": "Friday" },
                    "confirm_url": format!("{}/_home/fix/confirm?token={token}", self.origin),
                    "forget_after": "30 days",
                    "consent": { "words": WEEKLY_WORDS, "given_at": given_at, "source": SOURCE },
                }),
            )
            .await
                == Ok(true);
        let weekly_line = match (weekly, confirm_sent) {
            (true, true) => " Check your email to confirm the weekly fix.",
            (true, false) => " You'll get the weekly fix only after you confirm it.",
            _ => "",
        };
        Kept {
            status: if sent { "sent" } else { "saved" },
            message: match outcome {
                Ok(true) => format!("Sent to {email}. We keep it for 30 days.{weekly_line}"),
                Ok(false) => format!(
                    "Saved for {email}. Email isn't switched on yet, so keep this link for now.{weekly_line}"
                ),
                Err(()) => format!(
                    "Saved for {email}. We couldn't email it just now, so keep this link for now.{weekly_line}"
                ),
            },
            report_url,
        }
    }

    /// Reports asked for and sent, for the Sales WebAgent's state on screen 2.
    pub async fn counts(&self) -> crate::view::KeptCounts {
        let Some(pool) = &self.pool else {
            return crate::view::KeptCounts::default();
        };
        let row: Result<(i64, i64), _> = sqlx::query_as(
            // Newsletter sign-ups (screen 7) keep a row here too, but they asked for no report.
            "SELECT COUNT(*), COALESCE(SUM(report_status = 'sent'), 0) FROM home_contacts \
             WHERE assessment_id <> 'newsletter'",
        )
        .fetch_one(pool)
        .await;
        match row {
            Ok((asked, sent)) => crate::view::KeptCounts {
                asked: u64::try_from(asked).unwrap_or(0),
                sent: u64::try_from(sent).unwrap_or(0),
            },
            Err(e) => {
                tracing::warn!(error = %e, "home: counting kept reports");
                crate::view::KeptCounts::default()
            }
        }
    }

    /// Deletes what the form promised to keep for 30 days only.
    pub async fn purge(&self, now: DateTime<Utc>) -> anyhow::Result<u64> {
        let Some(pool) = &self.pool else { return Ok(0) };
        let done = sqlx::query(
            "DELETE FROM home_contacts WHERE delete_after IS NOT NULL AND delete_after < ?",
        )
        .bind(now.to_rfc3339())
        .execute(pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// Purges once an hour, for node to spawn.
    pub fn task(self) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(async move {
            let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
            loop {
                every.tick().await;
                match self.purge(Utc::now()).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(deleted = n, "home: deleted kept reports past 30 days"),
                    Err(e) => tracing::error!(error = %e, "home: purging kept reports"),
                }
            }
        })
    }
}

impl Keep {
    /// Screen 7's sign-up: stored with its words, then the same confirm link as the tick.
    pub async fn join(&self, email: &str, now: DateTime<Utc>) -> Kept {
        let email = email.trim().to_ascii_lowercase();
        if !valid_email(&email) {
            return Kept {
                status: "invalid",
                message: "Please check the email address.".into(),
                report_url: "/".into(),
            };
        }
        let Some(pool) = &self.pool else {
            return Kept {
                status: "saved",
                message: "We couldn't save that just now. Try again in a minute.".into(),
                report_url: "/".into(),
            };
        };
        let token = uuid::Uuid::new_v4().simple().to_string();
        let stored = sqlx::query(
            "INSERT INTO home_contacts (id, channel, contact, assessment_id, report_url, report_words, \
             weekly, weekly_words, source_page, created_at, delete_after, token) \
             VALUES (?, 'email', ?, 'newsletter', '', ?, 1, ?, '/#start', ?, ?, ?) \
             ON CONFLICT (assessment_id, contact) DO NOTHING",
        )
        .bind(format!("kept_{}", uuid::Uuid::new_v4().simple()))
        .bind(&email)
        .bind(JOIN_WORDS)
        .bind(JOIN_WORDS)
        .bind(now.to_rfc3339())
        .bind((now + Duration::days(30)).to_rfc3339())
        .bind(&token)
        .execute(pool)
        .await;
        if let Err(e) = stored {
            tracing::error!(error = %e, "home: a newsletter sign-up");
            return Kept {
                status: "saved",
                message: "We couldn't save that just now. Try again in a minute.".into(),
                report_url: "/".into(),
            };
        }
        let token: String = sqlx::query_scalar(
            "SELECT token FROM home_contacts WHERE assessment_id = 'newsletter' AND contact = ?",
        )
        .bind(&email)
        .fetch_one(pool)
        .await
        .unwrap_or(token);
        let context = serde_json::json!({
            "person": { "name": null },
            "newsletter": { "day": "Friday" },
            "confirm_url": format!("{}/_home/fix/confirm?token={token}", self.origin),
            "forget_after": "30 days",
            "consent": { "words": JOIN_WORDS, "given_at": eat(now), "source": "agentrix.website sign-up" },
        });
        let outcome = match &self.sender {
            Some(s) => s
                .send(
                    CONFIRM_MESSAGE,
                    &email,
                    context,
                    &format!("newsletter:{email}"),
                )
                .await
                .map_err(|e| {
                    tracing::error!(error = %format!("{e:#}"), "home: a newsletter confirm link");
                }),
            None => Ok(false),
        };
        Kept {
            status: if outcome == Ok(true) { "sent" } else { "saved" },
            message: match outcome {
                Ok(true) => format!("Check {email} for a confirm link."),
                // Messages::send_waiting sends it once email is switched on.
                Ok(false) => format!(
                    "Saved for {email}. Email isn't switched on yet; we'll ask you to confirm when it is."
                ),
                Err(()) => format!(
                    "Saved for {email}. We couldn't send the confirm link just now. Try again in a minute."
                ),
            },
            report_url: "/".into(),
        }
    }

    /// The confirm link in `newsletter.confirm`: the weekly fix starts, and the row is kept
    /// until its reader stops it. Returns whether a request matched.
    pub async fn confirm(&self, token: &str, now: DateTime<Utc>) -> bool {
        let (Some(pool), false) = (&self.pool, token.is_empty()) else {
            return false;
        };
        sqlx::query(
            "UPDATE home_contacts SET weekly_confirmed_at = COALESCE(weekly_confirmed_at, ?), delete_after = NULL \
             WHERE token = ? AND weekly = 1",
        )
        .bind(now.to_rfc3339())
        .bind(token)
        .execute(pool)
        .await
        .is_ok_and(|r| r.rows_affected() > 0)
    }

    /// The one-tap stop: the reader's details are deleted. Returns whether a row matched.
    pub async fn stop(&self, token: &str) -> bool {
        let (Some(pool), false) = (&self.pool, token.is_empty()) else {
            return false;
        };
        sqlx::query("DELETE FROM home_contacts WHERE token = ?")
            .bind(token)
            .execute(pool)
            .await
            .is_ok_and(|r| r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plausible_addresses_and_checks_pass() {
        assert!(valid_email("wanjiru@kilele.example"));
        assert!(!valid_email("wanjiru"));
        assert!(!valid_email("a b@c.example"));
        assert!(!valid_email("x@localhost"));
        assert!(valid_assessment("asmt_01HX9"));
        assert!(!valid_assessment("../etc"));
        assert!(!valid_assessment("asmt_<script>"));
    }
}
