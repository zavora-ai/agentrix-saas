//! What a form says when it comes back. Every form on the ops host posts, then redirects to its
//! own page (POST, redirect, GET), and that page shows one line at the top in plain words: what
//! was done, or what was wrong and what to do now. Nothing comes back as raw JSON or text.
//!
//! The words stay on the server, keyed by a random id that rides the redirect as `?n=…`, so a
//! link from anywhere else can't put words on an operator's screen. Each one is shown once and
//! kept ten minutes at most.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use rand::RngCore as _;
use serde::Serialize;

use crate::OperatorState;

/// Whether the line reports something done or something that stopped the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Tone {
    /// It happened.
    Done,
    /// It didn't happen; the words say why and what to do.
    Problem,
}

/// One line for the top of a page.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Notice {
    pub tone: Tone,
    pub words: String,
    /// What the person typed, put back in the form when it didn't pass, so nothing is lost.
    #[serde(skip)]
    pub kept: BTreeMap<String, String>,
}

/// Notices waiting for their page, by id.
#[derive(Default)]
pub(crate) struct Notices(Mutex<HashMap<String, (Notice, Instant)>>);

impl Notices {
    const TTL: Duration = Duration::from_secs(600);
    const MAX: usize = 1024;

    fn put(&self, notice: Notice) -> String {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let id = hex::encode(bytes);
        let mut map = self.0.lock().expect("notices lock");
        map.retain(|_, (_, at)| at.elapsed() < Self::TTL);
        if map.len() < Self::MAX {
            map.insert(id.clone(), (notice, Instant::now()));
        }
        id
    }

    fn take(&self, id: &str) -> Option<Notice> {
        let mut map = self.0.lock().expect("notices lock");
        let (notice, at) = map.remove(id)?;
        (at.elapsed() < Self::TTL).then_some(notice)
    }
}

tokio::task_local! {
    /// The `n` the current request asked for.
    static ASKED: Option<String>;
}

/// Middleware: remembers the request's `?n=` so the page it renders can show that notice.
pub(crate) async fn scope(req: Request, next: Next) -> Response {
    let id = req.uri().query().and_then(|q| {
        url::form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "n")
            .map(|(_, v)| v.into_owned())
    });
    ASKED.scope(id, next.run(req)).await
}

/// The notice the current request carries, taken so a reload doesn't show it again.
pub(crate) fn current(s: &OperatorState) -> Option<Notice> {
    let id = ASKED.try_with(Clone::clone).ok().flatten()?;
    s.inner.notices.take(&id)
}

/// `to` (a path, perhaps with `?query` and `#fragment`) carrying a new notice.
pub(crate) fn link(s: &OperatorState, to: &str, tone: Tone, words: impl Into<String>) -> String {
    link_keeping(s, to, tone, words, &[])
}

fn link_keeping(
    s: &OperatorState,
    to: &str,
    tone: Tone,
    words: impl Into<String>,
    kept: &[(&str, &str)],
) -> String {
    let id = s.inner.notices.put(Notice {
        tone,
        words: words.into(),
        kept: kept
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    });
    let (path, fragment) = match to.split_once('#') {
        Some((p, f)) => (p, format!("#{f}")),
        None => (to, String::new()),
    };
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}n={id}{fragment}")
}

/// Back to `to` with "it's done" words.
pub(crate) fn done(s: &OperatorState, to: &str, words: impl Into<String>) -> Response {
    Redirect::to(&link(s, to, Tone::Done, words)).into_response()
}

/// Back to `to` with what stopped the form and what to do now.
pub(crate) fn problem(s: &OperatorState, to: &str, words: impl Into<String>) -> Response {
    Redirect::to(&link(s, to, Tone::Problem, words)).into_response()
}

/// Back to `to` with what stopped the form, putting `kept` (field, typed text) back in it.
pub(crate) fn problem_keeping(
    s: &OperatorState,
    to: &str,
    words: impl Into<String>,
    kept: &[(&str, &str)],
) -> Response {
    Redirect::to(&link_keeping(s, to, Tone::Problem, words, kept)).into_response()
}

/// A form posted from another site: refused, in words, with nothing changed.
pub(crate) fn cross_site() -> Response {
    (
        axum::http::StatusCode::FORBIDDEN,
        axum::response::Html("<!doctype html><meta charset=utf-8><title>Control plane</title><p style=\"font-family:system-ui;padding:24px\">This form came from another site, so it was refused and nothing changed. Open the page again and try there.</p>"),
    )
        .into_response()
}

/// Where a signed-out form post goes: sign-in, then back to the form's page.
pub(crate) fn sign_in_again(back: &str) -> Response {
    let path = back.split('#').next().unwrap_or("/");
    let next: String = url::form_urlencoded::byte_serialize(path.as_bytes()).collect();
    Redirect::to(&format!("/signin?next={next}")).into_response()
}

/// "1 hour", "4 hours".
pub(crate) fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plurals_read_right() {
        assert_eq!(plural(1, "hour", "hours"), "1 hour");
        assert_eq!(plural(4, "hour", "hours"), "4 hours");
        assert_eq!(plural(0, "site", "sites"), "0 sites");
    }

    #[test]
    fn a_notice_is_shown_once() {
        let n = Notices::default();
        let id = n.put(Notice {
            tone: Tone::Done,
            words: "Saved.".into(),
            kept: BTreeMap::new(),
        });
        assert_eq!(n.take(&id).unwrap().words, "Saved.");
        assert!(n.take(&id).is_none());
        assert!(n.take("made-up").is_none());
    }
}
