//! Console apps (contract C9): how other workstreams put their pages inside the shell.
//!
//! An app is a WebAgent's set of objects the owner runs by hand (01b §2.20): W2's Orders,
//! Catalogue, Bookings and Stories, W3's billing view, W9's notification settings, W11's media
//! library. An app implements [`ConsoleApp`]; `agentrix-node` passes the list to
//! [`crate::ShellState`]. The shell then:
//!
//! - puts the app in the dock (after the WebAgents and the system apps) on every site where
//!   [`ConsoleApp::available`] says yes, with its badge;
//! - signs the owner in and checks they own the site, then calls [`ConsoleApp::page`] for
//!   `GET /console/{site}/apps/{id}/{path}` and wraps the returned HTML in the shell (status bar,
//!   dock, lane, command bar), so the app looks like every other page;
//! - passes `POST /console/{site}/apps/{id}/{path}` to [`ConsoleApp::post`] after the same check.
//!
//! The app writes the field's HTML with the shell's classes (`field-head`, `cards`, `fcard`,
//! `verbs`, `btn`, `chip`, `procs`, `surface-table`; see `static/console.css`) and escapes what
//! it prints. Money, messages and public changes never run from a POST: the app raises a
//! decision card and the owner approves it in the lane (07 §3.4).

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;
use serde_json::Value;

/// How an app shows in the dock.
#[derive(Debug, Clone, Serialize)]
pub struct AppSpec {
    /// The path segment, unique across apps: `orders`, `catalogue`, `billing`.
    pub id: String,
    /// The label under the icon: "Orders".
    pub name: String,
    /// The tooltip: "Orders: what customers bought, and what needs you".
    pub title: String,
    /// A system icon name (`orders`, `catalogue`, `bookings`, `stories`, `billing`, `bell`,
    /// `media`, `grid`); unknown names draw the grid icon.
    pub icon: String,
    /// The WebAgent that looks after these objects, if one does ("Cashier").
    pub webagent: Option<String>,
    /// Sort order in the dock, lowest first.
    pub order: u32,
}

/// Who is asking, for which site.
#[derive(Debug, Clone, Serialize)]
pub struct AppContext {
    /// The site's id.
    pub site_id: String,
    /// The business name.
    pub site_name: String,
    /// The site's public host.
    pub host: String,
    /// True for a `-sample` site: say "Sample" where a person could mistake it for a real business.
    pub sample: bool,
    /// The owner's account id.
    pub account: String,
    /// The owner's first name.
    pub owner_name: String,
    /// The owner's UTC offset in minutes.
    pub utc_offset_minutes: i32,
    /// This app's root inside the shell: `/console/{site}/apps/{id}`. Link with it.
    pub base: String,
    /// The shell's root for the site: `/console/{site}`.
    pub console: String,
}

/// What a GET returns.
#[derive(Debug, Clone)]
pub enum AppPage {
    /// HTML for the field, and the page title.
    Field {
        /// The `<title>`, without " · Agentrix".
        title: String,
        /// The field's HTML. The app escapes everything it prints.
        html: String,
    },
    /// Go somewhere else.
    Redirect(String),
    /// No such page in this app.
    NotFound,
}

/// What a POST returns.
#[derive(Debug, Clone)]
pub enum AppReply {
    /// 303 to this path (a form post).
    Redirect(String),
    /// A JSON answer with a status.
    Json(StatusCode, Value),
    /// Plain words for the owner, with a status.
    Error(StatusCode, String),
    /// No such action.
    NotFound,
}

/// A console app (C9). Implement it in your crate; node registers it.
#[async_trait]
pub trait ConsoleApp: Send + Sync + 'static {
    /// How the app shows in the dock.
    fn spec(&self) -> AppSpec;

    /// Whether the app belongs on this site (an Orders app only where the site sells).
    async fn available(&self, _ctx: &AppContext) -> bool {
        true
    }

    /// The dock badge: how many things in this app need the owner. 0 hides it.
    async fn badge(&self, _ctx: &AppContext) -> anyhow::Result<u32> {
        Ok(0)
    }

    /// `GET {base}/{path}?{query}`. `path` has no leading slash; the app's home is `""`.
    async fn page(&self, ctx: &AppContext, path: &str, query: &str) -> anyhow::Result<AppPage>;

    /// `POST {base}/{path}` with the raw body (form or JSON, see `headers`).
    async fn post(
        &self,
        _ctx: &AppContext,
        _path: &str,
        _headers: &HeaderMap,
        _body: Bytes,
    ) -> anyhow::Result<AppReply> {
        Ok(AppReply::NotFound)
    }
}

/// Something waiting for the owner that another crate owns and decides on its own page, such as
/// W3's support-access requests. It shows in the lane's "Needs you" beside decision cards and
/// links to `href`, where the owner reads the exact terms and approves through the frozen
/// approve contract. The shell never approves it.
#[derive(Debug, Clone, Serialize)]
pub struct LaneItem {
    /// Unique within its source.
    pub id: String,
    /// Plain words: what the owner is asked, e.g. "Zavora support asks for read access for 2 hours".
    pub title: String,
    /// One line: who, what, how long.
    pub summary: String,
    /// Who asks, e.g. "Zavora support".
    pub from: String,
    /// `ok`, `wait` or `risk`, for the chip.
    pub tone: String,
    /// The chip's words, e.g. "Read only".
    pub chip: String,
    /// "Ends Sun 8 Nov 18:00" or "Waits until …", already in the owner's words.
    pub when_words: String,
    /// The page where the owner decides.
    pub href: String,
}

/// A source of [`LaneItem`]s (contract C9). Implement it in your crate; node passes it to
/// [`crate::mount_with`].
#[async_trait]
pub trait LaneSource: Send + Sync + 'static {
    /// What waits for this owner on this site now. Errors are logged and the lane shows the rest.
    async fn needs_you(&self, ctx: &AppContext) -> anyhow::Result<Vec<LaneItem>>;
}
