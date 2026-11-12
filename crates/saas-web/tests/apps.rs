//! v3's apps: the owner's passkey moments, badges with Verify, the wallet, and the judges'
//! run, as each host serves them; and the HTTP clients against a stand-in for settle.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::{get, post};
use saas_web::apps::http::{HttpDay, HttpSettle};
use saas_web::apps::{self, AppError, Badges, Ceremonies, DayRunner, Moment, SiteApps};
use saas_web::hosts::Hosts;
use saas_web::passkeys::Passkeys;
use saas_web::{Access, Face};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    host: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, String, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host)
        .header(header::ACCEPT, "application/json");
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json");
    }
    let res = app
        .clone()
        .oneshot(
            req.body(
                body.map(|b| Body::from(b.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let csp = res
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, csp, String::from_utf8_lossy(&bytes).into_owned())
}

async fn body_has(app: &Router, host: &str, path: &str, want: &str) -> bool {
    call(app, "GET", host, path, None).await.2.contains(want)
}

const KILELE: &str = "kilele.money.localhost:18183";
const MONEY: &str = "money.localhost:18183";

fn demo() -> Router {
    Face::demo(Hosts::default())
        .with_board_dist(None)
        .with_grid_js(None)
        .router()
        .unwrap()
}

#[tokio::test]
async fn the_owner_is_asked_at_three_moments_on_their_own_host() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let app = demo();
    let (status, _, body) = call(&app, "GET", KILELE, "/console/kilele/passkey", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for want in [
        "Link Kilele Bakery&#x27;s PayPal to its Cashier",
        "Sign Wanjiru&#x27;s mandate for the Cashier",
        "Pay a new payee: Unga Millers",
        "Set up my passkey",
        "kilele.money.localhost",
    ] {
        assert!(body.contains(want), "passkey page lacks {want:?}");
    }
    assert!(
        !body.contains("Agentrix&#x27;s PayPal"),
        "only this site's moments"
    );

    let (status, _, body) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/passkey/link-paypal/bind-kilele",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Your passkey signs")
            && body.contains("sha256:")
            && body.contains("Demo subject"),
        "{body}"
    );
    assert!(body.contains("Set up your passkey first"));

    // No passkey yet: no challenge, and no seal.
    let (status, _, _) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/passkey/link-paypal/bind-kilele/challenge",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    let (status, _, _) = call(
        &app,
        "POST",
        KILELE,
        "/console/kilele/passkey/link-paypal/bind-kilele/sign",
        Some(json!({"id": "x"})),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED,
        "{status}"
    );
    // A moment that doesn't exist.
    let (status, _, _) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/passkey/pay-anyone/x",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The passkey's relying party is the console's own host.
    let (status, _, body) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/passkey/setup/options",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["publicKey"]["rp"]["id"],
        "kilele.money.localhost"
    );
    // A visitor on the public host can't start one.
    let (status, _, _) = call(
        &app,
        "GET",
        "kilele.money.agentrix.website",
        "/console/kilele/passkey/setup/options",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn badges_list_dids_and_verify_never_claims_what_it_did_not_check() {
    let app = demo();
    let (status, _, body) = call(&app, "GET", KILELE, "/console/kilele/badges", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("did:web:kilele.money.agentrix.website:agents:payables"),
        "{body}"
    );
    let (_, _, body) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/badges/verify?did=did:web:kilele.money.agentrix.website:agents:payables",
        None,
    )
    .await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["verified"], false);
    assert!(
        v["failure"]["message"]
            .as_str()
            .unwrap()
            .contains("No badge service")
    );
    let (status, _, _) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/badges/verify?did=not-a-did",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The public page on the site's host, for anyone.
    let (status, _, body) = call(
        &app,
        "GET",
        "kilele.money.agentrix.website",
        "/agents",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("Kilele Bakery's agents") && body.contains("/agents/check"),
        "{body}"
    );
    let (status, _, _) = call(
        &app,
        "GET",
        "kilele.money.agentrix.website",
        "/agents/check?did=did:web:x:agents:y",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The token check: nothing behind it on this node, and junk is refused before asking.
    assert!(
        body_has(
            &app,
            "kilele.money.agentrix.website",
            "/agents",
            "Check an agent"
        )
        .await
    );
    let (status, _, body) = call(
        &app,
        "POST",
        "kilele.money.agentrix.website",
        "/agents/check-token",
        Some(json!({"token": "eyJ.x.y"})),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let (status, _, _) = call(
        &app,
        "POST",
        KILELE,
        "/console/kilele/badges/verify-token",
        Some(json!({"token": "two words"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_wallet_reads_the_books_and_says_it_is_not_reconciled_here() {
    let app = Face::fakebooks(Hosts::default(), "2026-11-10".parse().unwrap())
        .await
        .unwrap()
        .with_board_dist(None)
        .with_grid_js(None)
        .router()
        .unwrap();
    let (status, csp, body) = call(&app, "GET", MONEY, "/console/agentrix/wallet", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        csp.contains("style-src 'self' 'unsafe-inline'"),
        "the grid themes itself"
    );
    for want in [
        "<agentrix-wallet",
        "Not reconciled against PayPal on this node",
        "DEMO-2PF93355",
        "Three-way matched",
    ] {
        assert!(body.contains(want), "wallet lacks {want:?}");
    }
    let (_, _, body) = call(
        &app,
        "GET",
        MONEY,
        "/console/agentrix/wallet/grid.json",
        None,
    )
    .await;
    let w: Value = serde_json::from_str(&body).unwrap();
    let e = w["entries"].as_array().unwrap();
    assert!(e.iter().any(|x| x["paypalId"] == "DEMO-2PF93355"
        && x["kind"] == "capture"
        && x["counterparty"] == "Kilele Bakery"));
    assert!(
        e.iter()
            .any(|x| x["kind"] == "payout" && x["counterparty"] == "Vultr")
    );
    assert!(
        e.iter()
            .filter(|x| x["kind"] == "payout")
            .all(|x| x["amountMinor"].as_i64().unwrap() < 0),
        "money out is negative"
    );
    assert!(
        w["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["state"] == "matched" && m["paypalStatus"].is_null()),
        "in the books, but nothing was read back from PayPal"
    );
    assert!(
        w["checkpoint"].is_null()
            && w["note"]
                .as_str()
                .unwrap()
                .contains("not read back from PayPal")
    );
    assert!(
        body.contains("\"source\":\"read back: the books on this node (demo)\""),
        "{body}"
    );
    // The page never counts a books-only row as three-way.
    let (_, _, page) = call(&app, "GET", MONEY, "/console/agentrix/wallet", None).await;
    assert!(page.contains(">0 of "), "three-way matched is 0 here");

    // The feed itself, as saas-core's contract has it.
    let (status, _, body) = call(
        &app,
        "GET",
        MONEY,
        "/console/agentrix/wallet/feed.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let feed: saas_core::wallet::WalletFeed = serde_json::from_str(&body).unwrap();
    assert_eq!(feed.wallet.to_string(), "agentrix/cashier");
    assert_eq!(feed.entries.len(), feed.matches.len());
    let (status, _, _) = call(&app, "GET", MONEY, "/static/saas/wallet-grid.js", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no grid built in this test");

    // Kilele's wallet holds the same PayPal id from the other side.
    let (_, _, body) = call(
        &app,
        "GET",
        KILELE,
        "/console/kilele/wallet/grid.json",
        None,
    )
    .await;
    let w: Value = serde_json::from_str(&body).unwrap();
    assert!(
        w["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["paypalId"] == "DEMO-2PF93355" && x["kind"] == "payout")
    );

    // Without books, the wallet says it isn't connected.
    let (status, _, body) = call(&demo(), "GET", MONEY, "/console/agentrix/wallet", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("isn&#x27;t connected on this node")
            || body.contains("isn't connected on this node")
    );
}

/// A wallet that answers with one fixed feed.
struct FixedWallet(saas_core::wallet::WalletFeed);

#[async_trait]
impl apps::Wallets for FixedWallet {
    async fn wallet(&self, _site: &str) -> Result<saas_core::wallet::WalletFeed, AppError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn the_wallet_draws_books_real_feed() {
    // Books' sample: Agentrix's Cashier after one sandbox capture, matched in ERA (the
    // counterparty's sandbox email replaced).
    let feed: saas_core::wallet::WalletFeed =
        serde_json::from_str(include_str!("data/wallet-feed-sample.json")).unwrap();
    let site = SiteApps {
        site: "agentrix".into(),
        name: "Agentrix".into(),
        owner: "James".into(),
        base: "/console/agentrix".into(),
        ceremonies: Arc::new(apps::demo::NotConnected),
        badges: Arc::new(apps::demo::DemoBadges::new(HashMap::new())),
        wallet: Arc::new(FixedWallet(feed)),
        paypal_link: "/console/agentrix/paypal-link".into(),
        passkeys: Arc::new(Passkeys::in_memory()),
        access: Access {
            owner_token: None,
            open: true,
        },
        fragment: false,
    };
    let app = Router::new().nest("/console/agentrix", apps::console_router(site));
    let (status, _, body) = call(
        &app,
        "GET",
        MONEY,
        "/console/agentrix/wallet/grid.json",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let w: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(w["wallet"], "agentrix/cashier");
    let m = w["matches"].as_array().unwrap();
    assert_eq!(
        (
            &m[0]["state"],
            &m[0]["paypalStatus"],
            &m[0]["amountMinor"],
            &m[0]["paypalAmountMinor"],
            &m[0]["booksAmountMinor"]
        ),
        (
            &json!("matched"),
            &json!("COMPLETED"),
            &json!(1900),
            &json!(1900),
            &json!(1900)
        )
    );
    // The fee is money out: negative on every side, and its journal is the fee journal.
    assert_eq!(
        (&m[1]["kind"], &m[1]["booksAmountMinor"]),
        (&json!("fee"), &json!(-104))
    );
    assert_eq!(m[1]["journal"], m[1]["feeJournal"]);
    // The books' own numbers, for the grid to show instead of ERA's UUIDs.
    assert_eq!(
        (&m[0]["paymentNumber"], &m[0]["journalNumber"]),
        (&json!("PAY-2026-0009"), &json!("JE-2026-0029"))
    );
    assert_eq!(
        (&m[1]["paymentNumber"], &m[1]["journalNumber"]),
        (&Value::Null, &json!("JE-2026-0030"))
    );
    let e = &w["entries"][0];
    assert_eq!(
        e["source"],
        "read back: GET /v2/payments/captures/3KT78856M2504654G"
    );
    assert!(
        e["authority"]
            .as_str()
            .unwrap()
            .starts_with("approved by operator")
    );
    assert_eq!(e["cause"], "agentrix-saas/wallet/1a1c6967e45");
    // Checkpoints, newest first.
    let h = w["history"].as_array().unwrap();
    assert_eq!(
        h.iter()
            .map(|c| c["size"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [6, 5, 4]
    );
    assert_eq!(w["checkpoint"]["words"], "Matches PayPal as of 20:57 EAT");

    let (_, csp, page) = call(&app, "GET", MONEY, "/console/agentrix/wallet", None).await;
    assert!(csp.contains("'unsafe-inline'"));
    for want in [
        "Matches PayPal as of 20:57 EAT",
        ">2 of 2<",
        "Wallet checkpoints",
        "c3a23037b45bb28c442c4cc…",
        "-$1.04",
    ] {
        assert!(page.contains(want), "wallet page lacks {want:?}");
    }
    assert!(!page.contains("Not reconciled against PayPal"));
}

#[tokio::test]
async fn no_page_says_passport() {
    // v3 calls the identity document the badge, on every page, button and label.
    let app = demo();
    for (host, path) in [
        (MONEY, "/"),
        (MONEY, "/judges"),
        (MONEY, "/console"),
        (MONEY, "/console/agentrix/badges"),
        (MONEY, "/console/agentrix/passkey"),
        (MONEY, "/console/agentrix/wallet"),
        (KILELE, "/console"),
        (KILELE, "/console/kilele/badges"),
        (KILELE, "/console/kilele/passkey/link-paypal?binding=bind_1"),
        ("kilele.money.agentrix.website", "/agents"),
        (MONEY, "/static/saas/paypal-link.js"),
        (MONEY, "/static/saas/verify.js"),
        (MONEY, "/static/saas/judge.js"),
        (MONEY, "/static/saas/console.js"),
        (MONEY, "/static/saas/saas.css"),
    ] {
        let (status, _, body) = call(&app, "GET", host, path, None).await;
        assert_eq!(status, StatusCode::OK, "{host}{path}");
        assert!(
            !body.to_lowercase().contains("passport"),
            "{host}{path} says passport"
        );
    }
    let (_, _, body) = call(&app, "GET", KILELE, "/console/kilele/badges", None).await;
    assert!(body.contains("Holds no secrets; PayPal access is issued per call"));
}

#[tokio::test]
async fn the_owner_revokes_one_agent_and_only_a_passkey_brings_it_back() {
    let app = demo();
    let (status, _, body) = call(&app, "GET", KILELE, "/console", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("action=\"/console/agents/payables/revoke\"")
            && body.contains("no PayPal key is rotated"),
        "{body}"
    );
    assert!(body.contains("holds no secrets; PayPal access is issued per call"));

    // Off at once, no passkey.
    let (status, _, body) = call(
        &app,
        "POST",
        KILELE,
        "/console/agents/payables/revoke",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let r: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (&r["agent"], &r["revoked"]),
        (&json!("payables"), &json!(true))
    );
    assert_eq!(r["record"]["agent"], "payables");
    let (_, _, body) = call(&app, "GET", KILELE, "/console", None).await;
    assert!(
        body.contains(">Revoked<")
            && body.contains("data-restore=\"/console/agents/payables/restore\""),
        "{body}"
    );
    assert!(
        body.contains("Revoked by the owner"),
        "the feed says so too"
    );

    // Back on only with the passkey: none is set up here, so no challenge and no restore.
    let (status, _, body) = call(
        &app,
        "GET",
        KILELE,
        "/console/agents/payables/restore/challenge",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    let (status, _, _) = call(
        &app,
        "POST",
        KILELE,
        "/console/agents/payables/restore",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (_, _, body) = call(&app, "GET", KILELE, "/console", None).await;
    assert!(body.contains(">Revoked<"), "still revoked");

    // Only the owner, only real agents.
    let (status, _, _) = call(
        &app,
        "POST",
        "kilele.money.agentrix.website",
        "/console/agents/chief/revoke",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = call(&app, "POST", KILELE, "/console/agents/ghost/revoke", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = call(&app, "POST", KILELE, "/console/agents/..%2Fx/revoke", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pages_name_the_configured_hosts_and_checkout_says_sandbox() {
    // The judged hosts aren't agentrix.website's own: no page may name those.
    let hosts = Hosts {
        money: "paypal.agentrix.website".into(),
        kilele: "kilele-sample.paypal.agentrix.website".into(),
        pay: "pay.paypal.agentrix.website".into(),
        books: "books.paypal.agentrix.website".into(),
    };
    let app = Face::demo(hosts)
        .with_board_dist(None)
        .with_grid_js(None)
        .router()
        .unwrap();
    let money = "paypal.agentrix.website";
    for (host, path) in [
        (money, "/"),
        (money, "/plans"),
        (money, "/subscribe?plan=business"),
        (money, "/judges"),
        (money, "/console"),
        (money, "/console/agentrix/badges"),
        ("kilele-sample.paypal.agentrix.website", "/agents"),
        ("pay.paypal.agentrix.website", "/"),
        ("books.paypal.agentrix.website", "/"),
    ] {
        let (status, _, body) = call(&app, "GET", host, path, None).await;
        // The books page answers 503 with no ERA connected; its words still count.
        assert!(
            status.is_success() || host.starts_with("books."),
            "{host}{path}: {status}"
        );
        for old in [
            "money.agentrix.website",
            "pay.agentrix.website/",
            "books.agentrix.website",
        ] {
            assert!(!body.contains(old), "{host}{path} names {old}");
        }
    }
    let (_, _, body) = call(&app, "GET", money, "/", None).await;
    assert!(
        body.contains("Agentrix · paypal.agentrix.website"),
        "the front names its configured host"
    );
    let (_, _, body) = call(&app, "GET", money, "/judges", None).await;
    assert!(
        body.contains(r#"id="verify""#),
        "the home page links to #verify"
    );
    for path in ["/plans", "/subscribe?plan=business"] {
        let (_, _, body) = call(&app, "GET", money, path, None).await;
        assert!(
            body.contains("Sandbox: no real money moves"),
            "{path} says Sandbox"
        );
    }
    let (_, _, body) = call(
        &app,
        "POST",
        money,
        "/desk/sales",
        Some(json!({"q": "how do I subscribe and pay?"})),
    )
    .await;
    assert!(
        body.contains("https://pay.paypal.agentrix.website/"),
        "{body}"
    );
}

struct FakeDay;

#[async_trait]
impl DayRunner for FakeDay {
    async fn start(&self) -> Result<String, AppError> {
        Ok("run-1".into())
    }
    async fn status(&self, run: &str) -> Result<Value, AppError> {
        Ok(
            json!({"done": true, "scenes": [{"n": 1, "title": "A person buys a cake", "state": "done", "evidence": [{"label": "PayPal capture", "value": run}]}]}),
        )
    }
}

#[tokio::test]
async fn judges_run_the_day_only_in_judge_mode() {
    let app = demo();
    let (_, _, body) = call(&app, "GET", MONEY, "/judges", None).await;
    assert!(body.contains("Judge mode is off on this node") && body.contains("Verify any agent"));
    let (status, _, _) = call(&app, "POST", MONEY, "/judges/run-day", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let app = Face::demo(Hosts::default())
        .with_board_dist(None)
        .with_day_runner(Some(Arc::new(FakeDay)))
        .router()
        .unwrap();
    let (_, _, body) = call(&app, "GET", MONEY, "/judges", None).await;
    assert!(body.contains("data-run-day-start"));
    let (status, _, body) = call(&app, "POST", MONEY, "/judges/run-day", None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["run"],
        "run-1"
    );
    let (_, _, body) = call(&app, "GET", MONEY, "/judges/run-day/run-1", None).await;
    assert!(body.contains("A person buys a cake"));
    let (status, _, _) = call(&app, "GET", MONEY, "/judges/run-day/..%2Fx", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = call(
        &app,
        "GET",
        MONEY,
        "/judges/verify?did=did:web:money.agentrix.website:agents:payables",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_console_apps_nest_anywhere() {
    // As Agentrix OS would mount a sample site's apps in its own router.
    let ceremonies = Arc::new(apps::demo::DemoCeremonies::story(&[(
        "tana",
        "Tana Valley Times",
        "Amina",
    )]));
    let badges = Arc::new(apps::demo::DemoBadges::new(HashMap::from([(
        "tana".to_owned(),
        apps::demo::DemoSite {
            host: "tana-sample.agentrix.website".to_owned(),
            name: "Tana Valley Times".into(),
            owner: "Amina".into(),
            agents: vec![("cashier".to_owned(), "Cashier".to_owned())],
        },
    )])));
    let site = SiteApps {
        site: "tana".into(),
        name: "Tana Valley Times".into(),
        owner: "Amina".into(),
        base: "/console/tana".into(),
        ceremonies,
        badges,
        wallet: Arc::new(apps::demo::NotConnected),
        paypal_link: "/console/tana/paypal-link".into(),
        passkeys: Arc::new(Passkeys::in_memory()),
        access: Access {
            owner_token: None,
            open: true,
        },
        fragment: false,
    };
    let os = Router::new()
        .nest("/console/tana", apps::console_router(site.clone()))
        .merge(apps::public_router(site));
    let (status, _, body) = call(
        &os,
        "GET",
        "tana-sample.agentrix.website",
        "/console/tana/passkey",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("/console/tana/passkey/link-paypal/bind-tana"));
    let (_, _, body) = call(&os, "GET", "tana-sample.agentrix.website", "/agents", None).await;
    assert!(body.contains("did:web:tana-sample.agentrix.website:agents:cashier"));
    assert!(body.contains("urn:adk:agent:tana/cashier"), "{body}");

    // Link PayPal: PayPal sends the owner back with the binding's id; the page reads the draft
    // from saas-badge's desk and never takes a PayPal id typed in.
    let (status, _, body) = call(
        &os,
        "GET",
        "tana-sample.agentrix.website",
        "/console/tana/passkey/link-paypal?binding=bind_7Q",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("data-api=\"/console/tana/paypal-link\"")
            && body.contains("data-binding=\"bind_7Q\""),
        "{body}"
    );
    let (_, _, body) = call(
        &os,
        "GET",
        "tana-sample.agentrix.website",
        "/console/tana/passkey/link-paypal?binding=%3Cscript%3E",
        None,
    )
    .await;
    assert!(
        !body.contains("data-binding") && body.contains("/console/tana/paypal-link/start"),
        "{body}"
    );
}

#[tokio::test]
async fn the_http_clients_speak_the_contract() {
    // A stand-in for settle and the story runner, on a real port.
    let settle = Router::new()
        .route(
            "/owner/{site}/ceremonies/{moment}/{id}",
            get(|axum::extract::Path((site, moment, id)): axum::extract::Path<(String, String, String)>| async move {
                axum::Json(json!({"moment": moment, "id": id, "site": site, "state": "pending", "title": "Link PayPal", "lines": ["PayPal merchant id Q7"], "fingerprint": "sha256:ab", "expiresAt": "2026-11-11T12:00:00Z"}))
            })
            .post(|body: axum::Json<Value>| async move {
                if body["evidence"]["fingerprint"] == "sha256:ab" {
                    (StatusCode::OK, axum::Json(json!({"sealed": true, "record": {"bound": true}, "auditSeq": 412})))
                } else {
                    (StatusCode::CONFLICT, axum::Json(json!({"message": "changed"})))
                }
            }),
        )
        .route("/owner/{site}/ceremonies", get(|| async { axum::Json(json!([])) }))
        .route("/agents/verify", get(|q: axum::extract::Query<HashMap<String, String>>| async move {
            let did = q.get("did").cloned().unwrap_or_default();
            // saas-badge's verify route (its `names`); for one DID it claims verified beside
            // a failed check, which the face must never show as verified.
            let key_ok = !did.contains("payables");
            axum::Json(json!({"did": did, "verified": true, "checks": [
                {"check": "did.json resolves from the site's own domain", "ok": true, "detail": "200"},
                {"check": "key is the agent's in its DID document", "ok": key_ok, "detail": if key_ok { "" } else { "kid not in did.json" }}
            ], "failure": null, "badge": {}, "checked_at": "2026-11-10T09:00:00Z"}))
        }))
        .route("/agents/verify-token", post(|body: axum::Json<Value>| async move {
            // saas-badge's TokenReport.
            if body["token"] == "eyJ.good.sig" {
                axum::Json(json!({"ok": true, "principal": {"principal": {"id": "did:web:kilele-sample.agentrix.website:agents:cashier"}, "issuer": "https://kilele-sample.agentrix.website", "profile": "agentrix", "subject": "did:web:kilele-sample.agentrix.website:agents:cashier"}, "vouched_by": "agentrix (https://kilele-sample.agentrix.website)", "refused": null, "badge": null, "checked_at": "2026-11-10T09:00:00Z"}))
            } else {
                axum::Json(json!({"ok": false, "principal": null, "vouched_by": null, "refused": "the token was refused: bad signature", "badge": null, "checked_at": "2026-11-10T09:00:00Z"}))
            }
        }))
        .route("/agents/index.json", get(|| async {
            axum::Json(json!([
                {"role": "cashier", "name": "Cashier", "did": "did:web:kilele-sample.agentrix.website:agents:cashier", "badge": "/agents/cashier/badge.json"},
                {"role": "payables", "name": "Payables", "did": "did:web:kilele-sample.agentrix.website:agents:payables", "badge": "/agents/payables/badge.json"}
            ]))
        }))
        .route("/agents/{role}/badge.json", get(|axum::extract::Path(role): axum::extract::Path<String>| async move {
            let p = apps::demo::DemoBadges::new(HashMap::from([("kilele".to_owned(), apps::demo::DemoSite {
                host: "kilele-sample.agentrix.website".into(), name: "Kilele Bakery".into(), owner: "Wanjiru".into(), agents: vec![(role.clone(), role)],
            })]));
            let mut v = serde_json::to_value(&p.badges("kilele").await.unwrap()[0]).unwrap();
            v["proof"] = json!({"type": "JsonWebSignature2020", "jws": "eyJhbGciOiJFZERTQSJ9..sig"});
            axum::Json(v)
        }))
        .route("/judge/run-day", post(|| async { (StatusCode::ACCEPTED, axum::Json(json!({"run": "r9"}))) }))
        .route("/judge/run-day/{run}", get(|axum::extract::Path(run): axum::extract::Path<String>| async move { axum::Json(json!({"done": false, "run": run, "scenes": []})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, settle).await.unwrap() });

    let http = HttpSettle::new(&base, HashMap::from([("kilele".to_owned(), base.clone())]));
    let s = http
        .subject("kilele", Moment::LinkPaypal, "bind_1")
        .await
        .unwrap();
    assert_eq!(
        (s.moment, s.fingerprint.as_str(), s.expires_at.as_deref()),
        (
            Moment::LinkPaypal,
            "sha256:ab",
            Some("2026-11-11T12:00:00Z")
        )
    );
    let sealed = http
        .seal(
            "kilele",
            Moment::LinkPaypal,
            "bind_1",
            json!({"fingerprint": "sha256:ab"}),
        )
        .await
        .unwrap();
    assert_eq!((sealed.sealed, sealed.audit_seq), (true, Some(412)));
    assert_eq!(
        http.seal(
            "kilele",
            Moment::LinkPaypal,
            "bind_1",
            json!({"fingerprint": "sha256:cd"})
        )
        .await,
        Err(AppError::Changed)
    );
    assert!(http.pending("kilele").await.unwrap().is_empty());
    let v = http.verify("did:web:kilele:agents:cashier").await.unwrap();
    assert!(v.verified && v.failure.is_none());
    assert_eq!(
        v.checks[0].check,
        "did.json resolves from the site's own domain"
    );
    assert_eq!(v.checked_at.as_deref(), Some("2026-11-10T09:00:00Z"));
    // `ok: true` beside a failed check is never shown as verified.
    let v = http.verify("did:web:kilele:agents:payables").await.unwrap();
    assert!(!v.verified);
    assert_eq!(v.failure.unwrap().message, "kid not in did.json");
    let badges = http.badges("kilele").await.unwrap();
    assert_eq!(
        badges.iter().map(|p| p.role.as_str()).collect::<Vec<_>>(),
        ["cashier", "payables"]
    );
    assert_eq!(
        badges[0].id,
        "did:web:kilele-sample.agentrix.website:agents:cashier"
    );
    assert_eq!(http.badges("nowhere").await, Err(AppError::NotFound));
    let t = http.verify_token("eyJ.good.sig").await.unwrap();
    assert_eq!(
        (t["ok"].as_bool(), t["vouched_by"].as_str()),
        (
            Some(true),
            Some("agentrix (https://kilele-sample.agentrix.website)")
        )
    );
    assert_eq!(http.verify_token("eyJ.bad").await.unwrap()["ok"], false);

    let day = HttpDay::new(&base);
    let run = day.start().await.unwrap();
    assert_eq!(day.status(&run).await.unwrap()["run"], "r9");
}
