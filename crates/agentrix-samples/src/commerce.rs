//! Commerce on every shop host (Kilele and Classy): bag, checkout, pay, receipts, orders, the
//! order page and its change and help steps, and the account pages W2 keeps (home, details,
//! addresses). Sign-in itself is W6's (`/account/sign-in`, `/account/security`).
//!
//! Every figure comes from records: prices from the live price list, stock from today's bake or
//! count, slots from the delivery calendar, totals from the order's own lines, the rate pinned
//! on the order. Words that depend on the business (owner, cut-off, refund rule) come from the
//! bundle's manifest facts.

use agentrix_records::addresses::NewAddress;
use agentrix_records::orders::{
    CartView, Contact, OrderError, OrderView, Payment, Placement, Refund, Slot, Zone, state_words,
};
use agentrix_records::{Records, Site, money, new_id, time};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::App;
use crate::ask::{Action, Answer};
use crate::ports::{PaymentRequest, PaymentStart, Visitor};
use crate::shell::{Desk, Shell, urlencode};
use crate::web::{self, Ctx, Page, html, not_found, see_other};

/// The commerce routes.
pub fn routes() -> Router<App> {
    Router::new()
        .route("/bag", get(bag))
        .route("/bag/add", post(bag_add))
        .route("/bag/update", post(bag_update))
        .route("/checkout", get(checkout).post(checkout_post))
        .route("/orders", get(orders))
        .route("/orders/{id}", get(order_page))
        .route("/orders/{id}/pay", get(pay).post(pay_post))
        .route("/orders/{id}/pay/sample", post(pay_sample))
        .route("/orders/{id}/paid", get(paid))
        .route("/orders/{id}/receipt", get(receipt))
        .route("/orders/{id}/change", get(change).post(change_post))
        .route("/orders/{id}/change/withdraw", post(change_withdraw))
        .route("/orders/{id}/cancel", post(cancel_post))
        .route("/orders/{id}/help", get(help).post(help_post))
        .route("/account", get(account))
        .route("/account/details", get(details).post(details_post))
        .route("/account/addresses", get(addresses).post(addresses_post))
}

// ---------- Words and small helpers ----------

/// States an order can be paid in.
const PAYABLE: [&str; 2] = ["awaiting_payment", "approved"];
/// States after payment, while the order runs or once it's done.
const AFTER_PAID: [&str; 6] = ["paid", "baking", "ready", "out", "delivered", "collected"];
/// States that end an order.
const ENDED: [&str; 4] = ["declined", "lapsed", "cancelled", "refunded"];

/// An order state in visitor words, with the owner's name.
fn state_of(site: &Site, state: &str) -> String {
    state_words(state).replace("the owner", &owner(site))
}

fn sells(site: &Site) -> bool {
    matches!(site.archetype.as_str(), "bakery" | "shop")
}

fn owner(site: &Site) -> String {
    let o = site.fact("/owner/first");
    if o.is_empty() { site.name.clone() } else { o }
}

/// "Kilele" for "Kilele Bakery".
fn short(site: &Site) -> String {
    site.name
        .split_whitespace()
        .next()
        .unwrap_or(&site.name)
        .to_owned()
}

fn cash(site: &Site, minor: i64) -> String {
    money::fmt(&site.currency, minor)
}

/// "UTC+3".
fn zone_name(site: &Site) -> String {
    let (h, m) = (site.utc_offset_min / 60, (site.utc_offset_min % 60).abs());
    if m == 0 {
        format!("UTC{h:+}")
    } else {
        format!("UTC{h:+}:{m:02}")
    }
}

fn day_clock(site: &Site, ts: i64) -> String {
    time::fmt_day_clock(ts, site.utc_offset_min)
}

fn prefix(site: &Site) -> String {
    let p = site.fact("/order_prefix");
    if p.is_empty() { "ORD".into() } else { p }
}

fn sign_in(next: &str) -> String {
    format!("/account/sign-in?why=order&next={}", urlencode(next))
}

/// A safe local path to go back to.
fn local_path(p: Option<&str>) -> Option<String> {
    p.map(str::trim)
        .filter(|p| p.starts_with('/') && !p.starts_with("//") && !p.contains('\\'))
        .map(str::to_owned)
}

