//! Kilele Bakery's pages (01a §2.3, K-01 to K-19) and its desks' live facts and instant answers.

use std::collections::HashMap;

use agentrix_records::catalogue::Stock;
use agentrix_records::{Records, Site, money, time};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse as _};
use axum::routing::get;
use axum::{Extension, Router};
use serde::Serialize;
use serde_json::{Value, json};

use crate::App;
use crate::ask::{Action, Answer, Section, Source, number_in, order_number, plain, said};
use crate::shell::{DeskFacts, Fact, Shell, asof};
use crate::web::{Ctx, Page, html};

/// Kilele's page routes (the studio and the simple cake form are in [`crate::studio`]).
pub fn routes() -> Router<App> {
    Router::new()
        .route("/", get(home))
        .route("/bread", get(bread))
        .route("/cakes", get(cakes))
        .route("/visit", get(visit))
        .route("/delivery", get(delivery))
        .route("/allergens", get(allergens))
        .route("/help", get(help))
        .route("/shop/{slug}", get(product))
        .merge(crate::studio::routes())
        .fallback(notfound)
}

/// A product card.
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    /// Sku.
    pub sku: String,
    /// Slug, also the anchor `item-<slug>`.
    pub slug: String,
    /// Name.
    pub name: String,
    /// Description.
    pub description: String,
    /// "KSh 450".
    pub price: String,
    /// "6 left today", "Next batch 10:00", "Sold out · back at 6:30".
    pub stock: String,
    /// The stock badge is the sold-out style.
    pub out: bool,
    /// Units left, if kept.
    pub left: Option<i64>,
    /// "Flour, water, salt."
    pub ingredients: String,
    /// What it contains, for the allergen table.
    pub contains: Vec<String>,
    /// Placeholder tone for the image.
    pub tone: String,
    /// Section.
    pub section: String,
}

/// "6 left today", "2 left · next batch 10:00", "Sold out · back at 6:30".
pub fn stock_words(stock: Option<&Stock>, off: i32) -> (String, bool) {
    let Some(s) = stock else {
        return ("Ask about today's stock".into(), false);
    };
    let clock = |t: i64| {
        let c = time::fmt_clock(t, off);
        c.strip_prefix('0').map(str::to_owned).unwrap_or(c)
    };
    match (s.state, s.left) {
        ("out", _) => match s.back_at {
            Some(t) => (format!("Sold out \u{b7} back at {}", clock(t)), true),
            None => ("Sold out today".into(), true),
        },
        ("batch", Some(0)) | ("batch", None) => (
            format!(
                "Next batch {}",
                s.next_batch_at.map(clock).unwrap_or_default()
            ),
            false,
        ),
        ("batch", Some(n)) => (
            format!(
                "{n} left \u{b7} next batch {}",
                s.next_batch_at.map(clock).unwrap_or_default()
            ),
            false,
        ),
        (_, Some(n)) => (format!("{n} left today"), false),
        (_, None) => ("In stock".into(), false),
    }
}

/// Cards for products, with live stock.
pub async fn cards(
    records: &Records,
    site: &Site,
    section: Option<&str>,
    now: i64,
) -> anyhow::Result<Vec<Card>> {
    let stock = records.stock(site, now).await?;
    Ok(records
        .products(&site.site, section)
        .await?
        .into_iter()
        .map(|p| {
            let attrs = p.attrs();
            let (words, out) = stock_words(stock.get(&p.sku), site.utc_offset_min);
            Card {
                left: stock.get(&p.sku).and_then(|s| s.left),
                price: money::fmt(&site.currency, p.price_minor),
                stock: words,
                out,
                ingredients: attrs
                    .get("ingredients")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                contains: attrs
                    .get("contains")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                tone: attrs
                    .get("tone")
                    .and_then(Value::as_str)
                    .unwrap_or("crust")
                    .to_owned(),
                sku: p.sku,
                slug: p.slug,
                name: p.name,
                description: p.description,
                section: p.section,
            }
        })
        .collect())
}

/// A row of the cake price table.
#[derive(Debug, Clone, Serialize)]
pub struct CakeRow {
    /// Rule code.
    pub code: String,
    /// Anchor: `row-1tier`.
    pub anchor: String,
    /// "1 tier".
    pub label: String,
    /// "12–15".
    pub serves: String,
    /// "2 days".
    pub notice: String,
    /// "KSh 3,800".
    pub from: String,
}

/// An option with its price delta.
#[derive(Debug, Clone, Serialize)]
pub struct Option_ {
    /// Rule code.
    pub code: String,
    /// Label.
    pub label: String,
    /// "+KSh 300", or "" when free.
    pub delta: String,
    /// Colour swatch.
    pub swatch: String,
    /// Constraint words.
    pub note: String,
}

/// The cake price list as pages show it.
#[derive(Debug, Clone, Serialize)]
pub struct CakeList {
    /// Size rows.
    pub rows: Vec<CakeRow>,
    /// Options by group.
    pub sponges: Vec<Option_>,
    /// Buttercream colours.
    pub colours: Vec<Option_>,
    /// Finishes.
    pub finishes: Vec<Option_>,
    /// Decorations.
    pub decos: Vec<Option_>,
    /// "v14" (owners and agents only; visitors see the date).
    pub version: String,
    /// "Sun 1 Nov": the day the price list took effect.
    pub from_day: String,
    /// "the price list of Sun 1 Nov".
    pub named: String,
    /// "Sat 14 Nov".
    pub valid_until: String,
    /// "1 USD = KSh 129.00".
    pub rate: String,
    /// The gluten-free price, for the note.
    pub gf: String,
    /// Characters allowed in the message.
    pub max_chars: i64,
}

/// Kilele's cake price list from the live rules.
pub async fn cake_list(records: &Records, site: &Site) -> anyhow::Result<CakeList> {
    let list = records
        .live_price_list(&site.site)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no live price list"))?;
    let rules = records.rules(&site.site, &list.version).await?;
    let opt = |grp: &str| -> Vec<Option_> {
        rules
            .iter()
            .filter(|r| r.grp == grp)
            .map(|r| {
                let meta = r.meta();
                Option_ {
                    code: r.code.clone(),
                    label: r.label.clone(),
                    delta: if r.amount_minor > 0 {
                        format!("+{}", money::fmt(&site.currency, r.amount_minor))
                    } else {
                        String::new()
                    },
                    swatch: meta
                        .get("swatch")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    note: if meta.get("only_sizes").is_some() {
                        "1 and 2 tiers only".into()
                    } else {
                        String::new()
                    },
                }
            })
            .collect()
    };
    let rows = rules
        .iter()
        .filter(|r| r.grp == "size")
        .map(|r| {
            let meta = r.meta();
            let tiers = r.code.trim_start_matches("size.");
            let days = meta.get("notice_days").and_then(Value::as_i64).unwrap_or(2);
            CakeRow {
                code: r.code.clone(),
                anchor: format!("row-{tiers}tier"),
                label: r.label.clone(),
                serves: meta
                    .get("serves")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                notice: format!("{days} days"),
                from: money::fmt(&site.currency, r.amount_minor),
            }
        })
        .collect();
    Ok(CakeList {
        rows,
        sponges: opt("sponge"),
        colours: opt("colour"),
        finishes: opt("finish"),
        decos: opt("deco"),
        gf: rules
            .iter()
            .find(|r| r.code == "sponge.gf")
            .map(|r| money::fmt(&site.currency, r.amount_minor))
            .unwrap_or_default(),
        max_chars: rules
            .iter()
            .find(|r| r.code == "message")
            .and_then(|r| r.meta().get("max_chars").and_then(Value::as_i64))
            .unwrap_or(22),
        from_day: time::fmt_day(list.valid_from, site.utc_offset_min),
        named: crate::common::list_name(site, &list),
        version: list.version,
        valid_until: time::fmt_day(list.valid_until, site.utc_offset_min),
        rate: money::rate_line(list.usd_rate_minor),
    })
}

