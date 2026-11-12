//! Sealed rooms (design §3.1): the container flags every room gets, and the runtime that runs
//! them.
//!
//! [`RoomRuntime`] is the seam. [`DockerCli`] drives the `docker` CLI today; it swaps to
//! adk-sandbox's `DockerClient` once its gVisor and read-only-root hardening (#1347) is on the
//! pinned adk-rust rev.

use std::future::Future;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::Stdio;

use serde::Deserialize;
use tokio::process::{Child, Command};

/// The user every room runs as, whatever the image says: `nobody`.
pub const ROOM_USER: &str = "65534:65534";
/// The uid and gid that own a room's `/out`.
pub const ROOM_UID: u32 = 65534;
/// Label on every container and network the runner creates, valued with the job id.
pub const JOB_LABEL: &str = "agentrix.job";

/// Everything about one room except its image's contents.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomPlan {
    /// The job.
    pub job_id: String,
    /// The container name.
    pub name: String,
    /// The image, pinned by digest.
    pub image: String,
    /// The OCI runtime: `runsc` in production.
    pub runtime: String,
    /// CPU share.
    pub cpus: f64,
    /// Memory (and memory+swap) cap, MiB.
    pub memory_mb: u32,
    /// Process cap.
    pub pids: u32,
    /// Size of the `/tmp` tmpfs, MiB (counted against the memory cap).
    pub tmpfs_mb: u32,
    /// Host directory mounted read-only at `/in`.
    pub in_dir: PathBuf,
    /// Host directory mounted at `/out`.
    pub out_dir: PathBuf,
    /// The rooms network, or `None` for no network at all.
    pub network: Option<String>,
    /// The room's fixed address on that network; its proxy serves only this address.
    pub ip: Option<Ipv4Addr>,
    /// Environment: never secrets, only `HOME` and the proxy address.
    pub env: Vec<(String, String)>,
}

impl RoomPlan {
    /// The `docker create` arguments. Every hardening flag is here, so a test can see them all.
    pub fn create_args(&self) -> Vec<String> {
        let mut a: Vec<String> = vec![
            "create".into(),
            "--pull".into(),
            "never".into(),
            "--name".into(),
            self.name.clone(),
            "--label".into(),
            format!("{JOB_LABEL}={}", self.job_id),
            "--runtime".into(),
            self.runtime.clone(),
            "--read-only".into(),
            "--user".into(),
            ROOM_USER.into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--cpus".into(),
            format!("{}", self.cpus),
            "--memory".into(),
            format!("{}m", self.memory_mb),
            "--memory-swap".into(),
            format!("{}m", self.memory_mb),
            "--pids-limit".into(),
            self.pids.to_string(),
            "--tmpfs".into(),
            format!("/tmp:rw,nosuid,nodev,noexec,size={}m", self.tmpfs_mb),
            "--mount".into(),
            format!(
                "type=bind,source={},target=/in,readonly",
                self.in_dir.display()
            ),
            "--mount".into(),
            format!("type=bind,source={},target=/out", self.out_dir.display()),
            "--workdir".into(),
            "/tmp".into(),
            "--hostname".into(),
            "room".into(),
            "--log-driver".into(),
            "none".into(),
            "--no-healthcheck".into(),
            "--network".into(),
            self.network.clone().unwrap_or_else(|| "none".into()),
        ];
        if let (Some(ip), Some(_)) = (self.ip, &self.network) {
            a.push("--ip".into());
            a.push(ip.to_string());
        }
        for (k, v) in &self.env {
            a.push("--env".into());
            a.push(format!("{k}={v}"));
        }
        a.push(self.image.clone());
        a
    }
}

/// Usage from one `docker stats` sample.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    /// CPU, percent of one core.
    pub cpu_pct: f64,
    /// Memory in use, MiB.
    pub memory_mb: f64,
    /// Processes and threads.
    pub pids: u64,
}

/// How a container ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExitState {
    /// Its exit code.
    pub code: Option<i64>,
    /// The kernel's OOM killer ended it.
    pub oom_killed: bool,
}

/// What the runner needs from a container engine.
pub trait RoomRuntime: Send + Sync + 'static {
    /// The OCI runtimes the engine knows.
    fn runtimes(&self) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
    /// Whether a network exists.
    fn network_exists(&self, name: &str) -> impl Future<Output = anyhow::Result<bool>> + Send;
    /// Creates the room's container, stopped.
    fn create(&self, plan: &RoomPlan) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// Starts the room attached: the child's stdout is the room's stdout.
    fn start_attached(&self, name: &str) -> impl Future<Output = anyhow::Result<Child>> + Send;
    /// One usage sample, or `None` once the room has stopped.
    fn stats(&self, name: &str) -> impl Future<Output = anyhow::Result<Option<Usage>>> + Send;
    /// Kills the room at once.
    fn kill(&self, name: &str) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// How the room ended.
    fn exit_state(&self, name: &str) -> impl Future<Output = anyhow::Result<ExitState>> + Send;
    /// Removes the room's container.
    fn remove(&self, name: &str) -> impl Future<Output = anyhow::Result<()>> + Send;
    /// Whether the container exists.
    fn exists(&self, name: &str) -> impl Future<Output = anyhow::Result<bool>> + Send;
    /// Rooms a previous run left behind.
    fn leftovers(&self) -> impl Future<Output = anyhow::Result<Vec<String>>> + Send;
}