fn cart_of(ctx: &Ctx) -> Option<String> {
    ctx.cart.clone().filter(|c| {
        !c.is_empty()
            && c.len() <= 64
            && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

fn trimmed(s: Option<&str>, max: usize) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(max).collect())
}

fn phone_ok(p: &str) -> bool {
    let digits = p.chars().filter(char::is_ascii_digit).count();
    (7..=15).contains(&digits) && p.chars().all(|c| c.is_ascii_digit() || " +-()".contains(c))
}

fn orders_desk(shell: &Shell) -> Desk {
    shell
        .desks
        .iter()
        .find(|d| d.id == "orders")
        .cloned()
        .unwrap_or_else(|| shell.front.clone())
}

fn browse(shell: &Shell) -> Vec<Value> {
    shell
        .desks
        .iter()
        .filter(|d| d.id != "orders")
        .take(2)
        .map(|d| json!({ "name": d.name, "href": d.href }))
        .collect()
}

/// "12 Argwings Kodhek Rd, Kilimani".
fn place(details: &str, area: &str) -> String {
    [details, area]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// A POST from another site is refused (forms here only post to their own host).
fn foreign(ctx: &Ctx, headers: &HeaderMap) -> Option<Response> {
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let fetch = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
    let ok = match origin {
        Some(o) => o == ctx.host.origin,
        None => fetch.is_none_or(|f| f == "same-origin" || f == "none"),
    };
    (!ok).then(|| {
        (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "This form only works from this site's own pages. Nothing was changed.",
        )
            .into_response()
    })
}

const PHONE_WORDS: &str =
    "That phone number doesn't look right. Use digits, with the country code if it isn't Kenyan.";

// ---------- Bag ----------

/// A priced line as the bag and checkout show it.
#[derive(Debug, Clone, Serialize)]
struct LineView {
    sku: String,
    name: String,
    qty: i64,
    unit: String,
    amount: String,
    sources: Vec<String>,
    available: bool,
    avail: String,
    less: i64,
    more: i64,
    at_most: bool,
}

/// "today" or "Sat 14 Nov".
fn bake_day(site: &Site, day: Option<i64>, now: i64) -> String {
    match day {
        Some(ts)
            if time::day_of(ts, site.utc_offset_min) != time::day_of(now, site.utc_offset_min) =>
        {
            time::fmt_day(ts, site.utc_offset_min)
        }
        _ => "today".into(),
    }
}

/// "price list of Sun 1 Nov" for a cart's pinned list, or "" without one.
async fn list_words(records: &Records, site: &Site, view: &CartView) -> anyhow::Result<String> {
    Ok(match view.price_list.as_deref() {
        Some(v) => crate::common::list_name_of(records, site, v)
            .await?
            .trim_start_matches("the ")
            .to_owned(),
        None => String::new(),
    })
}

fn line_views(
    site: &Site,
    view: &CartView,
    list: &str,
    day: Option<i64>,
    now: i64,
) -> Vec<LineView> {
    let when = bake_day(site, day, now);
    view.lines
        .iter()
        .map(|l| {
            let mut sources = Vec::new();
            if !list.is_empty() {
                sources.push(capital(list));
            }
            if l.left.is_some() {
                sources.push(match site.archetype.as_str() {
                    "bakery" if when == "today" => "Stock: today's bake (live)".to_owned(),
                    "bakery" => format!("Stock: {when} bake (live)"),
                    _ => "Stock: today's count (live)".to_owned(),
                });
            }
            let avail = match (l.available, l.left) {
                (true, _) => format!("Available {when}"),
                (false, Some(0)) => format!("Sold out {when}"),
                (false, Some(n)) => format!("{n} in stock {when}"),
                (false, None) => "Not available".into(),
            };
            LineView {
                sku: l.sku.clone(),
                name: l.name.clone(),
                qty: l.qty,
                unit: format!("{} each", cash(site, l.unit_minor)),
                amount: cash(site, l.amount_minor),
                sources,
                available: l.available,
                avail,
                less: l.qty - 1,
                more: l.qty + 1,
                at_most: l.qty >= 20,
            }
        })
        .collect()
}

#[derive(Deserialize)]
struct AddForm {
    sku: String,
    #[serde(default)]
    qty: Option<i64>,
    #[serde(default)]
    back: Option<String>,
}

async fn bag_add(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Form(f): Form<AddForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let records = app.records().await?;
    let (cart, fresh) = match cart_of(&ctx) {
        Some(c) => (c, false),
        None => (new_id("cart"), true),
    };
    let qty = f.qty.unwrap_or(1).clamp(1, 20);
    let refused = match records
        .add_to_cart(&ctx.site.site, &cart, f.sku.trim(), qty, false)
        .await
    {
        Ok(_) => false,
        Err(OrderError::Refused(_)) => true,
        Err(OrderError::Other(e)) => return Err(e.into()),
    };
    let to = if refused {
        "/bag?e=menu".to_owned()
    } else {
        local_path(f.back.as_deref()).unwrap_or_else(|| "/bag".into())
    };
    let mut response = see_other(&to);
    if fresh {
        response.headers_mut().append(
            header::SET_COOKIE,
            web::set_cookie(&app, web::CART, &cart, web::CART_SECONDS),
        );
    }
    Ok(response)
}

#[derive(Deserialize)]
struct UpdateForm {
    sku: String,
    qty: i64,
}

async fn bag_update(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Form(f): Form<UpdateForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let Some(cart) = cart_of(&ctx) else {
        return Ok(see_other("/bag"));
    };
    let records = app.records().await?;
    match records
        .add_to_cart(
            &ctx.site.site,
            &cart,
            f.sku.trim(),
            f.qty.clamp(0, 20),
            true,
        )
        .await
    {
        Ok(_) | Err(OrderError::Refused(_)) => Ok(see_other("/bag")),
        Err(OrderError::Other(e)) => Err(e.into()),
    }
}

#[derive(Deserialize, Default)]
struct BagQuery {
    #[serde(default)]
    e: Option<String>,
}

async fn bag(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<BagQuery>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let cart = records
        .cart(&site.site, &cart_of(&ctx).unwrap_or_else(|| "none".into()))
        .await?;
    let view = records.price_cart(site, &cart, now).await?;
    let shell = app
        .shell(
            &ctx,
            "",
            "Your bag",
            "What's in your bag, with where each price and stock figure comes from.",
        )
        .await?;
    let page = json!({
        "browse": browse(&shell),
        "shell": shell,
        "lines": line_views(site, &view, &list_words(records, site, &view).await?, None, now),
        "items": view.items,
        "subtotal": cash(site, view.subtotal_minor),
        "usd": money::usd(money::usd_cents(view.subtotal_minor, view.usd_rate_minor)),
        "rate": money::rate_line(view.usd_rate_minor),
        "problems": view.problems,
        "notice": q.e.as_deref().map(|_| "That item isn't on the menu any more, so nothing was added.".to_owned()),
        "signed_in": ctx.visitor.is_some(),
        "short": short(site),
        "delivery_note": if site.archetype == "bakery" { "Delivery or collection, and the time, are picked at checkout." } else { "Delivery is picked at checkout." },
    });
    Ok(html(app.render("shop/bag.html", &page)?))
}

// ---------- Checkout (K-06) ----------

#[derive(Deserialize, Default, Clone)]
struct CheckoutForm {
    #[serde(default)]
    key: String,
    #[serde(default)]
    how: Option<String>,
    #[serde(default)]
    slot: Option<String>,
    #[serde(default)]
    zone: Option<String>,
    #[serde(default)]
    recipient: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    note: Option<String>,
    /// Re-price without placing the order (no-JS "Update the total").
    #[serde(default)]
    preview: Option<String>,
    /// Switch between delivery and collection, keeping what was typed.
    #[serde(default)]
    switch: Option<String>,
}

#[derive(Serialize)]
struct SlotView {
    id: String,
    day: String,
    hours: String,
    label: String,
    full: bool,
    checked: bool,
}

fn slot_view(site: &Site, s: &Slot, chosen: Option<&str>) -> SlotView {
    let off = site.utc_offset_min;
    SlotView {
        id: s.slot_id.clone(),
        day: time::fmt_day(s.starts_at, off),
        hours: format!(
            "{}\u{2013}{}",
            time::fmt_clock(s.starts_at, off),
            time::fmt_clock(s.ends_at, off)
        ),
        label: s.label.clone(),
        full: s.full,
        checked: chosen == Some(s.slot_id.as_str()),
    }
}

/// What checkout shows a visitor who can't order yet: sign in, or confirm the email first
/// (receipts and updates go there), with the reason on the page.
async fn gate(app: &App, ctx: &Ctx, next: &str) -> anyhow::Result<Option<Response>> {
    match &ctx.visitor {
        None => Ok(Some(see_other(&sign_in(next)))),
        Some(v) if !v.email_verified => {
            let shell = app
                .shell(
                    ctx,
                    "orders",
                    "Confirm your email",
                    "Confirm your email before you order.",
                )
                .await?;
            let page = json!({ "shell": shell, "email": v.email, "short": short(&ctx.site) });
            Ok(Some(html(app.render("shop/verify.html", &page)?)))
        }
        Some(_) => Ok(None),
    }
}

async fn checkout(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(f): Query<CheckoutForm>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    if let Some(r) = gate(&app, &ctx, "/checkout").await? {
        return Ok(r);
    }
    checkout_page(&app, &ctx, f, None).await
}

async fn checkout_page(app: &App, ctx: &Ctx, f: CheckoutForm, error: Option<String>) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let Some(visitor) = ctx.visitor.clone() else {
        return Ok(see_other(&sign_in("/checkout")));
    };
    let Some(cart_id) = cart_of(ctx) else {
        return Ok(see_other("/bag"));
    };
    let mut cart = records.cart(&site.site, &cart_id).await?;
    if cart.lines.is_empty() {
        return Ok(see_other("/bag"));
    }
    let bakery = site.archetype == "bakery";
    let hows: &[&str] = if bakery {
        &["delivery", "collection"]
    } else {
        &["delivery"]
    };
    let how = f
        .how
        .as_deref()
        .filter(|h| hows.contains(h))
        .unwrap_or("delivery")
        .to_owned();
    let slots: Vec<Slot> = if bakery {
        records
            .slots(site, &how, now, now + 14 * 86_400)
            .await?
            .into_iter()
            .filter(|s| s.starts_at > now)
            .collect()
    } else {
        Vec::new()
    };
    let chosen = f
        .slot
        .as_deref()
        .and_then(|id| slots.iter().find(|s| s.slot_id == id && !s.full))
        .or_else(|| slots.iter().find(|s| !s.full))
        .cloned();
    let zones = records.zones(&site.site).await?;
    let saved = records.addresses(&site.site, &visitor.account).await?;
    let zone = if how == "delivery" {
        f.zone
            .as_deref()
            .and_then(|z| zones.iter().find(|x| x.zone == z))
            .or_else(|| {
                saved
                    .first()
                    .and_then(|a| zones.iter().find(|x| x.zone == a.zone))
            })
            .or_else(|| zones.first())
            .cloned()
    } else {
        None
    };
    cart.slot_id = chosen.as_ref().map(|s| s.slot_id.clone());
    cart.zone = zone.as_ref().map(|z| z.zone.clone());
    let view = records.price_cart(site, &cart, now).await?;
    let list = list_words(records, site, &view).await?;
    let customer = records.customer(&site.site, &visitor.account).await?;
    let recipient = f.recipient.clone().unwrap_or_else(|| {
        saved
            .first()
            .map(|a| format!("{}, {}", a.recipient, place(&a.details, &a.area)))
            .unwrap_or_default()
    });
    let phone = f
        .phone
        .clone()
        .or_else(|| saved.first().and_then(|a| a.phone.clone()))
        .or_else(|| customer.as_ref().and_then(|c| c.phone.clone()))
        .or_else(|| visitor.phone.clone())
        .unwrap_or_default();
    let shell = app
        .shell(
            ctx,
            "orders",
            "Checkout",
            "Your bag, the delivery time and the total, pinned to the price list.",
        )
        .await?;
    let desk = orders_desk(&shell);
    let delivery = zone.as_ref().map(|z| {
        json!({
            "name": format!("Delivery to {}", z.label),
            "detail": chosen.as_ref().map(|s| s.label.clone()).unwrap_or_else(|| z.note.clone()),
            "amount": cash(site, z.fee_minor),
        })
    });
    let key = if f.key.is_empty() {
        new_id("chk")
    } else {
        f.key.clone()
    };
    let page = json!({
        "shell": shell,
        "desk": desk,
        "key": key,
        "lines": line_views(site, &view, &list, chosen.as_ref().map(|s| s.starts_at), now),
        "delivery": delivery,
        "hows": hows,
        "how": how,
        "slots": slots.iter().map(|s| slot_view(site, s, cart.slot_id.as_deref())).collect::<Vec<_>>(),
        "has_slots": bakery,
        "zones": zones.iter().map(|z| json!({ "zone": z.zone, "label": z.label, "fee": cash(site, z.fee_minor), "areas": z.areas.join(", "), "checked": cart.zone.as_deref() == Some(z.zone.as_str()) })).collect::<Vec<_>>(),
        "recipient": recipient,
        "phone": phone,
        "note": f.note.clone().unwrap_or_default(),
        "total": cash(site, view.total_minor),
        "usd": money::usd(view.usd_cents),
        "rate": money::rate_line(view.usd_rate_minor),
        "pinned": match (&view.price_list, view.valid_until) {
            (Some(_), Some(until)) => format!("Quote pinned to {}'s {list}. Valid until {}.", short(site), day_clock(site, until)),
            _ => String::new(),
        },
        "price_list": capital(&list),
        "problems": view.problems,
        "error": error,
        "short": short(site),
        "name": visitor.name,
        "address": site.fact("/address"),
    });
    Ok(html(app.render("shop/checkout.html", &page)?))
}

async fn checkout_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Form(f): Form<CheckoutForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    if let Some(r) = gate(&app, &ctx, "/checkout").await? {
        return Ok(r);
    }
    if let Some(how) = f.switch.clone() {
        let f = CheckoutForm {
            how: Some(how),
            slot: None,
            switch: None,
            ..f
        };
        return checkout_page(&app, &ctx, f, None).await;
    }
    if f.preview.is_some() {
        return checkout_page(&app, &ctx, f, None).await;
    }
    let Some(visitor) = ctx.visitor.clone() else {
        return Ok(see_other(&sign_in("/checkout")));
    };
    let Some(cart) = cart_of(&ctx) else {
        return Ok(see_other("/bag"));
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let how = match f.how.as_deref() {
        Some("collection") if site.archetype == "bakery" => "collection",
        _ => "delivery",
    };
    let phone = trimmed(f.phone.as_deref(), 24);
    if phone.as_deref().is_some_and(|p| !phone_ok(p)) {
        return checkout_page(&app, &ctx, f, Some(PHONE_WORDS.into())).await;
    }
    let recipient = trimmed(f.recipient.as_deref(), 120);
    if how == "delivery" && recipient.is_none() {
        return checkout_page(
            &app,
            &ctx,
            f,
            Some(
                "Add who receives it and where, like \u{201c}Amina, at home in Kilimani\u{201d}."
                    .into(),
            ),
        )
        .await;
    }
    let phone = phone.or(visitor.phone.clone());
    records
        .upsert_customer(
            &site.site,
            &visitor.account,
            &visitor.name,
            &visitor.email,
            phone.as_deref(),
            now,
        )
        .await?;
    let contact = Contact {
        name: visitor.name.clone(),
        email: Some(visitor.email.clone()),
        phone,
        recipient,
    };
    let placement = Placement {
        customer_id: Some(visitor.account.clone()),
        channel: "site".into(),
        agent: None,
        proof: None,
        fulfilment: how.into(),
        slot_id: if site.archetype == "bakery" {
            trimmed(f.slot.as_deref(), 80)
        } else {
            None
        },
        zone: if how == "delivery" {
            trimmed(f.zone.as_deref(), 40)
        } else {
            None
        },
        note: trimmed(f.note.as_deref(), 400),
        idempotency_key: trimmed(Some(&f.key), 64).map(|k| format!("{}:{k}", visitor.account)),
    };
    match records
        .place_order(site, &cart, &contact, &placement, now)
        .await
    {
        Ok(ov) => Ok(see_other(&format!("/orders/{}/pay", ov.order.order_id))),
        Err(OrderError::Refused(words)) => {
            checkout_page(&app, &ctx, f, Some(format!("{words} Nothing was charged."))).await
        }
        Err(OrderError::Other(e)) => Err(e.into()),
    }
}

// ---------- Orders: access, heads, timelines ----------

enum Access {
    Mine(Box<OrderView>),
    Theirs(Box<OrderView>),
    Missing(String),
}

fn normalise(id: &str) -> String {
    id.trim()
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(24)
        .collect()
}

async fn access(records: &Records, ctx: &Ctx, id: &str) -> anyhow::Result<Access> {
    let id = normalise(id);
    let Some(ov) = records.order(&ctx.site, &id).await? else {
        return Ok(Access::Missing(id));
    };
    let mine = ctx
        .visitor
        .as_ref()
        .is_some_and(|v| ov.order.customer_id.as_deref() == Some(v.account.as_str()));
    Ok(if mine {
        Access::Mine(Box::new(ov))
    } else {
        Access::Theirs(Box::new(ov))
    })
}

/// The order, if it's the visitor's; otherwise the response to send instead (sign in, or the
/// status-only view).
async fn mine(app: &App, ctx: &Ctx, id: &str) -> anyhow::Result<Result<OrderView, Response>> {
    let records = app.records().await?;
    match access(records, ctx, id).await? {
        Access::Mine(ov) => Ok(Ok(*ov)),
        Access::Theirs(_) if ctx.visitor.is_none() => Ok(Err(see_other(&sign_in(&ctx.path)))),
        Access::Theirs(ov) => Ok(Err(
            status_page(app, ctx, Some(&ov), &ov.order.order_id).await?
        )),
        Access::Missing(id) => Ok(Err(status_page(app, ctx, None, &id).await?)),
    }
}

fn pill(site: &Site, state: &str) -> (String, &'static str) {
    let owner = owner(site);
    match state {
        "awaiting_payment" => ("Not paid yet".into(), "wait"),
        "held" => (format!("Waiting for {owner}'s OK"), "wait"),
        "approved" => (format!("Approved by {owner} \u{b7} not paid yet"), "wait"),
        "paid" => ("Paid \u{b7} Confirmed".into(), "ok"),
        "baking" if site.archetype == "bakery" => ("Baking".into(), "ok"),
        "baking" => ("Being packed".into(), "ok"),
        "ready" => ("Ready to collect".into(), "ok"),
        "out" => ("Out for delivery".into(), "ok"),
        "delivered" => ("Delivered".into(), "ok"),
        "collected" => ("Collected".into(), "ok"),
        "declined" => ("Declined".into(), "mute"),
        "lapsed" => ("Lapsed".into(), "mute"),
        "cancelled" => ("Cancelled".into(), "mute"),
        "refunded" => ("Refunded".into(), "mute"),
        _ => ("In progress".into(), "mute"),
    }
}

fn title(ov: &OrderView) -> String {
    if ov.order.kind == "custom" {
        let size = ov.lines.first().map(|l| l.name.clone()).unwrap_or_default();
        let message = ov
            .order
            .spec()
            .and_then(|s| s.get("message").and_then(Value::as_str).map(str::to_owned))
            .filter(|m| !m.trim().is_empty());
        return match message {
            Some(m) => format!("Celebration cake, {size}, \u{201c}{m}\u{201d}"),
            None => format!("Celebration cake, {size}"),
        };
    }
    ov.lines
        .iter()
        .filter(|l| l.sku.is_some())
        .map(|l| {
            if l.qty > 1 {
                format!("{} \u{d7} {}", l.name, l.qty)
            } else {
                l.name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn zone_label(zones: &[Zone], zone: Option<&str>) -> String {
    zone.and_then(|z| zones.iter().find(|x| x.zone == z))
        .map(|z| z.label.clone())
        .unwrap_or_default()
}

/// "delivery Sat 14 Nov, 10:00–12:00" or "collection …".
fn fulfil(ov: &OrderView, zones: &[Zone]) -> String {
    let slot = ov.slot.as_ref().map(|s| s.label.clone());
    match (ov.order.fulfilment.as_str(), slot) {
        ("collection", Some(l)) => format!("collection {l}"),
        ("collection", None) => "collection".into(),
        (_, Some(l)) => format!("delivery {l}"),
        (_, None) => match zone_label(zones, ov.order.zone.as_deref()) {
            z if z.is_empty() => "delivery".into(),
            z => format!("delivery to {z}"),
        },
    }
}

#[derive(Debug, Clone, Serialize)]
struct Head {
    id: String,
    title: String,
    pill: String,
    tone: String,
    summary: String,
    total: String,
    usd: String,
    state: String,
    words: String,
    payable: bool,
    paid: bool,
    ended: bool,
    custom: bool,
    approved_at: String,
    agent: Option<String>,
}

fn head(site: &Site, ov: &OrderView, zones: &[Zone]) -> Head {
    let (pill, tone) = pill(site, &ov.order.state);
    let mut summary = format!(
        "{} \u{b7} {}",
        cash(site, ov.order.total_minor),
        fulfil(ov, zones)
    );
    if let Some(r) = &ov.order.recipient {
        summary.push_str(&format!(", {r}"));
    }
    Head {
        id: ov.order.order_id.clone(),
        title: title(ov),
        pill,
        tone: tone.into(),
        summary,
        total: cash(site, ov.order.total_minor),
        usd: money::usd(ov.order.usd_cents()),
        state: ov.order.state.clone(),
        words: state_of(site, &ov.order.state),
        payable: PAYABLE.contains(&ov.order.state.as_str()),
        paid: AFTER_PAID.contains(&ov.order.state.as_str()) || settled(ov).is_some(),
        ended: ENDED.contains(&ov.order.state.as_str()),
        custom: ov.order.kind == "custom",
        approved_at: ov
            .events
            .iter()
            .rev()
            .find(|e| e.step == "approved")
            .map(|e| day_clock(site, e.at))
            .unwrap_or_default(),
        agent: ov.order.agent.clone(),
    }
}

/// The payment that settled the order (payments are newest first).
fn settled(ov: &OrderView) -> Option<&Payment> {
    ov.payments
        .iter()
        .find(|p| p.state == "captured" || p.state == "sample")
}

/// The samples take PayPal only (James, 8 Nov), so every payment reads as PayPal.
fn rail_words(_rail: &str) -> &'static str {
    "PayPal"
}

/// What the Orders desk says about where an order stands, and a second line when there is one.
fn desk_line(site: &Site, ov: &OrderView, zones: &[Zone]) -> (String, Option<String>) {
    let owner = owner(site);
    let at = |step: &str| {
        ov.events
            .iter()
            .rev()
            .find(|e| e.step == step)
            .map(|e| day_clock(site, e.at))
            .unwrap_or_default()
    };
    let f = capital(&fulfil(ov, zones));
    match ov.order.state.as_str() {
        "held" => (
            format!("Waiting for {owner}'s OK. Nothing charged."),
            Some(format!(
                "No payment request yet. It appears here after {owner} approves."
            )),
        ),
        "approved" => (
            "This payment request comes straight from the approved order. Its amount can't change."
                .into(),
            None,
        ),
        "awaiting_payment" => (
            format!(
                "Not paid yet: {} at {}'s rate. Nothing charged so far.",
                money::usd(ov.order.usd_cents()),
                short(site)
            ),
            None,
        ),
        "paid" => (format!("Paid. {f}."), None),
        "baking" if site.archetype == "bakery" => (format!("Baking now. {f}."), None),
        "baking" => (format!("Being packed. {f}."), None),
        "ready" => ("Ready to collect.".into(), None),
        "out" => ("Out for delivery now.".into(), None),
        "delivered" => (format!("Delivered {}.", at("delivered")), None),
        "collected" => (format!("Collected {}.", at("collected")), None),
        "declined" => (format!("{owner} declined it. Nothing was charged."), None),
        "lapsed" => ("The request lapsed. Nothing was charged.".into(), None),
        "cancelled" => ("Cancelled. Nothing was charged.".into(), None),
        "refunded" => ("Refunded.".into(), None),
        s => (capital(&state_of(site, s)), None),
    }
}

#[derive(Debug, Clone, Serialize)]
struct Step {
    label: String,
    when: String,
    s: String,
    tag: String,
}

fn timeline(site: &Site, ov: &OrderView, zones: &[Zone]) -> Vec<Step> {
    let owner = owner(site);
    let o = &ov.order;
    let mut plan: Vec<&str> = vec!["requested"];
    if o.kind == "custom" {
        plan.push("approved");
    }
    plan.extend(["paid", "baking"]);
    if o.fulfilment == "collection" {
        plan.extend(["ready", "collected"]);
    } else {
        plan.extend(["out", "delivered"]);
    }
    let event = |step: &str| ov.events.iter().rev().find(|e| e.step == step);
    let slot = ov.slot.as_ref().map(|s| s.label.clone());
    let ended = ENDED.contains(&o.state.as_str());
    let mut steps = Vec::new();
    let mut pending_seen = false;
    for step in plan {
        let label = match step {
            "requested" if o.kind == "custom" => "Requested".to_owned(),
            "requested" => "Ordered".into(),
            "approved" => format!(
                "Approved by {}",
                event("approved").map_or(owner.clone(), |e| e.actor.clone())
            ),
            "paid" => "Paid".into(),
            "baking" if site.archetype == "bakery" => "Baking".into(),
            "baking" => "Packed".into(),
            "ready" => "Ready to collect".into(),
            "out" => "Out for delivery".into(),
            "delivered" => "Delivered".into(),
            _ => "Collected".into(),
        };
        if let Some(e) = event(step) {
            // Tracking: each step's time and who did it (the shop's agent, the customer's).
            let mut when = match step {
                "requested" | "paid" => day_clock(site, e.at),
                _ => format!("{} \u{b7} {}", day_clock(site, e.at), e.actor),
            };
            if step == "out"
                && let (Some(courier), Some(tracking)) = (&o.courier, &o.tracking)
            {
                when = format!("{when} \u{b7} {courier}, tracking {tracking}");
            }
            if step == "paid"
                && let Some(p) = settled(ov)
            {
                when = format!(
                    "{} by {} \u{b7} {when}",
                    money::fmt(&p.currency, p.amount_minor),
                    rail_words(&p.rail)
                );
            }
            steps.push(Step {
                label,
                when,
                s: "done".into(),
                tag: "Done".into(),
            });
            continue;
        }
        if ended {
            break;
        }
        let when = match step {
            "approved" => format!("Waiting for {owner}'s OK. Nothing charged."),
            "paid" if PAYABLE.contains(&o.state.as_str()) => "Not paid yet".into(),
            "paid" => format!("After {owner}'s OK"),
            "baking" => slot
                .as_ref()
                .map(|s| format!("Before {s}"))
                .unwrap_or_default(),
            "out" | "ready" => slot.clone().unwrap_or_default(),
            "delivered" => o
                .recipient
                .clone()
                .unwrap_or_else(|| zone_label(zones, o.zone.as_deref())),
            _ => site.fact("/address"),
        };
        let (s, tag) = if pending_seen {
            ("next", "Later")
        } else {
            ("now", "Next")
        };
        pending_seen = true;
        steps.push(Step {
            label,
            when,
            s: s.into(),
            tag: tag.into(),
        });
    }
    if ended {
        let when = event(&o.state)
            .map(|e| {
                format!(
                    "{} \u{b7} {}",
                    e.note.clone().unwrap_or_default(),
                    day_clock(site, e.at)
                )
            })
            .unwrap_or_default();
        steps.push(Step {
            label: capital(&state_of(site, &o.state)),
            when,
            s: "end".into(),
            tag: "Ended".into(),
        });
    }
    steps
}

#[derive(Debug, Clone, Serialize)]
struct Update {
    who: String,
    text: String,
    when: String,
}

fn updates(site: &Site, ov: &OrderView) -> Vec<Update> {
    ov.events
        .iter()
        .map(|e| Update {
            who: e.actor.clone(),
            text: e
                .note
                .clone()
                .unwrap_or_else(|| capital(&e.step.replace('_', " "))),
            when: day_clock(site, e.at),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
struct Card {
    head: Head,
    line: String,
    extra: Option<String>,
}

fn card(site: &Site, ov: &OrderView, zones: &[Zone]) -> Card {
    let (line, extra) = desk_line(site, ov, zones);
    Card {
        head: head(site, ov, zones),
        line,
        extra,
    }
}

// ---------- Orders list and guest lookup (K-07) ----------

#[derive(Deserialize, Default)]
struct LookupQuery {
    #[serde(default)]
    number: Option<String>,
}

async fn orders(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<LookupQuery>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let records = app.records().await?;
    let site = &ctx.site;
    if let Some(number) = q.number.as_deref().map(normalise).filter(|n| !n.is_empty()) {
        return match access(records, &ctx, &number).await? {
            Access::Mine(ov) => Ok(see_other(&format!("/orders/{}", ov.order.order_id))),
            Access::Theirs(ov) => Ok(status_page(&app, &ctx, Some(&ov), &number).await?),
            Access::Missing(id) => Ok(status_page(&app, &ctx, None, &id).await?),
        };
    }
    let zones = records.zones(&site.site).await?;
    let mut cards = Vec::new();
    if let Some(v) = &ctx.visitor {
        for ov in records.orders_for_customer(site, &v.account).await? {
            cards.push(card(site, &ov, &zones));
        }
    }
    let shell = app
        .shell(
            &ctx,
            "orders",
            "Your orders",
            "Where each of your orders stands, and how to pay, change or report a problem.",
        )
        .await?;
    let page = json!({
        "desk": orders_desk(&shell),
        "browse": browse(&shell),
        "shell": shell,
        "signed_in": ctx.visitor.is_some(),
        "cards": cards,
        "lookup": Value::Null,
        "number": "",
        "prefix": prefix(site),
        "confirmations": site.fact("/confirmations"),
        "sign_in": sign_in("/orders"),
        "owner": owner(site),
    });
    Ok(html(app.render("shop/orders.html", &page)?))
}

/// Status only, for someone who isn't the order's customer (or a number that doesn't exist).
async fn status_page(
    app: &App,
    ctx: &Ctx,
    ov: Option<&OrderView>,
    id: &str,
) -> anyhow::Result<Response> {
    let site = &ctx.site;
    let shell = app
        .shell(
            ctx,
            "orders",
            "Order status",
            "The status of an order, without its details.",
        )
        .await?;
    let lookup = match ov {
        Some(ov) => {
            let (pill, tone) = pill(site, &ov.order.state);
            json!({
                "found": true,
                "id": ov.order.order_id,
                "pill": pill,
                "tone": tone,
                "line": format!("{} is {}.", ov.order.order_id, state_of(site, &ov.order.state)),
                "waiting": ov.order.state == "held",
            })
        }
        None => json!({
            "found": false,
            "id": id,
            "pill": "",
            "tone": "",
            "line": format!("There's no order {id} on this site. Check the number on your confirmation."),
            "waiting": false,
        }),
    };
    let page = json!({
        "desk": orders_desk(&shell),
        "shell": shell,
        "signed_in": ctx.visitor.is_some(),
        "cards": Vec::<Card>::new(),
        "lookup": lookup,
        "number": id,
        "prefix": prefix(site),
        "confirmations": site.fact("/confirmations"),
        "browse": Vec::<Value>::new(),
        "sign_in": sign_in(&format!("/orders/{id}")),
        "owner": owner(site),
    });
    let status = if ov.is_some() {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    Ok((
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        app.render("shop/orders.html", &page)?,
    )
        .into_response())
}

// ---------- Order page (K-08) ----------

async fn order_page(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let zones = records.zones(&site.site).await?;
    let h = head(site, &ov, &zones);
    let (line, extra) = desk_line(site, &ov, &zones);
    let can_change = ov.slot.is_some()
        && !h.ended
        && !["out", "delivered", "collected"].contains(&ov.order.state.as_str());
    let can_help = AFTER_PAID.contains(&ov.order.state.as_str());
    let shell = app
        .shell(
            &ctx,
            "orders",
            &ov.order.order_id,
            "Where this order stands, its payment and what you can do next.",
        )
        .await?;
    let page = json!({
        "desk": orders_desk(&shell),
        "shell": shell,
        "o": h,
        "line": line,
        "extra": extra,
        "steps": timeline(site, &ov, &zones),
        "updates": updates(site, &ov),
        "request": request_card(records, site, &ov).await?,
        "can_change": can_change,
        "can_help": can_help,
        "cancel": cancel_view(site, &ov, time::now()),
        "receipt": settled(&ov).is_some(),
        "domain": site.fact("/domain"),
        "owner": owner(site),
        "short": short(site),
    });
    Ok(html(app.render("shop/order.html", &page)?))
}

/// The payment request: amount, payee, order (K-08 after approval, K-10), and "Price list of
/// Sun 1 Nov" for the list the order is pinned to.
async fn request_card(records: &Records, site: &Site, ov: &OrderView) -> anyhow::Result<Value> {
    let o = &ov.order;
    let list = match o.price_list.as_deref() {
        Some(v) => capital(
            crate::common::list_name_of(records, site, v)
                .await?
                .trim_start_matches("the "),
        ),
        None => String::new(),
    };
    Ok(json!({
        "usd": money::usd(o.usd_cents()),
        "local": cash(site, o.total_minor),
        "rate": money::rate_line(o.usd_rate_minor),
        "payee": site.name,
        "summary": format!("{} \u{b7} {}", o.order_id, title(ov)),
        "price_list": list,
    }))
}

// ---------- Pay (K-10) ----------

#[derive(Deserialize, Default)]
struct PayQuery {
    #[serde(default)]
    cancelled: Option<String>,
}

async fn pay(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<PayQuery>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let notice = q
        .cancelled
        .map(|_| ("Payment cancelled. Nothing was charged.".to_owned(), "warn"));
    pay_page(&app, &ctx, &ov, notice, None).await
}

fn sample_ok(app: &App, site: &Site, ov: &OrderView) -> bool {
    site.is_sample()
        && (app.payments.is_stand_in() || ov.payments.iter().any(|p| p.state == "not_executed"))
}

async fn pay_page(
    app: &App,
    ctx: &Ctx,
    ov: &OrderView,
    notice: Option<(String, &str)>,
    provider: Option<String>,
) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let zones = records.zones(&site.site).await?;
    let owner = owner(site);
    let state = ov.order.state.as_str();
    let why_not = match state {
        "held" => Some(format!(
            "Nothing to pay yet. {} is waiting for {owner}'s OK, and nothing has been charged. The payment request appears on your orders page after she approves.",
            ov.order.order_id
        )),
        s if ENDED.contains(&s) => Some(format!(
            "{} is {}, so there's nothing to pay.",
            ov.order.order_id,
            state_of(site, s)
        )),
        s if AFTER_PAID.contains(&s) => Some(format!("{} is paid.", ov.order.order_id)),
        _ => None,
    };
    let shell = app
        .shell(
            ctx,
            "orders",
            &format!("Pay for {}", ov.order.order_id),
            "The amount, the payee and the rate, before PayPal opens.",
        )
        .await?;
    let page = json!({
        "shell": shell,
        "o": head(site, ov, &zones),
        "request": request_card(records, site, ov).await?,
        "summary": format!("{} \u{b7} {}", title(ov), fulfil(ov, &zones)),
        "why_not": why_not,
        "notice": notice.as_ref().map(|(w, _)| w.clone()),
        "notice_tone": notice.as_ref().map(|(_, t)| (*t).to_owned()).unwrap_or_default(),
        "provider": provider,
        "sample_ok": sample_ok(app, site, ov),
        "short": short(site),
        "owner": owner,
    });
    Ok(html(app.render("shop/pay.html", &page)?))
}

fn payment_row(provider_ref: Option<String>, cents: i64, state: &str, now: i64) -> Payment {
    Payment {
        payment_id: Records::new_payment_id(),
        rail: "paypal".into(),
        provider_ref,
        amount_minor: cents,
        currency: "USD".into(),
        state: state.into(),
        created_at: now,
        settled_at: matches!(state, "captured" | "sample").then_some(now),
    }
}

async fn pay_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    if !PAYABLE.contains(&ov.order.state.as_str()) {
        return pay_page(&app, &ctx, &ov, None, None).await;
    }
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let order_id = ov.order.order_id.clone();
    let origin = ctx.host.origin.clone();
    let request = PaymentRequest {
        site: site.site.clone(),
        order_id: order_id.clone(),
        amount_usd_cents: ov.order.usd_cents(),
        description: format!(
            "{} order {}{}",
            site.name,
            order_id,
            if site.is_sample() { " (Sample)" } else { "" }
        ),
        customer: ctx.visitor.as_ref().map(|v| v.account.clone()),
        return_url: format!("{origin}/orders/{order_id}/paid"),
        cancel_url: format!("{origin}/orders/{order_id}/pay?cancelled=1"),
    };
    match app.payments.create(request).await {
        Ok(PaymentStart::Redirect {
            provider_ref,
            approve_url,
            ..
        }) => {
            records
                .record_payment(
                    &site.site,
                    &order_id,
                    &payment_row(Some(provider_ref), ov.order.usd_cents(), "started", now),
                )
                .await?;
            Ok(see_other(&approve_url))
        }
        Ok(PaymentStart::NotExecuted { words }) => {
            if !app.payments.is_stand_in() {
                records
                    .record_payment(
                        &site.site,
                        &order_id,
                        &payment_row(None, ov.order.usd_cents(), "not_executed", now),
                    )
                    .await?;
            }
            let ov = records.order(site, &order_id).await?.unwrap_or(ov);
            pay_page(&app, &ctx, &ov, None, Some(words)).await
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), order = order_id, "payment start failed");
            pay_page(
                &app,
                &ctx,
                &ov,
                Some((
                    "We couldn't reach PayPal. Nothing was charged. Try again in a minute.".into(),
                    "risk",
                )),
                None,
            )
            .await
        }
    }
}

async fn pay_sample(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let site = &ctx.site;
    if !sample_ok(&app, site, &ov) || !PAYABLE.contains(&ov.order.state.as_str()) {
        return pay_page(&app, &ctx, &ov, None, None).await;
    }
    let records = app.records().await?;
    let now = time::now();
    let order_id = ov.order.order_id.clone();
    records
        .record_payment(
            &site.site,
            &order_id,
            &payment_row(None, ov.order.usd_cents(), "sample", now),
        )
        .await?;
    let words = format!(
        "Marked paid on the sample: {} at {}'s rate. No money moved.",
        money::usd(ov.order.usd_cents()),
        short(site)
    );
    match records.mark_paid(site, &order_id, &words, now).await {
        Ok(v) => {
            app.tell_paid(&ctx, &v).await;
            Ok(see_other(&format!("/orders/{order_id}/receipt")))
        }
        Err(OrderError::Refused(_)) => Ok(see_other(&format!("/orders/{order_id}/receipt"))),
        Err(OrderError::Other(e)) => Err(e.into()),
    }
}

#[derive(Deserialize, Default)]
struct PaidQuery {
    #[serde(default)]
    token: Option<String>,
}

async fn paid(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<PaidQuery>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let order_id = ov.order.order_id.clone();
    if AFTER_PAID.contains(&ov.order.state.as_str()) {
        return Ok(see_other(&format!("/orders/{order_id}/receipt")));
    }
    let token = q.token.unwrap_or_default();
    let Some(started) = ov
        .payments
        .iter()
        .find(|p| !token.is_empty() && p.provider_ref.as_deref() == Some(token.as_str()))
        .cloned()
    else {
        let words = "This PayPal return doesn't match a payment for this order, so nothing was marked paid and nothing was charged twice. Pay again below, or open the order.";
        return pay_page(&app, &ctx, &ov, Some((words.into(), "warn")), None).await;
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let capture = match app.payments.capture(&site.site, &token).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), order = order_id, "capture failed");
            let words = "We couldn't reach PayPal to finish the payment. You haven't been charged twice. Reload this page in a minute.";
            return pay_page(&app, &ctx, &ov, Some((words.into(), "risk")), None).await;
        }
    };
    let mut row = started;
    row.state.clone_from(&capture.state);
    row.settled_at = (capture.state == "captured").then_some(now);
    if let Some(id) = &capture.provider_capture_id {
        row.provider_ref = Some(id.clone());
    }
    records.record_payment(&site.site, &order_id, &row).await?;
    if capture.state == "captured" {
        let words = format!(
            "Paid {} with PayPal at {}'s rate.",
            money::fmt(&row.currency, row.amount_minor),
            short(site)
        );
        return match records.mark_paid(site, &order_id, &words, now).await {
            Ok(v) => {
                app.tell_paid(&ctx, &v).await;
                Ok(see_other(&format!("/orders/{order_id}/receipt")))
            }
            Err(OrderError::Refused(_)) => Ok(see_other(&format!("/orders/{order_id}/receipt"))),
            Err(OrderError::Other(e)) => Err(e.into()),
        };
    }
    let (words, tone) = match capture.state.as_str() {
        "pending" => (
            "PayPal is still confirming the payment. Reload this page in a few minutes to see whether it went through.",
            "warn",
        ),
        "declined" => (
            "PayPal declined the payment. Nothing was charged. Try again, or use another PayPal account.",
            "risk",
        ),
        "cancelled" => ("Payment cancelled. Nothing was charged.", "warn"),
        _ => (
            "Payments aren't switched on for this sample yet. Nothing was charged.",
            "risk",
        ),
    };
    let ov = records.order(site, &order_id).await?.unwrap_or(ov);
    pay_page(&app, &ctx, &ov, Some((words.into(), tone)), None).await
}

// ---------- Receipt (K-11) ----------

async fn receipt(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let zones = records.zones(&site.site).await?;
    let off = site.utc_offset_min;
    let lines: Vec<Value> = ov
        .lines
        .iter()
        .map(|l| {
            let name = if l.sku.is_none() && l.name == "Delivery" {
                let slot = ov
                    .slot
                    .as_ref()
                    .map(|s| format!(", {}", s.label))
                    .unwrap_or_default();
                format!(
                    "Delivery to {}{slot}",
                    zone_label(&zones, ov.order.zone.as_deref())
                )
            } else if l.qty > 1 {
                format!(
                    "{} \u{d7} {}, {} each",
                    l.name,
                    l.qty,
                    cash(site, l.unit_minor)
                )
            } else {
                l.name.clone()
            };
            json!({ "name": name, "amount": cash(site, l.amount_minor) })
        })
        .collect();
    let payment = settled(&ov).map(|p| {
        let at = p.settled_at.unwrap_or(p.created_at);
        json!({
            "charged": format!("{}, by {}", money::fmt(&p.currency, p.amount_minor), rail_words(&p.rail)),
            "amount": money::fmt(&p.currency, p.amount_minor),
            "usd": p.currency == "USD",
            "rail": rail_words(&p.rail),
            "paid_at": format!("{}, {} ({})", time::fmt_date_long(at, off), time::fmt_clock(at, off), zone_name(site)),
            "id": p.payment_id,
            "provider_ref": p.provider_ref,
            "sample": p.state == "sample",
        })
    });
    let owner = owner(site);
    let refunds: Vec<Value> = ov
        .refunds
        .iter()
        .map(|r| {
            let words = match r.state.as_str() {
                "done" if r.rule.is_some() => format!(
                    "Refunded {} under {owner}'s rule \u{b7} {}",
                    cash(site, r.amount_minor),
                    r.refund_id
                ),
                "done" => format!(
                    "Refunded {} by {owner} \u{b7} {}",
                    cash(site, r.amount_minor),
                    r.refund_id
                ),
                "waiting" => format!(
                    "Refund of {} waiting for {owner}'s OK",
                    cash(site, r.amount_minor)
                ),
                _ => format!(
                    "Refund of {} declined by {owner}",
                    cash(site, r.amount_minor)
                ),
            };
            json!({ "id": r.refund_id, "state": r.state, "words": words })
        })
        .collect();
    let shell = app
        .shell(
            &ctx,
            "orders",
            &format!("Receipt \u{b7} {}", ov.order.order_id),
            "The lines, the total, what was charged and the pinned rate.",
        )
        .await?;
    let page = json!({
        "shell": shell,
        "o": head(site, &ov, &zones),
        "lines": lines,
        "payment": payment,
        "rate": money::rate_line(ov.order.usd_rate_minor),
        "payee": site.name,
        "domain": site.fact("/domain"),
        "short": short(site),
        "refunds": refunds,
    });
    Ok(html(app.render("shop/receipt.html", &page)?))
}

// ---------- Change the date (K-12) ----------

fn cutoff_hours(site: &Site) -> i64 {
    site.facts
        .get("changes_cutoff_hours")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// The waiting change request's words, if one is open.
fn held_change(ov: &OrderView) -> Option<String> {
    ov.events
        .iter()
        .rev()
        .find(|e| e.step.starts_with("change_"))
        .filter(|e| e.step == "change_requested")
        .map(|e| e.note.clone().unwrap_or_default())
}

#[derive(Deserialize, Default)]
struct ChangeQuery {
    #[serde(default)]
    done: Option<String>,
}

async fn change(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<ChangeQuery>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let notice = (q.done.as_deref() == Some("withdrawn")).then(|| {
        format!(
            "Request withdrawn. {} stays {}.",
            if ov.order.fulfilment == "collection" {
                "Collection"
            } else {
                "Delivery"
            },
            ov.slot
                .as_ref()
                .map(|s| s.label.clone())
                .unwrap_or_default()
        )
    });
    change_page(&app, &ctx, &ov, notice, None).await
}

async fn change_page(
    app: &App,
    ctx: &Ctx,
    ov: &OrderView,
    notice: Option<String>,
    error: Option<String>,
) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let zones = records.zones(&site.site).await?;
    let owner = owner(site);
    let hours = cutoff_hours(site);
    let (open, cutoff, slots, closed) = match &ov.slot {
        None => (
            false,
            String::new(),
            Vec::new(),
            format!(
                "This order has no delivery or collection time to move. For anything else, ask {owner}."
            ),
        ),
        Some(slot) => {
            let cutoff_at = slot.starts_at - hours * 3600;
            let state = ov.order.state.as_str();
            if ENDED.contains(&state) {
                (
                    false,
                    day_clock(site, cutoff_at),
                    Vec::new(),
                    format!(
                        "{} is {}, so there's nothing to move.",
                        ov.order.order_id,
                        state_of(site, state)
                    ),
                )
            } else if ["out", "delivered", "collected", "ready"].contains(&state)
                || now >= cutoff_at
            {
                let why = match state {
                    "out" | "delivered" | "collected" | "ready" => {
                        format!("It's {} now.", state_of(site, state))
                    }
                    _ if ov.order.kind == "custom" => "The cake is being made.".into(),
                    _ => format!("{owner} is getting it ready."),
                };
                (
                    false,
                    day_clock(site, cutoff_at),
                    Vec::new(),
                    format!("Changes closed {}. {why}", day_clock(site, cutoff_at)),
                )
            } else {
                let options: Vec<SlotView> = records
                    .slots(site, &slot.kind, now, now + 21 * 86_400)
                    .await?
                    .iter()
                    .filter(|s| {
                        !s.full && s.slot_id != slot.slot_id && s.starts_at - hours * 3600 > now
                    })
                    .map(|s| slot_view(site, s, None))
                    .collect();
                (true, day_clock(site, cutoff_at), options, String::new())
            }
        }
    };
    let shell = app
        .shell(
            ctx,
            "orders",
            &format!("Change {}", ov.order.order_id),
            "Ask to move the date before the cut-off.",
        )
        .await?;
    let page = json!({
        "desk": orders_desk(&shell),
        "shell": shell,
        "o": head(site, ov, &zones),
        "now": format!("{}{}", ov.slot.as_ref().map(|s| s.label.clone()).unwrap_or_default(), ov.order.recipient.as_ref().map(|r| format!(", {r}")).unwrap_or_default()),
        "open": open,
        "cutoff": cutoff,
        "zone": zone_name(site),
        "slots": slots,
        "closed": closed,
        "held": held_change(ov),
        "notice": notice,
        "error": error,
        "owner": owner,
        "rule": site.fact("/changes"),
    });
    Ok(html(app.render("shop/change.html", &page)?))
}

#[derive(Deserialize)]
struct ChangeForm {
    slot: String,
}

async fn change_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<ChangeForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let owner = owner(site);
    if held_change(&ov).is_some() {
        return change_page(&app, &ctx, &ov, None, Some(format!("A request is already waiting for {owner}. Withdraw it first to ask for another time."))).await;
    }
    let Some(current) = &ov.slot else {
        return change_page(&app, &ctx, &ov, None, None).await;
    };
    let hours = cutoff_hours(site);
    let open = now < current.starts_at - hours * 3600
        && !ENDED.contains(&ov.order.state.as_str())
        && !["out", "delivered", "collected", "ready"].contains(&ov.order.state.as_str());
    let target = records.slot(site, f.slot.trim()).await?.filter(|s| {
        open && s.kind == current.kind
            && !s.full
            && s.slot_id != current.slot_id
            && s.starts_at - hours * 3600 > now
    });
    let Some(target) = target else {
        return change_page(
            &app,
            &ctx,
            &ov,
            None,
            Some("That time can't be picked now. Choose one of the open times.".into()),
        )
        .await;
    };
    let who = ctx
        .visitor
        .as_ref()
        .map(|v| v.name.clone())
        .unwrap_or_default();
    let note = format!(
        "Asked to move {} to {}. Held for {owner}'s OK; nothing changes until she says yes.",
        ov.order.order_id, target.label
    );
    records
        .note_event(
            &site.site,
            &ov.order.order_id,
            "change_requested",
            &who,
            &note,
            now,
        )
        .await?;
    Ok(see_other(&format!("/orders/{}/change", ov.order.order_id)))
}

