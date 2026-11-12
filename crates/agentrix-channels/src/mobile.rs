//! The owner app's JSON API, `/api/m/v1/…` on the platform host (13 §9.1).
//!
//! - **Pairing, not passwords.** A signed-in owner taps Pair a phone in Settings → Channels and
//!   gets an eight-character code (ten minutes, single use, hashed). The app sends it to
//!   `POST /api/m/v1/pair` and gets a bearer token bound to that owner and site; only the
//!   token's SHA-256 is kept. Settings lists paired phones with Remove; 30 days unused ends one.
//! - **A token never approves on its own.** Approving sends a code from the owner's authenticator
//!   app, verified by node's approvals against the card's fingerprint through W5's decision
//!   contract, exactly as the console does. Passkey owners approve in the console (`/d/{card}`)
//!   until the app's associated domain is on the live host. A stolen token can decline and
//!   freeze, never move money.
//! - Every answer is JSON; errors are `{"message": "<words for the owner>"}`.

use std::sync::{Arc, Mutex};

use agentrix_decisions::{ApprovalKind, Attempt, CardState, RiskLevel};
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use chrono::{DateTime, Duration, FixedOffset, Timelike, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::service::{CardView, Channels, clock};
use crate::store::Device;

/// Failed pairings allowed a minute, across all callers.
const PAIR_FAILURES_PER_MINUTE: usize = 20;

#[derive(Clone)]
struct Api {
    channels: Channels,
    failures: Arc<Mutex<Vec<DateTime<Utc>>>>,
}

/// The routes.
pub fn router(channels: Channels) -> Router {
    let api = Api {
        channels,
        failures: Arc::default(),
    };
    Router::new()
        .route("/api/m/v1/pair", post(pair).options(preflight))
        .route("/api/m/v1/me", get(me).options(preflight))
        .route("/api/m/v1/now", get(now).options(preflight))
        .route("/api/m/v1/cards", get(cards).options(preflight))
        .route("/api/m/v1/cards/{id}", get(card).options(preflight))
        .route(
            "/api/m/v1/cards/{id}/approve",
            post(approve).options(preflight),
        )
        .route(
            "/api/m/v1/cards/{id}/decline",
            post(decline).options(preflight),
        )
        .route("/api/m/v1/freeze", post(freeze).options(preflight))
        .route("/api/m/v1/ask", post(ask).options(preflight))
        .route("/api/m/v1/webagents", get(webagents).options(preflight))
        .route("/api/m/v1/live", get(live).options(preflight))
        .route(
            "/api/m/v1/device",
            delete(remove_this_device).options(preflight),
        )
        .with_state(api)
}

/// An answer with plain words and a status.
struct Fail(StatusCode, String);

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "message": self.1 }))).into_response()
    }
}

fn fail(status: StatusCode, words: impl Into<String>) -> Fail {
    Fail(status, words.into())
}

fn internal(e: impl std::fmt::Display) -> Fail {
    tracing::error!(error = %e, "owner app API failed");
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Something on our side failed. Nothing changed. Try again in a minute.",
    )
}

/// Browsers only ever call this API from a local development build of the app (a Flutter web
/// build on `localhost`), and only when node itself runs on `localhost`. On a live host the
/// API answers no browser origin: the apps are native and send a bearer token, not cookies.
fn cors(api: &Api, headers: &HeaderMap, mut res: Response) -> Response {
    let local_node = matches!(
        api.channels.ctx().origin.host_str(),
        Some("localhost" | "127.0.0.1")
    );
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if let Some(o) = origin
        && local_node
        && url::Url::parse(o)
            .ok()
            .is_some_and(|u| matches!(u.host_str(), Some("localhost" | "127.0.0.1")))
        && let Ok(v) = HeaderValue::from_str(o)
    {
        let h = res.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    res
}

async fn preflight(State(api): State<Api>, headers: HeaderMap) -> Response {
    cors(&api, &headers, StatusCode::NO_CONTENT.into_response())
}

fn reply(api: &Api, headers: &HeaderMap, result: Result<Value, Fail>) -> Response {
    let res = match result {
        Ok(v) => Json(v).into_response(),
        Err(f) => f.into_response(),
    };
    cors(api, headers, res)
}

/// The paired device behind the request's bearer token, still owning its site.
async fn device(api: &Api, headers: &HeaderMap) -> Result<Device, Fail> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            fail(
                StatusCode::UNAUTHORIZED,
                "Pair this phone first: Settings → Channels → Pair a phone.",
            )
        })?;
    let dev = api
        .channels
        .store()
        .device_for_token(token, Utc::now())
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            fail(
                StatusCode::UNAUTHORIZED,
                "This phone isn't paired any more. Pair it again in Settings → Channels.",
            )
        })?;
    if !api.channels.owns(&dev.site_id, &dev.account_id).await {
        return Err(fail(
            StatusCode::FORBIDDEN,
            "This site isn't yours any more, so the phone can't open it.",
        ));
    }
    Ok(dev)
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
    #[serde(default)]
    device: String,
}

