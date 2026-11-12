//! Orders and Customers for a real (moved) site: read-only, over the tables that hold them today.
//!
//! **Where a real site's orders come from** (Sun 8 Nov 2026):
//!
//! - **A customer's AI agent**, through the site's agent door (`request_custom_order`, node's
//!   `door/`). Node holds each order as an `order-held` decision card in control.db's
//!   `decisions` (the first release's move lane); the card's effect carries what the agent sent
//!   (`description`, `date`, `name`, `email`). Approving tells the agent the order is accepted,
//!   with a reference (`K-` and six letters, from the card's id); nothing is charged. The door
//!   also writes a `door.order-held` receipt about the card.
//! - **PayPal**: W6's Effects Gateway keeps payments and refunds (`customer_payments`,
//!   `refunds`) by site and order id. They're read here by the order's card id or reference. No
//!   real site's checkout creates them yet, so a moved site has none.
//! - **The site's pages and Ask**: a moved site takes no orders there yet. Only the three
//!   samples do, in W2's `records.db`, which this module never reads.
//!
//! **Customers** are the people a site has served: W6's customer accounts on the site's own host
//! (`visitor_accounts`, with their consent receipts), the people named on orders, and visitors
//! who passed a question to the owner with a way to reach them (`passed_questions`). Customer
//! accounts are mounted only on the sample hosts today, so a moved site has none yet.
//!
//! Every read tolerates a missing table (a node without that crate's migrations): it reads as
//! nothing recorded. Nothing here writes. An owner's decision on an order goes through the
//! decisions path (the card's own page), never through this module.

use std::collections::BTreeMap;

use agentrix_types::Digest;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

/// Where an order came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A customer's AI agent, through the site's agent door.
    AgentDoor,
}

impl Source {
    /// Plain words for the owner.
    pub fn words(self) -> &'static str {
        match self {
            Source::AgentDoor => "A customer's AI agent",
        }
    }
}

/// Where an order is, from the owner's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    /// Held for the owner: the card waits for them.
    New,
    /// Approved, with a payment asked for and not yet taken.
    Waiting,
    /// Approved, and paid or taking no payment.
    Done,
    /// Money went back to the customer.
    Refunded,
    /// The owner declined it.
    Declined,
    /// Nobody answered before the card expired.
    Expired,
}

impl OrderState {
    /// The filter it falls under: `new`, `waiting`, `done`, `refunded` or `closed`.
    pub fn filter(self) -> &'static str {
        match self {
            OrderState::New => "new",
            OrderState::Waiting => "waiting",
            OrderState::Done => "done",
            OrderState::Refunded => "refunded",
            OrderState::Declined | OrderState::Expired => "closed",
        }
    }

    /// The chip's tone.
    pub fn tone(self) -> &'static str {
        match self {
            OrderState::New => "wait",
            OrderState::Waiting => "info",
            OrderState::Done => "ok",
            OrderState::Refunded | OrderState::Declined => "risk",
            OrderState::Expired => "",
        }
    }
}

/// A customer payment, as W6's Effects Gateway records it.
#[derive(Debug, Clone, PartialEq)]
pub struct Payment {
    /// `pay_…`.
    pub id: String,
    /// `paypal`.
    pub provider: String,
    /// ISO 4217.
    pub currency: String,
    /// What was asked, in minor units.
    pub amount_minor: i64,
    /// `created`, `captured`, `pending`, `declined`, `cancelled`, `refunded`, `partly_refunded`.
    pub state: String,
    /// What PayPal's sandbox took, not real money.
    pub sandbox: bool,
    /// When it was asked for.
    pub created_at: DateTime<Utc>,
}

/// A refund, each one an owner decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Refund {
    /// `rf_…`.
    pub id: String,
    /// ISO 4217.
    pub currency: String,
    /// In minor units.
    pub amount_minor: i64,
    /// `executed`, `not_executed` or `unknown`.
    pub state: String,
    /// The platform receipt's digest, when there is one.
    pub receipt: Option<String>,
    /// When.
    pub created_at: DateTime<Utc>,
}

/// A receipt about an order.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderReceipt {
    /// `sha256:…`.
    pub digest: String,
    /// Such as `door.order-held` or `decision.approved`.
    pub kind: String,
    /// When it was issued.
    pub at: DateTime<Utc>,
}