async fn change_withdraw(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    if held_change(&ov).is_some() {
        let records = app.records().await?;
        let who = ctx
            .visitor
            .as_ref()
            .map(|v| v.name.clone())
            .unwrap_or_default();
        records
            .note_event(
                &ctx.site.site,
                &ov.order.order_id,
                "change_withdrawn",
                &who,
                "Withdrew the request to move it.",
                time::now(),
            )
            .await?;
    }
    Ok(see_other(&format!(
        "/orders/{}/change?done=withdrawn",
        ov.order.order_id
    )))
}

// ---------- Cancel before it's made ----------

/// Whether the customer can cancel now: `now` (nothing charged, so it ends at once), `ask`
/// (paid and not started yet, so the owner decides and refunds), `waiting` (asked), `closed`
/// (too late here) or `none` (it has ended).
fn cancel_state(site: &Site, ov: &OrderView, now: i64) -> (&'static str, String) {
    let owner = owner(site);
    let state = ov.order.state.as_str();
    let made = if site.archetype == "bakery" {
        "baked"
    } else {
        "packed"
    };
    if ["awaiting_payment", "held", "approved"].contains(&state) {
        return (
            "now",
            "Nothing has been charged, so it's cancelled at once.".into(),
        );
    }
    if state == "paid" {
        if ov.cancel_asked().is_some() {
            return (
                "waiting",
                format!(
                    "You asked to cancel. {owner} decides, {}; if she agrees, the whole amount goes back the way you paid.",
                    crate::common::owner_reply(site)
                ),
            );
        }
        let cutoff = ov
            .slot
            .as_ref()
            .map(|s| s.starts_at - cutoff_hours(site) * 3600);
        return match cutoff {
            Some(c) if now >= c => (
                "closed",
                format!(
                    "Cancelling here closed {}. If something has changed, ask {owner}.",
                    day_clock(site, c)
                ),
            ),
            Some(c) => (
                "ask",
                format!(
                    "It's paid and not {made} yet. You can ask to cancel until {}. {owner} decides; if she agrees, the whole amount goes back the way you paid.",
                    day_clock(site, c)
                ),
            ),
            None => (
                "ask",
                format!(
                    "It's paid and not {made} yet. {owner} decides; if she agrees, the whole amount goes back the way you paid."
                ),
            ),
        };
    }
    if ["baking", "ready", "out"].contains(&state) {
        return (
            "closed",
            format!(
                "It's {} now, so it can't be cancelled here. If something is wrong, use Something's wrong.",
                state_of(site, state)
            ),
        );
    }
    ("none", String::new())
}

