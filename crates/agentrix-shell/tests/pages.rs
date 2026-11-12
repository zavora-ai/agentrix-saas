//! Every shell page renders in strict mode on sample data, and the shell's own actions (Freeze,
//! unfreeze, quiet hours, registered apps) do what they say.

use std::sync::Arc;

use agentrix_shell::apps::{AppContext, AppPage, AppSpec, ConsoleApp};
use agentrix_shell::sample::{SITE, SamplePort};
use agentrix_shell::{Shell, ShellState, router, store::ShellStore};
use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

struct Orders;

#[async_trait]
impl ConsoleApp for Orders {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "orders".into(),
            name: "Orders".into(),
            title: "Orders: what customers bought".into(),
            icon: "orders".into(),
            webagent: Some("Cashier".into()),
            order: 10,
        }
    }
    async fn badge(&self, _ctx: &AppContext) -> anyhow::Result<u32> {
        Ok(2)
    }
    async fn page(&self, ctx: &AppContext, path: &str, _query: &str) -> anyhow::Result<AppPage> {
        Ok(match path {
            "" => AppPage::Field {
                title: "Orders".into(),
                html: format!(
                    "<div class=\"field-head\"><h2>Orders for {}</h2></div>",
                    ctx.site_name
                ),
            },
            _ => AppPage::NotFound,
        })
    }
}

struct Channels;

#[async_trait]
impl ConsoleApp for Channels {
    fn spec(&self) -> AppSpec {
        AppSpec {
            id: "channels".into(),
            name: "Channels".into(),
            title: "Chat and email".into(),
            icon: "bell".into(),
            webagent: None,
            order: 20,
        }
    }
    async fn page(&self, _ctx: &AppContext, _path: &str, _query: &str) -> anyhow::Result<AppPage> {
        Ok(AppPage::NotFound)
    }
}

async fn make(port: SamplePort) -> (Router, ShellStore) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool");
    let store = ShellStore::open(pool).await.expect("schema");
    let state = ShellState {
        shell: Arc::new(Shell::new().expect("templates")),
        port: Arc::new(port),
        store: store.clone(),
        apps: vec![Arc::new(Orders)],
        lanes: Vec::new(),
        freeze_enabled: true,
    };
    (router(state), store)
}

async fn get(app: &Router, path: &str) -> (StatusCode, String, Option<String>) {
    let res = app
        .clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    let body =
        String::from_utf8(to_bytes(res.into_body(), 1 << 22).await.unwrap().to_vec()).unwrap();
    (status, body, location)
}

