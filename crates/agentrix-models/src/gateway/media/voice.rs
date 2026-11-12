//! Speech, as 02 §6's cascade (Vultr's API has no speech-to-speech):
//!
//! - `POST /v1/audio/transcriptions` (`voice.listen`): a recorded clip, `{input_audio: {data,
//!   format}, language?, prompt?}`, to `whisper-large-v3-turbo`. Answers `{text, usage}`.
//! - `POST /v1/audio/speech` (`voice.speak`): `{input, voice?, instructions?, language?}` to
//!   `qwen3-tts`. Always streamed back as Vultr's SSE (`speech.audio.delta` events carrying
//!   base64 PCM, 24 kHz, 16-bit mono, then `speech.audio.done`), so the console can play it as
//!   it is made and drive the mark's `speaking` ring from the real level.
//! - `GET /v1/realtime` (`voice.live`): a WebSocket that node opens for the owner's live
//!   speech. The gateway holds the one to Vultr (`/v1/realtime?intent=transcription`, server to
//!   server only, so the key never reaches a browser), configures it (PCM16 at 16 kHz, no
//!   server turn detection: the client commits each turn), passes on only audio and turn
//!   events, and returns transcription events. A subscription holds 5 sessions per model, so
//!   the gateway holds at most `AGENTRIX_REALTIME_SESSIONS` (default 5) and answers 429 past
//!   that. A session closes after 2 minutes without audio or 10 minutes in all.
//!
//! All three bill by seconds of audio, rounded up as Vultr bills them: speech by what was
//! spoken, transcription by what was heard, and a live session by the audio sent, silence
//! included. Recordings and replays work for the first two; a live session is never recorded,
//! and its tests run against a fake Vultr.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::extract::ws::{Message as ClientMessage, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use base64::Engine as _;
use bytes::Bytes;
use futures::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::tungstenite::Message as UpstreamMessage;

use super::super::{ApiError, Gateway, admit, recorded, respond, save, upstream_error};
use super::{Inline, media_forward};
use crate::cache::Mode;
use crate::meter::{MediaUnits, Outcome};
use crate::route::{Choice, Endpoint, Purpose, Tier, media_cost_micro_usd};

/// Vultr's limit for one clip, decoded.
const MAX_CLIP_BYTES: usize = 25 * 1024 * 1024;

/// The most text spoken in one call, in characters.
const MAX_SPOKEN_CHARS: usize = 4_000;

/// Live sessions close after this long without audio, to free one of the 5 places.
const IDLE: Duration = Duration::from_secs(120);

/// And after this long in all.
const LONGEST: Duration = Duration::from_secs(600);

/// The live audio format: PCM16, mono, little endian, 16 kHz (Vultr also takes 24 kHz).
const LIVE_RATE: u64 = 16_000;

pub(super) fn routes() -> Router<Gateway> {
    Router::new()
        .route("/v1/audio/transcriptions", post(transcriptions))
        .route("/v1/audio/speech", post(speech))
        .route("/v1/realtime", get(realtime))
}

fn seconds_units(choice: &Choice, seconds: f64) -> MediaUnits<'static> {
    MediaUnits {
        units: seconds.max(0.0).ceil() as i64,
        unit: "audio_second",
        cost_micro_usd: media_cost_micro_usd(&choice.model, seconds),
    }
}

/// The audio formats a clip may be in.
const CLIP_FORMATS: [&str; 7] = ["wav", "mp3", "flac", "m4a", "ogg", "webm", "aac"];

async fn transcriptions(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let caller = admit(&gw, &headers, &[Purpose::VoiceListen]).await?;
    let choice = gw
        .config()
        .routing
        .choose(Purpose::VoiceListen, Tier::Fast, "specialist");
    let format = body["input_audio"]["format"].as_str().unwrap_or_default();
    if !CLIP_FORMATS.contains(&format) {
        return Err(ApiError::bad(
            "bad_format",
            format!("Say the clip's format in input_audio.format: one of {CLIP_FORMATS:?}."),
        ));
    }
    let data = body["input_audio"]["data"].as_str().unwrap_or_default();
    let clip = Inline::parse(
        &format!("data:audio/{format};base64,{data}"),
        &[&format!("audio/{format}")],
        MAX_CLIP_BYTES,
    )
    .map_err(|m| ApiError::bad("bad_audio", m))?;
    let mut request = json!({
        "model": choice.model,
        "input_audio": { "data": data.trim(), "format": format },
        "response_format": "json",
    });
    let mut key = json!({ "model": choice.model, "audio": clip.digest(), "format": format });
    for field in ["language", "prompt"] {
        if let Some(value) = body[field].as_str() {
            request[field] = json!(value);
            key[field] = json!(value);
        }
    }
    let (status, content_type, bytes) = media_forward(
        &gw,
        &caller,
        &choice,
        Endpoint::Transcriptions.path(),
        &key,
        &request,
        |choice, answer| {
            let seconds = answer["usage"]["seconds"].as_f64()?;
            Some(seconds_units(choice, seconds))
        },
    )
    .await?;
    Ok(respond(status, &content_type, bytes, &choice))
}

