//! Owner email codes, recovery codes and recovery holds on the platform host (JSON; W4 draws
//! the pages). The table is in [`super`]'s docs.

use axum::extract::{Query, Request, State};
use axum::http::header::{CACHE_CONTROL, ORIGIN, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use super::{OwnerError, OwnerPurpose, Owners, client_ip};
use crate::visitor::next_of;

/// The owner routes, for the platform host.
pub fn router(owners: Owners) -> Router {
    Router::new()
        .route("/auth/email/start", post(email_start))
        .route("/auth/email/verify", post(email_verify))
        .route("/auth/api/recovery-codes/shown", get(shown))
        .route("/auth/recovery-codes", get(codes_left).post(regenerate))
        .route("/auth/recovery-codes/start", post(stepup_start))
        .route("/auth/recover/code", post(recover_code))
        .route("/auth/recover/holds", get(holds))
        .route("/auth/recover/cancel", post(cancel))
        .layer(middleware::from_fn_with_state(owners.clone(), same_origin))
        .with_state(owners)
}

/// Refuses a POST from any origin but the platform's; nothing here is cached.
async fn same_origin(State(owners): State<Owners>, request: Request, next: Next) -> Response {
    if request.method() == Method::POST {
        let headers = request.headers();
        let ok = match headers.get(ORIGIN).and_then(|v| v.to_str().ok()) {
            Some(origin) => origin.trim_end_matches('/') == owners.origin(),
            None => {
                headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("same-origin")
            }
        };
        if !ok {
            return OwnerError::CrossOrigin.into_response();
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn with_cookie(mut response: Response, cookie: Option<HeaderValue>) -> Response {
    if let Some(cookie) = cookie {
        response.headers_mut().append(SET_COOKIE, cookie);
    }
    response
}

#[derive(Debug, Deserialize)]
struct Start {
    email: String,
    purpose: String,
    display_name: Option<String>,
    next: Option<String>,
}

async fn email_start(
    State(o): State<Owners>,
    headers: HeaderMap,
    Json(body): Json<Start>,
) -> Result<Response, OwnerError> {
    let purpose = OwnerPurpose::parse(&body.purpose)
        .ok_or_else(|| OwnerError::BadRequest("Say signup, signin or recover.".into()))?;
    let next = next_of(body.next.as_deref(), "/os");
    let (pending, sent_to) = o
        .start(
            &body.email,
            purpose,
            body.display_name.as_deref(),
            next,
            &client_ip(&headers),
        )
        .await?;
    Ok(Json(json!({ "pending": pending, "sent_to": sent_to })).into_response())
}

#[derive(Debug, Deserialize)]
struct Verify {
    pending: String,
    code: String,
}

async fn email_verify(
    State(o): State<Owners>,
    headers: HeaderMap,
    Json(body): Json<Verify>,
) -> Result<Response, OwnerError> {
    let (verified, cookie) = o
        .verify(&body.pending, &body.code, &headers, &client_ip(&headers))
        .await?;
    Ok(with_cookie(Json(verified).into_response(), cookie))
}

#[derive(Debug, Deserialize)]
struct ShownQuery {
    r: String,
}

async fn shown(State(o): State<Owners>, Query(q): Query<ShownQuery>) -> Response {
    match o.take_shown(&q.r) {
        Some((codes, next)) => Json(json!({ "codes": codes, "next": next })).into_response(),
        None => (
            StatusCode::GONE,
            Json(json!({ "message": "These codes were shown once. Make new ones in your sign-in settings.", "code": "shown" })),
        )
            .into_response(),
    }
}

async fn codes_left(State(o): State<Owners>, headers: HeaderMap) -> Result<Response, OwnerError> {
    Ok(Json(json!({ "left": o.codes_left(&headers).await? })).into_response())
}

async fn stepup_start(State(o): State<Owners>, headers: HeaderMap) -> Result<Response, OwnerError> {
    let (challenge, passkey) = o.stepup_start(&headers).await?;
    Ok(Json(json!({ "challenge": challenge, "passkey": passkey })).into_response())
}

#[derive(Debug, Default, Deserialize)]
struct Regenerate {
    challenge: Option<String>,
    passkey: Option<serde_json::Value>,
    totp: Option<String>,
}

async fn regenerate(
    State(o): State<Owners>,
    headers: HeaderMap,
    body: Option<Json<Regenerate>>,
) -> Result<Response, OwnerError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let proof = match (body.passkey, body.totp) {
        (Some(credential), _) => Some(json!({ "passkey": credential })),
        (None, Some(code)) => Some(json!({ "totp": code })),
        (None, None) => None,
    };
    let codes = o
        .regenerate(&headers, body.challenge.as_deref(), proof)
        .await?;
    Ok(Json(json!({ "codes": codes })).into_response())
}

#[derive(Debug, Deserialize)]
struct RecoverCode {
    email: String,
    code: String,
}

async fn recover_code(
    State(o): State<Owners>,
    headers: HeaderMap,
    Json(body): Json<RecoverCode>,
) -> Result<Response, OwnerError> {
    let cookie = o
        .recover_with_code(&body.email, &body.code, &headers, &client_ip(&headers))
        .await?;
    Ok(with_cookie(
        Json(json!({ "redirect": "/console/settings/sign-in?recovered=1" })).into_response(),
        Some(cookie),
    ))
}

async fn holds(State(o): State<Owners>, headers: HeaderMap) -> Result<Response, OwnerError> {
    Ok(Json(json!({ "holds": o.holds(&headers).await? })).into_response())
}

#[derive(Debug, Deserialize)]
struct Cancel {
    hold: String,
}

async fn cancel(
    State(o): State<Owners>,
    headers: HeaderMap,
    Json(body): Json<Cancel>,
) -> Result<Response, OwnerError> {
    Ok(Json(json!({ "cancelled": o.cancel_hold(&headers, &body.hold).await? })).into_response())
}
