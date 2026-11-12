//! Support grants, break-glass and holds, end to end on both hosts.

use std::sync::Arc;

use agentrix_host::testing::{FixedOwner, MemoryReceipts};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use url::Url;

use crate::testing::{self, FakeApprovals};
use crate::{OperatorConfig, OperatorState, ledger, router};

const OPS: &str = "http://ops.localhost:18300";

struct World {
    ops: OperatorState,
    receipts: Arc<MemoryReceipts>,
    fake: Arc<FakeApprovals>,
    owner: uuid::Uuid,
    site: String,
}

async fn world() -> World {
    world_with(false).await
}

/// A world whose edge enforces holds (`AGENTRIX_HOLDS_ENFORCED=1`) or doesn't.
async fn world_with(holds_enforced: bool) -> World {
    let pool = testing::pool().await;
    let (fake, approvals) = testing::approvals();
    let receipts = Arc::new(MemoryReceipts::default());
    let mut config = OperatorConfig::new(Url::parse(OPS).unwrap());
    config.platform_origin = Some(Url::parse("http://localhost:8080").unwrap());
    config.holds_enforced = holds_enforced;
    let ops = OperatorState::new(config, pool.clone(), approvals)
        .unwrap()
        .with_receipts(receipts.clone());
    let owner = testing::account(&pool, "wanjiru@kilele.example", "Wanjiru Kamau").await;
    let site = testing::seed_live_site(&pool, owner, "test-bakery.example").await;
    World {
        ops,
        receipts,
        fake,
        owner,
        site,
    }
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn json_req(path: &str, cookie: &str, body: Value) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, OPS)
        .header(header::COOKIE, cookie)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn form_req(path: &str, cookie: &str, form: &str) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ORIGIN, OPS)
        .header(header::COOKIE, cookie)
        .body(Body::from(form.to_owned()))
        .unwrap()
}

async fn operator(w: &World, email: &str, name: &str, roles: &[&str]) -> String {
    let id = testing::account(w.ops.pool(), email, name).await;
    testing::grant(w.ops.pool(), id, roles).await;
    let cookie = w.ops.start_session(id, "totp").await.unwrap();
    cookie
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

/// Prepares and approves an action with the fake code; returns the approve response.
async fn act(w: &World, cookie: &str, action: Value) -> (StatusCode, String) {
    let (status, body) = send(
        router(w.ops.clone()),
        json_req("/actions/prepare", cookie, action),
    )
    .await;
    if status != StatusCode::OK {
        return (status, body);
    }
    let prepared: Value = serde_json::from_str(&body).unwrap();
    assert!(
        prepared["effect"]
            .as_str()
            .unwrap()
            .contains("If you approve, exactly this happens")
    );
    send(
        router(w.ops.clone()),
        json_req(
            &format!("/actions/{}/approve", prepared["id"].as_str().unwrap()),
            cookie,
            json!({ "proof": { "method": "totp", "code": "246810" } }),
        ),
    )
    .await
}

fn kinds(r: &MemoryReceipts) -> Vec<String> {
    r.issued
        .lock()
        .unwrap()
        .iter()
        .map(|(_, b)| b.kind.as_str().to_owned())
        .collect()
}

async fn site_page(w: &World, cookie: &str) -> String {
    let req = Request::get(format!("/sites/{}", w.site))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(router(w.ops.clone()), req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        &body[body.len().saturating_sub(800)..]
    );
    body
}

/// Posts a form, checks it comes back to a page (303), follows it, and returns where it went and
/// the page it showed.
async fn submit(w: &World, req: Request<Body>, cookie: &str) -> (String, String) {
    let res = router(w.ops.clone()).oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::SEE_OTHER,
        "a form comes back to a page"
    );
    let to = res
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let path = to.split('#').next().unwrap();
    let (status, body) = send(
        router(w.ops.clone()),
        Request::get(path)
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::NOT_FOUND,
        "{status}"
    );
    (to, body)
}

/// The notice line at the top of a page, if any.
fn notice_of(page: &str) -> Option<&str> {
    let start = page.find("<p class=\"notice ")?;
    let text = &page[start..];
    let open = text.find('>')? + 1;
    let close = text.find("</p>")?;
    Some(&text[open..close])
}

fn the_words(text: &str) -> String {
    text.replace("&#x27;", "'").replace("&quot;", "\"")
}

