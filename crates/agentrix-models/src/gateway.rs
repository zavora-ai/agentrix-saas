//! The gateway's HTTP surface (design §4.3), on loopback for agentrix-node only.
//!
//! - `POST /v1/chat/completions` (purposes `build`, `concierge`, `brief`, `vision-parity`) and
//!   `POST /v1/embeddings` (`embed`) take OpenAI wire JSON. Answers come back in the same
//!   shape whichever provider served them.
//! - `POST /v1/screen` (`screen`) takes `{text}` and answers `{safe, user_safety, categories,
//!   model, raw}`. Anything but an explicit "safe" is unsafe.
//! - `POST /v1/decide` (`decide`) takes a systemone body (`state`, `questions`) and returns
//!   systemone's answer shape.
//! - `POST /v1/rerank` (`rerank`) forwards to Vultr's rerank.
//! - `POST /v1/vector_store`, `POST /v1/vector_store/{id}/items` and
//!   `POST /v1/vector_store/{id}/search` (`embed`) forward to Vultr's vector store, for Ask's
//!   retrieval over a release's public text (R11.5, S3-06). Metered and replayable like
//!   embeddings; the store's own embedding model is unnamed, so usage is recorded as
//!   `vultr-vector-store`.
//! - `POST /v1/images/generations`, `/v1/images/edits`, `/v1/images/screen` and
//!   `/v1/images/describe` (`image`) draw, edit, screen and describe images on Vultr's image
//!   models, metered by images made (`media/image.rs`, W11).
//! - `GET /v1/status` gives the provider, the mode, the default routes and whether providers
//!   are answering, in plain words (R11.4).
//!
//! Every call names `x-agentrix-site` and `x-agentrix-purpose`, and may name
//! `x-agentrix-move` (cost per Move, S2-11), `x-agentrix-tier` (design §4.3), and
//! `x-agentrix-webagent` and `x-agentrix-job` (which WebAgent and job made the call, W1; kept
//! on the usage row for spend per WebAgent). Those
//! headers never reach a provider.
//!
//! Every answer says how it was routed, so node can put it on the receipt:
//! `x-agentrix-provider`, `x-agentrix-model`, `x-agentrix-tier` and `x-agentrix-tier-reason`.
//! In `record` and `replay` modes, answers come from recordings where they exist (D10a).

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio_stream::wrappers::ReceiverStream;

use crate::cache::{Mode, Recording, Recordings};
use crate::claude::{self, Claude, ClaudeError};
use crate::meter::{self, Outcome, Usage};
use crate::route::{Choice, Endpoint, Provider, Purpose, Routing, Tier, clamp};
use crate::select::{self, Asked, Descriptor};
use crate::upstream::{UpstreamError, Vultr};

/// What the gateway runs with.
pub struct Config {
    /// Vultr, in `live` and `record` modes. `None` in `replay`, which needs no key.
    pub vultr: Option<Vultr>,
    /// Anthropic, in `live` and `record` modes when a route can reach it.
    pub claude: Option<Claude>,
    pub pool: SqlitePool,
    pub routing: Routing,
    pub mode: Mode,
    pub recordings: Recordings,
    /// The hard limit per site per day (D10e), in micro-USD.
    pub daily_cap_micro_usd: i64,
    /// Daily limits sites share: every site together, and the sample sites together.
    pub shared_caps: meter::SharedCaps,
}

/// Shared state.
#[derive(Clone)]
pub struct Gateway {
    inner: Arc<Inner>,
}

struct Inner {
    config: Config,
    status: Mutex<ProviderStatus>,
}

/// Whether providers are answering, for the console.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub state: &'static str,
    pub since: String,
    pub message: String,
}

impl Gateway {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        if config.mode != Mode::Replay {
            anyhow::ensure!(
                config.vultr.is_some(),
                "{} mode calls Vultr (embeddings stay there), so it needs VULTR_INFERENCE_API_KEY",
                config.mode.as_str()
            );
            anyhow::ensure!(
                !config.routing.uses(Provider::Anthropic) || config.claude.is_some(),
                "{} mode routes to Anthropic, so it needs ANTHROPIC_API_KEY",
                config.mode.as_str()
            );
        }
        let (state, message) = match config.mode {
            Mode::Replay => (
                "replay",
                "Replay mode: answers come from recordings only; no provider is called.",
            ),
            Mode::Record | Mode::Live => ("unknown", "No provider call yet."),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                status: Mutex::new(ProviderStatus {
                    state,
                    since: meter::stamp(Utc::now()),
                    message: message.to_owned(),
                }),
                config,
            }),
        })
    }

    fn config(&self) -> &Config {
        &self.inner.config
    }

    fn set_status(&self, state: &'static str, message: String) {
        let mut status = self.inner.status.lock().unwrap_or_else(|e| e.into_inner());
        if status.state != state {
            status.since = meter::stamp(Utc::now());
        }
        status.state = state;
        status.message = message;
    }

    async fn record(
        &self,
        caller: &Caller,
        purpose: Purpose,
        choice: &Choice,
        tokens: Tokens,
        outcome: Outcome,
    ) {
        let usage = Usage {
            site: &caller.site,
            purpose,
            provider: choice.provider,
            tier: choice.tier,
            tier_reason: &choice.reason,
            model: &choice.model,
            move_id: caller.move_id.as_deref(),
            tokens_in: tokens.input,
            tokens_out: tokens.output,
            reasoning_tokens: tokens.reasoning,
            outcome,
            media: None,
            webagent: caller.webagent.as_deref(),
            job: caller.job.as_deref(),
        };
        if let Err(err) = meter::record(&self.config().pool, &usage).await {
            tracing::error!(error = %err, site = %caller.site, "could not record usage");
        }
    }
}

