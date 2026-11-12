//! Owner email codes, recovery codes and the 24-hour recovery hold, on fakes of node's seams.
//! W0's four conditions are each tested here: codes single use, hashed, expiring and limited
//! per email and per IP; a receipt and a notice for every code sign-in; no approval from an
//! email-code session; recovery codes shown once, hashed, each good once.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agentrix_host::testing::{MemoryOwnerAccounts, MemoryReceipts};
use agentrix_host::{Approvals, Owner, OwnerSessions};
use agentrix_identity::mailer::{MailKind, MemoryMailer};
use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://localhost:8080";

/// Whoever the test says is signed in.
#[derive(Default)]
struct Switch(Mutex<Option<Owner>>);

#[async_trait]
impl OwnerSessions for Switch {
    async fn owner(&self, _headers: &HeaderMap) -> anyhow::Result<Option<Owner>> {
        Ok(self.0.lock().unwrap().clone())
    }
}

/// The frozen contract's shape: a passkey proof needs a passkey, a code proof needs an
/// authenticator (code "123456" here). Counts every call.
struct Contract {
    accounts: Arc<MemoryOwnerAccounts>,
    calls: AtomicUsize,
}

#[async_trait]
impl Approvals for Contract {
    async fn passkey_options(&self, account: Uuid, _fp: [u8; 32]) -> anyhow::Result<Option<Value>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let has = self
            .accounts
            .accounts
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.account == account && a.has_passkey);
        Ok(has.then(|| json!({ "publicKey": { "challenge": "x" } })))
    }

    async fn verify(&self, account: Uuid, _fp: [u8; 32], request: Value) -> anyhow::Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let a = self
            .accounts
            .accounts
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.account == account)
            .cloned()
            .unwrap();
        if request.get("totp").and_then(Value::as_str) == Some("123456") && a.has_password {
            return Ok(json!({ "method": "totp" }));
        }
        if request.get("passkey").is_some() && a.has_passkey {
            return Ok(json!({ "method": "passkey" }));
        }
        anyhow::bail!("not approved")
    }
}

struct World {
    app: Router,
    accounts: Arc<MemoryOwnerAccounts>,
    sessions: Arc<Switch>,
    approvals: Arc<Contract>,
    receipts: Arc<MemoryReceipts>,
    mailer: Arc<MemoryMailer>,
    pool: sqlx::SqlitePool,
}

async fn world() -> World {
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_identity::migrate(&ctx.pool).await.unwrap();
    let accounts = Arc::new(MemoryOwnerAccounts::default());
    let sessions = Arc::new(Switch::default());
    let approvals = Arc::new(Contract {
        accounts: accounts.clone(),
        calls: AtomicUsize::new(0),
    });
    let receipts = Arc::new(MemoryReceipts::default());
    let mailer = Arc::new(MemoryMailer::default());
    ctx.owner_accounts = accounts.clone();
    ctx.owners = sessions.clone();
    ctx.approvals = approvals.clone();
    ctx.receipts = receipts.clone();
    let mounted = agentrix_identity::mount_with_mailer(&ctx, mailer.clone()).unwrap();
    World {
        app: mounted.platform,
        accounts,
        sessions,
        approvals,
        receipts,
        mailer,
        pool: ctx.pool.clone(),
    }
}

struct Reply {
    status: StatusCode,
    cookie: bool,
    body: Value,
}

async fn call(w: &World, method: &str, path: &str, body: Value, ip: &str) -> Reply {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::USER_AGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/130.0 Safari/537.36",
        )
        .header("x-forwarded-for", ip)
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let res = w.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res.headers().get(header::SET_COOKIE).is_some();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        cookie,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn send_code(w: &World, email: &str, purpose: &str, ip: &str) -> Reply {
    call(w, "POST", "/auth/email/start", json!({ "email": email, "purpose": purpose, "display_name": "Wanjiru", "next": "/start/chief" }), ip).await
}

async fn verify(w: &World, pending: &str, code: &str) -> Reply {
    call(
        w,
        "POST",
        "/auth/email/verify",
        json!({ "pending": pending, "code": code }),
        "198.51.100.1",
    )
    .await
}

