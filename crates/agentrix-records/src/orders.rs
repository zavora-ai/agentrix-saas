//! Delivery zones and slots, carts, orders and their events, payments, refunds, holds and
//! bookings.
//!
//! Order states: `awaiting_payment` (a catalogue order before PayPal), `held` (a custom order
//! waiting for the owner), `approved` (the payment request is out), `paid`, `baking`, `ready`,
//! `out`, `delivered`, `collected`; and the ends `declined`, `lapsed`, `cancelled`, `refunded`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalogue::ACTIVE;
use crate::pricing::{CakeChoice, CakeQuote};
use crate::{Records, Site, money, new_id, time};

/// A delivery zone.
#[derive(Debug, Clone, Serialize)]
pub struct Zone {
    /// `zone1`.
    pub zone: String,
    /// "Westlands and Kilimani".
    pub label: String,
    /// The fee for one delivery.
    pub fee_minor: i64,
    /// Areas it covers.
    pub areas: Vec<String>,
    /// "Same day if ordered before 14:00".
    pub note: String,
}

/// A delivery or collection slot with what is taken.
#[derive(Debug, Clone, Serialize)]
pub struct Slot {
    /// `sat-2026-11-14-1000`.
    pub slot_id: String,
    /// `delivery` or `collection`.
    pub kind: String,
    /// Start.
    pub starts_at: i64,
    /// End.
    pub ends_at: i64,
    /// Orders it takes.
    pub capacity: i64,
    /// Active orders in it.
    pub taken: i64,
    /// `taken >= capacity`.
    pub full: bool,
    /// "Sat 14 Nov, 10:00–12:00".
    pub label: String,
}

/// A line in a cart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CartLine {
    /// The sku.
    pub sku: String,
    /// How many.
    pub qty: i64,
    /// Gift wrap and card (shops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
}

/// A cart.
#[derive(Debug, Clone, Serialize)]
pub struct Cart {
    /// Its id (also the browser's cart cookie).
    pub cart_id: String,
    /// The customer, once signed in.
    pub customer_id: Option<String>,
    /// Lines.
    pub lines: Vec<CartLine>,
    /// The chosen slot.
    pub slot_id: Option<String>,
    /// The chosen zone.
    pub zone: Option<String>,
}

/// A priced cart line, with where each fact came from.
#[derive(Debug, Clone, Serialize)]
pub struct PricedLine {
    /// The sku.
    pub sku: String,
    /// Product slug.
    pub slug: String,
    /// Name.
    pub name: String,
    /// How many.
    pub qty: i64,
    /// Unit price.
    pub unit_minor: i64,
    /// `qty × unit`.
    pub amount_minor: i64,
    /// Units left for the day, if stock is kept.
    pub left: Option<i64>,
    /// Whether the quantity is available.
    pub available: bool,
    /// The card message, if any.
    pub card: Option<String>,
}

/// A cart priced from the live list, ready for checkout.
#[derive(Debug, Clone, Serialize)]
pub struct CartView {
    /// The cart.
    pub cart: Cart,
    /// Its priced lines.
    pub lines: Vec<PricedLine>,
    /// Number of items.
    pub items: i64,
    /// Sum of lines.
    pub subtotal_minor: i64,
    /// The zone's fee, when delivering.
    pub delivery_minor: i64,
    /// Subtotal plus delivery.
    pub total_minor: i64,
    /// Total in US cents at the posted rate.
    pub usd_cents: i64,
    /// The price list version.
    pub price_list: Option<String>,
    /// The posted rate.
    pub usd_rate_minor: i64,
    /// Until when the quote holds.
    pub valid_until: Option<i64>,
    /// What stops checkout, in words.
    pub problems: Vec<String>,
}

/// Who an order is for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contact {
    /// Name on the order.
    pub name: String,
    /// Email for receipts.
    pub email: Option<String>,
    /// Phone for updates.
    pub phone: Option<String>,
    /// Who receives it, if not the customer ("Amina, at home in Kilimani").
    pub recipient: Option<String>,
}

/// An agent that proved who it is at the agent door (v3, "Verify at sample doors").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct AgentProof {
    /// Its verified id: `wba:{origin}#{key}`, or the token's principal.
    pub id: String,
    /// Who vouched: the key directory's origin, or the token's issuer in words.
    pub vouched_by: String,
    /// How: `web-bot-auth` or `token`.
    pub method: String,
}

/// How an order is placed.
#[derive(Debug, Clone, Default)]
pub struct Placement {
    /// The customer row, when signed in.
    pub customer_id: Option<String>,
    /// `site`, `agent door`, `owner`.
    pub channel: String,
    /// The agent, when placed through the door ("assistant.example · known").
    pub agent: Option<String>,
    /// Who the agent proved itself to be at the door, when it did.
    pub proof: Option<AgentProof>,
    /// `delivery` or `collection`.
    pub fulfilment: String,
    /// The slot.
    pub slot_id: Option<String>,
    /// The zone, when delivering.
    pub zone: Option<String>,
    /// A note.
    pub note: Option<String>,
    /// Makes a retry return the first order.
    pub idempotency_key: Option<String>,
}

/// An order row.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Order {
    /// `KB-1042`.
    pub order_id: String,
    /// The customer row.
    pub customer_id: Option<String>,
    /// Name on the order.
    pub contact_name: String,
    /// Email.
    pub contact_email: Option<String>,
    /// Phone.
    pub contact_phone: Option<String>,
    /// `site`, `agent door`, `owner`.
    pub channel: String,
    /// `catalogue`, `custom`, `subscription`.
    pub kind: String,
    /// State.
    pub state: String,
    /// Sum of lines.
    pub subtotal_minor: i64,
    /// Delivery fee.
    pub delivery_minor: i64,
    /// Total.
    pub total_minor: i64,
    /// Currency.
    pub currency: String,
    /// Pinned rate.
    pub usd_rate_minor: i64,
    /// The slot.
    pub slot_id: Option<String>,
    /// The zone.
    pub zone: Option<String>,
    /// `delivery` or `collection`.
    pub fulfilment: String,
    /// Who receives it.
    pub recipient: Option<String>,
    /// A note.
    pub note: Option<String>,
    /// The custom design, JSON text.
    pub spec: Option<String>,
    /// Price list version pinned.
    pub price_list: Option<String>,
    /// Until when the quote held.
    pub quote_valid_until: Option<i64>,
    /// The agent, if placed through the door.
    pub agent: Option<String>,
    /// The proven agent's verified id ([`Order::proven_agent`]).
    pub agent_id: Option<String>,
    /// Who vouched for it.
    pub agent_vouched_by: Option<String>,
    /// How it proved itself.
    pub agent_proof_method: Option<String>,
    /// The courier a delivery was booked with (its site's host), once booked.
    pub courier: Option<String>,
    /// The courier's tracking number, once booked.
    pub tracking: Option<String>,
    /// Sample row from the bundle.
    pub sample: i64,
    /// Placed.
    pub created_at: i64,
    /// Last change.
    pub updated_at: i64,
}

