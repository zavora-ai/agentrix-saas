//! A local preview of the customer account pages on a sample host, in the plain shell.
//!
//! ```sh
//! cargo run -p agentrix-identity --example identity_preview
//! # then open http://kilele-bakery-sample.localhost:18600/account/sign-in
//! ```
//! Codes for `@example.com` addresses show on the code page (Sample mailbox). The database is in
//! memory and goes away when the preview stops.

use std::sync::Arc;

use agentrix_identity::mailer::MemoryMailer;
use agentrix_identity::{PlainShell, Visitors};
use agentrix_types::SiteId;
use agentrix_types::tenancy::SiteHost;
use axum::response::Redirect;
use axum::routing::get;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(18600);
    let ctx = agentrix_host::testing::context().await?;
    agentrix_identity::migrate(&ctx.pool).await?;
    let visitors = Visitors::new(
        ctx.pool.clone(),
        Arc::new(MemoryMailer::default()),
        Arc::new(PlainShell),
    )
    .with_receipts(ctx.receipts.clone());
    let host = "kilele-bakery-sample.localhost".to_owned();
    let site = SiteHost {
        site: SiteId::new("site_kilele_sample"),
        origin: format!("http://{host}:{port}"),
        host: host.clone(),
        name: "Kilele Bakery".into(),
        sample: true,
    };
    let app = visitors
        .router()
        .route("/", get(|| async { Redirect::to("/account/sign-in") }))
        .layer(axum::Extension(site));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Preview: http://{host}:{port}/account/sign-in");
    axum::serve(listener, app).await?;
    Ok(())
}
