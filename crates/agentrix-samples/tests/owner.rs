//! The owner apps on the platform host (`Host: localhost`): every page renders on the sample's
//! records, and the forms change those records: approve KB-1044, move an order along, change
//! a price, count stock, answer a question.

use std::path::Path;

use agentrix_samples::Config;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt as _;

const K: &str = "/samples/kilele-bakery-sample";
const T: &str = "/samples/tana-valley-times-sample";
const TANA: &str = "tana-valley-times-sample.localhost";

/// The owner apps, signed in to all three samples with their sample owner details.
async fn router() -> Router {
    let platform = mounted().await.platform;
    let mut jar = Vec::new();
    for (slug, email, password) in [
        (
            "kilele-bakery-sample",
            "wanjiru%40kilele-bakery.example",
            "kilele-sample-owner",
        ),
        (
            "classy-perfumes-sample",
            "makena%40classy-perfumes.example",
            "classy-sample-owner",
        ),
        (
            "tana-valley-times-sample",
            "nduku%40tana-valley-times.example",
            "tana-sample-editor",
        ),
    ] {
        let (status, headers, body) = send(
            &platform,
            post(
                &format!("/samples/{slug}/sign-in"),
                &format!("email={email}&password={password}"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "{body}");
        let name = format!("agx_owner_{}=", slug.replace('-', "_"));
        let cookie = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with(&name))
            .and_then(|v| v.split(';').next())
            .expect("owner session cookie")
            .to_owned();
        jar.push(cookie);
    }
    let jar = jar.join("; ");
    platform.layer(axum::middleware::map_request(
        move |mut req: Request<Body>| {
            let jar = jar.clone();
            async move {
                let merged = match req
                    .headers()
                    .get(header::COOKIE)
                    .and_then(|c| c.to_str().ok())
                {
                    Some(c) => format!("{c}; {jar}"),
                    None => jar,
                };
                if let Ok(v) = merged.parse() {
                    req.headers_mut().insert(header::COOKIE, v);
                }
                req
            }
        },
    ))
}

#[tokio::test]
async fn the_owner_apps_ask_for_the_sample_owners_sign_in() {
    let platform = mounted().await.platform;
    // Not signed in: the app sends you to the sign-in, with the details filled in.
    let (status, headers, _) =
        send(&platform, get("/samples/kilele-bakery-sample/orders", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let to = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        to.starts_with(
            "/samples/kilele-bakery-sample/sign-in?next=/samples/kilele-bakery-sample/orders"
        ),
        "{to}"
    );
    let (status, _, body) = send(&platform, get(to, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("value=\"wanjiru@kilele-bakery.example\""),
        "email filled in"
    );
    assert!(body.contains("kilele-sample-owner"), "password filled in");
    // A wrong password is refused, with the right details shown again.
    let (status, _, body) = send(
        &platform,
        post(
            "/samples/kilele-bakery-sample/sign-in",
            "email=wanjiru%40kilele-bakery.example&password=wrong",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("don't match"));
    // Signing in to one sample doesn't open another.
    let (status, _, _) = send(&platform, get("/samples/classy-perfumes-sample/", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}

/// Tana Valley Times's own pages, on its sample host.
async fn tana() -> Router {
    mounted()
        .await
        .hosts
        .into_iter()
        .next()
        .expect("sample hosts")
        .router
}

async fn mounted() -> agentrix_host::Mounted {
    let ctx = agentrix_host::testing::context().await.expect("context");
    agentrix_identity::migrate(&ctx.pool)
        .await
        .expect("identity migrations");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = Config {
        data_dir: std::env::temp_dir()
            .join(format!("agentrix-samples-owner-{}", rand::random::<u64>())),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: Some(root.join("crates/agentrix-samples/templates")),
        reseed: true,
    };
    agentrix_samples::mount_with(&ctx, config).expect("mount")
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let res = app.clone().oneshot(req).await.expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let body = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, headers, unescape(&String::from_utf8_lossy(&body)))
}

/// The page as a reader sees it: minijinja escapes `/`, quotes and `&` in values.
fn unescape(html: &str) -> String {
    html.replace("&#x2f;", "/")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::get(path).header(header::HOST, "localhost");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).expect("request")
}

fn post(path: &str, form: &str) -> Request<Body> {
    Request::post(path)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form.to_owned()))
        .expect("request")
}

/// Posts a form, expects a 303 back to `to`, and returns the one-line result the next page
/// shows.
async fn submit(app: &Router, path: &str, form: &str, to: &str) -> String {
    let (status, headers, body) = send(app, post(path, form)).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{body}");
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(location.starts_with(to), "{location}");
    let cookie = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("agx_owner_note="))
        .and_then(|v| v.split(';').next())
        .expect("result cookie")
        .to_owned();
    let page = location.split('#').next().unwrap_or(location);
    let (status, headers, body) = send(app, get(page, Some(&cookie))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().is_ok_and(|v| v.contains("Max-Age=0"))),
        "the result is shown once"
    );
    let start = body.find("class=\"flash").expect("a result line");
    let line = &body[start..];
    let line = &line[line.find('>').map_or(0, |i| i + 1)..line.find("</p>").unwrap_or(line.len())];
    line.to_owned()
}

