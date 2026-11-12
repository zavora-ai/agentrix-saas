//! The outbox: every message recorded before it is delivered, and what happened to it.
//!
//! The table is `message_outbox` (`migrations/1000_message_outbox.sql`, run by
//! [`crate::migrate`] in W9's range). One row per message per channel. Bodies are the stored copy: secrets are replaced
//! before the row is written (see [`crate::render::render_stored`]).

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::catalogue::Channel;

/// What happened to a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Recorded, delivery about to start.
    Queued,
    /// Waiting for quiet hours to end.
    Held,
    /// The provider accepted it.
    Sent,
    /// Deliberately not delivered, with a reason (no provider yet, a reserved address).
    NotSent,
    /// The provider refused it or couldn't be reached.
    Failed,
    /// The person turned this off, or the address bounced before.
    Suppressed,
}

impl Status {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Held => "held",
            Self::Sent => "sent",
            Self::NotSent => "not_sent",
            Self::Failed => "failed",
            Self::Suppressed => "suppressed",
        }
    }

    /// Parses the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => Self::Queued,
            "held" => Self::Held,
            "sent" => Self::Sent,
            "not_sent" => Self::NotSent,
            "failed" => Self::Failed,
            "suppressed" => Self::Suppressed,
            _ => return None,
        })
    }

    /// The word the console shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Sending",
            Self::Held => "Held",
            Self::Sent => "Sent",
            Self::NotSent => "Not sent",
            Self::Failed => "Failed",
            Self::Suppressed => "Turned off",
        }
    }
}

/// A row about to be recorded.
#[derive(Debug, Clone)]
pub struct NewEntry {
    /// The site it's about.
    pub site_id: Option<String>,
    /// The owner account it's for (owner and teammate messages).
    pub account_id: Option<String>,
    /// The catalogue id.
    pub message: String,
    /// `customer`, `owner` and so on.
    pub audience: String,
    /// `security`, `transactional` and so on.
    pub kind: String,
    /// `p1`, `p2` or `p3`.
    pub priority: String,
    /// Where it goes.
    pub channel: Channel,
    /// The address, phone number or device it goes to.
    pub recipient: String,
    /// How the console names the recipient.
    pub recipient_label: String,
    /// Why it was sent, in words.
    pub reason: String,
    /// The From name.
    pub from_name: String,
    /// The subject, or the one line for push and chat.
    pub subject: String,
    /// The stored plain text.
    pub body_text: String,
    /// The stored HTML (email only).
    pub body_html: Option<String>,
    /// Channel extras as JSON: push title and url, chat replies and link.
    pub payload: Option<serde_json::Value>,
    /// Sending the same key twice on one channel records one row.
    pub dedupe_key: Option<String>,
    /// The starting status.
    pub status: Status,
    /// Why, for held and not-sent rows.
    pub status_reason: Option<String>,
    /// When a held row is due.
    pub hold_until: Option<DateTime<Utc>>,
    /// The words the person agreed to, when the message rests on consent, as JSON
    /// (`{"words", "given_at", "source"}`).
    pub consent: Option<String>,
}

