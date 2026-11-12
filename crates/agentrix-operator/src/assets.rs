//! The operator console's stylesheet and scripts, compiled in and served under `/ops-static/`.

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

/// Changes whenever a static file changes, so pages may cache `/ops-static/*?v=…` as immutable.
pub const ASSETS_VERSION: &str = env!("AGENTRIX_ASSETS_VERSION");

const OPS_CSS: &str = include_str!("../static/ops.css");
const OPS_JS: &str = include_str!("../static/ops.js");
const SIGNIN_JS: &str = include_str!("../static/signin.js");
const GRANT_JS: &str = include_str!("../static/grant.js");

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

pub(crate) fn routes() -> Router {
    Router::new()
        .route(
            "/ops-static/ops.css",
            get(|| async { serve("text/css; charset=utf-8", OPS_CSS) }),
        )
        .route(
            "/ops-static/ops.js",
            get(|| async { serve("text/javascript; charset=utf-8", OPS_JS) }),
        )
        .route(
            "/ops-static/grant.js",
            get(|| async { serve("text/javascript; charset=utf-8", GRANT_JS) }),
        )
        .route(
            "/ops-static/signin.js",
            get(|| async { serve("text/javascript; charset=utf-8", SIGNIN_JS) }),
        )
}
