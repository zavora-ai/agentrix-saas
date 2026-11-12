//! Kilele's shop host end to end through the router: add to bag, checkout, the sample pay step
//! and the receipt for a signed-in customer; sign-in and status-only views for a guest.

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_samples::ports::{SampleHost, Visitor, Visitors};
use agentrix_samples::{App, Config};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use tower::ServiceExt as _;

const HOST: &str = "kilele-bakery-sample.localhost";

/// Njeri, as W6 would report her.
struct Njeri {
    verified: bool,
}

#[async_trait::async_trait]
impl Visitors for Njeri {
    async fn visitor(
        &self,
        _site: &SampleHost,
        _headers: &HeaderMap,
    ) -> anyhow::Result<Option<Visitor>> {
        Ok(Some(Visitor {
            account: "cus-njeri".into(),
            name: "Njeri".into(),
            email: "njeri@example.com".into(),
            email_verified: self.verified,
            phone: Some("+44 7700 900418".into()),
        }))
    }
}

async fn site(visitor: Option<Njeri>) -> Router {
    site_and_owner(visitor).await.0
}

/// The shop host and the owner apps (platform host) on the same records.
async fn site_and_owner(visitor: Option<Njeri>) -> (Router, Router) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ctx = agentrix_host::testing::context().await.unwrap();
    ctx.platform_host = "localhost".into();
    let config = Config {
        data_dir: std::env::temp_dir().join(agentrix_records::new_id("agentrix-samples-commerce")),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: Some(root.join("crates/agentrix-samples/templates")),
        reseed: true,
    };
    let mut app = App::new(config, &ctx).unwrap();
    if let Some(v) = visitor {
        app = app.with_visitors(Arc::new(v));
    }
    let mounted = agentrix_samples::mounted(app, Router::new());
    (
        mounted.hosts.into_iter().next().unwrap().router,
        mounted.platform,
    )
}

struct Reply {
    status: StatusCode,
    location: String,
    cookie: Option<String>,
    body: String,
}

