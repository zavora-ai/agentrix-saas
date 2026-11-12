//! Kilele's cake studio (K-04) and the simple cake form (K-18).
//!
//! The studio is server-rendered from the live price list (every option with its price and
//! constraints) and the next 21 days of delivery and collection slots. Its island
//! (`studio.js`) asks one thing at a time, draws the sketch from parts and asks
//! `POST /cakes/quote` for the price on every pick: rules price, no model. `POST /cakes/request`
//! (signed-in customers) and `POST /order-a-cake` (anyone, no JavaScript) both make the same
//! held request for the owner. Nothing is charged until she confirms.
//!
//! The studio's conversation and its idea of the cake are the Cakes desk's ([`desk`]):
//! `POST /cakes/talk` (the gateway's `concierge` purpose) and `POST /cakes/preview` (W11's
//! `image` purpose), with rules as the fallback.

use std::collections::BTreeMap;

use agentrix_records::orders::{Contact, OrderError, Placement, Slot, Zone};
use agentrix_records::pricing::{CakeChoice, CakeQuote, Flag, PriceList, Rule};
use agentrix_records::{Records, Site, money, time};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::App;
use crate::shell::{Desk, Shell};
use crate::web::{Ctx, Fail, Page, html};

pub(crate) mod desk;

/// Days the calendar shows, from today.
const CALENDAR_DAYS: i64 = 21;

/// Where the studio sends a guest who wants to request a cake.
const SIGN_IN: &str = "/account/sign-in?why=order&next=/cakes/studio";

/// The studio's routes.
pub fn routes() -> Router<App> {
    Router::new()
        .route("/cakes/studio", get(studio))
        .route("/cakes/quote", post(quote))
        .route("/cakes/request", post(request))
        .route("/order-a-cake", get(order_form).post(order_post))
        .merge(desk::routes())
}

/// One option from the live price rules, as a chip, a select option and island data.
#[derive(Debug, Clone, Serialize)]
pub struct Opt {
    /// Rule code, `sponge.lemon`.
    pub code: String,
    /// "Lemon sponge".
    pub label: String,
    /// "Lemon", for chips.
    pub short: String,
    /// "KSh 4,500" for sizes, "+KSh 250" for extras, "" when included.
    pub price: String,
    /// Colour swatch, for buttercream.
    pub swatch: String,
    /// "25–30", for sizes.
    pub serves: String,
    /// Days of notice, for sizes.
    pub notice_days: i64,
    /// Tiers, for sizes.
    pub tiers: i64,
    /// Sizes it is limited to, as rule codes.
    pub only_sizes: Vec<String>,
    /// "1 and 2 tiers only", or "".
    pub only: String,
}

/// The options by group, in the order the studio asks.
#[derive(Debug, Clone, Serialize)]
pub struct Groups {
    /// Sizes.
    pub size: Vec<Opt>,
    /// Sponges.
    pub sponge: Vec<Opt>,
    /// Buttercream colours.
    pub colour: Vec<Opt>,
    /// Finishes.
    pub finish: Vec<Opt>,
    /// Decorations.
    pub deco: Vec<Opt>,
}

/// A slot on a calendar day.
#[derive(Debug, Clone, Serialize)]
pub struct DaySlot {
    /// Slot id.
    pub id: String,
    /// "10:00–12:00".
    pub time: String,
    /// No room left.
    pub full: bool,
}

/// A day on the studio's calendar.
#[derive(Debug, Clone, Serialize)]
pub struct Day {
    /// `2026-11-14`.
    pub date: String,
    /// "Sat 14 Nov".
    pub label: String,
    /// "Sat".
    pub dow: String,
    /// "14".
    pub num: String,
    /// Days from today.
    pub away: i64,
    /// Delivery slots.
    pub delivery: Vec<DaySlot>,
    /// Collection slots.
    pub collection: Vec<DaySlot>,
    /// Why the day can't be picked, by size code (`""` for no size yet) and then by kind
    /// (`""`, `delivery`, `collection`); `""` when it can.
    pub why: BTreeMap<String, BTreeMap<String, String>>,
    /// Why it can't be picked before anything is chosen, or "".
    pub reason: String,
}

/// The delivery zone, as the studio words it.
#[derive(Debug, Clone, Serialize)]
pub struct ZoneView {
    /// `zone1`.
    pub zone: String,
    /// "Westlands and Kilimani".
    pub label: String,
    /// "KSh 300".
    pub fee: String,
    /// "Westlands or Kilimani".
    pub areas: String,
}

/// Everything both cake pages read from records and the manifest.
struct Book {
    list: PriceList,
    rules: Vec<Rule>,
    groups: Groups,
    max_chars: i64,
    days: Vec<Day>,
    zone: Option<Zone>,
}

