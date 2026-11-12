//! Notification preferences: which channels an owner wants each family of operational
//! messages on (`message_preferences`, migration 1001).
//!
//! Only operational messages can be turned off. Codes, security alerts, receipts, invoices and
//! P1 alarms always arrive, so nobody can quietly switch off an owner's alarms.

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::catalogue::{Channel, Family, Kind, MessageDef, Priority};

/// The families an owner can tune, in the order the settings page lists them.
pub const TUNABLE: [Family; 4] = [
    Family::OwnerOperations,
    Family::OwnerOnboarding,
    Family::OwnerPlatform,
    Family::OwnerData,
];

/// The stored form of a family.
pub fn family_key(f: Family) -> &'static str {
    match f {
        Family::CustomerAccount => "customer_account",
        Family::CustomerOrders => "customer_orders",
        Family::CustomerData => "customer_data",
        Family::Digest => "digest",
        Family::OwnerAccount => "owner_account",
        Family::OwnerOnboarding => "owner_onboarding",
        Family::OwnerOperations => "owner_operations",
        Family::OwnerBilling => "owner_billing",
        Family::OwnerPlatform => "owner_platform",
        Family::OwnerData => "owner_data",
        Family::Prospect => "prospect",
    }
}

/// Whether a person may turn this message off on a channel.
pub fn can_turn_off(def: &MessageDef) -> bool {
    def.kind == Kind::Operational && def.priority != Priority::P1
}

/// One family's channels, for the settings page.
#[derive(Debug, Clone, Serialize)]
pub struct FamilyPrefs {
    /// The stored key.
    pub key: &'static str,
    /// The heading.
    pub label: &'static str,
    /// What it covers, in words.
    pub covers: String,
    /// On or off per channel, in [`Channel::ALL`] order.
    pub channels: Vec<(String, bool)>,
}

/// The preferences store.
#[derive(Clone)]
pub struct Preferences {
    pool: SqlitePool,
}

impl Preferences {
    /// Over `control.db`, migrated.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Whether `account` wants `def` on `channel`. Always true for messages that can't be
    /// turned off.
    pub async fn wants(
        &self,
        account: &str,
        def: &MessageDef,
        channel: Channel,
    ) -> Result<bool, sqlx::Error> {
        if !can_turn_off(def) {
            return Ok(true);
        }
        let row = sqlx::query(
            "SELECT enabled FROM message_preferences WHERE account_id = ? AND family = ? AND channel = ?",
        )
        .bind(account)
        .bind(family_key(def.family))
        .bind(channel.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            Some(r) => r.try_get::<i64, _>("enabled")? == 1,
            None => true,
        })
    }

    /// Sets one family's channel.
    pub async fn set(
        &self,
        account: &str,
        family: Family,
        channel: Channel,
        enabled: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO message_preferences (account_id, family, channel, enabled, updated_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (account_id, family, channel) DO UPDATE SET enabled = excluded.enabled, updated_at = excluded.updated_at",
        )
        .bind(account)
        .bind(family_key(family))
        .bind(channel.as_str())
        .bind(i64::from(enabled))
        .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every tunable family with its channels, for the settings page.
    pub async fn for_account(&self, account: &str) -> Result<Vec<FamilyPrefs>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT family, channel, enabled FROM message_preferences WHERE account_id = ?",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;
        let off: Vec<(String, String)> = rows
            .iter()
            .filter(|r| r.try_get::<i64, _>("enabled").unwrap_or(1) == 0)
            .map(|r| {
                (
                    r.try_get::<String, _>("family").unwrap_or_default(),
                    r.try_get::<String, _>("channel").unwrap_or_default(),
                )
            })
            .collect();
        Ok(TUNABLE
            .iter()
            .map(|&family| {
                let key = family_key(family);
                let names: Vec<&str> = crate::catalogue::all()
                    .iter()
                    .filter(|d| d.family == family && can_turn_off(d))
                    .map(|d| d.trigger)
                    .collect();
                FamilyPrefs {
                    key,
                    label: family.label(),
                    covers: covers(family, names.len()),
                    channels: Channel::ALL
                        .iter()
                        .map(|c| {
                            let on = !off.iter().any(|(f, ch)| f == key && ch == c.as_str());
                            (c.as_str().to_string(), on)
                        })
                        .collect(),
                }
            })
            .collect())
    }
}

fn covers(family: Family, n: usize) -> String {
    let what = match family {
        Family::OwnerOperations => {
            "The morning brief, weekly review, decisions waiting, new orders, visitors' questions, budget and mission reports"
        }
        Family::OwnerOnboarding => {
            "The claim reminder, Move updates, going live and the old site's retirement"
        }
        Family::OwnerPlatform => "Notices from Zavora about the platform",
        Family::OwnerData => "Erasures that finish",
        _ => "",
    };
    let kinds = if n == 1 {
        "1 kind".to_string()
    } else {
        format!("{n} kinds")
    };
    format!("{what} ({kinds})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue;

    #[tokio::test]
    async fn only_operational_messages_can_be_turned_off() {
        let prefs = Preferences::new(crate::outbox::tests::pool().await);
        let brief = catalogue::get("owner.morning_brief").unwrap();
        let alarm = catalogue::get("owner.alarm").unwrap();
        let device = catalogue::get("owner.new_device").unwrap();
        assert!(prefs.wants("a", brief, Channel::Email).await.unwrap());
        prefs
            .set("a", Family::OwnerOperations, Channel::Email, false)
            .await
            .unwrap();
        assert!(!prefs.wants("a", brief, Channel::Email).await.unwrap());
        assert!(prefs.wants("a", brief, Channel::Push).await.unwrap());
        assert!(
            prefs.wants("a", alarm, Channel::Email).await.unwrap(),
            "P1 alarms always arrive"
        );
        assert!(
            prefs.wants("a", device, Channel::Email).await.unwrap(),
            "security always arrives"
        );
        assert!(
            prefs.wants("b", brief, Channel::Email).await.unwrap(),
            "other accounts unaffected"
        );
        let page = prefs.for_account("a").await.unwrap();
        assert_eq!(page[0].key, "owner_operations");
        assert_eq!(page[0].channels[0], ("email".to_string(), false));
    }
}
