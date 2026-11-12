//! The face inside Agentrix OS: the three console apps as the shell calls them (contract C9).

use std::collections::HashMap;
use std::sync::Arc;

use agentrix_shell::apps::{AppContext, AppPage, AppReply, ConsoleApp};
use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use saas_web::Access;
use saas_web::apps::{self, AppError, SiteApps};
use saas_web::passkeys::Passkeys;
use serde_json::json;

struct FixedWallet(saas_core::wallet::WalletFeed);

#[async_trait]
impl apps::Wallets for FixedWallet {
    async fn wallet(&self, _site: &str) -> Result<saas_core::wallet::WalletFeed, AppError> {
        Ok(self.0.clone())
    }
}

fn kilele() -> SiteApps {
    SiteApps {
        site: "kilele".into(),
        name: "Kilele Bakery".into(),
        owner: "Wanjiru".into(),
        base: "/ignored".into(),
        ceremonies: Arc::new(apps::demo::DemoCeremonies::story(&[(
            "kilele",
            "Kilele Bakery",
            "Wanjiru",
        )])),
        badges: Arc::new(apps::demo::DemoBadges::new(HashMap::from([(
            "kilele".to_owned(),
            apps::demo::DemoSite {
                host: "kilele-sample.agentrix.website".into(),
                name: "Kilele Bakery".into(),
                owner: "Wanjiru".into(),
                agents: vec![("cashier".into(), "Cashier".into())],
            },
        )]))),
        wallet: Arc::new(FixedWallet(
            serde_json::from_str(include_str!("data/wallet-feed-sample.json")).unwrap(),
        )),
        paypal_link: "/console/site_kilele/paypal-link".into(),
        passkeys: Arc::new(Passkeys::in_memory()),
        // The shell checks the owner; the apps open whatever this says.
        access: Access {
            owner_token: Some("x".repeat(32)),
            open: false,
        },
        fragment: false,
    }
}

fn ctx(app: &str) -> AppContext {
    AppContext {
        site_id: "site_kilele".into(),
        site_name: "Kilele Bakery".into(),
        host: "kilele-sample.agentrix.website".into(),
        sample: true,
        account: "acct_1".into(),
        owner_name: "Wanjiru".into(),
        utc_offset_minutes: 180,
        base: format!("/console/site_kilele/apps/{app}"),
        console: "/console/site_kilele".into(),
    }
}

fn the_apps() -> HashMap<String, Arc<dyn ConsoleApp>> {
    saas_web::os::console_apps(
        HashMap::from([("site_kilele".to_owned(), kilele())]),
        "agentrix.localhost:18183",
    )
    .into_iter()
    .map(|a| (a.spec().id, a))
    .collect()
}

fn field(page: AppPage) -> (String, String) {
    match page {
        AppPage::Field { title, html } => (title, html),
        other => panic!("not a field: {other:?}"),
    }
}

fn json_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        header::HOST,
        HeaderValue::from_static("agentrix.localhost:18183"),
    );
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    h
}

#[tokio::test]
async fn three_apps_in_the_dock_on_their_sites() {
    let apps = the_apps();
    let mut ids: Vec<_> = apps.keys().cloned().collect();
    ids.sort();
    assert_eq!(ids, ["badges", "passkey", "wallet"]);
    assert_eq!(apps["wallet"].spec().webagent.as_deref(), Some("Cashier"));
    assert!(apps["badges"].available(&ctx("badges")).await);
    let mut elsewhere = ctx("badges");
    elsewhere.site_id = "site_other".into();
    assert!(!apps["badges"].available(&elsewhere).await);
    // The three moments wait for Wanjiru.
    assert_eq!(apps["passkey"].badge(&ctx("passkey")).await.unwrap(), 3);
    assert_eq!(apps["badges"].badge(&ctx("badges")).await.unwrap(), 0);
}

