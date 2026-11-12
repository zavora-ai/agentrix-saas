//! After the sale (v4, "After the sale"): the shop's own agents move each paid order on in its
//! slot, and every change to an order goes through one function, [`transition`], which checks
//! who may make it, writes the timeline step (its time and who did it) and tells the customer's
//! agent through the order notices.
//!
//! - **The shop's pass** ([`pass`], run on a schedule by [`follow`]): `paid` → `baking` →
//!   `ready`, then `collected` for a collection order. Each step is due at a time that fits the
//!   order's slot, and steps are spaced so a late order doesn't jump through them at once.
//! - **Delivery is bought from a courier** (company's courier business): at `ready` the shop's
//!   agent books it at the courier's agent door and the order goes `out` with the courier's
//!   tracking number ([`Change::Booked`]). Until that door is connected, a delivery order waits
//!   at `ready`.
//! - **The customer's side** (settle's door intents) calls [`transition`] to confirm delivery,
//!   cancel, or record a refund it has made. This never moves money.

use std::time::Duration;

use agentrix_records::orders::{OrderError, OrderView, Refund};
use agentrix_records::{Site, money, time};
use serde_json::{Value, json};

use crate::App;
use crate::notify::status_label;

/// Who makes a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// One of the site's own agents (its Orders desk).
    Shop {
        /// Its name.
        agent: String,
    },
    /// The customer's agent, as the door proved it.
    Customer {
        /// Its verified id.
        agent_id: String,
        /// Who vouched for it.
        vouched_by: String,
        /// How it proved itself: `web-bot-auth` or `token`.
        method: String,
    },
    /// The site's owner.
    Owner {
        /// Their first name.
        name: String,
    },
}

impl Actor {
    /// How the timeline names it.
    pub fn words(&self) -> String {
        match self {
            Actor::Shop { agent } => agent.clone(),
            Actor::Customer {
                agent_id,
                vouched_by,
                ..
            } => format!("the customer's agent {agent_id} (vouched for by {vouched_by})"),
            Actor::Owner { name } => name.clone(),
        }
    }
}

/// A change to an order.
#[derive(Debug, Clone)]
pub enum Change {
    /// The shop starts making it (`paid` → `baking`).
    Baking,
    /// It's made (`paid` | `baking` → `ready`).
    Ready,
    /// A collection order was collected (`ready` → `collected`).
    Collected,
    /// The shop booked the delivery with `courier` (its host), which issued `tracking`
    /// (`ready` → `out`).
    Booked {
        /// The courier's host.
        courier: String,
        /// Its tracking number.
        tracking: String,
    },
    /// The customer's agent confirms the delivery (`out` → `delivered`).
    ConfirmDelivered,
    /// An unpaid order is cancelled (`held` | `approved` | `awaiting_payment` → `cancelled`).
    Cancel {
        /// Why, for the timeline.
        note: String,
    },
    /// A paid order is cancelled before it is made, `refund` already made (`paid` → `refunded`).
    CancelPaid {
        /// The refund made.
        refund: Refund,
        /// Why, for the timeline.
        note: String,
    },
    /// A refund after the sale, already made: in full (→ `refunded`) or in part (state kept).
    Refunded {
        /// The refund made.
        refund: Refund,
        /// Why, for the timeline.
        note: String,
        /// The whole order.
        full: bool,
    },
}

fn refused(words: impl Into<String>) -> OrderError {
    OrderError::Refused(words.into())
}

/// Whether `by` may make `change`.
fn allowed(change: &Change, by: &Actor) -> Result<(), OrderError> {
    let ok = match change {
        Change::Baking | Change::Ready | Change::Collected | Change::Booked { .. } => {
            matches!(by, Actor::Shop { .. } | Actor::Owner { .. })
        }
        Change::ConfirmDelivered | Change::Cancel { .. } => {
            matches!(by, Actor::Customer { .. } | Actor::Owner { .. })
        }
        Change::CancelPaid { .. } | Change::Refunded { .. } => {
            matches!(by, Actor::Shop { .. } | Actor::Owner { .. })
        }
    };
    if ok {
        Ok(())
    } else {
        Err(refused(match change {
            Change::ConfirmDelivered => "Only the customer's agent confirms a delivery.",
            Change::Cancel { .. } => "Only the customer, or the owner, cancels an order.",
            Change::CancelPaid { .. } | Change::Refunded { .. } => {
                "Only the shop decides a refund."
            }
            _ => "Only the shop moves an order on.",
        }))
    }
}

