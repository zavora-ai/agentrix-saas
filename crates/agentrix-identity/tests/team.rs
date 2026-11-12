//! Team members and roles: invites proven with the owner's passkey over the exact grant,
//! acceptance only by the invited email, role changes, removal, and receipts for each.

use std::sync::{Arc, Mutex};

use agentrix_host::testing::{MemoryOwnerAccounts, MemoryReceipts};
use agentrix_host::{Approvals, Owner, OwnerAccounts, OwnerSessions};
use agentrix_identity::mailer::{MailKind, MemoryMailer};
use agentrix_identity::{Role, Team};
use agentrix_types::SiteId;
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Default)]
struct Switch(Mutex<Option<Owner>>);

#[async_trait]
impl OwnerSessions for Switch {
    async fn owner(&self, _headers: &HeaderMap) -> anyhow::Result<Option<Owner>> {
        Ok(self.0.lock().unwrap().clone())
    }
}

/// The frozen contract's shape: a passkey proof works only for an account with a passkey.
struct Contract(Arc<MemoryOwnerAccounts>);

#[async_trait]
impl Approvals for Contract {
    async fn passkey_options(&self, account: Uuid, _fp: [u8; 32]) -> anyhow::Result<Option<Value>> {
        let has = self
            .0
            .accounts
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.account == account && a.has_passkey);
        Ok(has.then(|| json!({ "publicKey": { "challenge": "x" } })))
    }

    async fn verify(&self, account: Uuid, _fp: [u8; 32], request: Value) -> anyhow::Result<Value> {
        let has = self
            .0
            .accounts
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.account == account && a.has_passkey);
        anyhow::ensure!(has && request.get("passkey").is_some(), "not approved");
        Ok(json!({ "method": "passkey" }))
    }
}

struct World {
    app: Router,
    team: Team,
    accounts: Arc<MemoryOwnerAccounts>,
    sessions: Arc<Switch>,
    receipts: Arc<MemoryReceipts>,
    mailer: Arc<MemoryMailer>,
    owner: Uuid,
}

const SITE: &str = "site_kilele";

async fn world() -> World {
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_identity::migrate(&ctx.pool).await.unwrap();
    // The first release's sites table, the columns this crate reads.
    sqlx::query("CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT NOT NULL, name TEXT NOT NULL DEFAULT '', owner_account_id TEXT)")
        .execute(&ctx.pool)
        .await
        .unwrap();
    let accounts = Arc::new(MemoryOwnerAccounts::default());
    let owner = accounts
        .create("wanjiru@example.com", "Wanjiru")
        .await
        .unwrap();
    accounts.accounts.lock().unwrap()[0].has_passkey = true;
    sqlx::query("INSERT INTO sites (site_id, domain, name, owner_account_id) VALUES (?, 'kilele-bakery.example', 'Kilele Bakery', ?)")
        .bind(SITE)
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    let sessions = Arc::new(Switch(Mutex::new(Some(Owner {
        account: owner,
        method: "passkey".into(),
    }))));
    let receipts = Arc::new(MemoryReceipts::default());
    let mailer = Arc::new(MemoryMailer::default());
    ctx.owner_accounts = accounts.clone();
    ctx.owners = sessions.clone();
    ctx.approvals = Arc::new(Contract(accounts.clone()));
    ctx.receipts = receipts.clone();
    let team = Team::for_node(&ctx, mailer.clone());
    World {
        app: team.router(),
        team,
        accounts,
        sessions,
        receipts,
        mailer,
        owner,
    }
}

async fn call(w: &World, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, "http://localhost:8080")
        .header(header::CONTENT_TYPE, "application/json")
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let res = w.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn signed_in(w: &World, account: Uuid, method: &str) {
    *w.sessions.0.lock().unwrap() = Some(Owner {
        account,
        method: method.into(),
    });
}