/// The router.
pub fn router(gateway: Gateway) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/screen", post(screen))
        .route("/v1/decide", post(decide))
        .route("/v1/rerank", post(rerank))
        .route("/v1/vector_store", post(vector_store_create))
        .route("/v1/vector_store/{id}/items", post(vector_store_items))
        .route("/v1/vector_store/{id}/search", post(vector_store_search))
        .route("/v1/status", get(status))
        .route("/healthz", get(|| async { "ok" }))
        .merge(media::routes())
        .with_state(gateway)
}

/// An error in OpenAI's shape, so OpenAI-compatible clients show the message.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    /// Seconds to send in `Retry-After`.
    retry_after: Option<u64>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
        }
    }

    /// A shared daily limit is spent: 429 with `Retry-After` until midnight UTC.
    fn paused(message: impl Into<String>) -> Self {
        Self {
            retry_after: Some(meter::seconds_to_midnight(Utc::now())),
            ..Self::new(StatusCode::TOO_MANY_REQUESTS, "paused_for_today", message)
        }
    }

    fn bad(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "gateway error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "gateway_error",
            "The model gateway hit an internal error.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body =
            json!({ "error": { "code": self.code, "type": self.code, "message": self.message } });
        let mut response = (self.status, Json(body)).into_response();
        if let Some(seconds) = self.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

struct Caller {
    site: String,
    purpose: Purpose,
    move_id: Option<String>,
    webagent: Option<String>,
    job: Option<String>,
}

/// A WebAgent or job name from a header: lower case letters, digits, `.`, `_` and `-`, at most
/// 64. `Err` names the header for the refusal.
fn name_header(headers: &HeaderMap, name: &'static str) -> Result<Option<String>, ApiError> {
    let Some(value) = header_text(headers, name) else {
        return Ok(None);
    };
    let value = value.to_ascii_lowercase();
    let ok = value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        Ok(Some(value))
    } else {
        Err(ApiError::bad(
            "bad_name",
            format!("{name} takes a short name: letters, digits, dot, dash or underscore."),
        ))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Tokens {
    input: u64,
    output: u64,
    reasoning: u64,
}

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Checks who is calling and for what, and that the site may still spend today.
async fn admit(gw: &Gateway, headers: &HeaderMap, allowed: &[Purpose]) -> Result<Caller, ApiError> {
    let site = header_text(headers, "x-agentrix-site").ok_or_else(|| {
        ApiError::bad(
            "missing_site",
            "Every call names its site in x-agentrix-site.",
        )
    })?;
    let purpose_text = header_text(headers, "x-agentrix-purpose").ok_or_else(|| {
        ApiError::bad(
            "missing_purpose",
            "Every call names its purpose in x-agentrix-purpose.",
        )
    })?;
    let purpose = Purpose::parse(purpose_text).ok_or_else(|| {
        ApiError::bad(
            "unknown_purpose",
            format!("{purpose_text:?} is not a purpose this gateway routes."),
        )
    })?;
    if !allowed.contains(&purpose) {
        return Err(ApiError::bad(
            "wrong_route",
            format!("Purpose {} is not served on this route.", purpose.as_str()),
        ));
    }
    let move_id = header_text(headers, "x-agentrix-move").map(str::to_owned);
    let webagent = name_header(headers, "x-agentrix-webagent")?;
    let job = name_header(headers, "x-agentrix-job")?;
    let config = gw.config();
    if !meter::caller_known(&config.pool, site, move_id.as_deref())
        .await
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "unknown_site",
            "This site, or this Move for it, is not known to the platform.",
        ));
    }
    let caller = Caller {
        site: site.to_owned(),
        purpose,
        move_id,
        webagent,
        job,
    };
    if let Some(spent) = meter::over_budget(
        &config.pool,
        site,
        purpose,
        Utc::now(),
        config.daily_cap_micro_usd,
    )
    .await
    .map_err(ApiError::internal)?
    {
        let choice = config.routing.choose(
            purpose,
            purpose.profile().default_tier,
            "refused: over budget",
        );
        gw.record(
            &caller,
            purpose,
            &choice,
            Tokens::default(),
            Outcome::OverBudget,
        )
        .await;
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "over_budget",
            meter::exhausted_words(&spent),
        ));
    }
    // Then the limits sites share: the sample sites' and the platform's (W0, Sun 8 Nov).
    if let Some(spent) = meter::over_shared_cap(&config.pool, site, Utc::now(), config.shared_caps)
        .await
        .map_err(ApiError::internal)?
    {
        let reason = format!(
            "{} (${:.2} spent of ${:.2})",
            spent.limit.reason(),
            spent.spent_micro_usd as f64 / 1e6,
            spent.limit_micro_usd as f64 / 1e6,
        );
        tracing::warn!(site, purpose = purpose.as_str(), %reason, "model call refused");
        let choice = config
            .routing
            .choose(purpose, purpose.profile().default_tier, reason);
        gw.record(
            &caller,
            purpose,
            &choice,
            Tokens::default(),
            Outcome::OverBudget,
        )
        .await;
        return Err(ApiError::paused(meter::exhausted_words(&spent)));
    }
    Ok(caller)
}

