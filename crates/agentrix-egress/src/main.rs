//! `agentrix-egress`: run the sealed-room proxy for one job.
//!
//! ```text
//! agentrix-egress --job job-42 --allow kilele.example --allow '*.kilele.example' --listen 0.0.0.0:3128
//! ```
//!
//! Events go to stdout as JSON lines (one per request) for the runner to fold into the job's
//! receipt; diagnostics go to stderr.

use agentrix_egress::policy::{Origin, Policy};
use agentrix_egress::proxy;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about = "The sealed room's only way out")]
struct Args {
    /// Address to listen on (the room's network side).
    #[arg(long, env = "AGENTRIX_EGRESS_LISTEN", default_value = "0.0.0.0:3128")]
    listen: String,
    /// The job this proxy serves, recorded in every event.
    #[arg(long, env = "AGENTRIX_JOB")]
    job: String,
    /// An allowed destination: `host`, `host:port`, `*.suffix` or a URL. Repeatable.
    #[arg(long = "allow", value_name = "ORIGIN")]
    allow: Vec<String>,
    /// Maximum bytes per direction per connection.
    #[arg(long, default_value_t = 25 * 1024 * 1024)]
    max_bytes: u64,
    /// Maximum lifetime of one connection, in seconds.
    #[arg(long, default_value_t = 60)]
    max_connection_secs: u64,
    /// Maximum concurrent connections.
    #[arg(long, default_value_t = 32)]
    max_connections: usize,
    /// The job's room address: only connections from it are served. Repeatable; none means
    /// any (tests only; the runner always passes the room's address).
    #[arg(long = "peer", value_name = "IP")]
    peers: Vec<std::net::IpAddr>,
    /// Allow non-public destinations. For the loopback replay relay and tests only.
    #[arg(long, hide = true)]
    permit_non_public: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let allow = args
        .allow
        .iter()
        .map(|s| Origin::parse(s))
        .collect::<Result<Vec<_>, _>>()?;
    if allow.is_empty() {
        tracing::warn!(job = %args.job, "no --allow given: every request will be refused");
    }
    let mut policy = Policy::only(allow);
    policy.max_bytes = args.max_bytes;
    policy.max_connection_secs = args.max_connection_secs;
    policy.permit_non_public = args.permit_non_public;
    policy.peers = args.peers;
    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    tracing::info!(job = %args.job, listen = %args.listen, allow = ?policy.allow.iter().map(ToString::to_string).collect::<Vec<_>>(), "egress proxy ready");
    proxy::serve(
        listener,
        policy,
        args.job,
        args.max_connections,
        proxy::stdout_sink(),
    )
    .await?;
    Ok(())
}
