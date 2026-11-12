//! `POST /ask/pass` (R8.3): limits, the per-IP rate limit, storage as data with no model call,
//! and a plain-words refusal when no store is wired.

use std::sync::Mutex;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use futures::stream::BoxStream;
use tower::ServiceExt;

use super::pass::{PassedQuestion, QuestionSink, SENT};
use super::testkit::{DOMAIN, Receipts, Site};
use super::*;

/// A model that fails the test if Ask's pass route ever calls it.
struct NoModel;

#[async_trait]
impl AskModel for NoModel {
    async fn screen(&self, _: &str, _: &str) -> anyhow::Result<bool> {
        panic!("a passed question reached the model")
    }
    async fn embed(&self, _: &str, _: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        panic!("a passed question reached the model")
    }
    async fn answer(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        panic!("a passed question reached the model")
    }
}

#[derive(Default)]
struct Kept(Mutex<Vec<PassedQuestion>>);

#[async_trait]
impl QuestionSink for Kept {
    async fn pass(&self, q: PassedQuestion) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(q);
        Ok(())
    }
}

async fn app(kept: Option<Arc<Kept>>) -> Router {
    let site = Site::new().await;
    site.release(1, "Sourdough and cakes").await;
    let state = SiteState::new(
        site.memory.clone(),
        Arc::new(NoModel),
        Arc::new(Receipts::default()),
    );
    let state = match kept {
        Some(k) => state.with_questions(k),
        None => state,
    };
    router(state)
}

async fn pass(app: &Router, ip: &str, body: serde_json::Value) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri("/ask/pass")
        .header(header::HOST, DOMAIN)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-forwarded-for", ip)
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    (status, v["message"].as_str().unwrap_or_default().to_owned())
}

#[tokio::test]
async fn a_passed_question_is_stored_as_data_with_its_site_and_contact() {
    let kept = Arc::new(Kept::default());
    let app = app(Some(kept.clone())).await;
    let (status, message) = pass(
        &app,
        "203.0.113.7",
        serde_json::json!({"question": "  Do you bake\u{0007} sugar-free cakes?\n", "contact": " wanjiru@example.test "}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(message, SENT);
    let stored = kept.0.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].question, "Do you bake sugar-free cakes?");
    assert_eq!(stored[0].contact.as_deref(), Some("wanjiru@example.test"));
    assert_eq!(stored[0].site, "site-kilele");
    assert_eq!(stored[0].release_seq, 1);
}

#[tokio::test]
async fn limits_are_enforced_in_plain_words() {
    let kept = Arc::new(Kept::default());
    let app = app(Some(kept.clone())).await;
    let (s, m) = pass(&app, "203.0.113.8", serde_json::json!({"question": "   "})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(m.contains("Write the question"), "{m}");
    let (s, m) = pass(
        &app,
        "203.0.113.8",
        serde_json::json!({"question": "q".repeat(501)}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(m.contains("500"), "{m}");
    let (s, _) = pass(
        &app,
        "203.0.113.8",
        serde_json::json!({"question": "q".repeat(500)}),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "500 characters is allowed");
    let (s, m) = pass(
        &app,
        "203.0.113.8",
        serde_json::json!({"question": "Hello?", "contact": "c".repeat(121)}),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(m.contains("120"), "{m}");
    assert_eq!(kept.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn each_client_may_pass_five_an_hour() {
    let kept = Arc::new(Kept::default());
    let app = app(Some(kept.clone())).await;
    for _ in 0..5 {
        let (s, _) = pass(
            &app,
            "198.51.100.1",
            serde_json::json!({"question": "Open on Monday?"}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }
    let (s, m) = pass(
        &app,
        "198.51.100.1",
        serde_json::json!({"question": "Open on Monday?"}),
    )
    .await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    assert!(m.contains("Try again later"), "{m}");
    // Another visitor is not held back.
    let (s, _) = pass(
        &app,
        "198.51.100.2",
        serde_json::json!({"question": "Open on Monday?"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(kept.0.lock().unwrap().len(), 6);
}

#[tokio::test]
async fn without_a_store_it_says_so_instead_of_dropping_the_question() {
    let app = app(None).await;
    let (s, m) = pass(
        &app,
        "192.0.2.1",
        serde_json::json!({"question": "Do you deliver?"}),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
    assert!(m.contains("contact page"), "{m}");
}