/// Makes `change` to `order_id` at `site` as `by`: checks who may, writes the timeline step,
/// and tells the customer's agent. Never moves money: a refund is made before it is recorded
/// here.
///
/// # Errors
///
/// [`OrderError::Refused`], in words for the door, when `by` may not make the change or the
/// order isn't at a state it applies to.
pub async fn transition(
    app: &App,
    site: &Site,
    order_id: &str,
    change: Change,
    by: &Actor,
) -> Result<OrderView, OrderError> {
    allowed(&change, by)?;
    let records = app.records().await.map_err(OrderError::Other)?;
    let id = order_id.trim().to_uppercase();
    let now = time::now();
    let who = by.words();
    let (view, note) = match &change {
        Change::Baking => (records.advance(site, &id, "baking", &who, now).await?, None),
        Change::Ready => (records.advance(site, &id, "ready", &who, now).await?, None),
        Change::Collected => (
            records.advance(site, &id, "collected", &who, now).await?,
            None,
        ),
        Change::Booked { courier, tracking } => (
            records
                .book_courier(site, &id, courier, tracking, &who, now)
                .await?,
            Some(format!("{courier}, tracking {tracking}")),
        ),
        Change::ConfirmDelivered => {
            let v = records
                .order(site, &id)
                .await
                .map_err(OrderError::Other)?
                .ok_or_else(|| refused("There's no order with that number."))?;
            if v.order.state != "out" {
                return Err(refused(format!(
                    "{id} isn't out for delivery, so there's no delivery to confirm."
                )));
            }
            (
                records.advance(site, &id, "delivered", &who, now).await?,
                None,
            )
        }
        Change::Cancel { note } => {
            let mut v = records.advance(site, &id, "cancelled", &who, now).await?;
            if !note.trim().is_empty() {
                records
                    .note_event(&site.site, &id, "cancel_reason", &who, note.trim(), now)
                    .await
                    .map_err(OrderError::Other)?;
                v = records
                    .order(site, &id)
                    .await
                    .map_err(OrderError::Other)?
                    .unwrap_or(v);
            }
            (v, Some(note.clone()).filter(|n| !n.trim().is_empty()))
        }
        Change::CancelPaid { refund, note } => (
            records.cancel_paid(site, refund, &who, note, now).await?,
            Some(note.clone()),
        ),
        Change::Refunded { refund, note, full } => (
            records
                .refund_after_sale(site, refund, *full, &who, note, now)
                .await?,
            Some(note.clone()),
        ),
    };
    tell(app, site, &view, &change, &who, note).await;
    Ok(view)
}

/// Tells the customer's agent what changed, through the order notices.
async fn tell(
    app: &App,
    site: &Site,
    view: &OrderView,
    change: &Change,
    who: &str,
    note: Option<String>,
) {
    use agentrix_messages::orders::OrderEvent;
    let (host, origin) = app.site_origin(site);
    let o = &view.order;
    let event = match change {
        Change::CancelPaid { refund, .. } | Change::Refunded { refund, .. } => {
            OrderEvent::Refunded {
                refund_id: refund.refund_id.clone(),
                amount: money::fmt(&o.currency, refund.amount_minor),
                local: None,
                by: who.to_owned(),
                what: if matches!(change, Change::Refunded { full: false, .. }) {
                    "part of the order".into()
                } else {
                    "the whole order".into()
                },
            }
        }
        _ => {
            let Some(label) = status_label(&o.state, &o.fulfilment) else {
                return;
            };
            OrderEvent::Status {
                label,
                by: who.to_owned(),
                at: time::fmt_clock(time::now(), site.utc_offset_min),
                note,
            }
        }
    };
    app.tell_customer(site, &host, &origin, view, event).await;
}

// ---- the shop's pass ----------------------------------------------------------------------

/// The site's Orders desk, which moves its orders on: the desk named `orders` in the site's
/// manifest, or "the Orders desk".
pub fn orders_agent(site: &Site) -> String {
    crate::shell::desks(site, &Default::default())
        .into_iter()
        .find(|d| d.id == "orders")
        .map_or_else(|| "the Orders desk".to_owned(), |d| d.name)
}

/// How long before the slot baking starts, how long before it the order is ready, and the
/// least time between two steps.
const BAKE_LEAD: i64 = 75 * 60;
const READY_LEAD: i64 = 15 * 60;
const GAP: i64 = 2 * 60;

