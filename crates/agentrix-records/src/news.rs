//! Stories, authors, corrections, story timelines, subscribers and digest subscriptions.

use serde::Serialize;

use crate::{Records, new_id, sha256_hex};

/// A story.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Story {
    /// URL slug.
    pub slug: String,
    /// Section, such as `transport`.
    pub section: String,
    /// Kicker above the headline.
    pub kicker: Option<String>,
    /// Headline.
    pub title: String,
    /// Standfirst.
    pub standfirst: String,
    /// Paragraphs as a JSON array of strings.
    #[serde(skip)]
    pub body: String,
    /// The author.
    pub author_id: String,
    /// The running story it belongs to, such as `rail`.
    pub topic: Option<String>,
    /// `free` or `subscriber`.
    pub entitlement: String,
    /// `published` or `draft`.
    pub status: String,
    /// Published.
    pub published_at: i64,
    /// Last updated.
    pub updated_at: i64,
}

impl Story {
    /// The paragraphs.
    pub fn paragraphs(&self) -> Vec<String> {
        serde_json::from_str(&self.body).unwrap_or_default()
    }

    /// Whether only subscribers read it in full.
    pub fn locked(&self) -> bool {
        self.entitlement == "subscriber"
    }
}

/// An author.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Author {
    /// Id.
    pub author_id: String,
    /// Name.
    pub name: String,
    /// "Transport reporter".
    pub role: String,
    /// A line about them.
    pub bio: String,
}

/// A correction on a story.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Correction {
    /// The story.
    pub story_slug: String,
    /// When it was corrected.
    pub at: i64,
    /// What changed: "KSh 41bn, not 48bn".
    pub what: String,
    /// The note readers see.
    pub note: String,
}

/// A step in a running story.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TimelineStep {
    /// The running story.
    pub topic: String,
    /// When.
    pub at: i64,
    /// The line.
    pub line: String,
    /// The story it links to.
    pub story_slug: Option<String>,
    /// `step`, `correction`, `scheduled`.
    pub kind: String,
}

/// A subscriber entitlement.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Subscriber {
    /// Id.
    pub subscriber_id: String,
    /// The customer row.
    pub customer_id: String,
    /// Plan name.
    pub plan: String,
    /// `active`, `ended`.
    pub state: String,
    /// `owner grant`, `sandbox payment`, `sample`.
    pub source: String,
    /// Since.
    pub since: i64,
    /// Until, if it ends.
    pub until_at: Option<i64>,
}

/// A digest subscription.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DigestSub {
    /// Id.
    pub sub_id: String,
    /// Where it goes.
    pub email: String,
    /// Topics, JSON array text.
    pub topics: String,
    /// `pending`, `confirmed`, `stopped`.
    pub state: String,
    /// Created.
    pub created_at: i64,
    /// Confirmed.
    pub confirmed_at: Option<i64>,
}

const STORY_COLS: &str = "slug, section, kicker, title, standfirst, body, author_id, topic, entitlement, status, published_at, updated_at";