fn cancel_view(site: &Site, ov: &OrderView, now: i64) -> Value {
    let (can, words) = cancel_state(site, ov, now);
    let button = match can {
        "now" => "Cancel this order".to_owned(),
        "ask" => format!("Ask {} to cancel", owner(site)),
        _ => String::new(),
    };
    json!({ "can": can, "words": words, "button": button })
}

/// `POST /orders/{id}/cancel`: an unpaid order is cancelled at once; a paid one not yet made
/// goes to the owner as a request she decides in her Orders app.
async fn cancel_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let who = ctx
        .visitor
        .as_ref()
        .map_or_else(|| ov.order.contact_name.clone(), |v| v.name.clone());
    let order_id = ov.order.order_id.clone();
    match cancel_state(site, &ov, now).0 {
        "now" => match records
            .advance(site, &order_id, "cancelled", &who, now)
            .await
        {
            Ok(_) | Err(OrderError::Refused(_)) => {}
            Err(OrderError::Other(e)) => return Err(e.into()),
        },
        "ask" => {
            let note = format!(
                "Asked to cancel {order_id} before it's made. Held for {}'s OK; nothing is refunded until she says yes.",
                owner(site)
            );
            records
                .note_event(&site.site, &order_id, "cancel_requested", &who, &note, now)
                .await?;
        }
        _ => {}
    }
    Ok(see_other(&format!("/orders/{order_id}#cancel")))
}

