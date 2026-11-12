//! The kernel's routes on the platform host, for the signed-in owner of the site:
//!
//! - `GET /api/webagents/{site}`: the roster, with each mark rendered, for the console's
//!   islands (the Workshop's live preview, the dock). Server pages call [`crate::Rosters`].
//! - `GET /api/webagents/{site}/presence`: an SSE stream. First a `snapshot` of every member on
//!   duty, then a `presence` frame each time one really changes state. `marks.js`'s
//!   `connectPresence` applies them to the page's `<wa-mark>`s.
//! - `GET /api/webagents/{site}/dials`: raises waiting for the owner.
//! - `POST /api/webagents/{site}/dials` `{webagent, job, level}`: lowers at once
//!   (`outcome: "applied"`), or raises a decision card v2 (`outcome: "card"`) that the owner
//!   approves in the console's lane at `/decisions/v2/{id}/approve`.
//! - `POST /api/webagents/{site}/dials/{change}/approve` and `…/decline`: for changes raised
//!   without cards (`outcome: "needs-approval"`, `{"passkey": …}` or `{"totp": "…"}`).
//! - `GET /api/webagents/{site}/signals`: what the desks told the back office, newest first.
//! - `POST /api/webagents/{site}/marks` `{webagent, mark}`: the Workshop's Save. 409 with
//!   `suggestion` when the colour is too faint on the site.
//! - `GET /api/webagents/agentrix-home/live`: the one stream without sign-in, for visitors to
//!   agentrix.website: its own team's marks as a visitor sees them ([`crate::home`]).
//!
//! POSTs take JSON only and refuse a foreign `Origin`, as node's own sign-in routes do.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use agentrix_host::OwnerSessions;
use agentrix_marks::MarkState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::StreamExt;
use serde_json::json;
use tokio_stream::wrappers::BroadcastStream;

use crate::dials::{DialError, Dials};
use crate::home;
use crate::presence::{self, Presence};
use crate::roster::Roster;
use crate::spec::Level;
use crate::store::Rosters;
use crate::workshop::MarkSaveError;

#[derive(Clone)]
pub(crate) struct Ctx {
    pub(crate) rosters: Rosters,
    pub(crate) owners: Arc<dyn OwnerSessions>,
    pub(crate) presence: Presence,
    pub(crate) dials: Dials,
    pub(crate) receipts: Arc<dyn agentrix_host::Receipts>,
    /// Node's context, for the Chief WebAgent.
    pub(crate) node: agentrix_host::NodeContext,
    /// The platform's origin, without a trailing slash, for the same-origin check on POSTs.
    pub(crate) origin: String,
}

pub(crate) fn platform(cx: Ctx) -> Router {
    Router::new()
        .route("/api/webagents/{site}", get(roster))
        .route("/api/webagents/{site}/presence", get(presence_stream))
        .route(
            "/api/webagents/{site}/dials",
            get(dials_pending).post(dial_request),
        )
        .route(
            "/api/webagents/{site}/dials/{change}/approve",
            post(dial_approve),
        )
        .route(
            "/api/webagents/{site}/dials/{change}/decline",
            post(dial_decline),
        )
        .route("/api/webagents/{site}/signals", get(signals))
        .route("/api/webagents/{site}/marks", post(save_mark))
        .route("/api/webagents/{site}/chief", post(ask_chief))
        .route("/api/webagents/{site}/traces", get(chief_traces))
        .route("/api/webagents/{site}/live", get(live_stream))
        .with_state(cx)
}

