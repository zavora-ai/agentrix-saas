//! `agentrix-runner`: runs sealed rooms on the sandbox VM (design §3.1, §4.2; task S1-01).
//!
//! Verifies each control-signed job spec, starts the room (gVisor, read-only root, non-root,
//! caps, egress only through a per-job `agentrix-egress`), watches it against its caps, destroys
//! it and confirms that, and only then collects `/out` by digest. It holds no secrets: only
//! control's public key.
//!
//! Ports and networks follow `deploy/README.md`: the runner on `0.0.0.0:7443`, reachable from
//! control's VPC address only (host nftables); rooms on the Docker network `agx-rooms`
//! (`172.30.0.0/24`), each at its slot's address; each fetch or replay job's proxy on
//! `172.30.0.1`, at its slot's port in 3128-3191.
//!
//! ```text
//! agentrix-runner --listen 0.0.0.0:7443 --control-key /etc/agentrix/control-job.pub
//! ```

// A binary's root file looks for modules beside it, in `src/bin/`, where Cargo would take each
// file for another binary; so the runner's modules live in `src/bin/agentrix-runner/`.
#[path = "agentrix-runner/cas.rs"]
mod cas;
#[path = "agentrix-runner/collect.rs"]
mod collect;
#[path = "agentrix-runner/egress.rs"]
mod egress;
#[path = "agentrix-runner/http.rs"]
mod http;
#[path = "agentrix-runner/job.rs"]
mod job;
#[path = "agentrix-runner/room.rs"]
mod room;
#[path = "agentrix-runner/spec.rs"]
mod spec;

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use clap::Parser;
use ed25519_dalek::VerifyingKey;

use crate::egress::EgressConfig;
use crate::job::{RoomsNet, Runner, SlotPool};
use crate::room::{DockerCli, RoomRuntime};

/// Command-line arguments.
#[derive(Debug, Parser)]
#[command(version, about = "Agentrix sealed-room runner")]
struct Args {
    /// Address to serve on. The host's nftables admit only control's VPC address to it, and keep
    /// rooms from it (deploy/README.md).
    #[arg(long, env = "AGENTRIX_RUNNER_LISTEN", default_value = "0.0.0.0:7443")]
    listen: SocketAddr,
    /// The Docker network rooms join.
    #[arg(long, env = "AGENTRIX_ROOMS_NETWORK", default_value = "agx-rooms")]
    rooms_network: String,
    /// The rooms network's gateway, where proxies listen.
    #[arg(long, env = "AGENTRIX_ROOMS_GATEWAY", default_value = "172.30.0.1")]
    rooms_gateway: Ipv4Addr,
    /// Proxy ports, `first-last`; one per room slot.
    #[arg(long, env = "AGENTRIX_EGRESS_PORTS", default_value = "3128-3191")]
    egress_ports: String,
    /// The first room address; slot `i` gets this plus `i`.
    #[arg(long, default_value = "172.30.0.10")]
    first_room_ip: Ipv4Addr,
    /// Working directory: the store, job directories and the journal of seen job ids.
    #[arg(
        long,
        env = "AGENTRIX_RUNNER_WORK",
        default_value = "/var/lib/agentrix-runner"
    )]
    work_dir: PathBuf,
    /// Control's Ed25519 job-signing public key: 32 bytes as hex or base64, in a file.
    #[arg(long, env = "AGENTRIX_CONTROL_KEY")]
    control_key: PathBuf,
    /// The `agentrix-egress` binary.
    #[arg(long, default_value = "/usr/local/bin/agentrix-egress")]
    egress_bin: PathBuf,
    /// `uid:gid` the egress proxies run as (an account with no Docker access).
    #[arg(long, env = "AGENTRIX_EGRESS_USER", default_value = "65533:65533")]
    egress_user: String,
    /// `setpriv`, used to start each proxy with no groups and no capabilities.
    #[arg(long, default_value = "/usr/bin/setpriv")]
    setpriv: PathBuf,
    /// Bytes per direction per proxied connection.
    #[arg(long, default_value_t = 25 * 1024 * 1024)]
    egress_max_bytes: u64,
    /// OCI runtime for rooms.
    #[arg(long, default_value = "runsc")]
    runtime: String,
    /// Development only: allow a runtime other than runsc and run proxies as the runner's own
    /// user. Every result names the runtime it used.
    #[arg(long)]
    dev: bool,
    /// Rooms that may run at once.
    #[arg(long, default_value_t = 4)]
    max_rooms: usize,
    /// The `docker` binary.
    #[arg(long, default_value = "docker")]
    docker: PathBuf,
}

