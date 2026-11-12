//! The operators' activity page (face's [`saas_web::activity`]) over this node's own records:
//! each company's signed audit log, the wallets with their PayPal reconciliations, the decisions
//! waiting for owners, the model gateway's spend today against the daily cap, and the test
//! customer agents' visits to the sample sites.
//!
//! [`crate::platform::with_activity`] serves it on the ops host, to signed-in operators only.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use saas_web::activity::{
    Activity, AuditSource, Row, Source, SpendCap, VisitsSource, WalletSource,
};

use crate::Node;

/// The two companies, by face's site id for the wallets and by the name the page shows.
const COMPANIES: [(&str, &str); 2] = [("agentrix", "agentrix"), ("kilele", "kilele")];

/// The companies' sites in node's `control.db`, where the gateway meters their agents' calls.
const GATEWAY_SITES: [&str; 2] = ["saas-agentrix", "saas-kilele"];

/// One company's signed audit log, named for that company on the page.
pub struct CompanyLog {
    name: String,
    log: AuditSource,
}

impl CompanyLog {
    /// `company`'s log `log_id` in `store`.
    pub fn new(
        company: &str,
        log_id: &str,
        store: Arc<dyn adk_core::audit_log::AuditLogStore>,
    ) -> Self {
        Self {
            name: format!("Signed audit log: {company}"),
            log: AuditSource::new(
                store,
                HashMap::from([(log_id.to_owned(), company.to_owned())]),
            ),
        }
    }
}

#[async_trait]
impl Source for CompanyLog {
    fn name(&self) -> String {
        self.name.clone()
    }

    async fn poll(&self) -> anyhow::Result<Vec<Row>> {
        self.log.poll().await
    }
}

/// The gateway's meter in node's `control.db` (its `usage` table), for the day's spend only:
/// each model call already shows once, as the company's signed `model.call` audit record.
pub struct GatewaySpend {
    control: sqlx::SqlitePool,
}

impl GatewaySpend {
    /// Over node's `control.db`.
    pub fn new(control: sqlx::SqlitePool) -> Self {
        Self { control }
    }
}

#[async_trait]
impl Source for GatewaySpend {
    fn name(&self) -> String {
        "Model gateway spend (control.db)".into()
    }

    async fn poll(&self) -> anyhow::Result<Vec<Row>> {
        Ok(Vec::new())
    }

    async fn spend_since(&self, t: DateTime<Utc>) -> Option<i64> {
        let sites = serde_json::to_string(&GATEWAY_SITES).ok()?;
        let since = t.to_rfc3339_opts(SecondsFormat::Micros, true);
        let (micro,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage \
             WHERE site_id IN (SELECT value FROM json_each(?)) AND at >= ?",
        )
        .bind(sites)
        .bind(since)
        .fetch_one(&self.control)
        .await
        .inspect_err(|e| tracing::debug!(error = %e, "the gateway's meter couldn't be read"))
        .ok()?;
        Some(micro)
    }
}

/// The daily model spend cap the page measures against: `SAAS_DAILY_MODEL_CAP_USD`, else
/// USD 10.
pub fn cap(env: &crate::config::EnvFile) -> SpendCap {
    let usd = env
        .get("SAAS_DAILY_MODEL_CAP_USD")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(10.0);
    SpendCap {
        micro_usd: (usd * 1_000_000.0).round() as i64,
        per: "day".into(),
    }
}

/// The test customer agents' log of their visits to the sample sites:
/// `<TEST_AGENTS_DATA_DIR>/activity.jsonl`, else `<data dir>/test-agents/activity.jsonl`.
pub fn visits_log(config: &crate::config::Config) -> std::path::PathBuf {
    config
        .env
        .get("TEST_AGENTS_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.data_dir.join("test-agents"))
        .join("activity.jsonl")
}

/// The page over `node`'s records; `control` is node's `control.db`, where the gateway meters.
pub fn activity(node: &Arc<Node>, control: sqlx::SqlitePool) -> Activity {
    let mut activity = Activity::new();
    for rt in node.runtimes() {
        let audit = &rt.company.governance.audit;
        activity = activity.with_source(Arc::new(CompanyLog::new(
            &rt.company.config.slug,
            audit.log_id(),
            audit.store(),
        )));
    }
    activity
        .with_source(Arc::new(WalletSource::new(
            crate::os::wallets(),
            COMPANIES
                .iter()
                .map(|(site, company)| ((*site).to_owned(), (*company).to_owned()))
                .collect(),
        )))
        .with_source(Arc::new(GatewaySpend::new(control)))
        .with_source(Arc::new(VisitsSource::new(visits_log(&node.config))))
        .with_feed(node.feed(), Arc::new(saas_web::today_in_nairobi))
        .with_cap(Some(cap(&node.config.env)))
}
