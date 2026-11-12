//! The owner's apps for each sample, on the platform host under `/samples/<site>/`.
//!
//! A judge opens these to see the owner run the shop: the orders customers just placed, the
//! custom order waiting for approval, the catalogue and stock, questions passed to the owner,
//! and for the newspaper its stories and subscribers. Every page carries the Sample owner view
//! band: on a sample anyone can open it; on a real site only the owner and their team can,
//! after signing in, and approvals need their passkey.
//!
//! The pages are shaped like W5's console app registry: one app per page, each with its name,
//! icon and the WebAgent it works with, a dock of the site's apps and a site switcher. Every
//! figure is computed from records at request time. Forms post back and redirect (303) to the
//! page with a one-line result carried in a short-lived cookie.

use std::collections::HashMap;

use agentrix_records::catalogue::{Product, Stock};
use agentrix_records::news::{DigestSub, Story, Subscriber};
use agentrix_records::orders::{OrderError, OrderView};
use agentrix_records::questions::Question;
use agentrix_records::{Records, Site, money, time};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::web::{Page, cookie, html, see_other};
use crate::{App, SAMPLES};

/// The owner apps' routes (platform host).
pub fn router(app: App) -> Router {
    // Each sample's apps sit behind its own owner sign-in (filled in, since it's a sample).
    let gated = Router::new()
        .route("/samples/{site}/", get(today))
        .route("/samples/{site}/orders", get(orders))
        .route("/samples/{site}/orders/{id}", get(order).post(advance))
        .route("/samples/{site}/custom", get(custom))
        .route("/samples/{site}/custom/{id}", post(decide))
        .route("/samples/{site}/catalogue", get(catalogue))
        .route("/samples/{site}/catalogue/{sku}", post(update_product))
        .route("/samples/{site}/stock", get(stock))
        .route("/samples/{site}/stock/{sku}", post(count_stock))
        .route("/samples/{site}/questions", get(questions))
        .route("/samples/{site}/questions/{id}", post(answer))
        .route("/samples/{site}/feedback/{id}", post(teach_from_mark))
        .route("/samples/{site}/customers", get(customers))
        .route("/samples/{site}/stories", get(stories))
        .route("/samples/{site}/stories/{slug}", post(story_action))
        .route("/samples/{site}/subscribers", get(subscribers).post(grant))
        .route(
            "/samples/{site}/subscribers/{id}/end",
            post(end_subscription),
        )
        .route("/samples/{site}/digest", get(digest))
        .route_layer(axum::middleware::from_fn_with_state(
            app.clone(),
            owner_gate,
        ));
    // `/samples` itself is agentrix.website's Samples page (agentrix-home); the owner views
    // start at `/samples/`.
    Router::new()
        .route("/samples/", get(index))
        .route("/samples/_s/{file}", get(asset))
        .route("/samples/{site}", get(to_site))
        .route("/samples/{site}/sign-in", get(sign_in_page).post(sign_in))
        .route("/samples/{site}/sign-out", post(sign_out))
        .merge(gated)
        .with_state(app)
}

// ---------------------------------------------------------------------------------------------
// The sample owner's sign-in: the details are in the bundle and filled in on the page, so anyone
// can see the owner's side; on a real site the owner signs in on the platform, with a passkey.

fn owner_cookie(slug: &str) -> String {
    format!("agx_owner_{}", slug.replace('-', "_"))
}

fn login_of(site: &Site) -> (String, String) {
    (
        site.fact("/owner/login/email"),
        site.fact("/owner/login/password"),
    )
}

/// Whether the request carries a live owner session for this sample.
async fn signed_in(app: &App, slug: &str, headers: &HeaderMap) -> bool {
    let Some(token) = cookie(headers, &owner_cookie(slug)) else {
        return false;
    };
    let Ok(records) = app.records().await else {
        return false;
    };
    matches!(records.session_holder(slug, &token, time::now()).await, Ok(Some(who)) if who == "owner")
}

async fn owner_gate(
    State(app): State<App>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path().to_owned();
    let slug = path
        .trim_start_matches("/samples/")
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    if signed_in(&app, &slug, req.headers()).await {
        return next.run(req).await;
    }
    let back = if req.method() == axum::http::Method::GET {
        path.clone()
    } else {
        format!("/samples/{slug}/")
    };
    see_other(&format!(
        "/samples/{slug}/sign-in?next={}",
        crate::shell::urlencode(&back)
    ))
}

#[derive(Deserialize, Default)]
struct SignInQuery {
    #[serde(default)]
    next: String,
}

#[derive(Deserialize)]
struct SignInForm {
    email: String,
    password: String,
    #[serde(default)]
    next: String,
}

fn safe_next(slug: &str, next: &str) -> String {
    let home = format!("/samples/{slug}/");
    if next.starts_with(&home) && !next.contains("//") && !next.contains("sign-") {
        next.to_owned()
    } else {
        home
    }
}

async fn sign_in_view(app: &App, site: &Site, next: &str, error: &str) -> anyhow::Result<Response> {
    let (email, password) = login_of(site);
    let view = json!({
        "site": { "name": site.name, "slug": site.site, "owner": site.fact("/owner/name"), "first": site.fact("/owner/first"), "url": format!("{}/", app.config.origin_for(&site.host)) },
        "email": email,
        "password": password,
        "next": safe_next(&site.site, next),
        "error": error,
        "v": agentrix_themes::VERSION,
        "ui_v": agentrix_ui::VERSION,
    });
    Ok(html(app.render("owner/signin.html", &view)?))
}

async fn sign_in_page(
    State(app): State<App>,
    Path(slug): Path<String>,
    Query(q): Query<SignInQuery>,
    headers: HeaderMap,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    if signed_in(&app, &slug, &headers).await {
        return Ok(see_other(&safe_next(&slug, &q.next)));
    }
    Ok(sign_in_view(&app, &site, &q.next, "").await?)
}

async fn sign_in(
    State(app): State<App>,
    Path(slug): Path<String>,
    Form(form): Form<SignInForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let (email, password) = login_of(&site);
    let ok = !email.is_empty()
        && form.email.trim().eq_ignore_ascii_case(&email)
        && form.password == password;
    if !ok {
        let mut r = sign_in_view(&app, &site, &form.next, "That email and password don't match this sample's owner. The right ones are filled in below.").await?;
        *r.status_mut() = StatusCode::UNAUTHORIZED;
        return Ok(r);
    }
    let token = records
        .start_session(&slug, "owner", Some("sample owner sign-in"), time::now())
        .await?;
    let secure = if app.config.scheme == "https" {
        "; Secure"
    } else {
        ""
    };
    let set = format!(
        "{}={token}; Path=/samples/{slug}/; HttpOnly; SameSite=Lax; Max-Age=604800{secure}",
        owner_cookie(&slug)
    );
    let mut r = see_other(&safe_next(&slug, &form.next));
    if let Ok(v) = HeaderValue::from_str(&set) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    Ok(r)
}

async fn sign_out(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    if let Some(token) = cookie(&headers, &owner_cookie(&slug)) {
        app.records()
            .await?
            .end_session(&slug, &token, time::now())
            .await?;
    }
    let set = format!(
        "{}=; Path=/samples/{slug}/; HttpOnly; SameSite=Lax; Max-Age=0",
        owner_cookie(&slug)
    );
    let mut r = see_other("/samples/");
    if let Ok(v) = HeaderValue::from_str(&set) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    Ok(r)
}

// ---------------------------------------------------------------------------------------------
// The registry: one app per page.

struct AppDef {
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    /// The WebAgent whose records these are ("" for Today).
    with: &'static str,
    path: &'static str,
    archetypes: &'static [&'static str],
}

const COMMERCE: &[&str] = &["bakery", "shop"];
const NEWS: &[&str] = &["news"];
const ALL: &[&str] = &["bakery", "shop", "news"];

const APPS: &[AppDef] = &[
    AppDef {
        id: "today",
        name: "Today",
        icon: "today",
        with: "",
        path: "",
        archetypes: ALL,
    },
    AppDef {
        id: "orders",
        name: "Orders",
        icon: "orders",
        with: "Cashier",
        path: "orders",
        archetypes: COMMERCE,
    },
    AppDef {
        id: "custom",
        name: "Custom orders",
        icon: "custom",
        with: "Cashier",
        path: "custom",
        archetypes: COMMERCE,
    },
    AppDef {
        id: "catalogue",
        name: "Catalogue",
        icon: "catalogue",
        with: "Concierge",
        path: "catalogue",
        archetypes: COMMERCE,
    },
    AppDef {
        id: "stock",
        name: "Stock",
        icon: "stock",
        with: "Concierge",
        path: "stock",
        archetypes: COMMERCE,
    },
    AppDef {
        id: "stories",
        name: "Stories",
        icon: "stories",
        with: "Editor",
        path: "stories",
        archetypes: NEWS,
    },
    AppDef {
        id: "subscribers",
        name: "Subscribers",
        icon: "subscribers",
        with: "Cashier",
        path: "subscribers",
        archetypes: NEWS,
    },
    AppDef {
        id: "digest",
        name: "Friday five",
        icon: "digest",
        with: "Editor",
        path: "digest",
        archetypes: NEWS,
    },
    AppDef {
        id: "questions",
        name: "Questions",
        icon: "questions",
        // The Concierge's desks pass these on, and it's the WebAgent that reads them when the
        // Chief asks (agentrix-kernel `chief::sub_tools`).
        with: "Concierge",
        path: "questions",
        archetypes: ALL,
    },
    AppDef {
        id: "customers",
        name: "Customers",
        icon: "customers",
        with: "Sales",
        path: "customers",
        archetypes: ALL,
    },
];

fn app_def(id: &str) -> &'static AppDef {
    APPS.iter().find(|a| a.id == id).unwrap_or(&APPS[0])
}

