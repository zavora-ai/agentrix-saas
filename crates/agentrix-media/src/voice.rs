//! Talk to your Chief WebAgent (07 §3.6; adk-surface's voice rules; 02 §6's cascade).
//!
//! On the platform host, for the owner of `{site}`:
//! - `GET  /api/voice/{site}/live`: a WebSocket. The browser sends the owner's speech as binary
//!   PCM16 at 16 kHz while they hold to talk, then `{"type":"commit"}`; it gets back
//!   `{type:"ready"}`, `{type:"partial", text}` as words are heard, `{type:"final", text}` at
//!   the end of the turn, and `{type:"error"|"closed", message}`. Node relays to the Model
//!   Gateway's `/v1/realtime`, which holds the socket to Vultr: the browser never sees a key,
//!   and no audio is kept anywhere.
//! - `POST /api/voice/{site}/speak` `{text}`: the Chief's words as speech, streamed (Vultr's SSE
//!   of base64 PCM at 24 kHz), so the mark's `speaking` ring follows the real level.
//! - `GET  /api/voice/{site}/brief`: the latest brief as words to read aloud.
//! - `POST /api/voice/{site}/turn` `{said, card?}`: what to do with what was said. Decisions use
//!   a fixed grammar ([`crate::intent`]), never a model. "Stop everything" acts at once; a
//!   spoken decline waits 3 seconds for a cancel; approving always opens the approval sheet on
//!   the screen, because voice never approves money, publishing or anything high-risk, and every
//!   card in Horizon 1 is bound to a passkey. Anything else is a question for the Chief, answered
//!   by W1's `agentrix_kernel::chief` from the site's own records. Each spoken answer leaves a note (`voice_notes`): what was heard and
//!   what it did, never the recording.

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message as BrowserMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as GatewayMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use crate::intent::{self, Intent};
use crate::routes::{Failure, MediaState, internal, owner_of};

/// The Chief's speaking voice (one of qwen3-tts's built-in speakers) and its manner.
const CHIEF_VOICE: &str = "aiden";
const CHIEF_MANNER: &str = "Calm, clear and warm, like a trusted colleague reading a short note.";

/// How long a spoken decline waits for a cancel.
const CANCEL_MS: u64 = 3_000;

/// The voice routes.
pub fn voice_router(state: MediaState) -> Router {
    Router::new()
        .route("/api/voice/{site}/live", get(live))
        .route("/api/voice/{site}/speak", post(speak))
        .route("/api/voice/{site}/brief", get(brief))
        .route("/api/voice/{site}/turn", post(turn))
        .route("/api/voice/{site}/decide", post(decide))
        .with_state(state)
}

/// A WebSocket carries cookies from any page, so only the platform's own pages may open one.
fn same_origin(state: &MediaState, headers: &HeaderMap) -> Result<(), Failure> {
    let ours = state.ctx().origin.origin().ascii_serialization();
    let theirs = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if theirs == ours {
        Ok(())
    } else {
        Err(Failure(
            StatusCode::FORBIDDEN,
            "wrong_origin",
            "Live voice opens only from your console.".into(),
        ))
    }
}

type GatewaySocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens the gateway's live session for `site`, or says in plain words why not.
async fn open_gateway(state: &MediaState, site: &str) -> Result<GatewaySocket, Failure> {
    let base = state
        .ctx()
        .models_url
        .trim_end_matches('/')
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    let mut request = format!("{base}/v1/realtime")
        .into_client_request()
        .map_err(internal)?;
    let headers = request.headers_mut();
    headers.insert(
        "x-agentrix-site",
        HeaderValue::from_str(site).map_err(internal)?,
    );
    headers.insert("x-agentrix-purpose", HeaderValue::from_static("voice.live"));
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body: Value = response
                .body()
                .as_deref()
                .and_then(|b| serde_json::from_slice(b).ok())
                .unwrap_or(Value::Null);
            let message = body["error"]["message"]
                .as_str()
                .unwrap_or("Live voice isn't available right now. Type instead.")
                .to_owned();
            Err(Failure(status, "voice_unavailable", message))
        }
        Err(err) => {
            tracing::warn!(error = %err, "could not reach the gateway's live voice");
            Err(Failure(
                StatusCode::BAD_GATEWAY,
                "voice_unavailable",
                "Live voice isn't available right now. Type instead.".into(),
            ))
        }
    }
}