/// One order.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    /// The decision card's id; the order's page is `orders/{id}`.
    pub id: String,
    /// The reference the customer's agent is given once the owner approves: `K-` and six
    /// letters from the card's id (node's `decisions::held_of`).
    pub reference: String,
    /// Where it came in.
    pub source: Source,
    /// Where it is.
    pub state: OrderState,
    /// The customer's name, as sent.
    pub name: String,
    /// The customer's email, as sent.
    pub email: String,
    /// What they asked for, in their words.
    pub description: String,
    /// When they want it (`YYYY-MM-DD`), as sent.
    pub wanted: String,
    /// When it came in.
    pub created_at: DateTime<Utc>,
    /// After this the card can't be approved.
    pub expires_at: Option<DateTime<Utc>>,
    /// When the owner decided.
    pub decided_at: Option<DateTime<Utc>>,
    /// `passkey` or `totp`, for an approval.
    pub method: Option<String>,
    /// The decision's receipt.
    pub decision_receipt: Option<String>,
    /// Payments asked for on it.
    pub payments: Vec<Payment>,
    /// Refunds on those payments.
    pub refunds: Vec<Refund>,
}

impl Order {
    /// The customer's key in [`Customer::key`] terms, so the order links to its customer.
    pub fn contact(&self) -> String {
        self.email.trim().to_lowercase()
    }
}

/// A customer account on the site (W6).
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    /// `vac_…`.
    pub id: String,
    /// When they made it.
    pub created_at: DateTime<Utc>,
    /// Whether they proved the email address.
    pub verified: bool,
}

/// A consent a customer gave (W6's consent receipts).
#[derive(Debug, Clone, PartialEq)]
pub struct Consent {
    /// `order-updates`, `account-memory` or `offers`.
    pub purpose: String,
    /// When given.
    pub given_at: DateTime<Utc>,
    /// When withdrawn, if it was.
    pub withdrawn_at: Option<DateTime<Utc>>,
    /// The platform receipt that signs it.
    pub receipt: Option<String>,
}

/// A question a visitor passed to the owner from Ask, with a way to reach them.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    /// Their words.
    pub question: String,
    /// When.
    pub at: DateTime<Utc>,
}

/// One person the site served.
#[derive(Debug, Clone, PartialEq)]
pub struct Customer {
    /// The page's id: the account id, else `c_` and a hash of the site and the contact, so no
    /// email address is ever in a URL.
    pub key: String,
    /// Their name, from the account or the newest order; empty when only a contact is known.
    pub name: String,
    /// Their email or the contact they left, lowercased.
    pub contact: String,
    /// A phone number from the account.
    pub phone: Option<String>,
    /// Their account on the site, if they made one.
    pub account: Option<Account>,
    /// Their orders, newest first.
    pub orders: Vec<Order>,
    /// Questions they passed to the owner, newest first.
    pub questions: Vec<Question>,
    /// Consents on their account.
    pub consents: Vec<Consent>,
    /// The first time the site recorded them.
    pub first_at: DateTime<Utc>,
    /// The last time.
    pub last_at: DateTime<Utc>,
}

impl Customer {
    /// Whether `q` (already lowercased) is in their name or contact.
    pub fn matches(&self, q: &str) -> bool {
        q.is_empty() || self.name.to_lowercase().contains(q) || self.contact.contains(q)
    }
}

/// A table that isn't there yet: nothing recorded, not an error.
fn absent(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(d) => {
            let m = d.message();
            m.contains("no such table") || m.contains("unable to open")
        }
        sqlx::Error::Io(_) => true,
        _ => false,
    }
}