/// How an app reads on a site, as (name, the WebAgent it works with, the line under the
/// title). The WebAgent named is one on the sample's team whose tools read these records when
/// the Chief WebAgent asks it (agentrix-kernel `roster::sample_team`, `chief::sub_tools`); no
/// WebAgent changes them or works from them on its own yet. A newspaper's customers are its
/// readers, with no WebAgent selling to them; the bakery has no Sales WebAgent on its team; the
/// paper's stories are written and published by people.
fn app_words(def: &AppDef, archetype: &str) -> (&'static str, &'static str, &'static str) {
    match (def.id, archetype) {
        ("customers", "news") => ("Readers", "", ""),
        ("customers", "bakery") => (def.name, "", ""),
        ("stories", "news") => (
            def.name,
            def.with,
            "The Editor WebAgent (AI) reads these stories to answer your Chief WebAgent's questions. It doesn't write or publish them: you do.",
        ),
        _ => (def.name, def.with, ""),
    }
}

fn base(slug: &str) -> String {
    format!("/samples/{slug}/")
}

// ---------------------------------------------------------------------------------------------
// The one-line result after a form: a short-lived cookie read by the next page.

const NOTE: &str = "agx_owner_note";

struct Flash {
    kind: &'static str,
    text: String,
}

fn read_flash(headers: &HeaderMap) -> Option<Flash> {
    let raw = cookie(headers, NOTE)?;
    let text = url::form_urlencoded::parse(format!("m={raw}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())?;
    let (kind, text) = match text.split_once(':') {
        Some(("err", t)) => ("err", t.to_owned()),
        Some((_, t)) => ("ok", t.to_owned()),
        None => ("ok", text),
    };
    (!text.is_empty()).then_some(Flash { kind, text })
}

fn done(to: &str, ok: bool, text: &str) -> Response {
    let text: String = text.chars().take(600).collect();
    let value: String = url::form_urlencoded::byte_serialize(
        format!("{}:{text}", if ok { "ok" } else { "err" }).as_bytes(),
    )
    .collect();
    let cookie = format!("{NOTE}={value}; Path=/samples; HttpOnly; SameSite=Lax; Max-Age=120");
    let mut r = see_other(to);
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

fn finish(mut r: Response, had_flash: bool) -> Response {
    if had_flash {
        r.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_static(
                "agx_owner_note=; Path=/samples; HttpOnly; SameSite=Lax; Max-Age=0",
            ),
        );
    }
    let h = r.headers_mut();
    h.entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    h.entry("x-robots-tag")
        .or_insert(HeaderValue::from_static("noindex"));
    r
}

// ---------------------------------------------------------------------------------------------
// What every page of a site gathers: the records its figures come from.

struct Gathered {
    orders: Vec<OrderView>,
    questions: Vec<Question>,
    stock: HashMap<String, Stock>,
    products: Vec<Product>,
    stories: Vec<Story>,
    subscribers: Vec<Subscriber>,
    digest: Vec<DigestSub>,
    figures: Value,
    needs: Vec<Value>,
}

const TO_FULFIL: [&str; 4] = ["paid", "baking", "ready", "out"];
const UNPAID: [&str; 2] = ["awaiting_payment", "approved"];
const FINISHED: [&str; 2] = ["delivered", "collected"];
const CLOSED: [&str; 4] = ["declined", "lapsed", "cancelled", "refunded"];
const ALERT: [&str; 3] = ["low", "out", "batch"];

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

pub(crate) fn owner_first(site: &Site) -> String {
    let first = site.fact("/owner/first");
    if first.is_empty() {
        "the owner".to_owned()
    } else {
        first
    }
}

async fn gather(records: &Records, site: &Site, now: i64) -> anyhow::Result<Gathered> {
    let off = site.utc_offset_min;
    let today = time::day_of(now, off);
    let questions = records.questions(&site.site).await?;
    let open_q: Vec<&Question> = questions.iter().filter(|q| q.state == "open").collect();
    let slug = &site.site;
    let mut needs = Vec::new();
    let (orders, stock, products, stories, subscribers, digest, figures);
    if site.archetype == "news" {
        orders = Vec::new();
        stock = HashMap::new();
        products = Vec::new();
        stories = records.all_stories(slug).await?;
        subscribers = records.subscribers(slug).await?;
        digest = records.digest_subs(slug).await?;
        let week = stories
            .iter()
            .filter(|s| s.status == "published" && s.published_at >= now - 7 * 86_400)
            .count();
        let drafts = stories.iter().filter(|s| s.status != "published").count();
        let active = subscribers
            .iter()
            .filter(|s| s.state == "active" && s.until_at.is_none_or(|u| u > now))
            .count();
        let confirmed = digest.iter().filter(|d| d.state == "confirmed").count();
        let pending = digest.iter().filter(|d| d.state == "pending").count();
        let corrections = records.corrections(slug, None).await?;
        let corrected_week = corrections
            .iter()
            .filter(|c| c.at >= now - 7 * 86_400)
            .count();
        if drafts > 0 {
            needs.push(json!({
                "title": format!("{} not published", plural(drafts, "story", "stories")),
                "line": "Drafts and unpublished stories. Readers don't see them.",
                "href": format!("{}stories", base(slug)), "verb": "Open Stories", "kind": "Stories", "tone": "wait",
            }));
        }
        figures = json!({
            "stories_week": week, "drafts": drafts, "subscribers": active, "digest_confirmed": confirmed,
            "digest_pending": pending, "corrections_week": corrected_week, "stories": stories.len(),
            "questions_open": open_q.len(), "held": 0,
        });
    } else {
        orders = records.orders_for_owner(site, None, 500).await?;
        stock = records.stock(site, now).await?;
        products = records.all_products(slug).await?;
        stories = Vec::new();
        subscribers = Vec::new();
        digest = Vec::new();
        let is = |v: &OrderView, set: &[&str]| set.contains(&v.order.state.as_str());
        let to_fulfil = orders.iter().filter(|v| is(v, &TO_FULFIL)).count();
        let due_today = orders
            .iter()
            .filter(|v| is(v, &TO_FULFIL))
            .filter(|v| {
                v.slot.as_ref().map_or_else(
                    || time::day_of(v.order.created_at, off),
                    |s| time::day_of(s.starts_at, off),
                ) == today
            })
            .count();
        let unpaid = orders.iter().filter(|v| is(v, &UNPAID)).count();
        let held: Vec<&OrderView> = orders.iter().filter(|v| v.order.state == "held").collect();
        let held_minor: i64 = held.iter().map(|v| v.order.total_minor).sum();
        let placed: Vec<&OrderView> = orders
            .iter()
            .filter(|v| time::day_of(v.order.created_at, off) == today)
            .collect();
        let placed_minor: i64 = placed
            .iter()
            .filter(|v| !CLOSED.contains(&v.order.state.as_str()))
            .map(|v| v.order.total_minor)
            .sum();
        let names: HashMap<&str, &str> = products
            .iter()
            .map(|p| (p.sku.as_str(), p.name.as_str()))
            .collect();
        let mut alerts: Vec<(&str, &Stock)> = stock
            .values()
            .filter(|s| ALERT.contains(&s.state))
            .filter_map(|s| names.get(s.sku.as_str()).map(|n| (*n, s)))
            .collect();
        alerts.sort_by_key(|(n, _)| *n);
        for v in &held {
            let o = &v.order;
            needs.push(json!({
                "title": format!("Approve {} · {}", if o.kind == "custom" { "custom order" } else { "order" }, money::fmt(&o.currency, o.total_minor)),
                "line": format!("{} {} · {} · {}", o.order_id, o.contact_name, v.slot.as_ref().map(|s| format!("{} {}", o.fulfilment, s.label)).unwrap_or_else(|| o.fulfilment.clone()), ago(o.created_at, now, off)),
                "href": format!("{}custom#{}", base(slug), o.order_id), "verb": "Decide", "kind": "Custom order", "tone": "wait",
            }));
        }
        for v in orders.iter().filter(|v| v.cancel_asked().is_some()) {
            let o = &v.order;
            needs.push(json!({
                "title": format!("Cancel request · {} paid", money::fmt(&o.currency, o.total_minor)),
                "line": format!("{} {} asked to cancel {}", o.order_id, o.contact_name, v.cancel_asked().map(|e| ago(e.at, now, off)).unwrap_or_default()),
                "href": format!("{}orders/{}", base(slug), o.order_id), "verb": "Decide", "kind": "Cancel request", "tone": "wait",
            }));
        }
        if !alerts.is_empty() {
            needs.push(json!({
                "title": format!("{} low or sold out", plural(alerts.len(), "product", "products")),
                "line": alerts.iter().map(|(n, s)| format!("{n}: {}", s.left.map_or_else(|| "not kept".into(), |l| format!("{l} left")))).collect::<Vec<_>>().join(" · "),
                "href": format!("{}stock", base(slug)), "verb": "Open Stock", "kind": "Stock", "tone": "risk",
            }));
        }
        figures = json!({
            "to_fulfil": to_fulfil, "due_today": due_today, "unpaid": unpaid, "held": held.len(),
            "held_total": money::fmt(&site.currency, held_minor), "placed_today": placed.len(),
            "placed_today_total": money::fmt(&site.currency, placed_minor), "stock_alerts": alerts.len(),
            "questions_open": open_q.len(), "orders": orders.len(), "products": products.len(),
            "published": products.iter().filter(|p| p.status == "published").count(),
        });
    }
    if let Some(q) = open_q.first() {
        needs.push(json!({
            "title": format!("{} passed to you", plural(open_q.len(), "question", "questions")),
            "line": format!("Latest, at the {} desk: \u{201c}{}\u{201d}", q.desk, excerpt(&q.question, 90)),
            "href": format!("{}questions", base(slug)), "verb": "Answer", "kind": "Teach your site", "tone": "info",
        }));
    }
    Ok(Gathered {
        orders,
        questions,
        stock,
        products,
        stories,
        subscribers,
        digest,
        figures,
        needs,
    })
}

fn excerpt(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_owned();
    }
    let cut: String = t.chars().take(max).collect();
    format!("{}\u{2026}", cut.trim_end())
}

fn ago(ts: i64, now: i64, off: i32) -> String {
    let d = now - ts;
    if (0..60).contains(&d) {
        "just now".into()
    } else if (60..3600).contains(&d) {
        format!("{} min ago", d / 60)
    } else if (3600..6 * 3600).contains(&d) {
        format!("{} h ago", d / 3600)
    } else if time::day_of(ts, off) == time::day_of(now, off) {
        format!("today {}", time::fmt_clock(ts, off))
    } else {
        time::fmt_day_clock(ts, off)
    }
}

// ---------------------------------------------------------------------------------------------
// The chrome: band, bar with the site switcher, dock of apps, lane of what needs the owner.

struct Here<'a> {
    app: &'a App,
    records: &'a Records,
    site: Site,
    def: &'static AppDef,
    now: i64,
    g: Gathered,
    flash: Option<Flash>,
}

