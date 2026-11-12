//! The sample sites' agent door, proven: saas-settle's checker behind agentrix-samples'
//! `AgentProof`, with real Web Bot Auth signatures and agent tokens.

use std::sync::Arc;

use adk_auth::web_bot_auth::{
    ComponentId, Ed25519SigningKey, HttpSignatureSigner, InMemoryKeyDirectorySource, KeyDirectory,
    KeyDirectorySource,
};
use agentrix_samples::{AgentProof, DoorRefusal, ProofRequest};
use saas_settle::door::{CONTENT_DIGEST, content_digest};
use saas_settle::events::SettleEvent;
use saas_settle::keys::PAYABLES;
use saas_settle::tokens::{AgentId, AgentPrincipal, FixedTokens};
use saas_settle::{CompanyKeys, DoorAuth, SampleDoorProof, Store};
use serde_json::json;

const DOOR: &str = "https://kilele-bakery-sample.paypal.agentrix.example";
const AGENT: &str = "https://assistant.example";

async fn check(
    proof: &SampleDoorProof,
    request: &reqwest::Request,
) -> Result<Option<agentrix_samples::ProvenAgent>, agentrix_samples::ProofFailure> {
    let body = request
        .body()
        .and_then(|b| b.as_bytes())
        .unwrap_or_default()
        .to_vec();
    let uri: axum::http::Uri = request.url().as_str().parse().unwrap();
    proof
        .prove(ProofRequest {
            door: DOOR,
            method: request.method(),
            uri: &uri,
            headers: request.headers(),
            body: &body,
        })
        .await
}

#[tokio::test]
async fn an_agent_is_known_only_when_it_proves_it() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    saas_settle::migrate(&pool).await.unwrap();
    let store = Store::new(pool);
    let directories = Arc::new(InMemoryKeyDirectorySource::new());
    let key = Arc::new(Ed25519SigningKey::generate().unwrap());
    directories
        .insert(AGENT, KeyDirectory::from_keys([key.as_ref()]), None)
        .await;
    let source: Arc<dyn KeyDirectorySource> = directories.clone();
    let tokens = Arc::new(FixedTokens::default());
    tokens.accept(
        "entra-token",
        DOOR,
        AgentPrincipal::new(AgentId::new("urn:entra:tenant:agent:a9e1").unwrap()),
        "https://login.microsoftonline.com/tenant/v2.0",
        "entra",
    );
    tokens.accept(
        "elsewhere",
        "https://another-door.example",
        AgentPrincipal::new(AgentId::new("urn:entra:tenant:agent:a9e1").unwrap()),
        "https://login.microsoftonline.com/tenant/v2.0",
        "entra",
    );
    let events = saas_settle::events::channel();
    let mut kernel = events.subscribe();
    let agentrix = CompanyKeys::generate("https://paypal.agentrix.example", &[PAYABLES]).unwrap();
    let proof = SampleDoorProof::new(
        DoorAuth::any_signed(source, store.clone()),
        Some(tokens),
        agentrix.clone(),
        Some(events),
    );
    let http = reqwest::Client::new();
    let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "quote.create"}})).unwrap();
    let request = || {
        http.post(format!("{DOOR}/mcp"))
            .header("content-type", "application/json")
            .header(CONTENT_DIGEST, content_digest(&body))
            .body(body.clone())
            .build()
            .unwrap()
    };
    let signer = |key: Arc<Ed25519SigningKey>| {
        HttpSignatureSigner::builder(key, AGENT)
            .components(vec![
                ComponentId::new("@authority"),
                ComponentId::new("@method"),
                ComponentId::new("@path"),
                ComponentId::new("@query"),
                ComponentId::new(CONTENT_DIGEST),
            ])
            .build()
            .unwrap()
    };

    // No proof at all, and a bare Signature-Agent header: nothing proven (a claim only).
    assert_eq!(check(&proof, &request()).await.unwrap(), None);
    let mut claimed = request();
    claimed
        .headers_mut()
        .insert("signature-agent", AGENT.parse().unwrap());
    assert_eq!(check(&proof, &claimed).await.unwrap(), None);

    // Signed with a key the agent's own directory publishes: proven, named by its directory.
    let mut signed = request();
    signer(key.clone()).sign_reqwest(&mut signed).unwrap();
    let replay = signed.try_clone().unwrap();
    let proven = check(&proof, &signed).await.unwrap().expect("proven");
    assert!(
        proven.id.starts_with(&format!("wba:{AGENT}#")),
        "{proven:?}"
    );
    assert_eq!(proven.vouched_by, AGENT);
    assert_eq!(proven.method, "web-bot-auth");
    // Each signed request is used once.
    assert_eq!(
        check(&proof, &replay).await.unwrap_err().code,
        "replayed_request"
    );

    // Signed with a key the named directory doesn't publish: refused.
    let mut forged = request();
    signer(Arc::new(Ed25519SigningKey::generate().unwrap()))
        .sign_reqwest(&mut forged)
        .unwrap();
    assert_eq!(
        check(&proof, &forged).await.unwrap_err().code,
        "signature_invalid"
    );

    // A token from a trusted issuer, addressed to this door: proven; for another door: refused.
    let mut token = request();
    token
        .headers_mut()
        .insert("authorization", "Bearer entra-token".parse().unwrap());
    let by_token = check(&proof, &token).await.unwrap().expect("proven");
    assert_eq!(by_token.id, "urn:entra:tenant:agent:a9e1");
    assert_eq!(by_token.method, "token");
    assert!(by_token.vouched_by.contains("login.microsoftonline.com"));
    let mut other = request();
    other
        .headers_mut()
        .insert("authorization", "Bearer elsewhere".parse().unwrap());
    assert_eq!(
        check(&proof, &other).await.unwrap_err().code,
        "token_refused"
    );

    // The door's refusal: sealed in the settlement door's shape, kept, and told to the kernel.
    let receipt = proof
        .refusal_receipt(DoorRefusal {
            door: DOOR,
            site: "Kilele Bakery (sample)",
            tool: "quote.create",
            code: "agent_identity_required",
            reason: "quote.create needs a proven agent.",
            claimed: Some("assistant.example"),
            body: &body,
        })
        .await
        .expect("a sealed receipt");
    assert_eq!(receipt["signer"], "https://paypal.agentrix.example");
    assert_eq!(receipt["body"]["refused_by"], DOOR);
    assert_eq!(receipt["body"]["code"], "agent_identity_required");
    assert_eq!(receipt["body"]["claimed_agent"], "assistant.example");
    assert_eq!(receipt["body"]["nothing_moved"], true);
    let sealed: saas_settle::Sealed = serde_json::from_value(receipt).unwrap();
    assert!(!sealed.signature.is_empty());
    assert_eq!(store.refusals(10).await.unwrap().len(), 1);
    assert!(matches!(
        kernel.recv().await.unwrap(),
        SettleEvent::Refused { .. }
    ));
}
