//! Loads a sample bundle (03-data.md §5.2): `manifest.json` plus `records/*.jsonl`.
//!
//! Times in a bundle are relative (see [`crate::time::resolve`]); the loader maps them to the
//! load instant, so "today's bake" and "next Saturday" stay current. Money is in major units
//! (`"price": 450` is KSh 450). Totals are never in a bundle: an order's subtotal, delivery and
//! total are summed from its lines, products and zone at load.
//!
//! Loading is idempotent: a site whose loaded `seed_version` matches the bundle is left alone,
//! unless `force` is set. A reload replaces every row of that site.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::{Records, time};

/// Tables a bundle can fill, each wiped for the site on reload.
const TABLES: [&str; 26] = [
    "rec_products",
    "rec_stock",
    "rec_bake_plan",
    "rec_price_lists",
    "rec_price_rules",
    "rec_zones",
    "rec_slots",
    "rec_customers",
    "rec_carts",
    "rec_orders",
    "rec_order_lines",
    "rec_order_events",
    "rec_payments",
    "rec_refunds",
    "rec_holds",
    "rec_bookings",
    "rec_questions",
    "rec_counters",
    "rec_authors",
    "rec_stories",
    "rec_corrections",
    "rec_timeline",
    "rec_subscribers",
    "rec_digest_subs",
    "rec_sessions",
    "rec_feedback",
];

#[derive(Deserialize)]
struct Manifest {
    site: String,
    host: String,
    name: String,
    archetype: String,
    currency: String,
    utc_offset_min: i32,
    seed_version: String,
    facts: Value,
}

/// What a load did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// The bundle was loaded (rows written).
    Loaded {
        /// The site slug.
        site: String,
        /// Rows written.
        rows: usize,
    },
    /// The same version was already loaded.
    Current {
        /// The site slug.
        site: String,
    },
}

fn minor(v: f64) -> i64 {
    (v * 100.0).round() as i64
}

struct Ctx {
    site: String,
    anchor: i64,
    off: i32,
}

impl Ctx {
    fn at(&self, spec: &str) -> anyhow::Result<i64> {
        time::resolve(spec, self.anchor, self.off)
    }
    fn at_opt(&self, spec: Option<&String>) -> anyhow::Result<Option<i64>> {
        spec.map(|s| self.at(s)).transpose()
    }
}

fn rows(dir: &Path, name: &str) -> anyhow::Result<Vec<Value>> {
    let path = dir.join("records").join(format!("{name}.jsonl"));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
        .map(|(i, l)| {
            serde_json::from_str(l).with_context(|| format!("{}:{}", path.display(), i + 1))
        })
        .collect()
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| match x {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    })
}

fn req(v: &Value, k: &str) -> anyhow::Result<String> {
    s(v, k).with_context(|| format!("missing {k} in {v}"))
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn int(v: &Value, k: &str, default: i64) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(default)
}

fn json(v: &Value, k: &str) -> String {
    v.get(k).map_or_else(|| "{}".into(), Value::to_string)
}

fn slot_id(kind: &str, ts: i64, off: i32) -> String {
    format!("{kind}-{}", time::local(ts, off).format("%Y-%m-%d-%H%M"))
}