async fn post(
    app: &Router,
    path: &str,
    content_type: &str,
    body: &str,
) -> (StatusCode, String, Option<String>) {
    let req = Request::post(path)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::ACCEPT,
            if content_type.contains("json") {
                "application/json"
            } else {
                "text/html"
            },
        )
        .body(Body::from(body.to_owned()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    let body =
        String::from_utf8(to_bytes(res.into_body(), 1 << 22).await.unwrap().to_vec()).unwrap();
    (status, body, location)
}

const FORM: &str = "application/x-www-form-urlencoded";

#[tokio::test]
async fn every_page_renders() {
    let (app, _) = make(SamplePort::default()).await;
    let base = format!("/console/{SITE}");
    let (s, html, _) = get(&app, &base).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    for want in [
        "Chief WebAgent · your brief",
        "Approve cake order · KSh 4,500",
        "Overnight: 41 visitors",
        "Do you make gluten-free cakes?",
        "Sample",
        "Front of house",
        "Night Time",
        "Freeze",
        "Orders",
        "10 of 10",
    ] {
        assert!(html.contains(want), "home lacks {want:?}");
    }
    // No desk has sent Live a conversation: it stays out of the dock, the phone's tabs and the
    // command bar, and the phone's third tab is Orders.
    assert!(
        !html.contains(&format!("{base}/live\"")),
        "Live is offered before it's connected"
    );
    assert!(html.contains(&format!("<a href=\"{base}/orders\">")));
    for (path, want) in [
        ("/decisions", "Decisions"),
        (
            "/decisions/dec_order_118",
            "If you approve, exactly this happens",
        ),
        ("/decisions/dec_golive_1", "Approved with your passkey"),
        (
            "/decisions/dec_golive_1",
            "address record doesn't change. We don't edit your DNS",
        ),
        (
            "/decisions/dec_golive_1",
            "Your WebAgents start work: the Mover, the Editor and the Guardian.",
        ),
        ("/decisions/dec_golive_1", "Going live charges nothing."),
        ("/webagents", "Never:"),
        ("/trust", "Test attacks"),
        ("/quarantine", "Ask for it back"),
        ("/settings", "Quiet hours"),
        ("/website", "Put back the previous release"),
        ("/store", "new roles on your team"),
        ("/memory", "spends money or publishes always asks you first"),
        ("/gestures", "Six gestures, each with a key"),
        ("/missions", "Write the contract"),
        ("/live", "Live isn't connected to your desks yet"),
        ("/walk", "Walk your site"),
        ("/apps", "Your WebAgents"),
        ("/settings", "Where you can decide"),
        ("/webagents/editor", "<wa-workshop"),
        ("/store?add=sales", "Add Sales"),
        ("/apps/orders", "Orders for Kilele Bakery"),
    ] {
        let (s, html, _) = get(&app, &format!("{base}{path}")).await;
        assert_eq!(s, StatusCode::OK, "{path}: {html}");
        assert!(html.contains(want), "{path} lacks {want:?}");
        assert!(html.contains("class=\"os"), "{path} isn't inside the shell");
    }
    let (s, _, _) = get(&app, &format!("{base}/decisions/nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, &format!("{base}/apps/nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_mission_is_a_contract_card_first() {
    let (app, store) = make(SamplePort::default()).await;
    let base = format!("/console/{SITE}");
    let ends = (chrono::Utc::now() + chrono::Duration::days(20))
        .format("%Y-%m-%d")
        .to_string();
    let (s, body, _) = post(
        &app,
        &format!("{base}/missions"),
        "application/x-www-form-urlencoded",
        "goal=",
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let (s, body, to) = post(
        &app,
        &format!("{base}/missions"),
        "application/x-www-form-urlencoded",
        &format!("goal=60+Christmas+cake+orders&measure=questions&target=60&member=concierge&member=editor&member=sales&ends={ends}"),
    )
    .await;
    assert_eq!(s, StatusCode::SEE_OTHER, "{body}");
    let to = to.unwrap();
    assert!(
        to.starts_with(&format!("{base}/decisions/dec2_sample_mission_msn_")),
        "{to}"
    );
    let all = store.missions().list(SITE).await.unwrap();
    assert_eq!(all.len(), 1);
    let m = &all[0];
    assert_eq!(m.state, "proposed", "nothing starts before the approval");
    assert_eq!(
        m.members,
        vec!["concierge".to_owned(), "editor".to_owned()],
        "only WebAgents on duty"
    );
    assert_eq!(m.lead, "concierge");
    let (s, html, _) = get(&app, &format!("{base}/missions/{}", m.id)).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    assert!(
        html.contains("Starts when you approve its contract"),
        "{html}"
    );
    assert!(
        html.contains("A mission can&#x27;t raise any")
            || html.contains("A mission can't raise any")
    );
    let (s, _, _) = get(&app, &format!("{base}/missions/msn_nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Ending a proposed mission needs no signature; resuming an ended one isn't possible.
    let (s, _, _) = post(
        &app,
        &format!("{base}/missions/{}/state", m.id),
        "application/x-www-form-urlencoded",
        "to=ended",
    )
    .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (s, _, _) = post(
        &app,
        &format!("{base}/missions/{}/state", m.id),
        "application/x-www-form-urlencoded",
        "to=active",
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn the_owner_sits_in_takes_over_and_hands_back() {
    let (app, store) = make(SamplePort::default()).await;
    let base = format!("/console/{SITE}");
    let t = "lv_00112233445566778899aabbccddeeff";
    let live = store.live();
    live.visitor_said(SITE, t, "/cakes", "Cakes", "Do you make gluten-free tiers?")
        .await
        .unwrap();
    live.desk_said(SITE, t, "Your pages don't say. I can ask the bakery.", &[])
        .await
        .unwrap();

    let (s, html, _) = get(&app, &format!("{base}/live")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    assert!(
        html.contains("1 conversation in the last 10 minutes"),
        "{html}"
    );
    assert!(
        html.contains(&format!("{base}/live\"")),
        "once connected, Live is in the dock"
    );
    assert!(!html.contains("isn't connected"));
    assert!(html.contains("Do you make gluten-free tiers?"));
    assert!(
        html.contains("1 live"),
        "the Front of house switch counts it"
    );

    let (s, html, _) = get(&app, &format!("{base}/live/{t}")).await;
    assert_eq!(s, StatusCode::OK, "{html}");
    assert!(html.contains("You&#x27;re sitting in") || html.contains("You're sitting in"));
    assert!(html.contains("Cakes · AI"));
    let (s, _, _) = get(
        &app,
        &format!("{base}/live/lv_ffffffffffffffffffffffffffffffff"),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let form = "application/x-www-form-urlencoded";
    let (s, _, _) = post(&app, &format!("{base}/live/{t}/say"), form, "text=Hello").await;
    assert_eq!(s, StatusCode::CONFLICT, "replying needs taking over first");
    let (s, _, to) = post(&app, &format!("{base}/live/{t}/take"), form, "").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(to.as_deref(), Some(format!("{base}/live/{t}").as_str()));
    assert!(
        live.owner_has_it(SITE, t).await.unwrap(),
        "the desk must stay quiet"
    );
    let (s, _, _) = post(
        &app,
        &format!("{base}/live/{t}/say"),
        form,
        "text=Yes%2C+in+a+separate+tin.",
    )
    .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    let (s, body, _) = get(&app, &format!("{base}/live/{t}/turns?after=2")).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["owner"], true);
    let turns = v["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2, "joined, then the reply: {body}");
    assert_eq!(turns[1]["who"], "owner");
    assert!(
        turns[1]["by"].as_str().unwrap().ends_with("· a person"),
        "{body}"
    );
    let (s, _, _) = post(&app, &format!("{base}/live/{t}/back"), form, "").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert!(!live.owner_has_it(SITE, t).await.unwrap());
    let (s, _, _) = post(&app, &format!("{base}/live/{t}/shout"), form, "").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn visitors_read_their_own_conversation_on_the_site_host() {
    let (_, store) = make(SamplePort::default()).await;
    let t = "lv_00112233445566778899aabbccddeeff";
    let live = store.live();
    live.visitor_said(SITE, t, "/", "Bread", "Fresh today?")
        .await
        .unwrap();
    live.take_over(SITE, t, "Wanjiru").await.unwrap();
    live.owner_said(SITE, t, "Wanjiru", "Yes, from 6.")
        .await
        .unwrap();
    let site_of: agentrix_shell::live::SiteOfHost = Arc::new(|host: String| {
        Box::pin(async move { (host == "kilele.example").then(|| SITE.to_owned()) })
    });
    let router = agentrix_shell::live::visitor_routes(live, site_of);
    let call = |host: &'static str, path: String| {
        let router = router.clone();
        async move {
            let res = router
                .oneshot(
                    Request::get(path)
                        .header(header::HOST, host)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let s = res.status();
            let b = to_bytes(res.into_body(), 1 << 20).await.unwrap();
            (s, String::from_utf8(b.to_vec()).unwrap())
        }
    };
    let (s, body) = call("kilele.example", format!("/live/{t}?after=1")).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["person"], true);
    let labels: Vec<&str> = v["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["Wanjiru", "Wanjiru · a person"]);
    let (s, _) = call("other.example", format!("/live/{t}")).await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "another site's host can't read it"
    );
    let (s, _) = call("kilele.example", "/live/nope".to_owned()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn short_links_open_the_card_after_sign_in() {
    let (app, _) = make(SamplePort::default()).await;
    let (s, _, to) = get(&app, "/d/dec_order_118").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(
        to.as_deref(),
        Some(format!("/console/{SITE}/decisions/dec_order_118").as_str())
    );
    let (s, _, _) = get(&app, "/d/dec_nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = get(&app, "/d/..%2Fsettings").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (app, _) = make(SamplePort {
        signed_in: false,
        ..SamplePort::default()
    })
    .await;
    let (s, _, to) = get(&app, "/d/dec_order_118").await;
    assert!(s.is_redirection(), "{s}");
    let to = to.unwrap();
    assert!(
        to.contains("/d/dec_order_118") || to.contains("%2Fd%2Fdec_order_118"),
        "sign-in comes back to the link: {to}"
    );
}

#[tokio::test]
async fn the_phone_has_four_tabs_and_the_lane_leads_only_now() {
    let (app, store) = make(SamplePort::default()).await;
    let base = format!("/console/{SITE}");
    let tabs = |html: &str| {
        let start = html.find("class=\"ptabs\"").expect("the phone's tabs");
        let end = start + html[start..].find("</div>").unwrap();
        html[start..end].to_owned()
    };
    let has = |tabs: &str, tab: &str| {
        tabs.contains(&format!("{tab}</a>")) || tabs.contains(&format!("{tab}<span"))
    };
    // Before the desks send Live anything, Orders is the third tab.
    let (_, html, _) = get(&app, &base).await;
    let t = tabs(&html);
    for tab in ["Now", "Decisions", "Orders", "Apps"] {
        assert!(has(&t, tab), "{tab}");
    }
    assert!(!has(&t, "Live"));
    // Once a conversation is recorded, Live is.
    store
        .live()
        .visitor_said(
            SITE,
            "lv_00112233445566778899aabbccddeeff",
            "/",
            "Bread",
            "Fresh today?",
        )
        .await
        .unwrap();
    let (_, html, _) = get(&app, &base).await;
    let t = tabs(&html);
    for tab in ["Now", "Decisions", "Live", "Apps"] {
        assert!(has(&t, tab), "{tab}");
    }
    assert!(html.contains("on-home"));
    let (_, html, _) = get(&app, &format!("{base}/decisions")).await;
    assert!(html.contains("on-decisions"));
}

#[tokio::test]
async fn settings_link_to_channels_only_when_node_has_them() {
    let (app, _) = make(SamplePort::default()).await;
    let (_, html, _) = get(&app, &format!("/console/{SITE}/settings")).await;
    assert!(!html.contains("Set up chat and email"));
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let app = router(ShellState {
        shell: Arc::new(Shell::new().unwrap()),
        port: Arc::new(SamplePort::default()),
        store: ShellStore::open(pool).await.unwrap(),
        apps: vec![Arc::new(Channels)],
        lanes: Vec::new(),
        freeze_enabled: false,
    });
    let (_, html, _) = get(&app, &format!("/console/{SITE}/settings")).await;
    assert!(
        html.contains(&format!("href=\"/console/{SITE}/apps/channels\"")),
        "{html}"
    );
}

#[tokio::test]
async fn the_sheet_carries_the_fingerprint_and_a_ceremony() {
    let (app, _) = make(SamplePort::default()).await;
    let (s, body, _) = get(&app, &format!("/console/{SITE}/cards/dec_order_118/sheet")).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["fingerprint"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(v["approve_url"], "/decisions/dec_order_118/approve");
    assert!(v["passkey_options"].is_string());
    let (s, _, _) = get(&app, &format!("/console/{SITE}/cards/dec_golive_1/sheet")).await;
    assert_eq!(s, StatusCode::CONFLICT, "a decided card has no sheet");
}

#[tokio::test]
async fn signed_out_goes_to_sign_in_and_other_sites_are_not_found() {
    let (app, _) = make(SamplePort {
        signed_in: false,
        ..SamplePort::default()
    })
    .await;
    let (s, _, to) = get(&app, &format!("/console/{SITE}/trust")).await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert!(to.unwrap().starts_with("/auth/signin?next="));
    let (app, _) = make(SamplePort::default()).await;
    let (s, _, _) = get(&app, "/console/site_someone_else").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, to) = get(&app, "/os").await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(to.unwrap(), format!("/console/{SITE}"));
}

#[tokio::test]
async fn os_pages_open_the_same_page_of_the_owners_console() {
    let (app, _) = make(SamplePort::default()).await;
    for (from, to) in [
        ("/os/decisions", format!("/console/{SITE}/decisions")),
        ("/os/trust", format!("/console/{SITE}/trust")),
        ("/os/webagents", format!("/console/{SITE}/webagents")),
        ("/os/quarantine", format!("/console/{SITE}/quarantine")),
        (
            "/os/decisions/dec_order_118",
            format!("/console/{SITE}/decisions/dec_order_118"),
        ),
        (
            "/os/trust?from=mail",
            format!("/console/{SITE}/trust?from=mail"),
        ),
    ] {
        let (s, _, location) = get(&app, from).await;
        assert_eq!(s, StatusCode::SEE_OTHER, "{from}");
        assert_eq!(location.as_deref(), Some(to.as_str()), "{from}");
    }
    for bad in ["/os/..%2Fsettings", "/os/trust/..", "/os/a%20b"] {
        let (s, _, _) = get(&app, bad).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{bad}");
    }
    let (app, _) = make(SamplePort {
        signed_in: false,
        ..SamplePort::default()
    })
    .await;
    let (s, _, to) = get(&app, "/os/decisions").await;
    assert!(s.is_redirection(), "{s}");
    let to = to.unwrap();
    assert!(
        to.contains("/os/decisions") || to.contains("%2Fos%2Fdecisions"),
        "sign-in comes back to the page: {to}"
    );
}

#[tokio::test]
async fn freeze_then_unfreeze_with_an_approval() {
    let (app, store) = make(SamplePort::default()).await;
    let base = format!("/console/{SITE}");
    let (s, _, to) = post(
        &app,
        &format!("{base}/freeze"),
        FORM,
        &format!("back={base}%2Ftrust"),
    )
    .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(to.unwrap(), format!("{base}/trust"));
    assert!(store.freeze(SITE).await.unwrap().is_some());
    let (_, html, _) = get(&app, &base).await;
    assert!(html.contains("Frozen") && html.contains("data-unfreeze"));

    // Unfreezing without asking first is refused, and the site stays frozen.
    let (s, _, _) = post(
        &app,
        &format!("{base}/unfreeze"),
        FORM,
        "method=totp&code=123456&fingerprint=x",
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, body, _) = post(
        &app,
        &format!("{base}/unfreeze/sheet"),
        "application/json",
        "",
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let sheet: serde_json::Value = serde_json::from_str(&body).unwrap();
    let fp = sheet["fingerprint"].as_str().unwrap().to_owned();
    assert_eq!(sheet["approve_url"], format!("{base}/unfreeze"));

    // A code for another fingerprint, then a wrong code: both refused.
    let (s, _, _) = post(
        &app,
        &format!("{base}/unfreeze"),
        "application/json",
        r#"{"method":"totp","code":"123456","fingerprint":"sha256:00"}"#,
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, body, _) = post(
        &app,
        &format!("{base}/unfreeze"),
        "application/json",
        &format!(r#"{{"method":"totp","code":"000000","fingerprint":"{fp}"}}"#),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert!(store.freeze(SITE).await.unwrap().is_some());

    let (s, body, _) = post(
        &app,
        &format!("{base}/unfreeze"),
        "application/json",
        &format!(r#"{{"method":"totp","code":"123456","fingerprint":"{fp}"}}"#),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(store.freeze(SITE).await.unwrap().is_none());
}

#[tokio::test]
async fn quiet_hours_are_saved_and_checked() {
    let (app, store) = make(SamplePort::default()).await;
    let base = format!("/console/{SITE}");
    let (s, _, to) = post(
        &app,
        &format!("{base}/settings/quiet-hours"),
        FORM,
        "start=21%3A30&end=05%3A45",
    )
    .await;
    assert_eq!(s, StatusCode::SEE_OTHER);
    assert_eq!(to.unwrap(), format!("{base}/settings?saved=1"));
    let q = store.quiet_hours(SITE).await.unwrap();
    assert_eq!(q.words(), "21:30–05:45");
    let (_, html, _) = get(&app, &format!("{base}/settings?saved=1")).await;
    assert!(html.contains("Saved. Night Time now runs 21:30–05:45."));
    let (s, html, _) = post(
        &app,
        &format!("{base}/settings/quiet-hours"),
        FORM,
        "start=21%3A30&end=21%3A30",
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(html.contains("can't be the same"));
}

#[tokio::test]
async fn owners_without_a_passkey_get_the_code_path() {
    let (app, _) = make(SamplePort {
        has_passkey: false,
        ..SamplePort::default()
    })
    .await;
    let (_, html, _) = get(&app, &format!("/console/{SITE}")).await;
    assert!(html.contains("no-passkey"));
    assert!(!html.contains("No passkey on this device?"));
}

struct Support;

#[async_trait]
impl agentrix_shell::apps::LaneSource for Support {
    async fn needs_you(
        &self,
        _ctx: &AppContext,
    ) -> anyhow::Result<Vec<agentrix_shell::apps::LaneItem>> {
        Ok(vec![agentrix_shell::apps::LaneItem {
            id: "g1".into(),
            title: "Zavora support asks to read your site for 2 hours".into(),
            summary: "To look into a slow checkout you reported".into(),
            from: "Zavora support".into(),
            tone: "wait".into(),
            chip: "Read only".into(),
            when_words: "Waits until 18:00".into(),
            href: "/support-access/g1".into(),
        }])
    }
}

#[tokio::test]
async fn freeze_is_hidden_until_it_is_enforced_and_lane_sources_show() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let state = ShellState {
        shell: Arc::new(Shell::new().unwrap()),
        port: Arc::new(SamplePort::default()),
        store: ShellStore::open(pool).await.unwrap(),
        apps: Vec::new(),
        lanes: vec![Arc::new(Support)],
        freeze_enabled: false,
    };
    let app = router(state);
    let base = format!("/console/{SITE}");
    let (_, html, _) = get(&app, &base).await;
    assert!(
        !html.contains("/freeze\""),
        "the Freeze form shows while Freeze isn't enforced"
    );
    assert!(html.contains("Zavora support asks to read your site for 2 hours"));
    assert!(html.contains("href=\"/support-access/g1\""));
    let (s, _, _) = post(&app, &format!("{base}/freeze"), FORM, "").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