/// Finds the end of a speech stream (`speech.audio.done`) and its usage, across chunks.
#[derive(Default)]
struct SpeechScan {
    line: Vec<u8>,
    seconds: Option<f64>,
    cost_usd: Option<f64>,
    failed: bool,
}

impl SpeechScan {
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
        let Ok(event) = serde_json::from_slice::<Value>(data.trim_ascii()) else {
            return;
        };
        match event["type"].as_str() {
            Some("speech.audio.done") => {
                let usage = &event["usage"];
                self.seconds = ["seconds", "output_seconds", "audio_seconds", "duration"]
                    .iter()
                    .find_map(|k| usage[*k].as_f64())
                    .or(Some(0.0));
                // Vultr sends the cost as a string ("0.00112", seen live 8 Nov 2026).
                self.cost_usd = usage["cost"]
                    .as_f64()
                    .or_else(|| usage["cost"].as_str().and_then(|c| c.trim().parse().ok()));
            }
            Some("speech.audio.error") => self.failed = true,
            _ => {}
        }
    }

    fn units(&self, choice: &Choice) -> Option<MediaUnits<'static>> {
        let seconds = self.seconds?;
        let mut units = seconds_units(choice, seconds);
        if let Some(usd) = self.cost_usd.filter(|c| c.is_finite() && *c >= 0.0) {
            units.cost_micro_usd = (usd * 1e6).ceil() as i64;
        }
        Some(units)
    }
}

async fn speech(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let caller = admit(&gw, &headers, &[Purpose::VoiceSpeak]).await?;
    let choice = gw
        .config()
        .routing
        .choose(Purpose::VoiceSpeak, Tier::Fast, "specialist");
    let input = body["input"].as_str().map(str::trim).unwrap_or_default();
    if input.is_empty() || input.chars().count() > MAX_SPOKEN_CHARS {
        return Err(ApiError::bad(
            "bad_input",
            format!("Send the words to speak in \"input\", up to {MAX_SPOKEN_CHARS} characters."),
        ));
    }
    let mut request = json!({
        "model": choice.model,
        "input": input,
        "response_format": "pcm",
        "stream_format": "sse",
    });
    for field in ["voice", "instructions", "language"] {
        if let Some(value) = body[field].as_str().filter(|v| !v.trim().is_empty()) {
            request[field] = json!(value);
        }
    }
    if let Some(recording) = recorded(&gw, caller.purpose.as_str(), &choice.model, &request).await?
    {
        let bytes = recording.bytes();
        let mut scan = SpeechScan::default();
        scan.feed(&bytes);
        gw.record_media(&caller, &choice, scan.units(&choice), Outcome::Replayed)
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
    let response = match vultr.post(Endpoint::Speech.path(), &request).await {
        Ok(response) => response,
        Err(err) => {
            gw.record_media(&caller, &choice, None, Outcome::Error)
                .await;
            return Err(upstream_error(&gw, err));
        }
    };
    gw.set_status("ok", "Providers are answering.".to_owned());
    let status = response.status();
    if !status.is_success() {
        let bytes = response.bytes().await.unwrap_or_default();
        gw.record_media(&caller, &choice, None, Outcome::Error)
            .await;
        return Ok(respond(status, "application/json", bytes, &choice));
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let mut upstream = response.bytes_stream();
    let mut reply = Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(ApiError::internal)?;
    let template = respond(status, "text/event-stream", Bytes::new(), &choice);
    for (name, value) in template.headers() {
        reply.headers_mut().insert(name.clone(), value.clone());
    }
    tokio::spawn(async move {
        let recording = gw.config().mode == Mode::Record;
        let mut kept = Vec::new();
        let mut complete = true;
        let mut scan = SpeechScan::default();
        while let Some(chunk) = upstream.next().await {
            match chunk {
                Ok(bytes) => {
                    scan.feed(&bytes);
                    if recording {
                        kept.extend_from_slice(&bytes);
                    }
                    if tx.send(Ok(bytes)).await.is_err() {
                        // The listener left; Vultr still bills what it made.
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
        let units = scan.units(&choice);
        let outcome = if units.is_some() && !scan.failed {
            Outcome::Ok
        } else {
            Outcome::Error
        };
        gw.record_media(&caller, &choice, units, outcome).await;
        if recording && complete && outcome == Outcome::Ok {
            save(
                &gw,
                caller.purpose.as_str(),
                &choice.model,
                Endpoint::Speech.path(),
                &request,
                StatusCode::OK,
                "text/event-stream",
                &kept,
            )
            .await;
        }
    });
    Ok(reply)
}

/// The places for live sessions, shared by every caller (Vultr's limit is per subscription).
fn live_slots() -> &'static Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        let n = std::env::var("AGENTRIX_REALTIME_SESSIONS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| (1..=50).contains(n))
            .unwrap_or(5);
        Arc::new(Semaphore::new(n))
    })
}

/// The client events passed on to Vultr; anything else (a session change, a response
/// request) is dropped, so a caller cannot reconfigure the session.
const CLIENT_EVENTS: [&str; 3] = [
    "input_audio_buffer.append",
    "input_audio_buffer.commit",
    "input_audio_buffer.clear",
];

async fn realtime(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let caller = admit(&gw, &headers, &[Purpose::VoiceLive]).await?;
    let choice = gw
        .config()
        .routing
        .choose(Purpose::VoiceLive, Tier::Fast, "specialist");
    let Some(vultr) = gw.config().vultr.as_ref() else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_live_voice",
            "Live voice needs the gateway in live or record mode; replay has no recordings of it. Type instead.",
        ));
    };
    let permit = live_slots().clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "voice_busy",
            "Every live voice place is in use right now. Try again in a minute, or type.",
        )
    })?;
    let upstream = match vultr.realtime(&choice.model).await {
        Ok(upstream) => upstream,
        Err(err) => {
            gw.record_media(&caller, &choice, None, Outcome::Error)
                .await;
            return Err(upstream_error(&gw, err));
        }
    };
    gw.set_status("ok", "Providers are answering.".to_owned());
    Ok(upgrade.on_upgrade(move |client| async move {
        let sent_bytes = relay(client, upstream, &choice).await;
        drop(permit);
        let seconds = sent_bytes as f64 / (LIVE_RATE as f64 * 2.0);
        gw.record_media(
            &caller,
            &choice,
            Some(seconds_units(&choice, seconds)),
            Outcome::Ok,
        )
        .await;
    }))
}