enum Loaded<'a> {
    Here(Box<Here<'a>>),
    Missing(Response),
}

async fn load<'a>(
    app: &'a App,
    slug: &str,
    app_id: &str,
    headers: &HeaderMap,
) -> anyhow::Result<Loaded<'a>> {
    let records = app.records().await?;
    let def = app_def(app_id);
    let flash = read_flash(headers);
    let Some(site) = records.site(slug).await? else {
        let words = format!(
            "There's no sample called \u{201c}{slug}\u{201d}. Pick one of the samples below."
        );
        return Ok(Loaded::Missing(
            missing(app, records, &words, StatusCode::NOT_FOUND).await?,
        ));
    };
    if !def.archetypes.contains(&site.archetype.as_str()) {
        let words = format!(
            "{} has no {} app: it runs as a {}. Its apps are in the dock.",
            site.name,
            def.name,
            archetype_words(&site.archetype)
        );
        return Ok(Loaded::Missing(
            missing(app, records, &words, StatusCode::NOT_FOUND).await?,
        ));
    }
    let now = time::now();
    let g = gather(records, &site, now).await?;
    Ok(Loaded::Here(Box::new(Here {
        app,
        records,
        site,
        def,
        now,
        g,
        flash,
    })))
}

fn archetype_words(a: &str) -> &'static str {
    match a {
        "bakery" => "bakery",
        "shop" => "shop",
        "news" => "newspaper",
        _ => "site",
    }
}

fn site_chip(app: &App, site: &Site) -> Value {
    let color = site.fact("/front_color");
    json!({
        "slug": site.site,
        "name": site.name,
        "initial": site.name.chars().next().map(|c| c.to_string()).unwrap_or_default(),
        "color": if color.is_empty() { "#38356d".to_owned() } else { color },
        "domain": site.fact("/domain"),
        "release": site.fact("/release"),
        "owner": owner_first(site),
        "owner_name": site.fact("/owner/name"),
        "url": app.config.origin_for(&site.host),
        "archetype": site.archetype,
        "kind": archetype_words(&site.archetype),
        "currency": site.currency,
    })
}

fn humanize(slug: &str) -> String {
    slug.trim_end_matches("-sample")
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn switcher(records: &Records, current: Option<&str>) -> anyhow::Result<Vec<Value>> {
    let mut out = Vec::new();
    for (slug, _) in SAMPLES {
        let site = records.site(slug).await?;
        out.push(json!({
            "slug": slug,
            "name": site.as_ref().map_or_else(|| humanize(slug), |s| s.name.clone()),
            "kind": site.as_ref().map_or("", |s| archetype_words(&s.archetype)),
            "href": base(slug),
            "loaded": site.is_some(),
            "current": current == Some(slug),
        }));
    }
    Ok(out)
}

impl Here<'_> {
    fn slug(&self) -> &str {
        &self.site.site
    }

    fn off(&self) -> i32 {
        self.site.utc_offset_min
    }

    fn money(&self, minor: i64) -> String {
        money::fmt(&self.site.currency, minor)
    }

    async fn render(self, tpl: &str, title: &str, sub: String, mut page: Value) -> Page {
        let f = &self.g.figures;
        let badge = |id: &str| -> u64 {
            let key = match id {
                "custom" => "held",
                "questions" => "questions_open",
                "stock" => "stock_alerts",
                "stories" => "drafts",
                _ => return 0,
            };
            f.get(key).and_then(Value::as_u64).unwrap_or(0)
        };
        let apps: Vec<Value> = APPS
            .iter()
            .filter(|a| a.archetypes.contains(&self.site.archetype.as_str()))
            .map(|a| {
                json!({
                    "id": a.id, "name": app_words(a, &self.site.archetype).0, "icon": a.icon,
                    "href": format!("{}{}", base(self.slug()), a.path),
                    "current": a.id == self.def.id, "badge": badge(a.id),
                })
            })
            .collect();
        let (name, with, line) = app_words(self.def, &self.site.archetype);
        let had = self.flash.is_some();
        let c = json!({
            "v": agentrix_themes::VERSION,
            "title": title,
            "sub": sub,
            "app": { "id": self.def.id, "name": name, "icon": self.def.icon, "with": with, "line": line },
            "site": site_chip(self.app, &self.site),
            "base": base(self.slug()),
            "sites": switcher(self.records, Some(self.slug())).await?,
            "apps": apps,
            "needs": self.g.needs,
            "figures": self.g.figures,
            "flash": self.flash.as_ref().map(|f| json!({ "kind": f.kind, "text": f.text })),
            "today": time::fmt_date_long(self.now, self.off()),
            "clock": time::fmt_clock(self.now, self.off()),
        });
        page["c"] = c;
        let body = self.app.render(tpl, &page)?;
        Ok(finish(html(body), had))
    }
}

async fn missing(
    app: &App,
    records: &Records,
    words: &str,
    status: StatusCode,
) -> anyhow::Result<Response> {
    let view = json!({
        "words": words,
        "c": {
            "v": agentrix_themes::VERSION, "title": "Not here", "sub": "", "site": null, "base": "/samples/",
            "app": { "id": "missing", "name": "Sample owner views", "icon": "today", "with": "" },
            "sites": switcher(records, None).await?, "apps": [], "needs": [], "figures": {}, "flash": null,
            "today": "", "clock": "",
        },
    });
    let body = app.render("owner/missing.html", &view)?;
    let mut r = finish(html(body), false);
    *r.status_mut() = status;
    Ok(r)
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

async fn to_site(Path(slug): Path<String>) -> Response {
    see_other(&base(&slug))
}

// ---------------------------------------------------------------------------------------------
// GET /samples/: the three samples with today's figures.

async fn index(State(app): State<App>, headers: HeaderMap) -> Page {
    let records = app.records().await?;
    let now = time::now();
    let flash = read_flash(&headers);
    let mut cards = Vec::new();
    for (slug, _) in SAMPLES {
        let Some(site) = records.site(slug).await? else {
            cards.push(json!({ "loaded": false, "name": humanize(slug), "slug": slug }));
            continue;
        };
        let g = gather(records, &site, now).await?;
        let apps: Vec<Value> = APPS
            .iter()
            .filter(|a| a.id != "today" && a.archetypes.contains(&site.archetype.as_str()))
            .map(|a| json!({ "name": app_words(a, &site.archetype).0, "href": format!("{}{}", base(slug), a.path) }))
            .collect();
        cards.push(json!({
            "loaded": true, "slug": slug, "site": site_chip(&app, &site), "figures": g.figures,
            "apps": apps, "href": base(slug), "needs": g.needs.len(),
            "asof": time::fmt_clock(now, site.utc_offset_min),
        }));
    }
    let had = flash.is_some();
    let view = json!({
        "cards": cards,
        "c": {
            "v": agentrix_themes::VERSION, "title": "Sample owner views", "sub": "", "site": null, "base": "/samples/",
            "app": { "id": "index", "name": "Sample owner views", "icon": "today", "with": "" },
            "sites": switcher(records, None).await?, "apps": [], "needs": [], "figures": {},
            "flash": flash.as_ref().map(|f| json!({ "kind": f.kind, "text": f.text })), "today": "", "clock": "",
        },
    });
    Ok(finish(html(app.render("owner/index.html", &view)?), had))
}

// ---------------------------------------------------------------------------------------------
// Today.

async fn today(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "today", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let off = here.off();
    let latest: Vec<Value> = here
        .g
        .orders
        .iter()
        .take(6)
        .map(|v| order_row(&here.site, v, here.now))
        .collect();
    let authors: HashMap<String, String> = if here.site.archetype == "news" {
        here.records
            .authors(here.slug())
            .await?
            .into_iter()
            .map(|a| (a.author_id, a.name))
            .collect()
    } else {
        HashMap::new()
    };
    let stories: Vec<Value> = here
        .g
        .stories
        .iter()
        .take(6)
        .map(|s| story_row(&here, s, &authors, &HashMap::new()))
        .collect();
    let mut summary = if here.site.archetype == "news" {
        news_summary(&here)
    } else {
        shop_summary(&here)
    };
    // "Was this right?" on the desks' answers, last 7 days.
    let marks = here
        .records
        .feedback(here.slug(), here.now - 7 * 86_400)
        .await?;
    let right = marks.iter().filter(|m| m.right()).count();
    summary.push(match (right, marks.len() - right) {
        (0, 0) => "No one marked a desk's answer right or wrong in the last 7 days.".into(),
        (y, 0) => format!(
            "Visitors marked {} right in the last 7 days.",
            plural(y, "answer", "answers")
        ),
        (0, n) => format!(
            "Visitors marked {} wrong in the last 7 days. Teach the right answers in Questions.",
            plural(n, "answer", "answers")
        ),
        (y, n) => format!(
            "Visitors marked {} right and {} wrong in the last 7 days. Teach the right answers in Questions.",
            plural(y, "answer", "answers"),
            n
        ),
    });
    // Visits counted on the site today (W8's counter: by page only, no cookies).
    let span = agentrix_store::time::Window::Today.span(chrono::Utc::now(), off);
    if let Ok(report) = agentrix_store::visits::VisitStore::new(app.control.clone())
        .report(&agentrix_types::SiteId::new(here.slug()), span, off)
        .await
    {
        summary.push(report.words());
    }
    let sub = format!(
        "{} · as of {} · Sample",
        time::fmt_date_long(here.now, off),
        time::fmt_clock(here.now, off)
    );
    let title = format!("Today at {}", here.site.name);
    here.render(
        "owner/today.html",
        &title,
        sub,
        json!({ "latest": latest, "stories": stories, "summary": summary }),
    )
    .await
}

fn shop_summary(here: &Here<'_>) -> Vec<String> {
    let f = &here.g.figures;
    let n = |k: &str| f.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    let s = |k: &str| {
        f.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut out = vec![format!(
        "{} to fulfil, {} due today.",
        plural(n("to_fulfil"), "order", "orders"),
        n("due_today")
    )];
    out.push(if n("held") > 0 {
        format!(
            "{} {} for your OK ({}). Nothing is charged until you approve.",
            plural(n("held"), "custom order", "custom orders"),
            if n("held") == 1 { "waits" } else { "wait" },
            s("held_total")
        )
    } else {
        "No custom orders wait for you.".into()
    });
    out.push(format!(
        "{} placed today, {}.",
        plural(n("placed_today"), "order", "orders"),
        s("placed_today_total")
    ));
    if n("unpaid") > 0 {
        out.push(format!(
            "{} waiting for the customer to pay.",
            plural(n("unpaid"), "order is", "orders are")
        ));
    }
    out.push(match n("stock_alerts") {
        0 => "Nothing is low or sold out.".into(),
        k => format!("{} low or sold out.", plural(k, "product", "products")),
    });
    out.push(match n("questions_open") {
        0 => "No questions waiting.".into(),
        k => format!(
            "{} passed to you by the desks.",
            plural(k, "question", "questions")
        ),
    });
    out
}

fn news_summary(here: &Here<'_>) -> Vec<String> {
    let f = &here.g.figures;
    let n = |k: &str| f.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
    vec![
        format!(
            "{} published in the last 7 days; {} not published.",
            plural(n("stories_week"), "story", "stories"),
            n("drafts")
        ),
        format!(
            "{} with an active subscription.",
            plural(n("subscribers"), "reader", "readers")
        ),
        format!(
            "{} confirmed for the Friday five, {} waiting to confirm.",
            plural(n("digest_confirmed"), "address", "addresses"),
            n("digest_pending")
        ),
        format!(
            "{} in the last 7 days.",
            plural(n("corrections_week"), "correction", "corrections")
        ),
        match n("questions_open") {
            0 => "No questions waiting.".into(),
            k => format!(
                "{} passed to you by the desks.",
                plural(k, "question", "questions")
            ),
        },
    ]
}

// ---------------------------------------------------------------------------------------------
// Orders.

fn state_label(state: &str) -> &'static str {
    match state {
        "awaiting_payment" => "Waiting for payment",
        "held" => "Waiting for you",
        "approved" => "Approved, unpaid",
        "paid" => "Paid",
        "baking" => "Baking",
        "ready" => "Ready",
        "out" => "Out for delivery",
        "delivered" => "Delivered",
        "collected" => "Collected",
        "declined" => "Declined",
        "lapsed" => "Lapsed",
        "cancelled" => "Cancelled",
        "refunded" => "Refunded",
        _ => "In progress",
    }
}

fn state_tone(state: &str) -> &'static str {
    match state {
        "held" => "wait",
        "awaiting_payment" | "approved" | "out" => "info",
        "paid" | "ready" => "ok",
        "baking" => "coral",
        "declined" | "lapsed" | "cancelled" | "refunded" => "risk",
        _ => "",
    }
}

