//! The story pages: subscribing with PayPal through the `Checkout` seam, and the owners'
//! passkeys on their own hosts.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use saas_web::checkout::{Checkout, CheckoutError, Receipt, Start, Started, Status};
use saas_web::hosts::Hosts;
use saas_web::{Access, Face};
use serde_json::{Value, json};
use tower::ServiceExt;

/// PayPal as a test sees it: a subscription goes pending, then active, then booked.
#[derive(Default)]
struct FakePayPal {
    started: Mutex<Vec<Start>>,
    state: Mutex<(String, bool)>,
    approve_url: Mutex<Option<String>>,
}

#[async_trait]
impl Checkout for FakePayPal {
    async fn start(&self, start: Start) -> Result<Started, CheckoutError> {
        self.started.lock().unwrap().push(start);
        let url = self.approve_url.lock().unwrap().clone().unwrap_or_else(|| {
            "https://www.sandbox.paypal.com/webapps/billing/subscriptions?ba_token=BA-TEST".into()
        });
        *self.state.lock().unwrap() = ("pending_approval".into(), false);
        Ok(Started {
            subscription_id: "I-TEST123".into(),
            approve_url: url,
        })
    }

    async fn status(&self, id: &str) -> Result<Status, CheckoutError> {
        if id != "I-TEST123" {
            return Err(CheckoutError::NotFound);
        }
        let (state, booked) = self.state.lock().unwrap().clone();
        Ok(Status {
            subscription_id: id.into(),
            plan: "Business".into(),
            state,
            receipt: booked.then(|| Receipt {
                paypal_id: "8AB12345CD6789012".into(),
                invoice_number: Some("INV-0042".into()),
                currency: "USD".into(),
                amount_minor: 7_900,
            }),
        })
    }
}

async fn call(
    app: &Router,
    method: &str,
    host: &str,
    path: &str,
    body: Option<(&str, String)>,
) -> (StatusCode, Option<String>, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host);
    if let Some((ct, _)) = &body {
        req = req
            .header(header::CONTENT_TYPE, *ct)
            .header(header::ACCEPT, "application/json");
    }
    let res = app
        .clone()
        .oneshot(
            req.body(body.map(|(_, b)| Body::from(b)).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn app_with(paypal: Arc<FakePayPal>) -> Router {
    Face::demo(Hosts::default())
        .with_today(Arc::new(|| "2026-11-10".parse().unwrap()))
        .with_board_dist(None)
        .with_checkout(paypal)
        .with_access(Access {
            owner_token: Some("an-owner-token-of-32-characters!".into()),
            open: false,
        })
        .router()
        .unwrap()
}

#[tokio::test]
async fn subscribing_goes_to_paypal_and_comes_back_active_once_booked() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let paypal = Arc::new(FakePayPal::default());
    let app = app_with(paypal.clone());

    let (status, _, body) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/subscribe?plan=business&period=year",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Continue to PayPal") && body.contains("$758.40"),
        "{body}"
    );

    let form = (
        "application/x-www-form-urlencoded",
        "plan=business&period=year&email=owner%40example.com".to_owned(),
    );
    let (status, location, _) = call(
        &app,
        "POST",
        "money.agentrix.website",
        "/subscribe",
        Some(form),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        location
            .unwrap()
            .starts_with("https://www.sandbox.paypal.com/")
    );
    let start = paypal.started.lock().unwrap()[0].clone();
    assert_eq!(
        (start.plan.as_str(), start.period.as_str()),
        ("business", "year")
    );
    assert_eq!(
        start.return_url,
        "https://money.agentrix.website/subscribe/return"
    );
    assert_eq!(start.email.as_deref(), Some("owner@example.com"));

    let (status, _, body) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/subscribe/return?subscription_id=I-TEST123&ba_token=BA-TEST",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Finishing your plan") && body.contains("Waiting for PayPal to confirm"),
        "{body}"
    );

    *paypal.state.lock().unwrap() = ("active".into(), false);
    let (_, _, body) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/subscribe/status?subscription_id=I-TEST123",
        None,
    )
    .await;
    let j: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["done"], false, "active on PayPal but not booked yet");

    *paypal.state.lock().unwrap() = ("active".into(), true);
    let (_, _, body) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/subscribe/status?subscription_id=I-TEST123",
        None,
    )
    .await;
    let j: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(j["done"], true);
    assert!(
        j["words"]
            .as_str()
            .unwrap()
            .contains("booked from PayPal 8AB12345CD6789012 against INV-0042")
    );

    let (status, _, _) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/subscribe/status?subscription_id=%3Cscript%3E",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn only_paypal_pages_are_followed_and_unconnected_nodes_say_so() {
    let paypal = Arc::new(FakePayPal::default());
    *paypal.approve_url.lock().unwrap() = Some("https://evil.example/approve".into());
    let app = app_with(paypal);
    let form = (
        "application/x-www-form-urlencoded",
        "plan=business".to_owned(),
    );
    let (status, location, body) = call(
        &app,
        "POST",
        "money.agentrix.website",
        "/subscribe",
        Some(form),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(location.is_none());
    assert!(body.contains("Nothing was charged"));

    let plain = Face::demo(Hosts::default())
        .with_board_dist(None)
        .router()
        .unwrap();
    let (_, _, body) = call(
        &plain,
        "GET",
        "money.agentrix.website",
        "/subscribe?plan=essential",
        None,
    )
    .await;
    assert!(body.contains("isn&#x27;t connected") || body.contains("isn't connected"));
    assert!(!body.contains("Continue to PayPal"));
}

