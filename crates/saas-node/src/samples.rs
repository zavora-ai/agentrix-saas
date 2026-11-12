//! The join that matters (v3): a site is a company, and a sample site's PayPal capture posts to
//! that company's wallet and books.
//!
//! The sample sites take payments through node's own checkout (agentrix-samples over
//! agentrix-effects), which keeps each one in `control.db`'s `customer_payments`. [`follow`]
//! reads the captured ones for each joined site and records each capture in the company's
//! Cashier wallet ([`saas_wallet::Wallets::record`]): the wallet reads the capture back from
//! PayPal, writes the entry, and posts it to the company's books (Zavora ERA) once by PayPal id.
//! Nothing here trusts `control.db` for an amount; it only names the capture.
//!
//! PayPal's webhooks for those captures (and their refunds) arrive at Agentrix's runtime, since
//! the sites use Agentrix's PayPal app. [`SiteSales`] tells the runtime which ones are a site's
//! own sale, and keeps each as that sale's confirmation in the site's records instead of looking
//! for an invoice in Agentrix's books.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use saas_core::paypal::{PayPalCaptureId, PayPalOrderId, PayPalRef};
use saas_core::wallet::{Authority, WalletCause};
use saas_core::{AgentId, CompanyId};
use saas_wallet::{Context, Wallets};

/// The sample sites that are companies on this node: (site id in `control.db`, company).
pub fn joined() -> Vec<(&'static str, CompanyId)> {
    vec![("kilele-bakery-sample", CompanyId::kilele())]
}

/// One captured payment of a site, as `control.db` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    /// PayPal's capture id.
    pub capture: String,
    /// PayPal's order id.
    pub order: Option<String>,
    /// The site's own order reference.
    pub reference: String,
}

/// The site's captured payments, oldest first.
pub async fn captured(control: &sqlx::SqlitePool, site: &str) -> anyhow::Result<Vec<Captured>> {
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(
        "SELECT capture_id, provider_ref, order_id FROM customer_payments \
         WHERE site_id = ? AND provider = 'paypal' AND capture_id IS NOT NULL \
         AND state IN ('captured', 'partly_refunded', 'refunded') ORDER BY created_at",
    )
    .bind(site)
    .fetch_all(control)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(capture, order, reference)| Captured {
            capture,
            order,
            reference,
        })
        .collect())
}

/// A site's sale a PayPal event is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaleEvent {
    /// The site (`control.db`'s `site_id`).
    pub site: String,
    /// The site's order.
    pub order: String,
    /// `capture` or `refund`.
    pub kind: &'static str,
    /// PayPal's capture or refund id.
    pub object: String,
}

/// The capture a refund resource names: its `up` link, `…/v2/payments/captures/{id}`.
fn refunded_capture(resource: &serde_json::Value) -> Option<String> {
    resource["links"]
        .as_array()?
        .iter()
        .find(|l| l["rel"] == "up")?["href"]
        .as_str()?
        .split("/captures/")
        .nth(1)
        .map(|rest| rest.trim_end_matches('/').to_owned())
        .filter(|id| !id.is_empty())
}