fn strip_group_word(label: &str) -> String {
    let lower = label.to_lowercase();
    for word in [" buttercream", " sponge", " finish"] {
        if lower.ends_with(word) {
            return label[..label.len() - word.len()].to_owned();
        }
    }
    label.to_owned()
}

fn or_list(items: &[String], joiner: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} {joiner} {last}", init.join(", ")),
    }
}

fn opts(rules: &[Rule], grp: &str, currency: &str) -> Vec<Opt> {
    let size_label = |code: &str| {
        rules
            .iter()
            .find(|r| r.code == code)
            .map(|r| {
                r.label
                    .trim_end_matches(" tiers")
                    .trim_end_matches(" tier")
                    .to_owned()
            })
            .unwrap_or_default()
    };
    rules
        .iter()
        .filter(|r| r.grp == grp)
        .map(|r| {
            let meta = r.meta();
            let only_sizes: Vec<String> = meta
                .get("only_sizes")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let only = if only_sizes.is_empty() {
                String::new()
            } else {
                let names: Vec<String> = only_sizes.iter().map(|c| size_label(c)).collect();
                format!("{} tiers only", or_list(&names, "and"))
            };
            Opt {
                code: r.code.clone(),
                short: strip_group_word(&r.label),
                label: r.label.clone(),
                price: if grp == "size" {
                    money::fmt(currency, r.amount_minor)
                } else if r.amount_minor > 0 {
                    format!("+{}", money::fmt(currency, r.amount_minor))
                } else {
                    String::new()
                },
                swatch: meta
                    .get("swatch")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                serves: meta
                    .get("serves")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                notice_days: meta.get("notice_days").and_then(Value::as_i64).unwrap_or(0),
                tiers: r
                    .code
                    .strip_prefix("size.")
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0),
                only_sizes,
                only,
            }
        })
        .collect()
}

fn day_slots(slots: &[Slot], from: i64, to: i64, off: i32) -> Vec<DaySlot> {
    slots
        .iter()
        .filter(|s| s.starts_at >= from && s.starts_at < to)
        .map(|s| DaySlot {
            id: s.slot_id.clone(),
            time: format!(
                "{}\u{2013}{}",
                time::fmt_clock(s.starts_at, off),
                time::fmt_clock(s.ends_at, off)
            ),
            full: s.full,
        })
        .collect()
}

/// Why a day can't take a cake with this much notice, for this kind (`""` for either).
fn why_not(day: &Day, notice: i64, kind: &str) -> String {
    if day.away < notice {
        return format!("Needs {notice} days' notice");
    }
    let (slots, none): (Vec<&DaySlot>, &str) = match kind {
        "delivery" => (day.delivery.iter().collect(), "No delivery this day"),
        "collection" => (day.collection.iter().collect(), "No collection this day"),
        _ => (
            day.delivery.iter().chain(day.collection.iter()).collect(),
            "Closed for cake orders",
        ),
    };
    if slots.is_empty() {
        none.to_owned()
    } else if slots.iter().all(|s| s.full) {
        match kind {
            "delivery" => "Delivery is full".into(),
            "collection" => "Collection is full".into(),
            _ => "Fully booked".into(),
        }
    } else {
        String::new()
    }
}

async fn book(records: &Records, site: &Site, now: i64) -> anyhow::Result<Book> {
    let list = records
        .live_price_list(&site.site)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no live price list"))?;
    let rules = records.rules(&site.site, &list.version).await?;
    let groups = Groups {
        size: opts(&rules, "size", &site.currency),
        sponge: opts(&rules, "sponge", &site.currency),
        colour: opts(&rules, "colour", &site.currency),
        finish: opts(&rules, "finish", &site.currency),
        deco: opts(&rules, "deco", &site.currency),
    };
    let max_chars = rules
        .iter()
        .find(|r| r.code == "message")
        .and_then(|r| r.meta().get("max_chars").and_then(Value::as_i64))
        .unwrap_or(22);
    let off = site.utc_offset_min;
    let start = time::day_start(now, off);
    let end = start + CALENDAR_DAYS * 86_400 + 3_600;
    let delivery = records.slots(site, "delivery", start, end).await?;
    let collection = records.slots(site, "collection", start, end).await?;
    let today = time::local(now, off).date_naive();
    let min_notice = groups.size.iter().map(|s| s.notice_days).min().unwrap_or(0);
    let mut days = Vec::new();
    for away in 0..CALENDAR_DAYS {
        let date = today + chrono::Duration::days(away);
        let from = time::at_local(date, "00:00", off)?;
        let to = time::at_local(date + chrono::Duration::days(1), "00:00", off)?;
        let mut day = Day {
            date: date.format("%Y-%m-%d").to_string(),
            label: time::fmt_day(from, off),
            dow: date.format("%a").to_string(),
            num: date.format("%-d").to_string(),
            away,
            delivery: day_slots(&delivery, from, to, off),
            collection: day_slots(&collection, from, to, off),
            why: BTreeMap::new(),
            reason: String::new(),
        };
        let mut sizes: Vec<(String, i64)> = vec![(String::new(), min_notice)];
        sizes.extend(groups.size.iter().map(|s| (s.code.clone(), s.notice_days)));
        for (code, notice) in sizes {
            let by_kind = ["", "delivery", "collection"]
                .iter()
                .map(|k| ((*k).to_owned(), why_not(&day, notice, k)))
                .collect();
            day.why.insert(code, by_kind);
        }
        day.reason = day
            .why
            .get("")
            .and_then(|m| m.get(""))
            .cloned()
            .unwrap_or_default();
        days.push(day);
    }
    let zone = records.zones(&site.site).await?.into_iter().next();
    Ok(Book {
        list,
        rules,
        groups,
        max_chars,
        days,
        zone,
    })
}

