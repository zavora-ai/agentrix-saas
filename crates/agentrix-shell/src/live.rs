//! Live (concept/console/live.html): the owner sits in on a Front of house conversation, takes it
//! over as a person and hands it back to the desk.
//!
//! The seam for a site's Ask (W2's samples, the first release's moved sites). Ask keeps no conversation
//! today, so it calls [`Live`]:
//!
//! 1. before answering, [`Live::owner_has_it`]: when the owner has taken the conversation over,
//!    record the visitor's words with [`Live::visitor_said`] and let the desk stay quiet;
//! 2. otherwise record the question ([`Live::visitor_said`]) and the desk's answer
//!    ([`Live::desk_said`]) as it is sent;
//! 3. merge [`visitor_routes`] into the site's router, so the page's Ask widget can poll
//!    `GET /live/{thread}?after={seq}` for the owner's replies, each labelled "Wanjiru · a person".
//!
//! The thread id comes from the visitor's page (`lv_` and 32 hex characters, made in the
//! browser); knowing it is what lets a visitor read their own conversation. Until a site's Ask
//! records turns, Live shows no conversations rather than invented ones, and the console keeps
//! it out of the dock and the command bar and says on its page that it isn't connected
//! ([`Live::connected`]).
//!
//! No Ask records turns yet. The samples' privacy pages tell visitors "The site doesn't save
//! what you ask them: the conversation stays in this tab and is gone when you close it", so
//! connecting a site's Ask needs that page changed first (with how long conversations are kept),
//! and a retention sweep for these tables.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;

/// An owner who says nothing for this long hands the conversation back to the desk.
pub const HAND_BACK_AFTER_MINUTES: i64 = 10;
/// A conversation counts as live while its last turn is this recent.
pub const LIVE_FOR_MINUTES: i64 = 10;