/// Which site's sale a verified PayPal capture or refund event is about, from `control.db`'s
/// `customer_payments`; `None` when it is no site's.
pub async fn sale_for(
    control: &sqlx::SqlitePool,
    event: &serde_json::Value,
) -> anyhow::Result<Option<SaleEvent>> {
    let resource = &event["resource"];
    let (kind, capture, object) = match event["event_type"].as_str() {
        Some("PAYMENT.CAPTURE.COMPLETED") => match resource["id"].as_str() {
            Some(id) => ("capture", id.to_owned(), id.to_owned()),
            None => return Ok(None),
        },
        Some("PAYMENT.CAPTURE.REFUNDED") => {
            match (refunded_capture(resource), resource["id"].as_str()) {
                (Some(capture), Some(id)) => ("refund", capture, id.to_owned()),
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT site_id, order_id FROM customer_payments WHERE provider = 'paypal' AND capture_id = ? LIMIT 1",
    )
    .bind(&capture)
    .fetch_optional(control)
    .await?;
    Ok(row.map(|(site, order)| SaleEvent {
        site,
        order,
        kind,
        object,
    }))
}

/// The sample sites' own sales, for Agentrix's runtime ([`saas_company::runtime::SiteSales`]).
#[derive(Debug, Clone)]
pub struct SiteSales {
    control: sqlx::SqlitePool,
}

impl SiteSales {
    /// Over node's `control.db`.
    pub fn new(control: sqlx::SqlitePool) -> Self {
        Self { control }
    }
}

#[async_trait::async_trait]
impl saas_company::runtime::SiteSales for SiteSales {
    async fn confirm(&self, event: &serde_json::Value) -> anyhow::Result<Option<String>> {
        let Some(sale) = sale_for(&self.control, event).await? else {
            return Ok(None);
        };
        let confirmation = agentrix_records::confirmations::PayPalConfirmation {
            event_id: event["id"].as_str().unwrap_or_default().to_owned(),
            order_id: sale.order.clone(),
            event_type: event["event_type"].as_str().unwrap_or_default().to_owned(),
            kind: sale.kind.to_owned(),
            object_id: sale.object.clone(),
            status: event["resource"]["status"]
                .as_str()
                .unwrap_or("COMPLETED")
                .to_owned(),
            confirmed_at: chrono::Utc::now().timestamp(),
        };
        // It is the site's sale either way; only the record of PayPal's word can wait.
        match agentrix_samples::held::confirm_paypal(&sale.site, &confirmation).await {
            Ok(fresh) => tracing::info!(
                site = %sale.site, order = %sale.order, kind = sale.kind, id = %sale.object, fresh,
                "PayPal confirmed a site's sale"
            ),
            Err(e) => tracing::warn!(
                site = %sale.site, order = %sale.order, id = %sale.object, error = %format!("{e:#}"),
                "a site's sale was confirmed by PayPal, but the site's records didn't take it"
            ),
        }
        Ok(Some(format!(
            "{} {} confirms {} order {}",
            sale.kind, sale.object, sale.site, sale.order
        )))
    }
}

/// The company's Cashier, whose wallet a sale goes in.
pub fn cashier(company: &CompanyId) -> anyhow::Result<AgentId> {
    Ok(AgentId::new(company.clone(), AgentId::CASHIER)?)
}

/// Records one capture in the Cashier's wallet (and so in the books). Returns whether anything
/// new was written.
pub async fn record(wallets: &Wallets, sale: &Captured) -> anyhow::Result<bool> {
    let wallet = cashier(wallets.company())?;
    let ctx = Context {
        acting: wallet.clone(),
        authority: Authority::PayerApproved,
        cause: match &sale.order {
            Some(order) => Some(WalletCause::Order {
                order: PayPalOrderId::new(order.clone())?,
                reference: Some(sale.reference.clone()),
            }),
            None => None,
        },
        counterparty: None,
    };
    let object = PayPalRef::Capture(PayPalCaptureId::new(sale.capture.clone())?);
    let recorded = wallets.record(&wallet, &object, &ctx).await?;
    Ok(!recorded.already)
}

/// Follows `site`'s captures in `control.db` into `wallets`, every `every`, forever. A capture is
/// read back from PayPal once per run of the node; the wallet itself never writes one twice.
pub async fn follow(
    control: sqlx::SqlitePool,
    site: &'static str,
    wallets: Arc<Wallets>,
    every: Duration,
) {
    let mut done: BTreeSet<String> = match cashier(wallets.company()) {
        Ok(wallet) => wallets
            .entries(&wallet)
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .filter_map(|e| match e.paypal {
                        PayPalRef::Capture(id) => Some(id.to_string()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => BTreeSet::new(),
    };
    loop {
        match captured(&control, site).await {
            Ok(sales) => {
                let fresh: Vec<Captured> = sales
                    .into_iter()
                    .filter(|s| !done.contains(&s.capture))
                    .collect();
                for sale in fresh {
                    match record(&wallets, &sale).await {
                        Ok(fresh) => {
                            if fresh {
                                tracing::info!(site, capture = %sale.capture, "a sample-site sale is in the company's wallet and books");
                            }
                            done.insert(sale.capture);
                        }
                        Err(e) => {
                            tracing::warn!(site, capture = %sale.capture, error = %format!("{e:#}"), "a sample-site sale isn't in the wallet yet; trying again");
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(site, error = %e, "couldn't read the site's payments"),
        }
        tokio::time::sleep(every).await;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    async fn control() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        agentrix_effects::migrate(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO customer_payments (id, site_id, order_id, fingerprint, provider, provider_ref, currency, amount_minor, state, capture_id, captured_minor, sandbox, created_at, updated_at)
             VALUES ('pay_1', 'kilele-bakery-sample', 'KB-1055', 'sha256:1', 'paypal', 'ORDER-1', 'USD', 171, 'captured', '8F698678SM3689049', 171, 1, 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn a_capture_or_its_refund_is_found_among_the_sites_sales() {
        let pool = control().await;
        let capture = json!({
            "id": "WH-1", "event_type": "PAYMENT.CAPTURE.COMPLETED",
            "resource": { "id": "8F698678SM3689049", "status": "COMPLETED" },
        });
        assert_eq!(
            sale_for(&pool, &capture).await.unwrap(),
            Some(SaleEvent {
                site: "kilele-bakery-sample".into(),
                order: "KB-1055".into(),
                kind: "capture",
                object: "8F698678SM3689049".into(),
            })
        );
        let refund = json!({
            "id": "WH-2", "event_type": "PAYMENT.CAPTURE.REFUNDED",
            "resource": { "id": "1JU08902781691411", "status": "COMPLETED", "links": [
                { "rel": "self", "href": "https://api.sandbox.paypal.com/v2/payments/refunds/1JU08902781691411" },
                { "rel": "up", "href": "https://api.sandbox.paypal.com/v2/payments/captures/8F698678SM3689049" },
            ]},
        });
        let found = sale_for(&pool, &refund).await.unwrap().unwrap();
        assert_eq!((found.kind, found.object.as_str()), ("refund", "1JU08902781691411"));
        assert_eq!(found.order, "KB-1055");

        let other = json!({
            "id": "WH-3", "event_type": "PAYMENT.CAPTURE.COMPLETED",
            "resource": { "id": "SOMEONE-ELSES" },
        });
        assert_eq!(sale_for(&pool, &other).await.unwrap(), None);
        let payout = json!({ "id": "WH-4", "event_type": "PAYMENT.PAYOUTS-ITEM.SUCCEEDED", "resource": {} });
        assert_eq!(sale_for(&pool, &payout).await.unwrap(), None);
    }
}