/// Picks the tier, then the provider and model (design §4.3).
async fn choose(
    gw: &Gateway,
    caller: &Caller,
    headers: &HeaderMap,
    body: &Value,
) -> Result<Choice, ApiError> {
    let purpose = caller.purpose;
    let profile = purpose.profile();
    let asked = select::asked(headers).map_err(|m| ApiError::bad("bad_tier", m))?;
    let (tier, reason) = match asked {
        Asked::Default => (profile.default_tier, "default tier".to_owned()),
        Asked::Fixed(tier) => (
            clamp(purpose, tier),
            format!("caller asked for {}", tier.as_str()),
        ),
        Asked::Auto if !profile.dynamic => (
            profile.default_tier,
            format!("{} has no dynamic tiers; default tier", purpose.as_str()),
        ),
        Asked::Auto => {
            let descriptor = Descriptor::build(purpose, headers, body);
            match select::rules(&descriptor) {
                Some(rule) => rule,
                None => route_by_router(gw, caller, &descriptor).await,
            }
        }
    };
    let (tier, reason) = budget_step_down(gw, caller, tier, reason).await?;
    Ok(gw.config().routing.choose(purpose, tier, reason))
}

/// The budget has the last word: under a fifth of today's limit left, drop a tier (never
/// below the floor) and say so.
async fn budget_step_down(
    gw: &Gateway,
    caller: &Caller,
    tier: Tier,
    reason: String,
) -> Result<(Tier, String), ApiError> {
    let config = gw.config();
    let spent = meter::spent_today(&config.pool, &caller.site, Utc::now())
        .await
        .map_err(ApiError::internal)?;
    let left = config.daily_cap_micro_usd - spent;
    let lower = clamp(caller.purpose, tier.down());
    if left * 5 < config.daily_cap_micro_usd && lower != tier {
        return Ok((
            lower,
            format!("{reason}; budget: under a fifth of today's limit left, one tier down"),
        ));
    }
    let (platform, _) = meter::shared_spent_today(&config.pool, Utc::now())
        .await
        .map_err(ApiError::internal)?;
    let platform_cap = config.shared_caps.platform_micro_usd;
    if (platform_cap - platform) * 5 < platform_cap && lower != tier {
        return Ok((
            lower,
            format!(
                "{reason}; budget: under a fifth of the platform's daily limit left, one tier down"
            ),
        ));
    }
    Ok((tier, reason))
}

/// Step 2 of dynamic selection. On any failure the default tier stands; the router never
/// blocks the call it is routing.
async fn route_by_router(gw: &Gateway, caller: &Caller, descriptor: &Descriptor) -> (Tier, String) {
    let purpose = caller.purpose;
    let fallback = |why: &str| {
        (
            purpose.profile().default_tier,
            format!("default tier ({why})"),
        )
    };
    let config = gw.config();
    let choice = config.routing.choose(Purpose::Router, Tier::Fast, "router");
    let key = json!({ "descriptor": descriptor.to_json(), "model": choice.model });
    let answer = match recorded(gw, "router", &choice.model, &key).await {
        Ok(Some(recording)) => {
            gw.record(
                caller,
                Purpose::Router,
                &choice,
                Tokens::default(),
                Outcome::Replayed,
            )
            .await;
            recording.body
        }
        Ok(None) if config.mode == Mode::Replay => {
            return fallback("replay has no router recording");
        }
        Ok(None) => {
            let user = descriptor.to_json().to_string();
            let result = match choice.provider {
                Provider::Anthropic => {
                    let request = claude::structured_request(
                        &choice.model,
                        adk_core::Effort::Low,
                        select::ROUTER_SYSTEM,
                        user,
                        select::router_schema(),
                    );
                    claude_completion(gw, caller, Purpose::Router, &choice, None, request).await
                }
                Provider::Vultr => {
                    let body = json!({
                        "model": choice.model,
                        "max_tokens": Purpose::Router.default_max_tokens(),
                        "reasoning_effort": "low",
                        "messages": [
                            { "role": "system", "content": select::ROUTER_SYSTEM },
                            { "role": "user", "content": user }
                        ],
                        "response_format": { "type": "json_schema", "json_schema": {
                            "name": "tier", "strict": true, "schema": select::router_schema()
                        }}
                    });
                    vultr_json(gw, caller, Purpose::Router, &choice, &body).await
                }
            };
            let Ok(completion) = result else {
                return fallback("router unavailable");
            };
            let text = completion["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default();
            let Ok(answer) = serde_json::from_str::<Value>(text) else {
                return fallback("router answer unreadable");
            };
            if config.mode == Mode::Record {
                let bytes = answer.to_string();
                save(
                    gw,
                    "router",
                    &choice.model,
                    "router",
                    &key,
                    StatusCode::OK,
                    "application/json",
                    bytes.as_bytes(),
                )
                .await;
            }
            answer
        }
        Err(_) => return fallback("router recording unreadable"),
    };
    select::read_router(purpose, &answer).unwrap_or_else(|| fallback("router answer unreadable"))
}

fn upstream_error(gw: &Gateway, err: UpstreamError) -> ApiError {
    match err {
        UpstreamError::Busy {
            tries,
            last_status,
            waited,
        } => {
            let seen = last_status.map_or("no connection".to_owned(), |s| format!("HTTP {s}"));
            let message = format!(
                "Vultr Inference is busy right now ({seen}). The gateway tried {tries} times over {:.1} s and stopped; nothing was sent to another provider. Try again in a minute.",
                waited.as_secs_f64()
            );
            gw.set_status("busy", message.clone());
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "provider_busy", message)
        }
        UpstreamError::Unfunded => {
            let message = "Vultr says this account is not funded for this model. Add credit in the Vultr console.".to_owned();
            gw.set_status("unfunded", message.clone());
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "vultr_unfunded", message)
        }
    }
}