/// Opening hours and whether it's open now.
#[derive(Debug, Clone, Serialize)]
pub struct Hours {
    /// ("Mon–Sat", "6:30–19:00").
    pub rows: Vec<(String, String)>,
    /// "Open now until 19:00", "Closed now. Opens at 6:30" or "Closed now. Opens tomorrow at 7:00".
    pub now: String,
    /// Open right now.
    pub open: bool,
    /// "until 19:00" for the peek.
    pub until: String,
}

fn short_clock(c: &str) -> String {
    c.strip_prefix('0')
        .map(str::to_owned)
        .unwrap_or_else(|| c.to_owned())
}

/// Hours from the site's facts.
pub fn hours(site: &Site, now: i64) -> Hours {
    let local = time::local(now, site.utc_offset_min);
    let wd = chrono::Datelike::weekday(&local).num_days_from_sunday() as i64;
    let clock = local.format("%H:%M").to_string();
    let mut rows = Vec::new();
    let mut today: Option<(String, String)> = None;
    // Opening clock by weekday (0 is Sunday), for "Opens tomorrow at 7:00".
    let mut opens: HashMap<i64, String> = HashMap::new();
    for h in site
        .facts
        .get("hours")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let open = h
            .get("open")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let close = h
            .get("close")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        rows.push((
            h.get("days")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            format!("{}\u{2013}{}", short_clock(&open), short_clock(&close)),
        ));
        for d in h
            .get("weekdays")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
        {
            opens.insert(d, open.clone());
            if d == wd {
                today = Some((open.clone(), close.clone()));
            }
        }
    }
    let next_opening = (1..=7).find_map(|ahead| {
        let open = opens.get(&((wd + ahead) % 7))?;
        let when = if ahead == 1 {
            "tomorrow".to_owned()
        } else {
            (local + chrono::Days::new(ahead as u64))
                .format("on %A")
                .to_string()
        };
        Some((when, short_clock(open)))
    });
    let (open, now_words, until) = match &today {
        Some((o, c)) if clock.as_str() >= o.as_str() && clock.as_str() < c.as_str() => {
            (true, format!("Open now until {c}"), format!("until {c}"))
        }
        Some((o, _)) if clock.as_str() < o.as_str() => (
            false,
            format!("Closed now. Opens at {}", short_clock(o)),
            format!("opens {}", short_clock(o)),
        ),
        _ => match next_opening {
            Some((when, at)) => (
                false,
                format!("Closed now. Opens {when} at {at}"),
                format!("opens {when} at {at}"),
            ),
            None => (false, "Closed now".into(), "closed now".into()),
        },
    };
    Hours {
        rows,
        now: now_words,
        open,
        until,
    }
}

