//! Preview the three sample sites locally.
//!
//! ```sh
//! cargo run -p agentrix-samples --example preview
//! # http://kilele-bakery-sample.localhost:18200/
//! # http://localhost:18200/samples/kilele-bakery-sample/   (the owner's apps)
//! ```
//!
//! Templates and bundles are read from the repository on every request, so markup and seed
//! edits show on reload. `AGENTRIX_SAMPLES_DATA` keeps the database between runs.

use std::sync::Arc;

use agentrix_samples::Config;
use axum::Router;
use axum::extract::Request;
use axum::http::header;
use tower::ServiceExt as _;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(18200);
    let mut ctx = agentrix_host::testing::context().await?;
    ctx.origin = format!("http://localhost:{port}").parse()?;
    ctx.platform_host = "localhost".into();
    if let Ok(models) = std::env::var("AGENTRIX_MODELS_URL") {
        ctx.models_url = models;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let data = std::env::var_os("AGENTRIX_SAMPLES_DATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("agentrix-samples-preview-{port}")));
    let config = Config {
        data_dir: data,
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: Some(port),
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: Some(root.join("crates/agentrix-samples/templates")),
        reseed: std::env::var("AGENTRIX_SAMPLES_RESEED").is_ok_and(|v| v == "1"),
    };
    agentrix_identity::migrate(&ctx.pool).await?;
    // W9's outbox, so order and digest messages are recorded as they are in node.
    agentrix_messages::migrate(&ctx.pool).await?;
    // W11's picture library lives in node's migrator; the preview applies its table and seeds
    // the sample pictures the way node does.
    for sql in [
        include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
        // The cake studio's ideas (ADR-032).
        include_str!("../../agentrix-node/src/db/migrations/1202_media_ideas.sql"),
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&ctx.pool)
            .await?;
    }
    agentrix_media::samples::seed(&ctx.pool, &ctx.blobs).await?;
    // AGENTRIX_SAMPLES_DEV_VISITOR=njeri@example.com signs every request in as that customer
    // (preview only), so the bag, checkout and order pages can be checked without W6's code flow.
    let mounted = match std::env::var("AGENTRIX_SAMPLES_DEV_VISITOR")
        .ok()
        .filter(|e| !e.trim().is_empty())
    {
        Some(email) => {
            let app = agentrix_samples::App::new(config, &ctx)?
                .with_visitors(Arc::new(DevVisitor::new(&email)));
            // Pictures (/media/…) as node serves them on the sample hosts.
            agentrix_samples::mounted(app, agentrix_media::public_router(&ctx))
        }
        None => agentrix_samples::mount_with(&ctx, config)?,
    };
    for task in mounted.tasks {
        tokio::spawn(task.future);
    }
    let hosts: Vec<(agentrix_host::HostMatch, Router)> = mounted
        .hosts
        .into_iter()
        .map(|h| (h.matcher, h.router))
        .collect();
    let hosts = Arc::new(hosts);
    let platform = mounted.platform;
    let app = Router::new().fallback(move |req: Request| {
        let hosts = hosts.clone();
        let platform = platform.clone();
        async move {
            let host = req
                .headers()
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .map(|h| h.split(':').next().unwrap_or_default().to_ascii_lowercase())
                .unwrap_or_default();
            for (matcher, router) in hosts.iter() {
                if matcher.serves(&host).await.unwrap_or(false) {
                    return router
                        .clone()
                        .oneshot(req)
                        .await
                        .unwrap_or_else(|e| match e {});
                }
            }
            platform.oneshot(req).await.unwrap_or_else(|e| match e {})
        }
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    tracing::info!("http://kilele-bakery-sample.localhost:{port}/");
    axum::serve(listener, app).await?;
    Ok(())
}

/// The preview's stand-in visitor: one fixed customer for every request. Njeri's email maps to
/// the seeded `cus-njeri`, so KB-1042 and KB-1043 show as hers. Set
/// `AGENTRIX_SAMPLES_DEV_UNVERIFIED=1` to see the unconfirmed-email path.
struct DevVisitor(agentrix_samples::ports::Visitor);

impl DevVisitor {
    fn new(email: &str) -> Self {
        let email = email.trim().to_lowercase();
        let local = email.split('@').next().unwrap_or("guest").to_owned();
        let (account, name, phone) = match email.as_str() {
            "njeri@example.com" => (
                "cus-njeri".to_owned(),
                "Njeri".to_owned(),
                Some("+44 7700 900418".to_owned()),
            ),
            _ => (format!("dev-{local}"), local.clone(), None),
        };
        let email_verified =
            std::env::var("AGENTRIX_SAMPLES_DEV_UNVERIFIED").map_or(true, |v| v != "1");
        Self(agentrix_samples::ports::Visitor {
            account,
            name,
            email,
            email_verified,
            phone,
        })
    }
}

#[async_trait::async_trait]
impl agentrix_samples::ports::Visitors for DevVisitor {
    async fn visitor(
        &self,
        _site: &agentrix_samples::ports::SampleHost,
        _headers: &axum::http::HeaderMap,
    ) -> anyhow::Result<Option<agentrix_samples::ports::Visitor>> {
        Ok(Some(self.0.clone()))
    }
}