fn claude_error(gw: &Gateway, err: ClaudeError) -> ApiError {
    match err {
        ClaudeError::Bad(message) => ApiError::bad("not_for_claude", message),
        ClaudeError::Busy(detail) => {
            tracing::warn!(detail, "Anthropic busy after retries");
            let message = "Anthropic is busy right now. The gateway tried 3 times and stopped; nothing was sent to another provider. Try again in a minute.".to_owned();
            gw.set_status("busy", message.clone());
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "provider_busy", message)
        }
        ClaudeError::Failed(detail) => {
            tracing::error!(detail, "Anthropic call failed");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "provider_failed",
                format!("Anthropic did not answer: {detail}"),
            )
        }
    }
}

fn usage_of(value: &Value) -> Tokens {
    let usage = &value["usage"];
    Tokens {
        input: usage["prompt_tokens"]
            .as_u64()
            .or_else(|| usage["input_tokens"].as_u64())
            .unwrap_or(0),
        output: usage["completion_tokens"]
            .as_u64()
            .or_else(|| usage["output_tokens"].as_u64())
            .unwrap_or(0),
        reasoning: usage["completion_tokens_details"]["reasoning_tokens"]
            .as_u64()
            .unwrap_or(0),
    }
}

/// A response with the route headers.
fn respond(status: StatusCode, content_type: &str, bytes: Bytes, choice: &Choice) -> Response {
    let mut response = (status, bytes).into_response();
    let headers = response.headers_mut();
    let put = |headers: &mut HeaderMap, name: &'static str, value: &str| {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name, value);
        }
    };
    put(headers, "content-type", content_type);
    put(headers, "x-agentrix-provider", choice.provider.as_str());
    put(headers, "x-agentrix-model", &choice.model);
    put(
        headers,
        "x-agentrix-tier",
        choice.tier.map_or("specialist", Tier::as_str),
    );
    let reason: String = choice
        .reason
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .collect();
    put(headers, "x-agentrix-tier-reason", &reason);
    response
}

/// A recording for this request, in `record` and `replay` modes. In `replay` a missing
/// recording is an error that says how to make one; no provider is called.
async fn recorded(
    gw: &Gateway,
    name: &str,
    model: &str,
    key: &Value,
) -> Result<Option<Recording>, ApiError> {
    let config = gw.config();
    if config.mode == Mode::Live {
        return Ok(None);
    }
    let found = config
        .recordings
        .load(name, model, key)
        .await
        .map_err(ApiError::internal)?;
    if found.is_none() && config.mode == Mode::Replay && name != "router" {
        let path = config
            .recordings
            .path(name, model, key)
            .map_err(ApiError::internal)?;
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_recording",
            format!(
                "Replay mode has no recording for this {name} request on {model} ({}). Record it once with AGENTRIX_MODELS_MODE=record, then commit the file.",
                path.display()
            ),
        ));
    }
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
async fn save(
    gw: &Gateway,
    name: &str,
    model: &str,
    endpoint: &str,
    key: &Value,
    status: StatusCode,
    content_type: &str,
    bytes: &[u8],
) {
    match gw
        .config()
        .recordings
        .save(name, model, endpoint, key, status, content_type, bytes)
        .await
    {
        Ok(path) => tracing::info!(path = %path.display(), "recorded"),
        Err(err) => tracing::error!(error = %err, "could not save a recording"),
    }
}

/// One Claude completion, from a recording or from Anthropic, metered and (in `record` mode)
/// saved. `key` is `None` for calls cached elsewhere (the router).
async fn claude_completion(
    gw: &Gateway,
    caller: &Caller,
    purpose: Purpose,
    choice: &Choice,
    key: Option<&Value>,
    request: adk_core::LlmRequest,
) -> Result<Value, ApiError> {
    if let Some(key) = key
        && let Some(recording) = recorded(gw, purpose.as_str(), &choice.model, key).await?
    {
        let completion = recording.body;
        gw.record(
            caller,
            purpose,
            choice,
            usage_of(&completion),
            Outcome::Replayed,
        )
        .await;
        return Ok(completion);
    }
    let claude = gw
        .config()
        .claude
        .as_ref()
        .ok_or_else(|| ApiError::internal("no Anthropic client outside replay mode"))?;
    let answer = match claude.generate(&choice.model, request).await {
        Ok(answer) => answer,
        Err(err) => {
            gw.record(caller, purpose, choice, Tokens::default(), Outcome::Error)
                .await;
            return Err(claude_error(gw, err));
        }
    };
    gw.set_status("ok", "Providers are answering.".to_owned());
    let completion = claude::chat_completion(&choice.model, &answer);
    gw.record(caller, purpose, choice, usage_of(&completion), Outcome::Ok)
        .await;
    if let Some(key) = key
        && gw.config().mode == Mode::Record
    {
        let bytes = completion.to_string();
        save(
            gw,
            purpose.as_str(),
            &choice.model,
            "anthropic/messages",
            key,
            StatusCode::OK,
            "application/json",
            bytes.as_bytes(),
        )
        .await;
    }
    Ok(completion)
}