async fn live(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    same_origin(&state, &headers)?;
    owner_of(&state, &headers, &site).await?;
    let gateway = open_gateway(&state, &site).await?;
    Ok(upgrade.on_upgrade(move |browser| relay(browser, gateway)))
}

/// What the browser sees of a gateway event, or `None` to drop it.
fn for_browser(event: &Value) -> Option<Value> {
    Some(match event["type"].as_str()? {
        "session.updated" => json!({ "type": "ready" }),
        "conversation.item.input_audio_transcription.delta" => {
            json!({ "type": "partial", "text": event["delta"].as_str().unwrap_or_default() })
        }
        "conversation.item.input_audio_transcription.completed" => {
            json!({ "type": "final", "text": event["transcript"].as_str().unwrap_or_default().trim() })
        }
        "conversation.item.input_audio_transcription.failed" => json!({
            "type": "error",
            "message": "That didn't come through. Hold to talk and say it again."
        }),
        "error" => json!({
            "type": "error",
            "message": "Live voice stopped. Hold to talk to try again, or type."
        }),
        "agentrix.closed" => json!({
            "type": "closed",
            "message": event["message"].as_str().unwrap_or("The microphone closed.")
        }),
        _ => return None,
    })
}

async fn relay(browser: WebSocket, gateway: GatewaySocket) {
    let (mut to_browser, mut from_browser) = browser.split();
    let (mut to_gateway, mut from_gateway) = gateway.split();
    loop {
        tokio::select! {
            message = from_browser.next() => {
                let Some(Ok(message)) = message else { break };
                let forward = match message {
                    BrowserMessage::Binary(pcm) => GatewayMessage::binary(pcm),
                    BrowserMessage::Text(text) => {
                        let Ok(event) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                        let kind = match event["type"].as_str() {
                            Some("commit") => "input_audio_buffer.commit",
                            Some("clear") => "input_audio_buffer.clear",
                            _ => continue,
                        };
                        GatewayMessage::text(json!({ "type": kind }).to_string())
                    }
                    BrowserMessage::Close(_) => break,
                    _ => continue,
                };
                if to_gateway.send(forward).await.is_err() {
                    break;
                }
            }
            message = from_gateway.next() => {
                let Some(Ok(message)) = message else { break };
                let GatewayMessage::Text(text) = message else {
                    if matches!(message, GatewayMessage::Close(_)) { break; }
                    continue;
                };
                let Ok(event) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                if let Some(out) = for_browser(&event)
                    && to_browser.send(BrowserMessage::Text(out.to_string().into())).await.is_err()
                {
                    break;
                }
            }
        }
    }
    let _ = to_gateway.close().await;
    let _ = to_browser.close().await;
}

#[derive(Debug, Deserialize)]
struct SpeakBody {
    text: String,
}

async fn speak(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<SpeakBody>,
) -> Result<Response, Failure> {
    owner_of(&state, &headers, &site).await?;
    let text: String = body.text.trim().chars().take(4_000).collect();
    if text.is_empty() {
        return Err(Failure(
            StatusCode::BAD_REQUEST,
            "nothing_to_say",
            "There's nothing to read aloud.".into(),
        ));
    }
    let ctx = state.ctx();
    let response = ctx
        .http
        .post(format!(
            "{}/v1/audio/speech",
            ctx.models_url.trim_end_matches('/')
        ))
        .header("x-agentrix-site", &site)
        .header("x-agentrix-purpose", "voice.speak")
        .json(&json!({ "input": text, "voice": CHIEF_VOICE, "instructions": CHIEF_MANNER }))
        .send()
        .await
        .map_err(|_| {
            Failure(
                StatusCode::BAD_GATEWAY,
                "voice_unavailable",
                "The Chief can't speak right now. The words are on screen.".into(),
            )
        })?;
    if !response.status().is_success() {
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or("The Chief can't speak right now. The words are on screen.")
            .to_owned();
        return Err(Failure(status, "voice_unavailable", message));
    }
    let stream = response.bytes_stream();
    let mut reply = Response::new(Body::from_stream(stream));
    let h = reply.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(reply)
}