async fn pair(State(api): State<Api>, headers: HeaderMap, Json(body): Json<PairBody>) -> Response {
    let result = async {
        let now = Utc::now();
        {
            let mut f = api.failures.lock().unwrap_or_else(|e| e.into_inner());
            f.retain(|t| *t > now - Duration::minutes(1));
            if f.len() >= PAIR_FAILURES_PER_MINUTE {
                return Err(fail(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many wrong codes in the last minute. Wait a minute and try again.",
                ));
            }
        }
        let ch = &api.channels;
        let taken = ch
            .store()
            .take_link_code("app", &body.code, now)
            .await
            .map_err(internal)?;
        let Some((account, site)) = taken else {
            api.failures
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(now);
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "That code didn't work. Codes last 10 minutes and work once. Make a new one in Settings → Channels.",
            ));
        };
        let (dev, token) = ch
            .store()
            .add_device(&account, &site, &body.device, now)
            .await
            .map_err(internal)?;
        let info = ch.site(&site).await;
        Ok(json!({
            "token": token,
            "device": { "id": dev.id, "name": dev.name },
            "owner": { "name": ch.owner_first_name(&account).await },
            "site": site_json(ch, &info),
            "origin": ch.origin(),
        }))
    }
    .await;
    reply(&api, &headers, result)
}

fn site_json(ch: &Channels, s: &crate::service::SiteInfo) -> Value {
    json!({ "id": s.id, "name": s.name, "host": s.host, "sample": s.sample, "url": ch.site_origin(&s.host) })
}

async fn me(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let info = ch.site(&dev.site_id).await;
        let approvals = approval_methods(ch, &dev.account_id).await;
        Ok(json!({
            "owner": { "name": ch.owner_first_name(&dev.account_id).await },
            "site": site_json(ch, &info),
            "device": { "id": dev.id, "name": dev.name },
            "approvals": approvals,
        }))
    }
    .await;
    reply(&api, &headers, result)
}

/// How this owner can approve from the app: `code` (an authenticator app) and/or `passkey`
/// (in the console until the app's associated domain is live).
async fn approval_methods(ch: &Channels, account: &str) -> Value {
    let email = ch
        .messages()
        .directory()
        .owner(account)
        .await
        .ok()
        .flatten()
        .map(|c| c.email);
    let acct = match email {
        Some(e) => ch.ctx().owner_accounts.by_email(&e).await.ok().flatten(),
        None => None,
    };
    json!({
        "code": acct.as_ref().is_some_and(|a| a.has_password),
        "passkey": acct.as_ref().is_some_and(|a| a.has_passkey),
    })
}

fn greeting(at: DateTime<Utc>, offset_min: i32, name: Option<&str>) -> String {
    let local = at.with_timezone(
        &FixedOffset::east_opt(offset_min * 60)
            .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset")),
    );
    let part = match local.hour() {
        0..=11 => "Morning",
        12..=16 => "Afternoon",
        _ => "Evening",
    };
    match name {
        Some(n) => format!("{part}, {n}"),
        None => part.to_owned(),
    }
}

fn risk_words(level: &str) -> &'static str {
    match level {
        "low" => "Low risk",
        "medium" => "Medium risk",
        _ => "High risk",
    }
}

fn card_brief(ch: &Channels, c: &CardView, offset: i32) -> Value {
    json!({
        "id": c.id,
        "title": c.title,
        "agent": c.agent,
        "risk": c.risk,
        "risk_words": risk_words(&c.risk),
        "money": c.money,
        "since": clock(c.created_at, offset),
        "v2": c.v2,
        "url": ch.card_url(&c.id),
    })
}

