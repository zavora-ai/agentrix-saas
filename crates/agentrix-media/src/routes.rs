//! HTTP: the console's media library (owner only) and the public picture route.
//!
//! On the platform host, for the owner of `{site}` (W5 builds the page on these):
//! - `GET  /api/media/{site}`: the library, drafts first, with this month's picture spend;
//! - `POST /api/media/{site}/draw` `{slot, kind, brief, look?, context?, n?}`: draws and keeps
//!   drafts;
//! - `POST /api/media/{site}/{image}/approve`, `/decline`;
//! - `POST /api/media/{site}/{image}/again` `{brief?}`: a new draft for the same slot that would
//!   replace this picture;
//! - `GET  /api/media/{site}/{image}/file`: a draft's bytes, for the owner's eyes only.
//!
//! On every host that shows pictures (the platform host, and the sample hosts through
//! [`crate::public_router`]):
//! - `GET /media/{hex}.{ext}`: an approved picture's bytes. Anything not approved is a 404.

use std::sync::Arc;

use agentrix_host::NodeContext;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::Gateway;
use crate::library::{self, DecideError, Image, Request};
use crate::pipeline::{self, Ask, MakeError, Outcome};
use crate::policy::Kind;

/// What the routes run with.
#[derive(Clone)]
pub struct MediaState {
    ctx: Arc<NodeContext>,
    /// The effect runners decision cards use (the same list node gives agentrix-decisions), so
    /// a spoken approval runs exactly as a tapped one does.
    runners: Arc<Vec<Arc<dyn agentrix_decisions::EffectRunner>>>,
}

impl MediaState {
    /// From node's context.
    pub fn new(ctx: &NodeContext) -> Self {
        Self {
            ctx: Arc::new(ctx.clone()),
            runners: Arc::new(Vec::new()),
        }
    }

    /// With the decision cards' effect runners.
    #[must_use]
    pub fn with_runners(mut self, runners: Vec<Arc<dyn agentrix_decisions::EffectRunner>>) -> Self {
        self.runners = Arc::new(runners);
        self
    }

    /// Decision cards v2, with the runners.
    pub fn decisions(&self) -> agentrix_decisions::Decisions {
        agentrix_decisions::Decisions::new(&self.ctx).with_runners((*self.runners).clone())
    }

    /// The gateway for `site`.
    pub fn gateway(&self, site: &str) -> Gateway {
        Gateway::new(self.ctx.http.clone(), &self.ctx.models_url, site)
    }

    /// Node's context.
    pub fn ctx(&self) -> &NodeContext {
        &self.ctx
    }
}

/// An error as `{error: {code, message}}`, the message written for the owner.
pub(crate) struct Failure(
    pub(crate) StatusCode,
    pub(crate) &'static str,
    pub(crate) String,
);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({ "error": { "code": self.1, "message": self.2 } })),
        )
            .into_response()
    }
}

pub(crate) fn internal(err: impl std::fmt::Display) -> Failure {
    tracing::error!(error = %err, "media library error");
    Failure(
        StatusCode::INTERNAL_SERVER_ERROR,
        "media_error",
        "The media library hit an error. Nothing changed; try again.".into(),
    )
}

/// The console routes.
pub fn console_router(state: MediaState) -> Router {
    Router::new()
        .route("/api/media/{site}", get(list))
        .route("/api/media/{site}/draw", post(draw))
        .route("/api/media/{site}/{image}/approve", post(approve))
        .route("/api/media/{site}/{image}/decline", post(decline))
        .route("/api/media/{site}/{image}/again", post(again))
        .route("/api/media/{site}/{image}/file", get(draft_file))
        .with_state(state)
}

/// The public picture routes: approved pictures, and ideas drawn for a visitor's design
/// ([`crate::ideas`]), which are never approved pictures.
pub fn picture_router(state: MediaState) -> Router {
    Router::new()
        .route("/media/{file}", get(picture))
        .route("/media/ideas/{file}", get(idea_picture))
        .with_state(state)
}

/// The signed-in owner's account, if they own `site`.
pub(crate) async fn owner_of(
    state: &MediaState,
    headers: &HeaderMap,
    site: &str,
) -> Result<String, Failure> {
    let owner = state
        .ctx
        .owners
        .owner(headers)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            Failure(
                StatusCode::UNAUTHORIZED,
                "sign_in",
                "Sign in as the site's owner to see its pictures.".into(),
            )
        })?;
    let account = owner.account.to_string();
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?")
            .bind(site)
            .fetch_optional(&state.ctx.pool)
            .await
            .map_err(internal)?;
    match row {
        Some((Some(found),)) if found == account => Ok(account),
        _ => Err(Failure(
            StatusCode::FORBIDDEN,
            "not_owner",
            "Only the site's owner can manage its pictures.".into(),
        )),
    }
}

