//! The committed attack list, run in CI: FakeBooks and a PayPal stand-in (wiremock) behind the
//! recording relay, a real `agentrix-saas` child process for the kill -9 attack, and every
//! result scored from the books, the relay's record of PayPal and the exported signed logs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use adk_core::SystemClock;
use saas_node::attacks::{self, ChildSpec, Options};
use saas_node::config::{BooksMode, Config, EnvFile};
use saas_node::relay::Relay;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// PayPal's Payouts, as far as the attacks need: a batch per request id, read back by id.
#[derive(Clone, Default)]
struct Payouts {
    last: Arc<Mutex<Option<Value>>>,
}

impl Respond for Payouts {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        if request.method.as_str() == "POST" {
            *self.last.lock().unwrap() = Some(body.clone());
            return ResponseTemplate::new(201).set_body_json(json!({
                "batch_header": {
                    "payout_batch_id": "BATCH-A08",
                    "batch_status": "PENDING",
                    "sender_batch_header": body["sender_batch_header"],
                },
            }));
        }
        let sent = self.last.lock().unwrap().clone().unwrap_or(Value::Null);
        let item = &sent["items"][0];
        ResponseTemplate::new(200).set_body_json(json!({
            "batch_header": { "payout_batch_id": "BATCH-A08", "batch_status": "SUCCESS", "sender_batch_header": sent["sender_batch_header"] },
            "items": [{
                "payout_item_id": "ITEMA08KILL9", "payout_batch_id": "BATCH-A08",
                "transaction_id": "TXNA08", "transaction_status": "SUCCESS",
                "payout_item_fee": { "currency": "USD", "value": "0.25" },
                "payout_item": { "receiver": item["receiver"], "amount": item["amount"], "recipient_type": "EMAIL" },
                "time_processed": chrono::Utc::now().to_rfc3339(),
            }],
        }))
    }
}

async fn paypal() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "access_token": "A21AA-test", "expires_in": 32400 })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .and(body_string_contains("forged-transmission"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "verification_status": "FAILURE" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v2/payments/captures/[A-Z0-9]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "CAPTUREA02", "status": "COMPLETED",
            "amount": { "currency_code": "USD", "value": "49.00" },
        })))
        .mount(&server)
        .await;
    let payouts = Payouts::default();
    Mock::given(method("POST"))
        .and(path("/v1/payments/payouts"))
        .respond_with(payouts.clone())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/payments/payouts/BATCH-A08"))
        .respond_with(payouts)
        .mount(&server)
        .await;
    server
}

fn repo_specs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../specs/saas")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_committed_attack_is_defended_and_scored_from_books_paypal_and_the_signed_log() {
    let server = paypal().await;
    let dir = tempfile::tempdir().unwrap();
    let text = format!(
        "PAYPAL_CLIENT_ID=id\nPAYPAL_CLIENT_SECRET=secret\nPAYPAL_WEBHOOK_ID=WH-1\n\
         KILELE_PAYPAL_CLIENT_ID=kid\nKILELE_PAYPAL_CLIENT_SECRET=ksecret\n\
         SAAS_ATTACK_CAPTURE_ID=CAPTUREA02\nSAAS_PAYPAL_BASE_URL={}\nSAAS_DATA_DIR={}\n",
        server.uri(),
        dir.path().join("node").display()
    );
    let env_file = dir.path().join("test.env");
    std::fs::write(&env_file, &text).unwrap();
    let env = EnvFile::parse(&text);
    let mut config = Config::from_env(&env, BooksMode::Fake, true);
    let relay = Relay::start(&server.uri()).await.unwrap();
    for app in [&mut config.agentrix_paypal, &mut config.kilele_paypal]
        .into_iter()
        .flatten()
    {
        app.config.environment =
            agentrix_effects::paypal::Environment::Custom(relay.url().to_owned());
    }
    let proofs = dir.path().join("proofs-dir");
    std::fs::create_dir_all(&proofs).unwrap();
    std::fs::copy(repo_specs().join("PROOFS.md"), proofs.join("PROOFS.md")).unwrap();
    let options = Options {
        list_dir: repo_specs(),
        proofs_dir: proofs.clone(),
        child: Some(ChildSpec {
            exe: PathBuf::from(env!("CARGO_BIN_EXE_agentrix-saas")),
            args: vec![
                "--env-file".into(),
                env_file.display().to_string(),
                "--books".into(),
                "fake".into(),
                "--judge".into(),
            ],
        }),
        config: config.clone(),
        only: Vec::new(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let running = saas_node::run(config, Arc::new(SystemClock), listener, false)
        .await
        .unwrap();
    let report = attacks::run(&running.node, running.port, &relay, &options)
        .await
        .unwrap();

    let sandboxed = adk_sandbox::sandbox::get_enforcer().is_ok();
    let (list, _) = attacks::committed_list(&repo_specs()).unwrap();
    let summary = serde_json::to_string_pretty(&report).unwrap();
    assert_eq!(report.cases.len(), 10, "{summary}");
    assert!(report.committed.starts_with("sha256:1b02dfc7"), "{summary}");
    for case in &report.cases {
        match case.id.as_str() {
            // No settlement has happened on this node to replay (the sandbox run makes one).
            "A05-replayed-mandate" => assert!(case.outcome.starts_with("Not run"), "{summary}"),
            "A01-bec-vendor-email" if !sandboxed => assert!(
                case.evidence
                    .iter()
                    .all(|e| e.holds || e.source == "sandbox"),
                "{summary}"
            ),
            _ => assert!(case.passed, "{} wasn't defended:\n{summary}", case.id),
        }
        // Every check the committed list names is scored, from the source it names.
        if !case.outcome.starts_with("Not run") {
            let entry = list["attacks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["id"] == json!(case.id))
                .unwrap();
            for check in entry["checks"].as_array().unwrap() {
                let source = check.as_str().unwrap().split(':').next().unwrap();
                assert!(
                    case.evidence.iter().any(|e| e.source == source),
                    "{} has no {source} evidence",
                    case.id
                );
            }
        }
    }
    // The logs the checks read verify, and stop verifying with one byte changed.
    for audit in &report.audit {
        assert_eq!(audit["verified"], json!(true), "{summary}");
        assert_eq!(audit["tampered"]["verified"], json!(false), "{summary}");
    }
    for file in [
        "proofs/attacks.json",
        "proofs/attacks.eval.json",
        "proofs/audit/agentrix.jsonl",
        "proofs/audit/agentrix.vkey",
        "proofs/audit/kilele.jsonl",
        "proofs/audit/agentrix-restart.jsonl",
    ] {
        assert!(proofs.join(file).is_file(), "{file}");
    }
    // PROOFS.md's results come from this run's evidence.
    let doc = std::fs::read_to_string(proofs.join("PROOFS.md")).unwrap();
    assert!(doc.contains("attacks defended"), "{doc}");
    assert!(doc.contains("`A08-kill-9-mid-payout` | yes"), "{doc}");
    let (valid, _) = attacks::verify_file(
        &proofs.join("proofs/audit/agentrix.jsonl"),
        &proofs.join("proofs/audit/agentrix.vkey"),
    )
    .unwrap();
    assert!(valid);
}