async fn now(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let info = ch.site(&dev.site_id).await;
        let name = ch.owner_first_name(&dev.account_id).await;
        let cards = ch.waiting_cards(&dev.site_id).await;
        let brief = ch.brief_json(&dev.site_id).await;
        let quiet = ch.shell().quiet_hours(&dev.site_id).await.unwrap_or_default();
        let at = Utc::now();
        let local = at
            .with_timezone(
                &FixedOffset::east_opt(info.offset_min * 60)
                    .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset")),
            )
            .time();
        let night = if quiet.start < quiet.end {
            local >= quiet.start && local < quiet.end
        } else {
            local >= quiet.start || local < quiet.end
        };
        let frozen = match ch.shell().freeze(&dev.site_id).await.ok().flatten() {
            Some(f) => json!({ "at": clock(f.at, info.offset_min), "how": f.how }),
            None => Value::Null,
        };
        let n = cards.len();
        Ok(json!({
            "greeting": greeting(at, info.offset_min, name.as_deref()),
            "site": site_json(ch, &info),
            "night": night,
            "quiet": { "start": quiet.start.format("%H:%M").to_string(), "end": quiet.end.format("%H:%M").to_string() },
            "brief": brief,
            "waiting": n,
            "needs_you": cards.iter().take(5).map(|c| card_brief(ch, c, info.offset_min)).collect::<Vec<_>>(),
            "frozen": frozen,
            "freeze_enforced": ch.settings().freeze_enforced,
            // The doorbell's words for a local notification: never what a card is about.
            "doorbell": match n {
                0 => Value::Null,
                1 => json!("A decision is waiting."),
                n => json!(format!("{n} decisions are waiting.")),
            },
        }))
    }
    .await;
    reply(&api, &headers, result)
}

async fn cards(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let info = ch.site(&dev.site_id).await;
        let cards = ch.waiting_cards(&dev.site_id).await;
        Ok(json!({ "cards": cards.iter().map(|c| card_brief(ch, c, info.offset_min)).collect::<Vec<_>>() }))
    }
    .await;
    reply(&api, &headers, result)
}

async fn card(State(api): State<Api>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let info = ch.site(&dev.site_id).await;
        let approvals = approval_methods(ch, &dev.account_id).await;
        if let Some(row) = ch
            .decisions()
            .get(&dev.site_id, &id)
            .await
            .map_err(internal)?
        {
            let c = &row.card;
            let level = match c.risk.level {
                RiskLevel::Low => "low",
                RiskLevel::Medium => "medium",
                RiskLevel::High => "high",
            };
            let fp = c.fingerprint.to_string();
            let short = fp
                .trim_start_matches("sha256:")
                .get(..8)
                .unwrap_or("")
                .to_owned();
            return Ok(json!({
                "id": c.id,
                "v2": true,
                "title": c.title,
                "summary": c.summary,
                "effects": c.effects,
                "agent": crate::service::agent_words(&c.raised_by.webagent),
                "risk": level,
                "risk_words": risk_words(level),
                "reasons": c.risk.reasons,
                "money": c.effect.amount.is_some(),
                "approval": match c.approval { ApprovalKind::Reviewed => "reviewed", ApprovalKind::OneTap => "one-tap" },
                "since": clock(c.created_at, info.offset_min),
                "expires": clock(c.expires_at(), info.offset_min),
                "state": match row.state { CardState::Waiting => "waiting", CardState::Approved => "approved", CardState::Declined => "declined", CardState::Expired => "expired" },
                "fingerprint": fp,
                "fingerprint_short": short,
                "receipt": row.receipt,
                "url": ch.card_url(&c.id),
                "can": {
                    "approve_with_code": approvals["code"].as_bool().unwrap_or(false),
                    "approve_in_console": true,
                    "decline": true,
                },
            }));
        }
        let waiting = ch.waiting_cards(&dev.site_id).await;
        let Some(c) = waiting.iter().find(|c| c.id == id) else {
            return Err(fail(StatusCode::NOT_FOUND, "We couldn't find that decision. Your Decisions list shows what's still waiting."));
        };
        Ok(json!({
            "id": c.id, "v2": false, "title": c.title, "summary": "", "effects": [],
            "agent": c.agent, "risk": c.risk, "risk_words": risk_words(&c.risk), "reasons": [],
            "money": c.money, "approval": "reviewed", "since": clock(c.created_at, info.offset_min),
            "state": "waiting", "url": ch.card_url(&c.id),
            "can": { "approve_with_code": false, "approve_in_console": true, "decline": false },
        }))
    }
    .await;
    reply(&api, &headers, result)
}