/// The next step the shop takes for `v`, and when it is due; `None` when the shop waits (for the
/// courier, or the customer).
pub fn next_step(v: &OrderView) -> Option<(Change, i64)> {
    let o = &v.order;
    let last = v.events.iter().map(|e| e.at).max().unwrap_or(o.updated_at);
    let paid = v
        .events
        .iter()
        .rev()
        .find(|e| e.step == "paid")
        .map_or(o.updated_at, |e| e.at);
    // Without a slot, an hour after payment stands in for one.
    let (start, end) = v
        .slot
        .as_ref()
        .map_or((paid + 3600, paid + 7200), |s| (s.starts_at, s.ends_at));
    let collection = o.fulfilment == "collection";
    let (change, target) = match o.state.as_str() {
        "paid" => (Change::Baking, (start - BAKE_LEAD).max(paid + 60)),
        "baking" => (
            Change::Ready,
            if collection {
                start
            } else {
                start - READY_LEAD
            },
        ),
        "ready" if collection => (Change::Collected, start + (end - start) / 2),
        // A delivery order waits at ready for the courier's booking.
        _ => return None,
    };
    Some((change, target.max(last + GAP)))
}

/// One pass over every site's open orders: each order takes at most its next due step.
/// Returns what moved, in words.
///
/// # Errors
///
/// When the records can't be read; an order that can't move is logged and skipped.
pub async fn pass(app: &App, now: i64) -> anyhow::Result<Vec<String>> {
    let records = app.records().await?;
    let mut moved = Vec::new();
    for site in records.sites().await? {
        let agent = Actor::Shop {
            agent: orders_agent(&site),
        };
        for v in records.orders_in_fulfilment(&site, 500).await? {
            // The bundle's sample rows tell a fixed story; only real orders move.
            if v.order.sample != 0 {
                continue;
            }
            let Some((change, due)) = next_step(&v) else {
                continue;
            };
            if now < due {
                continue;
            }
            let id = v.order.order_id.clone();
            match transition(app, &site, &id, change, &agent).await {
                Ok(after) => moved.push(format!(
                    "{} {id}: {} by {}",
                    site.site,
                    after.order.state,
                    agent.words()
                )),
                Err(OrderError::Refused(words)) => {
                    tracing::info!(site = %site.site, order = %id, %words, "fulfilment: not moved");
                }
                Err(OrderError::Other(e)) => {
                    tracing::warn!(site = %site.site, order = %id, error = %format!("{e:#}"), "fulfilment: failed");
                }
            }
        }
    }
    Ok(moved)
}

/// [`pass`] on the sample sites mounted on this node.
///
/// # Errors
///
/// When the samples aren't mounted, or as [`pass`].
pub async fn pass_now(now: i64) -> anyhow::Result<Vec<String>> {
    pass(&crate::held::app()?, now).await
}

/// [`transition`] on the sample sites mounted on this node, by the site's slug.
///
/// # Errors
///
/// As [`transition`]; `Refused` when there's no such site.
pub async fn transition_on(
    slug: &str,
    order_id: &str,
    change: Change,
    by: &Actor,
) -> Result<OrderView, OrderError> {
    let app = crate::held::app().map_err(OrderError::Other)?;
    let records = app.records().await.map_err(OrderError::Other)?;
    let site = records
        .site(slug)
        .await
        .map_err(OrderError::Other)?
        .ok_or_else(|| refused("There's no such site."))?;
    transition(&app, &site, order_id, change, by).await
}

/// Runs [`pass`] every `every`, on the sample sites mounted on this node (saas-node's schedule
/// list spawns this).
pub async fn follow(every: Duration) {
    loop {
        // Until the samples are mounted there's nothing to move.
        if let Ok(app) = crate::held::app() {
            match pass(&app, time::now()).await {
                Ok(moved) if !moved.is_empty() => {
                    tracing::info!(moved = moved.len(), steps = %moved.join("; "), "fulfilment pass");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "fulfilment pass failed"),
            }
        }
        tokio::time::sleep(every).await;
    }
}

// ---- tracking -------------------------------------------------------------------------------

/// The order's timeline: each step with its time and who did it, oldest first, for the door's
/// status check and the order page.
pub fn timeline(v: &OrderView, site: &Site) -> Vec<Value> {
    v.events
        .iter()
        .map(|e| {
            json!({
                "step": e.step,
                "label": status_label(&e.step, &v.order.fulfilment).unwrap_or_else(|| e.step.replace('_', " ")),
                "at": time::fmt_clock(e.at, site.utc_offset_min),
                "by": e.actor,
                "note": e.note,
            })
        })
        .collect()
}
