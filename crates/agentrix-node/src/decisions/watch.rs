//! Watching a Move (R6.5; design §4.1, §4.3a), owner only:
//!
//! - `GET /moves/{id}`: the Move's current state as JSON (`Accept: application/json`), or a 303
//!   to the console's Move page.
//! - `GET /moves/{id}/events`: SSE. Every event has `id:` = its `seq`. With `Last-Event-ID`,
//!   the stored events after it are replayed; without one, a `snapshot` aligns the page that
//!   was just rendered, then live events follow. `: ping` every 15 s. The stream ends after
//!   `done`.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use agentrix_move::MoveStore;
use agentrix_move::store::{MoveRecord, StepRecord};
use agentrix_types::ui::{ParityEvent, SnapshotEvent, StepEvent, TickEvent};
use agentrix_types::{MoveId, UiEnvelope, UiEvent};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::Utc;
use futures::stream::{self, BoxStream, StreamExt as _};
use serde_json::json;
use tokio::sync::broadcast;

use super::{Sessions, wants_json};
use crate::launch::ports::MoveEvents;

/// What the watch routes need.
#[derive(Clone)]
pub struct WatchState {
    /// The move lane's tables.
    pub store: MoveStore,
    /// The Moves' events.
    pub events: Arc<MoveEvents>,
    /// Who is signed in.
    pub sessions: Arc<dyn Sessions>,
}

/// The routes, for `app.rs` to merge into the platform router.
pub fn router(state: WatchState) -> Router {
    Router::new()
        .route("/moves/{id}", get(show))
        .route("/moves/{id}/events", get(events))
        .with_state(state)
}

fn plain(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "message": message }))).into_response()
}

impl WatchState {
    /// The Move, if the signed-in account owns its site.
    async fn owned(&self, headers: &HeaderMap, id: &MoveId) -> Result<MoveRecord, Response> {
        let Some(account) = self.sessions.account(headers).await else {
            return Err(plain(StatusCode::UNAUTHORIZED, "Sign in first."));
        };
        let found = self.store.move_record(id).await.map_err(|e| {
            tracing::error!(error = %format!("{e:#}"), "reading a Move failed");
            plain(
                StatusCode::INTERNAL_SERVER_ERROR,
                "This Move couldn't be read just now.",
            )
        })?;
        let Some(mv) = found else {
            return Err(plain(StatusCode::NOT_FOUND, "We couldn't find that Move."));
        };
        let owner = self
            .store
            .site(&mv.site)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.owner);
        if owner.as_deref() != Some(account.as_str()) {
            return Err(plain(StatusCode::NOT_FOUND, "We couldn't find that Move."));
        }
        Ok(mv)
    }

    /// Each step's latest row, as the `step` events said it.
    async fn steps(&self, id: &MoveId) -> Vec<StepEvent> {
        let rows = self.store.steps(id).await.unwrap_or_default();
        let mut latest: Vec<StepRecord> = Vec::new();
        for row in rows {
            latest.retain(|r| r.step != row.step);
            latest.push(row);
        }
        latest
            .into_iter()
            .map(|r| {
                let ended = r.ended_at.unwrap_or_else(Utc::now);
                let elapsed = u64::try_from((ended - r.started_at).num_milliseconds()).unwrap_or(0);
                let mut event = StepEvent::new(r.step, r.state, elapsed, &r.summary, ended);
                event.receipt = r.receipt;
                event
            })
            .collect()
    }

    /// The latest site-wide parity event.
    async fn parity(&self, id: &MoveId) -> Option<ParityEvent> {
        self.events
            .replay(id, 0)
            .await
            .unwrap_or_default()
            .into_iter()
            .rev()
            .find_map(|e| match e.event {
                UiEvent::Parity(p) if p.page.is_none() => Some(p),
                _ => None,
            })
    }
}

async fn show(State(s): State<WatchState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let id = MoveId::new(id);
    let mv = match s.owned(&headers, &id).await {
        Ok(mv) => mv,
        Err(r) => return r,
    };
    if !wants_json(&headers) {
        return Redirect::to("/console/move").into_response();
    }
    let steps = s.steps(&id).await;
    let parity = s.parity(&id).await;
    let card = s
        .store
        .cards(&mv.site)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|c| c.card.effect.args.get("move").and_then(|m| m.as_str()) == Some(id.as_str()))
        .map(|c| json!({ "decision_id": c.card.decision_id, "title": c.card.title, "href": format!("/console/{}/decisions/{}", mv.site, c.card.decision_id) }));
    axum::Json(json!({
        "move_id": mv.id,
        "site": mv.site,
        "state": mv.state,
        "started_at": mv.created_at,
        "updated_at": mv.updated_at,
        "steps": steps,
        "parity": parity,
        "card": card,
    }))
    .into_response()
}

fn sse(env: &UiEnvelope) -> Event {
    Event::default()
        .id(env.seq.to_string())
        .event(env.event.name())
        .data(env.event.data_json().unwrap_or_else(|_| "{}".into()))
}

struct Tail {
    id: MoveId,
    rx: broadcast::Receiver<(MoveId, UiEnvelope)>,
    pending: VecDeque<UiEnvelope>,
    cursor: u64,
    events: Arc<MoveEvents>,
    finished: bool,
}

async fn events(
    State(s): State<WatchState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let id = MoveId::new(id);
    if let Err(r) = s.owned(&headers, &id).await {
        return r;
    }
    // Subscribe before reading the store, so no event falls between the two.
    let rx = s.events.subscribe();
    let last: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok());
    let stored = s
        .events
        .replay(&id, last.unwrap_or(0))
        .await
        .unwrap_or_default();
    let mut pending = VecDeque::new();
    let cursor;
    match last {
        Some(after) => {
            cursor = stored.last().map_or(after, |e| e.seq);
            pending.extend(stored);
        }
        None => {
            // A fresh page rendered the current state: align it, and never replay an old
            // `done`, which would send the owner away from the page they just opened.
            cursor = stored.last().map_or(0, |e| e.seq);
            pending.push_back(UiEnvelope {
                seq: cursor,
                event: UiEvent::Snapshot(SnapshotEvent {
                    steps: s.steps(&id).await,
                    tick: TickEvent {
                        job: None,
                        hosts: 0,
                        requests: 0,
                        bytes: 0,
                        refusals: 0,
                        at: Utc::now(),
                    },
                    last_frame: None,
                    parity: s.parity(&id).await,
                    at: Utc::now(),
                }),
            });
        }
    }
    let tail = Tail {
        id,
        rx,
        pending,
        cursor,
        events: s.events.clone(),
        finished: false,
    };
    let body: BoxStream<'static, Result<Event, Infallible>> =
        stream::unfold(tail, |mut t| async move {
            loop {
                if let Some(env) = t.pending.pop_front() {
                    if matches!(env.event, UiEvent::Done(_)) {
                        t.finished = true;
                        t.pending.clear();
                    }
                    return Some((Ok(sse(&env)), t));
                }
                if t.finished {
                    return None;
                }
                match t.rx.recv().await {
                    Ok((id, env)) if id == t.id && env.seq > t.cursor => {
                        t.cursor = env.seq;
                        t.pending.push_back(env);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let missed = t.events.replay(&t.id, t.cursor).await.unwrap_or_default();
                        if let Some(last) = missed.last() {
                            t.cursor = last.seq;
                        }
                        t.pending.extend(missed);
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed();
    Sse::new(body)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}