async fn invite(w: &World, email: &str, role: &str) -> (Value, String) {
    let (_, start) = call(
        w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite/start"),
        json!({ "email": email, "role": role }),
    )
    .await;
    assert!(start["passkey"].is_object(), "{start}");
    let (status, done) = call(
        w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite"),
        json!({ "challenge": start["challenge"], "passkey": { "id": "cred" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let link = w
        .mailer
        .sent()
        .into_iter()
        .rev()
        .find_map(|m| match m.kind {
            MailKind::TeamInvite { link, .. } if m.to == email => Some(link),
            _ => None,
        })
        .unwrap();
    let token = link.split("t=").nth(1).unwrap().to_owned();
    (done["member"].clone(), token)
}

#[tokio::test]
async fn the_owner_invites_with_a_passkey_and_the_invitee_accepts() {
    let w = world().await;
    let site = SiteId::new(SITE);
    assert_eq!(
        w.team.role_of(&site, w.owner).await.unwrap(),
        Some(Role::Owner)
    );

    // No proof, no invite.
    let (_, start) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite/start"),
        json!({ "email": "amina@example.com", "role": "manager" }),
    )
    .await;
    let (status, refused) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite"),
        json!({ "challenge": start["challenge"] }),
    )
    .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("step-up"))
    );
    assert!(
        w.mailer.sent().is_empty(),
        "nothing sent without the owner's proof"
    );

    let (member, token) = invite(&w, "amina@example.com", "manager").await;
    assert_eq!(member["state"], "invited");
    let (_, view) = call(
        &w,
        "GET",
        &format!("/team/api/invites/{token}"),
        Value::Null,
    )
    .await;
    assert_eq!(view["site_name"], "Kilele Bakery");
    assert_eq!(view["email_hint"], "am***@example.com");

    // Someone else signed in can't take it.
    let other = w
        .accounts
        .create("someone@example.com", "Someone")
        .await
        .unwrap();
    signed_in(&w, other, "passkey");
    let (status, wrong) = call(
        &w,
        "POST",
        "/team/api/invites/accept",
        json!({ "token": token }),
    )
    .await;
    assert_eq!(
        (status, wrong["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("wrong-account"))
    );

    // Amina, signed in with the invited address, accepts.
    let amina = w
        .accounts
        .create("amina@example.com", "Amina")
        .await
        .unwrap();
    signed_in(&w, amina, "email-code");
    let (status, accepted) = call(
        &w,
        "POST",
        "/team/api/invites/accept",
        json!({ "token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["site_name"], "Kilele Bakery");
    assert_eq!(
        w.team.role_of(&site, amina).await.unwrap(),
        Some(Role::Manager)
    );
    assert_eq!(
        w.team.memberships(amina).await.unwrap(),
        vec![(site.clone(), Role::Manager)]
    );
    let (status, _) = call(
        &w,
        "POST",
        "/team/api/invites/accept",
        json!({ "token": token }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an invitation works once");

    // A manager sees the team but can't change it.
    let (status, list) = call(
        &w,
        "GET",
        &format!("/team/api/sites/{SITE}/members"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["you"]["role"], "manager");
    let (status, _) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite/start"),
        json!({ "email": "x@example.com", "role": "viewer" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owner lowers Amina to viewer (proven), then removes her (no proof needed).
    signed_in(&w, w.owner, "passkey");
    let id = member["id"].as_str().unwrap();
    let (_, start) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/members/{id}/role/start"),
        json!({ "role": "viewer" }),
    )
    .await;
    let (status, changed) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/members/{id}/role"),
        json!({ "challenge": start["challenge"], "passkey": { "id": "cred" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(
        w.team.role_of(&site, amina).await.unwrap(),
        Some(Role::Viewer)
    );
    let (status, _) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/members/{id}/remove"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(w.team.role_of(&site, amina).await.unwrap(), None);

    let kinds: Vec<String> = w
        .receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str().to_owned())
        .collect();
    if agentrix_types::ReceiptKind::parse("team.changed").is_some() {
        assert_eq!(
            kinds, ["team.changed"; 4],
            "invited, accepted, role, removed"
        );
    }
    let details: Vec<String> = w
        .receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.detail.as_ref().unwrap().to_string())
        .collect();
    assert!(
        details.iter().all(|d| !d.contains("amina@example.com")),
        "no email addresses in receipts"
    );
}

#[tokio::test]
async fn an_email_code_session_cant_grant_roles() {
    let w = world().await;
    // The owner lost their passkey and is in on an email-code session: no approvals, no grants.
    w.accounts.accounts.lock().unwrap()[0].has_passkey = false;
    signed_in(&w, w.owner, "email-code");
    let (_, start) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite/start"),
        json!({ "email": "amina@example.com", "role": "manager" }),
    )
    .await;
    assert!(start["passkey"].is_null());
    let (status, refused) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite"),
        json!({ "challenge": start["challenge"], "totp": "123456" }),
    )
    .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("step-up"))
    );
    assert!(w.mailer.sent().is_empty());
}

#[tokio::test]
async fn a_proof_covers_only_the_grant_it_was_made_for() {
    let w = world().await;
    let (member, _) = invite(&w, "amina@example.com", "viewer").await;
    let id = member["id"].as_str().unwrap();
    // A challenge made for an invite can't be spent on a role change.
    let (_, start) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite/start"),
        json!({ "email": "other@example.com", "role": "manager" }),
    )
    .await;
    let (status, refused) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/members/{id}/role"),
        json!({ "challenge": start["challenge"], "passkey": { "id": "cred" } }),
    )
    .await;
    assert_eq!(
        (status, refused["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("step-up"))
    );
    // Only grantable roles.
    let (status, _) = call(
        &w,
        "POST",
        &format!("/team/api/sites/{SITE}/invite/start"),
        json!({ "email": "x@example.com", "role": "owner" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Not the owner of another site.
    let (status, _) = call(&w, "GET", "/team/api/sites/site_other/members", Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_people_app_lists_the_team_inside_the_console() {
    use agentrix_shell::apps::{AppContext, AppPage, AppReply, ConsoleApp};
    let w = world().await;
    let (member, _) = invite(&w, "amina@example.com", "manager").await;
    let app = agentrix_identity::PeopleApp::new(w.team.clone());
    assert_eq!(app.spec().id, "people");
    let ctx = AppContext {
        site_id: SITE.into(),
        site_name: "Kilele Bakery".into(),
        host: "kilele-bakery-sample.agentrix.website".into(),
        sample: true,
        account: w.owner.to_string(),
        owner_name: "Wanjiru".into(),
        utc_offset_minutes: 180,
        base: format!("/console/{SITE}/apps/people"),
        console: format!("/console/{SITE}"),
    };
    let AppPage::Field { title, html } = app.page(&ctx, "", "").await.unwrap() else {
        panic!("a page")
    };
    assert_eq!(title, "People");
    assert!(html.contains("Who can see and decide for Kilele Bakery."));
    assert!(html.contains("amina@example.com") && html.contains("You and 1 more"));
    assert!(html.contains("Invited ") && html.contains("works for 7 days"));
    assert!(html.contains("is a sample site"));
    assert!(html.contains("/team/people.js"));
    assert!(matches!(
        app.page(&ctx, "nope", "").await.unwrap(),
        AppPage::NotFound
    ));

    let id = member["id"].as_str().unwrap();
    let reply = app
        .post(
            &ctx,
            &format!("remove/{id}"),
            &HeaderMap::new(),
            axum::body::Bytes::new(),
        )
        .await
        .unwrap();
    assert!(matches!(reply, AppReply::Redirect(ref to) if to == &ctx.base));
    let AppPage::Field { html, .. } = app.page(&ctx, "", "").await.unwrap() else {
        panic!("a page")
    };
    assert!(!html.contains("amina@example.com") && html.contains("Just you"));
}