// ---------- Something's wrong (K-13) ----------

/// The owner's standing refund rule, from the manifest.
struct RefundRule {
    id: String,
    per_order_minor: i64,
    per_week: usize,
    until_at: Option<i64>,
    until: String,
}

fn refund_rule(site: &Site) -> Option<RefundRule> {
    let r = site.facts.get("refund_rule")?;
    let until = r
        .get("until")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let until_at = chrono::NaiveDate::parse_from_str(&until, "%d %b %Y")
        .ok()
        .and_then(|d| time::at_local(d, "23:59", site.utc_offset_min).ok());
    Some(RefundRule {
        id: r.get("id")?.as_str()?.to_owned(),
        per_order_minor: r.get("per_order")?.as_i64()? * 100,
        per_week: r
            .get("per_week")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0),
        until_at,
        until,
    })
}

#[derive(Deserialize, Default)]
struct HelpForm {
    #[serde(default)]
    what: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    confirm: Option<String>,
}

#[derive(Serialize)]
struct Item {
    key: String,
    label: String,
    amount: String,
    checked: bool,
}

/// What can be refunded: each line of a catalogue order, or the whole order.
fn refundable(ov: &OrderView) -> Vec<(String, String, i64)> {
    let mut out: Vec<(String, String, i64)> = if ov.order.kind == "custom" {
        Vec::new()
    } else {
        ov.lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.sku.is_some())
            .map(|(i, l)| {
                (
                    format!("l{i}"),
                    if l.qty > 1 {
                        format!("{} \u{d7} {}", l.name, l.qty)
                    } else {
                        l.name.clone()
                    },
                    l.amount_minor,
                )
            })
            .collect()
    };
    let all = if ov.order.kind == "custom" {
        title(ov)
    } else {
        "All of it".into()
    };
    out.push(("all".into(), all, ov.order.total_minor));
    out
}