/// One non-streaming Vultr call, from a recording or from Vultr, metered and saved.
async fn vultr_forward(
    gw: &Gateway,
    caller: &Caller,
    purpose: Purpose,
    choice: &Choice,
    body: &Value,
) -> Result<(StatusCode, String, Bytes), ApiError> {
    vultr_forward_at(
        gw,
        caller,
        purpose,
        choice,
        choice.endpoint.path(),
        body,
        body,
    )
    .await
}

/// [`vultr_forward`] to any path under Vultr's base URL, recorded under `key` (which must name
/// everything that makes the answer differ, such as the path's collection id).
#[allow(clippy::too_many_arguments)]
async fn vultr_forward_at(
    gw: &Gateway,
    caller: &Caller,
    purpose: Purpose,
    choice: &Choice,
    path: &str,
    key: &Value,
    body: &Value,
) -> Result<(StatusCode, String, Bytes), ApiError> {
    if let Some(recording) = recorded(gw, purpose.as_str(), &choice.model, key).await? {
        let bytes = Bytes::from(recording.bytes());
        let tokens = serde_json::from_slice::<Value>(&bytes)
            .map(|v| usage_of(&v))
            .unwrap_or_default();
        gw.record(caller, purpose, choice, tokens, Outcome::Replayed)
            .await;
        return Ok((recording.status(), recording.content_type, bytes));
    }
    let config = gw.config();
    let vultr = config
        .vultr
        .as_ref()
        .ok_or_else(|| ApiError::internal("no Vultr client outside replay mode"))?;
    let response = match vultr.post(path, body).await {
        Ok(response) => response,
        Err(err) => {
            gw.record(caller, purpose, choice, Tokens::default(), Outcome::Error)
                .await;
            return Err(upstream_error(gw, err));
        }
    };
    gw.set_status("ok", "Providers are answering.".to_owned());
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_owned();
    let bytes = response.bytes().await.map_err(|err| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "provider_failed",
            format!("Vultr's answer was cut off: {err}"),
        )
    })?;
    let tokens = serde_json::from_slice::<Value>(&bytes)
        .map(|v| usage_of(&v))
        .unwrap_or_default();
    let outcome = if status.is_success() {
        Outcome::Ok
    } else {
        Outcome::Error
    };
    gw.record(caller, purpose, choice, tokens, outcome).await;
    if config.mode == Mode::Record && status.is_success() {
        save(
            gw,
            purpose.as_str(),
            &choice.model,
            path,
            key,
            status,
            &content_type,
            &bytes,
        )
        .await;
    }
    Ok((status, content_type, bytes))
}

/// A Vultr call whose answer the gateway reads as JSON.
async fn vultr_json(
    gw: &Gateway,
    caller: &Caller,
    purpose: Purpose,
    choice: &Choice,
    body: &Value,
) -> Result<Value, ApiError> {
    let (status, _, bytes) = vultr_forward(gw, caller, purpose, choice, body).await?;
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "provider_failed",
            format!("Vultr answered HTTP {status}"),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "provider_failed",
            "Vultr's answer was not JSON",
        )
    })
}

fn object(body: &mut Value) -> Result<&mut serde_json::Map<String, Value>, ApiError> {
    body.as_object_mut()
        .ok_or_else(|| ApiError::bad("bad_body", "The body must be a JSON object."))
}

/// The cache key for a chat request: the request without its streaming flags, so a stream
/// and a plain call share one recording.
fn chat_key(body: &Value, choice: &Choice, purpose: Purpose) -> Value {
    let mut request = body.clone();
    if let Some(obj) = request.as_object_mut() {
        obj.remove("stream");
        obj.remove("stream_options");
        obj.remove("model");
    }
    json!({ "request": request, "model": choice.model, "effort": purpose.profile().effort })
}

