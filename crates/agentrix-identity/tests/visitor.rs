//! Customer accounts end to end, through the routes W2 merges into each site host's router.

use std::sync::Arc;

use agentrix_host::testing::MemoryReceipts;
use agentrix_identity::mailer::{MailKind, MemoryMailer};
use agentrix_identity::{PlainShell, Visitors};
use agentrix_types::SiteId;
use agentrix_types::tenancy::SiteHost;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

const UA_MAC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const UA_PC: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

fn kilele() -> SiteHost {
    SiteHost {
        site: SiteId::new("site_kilele"),
        host: "kilele-bakery-sample.agentrix.website".into(),
        origin: "https://kilele-bakery-sample.agentrix.website".into(),
        name: "Kilele Bakery".into(),
        sample: true,
    }
}

fn classy() -> SiteHost {
    SiteHost {
        site: SiteId::new("site_classy"),
        host: "classy-perfumes-sample.agentrix.website".into(),
        origin: "https://classy-perfumes-sample.agentrix.website".into(),
        name: "Classy Perfumes".into(),
        sample: true,
    }
}

struct World {
    visitors: Visitors,
    mailer: Arc<MemoryMailer>,
    receipts: Arc<MemoryReceipts>,
}

async fn world() -> World {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_identity::migrate(&ctx.pool).await.unwrap();
    let mailer = Arc::new(MemoryMailer::default());
    let receipts = Arc::new(MemoryReceipts::default());
    let visitors = Visitors::new(ctx.pool.clone(), mailer.clone(), Arc::new(PlainShell))
        .with_receipts(receipts.clone());
    World {
        visitors,
        mailer,
        receipts,
    }
}

fn app(w: &World, site: &SiteHost) -> Router {
    w.visitors.router().layer(axum::Extension(site.clone()))
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    cookies: Vec<String>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }
    fn session_cookie(&self) -> Option<String> {
        self.cookies
            .iter()
            .find(|c| c.starts_with("__Host-agx_customer=") && !c.contains("Max-Age=0"))
            .map(|c| c.split(';').next().unwrap().to_owned())
    }
}

async fn send(app: &Router, req: Request<Body>) -> Reply {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    let cookies = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        location,
        cookies,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn post(site: &SiteHost, path: &str, body: Value, cookie: Option<&str>, ua: &str) -> Request<Body> {
    let mut req = Request::post(path)
        .header(header::ORIGIN, &site.origin)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, ua);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    req.body(Body::from(body.to_string())).unwrap()
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut req = Request::get(path).header(header::USER_AGENT, UA_MAC);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    req.body(Body::empty()).unwrap()
}