async fn send(
    r: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    form: Option<&str>,
) -> Reply {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    if form.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let res = r
        .clone()
        .oneshot(req.body(Body::from(form.unwrap_or("").to_owned())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::to_owned);
    let body = String::from_utf8(
        axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    Reply {
        status,
        location,
        cookie,
        body,
    }
}

/// The value of the first `name="<name>"` input that isn't disabled.
fn field(html: &str, name: &str) -> String {
    let needle = format!("name=\"{name}\" value=\"");
    html.match_indices(&needle)
        .map(|(i, _)| &html[i + needle.len()..])
        .find(|rest| !rest[..rest.find('>').unwrap_or(0)].contains("disabled"))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn a_signed_in_customer_orders_pays_on_the_sample_and_gets_a_receipt() {
    let r = site(Some(Njeri { verified: true })).await;
    let empty = send(&r, "GET", "/bag", None, None).await;
    assert!(empty.body.contains("Your bag is empty."), "{}", empty.body);

    let added = send(
        &r,
        "POST",
        "/bag/add",
        None,
        Some("sku=rye&qty=1&back=/bag"),
    )
    .await;
    assert_eq!(added.status, StatusCode::SEE_OTHER);
    assert_eq!(added.location, "/bag");
    let cart = added.cookie.expect("a cart cookie");
    assert!(cart.starts_with("agx_cart="));
    let bag = send(&r, "GET", "/bag", Some(&cart), None).await;
    assert!(bag.body.contains("Rye and caraway"), "{}", bag.body);
    assert!(
        bag.body.contains("Price list of ") && !bag.body.contains("Price list v14"),
        "visitors see the price list by its date, never its version"
    );

    let checkout = send(&r, "GET", "/checkout", Some(&cart), None).await;
    assert_eq!(checkout.status, StatusCode::OK, "{}", checkout.body);
    assert!(
        checkout
            .body
            .contains("Quote pinned to Kilele&#x27;s price list of ")
            || checkout
                .body
                .contains("Quote pinned to Kilele's price list of "),
        "{}",
        checkout.body
    );
    let key = field(&checkout.body, "key");
    let slot = field(&checkout.body, "slot");
    assert!(!key.is_empty() && !slot.is_empty());

    let missing = send(
        &r,
        "POST",
        "/checkout",
        Some(&cart),
        Some(&format!("key={key}&how=delivery&zone=zone1&slot={slot}")),
    )
    .await;
    assert!(
        missing.body.contains("Add who receives it"),
        "{}",
        missing.body
    );

    let form = format!(
        "key={key}&how=delivery&zone=zone1&slot={slot}&recipient=Amina%2C+Kilimani&phone=%2B44+7700+900418"
    );
    let placed = send(&r, "POST", "/checkout", Some(&cart), Some(&form)).await;
    assert_eq!(placed.status, StatusCode::SEE_OTHER, "{}", placed.body);
    assert!(
        placed.location.starts_with("/orders/KB-") && placed.location.ends_with("/pay"),
        "{}",
        placed.location
    );
    let again = send(&r, "POST", "/checkout", Some(&cart), Some(&form)).await;
    assert_eq!(
        again.location, placed.location,
        "the same key gives the same order"
    );
    let id = placed
        .location
        .trim_start_matches("/orders/")
        .trim_end_matches("/pay")
        .to_owned();

    let pay = send(&r, "GET", &placed.location, None, None).await;
    assert!(
        pay.body.contains(&format!("Pay for {id}")) && pay.body.contains("Not paid yet"),
        "{}",
        pay.body
    );
    let started = send(&r, "POST", &placed.location, None, None).await;
    assert!(started.body.contains("mark it paid"), "{}", started.body);
    let marked = send(&r, "POST", &format!("/orders/{id}/pay/sample"), None, None).await;
    assert_eq!(marked.location, format!("/orders/{id}/receipt"));

    let receipt = send(&r, "GET", &marked.location, None, None).await;
    assert!(
        receipt
            .body
            .contains("Sample receipt: PayPal test payment, no real money moved."),
        "{}",
        receipt.body
    );
    assert!(receipt.body.contains("1 USD = KSh 129.00"));
    let orders = send(&r, "GET", "/orders", None, None).await;
    assert!(
        orders.body.contains(&id) && orders.body.contains("KB-1042"),
        "{}",
        orders.body
    );
    let page = send(&r, "GET", &format!("/orders/{id}"), None, None).await;
    assert!(
        page.body.contains("Something&#x27;s wrong") || page.body.contains("Something's wrong"),
        "{}",
        page.body
    );

    let theirs = send(&r, "GET", "/orders/KB-1044", None, None).await;
    assert_eq!(theirs.status, StatusCode::OK);
    assert!(theirs.body.contains("only the status"), "{}", theirs.body);
    assert!(
        !theirs.body.contains("Happy 30th Amani"),
        "another customer's details stay hidden"
    );
}

#[tokio::test]
async fn a_guest_is_asked_to_sign_in_and_sees_status_only() {
    let r = site(None).await;
    let added = send(&r, "POST", "/bag/add", None, Some("sku=rye")).await;
    let cart = added.cookie.expect("a cart cookie");
    let checkout = send(&r, "GET", "/checkout", Some(&cart), None).await;
    assert_eq!(checkout.status, StatusCode::SEE_OTHER);
    assert!(
        checkout
            .location
            .starts_with("/account/sign-in?why=order&next=/checkout"),
        "{}",
        checkout.location
    );
    let order = send(&r, "GET", "/orders/KB-1042", None, None).await;
    assert!(
        order.location.starts_with("/account/sign-in"),
        "{}",
        order.location
    );
    let lookup = send(&r, "GET", "/orders?number=kb-1042", None, None).await;
    assert!(
        lookup.body.contains("KB-1042 is delivered."),
        "{}",
        lookup.body
    );
    assert!(lookup.body.contains("only the status"));
    let missing = send(&r, "GET", "/orders?number=KB-9999", None, None).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let foreign = Request::builder()
        .method("POST")
        .uri("/bag/add")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, "https://elsewhere.example")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("sku=rye"))
        .unwrap();
    assert_eq!(
        r.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn an_unconfirmed_email_is_asked_to_confirm_before_checkout() {
    let r = site(Some(Njeri { verified: false })).await;
    let added = send(&r, "POST", "/bag/add", None, Some("sku=rye")).await;
    let checkout = send(&r, "GET", "/checkout", Some(&added.cookie.unwrap()), None).await;
    assert_eq!(checkout.status, StatusCode::OK);
    assert!(
        checkout
            .body
            .contains("has to be confirmed as yours before you order"),
        "{}",
        checkout.body
    );
    assert!(checkout.body.contains("href=\"/account/security\""));
}

/// Njeri orders a rye loaf for delivery at the latest open slot; the order's id.
async fn order_rye(r: &Router) -> String {
    let added = send(r, "POST", "/bag/add", None, Some("sku=rye&qty=1&back=/bag")).await;
    let cart = added.cookie.expect("a cart cookie");
    let checkout = send(r, "GET", "/checkout", Some(&cart), None).await;
    let key = field(&checkout.body, "key");
    // The latest slot, so the cut-off is days away whenever the test runs.
    let slot = checkout
        .body
        .match_indices("name=\"slot\" value=\"")
        .filter_map(|(i, n)| checkout.body[i + n.len()..].split('"').next())
        .last()
        .unwrap()
        .to_owned();
    let form = format!(
        "key={key}&how=delivery&zone=zone1&slot={slot}&recipient=Amina%2C+Kilimani&phone=%2B44+7700+900418"
    );
    let placed = send(r, "POST", "/checkout", Some(&cart), Some(&form)).await;
    assert_eq!(placed.status, StatusCode::SEE_OTHER, "{}", placed.body);
    placed
        .location
        .trim_start_matches("/orders/")
        .trim_end_matches("/pay")
        .to_owned()
}

#[tokio::test]
async fn a_customer_cancels_before_paying_and_asks_the_owner_once_paid() {
    let (r, platform) = site_and_owner(Some(Njeri { verified: true })).await;

    // Not paid yet: cancelled at once, nothing charged.
    let id = order_rye(&r).await;
    let page = send(&r, "GET", &format!("/orders/{id}"), None, None).await;
    assert!(page.body.contains("Cancel this order"), "{}", page.body);
    let done = send(&r, "POST", &format!("/orders/{id}/cancel"), None, None).await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    let page = send(&r, "GET", &format!("/orders/{id}"), None, None).await;
    assert!(page.body.contains(">Cancelled<"), "{}", page.body);
    assert!(!page.body.contains("Cancel this order"), "{}", page.body);

    // Paid and not baked: the request goes to Wanjiru.
    let id = order_rye(&r).await;
    send(&r, "POST", &format!("/orders/{id}/pay"), None, None).await;
    send(&r, "POST", &format!("/orders/{id}/pay/sample"), None, None).await;
    let page = send(&r, "GET", &format!("/orders/{id}"), None, None).await;
    assert!(page.body.contains("Ask Wanjiru to cancel"), "{}", page.body);
    send(&r, "POST", &format!("/orders/{id}/cancel"), None, None).await;
    let page = send(&r, "GET", &format!("/orders/{id}"), None, None).await;
    assert!(page.body.contains("You asked to cancel."), "{}", page.body);
    assert!(page.body.contains("Paid"), "still paid until she decides");

    // Wanjiru sees a decision card and cancels with a refund.
    let signed = Request::post("/samples/kilele-bakery-sample/sign-in")
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(
            "email=wanjiru%40kilele-bakery.example&password=kilele-sample-owner",
        ))
        .unwrap();
    let res = platform.clone().oneshot(signed).await.unwrap();
    let cookie = res.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let owner = |method: &str, path: &str, form: &str| {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "localhost")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_owned()))
            .unwrap()
    };
    let today = platform
        .clone()
        .oneshot(owner("GET", "/samples/kilele-bakery-sample/", ""))
        .await
        .unwrap();
    let today = axum::body::to_bytes(today.into_body(), usize::MAX)
        .await
        .unwrap();
    let today = String::from_utf8_lossy(&today);
    assert!(today.contains("Cancel request"), "{today}");
    let decided = platform
        .clone()
        .oneshot(owner(
            "POST",
            &format!("/samples/kilele-bakery-sample/orders/{id}"),
            "step=cancel_refund",
        ))
        .await
        .unwrap();
    assert_eq!(decided.status(), StatusCode::SEE_OTHER);
    let page = send(&r, "GET", &format!("/orders/{id}"), None, None).await;
    assert!(page.body.contains(">Refunded<"), "{}", page.body);
    assert!(
        page.body
            .contains("Cancelled by Wanjiru before it was made. Refunded USD"),
        "{}",
        page.body
    );
}
