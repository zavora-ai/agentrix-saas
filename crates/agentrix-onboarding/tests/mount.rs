//! The crate mounts through node's seam and serves its static files.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

#[tokio::test]
async fn mount_serves_the_stylesheet_and_the_island() {
    let ctx = agentrix_host::testing::context().await.unwrap();
    let mounted = agentrix_onboarding::mount(&ctx).unwrap();
    assert!(
        mounted.hosts.is_empty(),
        "onboarding lives on the platform host only"
    );
    for (path, kind) in [
        (
            "/static/onboarding/onboarding.css",
            "text/css; charset=utf-8",
        ),
        (
            "/static/onboarding/assess.js",
            "text/javascript; charset=utf-8",
        ),
    ] {
        let res = mounted
            .platform
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert_eq!(res.headers()[header::CONTENT_TYPE], kind);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(!body.is_empty());
    }
}

#[tokio::test]
async fn the_account_pages_answer_and_setup_wants_a_signed_in_owner() {
    let ctx = agentrix_host::testing::context().await.unwrap();
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    for path in [
        "/start",
        "/account/new?next=/sites/s1/claim&host=kilele-bakery.example",
        "/account/signin",
        "/account/recover?next=//evil.example/",
        "/static/onboarding/account.js",
    ] {
        let res = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(
            !String::from_utf8_lossy(&body).contains("evil.example"),
            "{path}"
        );
    }
    let res = router
        .oneshot(
            Request::get("/setup?next=/sites/s1/claim")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        res.headers()[header::LOCATION],
        "/account/signin?next=/setup%3Fnext%3D/sites/s1/claim"
    );
}

#[tokio::test]
async fn a_plan_choice_is_saved_once_migrated() {
    use agentrix_onboarding::store::{PlanChoice, plan_choice, save_plan_choice};
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap(); // twice is fine
    let mut c = PlanChoice {
        site_id: "site_1".into(),
        account: "acct".into(),
        plan_id: "business".into(),
        ai_budget_usd: 15,
        period: agentrix_billing::Period::Monthly,
        chosen_at: "2026-11-08T07:40:00.000Z".into(),
    };
    save_plan_choice(&ctx.pool, &c).await.unwrap();
    c.plan_id = "commerce".into();
    c.ai_budget_usd = 40;
    c.period = agentrix_billing::Period::Annual;
    save_plan_choice(&ctx.pool, &c).await.unwrap();
    assert_eq!(plan_choice(&ctx.pool, "site_1").await.unwrap(), Some(c));
    assert_eq!(plan_choice(&ctx.pool, "site_2").await.unwrap(), None);
}