async fn help(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
) -> Page {
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    help_page(&app, &ctx, &ov, &HelpForm::default(), Value::Null, None).await
}

async fn help_page(
    app: &App,
    ctx: &Ctx,
    ov: &OrderView,
    f: &HelpForm,
    outcome: Value,
    error: Option<String>,
) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let zones = records.zones(&site.site).await?;
    let owner = owner(site);
    let state = ov.order.state.as_str();
    let closed = if AFTER_PAID.contains(&state) {
        None
    } else if ENDED.contains(&state) {
        Some(format!(
            "{} is {}. For anything else, ask {owner}.",
            ov.order.order_id,
            state_of(site, state)
        ))
    } else {
        Some(format!(
            "Nothing has been charged on {}, so there's nothing to refund. To move it, use Change; for anything else, ask {owner}.",
            ov.order.order_id
        ))
    };
    let chosen = if f.what.is_empty() {
        "all"
    } else {
        f.what.as_str()
    };
    let items: Vec<Item> = refundable(ov)
        .into_iter()
        .map(|(key, label, amount)| Item {
            checked: key == chosen,
            key,
            label,
            amount: cash(site, amount),
        })
        .collect();
    let shell = app
        .shell(
            ctx,
            "orders",
            &format!("Something's wrong with {}", ov.order.order_id),
            "Tell the Orders desk what went wrong.",
        )
        .await?;
    let page = json!({
        "desk": orders_desk(&shell),
        "shell": shell,
        "o": head(site, ov, &zones),
        "closed": closed,
        "items": items,
        "message": f.message,
        "what": chosen,
        "outcome": outcome,
        "error": error,
        "rule_line": match refund_rule(site) {
            Some(rule) => format!("A refund of up to {} can run at once under {owner}'s rule; anything more goes to her.", cash(site, rule.per_order_minor)),
            None => format!("{owner} decides every refund herself."),
        },
        "owner": owner,
        "refunds": ov.refunds.iter().map(|r| json!({ "id": r.refund_id, "amount": cash(site, r.amount_minor), "state": r.state, "reason": r.reason })).collect::<Vec<_>>(),
    });
    Ok(html(app.render("shop/help.html", &page)?))
}