/// [`RoomRuntime`] over the `docker` CLI.
#[derive(Debug, Clone)]
pub struct DockerCli {
    /// Path to the `docker` binary.
    pub docker: PathBuf,
}

impl DockerCli {
    fn cmd(&self) -> Command {
        let mut c = Command::new(&self.docker);
        c.stdin(Stdio::null()).kill_on_drop(true);
        c
    }

    async fn output(&self, args: &[&str]) -> anyhow::Result<std::process::Output> {
        Ok(self
            .cmd()
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?)
    }

    async fn ok(&self, args: &[&str]) -> anyhow::Result<String> {
        let out = self.output(args).await?;
        anyhow::ensure!(
            out.status.success(),
            "docker {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

impl DockerCli {
    /// A pinned `name@sha256:<hex>` as Docker can run it without pulling: the reference itself
    /// when Docker knows it by repository digest, otherwise the local image whose ID is that
    /// digest, provided it is tagged `name`. Images built on the sandbox have no repository
    /// digest; their ID pins them just as well. Rooms never pull (`--pull never`).
    pub async fn resolve_image(&self, pinned: &str) -> anyhow::Result<String> {
        if self
            .output(&["image", "inspect", "--format", "{{.Id}}", pinned])
            .await?
            .status
            .success()
        {
            return Ok(pinned.to_string());
        }
        let (name, hex) = pinned
            .rsplit_once("@sha256:")
            .ok_or_else(|| anyhow::anyhow!("image {pinned} is not pinned by digest"))?;
        let id = format!("sha256:{hex}");
        let out = self
            .output(&["image", "inspect", "--format", "{{json .RepoTags}}", &id])
            .await?;
        anyhow::ensure!(
            out.status.success(),
            "image {pinned} is not on this host: build or load it first (rooms never pull)"
        );
        let tags: Vec<String> = serde_json::from_slice(&out.stdout).unwrap_or_default();
        anyhow::ensure!(
            tags.iter().any(|t| tag_name(t) == name),
            "the local image {id} is not tagged {name}"
        );
        Ok(id)
    }
}

/// `registry:5000/room-fetch:dev` → `registry:5000/room-fetch`.
fn tag_name(tag: &str) -> &str {
    match tag.rsplit_once(':') {
        Some((n, t)) if !t.contains('/') => n,
        _ => tag,
    }
}

impl RoomRuntime for DockerCli {
    async fn runtimes(&self) -> anyhow::Result<Vec<String>> {
        let json = self.ok(&["info", "--format", "{{json .Runtimes}}"]).await?;
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&json)?;
        Ok(map.keys().cloned().collect())
    }

    async fn network_exists(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self
            .output(&["network", "inspect", name])
            .await?
            .status
            .success())
    }

    async fn create(&self, plan: &RoomPlan) -> anyhow::Result<()> {
        let mut plan = plan.clone();
        plan.image = self.resolve_image(&plan.image).await?;
        let args = plan.create_args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.ok(&refs).await.map(drop)
    }

    async fn start_attached(&self, name: &str) -> anyhow::Result<Child> {
        Ok(self
            .cmd()
            .args(["start", "--attach", name])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?)
    }

    async fn stats(&self, name: &str) -> anyhow::Result<Option<Usage>> {
        let out = self
            .output(&["stats", "--no-stream", "--format", "{{json .}}", name])
            .await?;
        if !out.status.success() {
            return Ok(None);
        }
        Ok(parse_stats(&String::from_utf8_lossy(&out.stdout)))
    }

    async fn kill(&self, name: &str) -> anyhow::Result<()> {
        // A room that already stopped makes `docker kill` fail; that is fine.
        let _ = self.output(&["kill", "--signal", "KILL", name]).await?;
        Ok(())
    }

    async fn exit_state(&self, name: &str) -> anyhow::Result<ExitState> {
        let json = self
            .ok(&[
                "inspect",
                "--type",
                "container",
                "--format",
                "{{json .State}}",
                name,
            ])
            .await?;
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct State {
            exit_code: Option<i64>,
            #[serde(rename = "OOMKilled")]
            oom_killed: Option<bool>,
        }
        let s: State = serde_json::from_str(&json)?;
        Ok(ExitState {
            code: s.exit_code,
            oom_killed: s.oom_killed.unwrap_or(false),
        })
    }

    async fn remove(&self, name: &str) -> anyhow::Result<()> {
        let _ = self.output(&["rm", "--force", "--volumes", name]).await?;
        Ok(())
    }

    async fn exists(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self
            .output(&["inspect", "--type", "container", name])
            .await?
            .status
            .success())
    }