fn items_line(v: &OrderView) -> String {
    if v.order.kind == "custom" {
        let parts: Vec<&str> = v.lines.iter().map(|l| l.name.as_str()).collect();
        return format!("Custom: {}", parts.join(", "));
    }
    let mut parts: Vec<String> = v
        .lines
        .iter()
        .take(3)
        .map(|l| format!("{} \u{d7} {}", l.qty, l.name))
        .collect();
    if v.lines.len() > 3 {
        parts.push(format!("{} more", v.lines.len() - 3));
    }
    parts.join(", ")
}

fn order_row(site: &Site, v: &OrderView, now: i64) -> Value {
    let o = &v.order;
    let off = site.utc_offset_min;
    json!({
        "id": o.order_id,
        "href": format!("{}orders/{}", base(&site.site), o.order_id),
        "state": o.state, "label": state_label(&o.state), "tone": state_tone(&o.state),
        "kind": o.kind, "channel": o.channel, "agent": o.agent.clone().unwrap_or_default(),
        "name": o.contact_name, "total": money::fmt(&o.currency, o.total_minor),
        "items": items_line(v),
        "slot": v.slot.as_ref().map(|s| s.label.clone()).unwrap_or_default(),
        "fulfilment": if o.fulfilment == "delivery" { "Delivery" } else { "Collection" },
        "placed": ago(o.created_at, now, off),
        "sample": o.sample != 0,
    })
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    state: Option<String>,
}

const FILTERS: [(&str, &str, &[&str]); 6] = [
    ("all", "All", &[]),
    ("held", "Waiting for you", &["held"]),
    ("unpaid", "Waiting for payment", &UNPAID),
    ("open", "To fulfil", &TO_FULFIL),
    ("done", "Done", &FINISHED),
    ("closed", "Closed", &CLOSED),
];