#[tokio::test]
async fn pages_are_fields_that_link_inside_the_shell() {
    let apps = the_apps();
    let (title, html) = field(apps["badges"].page(&ctx("badges"), "", "").await.unwrap());
    assert_eq!(title, "Badges · Kilele Bakery");
    assert!(
        !html.contains("<html") && !html.contains("<!doctype"),
        "a field, not a page"
    );
    for want in [
        r#"action="/console/site_kilele/apps/badges/verify""#,
        "did:web:kilele-sample.agentrix.website:agents:cashier",
        "Holds no secrets; PayPal access is issued per call",
        "/static/saas/saas.css",
        "/static/saas/verify.js",
    ] {
        assert!(html.contains(want), "badges lacks {want:?}");
    }
    assert!(!html.to_lowercase().contains("passport"));

    let (_, html) = field(apps["passkey"].page(&ctx("passkey"), "", "").await.unwrap());
    assert!(
        html.contains("/console/site_kilele/apps/passkey/link-paypal/bind-kilele"),
        "{html}"
    );
    assert!(html.contains(r#"href="/console/site_kilele/apps/passkey/link-paypal""#));
    let (_, html) = field(
        apps["passkey"]
            .page(&ctx("passkey"), "link-paypal", "binding=bind_7Q")
            .await
            .unwrap(),
    );
    assert!(
        html.contains(r#"data-api="/console/site_kilele/paypal-link""#)
            && html.contains(r#"data-binding="bind_7Q""#)
    );

    let (_, html) = field(apps["wallet"].page(&ctx("wallet"), "", "").await.unwrap());
    assert!(
        html.contains(r#"<agentrix-wallet src="/console/site_kilele/apps/wallet/grid.json""#),
        "{html}"
    );
    assert!(
        html.contains("/static/saas/wallet-grid.js")
            && html.contains("Matches PayPal as of 20:57 EAT")
    );

    assert!(matches!(
        apps["badges"]
            .page(&ctx("badges"), "nope", "")
            .await
            .unwrap(),
        AppPage::NotFound
    ));
}

#[tokio::test]
async fn json_reads_and_actions_go_through_post() {
    let apps = the_apps();
    let did = "did:web:kilele-sample.agentrix.website:agents:cashier";
    let reply = apps["badges"]
        .post(
            &ctx("badges"),
            "verify",
            &json_headers(),
            Bytes::from(json!({"did": did}).to_string()),
        )
        .await
        .unwrap();
    let AppReply::Json(StatusCode::OK, v) = reply else {
        panic!("verify: {reply:?}");
    };
    assert_eq!(
        (v["did"].as_str(), v["verified"].as_bool()),
        (Some(did), Some(false))
    );

    let reply = apps["wallet"]
        .post(&ctx("wallet"), "grid.json", &json_headers(), Bytes::new())
        .await
        .unwrap();
    let AppReply::Json(StatusCode::OK, v) = reply else {
        panic!("grid: {reply:?}");
    };
    assert_eq!(v["matches"][0]["paypalStatus"], "COMPLETED");

    // The passkey's relying party is the console's host; no passkey yet, so no challenge.
    let reply = apps["passkey"]
        .post(
            &ctx("passkey"),
            "link-paypal/bind-kilele/challenge",
            &json_headers(),
            Bytes::new(),
        )
        .await
        .unwrap();
    assert!(
        matches!(reply, AppReply::Json(StatusCode::PRECONDITION_REQUIRED, _)),
        "{reply:?}"
    );
    let reply = apps["passkey"]
        .post(
            &ctx("passkey"),
            "setup/options",
            &json_headers(),
            Bytes::new(),
        )
        .await
        .unwrap();
    let AppReply::Json(StatusCode::OK, v) = reply else {
        panic!("setup: {reply:?}");
    };
    assert_eq!(v["publicKey"]["rp"]["id"], "agentrix.localhost");

    assert!(matches!(
        apps["wallet"]
            .post(&ctx("wallet"), "nope", &json_headers(), Bytes::new())
            .await
            .unwrap(),
        AppReply::NotFound
    ));
}