impl Records {
    /// Published stories, newest first, optionally in one section.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn stories(
        &self,
        site: &str,
        section: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<Story>> {
        Ok(sqlx::query_as::<_, Story>(sql!(
            "SELECT {STORY_COLS} FROM rec_stories WHERE site = ? AND status = 'published' AND (? IS NULL OR section = ?)
             ORDER BY published_at DESC LIMIT ?"
        ))
        .bind(site)
        .bind(section)
        .bind(section)
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    /// Stories published since a time, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn stories_since(&self, site: &str, since: i64) -> anyhow::Result<Vec<Story>> {
        Ok(sqlx::query_as::<_, Story>(sql!(
            "SELECT {STORY_COLS} FROM rec_stories WHERE site = ? AND status = 'published' AND published_at >= ? ORDER BY published_at DESC"
        ))
        .bind(site)
        .bind(since)
        .fetch_all(self.pool())
        .await?)
    }

    /// Every story, drafts included, for the owner.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn all_stories(&self, site: &str) -> anyhow::Result<Vec<Story>> {
        Ok(sqlx::query_as::<_, Story>(sql!(
            "SELECT {STORY_COLS} FROM rec_stories WHERE site = ? ORDER BY published_at DESC"
        ))
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// One story.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn story(&self, site: &str, slug: &str) -> anyhow::Result<Option<Story>> {
        Ok(sqlx::query_as::<_, Story>(sql!(
            "SELECT {STORY_COLS} FROM rec_stories WHERE site = ? AND slug = ?"
        ))
        .bind(site)
        .bind(slug)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Authors.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn authors(&self, site: &str) -> anyhow::Result<Vec<Author>> {
        Ok(sqlx::query_as::<_, Author>(
            "SELECT author_id, name, role, bio FROM rec_authors WHERE site = ? ORDER BY name",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// Corrections, newest first, optionally for one story.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn corrections(
        &self,
        site: &str,
        story: Option<&str>,
    ) -> anyhow::Result<Vec<Correction>> {
        Ok(sqlx::query_as::<_, Correction>(
            "SELECT story_slug, at, what, note FROM rec_corrections WHERE site = ? AND (? IS NULL OR story_slug = ?) ORDER BY at DESC",
        )
        .bind(site)
        .bind(story)
        .bind(story)
        .fetch_all(self.pool())
        .await?)
    }

    /// The owner publishes a correction on a story.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn add_correction(
        &self,
        site: &str,
        slug: &str,
        what: &str,
        note: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_corrections (site, story_slug, at, what, note) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(slug)
        .bind(now)
        .bind(what)
        .bind(note)
        .execute(self.pool())
        .await?;
        sqlx::query("UPDATE rec_stories SET updated_at = ? WHERE site = ? AND slug = ?")
            .bind(now)
            .bind(site)
            .bind(slug)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// The owner publishes or unpublishes a story.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn set_story_status(
        &self,
        site: &str,
        slug: &str,
        status: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE rec_stories SET status = ? WHERE site = ? AND slug = ?")
            .bind(status)
            .bind(site)
            .bind(slug)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// A running story's steps, oldest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn timeline(&self, site: &str, topic: &str) -> anyhow::Result<Vec<TimelineStep>> {
        Ok(sqlx::query_as::<_, TimelineStep>(
            "SELECT topic, at, line, story_slug, kind FROM rec_timeline WHERE site = ? AND topic = ? ORDER BY at",
        )
        .bind(site)
        .bind(topic)
        .fetch_all(self.pool())
        .await?)
    }

    /// Subscribers, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn subscribers(&self, site: &str) -> anyhow::Result<Vec<Subscriber>> {
        Ok(sqlx::query_as::<_, Subscriber>(
            "SELECT subscriber_id, customer_id, plan, state, source, since, until_at FROM rec_subscribers WHERE site = ? ORDER BY since DESC",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// Whether a customer holds an active subscription.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn is_subscriber(
        &self,
        site: &str,
        customer_id: &str,
        now: i64,
    ) -> anyhow::Result<bool> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM rec_subscribers WHERE site = ? AND customer_id = ? AND state = 'active' AND (until_at IS NULL OR until_at > ?)",
        )
        .bind(site)
        .bind(customer_id)
        .bind(now)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.is_some())
    }

    /// Grants or renews a subscription (an owner grant or a one-time sandbox payment).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn grant_subscription(
        &self,
        site: &str,
        customer_id: &str,
        plan: &str,
        source: &str,
        until_at: Option<i64>,
        now: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_subscribers (site, subscriber_id, customer_id, plan, state, source, since, until_at) VALUES (?, ?, ?, ?, 'active', ?, ?, ?)
             ON CONFLICT (site, customer_id) DO UPDATE SET state = 'active', plan = excluded.plan, source = excluded.source, until_at = excluded.until_at",
        )
        .bind(site)
        .bind(new_id("sub"))
        .bind(customer_id)
        .bind(plan)
        .bind(source)
        .bind(now)
        .bind(until_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The owner ends a subscription.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn end_subscription(
        &self,
        site: &str,
        subscriber_id: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE rec_subscribers SET state = 'ended', until_at = ? WHERE site = ? AND subscriber_id = ?")
            .bind(now)
            .bind(site)
            .bind(subscriber_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Starts a digest subscription, pending double opt-in. Returns the confirmation token.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn start_digest(
        &self,
        site: &str,
        email: &str,
        topics: &[String],
        now: i64,
    ) -> anyhow::Result<(String, String)> {
        let sub_id = new_id("dg");
        let token = hex::encode(rand::random::<[u8; 16]>());
        sqlx::query(
            "INSERT INTO rec_digest_subs (site, sub_id, email, topics, state, token_hash, created_at) VALUES (?, ?, ?, ?, 'pending', ?, ?)",
        )
        .bind(site)
        .bind(&sub_id)
        .bind(email.trim().to_lowercase())
        .bind(serde_json::to_string(topics)?)
        .bind(sha256_hex(&token))
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok((sub_id, token))
    }

    /// Confirms (`confirm`) or stops a digest subscription by its token. Stopping deletes the
    /// topics with it, as the contract promises.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn digest_by_token(
        &self,
        site: &str,
        token: &str,
        confirm: bool,
        now: i64,
    ) -> anyhow::Result<Option<DigestSub>> {
        let hash = sha256_hex(token);
        let sql = if confirm {
            "UPDATE rec_digest_subs SET state = 'confirmed', confirmed_at = ? WHERE site = ? AND token_hash = ? AND state = 'pending'"
        } else {
            "UPDATE rec_digest_subs SET state = 'stopped', stopped_at = ?, topics = '[]' WHERE site = ? AND (token_hash = ? OR stop_token = ?) AND state != 'stopped'"
        };
        sqlx::query(sql)
            .bind(now)
            .bind(site)
            .bind(&hash)
            .bind(token)
            .execute(self.pool())
            .await?;
        Ok(sqlx::query_as::<_, DigestSub>(
            "SELECT sub_id, email, topics, state, created_at, confirmed_at FROM rec_digest_subs WHERE site = ? AND (token_hash = ? OR (stop_token IS NOT NULL AND stop_token = ?))",
        )
        .bind(site)
        .bind(&hash)
        .bind(token)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Confirmed digest readers not yet sent the edition that started at `since`, each with
    /// a stable stop token (made on first use).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn digest_due(
        &self,
        site: &str,
        since: i64,
    ) -> anyhow::Result<Vec<(DigestSub, String)>> {
        let subs = sqlx::query_as::<_, DigestSub>(
            "SELECT sub_id, email, topics, state, created_at, confirmed_at FROM rec_digest_subs
             WHERE site = ? AND state = 'confirmed' AND (last_sent_at IS NULL OR last_sent_at < ?)",
        )
        .bind(site)
        .bind(since)
        .fetch_all(self.pool())
        .await?;
        let mut out = Vec::new();
        for sub in subs {
            let existing: Option<(Option<String>,)> = sqlx::query_as(
                "SELECT stop_token FROM rec_digest_subs WHERE site = ? AND sub_id = ?",
            )
            .bind(site)
            .bind(&sub.sub_id)
            .fetch_optional(self.pool())
            .await?;
            let token = match existing.and_then(|(t,)| t) {
                Some(t) => t,
                None => {
                    let t = hex::encode(rand::random::<[u8; 16]>());
                    sqlx::query(
                        "UPDATE rec_digest_subs SET stop_token = ? WHERE site = ? AND sub_id = ?",
                    )
                    .bind(&t)
                    .bind(site)
                    .bind(&sub.sub_id)
                    .execute(self.pool())
                    .await?;
                    t
                }
            };
            out.push((sub, token));
        }
        Ok(out)
    }

    /// Notes that a reader's weekly digest was recorded.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn mark_digest_sent(&self, site: &str, sub_id: &str, now: i64) -> anyhow::Result<()> {
        sqlx::query("UPDATE rec_digest_subs SET last_sent_at = ? WHERE site = ? AND sub_id = ?")
            .bind(now)
            .bind(site)
            .bind(sub_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Digest subscriptions, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn digest_subs(&self, site: &str) -> anyhow::Result<Vec<DigestSub>> {
        Ok(sqlx::query_as::<_, DigestSub>(
            "SELECT sub_id, email, topics, state, created_at, confirmed_at FROM rec_digest_subs WHERE site = ? ORDER BY created_at DESC",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }
}