async fn help_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<HelpForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    if !sells(&ctx.site) {
        return Ok(not_found());
    }
    let ov = match mine(&app, &ctx, &id).await? {
        Ok(ov) => ov,
        Err(r) => return Ok(r),
    };
    if !AFTER_PAID.contains(&ov.order.state.as_str()) {
        return help_page(&app, &ctx, &ov, &f, Value::Null, None).await;
    }
    let message: String = f.message.trim().chars().take(600).collect();
    if message.chars().count() < 3 {
        return help_page(
            &app,
            &ctx,
            &ov,
            &f,
            Value::Null,
            Some(
                "Say what went wrong in a sentence, so the Cashier and the owner can see it."
                    .into(),
            ),
        )
        .await;
    }
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let owner = owner(site);
    let Some((_, label, line_amount)) = refundable(&ov).into_iter().find(|(k, _, _)| *k == f.what)
    else {
        return help_page(
            &app,
            &ctx,
            &ov,
            &f,
            Value::Null,
            Some("Pick what went wrong.".into()),
        )
        .await;
    };
    let taken: i64 = ov
        .refunds
        .iter()
        .filter(|r| r.state != "declined")
        .map(|r| r.amount_minor)
        .sum();
    let left = ov.order.total_minor - taken;
    let amount = if f.what == "all" { left } else { line_amount };
    if amount <= 0 || amount > left {
        let words = format!(
            "That's more than is left to refund on {} ({}).",
            ov.order.order_id,
            cash(site, left.max(0))
        );
        return help_page(&app, &ctx, &ov, &f, Value::Null, Some(words)).await;
    }
    let rail = settled(&ov).map_or_else(|| "paypal".to_owned(), |p| p.rail.clone());
    let usd =
        (rail == "paypal").then(|| money::usd(money::usd_cents(amount, ov.order.usd_rate_minor)));
    let to = format!("your {}", rail_words(&rail));
    let rule = refund_rule(site);
    let ruled_here: i64 = ov
        .refunds
        .iter()
        .filter(|r| r.state == "done" && r.rule.is_some())
        .map(|r| r.amount_minor)
        .sum();
    let week = match &rule {
        Some(rule) => records
            .orders_for_owner(site, None, 1000)
            .await?
            .iter()
            .flat_map(|o| o.refunds.iter())
            .filter(|r| {
                r.state == "done"
                    && r.rule.as_deref() == Some(rule.id.as_str())
                    && r.created_at >= now - 7 * 86_400
            })
            .count(),
        None => 0,
    };
    let mut checks = Vec::new();
    let covered = match &rule {
        Some(rule) => {
            let in_amount = ruled_here + amount <= rule.per_order_minor;
            let in_week = week < rule.per_week;
            let in_date = rule.until_at.is_none_or(|u| now <= u);
            checks.push(if in_amount {
                format!(
                    "Amount: {}, within {} per order",
                    cash(site, amount),
                    cash(site, rule.per_order_minor)
                )
            } else {
                format!(
                    "Amount: {} is above {} per order, so the rule doesn't cover it",
                    cash(site, amount + ruled_here),
                    cash(site, rule.per_order_minor)
                )
            });
            checks.push(format!(
                "Recipient: the original {} payment",
                rail_words(&rail)
            ));
            checks.push(if in_week {
                format!(
                    "Frequency: number {} of at most {} this week",
                    week + 1,
                    rule.per_week
                )
            } else {
                format!(
                    "Frequency: this week's {} refunds under the rule are used up",
                    rule.per_week
                )
            });
            checks.push(if in_date {
                format!("Expiry: the rule runs to {}", rule.until)
            } else {
                format!("Expiry: the rule ended {}", rule.until)
            });
            in_amount && in_week && in_date
        }
        None => {
            checks.push(format!(
                "{owner} hasn't set a refund rule, so she decides every refund"
            ));
            false
        }
    };
    let money_words = match &usd {
        Some(u) => format!(
            "{}, which is {u} at the rate pinned on {}",
            cash(site, amount),
            ov.order.order_id
        ),
        None => cash(site, amount),
    };
    let thing = if f.what == "all" {
        "the whole order".to_owned()
    } else {
        format!("the {}", label.to_lowercase())
    };
    if f.confirm.is_none() {
        let outcome = json!({
            "step": "offer",
            "covered": covered,
            "words": if covered {
                format!("{} can be refunded: {money_words}. It goes back to {to}, the way you paid.", capital(&thing))
            } else {
                format!("Refunding {thing} ({money_words}) is more than the Cashier can do without {owner}, so {owner} decides.")
            },
            "button": if covered { format!("Refund {thing}") } else { format!("Ask {owner} to refund {thing}") },
            "checks": checks,
            "pill": "",
            "small": "",
        });
        return help_page(&app, &ctx, &ov, &f, outcome, None).await;
    }
    let digits: String = ov
        .order
        .order_id
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let refund_id = format!("R-{digits}-{}", ov.refunds.len() + 1);
    let who = ctx
        .visitor
        .as_ref()
        .map(|v| v.name.clone())
        .unwrap_or_default();
    let mut refund = Refund {
        refund_id: refund_id.clone(),
        order_id: ov.order.order_id.clone(),
        amount_minor: amount,
        reason: message.clone(),
        rule: None,
        state: "waiting".into(),
        created_at: now,
    };
    let outcome = if covered {
        refund.rule = rule.as_ref().map(|r| r.id.clone());
        refund.state = "done".into();
        records.record_refund(&site.site, &refund).await?;
        let paid = usd.clone().unwrap_or_else(|| cash(site, amount));
        let note = format!(
            "Refunded {paid} for {thing} to {who}'s {} under {owner}'s rule. Receipt {refund_id}.",
            rail_words(&rail)
        );
        records
            .note_event(
                &site.site,
                &ov.order.order_id,
                "refunded",
                "the Cashier",
                &note,
                now,
            )
            .await?;
        app.tell_customer(
            site,
            &ctx.host.host,
            &ctx.host.origin,
            &ov,
            agentrix_messages::orders::OrderEvent::Refunded {
                refund_id: refund_id.clone(),
                amount: paid.clone(),
                local: usd
                    .is_some()
                    .then(|| format!("{} at the order's rate", cash(site, amount))),
                by: "The Cashier".into(),
                what: thing.to_string(),
            },
        )
        .await;
        json!({
            "step": "done",
            "covered": true,
            "words": format!("Refunded {paid} to {to}. Receipt {refund_id}."),
            "button": "",
            "pill": format!("Refunded \u{b7} {}", cash(site, amount)),
            "small": if rail == "paypal" { "PayPal also emails you its own refund receipt." } else { "" },
            "checks": checks,
        })
    } else {
        records.record_refund(&site.site, &refund).await?;
        let note = format!(
            "Asked for a refund of {} for {thing}. Waiting for {owner}'s OK; nothing refunded yet.",
            cash(site, amount)
        );
        records
            .note_event(
                &site.site,
                &ov.order.order_id,
                "refund_requested",
                &who,
                &note,
                now,
            )
            .await?;
        app.tell_customer(
            site,
            &ctx.host.host,
            &ctx.host.origin,
            &ov,
            agentrix_messages::orders::OrderEvent::RefundHeld {
                amount: cash(site, amount),
                what: thing.to_string(),
                decide_by: crate::notify::two_days_on(site, now),
            },
        )
        .await;
        json!({
            "step": "waiting",
            "covered": false,
            "words": format!("That's above what can be refunded without {owner}. She will decide, {}. Nothing has been refunded yet.", crate::common::owner_reply(site)),
            "button": "",
            "pill": format!("Waiting for {owner}'s OK"),
            "small": "",
            "checks": checks,
        })
    };
    let ov = records.order(site, &ov.order.order_id).await?.unwrap_or(ov);
    help_page(
        &app,
        &ctx,
        &ov,
        &HelpForm {
            what: f.what.clone(),
            message,
            confirm: None,
        },
        outcome,
        None,
    )
    .await
}

// ---------- Account (A-09, A-10, A-20: W2's parts) ----------

// The error is the redirect to sign-in, returned as the page itself by every caller; boxing it
// would only add a deref at each call.
#[allow(clippy::result_large_err)]
fn needs_visitor(ctx: &Ctx, next: &str) -> Result<Visitor, Response> {
    ctx.visitor
        .clone()
        .ok_or_else(|| see_other(&format!("/account/sign-in?next={}", urlencode(next))))
}