async fn orders(
    State(app): State<App>,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Page {
    let here = match load(&app, &slug, "orders", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let pick = q
        .state
        .as_deref()
        .filter(|s| FILTERS.iter().any(|(id, _, _)| id == s))
        .unwrap_or("all");
    let in_filter = |v: &OrderView, id: &str| {
        let set = FILTERS
            .iter()
            .find(|(f, _, _)| *f == id)
            .map_or(&[][..], |(_, _, s)| *s);
        id == "all" || set.contains(&v.order.state.as_str())
    };
    let tabs: Vec<Value> = FILTERS
        .iter()
        .map(|(id, label, _)| {
            json!({
                "id": id, "label": label, "count": here.g.orders.iter().filter(|v| in_filter(v, id)).count(),
                "href": if *id == "all" { format!("{}orders", base(&slug)) } else { format!("{}orders?state={id}", base(&slug)) },
                "current": *id == pick,
            })
        })
        .collect();
    let rows: Vec<Value> = here
        .g
        .orders
        .iter()
        .filter(|v| in_filter(v, pick))
        .map(|v| order_row(&here.site, v, here.now))
        .collect();
    let label = FILTERS
        .iter()
        .find(|(id, _, _)| *id == pick)
        .map_or("All", |(_, l, _)| *l);
    let sub = format!(
        "{} · newest first · as of {}",
        plural(rows.len(), "order", "orders"),
        time::fmt_clock(here.now, here.off())
    );
    let empty = match pick {
        "held" => "No orders wait for you. Custom orders are held here until you approve; nothing is charged before that.".to_owned(),
        "unpaid" => "No orders are waiting for payment. Approved custom orders and unpaid checkouts show here.".to_owned(),
        "open" => "Nothing to fulfil right now. Paid orders appear here the moment the Cashier takes payment.".to_owned(),
        "done" => "No delivered or collected orders yet.".to_owned(),
        "closed" => "No declined, lapsed, cancelled or refunded orders.".to_owned(),
        _ => "No orders yet. They appear here the moment someone orders on the site or through the agent door.".to_owned(),
    };
    here.render(
        "owner/orders.html",
        "Orders",
        sub,
        json!({ "tabs": tabs, "rows": rows, "filter": label, "empty": empty }),
    )
    .await
}

/// The steps an order can take from its state, in the order the owner would press them.
fn steps_for(site: &Site, v: &OrderView) -> Vec<(&'static str, &'static str)> {
    let delivery = v.order.fulfilment == "delivery";
    let bakery = site.archetype == "bakery";
    let mut out = Vec::new();
    if v.cancel_asked().is_some() {
        // The customer asked to cancel before it's made: the owner decides that first.
        out.push(("cancel_keep", "Keep the order"));
        out.push(("cancel_refund", "Cancel and refund"));
        return out;
    }
    match v.order.state.as_str() {
        "paid" => {
            if bakery {
                out.push(("baking", "Baking"));
            }
            out.push(if delivery {
                ("out", "Out for delivery")
            } else {
                ("ready", "Ready")
            });
        }
        "baking" => out.push(if delivery {
            ("out", "Out for delivery")
        } else {
            ("ready", "Ready")
        }),
        "ready" => out.push(if delivery {
            ("out", "Out for delivery")
        } else {
            ("collected", "Collected")
        }),
        "out" => out.push(("delivered", "Delivered")),
        "held" | "approved" | "awaiting_payment" => out.push(("cancelled", "Cancel")),
        _ => {}
    }
    out
}

fn step_words(step: &str) -> &'static str {
    match step {
        "baking" => "marked baking",
        "ready" => "marked ready to collect",
        "out" => "marked out for delivery",
        "delivered" => "marked delivered",
        "collected" => "marked collected",
        "cancelled" => "cancelled; nothing was charged",
        _ => "moved on",
    }
}

async fn order(
    State(app): State<App>,
    Path((slug, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Page {
    let here = match load(&app, &slug, "orders", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let Some(v) = here.records.order(&here.site, &id).await? else {
        let words = format!(
            "There's no order {} on {}. Check the number in Orders.",
            id.to_uppercase(),
            here.site.name
        );
        let mut r = here
            .render(
                "owner/missing_in.html",
                "No such order",
                String::new(),
                json!({ "words": words, "back": format!("{}orders", base(&slug)) }),
            )
            .await?;
        *r.status_mut() = StatusCode::NOT_FOUND;
        return Ok(r);
    };
    let detail = order_detail(&here, &v).await?;
    let o = &v.order;
    let sub = format!(
        "{} · placed {} · {}",
        state_label(&o.state),
        ago(o.created_at, here.now, here.off()),
        money::fmt(&o.currency, o.total_minor)
    );
    let title = format!("Order {}", o.order_id);
    here.render("owner/order.html", &title, sub, json!({ "o": detail }))
        .await
}

async fn order_detail(here: &Here<'_>, v: &OrderView) -> anyhow::Result<Value> {
    let o = &v.order;
    let off = here.off();
    let cur = &o.currency;
    let lists = here.records.price_lists(here.slug()).await?;
    let live = lists
        .iter()
        .find(|l| l.status == "live")
        .map(|l| l.version.clone())
        .unwrap_or_default();
    let pinned = o
        .price_list
        .as_ref()
        .and_then(|p| lists.iter().find(|l| &l.version == p));
    let spec = o.spec();
    let past = o.customer_id.as_ref().map_or(0, |c| {
        here.g
            .orders
            .iter()
            .filter(|x| x.order.customer_id.as_ref() == Some(c) && x.order.order_id != o.order_id)
            .count()
    });
    let usd = (o.usd_rate_minor > 0).then(|| money::usd(o.usd_cents()));
    let zone = match &o.zone {
        Some(z) => here
            .records
            .zones(here.slug())
            .await?
            .into_iter()
            .find(|x| &x.zone == z)
            .map_or_else(|| z.clone(), |x| x.label),
        None => String::new(),
    };
    // A custom order carries its delivery as a rule line; it shows once, as the delivery row.
    let lines = v.lines.iter().filter(|l| {
        !(o.delivery_minor > 0
            && l.rule_code
                .as_deref()
                .is_some_and(|r| r.starts_with("delivery.")))
    });
    Ok(json!({
        "id": o.order_id, "state": o.state, "label": state_label(&o.state), "tone": state_tone(&o.state),
        "kind": o.kind, "channel": o.channel, "agent": o.agent.clone().unwrap_or_default(),
        "fulfilment": if o.fulfilment == "delivery" { "Delivery" } else { "Collection" },
        "zone": zone,
        "name": o.contact_name, "email": o.contact_email.clone().unwrap_or_default(),
        "phone": o.contact_phone.clone().unwrap_or_default(), "recipient": o.recipient.clone().unwrap_or_default(),
        "note": o.note.clone().unwrap_or_default(),
        "message": spec.as_ref().and_then(|s| s.get("message")).and_then(Value::as_str).unwrap_or_default(),
        "spec_note": spec.as_ref().and_then(|s| s.get("note")).and_then(Value::as_str).unwrap_or_default(),
        // The idea of the cake the customer saw in the studio (ADR-032), from the site's host.
        "idea": spec.as_ref().and_then(|s| s.get("idea")).and_then(Value::as_str)
            .filter(|u| u.starts_with("/media/ideas/"))
            .map(|u| format!("{}{u}", here.app.config.origin_for(&here.site.host)))
            .unwrap_or_default(),
        "past": past,
        "placed": time::fmt_day_clock(o.created_at, off), "placed_ago": ago(o.created_at, here.now, off),
        "lines": lines.map(|l| json!({
            "name": l.name, "qty": l.qty, "unit": money::fmt(cur, l.unit_minor), "rule": l.rule_code.clone().unwrap_or_default(),
            "amount": money::fmt(cur, l.amount_minor),
        })).collect::<Vec<_>>(),
        "subtotal": money::fmt(cur, o.subtotal_minor), "delivery": money::fmt(cur, o.delivery_minor),
        "has_delivery": o.delivery_minor > 0, "total": money::fmt(cur, o.total_minor),
        "usd": usd.unwrap_or_default(), "rate": if o.usd_rate_minor > 0 { money::rate_line(o.usd_rate_minor) } else { String::new() },
        "price_list": o.price_list.clone().unwrap_or_default(), "live_list": live,
        "list_source": pinned.map(|p| p.source.clone()).unwrap_or_default(),
        "stale_list": o.price_list.as_ref().is_some_and(|p| !live.is_empty() && *p != live),
        "valid_until": o.quote_valid_until.map(|t| time::fmt_day_clock(t, off)).unwrap_or_default(),
        "hold_ended": o.quote_valid_until.is_some_and(|t| t < here.now),
        "slot": v.slot.as_ref().map(|s| json!({ "label": s.label, "taken": s.taken, "capacity": s.capacity, "full": s.full, "kind": s.kind })),
        "events": v.events.iter().map(|e| json!({
            "at": time::fmt_day_clock(e.at, off), "step": e.step, "label": event_label(&e.step), "actor": e.actor,
            "note": e.note.clone().unwrap_or_default(), "tone": state_tone(&e.step),
        })).collect::<Vec<_>>(),
        "payments": v.payments.iter().map(|p| json!({
            "id": p.payment_id, "rail": rail_words(&p.rail), "amount": money::fmt(&p.currency, p.amount_minor), "state": p.state,
            "sample": p.state == "sample", "at": time::fmt_day_clock(p.settled_at.unwrap_or(p.created_at), off),
        })).collect::<Vec<_>>(),
        "refunds": v.refunds.iter().map(|r| json!({
            "id": r.refund_id, "amount": money::fmt(cur, r.amount_minor), "reason": r.reason, "rule": r.rule.clone().unwrap_or_default(),
            "rule_words": rule_words(&here.site, r.rule.as_deref().unwrap_or_default()),
            "state": r.state, "at": time::fmt_day_clock(r.created_at, off),
        })).collect::<Vec<_>>(),
        "steps": steps_for(&here.site, v).into_iter().map(|(step, label)| json!({ "step": step, "label": label, "danger": step == "cancelled" || step == "cancel_refund" })).collect::<Vec<_>>(),
        "cancel_asked": v.cancel_asked().map(|e| json!({ "who": e.actor, "when": ago(e.at, here.now, off) })),
        "customer_page": format!("{}/orders/{}", here.app.config.origin_for(&here.site.host), o.order_id),
        "href": format!("{}orders/{}", base(here.slug()), o.order_id),
        "sample": o.sample != 0,
    }))
}

fn event_label(step: &str) -> String {
    match step {
        "requested" => "Requested".into(),
        "approved" => "Approved".into(),
        s => state_label(s).to_owned(),
    }
}

/// A refund rule in the owner's words: "Refunds up to KSh 2,000 an order".
fn rule_words(site: &Site, rule: &str) -> String {
    let refund = site.facts.get("refund_rule");
    let id = refund
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let per_order = refund
        .and_then(|r| r.get("per_order"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if !rule.is_empty() && rule == id && per_order > 0.0 {
        format!(
            "Refunds up to {} an order",
            money::fmt(&site.currency, (per_order * 100.0).round() as i64)
        )
    } else {
        rule.to_owned()
    }
}

/// Where a subscription came from, in the owner's words.
fn source_words(source: &str) -> String {
    match source {
        "owner grant" => "Given by you".into(),
        "sandbox payment" => "PayPal test mode".into(),
        "sample payment" => "Sample payment, no money moved".into(),
        other => other.to_owned(),
    }
}

fn rail_words(rail: &str) -> String {
    // The samples take PayPal only (James, 8 Nov).
    match rail {
        "paypal" => "PayPal".into(),
        other => other.to_owned(),
    }
}

#[derive(Deserialize, Default)]
struct StepForm {
    #[serde(default)]
    step: String,
}

async fn advance(
    State(app): State<App>,
    Path((slug, id)): Path<(String, String)>,
    Form(form): Form<StepForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}orders/{}", base(&slug), id.to_uppercase());
    let owner = owner_first(&site);
    if let step @ ("cancel_keep" | "cancel_refund") = form.step.trim() {
        return decide_cancel(&app, records, &site, &id, step == "cancel_refund", &back).await;
    }
    match records
        .advance(&site, &id, form.step.trim(), &owner, time::now())
        .await
    {
        Ok(v) => {
            let (host, origin) = app.site_origin(&site);
            if let Some(label) = crate::notify::status_label(&v.order.state, &v.order.fulfilment) {
                let event = agentrix_messages::orders::OrderEvent::Status {
                    label,
                    by: owner.clone(),
                    at: time::fmt_clock(time::now(), site.utc_offset_min),
                    note: None,
                };
                app.tell_customer(&site, &host, &origin, &v, event).await;
            }
            let text = format!(
                "{} {} by {owner}. {} sees it on the order page.",
                v.order.order_id,
                step_words(&v.order.state),
                v.order.contact_name
            );
            Ok(done(&back, true, &text))
        }
        Err(OrderError::Refused(words)) => Ok(done(&back, false, &words)),
        Err(OrderError::Other(e)) => Err(e.into()),
    }
}

/// The owner decides a customer's request to cancel a paid order before it's made: cancel and
/// refund the whole amount left, the way it was paid, or keep the order.
async fn decide_cancel(
    app: &App,
    records: &Records,
    site: &Site,
    id: &str,
    cancel: bool,
    back: &str,
) -> Page {
    let owner = owner_first(site);
    let now = time::now();
    let Some(v) = records.order(site, &id.to_uppercase()).await? else {
        return Ok(done(back, false, "There's no order with that number."));
    };
    if v.cancel_asked().is_none() {
        return Ok(done(
            back,
            false,
            &format!(
                "{} has no cancel request waiting any more.",
                v.order.order_id
            ),
        ));
    }
    let o = &v.order;
    let (host, origin) = app.site_origin(site);
    if !cancel {
        records
            .note_event(
                &site.site,
                &o.order_id,
                "cancel_kept",
                &owner,
                &format!("{owner} kept the order. It goes ahead as planned."),
                now,
            )
            .await?;
        return Ok(done(
            back,
            true,
            &format!(
                "{} kept by {owner}. {} sees it on the order page.",
                o.order_id, o.contact_name
            ),
        ));
    }
    let taken: i64 = v
        .refunds
        .iter()
        .filter(|r| r.state != "declined")
        .map(|r| r.amount_minor)
        .sum();
    let amount = (o.total_minor - taken).max(0);
    let digits: String = o.order_id.chars().filter(char::is_ascii_digit).collect();
    let refund = agentrix_records::orders::Refund {
        refund_id: format!("R-{digits}-{}", v.refunds.len() + 1),
        order_id: o.order_id.clone(),
        amount_minor: amount,
        reason: "Cancelled before it was made, at the customer's request.".into(),
        rule: None,
        state: "done".into(),
        created_at: now,
    };
    let rail = v
        .payments
        .iter()
        .find(|p| matches!(p.state.as_str(), "captured" | "sample"))
        .map_or_else(|| "paypal".to_owned(), |p| p.rail.clone());
    let usd = (rail == "paypal" && o.usd_rate_minor > 0)
        .then(|| money::usd(money::usd_cents(amount, o.usd_rate_minor)));
    let paid = usd
        .clone()
        .unwrap_or_else(|| money::fmt(&o.currency, amount));
    let note = format!(
        "Cancelled by {owner} before it was made. Refunded {paid} the way it was paid. Receipt {}.",
        refund.refund_id
    );
    match records.cancel_paid(site, &refund, &owner, &note, now).await {
        Ok(v) => {
            let event = agentrix_messages::orders::OrderEvent::Refunded {
                refund_id: refund.refund_id.clone(),
                amount: paid.clone(),
                local: usd
                    .is_some()
                    .then(|| format!("{} at the order's rate", money::fmt(&o.currency, amount))),
                by: owner.clone(),
                what: "the whole order".into(),
            };
            app.tell_customer(site, &host, &origin, &v, event).await;
            Ok(done(
                back,
                true,
                &format!(
                    "{} cancelled by {owner} and {paid} refunded. {} sees it on the order page.",
                    o.order_id, o.contact_name
                ),
            ))
        }
        Err(OrderError::Refused(words)) => Ok(done(back, false, &words)),
        Err(OrderError::Other(e)) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Custom orders: held requests as decision cards.

async fn custom(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "custom", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let mut held = Vec::new();
    let mut decided = Vec::new();
    for v in here
        .g
        .orders
        .iter()
        .filter(|v| v.order.kind == "custom" || v.order.state == "held")
    {
        if v.order.state == "held" {
            held.push(order_detail(&here, v).await?);
        } else if decided.len() < 8 {
            let decision = v
                .events
                .iter()
                .rev()
                .find(|e| e.step == "approved" || e.step == "declined");
            decided.push(json!({
                "row": order_row(&here.site, v, here.now),
                "decided": decision.map(|e| format!("{} by {}, {}", event_label(&e.step), e.actor, time::fmt_day_clock(e.at, here.off()))).unwrap_or_default(),
            }));
        }
    }
    let live = here.records.live_price_list(here.slug()).await?;
    let sub = match held.len() {
        0 => "Nothing waits for you".to_owned(),
        n => format!(
            "{} for your OK · nothing is charged until you approve",
            plural(n, "request waits", "requests wait")
        ),
    };
    let rule = json!({
        "list": live.as_ref().map(|l| l.version.clone()).unwrap_or_default(),
        "list_source": live.as_ref().map(|l| l.source.clone()).unwrap_or_default(),
        "rate": live.as_ref().map(|l| money::rate_line(l.usd_rate_minor)).unwrap_or_default(),
        "valid_until": live.as_ref().map(|l| time::fmt_day_clock(l.valid_until, here.off())).unwrap_or_default(),
    });
    here.render(
        "owner/custom.html",
        "Custom orders",
        sub,
        json!({ "held": held, "decided": decided, "live": rule }),
    )
    .await
}

#[derive(Deserialize, Default)]
struct DecideForm {
    #[serde(default)]
    decision: String,
    #[serde(default)]
    note: String,
}

async fn decide(
    State(app): State<App>,
    Path((slug, id)): Path<(String, String)>,
    Form(form): Form<DecideForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}custom", base(&slug));
    let approve = match form.decision.as_str() {
        "approve" => true,
        "decline" => false,
        _ => return Ok(done(&back, false, "Choose Approve or Decline.")),
    };
    let owner = owner_first(&site);
    let note: String = form.note.trim().chars().take(500).collect();
    let note = (!note.is_empty()).then_some(note);
    match crate::held::decide_and_tell(&app, &site, &id, approve, &owner, note.as_deref()).await {
        Ok(v) => {
            let o = &v.order;
            let text = if approve {
                format!(
                    "{} approved by {owner}. {} sees a payment request for {} on the order page.",
                    o.order_id,
                    o.contact_name,
                    money::fmt(&o.currency, o.total_minor)
                )
            } else if note.is_some() {
                format!(
                    "{} declined by {owner}. Nothing was charged. {} sees your note on the order page.",
                    o.order_id, o.contact_name
                )
            } else {
                format!(
                    "{} declined by {owner}. Nothing was charged. {} sees that it was declined on the order page.",
                    o.order_id, o.contact_name
                )
            };
            Ok(done(&back, true, &text))
        }
        Err(OrderError::Refused(words)) => Ok(done(&back, false, &words)),
        Err(OrderError::Other(e)) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Catalogue.

async fn catalogue(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "catalogue", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let rows: Vec<Value> = here
        .g
        .products
        .iter()
        .map(|p| {
            json!({
                "sku": p.sku, "slug": p.slug, "name": p.name, "section": p.section, "size": p.size.clone().unwrap_or_default(),
                "family": p.family.clone().unwrap_or_default(), "price": here.money(p.price_minor),
                "price_input": price_input(p.price_minor), "status": p.status, "published": p.status == "published",
                "href": format!("{}catalogue/{}", base(here.slug()), p.sku),
                "page": format!("{}/{}/{}", here.app.config.origin_for(&here.site.host), if here.site.archetype == "shop" { "fragrances" } else { "shop" }, p.slug),
            })
        })
        .collect();
    let f = &here.g.figures;
    let sub = format!(
        "{} · {} published · prices in {}",
        plural(rows.len(), "product", "products"),
        f.get("published").and_then(Value::as_u64).unwrap_or(0),
        here.site.currency
    );
    here.render(
        "owner/catalogue.html",
        "Catalogue",
        sub,
        json!({ "rows": rows }),
    )
    .await
}

fn price_input(minor: i64) -> String {
    if minor % 100 == 0 {
        (minor / 100).to_string()
    } else {
        format!("{}.{:02}", minor / 100, minor % 100)
    }
}

/// "480", "KSh 4,800", "12.50" → minor units.
fn parse_price(text: &str) -> Option<i64> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let (whole, frac) = cleaned.split_once('.').unwrap_or((&cleaned, ""));
    if whole.is_empty() || frac.len() > 2 || frac.contains('.') {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let frac: i64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<2}").parse().ok()?
    };
    let minor = whole.checked_mul(100)?.checked_add(frac)?;
    (1..=100_000_000).contains(&minor).then_some(minor)
}

#[derive(Deserialize, Default)]
struct ProductForm {
    #[serde(default)]
    price: String,
    #[serde(default)]
    status: String,
}

async fn update_product(
    State(app): State<App>,
    Path((slug, sku)): Path<(String, String)>,
    Form(form): Form<ProductForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}catalogue#p-{sku}", base(&slug));
    let Some(p) = records.product(&site.site, &sku).await? else {
        return Ok(done(
            &back,
            false,
            "That product isn't in the catalogue any more.",
        ));
    };
    let Some(price) = parse_price(&form.price) else {
        return Ok(done(
            &back,
            false,
            &format!(
                "{}: write the price as a number, like {}.",
                p.name,
                price_input(p.price_minor)
            ),
        ));
    };
    let status = match form.status.as_str() {
        "published" | "draft" => form.status.as_str(),
        _ => p.status.as_str(),
    };
    records
        .update_product(&site.site, &sku, price, status)
        .await?;
    let owner = owner_first(&site);
    let fmt = |m: i64| money::fmt(&site.currency, m);
    let mut parts = Vec::new();
    if price != p.price_minor {
        parts.push(format!(
            "now costs {} (was {})",
            fmt(price),
            fmt(p.price_minor)
        ));
    }
    if status != p.status {
        parts.push(if status == "published" {
            "is published, so the site lists it".to_owned()
        } else {
            "is a draft, so the site no longer lists it".to_owned()
        });
    }
    let text = if parts.is_empty() {
        format!("{}: nothing changed.", p.name)
    } else {
        format!(
            "{} {}. Changed by {owner}; the site shows it on the next page load.",
            p.name,
            parts.join(" and ")
        )
    };
    Ok(done(&back, true, &text))
}

// ---------------------------------------------------------------------------------------------
// Stock.

async fn stock(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "stock", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let off = here.off();
    let names: HashMap<&str, &str> = here
        .g
        .products
        .iter()
        .map(|p| (p.sku.as_str(), p.name.as_str()))
        .collect();
    let rows: Vec<Value> = here
        .g
        .products
        .iter()
        .map(|p| {
            let s = here.g.stock.get(&p.sku);
            json!({
                "sku": p.sku, "name": p.name, "draft": p.status != "published",
                "kept": s.is_some(),
                "left": s.and_then(|s| s.left).unwrap_or(0),
                "state": s.map_or("none", |s| s.state),
                "label": s.map_or("Not kept", |s| stock_label(s.state)),
                "tone": s.map_or("", |s| match s.state { "in" => "ok", "low" | "batch" => "wait", "out" => "risk", _ => "" }),
                "next_batch": s.and_then(|s| s.next_batch_at).map(|t| time::fmt_clock(t, off)).unwrap_or_default(),
                "back": s.and_then(|s| s.back_at).map(|t| time::fmt_day_clock(t, off)).unwrap_or_default(),
                "as_of": s.map(|s| ago(s.as_of, here.now, off)).unwrap_or_default(),
                "href": format!("{}stock/{}", base(here.slug()), p.sku),
            })
        })
        .collect();
    let holds: Vec<Value> = here
        .records
        .holds(&here.site, here.now)
        .await?
        .iter()
        .map(|h| {
            json!({
                "id": h.hold_id, "product": names.get(h.sku.as_str()).copied().unwrap_or(h.sku.as_str()), "qty": h.qty,
                "name": h.name, "until": time::fmt_clock(h.until_at, off), "state": h.state, "grant": h.grant_id,
                "placed": ago(h.created_at, here.now, off),
            })
        })
        .collect();
    let grant = here.site.fact("/hold_grant/words");
    let bakery = here.site.archetype == "bakery";
    let sub = if bakery {
        format!(
            "Today's bake, {}. Left means what you planned, minus today's orders and holds.",
            time::fmt_day(here.now, off)
        )
    } else {
        "Left means the last count, minus orders placed since.".to_owned()
    };
    here.render(
        "owner/stock.html",
        "Stock",
        sub,
        json!({ "rows": rows, "holds": holds, "grant": grant, "bakery": bakery }),
    )
    .await
}

fn stock_label(state: &str) -> &'static str {
    match state {
        "in" => "In stock",
        "low" => "Low",
        "batch" => "Low, batch coming",
        "out" => "Sold out",
        _ => "Not kept",
    }
}

#[derive(Deserialize, Default)]
struct CountForm {
    #[serde(default)]
    count: String,
}

async fn count_stock(
    State(app): State<App>,
    Path((slug, sku)): Path<(String, String)>,
    Form(form): Form<CountForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}stock#s-{sku}", base(&slug));
    let Some(p) = records.product(&site.site, &sku).await? else {
        return Ok(done(
            &back,
            false,
            "That product isn't in the catalogue any more.",
        ));
    };
    let Some(count) = form
        .count
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|c| (0..=10_000).contains(c))
    else {
        return Ok(done(
            &back,
            false,
            &format!(
                "{}: write how many you have now, as a whole number.",
                p.name
            ),
        ));
    };
    let now = time::now();
    records.set_stock(&site, &sku, count, now).await?;
    let left = records
        .stock(&site, now)
        .await?
        .get(&sku)
        .and_then(|s| s.left)
        .unwrap_or(count);
    let text = format!(
        "{} counted by {}: {} left, as of {}. The site and its desks show the new count now.",
        p.name,
        owner_first(&site),
        left,
        time::fmt_clock(now, site.utc_offset_min)
    );
    Ok(done(&back, true, &text))
}

// ---------------------------------------------------------------------------------------------
// Questions passed to the owner.

async fn questions(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "questions", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let off = here.off();
    let row = |q: &Question| {
        json!({
            "id": q.question_id, "desk": q.desk, "question": q.question, "contact": q.contact.clone().unwrap_or_default(),
            "page": q.page.clone().unwrap_or_default(), "answer": q.answer.clone().unwrap_or_default(),
            "asked": ago(q.created_at, here.now, off), "href": format!("{}questions/{}", base(here.slug()), q.question_id),
        })
    };
    let open: Vec<Value> = here
        .g
        .questions
        .iter()
        .filter(|q| q.state == "open")
        .map(row)
        .collect();
    let answered: Vec<Value> = here
        .g
        .questions
        .iter()
        .filter(|q| q.state != "open")
        .map(row)
        .collect();
    let sub = format!(
        "Teach your site · {} open, {} answered",
        open.len(),
        answered.len()
    );
    let owner = owner_first(&here.site);
    let marks = marks_view(&here).await?;
    here.render(
        "owner/questions.html",
        "Questions",
        sub,
        json!({ "open": open, "answered": answered, "owner": owner, "marks": marks }),
    )
    .await
}

/// Days of "Was this right?" marks the Questions app counts.
const MARK_DAYS: i64 = 30;

/// "Was this right?" for the Questions app: Yes and No by desk over the last 30 days, and the
/// answers marked wrong, each with a form to teach the right answer (or the date it was taught).
async fn marks_view(here: &Here<'_>) -> anyhow::Result<Value> {
    let off = here.off();
    let marks = here
        .records
        .feedback(here.slug(), here.now - MARK_DAYS * 86_400)
        .await?;
    let taught = here.records.taught(here.slug()).await?;
    let mut desks: Vec<(String, usize, usize)> = Vec::new();
    for m in &marks {
        let at = match desks.iter().position(|(d, _, _)| *d == m.desk) {
            Some(i) => i,
            None => {
                desks.push((m.desk.clone(), 0, 0));
                desks.len() - 1
            }
        };
        if m.right() {
            desks[at].1 += 1;
        } else {
            desks[at].2 += 1;
        }
    }
    desks.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)).then(a.0.cmp(&b.0)));
    let yes = marks.iter().filter(|m| m.right()).count();
    let wrong: Vec<Value> = marks
        .iter()
        .filter(|m| !m.right())
        .take(20)
        .map(|m| {
            let since = crate::ask::taught_for(&taught, &m.question)
                .filter(|t| t.taught_at() >= m.created_at)
                .map(|t| time::fmt_day(t.taught_at(), off));
            json!({
                "id": m.answer_id, "desk": m.desk, "question": m.question,
                "answer": m.answer.as_deref().map(|a| excerpt(a, 240)).unwrap_or_default(),
                "note": m.note.clone().unwrap_or_default(), "when": ago(m.updated_at, here.now, off),
                "taught": since, "href": format!("{}feedback/{}", base(here.slug()), m.answer_id),
            })
        })
        .collect();
    Ok(json!({
        "days": MARK_DAYS, "yes": yes, "no": marks.len() - yes, "total": marks.len(),
        "desks": desks.iter().map(|(d, y, n)| json!({ "desk": d, "yes": y, "no": n })).collect::<Vec<_>>(),
        "wrong": wrong,
    }))
}