impl Records {
    /// Loads the bundle in `dir` unless that version is already loaded (or `force`).
    /// `host` overrides the manifest's host (previews on `*.localhost`).
    ///
    /// # Errors
    ///
    /// When the bundle is malformed or on a database error.
    pub async fn load_bundle(
        &self,
        dir: &Path,
        host: Option<&str>,
        force: bool,
        now: i64,
    ) -> anyhow::Result<Loaded> {
        let manifest: Manifest = serde_json::from_str(
            &std::fs::read_to_string(dir.join("manifest.json"))
                .with_context(|| format!("reading {}/manifest.json", dir.display()))?,
        )
        .context("manifest.json")?;
        if !force
            && let Some(existing) = self.site(&manifest.site).await?
            && existing.seed_version == manifest.seed_version
        {
            return Ok(Loaded::Current {
                site: manifest.site,
            });
        }
        let cx = Ctx {
            site: manifest.site.clone(),
            anchor: now,
            off: manifest.utc_offset_min,
        };
        let site = cx.site.as_str();
        let mut tx = self.pool().begin().await?;
        for table in TABLES {
            sqlx::query(sql!("DELETE FROM {table} WHERE site = ?"))
                .bind(site)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM rec_sites WHERE site = ?")
            .bind(site)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO rec_sites (site, host, name, archetype, currency, utc_offset_min, origin, facts, seed_version, anchor_at, loaded_at)
             VALUES (?, ?, ?, ?, ?, ?, 'sample', ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(host.unwrap_or(&manifest.host))
        .bind(&manifest.name)
        .bind(&manifest.archetype)
        .bind(&manifest.currency)
        .bind(manifest.utc_offset_min)
        .bind(manifest.facts.to_string())
        .bind(&manifest.seed_version)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let mut count = 0usize;

        let mut prices: HashMap<String, i64> = HashMap::new();
        let mut names: HashMap<String, String> = HashMap::new();
        for (i, p) in rows(dir, "products")?.iter().enumerate() {
            let sku = req(p, "sku")?;
            let price = minor(num(p, "price").with_context(|| format!("price of {sku}"))?);
            prices.insert(sku.clone(), price);
            names.insert(sku.clone(), req(p, "name")?);
            sqlx::query(
                "INSERT INTO rec_products (site, sku, slug, section, name, description, family, size, price_minor, attrs, status, sort)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(&sku)
            .bind(s(p, "slug").unwrap_or_else(|| sku.clone()))
            .bind(req(p, "section")?)
            .bind(req(p, "name")?)
            .bind(s(p, "description").unwrap_or_default())
            .bind(s(p, "family"))
            .bind(s(p, "size"))
            .bind(price)
            .bind(json(p, "attrs"))
            .bind(s(p, "status").unwrap_or_else(|| "published".into()))
            .bind(int(p, "sort", i as i64))
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for b in rows(dir, "bake_plan")? {
            sqlx::query(
                "INSERT INTO rec_bake_plan (site, sku, planned, batches) VALUES (?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&b, "sku")?)
            .bind(int(&b, "planned", 0))
            .bind(
                b.get("batches")
                    .map_or_else(|| "[]".into(), Value::to_string),
            )
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for st in rows(dir, "stock")? {
            sqlx::query(
                "INSERT INTO rec_stock (site, sku, day, planned, next_batch_at, back_at, as_of, source) VALUES (?, ?, '', ?, NULL, NULL, ?, ?)",
            )
            .bind(site)
            .bind(req(&st, "sku")?)
            .bind(int(&st, "on_hand", 0))
            .bind(cx.at(&s(&st, "as_of").unwrap_or_else(|| "m-0".into()))?)
            .bind(s(&st, "source").unwrap_or_else(|| "stock take".into()))
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        let mut rules: HashMap<String, (String, i64)> = HashMap::new();
        let mut live_rate = 0;
        let mut live_version = String::new();
        for pl in rows(dir, "price_lists")? {
            let rate = minor(num(&pl, "usd_rate").unwrap_or(0.0));
            if s(&pl, "status").as_deref() == Some("live") {
                live_rate = rate;
                live_version = req(&pl, "version")?;
            }
            sqlx::query(
                "INSERT INTO rec_price_lists (site, version, currency, status, usd_rate_minor, valid_from, valid_until, source) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&pl, "version")?)
            .bind(&manifest.currency)
            .bind(req(&pl, "status")?)
            .bind(rate)
            .bind(cx.at(&req(&pl, "valid_from")?)?)
            .bind(cx.at(&req(&pl, "valid_until")?)?)
            .bind(s(&pl, "source").unwrap_or_default())
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for (i, r) in rows(dir, "price_rules")?.iter().enumerate() {
            let code = req(r, "code")?;
            let amount = minor(num(r, "amount").unwrap_or(0.0));
            let version = req(r, "version")?;
            if version == live_version {
                rules.insert(code.clone(), (req(r, "label")?, amount));
            }
            sqlx::query(
                "INSERT INTO rec_price_rules (site, version, code, grp, label, amount_minor, meta, sort) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(&version)
            .bind(&code)
            .bind(req(r, "grp")?)
            .bind(req(r, "label")?)
            .bind(amount)
            .bind(json(r, "meta"))
            .bind(int(r, "sort", i as i64))
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        let mut zones: HashMap<String, i64> = HashMap::new();
        for z in rows(dir, "zones")? {
            let fee = minor(num(&z, "fee").unwrap_or(0.0));
            zones.insert(req(&z, "zone")?, fee);
            sqlx::query("INSERT INTO rec_zones (site, zone, label, fee_minor, areas, note) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(site)
                .bind(req(&z, "zone")?)
                .bind(req(&z, "label")?)
                .bind(fee)
                .bind(z.get("areas").map_or_else(|| "[]".into(), Value::to_string))
                .bind(s(&z, "note").unwrap_or_default())
                .execute(&mut *tx)
                .await?;
            count += 1;
        }
        // Slots: weekly rules made concrete for the coming weeks.
        for rule in rows(dir, "slot_rules")? {
            let kind = req(&rule, "kind")?;
            let weeks = int(&rule, "weeks", 4);
            let days: Vec<String> = rule
                .get("weekdays")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            for day in days {
                let today = time::local(now, cx.off)
                    .format("%a")
                    .to_string()
                    .to_lowercase();
                for w in -2..=weeks {
                    // Week 0 is today, when today is that weekday.
                    let at = match w {
                        0 if today == day => "d+0".to_owned(),
                        0 => continue,
                        w => format!("{day}{w:+}"),
                    };
                    let starts = cx.at(&format!("{at} {}", req(&rule, "start")?))?;
                    let ends = cx.at(&format!("{at} {}", req(&rule, "end")?))?;
                    sqlx::query(
                        "INSERT OR IGNORE INTO rec_slots (site, slot_id, kind, starts_at, ends_at, capacity) VALUES (?, ?, ?, ?, ?, ?)",
                    )
                    .bind(site)
                    .bind(slot_id(&kind, starts, cx.off))
                    .bind(&kind)
                    .bind(starts)
                    .bind(ends)
                    .bind(int(&rule, "capacity", 6))
                    .execute(&mut *tx)
                    .await?;
                    count += 1;
                }
            }
        }
        for c in rows(dir, "customers")? {
            sqlx::query(
                "INSERT INTO rec_customers (site, customer_id, name, email, phone, kind, place, sample, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)",
            )
            .bind(site)
            .bind(req(&c, "customer_id")?)
            .bind(req(&c, "name")?)
            .bind(req(&c, "email")?.to_lowercase())
            .bind(s(&c, "phone"))
            .bind(s(&c, "kind").unwrap_or_else(|| "person".into()))
            .bind(s(&c, "place"))
            .bind(cx.at(&s(&c, "created_at").unwrap_or_else(|| "d-60 10:00".into()))?)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        let mut max_order = 0i64;
        for o in rows(dir, "orders")? {
            let order_id = req(&o, "order_id")?;
            if let Some(n) = order_id
                .rsplit('-')
                .next()
                .and_then(|n| n.parse::<i64>().ok())
            {
                max_order = max_order.max(n);
            }
            let created = cx.at(&req(&o, "created_at")?)?;
            let slot = match (s(&o, "slot"), s(&o, "slot_kind")) {
                (Some(spec), Some(kind)) => Some(slot_id(&kind, cx.at(&spec)?, cx.off)),
                _ => None,
            };
            let zone = s(&o, "zone");
            let delivery = zone
                .as_ref()
                .and_then(|z| zones.get(z))
                .copied()
                .unwrap_or(0);
            // (sku, name, qty, unit, rule code)
            type Line = (Option<String>, String, i64, i64, Option<String>);
            let mut lines: Vec<Line> = Vec::new();
            for l in o
                .get("lines")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if let Some(sku) = s(&l, "sku") {
                    let unit = *prices
                        .get(&sku)
                        .with_context(|| format!("{order_id}: no product {sku}"))?;
                    let name = names.get(&sku).cloned().unwrap_or_default();
                    lines.push((Some(sku), name, int(&l, "qty", 1), unit, None));
                } else if let Some(code) = s(&l, "rule") {
                    let (label, amount) = rules
                        .get(&code)
                        .cloned()
                        .with_context(|| format!("{order_id}: no rule {code}"))?;
                    lines.push((None, label, 1, amount, Some(code)));
                } else if let Some(name) = s(&l, "name") {
                    lines.push((
                        None,
                        name,
                        int(&l, "qty", 1),
                        minor(num(&l, "price").unwrap_or(0.0)),
                        None,
                    ));
                } else {
                    bail!("{order_id}: a line needs sku, rule or name");
                }
            }
            let subtotal: i64 = lines.iter().map(|(_, _, q, u, _)| q * u).sum();
            sqlx::query(
                "INSERT INTO rec_orders (site, order_id, customer_id, contact_name, contact_email, contact_phone, channel, kind, state,
                   subtotal_minor, delivery_minor, total_minor, currency, usd_rate_minor, slot_id, zone, fulfilment, recipient, note,
                   spec, price_list, quote_valid_until, agent, idempotency_key, sample, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL, 1, ?, ?)",
            )
            .bind(site)
            .bind(&order_id)
            .bind(s(&o, "customer"))
            .bind(req(&o, "contact_name")?)
            .bind(s(&o, "contact_email"))
            .bind(s(&o, "contact_phone"))
            .bind(s(&o, "channel").unwrap_or_else(|| "site".into()))
            .bind(req(&o, "kind")?)
            .bind(req(&o, "state")?)
            .bind(subtotal)
            .bind(delivery)
            .bind(subtotal + delivery)
            .bind(&manifest.currency)
            .bind(live_rate)
            .bind(&slot)
            .bind(&zone)
            .bind(s(&o, "fulfilment").unwrap_or_else(|| "collection".into()))
            .bind(s(&o, "recipient"))
            .bind(s(&o, "note"))
            .bind(o.get("spec").map(Value::to_string))
            .bind(s(&o, "price_list"))
            .bind(s(&o, "agent"))
            .bind(created)
            .bind(created)
            .execute(&mut *tx)
            .await?;
            let mut n = 0;
            for (sku, name, qty, unit, code) in &lines {
                n += 1;
                sqlx::query(
                    "INSERT INTO rec_order_lines (site, order_id, line_no, sku, name, qty, unit_minor, rule_code, amount_minor) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(site)
                .bind(&order_id)
                .bind(n)
                .bind(sku)
                .bind(name)
                .bind(qty)
                .bind(unit)
                .bind(code)
                .bind(qty * unit)
                .execute(&mut *tx)
                .await?;
            }
            if delivery > 0 {
                sqlx::query(
                    "INSERT INTO rec_order_lines (site, order_id, line_no, sku, name, qty, unit_minor, rule_code, amount_minor) VALUES (?, ?, ?, NULL, 'Delivery', 1, ?, ?, ?)",
                )
                .bind(site)
                .bind(&order_id)
                .bind(n + 1)
                .bind(delivery)
                .bind(zone.as_ref().map(|z| format!("delivery.{z}")))
                .bind(delivery)
                .execute(&mut *tx)
                .await?;
            }
            for (i, e) in o
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                sqlx::query("INSERT INTO rec_order_events (site, order_id, seq, step, at, actor, note) VALUES (?, ?, ?, ?, ?, ?, ?)")
                    .bind(site)
                    .bind(&order_id)
                    .bind(i as i64 + 1)
                    .bind(req(e, "step")?)
                    .bind(cx.at(&req(e, "at")?)?)
                    .bind(req(e, "actor")?)
                    .bind(s(e, "note"))
                    .execute(&mut *tx)
                    .await?;
            }
            for p in o
                .get("payments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let usd = crate::money::usd_cents(subtotal + delivery, live_rate);
                let (amount, currency) = match s(&p, "currency").as_deref() {
                    Some("USD") => (usd, "USD".to_owned()),
                    _ => (subtotal + delivery, manifest.currency.clone()),
                };
                sqlx::query(
                    "INSERT INTO rec_payments (site, payment_id, order_id, rail, provider_ref, amount_minor, currency, state, created_at, settled_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(site)
                .bind(req(&p, "payment_id")?)
                .bind(&order_id)
                .bind(req(&p, "rail")?)
                .bind(s(&p, "provider_ref"))
                .bind(amount)
                .bind(currency)
                .bind(s(&p, "state").unwrap_or_else(|| "sample".into()))
                .bind(cx.at(&req(&p, "at")?)?)
                .bind(cx.at(&req(&p, "at")?)?)
                .execute(&mut *tx)
                .await?;
            }
            count += 1 + lines.len();
        }
        for r in rows(dir, "refunds")? {
            let order: String = req(&r, "order_id")?;
            let amount = match s(&r, "sku") {
                Some(sku) => prices.get(&sku).copied().unwrap_or(0) * int(&r, "qty", 1),
                None => minor(num(&r, "amount").unwrap_or(0.0)),
            };
            sqlx::query("INSERT INTO rec_refunds (site, refund_id, order_id, amount_minor, reason, rule, state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(site)
                .bind(req(&r, "refund_id")?)
                .bind(order)
                .bind(amount)
                .bind(req(&r, "reason")?)
                .bind(s(&r, "rule"))
                .bind(req(&r, "state")?)
                .bind(cx.at(&req(&r, "at")?)?)
                .execute(&mut *tx)
                .await?;
            count += 1;
        }
        if max_order > 0 {
            sqlx::query("INSERT INTO rec_counters (site, name, next) VALUES (?, 'order', ?)")
                .bind(site)
                .bind(max_order + 1)
                .execute(&mut *tx)
                .await?;
        }
        let mut max_hold = 0i64;
        for h in rows(dir, "holds")? {
            let id = req(&h, "hold_id")?;
            if let Some(n) = id.rsplit('-').next().and_then(|n| n.parse::<i64>().ok()) {
                max_hold = max_hold.max(n);
            }
            let until = cx.at(&req(&h, "until")?)?;
            sqlx::query(
                "INSERT INTO rec_holds (site, hold_id, sku, qty, day, until_at, name, grant_id, state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(&id)
            .bind(req(&h, "sku")?)
            .bind(int(&h, "qty", 1))
            .bind(time::day_of(until, cx.off))
            .bind(until)
            .bind(req(&h, "name")?)
            .bind(s(&h, "grant_id").unwrap_or_else(|| "g-2".into()))
            .bind(s(&h, "state").unwrap_or_else(|| "held".into()))
            .bind(cx.at(&req(&h, "created_at")?)?)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        if max_hold > 0 {
            sqlx::query("INSERT INTO rec_counters (site, name, next) VALUES (?, 'hold', ?)")
                .bind(site)
                .bind(max_hold + 1)
                .execute(&mut *tx)
                .await?;
        }
        for b in rows(dir, "bookings")? {
            sqlx::query(
                "INSERT INTO rec_bookings (site, booking_id, service, starts_at, customer_id, name, seats, state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&b, "booking_id")?)
            .bind(req(&b, "service")?)
            .bind(cx.at(&req(&b, "starts_at")?)?)
            .bind(s(&b, "customer"))
            .bind(req(&b, "name")?)
            .bind(int(&b, "seats", 1))
            .bind(s(&b, "state").unwrap_or_else(|| "booked".into()))
            .bind(cx.at(&req(&b, "created_at")?)?)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for q in rows(dir, "questions")? {
            sqlx::query(
                "INSERT INTO rec_questions (site, question_id, desk, question, contact, page, state, answer, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&q, "question_id")?)
            .bind(req(&q, "desk")?)
            .bind(req(&q, "question")?)
            .bind(s(&q, "contact"))
            .bind(s(&q, "page"))
            .bind(s(&q, "state").unwrap_or_else(|| "open".into()))
            .bind(s(&q, "answer"))
            .bind(cx.at(&req(&q, "at")?)?)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for a in rows(dir, "authors")? {
            sqlx::query(
                "INSERT INTO rec_authors (site, author_id, name, role, bio) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&a, "author_id")?)
            .bind(req(&a, "name")?)
            .bind(s(&a, "role").unwrap_or_default())
            .bind(s(&a, "bio").unwrap_or_default())
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for st in rows(dir, "stories")? {
            let published = cx.at(&req(&st, "published_at")?)?;
            sqlx::query(
                "INSERT INTO rec_stories (site, slug, section, kicker, title, standfirst, body, author_id, topic, entitlement, status, published_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&st, "slug")?)
            .bind(req(&st, "section")?)
            .bind(s(&st, "kicker"))
            .bind(req(&st, "title")?)
            .bind(s(&st, "standfirst").unwrap_or_default())
            .bind(st.get("body").map_or_else(|| "[]".into(), Value::to_string))
            .bind(req(&st, "author")?)
            .bind(s(&st, "topic"))
            .bind(s(&st, "entitlement").unwrap_or_else(|| "free".into()))
            .bind(s(&st, "status").unwrap_or_else(|| "published".into()))
            .bind(published)
            .bind(cx.at_opt(s(&st, "updated_at").as_ref())?.unwrap_or(published))
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for c in rows(dir, "corrections")? {
            sqlx::query("INSERT INTO rec_corrections (site, story_slug, at, what, note) VALUES (?, ?, ?, ?, ?)")
                .bind(site)
                .bind(req(&c, "story")?)
                .bind(cx.at(&req(&c, "at")?)?)
                .bind(req(&c, "what")?)
                .bind(req(&c, "note")?)
                .execute(&mut *tx)
                .await?;
            count += 1;
        }
        for t in rows(dir, "timeline")? {
            sqlx::query("INSERT INTO rec_timeline (site, topic, at, line, story_slug, kind) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(site)
                .bind(req(&t, "topic")?)
                .bind(cx.at(&req(&t, "at")?)?)
                .bind(req(&t, "line")?)
                .bind(s(&t, "story"))
                .bind(s(&t, "kind").unwrap_or_else(|| "step".into()))
                .execute(&mut *tx)
                .await?;
            count += 1;
        }
        for sb in rows(dir, "subscribers")? {
            sqlx::query(
                "INSERT INTO rec_subscribers (site, subscriber_id, customer_id, plan, state, source, since, until_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&sb, "subscriber_id")?)
            .bind(req(&sb, "customer")?)
            .bind(req(&sb, "plan")?)
            .bind(s(&sb, "state").unwrap_or_else(|| "active".into()))
            .bind(s(&sb, "source").unwrap_or_else(|| "sample".into()))
            .bind(cx.at(&req(&sb, "since")?)?)
            .bind(cx.at_opt(s(&sb, "until").as_ref())?)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        for d in rows(dir, "digest_subs")? {
            sqlx::query(
                "INSERT INTO rec_digest_subs (site, sub_id, email, topics, state, token_hash, created_at, confirmed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(site)
            .bind(req(&d, "sub_id")?)
            .bind(req(&d, "email")?)
            .bind(d.get("topics").map_or_else(|| "[]".into(), Value::to_string))
            .bind(s(&d, "state").unwrap_or_else(|| "confirmed".into()))
            .bind(crate::sha256_hex(&req(&d, "sub_id")?))
            .bind(cx.at(&req(&d, "created_at")?)?)
            .bind(cx.at_opt(s(&d, "confirmed_at").as_ref())?)
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        tx.commit().await?;
        Ok(Loaded::Loaded {
            site: manifest.site,
            rows: count,
        })
    }
}