async fn chat(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Result<Response, ApiError> {
    let caller = admit(
        &gw,
        &headers,
        &[
            Purpose::Build,
            Purpose::Concierge,
            Purpose::Brief,
            Purpose::VisionParity,
            Purpose::Negotiate,
            Purpose::Collect,
            Purpose::Extract,
        ],
    )
    .await?;
    object(&mut body)?;
    let choice = choose(&gw, &caller, &headers, &body).await?;
    let purpose = caller.purpose;
    let stream = body["stream"].as_bool().unwrap_or(false);
    match choice.provider {
        Provider::Anthropic => {
            let key = chat_key(&body, &choice, purpose);
            let (request, _) =
                claude::chat_request(&body, &choice.model, purpose, purpose.default_max_tokens())
                    .map_err(|e| claude_error(&gw, e))?;
            let completion =
                claude_completion(&gw, &caller, purpose, &choice, Some(&key), request).await?;
            if stream {
                let sse = claude::chat_stream(&completion);
                return Ok(respond(
                    StatusCode::OK,
                    "text/event-stream",
                    Bytes::from(sse),
                    &choice,
                ));
            }
            Ok(respond(
                StatusCode::OK,
                "application/json",
                Bytes::from(completion.to_string()),
                &choice,
            ))
        }
        Provider::Vultr => {
            let obj = object(&mut body)?;
            obj.insert("model".to_owned(), json!(choice.model));
            if !obj.contains_key("max_tokens")
                && !obj.contains_key("max_completion_tokens")
                && let Some(n) = purpose.default_max_tokens()
            {
                obj.insert("max_tokens".to_owned(), json!(n));
            }
            if !obj.contains_key("reasoning_effort") && !obj.contains_key("reasoning") {
                obj.insert(
                    "reasoning_effort".to_owned(),
                    json!(purpose.profile().effort),
                );
            }
            if stream {
                obj.insert(
                    "stream_options".to_owned(),
                    json!({ "include_usage": true }),
                );
                return vultr_stream(gw, caller, choice, body).await;
            }
            let (status, content_type, bytes) =
                vultr_forward(&gw, &caller, purpose, &choice, &body).await?;
            Ok(respond(status, &content_type, bytes, &choice))
        }
    }
}

/// Streams Vultr's SSE through unchanged and meters the usage chunk at the end. A recorded
/// stream is replayed whole.
async fn vultr_stream(
    gw: Gateway,
    caller: Caller,
    choice: Choice,
    body: Value,
) -> Result<Response, ApiError> {
    let purpose = caller.purpose;
    if let Some(recording) = recorded(&gw, purpose.as_str(), &choice.model, &body).await? {
        let bytes = recording.bytes();
        let mut scan = UsageScan::default();
        scan.feed(&bytes);
        gw.record(
            &caller,
            purpose,
            &choice,
            scan.usage.unwrap_or_default(),
            Outcome::Replayed,
        )
        .await;
        return Ok(respond(
            recording.status(),
            &recording.content_type,
            Bytes::from(bytes),
            &choice,
        ));
    }
    let vultr = gw
        .config()
        .vultr
        .as_ref()
        .ok_or_else(|| ApiError::internal("no Vultr client outside replay mode"))?;
    let response = match vultr.post(Endpoint::Chat.path(), &body).await {
        Ok(response) => response,
        Err(err) => {
            gw.record(&caller, purpose, &choice, Tokens::default(), Outcome::Error)
                .await;
            return Err(upstream_error(&gw, err));
        }
    };
    gw.set_status("ok", "Providers are answering.".to_owned());
    let status = response.status();
    if !status.is_success() {
        let bytes = response.bytes().await.unwrap_or_default();
        gw.record(&caller, purpose, &choice, Tokens::default(), Outcome::Error)
            .await;
        return Ok(respond(status, "application/json", bytes, &choice));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/event-stream")
        .to_owned();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(32);
    let mut upstream = response.bytes_stream();
    let mut reply = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(ApiError::internal)?;
    {
        let headers = reply.headers_mut();
        let template = respond(status, &content_type, Bytes::new(), &choice);
        for (name, value) in template.headers() {
            headers.insert(name.clone(), value.clone());
        }
    }
    tokio::spawn(async move {
        let recording = gw.config().mode == Mode::Record;
        let mut kept = Vec::new();
        let mut complete = true;
        let mut scan = UsageScan::default();
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(bytes) => {
                    scan.feed(&bytes);
                    if recording {
                        kept.extend_from_slice(&bytes);
                    }
                    if tx.send(Ok(bytes)).await.is_err() {
                        complete = false;
                        break;
                    }
                }
                Err(err) => {
                    complete = false;
                    let _ = tx.send(Err(std::io::Error::other(err))).await;
                    break;
                }
            }
        }
        let outcome = if scan.usage.is_some() {
            Outcome::Ok
        } else {
            Outcome::Error
        };
        gw.record(
            &caller,
            purpose,
            &choice,
            scan.usage.unwrap_or_default(),
            outcome,
        )
        .await;
        if recording && complete && scan.usage.is_some() {
            save(
                &gw,
                purpose.as_str(),
                &choice.model,
                Endpoint::Chat.path(),
                &body,
                status,
                &content_type,
                &kept,
            )
            .await;
        }
    });
    Ok(reply)
}

/// Finds the usage chunk in an SSE stream, across chunk boundaries.
#[derive(Default)]
struct UsageScan {
    line: Vec<u8>,
    usage: Option<Tokens>,
}

impl UsageScan {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                let line = std::mem::take(&mut self.line);
                self.line_done(&line);
            } else {
                self.line.push(byte);
            }
        }
    }

    fn line_done(&mut self, line: &[u8]) {
        let Some(data) = line.strip_prefix(b"data:") else {
            return;
        };
        let data = data.trim_ascii();
        if data == b"[DONE]" {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(data)
            && value.get("usage").is_some_and(|u| !u.is_null())
        {
            self.usage = Some(usage_of(&value));
        }
    }
}