/// Teaches the right answer to a question a visitor marked wrong. It is kept with the answered
/// questions, and the desks give it to the next visitor who asks the same thing.
async fn teach_from_mark(
    State(app): State<App>,
    Path((slug, id)): Path<(String, String)>,
    Form(form): Form<AnswerForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}questions", base(&slug));
    let Some(mark) = records.feedback_one(&site.site, &id).await? else {
        return Ok(done(
            &back,
            false,
            "That answer isn't on the list any more.",
        ));
    };
    let text = form.answer.trim();
    if text.is_empty() || text.chars().count() > 2000 {
        return Ok(done(
            &back,
            false,
            "Write the answer in a sentence or two, then save it.",
        ));
    }
    records
        .teach(&site.site, &mark.desk, &mark.question, text, time::now())
        .await?;
    Ok(done(
        &back,
        true,
        &format!(
            "Answer saved by {} for \u{201c}{}\u{201d}. The desks give it to the next visitor who asks.",
            owner_first(&site),
            excerpt(&mark.question, 60)
        ),
    ))
}

#[derive(Deserialize, Default)]
struct AnswerForm {
    #[serde(default)]
    answer: String,
}

async fn answer(
    State(app): State<App>,
    Path((slug, id)): Path<(String, String)>,
    Form(form): Form<AnswerForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}questions", base(&slug));
    let Some(q) = records
        .questions(&site.site)
        .await?
        .into_iter()
        .find(|q| q.question_id == id)
    else {
        return Ok(done(
            &back,
            false,
            "That question isn't on the list any more.",
        ));
    };
    let text = form.answer.trim();
    if text.is_empty() || text.chars().count() > 2000 {
        return Ok(done(
            &back,
            false,
            "Write the answer in a sentence or two, then save it.",
        ));
    }
    records
        .answer_question(&site.site, &id, text, time::now())
        .await?;
    let said = format!(
        "Answer saved by {} for \u{201c}{}\u{201d}. The desks give it to the next visitor who asks.",
        owner_first(&site),
        excerpt(&q.question, 60)
    );
    let words = match q.contact.as_deref().filter(|c| !c.trim().is_empty()) {
        Some(c) => format!(
            "{said} A sample sends no email or WhatsApp, so on a real site you'd reply to {c} yourself."
        ),
        None => format!("{said} The visitor left no contact, so the answer stays here."),
    };
    Ok(done(&back, true, &words))
}

