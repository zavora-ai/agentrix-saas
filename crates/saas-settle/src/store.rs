//! What each side keeps: used cart mandates (claimed before money moves), month totals per
//! intent mandate, agreed plans and their schedule, the payer's saved wallet, request nonces,
//! and the bills offered to a payer. SQLite, migrations 1700–1709 in their own table.

use agentrix_types::Money;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

/// The name of this crate's migrations table.
pub const MIGRATIONS_TABLE: &str = "_sqlx_migrations_saas_settle";

/// Applies this crate's migrations to `pool`.
pub fn migrate(pool: &SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
        migrator.set_ignore_missing(true);
        migrator.run(pool).await?;
        Ok(())
    })
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A cart mandate's row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartRow {
    /// The cart mandate.
    pub cart_mandate_id: String,
    /// `claimed`, `settled`, `failed` or `unknown`.
    pub state: String,
    /// The PayPal key once settled.
    pub paypal_key: Option<String>,
    /// The sealed receipt, once settled.
    pub receipt_json: Option<String>,
}

/// What claiming a cart mandate found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// New: go ahead.
    Fresh,
    /// PayPal refused it before: try again with the same idempotency key.
    Retry,
    /// Its outcome is unknown: settle it by asking PayPal again with the same key.
    Resume,
    /// Used (settled) or in progress.
    Used(CartRow),
}

/// What a claim records.
#[derive(Debug, Clone)]
pub struct NewCart<'a> {
    /// The cart mandate.
    pub cart_mandate_id: &'a str,
    /// `sha256:` of the sealed cart mandate.
    pub cart_digest: &'a str,
    /// Its intent mandate.
    pub intent_mandate_id: &'a str,
    /// The invoice.
    pub invoice_id: &'a str,
    /// The payer's origin.
    pub payer: &'a str,
    /// How much.
    pub amount: &'a Money,
    /// `2026-11`.
    pub month: &'a str,
    /// `vault` or `payout`.
    pub route: &'a str,
}