#[tokio::test]
async fn every_page_renders_on_the_records_with_the_sample_band() {
    let app = router().await;
    for path in [
        "/samples/".to_owned(),
        format!("{K}/"),
        format!("{K}/orders"),
        format!("{K}/orders?state=held"),
        format!("{K}/orders?state=open"),
        format!("{K}/orders/KB-1044"),
        format!("{K}/orders/KB-1043"),
        format!("{K}/custom"),
        format!("{K}/catalogue"),
        format!("{K}/stock"),
        format!("{K}/questions"),
        format!("{K}/customers"),
        format!("{T}/"),
        format!("{T}/stories"),
        format!("{T}/subscribers"),
        format!("{T}/digest"),
        format!("{T}/questions"),
        format!("{T}/customers"),
    ] {
        let (status, _, body) = send(&app, get(&path, None)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert!(body.contains("approvals need it too"), "{path}");
        if path != "/samples/" {
            assert!(
                body.contains("signed in with this sample's owner details"),
                "{path}"
            );
            assert!(body.contains("Sign out"), "{path}");
        }
        assert!(!body.contains('\u{2014}'), "{path} has an em dash");
    }
    let (_, _, body) = send(&app, get(&format!("{K}/custom"), None)).await;
    assert!(body.contains("KB-1044") && body.contains("size.2") && body.contains("Amani W."));
    let (status, _, _) = send(&app, get(&format!("{K}/orders/KB-9999"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(&app, get(&format!("{K}/stories"), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a bakery has no Stories app");
}

#[tokio::test]
async fn approving_kb_1044_puts_the_payment_request_on_the_order() {
    let app = router().await;
    let line = submit(
        &app,
        &format!("{K}/custom/KB-1044"),
        "decision=approve&note=",
        &format!("{K}/custom"),
    )
    .await;
    assert!(
        line.starts_with("KB-1044 approved by Wanjiru. Amani W. sees a payment request for KSh"),
        "{line}"
    );
    let (_, _, body) = send(&app, get(&format!("{K}/orders/KB-1044"), None)).await;
    assert!(body.contains("Approved, unpaid"), "the order is approved");
    assert!(
        body.contains("Approved</b> · Wanjiru"),
        "the timeline says who approved it"
    );
    let (_, _, body) = send(&app, get(&format!("{K}/custom"), None)).await;
    assert!(
        !body.contains("custom order KB-1044"),
        "KB-1044 has left the queue"
    );
    assert!(
        body.contains("KB-1045") && body.contains("KB-1046"),
        "the other held cakes still wait"
    );
    // Deciding twice is refused in words.
    let line = submit(
        &app,
        &format!("{K}/custom/KB-1044"),
        "decision=decline&note=Sorry",
        &format!("{K}/custom"),
    )
    .await;
    assert!(line.contains("that step doesn't apply"), "{line}");
}

#[tokio::test]
async fn an_order_moves_along_only_by_the_steps_that_apply() {
    let app = router().await;
    let order = format!("{K}/orders/KB-1043");
    let (_, _, body) = send(&app, get(&order, None)).await;
    assert!(
        body.contains("value=\"baking\"") && body.contains("value=\"ready\""),
        "a paid collection bakes or is ready"
    );
    assert!(
        !body.contains("value=\"out\""),
        "a collection never goes out for delivery"
    );
    let line = submit(&app, &order, "step=ready", &order).await;
    assert_eq!(
        line,
        "KB-1043 marked ready to collect by Wanjiru. Njeri sees it on the order page."
    );
    let (_, _, body) = send(&app, get(&order, None)).await;
    assert!(body.contains("value=\"collected\""), "next is collected");
    let line = submit(&app, &order, "step=baking", &order).await;
    assert!(
        line.contains("ready to collect now, so that step doesn't apply"),
        "{line}"
    );
}

#[tokio::test]
async fn changing_a_price_changes_the_catalogue() {
    let app = router().await;
    let line = submit(
        &app,
        &format!("{K}/catalogue/sourdough"),
        "price=480&status=published",
        &format!("{K}/catalogue"),
    )
    .await;
    assert_eq!(
        line,
        "Country sourdough now costs KSh 480 (was KSh 450). Changed by Wanjiru; the site shows it on the next page load."
    );
    let (_, _, body) = send(&app, get(&format!("{K}/catalogue"), None)).await;
    assert!(
        body.contains("value=\"480\""),
        "the new price is in the form"
    );
    let line = submit(
        &app,
        &format!("{K}/catalogue/sourdough"),
        "price=lots&status=published",
        &format!("{K}/catalogue"),
    )
    .await;
    assert!(line.contains("write the price as a number"), "{line}");
}

#[tokio::test]
async fn counting_stock_and_answering_a_question() {
    let app = router().await;
    let line = submit(
        &app,
        &format!("{K}/stock/croissant"),
        "count=9",
        &format!("{K}/stock"),
    )
    .await;
    assert!(
        line.starts_with("Butter croissant counted by Wanjiru: 9 left, as of"),
        "{line}"
    );
    let (_, _, body) = send(&app, get(&format!("{K}/questions"), None)).await;
    let id = body
        .split("/questions/")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("an open question")
        .to_owned();
    let line = submit(
        &app,
        &format!("{K}/questions/{id}"),
        "answer=Yes%2C+we+deliver+there+on+Saturdays.",
        &format!("{K}/questions"),
    )
    .await;
    assert!(line.starts_with("Answer saved by Wanjiru"), "{line}");
}

#[tokio::test]
async fn the_papers_owner_apps_use_its_own_words() {
    let app = router().await;
    let (_, _, body) = send(&app, get("/samples/", None)).await;
    assert!(body.contains("a bakery, a perfume shop and a local paper"));
    assert!(body.contains("Stories, last 7 days"));
    let (_, _, body) = send(&app, get(&format!("{T}/stories"), None)).await;
    assert!(body.contains("The Editor WebAgent (AI) reads these stories"));
    assert!(body.contains("write or publish them: you do."));
    assert!(
        !body.contains("drafts from these stories"),
        "nothing drafts from them"
    );
    let (_, _, body) = send(&app, get(&format!("{T}/customers"), None)).await;
    assert!(body.contains("<h1>Readers</h1>"), "a paper has readers");
    assert!(
        !body.contains("Spent") && !body.contains("Sales"),
        "no shop words on a paper"
    );
    let (_, _, body) = send(&app, get(&format!("{T}/subscribers"), None)).await;
    assert!(body.contains("Give a subscription") && body.contains(">Give</button>"));
    assert!(body.contains("Given by you") && body.contains("PayPal test mode"));
    assert!(!body.contains("owner grant") && !body.contains("Grant"));
    let (_, _, body) = send(&app, get(&format!("{T}/digest"), None)).await;
    assert!(body.contains("<h1>Friday five</h1>"));
    assert!(
        !body.contains("works from these records"),
        "no WebAgent works from them yet"
    );
    // Each app names a WebAgent on the sample's team that reads these records when the Chief
    // asks it: the bakery has no Sales WebAgent, and the Concierge reads passed-on questions.
    let (_, _, body) = send(&app, get(&format!("{K}/customers"), None)).await;
    assert!(
        !body.contains("Sales WebAgent"),
        "Kilele has no Sales WebAgent"
    );
    let (_, _, body) = send(&app, get(&format!("{K}/questions"), None)).await;
    assert!(
        body.contains("The Concierge WebAgent (AI) reads these records"),
        "the questions' reader"
    );
    let (_, _, body) = send(&app, get(&format!("{K}/orders/KB-1044"), None)).await;
    assert!(
        body.contains("<summary>Details</summary>"),
        "the rule id sits behind Details"
    );
}

async fn tana_page(app: &Router, path: &str) -> String {
    let req = Request::get(path)
        .header(header::HOST, TANA)
        .body(Body::empty())
        .expect("request");
    let (status, _, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "{path}: {body}");
    assert!(!body.contains('\u{2014}'), "{path} has an em dash");
    body
}

#[tokio::test]
async fn tana_pages_follow_the_copy_review() {
    let app = tana().await;
    let body = tana_page(&app, "/").await;
    assert!(body.contains(
        "The WebAgents on this site are AI. They answer only from published stories, link every one, and never ask for a password, a code or a card."
    ));
    assert!(body.contains("email, usually within a day"));
    assert!(body.contains(
        "Every Friday at 07:00: five stories published that week, picked by the editor's rules."
    ));
    assert!(!body.contains("Friday digest"));

    let all = tana_page(&app, "/archive").await;
    // The stories are dated relative to today, so between midnight and the day's first story
    // fewer than all 63 are published yet: check the line, not the count.
    assert!(
        all.contains(" stories across ") && all.contains(" weekly issues, newest first."),
        "{all}"
    );
    let council = tana_page(&app, "/archive?section=council").await;
    assert!(
        council.contains("stories in Council across"),
        "the count follows the filter"
    );
    assert!(!council.contains("63 stories"));

    // A subscriber story shows its headline and link only: no standfirst, here or in lists.
    let standfirst = "The monthly fuel price review sets what operators pay.";
    let story = tana_page(&app, "/2026/11/what-the-fuel-price-review-means-for-fares").await;
    assert!(story.contains("What the fuel price review means for matatu fares"));
    assert!(!story.contains(standfirst));
    let business = tana_page(&app, "/business").await;
    assert!(business.contains("What the fuel price review means for matatu fares"));
    assert!(!business.contains(standfirst));
    assert!(business.contains("Traders can send their prices to the newsroom each week"));

    let subscribe = tana_page(&app, "/subscribe").await;
    assert!(subscribe.contains("KSh 300 a month (sample price)"));
    assert!(subscribe.contains("USD 2.33 to Tana Valley Times (KSh 300 at 1 USD = KSh 129.00)"));
    assert!(!subscribe.contains("advertising") && !subscribe.contains("reminds you"));

    let corrections = tana_page(&app, "/corrections").await;
    assert!(corrections.contains("use the corrected version"));
    assert!(corrections.contains("usually replies by email within a day"));
}
