//! The individual WebAgent (`specs/platform/02-architecture.md` §3; `docs/05-webagents.md`).
//!
//! A WebAgent is a governed worker with its own principal, mandate, jobs, budget and per-job
//! autonomy dial, and a living mark that shows truthfully what it is doing (ADR-010, ADR-031).
//! Horizon 1 builds the platform layer beside the first release's code:
//! - the living marks' assets, served on every surface ([`agentrix_marks`]);
//! - the roster of nine as data, with `MarkSpec`s, for every site ([`roster`], [`store`]);
//! - presence from real activity over SSE; the dial check on the governed path; section desks
//!   (next).
//!
//! Horizon 2 hosts each site's WebAgents as `adk-kernel` processes (02 §3, "Recommended shape").

pub mod chief;
pub mod dials;
pub mod home;
pub mod looks;
pub mod policy;
pub mod presence;
pub mod roster;
mod routes;
pub mod signals;
pub mod spec;
pub mod store;
pub mod taught;
pub mod workshop;

pub use dials::{DIAL_TOOL, DialChange, DialError, DialRunner, Dials, Requested};
pub use looks::SiteLook;
pub use policy::{Action, RuleCode, Ruling, Verdict};
pub use presence::{Presence, PresenceFrame};
pub use roster::{DeskDraft, Member, Roster, Status};
pub use spec::{DeskSpec, DialKind, Effect, JobSpec, Level, WebAgentSpec};
pub use store::{Rosters, migrate};

use agentrix_host::{Mounted, NodeContext};

/// Mounts the WebAgent runtime on the platform host: the living marks' script and styles, each
/// owner's roster as JSON (`GET /api/webagents/{site}`) and its presence stream
/// (`GET /api/webagents/{site}/presence`). Node runs [`migrate`] first, and feeds
/// [`presence::global`] from the events it already has (see [`presence`]).
///
/// Site hosts need the marks' two paths too; node merges [`site_routes`] into its site router.
///
/// One task: agentrix.website's own team ([`home`]) is recorded the first time, and its Chief's
/// brief is kept current.
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
    Ok(Mounted::default()
        .task(
            "webagents: agentrix.website's team",
            Box::pin(home::run(
                rosters.clone(),
                presence::global().clone(),
                home::Platform {
                    host: ctx.platform_host.clone(),
                    origin: ctx.origin.to_string(),
                },
            )),
        )
        .platform(agentrix_marks::assets::router())
        .platform(workshop_assets::router())
        .platform(routes::platform(routes::Ctx {
            dials: Dials::new(rosters.clone(), ctx.receipts.clone(), ctx.approvals.clone())
                .with_cards(agentrix_decisions::Decisions::new(ctx)),
            receipts: ctx.receipts.clone(),
            node: ctx.clone(),
            rosters,
            owners: ctx.owners.clone(),
            presence: presence::global().clone(),
            origin: ctx.origin.as_str().trim_end_matches('/').to_string(),
        })))
}

/// The Workshop island (`<wa-workshop site="…" webagent="…">`), for the console's WebAgent card.
pub mod workshop_assets {
    /// `workshop.js`: an ES module that imports `./marks.js`.
    pub const JS: &str = include_str!("../assets/workshop.js");
    /// `workshop.css`: layout on agentrix-ui's tokens.
    pub const CSS: &str = include_str!("../assets/workshop.css");
    /// Where the console loads them from.
    pub const JS_PATH: &str = "/_agentrix/workshop.js";
    /// Where the console loads them from.
    pub const CSS_PATH: &str = "/_agentrix/workshop.css";

    /// Serves both, with validators like the marks' assets.
    pub fn router() -> axum::Router {
        use axum::http::{HeaderMap, StatusCode, header};
        use axum::response::IntoResponse;
        use axum::routing::get;
        fn serve(
            h: &HeaderMap,
            body: &'static str,
            kind: &'static str,
        ) -> axum::response::Response {
            let tag = agentrix_marks::assets::etag(body);
            let fresh = h
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
        axum::Router::new()
            .route(
                JS_PATH,
                get(|h: HeaderMap| async move { serve(&h, JS, "text/javascript; charset=utf-8") }),
            )
            .route(
                CSS_PATH,
                get(|h: HeaderMap| async move { serve(&h, CSS, "text/css; charset=utf-8") }),
            )
    }
}

/// The runner for approved `dial.raise` cards. Node registers it with
/// `agentrix_decisions::mount_with(ctx, vec![agentrix_kernel::dial_runner(ctx), …])`.
pub fn dial_runner(ctx: &NodeContext) -> std::sync::Arc<dyn agentrix_decisions::EffectRunner> {
    let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
    std::sync::Arc::new(dials::DialRunner::new(Dials::new(
        rosters,
        ctx.receipts.clone(),
        ctx.approvals.clone(),
    )))
}

/// Routes every site host serves, same-origin: `/_agentrix/marks.js` and `/_agentrix/marks.css`.
pub fn site_routes() -> axum::Router {
    agentrix_marks::assets::router()
}

#[cfg(test)]
mod tests {
    use tower::ServiceExt;

    #[tokio::test]
    async fn mount_serves_the_marks_on_the_platform_host() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        super::migrate(&ctx.pool).await.unwrap();
        let mounted = super::mount(&ctx).unwrap();
        for path in [
            "/_agentrix/marks.js",
            "/_agentrix/marks.css",
            "/_agentrix/workshop.js",
            "/_agentrix/workshop.css",
        ] {
            let res = mounted
                .platform
                .clone()
                .oneshot(
                    axum::http::Request::get(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), 200, "{path}");
        }
    }
}
