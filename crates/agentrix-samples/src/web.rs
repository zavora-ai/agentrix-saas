//! The router for sample hosts, the per-request site context, cookies, filters and the
//! stand-in customer sign-in.

use std::sync::Arc;

use agentrix_records::{Records, Site, money};
use async_trait::async_trait;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use minijinja::Environment;
use tokio::sync::OnceCell;
use tower::ServiceExt as _;

use crate::App;
use crate::ports::{SampleHost, Visitor, Visitors};

/// What every handler on a sample host knows about the request.
#[derive(Clone)]
pub struct Ctx {
    /// The site.
    pub site: Site,
    /// The host, as W6's `SiteHost`.
    pub host: SampleHost,
    /// The signed-in customer, if any.
    pub visitor: Option<Visitor>,
    /// The request path (for "next" links and the shell).
    pub path: String,
    /// The cart cookie, if the browser has one.
    pub cart: Option<String>,
}

/// A handler error: logged, shown as a plain apology.
pub struct Fail(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for Fail {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        tracing::error!(error = %format!("{:#}", self.0), "sample page failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "This page didn't load. Try again in a moment; the rest of the site still works.",
        )
            .into_response()
    }
}

/// A handler result.
pub type Page = Result<Response, Fail>;

/// An HTML response.
pub fn html(body: String) -> Response {
    Html(body).into_response()
}

/// A 303 redirect.
#[allow(dead_code)] // used by the commerce and owner pages
pub fn see_other(to: &str) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, to.to_owned())]).into_response()
}

/// Not found, plainly. Pages render their own branded 404 where they can.
pub fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "That page isn't on this site.",
    )
        .into_response()
}

/// A cookie value by name.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}

/// A host-only cookie for this site, `Secure` on https.
#[allow(dead_code)] // used by the commerce pages
pub fn set_cookie(app: &App, name: &str, value: &str, max_age: i64) -> HeaderValue {
    let secure = if app.config.scheme == "https" {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// The cart cookie's name.
pub const CART: &str = "agx_cart";
/// How long the cart cookie lasts, in seconds (30 days; the privacy page says so).
pub const CART_SECONDS: i64 = 60 * 60 * 24 * 30;
/// The stand-in session cookie's name.
pub const SESSION: &str = "agx_customer";

fn slug_of(host: &str) -> String {
    let host = host
        .split(':')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    host.split('.').next().unwrap_or_default().to_owned()
}

/// The router node sends the three sample hosts to. `extra` (W6's account pages, W7's kit
/// assets) is merged inside the layer that sets the site context, so it sees `SiteHost` too.
pub fn site_router(app: App, extra: Router) -> Router {
    let bakery = crate::bakery::routes().with_state(app.clone());
    let shop = crate::classy::routes().with_state(app.clone());
    let news = crate::news::routes().with_state(app.clone());
    let pick = move |req: Request| {
        let (bakery, shop, news) = (bakery.clone(), shop.clone(), news.clone());
        async move {
            let archetype = req
                .extensions()
                .get::<Ctx>()
                .map(|c| c.site.archetype.clone())
                .unwrap_or_default();
            let router = match archetype.as_str() {
                "bakery" => bakery,
                "shop" => shop,
                "news" => news,
                _ => return not_found(),
            };
            match router.oneshot(req).await {
                Ok(r) => r,
                Err(never) => match never {},
            }
        }
    };
    Router::new()
        .merge(crate::common::routes())
        .merge(crate::commerce::routes())
        .merge(crate::door::routes())
        .fallback(pick)
        .with_state(app.clone())
        .merge(extra)
        .merge(agentrix_ui::assets())
        .layer(middleware::from_fn_with_state(app, with_site))
}

async fn with_site(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or_default().to_ascii_lowercase())
        .unwrap_or_default();
    let records = match app.records().await {
        Ok(r) => r,
        Err(e) => return Fail(e).into_response(),
    };
    let site = match records.site(&slug_of(&host)).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found(),
        Err(e) => return Fail(e).into_response(),
    };
    let sample_host = SampleHost {
        site: site.site.clone(),
        origin: app.config.origin_for(&host),
        host,
        name: site.name.clone(),
        sample: site.is_sample(),
    };
    let visitor = app
        .visitors
        .visitor(&sample_host, req.headers())
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "visitor lookup failed");
            None
        });
    let ctx = Ctx {
        path: req.uri().path().to_owned(),
        cart: cookie(req.headers(), CART),
        site,
        host: sample_host,
        visitor,
    };
    req.extensions_mut()
        .insert(crate::identity::site_host(&ctx.host));
    req.extensions_mut().insert(ctx.host.clone());
    req.extensions_mut().insert(ctx);
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers
        .entry(header::X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    headers
        .entry(header::REFERRER_POLICY)
        .or_insert(HeaderValue::from_static("same-origin"));
    headers
        .entry("x-robots-tag")
        .or_insert(HeaderValue::from_static("noindex"));
    response
}

/// Template filters: `ksh`, `usd`, `money(currency)`.
pub fn filters(env: &mut Environment<'static>) {
    env.add_filter("ksh", |minor: i64| money::ksh(minor));
    env.add_filter("usd", |cents: i64| money::usd(cents));
    env.add_filter("money", |minor: i64, currency: String| {
        money::fmt(&currency, minor)
    });
}

/// The samples' own customer sign-in, until W6's is wired: an emailed one-time code (shown on
/// screen as the "Sample mailbox", since a sample sends no email) and a session cookie.
pub struct StandInVisitors {
    records: Arc<OnceCell<Records>>,
}

impl StandInVisitors {
    /// Over the app's records cell.
    pub fn new(records: Arc<OnceCell<Records>>) -> Self {
        Self { records }
    }
}

#[async_trait]
impl Visitors for StandInVisitors {
    async fn visitor(
        &self,
        site: &SampleHost,
        headers: &HeaderMap,
    ) -> anyhow::Result<Option<Visitor>> {
        let Some(records) = self.records.get() else {
            return Ok(None);
        };
        let Some(token) = cookie(headers, SESSION) else {
            return Ok(None);
        };
        let customer = records
            .session_customer(&site.site, &token, agentrix_records::time::now())
            .await?;
        Ok(customer.map(|c| Visitor {
            account: c.customer_id,
            name: c.name,
            email: c.email,
            email_verified: true,
            phone: c.phone,
        }))
    }
    fn is_stand_in(&self) -> bool {
        true
    }
}
