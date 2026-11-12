//! Ideas (ADR-032) against a fake Model Gateway: drawn once per design, labelled, screened,
//! served apart from approved pictures and never in the owner's library. No model is called.

use std::sync::{Arc, Mutex};

use agentrix_host::NodeContext;
use agentrix_host::testing::{FixedOwner, context};
use agentrix_media::ideas::{self, IdeaAsk, IdeaError};
use agentrix_media::policy::Kind;
use agentrix_media::{library, routes};
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const PICTURE: &[u8] = b"RIFF\x1a\x00\x00\x00WEBPVP8 idea-picture";

/// What the fake gateway was asked to draw, in order.
#[derive(Clone, Default)]
struct Fake {
    prompts: Arc<Mutex<Vec<String>>>,
}

async fn fake_draw(State(fake): State<Fake>, Json(body): Json<Value>) -> Json<Value> {
    let prompt = body["prompt"].as_str().unwrap_or_default().to_owned();
    let n = {
        let mut p = fake.prompts.lock().unwrap();
        p.push(prompt);
        p.len()
    };
    let mut bytes = PICTURE.to_vec();
    bytes.extend_from_slice(n.to_string().as_bytes());
    Json(json!({
        "data": [{ "b64_json": base64::engine::general_purpose::STANDARD.encode(bytes), "media_type": "image/webp" }],
        "size": body["size"],
        "usage": { "cost": 0.004 }
    }))
}

async fn fake_screen(Json(body): Json<Value>) -> Json<Value> {
    let unsafe_ = body["context"]
        .as_str()
        .unwrap_or_default()
        .contains("unsafe");
    Json(json!({
        "safe": !unsafe_,
        "user_safety": if unsafe_ { "unsafe" } else { "safe" },
        "categories": [],
        "model": "nemotron-3.5-content-safety"
    }))
}

/// Says a person shows when the brief asks for "a figure-like shape", letters when it asks for
/// "a lettered plaque" (but only on the first draw of that key).
async fn fake_describe(State(fake): State<Fake>, Json(body): Json<Value>) -> Json<Value> {
    let context = body["context"].as_str().unwrap_or_default();
    let draws = fake.prompts.lock().unwrap().len();
    Json(json!({
        "alt": "A two-tier cake in forest green buttercream on a white stand",
        "photographic": true,
        "shows_person": context.contains("figure-like shape"),
        "shows_text": context.contains("lettered plaque") && draws % 2 == 1,
        "check": ""
    }))
}

struct Rig {
    ctx: NodeContext,
    state: routes::MediaState,
    app: Router,
    fake: Fake,
}

async fn rig() -> Rig {
    let fake = Fake::default();
    let gateway = Router::new()
        .route("/v1/images/generations", post(fake_draw))
        .route("/v1/images/screen", post(fake_screen))
        .route("/v1/images/describe", post(fake_describe))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, gateway).await.unwrap() });
    let owner = Uuid::new_v4();
    let mut ctx = context().await.unwrap();
    ctx.models_url = format!("http://{addr}");
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    sqlx::raw_sql("CREATE TABLE sites (site_id TEXT PRIMARY KEY, owner_account_id TEXT);")
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sites VALUES ('kilele-bakery-sample', ?)")
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    for sql in [
        include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
        include_str!("../../agentrix-node/src/db/migrations/1202_media_ideas.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&ctx.pool).await.unwrap();
    }
    let state = routes::MediaState::new(&ctx);
    let app = routes::console_router(state.clone()).merge(routes::picture_router(state.clone()));
    Rig {
        ctx,
        state,
        app,
        fake,
    }
}

impl Rig {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Vec<u8>) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    fn draws(&self) -> usize {
        self.fake.prompts.lock().unwrap().len()
    }
}

fn ask<'a>(key: &'a str, brief: &'a str, context: &'a str) -> IdeaAsk<'a> {
    IdeaAsk {
        site: "kilele-bakery-sample",
        key,
        kind: Kind::CakeIdea,
        brief,
        context,
        asked_by: "studio",
    }
}