fn zone_view(zone: Option<&Zone>, currency: &str) -> Option<ZoneView> {
    zone.map(|z| ZoneView {
        zone: z.zone.clone(),
        label: z.label.clone(),
        fee: money::fmt(currency, z.fee_minor),
        areas: or_list(&z.areas, "or"),
    })
}

fn desk(shell: &Shell, id: &str) -> Desk {
    shell
        .desks
        .iter()
        .find(|d| d.id == id)
        .cloned()
        .unwrap_or_else(|| shell.front.clone())
}

fn not_offered(site: &Site) -> Vec<String> {
    site.facts
        .get("not_offered")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// "2 days' notice, 3 tiers need 3", from the size rules.
fn notice_words(sizes: &[Opt]) -> String {
    let min = sizes.iter().map(|s| s.notice_days).min().unwrap_or(0);
    let longer: Vec<&Opt> = sizes.iter().filter(|s| s.notice_days > min).collect();
    match longer.as_slice() {
        [] => format!("{min} days' notice"),
        more => {
            let names: Vec<String> = more.iter().map(|s| s.label.clone()).collect();
            let max = more.iter().map(|s| s.notice_days).max().unwrap_or(min);
            format!(
                "{min} days' notice, {} need{} {max}",
                or_list(&names, "and"),
                if names.len() == 1 && !names[0].ends_with('s') {
                    "s"
                } else {
                    ""
                }
            )
        }
    }
}

#[derive(Serialize)]
struct StudioView {
    shell: Shell,
    desk: Desk,
    orders_desk: Desk,
    groups: Groups,
    max_chars: i64,
    version: String,
    valid_until: String,
    rate: String,
    days: Vec<Day>,
    zone: Option<ZoneView>,
    address: String,
    owner: String,
    confirmations: String,
    notice: String,
    gf_only: String,
    visitor: Option<String>,
    data: Value,
}

async fn studio(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let b = book(records, site, now).await?;
    let shell = app
        .shell(&ctx, "cakes", "Cake studio", "Design a celebration cake from Kilele's own options, priced line by line from the price list. Wanjiru confirms every cake.")
        .await?;
    let zone = zone_view(b.zone.as_ref(), &site.currency);
    let owner = site.fact("/owner/first");
    let valid_until = time::fmt_day(b.list.valid_until, site.utc_offset_min);
    let gf_only = b
        .groups
        .sponge
        .iter()
        .find(|o| !o.only.is_empty())
        .map(|o| format!("{} is for {}.", o.label, o.only.trim_end_matches(" only")))
        .unwrap_or_default();
    let data = json!({
        "groups": b.groups,
        "max_chars": b.max_chars,
        "days": b.days,
        "not_offered": not_offered(site),
        "owner": owner,
        "signed_in": ctx.visitor.is_some(),
        "visitor": ctx.visitor.as_ref().map(|v| v.name.clone()),
        "sign_in": SIGN_IN,
        "zone": zone,
        "address": site.fact("/address"),
        "version": b.list.version,
        "valid_until": valid_until,
        "gf_only": gf_only,
        "desk_name": desk(&shell, "cakes").name,
        // What an idea of the cake draws for an unchosen finish or decoration (ADR-032).
        "idea_defaults": {
            "finish": default_code(&b.rules, "finish", "finish.smooth"),
            "deco": default_code(&b.rules, "deco", "deco.none"),
        },
    });
    let view = StudioView {
        desk: desk(&shell, "cakes"),
        orders_desk: desk(&shell, "orders"),
        shell,
        notice: notice_words(&b.groups.size),
        groups: b.groups,
        max_chars: b.max_chars,
        version: b.list.version.clone(),
        valid_until,
        rate: money::rate_line(b.list.usd_rate_minor),
        days: b.days,
        zone,
        address: site.fact("/address"),
        confirmations: site.fact("/confirmations"),
        owner,
        gf_only,
        visitor: ctx.visitor.as_ref().map(|v| v.name.clone()),
        data,
    };
    Ok(html(app.render("kilele/studio.html", &view)?))
}

/// What the island sends for a quote or a request.
#[derive(Debug, Deserialize)]
struct CakeBody {
    #[serde(flatten)]
    choice: CakeChoice,
    #[serde(default)]
    fulfilment: Option<String>,
    #[serde(default)]
    recipient: String,
    #[serde(default)]
    idempotency_key: Option<String>,
    /// The idea of the cake the page showed (its digest), if any: kept with the request only
    /// when it was drawn for exactly these options.
    #[serde(default)]
    idea: Option<String>,
}

fn blank_to_none(v: &mut Option<String>) {
    if v.as_deref().is_some_and(|s| s.trim().is_empty()) {
        *v = None;
    }
}

fn tidy(mut c: CakeChoice) -> CakeChoice {
    for v in [
        &mut c.size,
        &mut c.sponge,
        &mut c.colour,
        &mut c.finish,
        &mut c.deco,
        &mut c.date,
    ] {
        blank_to_none(v);
    }
    c.message = c.message.trim().to_owned();
    c.note = c.note.trim().to_owned();
    c
}

fn kind_of(f: Option<&str>) -> Option<&'static str> {
    match f {
        Some("delivery") => Some("delivery"),
        Some("collection") => Some("collection"),
        _ => None,
    }
}