#[tokio::test]
async fn a_support_grant_needs_the_owner_and_every_look_is_recorded() {
    let w = world().await;
    let support = operator(&w, "juma@zavora.example", "Juma Mwangi", &["op.support"]).await;
    // Before any grant: no content.
    assert!(
        !site_page(&w, &support)
            .await
            .contains("What you can read now")
    );
    let (status, _) = send(router(w.ops.clone()), form_req(&format!("/sites/{}/support-grants", w.site), &support, "area_receipts=on&area_jobs=on&access=read&hours=4&reason=Checkout+fails+for+one+customer")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let page = site_page(&w, &support).await;
    assert!(page.contains("requested") && page.contains("/support-access/grant_"));
    assert!(
        !page.contains("What you can read now"),
        "a request alone opens nothing"
    );

    // The owner answers on the platform host.
    let grant: (String, String) =
        sqlx::query_as("SELECT grant_id, fingerprint FROM support_grants")
            .fetch_one(w.ops.pool())
            .await
            .unwrap();
    let owner_side = crate::owner::OwnerState::new(
        w.ops.pool().clone(),
        w.ops.inner.approvals.clone(),
        Arc::new(FixedOwner::passkey(w.owner)),
        Some(w.receipts.clone()),
        Url::parse(OPS).unwrap(),
    )
    .unwrap();
    let (status, body) = send(
        crate::owner::router(owner_side.clone()),
        Request::get(format!("/support-access/{}", grant.0))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("Let Juma Mwangi read test-bakery.example")
            && body.contains("Never your customers' payment details")
    );
    let approve = Request::post(format!("/support-access/{}/approve", grant.0))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "method": "totp", "code": "246810" }).to_string(),
        ))
        .unwrap();
    let (status, body) = send(crate::owner::router(owner_side.clone()), approve).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let signed = w.fake.signed.lock().unwrap().clone();
    assert_eq!(
        hex::encode(signed.last().unwrap().1),
        grant.1,
        "the owner approved the grant's exact terms"
    );

    // The operator now reads receipts and jobs, and the owner gets one receipt per session.
    let page = site_page(&w, &support).await;
    assert!(page.contains("What you can read now") && page.contains("private words"));
    site_page(&w, &support).await;
    assert_eq!(
        kinds(&w.receipts),
        vec!["decision.approved", "operator.access"]
    );
    let (accesses,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM support_access")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    assert_eq!(accesses, 4, "two visits, two areas each");
    // Another operator isn't named on the grant.
    let other = operator(&w, "achieng@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    assert!(
        !site_page(&w, &other)
            .await
            .contains("What you can read now")
    );
    // The owner ends it early.
    let end = Request::post(format!("/support-access/{}/end", grant.0))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(crate::owner::router(owner_side), end).await.0,
        StatusCode::SEE_OTHER
    );
    assert!(
        !site_page(&w, &support)
            .await
            .contains("What you can read now")
    );
    assert!(ledger::verify(w.ops.pool()).await.unwrap().ok);
}