async fn account(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let v = match needs_visitor(&ctx, "/account") {
        Ok(v) => v,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let customer = records.customer(&site.site, &v.account).await?;
    let zones = records.zones(&site.site).await?;
    let mut cards = Vec::new();
    if sells(site) {
        for ov in records
            .orders_for_customer(site, &v.account)
            .await?
            .iter()
            .take(5)
        {
            cards.push(card(site, ov, &zones));
        }
    }
    let saved = records.addresses(&site.site, &v.account).await?.len();
    let shell = app
        .shell(
            &ctx,
            "",
            "Your account",
            "Your details, your orders and how you sign in.",
        )
        .await?;
    let page = json!({
        "shell": shell,
        "name": customer.as_ref().map_or_else(|| v.name.clone(), |c| c.name.clone()),
        "email": v.email,
        "verified": v.email_verified,
        "phone": customer.as_ref().and_then(|c| c.phone.clone()).or(v.phone.clone()).unwrap_or_default(),
        "cards": cards,
        "sells": sells(site),
        "saved": saved,
        "short": short(site),
    });
    Ok(html(app.render("shop/account.html", &page)?))
}

#[derive(Deserialize, Default)]
struct DetailsForm {
    #[serde(default)]
    name: String,
    #[serde(default)]
    phone: String,
}

async fn details(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let v = match needs_visitor(&ctx, "/account/details") {
        Ok(v) => v,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let customer = records.customer(&ctx.site.site, &v.account).await?;
    let f = DetailsForm {
        name: customer
            .as_ref()
            .map_or_else(|| v.name.clone(), |c| c.name.clone()),
        phone: customer
            .as_ref()
            .and_then(|c| c.phone.clone())
            .or(v.phone.clone())
            .unwrap_or_default(),
    };
    details_page(&app, &ctx, &v, &f, None, None).await
}

async fn details_page(
    app: &App,
    ctx: &Ctx,
    v: &Visitor,
    f: &DetailsForm,
    error: Option<String>,
    saved: Option<String>,
) -> Page {
    let shell = app
        .shell(
            ctx,
            "",
            "Your details",
            "Your name and phone number for orders.",
        )
        .await?;
    let page = json!({ "shell": shell, "name": f.name, "phone": f.phone, "email": v.email, "error": error, "saved": saved, "short": short(&ctx.site) });
    Ok(html(app.render("shop/details.html", &page)?))
}

async fn details_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Form(f): Form<DetailsForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    let v = match needs_visitor(&ctx, "/account/details") {
        Ok(v) => v,
        Err(r) => return Ok(r),
    };
    let name: String = f.name.trim().chars().take(80).collect();
    let phone = f.phone.trim().to_owned();
    if name.is_empty() {
        return details_page(
            &app,
            &ctx,
            &v,
            &f,
            Some("Add the name to put on your orders.".into()),
            None,
        )
        .await;
    }
    if !phone.is_empty() && !phone_ok(&phone) {
        return details_page(&app, &ctx, &v, &f, Some(PHONE_WORDS.into()), None).await;
    }
    let records = app.records().await?;
    records
        .upsert_customer(
            &ctx.site.site,
            &v.account,
            &name,
            &v.email,
            (!phone.is_empty()).then_some(phone.as_str()),
            time::now(),
        )
        .await?;
    details_page(
        &app,
        &ctx,
        &v,
        &DetailsForm { name, phone },
        None,
        Some("Saved. New orders use these details.".into()),
    )
    .await
}

#[derive(Deserialize, Default)]
struct AddressForm {
    #[serde(default)]
    action: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    recipient: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    area: String,
    #[serde(default)]
    details: String,
    #[serde(default)]
    notes: String,
}

async fn addresses(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let v = match needs_visitor(&ctx, "/account/addresses") {
        Ok(v) => v,
        Err(r) => return Ok(r),
    };
    addresses_page(&app, &ctx, &v, &AddressForm::default(), None, None).await
}

async fn addresses_page(
    app: &App,
    ctx: &Ctx,
    v: &Visitor,
    f: &AddressForm,
    error: Option<String>,
    saved: Option<String>,
) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let zones = records.zones(&site.site).await?;
    let list: Vec<Value> = records
        .addresses(&site.site, &v.account)
        .await?
        .into_iter()
        .map(|a| {
            json!({
                "id": a.address_id,
                "recipient": a.recipient,
                "phone": a.phone.unwrap_or_default(),
                "place": place(&a.details, &a.area),
                "zone": zone_label(&zones, Some(&a.zone)),
                "notes": a.notes,
            })
        })
        .collect();
    let areas: Vec<Value> = zones
        .iter()
        .flat_map(|z| z.areas.iter().map(move |area| (area, z)))
        .map(|(area, z)| json!({ "area": area, "label": format!("{area} \u{b7} {} delivery", cash(site, z.fee_minor)), "checked": f.area == *area }))
        .collect();
    let covered: Vec<String> = zones.iter().flat_map(|z| z.areas.clone()).collect();
    let shell = app
        .shell(
            ctx,
            "",
            "Addresses",
            "Where to deliver, and who receives it.",
        )
        .await?;
    let page = json!({
        "shell": shell,
        "list": list,
        "areas": areas,
        "covered": covered.join(", "),
        "f": { "recipient": f.recipient, "phone": f.phone, "details": f.details, "notes": f.notes },
        "error": error,
        "saved": saved,
        "short": short(site),
    });
    Ok(html(app.render("shop/addresses.html", &page)?))
}

async fn addresses_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    headers: HeaderMap,
    Form(f): Form<AddressForm>,
) -> Page {
    if let Some(r) = foreign(&ctx, &headers) {
        return Ok(r);
    }
    let v = match needs_visitor(&ctx, "/account/addresses") {
        Ok(v) => v,
        Err(r) => return Ok(r),
    };
    let records = app.records().await?;
    let site = &ctx.site;
    if f.action == "remove" {
        let gone = records
            .remove_address(&site.site, &v.account, f.id.trim())
            .await?;
        let words = if gone {
            "Address removed."
        } else {
            "That address was already gone."
        };
        return addresses_page(
            &app,
            &ctx,
            &v,
            &AddressForm::default(),
            None,
            Some(words.into()),
        )
        .await;
    }
    let recipient: String = f.recipient.trim().chars().take(80).collect();
    if recipient.is_empty() {
        return addresses_page(
            &app,
            &ctx,
            &v,
            &f,
            Some("Add who receives deliveries there.".into()),
            None,
        )
        .await;
    }
    let phone = f.phone.trim().to_owned();
    if !phone.is_empty() && !phone_ok(&phone) {
        return addresses_page(&app, &ctx, &v, &f, Some(PHONE_WORDS.into()), None).await;
    }
    let Some(zone) = records.zone_for(&site.site, &f.area).await? else {
        let covered: Vec<String> = records
            .zones(&site.site)
            .await?
            .into_iter()
            .flat_map(|z| z.areas)
            .collect();
        let area = if f.area.trim().is_empty() {
            "that area".to_owned()
        } else {
            f.area.trim().to_owned()
        };
        let words = format!(
            "{} doesn't deliver to {area}. It delivers to {}.",
            short(site),
            covered.join(", ")
        );
        return addresses_page(&app, &ctx, &v, &f, Some(words), None).await;
    };
    let area = zone
        .areas
        .iter()
        .find(|a| a.eq_ignore_ascii_case(f.area.trim()))
        .cloned()
        .unwrap_or_else(|| f.area.trim().to_owned());
    let new = NewAddress {
        recipient,
        phone: (!phone.is_empty()).then_some(phone),
        area,
        zone: zone.zone.clone(),
        details: f.details.trim().chars().take(160).collect(),
        notes: f.notes.trim().chars().take(240).collect(),
    };
    records
        .add_address(&site.site, &v.account, &new, time::now())
        .await?;
    let words = format!("Saved. Checkout offers it for delivery to {}.", zone.label);
    addresses_page(&app, &ctx, &v, &AddressForm::default(), None, Some(words)).await
}

// ---------- The Orders desk's answer ----------

/// "KB-1042" from a question, given the site's prefix.
fn find_number(q: &str, prefix: &str) -> Option<String> {
    let up = q.to_uppercase();
    let p = prefix.to_uppercase();
    let mut from = 0;
    while let Some(pos) = up[from..].find(&p) {
        let start = from + pos + p.len();
        let rest = &up[start..];
        let rest = rest
            .strip_prefix('-')
            .or_else(|| rest.strip_prefix(' '))
            .unwrap_or(rest);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if (3..=6).contains(&digits.len()) {
            return Some(format!("{p}-{digits}"));
        }
        from = start;
    }
    up.split(|c: char| !c.is_ascii_digit())
        .find(|d| (4..=6).contains(&d.len()))
        .map(|d| format!("{p}-{d}"))
}

/// The Orders desk's answer to "where is my order?": status only for anyone but the order's own
/// customer, who also gets the time and a link to the order page.
pub async fn order_status_answer(app: &App, ctx: &Ctx, q: &str) -> anyhow::Result<Answer> {
    let records = app.records().await?;
    let site = &ctx.site;
    let prefix = prefix(site);
    let mut a = Answer::instant("orders");
    a.meta = format!(
        "From {}'s order records, {}",
        short(site),
        time::fmt_clock(time::now(), site.utc_offset_min)
    );
    let Some(id) = find_number(q, &prefix) else {
        a.say(format!(
            "Type your order number, like **{prefix}-1042**, and I'll tell you where it stands."
        ));
        a.action(Action::link("Open my orders", "/orders", true));
        return Ok(a);
    };
    a.progress.push(format!("Looking up {id}"));
    let Some(ov) = records.order(site, &id).await? else {
        a.say(format!(
            "There's no order **{id}** on this site. Check the number on your confirmation."
        ));
        a.action(Action::link("Open my orders", "/orders", false));
        return Ok(a);
    };
    let zones = records.zones(&site.site).await?;
    let state = ov.order.state.as_str();
    let words = state_of(site, state);
    let mine = ctx
        .visitor
        .as_ref()
        .is_some_and(|v| ov.order.customer_id.as_deref() == Some(v.account.as_str()));
    let confirmations = site.fact("/confirmations");
    if mine {
        a.say(format!("**{id}** ({}) is **{words}**.", title(&ov)));
        if ov.slot.is_some()
            && !ENDED.contains(&state)
            && !["delivered", "collected"].contains(&state)
        {
            a.say(format!("{}.", capital(&fulfil(&ov, &zones))));
        }
    } else {
        a.say(format!("**{id}** is **{words}**."));
    }
    if state == "held" {
        a.waiting = true;
        if !confirmations.is_empty() {
            a.say(confirmations);
        }
    }
    if mine {
        a.action(Action::link(
            &format!("Open {id}"),
            &format!("/orders/{id}"),
            true,
        ));
        if PAYABLE.contains(&state) {
            a.action(Action::link(
                &format!("Pay for {id}"),
                &format!("/orders/{id}/pay"),
                false,
            ));
        }
    } else {
        a.say("For your privacy I've shown only the status. Sign in with the account on the order to see the details.".into());
        if ctx.visitor.is_none() {
            a.action(Action::link(
                "Sign in to see details",
                &format!(
                    "/account/sign-in?next={}",
                    urlencode(&format!("/orders/{id}"))
                ),
                true,
            ));
        }
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_numbers_are_found_in_questions() {
        assert_eq!(
            find_number("where is kb-1042?", "KB").as_deref(),
            Some("KB-1042")
        );
        assert_eq!(
            find_number("Track KB 1043 please", "KB").as_deref(),
            Some("KB-1043")
        );
        assert_eq!(
            find_number("my order 1044", "KB").as_deref(),
            Some("KB-1044")
        );
        assert_eq!(find_number("where is my order", "KB"), None);
    }

    #[test]
    fn back_links_stay_on_the_site() {
        assert_eq!(
            local_path(Some("/bread#item-sourdough")).as_deref(),
            Some("/bread#item-sourdough")
        );
        assert_eq!(local_path(Some("//evil.example")), None);
        assert_eq!(local_path(Some("https://evil.example")), None);
    }

    #[test]
    fn phones_are_digits_with_a_country_code() {
        assert!(phone_ok("+44 7700 900418"));
        assert!(phone_ok("0712 345678"));
        assert!(!phone_ok("call me"));
        assert!(!phone_ok("12"));
    }
}
