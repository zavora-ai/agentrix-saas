//! The media library end to end against a fake Model Gateway: draw a draft, keep it off the
//! site until the owner approves, show it, replace it. No model is called.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentrix_host::testing::{FixedOwner, MemoryReceipts, context};
use agentrix_host::{NodeContext, Receipts};
use agentrix_media::samples::{SampleSite, seed_from};
use agentrix_media::{library, routes};
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

/// A tiny WebP header, enough for the library (the bytes are never decoded).
const PICTURE: &[u8] = b"RIFF\x1a\x00\x00\x00WEBPVP8 test-picture";

#[derive(Clone, Default)]
struct Fake {
    draws: Arc<AtomicUsize>,
}

async fn fake_draw(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let n = fake.draws.fetch_add(1, Ordering::SeqCst);
    assert_eq!(headers["x-agentrix-purpose"], "image");
    assert_eq!(headers["x-agentrix-site"], "site_1");
    assert!(
        body["prompt"]
            .as_str()
            .unwrap()
            .contains("No people, no faces"),
        "house rules ride on every prompt"
    );
    // Each draw differs, so each keeps its own digest.
    let mut bytes = PICTURE.to_vec();
    bytes.extend_from_slice(n.to_string().as_bytes());
    use base64::Engine as _;
    Json(json!({
        "data": [{ "b64_json": base64::engine::general_purpose::STANDARD.encode(bytes), "media_type": "image/webp" }],
        "size": body["size"],
        "usage": { "cost": 0.011797, "megapixels": 0.589824 }
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
        "categories": if unsafe_ { json!(["Violence"]) } else { json!([]) },
        "model": "nemotron-3.5-content-safety"
    }))
}

async fn fake_describe() -> Json<Value> {
    Json(json!({
        "alt": "A round sourdough loaf with a dark, cracked crust",
        "photographic": true,
        "shows_person": false,
        "shows_text": false,
        "check": ""
    }))
}