/// agentrix.website's team for anyone on agentrix.website, without sign-in, and only for
/// [`home::SITE`]: an SSE stream like `/presence` (a `snapshot`, then a `presence` frame per
/// change), each frame as a visitor sees it ([`home::for_visitor`]).
async fn live_stream(State(cx): State<Ctx>, Path(site): Path<String>) -> Response {
    let roster = match (site == home::SITE)
        .then_some(())
        .map(|()| cx.rosters.roster(&site))
    {
        Some(r) => match r.await {
            Ok(Some(r)) => r,
            Ok(None) => return refuse(StatusCode::NOT_FOUND, "There's no live team here."),
            Err(e) => {
                tracing::warn!(%e, "agentrix.website's roster unavailable");
                return refuse(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The team couldn't be read just now.",
                );
            }
        },
        None => return refuse(StatusCode::NOT_FOUND, "There's no live team here."),
    };
    if let Err(e) = home::refresh(&cx.presence, cx.rosters.pool()).await {
        tracing::warn!(error = %format!("{e:#}"), "agentrix.website's records couldn't be read");
    }
    let (_, rx) = cx.presence.subscribe(&site);
    home::watch(&cx.presence, cx.rosters.pool());
    let owner = roster.look.owner.clone();
    let snapshot: Vec<serde_json::Value> = cx
        .presence
        .snapshot(&roster)
        .iter()
        .map(|f| home::for_visitor(f, &owner))
        .collect();
    let first = Event::default()
        .event("snapshot")
        .json_data(json!({ "frames": snapshot }))
        .unwrap_or_else(|_| Event::default().event("snapshot").data("{\"frames\":[]}"));
    let frames = BroadcastStream::new(rx).filter_map(move |r| {
        let owner = owner.clone();
        async move {
            let frame = r.ok()?;
            Event::default()
                .event("presence")
                .id(frame.seq.to_string())
                .json_data(home::for_visitor(&frame, &owner))
                .ok()
        }
    });
    let stream = futures::stream::once(async move { first })
        .chain(frames)
        .map(Ok::<Event, Infallible>);
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
        .into_response()
}

/// Refuses a POST from another origin (a third layer after SameSite cookies and JSON bodies).
fn cross_origin(cx: &Ctx, headers: &HeaderMap) -> Option<Response> {
    let origin = headers.get(axum::http::header::ORIGIN)?.to_str().ok()?;
    (origin != cx.origin).then(|| {
        refuse(
            StatusCode::FORBIDDEN,
            "That request came from another site, so nothing changed.",
        )
    })
}

/// The signed-in owner's roster for `site`, or the response that refuses it.
async fn owned_roster(cx: &Ctx, site: &str, headers: &HeaderMap) -> Result<Roster, Response> {
    owner_and_roster(cx, site, headers).await.map(|(_, r)| r)
}

/// The signed-in owner's account and roster for `site`, or the response that refuses it.
async fn owner_and_roster(
    cx: &Ctx,
    site: &str,
    headers: &HeaderMap,
) -> Result<(uuid::Uuid, Roster), Response> {
    let owner = match cx.owners.owner(headers).await {
        Ok(Some(o)) => o,
        Ok(None) => {
            return Err(refuse(
                StatusCode::UNAUTHORIZED,
                "Sign in to see your WebAgents.",
            ));
        }
        Err(e) => {
            tracing::warn!(%e, "owner session check failed");
            return Err(refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't check your sign-in. Try again.",
            ));
        }
    };
    match cx.rosters.owns(site, owner.account).await {
        Ok(true) => {}
        Ok(false) => {
            return Err(refuse(
                StatusCode::NOT_FOUND,
                "There's no site with that id on your account.",
            ));
        }
        Err(e) => {
            tracing::warn!(%e, "ownership check failed");
            return Err(refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't read your sites. Try again.",
            ));
        }
    }
    match cx.rosters.roster(site).await {
        Ok(Some(r)) => Ok((owner.account, r)),
        Ok(None) => Err(refuse(
            StatusCode::NOT_FOUND,
            "There's no site with that id on your account.",
        )),
        Err(e) => {
            tracing::warn!(%e, "roster read failed");
            Err(refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't read your WebAgents. Try again.",
            ))
        }
    }
}

