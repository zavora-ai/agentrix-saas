//! Who owns which site, and how to reach an owner: read from control.db.
//!
//! The first release keeps sites in `sites` (with `owner_account_id`) and owners in `accounts`
//! (control-plane lane). This module only reads them. When W6's `agentrix-identity` and W8's
//! stores take over, node swaps in another [`Directory`].

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};

/// A site, as messages need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteRef {
    /// Its id.
    pub id: String,
    /// Its name ("Kilele Bakery"), or the domain when it has none.
    pub name: String,
    /// Its domain.
    pub domain: String,
    /// Its owner's account, once claimed.
    pub owner_account: Option<String>,
    /// The site's offset from UTC, for showing times.
    pub utc_offset_minutes: i32,
}

/// How to reach an owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// Their account.
    pub account: String,
    /// Their email.
    pub email: String,
    /// The name they gave.
    pub name: String,
}

/// Sites and owners.
#[async_trait]
pub trait Directory: Send + Sync {
    /// The sites this account owns.
    async fn sites_of(&self, account: &str) -> anyhow::Result<Vec<SiteRef>>;
    /// One site.
    async fn site(&self, site_id: &str) -> anyhow::Result<Option<SiteRef>>;
    /// An owner's contact details.
    async fn owner(&self, account: &str) -> anyhow::Result<Option<Contact>>;
    /// The owner account with this email, if any.
    async fn owner_by_email(&self, email: &str) -> anyhow::Result<Option<Contact>>;
}

/// The [`Directory`] over the first release's `sites` and `accounts` tables.
#[derive(Clone)]
pub struct ControlDb {
    pool: SqlitePool,
}

impl ControlDb {
    /// Reads from `control.db`.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn site_from(row: &sqlx::sqlite::SqliteRow) -> Result<SiteRef, sqlx::Error> {
    let name: String = row.try_get("name")?;
    let domain: String = row.try_get("domain")?;
    Ok(SiteRef {
        id: row.try_get("site_id")?,
        name: if name.trim().is_empty() {
            domain.clone()
        } else {
            name
        },
        domain,
        owner_account: row.try_get("owner_account_id")?,
        utc_offset_minutes: row.try_get::<i64, _>("utc_offset_minutes")? as i32,
    })
}

#[async_trait]
impl Directory for ControlDb {
    async fn sites_of(&self, account: &str) -> anyhow::Result<Vec<SiteRef>> {
        let rows = sqlx::query(
            "SELECT site_id, domain, name, owner_account_id, utc_offset_minutes FROM sites
             WHERE owner_account_id = ? ORDER BY created_at",
        )
        .bind(account)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(site_from).collect::<Result<_, _>>()?)
    }

    async fn site(&self, site_id: &str) -> anyhow::Result<Option<SiteRef>> {
        let row = sqlx::query(
            "SELECT site_id, domain, name, owner_account_id, utc_offset_minutes FROM sites WHERE site_id = ?",
        )
        .bind(site_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(site_from).transpose()?)
    }

    async fn owner(&self, account: &str) -> anyhow::Result<Option<Contact>> {
        let row = sqlx::query("SELECT id, email, display_name FROM accounts WHERE id = ?")
            .bind(account)
            .fetch_optional(&self.pool)
            .await?;
        Ok(match row {
            Some(r) => Some(Contact {
                account: r.try_get("id")?,
                email: r.try_get("email")?,
                name: r.try_get("display_name")?,
            }),
            None => None,
        })
    }

    async fn owner_by_email(&self, email: &str) -> anyhow::Result<Option<Contact>> {
        let row = sqlx::query(
            "SELECT id, email, display_name FROM accounts WHERE email = ? COLLATE NOCASE",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(match row {
            Some(r) => Some(Contact {
                account: r.try_get("id")?,
                email: r.try_get("email")?,
                name: r.try_get("display_name")?,
            }),
            None => None,
        })
    }
}

/// A fixed directory for tests and previews.
#[derive(Debug, Clone, Default)]
pub struct Fixed {
    /// The sites.
    pub sites: Vec<SiteRef>,
    /// The owners.
    pub owners: Vec<Contact>,
}

#[async_trait]
impl Directory for Fixed {
    async fn sites_of(&self, account: &str) -> anyhow::Result<Vec<SiteRef>> {
        Ok(self
            .sites
            .iter()
            .filter(|s| s.owner_account.as_deref() == Some(account))
            .cloned()
            .collect())
    }

    async fn site(&self, site_id: &str) -> anyhow::Result<Option<SiteRef>> {
        Ok(self.sites.iter().find(|s| s.id == site_id).cloned())
    }

    async fn owner(&self, account: &str) -> anyhow::Result<Option<Contact>> {
        Ok(self.owners.iter().find(|o| o.account == account).cloned())
    }

    async fn owner_by_email(&self, email: &str) -> anyhow::Result<Option<Contact>> {
        Ok(self
            .owners
            .iter()
            .find(|o| o.email.eq_ignore_ascii_case(email))
            .cloned())
    }
}