// ---------------------------------------------------------------------------------------------
// Customers.

async fn customers(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "customers", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let off = here.off();
    let list = here.records.customers(here.slug()).await?;
    let mut counts: HashMap<&str, (usize, i64, i64)> = HashMap::new();
    for v in &here.g.orders {
        if let Some(c) = v.order.customer_id.as_deref() {
            let e = counts.entry(c).or_insert((0, 0, 0));
            e.0 += 1;
            if !CLOSED.contains(&v.order.state.as_str()) {
                e.1 += v.order.total_minor;
            }
            e.2 = e.2.max(v.order.created_at);
        }
    }
    let subs: HashMap<&str, &Subscriber> = here
        .g
        .subscribers
        .iter()
        .map(|s| (s.customer_id.as_str(), s))
        .collect();
    let news = here.site.archetype == "news";
    let mut rows: Vec<(i64, Value)> = list
        .iter()
        .map(|c| {
            let (n, spent, last) = counts.get(c.customer_id.as_str()).copied().unwrap_or((0, 0, 0));
            let sub = subs.get(c.customer_id.as_str());
            (
                if news { c.created_at } else { last.max(c.created_at) },
                json!({
                    "name": c.name, "email": c.email, "phone": c.phone.clone().unwrap_or_default(),
                    "place": c.place.clone().unwrap_or_default(), "kind": c.kind, "orders": n,
                    "spent": here.money(spent), "last": if last > 0 { ago(last, here.now, off) } else { String::new() },
                    "since": time::fmt_day(c.created_at, off), "sample": c.sample != 0,
                    "subscriber": sub.map(|s| s.state.clone()).unwrap_or_default(),
                    "plan": sub.map(|s| s.plan.clone()).unwrap_or_default(),
                }),
            )
        })
        .collect();
    rows.sort_by_key(|(k, _)| std::cmp::Reverse(*k));
    let with_orders = counts.len();
    let sub = if news {
        format!("{} · newest first", plural(rows.len(), "reader", "readers"))
    } else {
        format!(
            "{} · {} with orders · most recent first",
            plural(rows.len(), "customer", "customers"),
            with_orders
        )
    };
    let rows: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
    here.render(
        "owner/customers.html",
        if news { "Readers" } else { "Customers" },
        sub,
        json!({ "rows": rows, "news": news }),
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// Stories (news).

fn story_row(
    here: &Here<'_>,
    s: &Story,
    authors: &HashMap<String, String>,
    corrections: &HashMap<String, usize>,
) -> Value {
    let off = here.off();
    json!({
        "slug": s.slug, "title": s.title, "kicker": s.kicker.clone().unwrap_or_default(), "section": s.section,
        "author": authors.get(&s.author_id).cloned().unwrap_or_else(|| s.author_id.clone()),
        "locked": s.locked(), "status": s.status, "published": s.status == "published",
        "date": time::fmt_day(s.published_at, off), "updated": if s.updated_at > s.published_at { time::fmt_day_clock(s.updated_at, off) } else { String::new() },
        "corrections": corrections.get(&s.slug).copied().unwrap_or(0),
        "href": format!("{}stories/{}", base(here.slug()), s.slug),
        "page": format!("{}/{}/{}", here.app.config.origin_for(&here.site.host), time::local(s.published_at, off).format("%Y/%m"), s.slug),
    })
}

async fn stories(
    State(app): State<App>,
    Path(slug): Path<String>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Page {
    let here = match load(&app, &slug, "stories", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let authors: HashMap<String, String> = here
        .records
        .authors(here.slug())
        .await?
        .into_iter()
        .map(|a| (a.author_id, a.name))
        .collect();
    let mut corrections: HashMap<String, usize> = HashMap::new();
    for c in here.records.corrections(here.slug(), None).await? {
        *corrections.entry(c.story_slug).or_default() += 1;
    }
    let pick = match q.state.as_deref() {
        Some("published") => "published",
        Some("draft") => "draft",
        _ => "all",
    };
    let count = |id: &str| {
        here.g
            .stories
            .iter()
            .filter(|s| id == "all" || (id == "published") == (s.status == "published"))
            .count()
    };
    let tabs: Vec<Value> = [("all", "All"), ("published", "Published"), ("draft", "Not published")]
        .iter()
        .map(|(id, label)| {
            json!({
                "id": id, "label": label, "count": count(id), "current": *id == pick,
                "href": if *id == "all" { format!("{}stories", base(&slug)) } else { format!("{}stories?state={id}", base(&slug)) },
            })
        })
        .collect();
    let rows: Vec<Value> = here
        .g
        .stories
        .iter()
        .filter(|s| pick == "all" || (pick == "published") == (s.status == "published"))
        .map(|s| story_row(&here, s, &authors, &corrections))
        .collect();
    let sub = format!("{} · newest first", plural(rows.len(), "story", "stories"));
    here.render(
        "owner/stories.html",
        "Stories",
        sub,
        json!({ "rows": rows, "tabs": tabs }),
    )
    .await
}

#[derive(Deserialize, Default)]
struct StoryForm {
    #[serde(default, rename = "do")]
    action: String,
    #[serde(default)]
    what: String,
    #[serde(default)]
    note: String,
}

async fn story_action(
    State(app): State<App>,
    Path((slug, story)): Path<(String, String)>,
    Form(form): Form<StoryForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}stories#st-{story}", base(&slug));
    let Some(s) = records.story(&site.site, &story).await? else {
        return Ok(done(&back, false, "That story isn't there any more."));
    };
    let owner = owner_first(&site);
    let text = match form.action.as_str() {
        "publish" => {
            records
                .set_story_status(&site.site, &story, "published")
                .await?;
            format!(
                "\u{201c}{}\u{201d} published by {owner}. Readers see it on the site now.",
                s.title
            )
        }
        "unpublish" => {
            records
                .set_story_status(&site.site, &story, "draft")
                .await?;
            format!(
                "\u{201c}{}\u{201d} unpublished by {owner}. Readers no longer see it.",
                s.title
            )
        }
        "correct" => {
            let what = form.what.trim();
            let note = form.note.trim();
            if what.is_empty()
                || note.is_empty()
                || what.chars().count() > 200
                || note.chars().count() > 600
            {
                return Ok(done(
                    &back,
                    false,
                    "Say what changed and write the note readers see, then add the correction.",
                ));
            }
            records
                .add_correction(&site.site, &story, what, note, time::now())
                .await?;
            format!(
                "Correction added to \u{201c}{}\u{201d} by {owner}. Readers see the note with the story.",
                s.title
            )
        }
        _ => {
            return Ok(done(
                &back,
                false,
                "Choose publish, unpublish or a correction.",
            ));
        }
    };
    Ok(done(&back, true, &text))
}

// ---------------------------------------------------------------------------------------------
// Subscribers and the digest (news).

async fn subscribers(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "subscribers", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let off = here.off();
    let people: HashMap<String, (String, String)> = here
        .records
        .customers(here.slug())
        .await?
        .into_iter()
        .map(|c| (c.customer_id, (c.name, c.email)))
        .collect();
    let rows: Vec<Value> = here
        .g
        .subscribers
        .iter()
        .map(|s| {
            let (name, email) = people.get(&s.customer_id).cloned().unwrap_or_else(|| (s.customer_id.clone(), String::new()));
            let active = s.state == "active" && s.until_at.is_none_or(|u| u > here.now);
            json!({
                "id": s.subscriber_id, "name": name, "email": email, "plan": s.plan, "source": source_words(&s.source),
                "state": if active { "active" } else { "ended" }, "active": active,
                "since": time::fmt_date_long(s.since, off), "until": s.until_at.map(|u| time::fmt_date_long(u, off)).unwrap_or_default(),
                "end": format!("{}subscribers/{}/end", base(here.slug()), s.subscriber_id),
            })
        })
        .collect();
    let plan = here
        .g
        .subscribers
        .first()
        .map(|s| s.plan.clone())
        .or_else(|| Some(here.site.fact("/subscription/plan")).filter(|p| !p.is_empty()))
        .unwrap_or_else(|| "Monthly".into());
    let active = rows
        .iter()
        .filter(|r| r["active"].as_bool().unwrap_or(false))
        .count();
    let sub = format!("{} active, {} ended", active, rows.len() - active);
    here.render(
        "owner/subscribers.html",
        "Subscribers",
        sub,
        json!({ "rows": rows, "plan": plan }),
    )
    .await
}

#[derive(Deserialize, Default)]
struct GrantForm {
    #[serde(default)]
    email: String,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    lasts: String,
}

async fn grant(
    State(app): State<App>,
    Path(slug): Path<String>,
    Form(form): Form<GrantForm>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}subscribers", base(&slug));
    let email = form.email.trim().to_lowercase();
    if !email.contains('@') {
        return Ok(done(
            &back,
            false,
            "Write the reader's email address, like reader@example.com.",
        ));
    }
    let Some(c) = records.customer_by_email(&site.site, &email).await? else {
        return Ok(done(
            &back,
            false,
            &format!(
                "No reader with {email} on {}. They sign in on the site once, then you can give them a subscription.",
                site.name
            ),
        ));
    };
    let now = time::now();
    let until = match form.lasts.as_str() {
        "month" => Some(now + 30 * 86_400),
        "year" => Some(now + 365 * 86_400),
        _ => None,
    };
    let plan = form.plan.trim();
    let plan = if plan.is_empty() { "Monthly" } else { plan };
    let plan: String = plan.chars().take(60).collect();
    records
        .grant_subscription(&site.site, &c.customer_id, &plan, "owner grant", until, now)
        .await?;
    let till = until.map_or_else(
        || "with no end date".to_owned(),
        |u| format!("until {}", time::fmt_date_long(u, site.utc_offset_min)),
    );
    let text = format!(
        "{} gave {}, {email}, the {plan} subscription {till}. They read subscriber stories in full when signed in.",
        owner_first(&site),
        c.name
    );
    Ok(done(&back, true, &text))
}

async fn end_subscription(
    State(app): State<App>,
    Path((slug, id)): Path<(String, String)>,
) -> Page {
    let records = app.records().await?;
    let Some(site) = records.site(&slug).await? else {
        return Ok(see_other("/samples/"));
    };
    let back = format!("{}subscribers", base(&slug));
    let Some(s) = records
        .subscribers(&site.site)
        .await?
        .into_iter()
        .find(|s| s.subscriber_id == id)
    else {
        return Ok(done(
            &back,
            false,
            "That subscription isn't on the list any more.",
        ));
    };
    let name = records
        .customer(&site.site, &s.customer_id)
        .await?
        .map_or_else(|| "The reader".to_owned(), |c| c.name);
    records
        .end_subscription(&site.site, &id, time::now())
        .await?;
    let text = format!(
        "{} ended the {} subscription of {name} now. Subscriber stories show them the headline and link only again.",
        owner_first(&site),
        s.plan
    );
    Ok(done(&back, true, &text))
}

async fn digest(State(app): State<App>, Path(slug): Path<String>, headers: HeaderMap) -> Page {
    let here = match load(&app, &slug, "digest", &headers).await? {
        Loaded::Here(h) => *h,
        Loaded::Missing(r) => return Ok(r),
    };
    let off = here.off();
    let groups: Vec<Value> = [
        ("confirmed", "Confirmed", "They confirmed by email and get the Friday five."),
        ("pending", "Waiting to confirm", "They asked for it but haven't opened the confirmation link, so nothing is sent."),
        ("stopped", "Stopped", "They stopped it; their topics were deleted with it."),
    ]
    .iter()
    .map(|(state, label, about)| {
        let rows: Vec<Value> = here
            .g
            .digest
            .iter()
            .filter(|d| d.state == *state)
            .map(|d| {
                let topics: Vec<String> = serde_json::from_str(&d.topics).unwrap_or_default();
                json!({
                    "email": d.email, "topics": topics.join(", "), "since": time::fmt_day(d.created_at, off),
                    "confirmed": d.confirmed_at.map(|t| time::fmt_day(t, off)).unwrap_or_default(),
                })
            })
            .collect();
        json!({ "state": state, "label": label, "about": about, "count": rows.len(), "rows": rows })
    })
    .collect();
    let sub = format!(
        "{} in all, by state",
        plural(
            here.g.digest.len(),
            "Friday five address",
            "Friday five addresses"
        )
    );
    here.render(
        "owner/digest.html",
        "Friday five",
        sub,
        json!({ "groups": groups }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_read_as_people_type_them() {
        assert_eq!(parse_price("480"), Some(48_000));
        assert_eq!(parse_price("KSh 4,800"), Some(480_000));
        assert_eq!(parse_price("12.5"), Some(1250));
        assert_eq!(parse_price("12.50"), Some(1250));
        assert_eq!(parse_price("0"), None);
        assert_eq!(parse_price("abc"), None);
        assert_eq!(parse_price("1.234"), None);
        assert_eq!(price_input(45_000), "450");
        assert_eq!(price_input(1250), "12.50");
    }

    #[test]
    fn every_page_app_is_in_the_registry() {
        for id in [
            "today",
            "orders",
            "custom",
            "catalogue",
            "stock",
            "questions",
            "customers",
            "stories",
            "subscribers",
            "digest",
        ] {
            assert_eq!(app_def(id).id, id);
        }
        assert_eq!(humanize("classy-perfumes-sample"), "Classy Perfumes");
    }
}