#[derive(Deserialize)]
struct ApproveBody {
    code: String,
}

async fn approve(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let row = ch
            .decisions()
            .get(&dev.site_id, &id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                fail(
                    StatusCode::NOT_FOUND,
                    "You approve that one in the console. Open it there.",
                )
            })?;
        let code: String = body.code.chars().filter(char::is_ascii_digit).collect();
        if code.len() != 6 {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Type the 6 digits from your authenticator app.",
            ));
        }
        let fp = row.card.fingerprint.to_string();
        ch.decisions()
            .approve(&id, &dev.account_id, Attempt::Totp(code), Some(&fp))
            .await
            .map_err(|r| {
                fail(
                    StatusCode::from_u16(r.status.as_u16()).unwrap_or(StatusCode::CONFLICT),
                    r.words,
                )
            })?;
        let after = ch
            .decisions()
            .get(&dev.site_id, &id)
            .await
            .map_err(internal)?;
        Ok(json!({
            "state": "approved",
            "receipt": after.and_then(|r| r.receipt),
            "message": format!("Approved \"{}\" with your authenticator code. It runs once, and the receipt records how you approved it.", row.card.title),
        }))
    }
    .await;
    reply(&api, &headers, result)
}

async fn decline(State(api): State<Api>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let waiting = ch.waiting_cards(&dev.site_id).await;
        if waiting.iter().any(|c| c.id == id && !c.v2) {
            return Err(fail(
                StatusCode::CONFLICT,
                "You decide this one in the console. Open it there.",
            ));
        }
        ch.decisions()
            .decline(&id, &dev.account_id, "app")
            .await
            .map_err(|r| {
                fail(
                    StatusCode::from_u16(r.status.as_u16()).unwrap_or(StatusCode::CONFLICT),
                    r.words,
                )
            })?;
        Ok(json!({ "state": "declined", "message": "Declined. Nothing changed." }))
    }
    .await;
    reply(&api, &headers, result)
}

async fn freeze(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        if !ch.settings().freeze_enforced {
            return Err(fail(
                StatusCode::CONFLICT,
                "Freeze isn't switched on for this site yet, so nothing was stopped.",
            ));
        }
        ch.shell()
            .set_frozen(&dev.site_id, &dev.account_id, "app", Utc::now())
            .await
            .map_err(internal)?;
        Ok(json!({ "frozen": true, "message": "Every WebAgent is frozen. Your site keeps serving its signed release. Unfreezing needs your passkey or authenticator code in the console." }))
    }
    .await;
    reply(&api, &headers, result)
}

#[derive(Deserialize)]
struct AskBody {
    said: String,
}

/// What the owner typed or said to the Chief WebAgent: the same grammar as voice and chat
/// (`agentrix_media::intent`). It never approves; "stop everything" freezes at once.
async fn ask(State(api): State<Api>, headers: HeaderMap, Json(body): Json<AskBody>) -> Response {
    use agentrix_media::intent::{self, Intent};
    let result = async {
        let dev = device(&api, &headers).await?;
        let ch = &api.channels;
        let said: String = body.said.trim().chars().take(600).collect();
        let heard = intent::read(&said);
        let (reply, action) = match heard {
            Intent::Stop => {
                if ch.settings().freeze_enforced {
                    ch.shell()
                        .set_frozen(&dev.site_id, &dev.account_id, "app", Utc::now())
                        .await
                        .map_err(internal)?;
                    (
                        "Every WebAgent is frozen now. Your site keeps serving its signed release."
                            .to_owned(),
                        json!({ "do": "frozen" }),
                    )
                } else {
                    (
                        "Freeze isn't switched on for this site yet, so nothing was stopped."
                            .to_owned(),
                        Value::Null,
                    )
                }
            }
            Intent::AllowOnce => {
                let first = ch.waiting_cards(&dev.site_id).await.into_iter().next();
                match first {
                    Some(c) => (
                        format!(
                            "Approving \"{}\" needs your passkey or an authenticator code, so I've opened it.",
                            c.title
                        ),
                        json!({ "do": "open_card", "card": c.id }),
                    ),
                    None => (
                        "Nothing is waiting for you to decide.".to_owned(),
                        Value::Null,
                    ),
                }
            }
            Intent::Brief => {
                let b = ch.brief_json(&dev.site_id).await;
                let mut words = vec![b["headline"].as_str().unwrap_or_default().to_owned()];
                for l in b["lines"].as_array().into_iter().flatten() {
                    words.push(l.as_str().unwrap_or_default().to_owned());
                }
                (words.join(" "), Value::Null)
            }
            Intent::Next => {
                let cards = ch.waiting_cards(&dev.site_id).await;
                match cards.first() {
                    Some(c) => (
                        format!(
                            "{} asks: {}. {}. Open it to approve, or decline it.",
                            c.agent,
                            c.title,
                            risk_words(&c.risk)
                        ),
                        json!({ "do": "open_card", "card": c.id }),
                    ),
                    None => ("Nothing needs you right now.".to_owned(), Value::Null),
                }
            }
            Intent::Deny | Intent::Cancel => (
                "Open the decision and tap Decline, so it's clear which one you mean.".to_owned(),
                Value::Null,
            ),
            Intent::Ask => (
                crate::chief::answer(ch, &dev.site_id, &said).await,
                Value::Null,
            ),
        };
        Ok(json!({ "heard": said, "intent": heard, "reply": reply, "action": action }))
    }
    .await;
    reply(&api, &headers, result)
}

