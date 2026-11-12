//! Questions visitors pass to the owner ("teach your site").

use serde::Serialize;

use crate::{Records, new_id};

/// A passed question.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Question {
    /// Id.
    pub question_id: String,
    /// The desk it was asked at.
    pub desk: String,
    /// The question, as typed.
    pub question: String,
    /// How to reply, if the visitor left a way.
    pub contact: Option<String>,
    /// The page it was asked on.
    pub page: Option<String>,
    /// `open`, `answered`.
    pub state: String,
    /// The owner's answer.
    pub answer: Option<String>,
    /// When.
    pub created_at: i64,
    /// When the owner answered it (`None` for answers saved before this was kept).
    pub answered_at: Option<i64>,
}

impl Question {
    /// When the answer was taught: when it was saved, else when it was asked.
    pub fn taught_at(&self) -> i64 {
        self.answered_at.unwrap_or(self.created_at)
    }
}

impl Records {
    /// Stores a question for the owner.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn pass_question(
        &self,
        site: &str,
        desk: &str,
        question: &str,
        contact: Option<&str>,
        page: Option<&str>,
        now: i64,
    ) -> anyhow::Result<String> {
        let id = new_id("q");
        sqlx::query(
            "INSERT INTO rec_questions (site, question_id, desk, question, contact, page, state, answer, created_at) VALUES (?, ?, ?, ?, ?, ?, 'open', NULL, ?)",
        )
        .bind(site)
        .bind(&id)
        .bind(desk)
        .bind(question.trim())
        .bind(contact.map(str::trim).filter(|c| !c.is_empty()))
        .bind(page)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    /// Questions, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn questions(&self, site: &str) -> anyhow::Result<Vec<Question>> {
        Ok(sqlx::query_as::<_, Question>(
            "SELECT question_id, desk, question, contact, page, state, answer, created_at, answered_at FROM rec_questions WHERE site = ? ORDER BY created_at DESC",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// The owner answers a question. The desks answer the same question with it from then on.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn answer_question(
        &self,
        site: &str,
        id: &str,
        answer: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE rec_questions SET state = 'answered', answer = ?, answered_at = ? WHERE site = ? AND question_id = ?")
            .bind(answer.trim())
            .bind(now)
            .bind(site)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// The owner teaches an answer to a question nobody passed on (one a visitor marked wrong):
    /// it is kept as an answered question, so the desks answer from it like any other.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn teach(
        &self,
        site: &str,
        desk: &str,
        question: &str,
        answer: &str,
        now: i64,
    ) -> anyhow::Result<String> {
        let id = new_id("q");
        sqlx::query(
            "INSERT INTO rec_questions (site, question_id, desk, question, contact, page, state, answer, created_at, answered_at) VALUES (?, ?, ?, ?, NULL, NULL, 'answered', ?, ?, ?)",
        )
        .bind(site)
        .bind(&id)
        .bind(desk)
        .bind(question.trim())
        .bind(answer.trim())
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    /// The answers the owner taught, newest first: answered questions with an answer.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn taught(&self, site: &str) -> anyhow::Result<Vec<Question>> {
        Ok(sqlx::query_as::<_, Question>(
            "SELECT question_id, desk, question, contact, page, state, answer, created_at, answered_at FROM rec_questions WHERE site = ? AND state = 'answered' AND answer IS NOT NULL AND answer != '' ORDER BY COALESCE(answered_at, created_at) DESC",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }
}