/// The latest brief as words to say, or `None` if the Chief hasn't written one.
pub async fn brief_words(state: &MediaState, site: &str) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT body FROM briefs WHERE site_id = ? ORDER BY written_at DESC LIMIT 1",
    )
    .bind(site)
    .fetch_optional(&state.ctx().pool)
    .await?;
    let Some((body,)) = row else {
        return Ok(None);
    };
    let brief: agentrix_types::records::Brief = serde_json::from_str(&body)?;
    let mut said = Vec::new();
    if let Some(headline) = brief.headline.filter(|h| !h.trim().is_empty()) {
        said.push(headline);
    }
    said.extend(
        brief
            .lines
            .into_iter()
            .map(|l| l.text)
            .filter(|t| !t.trim().is_empty()),
    );
    if let Some(spend) = brief.spend_words {
        said.push(spend);
    }
    Ok((!said.is_empty()).then(|| {
        said.iter()
            .map(|s| {
                let s = s.trim();
                if s.ends_with(['.', '?', '!']) {
                    s.to_owned()
                } else {
                    format!("{s}.")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }))
}

const NO_BRIEF: &str =
    "There's no brief yet. I write one each morning from what your WebAgents did.";

async fn brief(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    owner_of(&state, &headers, &site).await?;
    let words = brief_words(&state, &site).await.map_err(internal)?;
    Ok(Json(
        json!({ "text": words.as_deref().unwrap_or(NO_BRIEF), "written": words.is_some() }),
    ))
}

/// The card the owner is talking about, as the console shows it.
#[derive(Debug, Clone, Deserialize)]
struct Card {
    id: String,
    title: String,
    /// `ok`, `wait` or `risk` (the lane's chip class), or low / medium / high.
    #[serde(default)]
    risk: String,
    /// "Mover asks".
    #[serde(default)]
    from: String,
    /// "Low risk", "High risk".
    #[serde(default)]
    risk_words: String,
}

impl Card {
    fn high_risk(&self) -> bool {
        matches!(self.risk.as_str(), "risk" | "high")
    }

    /// "Cashier asks · Low risk", for the voice decision card.
    fn meta(&self) -> String {
        [self.from.as_str(), self.risk_words.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// A waiting v2 card as voice sees it.
struct SpokenCard {
    card: Card,
    /// Low risk and one-tap: "allow once" may be spoken (07 §5.3).
    one_tap: bool,
}

/// The site's waiting v2 cards, newest first. Any error reads as none: voice then sends
/// decisions to the screen.
async fn spoken_cards(state: &MediaState, site: &str) -> Vec<SpokenCard> {
    use agentrix_decisions::{CardState, RiskLevel};
    let rows = match state.decisions().list(site).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::debug!(error = %err, "no v2 cards for voice");
            return Vec::new();
        }
    };
    rows.into_iter()
        .filter(|r| r.state == CardState::Waiting)
        .map(|r| {
            let level = match r.card.risk.level {
                RiskLevel::Low => "low",
                RiskLevel::Medium => "medium",
                RiskLevel::High => "high",
            };
            let reasons = r.card.risk.reasons.join(", ");
            let risk_words = match (level, reasons.is_empty()) {
                ("low", true) => "Low risk".to_owned(),
                ("low", false) => format!("Low risk: {reasons}"),
                ("medium", true) => "Medium risk".to_owned(),
                ("medium", false) => format!("Medium risk: {reasons}"),
                (_, true) => "High risk".to_owned(),
                (_, false) => format!("High risk: {reasons}"),
            };
            SpokenCard {
                // W5's one rule table for every surface: voice may allow once only an everyday
                // card (low risk, one tap, no money, not publishing, not unfreezing).
                one_tap: agentrix_decisions::surface::powers(
                    agentrix_decisions::surface::Surface::Voice,
                    &r.card,
                )
                .allow_once,
                card: Card {
                    id: r.card.id.clone(),
                    title: r.card.title.clone(),
                    risk: level.to_owned(),
                    from: format!("{} asks", capitalise(&r.card.raised_by.webagent)),
                    risk_words,
                },
            }
        })
        .collect()
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Deserialize)]
struct DecideBody {
    card: String,
    allow: bool,
    #[serde(default)]
    heard: String,
}

/// Runs a spoken answer once its 3 seconds have passed: W5's decide_spoken, which allows only
/// low-risk one-tap cards, records the approval as `spoken` and never counts it as reviewed.
async fn decide(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DecideBody>,
) -> Result<Json<Value>, Failure> {
    let account = owner_of(&state, &headers, &site).await?;
    let heard: String = body.heard.trim().chars().take(300).collect();
    let decisions = state.decisions();
    let outcome = decisions
        .decide_spoken(&site, &account, &body.card, body.allow, "voice", &heard)
        .await;
    let intent = if body.allow {
        Intent::AllowOnce
    } else {
        Intent::Deny
    };
    let fake = Card {
        id: body.card.clone(),
        title: String::new(),
        risk: String::new(),
        from: String::new(),
        risk_words: String::new(),
    };
    match outcome {
        Ok(receipt) => {
            let did = if body.allow {
                "allowed once"
            } else {
                "declined"
            };
            note(&state, &site, &heard, intent, Some(&fake), did).await;
            let reply = if body.allow {
                "Done. I allowed it once. The receipt says you approved it by voice."
            } else {
                "Declined. Nothing changed."
            };
            Ok(Json(json!({ "reply": reply, "receipt": receipt })))
        }
        Err(refusal) => {
            note(&state, &site, &heard, intent, Some(&fake), "refused").await;
            let status =
                StatusCode::from_u16(refusal.status.as_u16()).unwrap_or(StatusCode::CONFLICT);
            Err(Failure(status, "not_decided", refusal.words))
        }
    }
}

#[derive(Debug, Deserialize)]
struct TurnBody {
    said: String,
    #[serde(default)]
    card: Option<Card>,
}

async fn note(
    state: &MediaState,
    site: &str,
    said: &str,
    intent: Intent,
    card: Option<&Card>,
    did: &str,
) {
    let result = sqlx::query(
        "INSERT INTO voice_notes (site_id, heard, intent, card_id, did, at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(site)
    .bind(said.chars().take(500).collect::<String>())
    .bind(serde_json::to_value(intent).ok().and_then(|v| v.as_str().map(str::to_owned)))
    .bind(card.map(|c| c.id.as_str()))
    .bind(did)
    .bind(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
    .execute(&state.ctx().pool)
    .await;
    if let Err(err) = result {
        tracing::error!(error = %err, "could not keep a voice note");
    }
}

async fn turn(
    State(state): State<MediaState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(body): Json<TurnBody>,
) -> Result<Json<Value>, Failure> {
    owner_of(&state, &headers, &site).await?;
    let said = body.said.trim();
    let heard = intent::read(said);
    // Cards v2 (W5's agentrix-decisions) can be answered by voice through decide_spoken; the
    // card in focus is the one at the top of the lane, else the first waiting v2 card.
    let waiting = spoken_cards(&state, &site).await;
    let fallback = waiting.first().map(|w| w.card.clone());
    let card = body.card.as_ref().or(fallback.as_ref());
    let v2 = card.and_then(|c| waiting.iter().find(|w| w.card.id == c.id));
    let nothing = "Nothing is waiting for you to decide.";
    let (reply, action, did) = match (heard, card) {
        (Intent::Stop, _) => (
            "Stopping every WebAgent now. Your site keeps serving its signed release.".to_owned(),
            json!({ "do": "freeze" }),
            "freeze",
        ),
        (Intent::Cancel, _) => (
            "Cancelled. Nothing changed.".to_owned(),
            json!({ "do": "cancel" }),
            "cancel",
        ),
        (Intent::AllowOnce | Intent::Deny, None) => (nothing.to_owned(), Value::Null, "none"),
        (Intent::AllowOnce, Some(c)) if v2.is_some_and(|w| w.one_tap) => (
            format!(
                "Allowing {} once, in 3 seconds. Say cancel to stop it.",
                c.title
            ),
            json!({ "do": "allow", "via": "decide", "card": c.id, "title": c.title, "meta": c.meta(), "after_ms": CANCEL_MS }),
            "allow once after 3 s",
        ),
        (Intent::AllowOnce, Some(c)) if c.high_risk() => (
            format!(
                "{} is high risk, so voice can't approve it. I've opened it on your screen.",
                c.title
            ),
            json!({ "do": "open_sheet", "card": c.id }),
            "sent to screen",
        ),
        (Intent::AllowOnce, Some(c)) => (
            format!(
                "Approving {} needs your passkey, so I've opened it on your screen.",
                c.title
            ),
            json!({ "do": "open_sheet", "card": c.id }),
            "sent to screen",
        ),
        (Intent::Deny, Some(c)) => (
            format!("Declining {} in 3 seconds. Say cancel to keep it.", c.title),
            json!({
                "do": "decline",
                "via": if v2.is_some() { "decide" } else { "form" },
                "card": c.id, "title": c.title, "meta": c.meta(), "after_ms": CANCEL_MS
            }),
            "decline after 3 s",
        ),
        (Intent::Brief, _) => (
            brief_words(&state, &site)
                .await
                .map_err(internal)?
                .unwrap_or_else(|| NO_BRIEF.to_owned()),
            Value::Null,
            "read brief",
        ),
        (Intent::Next, Some(c)) => {
            let from = if c.from.is_empty() {
                "A WebAgent asks".to_owned()
            } else {
                c.from.clone()
            };
            let risk = if c.risk_words.is_empty() {
                String::new()
            } else {
                format!(" {}.", c.risk_words.trim_end_matches('.'))
            };
            let ask = if v2.is_some_and(|w| w.one_tap) || (v2.is_none() && !c.high_risk()) {
                "Say allow once, or deny."
            } else {
                "This one needs your screen to approve; you can still say deny."
            };
            (
                format!("{from}: {}.{risk} {ask}", c.title.trim_end_matches('.')),
                Value::Null,
                "read card",
            )
        }
        (Intent::Next, None) => (
            "Nothing needs you right now.".to_owned(),
            Value::Null,
            "read card",
        ),
        (Intent::Ask, _) => (
            ask_chief(&state, &site, said).await,
            Value::Null,
            "answered",
        ),
    };
    note(&state, &site, said, heard, card, did).await;
    Ok(Json(
        json!({ "heard": said, "intent": heard, "reply": reply, "action": action }),
    ))
}

/// The Chief WebAgent's answer to a spoken question (W1's `agentrix_kernel::chief`): at most
/// two short sentences ready to speak, from the site's own records, with its own dial check and
/// tools. Its text is never empty (a refusal or a failure has its own words).
async fn ask_chief(state: &MediaState, site: &str, said: &str) -> String {
    agentrix_kernel::chief::answer(
        state.ctx(),
        site,
        said,
        agentrix_kernel::chief::Surface::Voice,
    )
    .await
    .text
}

/// The voice panel's script and styles, served on the platform host.
pub fn assets_router() -> Router {
    Router::new()
        .route(
            "/static/media/voice.js",
            get(|| async {
                asset(
                    include_str!("../static/voice.js"),
                    "text/javascript; charset=utf-8",
                )
            }),
        )
        .route(
            "/static/media/voice.css",
            get(|| async {
                asset(
                    include_str!("../static/voice.css"),
                    "text/css; charset=utf-8",
                )
            }),
        )
        .route(
            "/static/media/pcm-capture.js",
            get(|| async {
                asset(
                    include_str!("../static/pcm-capture.js"),
                    "text/javascript; charset=utf-8",
                )
            }),
        )
        .route(
            "/static/media/media.css",
            get(|| async {
                asset(
                    include_str!("../static/media.css"),
                    "text/css; charset=utf-8",
                )
            }),
        )
}

fn asset(body: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        body,
    )
        .into_response()
}