/// The slots of a kind on a local day.
async fn slots_on(
    records: &Records,
    site: &Site,
    kind: &str,
    date: &str,
) -> anyhow::Result<Vec<Slot>> {
    let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
        return Ok(Vec::new());
    };
    let from = time::at_local(day, "00:00", site.utc_offset_min)?;
    records.slots(site, kind, from, from + 86_400).await
}

#[derive(Debug, Serialize)]
struct LineOut {
    code: String,
    label: String,
    amount: String,
}

/// A priced design, every figure worded on the server.
#[derive(Debug, Serialize)]
struct Priced {
    lines: Vec<LineOut>,
    total: String,
    usd: String,
    rate: String,
    flags: Vec<Flag>,
    complete: bool,
    has_size: bool,
    serves: Option<String>,
    earliest: Option<String>,
    slot: Option<String>,
    slot_id: Option<String>,
    price_list: String,
    valid_until: String,
    total_minor: i64,
}

struct Quoted {
    quote: CakeQuote,
    priced: Priced,
    zone: Option<Zone>,
}

async fn price(
    records: &Records,
    site: &Site,
    choice: &CakeChoice,
    fulfilment: Option<&str>,
    now: i64,
) -> anyhow::Result<Quoted> {
    let quote = records.quote_cake(site, choice, now).await?;
    let off = site.utc_offset_min;
    let cur = site.currency.as_str();
    let mut lines: Vec<LineOut> = quote
        .lines
        .iter()
        .map(|l| LineOut {
            code: l.code.clone(),
            label: l.label.clone(),
            amount: if l.code.starts_with("size.") {
                money::fmt(cur, l.amount_minor)
            } else if l.amount_minor > 0 {
                format!("+{}", money::fmt(cur, l.amount_minor))
            } else if l.code == "message" {
                "free".into()
            } else {
                "included".into()
            },
        })
        .collect();
    let mut flags = quote.flags.clone();
    let mut total = quote.total_minor;
    let zone = records.zones(&site.site).await?.into_iter().next();
    let kind = kind_of(fulfilment);
    if kind == Some("delivery") {
        match &zone {
            Some(z) => {
                total += z.fee_minor;
                lines.push(LineOut {
                    code: format!("delivery.{}", z.zone),
                    label: format!("Delivery, {}", z.label),
                    amount: format!("+{}", money::fmt(cur, z.fee_minor)),
                });
            }
            None => flags.push(Flag {
                level: "error",
                text: "Kilele has no delivery area on the site yet. Pick collection.".into(),
            }),
        }
    }
    let mut slot = None;
    if let (Some(k), Some(date)) = (kind, choice.date.as_deref()) {
        let day = slots_on(records, site, k, date).await?;
        slot = day.iter().find(|s| !s.full).cloned();
        if slot.is_none()
            && !flags
                .iter()
                .any(|f| f.level == "error" && f.text.contains("notice"))
        {
            let label = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map(|d| d.format("%a %-d %b").to_string())
                .unwrap_or_else(|_| date.to_owned());
            let text = match (day.is_empty(), k) {
                (true, "delivery") => format!(
                    "Kilele doesn't deliver on {label}. Pick a day with delivery, or collect it."
                ),
                (true, _) => format!("There's no collection on {label}. Pick another day."),
                (false, _) => format!("{label} is full for {k}. Pick another day."),
            };
            flags.push(Flag {
                level: "error",
                text,
            });
        }
    }
    let ready = quote.complete
        && kind.is_some()
        && slot.is_some()
        && !flags.iter().any(|f| f.level == "error");
    let earliest = quote
        .earliest_day
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| d.format("%a %-d %b").to_string());
    let priced = Priced {
        lines,
        total: money::fmt(cur, total),
        usd: money::usd(money::usd_cents(total, quote.usd_rate_minor)),
        rate: money::rate_line(quote.usd_rate_minor),
        flags,
        complete: ready,
        has_size: choice.size.is_some(),
        serves: quote.serves.clone(),
        earliest,
        slot: slot.as_ref().map(|s| s.label.clone()),
        slot_id: slot.as_ref().map(|s| s.slot_id.clone()),
        price_list: quote.price_list.clone(),
        valid_until: time::fmt_day(quote.valid_until, off),
        total_minor: total,
    };
    Ok(Quoted {
        quote,
        priced,
        zone,
    })
}