/// Live facts for Kilele's desks.
pub async fn desk_facts(
    records: &Records,
    site: &Site,
    now: i64,
) -> anyhow::Result<HashMap<String, DeskFacts>> {
    let stock = records.stock(site, now).await?;
    let off = site.utc_offset_min;
    let mut out = HashMap::new();
    let mut bread = Vec::new();
    for sku in ["sourdough", "croissant"] {
        if let Some(p) = records.product(&site.site, sku).await? {
            let s = stock.get(sku);
            let v = match s {
                Some(s) if s.state == "batch" => format!(
                    "next batch {}",
                    s.next_batch_at
                        .map(|t| time::fmt_clock(t, off))
                        .unwrap_or_default()
                ),
                Some(s) if s.state == "out" => "sold out".into(),
                Some(s) => format!("{} left", s.left.unwrap_or(0)),
                None => continue,
            };
            let short = p
                .name
                .split_whitespace()
                .last()
                .unwrap_or(&p.name)
                .to_owned();
            bread.push(Fact {
                k: capitalise(&short),
                v,
            });
        }
    }
    out.insert(
        "bread".into(),
        DeskFacts {
            facts: bread,
            asof: asof(site, now),
        },
    );
    let list = cake_list(records, site).await?;
    let notice = list
        .rows
        .first()
        .map(|r| r.notice.clone())
        .unwrap_or_default();
    let next_free = next_free_day(records, site, now).await?;
    out.insert(
        "cakes".into(),
        DeskFacts {
            facts: vec![
                Fact {
                    k: "Next free date".into(),
                    v: next_free,
                },
                Fact {
                    k: "Notice".into(),
                    v: notice,
                },
                Fact {
                    k: "From".into(),
                    v: list
                        .rows
                        .first()
                        .map(|r| r.from.clone())
                        .unwrap_or_default(),
                },
            ],
            asof: asof(site, now),
        },
    );
    out.insert(
        "orders".into(),
        DeskFacts {
            facts: vec![
                Fact {
                    k: "Confirmations".into(),
                    v: crate::common::owner_reply(site),
                },
                Fact {
                    k: "Free changes".into(),
                    v: format!(
                        "until {} h before",
                        site.facts
                            .get("changes_cutoff_hours")
                            .and_then(Value::as_i64)
                            .unwrap_or(24)
                    ),
                },
            ],
            asof: String::new(),
        },
    );
    let h = hours(site, now);
    let zones = records.zones(&site.site).await?;
    out.insert(
        "visit".into(),
        DeskFacts {
            facts: vec![
                Fact {
                    k: if h.open {
                        "Open now".into()
                    } else {
                        "Closed".into()
                    },
                    v: h.until,
                },
                Fact {
                    k: "Delivery".into(),
                    v: zones
                        .iter()
                        .flat_map(|z| z.areas.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                },
            ],
            asof: asof(site, now),
        },
    );
    Ok(out)
}

/// The host messages link to: the sample host the visitor is on, or the business's domain.
pub fn link_host(ctx: &Ctx) -> String {
    if ctx.site.is_sample() {
        ctx.host.host.clone()
    } else {
        ctx.site.fact("/domain")
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// The first day a 1- or 2-tier cake can be baked that still has a delivery or collection slot.
pub async fn next_free_day(records: &Records, site: &Site, now: i64) -> anyhow::Result<String> {
    let earliest = time::day_start(now, site.utc_offset_min) + 2 * 86_400;
    for kind in ["delivery", "collection"] {
        let slots = records
            .slots(site, kind, earliest, earliest + 21 * 86_400)
            .await?;
        if let Some(s) = slots.iter().find(|s| !s.full) {
            return Ok(time::fmt_day(s.starts_at, site.utc_offset_min));
        }
    }
    Ok(time::fmt_day(earliest, site.utc_offset_min))
}

#[derive(Serialize)]
struct HomeView {
    shell: Shell,
    headline: Vec<String>,
    intro: String,
    bakes: Vec<Card>,
    cakes: CakeList,
    hours: Hours,
    confirmations: String,
    changes: String,
    collection: String,
    allergens: String,
    address: String,
    parking: String,
    zones: Vec<agentrix_records::orders::Zone>,
    delivery_days: String,
    asof: String,
    oven: String,
}

async fn home_view(
    app: &App,
    ctx: &Ctx,
    current: &str,
    title: &str,
    description: &str,
) -> anyhow::Result<HomeView> {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let all = cards(records, site, Some("bread"), now).await?;
    let featured = ["sourdough", "croissant", "cinnamon-roll", "red-velvet"];
    let bakes = featured
        .iter()
        .filter_map(|s| all.iter().find(|c| c.sku == *s).cloned())
        .collect();
    let stock = records.stock(site, now).await?;
    let first_batch = stock.values().filter_map(|s| s.next_batch_at).min();
    // Today's first batch from the bake plan: each day's stock is taken at its first batch.
    let baked_at = stock.values().map(|s| s.as_of).filter(|t| *t <= now).min();
    Ok(HomeView {
        shell: app.shell(ctx, current, title, description).await?,
        headline: site
            .fact("/headline")
            .split('\n')
            .map(str::to_owned)
            .collect(),
        intro: site.fact("/intro"),
        bakes,
        cakes: cake_list(records, site).await?,
        hours: hours(site, now),
        confirmations: site.fact("/confirmations"),
        changes: site.fact("/changes"),
        collection: site.fact("/collection"),
        allergens: site.fact("/allergens"),
        address: site.fact("/address"),
        parking: site.fact("/parking"),
        zones: records.zones(&site.site).await?,
        delivery_days: delivery_days(records, site, now).await?,
        asof: asof(site, now),
        oven: match (first_batch, baked_at) {
            (Some(t), _) => format!("Next batch {}", time::fmt_clock(t, site.utc_offset_min)),
            (None, Some(t)) => format!(
                "Out of the oven at {}",
                short_clock(&time::fmt_clock(t, site.utc_offset_min))
            ),
            (None, None) => String::new(),
        },
    })
}

/// "Saturdays 10:00–14:00, Sundays 10:00–12:00", from the next fortnight's delivery slots.
pub async fn delivery_days(records: &Records, site: &Site, now: i64) -> anyhow::Result<String> {
    let slots = records
        .slots(site, "delivery", now, now + 14 * 86_400)
        .await?;
    let mut by_day: Vec<(String, i64, i64)> = Vec::new();
    for s in &slots {
        let day = time::local(s.starts_at, site.utc_offset_min)
            .format("%A")
            .to_string();
        let (start, end) = (s.starts_at % 86_400, s.ends_at % 86_400);
        match by_day.iter_mut().find(|(d, _, _)| *d == day) {
            Some(entry) => {
                entry.1 = entry.1.min(start);
                entry.2 = entry.2.max(end);
            }
            None => by_day.push((day, start, end)),
        }
    }
    let off = i64::from(site.utc_offset_min) * 60;
    let words: Vec<String> = by_day
        .into_iter()
        .map(|(d, a, b)| {
            let f = |t: i64| {
                let t = (t + off).rem_euclid(86_400);
                format!("{:02}:{:02}", t / 3600, (t % 3600) / 60)
            };
            format!("{d}s {}\u{2013}{}", f(a), f(b))
        })
        .collect();
    Ok(words.join(", "))
}

async fn home(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "",
        &ctx.site.name.clone(),
        "Sourdough, croissants and celebration cakes in Westlands, Nairobi. A sample site.",
    )
    .await?;
    Ok(html(app.render("kilele/home.html", &view)?))
}

/// A page's view: the home view's fields with the page's own beside them (the layout reads
/// `shell` at the top level).
fn with(view: &impl Serialize, extra: Value) -> anyhow::Result<Value> {
    let mut v = serde_json::to_value(view)?;
    if let (Some(m), Value::Object(e)) = (v.as_object_mut(), extra) {
        m.extend(e);
    }
    Ok(v)
}

/// A product baked on set days, with when it is next baked.
#[derive(Debug, Clone, Serialize)]
pub struct Baked {
    /// The card, with live stock (or the next bake day when it isn't baked today).
    #[serde(flatten)]
    pub card: Card,
    /// "Saturdays", or empty when the product doesn't name its days.
    pub days: String,
    /// Baked today.
    pub today: bool,
    /// "Sat 14 Nov": the next bake day, today included; empty without days.
    pub next: String,
}

/// Cards for a section with their bake days from the catalogue (`attrs.days`, e.g. `["sat"]`).
pub async fn baked(
    records: &Records,
    site: &Site,
    section: Option<&str>,
    now: i64,
) -> anyhow::Result<Vec<Baked>> {
    let products = records.products(&site.site, section).await?;
    let today = time::local(now, site.utc_offset_min).date_naive();
    Ok(cards(records, site, section, now)
        .await?
        .into_iter()
        .map(|mut card| {
            let wanted: Vec<chrono::Weekday> = products
                .iter()
                .find(|p| p.sku == card.sku)
                .and_then(|p| p.attrs().get("days").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|d| d.parse().ok())
                .collect();
            if wanted.is_empty() {
                return Baked {
                    card,
                    days: String::new(),
                    today: true,
                    next: String::new(),
                };
            }
            let next_day = (0..7)
                .filter_map(|i| today.checked_add_days(chrono::Days::new(i)))
                .find(|d| wanted.contains(&chrono::Datelike::weekday(d)));
            let is_today = next_day == Some(today);
            let next = next_day
                .map(|d| d.format("%a %-d %b").to_string())
                .unwrap_or_default();
            let names: Vec<String> = (0..7)
                .filter_map(|i| today.checked_add_days(chrono::Days::new(i)))
                .filter(|d| wanted.contains(&chrono::Datelike::weekday(d)))
                .map(|d| format!("{}s", d.format("%A")))
                .collect();
            if !is_today {
                card.stock = format!("Next bake {next}");
                card.out = false;
            }
            Baked {
                card,
                days: names.join(" and "),
                today: is_today,
                next,
            }
        })
        .collect())
}

/// The standing hold rule as the bread page states it.
#[derive(Debug, Clone, Serialize)]
struct HoldNote {
    /// The owner's words for the grant.
    words: String,
    /// Loaves a desk may hold without asking.
    max: i64,
    /// "12:00".
    until: String,
    /// Holds can still be made today.
    open: bool,
    /// Loaves held for collection today.
    held: i64,
}

fn hold_note(site: &Site, now: i64, held: i64) -> HoldNote {
    let until = site.fact("/hold_grant/until");
    HoldNote {
        words: site.fact("/hold_grant/words"),
        max: site
            .facts
            .pointer("/hold_grant/max")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        open: !until.is_empty() && time::fmt_clock(now, site.utc_offset_min) < until,
        until,
        held,
    }
}

async fn bread(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "bread",
        "Today's bread",
        "What came out of the oven today, what's left and when the next batch is due.",
    )
    .await?;
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let bakes = cards(records, site, Some("bread"), now).await?;
    let boxes = baked(records, site, Some("box"), now).await?;
    let held = records
        .holds(site, now)
        .await?
        .iter()
        .filter(|h| h.state == "held")
        .map(|h| h.qty)
        .sum();
    let extra = json!({ "bakes": bakes, "boxes": boxes, "hold": hold_note(site, now, held) });
    Ok(html(app.render("kilele/bread.html", &with(&view, extra)?)?))
}

async fn cakes(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "cakes",
        "Celebration cakes",
        "Celebration cakes designed with you and confirmed by Wanjiru, priced from the price list.",
    )
    .await?;
    let records = app.records().await?;
    let site = &ctx.site;
    let base = view
        .cakes
        .rows
        .first()
        .map(|r| r.notice.clone())
        .unwrap_or_default();
    let longer: Vec<String> = view
        .cakes
        .rows
        .iter()
        .filter(|r| r.notice != base)
        .map(|r| format!("{} need {}", r.label, r.notice))
        .collect();
    let notice = if longer.is_empty() {
        format!("Every cake needs {base} of notice.")
    } else {
        format!("Cakes need {base} of notice; {}.", longer.join(", "))
    };
    let extra = json!({
        "not_offered": site.facts.get("not_offered").cloned().unwrap_or_else(|| json!([])),
        "next_free": next_free_day(records, site, time::now()).await?,
        "notice": notice,
        "list_source": records.live_price_list(&site.site).await?.map(|l| l.source).unwrap_or_default(),
    });
    Ok(html(app.render("kilele/cakes.html", &with(&view, extra)?)?))
}

