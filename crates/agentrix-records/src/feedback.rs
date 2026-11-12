//! "Was this right?" on the desks' answers: one row per answer a visitor marked, for the owner.

use serde::Serialize;

use crate::Records;

/// A visitor's mark on one answer.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Feedback {
    /// The answer's id, as the page got it with the answer.
    pub answer_id: String,
    /// The desk that answered, by name ("Bread", "Front desk").
    pub desk: String,
    /// The visitor's question.
    pub question: String,
    /// What the desk said, when the site still had it.
    pub answer: Option<String>,
    /// What wrote the answer: `ai`, `instant`, `rules` or `owner`.
    pub basis: Option<String>,
    /// `yes` or `no`.
    pub verdict: String,
    /// The visitor's note on a No.
    pub note: Option<String>,
    /// First marked.
    pub created_at: i64,
    /// Last changed (the other button, or a note).
    pub updated_at: i64,
}

impl Feedback {
    /// Whether the visitor said the answer was right.
    pub fn right(&self) -> bool {
        self.verdict == "yes"
    }
}

/// A mark to record.
#[derive(Debug, Clone, Default)]
pub struct Mark<'a> {
    /// The answer's id.
    pub answer_id: &'a str,
    /// The desk, by name.
    pub desk: &'a str,
    /// The question.
    pub question: &'a str,
    /// What the desk said, if known.
    pub answer: Option<&'a str>,
    /// What wrote it.
    pub basis: Option<&'a str>,
    /// Right or not.
    pub right: bool,
    /// A note on a No (kept only with a No).
    pub note: Option<&'a str>,
}

impl Records {
    /// Records a visitor's mark on an answer. Marking the same answer again (the other button,
    /// or a note) changes the one row; a note stays until the visitor says Yes.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn record_feedback(&self, site: &str, m: &Mark<'_>, now: i64) -> anyhow::Result<()> {
        let note = m.note.map(str::trim).filter(|n| !n.is_empty() && !m.right);
        sqlx::query(
            "INSERT INTO rec_feedback (site, answer_id, desk, question, answer, basis, verdict, note, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (site, answer_id) DO UPDATE SET
               verdict = excluded.verdict,
               note = CASE WHEN excluded.verdict = 'yes' THEN NULL ELSE COALESCE(excluded.note, rec_feedback.note) END,
               answer = COALESCE(rec_feedback.answer, excluded.answer),
               basis = COALESCE(rec_feedback.basis, excluded.basis),
               updated_at = excluded.updated_at",
        )
        .bind(site)
        .bind(m.answer_id)
        .bind(m.desk)
        .bind(m.question.trim())
        .bind(m.answer)
        .bind(m.basis)
        .bind(if m.right { "yes" } else { "no" })
        .bind(note)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Marks changed since `since`, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn feedback(&self, site: &str, since: i64) -> anyhow::Result<Vec<Feedback>> {
        Ok(sqlx::query_as::<_, Feedback>(
            "SELECT answer_id, desk, question, answer, basis, verdict, note, created_at, updated_at FROM rec_feedback WHERE site = ? AND updated_at >= ? ORDER BY updated_at DESC",
        )
        .bind(site)
        .bind(since)
        .fetch_all(self.pool())
        .await?)
    }

    /// One mark by its answer's id.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn feedback_one(
        &self,
        site: &str,
        answer_id: &str,
    ) -> anyhow::Result<Option<Feedback>> {
        Ok(sqlx::query_as::<_, Feedback>(
            "SELECT answer_id, desk, question, answer, basis, verdict, note, created_at, updated_at FROM rec_feedback WHERE site = ? AND answer_id = ?",
        )
        .bind(site)
        .bind(answer_id)
        .fetch_optional(self.pool())
        .await?)
    }
}