async fn quote(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Json(body): Json<CakeBody>,
) -> Result<Response, Fail> {
    let records = app.records().await?;
    let choice = tidy(body.choice);
    let q = price(
        records,
        &ctx.site,
        &choice,
        body.fulfilment.as_deref(),
        time::now(),
    )
    .await?;
    Ok(Json(q.priced).into_response())
}

fn refused(words: impl Into<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "refused": words.into() })),
    )
        .into_response()
}

/// Who a held request is for and how to recognise a retry.
struct Who {
    contact: Contact,
    customer_id: Option<String>,
    key: Option<String>,
}

/// Makes the held request; shared by the studio and the simple form.
async fn place(
    records: &Records,
    site: &Site,
    choice: &CakeChoice,
    fulfilment: Option<&str>,
    who: Who,
    now: i64,
) -> anyhow::Result<Result<(String, Priced), String>> {
    let Who {
        contact,
        customer_id,
        key,
    } = who;
    let Some(kind) = kind_of(fulfilment) else {
        return Ok(Err("Pick delivery or collection.".into()));
    };
    let q = price(records, site, choice, Some(kind), now).await?;
    if let Some(f) = q.priced.flags.iter().find(|f| f.level == "error") {
        return Ok(Err(f.text.clone()));
    }
    if !q.quote.complete {
        return Ok(Err(
            "Finish the design first: every choice and a day.".into()
        ));
    }
    let Some(slot_id) = q.priced.slot_id.clone() else {
        return Ok(Err(
            "That day has no slot with room now. Pick another day.".into()
        ));
    };
    if kind == "delivery"
        && contact
            .recipient
            .as_deref()
            .is_none_or(|r| r.trim().is_empty())
    {
        let areas = q
            .zone
            .as_ref()
            .map(|z| or_list(&z.areas, "or"))
            .unwrap_or_default();
        return Ok(Err(format!(
            "Say who receives the cake and where, in {areas}."
        )));
    }
    let how = Placement {
        customer_id,
        channel: "site".into(),
        agent: None,
        proof: None,
        fulfilment: kind.into(),
        slot_id: Some(slot_id),
        zone: if kind == "delivery" {
            q.zone.as_ref().map(|z| z.zone.clone())
        } else {
            None
        },
        note: None,
        idempotency_key: key
            .filter(|k| !k.trim().is_empty())
            .map(|k| k.chars().take(80).collect()),
    };
    match records
        .request_custom(site, choice, &q.quote, &contact, &how, now)
        .await
    {
        Ok(view) => Ok(Ok((view.order.order_id, q.priced))),
        Err(OrderError::Refused(words)) => Ok(Err(words)),
        Err(OrderError::Other(e)) => Err(e),
    }
}

async fn request(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Json(body): Json<CakeBody>,
) -> Result<Response, Fail> {
    let Some(visitor) = ctx.visitor.clone() else {
        return Ok((StatusCode::UNAUTHORIZED, Json(json!({ "signin": SIGN_IN }))).into_response());
    };
    let records = app.records().await?;
    let site = &ctx.site;
    let mut choice = tidy(body.choice);
    if body.idea.is_some()
        && let Some(list) = records.live_price_list(&site.site).await?
    {
        let rules = records.rules(&site.site, &list.version).await?;
        choice.idea = desk::idea_for(&app, site, &rules, &choice, body.idea.as_deref()).await?;
    }
    let recipient = body.recipient.trim();
    let contact = Contact {
        name: visitor.name.clone(),
        email: Some(visitor.email.clone()).filter(|e| !e.is_empty()),
        phone: visitor.phone.clone(),
        recipient: (!recipient.is_empty()).then(|| recipient.chars().take(160).collect()),
    };
    let owner = site.fact("/owner/first");
    let who = Who {
        contact,
        customer_id: Some(visitor.account.clone()),
        key: body.idempotency_key,
    };
    match place(
        records,
        site,
        &choice,
        body.fulfilment.as_deref(),
        who,
        time::now(),
    )
    .await?
    {
        Ok((id, priced)) => {
            app.tell_held_by_id(&ctx, &id).await;
            Ok(Json(json!({
                "order_id": id,
                "url": format!("/orders/{id}"),
                "pill": format!("{id} \u{b7} held for {owner}"),
                "status": format!("Waiting for {owner}'s OK. Nothing charged."),
                "total": priced.total,
                "usd": priced.usd,
            }))
            .into_response())
        }
        Err(words) => Ok(refused(words)),
    }
}