/// The settlement store over one pool.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// A store over `pool` (migrated with [`migrate`]).
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// The pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Claims a cart mandate before any money moves. A digest seen under another id is a
    /// replay too.
    pub async fn claim_cart(&self, cart: &NewCart<'_>) -> anyhow::Result<Claim> {
        let existing = sqlx::query(
            "SELECT cart_mandate_id, state, paypal_key, receipt_json FROM settle_carts
             WHERE cart_mandate_id = ? OR cart_digest = ?",
        )
        .bind(cart.cart_mandate_id)
        .bind(cart.cart_digest)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = existing {
            let found = CartRow {
                cart_mandate_id: row.get("cart_mandate_id"),
                state: row.get("state"),
                paypal_key: row.get("paypal_key"),
                receipt_json: row.get("receipt_json"),
            };
            if found.cart_mandate_id != cart.cart_mandate_id {
                return Ok(Claim::Used(found));
            }
            return Ok(match found.state.as_str() {
                "failed" => {
                    self.set_state(cart.cart_mandate_id, "claimed").await?;
                    Claim::Retry
                }
                "unknown" => {
                    self.set_state(cart.cart_mandate_id, "claimed").await?;
                    Claim::Resume
                }
                _ => Claim::Used(found),
            });
        }
        let at = now();
        sqlx::query(
            "INSERT INTO settle_carts (cart_mandate_id, cart_digest, intent_mandate_id, invoice_id, payer,
                currency, amount_minor, month, route, state, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'claimed', ?, ?)",
        )
        .bind(cart.cart_mandate_id)
        .bind(cart.cart_digest)
        .bind(cart.intent_mandate_id)
        .bind(cart.invoice_id)
        .bind(cart.payer)
        .bind(&cart.amount.currency)
        .bind(cart.amount.minor)
        .bind(cart.month)
        .bind(cart.route)
        .bind(at)
        .bind(at)
        .execute(&self.pool)
        .await?;
        Ok(Claim::Fresh)
    }

    async fn set_state(&self, cart_mandate_id: &str, state: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE settle_carts SET state = ?, updated_at = ? WHERE cart_mandate_id = ?")
            .bind(state)
            .bind(now())
            .bind(cart_mandate_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records how a claimed cart mandate ended.
    pub async fn finish_cart(
        &self,
        cart_mandate_id: &str,
        state: &str,
        paypal_key: Option<&str>,
        receipt_json: Option<&str>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE settle_carts SET state = ?, paypal_key = COALESCE(?, paypal_key),
                receipt_json = COALESCE(?, receipt_json), error = ?, updated_at = ?
             WHERE cart_mandate_id = ?",
        )
        .bind(state)
        .bind(paypal_key)
        .bind(receipt_json)
        .bind(error)
        .bind(now())
        .bind(cart_mandate_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Whether a cart mandate (by id or by digest) already moved money or is moving it.
    pub async fn used(&self, cart_mandate_id: &str, cart_digest: &str) -> anyhow::Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM settle_carts WHERE (cart_mandate_id = ? OR cart_digest = ?)
             AND state IN ('claimed', 'settled')",
        )
        .bind(cart_mandate_id)
        .bind(cart_digest)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Keeps the sealed cart mandate as sent, so a retry resends the same one.
    pub async fn set_cart_json(&self, cart_mandate_id: &str, json: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE settle_carts SET cart_json = ? WHERE cart_mandate_id = ?")
            .bind(json)
            .bind(cart_mandate_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The sealed cart mandate as sent.
    pub async fn cart_json(&self, cart_mandate_id: &str) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT cart_json FROM settle_carts WHERE cart_mandate_id = ?",
        )
        .bind(cart_mandate_id)
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// One cart mandate's row.
    pub async fn cart(&self, cart_mandate_id: &str) -> anyhow::Result<Option<CartRow>> {
        let row = sqlx::query(
            "SELECT cart_mandate_id, state, paypal_key, receipt_json FROM settle_carts WHERE cart_mandate_id = ?",
        )
        .bind(cart_mandate_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| CartRow {
            cart_mandate_id: row.get("cart_mandate_id"),
            state: row.get("state"),
            paypal_key: row.get("paypal_key"),
            receipt_json: row.get("receipt_json"),
        }))
    }

    /// Paid (or being paid) under `intent_mandate_id` in `month`, in `currency`, by cart
    /// mandates other than `except` (the one being settled, which may be resuming).
    pub async fn month_paid(
        &self,
        intent_mandate_id: &str,
        month: &str,
        currency: &str,
        except: &str,
    ) -> anyhow::Result<Money> {
        let minor: Option<i64> = sqlx::query_scalar(
            "SELECT SUM(amount_minor) FROM settle_carts
             WHERE intent_mandate_id = ? AND month = ? AND currency = ? AND cart_mandate_id != ?
               AND state IN ('claimed', 'settled', 'unknown')",
        )
        .bind(intent_mandate_id)
        .bind(month)
        .bind(currency)
        .bind(except)
        .fetch_one(&self.pool)
        .await?;
        Ok(Money {
            currency: currency.to_owned(),
            minor: minor.unwrap_or(0),
        })
    }

    /// After a restart nothing is in progress: a claim left mid-payment becomes unknown, to be
    /// settled by asking PayPal again with the same cart mandate. Returns how many.
    pub async fn recover_claims(&self) -> anyhow::Result<u64> {
        let done = sqlx::query(
            "UPDATE settle_carts SET state = 'unknown', updated_at = ? WHERE state = 'claimed'",
        )
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// A cart mandate on `invoice_id` whose outcome is unknown or still in progress.
    pub async fn pending_for_invoice(&self, invoice_id: &str) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT cart_mandate_id FROM settle_carts WHERE invoice_id = ? AND state IN ('unknown', 'claimed')
             ORDER BY created_at LIMIT 1",
        )
        .bind(invoice_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Settled so far on `invoice_id`, in `currency`.
    pub async fn invoice_paid(&self, invoice_id: &str, currency: &str) -> anyhow::Result<Money> {
        let minor: Option<i64> = sqlx::query_scalar(
            "SELECT SUM(amount_minor) FROM settle_carts WHERE invoice_id = ? AND currency = ? AND state = 'settled'",
        )
        .bind(invoice_id)
        .bind(currency)
        .fetch_one(&self.pool)
        .await?;
        Ok(Money {
            currency: currency.to_owned(),
            minor: minor.unwrap_or(0),
        })
    }

    /// Records a request nonce; false when it was seen before (a replay).
    pub async fn fresh_nonce(
        &self,
        origin: &str,
        nonce: &str,
        expires_at: i64,
    ) -> anyhow::Result<bool> {
        sqlx::query("DELETE FROM settle_nonces WHERE expires_at < ?")
            .bind(now() - 120)
            .execute(&self.pool)
            .await?;
        let done = sqlx::query(
            "INSERT INTO settle_nonces (origin, nonce, expires_at) VALUES (?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(origin)
        .bind(nonce)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// The payer's saved wallet: `(vault_id, customer_id)`.
    pub async fn vault(&self, payer: &str) -> anyhow::Result<Option<VaultRow>> {
        let row = sqlx::query(
            "SELECT payer, vault_id, customer_id, setup_token, approve_url, state FROM settle_vault WHERE payer = ?",
        )
        .bind(payer)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| VaultRow {
            payer: row.get("payer"),
            vault_id: row.get("vault_id"),
            customer_id: row.get("customer_id"),
            setup_token: row.get("setup_token"),
            approve_url: row.get("approve_url"),
            state: row.get("state"),
        }))
    }

    /// The wallet row awaiting (or done with) `setup_token`.
    pub async fn vault_by_token(&self, setup_token: &str) -> anyhow::Result<Option<VaultRow>> {
        let payer: Option<String> =
            sqlx::query_scalar("SELECT payer FROM settle_vault WHERE setup_token = ?")
                .bind(setup_token)
                .fetch_optional(&self.pool)
                .await?;
        match payer {
            Some(payer) => self.vault(&payer).await,
            None => Ok(None),
        }
    }

    /// Saves (or replaces) the payer's wallet row.
    pub async fn put_vault(&self, row: &VaultRow) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO settle_vault (payer, vault_id, customer_id, setup_token, approve_url, state, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (payer) DO UPDATE SET vault_id = excluded.vault_id, customer_id = excluded.customer_id,
               setup_token = excluded.setup_token, approve_url = excluded.approve_url, state = excluded.state,
               updated_at = excluded.updated_at",
        )
        .bind(&row.payer)
        .bind(&row.vault_id)
        .bind(&row.customer_id)
        .bind(&row.setup_token)
        .bind(&row.approve_url)
        .bind(&row.state)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Saves an agreed plan and its schedule.
    pub async fn put_plan(&self, plan: &PlanRow, schedule: &[ScheduledRow]) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO settle_plans (plan_id, invoice_id, payer, intent_json, plan_json, agreement_json, state, created_at)
             VALUES (?, ?, ?, ?, ?, ?, 'active', ?)",
        )
        .bind(&plan.plan_id)
        .bind(&plan.invoice_id)
        .bind(&plan.payer)
        .bind(&plan.intent_json)
        .bind(&plan.plan_json)
        .bind(&plan.agreement_json)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        for s in schedule {
            sqlx::query(
                "INSERT INTO settle_schedule (plan_id, idx, due_on, currency, amount_minor, cart_json, cart_mandate_id, state)
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'scheduled')",
            )
            .bind(&s.plan_id)
            .bind(s.index)
            .bind(&s.due_on)
            .bind(&s.amount.currency)
            .bind(s.amount.minor)
            .bind(&s.cart_json)
            .bind(&s.cart_mandate_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// One agreed plan.
    pub async fn plan(&self, plan_id: &str) -> anyhow::Result<Option<PlanRow>> {
        let row = sqlx::query(
            "SELECT plan_id, invoice_id, payer, intent_json, plan_json, agreement_json, state FROM settle_plans WHERE plan_id = ?",
        )
        .bind(plan_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| PlanRow {
            plan_id: row.get("plan_id"),
            invoice_id: row.get("invoice_id"),
            payer: row.get("payer"),
            intent_json: row.get("intent_json"),
            plan_json: row.get("plan_json"),
            agreement_json: row.get("agreement_json"),
            state: row.get("state"),
        }))
    }

    /// The plans with something on terms for `payer`, and what is still scheduled on them.
    pub async fn scheduled_for(&self, payer: &str, currency: &str) -> anyhow::Result<Money> {
        let minor: Option<i64> = sqlx::query_scalar(
            "SELECT SUM(s.amount_minor) FROM settle_schedule s JOIN settle_plans p ON p.plan_id = s.plan_id
             WHERE p.payer = ? AND s.currency = ? AND s.state = 'scheduled'",
        )
        .bind(payer)
        .bind(currency)
        .fetch_one(&self.pool)
        .await?;
        Ok(Money {
            currency: currency.to_owned(),
            minor: minor.unwrap_or(0),
        })
    }

    /// Instalments due on or before `as_of` (`YYYY-MM-DD`) and not yet collected.
    pub async fn due(&self, as_of: &str) -> anyhow::Result<Vec<ScheduledRow>> {
        let rows = sqlx::query(
            "SELECT plan_id, idx, due_on, currency, amount_minor, cart_json, cart_mandate_id, state FROM settle_schedule
             WHERE state IN ('scheduled', 'failed') AND due_on <= ? ORDER BY due_on, plan_id, idx",
        )
        .bind(as_of)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(scheduled).collect())
    }

    /// Every instalment not yet collected, in due order: what a kernel puts on its wake list
    /// (again, after a restart).
    pub async fn uncollected(&self) -> anyhow::Result<Vec<ScheduledRow>> {
        let rows = sqlx::query(
            "SELECT plan_id, idx, due_on, currency, amount_minor, cart_json, cart_mandate_id, state FROM settle_schedule
             WHERE state IN ('scheduled', 'failed') ORDER BY due_on, plan_id, idx",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(scheduled).collect())
    }

    /// One scheduled instalment (1-based `index`).
    pub async fn instalment(
        &self,
        plan_id: &str,
        index: u32,
    ) -> anyhow::Result<Option<(ScheduledRow, Option<String>)>> {
        let row = sqlx::query(
            "SELECT plan_id, idx, due_on, currency, amount_minor, cart_json, cart_mandate_id, state, receipt_json
             FROM settle_schedule WHERE plan_id = ? AND idx = ?",
        )
        .bind(plan_id)
        .bind(index)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| {
            let receipt: Option<String> = row.get("receipt_json");
            (scheduled(row), receipt)
        }))
    }

    /// A plan's schedule.
    pub async fn schedule(&self, plan_id: &str) -> anyhow::Result<Vec<ScheduledRow>> {
        let rows = sqlx::query(
            "SELECT plan_id, idx, due_on, currency, amount_minor, cart_json, cart_mandate_id, state FROM settle_schedule
             WHERE plan_id = ? ORDER BY idx",
        )
        .bind(plan_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(scheduled).collect())
    }

    /// Marks one instalment collected (or failed).
    pub async fn finish_instalment(
        &self,
        plan_id: &str,
        index: u32,
        state: &str,
        receipt_json: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE settle_schedule SET state = ?, receipt_json = COALESCE(?, receipt_json) WHERE plan_id = ? AND idx = ?",
        )
        .bind(state)
        .bind(receipt_json)
        .bind(plan_id)
        .bind(index)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE settle_plans SET state = 'paid' WHERE plan_id = ?
             AND NOT EXISTS (SELECT 1 FROM settle_schedule WHERE plan_id = ? AND state != 'settled')",
        )
        .bind(plan_id)
        .bind(plan_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The cart mandate an instalment's schedule row holds: `(plan_id, index)`.
    pub async fn instalment_of_cart(
        &self,
        cart_mandate_id: &str,
    ) -> anyhow::Result<Option<(String, u32)>> {
        let row = sqlx::query("SELECT plan_id, idx FROM settle_schedule WHERE cart_mandate_id = ?")
            .bind(cart_mandate_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| (r.get("plan_id"), r.get::<i64, _>("idx") as u32)))
    }

    /// The most recently settled cart mandate, as sent (the payer's side keeps them).
    pub async fn latest_settled_cart(&self) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar::<_, Option<String>>(
            "SELECT cart_json FROM settle_carts WHERE state = 'settled' AND cart_json IS NOT NULL
             ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }

    /// How many cart mandates have settled (money moved).
    pub async fn settled_count(&self) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM settle_carts WHERE state = 'settled'")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// The most recent bill offered to the payer: `(payee, invoice_id)`.
    pub async fn latest_offer(&self) -> anyhow::Result<Option<(String, String)>> {
        let row = sqlx::query(
            "SELECT payee, invoice_id FROM settle_offers ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| (r.get("payee"), r.get("invoice_id"))))
    }

    /// Keeps a sealed refusal receipt.
    pub async fn put_refusal(
        &self,
        refusal_id: &str,
        code: &str,
        receipt_json: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO settle_refusals (refusal_id, at, code, receipt_json) VALUES (?, ?, ?, ?)",
        )
        .bind(refusal_id)
        .bind(now())
        .bind(code)
        .bind(receipt_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The most recent sealed refusal receipts, newest first.
    pub async fn refusals(&self, limit: i64) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT receipt_json FROM settle_refusals ORDER BY at DESC, refusal_id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The payer's side: records a bill offered to it.
    pub async fn put_offer(
        &self,
        payee: &str,
        invoice_id: &str,
        invoice_json: &str,
        decision: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO settle_offers (invoice_id, payee, invoice_json, decision, created_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (payee, invoice_id) DO UPDATE SET invoice_json = excluded.invoice_json, decision = excluded.decision",
        )
        .bind(invoice_id)
        .bind(payee)
        .bind(invoice_json)
        .bind(decision)
        .bind(now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The payer's side: a bill offered to it, as sealed JSON.
    pub async fn offer(&self, payee: &str, invoice_id: &str) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT invoice_json FROM settle_offers WHERE payee = ? AND invoice_id = ?",
        )
        .bind(payee)
        .bind(invoice_id)
        .fetch_optional(&self.pool)
        .await?)
    }
}

