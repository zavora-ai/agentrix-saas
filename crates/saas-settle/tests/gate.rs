//! The Web Bot Auth gate in front of a door strangers' agents may use (a sample site's shop
//! door): a signed agent gets through with its identity, anything else is refused before the
//! handler runs, with a sealed refusal receipt kept and sent to the company's kernel.

use std::sync::Arc;

use adk_auth::web_bot_auth::{
    ComponentId, Ed25519SigningKey, HttpSignatureSigner, InMemoryKeyDirectorySource, KeyDirectory,
    KeyDirectorySource,
};
use axum::Extension;
use axum::routing::{get, post};
use saas_settle::door::{CONTENT_DIGEST, content_digest};
use saas_settle::events::SettleEvent;
use saas_settle::keys::PAYABLES;
use saas_settle::{CompanyKeys, DoorAuth, SignatureGate, Store, VerifiedAgent};
use serde_json::{Value, json};

const SHOP: &str = "https://kilele-sample.paypal.agentrix.example";
const STRANGER: &str = "https://shopper.example";

fn signer(key: Arc<Ed25519SigningKey>, origin: &str) -> HttpSignatureSigner {
    HttpSignatureSigner::builder(key, origin)
        .components(vec![
            ComponentId::new("@authority"),
            ComponentId::new("@method"),
            ComponentId::new("@path"),
            ComponentId::new("@query"),
            ComponentId::new(CONTENT_DIGEST),
        ])
        .build()
        .unwrap()
}

#[tokio::test]
async fn only_signed_agents_get_through_and_the_rest_are_refused_with_a_sealed_receipt() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    saas_settle::migrate(&pool).await.unwrap();
    let store = Store::new(pool);
    let directories = Arc::new(InMemoryKeyDirectorySource::new());
    let stranger = Arc::new(Ed25519SigningKey::generate().unwrap());
    directories
        .insert(STRANGER, KeyDirectory::from_keys([stranger.as_ref()]), None)
        .await;
    let source: Arc<dyn KeyDirectorySource> = directories.clone();
    let shop = CompanyKeys::generate(SHOP, &[PAYABLES]).unwrap();
    let events = saas_settle::events::channel();
    let mut kernel = events.subscribe();
    let gate = SignatureGate::new(
        "Kilele Bakery",
        shop.clone(),
        DoorAuth::any_signed(source, store.clone()),
        Some(events),
    );
    // The shop door's handler: it sees who signed.
    let who = |Extension(agent): Extension<VerifiedAgent>| async move { agent.id };
    let app = axum::Router::new()
        .route("/mcp", post(who))
        .route("/mcp", get(who))
        .layer(axum::middleware::from_fn_with_state(
            gate,
            SignatureGate::require,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::new();
    let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "place_order"}})).unwrap();
    let request = |body: &[u8], digest_of: &[u8]| {
        http.post(&url)
            .header("content-type", "application/json")
            .header(CONTENT_DIGEST, content_digest(digest_of))
            .body(body.to_vec())
            .build()
            .unwrap()
    };

    // An impostor that only names itself: refused before the handler, receipt sealed and kept.
    let unsigned = http.execute(request(&body, &body)).await.unwrap();
    assert_eq!(unsigned.status().as_u16(), 401);
    let answer: Value = unsigned.json().await.unwrap();
    assert_eq!(answer["id"], 7);
    assert_eq!(answer["error"]["data"]["code"], "signature_required");
    let receipt = &answer["error"]["data"]["receipt"];
    assert_eq!(receipt["body"]["code"], "signature_required");
    assert_eq!(receipt["body"]["refused_by"], SHOP);
    assert_eq!(receipt["body"]["nothing_moved"], true);
    assert_eq!(store.refusals(10).await.unwrap().len(), 1);
    assert!(matches!(
        kernel.recv().await.unwrap(),
        SettleEvent::Refused { .. }
    ));

    // A stranger's agent that signs with a key its own directory publishes: let through.
    let signed = signer(stranger.clone(), STRANGER);
    let mut ok = request(&body, &body);
    signed.sign_reqwest(&mut ok).unwrap();
    let replay = ok.try_clone().unwrap();
    let through = http.execute(ok).await.unwrap();
    assert_eq!(through.status().as_u16(), 200);
    let id = through.text().await.unwrap();
    assert!(id.starts_with(&format!("wba:{STRANGER}#")), "{id}");

    // The same signed request again: refused.
    let again: Value = http.execute(replay).await.unwrap().json().await.unwrap();
    assert_eq!(again["error"]["data"]["code"], "replayed_request");

    // Signed with a key the named directory doesn't publish: refused.
    let forger = signer(Arc::new(Ed25519SigningKey::generate().unwrap()), STRANGER);
    let mut forged = request(&body, &body);
    forger.sign_reqwest(&mut forged).unwrap();
    let forged: Value = http.execute(forged).await.unwrap().json().await.unwrap();
    assert_eq!(forged["error"]["data"]["code"], "signature_invalid");

    // Signed, then the body changed: refused.
    let mut edited_body = body.clone();
    edited_body[10] = b' ';
    let mut tampered = request(&edited_body, &body);
    signed.sign_reqwest(&mut tampered).unwrap();
    let tampered: Value = http.execute(tampered).await.unwrap().json().await.unwrap();
    assert_eq!(tampered["error"]["data"]["code"], "content_digest_mismatch");

    // A signed GET (the digest of an empty body) gets through too.
    let mut read = http
        .get(&url)
        .header(CONTENT_DIGEST, content_digest(b""))
        .build()
        .unwrap();
    signed.sign_reqwest(&mut read).unwrap();
    assert_eq!(http.execute(read).await.unwrap().status().as_u16(), 200);
}