/// The simple form's fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CakeForm {
    #[serde(default)]
    size: String,
    #[serde(default)]
    sponge: String,
    #[serde(default)]
    fulfilment: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    recipient: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    key: String,
}

/// A day the simple form offers.
#[derive(Debug, Serialize)]
struct FormDay {
    date: String,
    label: String,
}

/// The held card after the simple form.
#[derive(Debug, Serialize)]
struct Held {
    order_id: String,
    url: String,
    total: String,
    usd: String,
    rate: String,
    cake: String,
    message: String,
    note: String,
    when: String,
    last_step: String,
    signed_in: bool,
    phone: String,
}

#[derive(Serialize)]
struct FormView {
    shell: Shell,
    sizes: Vec<Opt>,
    sponges: Vec<Opt>,
    days: Vec<FormDay>,
    max_chars: i64,
    version: String,
    valid_until: String,
    zone: Option<ZoneView>,
    address: String,
    notice: String,
    owner: String,
    confirmations: String,
    defaults: String,
    form: CakeForm,
    error: String,
    held: Option<Held>,
}

/// The default buttercream, finish and decoration for the simple form: the briefed codes when
/// the price list has them, else the first option in the group.
fn default_code(rules: &[Rule], grp: &str, wanted: &str) -> Option<String> {
    rules
        .iter()
        .find(|r| r.code == wanted && r.grp == grp)
        .or_else(|| rules.iter().find(|r| r.grp == grp))
        .map(|r| r.code.clone())
}

fn label_of(rules: &[Rule], code: Option<&str>) -> String {
    code.and_then(|c| rules.iter().find(|r| r.code == c))
        .map(|r| r.label.clone())
        .unwrap_or_default()
}

async fn form_view(
    app: &App,
    ctx: &Ctx,
    form: CakeForm,
    error: String,
    held: Option<Held>,
) -> anyhow::Result<FormView> {
    let records = app.records().await?;
    let site = &ctx.site;
    let b = book(records, site, time::now()).await?;
    let shell = app
        .shell(ctx, "cakes", "Order a cake", "Request a celebration cake with a simple form. Wanjiru confirms every cake and nothing is charged until she does.")
        .await?;
    let days = b
        .days
        .iter()
        .filter(|d| d.reason.is_empty())
        .map(|d| {
            let only = match (
                d.why[""]["delivery"].is_empty(),
                d.why[""]["collection"].is_empty(),
            ) {
                (false, true) => " \u{b7} collection only",
                (true, false) => " \u{b7} delivery only",
                _ => "",
            };
            FormDay {
                date: d.date.clone(),
                label: format!("{}{only}", d.label),
            }
        })
        .collect();
    let defaults = [
        ("colour", "colour.cream"),
        ("finish", "finish.smooth"),
        ("deco", "deco.none"),
    ]
    .iter()
    .map(|(g, c)| label_of(&b.rules, default_code(&b.rules, g, c).as_deref()).to_lowercase())
    .collect::<Vec<_>>();
    Ok(FormView {
        shell,
        notice: notice_words(&b.groups.size),
        sizes: b.groups.size,
        sponges: b.groups.sponge,
        days,
        max_chars: b.max_chars,
        version: b.list.version,
        valid_until: time::fmt_day(b.list.valid_until, site.utc_offset_min),
        zone: zone_view(b.zone.as_ref(), &site.currency),
        address: site.fact("/address"),
        owner: site.fact("/owner/first"),
        confirmations: site.fact("/confirmations"),
        defaults: or_list(&defaults, "and"),
        form,
        error,
        held,
    })
}

async fn order_form(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let mut form = CakeForm {
        key: agentrix_records::new_id("oc"),
        fulfilment: "collection".into(),
        ..CakeForm::default()
    };
    if let Some(v) = &ctx.visitor {
        form.name = v.name.clone();
        form.phone = v.phone.clone().unwrap_or_default();
    }
    let view = form_view(&app, &ctx, form, String::new(), None).await?;
    Ok(html(app.render("kilele/order_cake.html", &view)?))
}

