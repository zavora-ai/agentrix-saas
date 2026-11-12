//! Every host's routes, through the one router, as a browser or an agent would call them.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::NaiveDate;
use saas_web::feed::Feed;
use saas_web::hosts::Hosts;
use saas_web::{Access, Face};
use serde_json::{Value, json};
use tower::ServiceExt;

fn as_of() -> NaiveDate {
    "2026-11-10".parse().unwrap()
}

async fn app() -> Router {
    Face::demo(Hosts::default())
        .with_today(Arc::new(as_of))
        .with_board_dist(None)
        .with_access(Access {
            owner_token: Some("an-owner-token-of-32-characters!".into()),
            open: false,
        })
        .router()
        .unwrap()
}

struct Got {
    status: StatusCode,
    content_type: String,
    body: String,
}

impl Got {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

async fn call(
    app: &Router,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
) -> Got {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = app
        .clone()
        .oneshot(
            req.body(body.map(Body::from).unwrap_or_else(Body::empty))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    Got {
        status,
        content_type,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, host: &str, path: &str) -> Got {
    call(app, "GET", host, path, &[], None).await
}

const MONEY: &str = "money.agentrix.website";
const KILELE: &str = "kilele.money.agentrix.website";
const PAY: &str = "pay.agentrix.website";
const BOOKS: &str = "books.agentrix.website";
const MONEY_LOCAL: &str = "money.localhost:18183";
const KILELE_LOCAL: &str = "kilele.money.localhost:18183";

#[tokio::test]
async fn money_serves_the_front_plans_desks_and_kit() {
    let app = app().await;
    let r = get(&app, MONEY, "/").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.content_type.starts_with("text/html"));
    for want in [
        "An AI company that bills, collects, pays and settles",
        "Business",
        "$79.00",
        "Talk to us",
        "Receivables",
        "/static/ui/kit.css",
    ] {
        assert!(r.body.contains(want), "front lacks {want:?}");
    }
    assert_eq!(get(&app, MONEY, "/plans").await.status, StatusCode::OK);
    let r = get(&app, MONEY, "/subscribe?plan=business").await;
    assert!(r.body.contains("Subscribe to Business"));
    assert!(r.body.contains("Nothing can be charged here"));

    let r = call(
        &app,
        "POST",
        MONEY,
        "/desk/sales",
        &[("content-type", "application/json")],
        Some(json!({"q": "What does the Business plan include?"}).to_string()),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    let j = r.json();
    assert!(j["text"].as_str().unwrap().contains("$79.00 a month"));
    assert_eq!(j["ai"], false);
    let r = call(
        &app,
        "POST",
        MONEY,
        "/desk/nope",
        &[("content-type", "application/json")],
        Some(json!({"q": "hi"}).to_string()),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    for path in [
        "/static/ui/kit.css",
        "/static/ui/tokens.css",
        "/static/saas/saas.css",
        "/static/saas/desk.js",
        "/static/saas/console.js",
    ] {
        assert_eq!(
            get(&app, MONEY, path).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
}

#[tokio::test]
async fn the_feed_is_whole_and_adds_up() {
    let app = app().await;
    let r = get(&app, MONEY, "/feed.json").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.content_type.starts_with("application/json"));
    let feed: Feed = serde_json::from_str(&r.body).unwrap();
    assert_eq!(feed.version, 1);
    assert_eq!(feed.as_of, as_of());
    assert_eq!(feed.companies.len(), 2);
    assert!(
        !feed.receivables.is_empty() && !feed.payables.is_empty() && !feed.debt_plans.is_empty()
    );
    assert!(
        !feed.cash.is_empty()
            && !feed.mrr.is_empty()
            && !feed.agent_spend.is_empty()
            && !feed.mandates.is_empty()
    );

    // Ageing is the open receivables, bucket by bucket.
    for company in ["agentrix", "kilele"] {
        let open: i64 = feed
            .receivables
            .iter()
            .filter(|r| r.company == company)
            .map(|r| r.open_minor)
            .sum();
        let aged: i64 = feed
            .ageing
            .iter()
            .filter(|a| a.company == company)
            .map(|a| a.total_minor)
            .sum();
        assert_eq!(open, aged, "{company}");
        for a in feed.ageing.iter().filter(|a| a.company == company) {
            assert_eq!(
                a.total_minor,
                a.current_minor + a.d1_30_minor + a.d31_60_minor + a.d61_90_minor + a.over90_minor
            );
        }
    }
    // A plan's instalments add up to its principal, and what is paid matches the paid ones.
    for p in &feed.debt_plans {
        assert_eq!(
            p.instalments.iter().map(|i| i.amount_minor).sum::<i64>(),
            p.principal_minor
        );
        assert_eq!(
            p.instalments
                .iter()
                .filter(|i| i.status == "paid")
                .map(|i| i.amount_minor)
                .sum::<i64>(),
            p.paid_minor
        );
        let inv = feed
            .receivables
            .iter()
            .find(|r| r.number == p.invoice_number)
            .unwrap();
        assert_eq!(inv.open_minor, p.principal_minor - p.paid_minor);
    }
    // Every settlement's PayPal id is a demo one: real ids come only from the sandbox.
    assert!(
        feed.settlements
            .iter()
            .all(|s| s.paypal_id.starts_with("DEMO-"))
    );

    // Kilele's host shows Kilele's rows only.
    let kilele: Feed = serde_json::from_str(&get(&app, KILELE, "/feed.json").await.body).unwrap();
    assert_eq!(kilele.companies.len(), 1);
    assert!(kilele.receivables.iter().all(|r| r.company == "kilele"));
    assert!(kilele.payables.iter().all(|r| r.company == "kilele"));
    assert!(kilele.mandates.iter().all(|r| r.company == "kilele"));
    assert!(kilele.mrr.is_empty());
    assert_eq!(
        kilele.debt_plans.len(),
        1,
        "Kilele sees the plan it owes on"
    );
}

#[tokio::test]
async fn the_owner_console_shows_mandates_decisions_and_the_freeze() {
    let app = app().await;
    let r = get(&app, MONEY, "/console").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.body.contains("James&#x27;s mandate for Payables")
            || r.body.contains("James's mandate for Payables")
    );
    assert!(r.body.contains("Pay Amina Yusuf $360.00 for design work"));
    assert!(
        r.body.contains("approve-dec-amn-0007"),
        "the approval sheet is there"
    );
    assert!(r.body.contains("AP2-style intent"));
    assert!(!r.body.contains("compliant"));
    // No freeze button for a visitor on the public host without the owner's token.
    assert!(!r.body.contains("Freeze my agents"));
    assert!(
        !r.body.contains("default-src"),
        "CSP is a header, not markup"
    );

    // The public host refuses a visitor's freeze, and accepts the owner's token.
    let r = call(
        &app,
        "POST",
        MONEY,
        "/console/freeze",
        &[("content-type", "application/x-www-form-urlencoded")],
        Some("frozen=1".into()),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = call(
        &app,
        "POST",
        MONEY,
        "/console/freeze",
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("cookie", "saas_owner=an-owner-token-of-32-characters!"),
        ],
        Some("frozen=1".into()),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let feed = get(&app, MONEY, "/feed.json").await.json();
    assert_eq!(feed["companies"][0]["frozen"], true);
    assert_eq!(feed["mandates"][0]["status"], "frozen");

    // Frozen: an approval is refused. Unfrozen: it goes through once.
    let approve = |host: &'static str| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                host,
                "/console/decisions/dec-amn-0007/approve",
                &[
                    ("content-type", "application/json"),
                    ("accept", "application/json"),
                ],
                Some(json!({"approve": true, "proof": null}).to_string()),
            )
            .await
        }
    };
    let r = approve(MONEY_LOCAL).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert!(r.json()["message"].as_str().unwrap().contains("frozen"));
    let r = call(
        &app,
        "POST",
        MONEY_LOCAL,
        "/console/freeze",
        &[("content-type", "application/x-www-form-urlencoded")],
        Some("frozen=0".into()),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let r = approve(MONEY_LOCAL).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.json()["outcome"], "approved");
    let r = approve(MONEY_LOCAL).await;
    assert_eq!(
        r.status,
        StatusCode::CONFLICT,
        "a second approval is refused"
    );
    let feed = get(&app, MONEY, "/feed.json").await.json();
    let d = feed["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "dec-amn-0007")
        .unwrap()
        .clone();
    assert_eq!(d["state"], "approved");

    // Kilele's decision is not Agentrix's to take.
    let r = call(
        &app,
        "POST",
        MONEY_LOCAL,
        "/console/decisions/dec-ung-5520/approve",
        &[("accept", "application/json")],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn kilele_has_its_own_console() {
    let app = app().await;
    for path in ["/", "/console"] {
        let r = get(&app, KILELE, path).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        for want in [
            "Kilele Bakery",
            "Wanjiru",
            "Unga Millers",
            "INV-1042",
            "Instalment 2",
            "$252.80",
        ] {
            assert!(r.body.contains(want), "{path} lacks {want:?}");
        }
        assert!(
            !r.body.contains("Amina Yusuf"),
            "Agentrix's bills stay on Agentrix's console"
        );
    }
    let r = call(
        &app,
        "POST",
        KILELE_LOCAL,
        "/console/decisions/dec-ung-5520/decline",
        &[("accept", "application/json")],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert_eq!(r.json()["outcome"], "declined");
    // The money host's board isn't Kilele's.
    assert_eq!(
        get(&app, KILELE, "/board/").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_agent_door_publishes_awp_keys_and_waits_for_a2a() {
    let app = app().await;
    let r = get(&app, PAY, "/").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("The agent door"));

    let r = get(&app, PAY, "/.well-known/awp.json").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.json()["manifest"],
        "https://pay.agentrix.website/awp/v2/manifest"
    );

    let r = call(
        &app,
        "GET",
        PAY,
        "/awp/v2/manifest",
        &[("awp-version", "2.0")],
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert!(
        r.body.contains("list_plans") && r.body.contains("quote_plan"),
        "{}",
        r.body
    );

    let r = call(
        &app,
        "POST",
        PAY,
        "/awp/v2/capabilities/quote_plan",
        &[("awp-version", "2.0"), ("content-type", "application/json")],
        Some(json!({"plan": "business", "period": "year"}).to_string()),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert!(r.body.contains("75840"), "{}", r.body);

    let r = get(&app, PAY, "/.well-known/http-message-signatures-directory").await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.content_type,
        "application/http-message-signatures-directory+json"
    );
    assert_eq!(r.json()["keys"], json!([]));

    let r = get(&app, PAY, "/.well-known/agentrix/agents.json").await;
    assert_eq!(
        r.json()["companies"][1]["agents"][1]["keyId"],
        "kilele-payables-2026-11"
    );

    let r = call(&app, "POST", PAY, "/a2a", &[], Some("{}".into())).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.json()["error"], "not_mounted");
    // The console is not on the door.
    assert_eq!(
        get(&app, PAY, "/console").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn books_is_a_stub_until_era_is_connected() {
    let app = app().await;
    let r = get(&app, BOOKS, "/").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.body.contains("Zavora ERA"));
    let r = get(&app, BOOKS, "/api/v1/invoices").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(r.json()["error"], "era_not_connected");
}

#[tokio::test]
async fn unknown_hosts_and_the_unbuilt_board() {
    let app = app().await;
    assert_eq!(
        get(&app, "agentrix.website", "/").await.status,
        StatusCode::MISDIRECTED_REQUEST
    );
    let r = get(&app, MONEY, "/board").await;
    assert_eq!(r.status, StatusCode::PERMANENT_REDIRECT);
    let r = get(&app, MONEY, "/board/").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(r.body.contains("isn&#x27;t built") || r.body.contains("isn't built"));
    let r = get(&app, MONEY, "/board/config.json").await;
    assert_eq!(
        r.json(),
        json!({"licenseKey": null, "doorHost": "pay.agentrix.website"})
    );
    // Loopback is the money host, and x-saas-site picks another.
    assert!(
        get(&app, "127.0.0.1:18183", "/")
            .await
            .body
            .contains("Talk to us")
    );
    let r = call(
        &app,
        "GET",
        "127.0.0.1:18183",
        "/",
        &[("x-saas-site", "pay")],
        None,
    )
    .await;
    assert!(r.body.contains("The agent door"));
}