fn or_none<T>(r: Result<Vec<T>, sqlx::Error>) -> anyhow::Result<Vec<T>> {
    match r {
        Ok(v) => Ok(v),
        Err(e) if absent(&e) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn rfc3339(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn unix(t: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(t, 0).single().unwrap_or_default()
}

/// The reference node gives the customer's agent for an approved order.
pub fn reference(decision_id: &str) -> String {
    let hex = Digest::of(decision_id.as_bytes()).hex();
    format!("K-{}", hex[..6].to_ascii_uppercase())
}

/// The page key for a customer without an account on `site`.
pub fn contact_key(site: &str, contact: &str) -> String {
    let hex = Digest::of(format!("{site}\0{contact}").as_bytes()).hex();
    format!("c_{}", &hex[..16])
}

/// Money in the owner's words: "KSh 2,400", "USD 40.70".
pub fn money(currency: &str, minor: i64) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let m = minor.unsigned_abs();
    let (whole, cents) = (m / 100, m % 100);
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let unit = if currency.eq_ignore_ascii_case("KES") {
        "KSh".to_owned()
    } else {
        currency.to_ascii_uppercase()
    };
    if cents == 0 {
        format!("{sign}{unit} {grouped}")
    } else {
        format!("{sign}{unit} {grouped}.{cents:02}")
    }
}

/// The tables Orders and Customers read, in `control.db`.
#[derive(Debug, Clone)]
pub struct Commerce {
    pool: SqlitePool,
}

impl Commerce {
    /// Over control.db.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Every order of `site`, newest first, as of `now`.
    pub async fn orders(&self, site: &str, now: DateTime<Utc>) -> anyhow::Result<Vec<Order>> {
        let rows = or_none(
            sqlx::query(
                "SELECT decision_id, card, state, method, receipt, created_at, decided_at \
                 FROM decisions WHERE site_id = ? AND kind = 'order-held' \
                 ORDER BY created_at DESC, decision_id DESC",
            )
            .bind(site)
            .fetch_all(&self.pool)
            .await,
        )?;
        let payments = self.payments(site).await?;
        let refunds = self.refunds(site).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get(0)?;
            let card: Value = serde_json::from_str(&r.try_get::<String, _>(1)?).unwrap_or_default();
            let decided: String = r.try_get(2)?;
            let created_at = rfc3339(&r.try_get::<String, _>(5)?).unwrap_or_default();
            let decided_at = r
                .try_get::<Option<String>, _>(6)?
                .as_deref()
                .and_then(rfc3339);
            let input = &card["effect"]["args"]["input"];
            let text = |k: &str| input[k].as_str().unwrap_or_default().trim().to_owned();
            let expires_at = card["effect"]["expiry"].as_str().and_then(rfc3339);
            let reference = reference(&id);
            let mine = |order_id: &str| order_id == id || order_id == reference;
            let pays: Vec<(String, Payment)> =
                payments.iter().filter(|(o, _)| mine(o)).cloned().collect();
            let backs: Vec<Refund> = refunds
                .iter()
                .filter(|(p, _)| pays.iter().any(|(_, x)| &x.id == p))
                .map(|(_, r)| r.clone())
                .collect();
            let pays: Vec<Payment> = pays.into_iter().map(|(_, p)| p).collect();
            let state = match decided.as_str() {
                "waiting" if expires_at.is_some_and(|e| now >= e) => OrderState::Expired,
                "waiting" => OrderState::New,
                "declined" => OrderState::Declined,
                "expired" => OrderState::Expired,
                _ if backs.iter().any(|r| r.state == "executed")
                    || pays
                        .iter()
                        .any(|p| matches!(p.state.as_str(), "refunded" | "partly_refunded")) =>
                {
                    OrderState::Refunded
                }
                _ if pays.iter().any(|p| p.state == "captured") => OrderState::Done,
                _ if pays
                    .iter()
                    .any(|p| matches!(p.state.as_str(), "created" | "pending")) =>
                {
                    OrderState::Waiting
                }
                _ => OrderState::Done,
            };
            out.push(Order {
                reference,
                source: Source::AgentDoor,
                state,
                name: text("name"),
                email: text("email"),
                description: text("description"),
                wanted: text("date"),
                created_at,
                expires_at,
                decided_at,
                method: r.try_get(3)?,
                decision_receipt: r.try_get(4)?,
                payments: pays,
                refunds: backs,
                id,
            });
        }
        Ok(out)
    }

    /// One order of `site`.
    pub async fn order(
        &self,
        site: &str,
        id: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<Order>> {
        Ok(self
            .orders(site, now)
            .await?
            .into_iter()
            .find(|o| o.id == id))
    }

    /// How many orders wait for the owner, for the dock's badge.
    pub async fn waiting_count(&self, site: &str, now: DateTime<Utc>) -> anyhow::Result<u32> {
        let n = self
            .orders(site, now)
            .await?
            .iter()
            .filter(|o| o.state == OrderState::New)
            .count();
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Receipts about an order: the door's and the decision's, oldest first.
    pub async fn receipts(&self, site: &str, order: &Order) -> anyhow::Result<Vec<OrderReceipt>> {
        let rows = or_none(
            sqlx::query(
                "SELECT digest, kind, issued_at FROM receipts \
                 WHERE site_id = ? AND subject_id = ? ORDER BY issued_at",
            )
            .bind(site)
            .bind(&order.id)
            .fetch_all(&self.pool)
            .await,
        )?;
        let mut out: Vec<OrderReceipt> = Vec::new();
        for r in rows {
            out.push(OrderReceipt {
                digest: r.try_get(0)?,
                kind: r.try_get(1)?,
                at: rfc3339(&r.try_get::<String, _>(2)?).unwrap_or_default(),
            });
        }
        if let Some(d) = &order.decision_receipt
            && !out.iter().any(|r| &r.digest == d)
        {
            out.push(OrderReceipt {
                digest: d.clone(),
                kind: "decision".into(),
                at: order.decided_at.unwrap_or(order.created_at),
            });
        }
        for f in &order.refunds {
            if let Some(d) = &f.receipt
                && !out.iter().any(|r| &r.digest == d)
            {
                out.push(OrderReceipt {
                    digest: d.clone(),
                    kind: "payment.refunded".into(),
                    at: f.created_at,
                });
            }
        }
        out.sort_by_key(|r| r.at);
        Ok(out)
    }

    async fn payments(&self, site: &str) -> anyhow::Result<Vec<(String, Payment)>> {
        let rows = or_none(
            sqlx::query(
                "SELECT order_id, id, provider, currency, amount_minor, state, sandbox, created_at \
                 FROM customer_payments WHERE site_id = ? ORDER BY created_at, id",
            )
            .bind(site)
            .fetch_all(&self.pool)
            .await,
        )?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get(0)?,
                    Payment {
                        id: r.try_get(1)?,
                        provider: r.try_get(2)?,
                        currency: r.try_get(3)?,
                        amount_minor: r.try_get(4)?,
                        state: r.try_get(5)?,
                        sandbox: r.try_get::<i64, _>(6)? != 0,
                        created_at: unix(r.try_get(7)?),
                    },
                ))
            })
            .collect()
    }

    async fn refunds(&self, site: &str) -> anyhow::Result<Vec<(String, Refund)>> {
        let rows = or_none(
            sqlx::query(
                "SELECT payment_id, id, currency, amount_minor, state, receipt, created_at \
                 FROM refunds WHERE site_id = ? ORDER BY created_at, id",
            )
            .bind(site)
            .fetch_all(&self.pool)
            .await,
        )?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get(0)?,
                    Refund {
                        id: r.try_get(1)?,
                        currency: r.try_get(2)?,
                        amount_minor: r.try_get(3)?,
                        state: r.try_get(4)?,
                        receipt: r.try_get(5)?,
                        created_at: unix(r.try_get(6)?),
                    },
                ))
            })
            .collect()
    }

    /// Everyone `site` has served, most recent first, as of `now`.
    pub async fn customers(&self, site: &str, now: DateTime<Utc>) -> anyhow::Result<Vec<Customer>> {
        let mut by: BTreeMap<String, Customer> = BTreeMap::new();
        let blank = |key: String, contact: String, at: DateTime<Utc>| Customer {
            key,
            name: String::new(),
            contact,
            phone: None,
            account: None,
            orders: Vec::new(),
            questions: Vec::new(),
            consents: Vec::new(),
            first_at: at,
            last_at: at,
        };

        // W6's customer accounts on the site's own host.
        let accounts = or_none(
            sqlx::query(
                "SELECT id, email, name, phone, email_verified_at, created_at \
                 FROM visitor_accounts WHERE site_id = ?",
            )
            .bind(site)
            .fetch_all(&self.pool)
            .await,
        )?;
        for r in accounts {
            let id: String = r.try_get(0)?;
            let contact = r.try_get::<String, _>(1)?.trim().to_lowercase();
            let at = unix(r.try_get(5)?);
            let c = by
                .entry(contact.clone())
                .or_insert_with(|| blank(id.clone(), contact, at));
            c.key = id.clone();
            c.name = r.try_get(2)?;
            c.phone = r.try_get::<Option<String>, _>(3)?.filter(|p| !p.is_empty());
            c.account = Some(Account {
                id,
                created_at: at,
                verified: r.try_get::<Option<i64>, _>(4)?.is_some(),
            });
        }
        for c in by.values_mut() {
            if let Some(a) = &c.account {
                c.consents = self.consents(site, &a.id).await?;
            }
        }

        // The people named on orders.
        for o in self.orders(site, now).await? {
            let contact = o.contact();
            if contact.is_empty() {
                continue;
            }
            let c = by
                .entry(contact.clone())
                .or_insert_with(|| blank(contact_key(site, &contact), contact, o.created_at));
            if c.account.is_none() && c.name.is_empty() {
                c.name = o.name.clone();
            }
            c.first_at = c.first_at.min(o.created_at);
            c.last_at = c.last_at.max(o.created_at);
            c.orders.push(o);
        }

        // Visitors who passed a question with a way to reach them.
        let passed = or_none(
            sqlx::query(
                "SELECT question, contact, passed_at FROM passed_questions \
                 WHERE site_id = ? AND contact IS NOT NULL ORDER BY passed_at DESC",
            )
            .bind(site)
            .fetch_all(&self.pool)
            .await,
        )?;
        for r in passed {
            let contact = r.try_get::<String, _>(1)?.trim().to_lowercase();
            if contact.is_empty() {
                continue;
            }
            let at = rfc3339(&r.try_get::<String, _>(2)?).unwrap_or_default();
            let c = by
                .entry(contact.clone())
                .or_insert_with(|| blank(contact_key(site, &contact), contact, at));
            c.first_at = c.first_at.min(at);
            c.last_at = c.last_at.max(at);
            c.questions.push(Question {
                question: r.try_get(0)?,
                at,
            });
        }

        let mut out: Vec<Customer> = by.into_values().collect();
        for c in &mut out {
            c.orders.sort_by_key(|o| std::cmp::Reverse(o.created_at));
            c.questions.sort_by_key(|q| std::cmp::Reverse(q.at));
        }
        out.sort_by(|a, b| b.last_at.cmp(&a.last_at).then(a.key.cmp(&b.key)));
        Ok(out)
    }

    /// One customer of `site` by page key.
    pub async fn customer(
        &self,
        site: &str,
        key: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<Customer>> {
        Ok(self
            .customers(site, now)
            .await?
            .into_iter()
            .find(|c| c.key == key))
    }

    async fn consents(&self, site: &str, account: &str) -> anyhow::Result<Vec<Consent>> {
        let rows = or_none(
            sqlx::query(
                "SELECT purpose, given_at, withdrawn_at, receipt FROM consent_receipts \
                 WHERE site_id = ? AND account_id = ? ORDER BY given_at DESC",
            )
            .bind(site)
            .bind(account)
            .fetch_all(&self.pool)
            .await,
        )?;
        rows.iter()
            .map(|r| {
                Ok(Consent {
                    purpose: r.try_get(0)?,
                    given_at: unix(r.try_get(1)?),
                    withdrawn_at: r.try_get::<Option<i64>, _>(2)?.map(unix),
                    receipt: r.try_get(3)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_like_the_owner_writes_it() {
        assert_eq!(money("KES", 450_000), "KSh 4,500");
        assert_eq!(money("KES", 123_456_700), "KSh 1,234,567");
        assert_eq!(money("USD", 4070), "USD 40.70");
        assert_eq!(money("KES", 0), "KSh 0");
    }

    #[test]
    fn the_reference_matches_what_the_agent_is_told() {
        let r = reference("dec_1");
        assert!(r.starts_with("K-") && r.len() == 8, "{r}");
        assert_eq!(r, reference("dec_1"));
        assert!(contact_key("s", "a@example.com").starts_with("c_"));
        assert!(!contact_key("s", "a@example.com").contains('@'));
    }

    #[tokio::test]
    async fn missing_tables_read_as_nothing_recorded() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let c = Commerce::new(pool);
        assert!(c.orders("s", Utc::now()).await.unwrap().is_empty());
        assert!(c.customers("s", Utc::now()).await.unwrap().is_empty());
        assert_eq!(c.waiting_count("s", Utc::now()).await.unwrap(), 0);
    }
}