/// Embeddings and rerank: specialists on Vultr; the rest passes through.
async fn specialist(
    gw: &Gateway,
    headers: &HeaderMap,
    purpose: Purpose,
    mut body: Value,
) -> Result<Response, ApiError> {
    let caller = admit(gw, headers, &[purpose]).await?;
    let choice = gw
        .config()
        .routing
        .choose(purpose, Tier::Fast, "specialist");
    object(&mut body)?.insert("model".to_owned(), json!(choice.model));
    let (status, content_type, bytes) = vultr_forward(gw, &caller, purpose, &choice, &body).await?;
    Ok(respond(status, &content_type, bytes, &choice))
}

async fn embeddings(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    specialist(&gw, &headers, Purpose::Embed, body).await
}

async fn rerank(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    specialist(&gw, &headers, Purpose::Rerank, body).await
}

/// The model name usage rows carry for Vultr's vector store, whose embedder is unnamed.
pub const VECTOR_STORE_MODEL: &str = "vultr-vector-store";

/// One vector-store call: a specialist on Vultr whatever the provider, metered under `embed`.
async fn vector_store(
    gw: &Gateway,
    headers: &HeaderMap,
    path: String,
    body: Value,
) -> Result<Response, ApiError> {
    let caller = admit(gw, headers, &[Purpose::Embed]).await?;
    let choice = Choice {
        provider: Provider::Vultr,
        model: VECTOR_STORE_MODEL.to_owned(),
        endpoint: Endpoint::Embeddings,
        tier: None,
        reason: "specialist: Vultr's vector store".to_owned(),
    };
    let key = json!({ "path": path, "body": body });
    let (status, content_type, bytes) =
        vultr_forward_at(gw, &caller, Purpose::Embed, &choice, &path, &key, &body).await?;
    Ok(respond(status, &content_type, bytes, &choice))
}

/// A collection id from the path: Vultr's ids are short lowercase names; anything else is
/// refused before it reaches a URL.
fn collection_id(id: &str) -> Result<&str, ApiError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(id)
    } else {
        Err(ApiError::bad(
            "bad_collection",
            "A collection id is letters, digits, '-' or '_'.",
        ))
    }
}

async fn vector_store_create(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    vector_store(&gw, &headers, "vector_store".to_owned(), body).await
}

async fn vector_store_items(
    State(gw): State<Gateway>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let id = collection_id(&id)?;
    vector_store(&gw, &headers, format!("vector_store/{id}/items"), body).await
}

async fn vector_store_search(
    State(gw): State<Gateway>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let id = collection_id(&id)?;
    vector_store(&gw, &headers, format!("vector_store/{id}/search"), body).await
}

async fn decide(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Result<Response, ApiError> {
    let caller = admit(&gw, &headers, &[Purpose::Decide]).await?;
    object(&mut body)?;
    let choice = choose(&gw, &caller, &headers, &body).await?;
    match (choice.provider, choice.endpoint) {
        (Provider::Vultr, Endpoint::SystemOne) => {
            // On Vultr, systemone: the caller may name clef-flash for bulk decisions.
            let named = body["model"].as_str().map(str::to_owned);
            let model = match named {
                None => choice.model.clone(),
                Some(m) if crate::route::DECISION_MODELS.contains(&m.as_str()) => m,
                Some(m) => {
                    return Err(ApiError::bad(
                        "not_a_decision_model",
                        format!(
                            "{m:?} is not a decision model; use one of {:?}.",
                            crate::route::DECISION_MODELS
                        ),
                    ));
                }
            };
            let choice = Choice { model, ..choice };
            object(&mut body)?.insert("model".to_owned(), json!(choice.model));
            let (status, content_type, bytes) =
                vultr_forward(&gw, &caller, Purpose::Decide, &choice, &body).await?;
            Ok(respond(status, &content_type, bytes, &choice))
        }
        (Provider::Anthropic, _) | (Provider::Vultr, _) => {
            let schema = claude::decide_schema(&body).map_err(|e| claude_error(&gw, e))?;
            let system = claude::decide_system(&body).map_err(|e| claude_error(&gw, e))?;
            let user =
                json!({ "state": body["state"], "questions": body["questions"] }).to_string();
            let key = json!({ "decide": { "state": body["state"], "questions": body["questions"] }, "model": choice.model });
            let completion = match choice.provider {
                Provider::Anthropic => {
                    let request = claude::structured_request(
                        &choice.model,
                        adk_core::Effort::Low,
                        &system,
                        user,
                        schema,
                    );
                    claude_completion(&gw, &caller, Purpose::Decide, &choice, Some(&key), request)
                        .await?
                }
                Provider::Vultr => {
                    let request = json!({
                        "model": choice.model,
                        "reasoning_effort": "low",
                        "messages": [
                            { "role": "system", "content": system },
                            { "role": "user", "content": user }
                        ],
                        "response_format": { "type": "json_schema", "json_schema": { "name": "decision", "strict": true, "schema": schema } }
                    });
                    vultr_json(&gw, &caller, Purpose::Decide, &choice, &request).await?
                }
            };
            let text = completion["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default();
            let reply: Value = claude::json_object(text).ok_or_else(|| {
                tracing::warn!(reply = %text.chars().take(200).collect::<String>(), "decide reply was not readable");
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "decide_failed",
                    "The model's decision was not readable; treat the item as risky.",
                )
            })?;
            let answers =
                claude::decide_answers(&body, &reply).map_err(|e| claude_error(&gw, e))?;
            let out = json!({
                "model": choice.model,
                "provider": choice.provider.as_str(),
                "answers": answers,
                "usage": { "input_tokens": completion["usage"]["prompt_tokens"], "output_tokens": completion["usage"]["completion_tokens"] },
            });
            Ok(respond(
                StatusCode::OK,
                "application/json",
                Bytes::from(out.to_string()),
                &choice,
            ))
        }
    }
}

#[derive(Debug, Deserialize)]
struct ScreenRequest {
    text: String,
}

/// The safety model's verdict on one text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenVerdict {
    pub safe: bool,
    pub user_safety: Option<String>,
    pub categories: Vec<String>,
    pub model: String,
    pub raw: String,
}