    async fn leftovers(&self) -> anyhow::Result<Vec<String>> {
        let filter = format!("label={JOB_LABEL}");
        let rooms = self
            .ok(&["ps", "--all", "--filter", &filter, "--format", "{{.Names}}"])
            .await?;
        Ok(rooms
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect())
    }
}

/// Parses one line of `docker stats --format '{{json .}}'`.
pub fn parse_stats(line: &str) -> Option<Usage> {
    #[derive(Deserialize)]
    struct Line {
        #[serde(rename = "CPUPerc")]
        cpu: String,
        #[serde(rename = "MemUsage")]
        mem: String,
        #[serde(rename = "PIDs")]
        pids: String,
    }
    let l: Line = serde_json::from_str(line.lines().next()?.trim()).ok()?;
    let cpu_pct = l.cpu.trim().trim_end_matches('%').parse().unwrap_or(0.0);
    let memory_mb = l
        .mem
        .split('/')
        .next()
        .and_then(parse_size_mib)
        .unwrap_or(0.0);
    let pids = l.pids.trim().parse().unwrap_or(0);
    Some(Usage {
        cpu_pct,
        memory_mb,
        pids,
    })
}

/// Parses Docker's human sizes (`1.5GiB`, `512kB`, `0B`) into MiB.
pub fn parse_size_mib(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let bytes = match unit {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some(n * bytes / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(network: Option<&str>) -> RoomPlan {
        RoomPlan {
            job_id: "job-1".into(),
            name: "agx-room-job-1".into(),
            image: format!("room-fetch@sha256:{}", "b".repeat(64)),
            runtime: "runsc".into(),
            cpus: 2.0,
            memory_mb: 2048,
            pids: 256,
            tmpfs_mb: 512,
            in_dir: "/var/lib/agentrix-runner/jobs/job-1/in".into(),
            out_dir: "/var/lib/agentrix-runner/jobs/job-1/out".into(),
            network: network.map(str::to_string),
            ip: network.map(|_| Ipv4Addr::new(172, 30, 0, 10)),
            env: vec![("HOME".into(), "/tmp".into())],
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    #[test]
    fn every_room_is_sealed() {
        let a = plan(None).create_args();
        assert!(has_pair(&a, "--runtime", "runsc"));
        assert!(has_pair(&a, "--pull", "never"), "rooms never pull");
        assert!(a.contains(&"--read-only".to_string()));
        assert!(has_pair(&a, "--user", "65534:65534"));
        assert!(has_pair(&a, "--cap-drop", "ALL"));
        assert!(has_pair(&a, "--security-opt", "no-new-privileges"));
        assert!(has_pair(&a, "--memory", "2048m"));
        assert!(
            has_pair(&a, "--memory-swap", "2048m"),
            "no swap beyond the cap"
        );
        assert!(has_pair(&a, "--pids-limit", "256"));
        assert!(has_pair(&a, "--cpus", "2"));
        assert!(has_pair(&a, "--network", "none"));
        assert!(has_pair(
            &a,
            "--mount",
            "type=bind,source=/var/lib/agentrix-runner/jobs/job-1/in,target=/in,readonly"
        ));
        assert!(
            !a.iter()
                .any(|x| x == "--privileged" || x.contains("docker.sock"))
        );
        assert_eq!(
            a.last().unwrap(),
            &format!("room-fetch@sha256:{}", "b".repeat(64))
        );
        assert!(
            !a.contains(&"--ip".to_string()),
            "no address without a network"
        );
        let net = plan(Some("agx-rooms")).create_args();
        assert!(has_pair(&net, "--network", "agx-rooms"));
        assert!(has_pair(&net, "--ip", "172.30.0.10"));
    }

    #[test]
    fn tag_names_drop_only_the_tag() {
        assert_eq!(tag_name("room-fetch:dev"), "room-fetch");
        assert_eq!(
            tag_name("localhost:5000/room-fetch:dev"),
            "localhost:5000/room-fetch"
        );
        assert_eq!(
            tag_name("localhost:5000/room-fetch"),
            "localhost:5000/room-fetch"
        );
    }

    #[test]
    fn stats_lines_parse() {
        let line = r#"{"BlockIO":"0B / 0B","CPUPerc":"187.25%","Container":"x","ID":"x","MemPerc":"50%","MemUsage":"1.5GiB / 2GiB","Name":"x","NetIO":"0B / 0B","PIDs":"256"}"#;
        let u = parse_stats(line).unwrap();
        assert_eq!(u.pids, 256);
        assert!((u.memory_mb - 1536.0).abs() < 0.01);
        assert!((u.cpu_pct - 187.25).abs() < 0.01);
        assert_eq!(
            parse_size_mib("512kB").map(|m| (m * 1e6).round()),
            Some(488_281.0)
        );
        assert_eq!(parse_size_mib("0B"), Some(0.0));
    }
}