/// A recorded row.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    /// Its id (UUID v7).
    pub id: String,
    /// The site it's about.
    pub site_id: Option<String>,
    /// The owner account it's for.
    pub account_id: Option<String>,
    /// The catalogue id.
    pub message: String,
    /// `customer`, `owner` and so on.
    pub audience: String,
    /// `security`, `transactional` and so on.
    pub kind: String,
    /// `p1`, `p2` or `p3`.
    pub priority: String,
    /// Where it goes.
    pub channel: Channel,
    /// The address, phone number or device it goes to.
    pub recipient: String,
    /// How the console names the recipient.
    pub recipient_label: String,
    /// Why it was sent.
    pub reason: String,
    /// The From name.
    pub from_name: String,
    /// The subject or one line.
    pub subject: String,
    /// The stored plain text.
    pub body_text: String,
    /// The stored HTML.
    pub body_html: Option<String>,
    /// Channel extras.
    pub payload: Option<serde_json::Value>,
    /// The dedupe key.
    pub dedupe_key: Option<String>,
    /// What happened.
    pub status: Status,
    /// Why.
    pub status_reason: Option<String>,
    /// When a held row is due.
    pub hold_until: Option<DateTime<Utc>>,
    /// The provider that took it.
    pub provider: Option<String>,
    /// The provider's id for it.
    pub provider_ref: Option<String>,
    /// Delivery attempts so far.
    pub attempts: i64,
    /// When it was recorded.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// When the provider accepted it.
    pub sent_at: Option<DateTime<Utc>>,
    /// The consent words behind it, as JSON, if any.
    pub consent: Option<String>,
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_default()
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Entry, sqlx::Error> {
    let channel: String = row.try_get("channel")?;
    let status: String = row.try_get("status")?;
    let payload: Option<String> = row.try_get("payload")?;
    let hold_until: Option<String> = row.try_get("hold_until")?;
    let sent_at: Option<String> = row.try_get("sent_at")?;
    Ok(Entry {
        id: row.try_get("id")?,
        site_id: row.try_get("site_id")?,
        account_id: row.try_get("account_id")?,
        message: row.try_get("message")?,
        audience: row.try_get("audience")?,
        kind: row.try_get("kind")?,
        priority: row.try_get("priority")?,
        channel: Channel::parse(&channel).unwrap_or(Channel::Email),
        recipient: row.try_get("recipient")?,
        recipient_label: row.try_get("recipient_label")?,
        reason: row.try_get("reason")?,
        from_name: row.try_get("from_name")?,
        subject: row.try_get("subject")?,
        body_text: row.try_get("body_text")?,
        body_html: row.try_get("body_html")?,
        payload: payload.and_then(|p| serde_json::from_str(&p).ok()),
        dedupe_key: row.try_get("dedupe_key")?,
        status: Status::parse(&status).unwrap_or(Status::Failed),
        status_reason: row.try_get("status_reason")?,
        hold_until: hold_until.as_deref().map(parse_ts),
        provider: row.try_get("provider")?,
        provider_ref: row.try_get("provider_ref")?,
        attempts: row.try_get("attempts")?,
        created_at: parse_ts(&row.try_get::<String, _>("created_at")?),
        updated_at: parse_ts(&row.try_get::<String, _>("updated_at")?),
        sent_at: sent_at.as_deref().map(parse_ts),
        consent: row.try_get("consent").ok().flatten(),
    })
}

/// Which rows a list shows.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Only this status.
    pub status: Option<Status>,
    /// Only this channel.
    pub channel: Option<Channel>,
    /// Rows recorded before this time (for paging).
    pub before: Option<DateTime<Utc>>,
    /// At most this many rows (default 50, at most 200).
    pub limit: Option<i64>,
}

/// Counts for the console's header.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    /// Every row in scope.
    pub all: i64,
    /// Sent.
    pub sent: i64,
    /// Not sent.
    pub not_sent: i64,
    /// Held for quiet hours.
    pub held: i64,
    /// Failed.
    pub failed: i64,
}

/// The outbox store.
#[derive(Clone)]
pub struct Outbox {
    pool: SqlitePool,
}

