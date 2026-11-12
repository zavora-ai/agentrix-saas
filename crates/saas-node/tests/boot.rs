//! The node boots on FakeBooks with PayPal faked over HTTP, serves all four hosts, and a
//! verified PayPal webhook posts to the books, closes the invoice's flow, and shows in the feed.

use std::sync::Arc;

use adk_core::{Clock, SystemClock};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use saas_company::flows::FlowStatus;
use saas_node::Node;
use saas_node::config::{BooksMode, Config, EnvFile};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    // PayPal's signature check refuses the forged delivery and accepts the real ones.
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .and(body_string_contains("forged-transmission"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "verification_status": "FAILURE" })),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/notifications/verify-webhook-signature"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "verification_status": "SUCCESS" })),
        )
        .with_priority(5)
        .mount(&server)
        .await;
    server
}

async fn get(app: &Router, host: &str, uri: &str) -> (StatusCode, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, bytes.to_vec())
}

async fn webhook(app: &Router, event: &Value, transmission: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::post("/paypal/webhook")
                .header(header::HOST, "pay.localhost")
                .header(header::CONTENT_TYPE, "application/json")
                .header("paypal-auth-algo", "SHA256withRSA")
                .header(
                    "paypal-cert-url",
                    "https://api.sandbox.paypal.com/v1/notifications/certs/CERT-1",
                )
                .header("paypal-transmission-id", transmission)
                .header("paypal-transmission-sig", "c2lnbmF0dXJl")
                .header("paypal-transmission-time", "2026-11-10T09:00:00Z")
                .body(Body::from(event.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn receivable(app: &Router, invoice: &str) -> Value {
    let (status, body) = get(app, "money.localhost", "/feed.json").await;
    assert_eq!(status, StatusCode::OK);
    let feed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(feed["source"], "fakebooks");
    feed["receivables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["invoiceId"] == json!(invoice))
        .cloned()
        .unwrap_or_else(|| panic!("{invoice} isn't in the feed"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_boots_and_a_paypal_webhook_posts_wakes_its_flow_and_shows_in_the_feed() {
    let server = paypal().await;
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!(
        "PAYPAL_CLIENT_ID=id\nPAYPAL_CLIENT_SECRET=secret\nPAYPAL_WEBHOOK_ID=WH-1\n\
         KILELE_PAYPAL_CLIENT_ID=kid\nKILELE_PAYPAL_CLIENT_SECRET=ksecret\n\
         AGENTRIX_MODELS_MODE=replay\nSAAS_PAYPAL_BASE_URL={}\nSAAS_DATA_DIR={}\n",
        server.uri(),
        dir.path().display()
    ));
    let config = Config::from_env(&env, BooksMode::Fake, false);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let node = Node::start(config, clock).await.unwrap();
    let app = node.router().unwrap();

    // Every host answers, by Host.
    for (host, uri) in [
        ("money.localhost", "/"),
        ("kilele.money.localhost", "/"),
        ("pay.localhost", "/.well-known/awp.json"),
    ] {
        let (status, _) = get(&app, host, uri).await;
        assert_eq!(status, StatusCode::OK, "{host}{uri}");
    }
    let (status, _) = get(&app, "books.localhost", "/").await;
    assert_ne!(
        status,
        StatusCode::MISDIRECTED_REQUEST,
        "the books host is served"
    );
    assert!(
        dir.path().join("agentrix").join("agentrix.db").exists(),
        "durable stores"
    );
    assert!(
        dir.path().join("models").join("control.db").exists(),
        "the model gateway runs inside the node"
    );

    // Receivables invoices Kilele through the invoice-to-cash flow (on the governed path, inside
    // the mandate); the flow waits for the cash.
    let kilele = node.story.kilele_customer.clone().unwrap().to_string();
    let status = node
        .agentrix
        .flows
        .invoice(
            "kil-q4-seats",
            &kilele,
            json!([{ "description": "Two extra seats, Q4", "amount_usd": 100.0 }]),
            14,
        )
        .await
        .unwrap();
    let FlowStatus::Waiting { data, .. } = status else {
        panic!("waiting for the cash, not {status:?}");
    };
    let invoice = data.unwrap()["invoice_id"].as_str().unwrap().to_owned();
    let before = receivable(&app, &invoice).await;
    assert_eq!(before["openMinor"], json!(10_000));

    // PayPal tells the pay host the capture completed.
    let event = json!({
        "id": "WH-1XY45678AB9012345", "event_type": "PAYMENT.CAPTURE.COMPLETED",
        "resource_type": "capture",
        "resource": {
            "id": "2GG279541U471931P", "status": "COMPLETED",
            "amount": { "currency_code": "USD", "value": "100.00" },
            "custom_id": invoice, "create_time": "2026-11-10T09:05:00Z",
        },
    });
    let (status, handled) = webhook(&app, &event, "real-transmission-1").await;
    assert_eq!(status, StatusCode::OK, "{handled}");
    assert_eq!(handled["posted"], json!(true), "{handled}");
    assert_eq!(handled["flow"]["thread"], json!("i2c-kil-q4-seats"));
    assert_eq!(handled["flow"]["done"], json!(true), "the flow closed");

    // The feed shows it paid.
    let after = receivable(&app, &invoice).await;
    assert_eq!(after["openMinor"], json!(0), "{after}");
    assert_eq!(after["status"], json!("paid"));
    let profile = node
        .agentrix
        .company
        .desk
        .memory
        .profile(&kilele)
        .await
        .unwrap();
    assert_eq!(profile.payments, 1, "the flow remembered how Kilele paid");

    // PayPal delivers it again: nothing posts twice.
    let (status, again) = webhook(&app, &event, "real-transmission-2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["posted"], json!(false), "{again}");

    // A forged delivery is refused before anything is read from it.
    let mut forged = event.clone();
    forged["id"] = json!("WH-FORGED0000000000");
    forged["resource"]["id"] = json!("9FORGED0000000000");
    let (status, _) = webhook(&app, &forged, "forged-transmission").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let still = receivable(&app, &invoice).await;
    assert_eq!(still["openMinor"], json!(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn judge_mode_replays_the_vendor_email_attack_with_a_signed_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!("SAAS_DATA_DIR={}\n", dir.path().display()));
    let config = Config::from_env(&env, BooksMode::Fake, true);
    let node = Node::start(config, Arc::new(SystemClock)).await.unwrap();
    let app = node.router().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::post("/judge/vendor-email")
                .header(header::HOST, "money.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["screen"]["blocked"], json!(false));
    assert!(
        body["refused"][0]
            .as_str()
            .is_some_and(|r| r.contains("payee")),
        "{body}"
    );
    assert_eq!(body["receipt_verified"], json!(true), "{body}");

    // Not on another host, and an unsigned "PayPal" delivery is refused outright.
    let (status, _) = get(&app, "kilele.money.localhost", "/judge").await;
    assert_ne!(status, StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::post("/paypal/webhook")
                .header(header::HOST, "pay.localhost")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::SERVICE_UNAVAILABLE
    ));

    // Run the day: the five scenes tick off in the background; each ends done or failed (with
    // no PayPal app, the money scenes stop and say why).
    let response = app
        .clone()
        .oneshot(
            Request::post("/judge/run-day")
                .header(header::HOST, "money.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let started: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let run = started["run"].as_str().unwrap().to_owned();
    let mut day = Value::Null;
    for _ in 0..120 {
        let (status, body) = get(&app, "money.localhost", &format!("/judge/run-day/{run}")).await;
        assert_eq!(status, StatusCode::OK);
        day = serde_json::from_slice(&body).unwrap();
        if day["done"] == json!(true) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(day["done"], json!(true), "{day}");
    let scenes = day["scenes"].as_array().unwrap();
    assert_eq!(scenes.len(), 5);
    assert!(
        scenes
            .iter()
            .all(|s| s["state"] == json!("done") || s["state"] == json!("failed")),
        "{day}"
    );
    assert_eq!(
        scenes[4]["state"],
        json!("done"),
        "the vendor email is refused: {day}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_kernels_keep_their_state_and_a_freeze_outlives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!("SAAS_DATA_DIR={}\n", dir.path().display()));
    let node = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    // Every kernel store is on the company's database; none is kept in memory.
    for rt in node.runtimes() {
        let in_memory: Vec<String> = rt
            .supervisor
            .kernel()
            .store_uses()
            .iter()
            .filter(|u| u.in_memory)
            .map(|u| u.store.clone())
            .collect();
        assert!(in_memory.is_empty(), "kept in memory: {in_memory:?}");
    }
    node.agentrix
        .company
        .governance
        .freeze("the founder stopped all money")
        .await
        .unwrap();
    drop(node);

    let again = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let mut frozen = false;
    for _ in 0..40 {
        frozen = again.agentrix.company.governance.control.is_frozen();
        if frozen {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(frozen, "the freeze came back after the restart");
    assert!(!again.kilele.company.governance.control.is_frozen());
}

async fn as_role(
    rt: &saas_company::runtime::Runtime,
    role: &str,
    call: &str,
    tool: &str,
) -> anyhow::Result<saas_company::governed::CallOutcome> {
    let member = rt.company.member(role)?;
    let scope = adk_core::ApprovalScope::new(
        member.identity.app_name().as_str(),
        member.user(),
        "revoke-test",
    );
    rt.company
        .governance
        .call(
            &rt.company.desk,
            &member.identity,
            &scope,
            "revoke-test",
            tool,
            &format!("revoke-test/{call}"),
            json!({}),
            None,
        )
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_revokes_one_agent_and_only_a_passkey_brings_it_back() {
    use saas_company::governed::CallOutcome;
    use saas_company::identity::{PAYABLES, SUPPORT};
    use saas_web::console::Console;
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!("SAAS_DATA_DIR={}\n", dir.path().display()));
    let node = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let console = saas_node::console::NodeConsole::new(node.runtimes());
    let rt = &node.agentrix;

    let before = as_role(rt, PAYABLES, "1", "ap_bills_due").await.unwrap();
    assert!(matches!(before, CallOutcome::Done(_)), "{before:?}");

    // Switched off: its next call stops at the token check, and no PayPal key changed.
    let revoked = console
        .set_revoked("agentrix", PAYABLES, true, None)
        .await
        .unwrap();
    assert_eq!(revoked["agent"], json!("urn:adk:agent:agentrix/payables"));
    let stopped = as_role(rt, PAYABLES, "2", "ap_bills_due").await;
    let said = format!("{stopped:?}");
    assert!(said.contains("revoked"), "{said}");
    // The others keep working.
    let chief = as_role(rt, saas_company::identity::CHIEF, "3", "company_brief")
        .await
        .unwrap();
    assert!(matches!(chief, CallOutcome::Done(_)), "{chief:?}");

    // Back on only with the passkey.
    assert!(
        console
            .set_revoked("agentrix", PAYABLES, false, None)
            .await
            .is_err()
    );
    assert!(
        console
            .set_revoked(
                "agentrix",
                PAYABLES,
                false,
                Some(json!({ "userVerified": false }))
            )
            .await
            .is_err()
    );
    // A passkey over something else doesn't switch it back on.
    assert!(
        console
            .set_revoked(
                "agentrix",
                PAYABLES,
                false,
                Some(json!({ "userVerified": true,
                    "fingerprint": saas_web::console::restore_fingerprint("agentrix", "support") })),
            )
            .await
            .is_err()
    );
    // Revoking again answers the same.
    let twice = console
        .set_revoked("agentrix", PAYABLES, true, None)
        .await
        .unwrap();
    assert_eq!(twice["revoked"], json!(true));
    console
        .set_revoked(
            "agentrix",
            PAYABLES,
            false,
            Some(
                json!({ "kind": "passkey", "credential": "cred-1", "userVerified": true,
                "fingerprint": saas_web::console::restore_fingerprint("agentrix", PAYABLES) }),
            ),
        )
        .await
        .unwrap();
    let again = as_role(rt, PAYABLES, "4", "ap_bills_due").await.unwrap();
    assert!(matches!(again, CallOutcome::Done(_)), "{again:?}");

    let records = rt.company.governance.audit.records().await.unwrap();
    assert!(records.iter().any(|r| r.action == "agent.revoke"));
    assert!(records.iter().any(|r| r.action == "agent.restore"));
    assert!(
        rt.company
            .governance
            .audit
            .verify()
            .await
            .unwrap()
            .is_valid()
    );

    // A revocation outlives a restart.
    console
        .set_revoked("agentrix", SUPPORT, true, None)
        .await
        .unwrap();
    drop(console);
    drop(node);
    let again = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let access = again.agentrix.company.access().unwrap();
    assert!(access.is_revoked(SUPPORT));
    assert!(!access.is_revoked(PAYABLES));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_node_runs_the_day_on_a_node_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!(
        "AGENTRIX_MODELS_MODE=replay\nSAAS_DATA_DIR={}\n",
        dir.path().display()
    ));
    // Not judge mode: the live companies' models, stores and books stay out of the run.
    let node = Node::start(
        Config::from_env(&env, BooksMode::Fake, false),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let app = node.router().unwrap();
    let before = node
        .agentrix
        .company
        .governance
        .audit
        .records()
        .await
        .unwrap()
        .len();

    let response = app
        .clone()
        .oneshot(
            Request::post("/judges/run-day")
                .header(header::HOST, "money.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let started: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let run = started["run"].as_str().unwrap().to_owned();
    let mut day = Value::Null;
    for _ in 0..240 {
        let (status, body) = get(&app, "money.localhost", &format!("/judges/run-day/{run}")).await;
        assert_eq!(status, StatusCode::OK);
        day = serde_json::from_slice(&body).unwrap();
        if day["done"] == json!(true) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(day["done"], json!(true), "{day}");
    assert_eq!(day["scenes"].as_array().unwrap().len(), 5, "{day}");
    // The vendor email is refused on the run's own node.
    assert_eq!(day["scenes"][4]["state"], json!("done"), "{day}");

    // Nothing reached the live company, and the run left nothing behind.
    let after = node
        .agentrix
        .company
        .governance
        .audit
        .records()
        .await
        .unwrap()
        .len();
    assert_eq!(before, after, "the live audit log is untouched");
    assert!(!dir.path().join("judge-runs").join(&run).exists());

    // A second start within five minutes answers the same run.
    let again = app
        .clone()
        .oneshot(
            Request::post("/judges/run-day")
                .header(header::HOST, "money.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let again: Value = serde_json::from_slice(
        &axum::body::to_bytes(again.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(again["run"], json!(run));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_live_schedules_sleep_on_the_kernel_and_a_restart_keeps_one_of_each() {
    use saas_company::supervisor::live_schedules;
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!("SAAS_DATA_DIR={}\n", dir.path().display()));
    let node = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let standing = live_schedules(true);
    assert_eq!(standing.len(), 7);
    let first = node
        .agentrix
        .supervisor
        .schedule_standing(&standing)
        .await
        .unwrap();
    let again = node
        .agentrix
        .supervisor
        .schedule_standing(&standing)
        .await
        .unwrap();
    assert_eq!(first, again, "scheduling again finds the same processes");
    for pid in &first {
        let process = node
            .agentrix
            .supervisor
            .kernel()
            .processes()
            .get(pid)
            .await
            .unwrap()
            .expect("the scheduled process");
        let wake = serde_json::to_string(&process.meta.wake).unwrap();
        assert!(wake.contains("cron"), "{wake}");
    }
    // Kilele runs payroll and pays what is due, but bills no model spend and invoices no
    // usage.
    let kilele: Vec<&str> = live_schedules(false).iter().map(|s| s.name).collect();
    assert_eq!(
        kilele,
        ["collections", "brief", "reconcile", "payroll", "payables"]
    );
}

/// The operators' activity page reads the node's own records: an agent's tool call and the
/// owner's revoke come through from the company's signed audit log, and the day's model spend
/// is measured against the daily cap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_activity_page_shows_what_the_agents_did_from_the_signed_logs() {
    use saas_company::governed::CallOutcome;
    use saas_company::identity::PAYABLES;
    use saas_web::console::Console;

    struct Anyone;
    #[async_trait::async_trait]
    impl saas_web::activity::OperatorGate for Anyone {
        async fn allows(&self, _: &axum::http::HeaderMap) -> bool {
            true
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!(
        "SAAS_DATA_DIR={}\nSAAS_DAILY_MODEL_CAP_USD=10\n",
        dir.path().display()
    ));
    let node = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    let rt = &node.agentrix;
    let done = as_role(rt, PAYABLES, "1", "ap_bills_due").await.unwrap();
    assert!(matches!(done, CallOutcome::Done(_)), "{done:?}");
    saas_node::console::NodeConsole::new(node.runtimes())
        .set_revoked("agentrix", PAYABLES, true, None)
        .await
        .unwrap();

    // Node's control.db meters model calls in `usage`: today's spend is the companies' sites'.
    let control = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE usage (site_id TEXT NOT NULL, cost_micro_usd INTEGER NOT NULL, at TEXT NOT NULL)",
    )
    .execute(&control)
    .await
    .unwrap();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    for (site, cost) in [
        ("saas-agentrix", 1_000_000),
        ("saas-kilele", 250_000),
        ("classy-perfumes-sample", 9_000_000),
    ] {
        sqlx::query("INSERT INTO usage VALUES (?, ?, ?)")
            .bind(site)
            .bind(cost)
            .bind(&now)
            .execute(&control)
            .await
            .unwrap();
    }
    let page = saas_web::activity::router(
        saas_node::activity::activity(&node, control),
        Arc::new(Anyone),
    );
    let response = page
        .oneshot(
            Request::get("/activity/rows.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let view: Value = serde_json::from_slice(&body).unwrap();
    let rows = view["rows"].as_array().unwrap();
    assert!(
        rows.iter().any(|r| r["company"] == json!("agentrix")
            && r["tool"] == json!("ap_bills_due")
            && r["agent"] == json!("payables")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|r| r["company"] == json!("agentrix")
            && r["what"].as_str().is_some_and(|w| w.contains("revoke"))),
        "{rows:#?}"
    );
    assert_eq!(view["totals"]["cap"], json!("$10.00 a day"), "{view:#}");
    assert_eq!(view["totals"]["spend"], json!("$1.25"), "{view:#}");
    let sources = view["sources"].to_string();
    assert!(sources.contains("Signed audit log: agentrix"), "{sources}");
    assert!(sources.contains("Signed audit log: kilele"), "{sources}");
}

/// The operator console's dock shows each of Agentrix's agents as its kernel has it (Payables
/// asleep until its next scheduled run, with the last thing it did), and the askline's question
/// reaches Agentrix's Chief on the governed path, its steps coming as they happen.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_operator_console_sees_agentrixs_real_agents_and_asks_its_chief() {
    use saas_company::identity::{CHIEF, PAYABLES};
    use saas_company::staff::TurnStep;
    let dir = tempfile::tempdir().unwrap();
    let env = EnvFile::parse(&format!("SAAS_DATA_DIR={}\n", dir.path().display()));
    let node = Node::start(
        Config::from_env(&env, BooksMode::Fake, true),
        Arc::new(SystemClock),
    )
    .await
    .unwrap();
    node.agentrix
        .supervisor
        .schedule_standing(&saas_company::supervisor::live_schedules(true))
        .await
        .unwrap();
    as_role(&node.agentrix, PAYABLES, "1", "ap_bills_due")
        .await
        .unwrap();

    // A schedule whose last tick was missed fires once at start: wait for those runs to end.
    let mut dock = saas_node::ops_console::agents(&node).await;
    for _ in 0..100 {
        let busy = dock["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["role"] == "payables" && a["state"] == "working");
        if !busy {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        dock = saas_node::ops_console::agents(&node).await;
    }
    let agents = dock["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 5, "{dock}");
    assert_eq!(dock["chief"]["ready"], true);
    let payables = agents.iter().find(|a| a["role"] == "payables").unwrap();
    assert_eq!(payables["state"], "sleeping", "{payables}");
    assert!(
        payables["words"]
            .as_str()
            .unwrap()
            .starts_with("Asleep until"),
        "{payables}"
    );
    assert!(
        payables["last"]["what"]
            .as_str()
            .is_some_and(|w| w.starts_with("ap bills due")),
        "{payables}"
    );

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let report = node
        .agentrix
        .company
        .ask_observed(CHIEF, "ops-test", "How much cash do we have?", Some(&tx))
        .await
        .unwrap();
    drop(tx);
    let mut steps = Vec::new();
    while let Ok(step) = rx.try_recv() {
        steps.push(step);
    }
    assert!(!report.text.is_empty(), "{report:?}");
    assert!(
        steps
            .iter()
            .any(|s| matches!(s, TurnStep::Text { text } if *text == report.text)),
        "{steps:?}"
    );
}