async fn presence_stream(
    State(cx): State<Ctx>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Response {
    let roster = match owned_roster(&cx, &site, &headers).await {
        Ok(r) => r,
        Err(refused) => return refused,
    };
    if let Err(e) = presence::baseline(&cx.presence, cx.rosters.pool(), &site).await {
        tracing::warn!(%e, site, "presence baseline unavailable");
    }
    if site == home::SITE
        && let Err(e) = home::refresh(&cx.presence, cx.rosters.pool()).await
    {
        tracing::warn!(error = %format!("{e:#}"), "agentrix.website's records couldn't be read");
    }
    let (_, rx) = cx.presence.subscribe(&site);
    if site == home::SITE {
        home::watch(&cx.presence, cx.rosters.pool());
    }
    let snapshot = cx.presence.snapshot(&roster);
    let first = Event::default()
        .event("snapshot")
        .json_data(json!({ "frames": snapshot }))
        .unwrap_or_else(|_| Event::default().event("snapshot").data("{\"frames\":[]}"));
    let frames = BroadcastStream::new(rx).filter_map(|r| async move {
        // A lagging stream skips what it missed; the next frame is the current state anyway.
        let frame = r.ok()?;
        Event::default()
            .event("presence")
            .id(frame.seq.to_string())
            .json_data(&frame)
            .ok()
    });
    let stream = futures::stream::once(async move { first })
        .chain(frames)
        .map(Ok::<Event, Infallible>);
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
        .into_response()
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn roster(State(cx): State<Ctx>, Path(site): Path<String>, headers: HeaderMap) -> Response {
    let roster = match owned_roster(&cx, &site, &headers).await {
        Ok(r) => r,
        Err(refused) => return refused,
    };
    let members: Vec<_> = roster
        .members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            json!({
                "id": m.spec.id,
                "name": m.spec.name,
                "role_words": m.spec.role_words,
                "status": m.status,
                "status_words": m.status.words(),
                "principal": m.principal,
                "svg": roster.mark_svg(&m.spec.id, 40.0, MarkState::Idle, &format!("r{i}")),
                "default_mark": crate::workshop::default_mark(&roster, &m.spec.id),
                "spec": m.spec,
            })
        })
        .collect();
    let desks: Vec<_> = roster
        .desks()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            json!({
                "id": d.id,
                "name": d.name,
                "mandate": d.mandate,
                "path": d.path,
                "mark": d.mark,
                "default_mark": crate::workshop::default_mark(&roster, &d.id),
                "svg": roster.mark_svg(&d.id, 40.0, MarkState::Idle, &format!("d{i}")),
            })
        })
        .collect();
    // What the owner and a visitor read beside a mark in each state (06 §4.3), for previews.
    let cx_words = agentrix_marks::WordsContext {
        owner: Some(&roster.look.owner),
        ..Default::default()
    };
    let words: serde_json::Map<String, serde_json::Value> = MarkState::ALL
        .iter()
        .map(|s| {
            (
                s.key().to_string(),
                json!({
                    "name": s.name(),
                    "owner": agentrix_marks::words(*s, agentrix_marks::Audience::Owner, &cx_words),
                    "visitor": agentrix_marks::words(*s, agentrix_marks::Audience::Visitor, &cx_words),
                }),
            )
        })
        .collect();
    Json(json!({
        "site": roster.site,
        "look": roster.look,
        "team_svg": roster.team_svg(44.0, MarkState::Idle, &[]),
        "members": members,
        "desks": desks,
        "words": words,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct ChiefBody {
    said: String,
    #[serde(default)]
    surface: Option<crate::chief::Surface>,
}

/// The Chief WebAgent answers the signed-in owner (the console's command bar, voice in the
/// browser): `{said, surface}` in, [`crate::chief::Answer`] out.
async fn ask_chief(
    State(cx): State<Ctx>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ChiefBody>,
) -> Response {
    if let Some(refused) = cross_origin(&cx, &headers) {
        return refused;
    }
    if let Err(refused) = owner_and_roster(&cx, &site, &headers).await {
        return refused;
    }
    let surface = body.surface.unwrap_or(crate::chief::Surface::Console);
    Json(crate::chief::answer(&cx.node, &site, &body.said, surface).await).into_response()
}

/// The site's latest Chief answers with their steps, for the console's Activity view.
async fn chief_traces(
    State(cx): State<Ctx>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(refused) = owned_roster(&cx, &site, &headers).await {
        return refused;
    }
    match crate::chief::traces(cx.rosters.pool(), &site, 20).await {
        Ok(list) => Json(json!({ "traces": list })).into_response(),
        Err(e) => {
            tracing::warn!(%e, "traces read failed");
            refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't read the Chief's activity. Try again.",
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct MarkBody {
    webagent: String,
    mark: agentrix_marks::MarkSpec,
}

async fn save_mark(
    State(cx): State<Ctx>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<MarkBody>,
) -> Response {
    if let Some(refused) = cross_origin(&cx, &headers) {
        return refused;
    }
    let account = match owner_and_roster(&cx, &site, &headers).await {
        Ok((a, _)) => a,
        Err(refused) => return refused,
    };
    match cx
        .rosters
        .save_mark(
            &site,
            &body.webagent,
            body.mark,
            account,
            cx.receipts.as_ref(),
        )
        .await
    {
        Ok(receipt) => Json(json!({ "saved": body.webagent, "receipt": receipt })).into_response(),
        Err(MarkSaveError::NotFound(m)) => refuse(StatusCode::NOT_FOUND, &m),
        Err(MarkSaveError::NotAllowed {
            message,
            suggestion,
        }) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": message, "suggestion": suggestion })),
        )
            .into_response(),
        Err(MarkSaveError::Other(e)) => {
            tracing::warn!(error = %format!("{e:#}"), "saving a mark failed");
            refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't save that look. Nothing changed. Try again.",
            )
        }
    }
}

async fn signals(State(cx): State<Ctx>, Path(site): Path<String>, headers: HeaderMap) -> Response {
    if let Err(refused) = owned_roster(&cx, &site, &headers).await {
        return refused;
    }
    match cx.rosters.signals(&site, false, 100).await {
        Ok(list) => Json(json!({ "signals": list })).into_response(),
        Err(e) => {
            tracing::warn!(%e, "signals read failed");
            refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't read your signals. Try again.",
            )
        }
    }
}

#[derive(serde::Deserialize)]
struct DialBody {
    webagent: String,
    job: String,
    level: Level,
}

fn dial_refusal(e: DialError) -> Response {
    let status = match &e {
        DialError::NotFound(_) => StatusCode::NOT_FOUND,
        DialError::NotAllowed(_) | DialError::NotWaiting(_) => StatusCode::CONFLICT,
        DialError::NotApproved => StatusCode::FORBIDDEN,
        DialError::Other(err) => {
            tracing::warn!(error = %format!("{err:#}"), "dial change failed");
            return refuse(
                StatusCode::SERVICE_UNAVAILABLE,
                "We couldn't change that dial. Nothing changed. Try again.",
            );
        }
    };
    refuse(status, &e.to_string())
}

async fn dials_pending(
    State(cx): State<Ctx>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(refused) = owned_roster(&cx, &site, &headers).await {
        return refused;
    }
    match cx.dials.pending(&site).await {
        Ok(p) => Json(json!({ "pending": p })).into_response(),
        Err(e) => dial_refusal(DialError::Other(e)),
    }
}

async fn dial_request(
    State(cx): State<Ctx>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DialBody>,
) -> Response {
    if let Some(refused) = cross_origin(&cx, &headers) {
        return refused;
    }
    let account = match owner_and_roster(&cx, &site, &headers).await {
        Ok((a, _)) => a,
        Err(refused) => return refused,
    };
    match cx
        .dials
        .request(&site, account, &body.webagent, &body.job, body.level)
        .await
    {
        Ok(out) => Json(out).into_response(),
        Err(e) => dial_refusal(e),
    }
}

async fn dial_approve(
    State(cx): State<Ctx>,
    Path((site, change)): Path<(String, String)>,
    headers: HeaderMap,
    Json(proof): Json<serde_json::Value>,
) -> Response {
    if let Some(refused) = cross_origin(&cx, &headers) {
        return refused;
    }
    let account = match owner_and_roster(&cx, &site, &headers).await {
        Ok((a, _)) => a,
        Err(refused) => return refused,
    };
    match cx.dials.approve(&site, account, &change, proof).await {
        Ok((level, receipt)) => Json(json!({ "level": level, "receipt": receipt })).into_response(),
        Err(e) => dial_refusal(e),
    }
}

async fn dial_decline(
    State(cx): State<Ctx>,
    Path((site, change)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if let Some(refused) = cross_origin(&cx, &headers) {
        return refused;
    }
    if let Err(refused) = owned_roster(&cx, &site, &headers).await {
        return refused;
    }
    match cx.dials.decline(&site, &change).await {
        Ok(()) => Json(json!({ "declined": change })).into_response(),
        Err(e) => dial_refusal(e),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agentrix_host::testing::FixedOwner;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn get(app: &Router, path: &str) -> (StatusCode, serde_json::Value) {
        let res = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    use super::*;

    #[tokio::test]
    async fn agentrix_website_shows_its_team_live_to_anyone_and_no_other_site() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        crate::home::seed(&rosters).await.unwrap();
        rosters
            .seed_sample("kilele-sample", "kilele")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE gauntlet_runs (run_id TEXT PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT)")
            .execute(&ctx.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO gauntlet_runs VALUES ('g1', ?, NULL)")
            .bind(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
            .execute(&ctx.pool)
            .await
            .unwrap();
        let hub = Presence::new();
        hub.publish(
            crate::home::SITE,
            "coordinator",
            MarkState::Working,
            Some("Reading the catalogue"),
            None,
        );
        let app = platform(Ctx {
            rosters: rosters.clone(),
            owners: Arc::new(FixedOwner::nobody()),
            presence: hub.clone(),
            dials: Dials::new(rosters.clone(), ctx.receipts.clone(), ctx.approvals.clone()),
            receipts: ctx.receipts.clone(),
            node: ctx.clone(),
            origin: "http://localhost:8080".into(),
        });
        assert_eq!(
            get(&app, "/api/webagents/kilele-sample/live").await.0,
            StatusCode::NOT_FOUND,
            "a site's own presence stays its owner's"
        );
        let res = app
            .clone()
            .oneshot(
                Request::get("/api/webagents/agentrix-home/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "no sign-in needed");
        let mut body = res.into_body().into_data_stream();
        let chunk = body.next().await.unwrap().unwrap();
        let text = String::from_utf8_lossy(&chunk);
        let data = text.lines().find_map(|l| l.strip_prefix("data: ")).unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(data).unwrap();
        let frames = snapshot["frames"].as_array().unwrap();
        let of = |id: &str| frames.iter().find(|f| f["webagent"] == id).unwrap().clone();
        assert_eq!(of("guardian")["words"], "Running the gauntlet");
        assert_eq!(of("desk-your-site")["state"], "idle");
        let chief = of("coordinator");
        assert!(
            chief["label"].is_null(),
            "the Chief's own work stays private"
        );
        assert_eq!(
            of("sales")["state"],
            "idle",
            "asleep reads as still to visitors"
        );
    }

    #[tokio::test]
    async fn only_the_sites_owner_reads_its_roster() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        sqlx::query("CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT, name TEXT, owner_account_id TEXT)")
            .execute(&ctx.pool)
            .await
            .unwrap();
        let wanjiru = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO sites VALUES ('kilele-sample', 'kilele-bakery-sample.agentrix.website', 'Kilele Bakery', ?)")
            .bind(wanjiru.to_string())
            .execute(&ctx.pool)
            .await
            .unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        rosters
            .seed_sample("kilele-sample", "kilele")
            .await
            .unwrap();

        let hub = Presence::new();
        let cx = |owner: FixedOwner| {
            platform(Ctx {
                rosters: rosters.clone(),
                owners: Arc::new(owner),
                presence: hub.clone(),
                dials: Dials::new(rosters.clone(), ctx.receipts.clone(), ctx.approvals.clone()),
                receipts: ctx.receipts.clone(),
                node: ctx.clone(),
                origin: "http://localhost:8080".into(),
            })
        };
        let nobody = cx(FixedOwner::nobody());
        assert_eq!(
            get(&nobody, "/api/webagents/kilele-sample").await.0,
            StatusCode::UNAUTHORIZED
        );

        let stranger = cx(FixedOwner::passkey(uuid::Uuid::new_v4()));
        assert_eq!(
            get(&stranger, "/api/webagents/kilele-sample").await.0,
            StatusCode::NOT_FOUND
        );

        let owner = cx(FixedOwner::passkey(wanjiru));
        let (status, body) = get(&owner, "/api/webagents/kilele-sample").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["members"].as_array().unwrap().len(), 9);
        assert_eq!(body["members"][0]["name"], "Chief WebAgent");
        assert!(body["team_svg"].as_str().unwrap().contains("wa chief"));
        assert!(
            body["members"][1]["svg"]
                .as_str()
                .unwrap()
                .starts_with("<svg class=\"wa\"")
        );
    }

    async fn post(
        app: &Router,
        path: &str,
        origin: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let res = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("content-type", "application/json")
                    .header("origin", origin)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn the_owner_moves_a_dial_and_a_raise_needs_approval() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        rosters.seed_sample("kilele", "kilele").await.unwrap();
        let wanjiru = uuid::Uuid::new_v4();
        rosters.set_owner("kilele", wanjiru).await.unwrap();
        let app = platform(Ctx {
            rosters: rosters.clone(),
            owners: Arc::new(FixedOwner::passkey(wanjiru)),
            presence: Presence::new(),
            dials: Dials::new(rosters.clone(), ctx.receipts.clone(), ctx.approvals.clone()),
            receipts: ctx.receipts.clone(),
            node: ctx.clone(),
            origin: "http://localhost:8080".into(),
        });
        let me = "http://localhost:8080";

        // Another site can't move a dial.
        let (status, body) = post(
            &app,
            "/api/webagents/kilele/dials",
            "https://evil.example",
            json!({"webagent": "editor", "job": "freshness", "level": "do-and-tell"}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(
            body["error"]
                .as_str()
                .is_some_and(|e| e.contains("another site")),
            "a refused POST says why: {body}"
        );

        // Above the ceiling is refused with the reason.
        let (status, body) = post(
            &app,
            "/api/webagents/kilele/dials",
            me,
            json!({"webagent": "cashier", "job": "payments", "level": "do-and-tell"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().unwrap().contains("Money"));

        // A raise comes back as a card to approve.
        let (status, body) = post(
            &app,
            "/api/webagents/kilele/dials",
            me,
            json!({"webagent": "editor", "job": "freshness", "level": "do-and-tell"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["outcome"], "needs-approval");
        let change = body["change"]["change_id"].as_str().unwrap().to_string();
        let (_, pending) = get(&app, "/api/webagents/kilele/dials").await;
        assert_eq!(pending["pending"].as_array().unwrap().len(), 1);
        let (status, body) = post(
            &app,
            &format!("/api/webagents/kilele/dials/{change}/approve"),
            me,
            json!({"totp": "123456"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["level"], "do-and-tell");

        // The owner reads what the desks told the back office.
        rosters
            .emit(
                "kilele",
                "desk-cakes",
                awp_types::SignalKind::ContentGap {
                    topic: crate::signals::bounded("gluten-free cakes", 120),
                    question_count: 1,
                    samples: vec![],
                },
            )
            .await
            .unwrap();
        let (status, body) = get(&app, "/api/webagents/kilele/signals").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["signals"][0]["signal"]["kind"]["type"], "content_gap");
        assert_eq!(body["signals"][0]["wakes"][0], "editor");

        // Lowering applies at once.
        let (status, body) = post(
            &app,
            "/api/webagents/kilele/dials",
            me,
            json!({"webagent": "editor", "job": "freshness", "level": "watch"}),
        )
        .await;
        assert_eq!(
            (status, body["outcome"].as_str()),
            (StatusCode::OK, Some("applied"))
        );
    }
}
