//! `routing.changed` receipts (R11.4, R11.6, design §4.4; integration lane).
//!
//! The Model Gateway runs as its own user and holds no signing key, so node watches it. At startup
//! and then on a timer, [`RoutingWatch::check`] reads the gateway's `GET /v1/status` (its provider
//! and the route for every purpose) and compares it with each owned site's latest
//! `routing.changed` receipt. When they differ, or a site has none yet, it signs one receipt per
//! site:
//!
//! - **detail:** `{from, to, reason}`. `from` and `to` are `{provider, routes}`, and `reason` comes
//!   from `AGENTRIX_ROUTING_REASON`, for example the bake-off that chose the routes.
//! - **inputs:** the digest of `to`.
//!
//! Switching providers is then configuration only (`AGENTRIX_PROVIDER`), and every owner sees the
//! change on their Trust tab. Nothing here can change a route.

use std::sync::Arc;
use std::time::Duration;

use agentrix_types::{Digest, ReceiptBody, ReceiptKind, SiteId, Subject};
use anyhow::Context as _;
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::db::receipts::ReceiptStore;

/// The purposes named in a receipt's title, in this order, when they are routed.
const TITLE_PURPOSES: [&str; 3] = ["build", "vision-parity", "concierge"];

/// Watches the gateway's routing and receipts each change.
pub struct RoutingWatch {
    pool: SqlitePool,
    receipts: Arc<ReceiptStore>,
    http: reqwest::Client,
    status_url: String,
    reason: Option<String>,
}

impl RoutingWatch {
    /// A watch on the gateway at `models_url`. `reason` goes into every receipt it writes.
    pub fn new(
        pool: SqlitePool,
        receipts: Arc<ReceiptStore>,
        http: reqwest::Client,
        models_url: &str,
        reason: Option<String>,
    ) -> Self {
        Self {
            pool,
            receipts,
            http,
            status_url: format!("{}/v1/status", models_url.trim_end_matches('/')),
            reason: reason.filter(|r| !r.trim().is_empty()),
        }
    }

