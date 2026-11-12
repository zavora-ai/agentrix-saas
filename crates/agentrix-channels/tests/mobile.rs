//! The owner app's API: pair with a one-time code from Settings → Channels, read Now and a card,
//! approve with an authenticator code bound to the card's fingerprint (the effect runs once),
//! decline another, and a removed phone is shut out at once.

use std::sync::{Arc, Mutex};

use agentrix_channels::app::ChannelsApp;
use agentrix_channels::service::{Channels, Settings};
use agentrix_decisions::{
    ApprovalKind, CardState, DecisionCardV2, EffectRunner, NewCard, RaisedBy, Risk, RiskLevel,
};
use agentrix_host::testing::{FixedOwner, context};
use agentrix_shell::apps::{AppContext, AppPage, ConsoleApp};
use agentrix_types::{EffectRequest, SiteId};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const SITE: &str = "kilele-bakery-sample";

#[derive(Default)]
struct Ran(Mutex<Vec<String>>);

#[async_trait]
impl EffectRunner for Ran {
    fn tool(&self) -> &str {
        "webagent.add"
    }
    async fn run(&self, card: &DecisionCardV2, _receipt: &str) -> anyhow::Result<Option<String>> {
        self.0.lock().unwrap().push(card.id.clone());
        Ok(None)
    }
}

