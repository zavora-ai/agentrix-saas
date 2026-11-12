//! `agentrix-badge verify <did> [--local] [--operator <did:web:host | host>]`: checks an agent's
//! badge from its DID alone (see [`saas_badge::cli`]). The node runs the same command as
//! `agentrix-saas badge verify`.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(saas_badge::cli::run(&args, None).await);
}