impl Outbox {
    /// The outbox in `control.db` (already migrated by node).
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The database it writes to.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Records a row. With a dedupe key already used on the same channel, returns the existing
    /// row and `false`.
    pub async fn record(
        &self,
        new: NewEntry,
        now: DateTime<Utc>,
    ) -> Result<(Entry, bool), sqlx::Error> {
        if let Some(key) = &new.dedupe_key
            && let Some(existing) = self.by_dedupe(key, new.channel).await?
        {
            return Ok((existing, false));
        }
        let id = uuid::Uuid::now_v7().to_string();
        let payload = new.payload.as_ref().map(|p| p.to_string());
        let result = sqlx::query(
            "INSERT INTO message_outbox (id, site_id, account_id, message, audience, kind, priority, channel,
                recipient, recipient_label, reason, from_name, subject, body_text, body_html, payload,
                dedupe_key, status, status_reason, hold_until, created_at, updated_at, consent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.site_id)
        .bind(&new.account_id)
        .bind(&new.message)
        .bind(&new.audience)
        .bind(&new.kind)
        .bind(&new.priority)
        .bind(new.channel.as_str())
        .bind(&new.recipient)
        .bind(&new.recipient_label)
        .bind(&new.reason)
        .bind(&new.from_name)
        .bind(&new.subject)
        .bind(&new.body_text)
        .bind(&new.body_html)
        .bind(payload)
        .bind(&new.dedupe_key)
        .bind(new.status.as_str())
        .bind(&new.status_reason)
        .bind(new.hold_until.map(ts))
        .bind(ts(now))
        .bind(ts(now))
        .bind(&new.consent)
        .execute(&self.pool)
        .await;
        match result {
            Ok(_) => {}
            // A concurrent send with the same key won the race.
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                if let Some(key) = &new.dedupe_key
                    && let Some(existing) = self.by_dedupe(key, new.channel).await?
                {
                    return Ok((existing, false));
                }
                return Err(sqlx::Error::Database(e));
            }
            Err(e) => return Err(e),
        }
        let entry = self.get(&id).await?.ok_or(sqlx::Error::RowNotFound)?;
        Ok((entry, true))
    }

    async fn by_dedupe(&self, key: &str, channel: Channel) -> Result<Option<Entry>, sqlx::Error> {
        sqlx::query("SELECT * FROM message_outbox WHERE dedupe_key = ? AND channel = ?")
            .bind(key)
            .bind(channel.as_str())
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    /// One row.
    pub async fn get(&self, id: &str) -> Result<Option<Entry>, sqlx::Error> {
        sqlx::query("SELECT * FROM message_outbox WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(from_row)
            .transpose()
    }

    /// Records the outcome of a delivery attempt.
    #[allow(clippy::too_many_arguments)]
    pub async fn mark(
        &self,
        id: &str,
        status: Status,
        reason: Option<&str>,
        provider: Option<&str>,
        provider_ref: Option<&str>,
        attempted: bool,
        now: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        let sent_at = (status == Status::Sent).then(|| ts(now));
        sqlx::query(
            "UPDATE message_outbox SET status = ?, status_reason = ?, provider = COALESCE(?, provider),
                provider_ref = COALESCE(?, provider_ref), attempts = attempts + ?, updated_at = ?,
                sent_at = COALESCE(?, sent_at), hold_until = CASE WHEN ? = 'held' THEN hold_until ELSE NULL END
             WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(reason)
        .bind(provider)
        .bind(provider_ref)
        .bind(i64::from(attempted))
        .bind(ts(now))
        .bind(sent_at)
        .bind(status.as_str())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records a provider's later news about a sent message (a bounce or a complaint), found by
    /// the provider's reference.
    pub async fn mark_by_provider_ref(
        &self,
        provider_ref: &str,
        status: Status,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE message_outbox SET status = ?, status_reason = ?, updated_at = ? WHERE provider_ref = ?",
        )
        .bind(status.as_str())
        .bind(reason)
        .bind(ts(now))
        .bind(provider_ref)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// Takes a row that never left (`not_sent`, or `failed` at the provider) back for another
    /// try, with the copy from this request, so the console shows what goes out now. Returns
    /// `false` when the row has moved on meanwhile (another request took it, or it was sent).
    pub async fn requeue(
        &self,
        id: &str,
        new: &NewEntry,
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE message_outbox SET status = 'queued', status_reason = NULL, reason = ?, from_name = ?,
                subject = ?, body_text = ?, body_html = ?, payload = ?, consent = COALESCE(?, consent),
                updated_at = ?
             WHERE id = ? AND status IN ('not_sent', 'failed')",
        )
        .bind(&new.reason)
        .bind(&new.from_name)
        .bind(&new.subject)
        .bind(&new.body_text)
        .bind(&new.body_html)
        .bind(new.payload.as_ref().map(|p| p.to_string()))
        .bind(&new.consent)
        .bind(ts(now))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Emails of `messages` kept as not sent for `reason` since `since`, oldest first, each the
    /// newest row for its message and address: an older copy, or one followed by another try,
    /// stays as it was.
    pub async fn waiting_email(
        &self,
        messages: &[&str],
        reason: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Entry>, sqlx::Error> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        // Only `?` marks are formatted in; every value is bound.
        let marks = vec!["?"; messages.len()].join(", ");
        let sql = format!(
            "SELECT * FROM message_outbox o
             WHERE o.status = 'not_sent' AND o.channel = 'email' AND o.status_reason = ?
               AND o.created_at >= ? AND o.message IN ({marks})
               AND NOT EXISTS (SELECT 1 FROM message_outbox n WHERE n.channel = 'email'
                   AND n.message = o.message AND n.recipient = o.recipient
                   AND n.created_at > o.created_at)
             ORDER BY o.created_at LIMIT ?"
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(reason)
            .bind(ts(since));
        for m in messages {
            q = q.bind(*m);
        }
        let rows = q.bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(from_row).collect()
    }

    /// Takes a waiting row for delivery. Returns `false` when something else already did.
    pub async fn claim(&self, id: &str, now: DateTime<Utc>) -> Result<bool, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE message_outbox SET status = 'queued', status_reason = NULL, updated_at = ?
             WHERE id = ? AND status = 'not_sent'",
        )
        .bind(ts(now))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Held rows whose time has come.
    pub async fn due(&self, now: DateTime<Utc>, limit: i64) -> Result<Vec<Entry>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT * FROM message_outbox WHERE status = 'held' AND hold_until <= ? ORDER BY hold_until LIMIT ?",
        )
        .bind(ts(now))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(from_row).collect()
    }

    fn scope_sql(sites: &[String]) -> String {
        let marks = vec!["?"; sites.len()].join(", ");
        if sites.is_empty() {
            "(account_id = ?)".to_string()
        } else {
            format!("(account_id = ? OR site_id IN ({marks}))")
        }
    }

    /// The rows an owner may see: messages to their account, and messages about their sites.
    pub async fn list_for_owner(
        &self,
        account: &str,
        sites: &[String],
        filter: &Filter,
    ) -> Result<Vec<Entry>, sqlx::Error> {
        let mut sql = format!(
            "SELECT * FROM message_outbox WHERE {}",
            Self::scope_sql(sites)
        );
        if filter.status.is_some() {
            sql.push_str(" AND status = ?");
        }
        if filter.channel.is_some() {
            sql.push_str(" AND channel = ?");
        }
        if filter.before.is_some() {
            sql.push_str(" AND created_at < ?");
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        // Only fixed fragments and `?` placeholders are joined into `sql`; every value is bound.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(account);
        for s in sites {
            q = q.bind(s);
        }
        if let Some(status) = filter.status {
            q = q.bind(status.as_str());
        }
        if let Some(channel) = filter.channel {
            q = q.bind(channel.as_str());
        }
        if let Some(before) = filter.before {
            q = q.bind(ts(before));
        }
        q = q.bind(filter.limit.unwrap_or(50).clamp(1, 200));
        let rows = q.fetch_all(&self.pool).await?;
        rows.iter().map(from_row).collect()
    }

    /// Whether an owner may see this row.
    pub fn visible_to(entry: &Entry, account: &str, sites: &[String]) -> bool {
        entry.account_id.as_deref() == Some(account)
            || entry.site_id.as_ref().is_some_and(|s| sites.contains(s))
    }

    /// Counts by status for an owner's scope.
    pub async fn counts_for_owner(
        &self,
        account: &str,
        sites: &[String],
    ) -> Result<Counts, sqlx::Error> {
        let sql = format!(
            "SELECT status, COUNT(*) AS n FROM message_outbox WHERE {} GROUP BY status",
            Self::scope_sql(sites)
        );
        // Only fixed fragments and `?` placeholders are joined into `sql`; every value is bound.
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(account);
        for s in sites {
            q = q.bind(s);
        }
        let mut counts = Counts::default();
        for row in q.fetch_all(&self.pool).await? {
            let status: String = row.try_get("status")?;
            let n: i64 = row.try_get("n")?;
            counts.all += n;
            match Status::parse(&status) {
                Some(Status::Sent) => counts.sent += n,
                Some(Status::NotSent) => counts.not_sent += n,
                Some(Status::Held) => counts.held += n,
                Some(Status::Failed) => counts.failed += n,
                _ => {}
            }
        }
        Ok(counts)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An in-memory database with node's migration for the outbox applied.
    pub(crate) async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrate(&pool).await.unwrap();
        pool
    }

    fn new_entry(key: Option<&str>) -> NewEntry {
        NewEntry {
            site_id: Some("site_k".into()),
            account_id: None,
            message: "customer.order_received".into(),
            audience: "customer".into(),
            kind: "transactional".into(),
            priority: "p2".into(),
            channel: Channel::Email,
            recipient: "amina@example.com".into(),
            recipient_label: "Amina (customer)".into(),
            reason: "Njeri paid for order KB-1042".into(),
            from_name: "Kilele Bakery (sample)".into(),
            subject: "Kilele Bakery has your order KB-1042".into(),
            body_text: "Thank you".into(),
            body_html: Some("<p>Thank you</p>".into()),
            payload: None,
            dedupe_key: key.map(str::to_string),
            status: Status::Queued,
            status_reason: None,
            hold_until: None,
            consent: None,
        }
    }

    #[tokio::test]
    async fn records_once_per_dedupe_key_and_scopes_by_owner() {
        let outbox = Outbox::new(pool().await);
        let now = Utc::now();
        let (a, fresh) = outbox
            .record(new_entry(Some("KB-1042:paid")), now)
            .await
            .unwrap();
        assert!(fresh);
        let (b, fresh) = outbox
            .record(new_entry(Some("KB-1042:paid")), now)
            .await
            .unwrap();
        assert!(!fresh);
        assert_eq!(a.id, b.id);
        outbox
            .mark(
                &a.id,
                Status::NotSent,
                Some("Email isn't switched on yet"),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();
        let mine = outbox
            .list_for_owner("acct_w", &["site_k".into()], &Filter::default())
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].status, Status::NotSent);
        let theirs = outbox
            .list_for_owner("acct_x", &["site_other".into()], &Filter::default())
            .await
            .unwrap();
        assert!(theirs.is_empty());
        let counts = outbox
            .counts_for_owner("acct_w", &["site_k".into()])
            .await
            .unwrap();
        assert_eq!((counts.all, counts.not_sent), (1, 1));
    }
}
