//! Routes every sample host has: assets, Ask, passing a question to the owner, holds, and the
//! privacy page with its look-alike report.

use std::collections::HashMap;
use std::sync::Mutex;

use agentrix_records::pricing::PriceList;
use agentrix_records::{Records, Site, time};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::App;
use crate::web::{Ctx, Fail, Page, html};

/// The common routes.
pub fn routes() -> Router<App> {
    Router::new()
        .route("/_s/{file}", get(asset))
        .route("/ask", post(crate::ask::ask))
        .route("/ask/pass", post(pass))
        .route("/ask/feedback", post(crate::ask::feedback))
        .route("/hold", post(hold))
        .route("/privacy", get(privacy))
}

/// Where look-alike reports go: the platform's own address (`AGENTRIX_EMAIL_FROM`'s default).
/// A sample's owner is invented, so Agentrix, which runs the sample, takes the report.
pub const REPORT_TO: &str = agentrix_messages::email::DEFAULT_FROM;

/// The facts the privacy page states, each from the code that keeps them, so the page can't
/// drift from what the site does.
#[derive(Debug, Clone, Serialize)]
pub struct Privacy {
    /// A sign-in code works for this many minutes.
    pub code_minutes: u32,
    /// Wrong tries before a code stops working.
    pub code_tries: u8,
    /// Days the sign-in cookie lasts.
    pub session_days: i64,
    /// Days the bag cookie lasts.
    pub bag_days: i64,
    /// Months the daily visit totals are kept.
    pub visit_months: i64,
    /// Where look-alike reports go.
    pub report_to: &'static str,
    /// A `mailto:` link for a report, with the subject and the questions filled in.
    pub report_href: String,
}

impl Privacy {
    /// The facts for this site, with a report link that names it.
    pub fn for_site(ctx: &Ctx) -> Self {
        let subject = format!("A site pretending to be {} (sample)", ctx.site.name);
        let body = format!(
            "The address I saw:\nWhat it asked for:\nHow the link reached me:\n\nThe real site is {}",
            ctx.host.origin
        );
        Self {
            code_minutes: agentrix_identity::visitor::CODE_MINUTES,
            code_tries: agentrix_identity::visitor::CODE_TRIES,
            session_days: agentrix_identity::visitor::SESSION_SECONDS / 86_400,
            bag_days: crate::web::CART_SECONDS / 86_400,
            visit_months: agentrix_store::visits::KEEP_DAYS * 12 / 365,
            report_to: REPORT_TO,
            report_href: format!(
                "mailto:{REPORT_TO}?subject={}&body={}",
                mailto_encode(&subject),
                mailto_encode(&body)
            ),
        }
    }
}

/// Percent-encodes a `mailto:` header value (RFC 6068): everything but letters, digits and
/// `-._~`, so slashes and line breaks survive the page's HTML escaping and the mail app.
fn mailto_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `GET /privacy`: what the sample keeps, in its own look, and how to report a look-alike.
async fn privacy(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let description = format!(
        "What {} keeps about you and for how long, and how to report a site pretending to be it.",
        ctx.site.name
    );
    let extra = json!({ "privacy": Privacy::for_site(&ctx) });
    let (template, view) = match ctx.site.archetype.as_str() {
        "news" => (
            "tana/privacy.html",
            crate::news::view(&app, &ctx, "", "Privacy", &description, extra).await?,
        ),
        archetype => {
            let shell = app.shell(&ctx, "", "Privacy", &description).await?;
            let mut view = json!({ "shell": shell });
            if let (Some(v), serde_json::Value::Object(e)) = (view.as_object_mut(), extra) {
                v.extend(e);
            }
            let template = if archetype == "shop" {
                "classy/privacy.html"
            } else {
                "kilele/privacy.html"
            };
            (template, view)
        }
    };
    Ok(html(app.render(template, &view)?))
}

/// "the price list of Sun 1 Nov": a price list as visitors see it, by the day it took effect
/// (01a §6.3: no version numbers on visitor pages).
pub fn list_name(site: &Site, list: &PriceList) -> String {
    format!(
        "the price list of {}",
        time::fmt_day(list.valid_from, site.utc_offset_min)
    )
}

/// When the owner usually replies: "usually within 2 hours (7:00–20:00)" from the manifest's
/// `owner.reply`, else from `owner.wait` and the channel ("usually within 3 h on WhatsApp").
pub fn owner_reply(site: &Site) -> String {
    let reply = site.fact("/owner/reply");
    if !reply.is_empty() {
        return reply;
    }
    format!(
        "usually within {} on {}",
        site.fact("/owner/wait").trim_start_matches('~'),
        site.fact("/owner/channel")
    )
}