#[tokio::test]
async fn someone_elses_site_or_no_role_is_refused() {
    let w = world().await;
    let viewer = operator(&w, "audit@zavora.example", "Ann Auditor", &["op.auditor"]).await;
    let (to, page) = submit(
        &w,
        form_req(
            &format!("/sites/{}/support-grants", w.site),
            &viewer,
            "area_receipts=on&access=read&hours=4&reason=x",
        ),
        &viewer,
    )
    .await;
    assert!(to.starts_with(&format!("/sites/{}?n=", w.site)) && to.ends_with("#access"));
    assert!(
        the_words(notice_of(&page).unwrap()).contains("needs op.support, op.sre or op.admin"),
        "{page}"
    );
    assert!(page.contains("<p class=\"notice problem\" role=\"alert\">"));
    let (grants,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM support_grants")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    assert_eq!(grants, 0, "a refused form changes nothing");
    assert_eq!(
        act(
            &w,
            &viewer,
            json!({ "action": "hold", "site": w.site, "mode": "read-only", "reason": "x" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        act(
            &w,
            &viewer,
            json!({ "action": "break-glass", "site": w.site, "reason": "x" })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // A different owner can't answer this site's request.
    let support = operator(&w, "juma@zavora.example", "Juma Mwangi", &["op.support"]).await;
    send(
        router(w.ops.clone()),
        form_req(
            &format!("/sites/{}/support-grants", w.site),
            &support,
            "area_receipts=on&access=read&hours=4&reason=Help",
        ),
    )
    .await;
    let (grant,): (String,) = sqlx::query_as("SELECT grant_id FROM support_grants")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    let stranger = testing::account(w.ops.pool(), "someone@else.example", "Someone Else").await;
    let owner_side = crate::owner::OwnerState::new(
        w.ops.pool().clone(),
        w.ops.inner.approvals.clone(),
        Arc::new(FixedOwner::passkey(stranger)),
        None,
        Url::parse(OPS).unwrap(),
    )
    .unwrap();
    let (status, body) = send(
        crate::owner::router(owner_side.clone()),
        Request::get(format!("/support-access/{grant}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // A page in words, never raw JSON.
    assert!(
        body.starts_with("<!doctype html>")
            && the_words(&body).contains("Only the site's owner can answer this request."),
        "{body}"
    );
    // Their form post gets the same page, and nothing changes.
    let (status, body) = send(
        crate::owner::router(owner_side),
        Request::post(format!("/support-access/{grant}/decline"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.starts_with("<!doctype html>"), "{body}");
    let (state,): (String,) = sqlx::query_as("SELECT state FROM support_grants")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    assert_eq!(state, "requested");
}

#[tokio::test]
async fn break_glass_takes_two_different_people_and_tells_the_owner() {
    let w = world().await;
    let security = operator(&w, "sec@zavora.example", "Sam Security", &["op.security"]).await;
    let director = operator(&w, "dir@zavora.example", "Dana Director", &["director"]).await;
    let (status, body) = act(&w, &security, json!({ "action": "break-glass", "site": w.site, "reason": "Pages leak a token; SEV1", "incident": "INC-0001" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        kinds(&w.receipts).is_empty(),
        "nobody is told until it opens"
    );
    // The first person can't sign twice.
    let (bg,): (String,) = sqlx::query_as("SELECT bg_id FROM break_glass")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    assert_eq!(
        act(
            &w,
            &security,
            json!({ "action": "break-glass-second", "bg_id": bg })
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // The director sees it in the lane and signs.
    let fleet = send(
        router(w.ops.clone()),
        Request::get("/fleet")
            .header(header::COOKIE, &director)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .1;
    assert!(fleet.contains("Open break-glass on test-bakery.example"));
    let (status, body) = act(
        &w,
        &director,
        json!({ "action": "break-glass-second", "bg_id": bg }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(kinds(&w.receipts), vec!["operator.access"]);
    let page = site_page(&w, &director).await;
    assert!(page.contains("What you can read now") && page.contains("break-glass"));
    let (state, second): (String, String) =
        sqlx::query_as("SELECT state, second_role FROM break_glass")
            .fetch_one(w.ops.pool())
            .await
            .unwrap();
    assert_eq!((state.as_str(), second.as_str()), ("open", "director"));
    let (status, _) = send(
        router(w.ops.clone()),
        form_req(&format!("/break-glass/{bg}/close"), &security, ""),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        !site_page(&w, &director)
            .await
            .contains("What you can read now")
    );
}

#[tokio::test]
async fn a_hold_is_placed_receipted_and_lifted() {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let (status, body) = act(&w, &sre, json!({ "action": "hold", "site": w.site, "mode": "read-only", "reason": "Phishing report A-311 under review" })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let hold = crate::holds::active(w.ops.pool(), &w.site)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hold.serve_mode(), crate::holds::Mode::ReadOnly);
    assert!(
        site_page(&w, &sre)
            .await
            .contains("Recorded, not enforced on this build")
    );
    assert_eq!(
        act(
            &w,
            &sre,
            json!({ "action": "hold", "site": w.site, "mode": "notice", "reason": "again" })
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, _) = act(
        &w,
        &sre,
        json!({ "action": "lift", "hold_id": hold.hold_id, "reason": "Report was mistaken" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        crate::holds::active(w.ops.pool(), &w.site)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(kinds(&w.receipts), vec!["platform.hold", "platform.hold"]);
    let verbs: Vec<String> = ledger::recent(w.ops.pool(), 5)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.verb)
        .collect();
    assert_eq!(
        verbs[..2],
        ["hold.lift".to_owned(), "hold.place".to_owned()]
    );
}

#[tokio::test]
async fn a_wrong_code_runs_nothing() {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let (_, body) = send(
        router(w.ops.clone()),
        json_req(
            "/actions/prepare",
            &sre,
            json!({ "action": "hold", "site": w.site, "mode": "read-only", "reason": "x" }),
        ),
    )
    .await;
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _) = send(
        router(w.ops.clone()),
        json_req(
            &format!("/actions/{id}/approve"),
            &sre,
            json!({ "proof": { "method": "totp", "code": "000000" } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        crate::holds::active(w.ops.pool(), &w.site)
            .await
            .unwrap()
            .is_none()
    );
    // The prepared action is gone: a retry has to start again.
    let (status, _) = send(
        router(w.ops.clone()),
        json_req(
            &format!("/actions/{id}/approve"),
            &sre,
            json!({ "proof": { "method": "totp", "code": "246810" } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn money_shows_real_subscriptions_and_only_op_finance_issues_the_run() {
    let w = world().await;
    let billing = agentrix_billing::Billing::new(w.ops.pool().clone());
    billing
        .start(&w.site, "business", "I-TEST", None)
        .await
        .unwrap();
    let now = chrono::Utc::now();
    billing
        .on_event(&agentrix_billing::SubscriptionEvent {
            provider_ref: "I-TEST".into(),
            kind: agentrix_billing::EventKind::Activated,
            amount_cents: None,
            provider_payment_id: None,
            at: now,
            period_end: None,
            receipt: None,
        })
        .await
        .unwrap();
    billing.went_live(&w.site, now).await.unwrap();

    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let page = send(
        router(w.ops.clone()),
        Request::get("/money")
            .header(header::COOKIE, &sre)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        page.0,
        StatusCode::OK,
        "{}",
        &page.1[page.1.len().saturating_sub(800)..]
    );
    for needle in [
        "Money · ",
        "$79.00",
        "Business",
        "Invoice run drafted · 1",
        "Only someone with op.finance can approve this run",
        "PayPal Subscriptions · sandbox",
        "not imported",
    ] {
        assert!(page.1.contains(needle), "money lacks {needle:?}");
    }
    let run = billing
        .draft_run(&agentrix_billing::period_of(now))
        .await
        .unwrap();
    let action = json!({ "action": "invoice-run", "period": run.period, "digest": run.digest });
    assert_eq!(act(&w, &sre, action.clone()).await.0, StatusCode::FORBIDDEN);

    let finance = operator(&w, "fin@zavora.example", "Wairimu Njeri", &["op.finance"]).await;
    let stale = json!({ "action": "invoice-run", "period": run.period, "digest": "sha256:old" });
    assert_eq!(act(&w, &finance, stale).await.0, StatusCode::CONFLICT);
    let (status, body) = act(&w, &finance, action).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let m = billing.money(now).await.unwrap();
    assert_eq!((m.outstanding_invoices, m.outstanding_cents), (1, 7900));
    let entry = ledger::recent(w.ops.pool(), 1).await.unwrap().remove(0);
    assert_eq!(entry.verb, "money.invoice_run");
    assert_eq!(
        entry.summary,
        format!(
            "Wairimu Njeri issued 1 invoice for {}, $79.00 in all.",
            run.period
        )
    );
}

#[tokio::test]
async fn an_incident_is_declared_worked_resolved_and_written_up() {
    let w = world().await;
    let auditor = operator(&w, "audit@zavora.example", "Ann Auditor", &["op.auditor"]).await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let declare = "severity=sev2&title=Ask+answers+slow+on+one+site&summary=Owners+report+slow+answers&site=test-bakery.example";
    let (to, page) = submit(&w, form_req("/incidents", &auditor, declare), &auditor).await;
    assert!(to.starts_with("/incidents?n="));
    assert!(
        notice_of(&page)
            .unwrap()
            .starts_with("Nothing was declared")
    );
    let (to, page) = submit(&w, form_req("/incidents", &sre, declare), &sre).await;
    assert!(to.starts_with("/incidents/INC-0001?n="), "{to}");
    assert_eq!(
        the_words(notice_of(&page).unwrap()),
        "You declared INC-0001, SEV2, and you're its commander. Add what you see to the timeline as you go."
    );
    let (id, site): (String, Option<String>) =
        sqlx::query_as("SELECT incident_id, site_id FROM incidents")
            .fetch_one(w.ops.pool())
            .await
            .unwrap();
    assert_eq!(
        (id.as_str(), site.as_deref()),
        ("INC-0001", Some(w.site.as_str()))
    );
    for (kind, words) in [
        (
            "evidence",
            "Gateway calls to concierge failed 4 of 31 in 10 minutes",
        ),
        ("comms", "Some answers were slow between 07:00 and 07:20."),
    ] {
        let form = format!(
            "kind={kind}&words={}",
            url::form_urlencoded::byte_serialize(words.as_bytes()).collect::<String>()
        );
        assert_eq!(
            send(
                router(w.ops.clone()),
                form_req(&format!("/incidents/{id}/events"), &sre, &form)
            )
            .await
            .0,
            StatusCode::SEE_OTHER
        );
    }
    for state in ["mitigated", "resolved"] {
        assert_eq!(
            send(
                router(w.ops.clone()),
                form_req(
                    &format!("/incidents/{id}/status"),
                    &sre,
                    &format!("state={state}")
                )
            )
            .await
            .0,
            StatusCode::SEE_OTHER
        );
    }
    assert_eq!(
        send(
            router(w.ops.clone()),
            form_req(
                &format!("/incidents/{id}/postmortem"),
                &sre,
                "words=The+provider+throttled+us.+We+now+alert+on+errors."
            )
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    let page = send(
        router(w.ops.clone()),
        Request::get(format!("/incidents/{id}"))
            .header(header::COOKIE, &sre)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        page.0,
        StatusCode::OK,
        "{}",
        &page.1[page.1.len().saturating_sub(800)..]
    );
    for needle in [
        "Ask answers slow on one site",
        "SEV2",
        "Resolved",
        "Gateway calls to concierge failed",
        "Draft to send",
        "The provider throttled us",
        "test-bakery.example",
    ] {
        assert!(page.1.contains(needle), "incident page lacks {needle:?}");
    }
    let list = send(
        router(w.ops.clone()),
        Request::get("/incidents")
            .header(header::COOKIE, &sre)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(
        list.1.contains("Nothing open")
            && list.1.contains("INC-0001")
            && list.1.contains("written"),
        "{}",
        list.1
    );
    let verbs: Vec<String> = ledger::recent(w.ops.pool(), 3)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.verb)
        .collect();
    assert_eq!(
        verbs,
        ["incident.resolved", "incident.mitigated", "incident.open"]
    );
    assert_eq!(
        send(
            router(w.ops.clone()),
            Request::get("/incidents/INC-9999")
                .header(header::COOKIE, &sre)
                .body(Body::empty())
                .unwrap()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn rollouts_show_rings_and_the_route_changes_sites_were_told_about() {
    let w = world().await;
    sqlx::query(
        "INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, started_at, ended_at, issued_at, body) \
         VALUES ('sha256:rt', 'rcpt_rt', ?, 'routing.changed', 'site', ?, 'k', ?1, ?1, ?3, '{\"detail\":{\"title\":\"Models moved from anthropic to vultr\",\"to\":{\"provider\":\"vultr\"}}}')",
    )
    .bind(&w.site)
    .bind(&w.site)
    .bind(ledger::now())
    .execute(w.ops.pool())
    .await
    .unwrap();
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let page = send(
        router(w.ops.clone()),
        Request::get("/rollouts")
            .header(header::COOKIE, &sre)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        page.0,
        StatusCode::OK,
        "{}",
        &page.1[page.1.len().saturating_sub(800)..]
    );
    for needle in [
        "No train in flight",
        "Ring 0",
        "1 of 1 customer site",
        "Models moved from anthropic to vultr",
        "1 site told",
        "1 of 1 runs held every attack",
    ] {
        assert!(page.1.contains(needle), "rollouts lacks {needle:?}");
    }
    // The seeded release was signed 84 minutes ago: it counts on that day in EAT.
    let eat = chrono::FixedOffset::east_opt(180 * 60).unwrap();
    let day = (chrono::Utc::now() - chrono::Duration::minutes(84))
        .with_timezone(&eat)
        .format("%a %-d %b")
        .to_string();
    assert!(
        page.1.contains(&day) && page.1.contains("Site releases signed, last 7 days"),
        "release day {day:?} in the operator's zone"
    );
}

#[tokio::test]
async fn a_support_request_shows_in_the_owners_lane_and_nobody_elses() {
    let w = world().await;
    let support = operator(&w, "juma@zavora.example", "Juma Mwangi", &["op.support"]).await;
    send(
        router(w.ops.clone()),
        form_req(
            &format!("/sites/{}/support-grants", w.site),
            &support,
            "area_receipts=on&access=read&hours=2&reason=Checkout+fails+for+one+customer",
        ),
    )
    .await;
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    ctx.pool = w.ops.pool().clone();
    let lane = crate::support_lane(&ctx);
    let app = |account: String| agentrix_shell::apps::AppContext {
        site_id: w.site.clone(),
        site_name: "Test Bakery".into(),
        host: "test-bakery.example".into(),
        sample: false,
        account,
        owner_name: "Wanjiru".into(),
        utc_offset_minutes: 180,
        base: String::new(),
        console: String::new(),
    };
    let items = lane.needs_you(&app(w.owner.to_string())).await.unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(
        item.title,
        "Zavora support asks to read your site for 2 hours"
    );
    assert_eq!(
        (item.tone.as_str(), item.chip.as_str()),
        ("wait", "Read only")
    );
    assert_eq!(item.summary, "Checkout fails for one customer");
    assert!(
        item.href.starts_with("/support-access/grant_") && item.when_words.starts_with("Asked ")
    );
    assert!(
        lane.needs_you(&app(uuid::Uuid::now_v7().to_string()))
            .await
            .unwrap()
            .is_empty(),
        "only the owner sees it"
    );
}

#[tokio::test]
async fn a_partner_gets_a_client_only_when_the_owner_grants_it_and_its_people_see_only_their_portfolio()
 {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let partners = operator(&w, "ps@zavora.example", "Pat Partners", &["op.partners"]).await;
    let (to, page) = submit(
        &w,
        form_req("/partners", &sre, "name=Studio+Mwangaza&kind=agency"),
        &sre,
    )
    .await;
    assert!(to.starts_with("/partners?n="));
    assert_eq!(
        notice_of(&page).unwrap(),
        "No partner was added: adding one needs op.partners or op.admin."
    );
    let (none,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM partners")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    assert_eq!(none, 0);
    assert_eq!(
        send(
            router(w.ops.clone()),
            form_req("/partners", &partners, "name=Studio+Mwangaza&kind=agency")
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    let (pt,): (String,) = sqlx::query_as("SELECT partner_id FROM partners")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    let invite = send(
        router(w.ops.clone()),
        form_req(
            &format!("/partners/{pt}/clients"),
            &partners,
            "site=test-bakery.example&role=editor&ring=canary",
        ),
    )
    .await;
    assert_eq!(invite.0, StatusCode::SEE_OTHER, "{}", invite.1);
    // A person at the agency.
    let muthoni =
        testing::account(w.ops.pool(), "muthoni@mwangaza.example", "Muthoni Wairimu").await;
    assert_eq!(
        send(
            router(w.ops.clone()),
            form_req(
                &format!("/partners/{pt}/members"),
                &partners,
                "email=muthoni%40mwangaza.example&role=partner.manager"
            )
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );

    // Before the owner answers: invited, not a client yet; the owner's lane shows the ask.
    let page = send(
        router(w.ops.clone()),
        Request::get(format!("/partners/{pt}"))
            .header(header::COOKIE, &partners)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        page.0,
        StatusCode::OK,
        "{}",
        &page.1[page.1.len().saturating_sub(600)..]
    );
    assert!(page.1.contains("Waiting for the owner's grant") && page.1.contains("Muthoni Wairimu"));
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    ctx.pool = w.ops.pool().clone();
    let app = agentrix_shell::apps::AppContext {
        site_id: w.site.clone(),
        site_name: "Test Bakery".into(),
        host: "test-bakery.example".into(),
        sample: false,
        account: w.owner.to_string(),
        owner_name: "Wanjiru".into(),
        utc_offset_minutes: 180,
        base: String::new(),
        console: String::new(),
    };
    let items = crate::support_lane(&ctx).needs_you(&app).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].title,
        "Studio Mwangaza asks to be your site's editor"
    );

    // The owner grants on the platform host, approving exactly this partner, site and role.
    let owner_side = crate::owner::OwnerState::new(
        w.ops.pool().clone(),
        w.ops.inner.approvals.clone(),
        Arc::new(FixedOwner::passkey(w.owner)),
        Some(w.receipts.clone()),
        Url::parse(OPS).unwrap(),
    )
    .unwrap();
    let (status, body) = send(
        crate::owner::router(owner_side.clone()),
        Request::get(format!("/agency-access/{pt}/{}", w.site))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Make Studio Mwangaza your site's editor"));
    let approve = Request::post(format!("/agency-access/{pt}/{}/approve", w.site))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "method": "totp", "code": "246810" }).to_string(),
        ))
        .unwrap();
    assert_eq!(
        send(crate::owner::router(owner_side.clone()), approve)
            .await
            .0,
        StatusCode::OK
    );
    let signed = w.fake.signed.lock().unwrap().clone();
    assert_eq!(
        signed.last().unwrap().1,
        crate::partners::grant_fingerprint(&pt, &w.site, "editor")
    );
    assert!(kinds(&w.receipts).contains(&"decision.approved".to_owned()));

    // Muthoni signs in and lands on her portfolio, in the portfolio scope; the fleet stays closed.
    let cookie = w.ops.start_session(muthoni, "passkey").await.unwrap();
    let cookie = cookie
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let mine = send(
        router(w.ops.clone()),
        Request::get("/portfolio")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        mine.0,
        StatusCode::OK,
        "{}",
        &mine.1[mine.1.len().saturating_sub(600)..]
    );
    for needle in [
        "Portfolio · on Agentrix",
        "Studio Mwangaza · Clients",
        "Test Bakery",
        "Live · v1",
        "Editor",
        "canary",
        "partner.manager",
    ] {
        assert!(mine.1.contains(needle), "portfolio lacks {needle:?}");
    }
    assert!(!mine.1.contains("Operator · Zavora Cloud"));
    assert_eq!(
        send(
            router(w.ops.clone()),
            Request::get("/fleet")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap()
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        send(
            router(w.ops.clone()),
            Request::get(format!("/sites/{}", w.site))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap()
        )
        .await
        .0,
        StatusCode::SEE_OTHER
    );

    // The owner takes it back: the portfolio drops the client.
    let revoke = Request::post(format!("/agency-access/{pt}/{}/revoke", w.site))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(crate::owner::router(owner_side), revoke).await.0,
        StatusCode::SEE_OTHER
    );
    let mine = send(
        router(w.ops.clone()),
        Request::get("/portfolio")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(mine.1.contains("No clients yet"));
    assert!(ledger::verify(w.ops.pool()).await.unwrap().ok);
}

#[tokio::test]
async fn sites_carry_their_origin_after_0340() {
    let w = world().await;
    let pool = w.ops.pool();
    sqlx::query("INSERT INTO sites (site_id, domain, old_origin, fixture, created_at) VALUES ('s_fx', 'fixture.zavora.example', 'x', 1, 'x'), ('s_sm', 'kilele-bakery-sample.agentrix.website', 'x', 0, 'x')")
        .execute(pool)
        .await
        .unwrap();
    // 0340 ran before these rows existed in tests, so run its backfill again the way a deploy would.
    let sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../agentrix-node/src/db/migrations/0340_registry_columns.sql"
    ))
    .unwrap();
    let backfill = sql
        .split("UPDATE sites")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("UPDATE sites{backfill};")))
        .execute(pool)
        .await
        .unwrap();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT site_id, origin FROM sites ORDER BY site_id")
            .fetch_all(pool)
            .await
            .unwrap();
    let origin = |id: &str| {
        rows.iter()
            .find(|(s, _)| s == id)
            .map(|(_, o)| o.clone())
            .unwrap()
    };
    assert_eq!(
        (origin("s_fx"), origin("s_sm"), origin(&w.site)),
        ("fixture".into(), "sample".into(), "real".into())
    );
}

fn get_as(path: &str, cookie: &str) -> Request<Body> {
    Request::get(path)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn last_receipt_title(r: &MemoryReceipts) -> String {
    let issued = r.issued.lock().unwrap();
    let (_, body) = issued.last().expect("a receipt");
    body.detail.as_ref().unwrap()["title"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn forms_come_back_to_their_page_with_plain_words_once() {
    let w = world().await;
    let support = operator(&w, "juma@zavora.example", "Juma Mwangi", &["op.support"]).await;
    let ask = format!("/sites/{}/support-grants", w.site);
    // Done: the site page says what was asked, once.
    let (to, page) = submit(
        &w,
        form_req(
            &ask,
            &support,
            "area_receipts=on&access=read&hours=1&reason=Checkout+fails",
        ),
        &support,
    )
    .await;
    assert_eq!(
        the_words(notice_of(&page).unwrap()),
        "You asked the owner of test-bakery.example for read access to receipts for 1 hour. Nothing opens until they approve it."
    );
    assert!(page.contains("<p class=\"notice done\" role=\"status\">"));
    // One hour reads "1 hour" on the page and in the ledger.
    assert!(page.contains("for 1 hour<div") && !page.contains("1 hours"));
    let entry = ledger::recent(w.ops.pool(), 1).await.unwrap().remove(0);
    assert!(entry.summary.ends_with("for 1 hour."), "{}", entry.summary);
    // Shown once: the same address again has no notice, and a made-up id shows nothing.
    let (_, again) = send(
        router(w.ops.clone()),
        get_as(to.split('#').next().unwrap(), &support),
    )
    .await;
    assert!(notice_of(&again).is_none());
    let (_, made_up) = send(
        router(w.ops.clone()),
        get_as(&format!("/sites/{}?n=0123abcd", w.site), &support),
    )
    .await;
    assert!(notice_of(&made_up).is_none());
    // A form that doesn't pass says what to do, on the same page.
    let (_, page) = submit(
        &w,
        form_req(&ask, &support, "access=read&hours=99&reason=x"),
        &support,
    )
    .await;
    assert!(the_words(notice_of(&page).unwrap()).starts_with("The owner wasn't asked."));
    // Withdrawing says so.
    let (grant,): (String,) = sqlx::query_as("SELECT grant_id FROM support_grants")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    let (_, page) = submit(
        &w,
        form_req(&format!("/support-grants/{grant}/end"), &support, ""),
        &support,
    )
    .await;
    assert_eq!(
        notice_of(&page).unwrap(),
        "You withdrew the request. The owner can no longer approve it."
    );
    // Signed out: sign in, then back to the form's page.
    let res = router(w.ops.clone())
        .oneshot(form_req(
            &ask,
            "agx_ops=expired",
            "area_receipts=on&access=read&hours=1&reason=x",
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        res.headers().get(header::LOCATION).unwrap(),
        &format!("/signin?next=%2Fsites%2F{}", w.site)
    );
    // From another site: refused in words, and nothing changes.
    let (status, body) = send(
        router(w.ops.clone()),
        Request::post(&ask)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::ORIGIN, "https://elsewhere.example")
            .header(header::COOKIE, &support)
            .body(Body::from("area_receipts=on&access=read&hours=1&reason=x"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("This form came from another site"));
    let (grants,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM support_grants")
        .fetch_one(w.ops.pool())
        .await
        .unwrap();
    assert_eq!(grants, 1);
}

#[tokio::test]
async fn holds_say_what_this_build_does_and_the_holds_page_lists_them() {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let (_, page) = send(router(w.ops.clone()), get_as("/holds", &sre)).await;
    assert!(page.contains("No site is on platform hold.") && page.contains("0 holds in force"));
    let (status, body) = act(
        &w,
        &sre,
        json!({ "action": "hold", "site": w.site, "mode": "notice", "reason": "Phishing report A-311" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Not enforced on this build: the owner's receipt says so and promises no notice page.
    let title = last_receipt_title(&w.receipts);
    assert!(
        title.contains(
            "for now Agentrix only records holds, so visitors still see your site as usual"
        ) && !title.contains("neutral notice page"),
        "{title}"
    );
    // The approval comes back to the site page with what happened.
    let next = serde_json::from_str::<Value>(&body).unwrap()["next"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(next.starts_with(&format!("/sites/{}?n=", w.site)), "{next}");
    let (_, page) = send(
        router(w.ops.clone()),
        get_as(next.split('#').next().unwrap(), &sre),
    )
    .await;
    assert_eq!(
        the_words(notice_of(&page).unwrap()),
        "The hold on test-bakery.example is recorded and the owner has a receipt. The edge doesn't enforce holds on this build, so visitors still see the site as usual."
    );
    // /holds lists it from the store.
    let (status, page) = send(router(w.ops.clone()), get_as("/holds", &sre)).await;
    assert_eq!(status, StatusCode::OK);
    for needle in [
        "1 hold in force",
        "test-bakery.example",
        "Phishing report A-311",
        "notice page",
        "Recorded, not enforced",
        "Placed by Achieng Otieno",
    ] {
        assert!(page.contains(needle), "/holds lacks {needle:?}");
    }
    assert!(!page.contains("No site is on platform hold."));
    // Lifted, it moves to "Lifted lately", and the owner hears nothing changes for visitors.
    let hold = crate::holds::active(w.ops.pool(), &w.site)
        .await
        .unwrap()
        .unwrap();
    let (status, _) = act(
        &w,
        &sre,
        json!({ "action": "lift", "hold_id": hold.hold_id, "reason": "Report was mistaken" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        last_receipt_title(&w.receipts)
            .contains("Agentrix only recorded it, so nothing changes for visitors")
    );
    let (_, page) = send(router(w.ops.clone()), get_as("/holds", &sre)).await;
    assert!(page.contains("0 holds in force") && page.contains("Report was mistaken"));
}

#[tokio::test]
async fn an_enforced_hold_tells_the_owner_what_visitors_see() {
    let w = world_with(true).await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let (status, body) = act(
        &w,
        &sre,
        json!({ "action": "hold", "site": w.site, "mode": "notice", "reason": "Malware on the old site" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let title = last_receipt_title(&w.receipts);
    assert!(
        title.contains("Visitors see a neutral notice page instead of your site."),
        "{title}"
    );
    let (_, page) = send(router(w.ops.clone()), get_as("/holds", &sre)).await;
    assert!(page.contains("Enforced at the edge"));
}

#[tokio::test]
async fn the_overview_shows_the_three_scopes_from_the_records() {
    let w = world().await;
    let me = operator(&w, "achieng@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let now = ledger::now();
    sqlx::query("INSERT INTO partners (partner_id, kind, name, created_by, created_at) VALUES ('pt_1', 'agency', 'Studio Mwangaza', 'test', ?)")
        .bind(&now)
        .execute(w.ops.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO partner_clients (partner_id, site_id, charter_role, ring, state, fingerprint, invited_by, invited_at) VALUES ('pt_1', ?, 'editor', 'canary', 'granted', 'f', 'test', ?)")
        .bind(&w.site)
        .bind(&now)
        .execute(w.ops.pool())
        .await
        .unwrap();
    submit(
        &w,
        form_req(
            "/incidents",
            &me,
            "severity=sev2&title=Answers+slow&summary=&site=",
        ),
        &me,
    )
    .await;
    let (status, page) = send(router(w.ops.clone()), get_as("/", &me)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        &page[page.len().saturating_sub(800)..]
    );
    for needle in [
        "Zavora Cloud platform",
        "<b>1</b><span>site live",
        "1 claimed of 1 address assessed",
        "<b>1</b><span>incident open, worst SEV2",
        "<b>$0.40</b><span>model spend",
        "no paying subscription yet",
        "Partners' portfolios",
        "Studio Mwangaza",
        "1 client · 1 live",
        "1 decision waiting on owners · $0.40 AI spend this month",
        "What waits for you, Achieng",
        "INC-0001",
        "Answers slow",
    ] {
        assert!(page.contains(needle), "the overview lacks {needle:?}");
    }
    // The scope chip leads back here from every page.
    assert!(
        page.contains("href=\"/\" title=\"Overview of the three scopes\" aria-current=\"page\"")
    );
    // Signed out, / asks for sign-in and comes back.
    let (status, _) = send(router(w.ops.clone()), get_as("/", "agx_ops=expired")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn the_portfolio_computes_ai_above_plans_and_its_dock_reaches_the_ring() {
    let w = world().await;
    let pool = w.ops.pool();
    let partner = crate::partners::Partner {
        partner_id: "pt_1".into(),
        kind: "agency".into(),
        name: "Studio Mwangaza".into(),
        console_host: None,
        email_sender: None,
        created_at: ledger::now(),
    };
    sqlx::query("INSERT INTO partner_clients (partner_id, site_id, charter_role, ring, state, fingerprint, invited_by, invited_at) VALUES ('pt_1', ?, 'editor', 'canary', 'granted', 'f', 'test', ?)")
        .bind(&w.site)
        .bind(ledger::now())
        .execute(pool)
        .await
        .unwrap();
    // No plan: nothing to rest an overage on.
    let p = crate::partners::portfolio(pool, partner.clone(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!((p.clients_with_plan, p.ai_over_micro_usd), (0, 0));
    assert_eq!((p.ring_canary, p.ring_stable), (1, 0));
    // On a plan whose included AI is below this month's $0.40, the overage is the difference.
    let plan = agentrix_billing::catalogue()
        .plans
        .iter()
        .find(|p| p.included_ai_micro_usd > 0)
        .expect("a plan with included AI");
    let spend = 400_000_i64;
    let extra = plan.included_ai_micro_usd + 1_000_000 - spend;
    sqlx::query("INSERT INTO usage (site_id, purpose, provider, model, tokens_in, tokens_out, cost_micro_usd, outcome, at) VALUES (?, 'concierge', 'vultr', 'm', 1, 1, ?, 'ok', ?)")
        .bind(&w.site)
        .bind(extra)
        .bind(ledger::now())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO subscriptions (subscription_id, site_id, plan_id, plan_version, price_cents, state, created_at, updated_at) VALUES ('sub_1', ?, ?, 1, 0, 'active', ?, ?)")
        .bind(&w.site)
        .bind(&plan.id)
        .bind(ledger::now())
        .bind(ledger::now())
        .execute(pool)
        .await
        .unwrap();
    let p = crate::partners::portfolio(pool, partner, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!((p.clients_with_plan, p.ai_over_micro_usd), (1, 1_000_000));
    // The portfolio dock: Sites to the Moves card, Rollouts to the ring card, which exists.
    let render = crate::render::Render::new(180).unwrap();
    let html = render
        .page(
            "portfolio.html",
            &json!({
                "shell": { "scope": "pf", "app": "clients", "partner": { "name": "Studio Mwangaza", "kind": "agency" },
                    "operator": { "name": "Muthoni", "email": "m@example.test", "method": "totp" },
                    "roles": ["partner.manager"], "waiting": 0, "lane": [], "open_incidents": 0 },
                "p": p, "as_operator": false, "can_manage": false, "owner_link": null, "kept": {},
            }),
        )
        .unwrap();
    assert!(
        html.contains("href=\"/portfolio#import\"") && html.contains("href=\"/portfolio#ring\"")
    );
    assert!(html.contains("id=\"ring\"") && html.contains("$1.00"));
    assert!(!html.contains("$0.00</span>"));
}

#[tokio::test]
async fn the_fleet_reads_the_gateway_health_checks_and_money_shows_vultr_calls() {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    // A database without agentrix-store's table says so rather than showing a number.
    let (status, page) = send(router(w.ops.clone()), get_as("/fleet", &sre)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("not recorded here") && page.contains("Not measured"));
    // The table as node migrates it, with one hour of the status page's checks.
    let sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../agentrix-store/migrations/control/0802_health_hours.sql"
    ))
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(w.ops.pool())
        .await
        .unwrap();
    let hour = (chrono::Utc::now() - chrono::Duration::hours(2))
        .format("%Y-%m-%dT%H")
        .to_string();
    sqlx::query("INSERT INTO health_hours (check_id, hour, ok_minutes, slow_minutes, down_minutes, last_state, last_at) VALUES ('ask', ?, 50, 8, 2, 'ok', ?), ('websites', ?, 60, 0, 0, 'ok', ?)")
        .bind(&hour)
        .bind(ledger::now())
        .bind(&hour)
        .bind(ledger::now())
        .execute(w.ops.pool())
        .await
        .unwrap();
    let (_, page) = send(router(w.ops.clone()), get_as("/fleet", &sre)).await;
    for needle in [
        "<td class=\"num\">96.66%</td>",
        "down 2 minutes",
        "its health check was slow 8 of 60 minutes this week",
        "30 d · 60 minutes",
    ] {
        assert!(page.contains(needle), "the fleet lacks {needle:?}");
    }
    // Edge availability stays unmeasured: the websites check counts releases, it loads no page.
    assert!(page.contains("Edge availability · 30 d</div><div class=\"value none\">Not measured"));
    // Money: the gateway's own Vultr calls beside each site, and the bill still not imported.
    let (status, page) = send(router(w.ops.clone()), get_as("/money", &sre)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        &page[page.len().saturating_sub(600)..]
    );
    assert!(page.contains("Sent to Vultr"));
    assert!(
        the_words(&page).contains(
            "The gateway says it sent 3 calls to Vultr Inference this month, $0.15 at catalogue prices."
        ),
        "money lacks the Vultr line"
    );
}

#[tokio::test]
async fn a_refused_form_keeps_what_was_typed() {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    let (_, page) = submit(
        &w,
        form_req(
            "/incidents",
            &sre,
            "severity=sev2&title=Answers+slow+on+one+site&summary=Owners+say+so&site=nope.example",
        ),
        &sre,
    )
    .await;
    assert!(
        notice_of(&page)
            .unwrap()
            .starts_with("Nothing was declared: no site has")
    );
    assert!(
        page.contains("value=\"Answers slow on one site\""),
        "the title is kept"
    );
    assert!(page.contains(">Owners say so</textarea>") && page.contains("value=\"nope.example\""));
    // Shown once: the next visit starts empty.
    let (_, page) = send(router(w.ops.clone()), get_as("/incidents", &sre)).await;
    assert!(!page.contains("Answers slow on one site"));
}

#[tokio::test]
async fn an_incident_shows_the_gateway_checks_around_it() {
    let w = world().await;
    let sre = operator(&w, "sre@zavora.example", "Achieng Otieno", &["op.sre"]).await;
    submit(
        &w,
        form_req(
            "/incidents",
            &sre,
            "severity=sev3&title=Slow+answers&summary=&site=",
        ),
        &sre,
    )
    .await;
    let (_, page) = send(router(w.ops.clone()), get_as("/incidents/INC-0001", &sre)).await;
    assert!(page.contains("doesn't record the status page's health checks"));
    let sql = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../agentrix-store/migrations/control/0802_health_hours.sql"
    ))
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(w.ops.pool())
        .await
        .unwrap();
    let (_, page) = send(router(w.ops.clone()), get_as("/incidents/INC-0001", &sre)).await;
    assert!(page.contains("No health checks ran around this incident"));
    sqlx::query("INSERT INTO health_hours (check_id, hour, ok_minutes, slow_minutes, down_minutes, last_state, last_at) VALUES ('ask', ?, 50, 8, 2, 'down', ?)")
        .bind(chrono::Utc::now().format("%Y-%m-%dT%H").to_string())
        .bind(ledger::now())
        .execute(w.ops.pool())
        .await
        .unwrap();
    let (_, page) = send(router(w.ops.clone()), get_as("/incidents/INC-0001", &sre)).await;
    assert!(
        page.contains("2 down · 8 slow of 60 min"),
        "the incident lacks the hour"
    );
}