/// A picture as the console shows it: a draft's bytes come from the owner-only route.
fn for_console(site: &str, image: &Image) -> Value {
    let mut value = serde_json::to_value(image).unwrap_or(Value::Null);
    let src = match image.status.as_str() {
        "approved" => Some(image.url()),
        "blocked" => None,
        _ => Some(format!("/api/media/{site}/{}/file", image.image_id)),
    };
    value["src"] = json!(src);
    value["screen"] = serde_json::from_str(&image.screen).unwrap_or(Value::Null);
    value
}

async fn list(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    owner_of(&state, &headers, &site).await?;
    let images = library::list(&state.ctx.pool, &site)
        .await
        .map_err(internal)?;
    let spend = library::month_spend(&state.ctx.pool, &site)
        .await
        .map_err(internal)?;
    let waiting = images.iter().filter(|i| i.status == "draft").count();
    Ok(Json(json!({
        "site": site,
        "waiting": waiting,
        "spend": spend,
        "images": images.iter().map(|i| for_console(&site, i)).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
struct DrawBody {
    slot: String,
    kind: Kind,
    brief: String,
    #[serde(default)]
    look: String,
    #[serde(default)]
    context: String,
    #[serde(default)]
    n: Option<u8>,
}

fn slot_ok(slot: &str) -> bool {
    !slot.is_empty()
        && slot.len() <= 120
        && slot
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '/' | '#' | '.'))
}

/// Draws and keeps drafts for `request`; answers what was made and what was stopped.
pub async fn draw_and_keep(
    state: &MediaState,
    request: &Request<'_>,
    ask: &Ask,
) -> Result<(Vec<Image>, Vec<Image>), MakeError> {
    let outcomes = pipeline::make(&state.gateway(request.site), ask).await?;
    let (mut made, mut blocked) = (Vec::new(), Vec::new());
    for outcome in outcomes {
        let kept = match &outcome {
            Outcome::Made(m) => library::keep(&state.ctx.pool, &state.ctx.blobs, request, m).await,
            Outcome::Blocked(b) => library::keep_blocked(&state.ctx.pool, request, b).await,
        };
        match (kept, outcome) {
            (Ok(image), Outcome::Made(_)) => made.push(image),
            (Ok(image), Outcome::Blocked(_)) => blocked.push(image),
            (Err(err), _) => tracing::error!(error = %err, "could not keep a picture"),
        }
    }
    Ok((made, blocked))
}

fn make_failure(err: MakeError) -> Failure {
    match err {
        MakeError::Refused(r) => Failure(StatusCode::UNPROCESSABLE_ENTITY, "refused", r.0),
        MakeError::Gateway(e) if e.code == "over_budget" => {
            Failure(StatusCode::PAYMENT_REQUIRED, "over_budget", e.message)
        }
        MakeError::Gateway(e) => Failure(StatusCode::BAD_GATEWAY, "not_drawn", e.message),
    }
}

async fn draw(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DrawBody>,
) -> Result<Json<Value>, Failure> {
    owner_of(&state, &headers, &site).await?;
    if body.kind == Kind::Film {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "refused",
            "Pictures for your site never show people. Choose a product, a section or a post."
                .into(),
        ));
    }
    if body.kind == Kind::CakeIdea {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "refused",
            IDEAS_ARE_FOR_VISITORS.into(),
        ));
    }
    if !slot_ok(&body.slot) {
        return Err(Failure(
            StatusCode::BAD_REQUEST,
            "bad_slot",
            "Name the slot with letters, digits and : - _ / # . only.".into(),
        ));
    }
    let request = Request {
        site: &site,
        slot: &body.slot,
        kind: body.kind,
        brief: &body.brief,
        asked_by: "owner",
        replaces: None,
    };
    let ask = Ask {
        kind: body.kind,
        brief: body.brief.clone(),
        look: body.look.clone(),
        context: body.context.clone(),
        n: body.n.unwrap_or(1),
        seed: None,
        tier: "fast",
        sample: is_sample(&site),
    };
    let (made, blocked) = draw_and_keep(&state, &request, &ask)
        .await
        .map_err(make_failure)?;
    Ok(Json(json!({
        "made": made.iter().map(|i| for_console(&site, i)).collect::<Vec<_>>(),
        "blocked": blocked.len(),
    })))
}

#[derive(Debug, Default, Deserialize)]
struct AgainBody {
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    look: String,
}

