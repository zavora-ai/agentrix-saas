//! Static assets, compiled into the binary and served under `/static/`.

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

/// A version stamp that changes whenever a static file changes. Templates append it as
/// `?v=…`, so browsers may cache assets as immutable.
pub const ASSETS_VERSION: &str = env!("AGENTRIX_ASSETS_VERSION");

const APP_CSS: &str = include_str!("../static/app.css");
const LIVE_JS: &str = include_str!("../static/live.js");
const PASSKEY_JS: &str = include_str!("../static/passkey.js");
const VERIFY_JS: &str = include_str!("../static/verify.js");
const AUTH_JS: &str = include_str!("../static/auth.js");
const UPLOAD_JS: &str = include_str!("../static/upload.js");

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

/// The `/static/*` routes, for `agentrix-node` to merge into its router.
pub fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/static/app.css",
            get(|| async { serve("text/css; charset=utf-8", APP_CSS) }),
        )
        .route(
            "/static/live.js",
            get(|| async { serve("text/javascript; charset=utf-8", LIVE_JS) }),
        )
        .route(
            "/static/passkey.js",
            get(|| async { serve("text/javascript; charset=utf-8", PASSKEY_JS) }),
        )
        .route(
            "/static/verify.js",
            get(|| async { serve("text/javascript; charset=utf-8", VERIFY_JS) }),
        )
        .route(
            "/static/auth.js",
            get(|| async { serve("text/javascript; charset=utf-8", AUTH_JS) }),
        )
        .route(
            "/static/upload.js",
            get(|| async { serve("text/javascript; charset=utf-8", UPLOAD_JS) }),
        )
}