const BRIEF: &str =
    "A two-tier round celebration cake covered in forest green buttercream, with a plain top.";

#[tokio::test]
async fn an_idea_is_drawn_once_labelled_and_kept_out_of_the_library() {
    let rig = rig().await;
    let (idea, drawn) = ideas::draw(&rig.state, &ask("0a1b2c3d4e5f", BRIEF, "An idea of a cake"))
        .await
        .expect("drawn");
    assert!(drawn, "the first ask draws");
    assert_eq!(rig.draws(), 1);
    assert_eq!(idea.label, "An idea of your cake, made with AI");
    assert!(idea.url().starts_with("/media/ideas/"));
    let prompt = rig.fake.prompts.lock().unwrap()[0].clone();
    assert!(prompt.starts_with("A two-tier round celebration cake"));
    assert!(prompt.contains("No text, letters, numbers"), "{prompt}");

    // The same design again costs nothing.
    let (again, drawn) = ideas::draw(&rig.state, &ask("0a1b2c3d4e5f", BRIEF, "An idea of a cake"))
        .await
        .expect("cached");
    assert!(!drawn);
    assert_eq!(again, idea);
    assert_eq!(rig.draws(), 1);

    // Served as an idea, never as an approved picture, and never in the owner's library.
    let (status, bytes) = rig.call("GET", &idea.url(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.starts_with(b"RIFF"));
    let hex = idea.digest.trim_start_matches("sha256:");
    assert_eq!(
        rig.call("GET", &format!("/media/{hex}.webp"), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert!(
        library::list(&rig.ctx.pool, "kilele-bakery-sample")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        library::shown(&rig.ctx.pool, "kilele-bakery-sample")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        ideas::by_digest(&rig.ctx.pool, "kilele-bakery-sample", &idea.digest)
            .await
            .unwrap(),
        Some(idea)
    );
    assert_eq!(
        ideas::drawn_since(&rig.ctx.pool, "kilele-bakery-sample", "2000-01-01")
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn an_unsafe_idea_or_one_with_a_person_is_never_kept() {
    let rig = rig().await;
    let stopped = ideas::draw(&rig.state, &ask("1111111aaaa", BRIEF, "unsafe context")).await;
    assert_eq!(stopped.unwrap_err(), IdeaError::Blocked);
    let person = ideas::draw(
        &rig.state,
        &ask(
            "2222222bbbb",
            "A cake with a figure-like shape on top.",
            "An idea of a cake",
        ),
    )
    .await;
    assert_eq!(person.unwrap_err(), IdeaError::Blocked);
    assert!(
        ideas::find(&rig.ctx.pool, "kilele-bakery-sample", "1111111aaaa")
            .await
            .unwrap()
            .is_none()
    );
    // People and logos are refused before any model is called.
    let before = rig.draws();
    let refused = ideas::draw(
        &rig.state,
        &ask(
            "3333333cccc",
            "A cake with the baker's portrait",
            "An idea of a cake",
        ),
    )
    .await;
    assert!(matches!(refused, Err(IdeaError::Refused(_))));
    assert_eq!(rig.draws(), before);
}

#[tokio::test]
async fn letters_get_one_more_draw() {
    let rig = rig().await;
    let (idea, _) = ideas::draw(
        &rig.state,
        &ask(
            "4444444dddd",
            "A cake with a lettered plaque.",
            "An idea of a cake",
        ),
    )
    .await
    .expect("drawn");
    assert_eq!(rig.draws(), 2, "drawn once more with another seed");
    assert!(!idea.shows_text);
}

#[tokio::test]
async fn the_owner_library_never_draws_an_idea() {
    let rig = rig().await;
    let (status, body) = rig
        .call(
            "POST",
            "/api/media/kilele-bakery-sample/draw",
            Some(json!({ "slot": "product:cake", "kind": "cake-idea", "brief": BRIEF })),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&body).contains("cake studio"));
    assert_eq!(rig.draws(), 0);
}