/// The `days` label of today's row in the opening hours ("Mon–Sat").
fn today_hours(site: &Site, now: i64) -> String {
    let wd = i64::from(
        chrono::Datelike::weekday(&time::local(now, site.utc_offset_min)).num_days_from_sunday(),
    );
    site.facts
        .get("hours")
        .and_then(Value::as_array)
        .and_then(|hs| {
            hs.iter().find(|h| {
                h.get("weekdays")
                    .and_then(Value::as_array)
                    .is_some_and(|a| a.iter().any(|d| d.as_i64() == Some(wd)))
            })
        })
        .and_then(|h| h.get("days").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned()
}

async fn visit(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "visit",
        "Visit",
        "Hours, directions, parking and where Kilele delivers.",
    )
    .await?;
    let extra = json!({ "today": today_hours(&ctx.site, time::now()) });
    Ok(html(app.render("kilele/visit.html", &with(&view, extra)?)?))
}

/// A delivery slot as the delivery page shows it.
#[derive(Debug, Clone, Serialize)]
struct SlotRow {
    /// "10:00–12:00".
    time: String,
    /// Orders in it.
    taken: i64,
    /// Orders it takes.
    capacity: i64,
    /// Full.
    full: bool,
    /// How full, 0 to 100.
    pct: i64,
    /// "4 of 6 left", "Full".
    words: String,
}

/// A day of delivery slots.
#[derive(Debug, Clone, Serialize)]
struct SlotDay {
    /// "Sat 14 Nov".
    day: String,
    /// Its slots.
    slots: Vec<SlotRow>,
}

async fn delivery(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "visit",
        "Delivery",
        "Where Kilele delivers, the fee and the delivery times.",
    )
    .await?;
    let records = app.records().await?;
    let site = &ctx.site;
    let off = site.utc_offset_min;
    let now = time::now();
    let mut days: Vec<SlotDay> = Vec::new();
    for s in records
        .slots(site, "delivery", now, now + 14 * 86_400)
        .await?
    {
        let day = time::fmt_day(s.starts_at, off);
        let left = (s.capacity - s.taken).max(0);
        let row = SlotRow {
            time: format!(
                "{}\u{2013}{}",
                time::fmt_clock(s.starts_at, off),
                time::fmt_clock(s.ends_at, off)
            ),
            pct: if s.capacity > 0 {
                (s.taken * 100 / s.capacity).min(100)
            } else {
                100
            },
            words: if s.full {
                "Full".into()
            } else if s.taken == 0 {
                format!("All {} open", s.capacity)
            } else {
                format!("{left} of {} left", s.capacity)
            },
            taken: s.taken,
            capacity: s.capacity,
            full: s.full,
        };
        match days.last_mut() {
            Some(d) if d.day == day => d.slots.push(row),
            _ => days.push(SlotDay {
                day,
                slots: vec![row],
            }),
        }
    }
    let extra = json!({ "days": days, "cutoff": site.facts.get("changes_cutoff_hours").and_then(Value::as_i64).unwrap_or(24) });
    Ok(html(
        app.render("kilele/delivery.html", &with(&view, extra)?)?,
    ))
}

/// A row of the allergen table.
#[derive(Debug, Clone, Serialize)]
struct AllergenRow {
    /// Product name.
    name: String,
    /// Where it is on the site.
    href: String,
    /// "Contains butter, milk."
    ingredients: String,
    /// One per column: it contains that.
    cells: Vec<bool>,
}

async fn allergens(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "bread",
        "Allergens",
        "The kitchen allergen note and what each bake contains.",
    )
    .await?;
    let all = cards(app.records().await?, &ctx.site, None, time::now()).await?;
    let order = ["wheat", "rye", "milk", "egg", "nuts", "coconut"];
    let mut keys: Vec<String> = order
        .iter()
        .filter(|k| all.iter().any(|c| c.contains.iter().any(|x| x == *k)))
        .map(|k| (*k).to_owned())
        .collect();
    for c in &all {
        for x in &c.contains {
            if !keys.contains(x) {
                keys.push(x.clone());
            }
        }
    }
    let rows: Vec<AllergenRow> = all
        .iter()
        .map(|c| AllergenRow {
            name: c.name.clone(),
            href: if c.section == "bread" {
                format!("/bread#item-{}", c.slug)
            } else {
                format!("/shop/{}", c.slug)
            },
            ingredients: c.ingredients.clone(),
            cells: keys.iter().map(|k| c.contains.contains(k)).collect(),
        })
        .collect();
    let columns: Vec<String> = keys.iter().map(|k| capitalise(k)).collect();
    let extra = json!({ "columns": columns, "rows": rows, "note": ctx.site.fact("/allergens"), "gf": view.cakes.gf });
    Ok(html(
        app.render("kilele/allergens.html", &with(&view, extra)?)?,
    ))
}

