//! Products and stock.
//!
//! Stock on screen is computed: what was planned (a bakery's bake plan for the day, or a shop's
//! count at its last stock take) less what active orders and holds took since.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::{Records, Site, time};

/// A product.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Product {
    /// Stock-keeping id.
    pub sku: String,
    /// URL slug, also the anchor id on list pages (`item-<slug>`).
    pub slug: String,
    /// Where it is listed: `bread`, `box`, `fragrance`, `gift`, `sample`.
    pub section: String,
    /// Name.
    pub name: String,
    /// A plain description.
    pub description: String,
    /// Scent family, or `None`.
    pub family: Option<String>,
    /// Size, such as `50 ml`.
    pub size: Option<String>,
    /// Price in the site's minor units.
    pub price_minor: i64,
    /// Attributes as JSON text: ingredients, allergens, notes, gift-ready, image tone.
    #[serde(skip)]
    pub attrs: String,
    /// `published` or `draft`.
    pub status: String,
    /// List order.
    pub sort: i64,
}

impl Product {
    /// The attributes, parsed.
    pub fn attrs(&self) -> Value {
        serde_json::from_str(&self.attrs).unwrap_or(Value::Null)
    }
}

/// Stock for one product, as of now.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stock {
    /// The sku.
    pub sku: String,
    /// Units left, or `None` when no stock is kept for it.
    pub left: Option<i64>,
    /// When the next batch is due (bakeries), if later today.
    pub next_batch_at: Option<i64>,
    /// When it is back, when sold out.
    pub back_at: Option<i64>,
    /// When the plan or count was taken.
    pub as_of: i64,
    /// `in`, `low`, `batch`, `out` or `none`.
    pub state: &'static str,
}

/// Order states that hold stock.
pub(crate) const ACTIVE: &str =
    "('held','approved','awaiting_payment','paid','baking','ready','out','delivered','collected')";