fn stored(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parsed(t: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(t)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_default()
}

/// Whether `id` looks like a thread id a page made: `lv_` and 32 hex characters.
pub fn valid_thread(id: &str) -> bool {
    id.strip_prefix("lv_")
        .is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// One turn of a conversation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Turn {
    /// Its place in the conversation, from 1.
    pub seq: i64,
    /// When.
    pub at: DateTime<Utc>,
    /// `visitor`, `desk`, `owner` or `note`.
    pub who: String,
    /// The visitor's label, the desk's name or the owner's first name.
    pub name: String,
    /// What was said.
    pub text: String,
    /// Pages the desk cited.
    pub cites: Vec<String>,
}

impl Turn {
    /// What a visitor sees above the words: "Cakes · AI", "Wanjiru · a person".
    pub fn label(&self) -> String {
        match self.who.as_str() {
            "desk" => format!("{} · AI", self.name),
            "owner" => format!("{} · a person", self.name),
            _ => self.name.clone(),
        }
    }
}

/// A conversation.
#[derive(Debug, Clone, Serialize)]
pub struct Thread {
    /// `lv_…`.
    pub id: String,
    /// The site.
    pub site_id: String,
    /// The page the visitor is on.
    pub page: String,
    /// How the visitor is named: "Visitor", or a first name they gave.
    pub visitor: String,
    /// The desk answering.
    pub desk: String,
    /// True while the owner has it.
    pub owner: bool,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// The last turn.
    pub last_at: DateTime<Utc>,
}

/// Why a write was refused.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    /// Not a thread id a page makes.
    #[error("not a conversation id")]
    BadId,
    /// Empty or over 2,000 characters.
    #[error("say something, up to 2,000 characters")]
    BadText,
    /// No such conversation for the site.
    #[error("no such conversation")]
    NotFound,
    /// The owner can only reply after taking the conversation over.
    #[error("take the conversation over first")]
    NotTaken,
    /// The database.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

const THREAD_COLS: &str =
    "id, site_id, page, visitor, desk, mode, owner_last_at, started_at, last_at";

fn thread_row(r: &sqlx::sqlite::SqliteRow, now: DateTime<Utc>) -> Thread {
    let owner_last: Option<String> = r.get(6);
    Thread {
        id: r.get(0),
        site_id: r.get(1),
        page: r.get(2),
        visitor: r.get(3),
        desk: r.get(4),
        owner: r.get::<String, _>(5) == "owner"
            && owner_last
                .is_some_and(|t| now - parsed(&t) < Duration::minutes(HAND_BACK_AFTER_MINUTES)),
        started_at: parsed(r.get::<&str, _>(7)),
        last_at: parsed(r.get::<&str, _>(8)),
    }
}

fn clean(text: &str) -> Result<String, LiveError> {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 2000 {
        return Err(LiveError::BadText);
    }
    Ok(t.to_owned())
}

/// Live conversations in `control.db`.
#[derive(Debug, Clone)]
pub struct Live {
    pool: SqlitePool,
}

impl Live {
    /// Over `pool`, whose shell migrations have run.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn add(
        &self,
        thread: &str,
        who: &str,
        name: &str,
        text: &str,
        cites: &[String],
        now: DateTime<Utc>,
    ) -> Result<Turn, LiveError> {
        let mut tx = self.pool.begin().await?;
        let (seq,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM shell_live_turns WHERE thread_id = ?",
        )
        .bind(thread)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO shell_live_turns (thread_id, seq, at, who, name, text, cites) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(thread)
        .bind(seq)
        .bind(stored(now))
        .bind(who)
        .bind(name)
        .bind(text)
        .bind(serde_json::to_string(cites).unwrap_or_else(|_| "[]".into()))
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE shell_live_threads SET last_at = ? WHERE id = ?")
            .bind(stored(now))
            .bind(thread)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Turn {
            seq,
            at: now,
            who: who.to_owned(),
            name: name.to_owned(),
            text: text.to_owned(),
            cites: cites.to_vec(),
        })
    }

    /// Records what the visitor said, opening the conversation on its first words.
    pub async fn visitor_said(
        &self,
        site: &str,
        thread: &str,
        page: &str,
        desk: &str,
        text: &str,
    ) -> Result<Turn, LiveError> {
        if !valid_thread(thread) {
            return Err(LiveError::BadId);
        }
        let text = clean(text)?;
        let now = Utc::now();
        let mine = sqlx::query(
            "INSERT INTO shell_live_threads (id, site_id, page, visitor, desk, started_at, last_at)
             VALUES (?, ?, ?, 'Visitor', ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET page = excluded.page WHERE shell_live_threads.site_id = excluded.site_id",
        )
        .bind(thread)
        .bind(site)
        .bind(page)
        .bind(desk)
        .bind(stored(now))
        .bind(stored(now))
        .execute(&self.pool)
        .await?;
        if mine.rows_affected() == 0 {
            return Err(LiveError::NotFound);
        }
        self.add(thread, "visitor", "Visitor", &text, &[], now)
            .await
    }

    /// Records the desk's answer as it is sent.
    pub async fn desk_said(
        &self,
        site: &str,
        thread: &str,
        text: &str,
        cites: &[String],
    ) -> Result<Turn, LiveError> {
        let t = self
            .thread(site, thread)
            .await?
            .ok_or(LiveError::NotFound)?;
        let text = clean(text)?;
        self.add(&t.id, "desk", &t.desk, &text, cites, Utc::now())
            .await
    }

    /// True while the owner has the conversation, so the desk must stay quiet. Ten minutes after
    /// the owner last spoke, the desk has it back and the conversation says so.
    pub async fn owner_has_it(&self, site: &str, thread: &str) -> Result<bool, LiveError> {
        let Some(t) = self.thread(site, thread).await? else {
            return Ok(false);
        };
        if !t.owner {
            let lapsed = sqlx::query(
                "UPDATE shell_live_threads SET mode = 'desk' WHERE id = ? AND mode = 'owner'",
            )
            .bind(&t.id)
            .execute(&self.pool)
            .await?;
            if lapsed.rows_affected() == 1 {
                let words = format!(
                    "{} took the conversation back after {HAND_BACK_AFTER_MINUTES} minutes without a reply",
                    t.desk
                );
                self.add(&t.id, "note", &t.desk, &words, &[], Utc::now())
                    .await?;
            }
        }
        Ok(t.owner)
    }

    /// The owner takes the conversation over; the visitor sees that a person joined.
    pub async fn take_over(&self, site: &str, thread: &str, owner: &str) -> Result<(), LiveError> {
        let t = self
            .thread(site, thread)
            .await?
            .ok_or(LiveError::NotFound)?;
        if t.owner {
            return Ok(());
        }
        let now = Utc::now();
        sqlx::query(
            "UPDATE shell_live_threads SET mode = 'owner', owner_name = ?, owner_last_at = ? WHERE id = ?",
        )
        .bind(owner)
        .bind(stored(now))
        .bind(&t.id)
        .execute(&self.pool)
        .await?;
        let words = format!(
            "{owner} joined the conversation. {} stays quiet while {owner} replies",
            t.desk
        );
        self.add(&t.id, "note", owner, &words, &[], now).await?;
        Ok(())
    }

    /// The owner's reply, as a person.
    pub async fn owner_said(
        &self,
        site: &str,
        thread: &str,
        owner: &str,
        text: &str,
    ) -> Result<Turn, LiveError> {
        let text = clean(text)?;
        if !self.owner_has_it(site, thread).await? {
            return Err(LiveError::NotTaken);
        }
        let now = Utc::now();
        sqlx::query("UPDATE shell_live_threads SET owner_last_at = ? WHERE id = ?")
            .bind(stored(now))
            .bind(thread)
            .execute(&self.pool)
            .await?;
        self.add(thread, "owner", owner, &text, &[], now).await
    }

    /// Hands the conversation back to the desk.
    pub async fn hand_back(&self, site: &str, thread: &str) -> Result<(), LiveError> {
        let t = self
            .thread(site, thread)
            .await?
            .ok_or(LiveError::NotFound)?;
        let back = sqlx::query(
            "UPDATE shell_live_threads SET mode = 'desk' WHERE id = ? AND mode = 'owner'",
        )
        .bind(&t.id)
        .execute(&self.pool)
        .await?;
        if back.rows_affected() == 1 {
            let words = format!("{} is answering again", t.desk);
            self.add(&t.id, "note", &t.desk, &words, &[], Utc::now())
                .await?;
        }
        Ok(())
    }

    /// Whether the site's Ask has recorded any conversation here, so Live has something to show.
    pub async fn connected(&self, site: &str) -> Result<bool, LiveError> {
        let (any,): (bool,) =
            sqlx::query_as("SELECT EXISTS (SELECT 1 FROM shell_live_threads WHERE site_id = ?)")
                .bind(site)
                .fetch_one(&self.pool)
                .await?;
        Ok(any)
    }

    /// One conversation of the site.
    pub async fn thread(&self, site: &str, thread: &str) -> Result<Option<Thread>, LiveError> {
        let r = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {THREAD_COLS} FROM shell_live_threads WHERE site_id = ? AND id = ?"
        )))
        .bind(site)
        .bind(thread)
        .fetch_optional(&self.pool)
        .await?;
        Ok(r.as_ref().map(|r| thread_row(r, Utc::now())))
    }

    /// The site's conversations with a turn since `since`, latest first.
    pub async fn threads(
        &self,
        site: &str,
        since: DateTime<Utc>,
    ) -> Result<Vec<Thread>, LiveError> {
        let now = Utc::now();
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {THREAD_COLS} FROM shell_live_threads WHERE site_id = ? AND last_at >= ? ORDER BY last_at DESC LIMIT 50"
        )))
        .bind(site)
        .bind(stored(since))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| thread_row(r, now)).collect())
    }

    /// The turns after `after`, in order.
    pub async fn turns(
        &self,
        site: &str,
        thread: &str,
        after: i64,
    ) -> Result<Vec<Turn>, LiveError> {
        let t = self
            .thread(site, thread)
            .await?
            .ok_or(LiveError::NotFound)?;
        let rows = sqlx::query(
            "SELECT seq, at, who, name, text, cites FROM shell_live_turns WHERE thread_id = ? AND seq > ? ORDER BY seq LIMIT 500",
        )
        .bind(&t.id)
        .bind(after)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| Turn {
                seq: r.get(0),
                at: parsed(r.get::<&str, _>(1)),
                who: r.get(2),
                name: r.get(3),
                text: r.get(4),
                cites: serde_json::from_str(r.get::<&str, _>(5)).unwrap_or_default(),
            })
            .collect())
    }
}