impl Order {
    /// The agent that raised it, as it proved itself at the door; `None` when no proven agent
    /// did.
    pub fn proven_agent(&self) -> Option<AgentProof> {
        Some(AgentProof {
            id: self.agent_id.clone()?,
            vouched_by: self.agent_vouched_by.clone().unwrap_or_default(),
            method: self.agent_proof_method.clone().unwrap_or_default(),
        })
    }

    /// Total in US cents at the pinned rate.
    pub fn usd_cents(&self) -> i64 {
        money::usd_cents(self.total_minor, self.usd_rate_minor)
    }

    /// Whether it still holds stock or a slot.
    pub fn is_active(&self) -> bool {
        ACTIVE.contains(&format!("'{}'", self.state))
    }

    /// The custom design, parsed.
    pub fn spec(&self) -> Option<Value> {
        self.spec
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
    }
}

/// An order line.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OrderLine {
    /// The sku, for catalogue lines.
    pub sku: Option<String>,
    /// What it is.
    pub name: String,
    /// How many.
    pub qty: i64,
    /// Unit price.
    pub unit_minor: i64,
    /// The price rule, for configured lines.
    pub rule_code: Option<String>,
    /// Line amount.
    pub amount_minor: i64,
}

/// An order event.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OrderEvent {
    /// Its number in the order.
    pub seq: i64,
    /// `requested`, `approved`, `paid`, `baking`, `out`, `delivered` …
    pub step: String,
    /// When.
    pub at: i64,
    /// Who: "Njeri", "Wanjiru", "the Cashier", "the Orders desk".
    pub actor: String,
    /// Words for the timeline.
    pub note: Option<String>,
}

/// A payment.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Payment {
    /// `pay_…`.
    pub payment_id: String,
    /// `paypal` (the samples take PayPal only).
    pub rail: String,
    /// The provider's id.
    pub provider_ref: Option<String>,
    /// Amount in `currency` minor units.
    pub amount_minor: i64,
    /// `USD` for PayPal.
    pub currency: String,
    /// `started`, `captured`, `declined`, `cancelled`, `not_executed`, `sample`.
    pub state: String,
    /// Started.
    pub created_at: i64,
    /// Settled.
    pub settled_at: Option<i64>,
}

/// A refund.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Refund {
    /// `R-1042-1`.
    pub refund_id: String,
    /// The order.
    pub order_id: String,
    /// Amount in the order's currency.
    pub amount_minor: i64,
    /// Why.
    pub reason: String,
    /// The standing rule it ran under, or `None` when the owner decided.
    pub rule: Option<String>,
    /// `done`, `waiting`, `declined`.
    pub state: String,
    /// When.
    pub created_at: i64,
}

/// An order with everything its page shows.
#[derive(Debug, Clone, Serialize)]
pub struct OrderView {
    /// The order.
    pub order: Order,
    /// Lines.
    pub lines: Vec<OrderLine>,
    /// Events, oldest first.
    pub events: Vec<OrderEvent>,
    /// Payments, newest first.
    pub payments: Vec<Payment>,
    /// Refunds.
    pub refunds: Vec<Refund>,
    /// The slot, if any.
    pub slot: Option<Slot>,
}

impl OrderView {
    /// The customer's request to cancel, while it waits for the owner: the last cancel step is
    /// `cancel_requested` (not `cancel_kept`) and the order is still paid.
    pub fn cancel_asked(&self) -> Option<&OrderEvent> {
        if self.order.state != "paid" {
            return None;
        }
        self.events
            .iter()
            .rev()
            .find(|e| e.step.starts_with("cancel_"))
            .filter(|e| e.step == "cancel_requested")
    }
}

/// A hold under a standing grant.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Hold {
    /// `H-0193`.
    pub hold_id: String,
    /// The sku.
    pub sku: String,
    /// How many.
    pub qty: i64,
    /// The day.
    pub day: String,
    /// Until when.
    pub until_at: i64,
    /// Under whose name.
    pub name: String,
    /// The standing grant, `g-2`.
    pub grant_id: String,
    /// `held`, `collected`, `released`.
    pub state: String,
    /// When.
    pub created_at: i64,
}

/// A booking (a seat on a class).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Booking {
    /// `BK-…`.
    pub booking_id: String,
    /// The service.
    pub service: String,
    /// When it starts.
    pub starts_at: i64,
    /// The customer row.
    pub customer_id: Option<String>,
    /// Name.
    pub name: String,
    /// Seats.
    pub seats: i64,
    /// `booked`, `cancelled`.
    pub state: String,
    /// When.
    pub created_at: i64,
}