async fn help(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    let view = home_view(
        &app,
        &ctx,
        "orders",
        "Help",
        "What the desks can and can't do, how Kilele contacts you and how to change your details.",
    )
    .await?;
    let site = &ctx.site;
    let per_order = site
        .facts
        .pointer("/refund_rule/per_order")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let extra = json!({
        "grant": { "max": site.facts.pointer("/hold_grant/max").and_then(Value::as_i64).unwrap_or(0), "until": site.fact("/hold_grant/until"), "words": site.fact("/hold_grant/words") },
        "refund": {
            "per_order": money::fmt(&site.currency, per_order * 100),
            "per_week": site.facts.pointer("/refund_rule/per_week").and_then(Value::as_i64).unwrap_or(0),
            "until": site.fact("/refund_rule/until"),
            "to": site.fact("/refund_rule/to"),
        },
        "domain": link_host(&ctx),
        "reply": crate::common::owner_reply(site),
        "classes": site.fact("/classes"),
        "cutoff": site.facts.get("changes_cutoff_hours").and_then(Value::as_i64).unwrap_or(24),
    });
    Ok(html(app.render("kilele/help.html", &with(&view, extra)?)?))
}

async fn product(
    State(app): State<App>,
    Extension(ctx): Extension<Ctx>,
    Path(slug): Path<String>,
) -> Page {
    let records = app.records().await?;
    let site = &ctx.site;
    let Some(p) = records.product_by_slug(&site.site, &slug).await? else {
        return notfound_page(&app, &ctx).await;
    };
    let now = time::now();
    let Some(item) = baked(records, site, Some(&p.section), now)
        .await?
        .into_iter()
        .find(|b| b.card.slug == slug)
    else {
        return notfound_page(&app, &ctx).await;
    };
    let view = home_view(&app, &ctx, "bread", &p.name, &p.description).await?;
    let ld = json!({
        "@context": "https://schema.org",
        "@type": "Product",
        "name": p.name,
        "description": p.description,
        "sku": p.sku,
        "brand": { "@type": "Brand", "name": format!("{} (sample)", site.name) },
        "offers": {
            "@type": "Offer",
            "priceCurrency": site.currency,
            "price": format!("{}.{:02}", p.price_minor / 100, p.price_minor % 100),
            "availability": if item.card.out { "https://schema.org/SoldOut" } else if item.today { "https://schema.org/InStock" } else { "https://schema.org/PreOrder" },
        },
    });
    let extra = json!({ "item": item, "ld": ld, "asof": asof(site, now) });
    Ok(html(
        app.render("kilele/product.html", &with(&view, extra)?)?,
    ))
}

/// Kilele's own not-found page, with the shell and the Ask bar.
async fn notfound_page(app: &App, ctx: &Ctx) -> Page {
    let shell = app
        .shell(ctx, "", "Page not found", "That page isn't on this site.")
        .await?;
    let body = app.render(
        "kilele/notfound.html",
        &json!({ "shell": shell, "path": ctx.path }),
    )?;
    Ok((StatusCode::NOT_FOUND, Html(body)).into_response())
}

async fn notfound(State(app): State<App>, Extension(ctx): Extension<Ctx>) -> Page {
    notfound_page(&app, &ctx).await
}

/// What a question to Kilele is about, as the rules read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Status,
    Order,
    Hold,
    Stock,
    Dairy,
    Allergens,
    Design,
    Serves,
    Cancel,
    Change,
    Hours,
    Park,
    Deliver,
}

/// The questions Kilele answers instantly, from records, without a model (01a K-01, 06 §4.7):
/// order status, holds, what's in stock, opening hours and cake prices for a number of people.
/// Matched on whole words, so "are you open today?" is about hours, not stock.
fn instant_kind(q: &str, prefix: &str) -> Option<Kind> {
    let p = plain(q);
    let bread = [
        "loaf",
        "loaves",
        "bread",
        "sourdough",
        "croissant",
        "croissants",
        "slice",
        "slices",
    ];
    if order_number(&p, prefix)
        || said(
            &p,
            &[
                "my order",
                "order status",
                "track my order",
                "wheres my order",
                "where is my order",
            ],
        )
    {
        return Some(Kind::Status);
    }
    if said(&p, &["hold", "reserve"]) && said(&p, &bread) {
        return Some(Kind::Hold);
    }
    // "I'd like two loaves", "can I order three croissants": an order for today's bakes.
    if said(
        &p,
        &[
            "id like",
            "i would like",
            "ill take",
            "i will take",
            "can i order",
            "can i buy",
            "can i get",
            "can i have",
            "i want to order",
            "i want to buy",
        ],
    ) && (said(&p, &bread) || said(&p, &["mandazi", "bun", "buns", "rye", "box"]))
        && !said(
            &p,
            &[
                "gluten", "allergy", "allergic", "allergen", "dairy", "vegan", "nut", "nuts",
                "egg", "cake", "cakes", "change", "cancel", "refund", "hold", "reserve",
            ],
        )
    {
        return Some(Kind::Order);
    }
    if said(
        &p,
        &[
            "still warm",
            "in stock",
            "whats left",
            "what is left",
            "anything left",
            "sold out",
            "next batch",
            "out of the oven",
            "fresh today",
        ],
    ) || (said(&p, &["left"]) && said(&p, &bread))
    {
        return Some(Kind::Stock);
    }
    if said(
        &p,
        &[
            "open", "opens", "opening", "hours", "close", "closes", "closing",
        ],
    ) && !said(&p, &["deliver", "delivery", "delivers"])
    {
        return Some(Kind::Hours);
    }
    if number_in(&p).is_some_and(|n| n > 3)
        && said(
            &p,
            &[
                "people", "guests", "serve", "serves", "persons", "kids", "children",
            ],
        )
    {
        return Some(Kind::Serves);
    }
    None
}

/// The rules' broad reading, for the desks' model to write from, or to answer as they are when
/// no model can: any of these words, anywhere in the question.
fn rules_kind(q: &str) -> Option<Kind> {
    let t = q.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if has(&["kb-", "kb ", "my order", "where is", "track", "status"]) {
        return Some(Kind::Status);
    }
    if has(&["hold", "reserve", "keep "]) {
        return Some(Kind::Hold);
    }
    if has(&["warm", "fresh", "left", "stock", "today", "oven"]) {
        return Some(Kind::Stock);
    }
    if has(&["dairy", "vegan", "milk", "lactose", "butter"]) {
        return Some(Kind::Dairy);
    }
    if has(&["gluten", "coeliac", "celiac", "allerg", "nut"]) {
        return Some(Kind::Allergens);
    }
    if has(&[
        "design",
        "custom",
        "birthday",
        "wedding cake",
        "make me",
        "make a cake",
    ]) && !has(&["how much", "price", "cost"])
    {
        return Some(Kind::Design);
    }
    if has(&[
        "people", "serve", "how much", "price", "cost", "tier", "guests",
    ]) {
        return Some(Kind::Serves);
    }
    if has(&["cancel"]) {
        return Some(Kind::Cancel);
    }
    if has(&[
        "change",
        "move",
        "pickup time",
        "collection time",
        "reschedule",
        "different time",
    ]) {
        return Some(Kind::Change);
    }
    if has(&[
        "sunday",
        "open",
        "hour",
        "close",
        "monday",
        "saturday",
        "time do you",
    ]) && !has(&["deliver"])
    {
        return Some(Kind::Hours);
    }
    if has(&["park"]) {
        return Some(Kind::Park);
    }
    if has(&["deliver", "delivery"]) {
        return Some(Kind::Deliver);
    }
    None
}