/// Creates an account with an email code; returns the session cookie and the recovery codes.
async fn create(w: &World, site: &SiteHost, email: &str) -> (String, Vec<String>) {
    let app = app(w, site);
    let sent = send(&app, post(site, "/account/api/code/start", json!({"purpose": "create", "email": email, "name": "Njeri", "phone": "+44 7700 900418", "next": "/cakes"}), None, UA_MAC)).await;
    assert_eq!(sent.status, StatusCode::OK, "{}", sent.body);
    let pending = sent.json()["pending"].as_str().unwrap().to_owned();
    assert_eq!(
        sent.json()["sent_to"],
        format!("{}***@example.com", &email[..2])
    );
    let code = w.mailer.last_code(email).unwrap();
    let done = send(
        &app,
        post(
            site,
            "/account/api/code/verify",
            json!({"pending": pending, "code": code}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(done.json()["created"], true);
    let cookie = done.session_cookie().expect("signed in");
    let redirect = done.json()["redirect"].as_str().unwrap().to_owned();
    assert!(
        redirect.starts_with("/account/recovery-codes?r="),
        "{redirect}"
    );
    let page = send(&app, get(&redirect, Some(&cookie))).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Save your recovery codes"));
    let codes: Vec<String> = page
        .body
        .split("<li>")
        .skip(1)
        .map(|s| s.split("</li>").next().unwrap().to_owned())
        .filter(|s| s.len() == 9 && s.contains('-'))
        .collect();
    assert_eq!(codes.len(), 8, "{}", page.body);
    // After the codes: a passkey offer, then the choices, then back to the cakes.
    assert!(page.body.contains("passkey-offer?next="), "{}", page.body);
    let again = send(&app, get(&redirect, Some(&cookie))).await;
    assert!(
        again.body.contains("These codes were shown once"),
        "shown once only"
    );
    (cookie, codes)
}

#[tokio::test]
async fn migrations_live_beside_nodes_own() {
    // Node's migrator uses `_sqlx_migrations`; ours uses `_sqlx_migrations_identity`, so even
    // the same version number in both never clashes.
    let dir = std::env::temp_dir().join(format!("w6-node-migrations-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("0001_node.sql"),
        "CREATE TABLE node_things (id TEXT PRIMARY KEY);",
    )
    .unwrap();
    std::fs::write(
        dir.join("0600_node_other.sql"),
        "CREATE TABLE node_more (id TEXT PRIMARY KEY);",
    )
    .unwrap();
    let ctx = agentrix_host::testing::context().await.unwrap();
    let node = sqlx::migrate::Migrator::new(dir.as_path()).await.unwrap();
    node.run(&ctx.pool).await.unwrap();
    agentrix_identity::migrate(&ctx.pool).await.unwrap();
    agentrix_identity::migrate(&ctx.pool).await.unwrap();
    node.run(&ctx.pool).await.unwrap();
    let tables: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .fetch_all(&ctx.pool)
            .await
            .unwrap();
    let names: Vec<_> = tables.into_iter().map(|(n,)| n).collect();
    for t in [
        "node_things",
        "node_more",
        "visitor_accounts",
        "consent_receipts",
        "_sqlx_migrations",
        "_sqlx_migrations_identity",
    ] {
        assert!(names.iter().any(|n| n == t), "{t} missing from {names:?}");
    }
    let f: agentrix_host::MigrateFn = agentrix_identity::migrate;
    f(&ctx.pool).await.unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn create_with_a_code_then_sign_in_again_and_see_devices() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    let (cookie, _codes) = create(&w, &site, "njeri@example.com").await;

    let session = send(&app, get("/account/api/session", Some(&cookie)))
        .await
        .json();
    assert_eq!(session["signed_in"], true);
    assert_eq!(
        session["account"]["email_verified"], true,
        "a code proves the email"
    );
    assert_eq!(session["account"]["has_passkey"], false);

    // A second device signs in with a code: a new-device alert and an email.
    let sent = send(
        &app,
        post(
            &site,
            "/account/api/code/start",
            json!({"purpose": "sign-in", "email": "NJERI@example.com", "next": "/orders"}),
            None,
            UA_PC,
        ),
    )
    .await;
    let pending = sent.json()["pending"].as_str().unwrap().to_owned();
    let code = w.mailer.last_code("njeri@example.com").unwrap();
    let done = send(
        &app,
        post(
            &site,
            "/account/api/code/verify",
            json!({"pending": pending, "code": code}),
            None,
            UA_PC,
        ),
    )
    .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert_eq!(
        done.json()["redirect"],
        "/account/passkey-offer?next=%2Forders",
        "one passkey offer per device"
    );
    let pc = done.session_cookie().unwrap();
    assert!(w.mailer.sent().iter().any(|m| matches!(&m.kind, MailKind::NewSignIn { device, .. } if device == "Chrome on a Windows PC")));

    let page = send(&app, get("/account/security", Some(&cookie))).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("New sign-in on Chrome on a Windows PC"),
        "{}",
        page.body
    );
    assert!(page.body.contains("Chrome on a Mac"));
    assert!(page.body.contains("Now, with a code by email"));
    assert!(
        page.body.contains("Sample site."),
        "the plain shell marks samples"
    );

    // "Sign out everywhere" ends both sessions, this one too.
    let ended = send(
        &app,
        post(
            &site,
            "/account/api/sessions/end",
            json!({"all": true}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(ended.json()["ended"], 2);
    for c in [&cookie, &pc] {
        assert_eq!(
            send(&app, get("/account/api/session", Some(c)))
                .await
                .json()["signed_in"],
            false
        );
    }
    let gone = send(&app, get("/account/security", Some(&cookie))).await;
    assert_eq!(gone.status, StatusCode::SEE_OTHER);
    assert_eq!(
        gone.location.as_deref(),
        Some("/account/sign-in?next=%2Faccount%2Fsecurity")
    );
}

#[tokio::test]
async fn codes_are_checked_and_limited() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    create(&w, &site, "njeri@example.com").await;

    // Unknown email: the sign-in page sends people to create an account.
    let none = send(
        &app,
        post(
            &site,
            "/account/api/code/start",
            json!({"purpose": "sign-in", "email": "nobody@example.com"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(none.status, StatusCode::NOT_FOUND);
    assert_eq!(none.json()["code"], "no-account");
    // Creating again with a taken email.
    let taken = send(
        &app,
        post(
            &site,
            "/account/api/code/start",
            json!({"purpose": "create", "email": "njeri@example.com", "name": "N"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(taken.status, StatusCode::CONFLICT);

    // Five wrong tries and the code stops working.
    let sent = send(
        &app,
        post(
            &site,
            "/account/api/code/start",
            json!({"purpose": "sign-in", "email": "njeri@example.com"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    let pending = sent.json()["pending"].as_str().unwrap().to_owned();
    let right = w.mailer.last_code("njeri@example.com").unwrap();
    let wrong = if right == "000000" {
        "111111"
    } else {
        "000000"
    };
    for i in 0..5 {
        let r = send(
            &app,
            post(
                &site,
                "/account/api/code/verify",
                json!({"pending": pending, "code": wrong}),
                None,
                UA_MAC,
            ),
        )
        .await;
        let expect = if i < 4 {
            "wrong-code"
        } else {
            "too-many-tries"
        };
        assert_eq!(r.json()["code"], expect, "try {i}");
    }
    let late = send(
        &app,
        post(
            &site,
            "/account/api/code/verify",
            json!({"pending": pending, "code": right}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(late.json()["code"], "expired");

    // Sends per email are limited (5 in 15 minutes, counting the one above).
    let mut last = StatusCode::OK;
    for _ in 0..6 {
        last = send(
            &app,
            post(
                &site,
                "/account/api/code/start",
                json!({"purpose": "sign-in", "email": "njeri@example.com"}),
                None,
                UA_MAC,
            ),
        )
        .await
        .status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn recovery_codes_work_once() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    let (_, codes) = create(&w, &site, "njeri@example.com").await;
    let typed = codes[3].to_uppercase().replace('-', " ");
    let ok = send(
        &app,
        post(
            &site,
            "/account/api/recovery/use",
            json!({"email": "njeri@example.com", "code": typed}),
            None,
            UA_PC,
        ),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert_eq!(ok.json()["redirect"], "/account/security?recovered=1");
    let cookie = ok.session_cookie().unwrap();
    assert!(
        w.mailer
            .sent()
            .iter()
            .any(|m| matches!(m.kind, MailKind::RecoveryCodeUsed { remaining: 7, .. }))
    );
    let page = send(&app, get("/account/security?recovered=1", Some(&cookie))).await;
    assert!(page.body.contains("You're back in."));
    assert!(page.body.contains("7 of 8 left."));
    let twice = send(
        &app,
        post(
            &site,
            "/account/api/recovery/use",
            json!({"email": "njeri@example.com", "code": codes[3]}),
            None,
            UA_PC,
        ),
    )
    .await;
    assert_eq!(twice.status, StatusCode::UNAUTHORIZED);
    assert_eq!(twice.json()["code"], "wrong-recovery-code");
}

#[tokio::test]
async fn consents_leave_signed_receipts_and_memory_needs_18_plus() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    let (cookie, _) = create(&w, &site, "njeri@example.com").await;

    let on = send(
        &app,
        post(
            &site,
            "/account/api/consents",
            json!({"purpose": "order-updates", "on": true}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.body);
    assert_eq!(
        on.json()["receipt"]["what"],
        "Your phone number, your order numbers and their status. No offers."
    );

    let child = send(
        &app,
        post(
            &site,
            "/account/api/consents",
            json!({"purpose": "account-memory", "on": true}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(child.status, StatusCode::BAD_REQUEST);
    let adult = send(
        &app,
        post(
            &site,
            "/account/api/consents",
            json!({"purpose": "account-memory", "on": true, "adult": true}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(adult.json()["receipt"]["adult_confirmed"], true);

    let off = send(
        &app,
        post(
            &site,
            "/account/api/consents",
            json!({"purpose": "order-updates", "on": false}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(off.status, StatusCode::OK);

    let kinds: Vec<&str> = w
        .receipts
        .issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str())
        .collect::<Vec<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        kinds,
        ["consent.given", "consent.given", "consent.withdrawn"]
    );

    let page = send(&app, get("/account/choices?next=/cakes", Some(&cookie))).await;
    assert!(page.body.contains("Consent receipt · WhatsApp updates"));
    assert!(page.body.contains("Withdrawn"));
    assert!(page.body.contains("You confirmed you're 18 or over"));
    assert!(
        page.body
            .contains("signed by kilele-bakery-sample.agentrix.website")
    );
    assert!(page.body.contains("Back to where you were"));
}

#[tokio::test]
async fn a_session_never_crosses_sites_and_posts_must_be_same_origin() {
    let w = world().await;
    let (k, c) = (kilele(), classy());
    let (cookie, _) = create(&w, &k, "njeri@example.com").await;
    let classy_app = app(&w, &c);
    let other = send(&classy_app, get("/account/api/session", Some(&cookie))).await;
    assert_eq!(
        other.json()["signed_in"],
        false,
        "Kilele's cookie means nothing on Classy"
    );
    // The same email can have a separate account on another site.
    create(&w, &c, "njeri@example.com").await;

    let kilele_app = app(&w, &k);
    let mut forged = post(
        &k,
        "/account/api/sessions/end",
        json!({"all": true}),
        Some(&cookie),
        UA_MAC,
    );
    forged.headers_mut().insert(
        header::ORIGIN,
        "https://kilele-bakery-sample.agentrix.website.evil.example"
            .parse()
            .unwrap(),
    );
    let refused = send(&kilele_app, forged).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json()["code"], "cross-origin");

    let bare = w.visitors.router();
    let not_site = send(&bare, get("/account/api/session", None)).await;
    assert_eq!(
        not_site.status,
        StatusCode::NOT_FOUND,
        "no SiteHost: not a site page"
    );
}

#[tokio::test]
async fn passkey_options_use_the_sites_own_host() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    let start = send(
        &app,
        post(
            &site,
            "/account/api/passkey/register/start",
            json!({"name": "Njeri", "email": "njeri@example.com", "next": "/cakes"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.body);
    let options = &start.json()["options"]["publicKey"];
    assert_eq!(options["rp"]["id"], "kilele-bakery-sample.agentrix.website");
    assert_eq!(options["user"]["name"], "njeri@example.com");
    // Signing in with a passkey for an account that has none says so.
    create(&w, &site, "amina@example.com").await;
    let none = send(
        &app,
        post(
            &site,
            "/account/api/passkey/login/start",
            json!({"email": "amina@example.com"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(none.json()["code"], "no-passkey");
}

#[tokio::test]
async fn no_javascript_code_sign_in_and_the_sample_mailbox() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    create(&w, &site, "njeri@example.com").await;
    let form = Request::post("/account/code/send")
        .header(header::ORIGIN, &site.origin)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(
            "purpose=sign-in&email=njeri%40example.com&next=%2Forders",
        ))
        .unwrap();
    let sent = send(&app, form).await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    let code_page = sent.location.unwrap();
    assert!(code_page.starts_with("/account/code?p="));
    let page = send(&app, get(&code_page, None)).await;
    let code = w.mailer.last_code("njeri@example.com").unwrap();
    assert!(
        page.body.contains("Sample mailbox."),
        "reserved address on a sample site"
    );
    assert!(page.body.contains(&code));
    let pending = code_page.trim_start_matches("/account/code?p=");
    let pending = url::form_urlencoded::parse(format!("p={pending}").as_bytes())
        .next()
        .unwrap()
        .1
        .into_owned();
    let check = Request::post("/account/code/check")
        .header(header::ORIGIN, &site.origin)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, "agx_passkey_offer=no")
        .body(Body::from(format!(
            "pending={}&code={code}",
            url::form_urlencoded::byte_serialize(pending.as_bytes()).collect::<String>()
        )))
        .unwrap();
    let done = send(&app, check).await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    assert_eq!(
        done.location.as_deref(),
        Some("/orders"),
        "this device said Not now, so no passkey offer"
    );
    assert!(done.session_cookie().is_some());

    // A real-looking address on the same sample: no code on the page.
    let w2 = world().await;
    let app2 = w2.visitors.router().layer(axum::Extension(site.clone()));
    let sent = send(
        &app2,
        post(
            &site,
            "/account/api/code/start",
            json!({"purpose": "create", "email": "njeri@gmail.com", "name": "Njeri"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    let page = send(&app2, get(sent.json()["redirect"].as_str().unwrap(), None)).await;
    assert!(!page.body.contains("Sample mailbox."));
}

#[tokio::test]
async fn every_page_renders_signed_out_and_signed_in() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    let signed_out = [
        ("/account/sign-in?why=order&next=/cakes", "Sign in to order"),
        ("/account/sign-in", "Sign in to Kilele Bakery"),
        (
            "/account/sign-in?again=1&expired=1&error=no-account",
            "Sign in again to change this.",
        ),
        ("/account/create?next=/cakes", "Create your account"),
        ("/account/code?p=nope", "That code has expired"),
        (
            "/account/recovery-codes?r=nope",
            "These codes were shown once",
        ),
        ("/account/recover", "Get back into your account"),
        ("/account/keeps", "What Kilele Bakery keeps"),
        ("/account/signed-out", "Your bag stays on this device."),
    ];
    for (path, words) in signed_out {
        let page = send(&app, get(path, None)).await;
        assert_eq!(page.status, StatusCode::OK, "{path}: {}", page.body);
        assert!(page.body.contains(words), "{path} should say {words:?}");
        assert!(page.body.contains("Sample site."), "{path}");
    }
    for path in [
        "/account/security",
        "/account/choices",
        "/account/passkey-offer",
    ] {
        let page = send(&app, get(path, None)).await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{path} needs sign-in");
    }
    let (cookie, _) = create(&w, &site, "njeri@example.com").await;
    let signed_in = [
        ("/account/security", "Sign-in and security"),
        ("/account/choices?next=/cakes", "Your choices"),
        (
            "/account/passkey-offer?next=/cakes",
            "Next time, sign in with this device's screen lock",
        ),
        ("/account/keeps", "What Kilele Bakery keeps"),
    ];
    for (path, words) in signed_in {
        let page = send(&app, get(path, Some(&cookie))).await;
        assert_eq!(page.status, StatusCode::OK, "{path}: {}", page.body);
        assert!(page.body.contains(words), "{path} should say {words:?}");
        assert!(
            page.body.contains(">N</span>Njeri"),
            "{path}: the account chip"
        );
    }
    for path in ["/account/account.js", "/account/account.css"] {
        assert_eq!(send(&app, get(path, None)).await.status, StatusCode::OK);
    }
}

/// Another workstream's store, as W2's orders would register it.
struct Orders {
    open: bool,
    erased: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl adk_core::subject::SubjectDataStore for Orders {
    fn name(&self) -> &str {
        "Orders and receipts"
    }

    async fn export(&self, scope: &adk_core::subject::SubjectScope) -> adk_core::Result<Value> {
        Ok(json!([{ "order": "KB-1042", "customer": scope.subject.id }]))
    }

    async fn erase(
        &self,
        scope: &adk_core::subject::SubjectScope,
    ) -> adk_core::Result<adk_core::subject::StoreErasure> {
        if self.open {
            return Err(adk_core::AdkError::internal(
                adk_core::ErrorComponent::Tool,
                agentrix_identity::visitor::rights::OPEN_ORDERS_CODE,
                "open order KB-1043",
            ));
        }
        self.erased.lock().unwrap().push(scope.subject.id.clone());
        Ok(adk_core::subject::StoreErasure {
            deleted: 0,
            retained: 2,
            completes_by: None,
        })
    }
}

#[tokio::test]
async fn download_my_data_and_delete_my_account() {
    let w = world().await;
    let orders = Arc::new(Orders {
        open: false,
        erased: Default::default(),
    });
    let visitors = w.visitors.clone().with_subject_store(orders.clone());
    let site = kilele();
    let app = visitors.router().layer(axum::Extension(site.clone()));
    let w2 = World {
        visitors,
        mailer: w.mailer.clone(),
        receipts: w.receipts.clone(),
    };
    let (cookie, _) = create(&w2, &site, "njeri@example.com").await;
    send(
        &app,
        post(
            &site,
            "/account/api/consents",
            json!({"purpose": "offers", "on": true}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;

    let page = send(&app, get("/account/data", Some(&cookie))).await;
    assert!(
        page.body.contains("Download my data") && page.body.contains("Orders and receipts"),
        "{} {:?} {}",
        page.status,
        page.location,
        page.body
    );
    let export = send(&app, get("/account/api/export", Some(&cookie))).await;
    assert_eq!(export.status, StatusCode::OK);
    let bundle = export.json();
    assert_eq!(bundle["type"], "agentrix.subject-export.v1");
    assert_eq!(
        bundle["stores"]["identity.accounts"][0]["account"]["email"],
        "njeri@example.com"
    );
    assert_eq!(
        bundle["stores"]["identity.accounts"][0]["consents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        bundle["stores"]["Orders and receipts"][0]["order"],
        "KB-1042"
    );
    assert!(
        !export.body.contains("code_hash") && !export.body.contains("passkey\":{"),
        "no secrets in the bundle"
    );

    let deleted = send(
        &app,
        post(
            &site,
            "/account/api/delete",
            json!({}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.body);
    let receipt_page = deleted.json()["redirect"].as_str().unwrap().to_owned();
    assert_eq!(
        orders.erased.lock().unwrap().len(),
        1,
        "the other store was asked too"
    );
    assert_eq!(
        send(&app, get("/account/api/session", Some(&cookie)))
            .await
            .json()["signed_in"],
        false
    );
    let page = send(&app, get(&receipt_page, None)).await;
    assert!(
        page.body.contains("Erasure receipt") && page.body.contains("Complete"),
        "{}",
        page.body
    );
    assert!(page.body.contains("kept 2 as the law requires"));
    assert!(
        page.body.contains("Account, sign-in and choices"),
        "stores are named in words"
    );
    assert!(
        !page.body.contains("njeri@example.com"),
        "the receipt holds no personal details"
    );
    // The account is gone: the same email can start fresh.
    let again = send(
        &app,
        post(
            &site,
            "/account/api/code/start",
            json!({"purpose": "sign-in", "email": "njeri@example.com"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(again.json()["code"], "no-account");
}

#[tokio::test]
async fn an_open_order_keeps_the_account() {
    let w = world().await;
    let visitors = w.visitors.clone().with_subject_store(Arc::new(Orders {
        open: true,
        erased: Default::default(),
    }));
    let site = kilele();
    let app = visitors.router().layer(axum::Extension(site.clone()));
    let w2 = World {
        visitors,
        mailer: w.mailer.clone(),
        receipts: w.receipts.clone(),
    };
    let (cookie, _) = create(&w2, &site, "njeri@example.com").await;
    let refused = send(
        &app,
        post(
            &site,
            "/account/api/delete",
            json!({}),
            Some(&cookie),
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(
        refused.json()["message"]
            .as_str()
            .unwrap()
            .contains("open order")
    );
    assert_eq!(
        send(&app, get("/account/api/session", Some(&cookie)))
            .await
            .json()["signed_in"],
        true,
        "nothing was erased"
    );
}

fn form(site: &SiteHost, path: &str, body: &str) -> Request<Body> {
    Request::post(path)
        .header(header::ORIGIN, &site.origin)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// An address in a URL lands in server and proxy logs, browser history and Referer headers.
fn no_address(location: &str) {
    let lower = location.to_ascii_lowercase();
    assert!(
        !lower.contains('@') && !lower.contains("%40") && !lower.contains("email="),
        "an address in {location}"
    );
}

#[tokio::test]
async fn no_account_redirect_carries_an_email_address() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    create(&w, &site, "njeri@example.com").await;

    // "Email me a code" for a new address: on to create, with the address kept on the server.
    let sent = send(
        &app,
        form(
            &site,
            "/account/code/send",
            "purpose=sign-in&email=amina%40example.com&next=%2Fcakes",
        ),
    )
    .await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    let to = sent.location.unwrap();
    assert!(to.starts_with("/account/create?next=%2Fcakes&f="), "{to}");
    no_address(&to);
    let page = send(&app, get(&to, None)).await;
    assert!(
        page.body.contains("value=\"amina@example.com\""),
        "the form keeps what was typed"
    );

    // Creating with a taken address: back to the form with the error and what was typed.
    let taken = send(&app, form(&site, "/account/code/send", "purpose=create&email=njeri%40example.com&name=Njeri&phone=%2B254700000000&next=%2Fcakes")).await;
    let to = taken.location.unwrap();
    no_address(&to);
    assert!(to.contains("error=email-taken"));
    let page = send(&app, get(&to, None)).await;
    assert!(
        page.body.contains("value=\"njeri@example.com\"") && page.body.contains("value=\"Njeri\"")
    );
    assert!(page.body.contains("already exists"));

    // A bad address and recovery for an unknown one: no address either.
    for body in [
        "purpose=sign-in&email=not-an-address&next=%2F",
        "purpose=recover&email=nobody%40example.com&next=%2F",
    ] {
        let reply = send(&app, form(&site, "/account/code/send", body)).await;
        no_address(&reply.location.unwrap());
    }

    // The passkey button for a new address: the island follows `redirect`, which has no address.
    let start = send(
        &app,
        post(
            &site,
            "/account/api/passkey/login/start",
            json!({"email": "amina@example.com", "next": "/cakes"}),
            None,
            UA_MAC,
        ),
    )
    .await;
    assert_eq!(start.json()["code"], "no-account");
    let to = start.json()["redirect"].as_str().unwrap().to_owned();
    no_address(&to);
    assert!(
        send(&app, get(&to, None))
            .await
            .body
            .contains("value=\"amina@example.com\"")
    );

    // A prefill made on one site shows nothing on another.
    let classy_app = w.visitors.router().layer(axum::Extension(classy()));
    let other = send(&classy_app, get(&to, None)).await;
    assert!(!other.body.contains("amina@example.com"));
}

/// A test customer agent (`test-agent-<name>@example.com`) on a sample site reads its code on
/// the page, labelled as a test agent's, and nothing is mailed to its reserved address.
#[tokio::test]
async fn a_test_customer_agent_reads_its_code_on_the_page_and_no_mail_goes_out() {
    let w = world().await;
    let site = kilele();
    let app = app(&w, &site);
    let form = Request::post("/account/code/send")
        .header(header::ORIGIN, &site.origin)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(
            "purpose=create&name=Amani+%28Test+customer+agent%29&email=test-agent-amani%40example.com&next=%2Fcheckout",
        ))
        .unwrap();
    let sent = send(&app, form).await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    let code_page = sent.location.unwrap();
    let page = send(&app, get(&code_page, None)).await;
    assert!(page.body.contains("Test customer agent."), "{}", page.body);
    assert!(!page.body.contains("Sample mailbox."));
    assert!(page.body.contains("acct-test-code"));
    assert!(
        w.mailer.last_code("test-agent-amani@example.com").is_none(),
        "no email to a test agent's reserved address"
    );
}
