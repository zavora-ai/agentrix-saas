//! Platform holds (08 §2.4, §3.8): the operator's switch on a site, separate from the owner's
//! freeze. Neither side can lift the other's.
//!
//! For the edge: [`active`] says whether a site is held right now and how. `ReadOnly` serves the
//! last signed release with answers and forms paused ("Answers and forms are paused right now.
//! You can still read the site."); `Notice` serves the neutral "This site isn't available right
//! now" page. Visitors never see the reason (08 §2.6).

use serde::Serialize;
use sqlx::SqlitePool;

/// How a held site is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// The last signed release, read only: answers and forms paused.
    ReadOnly,
    /// The neutral notice page (phishing, malware).
    Notice,
}

impl Mode {
    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::ReadOnly => "read-only",
            Mode::Notice => "notice",
        }
    }

    /// The mode for a stored name.
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "read-only" => Some(Mode::ReadOnly),
            "notice" => Some(Mode::Notice),
            _ => None,
        }
    }
}

/// A hold, as stored.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Hold {
    /// `hold_…`.
    pub hold_id: String,
    /// `site`, `partner`, `cell` or `fleet`.
    pub scope: String,
    /// The site id (or partner, cell; `fleet` for the fleet).
    pub target: String,
    /// `read-only` or `notice`.
    pub mode: String,
    /// Shown to the owner, never to visitors.
    pub reason: String,
    /// Who placed it (account id).
    pub placed_by: String,
    /// `passkey` or `totp`.
    pub placed_method: String,
    /// When.
    pub placed_at: String,
    /// Who lifted it.
    pub lifted_by: Option<String>,
    /// When.
    pub lifted_at: Option<String>,
    /// Why it was lifted.
    pub lift_reason: Option<String>,
    /// The owner's receipt, if one was written.
    pub receipt: Option<String>,
}

impl Hold {
    /// How the edge should serve the site.
    pub fn serve_mode(&self) -> Mode {
        Mode::parse(&self.mode).unwrap_or(Mode::ReadOnly)
    }
}

macro_rules! select {
    ($rest:literal) => {
        concat!(
            "SELECT hold_id, scope, target, mode, reason, placed_by, placed_method, placed_at, lifted_by, lifted_at, lift_reason, receipt FROM platform_holds ",
            $rest
        )
    };
}

/// The hold on `site_id` right now, if any: a site hold, or a fleet hold. The edge calls this.
pub async fn active(pool: &SqlitePool, site_id: &str) -> Result<Option<Hold>, sqlx::Error> {
    sqlx::query_as(select!(
        "WHERE lifted_at IS NULL AND ((scope = 'site' AND target = ?) OR scope = 'fleet') ORDER BY placed_at DESC LIMIT 1"
    ))
    .bind(site_id)
    .fetch_optional(pool)
    .await
}

/// Every hold in force, newest first.
pub async fn all_active(pool: &SqlitePool) -> Result<Vec<Hold>, sqlx::Error> {
    sqlx::query_as(select!("WHERE lifted_at IS NULL ORDER BY placed_at DESC"))
        .fetch_all(pool)
        .await
}

/// Holds lifted, newest lift first, at most `limit`.
pub async fn recently_lifted(pool: &SqlitePool, limit: i64) -> Result<Vec<Hold>, sqlx::Error> {
    sqlx::query_as(select!(
        "WHERE lifted_at IS NOT NULL ORDER BY lifted_at DESC LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// A site's holds, newest first, lifted ones included.
pub async fn for_site(pool: &SqlitePool, site_id: &str) -> Result<Vec<Hold>, sqlx::Error> {
    sqlx::query_as(select!(
        "WHERE scope = 'site' AND target = ? ORDER BY placed_at DESC LIMIT 20"
    ))
    .bind(site_id)
    .fetch_all(pool)
    .await
}

/// One hold.
pub async fn get(pool: &SqlitePool, hold_id: &str) -> Result<Option<Hold>, sqlx::Error> {
    sqlx::query_as(select!("WHERE hold_id = ?"))
        .bind(hold_id)
        .fetch_optional(pool)
        .await
}
