//! The money layer's hourly calls, for the company's schedules.
//!
//! - [`reconcile`]: a site's wallet record read back against PayPal, with a signed checkpoint
//!   over it ([`saas_wallet::Wallets::reconcile`]): it matches, or these entries drifted.
//! - [`bill_model_spend`]: what node's model gateway metered in `control.db` for a company's
//!   sites, as the providers' vendor bills in its books (Anthropic, Vultr), once per window
//!   ([`saas_core::spend`]). Payables then pays them by PayPal Payout, inside the owner's
//!   mandate.
//!
//! Nothing here trusts `control.db` for more than the gateway's own meter: each usage row's cost
//! is the gateway's estimate from the provider's prices, summed per model.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use saas_core::spend::{self, ModelSpend, ModelUsage, SpendWindow};
use saas_core::{Books, CompanyId};
use sqlx::SqlitePool;

/// The hourly reconciliation of `site`'s wallet record (face's site id: `kilele`, `agentrix`).
pub async fn reconcile(site: &str) -> anyhow::Result<saas_wallet::Reconcile> {
    let wallets = crate::os::shared()
        .wallets(site)
        .ok_or_else(|| anyhow::anyhow!("{site}'s wallet isn't started yet"))?;
    Ok(wallets.reconcile().await?)
}

/// The sites in `control.db` whose model calls are the company's: the site its agents run
/// under, and the sample site that is the company.
pub fn sites_of(company: &CompanyId) -> Vec<String> {
    let own = if *company == CompanyId::kilele() {
        "saas-kilele"
    } else {
        "saas-agentrix"
    };
    let mut out = vec![own.to_owned()];
    out.extend(
        crate::samples::joined()
            .into_iter()
            .filter(|(_, c)| c == company)
            .map(|(site, _)| site.to_owned()),
    );
    out
}

/// The providers' PayPal emails for Payables' Payouts, from `SAAS_PAYEE_ANTHROPIC` and
/// `SAAS_PAYEE_VULTR` (sandbox accounts while PayPal is on sandbox). A provider without one is
/// still billed; its vendor just has no Payout email yet.
pub fn payees_from_env() -> BTreeMap<String, String> {
    [
        ("anthropic", "SAAS_PAYEE_ANTHROPIC"),
        ("vultr", "SAAS_PAYEE_VULTR"),
    ]
    .into_iter()
    .filter_map(|(provider, key)| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty() && !v.starts_with('#'))
            .map(|v| (provider.to_owned(), v))
    })
    .collect()
}

/// A time as `control.db` stores it: RFC 3339 UTC with microseconds, so text order is time
/// order.
fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Whether `usage` has W8's `units` and `unit` columns. Node's `control.db` does; one made from
/// migration 0001 alone doesn't.
async fn has_units(control: &SqlitePool) -> anyhow::Result<bool> {
    let columns: Vec<(String,)> = sqlx::query_as("SELECT name FROM pragma_table_info('usage')")
        .fetch_all(control)
        .await?;
    let has = |name: &str| columns.iter().any(|(c,)| c == name);
    Ok(has("units") && has("unit"))
}

/// The providers with metered spend for `sites` since `since`.
pub async fn providers(
    control: &SqlitePool,
    sites: &[String],
    since: DateTime<Utc>,
) -> anyhow::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT provider FROM usage \
         WHERE site_id IN (SELECT value FROM json_each(?)) AND at >= ? AND cost_micro_usd > 0 \
         ORDER BY provider",
    )
    .bind(serde_json::to_string(sites)?)
    .bind(stamp(since))
    .fetch_all(control)
    .await?;
    Ok(rows.into_iter().map(|(p,)| p).collect())
}

/// One provider's metered usage for `sites` in `window`, one group per model (and unit), the
/// most costly first.
pub async fn usage(
    control: &SqlitePool,
    sites: &[String],
    provider: &str,
    window: &SpendWindow,
) -> anyhow::Result<Vec<ModelUsage>> {
    let sql = if has_units(control).await? {
        "SELECT model, unit, COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0), \
         COALESCE(SUM(units), 0), SUM(cost_micro_usd) FROM usage \
         WHERE site_id IN (SELECT value FROM json_each(?)) AND provider = ? AND at >= ? AND at < ? \
         AND cost_micro_usd > 0 GROUP BY model, unit ORDER BY SUM(cost_micro_usd) DESC, model"
    } else {
        "SELECT model, NULL, COUNT(*), COALESCE(SUM(tokens_in), 0), COALESCE(SUM(tokens_out), 0), \
         0, SUM(cost_micro_usd) FROM usage \
         WHERE site_id IN (SELECT value FROM json_each(?)) AND provider = ? AND at >= ? AND at < ? \
         AND cost_micro_usd > 0 GROUP BY model ORDER BY SUM(cost_micro_usd) DESC, model"
    };
    type Row = (String, Option<String>, i64, i64, i64, i64, i64);
    let rows: Vec<Row> = sqlx::query_as(sql)
        .bind(serde_json::to_string(sites)?)
        .bind(provider)
        .bind(stamp(window.from))
        .bind(stamp(window.until))
        .fetch_all(control)
        .await?;
    let n = |v: i64| u64::try_from(v).unwrap_or(0);
    Ok(rows
        .into_iter()
        .map(
            |(model, unit, calls, tokens_in, tokens_out, units, cost)| ModelUsage {
                provider: provider.to_owned(),
                model,
                unit,
                calls: n(calls),
                tokens_in: n(tokens_in),
                tokens_out: n(tokens_out),
                units: n(units),
                cost_micro_usd: cost.max(0),
            },
        )
        .collect())
}