fn scheduled(row: sqlx::sqlite::SqliteRow) -> ScheduledRow {
    ScheduledRow {
        plan_id: row.get("plan_id"),
        index: row.get::<i64, _>("idx") as u32,
        due_on: row.get("due_on"),
        amount: Money {
            currency: row.get("currency"),
            minor: row.get("amount_minor"),
        },
        cart_json: row.get("cart_json"),
        cart_mandate_id: row.get("cart_mandate_id"),
        state: row.get("state"),
    }
}

/// The payer's saved wallet as the payee holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultRow {
    /// The payer's origin.
    pub payer: String,
    /// PayPal's payment token id (the `vault_id`), once saved.
    pub vault_id: Option<String>,
    /// PayPal's customer id.
    pub customer_id: Option<String>,
    /// The setup token awaiting the payer's approval.
    pub setup_token: Option<String>,
    /// Where the payer approves.
    pub approve_url: Option<String>,
    /// `pending` or `saved`.
    pub state: String,
}

/// An agreed plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRow {
    /// `plan_…`.
    pub plan_id: String,
    /// The invoice.
    pub invoice_id: String,
    /// The payer's origin.
    pub payer: String,
    /// The payer's sealed intent mandate.
    pub intent_json: String,
    /// The sealed plan.
    pub plan_json: String,
    /// The sealed agreement.
    pub agreement_json: String,
    /// `active` or `paid`.
    pub state: String,
}

/// One scheduled collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledRow {
    /// The plan.
    pub plan_id: String,
    /// 1-based.
    pub index: u32,
    /// `YYYY-MM-DD`.
    pub due_on: String,
    /// How much.
    pub amount: Money,
    /// The payer's sealed cart mandate for it.
    pub cart_json: String,
    /// Its id.
    pub cart_mandate_id: String,
    /// `scheduled`, `settled` or `failed`.
    pub state: String,
}