struct Rig {
    ctx: NodeContext,
    app: Router,
    draws: Arc<AtomicUsize>,
    receipts: Arc<MemoryReceipts>,
    owner: Uuid,
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
    let receipts = Arc::new(MemoryReceipts::default());
    let mut ctx = context().await.unwrap();
    ctx.models_url = format!("http://{addr}");
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    ctx.receipts = receipts.clone() as Arc<dyn Receipts>;
    sqlx::raw_sql("CREATE TABLE sites (site_id TEXT PRIMARY KEY, owner_account_id TEXT);")
        .execute(&ctx.pool)
        .await
        .unwrap();
    for (site, account) in [("site_1", owner), ("site_2", Uuid::new_v4())] {
        sqlx::query("INSERT INTO sites VALUES (?, ?)")
            .bind(site)
            .bind(account.to_string())
            .execute(&ctx.pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(include_str!(
        "../../agentrix-node/src/db/migrations/1200_media.sql"
    ))
    .execute(&ctx.pool)
    .await
    .unwrap();
    let state = routes::MediaState::new(&ctx);
    let app = routes::console_router(state.clone()).merge(routes::picture_router(state));
    Rig {
        ctx,
        app,
        draws: fake.draws,
        receipts,
        owner,
    }
}

impl Rig {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
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
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn status_of(&self, path: &str) -> StatusCode {
        self.call("GET", path, None).await.0
    }
}

fn draw_body(brief: &str) -> Value {
    json!({ "slot": "product:sourdough", "kind": "product", "brief": brief, "context": "Product picture for Country sourdough, KSh 450" })
}

#[tokio::test]
async fn a_picture_waits_for_the_owner_then_shows_and_can_be_replaced() {
    let rig = rig().await;
    let (status, drawn) = rig
        .call(
            "POST",
            "/api/media/site_1/draw",
            Some(draw_body("A round sourdough loaf")),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{drawn}");
    let draft = &drawn["made"][0];
    assert_eq!(draft["status"], "draft");
    assert_eq!(
        draft["alt"],
        "A round sourdough loaf with a dark, cracked crust"
    );
    assert_eq!(draft["label"], "Made with AI, not a photo");
    assert_eq!(draft["cost_micro_usd"], 11_797);
    let id = draft["image_id"].as_str().unwrap().to_owned();
    let public = library::url_for(draft["digest"].as_str().unwrap(), "image/webp");

    // A draft is the owner's to see, not a visitor's.
    assert_eq!(rig.status_of(&public).await, StatusCode::NOT_FOUND);
    assert_eq!(
        rig.status_of(&format!("/api/media/site_1/{id}/file")).await,
        StatusCode::OK
    );
    assert!(
        library::shown(&rig.ctx.pool, "site_1")
            .await
            .unwrap()
            .is_empty()
    );

    let (status, approved) = rig
        .call("POST", &format!("/api/media/site_1/{id}/approve"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(approved["status"], "approved");
    assert_eq!(approved["decided_by"], format!("owner:{}", rig.owner));
    assert!(approved["receipt"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(rig.receipts.issued.lock().unwrap().len(), 1);
    assert_eq!(rig.status_of(&public).await, StatusCode::OK);
    let shown = library::shown(&rig.ctx.pool, "site_1").await.unwrap();
    assert_eq!(shown["product:sourdough"].url, public);
    assert_eq!(
        shown["product:sourdough"].label.as_deref(),
        Some("Made with AI, not a photo")
    );

    // Approving twice changes nothing.
    let (status, _) = rig
        .call("POST", &format!("/api/media/site_1/{id}/approve"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (_, again) = rig
        .call(
            "POST",
            &format!("/api/media/site_1/{id}/again"),
            Some(json!({})),
        )
        .await;
    let second = again["made"][0]["image_id"].as_str().unwrap().to_owned();
    assert_eq!(again["made"][0]["replaces"], id);
    rig.call("POST", &format!("/api/media/site_1/{second}/approve"), None)
        .await;
    let (_, library) = rig.call("GET", "/api/media/site_1", None).await;
    let statuses: Vec<(String, String)> = library["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["image_id"].as_str().unwrap().to_owned(),
                i["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(statuses.contains(&(second.clone(), "approved".to_owned())));
    assert!(statuses.contains(&(id.clone(), "replaced".to_owned())));
    assert_eq!(
        rig.status_of(&public).await,
        StatusCode::NOT_FOUND,
        "a replaced picture leaves the site"
    );
    assert_eq!(library["spend"]["made"], 2);
}

#[tokio::test]
async fn a_brief_for_a_person_is_refused_before_any_drawing() {
    let rig = rig().await;
    let (status, error) = rig
        .call(
            "POST",
            "/api/media/site_1/draw",
            Some(draw_body("A portrait of the owner with her bread")),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error["error"]["code"], "refused");
    assert_eq!(rig.draws.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_picture_the_safety_check_stops_keeps_no_bytes() {
    let rig = rig().await;
    let mut body = draw_body("A loaf of bread");
    body["context"] = json!("unsafe test");
    let (status, drawn) = rig.call("POST", "/api/media/site_1/draw", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(drawn["made"], json!([]));
    assert_eq!(drawn["blocked"], 1);
    let (_, library) = rig.call("GET", "/api/media/site_1", None).await;
    let blocked = &library["images"][0];
    assert_eq!(blocked["status"], "blocked");
    assert_eq!(blocked["digest"], "");
    assert_eq!(blocked["src"], Value::Null);
    assert_eq!(blocked["screen"]["categories"], json!(["Violence"]));
}

#[tokio::test]
async fn only_the_sites_owner_manages_its_pictures() {
    let rig = rig().await;
    assert_eq!(
        rig.status_of("/api/media/site_2").await,
        StatusCode::FORBIDDEN
    );
    let (status, _) = rig
        .call("POST", "/api/media/site_2/draw", Some(draw_body("A loaf")))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let mut nobody = rig.ctx.clone();
    nobody.owners = Arc::new(FixedOwner::nobody());
    let state = routes::MediaState::new(&nobody);
    let app = routes::console_router(state);
    let response = app
        .oneshot(
            Request::get("/api/media/site_1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(rig.draws.load(Ordering::SeqCst), 0);
    assert_eq!(
        rig.status_of(&format!("/media/{}.webp", "0".repeat(64)))
            .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        rig.status_of("/media/../../etc.webp").await,
        StatusCode::NOT_FOUND
    );
}

static SAMPLE_FILES: [(&str, &[u8]); 1] = [("hero.webp", PICTURE)];

#[tokio::test]
async fn sample_pictures_are_seeded_once_and_shown_with_their_label() {
    let rig = rig().await;
    let manifest = r#"{"site": "kilele-bakery-sample", "images": {"hero": {
        "file": "hero.webp", "kind": "hero", "media_type": "image/webp", "width": 1344, "height": 768,
        "label": "Sample image, made with AI", "model": "z-image-turbo", "seed": 1101,
        "description": {"alt": "Loaves on wooden racks by a window at sunrise", "photographic": true, "shows_person": false, "shows_text": false, "check": ""},
        "verdict": {"safe": true}, "cost_micro_usd": 20644}}}"#;
    let sites = [SampleSite {
        manifest: Box::leak(manifest.to_owned().into_boxed_str()),
        files: &SAMPLE_FILES,
    }];
    assert_eq!(
        seed_from(&rig.ctx.pool, &rig.ctx.blobs, &sites)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        seed_from(&rig.ctx.pool, &rig.ctx.blobs, &sites)
            .await
            .unwrap(),
        0,
        "never twice"
    );
    let shown = library::shown(&rig.ctx.pool, "kilele-bakery-sample")
        .await
        .unwrap();
    let hero = &shown["hero"];
    assert_eq!(hero.alt, "Loaves on wooden racks by a window at sunrise");
    assert_eq!(hero.label.as_deref(), Some("Sample image, made with AI"));
    assert_eq!(rig.status_of(&hero.url).await, StatusCode::OK);
}

#[tokio::test]
async fn the_embedded_samples_all_load() {
    let rig = rig().await;
    let added = agentrix_media::samples::seed(&rig.ctx.pool, &rig.ctx.blobs)
        .await
        .unwrap();
    let embedded: usize = agentrix_media::samples::SAMPLES
        .iter()
        .map(|s| s.files.len())
        .sum();
    assert_eq!(added, embedded);
}

#[tokio::test]
async fn the_move_fills_bare_pages_with_drafts_once() {
    use agentrix_media::fill::{Page, fill};
    let rig = rig().await;
    let state = routes::MediaState::new(&rig.ctx);
    let pages = vec![
        Page {
            path: "/".into(),
            title: "Home".into(),
            summary: "Fresh bread every morning.".into(),
            has_picture: true,
        },
        Page {
            path: "/cakes".into(),
            title: "Cakes".into(),
            summary: "Celebration cakes to order.".into(),
            has_picture: false,
        },
        Page {
            path: "/about".into(),
            title: "About us".into(),
            summary: "Our team has baked here since 2016.".into(),
            has_picture: false,
        },
    ];
    let first = fill(&state, "site_1", "", &pages, 5).await.unwrap();
    let slots: Vec<&str> = first.drafts.iter().map(|d| d.slot.as_str()).collect();
    assert_eq!(slots, ["page:cakes", "page:about"]);
    assert!(
        first
            .drafts
            .iter()
            .all(|d| d.status == "draft" && d.asked_by == "move")
    );
    assert_eq!(
        first.drafts[1].brief, "A picture for a page titled \"About us\"",
        "a summary the policy refuses is left out"
    );
    assert_eq!(first.skipped.len(), 1);
    assert_eq!(
        library::waiting(&rig.ctx.pool, "site_1", Some("move"))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        library::waiting(&rig.ctx.pool, "site_1", Some("owner"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        library::waiting(&rig.ctx.pool, "site_1", None)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        library::pictures_url("site_1"),
        "/console/site_1/apps/pictures"
    );
    let again = fill(&state, "site_1", "", &pages, 5).await.unwrap();
    assert!(
        again.drafts.is_empty(),
        "a page with a draft waiting is not drawn twice"
    );
    assert_eq!(rig.draws.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn another_crate_asks_for_a_picture_in_one_call() {
    let rig = rig().await;
    let drafts = agentrix_media::draft(
        &rig.ctx,
        "site_1",
        "post:hello-autumn",
        agentrix_media::policy::Kind::Post,
        "A loaf of rye bread on a table by a rainy window",
        "editor",
    )
    .await
    .unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        (
            drafts[0].slot.as_str(),
            drafts[0].asked_by.as_str(),
            drafts[0].status.as_str()
        ),
        ("post:hello-autumn", "editor", "draft")
    );
    let refused = agentrix_media::draft(
        &rig.ctx,
        "site_1",
        "post:team",
        agentrix_media::policy::Kind::Post,
        "Our team at the counter",
        "editor",
    )
    .await;
    assert!(refused.is_err(), "people are refused before any drawing");
}
