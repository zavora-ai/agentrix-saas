//! Images and speech through the gateway, against a fake Vultr on loopback. No test touches
//! the real network.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use super::super::{Config, Gateway, router};
use crate::cache::{Mode, Recordings};
use crate::route::{Provider, Routing};
use crate::upstream::{ApiKey, Vultr};

/// A 1×1 PNG, as Vultr would send it back in `b64_json`.
const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[derive(Clone)]
struct Fake {
    calls: Arc<AtomicUsize>,
    last: Arc<std::sync::Mutex<(String, Value)>>,
}

impl Fake {
    fn saw(&self, path: &str, headers: &HeaderMap, body: &Value) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(headers["authorization"], "Bearer test-key");
        assert!(
            headers.get("x-agentrix-site").is_none(),
            "caller headers never reach Vultr"
        );
        *self.last.lock().unwrap() = (path.to_owned(), body.clone());
    }
}

/// Vultr's image answer (`create-image-response-payload`, 8 Nov 2026).
fn image_answer(body: &Value) -> Value {
    let n = body["n"].as_u64().unwrap_or(1) as usize;
    json!({
        "created": 1_762_560_000,
        "data": vec![json!({ "b64_json": PIXEL, "media_type": "image/png" }); n],
        "size": body["size"].as_str().unwrap_or("1024x1024"),
        "output_format": "png",
        "usage": { "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0, "cost": 0.020972 * n as f64, "megapixels": 1.048576 * n as f64 }
    })
}

async fn fake_generations(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    fake.saw("images/generations", &headers, &body);
    Json(image_answer(&body))
}

async fn fake_edits(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    fake.saw("images/edits", &headers, &body);
    if body["model"] == "z-image-turbo" {
        return (
            StatusCode::BAD_REQUEST,
            "z-image-turbo has no reference editing",
        )
            .into_response();
    }
    let mut answer = image_answer(&body);
    answer["usage"]["cost"] = json!(0);
    Json(answer).into_response()
}

async fn fake_chat(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    fake.saw("chat/completions", &headers, &body);
    let content = if body["model"] == "nemotron-3.5-content-safety" {
        let note = body["messages"][0]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        if note.contains("weapon") {
            "User Safety: unsafe\nSafety Categories: Guns and Illegal Weapons".to_owned()
        } else {
            "User Safety: safe".to_owned()
        }
    } else {
        json!({
            "alt": "A round sourdough loaf with a dark, cracked crust on a wooden board",
            "photographic": true,
            "shows_person": false,
            "shows_text": false,
            "check": ""
        })
        .to_string()
    };
    Json(json!({
        "model": body["model"],
        "choices": [{ "message": { "content": content }, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 900, "completion_tokens": 60 }
    }))
}

async fn fake_transcriptions(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    fake.saw("audio/transcriptions", &headers, &body);
    Json(json!({ "text": "Allow once.", "usage": { "type": "duration", "seconds": 3 } }))
}

/// Vultr's speech stream: base64 PCM deltas, then the usage (8 Nov 2026 docs).
async fn fake_speech(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    fake.saw("audio/speech", &headers, &body);
    let sse = "event: speech.audio.delta\ndata: {\"type\":\"speech.audio.delta\",\"audio\":\"AAABAAIA\"}\n\n\
               event: speech.audio.delta\ndata: {\"type\":\"speech.audio.delta\",\"audio\":\"AwAEAA==\"}\n\n\
               event: speech.audio.done\ndata: {\"type\":\"speech.audio.done\",\"usage\":{\"seconds\":3,\"cost\":\"0.00084\"}}\n\n";
    ([("content-type", "text/event-stream")], sse).into_response()
}

/// Vultr's realtime transcription socket: one turn per commit.
async fn fake_realtime(
    State(fake): State<Fake>,
    headers: HeaderMap,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
    upgrade: WebSocketUpgrade,
) -> Response {
    assert_eq!(headers["authorization"], "Bearer test-key");
    assert!(headers.get("x-agentrix-site").is_none());
    assert_eq!(
        query.as_deref(),
        Some("intent=transcription&model=voxtral-mini-4b-realtime")
    );
    fake.calls.fetch_add(1, Ordering::SeqCst);
    upgrade.on_upgrade(move |mut socket| async move {
        let _ = socket
            .send(WsMessage::Text(json!({ "type": "session.created" }).to_string().into()))
            .await;
        let mut heard = 0usize;
        let mut updates = Vec::new();
        while let Some(Ok(message)) = socket.recv().await {
            let WsMessage::Text(text) = message else { continue };
            let event: Value = serde_json::from_str(text.as_str()).unwrap();
            match event["type"].as_str().unwrap() {
                "session.update" => {
                    updates.push(event.clone());
                    *fake.last.lock().unwrap() = ("realtime".to_owned(), json!(updates));
                    let _ = socket
                        .send(WsMessage::Text(json!({ "type": "session.updated" }).to_string().into()))
                        .await;
                }
                "input_audio_buffer.append" => {
                    use base64::Engine as _;
                    heard += base64::engine::general_purpose::STANDARD
                        .decode(event["audio"].as_str().unwrap())
                        .unwrap()
                        .len();
                }
                "input_audio_buffer.commit" => {
                    for out in [
                        json!({ "type": "input_audio_buffer.committed", "item_id": "item_1" }),
                        json!({ "type": "conversation.item.input_audio_transcription.delta", "delta": "Allow" }),
                        json!({ "type": "conversation.item.input_audio_transcription.completed", "transcript": "Allow once.",
                                "usage": { "type": "duration", "seconds": heard as f64 / 32_000.0 } }),
                    ] {
                        let _ = socket.send(WsMessage::Text(out.to_string().into())).await;
                    }
                }
                other => panic!("Vultr got a client event it should never see: {other}"),
            }
        }
    })
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

/// `usage` as W8's migration 0801 leaves it: no purpose or provider CHECK, and `units` and
/// `unit` for calls billed by units. Replace with 0801 itself once it is on platform.
const USAGE_OPEN: &str = "DROP INDEX usage_site_at; DROP INDEX usage_move; DROP TABLE usage;
CREATE TABLE usage (
    usage_id         INTEGER PRIMARY KEY,
    site_id          TEXT NOT NULL REFERENCES sites (site_id),
    purpose          TEXT NOT NULL,
    provider         TEXT NOT NULL DEFAULT 'vultr',
    model            TEXT NOT NULL,
    tier             TEXT CHECK (tier IN ('fast', 'standard', 'deep')),
    tier_reason      TEXT NOT NULL DEFAULT '',
    move_id          TEXT REFERENCES moves (move_id),
    tokens_in        INTEGER NOT NULL CHECK (tokens_in >= 0),
    tokens_out       INTEGER NOT NULL CHECK (tokens_out >= 0),
    reasoning_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_tokens >= 0),
    cost_micro_usd   INTEGER NOT NULL CHECK (cost_micro_usd >= 0),
    outcome          TEXT NOT NULL CHECK (outcome IN ('ok', 'error', 'over-budget', 'replayed')),
    at               TEXT NOT NULL,
    units            INTEGER,
    unit             TEXT
) STRICT;
CREATE INDEX usage_site_at ON usage (site_id, at);";

async fn database() -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE accounts (id TEXT PRIMARY KEY);")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../agentrix-node/src/db/migrations/0001_control.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(USAGE_OPEN).execute(&pool).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO sites (site_id, domain, old_origin, created_at) \
         VALUES ('site_1', 'bakery.example', 'https://bakery.example', '2026-11-07T00:00:00.000000Z');",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "agentrix-media-{name}-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Rig {
    url: String,
    calls: Arc<AtomicUsize>,
    last: Arc<std::sync::Mutex<(String, Value)>>,
    pool: SqlitePool,
    http: reqwest::Client,
}

async fn rig(mode: Mode, recordings: Option<PathBuf>, pool: Option<SqlitePool>) -> Rig {
    let fake = Fake {
        calls: Arc::new(AtomicUsize::new(0)),
        last: Arc::new(std::sync::Mutex::new((String::new(), Value::Null))),
    };
    let vultr_url = serve(
        Router::new()
            .route("/v1/images/generations", post(fake_generations))
            .route("/v1/images/edits", post(fake_edits))
            .route("/v1/chat/completions", post(fake_chat))
            .route("/v1/audio/transcriptions", post(fake_transcriptions))
            .route("/v1/audio/speech", post(fake_speech))
            .route("/v1/realtime", get(fake_realtime))
            .with_state(fake.clone()),
    )
    .await;
    let pool = match pool {
        Some(pool) => pool,
        None => database().await,
    };
    let vultr = (mode != Mode::Replay).then(|| {
        Vultr::new(format!("{vultr_url}/v1"), ApiKey::for_tests("test-key"))
            .unwrap()
            .with_base_delay(Duration::from_millis(1))
    });
    let gateway = Gateway::new(Config {
        vultr,
        claude: None,
        pool: pool.clone(),
        routing: Routing::on(Provider::Vultr),
        mode,
        recordings: Recordings::new(recordings.unwrap_or_else(|| temp_dir("unused"))),
        daily_cap_micro_usd: 5_000_000,
        shared_caps: crate::meter::SharedCaps {
            platform_micro_usd: 100_000_000,
            samples_micro_usd: 100_000_000,
        },
    })
    .unwrap();
    Rig {
        url: serve(router(gateway)).await,
        calls: fake.calls,
        last: fake.last,
        pool,
        http: reqwest::Client::new(),
    }
}

type Row = (
    String,
    String,
    i64,
    Option<i64>,
    Option<String>,
    String,
    i64,
);

impl Rig {
    async fn post(&self, path: &str, extra: &[(&str, &str)], body: Value) -> reqwest::Response {
        let mut request = self
            .http
            .post(format!("{}{path}", self.url))
            .header("x-agentrix-site", "site_1")
            .header("x-agentrix-purpose", "image");
        for (name, value) in extra {
            request = request.header(*name, *value);
        }
        request.json(&body).send().await.unwrap()
    }

    async fn usage(&self) -> Vec<Row> {
        sqlx::query_as(
            "SELECT purpose, model, cost_micro_usd, units, unit, outcome, tokens_in \
             FROM usage ORDER BY usage_id",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    fn last(&self) -> (String, Value) {
        self.last.lock().unwrap().clone()
    }
}

fn data_uri() -> String {
    format!("data:image/png;base64,{PIXEL}")
}

#[tokio::test]
async fn a_draft_image_is_drawn_fast_and_metered_by_images_made() {
    let rig = rig(Mode::Live, None, None).await;
    let response = rig
        .post(
            "/v1/images/generations",
            &[],
            json!({ "prompt": "A sourdough loaf on a cooling rack, morning light", "size": "1024x1024", "n": 2, "response_format": "url" }),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-agentrix-model"], "z-image-turbo");
    assert_eq!(response.headers()["x-agentrix-tier"], "specialist");
    let answer: Value = response.json().await.unwrap();
    assert_eq!(answer["data"].as_array().unwrap().len(), 2);
    assert_eq!(answer["data"][0]["b64_json"], PIXEL);

    let (path, sent) = rig.last();
    assert_eq!(path, "images/generations");
    assert_eq!(sent["model"], "z-image-turbo");
    assert_eq!(
        sent["response_format"], "b64_json",
        "Vultr's links never leave the gateway"
    );
    assert_eq!(
        rig.usage().await,
        vec![(
            "image".to_owned(),
            "z-image-turbo".to_owned(),
            41_944,
            Some(2),
            Some("image".to_owned()),
            "ok".to_owned(),
            0
        )]
    );
}

#[tokio::test]
async fn standard_draws_with_the_detailed_model() {
    let rig = rig(Mode::Live, None, None).await;
    let response = rig
        .post(
            "/v1/images/generations",
            &[("x-agentrix-tier", "standard")],
            json!({ "prompt": "A bottle of amber perfume on dark slate" }),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-agentrix-model"], "qwen-image-2.1");
    assert_eq!(rig.last().1["model"], "qwen-image-2.1");
}

#[tokio::test]
async fn bad_requests_never_reach_vultr() {
    let rig = rig(Mode::Live, None, None).await;
    for (path, body, code) in [
        (
            "/v1/images/generations",
            json!({ "prompt": "" }),
            "missing_prompt",
        ),
        (
            "/v1/images/generations",
            json!({ "prompt": "x", "size": "1000x1000" }),
            "bad_size",
        ),
        (
            "/v1/images/generations",
            json!({ "prompt": "x", "n": 10 }),
            "bad_n",
        ),
        (
            "/v1/images/edits",
            json!({ "prompt": "x", "images": ["https://example.com/a.png"] }),
            "bad_image",
        ),
        (
            "/v1/images/edits",
            json!({ "prompt": "x" }),
            "missing_images",
        ),
        (
            "/v1/images/screen",
            json!({ "image": "data:image/gif;base64,R0lGOD" }),
            "bad_image",
        ),
    ] {
        let response = rig.post(path, &[], body).await;
        assert_eq!(response.status(), 400, "{path} {code}");
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], code);
    }
    let wrong = rig
        .http
        .post(format!("{}/v1/images/generations", rig.url))
        .header("x-agentrix-site", "site_1")
        .header("x-agentrix-purpose", "concierge")
        .json(&json!({ "prompt": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 400);
    assert_eq!(rig.calls.load(Ordering::SeqCst), 0);
    assert!(rig.usage().await.is_empty());
}

#[tokio::test]
async fn an_edit_sends_the_images_to_the_detailed_model_and_keys_them_by_digest() {
    let dir = temp_dir("edit");
    let rig = rig(Mode::Record, Some(dir.clone()), None).await;
    let response = rig
        .post(
            "/v1/images/edits",
            &[],
            json!({ "prompt": "Put the cake on a blue plate", "images": [data_uri()], "size": "1024x1024", "seed": 3 }),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-agentrix-model"], "qwen-image-2.1");
    let (path, sent) = rig.last();
    assert_eq!(path, "images/edits");
    assert_eq!(sent["images"], json!([{ "image_url": data_uri() }]));
    let usage = rig.usage().await;
    assert_eq!(
        (usage[0].2, usage[0].3),
        (0, Some(1)),
        "free during evaluation"
    );

    let saved = walk(&dir);
    assert_eq!(saved.len(), 1);
    let recording: Value = serde_json::from_slice(&std::fs::read(&saved[0]).unwrap()).unwrap();
    let keyed = recording["request"]["images"][0]["image_url"]
        .as_str()
        .unwrap();
    assert!(
        keyed.starts_with("sha256:"),
        "the recording names the picture by digest, not its bytes"
    );
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn a_recorded_image_replays_without_the_network_or_the_key() {
    let dir = temp_dir("replay");
    let body =
        json!({ "prompt": "Three croissants on a linen cloth", "size": "768x768", "seed": 11 });
    let recorder = rig(Mode::Record, Some(dir.clone()), None).await;
    let first: Value = recorder
        .post("/v1/images/generations", &[], body.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(recorder.calls.load(Ordering::SeqCst), 1);

    let replayer = rig(Mode::Replay, Some(dir), None).await;
    let response = replayer.post("/v1/images/generations", &[], body).await;
    assert_eq!(response.status(), 200);
    let again: Value = response.json().await.unwrap();
    assert_eq!(again["data"], first["data"]);
    assert_eq!(replayer.calls.load(Ordering::SeqCst), 0);
    let usage = replayer.usage().await;
    assert_eq!(
        (usage[0].2, usage[0].3, usage[0].5.as_str()),
        (0, Some(1), "replayed")
    );

    let missing = replayer
        .post(
            "/v1/images/generations",
            &[],
            json!({ "prompt": "Something never recorded" }),
        )
        .await;
    assert_eq!(missing.status(), 503);
    let error: Value = missing.json().await.unwrap();
    assert_eq!(error["error"]["code"], "no_recording");
}

#[tokio::test]
async fn a_spent_image_budget_stops_drawing_before_vultr() {
    let pool = database().await;
    sqlx::raw_sql(
        "INSERT INTO budgets (site_id, purpose, period, limit_micro_usd) VALUES ('site_1', 'image', 'day', 20000);",
    )
    .execute(&pool)
    .await
    .unwrap();
    let rig = rig(Mode::Live, None, Some(pool)).await;
    let body = json!({ "prompt": "A rye loaf" });
    assert_eq!(
        rig.post("/v1/images/generations", &[], body.clone())
            .await
            .status(),
        200
    );
    let refused = rig.post("/v1/images/generations", &[], body).await;
    assert_eq!(refused.status(), 403);
    let error: Value = refused.json().await.unwrap();
    assert_eq!(error["error"]["code"], "over_budget");
    assert_eq!(rig.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn screening_looks_at_the_picture_and_fails_closed() {
    let rig = rig(Mode::Live, None, None).await;
    let safe: Value = rig
        .post(
            "/v1/images/screen",
            &[],
            json!({ "image": data_uri(), "context": "Hero image for a bakery" }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(safe["safe"], true);
    let (_, sent) = rig.last();
    assert_eq!(sent["model"], "nemotron-3.5-content-safety");
    assert_eq!(
        sent["messages"][0]["content"][1]["image_url"]["url"],
        data_uri()
    );

    let unsafe_: Value = rig
        .post(
            "/v1/images/screen",
            &[],
            json!({ "image": data_uri(), "context": "a weapon" }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(unsafe_["safe"], false);
    assert_eq!(unsafe_["categories"], json!(["Guns and Illegal Weapons"]));

    let usage = rig.usage().await;
    assert_eq!(usage.len(), 2);
    assert_eq!(
        usage[0].0, "image",
        "screening an image is part of its cost"
    );
    assert_eq!(usage[0].1, "nemotron-3.5-content-safety");
    assert_eq!(usage[0].6, 900);
}

#[tokio::test]
async fn alt_text_comes_from_a_vision_model_with_the_photograph_flag() {
    let rig = rig(Mode::Live, None, None).await;
    let response = rig
        .post(
            "/v1/images/describe",
            &[],
            json!({ "image": data_uri(), "context": "Product photo for Country loaf, KSh 450" }),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-agentrix-model"], "qwen3.8-27b");
    let answer: Value = response.json().await.unwrap();
    assert_eq!(
        answer["alt"],
        "A round sourdough loaf with a dark, cracked crust on a wooden board"
    );
    assert_eq!(answer["photographic"], true);
    assert_eq!(answer["shows_person"], false);
    let (_, sent) = rig.last();
    assert_eq!(sent["response_format"]["json_schema"]["name"], "alt");
    assert_eq!(rig.usage().await[0].0, "image");
}

impl Rig {
    async fn post_as(&self, path: &str, purpose: &str, body: Value) -> reqwest::Response {
        self.http
            .post(format!("{}{path}", self.url))
            .header("x-agentrix-site", "site_1")
            .header("x-agentrix-purpose", purpose)
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    async fn wait_for_usage(&self, rows: usize) -> Vec<Row> {
        for _ in 0..300 {
            let usage = self.usage().await;
            if usage.len() >= rows {
                return usage;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("usage was never recorded");
    }
}

#[tokio::test]
async fn a_recorded_clip_is_transcribed_and_billed_by_the_second() {
    let rig = rig(Mode::Live, None, None).await;
    let response = rig
        .post_as(
            "/v1/audio/transcriptions",
            "voice.listen",
            json!({ "input_audio": { "data": "UklGRiQAAABXQVZFZm10IBAAAAABAAEAgD4AAAB9AAACABAAZGF0YQAAAAA=", "format": "wav" }, "language": "en" }),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["x-agentrix-model"],
        "whisper-large-v3-turbo"
    );
    let answer: Value = response.json().await.unwrap();
    assert_eq!(answer["text"], "Allow once.");
    let (path, sent) = rig.last();
    assert_eq!(path, "audio/transcriptions");
    assert_eq!(sent["model"], "whisper-large-v3-turbo");
    assert_eq!(sent["language"], "en");
    // 3 s at $0.0000111.
    assert_eq!(
        rig.usage().await,
        vec![(
            "voice.listen".to_owned(),
            "whisper-large-v3-turbo".to_owned(),
            34,
            Some(3),
            Some("audio_second".to_owned()),
            "ok".to_owned(),
            0
        )]
    );
    let bad = rig
        .post_as(
            "/v1/audio/transcriptions",
            "voice.listen",
            json!({ "input_audio": { "data": "AAAA", "format": "opus" } }),
        )
        .await;
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn speech_streams_through_as_it_is_made_and_is_billed_when_it_ends() {
    let rig = rig(Mode::Live, None, None).await;
    let response = rig
        .post_as(
            "/v1/audio/speech",
            "voice.speak",
            json!({ "input": "Good morning. Three orders came in overnight.", "voice": "aiden", "response_format": "mp3" }),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(
        response.headers()["x-agentrix-model"],
        "qwen3-tts-12hz-1.7b"
    );
    let text = response.text().await.unwrap();
    assert!(text.contains("speech.audio.delta"));
    assert!(text.contains("speech.audio.done"));
    let (path, sent) = rig.last();
    assert_eq!(path, "audio/speech");
    assert_eq!(
        (
            sent["response_format"].as_str(),
            sent["stream_format"].as_str()
        ),
        (Some("pcm"), Some("sse")),
        "always PCM over SSE, whatever the caller asked"
    );
    assert_eq!(sent["voice"], "aiden");
    // 3 s at $0.00028, as Vultr reports it (a string).
    let usage = rig.wait_for_usage(1).await;
    assert_eq!(
        (
            usage[0].0.as_str(),
            usage[0].2,
            usage[0].3,
            usage[0].5.as_str()
        ),
        ("voice.speak", 840, Some(3), "ok")
    );
    let empty = rig
        .post_as("/v1/audio/speech", "voice.speak", json!({ "input": "  " }))
        .await;
    assert_eq!(empty.status(), 400);
}

#[tokio::test]
async fn speech_replays_from_a_recording() {
    let dir = temp_dir("speech");
    let body = json!({ "input": "Your brief for Sunday." });
    let recorder = rig(Mode::Record, Some(dir.clone()), None).await;
    let first = recorder
        .post_as("/v1/audio/speech", "voice.speak", body.clone())
        .await
        .text()
        .await
        .unwrap();
    recorder.wait_for_usage(1).await;
    for _ in 0..100 {
        if !walk(&dir).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let replayer = rig(Mode::Replay, Some(dir), None).await;
    let again = replayer
        .post_as("/v1/audio/speech", "voice.speak", body)
        .await
        .text()
        .await
        .unwrap();
    assert_eq!(again, first);
    assert_eq!(replayer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(replayer.usage().await[0].5, "replayed");
}

#[tokio::test]
async fn a_live_session_relays_audio_and_turns_and_bills_the_audio_sent() {
    use futures::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

    let rig = rig(Mode::Live, None, None).await;
    let url = format!("{}/v1/realtime", rig.url.replacen("http://", "ws://", 1));
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("x-agentrix-site", "site_1".parse().unwrap());
    request
        .headers_mut()
        .insert("x-agentrix-purpose", "voice.live".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    // A caller may not reconfigure the session; only audio and turns pass.
    socket
        .send(Message::text(json!({ "type": "session.update", "session": { "audio": { "input": { "format": { "rate": 8000 } } } } }).to_string()))
        .await
        .unwrap();
    // One second of 16 kHz PCM16 as a binary frame, then the end of the turn.
    socket
        .send(Message::binary(vec![0u8; 32_000]))
        .await
        .unwrap();
    socket
        .send(Message::text(
            json!({ "type": "input_audio_buffer.commit" }).to_string(),
        ))
        .await
        .unwrap();
    let mut transcript = None;
    while let Some(Ok(message)) = socket.next().await {
        if let Message::Text(text) = message {
            let event: Value = serde_json::from_str(text.as_str()).unwrap();
            if event["type"] == "conversation.item.input_audio_transcription.completed" {
                transcript = event["transcript"].as_str().map(str::to_owned);
                break;
            }
        }
    }
    assert_eq!(transcript.as_deref(), Some("Allow once."));
    socket.close(None).await.unwrap();

    let usage = rig.wait_for_usage(1).await;
    // 1 s at $0.000133.
    assert_eq!(
        (
            usage[0].0.as_str(),
            usage[0].1.as_str(),
            usage[0].2,
            usage[0].3,
            usage[0].4.as_deref()
        ),
        (
            "voice.live",
            "voxtral-mini-4b-realtime",
            133,
            Some(1),
            Some("audio_second")
        )
    );
    let (_, updates) = rig.last();
    let updates = updates.as_array().unwrap();
    assert_eq!(updates.len(), 1, "only the gateway configures the session");
    assert_eq!(
        updates[0]["session"]["audio"]["input"]["format"]["rate"],
        16_000
    );
    assert_eq!(
        updates[0]["session"]["audio"]["input"]["turn_detection"],
        Value::Null
    );
}

#[tokio::test]
async fn live_voice_needs_a_known_site_and_a_live_gateway() {
    let rig = rig(Mode::Live, None, None).await;
    let url = format!("{}/v1/realtime", rig.url.replacen("http://", "ws://", 1));
    let refused = tokio_tungstenite::connect_async(url.clone()).await;
    assert!(
        refused.is_err(),
        "no site, no purpose: refused before the upgrade"
    );
    let replay = rig_replay_url().await;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let mut request = replay.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("x-agentrix-site", "site_1".parse().unwrap());
    request
        .headers_mut()
        .insert("x-agentrix-purpose", "voice.live".parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), 503);
        }
        other => panic!("replay mode opened a live session: {:?}", other.map(|_| ())),
    }
    assert_eq!(rig.calls.load(Ordering::SeqCst), 0);
}

async fn rig_replay_url() -> String {
    let rig = rig(Mode::Replay, None, None).await;
    format!("{}/v1/realtime", rig.url.replacen("http://", "ws://", 1))
}