/// Bills the company's model spend from `since` (or from where each provider's last bill ended,
/// when later) to the start of this hour: one posted vendor bill per provider with spend,
/// found again rather than written twice when the same window comes round. `payees` are the
/// providers' PayPal emails for Payables' Payouts, by provider (`anthropic`, `vultr`).
pub async fn bill_model_spend(
    control: &SqlitePool,
    books: &dyn Books,
    company: &CompanyId,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
    payees: &BTreeMap<String, String>,
) -> anyhow::Result<ModelSpend> {
    let sites = sites_of(company);
    let ends = spend::billed_until(books).await?;
    let mut out = ModelSpend::default();
    for provider in providers(control, &sites, since).await? {
        let window = spend::next_window(ends.get(&provider).copied(), since, now);
        if window.is_empty() {
            continue;
        }
        let rows = usage(control, &sites, &provider, &window).await?;
        let payee = payees.get(&provider).cloned();
        match spend::bill_model_spend(books, &provider, &window, &rows, &sites, payee).await? {
            Some(bill) => {
                tracing::info!(
                    company = %company,
                    provider = %provider,
                    bill = bill.bill.number.as_deref().unwrap_or("-"),
                    total = %bill.bill.total,
                    already = bill.already,
                    "model spend billed"
                );
                out.bills.push(bill);
            }
            None => {
                let micro: i64 = rows.iter().map(|u| u.cost_micro_usd).sum();
                if micro > 0 {
                    out.waiting.insert(provider, micro);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use saas_core::books::{DocumentFilter, DocumentStatus};
    use saas_core::fake::FakeBooks;
    use saas_core::money::Money;

    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 11, h, m, 0).unwrap()
    }

    async fn control(dir: &std::path::Path) -> SqlitePool {
        let path = dir.join("control.db");
        agentrix_models::init_control_db(
            &path,
            &[
                ("saas-kilele", "kilele.example"),
                ("kilele-bakery-sample", "kilele-bakery-sample.example"),
                ("saas-agentrix", "pay.example"),
            ],
        )
        .await
        .unwrap();
        SqlitePool::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap()
    }

    async fn call(
        pool: &SqlitePool,
        site: &str,
        provider: &str,
        model: &str,
        cost: i64,
        when: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO usage (site_id, purpose, provider, model, tokens_in, tokens_out, \
             cost_micro_usd, outcome, at) VALUES (?, 'concierge', ?, ?, 1000, 200, ?, 'ok', ?)",
        )
        .bind(site)
        .bind(provider)
        .bind(model)
        .bind(cost)
        .bind(stamp(when))
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_gateways_meter_becomes_one_bill_per_provider_and_window() {
        let dir = tempfile::tempdir().unwrap();
        let pool = control(dir.path()).await;
        let k = "kilele-bakery-sample";
        call(
            &pool,
            "saas-kilele",
            "anthropic",
            "claude-sonnet-5-5",
            600_000,
            at(7, 5),
        )
        .await;
        call(
            &pool,
            k,
            "anthropic",
            "claude-sonnet-5-5",
            400_000,
            at(8, 10),
        )
        .await;
        call(&pool, k, "vultr", "llama-4-scout", 250_000, at(8, 20)).await;
        // Another company's call, and one in the hour not yet over: neither is billed.
        call(
            &pool,
            "saas-agentrix",
            "anthropic",
            "claude-sonnet-5-5",
            900_000,
            at(8, 0),
        )
        .await;
        call(&pool, k, "vultr", "llama-4-scout", 70_000, at(9, 30)).await;

        let books = FakeBooks::new(CompanyId::kilele());
        let none = BTreeMap::new();
        let kilele = CompanyId::kilele();
        let run = bill_model_spend(&pool, &books, &kilele, at(6, 0), at(9, 41), &none)
            .await
            .unwrap();
        assert_eq!(run.bills.len(), 2, "{run:?}");
        let a = &run.bills[0];
        assert_eq!(a.provider, "anthropic");
        assert_eq!(a.bill.total, Money::usd(100));
        assert_eq!(a.usage[0].calls, 2);
        assert_eq!(a.bill.status, DocumentStatus::Posted);
        assert_eq!(run.bills[1].bill.total, Money::usd(25));

        // The same hour again: nothing new. The next hour: only the new call.
        let again = bill_model_spend(&pool, &books, &kilele, at(6, 0), at(9, 55), &none)
            .await
            .unwrap();
        assert!(again.bills.is_empty(), "{again:?}");
        let next = bill_model_spend(&pool, &books, &kilele, at(6, 0), at(10, 3), &none)
            .await
            .unwrap();
        assert_eq!(next.bills.len(), 1);
        assert_eq!(next.bills[0].bill.total, Money::usd(7));
        assert_eq!(
            next.bills[0].window,
            SpendWindow {
                from: at(9, 0),
                until: at(10, 0)
            }
        );
        let bills = books.bills(&DocumentFilter::default()).await.unwrap();
        assert_eq!(bills.len(), 3);
    }
}