async fn again(
    State(state): State<MediaState>,
    Path((site, image_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Option<Json<AgainBody>>,
) -> Result<Json<Value>, Failure> {
    owner_of(&state, &headers, &site).await?;
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let old = library::get(&state.ctx.pool, &site, &image_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            Failure(
                StatusCode::NOT_FOUND,
                "not_found",
                DecideError::NotFound.to_string(),
            )
        })?;
    let kind: Kind = serde_json::from_value(json!(old.kind)).unwrap_or(Kind::Section);
    let brief = body.brief.unwrap_or_else(|| old.brief.clone());
    let request = Request {
        site: &site,
        slot: &old.slot,
        kind,
        brief: &brief,
        asked_by: "owner",
        replaces: Some(&old.image_id),
    };
    let ask = Ask {
        kind,
        brief: brief.clone(),
        look: body.look,
        context: format!("A new picture for the {} slot", old.slot),
        n: 1,
        seed: None,
        tier: "fast",
        sample: is_sample(&site),
    };
    let (made, blocked) = draw_and_keep(&state, &request, &ask)
        .await
        .map_err(make_failure)?;
    Ok(Json(json!({
        "made": made.iter().map(|i| for_console(&site, i)).collect::<Vec<_>>(),
        "blocked": blocked.len(),
    })))
}

fn decide_failure(err: anyhow::Error) -> Failure {
    match err.downcast::<DecideError>() {
        Ok(DecideError::NotFound) => Failure(
            StatusCode::NOT_FOUND,
            "not_found",
            DecideError::NotFound.to_string(),
        ),
        Ok(e @ DecideError::NotADraft(_)) => {
            Failure(StatusCode::CONFLICT, "not_a_draft", e.to_string())
        }
        Err(err) => internal(err),
    }
}

async fn approve(
    State(state): State<MediaState>,
    Path((site, image_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    let account = owner_of(&state, &headers, &site).await?;
    let image = library::approve(
        &state.ctx.pool,
        &state.ctx.receipts,
        &site,
        &image_id,
        &format!("owner:{account}"),
    )
    .await
    .map_err(decide_failure)?;
    Ok(Json(for_console(&site, &image)))
}

async fn decline(
    State(state): State<MediaState>,
    Path((site, image_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    let account = owner_of(&state, &headers, &site).await?;
    let image = library::decline(
        &state.ctx.pool,
        &site,
        &image_id,
        &format!("owner:{account}"),
    )
    .await
    .map_err(decide_failure)?;
    Ok(Json(for_console(&site, &image)))
}

/// The bytes, with headers for an image that never changes under its name.
fn image_response(bytes: bytes::Bytes, media_type: &str, cache: &'static str) -> Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(media_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn draft_file(
    State(state): State<MediaState>,
    Path((site, image_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, Failure> {
    owner_of(&state, &headers, &site).await?;
    let image = library::get(&state.ctx.pool, &site, &image_id)
        .await
        .map_err(internal)?
        .filter(|i| !i.digest.is_empty())
        .ok_or_else(|| {
            Failure(
                StatusCode::NOT_FOUND,
                "not_found",
                DecideError::NotFound.to_string(),
            )
        })?;
    let bytes = state
        .ctx
        .blobs
        .get(&image.digest)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal("a kept picture's bytes are missing"))?;
    Ok(image_response(
        bytes,
        &image.media_type,
        "private, no-store",
    ))
}

/// `sha256:<hex>` from `<hex>.<ext>`, when it is one.
fn digest_of(file: &str) -> Option<String> {
    let (hex, _ext) = file.split_once('.')?;
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| format!("sha256:{}", hex.to_ascii_lowercase()))
}

async fn picture(State(state): State<MediaState>, Path(file): Path<String>) -> Response {
    let not_found = || (StatusCode::NOT_FOUND, "No picture here.").into_response();
    let Some(digest) = digest_of(&file) else {
        return not_found();
    };
    let media_type = match library::approved_type(&state.ctx.pool, &digest).await {
        Ok(Some(t)) => t,
        Ok(None) => return not_found(),
        Err(err) => return internal(err).into_response(),
    };
    match state.ctx.blobs.get(&digest).await {
        Ok(Some(bytes)) => {
            image_response(bytes, &media_type, "public, max-age=31536000, immutable")
        }
        Ok(None) => not_found(),
        Err(err) => internal(err).into_response(),
    }
}

/// An idea's bytes. Only digests kept as ideas are served, never a draft from the library.
async fn idea_picture(State(state): State<MediaState>, Path(file): Path<String>) -> Response {
    let not_found = || (StatusCode::NOT_FOUND, "No picture here.").into_response();
    let Some(digest) = digest_of(&file) else {
        return not_found();
    };
    let media_type = match crate::ideas::media_type(&state.ctx.pool, &digest).await {
        Ok(Some(t)) => t,
        Ok(None) => return not_found(),
        Err(err) => return internal(err).into_response(),
    };
    match state.ctx.blobs.get(&digest).await {
        Ok(Some(bytes)) => {
            image_response(bytes, &media_type, "public, max-age=31536000, immutable")
        }
        Ok(None) => not_found(),
        Err(err) => internal(err).into_response(),
    }
}

/// Why the owner's library never draws a cake idea: ideas are drawn for one visitor's design
/// and never become the site's own picture (ADR-032).
pub(crate) const IDEAS_ARE_FOR_VISITORS: &str = "Ideas of a cake are drawn for visitors in the cake studio, never for the site's own pictures. Choose a product, a section or a post.";

/// Sample sites always label their pictures.
pub fn is_sample(site: &str) -> bool {
    site.ends_with("-sample")
}