/// Why an order or hold could not be made.
#[derive(Debug, thiserror::Error)]
pub enum OrderError {
    /// Visitor words: what stops it.
    #[error("{0}")]
    Refused(String),
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<sqlx::Error> for OrderError {
    fn from(e: sqlx::Error) -> Self {
        Self::Other(e.into())
    }
}

const ORDER_COLS: &str = "order_id, customer_id, contact_name, contact_email, contact_phone, channel, kind, state, subtotal_minor, delivery_minor, total_minor, currency, usd_rate_minor, slot_id, zone, fulfilment, recipient, note, spec, price_list, quote_valid_until, agent, agent_id, agent_vouched_by, agent_proof_method, courier, tracking, sample, created_at, updated_at";

fn slot_label(starts: i64, ends: i64, off: i32) -> String {
    format!(
        "{}, {}\u{2013}{}",
        time::fmt_day(starts, off),
        time::fmt_clock(starts, off),
        time::fmt_clock(ends, off)
    )
}

impl Records {
    /// Delivery zones.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn zones(&self, site: &str) -> anyhow::Result<Vec<Zone>> {
        let rows: Vec<(String, String, i64, String, String)> = sqlx::query_as(
            "SELECT zone, label, fee_minor, areas, note FROM rec_zones WHERE site = ? ORDER BY fee_minor, zone",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|(zone, label, fee_minor, areas, note)| Zone {
                zone,
                label,
                fee_minor,
                areas: serde_json::from_str(&areas).unwrap_or_default(),
                note,
            })
            .collect())
    }

    /// The zone that covers an area, by name, if any.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn zone_for(&self, site: &str, area: &str) -> anyhow::Result<Option<Zone>> {
        let area = area.trim().to_lowercase();
        Ok(self
            .zones(site)
            .await?
            .into_iter()
            .find(|z| z.areas.iter().any(|a| a.to_lowercase() == area)))
    }

    /// Slots of a kind between two times, with what active orders took.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn slots(
        &self,
        site: &Site,
        kind: &str,
        from: i64,
        to: i64,
    ) -> anyhow::Result<Vec<Slot>> {
        let sql = format!(
            "SELECT s.slot_id, s.kind, s.starts_at, s.ends_at, s.capacity,
                    (SELECT COUNT(*) FROM rec_orders o WHERE o.site = s.site AND o.slot_id = s.slot_id AND o.state IN {ACTIVE})
             FROM rec_slots s WHERE s.site = ? AND s.kind = ? AND s.starts_at >= ? AND s.starts_at < ?
             ORDER BY s.starts_at"
        );
        let rows: Vec<(String, String, i64, i64, i64, i64)> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(&site.site)
                .bind(kind)
                .bind(from)
                .bind(to)
                .fetch_all(self.pool())
                .await?;
        Ok(rows
            .into_iter()
            .map(
                |(slot_id, kind, starts_at, ends_at, capacity, taken)| Slot {
                    label: slot_label(starts_at, ends_at, site.utc_offset_min),
                    slot_id,
                    kind,
                    starts_at,
                    ends_at,
                    capacity,
                    taken,
                    full: taken >= capacity,
                },
            )
            .collect())
    }

    /// One slot.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn slot(&self, site: &Site, slot_id: &str) -> anyhow::Result<Option<Slot>> {
        let sql = format!(
            "SELECT s.slot_id, s.kind, s.starts_at, s.ends_at, s.capacity,
                    (SELECT COUNT(*) FROM rec_orders o WHERE o.site = s.site AND o.slot_id = s.slot_id AND o.state IN {ACTIVE})
             FROM rec_slots s WHERE s.site = ? AND s.slot_id = ?"
        );
        let row: Option<(String, String, i64, i64, i64, i64)> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql))
                .bind(&site.site)
                .bind(slot_id)
                .fetch_optional(self.pool())
                .await?;
        Ok(row.map(
            |(slot_id, kind, starts_at, ends_at, capacity, taken)| Slot {
                label: slot_label(starts_at, ends_at, site.utc_offset_min),
                slot_id,
                kind,
                starts_at,
                ends_at,
                capacity,
                taken,
                full: taken >= capacity,
            },
        ))
    }

    /// A cart, or an empty one with that id.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn cart(&self, site: &str, cart_id: &str) -> anyhow::Result<Cart> {
        type CartRow = (Option<String>, String, Option<String>, Option<String>);
        let row: Option<CartRow> = sqlx::query_as(
            "SELECT customer_id, lines, slot_id, zone FROM rec_carts WHERE site = ? AND cart_id = ?",
        )
        .bind(site)
        .bind(cart_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(match row {
            Some((customer_id, lines, slot_id, zone)) => Cart {
                cart_id: cart_id.to_owned(),
                customer_id,
                lines: serde_json::from_str(&lines).unwrap_or_default(),
                slot_id,
                zone,
            },
            None => Cart {
                cart_id: cart_id.to_owned(),
                customer_id: None,
                lines: Vec::new(),
                slot_id: None,
                zone: None,
            },
        })
    }

    /// Saves a cart.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn save_cart(&self, site: &str, cart: &Cart) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_carts (site, cart_id, customer_id, lines, slot_id, zone, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (site, cart_id) DO UPDATE SET customer_id = excluded.customer_id, lines = excluded.lines,
               slot_id = excluded.slot_id, zone = excluded.zone, updated_at = excluded.updated_at",
        )
        .bind(site)
        .bind(&cart.cart_id)
        .bind(&cart.customer_id)
        .bind(serde_json::to_string(&cart.lines)?)
        .bind(&cart.slot_id)
        .bind(&cart.zone)
        .bind(time::now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Adds `qty` of a product to a cart (a quantity of 0 or less removes the line).
    ///
    /// # Errors
    ///
    /// When the product doesn't exist, or on a database error.
    pub async fn add_to_cart(
        &self,
        site: &str,
        cart_id: &str,
        sku: &str,
        qty: i64,
        replace: bool,
    ) -> Result<Cart, OrderError> {
        if self.product(site, sku).await?.is_none() {
            return Err(OrderError::Refused("That item isn't on the menu.".into()));
        }
        let mut cart = self.cart(site, cart_id).await?;
        match cart.lines.iter_mut().find(|l| l.sku == sku) {
            Some(line) => line.qty = if replace { qty } else { line.qty + qty },
            None => cart.lines.push(CartLine {
                sku: sku.to_owned(),
                qty,
                card: None,
            }),
        }
        cart.lines.retain(|l| l.qty > 0);
        for l in &mut cart.lines {
            l.qty = l.qty.min(20);
        }
        self.save_cart(site, &cart).await?;
        Ok(cart)
    }

    /// Prices a cart from the live list, with stock for the slot's day and the zone's fee.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn price_cart(&self, site: &Site, cart: &Cart, now: i64) -> anyhow::Result<CartView> {
        let list = self.live_price_list(&site.site).await?;
        let rate = list.as_ref().map_or(0, |l| l.usd_rate_minor);
        let stock = self.stock(site, now).await?;
        let slot = match &cart.slot_id {
            Some(id) => self.slot(site, id).await?,
            None => None,
        };
        let mut problems = Vec::new();
        let mut lines = Vec::new();
        for line in &cart.lines {
            let Some(p) = self.product(&site.site, &line.sku).await? else {
                continue;
            };
            // Today's stock applies to today; a later slot uses that day's plan.
            let left = match (&slot, site.archetype.as_str()) {
                (Some(s), "bakery")
                    if time::day_of(s.starts_at, site.utc_offset_min)
                        != time::day_of(now, site.utc_offset_min) =>
                {
                    self.left_for_day(site, &p.sku, s.starts_at).await?
                }
                _ => stock.get(&p.sku).and_then(|s| s.left),
            };
            let available = left.is_none_or(|n| n >= line.qty);
            if !available {
                problems.push(match left {
                    Some(0) => format!("{} is sold out for that day.", p.name),
                    Some(n) => format!("Only {n} {} left for that day.", p.name.to_lowercase()),
                    None => String::new(),
                });
            }
            lines.push(PricedLine {
                sku: p.sku.clone(),
                slug: p.slug.clone(),
                name: p.name.clone(),
                qty: line.qty,
                unit_minor: p.price_minor,
                amount_minor: p.price_minor * line.qty,
                left,
                available,
                card: line.card.clone(),
            });
        }
        let zone = match &cart.zone {
            Some(z) => self
                .zones(&site.site)
                .await?
                .into_iter()
                .find(|x| &x.zone == z),
            None => None,
        };
        let delivery_minor = zone.as_ref().map_or(0, |z| z.fee_minor);
        let subtotal_minor: i64 = lines.iter().map(|l| l.amount_minor).sum();
        let total_minor = subtotal_minor + delivery_minor;
        if let Some(s) = &slot
            && s.full
        {
            problems.push(format!("{} is full. Pick another time.", s.label));
        }
        Ok(CartView {
            items: lines.iter().map(|l| l.qty).sum(),
            usd_cents: money::usd_cents(total_minor, rate),
            price_list: list.as_ref().map(|l| l.version.clone()),
            usd_rate_minor: rate,
            valid_until: list.as_ref().map(|l| l.valid_until),
            cart: cart.clone(),
            lines,
            subtotal_minor,
            delivery_minor,
            total_minor,
            problems,
        })
    }

    /// Units of a product left on another day: that day's plan less its active orders.
    async fn left_for_day(&self, site: &Site, sku: &str, ts: i64) -> anyhow::Result<Option<i64>> {
        let planned: Option<(i64,)> =
            sqlx::query_as("SELECT planned FROM rec_bake_plan WHERE site = ? AND sku = ?")
                .bind(&site.site)
                .bind(sku)
                .fetch_optional(self.pool())
                .await?;
        let Some((planned,)) = planned else {
            return Ok(None);
        };
        let day = time::day_of(ts, site.utc_offset_min);
        let off = site.utc_offset_min * 60;
        let sql = format!(
            "SELECT COALESCE(SUM(l.qty), 0) FROM rec_order_lines l
             JOIN rec_orders o ON o.site = l.site AND o.order_id = l.order_id
             LEFT JOIN rec_slots s ON s.site = o.site AND s.slot_id = o.slot_id
             WHERE l.site = ? AND l.sku = ? AND o.state IN {ACTIVE}
               AND date(COALESCE(s.starts_at, o.created_at) + ?, 'unixepoch') = ?"
        );
        let (taken,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(&site.site)
            .bind(sku)
            .bind(off)
            .bind(&day)
            .fetch_one(self.pool())
            .await?;
        Ok(Some((planned - taken).max(0)))
    }

    async fn order_number(&self, site: &Site) -> anyhow::Result<String> {
        let prefix = site.fact("/order_prefix");
        let prefix = if prefix.is_empty() {
            "ORD".to_owned()
        } else {
            prefix
        };
        let n = self.next_number(&site.site, "order", 1001).await?;
        Ok(format!("{prefix}-{n:04}"))
    }

    async fn add_event(
        &self,
        site: &str,
        order_id: &str,
        step: &str,
        at: i64,
        actor: &str,
        note: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_order_events (site, order_id, seq, step, at, actor, note)
             VALUES (?, ?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM rec_order_events WHERE site = ? AND order_id = ?), ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(order_id)
        .bind(site)
        .bind(order_id)
        .bind(step)
        .bind(at)
        .bind(actor)
        .bind(note)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Places a catalogue order from a cart, re-checking stock and the slot at submit. The cart
    /// empties. The order waits for payment.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] with visitor words when the cart is empty, an item ran out or the
    /// slot filled since the page loaded.
    pub async fn place_order(
        &self,
        site: &Site,
        cart_id: &str,
        contact: &Contact,
        how: &Placement,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        if let Some(key) = &how.idempotency_key
            && let Some(existing) = self.order_by_key(&site.site, key).await?
        {
            return Ok(existing);
        }
        let mut cart = self.cart(&site.site, cart_id).await?;
        cart.slot_id = how.slot_id.clone().or(cart.slot_id);
        cart.zone = if how.fulfilment == "delivery" {
            how.zone.clone().or(cart.zone)
        } else {
            None
        };
        let view = self.price_cart(site, &cart, now).await?;
        if view.lines.is_empty() {
            return Err(OrderError::Refused("Your bag is empty.".into()));
        }
        if let Some(p) = view.problems.first() {
            return Err(OrderError::Refused(p.clone()));
        }
        if site.archetype == "bakery" && cart.slot_id.is_none() {
            return Err(OrderError::Refused(
                "Pick a delivery or collection time.".into(),
            ));
        }
        if how.fulfilment == "delivery" && cart.zone.is_none() {
            return Err(OrderError::Refused("Pick where to deliver.".into()));
        }
        let order_id = self.order_number(site).await?;
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "INSERT INTO rec_orders (site, order_id, customer_id, contact_name, contact_email, contact_phone, channel, kind, state,
               subtotal_minor, delivery_minor, total_minor, currency, usd_rate_minor, slot_id, zone, fulfilment, recipient, note,
               spec, price_list, quote_valid_until, agent, idempotency_key, agent_id, agent_vouched_by, agent_proof_method, sample, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'catalogue', 'awaiting_payment', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&site.site)
        .bind(&order_id)
        .bind(&how.customer_id)
        .bind(&contact.name)
        .bind(&contact.email)
        .bind(&contact.phone)
        .bind(&how.channel)
        .bind(view.subtotal_minor)
        .bind(view.delivery_minor)
        .bind(view.total_minor)
        .bind(&site.currency)
        .bind(view.usd_rate_minor)
        .bind(&cart.slot_id)
        .bind(&cart.zone)
        .bind(&how.fulfilment)
        .bind(&contact.recipient)
        .bind(&how.note)
        .bind(&view.price_list)
        .bind(view.valid_until)
        .bind(&how.agent)
        .bind(&how.idempotency_key)
        .bind(how.proof.as_ref().map(|p| p.id.clone()))
        .bind(how.proof.as_ref().map(|p| p.vouched_by.clone()))
        .bind(how.proof.as_ref().map(|p| p.method.clone()))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let mut n = 0;
        for l in &view.lines {
            n += 1;
            sqlx::query(
                "INSERT INTO rec_order_lines (site, order_id, line_no, sku, name, qty, unit_minor, rule_code, amount_minor)
                 VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            )
            .bind(&site.site)
            .bind(&order_id)
            .bind(n)
            .bind(&l.sku)
            .bind(match &l.card {
                Some(c) => format!("{} (card: \u{201c}{c}\u{201d})", l.name),
                None => l.name.clone(),
            })
            .bind(l.qty)
            .bind(l.unit_minor)
            .bind(l.amount_minor)
            .execute(&mut *tx)
            .await?;
        }
        if view.delivery_minor > 0 {
            n += 1;
            sqlx::query(
                "INSERT INTO rec_order_lines (site, order_id, line_no, sku, name, qty, unit_minor, rule_code, amount_minor)
                 VALUES (?, ?, ?, NULL, 'Delivery', 1, ?, ?, ?)",
            )
            .bind(&site.site)
            .bind(&order_id)
            .bind(n)
            .bind(view.delivery_minor)
            .bind(cart.zone.as_ref().map(|z| format!("delivery.{z}")))
            .bind(view.delivery_minor)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM rec_carts WHERE site = ? AND cart_id = ?")
            .bind(&site.site)
            .bind(cart_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.add_event(
            &site.site,
            &order_id,
            "requested",
            now,
            &contact.name,
            Some("Order placed"),
        )
        .await?;
        self.order(site, &order_id)
            .await?
            .ok_or_else(|| OrderError::Other(anyhow::anyhow!("order vanished")))
    }

    /// Requests a custom cake: the order is held for the owner. Nothing is charged until the
    /// owner approves and the customer pays.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the design is incomplete or breaks a rule.
    pub async fn request_custom(
        &self,
        site: &Site,
        choice: &CakeChoice,
        quote: &CakeQuote,
        contact: &Contact,
        how: &Placement,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        if let Some(key) = &how.idempotency_key
            && let Some(existing) = self.order_by_key(&site.site, key).await?
        {
            return Ok(existing);
        }
        if !quote.complete {
            let why = quote.flags.iter().find(|f| f.level == "error").map_or_else(
                || "Finish the design first: every choice and a day.".to_owned(),
                |f| f.text.clone(),
            );
            return Err(OrderError::Refused(why));
        }
        if contact.name.trim().is_empty() {
            return Err(OrderError::Refused("Add the name for the order.".into()));
        }
        let zone = if how.fulfilment == "delivery" {
            match &how.zone {
                Some(z) => self
                    .zones(&site.site)
                    .await?
                    .into_iter()
                    .find(|x| &x.zone == z),
                None => return Err(OrderError::Refused("Pick where to deliver.".into())),
            }
        } else {
            None
        };
        let delivery_minor = zone.as_ref().map_or(0, |z| z.fee_minor);
        let total_minor = quote.total_minor + delivery_minor;
        let order_id = self.order_number(site).await?;
        let spec = serde_json::to_string(choice).map_err(anyhow::Error::from)?;
        sqlx::query(
            "INSERT INTO rec_orders (site, order_id, customer_id, contact_name, contact_email, contact_phone, channel, kind, state,
               subtotal_minor, delivery_minor, total_minor, currency, usd_rate_minor, slot_id, zone, fulfilment, recipient, note,
               spec, price_list, quote_valid_until, agent, idempotency_key, agent_id, agent_vouched_by, agent_proof_method, sample, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'custom', 'held', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&site.site)
        .bind(&order_id)
        .bind(&how.customer_id)
        .bind(&contact.name)
        .bind(&contact.email)
        .bind(&contact.phone)
        .bind(&how.channel)
        .bind(quote.total_minor)
        .bind(delivery_minor)
        .bind(total_minor)
        .bind(&site.currency)
        .bind(quote.usd_rate_minor)
        .bind(&how.slot_id)
        .bind(zone.as_ref().map(|z| z.zone.clone()))
        .bind(&how.fulfilment)
        .bind(&contact.recipient)
        .bind(if choice.note.trim().is_empty() { how.note.clone() } else { Some(choice.note.trim().to_owned()) })
        .bind(&spec)
        .bind(&quote.price_list)
        .bind(quote.valid_until)
        .bind(&how.agent)
        .bind(&how.idempotency_key)
        .bind(how.proof.as_ref().map(|p| p.id.clone()))
        .bind(how.proof.as_ref().map(|p| p.vouched_by.clone()))
        .bind(how.proof.as_ref().map(|p| p.method.clone()))
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;
        let mut n = 0;
        for l in &quote.lines {
            n += 1;
            sqlx::query(
                "INSERT INTO rec_order_lines (site, order_id, line_no, sku, name, qty, unit_minor, rule_code, amount_minor)
                 VALUES (?, ?, ?, NULL, ?, 1, ?, ?, ?)",
            )
            .bind(&site.site)
            .bind(&order_id)
            .bind(n)
            .bind(&l.label)
            .bind(l.amount_minor)
            .bind(&l.code)
            .bind(l.amount_minor)
            .execute(self.pool())
            .await?;
        }
        if let Some(z) = &zone {
            sqlx::query(
                "INSERT INTO rec_order_lines (site, order_id, line_no, sku, name, qty, unit_minor, rule_code, amount_minor)
                 VALUES (?, ?, ?, NULL, 'Delivery', 1, ?, ?, ?)",
            )
            .bind(&site.site)
            .bind(&order_id)
            .bind(n + 1)
            .bind(z.fee_minor)
            .bind(format!("delivery.{}", z.zone))
            .bind(z.fee_minor)
            .execute(self.pool())
            .await?;
        }
        let actor = match &how.agent {
            Some(agent) => format!("{} through the agent door", agent),
            None => contact.name.clone(),
        };
        self.add_event(
            &site.site,
            &order_id,
            "requested",
            now,
            &actor,
            Some("Custom cake requested. Held for the owner; nothing charged."),
        )
        .await?;
        self.order(site, &order_id)
            .await?
            .ok_or_else(|| OrderError::Other(anyhow::anyhow!("order vanished")))
    }

    async fn order_by_key(&self, site: &str, key: &str) -> anyhow::Result<Option<OrderView>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT order_id FROM rec_orders WHERE site = ? AND idempotency_key = ?",
        )
        .bind(site)
        .bind(key)
        .fetch_optional(self.pool())
        .await?;
        match row {
            Some((id,)) => {
                let s = self
                    .site(site)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no site"))?;
                self.order(&s, &id).await
            }
            None => Ok(None),
        }
    }

    /// One order with its lines, events, payments, refunds and slot.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn order(&self, site: &Site, order_id: &str) -> anyhow::Result<Option<OrderView>> {
        let order: Option<Order> = sqlx::query_as(sql!(
            "SELECT {ORDER_COLS} FROM rec_orders WHERE site = ? AND order_id = ?"
        ))
        .bind(&site.site)
        .bind(order_id.trim().to_uppercase())
        .fetch_optional(self.pool())
        .await?;
        let Some(order) = order else { return Ok(None) };
        self.order_view(site, order).await.map(Some)
    }

    async fn order_view(&self, site: &Site, order: Order) -> anyhow::Result<OrderView> {
        let lines = sqlx::query_as::<_, OrderLine>(
            "SELECT sku, name, qty, unit_minor, rule_code, amount_minor FROM rec_order_lines WHERE site = ? AND order_id = ? ORDER BY line_no",
        )
        .bind(&site.site)
        .bind(&order.order_id)
        .fetch_all(self.pool())
        .await?;
        let events = sqlx::query_as::<_, OrderEvent>(
            "SELECT seq, step, at, actor, note FROM rec_order_events WHERE site = ? AND order_id = ? ORDER BY seq",
        )
        .bind(&site.site)
        .bind(&order.order_id)
        .fetch_all(self.pool())
        .await?;
        let payments = sqlx::query_as::<_, Payment>(
            "SELECT payment_id, rail, provider_ref, amount_minor, currency, state, created_at, settled_at FROM rec_payments
             WHERE site = ? AND order_id = ? ORDER BY created_at DESC",
        )
        .bind(&site.site)
        .bind(&order.order_id)
        .fetch_all(self.pool())
        .await?;
        let refunds = sqlx::query_as::<_, Refund>(
            "SELECT refund_id, order_id, amount_minor, reason, rule, state, created_at FROM rec_refunds WHERE site = ? AND order_id = ? ORDER BY created_at",
        )
        .bind(&site.site)
        .bind(&order.order_id)
        .fetch_all(self.pool())
        .await?;
        let slot = match &order.slot_id {
            Some(id) => self.slot(site, id).await?,
            None => None,
        };
        Ok(OrderView {
            order,
            lines,
            events,
            payments,
            refunds,
            slot,
        })
    }

    /// A customer's orders, newest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn orders_for_customer(
        &self,
        site: &Site,
        customer_id: &str,
    ) -> anyhow::Result<Vec<OrderView>> {
        let orders: Vec<Order> = sqlx::query_as(sql!(
            "SELECT {ORDER_COLS} FROM rec_orders WHERE site = ? AND customer_id = ? ORDER BY created_at DESC"
        ))
        .bind(&site.site)
        .bind(customer_id)
        .fetch_all(self.pool())
        .await?;
        let mut out = Vec::new();
        for o in orders {
            out.push(self.order_view(site, o).await?);
        }
        Ok(out)
    }

    /// Orders for the owner, newest first, optionally of one kind and in some states.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn orders_for_owner(
        &self,
        site: &Site,
        kind: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<OrderView>> {
        let orders: Vec<Order> = sqlx::query_as(sql!(
            "SELECT {ORDER_COLS} FROM rec_orders WHERE site = ? AND (? IS NULL OR kind = ?) ORDER BY created_at DESC LIMIT ?"
        ))
        .bind(&site.site)
        .bind(kind)
        .bind(kind)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        let mut out = Vec::new();
        for o in orders {
            out.push(self.order_view(site, o).await?);
        }
        Ok(out)
    }

    async fn set_state(
        &self,
        site: &str,
        order_id: &str,
        from: &[&str],
        to: &str,
        now: i64,
    ) -> Result<(), OrderError> {
        let current: Option<(String,)> =
            sqlx::query_as("SELECT state FROM rec_orders WHERE site = ? AND order_id = ?")
                .bind(site)
                .bind(order_id)
                .fetch_optional(self.pool())
                .await?;
        let Some((state,)) = current else {
            return Err(OrderError::Refused(
                "There's no order with that number.".into(),
            ));
        };
        if !from.contains(&state.as_str()) {
            return Err(OrderError::Refused(format!(
                "{order_id} is {} now, so that step doesn't apply.",
                state_words(&state)
            )));
        }
        sqlx::query(
            "UPDATE rec_orders SET state = ?, updated_at = ? WHERE site = ? AND order_id = ?",
        )
        .bind(to)
        .bind(now)
        .bind(site)
        .bind(order_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The owner decides a held custom order.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the order isn't held.
    pub async fn decide(
        &self,
        site: &Site,
        order_id: &str,
        approve: bool,
        actor: &str,
        note: Option<&str>,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        let (to, step, words) = if approve {
            (
                "approved",
                "approved",
                "Approved. The payment request is on the order page.",
            )
        } else {
            ("declined", "declined", "Declined. Nothing was charged.")
        };
        self.set_state(&site.site, order_id, &["held"], to, now)
            .await?;
        self.add_event(
            &site.site,
            order_id,
            step,
            now,
            actor,
            Some(note.filter(|n| !n.trim().is_empty()).unwrap_or(words)),
        )
        .await?;
        self.order(site, order_id)
            .await?
            .ok_or_else(|| OrderError::Refused("There's no order with that number.".into()))
    }

    /// The owner moves an order along: `baking`, `ready`, `out`, `delivered`, `collected`, or
    /// cancels it.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the step doesn't follow from the current state.
    pub async fn advance(
        &self,
        site: &Site,
        order_id: &str,
        step: &str,
        actor: &str,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        let (from, words): (&[&str], &str) = match step {
            "baking" => (&["paid"], "Baking now."),
            "ready" => (&["paid", "baking"], "Ready to collect."),
            "out" => (&["paid", "baking", "ready"], "Out for delivery."),
            "delivered" => (&["out", "ready"], "Delivered."),
            "collected" => (&["ready"], "Collected."),
            "cancelled" => (
                &["held", "approved", "awaiting_payment"],
                "Cancelled. Nothing was charged.",
            ),
            "lapsed" => (
                &["held", "approved", "awaiting_payment"],
                "The request lapsed. Nothing was charged.",
            ),
            _ => {
                return Err(OrderError::Refused(
                    "That isn't a step an order takes.".into(),
                ));
            }
        };
        self.set_state(&site.site, order_id, from, step, now)
            .await?;
        self.add_event(&site.site, order_id, step, now, actor, Some(words))
            .await?;
        self.order(site, order_id)
            .await?
            .ok_or_else(|| OrderError::Refused("There's no order with that number.".into()))
    }

    /// Orders the shop still has to fulfil (`paid`, `baking`, `ready`, `out`), oldest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn orders_in_fulfilment(
        &self,
        site: &Site,
        limit: i64,
    ) -> anyhow::Result<Vec<OrderView>> {
        let orders: Vec<Order> = sqlx::query_as(sql!(
            "SELECT {ORDER_COLS} FROM rec_orders WHERE site = ? AND state IN ('paid', 'baking', 'ready', 'out') ORDER BY created_at LIMIT ?"
        ))
        .bind(&site.site)
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        let mut out = Vec::new();
        for o in orders {
            out.push(self.order_view(site, o).await?);
        }
        Ok(out)
    }

    /// The shop booked the delivery of a ready order with `courier` (its host), which issued
    /// `tracking`: the order is out, and the timeline says who booked it.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the order isn't ready for delivery.
    pub async fn book_courier(
        &self,
        site: &Site,
        order_id: &str,
        courier: &str,
        tracking: &str,
        actor: &str,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        self.set_state(&site.site, order_id, &["ready"], "out", now)
            .await?;
        sqlx::query(
            "UPDATE rec_orders SET courier = ?, tracking = ? WHERE site = ? AND order_id = ?",
        )
        .bind(courier)
        .bind(tracking)
        .bind(&site.site)
        .bind(order_id)
        .execute(self.pool())
        .await?;
        let note = format!("Booked with {courier} for delivery, tracking {tracking}.");
        self.add_event(&site.site, order_id, "out", now, actor, Some(&note))
            .await?;
        self.order(site, order_id)
            .await?
            .ok_or_else(|| OrderError::Refused("There's no order with that number.".into()))
    }

    /// A refund after the sale (a Support desk's decision, the PayPal refund already made):
    /// `refund` is recorded and the timeline says `note`. A full refund ends the order as
    /// `refunded`; a part refund leaves its state as it is.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the order hasn't been paid, or is refunded already.
    pub async fn refund_after_sale(
        &self,
        site: &Site,
        refund: &Refund,
        full: bool,
        actor: &str,
        note: &str,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        const SOLD: &[&str] = &["paid", "baking", "ready", "out", "delivered", "collected"];
        if full {
            self.set_state(&site.site, &refund.order_id, SOLD, "refunded", now)
                .await?;
        } else {
            let state: Option<(String,)> =
                sqlx::query_as("SELECT state FROM rec_orders WHERE site = ? AND order_id = ?")
                    .bind(&site.site)
                    .bind(&refund.order_id)
                    .fetch_optional(self.pool())
                    .await?;
            match state {
                Some((s,)) if SOLD.contains(&s.as_str()) => {}
                Some((s,)) => {
                    return Err(OrderError::Refused(format!(
                        "{} is {} now, so there's nothing to refund.",
                        refund.order_id,
                        state_words(&s)
                    )));
                }
                None => {
                    return Err(OrderError::Refused(
                        "There's no order with that number.".into(),
                    ));
                }
            }
        }
        self.record_refund(&site.site, refund).await?;
        let step = if full { "refunded" } else { "part_refunded" };
        self.add_event(&site.site, &refund.order_id, step, now, actor, Some(note))
            .await?;
        self.order(site, &refund.order_id)
            .await?
            .ok_or_else(|| OrderError::Refused("There's no order with that number.".into()))
    }

    /// The owner agrees to cancel a paid order before it is baked: the order ends as refunded,
    /// with `refund` (the whole amount left, back the way it was paid) and a timeline line.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the order isn't paid and waiting any more (baking, out …).
    pub async fn cancel_paid(
        &self,
        site: &Site,
        refund: &Refund,
        actor: &str,
        note: &str,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        self.set_state(&site.site, &refund.order_id, &["paid"], "refunded", now)
            .await?;
        self.record_refund(&site.site, refund).await?;
        self.add_event(
            &site.site,
            &refund.order_id,
            "cancel_refunded",
            now,
            actor,
            Some(note),
        )
        .await?;
        self.order(site, &refund.order_id)
            .await?
            .ok_or_else(|| OrderError::Refused("There's no order with that number.".into()))
    }

    /// Records a payment attempt (started, captured, declined …).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn record_payment(
        &self,
        site: &str,
        order_id: &str,
        p: &Payment,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_payments (site, payment_id, order_id, rail, provider_ref, amount_minor, currency, state, created_at, settled_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (site, payment_id) DO UPDATE SET state = excluded.state, settled_at = excluded.settled_at, provider_ref = excluded.provider_ref",
        )
        .bind(site)
        .bind(&p.payment_id)
        .bind(order_id)
        .bind(&p.rail)
        .bind(&p.provider_ref)
        .bind(p.amount_minor)
        .bind(&p.currency)
        .bind(&p.state)
        .bind(p.created_at)
        .bind(p.settled_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Marks an order paid after a captured payment.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the order isn't waiting for payment.
    pub async fn mark_paid(
        &self,
        site: &Site,
        order_id: &str,
        words: &str,
        now: i64,
    ) -> Result<OrderView, OrderError> {
        self.set_state(
            &site.site,
            order_id,
            &["awaiting_payment", "approved"],
            "paid",
            now,
        )
        .await?;
        self.add_event(
            &site.site,
            order_id,
            "paid",
            now,
            "the Cashier",
            Some(words),
        )
        .await?;
        self.order(site, order_id)
            .await?
            .ok_or_else(|| OrderError::Refused("There's no order with that number.".into()))
    }

    /// Notes a change request on an order (held for the owner).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn note_event(
        &self,
        site: &str,
        order_id: &str,
        step: &str,
        actor: &str,
        note: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        self.add_event(site, order_id, step, now, actor, Some(note))
            .await
    }

    /// A hold under the standing grant: up to `max` units until a clock time today, no charge.
    ///
    /// # Errors
    ///
    /// [`OrderError::Refused`] when the grant doesn't cover it or there isn't enough left.
    pub async fn create_hold(
        &self,
        site: &Site,
        sku: &str,
        qty: i64,
        name: &str,
        now: i64,
    ) -> Result<Hold, OrderError> {
        let grant = site.facts.get("hold_grant").cloned().unwrap_or(Value::Null);
        let max = grant.get("max").and_then(Value::as_i64).unwrap_or(0);
        let until = grant
            .get("until")
            .and_then(Value::as_str)
            .unwrap_or("12:00");
        let grant_id = grant
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("g-2")
            .to_owned();
        let owner = site.fact("/owner/first");
        if qty < 1 || qty > max {
            return Err(OrderError::Refused(format!(
                "The site can hold up to {max} without asking. For more, {owner} decides."
            )));
        }
        let today = time::local(now, site.utc_offset_min).date_naive();
        let until_at = time::at_local(today, until, site.utc_offset_min)?;
        if now >= until_at {
            return Err(OrderError::Refused(format!(
                "Holds run until {until} each day. Come by, or order for tomorrow."
            )));
        }
        let stock = self.stock(site, now).await?;
        let left = stock.get(sku).and_then(|s| s.left).unwrap_or(0);
        if left < qty {
            return Err(OrderError::Refused(format!("Only {left} left today.")));
        }
        let n = self.next_number(&site.site, "hold", 194).await?;
        let hold = Hold {
            hold_id: format!("H-{n:04}"),
            sku: sku.to_owned(),
            qty,
            day: time::day_of(now, site.utc_offset_min),
            until_at,
            name: name.to_owned(),
            grant_id,
            state: "held".into(),
            created_at: now,
        };
        sqlx::query(
            "INSERT INTO rec_holds (site, hold_id, sku, qty, day, until_at, name, grant_id, state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&site.site)
        .bind(&hold.hold_id)
        .bind(&hold.sku)
        .bind(hold.qty)
        .bind(&hold.day)
        .bind(hold.until_at)
        .bind(&hold.name)
        .bind(&hold.grant_id)
        .bind(&hold.state)
        .bind(hold.created_at)
        .execute(self.pool())
        .await?;
        Ok(hold)
    }

    /// Today's holds.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn holds(&self, site: &Site, now: i64) -> anyhow::Result<Vec<Hold>> {
        Ok(sqlx::query_as::<_, Hold>(
            "SELECT hold_id, sku, qty, day, until_at, name, grant_id, state, created_at FROM rec_holds WHERE site = ? AND day = ? ORDER BY created_at",
        )
        .bind(&site.site)
        .bind(time::day_of(now, site.utc_offset_min))
        .fetch_all(self.pool())
        .await?)
    }

    /// Bookings, soonest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn bookings(&self, site: &str) -> anyhow::Result<Vec<Booking>> {
        Ok(sqlx::query_as::<_, Booking>(
            "SELECT booking_id, service, starts_at, customer_id, name, seats, state, created_at FROM rec_bookings WHERE site = ? ORDER BY starts_at",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// Records a refund.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn record_refund(&self, site: &str, r: &Refund) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_refunds (site, refund_id, order_id, amount_minor, reason, rule, state, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(&r.refund_id)
        .bind(&r.order_id)
        .bind(r.amount_minor)
        .bind(&r.reason)
        .bind(&r.rule)
        .bind(&r.state)
        .bind(r.created_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Lapses held or unpaid orders older than `hours`, and returns the ones it lapsed.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn lapse_stale(
        &self,
        site: &Site,
        hours: i64,
        now: i64,
    ) -> anyhow::Result<Vec<OrderView>> {
        let stale: Vec<(String,)> = sqlx::query_as(
            "SELECT order_id FROM rec_orders WHERE site = ? AND state IN ('held', 'awaiting_payment') AND sample = 0 AND created_at < ?",
        )
        .bind(&site.site)
        .bind(now - hours * 3600)
        .fetch_all(self.pool())
        .await?;
        let mut lapsed = Vec::new();
        for (id,) in &stale {
            if let Ok(view) = self
                .advance(site, id, "lapsed", "the Orders desk", now)
                .await
            {
                lapsed.push(view);
            }
        }
        Ok(lapsed)
    }

    /// A new payment id.
    pub fn new_payment_id() -> String {
        new_id("pay")
    }
}

/// An order state in visitor words.
pub fn state_words(state: &str) -> &'static str {
    match state {
        "awaiting_payment" => "waiting for payment",
        "held" => "waiting for the owner's OK",
        "approved" => "approved and waiting for payment",
        "paid" => "paid",
        "baking" => "being baked",
        "ready" => "ready to collect",
        "out" => "out for delivery",
        "delivered" => "delivered",
        "collected" => "collected",
        "declined" => "declined",
        "lapsed" => "lapsed",
        "cancelled" => "cancelled",
        "refunded" => "refunded",
        _ => "in progress",
    }
}