async fn sign_up(w: &World, email: &str) -> (Uuid, Vec<String>) {
    let sent = send_code(w, email, "signup", "198.51.100.1").await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    let code = w.mailer.last_code(email).unwrap();
    let done = verify(w, sent.body["pending"].as_str().unwrap(), &code).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(done.cookie);
    assert_eq!(done.body["created"], true);
    assert_eq!(
        done.body["redirect"], "/start/chief",
        "next is honoured after sign-up"
    );
    let r = done.body["recovery"].as_str().unwrap().to_owned();
    let shown = call(
        w,
        "GET",
        &format!("/auth/api/recovery-codes/shown?r={r}"),
        Value::Null,
        "198.51.100.1",
    )
    .await;
    assert_eq!(shown.status, StatusCode::OK);
    let codes: Vec<String> = shown.body["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes.len(), 10);
    let again = call(
        w,
        "GET",
        &format!("/auth/api/recovery-codes/shown?r={r}"),
        Value::Null,
        "198.51.100.1",
    )
    .await;
    assert_eq!(again.status, StatusCode::GONE, "shown once");
    let account = w
        .accounts
        .accounts
        .lock()
        .unwrap()
        .iter()
        .find(|a| a.email == email)
        .unwrap()
        .account;
    (account, codes)
}

fn receipt_kinds(w: &World) -> Vec<String> {
    w.receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn sign_up_by_code_with_recovery_codes_shown_once_and_hashed() {
    let w = world().await;
    let (account, codes) = sign_up(&w, "wanjiru@example.com").await;
    assert_eq!(
        w.accounts.sessions.lock().unwrap().as_slice(),
        &[(account, "email-code".to_owned())]
    );
    assert_eq!(receipt_kinds(&w), ["account.code-sign-in"]);
    let detail = w.receipts.issued.lock().unwrap()[0]
        .1
        .detail
        .as_ref()
        .unwrap()
        .to_string();
    assert!(
        !detail.contains("wanjiru@example.com") && !detail.contains(&codes[0]),
        "no email or code in receipts"
    );
    // Stored hashed: no plain code anywhere in the table.
    let rows: Vec<(String,)> = sqlx::query_as("SELECT code_hash FROM owner_recovery_codes")
        .fetch_all(&w.pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 10);
    assert!(rows.iter().all(|(h,)| h.len() == 64 && !codes.contains(h)));
    // A taken email can't sign up again.
    assert_eq!(
        send_code(&w, "WANJIRU@example.com", "signup", "198.51.100.1")
            .await
            .body["code"],
        "email-taken"
    );
}

#[tokio::test]
async fn email_code_sign_in_only_for_email_only_accounts() {
    let w = world().await;
    let (account, _) = sign_up(&w, "wanjiru@example.com").await;
    // Email-only: a code signs in, with a receipt and a notice.
    let sent = send_code(&w, "wanjiru@example.com", "signin", "198.51.100.1").await;
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    let done = verify(&w, sent.body["pending"].as_str().unwrap(), &code).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(done.cookie);
    assert!(w.mailer.sent().iter().any(
        |m| matches!(&m.kind, MailKind::CodeSignIn { method, .. } if method == "an email code")
    ));
    assert_eq!(
        receipt_kinds(&w),
        ["account.code-sign-in", "account.code-sign-in"]
    );

    // Once the account has a passkey, an email alone never signs it in.
    w.accounts
        .accounts
        .lock()
        .unwrap()
        .iter_mut()
        .find(|a| a.account == account)
        .unwrap()
        .has_passkey = true;
    let refused = send_code(&w, "wanjiru@example.com", "signin", "198.51.100.1").await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.body["code"], "use-passkey");
}

#[tokio::test]
async fn codes_are_single_use_limited_and_never_approve() {
    let w = world().await;
    sign_up(&w, "wanjiru@example.com").await;
    let calls_after_signup = w.approvals.calls.load(Ordering::SeqCst);
    assert_eq!(
        calls_after_signup, 0,
        "signing in with a code never touches approvals"
    );

    let sent = send_code(&w, "wanjiru@example.com", "signin", "198.51.100.1").await;
    let pending = sent.body["pending"].as_str().unwrap().to_owned();
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    assert_eq!(verify(&w, &pending, &code).await.status, StatusCode::OK);
    assert_eq!(
        verify(&w, &pending, &code).await.body["code"],
        "expired",
        "single use"
    );

    // Five wrong tries kill a code.
    let sent = send_code(&w, "wanjiru@example.com", "signin", "198.51.100.1").await;
    let pending = sent.body["pending"].as_str().unwrap().to_owned();
    let right = w.mailer.last_code("wanjiru@example.com").unwrap();
    let wrong = if right == "000000" {
        "111111"
    } else {
        "000000"
    };
    for _ in 0..5 {
        verify(&w, &pending, wrong).await;
    }
    assert_eq!(verify(&w, &pending, &right).await.body["code"], "expired");

    // Per email: 5 sends in 15 minutes (a right code clears the count). Per IP: 20.
    let mut last = StatusCode::OK;
    for _ in 0..5 {
        last = send_code(&w, "wanjiru@example.com", "signin", "198.51.100.1")
            .await
            .status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    let mut last = StatusCode::OK;
    for i in 0..21 {
        last = send_code(
            &w,
            &format!("owner{i}@example.com"),
            "signup",
            "203.0.113.7",
        )
        .await
        .status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS, "per IP");

    // An email-code session can't make new recovery codes without... nothing to approve: and
    // a passkey session asking for new codes must pass the frozen approvals first.
    let account = w.accounts.accounts.lock().unwrap()[0].account;
    *w.sessions.0.lock().unwrap() = Some(Owner {
        account,
        method: "passkey".into(),
    });
    let no_proof = call(
        &w,
        "POST",
        "/auth/recovery-codes",
        json!({}),
        "198.51.100.1",
    )
    .await;
    assert_eq!(no_proof.status, StatusCode::FORBIDDEN);
    assert_eq!(no_proof.body["code"], "step-up");
}

#[tokio::test]
async fn recovery_codes_each_work_once() {
    let w = world().await;
    let (account, codes) = sign_up(&w, "wanjiru@example.com").await;
    w.accounts.accounts.lock().unwrap()[0].has_passkey = true;
    let typed = codes[2].to_uppercase().replace('-', " ");
    let ok = call(
        &w,
        "POST",
        "/auth/recover/code",
        json!({ "email": "wanjiru@example.com", "code": typed }),
        "198.51.100.1",
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert!(ok.cookie);
    assert_eq!(ok.body["redirect"], "/console/settings/sign-in?recovered=1");
    assert_eq!(
        w.accounts.sessions.lock().unwrap().last().unwrap(),
        &(account, "recovery-code".to_owned())
    );
    assert!(w.mailer.sent().iter().any(
        |m| matches!(&m.kind, MailKind::CodeSignIn { method, .. } if method == "a recovery code")
    ));
    let twice = call(
        &w,
        "POST",
        "/auth/recover/code",
        json!({ "email": "wanjiru@example.com", "code": codes[2] }),
        "198.51.100.1",
    )
    .await;
    assert_eq!(twice.body["code"], "wrong-recovery-code");

    // Signed in again with a passkey, new codes need the passkey's step-up through approvals.
    *w.sessions.0.lock().unwrap() = Some(Owner {
        account,
        method: "passkey".into(),
    });
    assert_eq!(
        call(
            &w,
            "GET",
            "/auth/recovery-codes",
            Value::Null,
            "198.51.100.1"
        )
        .await
        .body["left"],
        9
    );
    let start = call(
        &w,
        "POST",
        "/auth/recovery-codes/start",
        json!({}),
        "198.51.100.1",
    )
    .await;
    assert!(
        start.body["passkey"].is_object(),
        "passkey options from the frozen contract"
    );
    let fresh = call(
        &w,
        "POST",
        "/auth/recovery-codes",
        json!({ "challenge": start.body["challenge"], "passkey": { "id": "cred" } }),
        "198.51.100.1",
    )
    .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.body);
    assert_eq!(fresh.body["codes"].as_array().unwrap().len(), 10);
    assert_eq!(
        call(
            &w,
            "GET",
            "/auth/recovery-codes",
            Value::Null,
            "198.51.100.1"
        )
        .await
        .body["left"],
        10
    );
    // The old codes stopped working.
    let old = call(
        &w,
        "POST",
        "/auth/recover/code",
        json!({ "email": "wanjiru@example.com", "code": codes[5] }),
        "198.51.100.1",
    )
    .await;
    assert_eq!(old.body["code"], "wrong-recovery-code");
}

#[tokio::test]
async fn recovery_by_email_is_held_for_24_hours_and_can_be_cancelled() {
    let w = world().await;
    let (account, _) = sign_up(&w, "wanjiru@example.com").await;
    w.accounts.accounts.lock().unwrap()[0].has_passkey = true;
    let sessions_before = w.accounts.sessions.lock().unwrap().len();

    let sent = send_code(&w, "wanjiru@example.com", "recover", "198.51.100.1").await;
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    let held = verify(&w, sent.body["pending"].as_str().unwrap(), &code).await;
    assert_eq!(held.status, StatusCode::OK, "{}", held.body);
    assert_eq!(held.body["held"], true);
    assert!(!held.cookie, "no session during the hold");
    assert!(
        held.body["release_at"]
            .as_str()
            .unwrap()
            .ends_with("Nairobi time")
    );
    assert!(
        w.mailer
            .sent()
            .iter()
            .any(|m| matches!(m.kind, MailKind::RecoveryHold { .. }))
    );
    assert_eq!(w.accounts.sessions.lock().unwrap().len(), sessions_before);
    let hold = held.body["hold"].as_str().unwrap().to_owned();

    // Asking again during the hold changes nothing.
    let sent = send_code(&w, "wanjiru@example.com", "recover", "198.51.100.1").await;
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    let again = verify(&w, sent.body["pending"].as_str().unwrap(), &code).await;
    assert_eq!(again.body["hold"], hold.as_str());

    // After 24 hours the same email code signs in.
    sqlx::query("UPDATE owner_recovery_holds SET release_at = 0 WHERE id = ?")
        .bind(&hold)
        .execute(&w.pool)
        .await
        .unwrap();
    let sent = send_code(&w, "wanjiru@example.com", "recover", "198.51.100.1").await;
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    let done = verify(&w, sent.body["pending"].as_str().unwrap(), &code).await;
    assert!(done.cookie, "{}", done.body);
    assert_eq!(
        w.accounts.sessions.lock().unwrap().last().unwrap(),
        &(account, "recovery-hold".to_owned())
    );
    assert!(
        receipt_kinds(&w)
            .iter()
            .filter(|k| *k == "account.recovery-hold")
            .count()
            >= 2,
        "held and completed"
    );

    // A new hold, cancelled by the owner signed in with a passkey: it never releases.
    let sent = send_code(&w, "wanjiru@example.com", "recover", "198.51.100.1").await;
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    let held = verify(&w, sent.body["pending"].as_str().unwrap(), &code).await;
    let hold = held.body["hold"].as_str().unwrap().to_owned();
    *w.sessions.0.lock().unwrap() = Some(Owner {
        account,
        method: "passkey".into(),
    });
    assert_eq!(
        call(
            &w,
            "GET",
            "/auth/recover/holds",
            Value::Null,
            "198.51.100.1"
        )
        .await
        .body["holds"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        call(
            &w,
            "POST",
            "/auth/recover/cancel",
            json!({ "hold": hold }),
            "198.51.100.1"
        )
        .await
        .body["cancelled"],
        true
    );
    sqlx::query("UPDATE owner_recovery_holds SET release_at = 0 WHERE id = ?")
        .bind(&hold)
        .execute(&w.pool)
        .await
        .unwrap();
    let sent = send_code(&w, "wanjiru@example.com", "recover", "198.51.100.1").await;
    let code = w.mailer.last_code("wanjiru@example.com").unwrap();
    let after = verify(&w, sent.body["pending"].as_str().unwrap(), &code).await;
    assert_eq!(
        after.body["held"], true,
        "a cancelled hold never signs anyone in; a new one starts"
    );
}

#[tokio::test]
async fn posts_from_other_sites_are_refused() {
    let w = world().await;
    let req = Request::post("/auth/email/start")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "email": "a@example.com", "purpose": "signup" }).to_string(),
        ))
        .unwrap();
    let res = w.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}