impl Records {
    /// Published products of a site, optionally in one section, in list order.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn products(
        &self,
        site: &str,
        section: Option<&str>,
    ) -> anyhow::Result<Vec<Product>> {
        let rows = sqlx::query_as::<_, Product>(
            "SELECT sku, slug, section, name, description, family, size, price_minor, attrs, status, sort
             FROM rec_products WHERE site = ? AND status = 'published' AND (? IS NULL OR section = ?)
             ORDER BY sort, name",
        )
        .bind(site)
        .bind(section)
        .bind(section)
        .fetch_all(self.pool())
        .await?;
        Ok(rows)
    }

    /// Every product of a site, drafts included, for the owner's Catalogue.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn all_products(&self, site: &str) -> anyhow::Result<Vec<Product>> {
        Ok(sqlx::query_as::<_, Product>(
            "SELECT sku, slug, section, name, description, family, size, price_minor, attrs, status, sort
             FROM rec_products WHERE site = ? ORDER BY section, sort, name",
        )
        .bind(site)
        .fetch_all(self.pool())
        .await?)
    }

    /// One product by slug.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn product_by_slug(&self, site: &str, slug: &str) -> anyhow::Result<Option<Product>> {
        Ok(sqlx::query_as::<_, Product>(
            "SELECT sku, slug, section, name, description, family, size, price_minor, attrs, status, sort
             FROM rec_products WHERE site = ? AND slug = ?",
        )
        .bind(site)
        .bind(slug)
        .fetch_optional(self.pool())
        .await?)
    }

    /// One product by sku.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn product(&self, site: &str, sku: &str) -> anyhow::Result<Option<Product>> {
        Ok(sqlx::query_as::<_, Product>(
            "SELECT sku, slug, section, name, description, family, size, price_minor, attrs, status, sort
             FROM rec_products WHERE site = ? AND sku = ?",
        )
        .bind(site)
        .bind(sku)
        .fetch_optional(self.pool())
        .await?)
    }

    /// The owner changes a product's price or status (the owner's Catalogue app).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn update_product(
        &self,
        site: &str,
        sku: &str,
        price_minor: i64,
        status: &str,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "UPDATE rec_products SET price_minor = ?, status = ? WHERE site = ? AND sku = ?",
        )
        .bind(price_minor)
        .bind(status)
        .bind(site)
        .bind(sku)
        .execute(self.pool())
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// Makes today's stock rows from the bake plan if the day has none yet. Batches in the plan
    /// are clock times; the first one after now is the next batch.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn ensure_day_stock(&self, site: &Site, day_ts: i64) -> anyhow::Result<()> {
        let day = time::day_of(day_ts, site.utc_offset_min);
        let plan: Vec<(String, i64, String)> =
            sqlx::query_as("SELECT sku, planned, batches FROM rec_bake_plan WHERE site = ?")
                .bind(&site.site)
                .fetch_all(self.pool())
                .await?;
        let start = time::day_start(day_ts, site.utc_offset_min);
        for (sku, planned, batches) in plan {
            let batches: Vec<String> = serde_json::from_str(&batches).unwrap_or_default();
            let first = batches
                .first()
                .and_then(|c| {
                    time::at_local(
                        time::local(start, site.utc_offset_min).date_naive(),
                        c,
                        site.utc_offset_min,
                    )
                    .ok()
                })
                .unwrap_or(start);
            sqlx::query(
                "INSERT OR IGNORE INTO rec_stock (site, sku, day, planned, next_batch_at, back_at, as_of, source)
                 VALUES (?, ?, ?, ?, NULL, NULL, ?, 'bake plan')",
            )
            .bind(&site.site)
            .bind(&sku)
            .bind(&day)
            .bind(planned)
            .bind(first)
            .execute(self.pool())
            .await?;
        }
        Ok(())
    }

    /// Stock for every product of a site, now. For a bakery this is today's bake: the plan, less
    /// today's active order lines and holds; for a shop, the last count less active orders since.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn stock(&self, site: &Site, now: i64) -> anyhow::Result<HashMap<String, Stock>> {
        let off = site.utc_offset_min;
        let day = time::day_of(now, off);
        if site.archetype == "bakery" {
            self.ensure_day_stock(site, now).await?;
        }
        let rows: Vec<(String, String, i64, Option<i64>, i64)> = sqlx::query_as(
            "SELECT sku, day, planned, back_at, as_of FROM rec_stock WHERE site = ? AND (day = ? OR day = '')",
        )
        .bind(&site.site)
        .bind(&day)
        .fetch_all(self.pool())
        .await?;
        let plan: HashMap<String, Vec<String>> = sqlx::query_as::<_, (String, String)>(
            "SELECT sku, batches FROM rec_bake_plan WHERE site = ?",
        )
        .bind(&site.site)
        .fetch_all(self.pool())
        .await?
        .into_iter()
        .map(|(s, b)| (s, serde_json::from_str(&b).unwrap_or_default()))
        .collect();
        let today = time::local(now, off).date_naive();
        let mut out = HashMap::new();
        for (sku, row_day, planned, back_at, as_of) in rows {
            let taken = if row_day.is_empty() {
                self.taken_since(&site.site, &sku, as_of).await?
            } else {
                self.taken_on_day(site, &sku, &row_day).await?
            };
            let left = (planned - taken).max(0);
            // Batches still to come today, by the clock.
            let upcoming: Vec<i64> = plan
                .get(&sku)
                .map(|b| {
                    b.iter()
                        .filter_map(|c| time::at_local(today, c, off).ok())
                        .filter(|t| *t > now)
                        .collect()
                })
                .unwrap_or_default();
            let next_batch_at = upcoming.first().copied();
            let state = if planned == 0 {
                "out"
            } else if left == 0 && next_batch_at.is_some() {
                "batch"
            } else if left == 0 {
                "out"
            } else if next_batch_at.is_some() && left <= 3 {
                "batch"
            } else if left <= 3 {
                "low"
            } else {
                "in"
            };
            let back_at = if state == "out" {
                back_at.or_else(|| {
                    plan.get(&sku)
                        .and_then(|b| b.first())
                        .and_then(|c| time::at_local(today.succ_opt()?, c, off).ok())
                })
            } else {
                None
            };
            out.insert(
                sku.clone(),
                Stock {
                    sku,
                    left: Some(left),
                    next_batch_at,
                    back_at,
                    as_of: as_of.min(now),
                    state,
                },
            );
        }
        Ok(out)
    }

    /// Units a day's active orders and holds took (orders count on their slot's day, or the day
    /// they were placed when collected the same day).
    async fn taken_on_day(&self, site: &Site, sku: &str, day: &str) -> anyhow::Result<i64> {
        let off = site.utc_offset_min * 60;
        let sql = format!(
            "SELECT COALESCE(SUM(l.qty), 0) FROM rec_order_lines l
             JOIN rec_orders o ON o.site = l.site AND o.order_id = l.order_id
             LEFT JOIN rec_slots s ON s.site = o.site AND s.slot_id = o.slot_id
             WHERE l.site = ? AND l.sku = ? AND o.state IN {ACTIVE}
               AND date(COALESCE(s.starts_at, o.created_at) + ?, 'unixepoch') = ?"
        );
        let (orders,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(&site.site)
            .bind(sku)
            .bind(off)
            .bind(day)
            .fetch_one(self.pool())
            .await?;
        let (holds,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(qty), 0) FROM rec_holds WHERE site = ? AND sku = ? AND day = ? AND state = 'held'",
        )
        .bind(&site.site)
        .bind(sku)
        .bind(day)
        .fetch_one(self.pool())
        .await?;
        Ok(orders + holds)
    }

    /// Units active orders took since a stock count.
    async fn taken_since(&self, site: &str, sku: &str, since: i64) -> anyhow::Result<i64> {
        let sql = format!(
            "SELECT COALESCE(SUM(l.qty), 0) FROM rec_order_lines l
             JOIN rec_orders o ON o.site = l.site AND o.order_id = l.order_id
             WHERE l.site = ? AND l.sku = ? AND o.state IN {ACTIVE} AND o.created_at >= ?"
        );
        let (n,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(site)
            .bind(sku)
            .bind(since)
            .fetch_one(self.pool())
            .await?;
        Ok(n)
    }

    /// The owner counts stock (the owner's Stock app): the count becomes the new baseline.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn set_stock(
        &self,
        site: &Site,
        sku: &str,
        count: i64,
        now: i64,
    ) -> anyhow::Result<()> {
        let day = if site.archetype == "bakery" {
            time::day_of(now, site.utc_offset_min)
        } else {
            String::new()
        };
        // For a bakery the plan counts today's orders, so the count is added to what they took.
        let base = if day.is_empty() {
            count
        } else {
            count + self.taken_on_day(site, sku, &day).await?
        };
        sqlx::query(
            "INSERT INTO rec_stock (site, sku, day, planned, next_batch_at, back_at, as_of, source)
             VALUES (?, ?, ?, ?, NULL, NULL, ?, 'owner count')
             ON CONFLICT (site, sku, day) DO UPDATE SET planned = excluded.planned, as_of = excluded.as_of, source = excluded.source, back_at = NULL",
        )
        .bind(&site.site)
        .bind(sku)
        .bind(&day)
        .bind(base)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}