fn read_key(path: &std::path::Path) -> anyhow::Result<VerifyingKey> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    let text = std::fs::read_to_string(path)?;
    let text = text.trim();
    // Hex, standard base64, or the unpadded base64url of /.well-known/agentrix/keys.
    let bytes = hex::decode(text)
        .ok()
        .or_else(|| URL_SAFE_NO_PAD.decode(text).ok())
        .or_else(|| STANDARD.decode(text).ok())
        .ok_or_else(|| anyhow::anyhow!("{} is not hex or base64", path.display()))?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| {
        anyhow::anyhow!("{} must hold a 32-byte Ed25519 public key", path.display())
    })?;
    Ok(VerifyingKey::from_bytes(&arr)?)
}

fn parse_ports(s: &str) -> anyhow::Result<(u16, u16)> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| anyhow::anyhow!("--egress-ports must be first-last"))?;
    let (a, b): (u16, u16) = (a.trim().parse()?, b.trim().parse()?);
    anyhow::ensure!(a <= b, "--egress-ports: {a} is above {b}");
    Ok((a, b))
}

fn parse_user(s: &str) -> anyhow::Result<(u32, u32)> {
    let (u, g) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("--egress-user must be uid:gid"))?;
    Ok((u.parse()?, g.parse()?))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();

    anyhow::ensure!(
        args.dev || args.runtime == "runsc",
        "rooms run under runsc; --runtime {} needs --dev",
        args.runtime
    );

    let docker = DockerCli {
        docker: args.docker.clone(),
    };
    let runtimes = docker.runtimes().await?;
    anyhow::ensure!(
        runtimes.iter().any(|r| r == &args.runtime),
        "Docker has no {:?} runtime (it has {runtimes:?}); install gVisor and register runsc",
        args.runtime
    );

    anyhow::ensure!(
        docker.network_exists(&args.rooms_network).await?,
        "the rooms network {:?} does not exist (cloud-init creates it; see deploy/README.md)",
        args.rooms_network
    );
    for room in docker.leftovers().await? {
        tracing::warn!(%room, "removing a room a previous run left behind");
        docker.remove(&room).await?;
    }
    let (first_port, last_port) = parse_ports(&args.egress_ports)?;
    let rooms = args.max_rooms.min(usize::from(last_port - first_port) + 1);

    std::fs::create_dir_all(args.work_dir.join("jobs"))?;
    let seen: HashSet<String> = std::fs::read_to_string(args.work_dir.join("seen-jobs"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let runner = Arc::new(Runner {
        runtime: docker,
        cas: cas::Cas::open(args.work_dir.join("cas")).await?,
        work: args.work_dir.clone(),
        key: read_key(&args.control_key)?,
        oci_runtime: args.runtime.clone(),
        egress: EgressConfig {
            bin: args.egress_bin.clone(),
            user: if args.dev {
                None
            } else {
                Some(parse_user(&args.egress_user)?)
            },
            setpriv: args.setpriv.clone(),
            max_bytes: args.egress_max_bytes,
        },
        seen: Mutex::new(seen),
        slots: SlotPool::new(rooms),
        net: RoomsNet {
            network: args.rooms_network.clone(),
            gateway: args.rooms_gateway,
            first_port,
            first_ip: args.first_room_ip,
        },
    });

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, runtime = %args.runtime, rooms, network = %args.rooms_network, "runner ready");
    axum::serve(listener, http::router(runner)).await?;
    Ok(())
}