fn new_card(title: &str, key: &str) -> NewCard {
    NewCard {
        kind: "webagent.add".into(),
        title: title.into(),
        summary: "Sales follows up with customers who asked about cakes".into(),
        effects: vec!["The Sales WebAgent goes on duty at Watch".into()],
        effect: EffectRequest {
            site: SiteId::new(SITE),
            webagent: "chief".into(),
            tool: "webagent.add".into(),
            args: json!({ "webagent": key }),
            amount: None,
            expiry: Utc::now() + Duration::hours(8),
        },
        raised_by: RaisedBy {
            webagent: "chief".into(),
            job: None,
        },
        risk: Risk {
            level: RiskLevel::Medium,
            reasons: vec!["Adds a WebAgent".into()],
        },
        approval: ApprovalKind::Reviewed,
        back: None,
        dedupe_key: Some(key.into()),
    }
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn pair_read_approve_decline_and_remove() {
    let owner = Uuid::now_v7();
    let mut ctx = context().await.unwrap();
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    sqlx::raw_sql(
        "CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT NOT NULL, display_name TEXT NOT NULL);
         CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', utc_offset_minutes INTEGER NOT NULL DEFAULT 0, owner_account_id TEXT, created_at TEXT NOT NULL DEFAULT '');
         CREATE TABLE briefs (site_id TEXT NOT NULL, written_at TEXT NOT NULL, body TEXT NOT NULL);",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO accounts VALUES (?, 'wanjiru@example.com', 'Wanjiru Kamau')")
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sites (site_id, domain, name, utc_offset_minutes, owner_account_id) VALUES (?, 'kilele-bakery-sample.localhost', 'Kilele Bakery', 180, ?)")
        .bind(SITE)
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    agentrix_decisions::migrate(&ctx.pool).await.unwrap();
    agentrix_messages::migrate(&ctx.pool).await.unwrap();
    agentrix_shell::migrate(&ctx.pool).await.unwrap();
    agentrix_channels::migrate(&ctx.pool).await.unwrap();

    let channels = Channels::with_bots(
        &ctx,
        Vec::new(),
        agentrix_messages::from_context(&ctx),
        Settings::default(),
    );
    let ran = Arc::new(Ran::default());
    channels.set_runners(vec![ran.clone()]);
    let api = agentrix_channels::mobile::router(channels.clone());

    // No token, no data.
    let (s, body) = call(&api, "GET", "/api/m/v1/now", None, Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("Pair this phone")
    );

    // Settings → Channels → Pair a phone shows a code.
    let settings = ChannelsApp::new(channels.clone());
    let actx = AppContext {
        site_id: SITE.into(),
        site_name: "Kilele Bakery".into(),
        host: "kilele-bakery-sample.localhost".into(),
        sample: true,
        account: owner.to_string(),
        owner_name: "Wanjiru".into(),
        utc_offset_minutes: 180,
        base: format!("/console/{SITE}/apps/channels"),
        console: format!("/console/{SITE}"),
    };
    settings
        .post(&actx, "app/pair", &Default::default(), Default::default())
        .await
        .unwrap();
    let AppPage::Field { html, .. } = settings.page(&actx, "", "connect=app").await.unwrap() else {
        panic!("no page");
    };
    let marker = "aria-label=\"Pairing code\">";
    let at = html.find(marker).expect("a pairing code") + marker.len();
    let code: String = html[at..].chars().take(9).collect();
    assert_eq!(code.len(), 9, "{code}");

    // A wrong code fails; the right one, typed in lower case, pairs once.
    let (s, _) = call(
        &api,
        "POST",
        "/api/m/v1/pair",
        None,
        json!({ "code": "ZZZZ-ZZZZ", "device": "x" }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, paired) = call(
        &api,
        "POST",
        "/api/m/v1/pair",
        None,
        json!({ "code": code.to_lowercase(), "device": "Wanjiru's iPhone" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{paired}");
    let token = paired["token"].as_str().unwrap().to_owned();
    assert_eq!(paired["site"]["id"], json!(SITE));
    assert_eq!(paired["owner"]["name"], json!("Wanjiru"));
    let (s, _) = call(
        &api,
        "POST",
        "/api/m/v1/pair",
        None,
        json!({ "code": code, "device": "again" }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a code works once");

    // Now: the greeting, the doorbell and the cards waiting.
    let d = channels.decisions();
    let first = d
        .raise(new_card("Put Sales on duty", "sales"))
        .await
        .unwrap();
    let second = d
        .raise(new_card("Put Reputation on duty", "reputation"))
        .await
        .unwrap();
    let (s, now) = call(&api, "GET", "/api/m/v1/now", Some(&token), Value::Null).await;
    assert_eq!(s, StatusCode::OK, "{now}");
    assert!(now["greeting"].as_str().unwrap().ends_with(", Wanjiru"));
    assert_eq!(now["waiting"], json!(2));
    assert_eq!(now["doorbell"], json!("2 decisions are waiting."));
    assert!(
        !now["doorbell"].as_str().unwrap().contains("Sales"),
        "the doorbell never says what"
    );

    // A card: the exact effect, why this risk, and its fingerprint.
    let (s, card) = call(
        &api,
        "GET",
        &format!("/api/m/v1/cards/{}", first.id),
        Some(&token),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        card["effects"][0],
        json!("The Sales WebAgent goes on duty at Watch")
    );
    assert_eq!(card["reasons"][0], json!("Adds a WebAgent"));
    assert!(card["fingerprint"].as_str().unwrap().starts_with("sha256:"));

    // Approve needs six digits; with them, W5's contract approves and the effect runs once.
    let (s, _) = call(
        &api,
        "POST",
        &format!("/api/m/v1/cards/{}/approve", first.id),
        Some(&token),
        json!({ "code": "12" }),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, ok) = call(
        &api,
        "POST",
        &format!("/api/m/v1/cards/{}/approve", first.id),
        Some(&token),
        json!({ "code": "123 456" }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{ok}");
    assert!(ok["receipt"].as_str().is_some());
    assert_eq!(*ran.0.lock().unwrap(), vec![first.id.clone()]);
    let row = d.get(SITE, &first.id).await.unwrap().unwrap();
    assert_eq!(
        (row.state, row.method.as_deref()),
        (CardState::Approved, Some("totp"))
    );
    let (s, _) = call(
        &api,
        "POST",
        &format!("/api/m/v1/cards/{}/approve", first.id),
        Some(&token),
        json!({ "code": "123456" }),
    )
    .await;
    assert_ne!(s, StatusCode::OK, "approving twice is refused");

    // Decline the other.
    let (s, _) = call(
        &api,
        "POST",
        &format!("/api/m/v1/cards/{}/decline", second.id),
        Some(&token),
        Value::Null,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        d.get(SITE, &second.id).await.unwrap().unwrap().state,
        CardState::Declined
    );

    // Freeze says honestly that it isn't switched on here.
    let (s, f) = call(&api, "POST", "/api/m/v1/freeze", Some(&token), Value::Null).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(
        f["message"]
            .as_str()
            .unwrap()
            .contains("nothing was stopped")
    );

    // Removing the phone in Settings shuts it out at once.
    let device = paired["device"]["id"].as_str().unwrap();
    let form = format!("device={device}");
    settings
        .post(
            &actx,
            "app/remove",
            &Default::default(),
            form.into_bytes().into(),
        )
        .await
        .unwrap();
    let (s, _) = call(&api, "GET", "/api/m/v1/now", Some(&token), Value::Null).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
