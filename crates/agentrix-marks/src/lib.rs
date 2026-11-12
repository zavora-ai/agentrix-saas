//! The WebAgent living mark ([ADR-031](../../../docs/adr/031-a-webagent-is-a-living-mark.md)).
//!
//! A WebAgent's look is a [`MarkSpec`] in its spec. One generator turns a spec into SVG:
//! [`render`] here for signed releases and server-rendered pages, and `assets/marks.js` in the
//! browser for the Workshop's live preview. Both are built from the same arithmetic
//! ([`num`]) and the same seeded generator ([`rng`]); the golden tests in `tests/` render the
//! same cases through both and compare them byte for byte.
//!
//! - **Four layers:** silhouette (a squircle, never a circle), signature (one family per site,
//!   one seed per WebAgent, from 28 px up, at most 30% over the fill), glyph (a role icon or a
//!   monogram), and state (the platform's, set only by presence frames).
//! - **States:** ten, in [`MarkState`], with the owner's and the visitor's words in [`words`].
//!   All motion is CSS on transform and opacity (`assets/marks.css`); it stops under reduced
//!   motion and settles after 5 seconds in one state, except `speaking`, which follows the real
//!   voice level.
//! - **Team mark:** the Chief WebAgent's mark, [`render_team`], with one dot per WebAgent.
//! - **Contrast:** [`color::passes`] and [`color::nearest_passing`] are the editor's checks.
//!
//! The crate has no I/O and no async; it is safe to call from templates and release builds.

pub mod color;
pub mod glyphs;
pub mod num;
pub mod rng;
pub mod spec;
pub mod state;
pub mod svg;

pub use spec::{
    Family, Glyph, MarkError, MarkSpec, MarkSpecParseError, Shape, Signature, Silhouette,
};
pub use state::{Audience, MarkState, WordsContext, words};
pub use svg::{Render, TeamMark, TeamMember, content_uid, render, render_team};

/// The browser half: the same generator in JavaScript, the `<wa-mark>` element and the states'
/// CSS. Served by `agentrix-kernel` at `/_agentrix/marks.js` and `/_agentrix/marks.css`.
pub mod assets {
    /// `marks.js`: an ES module.
    pub const JS: &str = include_str!("../assets/marks.js");
    /// `marks.css`: the ten states, settling and reduced motion.
    pub const CSS: &str = include_str!("../assets/marks.css");
    /// Where pages load them from.
    pub const JS_PATH: &str = "/_agentrix/marks.js";
    /// Where pages load them from.
    pub const CSS_PATH: &str = "/_agentrix/marks.css";

    /// A weak validator for each asset: the same text gives the same tag across restarts.
    pub fn etag(body: &str) -> String {
        format!(
            "W/\"{:08x}{:04x}\"",
            crate::rng::seed_from(body),
            body.len() & 0xffff
        )
    }

    /// Serves [`JS_PATH`] and [`CSS_PATH`] on any host: the platform host, the console, and
    /// every site (pages load them same-origin, so `script-src 'self'` holds).
    #[cfg(feature = "serve")]
    pub fn router() -> axum::Router {
        use axum::routing::get;
        axum::Router::new()
            .route(JS_PATH, get(|h: http::HeaderMap| async move { serve(&h, JS, "text/javascript; charset=utf-8") }))
            .route(CSS_PATH, get(|h: http::HeaderMap| async move { serve(&h, CSS, "text/css; charset=utf-8") }))
    }

    #[cfg(feature = "serve")]
    fn serve(
        headers: &http::HeaderMap,
        body: &'static str,
        kind: &'static str,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        use http::{StatusCode, header};
        let tag = etag(body);
        let fresh = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(',').any(|t| t.trim() == tag));
        let common = [
            (header::ETAG, tag),
            (
                header::CACHE_CONTROL,
                "public, max-age=300, must-revalidate".to_string(),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ];
        if fresh {
            return (StatusCode::NOT_MODIFIED, common).into_response();
        }
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, kind.to_string())],
            common,
            body,
        )
            .into_response()
    }
}

#[cfg(all(test, feature = "serve"))]
mod serve_tests {
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_both_assets_with_validators() {
        let app = super::assets::router();
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::get(super::assets::JS_PATH)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/javascript")
        );
        let tag = res.headers()["etag"].clone();
        let again = app
            .oneshot(
                axum::http::Request::get(super::assets::JS_PATH)
                    .header("if-none-match", tag)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(again.status(), 304);
    }
}
