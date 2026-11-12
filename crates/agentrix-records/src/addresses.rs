//! Delivery addresses and recipients a customer keeps on a site (01a A-20). Checkout offers
//! them; the area is checked against the site's delivery zones before one is saved.

use serde::Serialize;

use crate::{Records, new_id};

/// A saved address.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Address {
    /// `adr-…`.
    pub address_id: String,
    /// The customer it belongs to.
    pub customer_id: String,
    /// Who receives deliveries there ("Amina").
    pub recipient: String,
    /// The recipient's phone, for the rider.
    pub phone: Option<String>,
    /// The area, as a zone lists it ("Kilimani").
    pub area: String,
    /// The zone that covers the area.
    pub zone: String,
    /// House, street, gate.
    pub details: String,
    /// Delivery notes.
    pub notes: String,
    /// When it was saved.
    pub created_at: i64,
}

/// An address to save.
#[derive(Debug, Clone, Default)]
pub struct NewAddress {
    /// Who receives deliveries there.
    pub recipient: String,
    /// Their phone.
    pub phone: Option<String>,
    /// The area, as a zone lists it.
    pub area: String,
    /// The zone that covers it.
    pub zone: String,
    /// House, street, gate.
    pub details: String,
    /// Delivery notes.
    pub notes: String,
}

const COLS: &str =
    "address_id, customer_id, recipient, phone, area, zone, details, notes, created_at";

impl Records {
    /// A customer's saved addresses, oldest first.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn addresses(&self, site: &str, customer_id: &str) -> anyhow::Result<Vec<Address>> {
        Ok(sqlx::query_as::<_, Address>(sql!(
            "SELECT {COLS} FROM rec_addresses WHERE site = ? AND customer_id = ? ORDER BY created_at, address_id"
        ))
        .bind(site)
        .bind(customer_id)
        .fetch_all(self.pool())
        .await?)
    }

    /// Saves an address for a customer.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn add_address(
        &self,
        site: &str,
        customer_id: &str,
        a: &NewAddress,
        now: i64,
    ) -> anyhow::Result<Address> {
        let address = Address {
            address_id: new_id("adr"),
            customer_id: customer_id.to_owned(),
            recipient: a.recipient.clone(),
            phone: a.phone.clone(),
            area: a.area.clone(),
            zone: a.zone.clone(),
            details: a.details.clone(),
            notes: a.notes.clone(),
            created_at: now,
        };
        sqlx::query(
            "INSERT INTO rec_addresses (site, address_id, customer_id, recipient, phone, area, zone, details, notes, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(site)
        .bind(&address.address_id)
        .bind(&address.customer_id)
        .bind(&address.recipient)
        .bind(&address.phone)
        .bind(&address.area)
        .bind(&address.zone)
        .bind(&address.details)
        .bind(&address.notes)
        .bind(now)
        .execute(self.pool())
        .await?;
        Ok(address)
    }

    /// Removes one of a customer's addresses; false when it isn't theirs.
    ///
    /// # Errors
    ///
    /// On a database error.
    pub async fn remove_address(
        &self,
        site: &str,
        customer_id: &str,
        address_id: &str,
    ) -> anyhow::Result<bool> {
        let done = sqlx::query(
            "DELETE FROM rec_addresses WHERE site = ? AND customer_id = ? AND address_id = ?",
        )
        .bind(site)
        .bind(customer_id)
        .bind(address_id)
        .execute(self.pool())
        .await?;
        Ok(done.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn addresses_belong_to_one_customer() {
        let r = Records::open_in_memory().await.unwrap();
        let a = NewAddress {
            recipient: "Amina".into(),
            area: "Kilimani".into(),
            zone: "zone1".into(),
            ..Default::default()
        };
        let saved = r.add_address("s", "cus-njeri", &a, 10).await.unwrap();
        assert_eq!(r.addresses("s", "cus-njeri").await.unwrap().len(), 1);
        assert!(r.addresses("s", "cus-other").await.unwrap().is_empty());
        assert!(
            !r.remove_address("s", "cus-other", &saved.address_id)
                .await
                .unwrap()
        );
        assert!(
            r.remove_address("s", "cus-njeri", &saved.address_id)
                .await
                .unwrap()
        );
    }
}