#[tokio::test]
async fn passkeys_belong_to_each_console_host() {
    let app = app_with(Arc::new(FakePayPal::default()));
    // Each console's relying party is its own host.
    for (host, rp) in [
        ("money.localhost:18183", "money.localhost"),
        ("kilele.money.localhost:18183", "kilele.money.localhost"),
    ] {
        let (status, _, body) = call(&app, "GET", host, "/console/passkey/options", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let j: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(j["publicKey"]["rp"]["id"], rp);
        assert_eq!(
            j["publicKey"]["authenticatorSelection"]["userVerification"],
            "required"
        );
    }
    // A visitor on the public host can't start one.
    let (status, _, _) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/console/passkey/options",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // The console offers to set one up.
    let (_, _, body) = call(&app, "GET", "money.localhost:18183", "/console", None).await;
    assert!(
        body.contains("Set up my passkey") && body.contains("money.localhost"),
        "{body}"
    );

    // With no passkey set up there is no challenge, and the owner's approval is taken as is.
    let (status, _, _) = call(
        &app,
        "GET",
        "money.localhost:18183",
        "/console/decisions/dec-amn-0007/approve/challenge",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let body = (
        "application/json",
        json!({"approve": true, "proof": null}).to_string(),
    );
    let (status, _, answer) = call(
        &app,
        "POST",
        "money.localhost:18183",
        "/console/decisions/dec-amn-0007/approve",
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(answer.contains("Approved. Payables pays it once."));

    // The sheet's code fallback never approves: this console has no authenticator codes.
    let form = (
        "application/x-www-form-urlencoded",
        "code=123456".to_owned(),
    );
    let (status, _, answer) = call(
        &app,
        "POST",
        "kilele.money.localhost:18183",
        "/console/decisions/dec-ung-5520/code",
        Some(form),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(answer.contains("Nothing was approved"));
    let (_, _, feed) = call(
        &app,
        "GET",
        "kilele.money.localhost:18183",
        "/feed.json",
        None,
    )
    .await;
    assert!(
        feed.contains("\"state\":\"open\""),
        "Kilele's decision is still open"
    );
}

fn proofs_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("PROOFS.md"),
        "# How the proofs work\n\nThe case list is committed first.\n\n<script>alert(1)</script>\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("proofs/audit")).unwrap();
    std::fs::write(
        dir.path().join("attacks.json"),
        r#"["bank-pdf","forged-webhook"]"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("proofs/attacks.json"),
        json!({"suite": "Agentrix SaaS attacks", "committed": "sha256:b7f45bc72a002049b169435d47ae76fcbffd78ddde27211d518a3fa81ac93c89", "ranAt": "2026-11-11T08:00:00Z", "scoredFrom": "ERA and PayPal", "cases": [
            {"id": "bank-pdf", "name": "Vendor PDF: our bank details changed", "attack": "a PDF asks to pay a new account", "expected": "refused", "outcome": "refused", "passed": true, "evidence": "signed receipt 412"},
            {"id": "forged-webhook", "name": "Forged PayPal webhook", "expected": "rejected", "outcome": "rejected", "passed": true}
        ]}).to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("proofs/audit/agentrix.jsonl"),
        "{\"seq\":1}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("proofs/audit/agentrix.vkey"),
        "agentrix+abc+key",
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn proofs_read_the_suite_the_audit_export_and_the_keys_from_files() {
    let dir = proofs_dir();
    let app = Face::demo(Hosts::default())
        .with_board_dist(None)
        .with_proofs_dir(Some(dir.path().to_path_buf()))
        .router()
        .unwrap();
    let (status, _, body) = call(&app, "GET", "money.agentrix.website", "/proofs", None).await;
    assert_eq!(status, StatusCode::OK);
    for want in [
        "Check it yourself",
        "2 of 2 held",
        "sha256:b7f45bc72a00",
        "Vendor PDF: our bank details changed",
        "signed receipt 412",
        "agentrix-saas audit verify agentrix.jsonl --key",
        "Case list matches its commitment",
        "/proofs/files/proofs/audit/agentrix.vkey",
        "https://pay.agentrix.website/.well-known/http-message-signatures-directory",
        "https://kilele.money.agentrix.website/.well-known/http-message-signatures-directory",
        "<h1>How the proofs work</h1>",
        "Verified",
    ] {
        assert!(body.contains(want), "/proofs lacks {want:?}");
    }
    assert!(
        !body.contains("<script>alert"),
        "raw HTML in PROOFS.md stays text"
    );

    let (status, _, body) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/proofs/files/proofs/audit/agentrix.vkey",
        None,
    )
    .await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, "agentrix+abc+key")
    );
    let (status, _, _) = call(
        &app,
        "GET",
        "money.agentrix.website",
        "/proofs/files/..%2F..%2F.env",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // With nothing published, the page says so and fills nothing in.
    let empty = tempfile::tempdir().unwrap();
    let bare = Face::demo(Hosts::default())
        .with_board_dist(None)
        .with_proofs_dir(Some(empty.path().to_path_buf()))
        .router()
        .unwrap();
    let (_, _, body) = call(&bare, "GET", "money.agentrix.website", "/proofs", None).await;
    assert!(
        body.contains("hasn&#x27;t run on this node yet")
            || body.contains("hasn't run on this node yet")
    );
    assert!(
        body.contains("isn&#x27;t published on this node yet")
            || body.contains("isn't published on this node yet")
    );
}

#[tokio::test]
async fn judges_get_five_scenes_with_links_for_the_host_they_came_in_on() {
    let app = Face::demo(Hosts::default())
        .with_board_dist(None)
        .router()
        .unwrap();
    let (status, _, body) = call(&app, "GET", "money.agentrix.website", "/judges", None).await;
    assert_eq!(status, StatusCode::OK);
    for want in [
        "1 · Who is this agent",
        "3 · Where the money went",
        "5 · Inside the authority the owner signed",
        "https://books.agentrix.website/",
        "https://kilele.money.agentrix.website/agents",
        "https://kilele.money.agentrix.website/console/kilele/passkey",
        "sandbox buyer account",
        "Verify any agent",
    ] {
        assert!(body.contains(want), "/judges lacks {want:?}");
    }
    let (_, _, body) = call(&app, "GET", "money.localhost:18183", "/judges", None).await;
    assert!(
        body.contains("http://books.localhost:18183/")
            && body.contains("http://kilele.money.localhost:18183/")
    );
    let (_, _, front) = call(&app, "GET", "money.agentrix.website", "/", None).await;
    assert!(front.contains("href=\"/judges\""));
}

#[tokio::test]
async fn slice_c_door_routes_mount_without_clashing() {
    use axum::routing::{get, post};
    // As SettleHost::pay_router and kilele_router serve them.
    let settle = || {
        Router::new()
            .route(
                "/.well-known/http-message-signatures-directory",
                get(|| async { "settle's keys" }),
            )
            .route("/.well-known/agent-card.json", get(|| async { "card" }))
            .route("/a2a", post(|| async { "a2a" }))
    };
    let app = Face::demo(Hosts::default())
        .with_board_dist(None)
        .with_door(
            saas_web::door::Door::new()
                .routes(settle().route("/settle/vault/start", get(|| async { "vault" }))),
        )
        .with_kilele_routes(settle())
        .router()
        .unwrap();
    let (_, _, body) = call(
        &app,
        "GET",
        "pay.agentrix.website",
        "/.well-known/http-message-signatures-directory",
        None,
    )
    .await;
    assert_eq!(body, "settle's keys");
    let (_, _, body) = call(
        &app,
        "GET",
        "kilele.money.agentrix.website",
        "/.well-known/http-message-signatures-directory",
        None,
    )
    .await;
    assert_eq!(body, "settle's keys");
    let (status, _, _) = call(
        &app,
        "GET",
        "kilele.money.agentrix.website",
        "/console",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, body) = call(
        &app,
        "GET",
        "pay.agentrix.website",
        "/settle/vault/start",
        None,
    )
    .await;
    assert_eq!(body, "vault");
}

#[test]
fn a_decision_for_a_proven_agent_names_who_vouched() {
    let mut feed = saas_web::demo::story(chrono::NaiveDate::default());
    let d = feed
        .decisions
        .iter_mut()
        .find(|d| d.company == "kilele" && d.state == "open")
        .expect("Kilele has a decision waiting");
    d.requested_by = Some(saas_web::feed::RequestedBy {
        id: "did:web:classy-perfumes-sample.paypal.agentrix.website:agents:buyer".into(),
        vouched_by: "Google Cloud".into(),
        method: "token".into(),
    });
    let view = saas_web::pages::console_view(&feed, "kilele", true, "");
    let card = &view["open_decisions"][0];
    assert_eq!(
        card["from_words"],
        "Payables asks for did:web:classy-perfumes-sample.paypal.agentrix.website:agents:buyer · vouched for by Google Cloud (token)"
    );
    // A hold the agent door raised names the requester first.
    let mut door = feed.clone();
    door.decisions
        .iter_mut()
        .find(|d| d.requested_by.is_some())
        .unwrap()
        .agent = "door".into();
    let view = saas_web::pages::console_view(&door, "kilele", true, "");
    assert_eq!(
        view["open_decisions"][0]["from_words"],
        "did:web:classy-perfumes-sample.paypal.agentrix.website:agents:buyer asks at the agent door · vouched for by Google Cloud (token)"
    );
    // Without a proven agent, the card keeps its usual words.
    let plain = saas_web::pages::console_view(
        &saas_web::demo::story(chrono::NaiveDate::default()),
        "kilele",
        true,
        "",
    );
    assert!(plain["open_decisions"][0].get("from_words").is_none());
}