async fn order_post(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Form(form): Form<CakeForm>,
) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let list = records
        .live_price_list(&site.site)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no live price list"))?;
    let rules = records.rules(&site.site, &list.version).await?;
    let choice = tidy(CakeChoice {
        size: Some(form.size.clone()),
        sponge: Some(form.sponge.clone()),
        colour: default_code(&rules, "colour", "colour.cream"),
        finish: default_code(&rules, "finish", "finish.smooth"),
        deco: default_code(&rules, "deco", "deco.none"),
        message: form.message.clone(),
        date: Some(form.date.clone()),
        note: form.note.chars().take(240).collect(),
        idea: None,
    });
    let error = if form.name.trim().is_empty() {
        Some("Add your name for the order.".to_owned())
    } else if form.phone.chars().filter(char::is_ascii_digit).count() < 9 {
        Some(format!(
            "Add a WhatsApp number so {} can confirm with you.",
            site.fact("/owner/first")
        ))
    } else {
        None
    };
    let result = match error {
        Some(e) => Err(e),
        None => {
            let recipient = form.recipient.trim();
            let contact = Contact {
                name: form.name.trim().chars().take(80).collect(),
                email: ctx
                    .visitor
                    .as_ref()
                    .map(|v| v.email.clone())
                    .filter(|e| !e.is_empty()),
                phone: Some(form.phone.trim().chars().take(30).collect()),
                recipient: (!recipient.is_empty()).then(|| recipient.chars().take(160).collect()),
            };
            let customer = ctx.visitor.as_ref().map(|v| v.account.clone());
            let who = Who {
                contact,
                customer_id: customer,
                key: Some(form.key.clone()),
            };
            place(
                records,
                site,
                &choice,
                Some(form.fulfilment.as_str()),
                who,
                now,
            )
            .await?
        }
    };
    let view = match result {
        Ok((id, priced)) => {
            app.tell_held_by_id(&ctx, &id).await;
            let day = chrono::NaiveDate::parse_from_str(&form.date, "%Y-%m-%d")
                .map(|d| d.format("%a %-d %b").to_string())
                .unwrap_or_default();
            let cake = [
                choice.size.as_deref(),
                choice.sponge.as_deref(),
                choice.colour.as_deref(),
                choice.finish.as_deref(),
                choice.deco.as_deref(),
            ]
            .iter()
            .map(|c| label_of(&rules, *c).to_lowercase())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
            let when = match (form.fulfilment.as_str(), &priced.slot) {
                ("delivery", Some(s)) => format!("Delivery {s} \u{b7} {}", form.recipient.trim()),
                (_, Some(s)) => format!("Collection {s} \u{b7} {}", site.fact("/address")),
                _ => day.clone(),
            };
            let held = Held {
                url: format!("/orders/{id}"),
                order_id: id,
                total: priced.total,
                usd: priced.usd,
                rate: priced.rate,
                cake: capitalise(&cake),
                message: choice.message.clone(),
                note: choice.note.clone(),
                when,
                last_step: if form.fulfilment == "delivery" {
                    format!("Delivered on {day}")
                } else {
                    format!("Collect on {day}")
                },
                signed_in: ctx.visitor.is_some(),
                phone: form.phone.trim().to_owned(),
            };
            form_view(&app, &ctx, form, String::new(), Some(held)).await?
        }
        Err(words) => form_view(&app, &ctx, form, words, None).await?,
    };
    Ok(html(app.render("kilele/order_cake.html", &view)?))
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(label: &str, notice: i64) -> Opt {
        Opt {
            code: String::new(),
            label: label.into(),
            short: String::new(),
            price: String::new(),
            swatch: String::new(),
            serves: String::new(),
            notice_days: notice,
            tiers: 0,
            only_sizes: vec![],
            only: String::new(),
        }
    }

    #[test]
    fn notice_reads_from_the_size_rules() {
        assert_eq!(
            notice_words(&[opt("1 tier", 2), opt("2 tiers", 2), opt("3 tiers", 3)]),
            "2 days' notice, 3 tiers need 3"
        );
        assert_eq!(
            notice_words(&[opt("1 tier", 2), opt("2 tiers", 2)]),
            "2 days' notice"
        );
    }

    #[test]
    fn chips_drop_the_group_word() {
        assert_eq!(strip_group_word("Forest green buttercream"), "Forest green");
        assert_eq!(strip_group_word("Lemon sponge"), "Lemon");
        assert_eq!(strip_group_word("Chocolate drip"), "Chocolate drip");
    }

    async fn kilele() -> (Records, Site, i64) {
        let records = Records::open_in_memory().await.expect("records");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/samples/kilele-bakery");
        let now = time::now();
        records
            .load_bundle(&dir, Some("kilele-bakery-sample.localhost"), true, now)
            .await
            .expect("bundle");
        let site = records
            .site("kilele-bakery-sample")
            .await
            .expect("site")
            .expect("kilele");
        (records, site, now)
    }

    fn cake(date: &str) -> CakeChoice {
        CakeChoice {
            size: Some("size.2".into()),
            sponge: Some("sponge.lemon".into()),
            colour: Some("colour.forest".into()),
            finish: Some("finish.ombre".into()),
            deco: Some("deco.berries".into()),
            message: "Happy 7th, Amina!".into(),
            date: Some(date.into()),
            note: "No nuts".into(),
            idea: None,
        }
    }

    fn who(customer: Option<&str>, recipient: Option<&str>, key: &str) -> Who {
        Who {
            contact: Contact {
                name: "Njeri".into(),
                email: Some("njeri@example.com".into()),
                phone: None,
                recipient: recipient.map(str::to_owned),
            },
            customer_id: customer.map(str::to_owned),
            key: Some(key.into()),
        }
    }

    #[tokio::test]
    async fn the_studio_and_the_form_make_one_held_request() {
        let (records, site, now) = kilele().await;
        let b = book(&records, &site, now).await.expect("book");
        assert_eq!(b.groups.size.len(), 3);
        assert_eq!(b.days.len(), CALENDAR_DAYS as usize);
        assert!(
            !b.days[0].reason.is_empty(),
            "today is too soon for any cake"
        );
        let sat = b
            .days
            .iter()
            .find(|d| d.dow == "Sat" && d.away >= 3)
            .expect("a Saturday")
            .date
            .clone();
        let weekday = b
            .days
            .iter()
            .find(|d| d.dow == "Wed" && d.away >= 3)
            .expect("a Wednesday")
            .date
            .clone();

        // Delivery on a day without delivery is refused in words, with nothing made.
        let no = place(
            &records,
            &site,
            &cake(&weekday),
            Some("delivery"),
            who(None, Some("Amina, Kilimani"), "k-0"),
            now,
        )
        .await
        .expect("place");
        assert!(no.expect_err("refused").contains("doesn't deliver"));

        // A signed-in delivery is held, priced with the zone fee, and a retry returns it.
        let (id, priced) = place(
            &records,
            &site,
            &cake(&sat),
            Some("delivery"),
            who(Some("cus-1"), Some("Amina, Kilimani"), "k-1"),
            now,
        )
        .await
        .expect("place")
        .expect("held");
        assert!(priced.lines.iter().any(|l| l.code.starts_with("delivery.")));
        let again = place(
            &records,
            &site,
            &cake(&sat),
            Some("delivery"),
            who(Some("cus-1"), Some("Amina, Kilimani"), "k-1"),
            now,
        )
        .await
        .expect("place")
        .expect("held");
        assert_eq!(again.0, id);
        let view = records
            .order(&site, &id)
            .await
            .expect("order")
            .expect("exists");
        assert_eq!(view.order.state, "held");
        assert_eq!(view.order.kind, "custom");
        assert_eq!(view.order.customer_id.as_deref(), Some("cus-1"));
        assert_eq!(view.order.note.as_deref(), Some("No nuts"));
        assert_eq!(view.order.total_minor, priced.total_minor);

        // A guest collection works too, and gluten-free with 3 tiers is refused.
        let guest = place(
            &records,
            &site,
            &cake(&weekday),
            Some("collection"),
            who(None, None, "k-2"),
            now,
        )
        .await
        .expect("place");
        assert!(guest.is_ok());
        let mut bad = cake(&sat);
        bad.size = Some("size.3".into());
        bad.sponge = Some("sponge.gf".into());
        let no = place(
            &records,
            &site,
            &bad,
            Some("collection"),
            who(None, None, "k-3"),
            now,
        )
        .await
        .expect("place");
        assert!(no.expect_err("refused").contains("Gluten-free"));
    }

    #[test]
    fn a_day_says_why_it_is_closed() {
        let day = Day {
            date: "2026-11-14".into(),
            label: "Sat 14 Nov".into(),
            dow: "Sat".into(),
            num: "14".into(),
            away: 2,
            delivery: vec![DaySlot {
                id: "d".into(),
                time: "10:00\u{2013}12:00".into(),
                full: true,
            }],
            collection: vec![DaySlot {
                id: "c".into(),
                time: "08:00\u{2013}10:00".into(),
                full: false,
            }],
            why: BTreeMap::new(),
            reason: String::new(),
        };
        assert_eq!(why_not(&day, 3, ""), "Needs 3 days' notice");
        assert_eq!(why_not(&day, 2, "delivery"), "Delivery is full");
        assert_eq!(why_not(&day, 2, "collection"), "");
        assert_eq!(why_not(&day, 2, ""), "");
    }
}
