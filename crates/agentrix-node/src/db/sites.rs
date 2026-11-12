//! The `sites` table. This module is its one writer: assess/ creates sites through
//! [`create_or_get`], and claim/ reads the domain and sets the owner through [`domain_of`] and
//! [`set_owner`] (its `SiteDirectory`).

use agentrix_types::SiteId;
use sqlx::SqlitePool;

use super::now;

/// The site for `domain`, creating it with no owner if it is new. Domains compare without
/// case, so `Bakery.example` and `bakery.example` are one site.
pub async fn create_or_get(
    pool: &SqlitePool,
    domain: &str,
    old_origin: &str,
    fixture: bool,
) -> Result<SiteId, sqlx::Error> {
    let fresh = SiteId::generate();
    sqlx::query(
        "INSERT INTO sites (site_id, domain, old_origin, fixture, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (domain) DO NOTHING",
    )
    .bind(fresh.as_str())
    .bind(domain)
    .bind(old_origin)
    .bind(i64::from(fixture))
    .bind(now())
    .execute(pool)
    .await?;
    let (site_id,): (String,) = sqlx::query_as("SELECT site_id FROM sites WHERE domain = ?")
        .bind(domain)
        .fetch_one(pool)
        .await?;
    Ok(SiteId::new(site_id))
}

/// The site's domain, if the site exists.
pub async fn domain_of(pool: &SqlitePool, site: &SiteId) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT domain FROM sites WHERE site_id = ?")
        .bind(site.as_str())
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(domain,)| domain))
}

/// Records `account` (an `accounts.id`) as the site's owner once a claim is verified.
/// `RowNotFound` if the site does not exist; a foreign-key error if the account does not.
pub async fn set_owner(pool: &SqlitePool, site: &SiteId, account: &str) -> Result<(), sqlx::Error> {
    let done = sqlx::query("UPDATE sites SET owner_account_id = ? WHERE site_id = ?")
        .bind(account)
        .bind(site.as_str())
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_domain_is_one_site_and_a_claim_sets_its_owner() {
        let pool = super::super::open_in_memory().await.unwrap();
        let site = create_or_get(&pool, "bakery.example", "https://bakery.example", false)
            .await
            .unwrap();
        let again = create_or_get(&pool, "Bakery.Example", "https://bakery.example", false)
            .await
            .unwrap();
        assert_eq!(site, again);
        assert_eq!(
            domain_of(&pool, &site).await.unwrap().as_deref(),
            Some("bakery.example")
        );

        sqlx::query("INSERT INTO accounts (id, email, display_name, created_at) VALUES ('acct_1', 'o@example.com', 'Owner', 0)")
            .execute(&pool)
            .await
            .unwrap();
        set_owner(&pool, &site, "acct_1").await.unwrap();
        assert!(set_owner(&pool, &site, "acct_missing").await.is_err());
        assert!(matches!(
            set_owner(&pool, &SiteId::new("site_missing"), "acct_1").await,
            Err(sqlx::Error::RowNotFound)
        ));
    }
}
