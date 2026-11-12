//! Asks the Chief WebAgent one question against a node's data, through a running Model Gateway,
//! and prints the answer, its sources and its steps. For checking the Chief live:
//!
//! ```text
//! cargo run -p agentrix-kernel --example chief_ask -- <data dir> <models url> <site> <question…>
//! ```
//!
//! `AGENTRIX_LISTEN` names the node whose sites' `/ask` the desks answer through. Run it on a
//! copy of a node's data directory: it adds the kernel's tables and keeps a trace.

use agentrix_kernel::chief::{Chief, Surface};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [data, models, site, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: chief_ask <data dir> <models url> <site> <question…>");
    };
    let question = rest.join(" ");
    let mut ctx = agentrix_host::testing::context().await?;
    ctx.pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&format!("sqlite://{data}/control.db"))
        .await?;
    agentrix_kernel::migrate(&ctx.pool).await?;
    ctx.data_dir = data.into();
    ctx.models_url = models.clone();
    let a = Chief::new(&ctx)
        .answer(site, &question, Surface::Chat)
        .await;
    println!("Q: {question}\nA: {}\n", a.text);
    println!("Sources:");
    for s in &a.sources {
        println!("  - {} ({})", s.label, s.tool);
    }
    println!("\nSteps:");
    for s in &a.trace {
        println!("  {:>6} ms  {:<14} {:<9} {}", s.ms, s.who, s.kind, s.detail);
    }
    println!("\nTrace kept as {:?}", a.trace_id);
    Ok(())
}