/// [`list_name`] for a pinned version, or "the price list" when that version isn't kept.
///
/// # Errors
///
/// On a database error.
pub async fn list_name_of(records: &Records, site: &Site, version: &str) -> anyhow::Result<String> {
    Ok(records
        .price_lists(&site.site)
        .await?
        .iter()
        .find(|l| l.version == version)
        .map_or_else(|| "the price list".to_owned(), |l| list_name(site, l)))
}

async fn asset(Path(file): Path<String>) -> Response {
    match agentrix_themes::asset(&file) {
        Some((bytes, kind)) => (
            [
                (header::CONTENT_TYPE, kind),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found.").into_response(),
    }
}

#[derive(Deserialize)]
struct PassBody {
    question: String,
    #[serde(default)]
    contact: Option<String>,
    #[serde(default)]
    desk: Option<String>,
    #[serde(default)]
    page: Option<String>,
}

static PASSED: Mutex<Option<HashMap<String, Vec<i64>>>> = Mutex::new(None);

/// The client's address as the proxy saw it, for per-client limits (never stored).
pub(crate) fn client_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// Five passes an hour per client, per site.
fn allow(key: &str, now: i64) -> bool {
    let mut guard = PASSED.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let list = map.entry(key.to_owned()).or_default();
    list.retain(|t| now - t < 3600);
    if list.len() >= 5 {
        return false;
    }
    list.push(now);
    true
}

async fn pass(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Json(body): Json<PassBody>,
) -> Result<Response, Fail> {
    let q = body.question.trim();
    let owner = ctx.site.fact("/owner/first");
    let owner = if owner.is_empty() {
        "the owner".to_owned()
    } else {
        owner
    };
    if q.is_empty() || q.chars().count() > 600 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "words": "Write the question in a sentence or two." })),
        )
            .into_response());
    }
    let now = time::now();
    let key = format!("{}:{}", ctx.site.site, client_key(&headers));
    if !allow(&key, now) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "ok": false, "words": format!("You've sent {owner} five questions this hour. Try again in an hour.") })),
        )
            .into_response());
    }
    let records = app.records().await?;
    records
        .pass_question(
            &ctx.site.site,
            body.desk.as_deref().unwrap_or("Front desk"),
            q,
            body.contact.as_deref(),
            body.page.as_deref(),
            now,
        )
        .await?;
    let reply = owner_reply(&ctx.site);
    let words = match body
        .contact
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        Some(c) => format!("Passed to {owner} (a person). She'll reply to {c}, {reply}."),
        None => format!(
            "Passed to {owner} (a person). You left no email or number, so she can't reply to you. Add one and send it again to get her answer."
        ),
    };
    Ok(Json(json!({ "ok": true, "words": words })).into_response())
}

#[derive(Deserialize)]
struct HoldBody {
    sku: String,
    qty: i64,
    #[serde(default)]
    name: Option<String>,
}

async fn hold(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Json(body): Json<HoldBody>,
) -> Result<Response, Fail> {
    let records = app.records().await?;
    let name = body
        .name
        .filter(|n| !n.trim().is_empty())
        .or_else(|| ctx.visitor.as_ref().map(|v| v.name.clone()))
        .unwrap_or_else(|| "a visitor".into());
    let now = time::now();
    match records
        .create_hold(&ctx.site, &body.sku, body.qty, name.trim(), now)
        .await
    {
        Ok(h) => {
            let product = records
                .product(&ctx.site.site, &h.sku)
                .await?
                .map(|p| p.name.to_lowercase())
                .unwrap_or_default();
            Ok(Json(json!({
                "ok": true,
                "words": format!(
                    "Held: {} {} until {}, hold {}. It's free; you pay at the counter.",
                    h.qty,
                    if h.qty == 1 { product.clone() } else { format!("{product} loaves").replace("country sourdough loaves", "sourdough loaves") },
                    time::fmt_clock(h.until_at, ctx.site.utc_offset_min),
                    h.hold_id,
                ),
            }))
            .into_response())
        }
        Err(agentrix_records::orders::OrderError::Refused(words)) => {
            Ok(Json(json!({ "ok": false, "words": words })).into_response())
        }
        Err(agentrix_records::orders::OrderError::Other(e)) => Err(e.into()),
    }
}
