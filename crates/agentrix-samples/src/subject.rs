//! W2's records about a customer, for W6's "Download my data" and "Delete my account"
//! (ADR-028): the customer's details, saved addresses, and orders with their receipts.
//!
//! W6 scopes each request to one site (`app_names = [site id]`) and the customer's account
//! (`vac_…`), which W2 stores as the order's `customer_id`.
//!
//! Erasure refuses while an order is still open, with W6's open-orders code, so a customer
//! can't delete an account with a cake still being baked. Otherwise it deletes the saved
//! addresses and the customer record. Orders and receipts stay as the business's sales record
//! and are reported as retained.

use adk_core::subject::{StoreErasure, SubjectDataStore, SubjectScope};
use adk_core::{AdkError, ErrorComponent};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::App;

/// Order states that are still open: something is owed, being made or on its way.
const OPEN: &[&str] = &[
    "awaiting_payment",
    "held",
    "approved",
    "paid",
    "baking",
    "ready",
    "out",
];

/// The sample sites' records as one of W6's subject stores.
pub struct SampleRecords(pub App);

fn failed(e: impl std::fmt::Display) -> AdkError {
    AdkError::internal(ErrorComponent::Tool, "samples.records", e.to_string())
}

#[async_trait]
impl SubjectDataStore for SampleRecords {
    fn name(&self) -> &str {
        "Orders, receipts and addresses"
    }

    async fn export(&self, scope: &SubjectScope) -> adk_core::Result<Value> {
        let records = self.0.records().await.map_err(failed)?;
        let account = scope.subject.id.as_str();
        let mut out = Vec::new();
        for site_id in &scope.app_names {
            let Some(site) = records.site(site_id).await.map_err(failed)? else {
                continue;
            };
            let customer = records.customer(site_id, account).await.map_err(failed)?;
            let addresses = records.addresses(site_id, account).await.map_err(failed)?;
            let orders = records
                .orders_for_customer(&site, account)
                .await
                .map_err(failed)?;
            if customer.is_none() && addresses.is_empty() && orders.is_empty() {
                continue;
            }
            out.push(json!({
                "site": site_id,
                "customer": customer,
                "addresses": addresses,
                "orders": orders,
            }));
        }
        Ok(Value::Array(out))
    }

    async fn erase(&self, scope: &SubjectScope) -> adk_core::Result<StoreErasure> {
        let records = self.0.records().await.map_err(failed)?;
        let account = scope.subject.id.as_str();
        let mut deleted = 0u64;
        let mut retained = 0u64;
        for site_id in &scope.app_names {
            let Some(site) = records.site(site_id).await.map_err(failed)? else {
                continue;
            };
            let orders = records
                .orders_for_customer(&site, account)
                .await
                .map_err(failed)?;
            if let Some(open) = orders
                .iter()
                .find(|o| OPEN.contains(&o.order.state.as_str()))
            {
                return Err(AdkError::internal(
                    ErrorComponent::Tool,
                    agentrix_identity::visitor::rights::OPEN_ORDERS_CODE,
                    format!("open order {}", open.order.order_id),
                ));
            }
            for address in records.addresses(site_id, account).await.map_err(failed)? {
                if records
                    .remove_address(site_id, account, &address.address_id)
                    .await
                    .map_err(failed)?
                {
                    deleted += 1;
                }
            }
            if records
                .forget_customer(site_id, account)
                .await
                .map_err(failed)?
            {
                deleted += 1;
            }
            retained += orders.len() as u64;
        }
        Ok(StoreErasure {
            deleted,
            retained,
            completes_by: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use adk_core::SubjectRef;
    use adk_core::subject::{SubjectDataStore, SubjectScope};

    use super::SampleRecords;
    use crate::{App, Config};

    async fn store(name: &str) -> SampleRecords {
        let mut ctx = agentrix_host::testing::context().await.unwrap();
        ctx.platform_host = "localhost".into();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config = Config {
            data_dir: std::env::temp_dir().join(format!(
                "agentrix-samples-subject-{name}-{}",
                std::process::id()
            )),
            platform_host: "localhost".into(),
            scheme: "http".into(),
            port: None,
            bundles_dir: Some(root.join("fixtures/samples")),
            templates_dir: None,
            reseed: true,
        };
        SampleRecords(App::new(config, &ctx).unwrap())
    }

    fn scope(account: &str) -> SubjectScope {
        SubjectScope::new(["kilele-bakery-sample"], SubjectRef::human(account))
    }

    #[tokio::test]
    async fn export_lists_the_customers_orders() {
        let s = store("export").await;
        let out = s.export(&scope("cus-njeri")).await.unwrap();
        let text = out.to_string();
        assert!(text.contains("KB-1042"), "{text}");
        assert!(text.contains("KB-1043"), "{text}");
        assert!(text.contains("njeri@example.com"), "{text}");
        // Someone with nothing here gets nothing back.
        let empty = s.export(&scope("vac_nobody")).await.unwrap();
        assert_eq!(empty, serde_json::json!([]));
    }

    #[tokio::test]
    async fn erase_refuses_while_an_order_is_open() {
        let s = store("open").await;
        // KB-1043, Amina's cake, is paid and not yet delivered.
        let err = s.erase(&scope("cus-njeri")).await.unwrap_err();
        assert!(err.to_string().contains("KB-1043"), "{err}");
        let records = s.0.records().await.unwrap();
        assert!(
            records
                .customer("kilele-bakery-sample", "cus-njeri")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn erase_forgets_the_customer_and_keeps_the_sales_record() {
        let s = store("erase").await;
        // Peter O.'s only order, KB-1038, was collected.
        let done = s.erase(&scope("cus-peter")).await.unwrap();
        assert!(done.deleted >= 1, "{done:?}");
        assert_eq!(done.retained, 1);
        let records = s.0.records().await.unwrap();
        assert!(
            records
                .customer("kilele-bakery-sample", "cus-peter")
                .await
                .unwrap()
                .is_none()
        );
        let site = records.site("kilele-bakery-sample").await.unwrap().unwrap();
        assert!(records.order(&site, "KB-1038").await.unwrap().is_some());
    }
}