    /// Checks now, then every `every`. A failed check is logged and retried next time.
    pub fn spawn(self: Arc<Self>, every: Duration) {
        tokio::spawn(async move {
            // The gateway may still be starting with node; give it a moment first.
            tokio::time::sleep(Duration::from_secs(5)).await;
            loop {
                match self.check().await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(sites = n, "routing.changed receipts written"),
                    Err(e) => tracing::warn!(error = %format!("{e:#}"), "routing check failed"),
                }
                tokio::time::sleep(every).await;
            }
        });
    }

    /// Compares the gateway's routing with each owned site's latest receipt and writes a
    /// `routing.changed` receipt where it differs. Returns how many receipts it wrote.
    pub async fn check(&self) -> anyhow::Result<usize> {
        let status: Value = self
            .http
            .get(&self.status_url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("asking the gateway for its routes")?
            .error_for_status()
            .context("the gateway's status")?
            .json()
            .await
            .context("reading the gateway's status")?;
        let now = routing_of(&status).context("the gateway's status has no routes")?;
        let sites: Vec<(String,)> = sqlx::query_as(
            "SELECT site_id FROM sites WHERE owner_account_id IS NOT NULL ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut written = 0;
        for (site,) in sites {
            let site = SiteId::new(site);
            let latest = self
                .receipts
                .list(&site, Some(ReceiptKind::RoutingChanged), 1)
                .await?
                .into_iter()
                .next();
            let before = latest
                .as_ref()
                .and_then(|s| s.receipt.body.detail.as_ref())
                .and_then(|d| d.get("to"))
                .cloned();
            if before.as_ref() == Some(&now) {
                continue;
            }
            let at = Utc::now();
            let mut body = ReceiptBody::new(
                ReceiptKind::RoutingChanged,
                site.clone(),
                Subject::Site(site.clone()),
                at,
                at,
            );
            body.inputs = vec![Digest::of(&agentrix_types::canonical_json(&now)?)];
            body.detail = Some(json!({ "from": before, "to": now, "reason": self.reason }));
            self.receipts
                .issue(body.with_title(title(before.as_ref(), &now)))
                .await
                .with_context(|| format!("writing routing.changed for {site}"))?;
            written += 1;
        }
        Ok(written)
    }
}

/// `{provider, routes: {purpose: {provider, model}}}` from the gateway's status. The tier is
/// left out: the same model under another tier name routes the same.
fn routing_of(status: &Value) -> Option<Value> {
    let provider = status.get("provider")?.as_str()?;
    let routes = status.get("routes")?.as_object()?;
    let routes: serde_json::Map<String, Value> = routes
        .iter()
        .map(|(purpose, r)| {
            (
                purpose.clone(),
                json!({ "provider": r.get("provider"), "model": r.get("model") }),
            )
        })
        .collect();
    Some(json!({ "provider": provider, "routes": routes }))
}

/// "Models now on vultr: build qwen3.8-27b, vision-parity deepseek-v4.1-flash-1017, …".
fn title(before: Option<&Value>, now: &Value) -> String {
    let provider = now.get("provider").and_then(Value::as_str).unwrap_or("?");
    let picks: Vec<String> = TITLE_PURPOSES
        .iter()
        .filter_map(|p| {
            let model = now.pointer(&format!("/routes/{p}/model"))?.as_str()?;
            Some(format!("{p} {model}"))
        })
        .collect();
    let picks = if picks.is_empty() {
        String::new()
    } else {
        format!(": {}", picks.join(", "))
    };
    match before
        .and_then(|b| b.get("provider"))
        .and_then(Value::as_str)
    {
        None => format!("Models in use, on {provider}{picks}"),
        Some(old) if old != provider => format!("Models moved from {old} to {provider}{picks}"),
        Some(_) => format!("Model routes changed on {provider}{picks}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::Signer;
    use axum::Router;
    use axum::routing::get;
    use std::sync::Mutex;

    fn status(provider: &str, build: &str) -> Value {
        json!({
            "provider": provider,
            "providers": {"anthropic": {"ok": true}},
            "mode": "live",
            "routes": {
                "build": {"provider": provider, "model": build, "tier": "standard"},
                "vision-parity": {"provider": provider, "model": "v-model", "tier": "fast"},
                "embed": {"provider": "vultr", "model": "qwen3-embedding-4b", "tier": null}
            },
            "daily_cap_usd": 5.0
        })
    }

    #[tokio::test]
    async fn a_route_change_is_receipted_once_per_owned_site() {
        // A fake gateway whose status the test can change.
        let current = Arc::new(Mutex::new(status("anthropic", "claude-sonnet-5-5")));
        let served = current.clone();
        let app = Router::new().route(
            "/v1/status",
            get(move || {
                let served = served.clone();
                async move { axum::Json(served.lock().unwrap().clone()) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let pool = crate::db::open_in_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id, email, display_name, created_at) VALUES ('acct-1', 'o@example.test', 'O', 0)")
            .execute(&pool)
            .await
            .unwrap();
        let owned = crate::db::sites::create_or_get(
            &pool,
            "kilele-bakery.example",
            "https://kilele-bakery.example",
            false,
        )
        .await
        .unwrap();
        crate::db::sites::set_owner(&pool, &owned, "acct-1")
            .await
            .unwrap();
        let unowned =
            crate::db::sites::create_or_get(&pool, "other.example", "https://other.example", false)
                .await
                .unwrap();

        let receipts = Arc::new(ReceiptStore::new(
            pool.clone(),
            Arc::new(Signer::from_secret_bytes(&[9; 32])),
        ));
        let watch = RoutingWatch::new(
            pool,
            receipts.clone(),
            reqwest::Client::new(),
            &url,
            Some("bake-off 375a5ca".into()),
        );

        // First sight: one receipt for the owned site, none for the unowned one.
        assert_eq!(watch.check().await.unwrap(), 1);
        assert_eq!(
            receipts
                .list(&unowned, Some(ReceiptKind::RoutingChanged), 5)
                .await
                .unwrap()
                .len(),
            0
        );
        // Nothing changed: nothing written.
        assert_eq!(watch.check().await.unwrap(), 0);

        // The switch: Anthropic to Vultr.
        *current.lock().unwrap() = status("vultr", "qwen3.8-27b");
        assert_eq!(watch.check().await.unwrap(), 1);
        let list = receipts
            .list(&owned, Some(ReceiptKind::RoutingChanged), 5)
            .await
            .unwrap();
        assert_eq!(list.len(), 2);
        let body = &list[0].receipt.body;
        let detail = body.detail.as_ref().unwrap();
        assert_eq!(detail.pointer("/from/provider").unwrap(), "anthropic");
        assert_eq!(
            detail.pointer("/to/routes/build/model").unwrap(),
            "qwen3.8-27b"
        );
        assert_eq!(detail["reason"], "bake-off 375a5ca");
        assert_eq!(
            body.title().unwrap(),
            "Models moved from anthropic to vultr: build qwen3.8-27b, vision-parity v-model"
        );
        assert!(
            list[0].receipt.verify_with(receipts.keys()).is_ok(),
            "signed by the published key"
        );
    }

    #[test]
    fn tiers_and_health_do_not_count_as_a_change() {
        let a = status("vultr", "qwen3.8-27b");
        let mut b = a.clone();
        b["routes"]["build"]["tier"] = json!("premium");
        b["providers"] = json!({"vultr": {"ok": false}});
        b["mode"] = json!("replay");
        assert_eq!(routing_of(&a), routing_of(&b));
    }
}