async fn screen(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let caller = admit(&gw, &headers, &[Purpose::Screen]).await?;
    let request: ScreenRequest = serde_json::from_value(body.clone())
        .map_err(|_| ApiError::bad("bad_body", "Send {\"text\": \"…\"}."))?;
    let choice = choose(&gw, &caller, &headers, &body).await?;
    let fail_closed = || {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "screen_failed",
            "The safety model gave no verdict, so treat the text as unsafe.",
        )
    };
    let verdict = if choice.model == "nemotron-3.5-content-safety" {
        let chat = json!({
            "model": choice.model,
            "max_tokens": Purpose::Screen.default_max_tokens(),
            "reasoning_effort": "low",
            "temperature": 0,
            "messages": [{ "role": "user", "content": request.text }],
        });
        let completion = vultr_json(&gw, &caller, Purpose::Screen, &choice, &chat).await?;
        let raw = completion["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(fail_closed)?;
        parse_verdict(raw, &choice.model)
    } else {
        let user = format!("<text>\n{}\n</text>", request.text);
        let key = json!({ "screen": request.text, "model": choice.model });
        let completion = match choice.provider {
            Provider::Anthropic => {
                let request = claude::structured_request(
                    &choice.model,
                    adk_core::Effort::Low,
                    claude::SCREEN_SYSTEM,
                    user,
                    claude::screen_schema(),
                );
                claude_completion(&gw, &caller, Purpose::Screen, &choice, Some(&key), request)
                    .await?
            }
            Provider::Vultr => {
                let chat = json!({
                    "model": choice.model,
                    "reasoning_effort": "low",
                    "messages": [
                        { "role": "system", "content": claude::SCREEN_SYSTEM },
                        { "role": "user", "content": user }
                    ],
                    "response_format": { "type": "json_schema", "json_schema": { "name": "screen", "strict": true, "schema": claude::screen_schema() } }
                });
                vultr_json(&gw, &caller, Purpose::Screen, &choice, &chat).await?
            }
        };
        let raw = completion["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(fail_closed)?;
        let parsed: Value = claude::json_object(raw).ok_or_else(fail_closed)?;
        let user_safety = claude::screen_safety(&parsed);
        ScreenVerdict {
            safe: user_safety.as_deref() == Some("safe"),
            user_safety,
            categories: parsed["categories"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect(),
            model: choice.model.clone(),
            raw: raw.trim().to_owned(),
        }
    };
    let bytes = serde_json::to_vec(&verdict).map_err(ApiError::internal)?;
    Ok(respond(
        StatusCode::OK,
        "application/json",
        Bytes::from(bytes),
        &choice,
    ))
}

/// Reads Vultr's safety model: `User Safety: …` and `Safety Categories: …`. Anything but an
/// explicit "safe" is unsafe, so a verdict the gateway cannot read fails closed.
fn parse_verdict(raw: &str, model: &str) -> ScreenVerdict {
    let mut user_safety = None;
    let mut categories = Vec::new();
    // On Vultr the model reasons first, in `<think>…</think>`, and may write "User Safety: …"
    // lines while it reasons; its answer follows the last `</think>`, often on the same line.
    // Only the answer counts.
    // Reasoning that never closed (cut off by the token cap) has no answer: fail closed, never
    // read a verdict from inside it.
    let answer = match raw.rsplit_once("</think>") {
        Some((_, after)) => after,
        None if raw.contains("<think>") => "",
        None => raw,
    };
    for line in answer.lines() {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "user safety" => user_safety = Some(value.trim().to_ascii_lowercase()),
            "safety categories" => {
                categories = value
                    .split(',')
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            _ => {}
        }
    }
    ScreenVerdict {
        safe: user_safety.as_deref() == Some("safe"),
        user_safety,
        categories,
        model: model.to_owned(),
        raw: raw.trim().to_owned(),
    }
}

async fn status(State(gw): State<Gateway>) -> Json<Value> {
    let providers = gw
        .inner
        .status
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let config = gw.config();
    let routes: serde_json::Map<String, Value> = config
        .routing
        .table()
        .into_iter()
        .map(|(purpose, choice)| {
            let entry = json!({
                "provider": choice.provider.as_str(),
                "model": choice.model,
                "tier": choice.tier.map(Tier::as_str),
            });
            (purpose.as_str().to_owned(), entry)
        })
        .collect();
    Json(json!({
        "provider": config.routing.provider.as_str(),
        "providers": providers,
        "mode": config.mode.as_str(),
        "routes": routes,
        "daily_cap_usd": config.daily_cap_micro_usd as f64 / 1e6,
    }))
}

mod media;

#[cfg(test)]
mod tests;