/// Finds the site a request's host serves (the site router already knows).
pub type SiteOfHost =
    Arc<dyn Fn(String) -> BoxFuture<'static, Option<String>> + Send + Sync + 'static>;

#[derive(Clone)]
struct VisitorState {
    live: Live,
    site_of: SiteOfHost,
}

#[derive(Deserialize)]
struct After {
    #[serde(default)]
    after: i64,
}

/// `GET /live/{thread}?after={seq}` on a site's own host: the conversation's turns after `seq`,
/// for the page's Ask widget, and whether a person has it. Merge into the site's router.
pub fn visitor_routes(live: Live, site_of: SiteOfHost) -> Router {
    Router::new()
        .route("/live/{thread}", get(visitor_turns))
        .with_state(VisitorState { live, site_of })
}

async fn visitor_turns(
    State(s): State<VisitorState>,
    Path(thread): Path<String>,
    Query(q): Query<After>,
    headers: HeaderMap,
) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or(h).to_ascii_lowercase());
    let site = match host {
        Some(h) => (s.site_of)(h).await,
        None => None,
    };
    let (Some(site), true) = (site, valid_thread(&thread)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let owner = match s.live.owner_has_it(&site, &thread).await {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(error = %e, "live: reading the conversation failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    match s.live.turns(&site, &thread, q.after).await {
        Ok(turns) => (
            [(header::CACHE_CONTROL, "no-store")],
            Json(json!({
                "person": owner,
                "turns": turns.iter().map(|t| json!({
                    "seq": t.seq, "who": t.who, "label": t.label(), "text": t.text,
                })).collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Err(LiveError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "live: reading the turns failed");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    const T: &str = "lv_0123456789abcdef0123456789abcdef";

    async fn live() -> Live {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::store::migrate(&pool).await.unwrap();
        Live::new(pool)
    }

    #[tokio::test]
    async fn sit_in_take_over_hand_back() {
        let l = live().await;
        assert!(matches!(
            l.visitor_said("s1", "nope", "/cakes", "Cakes", "Hi").await,
            Err(LiveError::BadId)
        ));
        l.visitor_said("s1", T, "/cakes", "Cakes", "Do you do wedding cakes?")
            .await
            .unwrap();
        l.desk_said("s1", T, "Yes, three tiers.", &["/cakes#wedding".into()])
            .await
            .unwrap();
        assert!(
            matches!(
                l.visitor_said("s2", T, "/", "Cakes", "Hi").await,
                Err(LiveError::NotFound)
            ),
            "another site can't write into this conversation"
        );
        assert!(l.thread("s2", T).await.unwrap().is_none());
        assert!(matches!(
            l.owner_said("s1", T, "Wanjiru", "Hello").await,
            Err(LiveError::NotTaken)
        ));
        assert!(!l.owner_has_it("s1", T).await.unwrap());
        l.take_over("s1", T, "Wanjiru").await.unwrap();
        assert!(l.owner_has_it("s1", T).await.unwrap());
        let said = l
            .owner_said("s1", T, "Wanjiru", "Yes, gluten-free on request.")
            .await
            .unwrap();
        assert_eq!(said.label(), "Wanjiru · a person");
        l.hand_back("s1", T).await.unwrap();
        assert!(!l.owner_has_it("s1", T).await.unwrap());
        let turns = l.turns("s1", T, 0).await.unwrap();
        let who: Vec<&str> = turns.iter().map(|t| t.who.as_str()).collect();
        assert_eq!(who, ["visitor", "desk", "note", "owner", "note"]);
        assert_eq!(turns[1].label(), "Cakes · AI");
        assert_eq!(turns[1].cites, vec!["/cakes#wedding".to_owned()]);
        assert_eq!(l.turns("s1", T, 3).await.unwrap().len(), 2);
        assert_eq!(
            l.threads("s1", Utc::now() - Duration::minutes(5))
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(l.connected("s1").await.unwrap());
        assert!(!l.connected("s2").await.unwrap(), "nothing recorded there");
    }

    #[tokio::test]
    async fn a_quiet_owner_hands_back_after_ten_minutes() {
        let l = live().await;
        l.visitor_said("s1", T, "/", "Bread", "Fresh today?")
            .await
            .unwrap();
        l.take_over("s1", T, "Wanjiru").await.unwrap();
        sqlx::query("UPDATE shell_live_threads SET owner_last_at = ?")
            .bind(stored(Utc::now() - Duration::minutes(11)))
            .execute(&l.pool)
            .await
            .unwrap();
        assert!(!l.owner_has_it("s1", T).await.unwrap());
        let last = l.turns("s1", T, 0).await.unwrap().pop().unwrap();
        assert_eq!(last.who, "note");
        assert!(
            last.text.contains("took the conversation back"),
            "{}",
            last.text
        );
        assert!(!l.owner_has_it("s1", T).await.unwrap());
        assert_eq!(l.turns("s1", T, 0).await.unwrap().len(), 3, "said once");
    }
}