/// The owner chooses 12 months on /setup/plan: the choice keeps `annual`, the plan page opens
/// on it again, and the payment step quotes the 12-month total PayPal will charge once (PayPal
/// keys are off here, so nothing is charged).
#[tokio::test]
async fn choosing_twelve_months_stores_annual_and_the_right_amount() {
    use agentrix_billing::Period;
    use agentrix_onboarding::store::plan_choice;
    use std::sync::Arc;
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap();
    let owner = uuid::Uuid::new_v4();
    sqlx::query("CREATE TABLE sites (site_id TEXT PRIMARY KEY, owner_account_id TEXT)")
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sites VALUES ('site_1', ?)")
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    ctx.owners = Arc::new(agentrix_host::testing::FixedOwner::passkey(owner));
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    let post = |body: &'static str| {
        Request::post("/setup/plan")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    };
    let res = router
        .clone()
        .oneshot(post(
            "site=site_1&plan_id=commerce&period=annual&ai_budget_usd=30&next=%2Fsetup%2Fpay%3Fsite%3Dsite_1",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let choice = plan_choice(&ctx.pool, "site_1").await.unwrap().unwrap();
    assert_eq!(
        (choice.plan_id.as_str(), choice.period),
        ("commerce", Period::Annual)
    );
    let commerce = agentrix_billing::plan("commerce").unwrap();
    assert_eq!(commerce.charge_cents(choice.period), 143_040);
    // The pages quote that amount: the plan page reopens on 12 months, the payment step says
    // what PayPal will charge once.
    let page = |uri: &'static str| {
        let router = router.clone();
        async move {
            let res = router
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            String::from_utf8(
                to_bytes(res.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap()
        }
    };
    let plan = page("/setup/plan?site=site_1&next=/setup/pay").await;
    assert!(plan.contains(r#"name="period" value="annual" checked"#));
    assert!(plan.contains("$1,430.40 for 12 months"));
    let pay = page("/setup/pay?site=site_1&next=/console/move").await;
    assert!(pay.contains("$1,430.40 for 12 months"), "{pay}");
    assert!(pay.contains("PayPal isn't switched on here yet"));
    // Monthly is the default for a form without the choice; anything else is refused.
    let res = router
        .clone()
        .oneshot(post("site=site_1&plan_id=essential&ai_budget_usd=5"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let choice = plan_choice(&ctx.pool, "site_1").await.unwrap().unwrap();
    assert_eq!(choice.period, Period::Monthly);
    let res = router
        .oneshot(post(
            "site=site_1&plan_id=essential&period=weekly&ai_budget_usd=5",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_plan_page_wants_a_signed_in_owner() {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap();
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    let res = router
        .oneshot(
            Request::get("/setup/plan?site=site_1&next=/sites/site_1/claim")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert!(
        res.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .starts_with("/account/signin?next=/setup/plan%3Fsite%3Dsite_1")
    );
}

#[tokio::test]
async fn pay_wants_the_proven_owner() {
    use std::sync::Arc;
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap();
    // Signed out: sign in first, then back.
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    let res = router
        .oneshot(
            Request::get("/setup/pay?site=site_1&next=/console/move")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    // Signed in, but the site isn't theirs (no sites row says so): refused, nothing charged.
    sqlx::query("CREATE TABLE sites (site_id TEXT PRIMARY KEY, owner_account_id TEXT)")
        .execute(&ctx.pool)
        .await
        .unwrap();
    ctx.owners = Arc::new(agentrix_host::testing::FixedOwner::passkey(
        uuid::Uuid::new_v4(),
    ));
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    let res = router
        .oneshot(
            Request::get("/setup/pay?site=site_1&next=/console/move")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn billing_wants_a_signed_in_owner() {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap();
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    let res = router
        .oneshot(
            Request::get("/billing?site=site_1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert!(
        res.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .starts_with("/account/signin?next=/billing")
    );
}

#[tokio::test]
async fn the_create_form_never_acts_on_an_existing_session() {
    use std::sync::Arc;
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_onboarding::migrate(&ctx.pool).await.unwrap();
    let get = |router: axum::Router, path: &'static str| async move {
        let res = router
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        String::from_utf8(
            to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    };
    // Signed out: the forms.
    let out = agentrix_onboarding::mount(&ctx).unwrap().platform;
    let page = get(out.clone(), "/account/new?next=/sites/s1/claim").await;
    assert!(page.contains(r#"id="pk-go""#) && !page.contains("You're signed in"));
    // Signed in: no create form, no sign-in form; who it is, continue, or sign out first.
    ctx.owners = Arc::new(agentrix_host::testing::FixedOwner::passkey(
        uuid::Uuid::new_v4(),
    ));
    let router = agentrix_onboarding::mount(&ctx).unwrap().platform;
    for path in [
        "/account/new?next=/sites/s1/claim",
        "/account/signin?next=/os",
    ] {
        let page = get(router.clone(), path).await;
        assert!(page.contains("You're signed in"), "{path}");
        for form in [
            r#"id="pk-go""#,
            r#"id="em-send""#,
            r#"id="pw-form""#,
            "Create with a passkey",
        ] {
            assert!(
                !page.contains(form),
                "{path} offers {form} to a signed-in browser"
            );
        }
        assert!(page.contains(r#"id="sign-out""#) && page.contains(r#"id="continue-as""#));
    }
    let create = get(router.clone(), "/account/new").await;
    assert!(create.contains("Sign out and create a new account"));
    assert!(create.contains("never adds anything to the account you're signed in to"));
    // Recovery keeps its page (it shows a signed-in owner's holds).
    let recover = get(router, "/account/recover").await;
    assert!(
        recover.contains("Get back into your account") && !recover.contains("You're signed in")
    );
}
