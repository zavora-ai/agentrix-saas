//! PayPal's own word on a site's sale. The sample sites take PayPal through Agentrix's app, so
//! PayPal's webhook for a site's capture or refund arrives at the platform; once verified, it is
//! kept here on the order it confirms, once per PayPal event id, with a line on the order's
//! timeline. It posts nothing: the sale was booked when it was captured.

use serde::Serialize;

use crate::Records;

/// One verified PayPal event about a site's sale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct PayPalConfirmation {
    /// PayPal's event id, `WH-…`.
    pub event_id: String,
    /// The site's order.
    pub order_id: String,
    /// `PAYMENT.CAPTURE.COMPLETED`, `PAYMENT.CAPTURE.REFUNDED` …
    pub event_type: String,
    /// `capture` or `refund`.
    pub kind: String,
    /// PayPal's capture or refund id.
    pub object_id: String,
    /// PayPal's status on it, such as `COMPLETED`.
    pub status: String,
    /// When it arrived (Unix seconds).
    pub confirmed_at: i64,
}

impl Records {
    /// Keeps `c` on its order, once per event id, with a timeline line the first time. Returns
    /// whether it was new (a repeated delivery changes nothing).
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn confirm_paypal(&self, site: &str, c: &PayPalConfirmation) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "INSERT OR IGNORE INTO rec_paypal_confirmations (site, event_id, order_id, event_type, kind, object_id, status, confirmed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(&c.event_id)
        .bind(&c.order_id)
        .bind(&c.event_type)
        .bind(&c.kind)
        .bind(&c.object_id)
        .bind(&c.status)
        .bind(c.confirmed_at)
        .execute(self.pool())
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
        let known: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM rec_orders WHERE site = ? AND order_id = ?")
                .bind(site)
                .bind(&c.order_id)
                .fetch_optional(self.pool())
                .await?;
        if known.is_some() {
            self.note_event(
                site,
                &c.order_id,
                "paypal_confirmed",
                "PayPal",
                &format!(
                    "PayPal confirmed the {} {}: {}.",
                    c.kind, c.object_id, c.status
                ),
                c.confirmed_at,
            )
            .await?;
        }
        Ok(true)
    }

    /// What PayPal confirmed on an order, oldest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn paypal_confirmations(
        &self,
        site: &str,
        order_id: &str,
    ) -> anyhow::Result<Vec<PayPalConfirmation>> {
        Ok(sqlx::query_as(
            "SELECT event_id, order_id, event_type, kind, object_id, status, confirmed_at FROM rec_paypal_confirmations
             WHERE site = ? AND order_id = ? ORDER BY confirmed_at, event_id",
        )
        .bind(site)
        .bind(order_id)
        .fetch_all(self.pool())
        .await?)
    }
}