/// What Kilele's capabilities answer instantly, without a model.
pub async fn instant(app: &App, ctx: &Ctx, q: &str) -> anyhow::Result<Option<Answer>> {
    match instant_kind(q, &ctx.site.fact("/order_prefix")) {
        Some(kind) => answer_kind(app, ctx, q, kind).await.map(Some),
        None => Ok(None),
    }
}

/// What Kilele's rules answer for a question: the facts the desks' model writes from, or the
/// answer itself when no model can (it then carries no AI label).
pub async fn rules(app: &App, ctx: &Ctx, q: &str) -> anyhow::Result<Option<Answer>> {
    match rules_kind(q) {
        Some(kind) => answer_kind(app, ctx, q, kind).await.map(Some),
        None => Ok(None),
    }
}

async fn answer_kind(app: &App, ctx: &Ctx, q: &str, kind: Kind) -> anyhow::Result<Answer> {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let t = q.to_lowercase();
    let owner = site.fact("/owner/first");
    match kind {
        Kind::Status => crate::commerce::order_status_answer(app, ctx, q).await,
        Kind::Order => {
            // The product the customer named, how many, what is left, and the total from the
            // price list. The card places the order; nothing is charged until they pay.
            let products = records.products(&site.site, None).await?;
            let words: Vec<String> = plain(q)
                .split_whitespace()
                .map(|w| w.trim_end_matches('s').to_owned())
                .collect();
            let score = |name: &str| {
                plain(name)
                    .split_whitespace()
                    .map(|w| w.trim_end_matches('s').to_owned())
                    .filter(|w| {
                        w.len() > 2
                            && (words.contains(w)
                                || (w == "sourdough"
                                    && words
                                        .iter()
                                        .any(|x| x == "loaf" || x == "loave" || x == "bread")))
                    })
                    .count()
            };
            let found = products
                .iter()
                .filter(|p| {
                    matches!(p.section.as_str(), "bread" | "box") && p.status == "published"
                })
                .map(|p| (score(&p.name), p))
                .filter(|(n, _)| *n > 0)
                .max_by_key(|(n, _)| *n)
                .map(|(_, p)| p);
            let mut a = Answer::instant("orders");
            let Some(p) = found else {
                a.say("Tell me which of today's bakes you'd like and how many, and I'll put the order together [1].".into());
                a.source(Source::new("Today's bakes", "/bread#bakes"));
                return Ok(a);
            };
            let count = |w: &str| match w {
                "a" | "an" | "one" => Some(1),
                "two" | "couple" => Some(2),
                "three" => Some(3),
                "four" => Some(4),
                "five" => Some(5),
                "six" | "half" => Some(6),
                "dozen" => Some(12),
                _ => w.parse::<i64>().ok(),
            };
            let n = plain(q)
                .split_whitespace()
                .filter_map(count)
                .find(|n| (1..=20).contains(n))
                .unwrap_or(1);
            let stock = records.stock(site, now).await?;
            let left = stock.get(p.sku.as_str()).and_then(|s| s.left);
            a.progress
                .push("Checking today's stock and the price list".into());
            if let Some(left) = left
                && left < n
            {
                a.say(format!(
                    "There {} **{left} {}** left today [1], so I can't take an order for {n}.",
                    if left == 1 { "is" } else { "are" },
                    p.name.to_lowercase()
                ));
                a.source(Source::new(&p.name, &format!("/bread#item-{}", p.slug)));
                return Ok(a);
            }
            let list = records
                .live_price_list(&site.site)
                .await?
                .ok_or_else(|| anyhow::anyhow!("no live price list"))?;
            let total = p.price_minor * n;
            let total_words = money::fmt(&site.currency, total);
            let usd = money::usd(money::usd_cents(total, list.usd_rate_minor));
            a.say(format!(
                "**{n} \u{d7} {}** comes to **{total_words}** ({usd}) [1]. Shall I place the order? You pay with PayPal, and nothing is charged until you do.",
                p.name
            ));
            a.source(Source::new(
                &format!(
                    "{} \u{b7} {}",
                    p.name,
                    money::fmt(&site.currency, p.price_minor)
                ),
                &format!("/bread#item-{}", p.slug),
            ));
            a.action(Action::pay(
                &format!("Place order \u{b7} pay {total_words} with PayPal"),
                json!({
                    "lines": [{ "sku": p.sku, "name": p.name, "qty": n, "each": money::fmt(&site.currency, p.price_minor), "amount": total_words }],
                    "total": total_words,
                    "usd": usd,
                    "note": format!("Priced from {}'s price list {}. You choose a collection or delivery slot at checkout.", site.name, list.version),
                }),
            ));
            a.meta = "From today's stock and the price list".into();
            Ok(a)
        }
        Kind::Hold => {
            let stock = records.stock(site, now).await?;
            let left = stock.get("sourdough").and_then(|s| s.left).unwrap_or(0);
            let max = site
                .facts
                .pointer("/hold_grant/max")
                .and_then(Value::as_i64)
                .unwrap_or(3);
            let until = site.fact("/hold_grant/until");
            let n = t
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|x| x.parse::<i64>().ok())
                .next()
                .unwrap_or(2);
            let mut a = Answer::new("bread");
            a.progress.push("Checking today's stock".into());
            if n > max {
                a.say(format!("I can hold up to **{max} loaves** without asking. For {n}, {owner} decides; I'll pass it to her."));
                a.action(Action::pass("Ask Wanjiru to hold them", q));
            } else if left < n {
                a.say(format!(
                    "Today there are **{left} country sourdough** left [1], so I can't hold {n}."
                ));
                a.source(Source::new("Country sourdough", "/bread#item-sourdough"));
            } else {
                a.say(format!("I can hold **{n} sourdough loaves** under your name until **{until}**. It's free, and you pay at the counter [1]."));
                a.source(Source::new(
                    &format!("Country sourdough \u{b7} {left} left"),
                    "/bread#item-sourdough",
                ));
                a.action(Action::hold(
                    &format!("Hold {n} loaves till {until}"),
                    "sourdough",
                    n,
                ));
            }
            Ok(a)
        }
        Kind::Stock => {
            let stock = records.stock(site, now).await?;
            let mut a = Answer::instant("bread");
            let sd = stock.get("sourdough").and_then(|s| s.left).unwrap_or(0);
            let rv = stock.get("red-velvet").and_then(|s| s.left).unwrap_or(0);
            a.say(format!(
                "Right now: **{sd} country sourdough** and **{rv} red velvet slices** [1]."
            ));
            if let Some(t) = stock.get("croissant").and_then(|s| s.next_batch_at) {
                a.say(format!(
                    "Croissants come out again at **{}** [2].",
                    time::fmt_clock(t, site.utc_offset_min)
                ));
            }
            a.source(Source::new("Today's bakes \u{b7} stock", "/bread#bakes"));
            a.source(Source::new("Butter croissant", "/bread#item-croissant"));
            a.meta = format!(
                "From today's stock list, {}",
                time::fmt_clock(now, site.utc_offset_min)
            );
            Ok(a)
        }
        Kind::Dairy => {
            let all = cards(records, site, Some("bread"), now).await?;
            let free: Vec<&Card> = all
                .iter()
                .filter(|c| !c.contains.iter().any(|x| x == "milk" || x == "egg"))
                .collect();
            let mut a = Answer::new("bread");
            a.progress.push("Reading today's ingredients".into());
            match free.as_slice() {
                [] => a.say("Everything on today's list uses butter, milk or egg [1].".into()),
                items => {
                    let names: Vec<String> = items
                        .iter()
                        .map(|c| {
                            format!(
                                "**{}** ({})",
                                c.name.to_lowercase(),
                                c.ingredients.trim_end_matches('.').to_lowercase()
                            )
                        })
                        .collect();
                    a.say(format!("Without dairy or egg: {} [1].", names.join(", ")));
                    a.say(
                        "Everything else on today's list uses butter or milk [2], so I'd skip those."
                            .into(),
                    );
                    a.source(Source::new(
                        &items[0].name,
                        &format!("/bread#item-{}", items[0].slug),
                    ));
                }
            }
            a.source(Source::new("Today's bakes", "/bread#bakes"));
            a.source(Source::new("Allergens: kitchen note", "/allergens#kitchen"));
            Ok(a)
        }
        Kind::Allergens => {
            let list = cake_list(records, site).await?;
            let mut a = Answer::new("cakes");
            a.progress.push("Reading allergen notes".into());
            a.say(format!(
                "Yes: a **gluten-free sponge** for 1- and 2-tier cakes, +{} [1].",
                list.gf.trim_start_matches('+')
            ));
            a.say(format!(
                "But {} [2]. {owner} can talk it through with you.",
                site.fact("/allergens").trim_end_matches('.')
            ));
            a.source(Source::new(
                "Cakes \u{b7} gluten-free note",
                "/cakes#note-gf",
            ));
            a.source(Source::new("Allergens: kitchen note", "/allergens#kitchen"));
            a.action(Action::pass("Ask Wanjiru about allergens", q));
            Ok(a)
        }
        Kind::Design => {
            let mut a = Answer::new("cakes");
            a.progress.push("Opening the cake studio".into());
            a.say(format!("I'll ask one thing at a time and price each choice from Kilele's price list. {owner} confirms every cake."));
            a.action(Action::link("Open the cake studio", "/cakes/studio", true));
            a.meta = "Prices come from the price list, never estimated".into();
            Ok(a)
        }
        Kind::Serves => {
            let list = cake_list(records, site).await?;
            let n = t
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|x| x.parse::<i64>().ok())
                .find(|n| *n > 3)
                .unwrap_or(30);
            let mut a = Answer::new("cakes");
            a.progress.push("Reading the cake price list".into());
            let fit = list.rows.iter().position(|r| {
                r.serves
                    .split('\u{2013}')
                    .nth(1)
                    .and_then(|m| m.parse::<i64>().ok())
                    .is_some_and(|max| max >= n)
            });
            match fit {
                Some(i) => {
                    let r = &list.rows[i];
                    a.say(format!(
                        "For about {n} people, **{}** fits: it serves {} and starts at **{}** [1].",
                        r.label.to_lowercase(),
                        r.serves,
                        r.from
                    ));
                    a.source(Source::new(
                        &format!("Cakes \u{b7} {}", r.label),
                        &format!("/cakes#{}", r.anchor),
                    ));
                    if let Some(next) = list.rows.get(i + 1) {
                        a.say(format!(
                            "**{}** serves {} from {} and needs {} notice [2].",
                            next.label, next.serves, next.from, next.notice
                        ));
                        a.source(Source::new(
                            &format!("Cakes \u{b7} {}", next.label),
                            &format!("/cakes#{}", next.anchor),
                        ));
                    }
                    a.say("Want to design one? I'll price each choice as we go.".into());
                }
                None => {
                    let big = list.rows.last();
                    a.say(format!(
                        "The largest cake on the price list is **{}**, for {} [1]. For {n} people, {owner} quotes it herself; I'll pass it to her.",
                        big.map(|r| r.label.to_lowercase()).unwrap_or_default(),
                        big.map(|r| r.serves.clone()).unwrap_or_default()
                    ));
                    if let Some(r) = big {
                        a.source(Source::new(
                            &format!("Cakes \u{b7} {}", r.label),
                            &format!("/cakes#{}", r.anchor),
                        ));
                    }
                    a.action(Action::pass("Ask Wanjiru for a quote", q));
                }
            }
            a.action(Action::link("Design it with me", "/cakes/studio", true));
            a.meta = format!("From {}", list.named);
            Ok(a)
        }
        Kind::Cancel => {
            let mut a = Answer::new("orders");
            a.say(format!(
                "Cancelling is free until {} hours before your slot [1].",
                site.facts
                    .get("changes_cutoff_hours")
                    .and_then(Value::as_i64)
                    .unwrap_or(24)
            ));
            a.say(format!(
                "Open the order and press Cancel. If it isn't paid yet, it's cancelled at once. If it's paid and not baked yet, {owner} decides, and if she agrees the whole amount goes back the way you paid."
            ));
            a.source(Source::new(
                "Help \u{b7} changes and cancellations",
                "/help#changes",
            ));
            a.action(Action::link("Find my order", "/orders", true));
            Ok(a)
        }
        Kind::Change => {
            let mut a = Answer::new("orders");
            a.say(format!(
                "I can ask {owner} to move it. {} [1] {} [2]",
                site.fact("/changes"),
                site.fact("/collection")
            ));
            a.source(Source::new(
                "Help \u{b7} changes and cancellations",
                "/help#changes",
            ));
            a.source(Source::new("Visit \u{b7} collection", "/visit#visit-hours"));
            a.action(Action::link("Open my orders", "/orders", true));
            Ok(a)
        }
        Kind::Hours => {
            let h = hours(site, now);
            let mut a = Answer::instant("visit");
            let rows: Vec<String> = h.rows.iter().map(|(d, t)| format!("**{d} {t}**")).collect();
            a.say(format!("{} [1]. {}.", rows.join(", "), h.now));
            a.source(Source::new("Visit \u{b7} Hours", "/visit#visit-hours"));
            let days = delivery_days(records, site, now).await?;
            if !days.is_empty() {
                a.say(format!("Delivery runs {days} [2]."));
                a.source(Source::new(
                    "Visit \u{b7} Delivery",
                    "/visit#visit-delivery",
                ));
            }
            a.meta = "From the hours on this site".into();
            Ok(a)
        }
        Kind::Park => {
            let mut a = Answer::instant("visit");
            a.say(format!("{} [1].", site.fact("/parking")));
            a.source(Source::new("Visit \u{b7} Find us", "/visit#visit-where"));
            a.meta = "From the directions on this site".into();
            Ok(a)
        }
        Kind::Deliver => {
            let zones = records.zones(&site.site).await?;
            let areas: Vec<String> = zones.iter().flat_map(|z| z.areas.clone()).collect();
            let fee = zones
                .first()
                .map(|z| money::fmt(&site.currency, z.fee_minor))
                .unwrap_or_default();
            let named = areas.iter().find(|a| t.contains(&a.to_lowercase()));
            let mut a = Answer::new("visit");
            a.progress.push("Reading delivery areas".into());
            a.source(Source::new(
                "Visit \u{b7} Delivery",
                "/visit#visit-delivery",
            ));
            match named {
                Some(area) => {
                    a.say(format!(
                        "Yes, Kilele delivers to **{area}** for **{fee}** [1]."
                    ));
                    let days = delivery_days(records, site, now).await?;
                    a.say(format!("Delivery runs {days} [2]."));
                    a.source(Source::new("Delivery \u{b7} times", "/delivery#slots"));
                }
                None => {
                    let asked = t
                        .split(" to ")
                        .nth(1)
                        .map(|s| s.trim_end_matches('?').trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .map_or_else(|| "there".to_owned(), |s| capitalise(&s));
                    a.say(format!(
                        "The site lists **{}** only [1], so I won't promise {asked}.",
                        areas.join(" and ")
                    ));
                    a.say(format!("I can ask {owner}. Want her answer?"));
                    a.action(Action::pass("Yes, ask Wanjiru", q));
                    a.meta = "From the delivery areas on this site".into();
                }
            }
            Ok(a)
        }
    }
}