type Upstream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Relays one live session until either side closes, a limit is reached, or Vultr ends it.
/// Returns how many bytes of audio were sent to Vultr (what it bills).
async fn relay(client: WebSocket, upstream: Upstream, choice: &Choice) -> u64 {
    let (mut to_client, mut from_client) = client.split();
    let (mut to_vultr, mut from_vultr) = upstream.split();
    let configure = json!({
        "type": "session.update",
        "session": { "type": "transcription", "audio": { "input": {
            "format": { "type": "audio/pcm", "rate": LIVE_RATE },
            "transcription": { "model": choice.model },
            "turn_detection": null
        }}}
    });
    if to_vultr
        .send(UpstreamMessage::text(configure.to_string()))
        .await
        .is_err()
    {
        return 0;
    }
    let mut sent: u64 = 0;
    let started = tokio::time::Instant::now();
    let mut last_audio = started;
    loop {
        let idle_left = IDLE.saturating_sub(last_audio.elapsed());
        let total_left = LONGEST.saturating_sub(started.elapsed());
        tokio::select! {
            message = from_client.next() => {
                let Some(Ok(message)) = message else { break };
                let forward = match message {
                    ClientMessage::Binary(pcm) => {
                        if pcm.len() % 2 != 0 { continue; }
                        sent += pcm.len() as u64;
                        last_audio = tokio::time::Instant::now();
                        let audio = base64::engine::general_purpose::STANDARD.encode(&pcm);
                        json!({ "type": "input_audio_buffer.append", "audio": audio }).to_string()
                    }
                    ClientMessage::Text(text) => {
                        let Ok(event) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                        let kind = event["type"].as_str().unwrap_or_default();
                        if !CLIENT_EVENTS.contains(&kind) { continue; }
                        if kind == "input_audio_buffer.append" {
                            let Some(audio) = event["audio"].as_str() else { continue };
                            sent += (audio.len() as u64 / 4) * 3;
                            last_audio = tokio::time::Instant::now();
                            json!({ "type": kind, "audio": audio }).to_string()
                        } else {
                            json!({ "type": kind }).to_string()
                        }
                    }
                    ClientMessage::Close(_) => break,
                    _ => continue,
                };
                if to_vultr.send(UpstreamMessage::text(forward)).await.is_err() {
                    break;
                }
            }
            message = from_vultr.next() => {
                let Some(Ok(message)) = message else { break };
                match message {
                    UpstreamMessage::Text(text)
                        if to_client.send(ClientMessage::Text(text.as_str().into())).await.is_err() =>
                    {
                        break;
                    }
                    UpstreamMessage::Close(_) => break,
                    _ => {}
                }
            }
            () = tokio::time::sleep(idle_left.min(total_left)) => {
                let message = if total_left.is_zero() {
                    "This voice session reached 10 minutes. Hold to talk again to start a new one."
                } else {
                    "No speech for 2 minutes, so the microphone closed. Hold to talk to start again."
                };
                let _ = to_client
                    .send(ClientMessage::Text(json!({ "type": "agentrix.closed", "message": message }).to_string().into()))
                    .await;
                break;
            }
        }
    }
    let _ = to_vultr.close().await;
    let _ = to_client.close().await;
    sent
}
