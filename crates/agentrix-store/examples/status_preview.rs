//! The status page with a week of invented check history, for a look in a browser. Never
//! deployed; the history lives in memory.
//!
//! ```sh
//! scripts/wcargo run -p agentrix-store --features testing --example status_preview
//! # http://localhost:18802/status
//! ```

use agentrix_store::health::{Health, State};
use chrono::{Duration, Utc};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pool = agentrix_store::testing::control_db().await;
    let health = Health::new(pool.clone(), reqwest::Client::new(), "http://127.0.0.1:9");
    let now = Utc::now();
    for days_ago in 1..8i64 {
        for minute in 0..30i64 {
            let at = now - Duration::days(days_ago) + Duration::minutes(minute);
            let mut results = health.check(at).await;
            for r in &mut results {
                if r.id == "ask" {
                    r.state = match (days_ago, minute) {
                        (3, m) if m < 12 => State::Down,
                        (5, m) if m < 20 => State::Degraded,
                        _ => State::Ok,
                    };
                }
            }
            health.record(&results, at).await?;
        }
    }
    let mut results = health.check(now).await;
    for r in &mut results {
        if r.id == "ask" {
            r.state = State::Ok;
            r.detail = "The Model Gateway is answering.".to_owned();
        }
    }
    health.record(&results, now).await?;

    let mut ctx = agentrix_host::testing::context().await?;
    ctx.pool = pool;
    let app = agentrix_store::mount(&ctx)?
        .platform
        .merge(agentrix_ui::assets());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18802").await?;
    println!("http://localhost:18802/status");
    axum::serve(listener, app).await?;
    Ok(())
}