/// The text Ask retrieves from: every section a page has, with its anchor.
pub async fn sections(app: &App, ctx: &Ctx) -> anyhow::Result<Vec<Section>> {
    let records = app.records().await?;
    let site = &ctx.site;
    let now = time::now();
    let mut out = Vec::new();
    for c in cards(records, site, None, now).await? {
        let page = if c.section == "box" || c.section == "seasonal" {
            format!("/shop/{}", c.slug)
        } else {
            "/bread".into()
        };
        out.push(Section::new(
            &format!(
                "{}{}",
                page,
                if page == "/bread" {
                    format!("#item-{}", c.slug)
                } else {
                    String::new()
                }
            ),
            &c.name,
            &format!(
                "{}. {} {}. {}. {}",
                c.name, c.price, c.description, c.ingredients, c.stock
            ),
        ));
    }
    let list = cake_list(records, site).await?;
    for r in &list.rows {
        out.push(Section::new(
            &format!("/cakes#{}", r.anchor),
            &format!("Cakes \u{b7} {}", r.label),
            &format!(
                "{} cake serves {}, needs {} notice, from {}.",
                r.label, r.serves, r.notice, r.from
            ),
        ));
    }
    let opts = |v: &[Option_]| {
        v.iter()
            .map(|o| format!("{} {}", o.label, o.delta))
            .collect::<Vec<_>>()
            .join(", ")
    };
    out.push(Section::new(
        "/cakes#options",
        "Cakes \u{b7} options",
        &format!("Sponges: {}. Buttercream: {}. Finishes: {}. Decorations: {}. A message of up to {} characters is free.", opts(&list.sponges), opts(&list.colours), opts(&list.finishes), opts(&list.decos), list.max_chars),
    ));
    out.push(Section::new(
        "/cakes#note-gf",
        "Cakes \u{b7} gluten-free note",
        &format!(
            "Gluten-free sponge is available for 1- and 2-tier cakes ({}). {}",
            list.gf,
            site.fact("/allergens")
        ),
    ));
    if let Some(no) = site.facts.get("not_offered").and_then(Value::as_array) {
        let items: Vec<&str> = no.iter().filter_map(Value::as_str).collect();
        out.push(Section::new(
            "/cakes#not-offered",
            "Cakes \u{b7} not offered",
            &format!("Kilele doesn't make: {}.", items.join(", ")),
        ));
    }
    out.push(Section::new(
        "/cakes#price-list",
        "Cakes \u{b7} price list",
        &format!(
            "Prices come from the price list of {}, valid until {}. Paying in dollars uses {}.",
            list.from_day, list.valid_until, list.rate
        ),
    ));
    let h = hours(site, now);
    out.push(Section::new(
        "/visit#visit-hours",
        "Visit \u{b7} Hours",
        &format!(
            "Hours: {}. {}",
            h.rows
                .iter()
                .map(|(d, t)| format!("{d} {t}"))
                .collect::<Vec<_>>()
                .join(", "),
            site.fact("/collection")
        ),
    ));
    out.push(Section::new(
        "/visit#visit-where",
        "Visit \u{b7} Find us",
        &format!("{}. {}.", site.fact("/address"), site.fact("/parking")),
    ));
    let zones = records.zones(&site.site).await?;
    for z in &zones {
        out.push(Section::new(
            "/visit#visit-delivery",
            "Visit \u{b7} Delivery",
            &format!(
                "Delivery to {} costs {}. {} Delivery runs {}.",
                z.areas.join(" and "),
                money::fmt(&site.currency, z.fee_minor),
                z.note,
                delivery_days(records, site, now).await?
            ),
        ));
    }
    out.push(Section::new(
        "/allergens#kitchen",
        "Allergens: kitchen note",
        &site.fact("/allergens"),
    ));
    out.push(Section::new(
        "/help#changes",
        "Help \u{b7} changes and cancellations",
        &site.fact("/changes"),
    ));
    out.push(Section::new(
        "/help#confirmations",
        "Help \u{b7} confirmations",
        &site.fact("/confirmations"),
    ));
    out.push(Section::new(
        "/help#classes",
        "Baking classes",
        &site.fact("/classes"),
    ));
    out.push(Section::new(
        "/help#desks",
        "Help \u{b7} what the desks can and can't do",
        "The desks on this site are AI. They answer from this site's pages, hold up to 3 loaves, design cakes from Kilele's own options and pass cake requests, changes and questions to Wanjiru. They never take payment details, change your account or promise anything the site doesn't say.",
    ));
    let owner = site.fact("/owner/first");
    out.push(Section::new(
        "/help#contact",
        "Help \u{b7} how Kilele contacts you",
        &format!(
            "{owner} answers herself on {}, {}. Messages from {} only link to {}. Payment requests appear only on your orders page on this site; don't pay through any other link.",
            site.fact("/owner/channel"),
            crate::common::owner_reply(site),
            site.name,
            link_host(ctx)
        ),
    ));
    out.push(Section::new(
        "/help#details",
        "Help \u{b7} changing your details",
        "Your name and phone are in Account, then Details; delivery addresses are in Account, then Addresses. Your email changes in Sign-in and security, which asks you to sign in again. A desk can never change your details.",
    ));
    for z in &zones {
        out.push(Section::new(
            "/delivery#fee-rule",
            "Delivery \u{b7} one fee per slot",
            &format!(
                "{} The fee for {} is {} a delivery.",
                z.note,
                z.label,
                money::fmt(&site.currency, z.fee_minor)
            ),
        ));
    }
    Ok(out)
}