async fn webagents(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ctx = api.channels.ctx();
        let rosters = agentrix_kernel::Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        let roster = rosters.roster(&dev.site_id).await.map_err(internal)?;
        let waiting = api.channels.waiting_cards(&dev.site_id).await;
        let members: Vec<Value> = roster
            .map(|r| {
                r.members
                    .iter()
                    .map(|m| {
                        let id = m.spec.id.clone();
                        let badge = waiting
                            .iter()
                            .filter(|c| {
                                c.agent
                                    .eq_ignore_ascii_case(&crate::service::agent_words(&id))
                            })
                            .count();
                        json!({
                            "id": id,
                            "name": m.spec.name,
                            "role": m.spec.role_words,
                            "status": m.status.words(),
                            "on_duty": matches!(m.status, agentrix_kernel::Status::OnDuty),
                            "mark": m.spec.mark,
                            "badge": badge,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(json!({ "webagents": members }))
    }
    .await;
    reply(&api, &headers, result)
}

/// Live (07 §4.3): every desk and back-office WebAgent's state now, from W1's presence hub, in
/// the owner's words ("Answering a visitor", "Resting").
async fn live(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        let ctx = api.channels.ctx();
        let rosters = agentrix_kernel::Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        let Some(roster) = rosters.roster(&dev.site_id).await.map_err(internal)? else {
            return Ok(json!({ "frames": [], "busy": 0 }));
        };
        let hub = agentrix_kernel::presence::global();
        let _ = agentrix_kernel::presence::baseline(hub, &ctx.pool, &dev.site_id).await;
        let names: std::collections::HashMap<String, String> = roster
            .members
            .iter()
            .map(|m| (m.spec.id.clone(), m.spec.name.clone()))
            .chain(
                roster
                    .desks()
                    .iter()
                    .map(|d| (d.id.clone(), d.name.clone())),
            )
            .collect();
        let frames: Vec<Value> = hub
            .snapshot(&roster)
            .into_iter()
            .map(|f| {
                let state = serde_json::to_value(f.state).unwrap_or(Value::Null);
                json!({
                    "id": f.webagent,
                    "name": names.get(&f.webagent).cloned().unwrap_or_else(|| f.webagent.clone()),
                    "state": state,
                    "words": f.words,
                    "label": f.label,
                })
            })
            .collect();
        let busy = frames
            .iter()
            .filter(|f| !matches!(f["state"].as_str(), Some("idle" | "asleep" | "resting")))
            .count();
        Ok(json!({ "frames": frames, "busy": busy }))
    }
    .await;
    reply(&api, &headers, result)
}

async fn remove_this_device(State(api): State<Api>, headers: HeaderMap) -> Response {
    let result = async {
        let dev = device(&api, &headers).await?;
        api.channels
            .store()
            .remove_device(&dev.id, &dev.account_id, Utc::now())
            .await
            .map_err(internal)?;
        Ok(json!({ "removed": true }))
    }
    .await;
    reply(&api, &headers, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greetings_follow_the_site_s_clock() {
        let t = DateTime::parse_from_rfc3339("2026-11-08T04:01:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(greeting(t, 180, Some("Wanjiru")), "Morning, Wanjiru");
        let t = DateTime::parse_from_rfc3339("2026-11-08T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(greeting(t, 180, None), "Afternoon");
    }
}
