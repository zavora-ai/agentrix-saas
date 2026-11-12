//! The shell's stylesheets and scripts, compiled in and served under `/static/os/`.
//!
//! The look comes from agentrix-ui (`/static/ui/`, mounted by the kit). `shell.css` holds the
//! console components the kit doesn't ship yet and the states the concepts never drew.

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

/// A version stamp that changes whenever a static file changes; pages append `?v=…`.
pub const ASSETS_VERSION: &str = env!("AGENTRIX_ASSETS_VERSION");

const SHELL_CSS: &str = include_str!("../static/shell.css");
const OS_JS: &str = include_str!("../static/os.js");
const APPROVE_JS: &str = include_str!("../static/approve.js");
const DIALS_JS: &str = include_str!("../static/dials.js");
const GESTURES_JS: &str = include_str!("../static/gestures.js");
const GESTURES_CORE_JS: &str = include_str!("../static/gestures-core.js");
const LIVE_JS: &str = include_str!("../static/live.js");

fn serve(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
}

const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";

/// The `/static/os/*` routes.
pub fn assets() -> Router {
    Router::new()
        .route(
            "/static/os/shell.css",
            get(|| async { serve(CSS, SHELL_CSS) }),
        )
        .route("/static/os/os.js", get(|| async { serve(JS, OS_JS) }))
        .route("/static/os/dials.js", get(|| async { serve(JS, DIALS_JS) }))
        .route("/static/os/live.js", get(|| async { serve(JS, LIVE_JS) }))
        .route(
            "/static/os/gestures.js",
            get(|| async { serve(JS, GESTURES_JS) }),
        )
        .route(
            "/static/os/gestures-core.js",
            get(|| async { serve(JS, GESTURES_CORE_JS) }),
        )
        .route(
            "/static/os/approve.js",
            get(|| async { serve(JS, APPROVE_JS) }),
        )
}
