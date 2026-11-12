//! The shared UI kit (contract C10, `specs/platform/02-architecture.md` §4 and §6.1).
//!
//! One look for every surface: the platform's own pages, the owner console, the control plane
//! and the sample sites. The kit holds:
//!
//! - **Tokens** (`/static/ui/tokens.css`): the Agentrix layer from `concept/styles.css`, one dark
//!   theme that Night Time switches on by schedule, the shell's `--os-*` surfaces, scope markers,
//!   presence colours and the host layer `--wa-*` with a set per sample (`data-host="kilele"`).
//! - **Fonts**, self-hosted under the SIL Open Font License (`/static/ui/fonts/OFL.txt`):
//!   Manrope, DM Sans, JetBrains Mono, and the sample sites' display faces.
//! - **Stylesheets** with the concepts' class names, so markup ported from `concept/` renders the
//!   same: `kit.css` (every surface), `shell.css` (console and control plane), `site.css`
//!   (site hosts).
//! - **Templates**: `ui/base.html`, the layouts in `ui/layouts/`, `ui/error.html`, and the
//!   macros in `ui/kit.html`. See `README.md` in this crate for every name and argument.
//! - **Islands**: `kit.js` (theme schedule, toasts, sheets). Plain ES modules, no inline scripts.
//! - **Words**: `words/en.json`, the fixed lines a desk can't rephrase, as the `words` global.
//! - **Marks**: `wa_mark`, `wa_team`, `wa_state` and `wa_words`, on `agentrix-marks` (W1).
//!
//! ```ignore
//! // tests/kit.rs runs this same path (`page_layout_renders_with_no_variables`).
//! let mut env = agentrix_ui::environment();
//! env.add_template("hello.html", r#"{% extends "ui/layouts/page.html" %}
//! {% import "ui/kit.html" as ui %}
//! {% block main %}{{ ui.button("Approve", kind="primary") }}{% endblock %}"#).unwrap();
//! let html = env.get_template("hello.html").unwrap().render(()).unwrap();
//! assert!(html.contains("btn btn-primary"));
//! ```
//!
//! A crate that already has its own environment calls [`install`] instead, and merges
//! [`assets`] into its router.

use std::sync::OnceLock;

use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use minijinja::{AutoEscape, Environment, Output, State, UndefinedBehavior, Value};

mod marks;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}

/// Changes whenever a static file changes. Layouts append it as `?v=…`.
pub const VERSION: &str = env!("AGENTRIX_UI_VERSION");

/// Where [`assets`] serves the kit's static files.
pub const BASE: &str = "/static/ui";

/// Every kit template, as `(name, source)`. Names start with `ui/`.
pub fn templates() -> impl Iterator<Item = (&'static str, &'static str)> {
    embedded::TEMPLATES.iter().copied()
}

/// One static file by its path under [`BASE`], with its content type.
pub fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    embedded::ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(name, bytes)| (*bytes, content_type(name)))
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("woff2") => "font/woff2",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The words catalogue for `lang` (`"en"` today; Swahili ships when a person has written it).
pub fn words(lang: &str) -> Option<&'static serde_json::Value> {
    static PARSED: OnceLock<Vec<(&'static str, serde_json::Value)>> = OnceLock::new();
    PARSED
        .get_or_init(|| {
            embedded::WORDS
                .iter()
                .map(|(lang, src)| {
                    let value = serde_json::from_str(src)
                        .unwrap_or_else(|e| panic!("words/{lang}.json is not valid JSON: {e}"));
                    (*lang, value)
                })
                .collect()
        })
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, v)| v)
}

/// One fixed line by its dotted key, for Rust code that writes outside a template (emails,
/// WhatsApp, SSE frames): `word("en", "visitor.disclosure")`.
pub fn word(lang: &str, key: &str) -> Option<&'static str> {
    key.split('.')
        .try_fold(words(lang)?, |node, part| node.get(part))?
        .as_str()
}

/// Adds the kit's templates and globals to an existing environment: `ui_v` (the asset version),
/// `ui_base` (where the assets live) and `words` (the English catalogue).
pub fn install(env: &mut Environment<'_>) -> Result<(), minijinja::Error> {
    for (name, source) in templates() {
        env.add_template(name, source)?;
    }
    env.add_global("ui_v", VERSION);
    env.add_global("ui_base", BASE);
    // Links /_agentrix/marks.css and marks.js, which agentrix-kernel serves on every host.
    env.add_global("ui_marks", true);
    marks::install(env);
    env.add_global(
        "words",
        Value::from_serialize(words("en").expect("the kit ships words/en.json")),
    );
    Ok(())
}

/// A fresh environment as every surface should run it: strict undefined (a missing value is an
/// error, never a blank on screen), the escaping [`format`]ter, and the kit installed.
pub fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_formatter(format);
    install(&mut env).expect("the kit's own templates parse");
    env
}

/// Writes a value into the page. Text is HTML-escaped (`& < > " '`), which is enough in text
/// and in quoted attributes. minijinja's default formatter also turns `/` into `&#x2f;`, which
/// is safe but makes every URL in the page source unreadable. Same as the first release's console.
pub fn format(out: &mut Output, state: &State, value: &Value) -> Result<(), minijinja::Error> {
    match (state.auto_escape(), value.as_str()) {
        (AutoEscape::Html, Some(text)) if !value.is_safe() => {
            let mut done = 0;
            for (i, byte) in text.bytes().enumerate() {
                let entity = match byte {
                    b'&' => "&amp;",
                    b'<' => "&lt;",
                    b'>' => "&gt;",
                    b'"' => "&quot;",
                    b'\'' => "&#x27;",
                    _ => continue,
                };
                out.write_str(&text[done..i])?;
                out.write_str(entity)?;
                done = i + 1;
            }
            Ok(out.write_str(&text[done..])?)
        }
        _ => minijinja::escape_formatter(out, state, value),
    }
}

async fn serve(Path(path): Path<String>) -> Response {
    match asset(&path) {
        Some((body, content_type)) => (
            [
                (header::CONTENT_TYPE, content_type),
                // Layouts link every file with `?v=` + VERSION, so a cached copy is never stale.
                // Fonts are linked from tokens.css without it; their names change if they do.
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ],
            body,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The `/static/ui/*` routes, for every host that renders kit pages (the platform host, the
/// operator host and every site host). A module that mounts routes on its own hosts merges
/// these into its router; [`mount`] puts them on the platform host. Fonts carry
/// `Access-Control-Allow-Origin: *` so a page may also load them from the platform host.
pub fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new().route("/static/ui/{*path}", get(serve))
}

/// The seam (contract C11): the kit's assets on the platform host. Other hosts get them from
/// the module that serves the host, by merging [`assets`] into its router, so the kit never
/// claims a host (which would make node's TLS ask say yes for it).
pub fn mount(_ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    Ok(agentrix_host::Mounted::default().platform(assets()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_static_file_has_a_known_type() {
        for (name, _) in embedded::ASSETS {
            assert_ne!(content_type(name), "application/octet-stream", "{name}");
        }
    }

    #[test]
    fn words_resolve_by_dotted_key() {
        assert!(
            word("en", "visitor.disclosure").is_some_and(|w| w.starts_with("You're talking to AI"))
        );
        assert_eq!(word("en", "visitor.nope"), None);
        assert_eq!(word("sw", "visitor.disclosure"), None);
    }
}
