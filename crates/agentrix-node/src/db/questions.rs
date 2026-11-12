//! Questions visitors passed to the owner from Ask (R8.3): the `passed_questions` table.
//!
//! The site's `POST /ask/pass` writes through [`QuestionStore`] (as the site's `QuestionSink`);
//! the console's Today reads [`QuestionStore::recent`] and [`QuestionStore::count`]. The text is
//! data only: nothing here sends it to a model.

use agentrix_types::SiteId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::time;
use crate::site::{PassedQuestion, QuestionSink};

/// One stored question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredQuestion {
    /// The question, as the visitor wrote it.
    pub question: String,
    /// How to reach the visitor, if they left one.
    pub contact: Option<String>,
    /// When it was passed (RFC 3339).
    pub passed_at: String,
}

/// The `passed_questions` table.
#[derive(Debug, Clone)]
pub struct QuestionStore {
    pool: SqlitePool,
}

impl QuestionStore {
    /// A store over `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Stores one passed question.
    pub async fn insert(&self, q: &PassedQuestion) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO passed_questions \
             (question_id, site_id, release_seq, question, contact, passed_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("pq_{}", uuid::Uuid::now_v7().simple()))
        .bind(&q.site)
        .bind(i64::try_from(q.release_seq).ok())
        .bind(&q.question)
        .bind(&q.contact)
        .bind(time(q.at))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A site's newest questions, newest first.
    pub async fn recent(&self, site: &SiteId, limit: u32) -> anyhow::Result<Vec<StoredQuestion>> {
        let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT question, contact, passed_at FROM passed_questions WHERE site_id = ? \
             ORDER BY passed_at DESC, rowid DESC LIMIT ?",
        )
        .bind(site.as_str())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(question, contact, passed_at)| StoredQuestion {
                question,
                contact,
                passed_at,
            })
            .collect())
    }

    /// How many questions a site's visitors have passed, in all or since a time.
    pub async fn count(&self, site: &SiteId, since: Option<DateTime<Utc>>) -> anyhow::Result<u32> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM passed_questions WHERE site_id = ? AND (? IS NULL OR passed_at >= ?)",
        )
        .bind(site.as_str())
        .bind(since.map(time))
        .bind(since.map(time))
        .fetch_one(&self.pool)
        .await?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

#[async_trait]
impl QuestionSink for QuestionStore {
    async fn pass(&self, question: PassedQuestion) -> anyhow::Result<()> {
        self.insert(&question).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_in_memory, sites};

    #[tokio::test]
    async fn stores_and_lists_newest_first() {
        let pool = open_in_memory().await.unwrap();
        let site = sites::create_or_get(&pool, "kilele.example", "https://kilele.example", false)
            .await
            .unwrap();
        let store = QuestionStore::new(pool);
        for (i, (q, c)) in [
            ("Do you deliver to Karen?", None),
            ("Gluten-free cakes?", Some("0700 000 000")),
        ]
        .into_iter()
        .enumerate()
        {
            store
                .pass(PassedQuestion {
                    site: site.as_str().to_owned(),
                    release_seq: 3,
                    question: q.into(),
                    contact: c.map(str::to_owned),
                    at: Utc::now() + chrono::Duration::seconds(i as i64),
                })
                .await
                .unwrap();
        }
        let recent = store.recent(&site, 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].question, "Gluten-free cakes?");
        assert_eq!(recent[0].contact.as_deref(), Some("0700 000 000"));
        assert_eq!(store.count(&site, None).await.unwrap(), 2);
        assert_eq!(
            store
                .count(&site, Some(Utc::now() + chrono::Duration::hours(1)))
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn the_table_refuses_text_past_its_limits() {
        let pool = open_in_memory().await.unwrap();
        let site = sites::create_or_get(&pool, "kilele.example", "https://kilele.example", false)
            .await
            .unwrap();
        let store = QuestionStore::new(pool);
        let long = PassedQuestion {
            site: site.as_str().to_owned(),
            release_seq: 1,
            question: "x".repeat(501),
            contact: None,
            at: Utc::now(),
        };
        assert!(store.insert(&long).await.is_err());
    }
}
