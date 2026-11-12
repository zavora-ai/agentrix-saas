//! Static assets, compiled into the binary and served under `/static/onboarding/`.

use axum::Router;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

/// A version stamp that changes whenever a static file changes. Templates append it as
/// `?v=…`, so browsers may cache assets as immutable.
pub const ONBOARDING_ASSETS_VERSION: &str = env!("AGENTRIX_ONBOARDING_ASSETS_VERSION");

const ONBOARDING_CSS: &str = include_str!("../static/onboarding.css");
const ASSESS_JS: &str = include_str!("../static/assess.js");
const ACCOUNT_JS: &str = include_str!("../static/account.js");
const MOVE_JS: &str = include_str!("../static/move.js");
const PLAN_JS: &str = include_str!("../static/plan.js");
const REACH_JS: &str = include_str!("../static/reach.js");
const BILLING_JS: &str = include_str!("../static/billing.js");
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

/// The `/static/onboarding/*` routes, for node to merge into its router.
pub fn assets<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/static/onboarding/onboarding.css",
            get(|| async { serve("text/css; charset=utf-8", ONBOARDING_CSS) }),
        )
        .route(
            "/static/onboarding/assess.js",
            get(|| async { serve("text/javascript; charset=utf-8", ASSESS_JS) }),
        )
        .route(
            "/static/onboarding/account.js",
            get(|| async { serve("text/javascript; charset=utf-8", ACCOUNT_JS) }),
        )
        .route(
            "/static/onboarding/move.js",
            get(|| async { serve("text/javascript; charset=utf-8", MOVE_JS) }),
        )
        .route(
            "/static/onboarding/billing.js",
            get(|| async { serve("text/javascript; charset=utf-8", BILLING_JS) }),
        )
        .route(
            "/static/onboarding/reach.js",
            get(|| async { serve("text/javascript; charset=utf-8", REACH_JS) }),
        )
        .route(
            "/static/onboarding/plan.js",
            get(|| async { serve("text/javascript; charset=utf-8", PLAN_JS) }),
        )
        .route(
            "/static/onboarding/upload.js",
            get(|| async { serve("text/javascript; charset=utf-8", UPLOAD_JS) }),
        )
}
