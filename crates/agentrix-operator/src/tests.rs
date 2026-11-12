//! The operator console end to end: sign-in, the pages, and what they must never show.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use url::Url;

use crate::testing::{self, FakeApprovals};
use crate::{OperatorConfig, OperatorState, ledger, router};

struct Harness {
    state: OperatorState,
    fake: std::sync::Arc<FakeApprovals>,
}

async fn harness() -> Harness {
    let pool = testing::pool().await;
    let (fake, approvals) = testing::approvals();
    let config = OperatorConfig::new(Url::parse("http://ops.localhost:18300").unwrap());
    let state = OperatorState::new(config, pool, approvals).unwrap();
    Harness { state, fake }
}

async fn call(
    state: &OperatorState,
    req: Request<Body>,
) -> (StatusCode, header::HeaderMap, String) {
    let res = router(state.clone()).oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .unwrap();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn post_json(path: &str, body: Value) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "http://ops.localhost:18300")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut req = Request::get(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    req.body(Body::empty()).unwrap()
}

/// Signs `email` in with the fake authenticator code and returns the session cookie.
async fn sign_in(state: &OperatorState, email: &str) -> String {
    let (status, _, body) =
        call(state, post_json("/signin/start", json!({ "email": email }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let started: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(started["method"], "totp");
    let (status, headers, body) = call(
        state,
        post_json("/signin/finish", json!({ "ceremony": started["ceremony"], "email": email, "next": "/customers", "proof": { "method": "totp", "code": "246810" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["next"],
        "/customers"
    );
    let cookie = headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        cookie.starts_with("agx_ops=")
            && cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict"),
        "{cookie}"
    );
    cookie.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn signed_out_pages_send_you_to_sign_in_and_back() {
    let h = harness().await;
    let (status, headers, _) = call(&h.state, get("/sites/site_x?y=1", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers.get(header::LOCATION).unwrap(),
        "/signin?next=%2Fsites%2Fsite_x%3Fy%3D1"
    );
    let (status, _, body) = call(&h.state, get("/signin?next=/fleet", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Sign in to the control plane"));
}

#[tokio::test]
async fn only_people_with_an_operator_role_can_sign_in() {
    let h = harness().await;
    let owner = testing::account(h.state.pool(), "wanjiru@kilele.example", "Wanjiru Kamau").await;
    let _ = owner;
    let (status, _, body) = call(
        &h.state,
        post_json(
            "/signin/start",
            json!({ "email": "wanjiru@kilele.example" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("can't sign in to the control plane"));
    // Unknown emails get the same answer, so the form doesn't reveal who has an account.
    let (status2, _, body2) = call(
        &h.state,
        post_json("/signin/start", json!({ "email": "nobody@example.test" })),
    )
    .await;
    assert_eq!((status2, body2), (status, body));
}

#[tokio::test]
async fn a_wrong_code_signs_nobody_in_and_a_cross_site_post_is_refused() {
    let h = harness().await;
    let op = testing::account(h.state.pool(), "achieng@zavora.example", "Achieng Otieno").await;
    testing::grant(h.state.pool(), op, &["op.sre"]).await;
    let (_, _, body) = call(
        &h.state,
        post_json(
            "/signin/start",
            json!({ "email": "achieng@zavora.example" }),
        ),
    )
    .await;
    let started: Value = serde_json::from_str(&body).unwrap();
    let (status, headers, _) = call(
        &h.state,
        post_json("/signin/finish", json!({ "ceremony": started["ceremony"], "email": "achieng@zavora.example", "proof": { "method": "totp", "code": "111111" } })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get(header::SET_COOKIE).is_none());
    // The ceremony was used up: retrying it fails even with the right code.
    let (status, _, _) = call(
        &h.state,
        post_json("/signin/finish", json!({ "ceremony": started["ceremony"], "email": "achieng@zavora.example", "proof": { "method": "totp", "code": "246810" } })),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    let req = Request::post("/signin/start")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, "https://kilele.agentrix.website")
        .body(Body::from(
            json!({ "email": "achieng@zavora.example" }).to_string(),
        ))
        .unwrap();
    let (status, _, _) = call(&h.state, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_passkey_sign_in_approves_the_sign_in_fingerprint() {
    let h = harness().await;
    let op = testing::account(h.state.pool(), "otieno@zavora.example", "Otieno Kamau").await;
    testing::grant(h.state.pool(), op, &["op.release"]).await;
    h.fake.passkeys.lock().unwrap().push(op);
    let (_, _, body) = call(
        &h.state,
        post_json("/signin/start", json!({ "email": "otieno@zavora.example" })),
    )
    .await;
    let started: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(started["method"], "passkey");
    let challenge = started["options"]["publicKey"]["challenge"].clone();
    let (status, headers, body) = call(
        &h.state,
        post_json("/signin/finish", json!({ "ceremony": started["ceremony"], "email": "otieno@zavora.example", "proof": { "method": "passkey", "credential": { "challenge": challenge } } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(headers.get(header::SET_COOKIE).is_some());
    let entry = ledger::recent(h.state.pool(), 1).await.unwrap().remove(0);
    assert_eq!(entry.verb, "operator.signin");
    assert_eq!(
        entry.summary,
        "Otieno Kamau signed in to the control plane with a passkey."
    );
}

#[tokio::test]
async fn the_pages_render_real_records_and_never_site_content() {
    let h = harness().await;
    let pool = h.state.pool().clone();
    let op = testing::account(&pool, "achieng@zavora.example", "Achieng Otieno").await;
    testing::grant(&pool, op, &["op.sre", "op.admin"]).await;
    let owner = testing::account(&pool, "wanjiru@kilele.example", "Wanjiru Kamau").await;
    let site = testing::seed_live_site(&pool, owner, "test-bakery.example").await;
    let cookie = sign_in(&h.state, "achieng@zavora.example").await;

    let (status, headers, fleet) = call(&h.state, get("/fleet", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK, "{fleet}");
    assert!(headers.get("content-security-policy").is_some());
    for needle in [
        "Fleet",
        "Good ",
        "Achieng",
        "1 site</b> serves signed releases",
        "$0.40",
        "1 of 1 live site held every attack",
        "Replay",
        "Go-live card",
        "Achieng Otieno signed in to the control plane with an authenticator code.",
    ] {
        assert!(fleet.contains(needle), "fleet lacks {needle:?}");
    }
    let (status, _, customers) = call(&h.state, get("/customers", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK, "{customers}");
    for needle in [
        "Test Bakery",
        "Wanjiru Kamau",
        "Live",
        "v1",
        "10/10",
        "$0.40",
        "of $5.00",
        "No plan yet",
    ] {
        assert!(customers.contains(needle), "customers lacks {needle:?}");
    }
    let (status, _, page) = call(&h.state, get(&format!("/sites/{site}"), Some(&cookie))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        &page[page.len().saturating_sub(1500)..]
    );
    for needle in [
        "Test Bakery",
        "golive",
        "release.signed",
        "order-held",
        "vision-parity",
        "anthropic",
    ] {
        assert!(page.contains(needle), "site page lacks {needle:?}");
    }
    // Content-free: a card's body and a receipt's title never reach the operator.
    for page in [&fleet, &customers, &page] {
        assert!(!page.contains("Amina") && !page.contains("private words"));
    }
    let (status, _, trust) = call(&h.state, get("/trust", Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK, "{trust}");
    assert!(trust.contains("Chain intact"));
    let (status, _, missing) = call(&h.state, get("/sites/site_nope", Some(&cookie))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(missing.contains("Site not found"));
    for app in [
        "/rollouts",
        "/incidents",
        "/money",
        "/partners",
        "/holds",
        "/cli",
    ] {
        let (status, _, _) = call(&h.state, get(app, Some(&cookie))).await;
        assert_eq!(status, StatusCode::OK, "{app}");
    }
}

#[tokio::test]
async fn signing_out_ends_the_session_and_a_revoked_role_locks_you_out() {
    let h = harness().await;
    let pool = h.state.pool().clone();
    let op = testing::account(&pool, "achieng@zavora.example", "Achieng Otieno").await;
    testing::grant(&pool, op, &["op.sre"]).await;
    let cookie = sign_in(&h.state, "achieng@zavora.example").await;
    assert_eq!(
        call(&h.state, get("/fleet", Some(&cookie))).await.0,
        StatusCode::OK
    );
    sqlx::query("UPDATE operator_roles SET revoked_at = 'x'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&h.state, get("/fleet", Some(&cookie))).await.0,
        StatusCode::SEE_OTHER
    );
    sqlx::query("UPDATE operator_roles SET revoked_at = NULL")
        .execute(&pool)
        .await
        .unwrap();
    let req = Request::post("/signout")
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, "http://ops.localhost:18300")
        .body(Body::empty())
        .unwrap();
    assert_eq!(call(&h.state, req).await.0, StatusCode::SEE_OTHER);
    assert_eq!(
        call(&h.state, get("/fleet", Some(&cookie))).await.0,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn mount_serves_the_ops_host() {
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    let pool = testing::pool().await;
    ctx.pool = pool;
    let mounted = crate::mount(&ctx).unwrap();
    assert_eq!(mounted.hosts.len(), 1);
    assert!(
        mounted.hosts[0]
            .matcher
            .serves("ops.localhost")
            .await
            .unwrap()
    );
    assert!(!mounted.hosts[0].matcher.serves("localhost").await.unwrap());
    assert_eq!(mounted.tasks.len(), 1);
}

#[test]
fn approval_requests_carry_both_the_seams_shape_and_the_first_releases() {
    // Node's adapter parses the first release's internally tagged ApproveRequest; the seam's doc says
    // {"totp": code}. The request must satisfy both.
    #[derive(serde::Deserialize)]
    #[serde(tag = "method", rename_all = "lowercase")]
    enum FirstRelease {
        Totp { code: String },
    }
    let (fake, approvals) = testing::approvals();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let body = json!({ "method": "totp", "code": "246 810" });
    let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
    struct Spy(
        std::sync::Arc<std::sync::Mutex<Option<Value>>>,
        std::sync::Arc<dyn agentrix_host::Approvals>,
    );
    #[async_trait::async_trait]
    impl agentrix_host::Approvals for Spy {
        async fn passkey_options(
            &self,
            a: uuid::Uuid,
            f: [u8; 32],
        ) -> anyhow::Result<Option<Value>> {
            self.1.passkey_options(a, f).await
        }
        async fn verify(&self, a: uuid::Uuid, f: [u8; 32], r: Value) -> anyhow::Result<Value> {
            *self.0.lock().unwrap() = Some(r.clone());
            self.1.verify(a, f, r).await
        }
    }
    let spy = Spy(seen.clone(), approvals);
    let proof = rt
        .block_on(crate::identity::approve(
            &spy,
            uuid::Uuid::nil(),
            [7; 32],
            &body,
        ))
        .unwrap();
    assert_eq!(proof.method, "totp");
    let sent = seen.lock().unwrap().clone().unwrap();
    assert_eq!(sent["totp"], "246810");
    let FirstRelease::Totp { code } = serde_json::from_value(sent).unwrap();
    assert_eq!(code, "246810");
    assert_eq!(fake.signed.lock().unwrap().len(), 1);
}
