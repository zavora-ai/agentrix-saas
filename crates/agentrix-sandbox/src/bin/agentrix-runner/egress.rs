//! The per-job egress proxy (design §3.1, S1-03).
//!
//! Each fetch or replay job gets its own `agentrix-egress` process on the rooms gateway
//! (`172.30.0.1`), on its slot's port (3128-3191), started with the spec's allowlist (a replay
//! gets none, so every attempt to leave is refused and recorded) and serving only its room's
//! address (`--peer`): rooms share one bridge, so without that a room could use another job's
//! proxy and allowlist. It runs as an unprivileged user with no Docker access. Its JSON-line
//! events become `request` and `refused` job events. The host's nftables
//! (`deploy/nftables/sandbox.nft`) let `agx-r*` bridges reach only these ports.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::spec::{JobEvent, Refusal};

/// How to start proxies.
#[derive(Debug, Clone)]
pub struct EgressConfig {
    /// Path to the `agentrix-egress` binary.
    pub bin: PathBuf,
    /// The uid and gid the proxy runs as; `None` keeps the runner's (development only).
    pub user: Option<(u32, u32)>,
    /// `setpriv` (util-linux), which drops to `user` before exec. Rust's `Command::uid` keeps
    /// the parent's supplementary groups when the parent is not root, and the runner is in the
    /// `docker` group: a proxy keeping that group could drive Docker. `setpriv` clears
    /// supplementary groups and the inheritable and ambient capabilities, and sets
    /// no-new-privileges, so the proxy starts with nothing.
    pub setpriv: PathBuf,
    /// Maximum bytes per direction per connection.
    pub max_bytes: u64,
}

/// One line the proxy writes (`agentrix_egress::proxy::Event`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProxyEvent {
    /// `CONNECT` or the HTTP method.
    pub method: String,
    /// Destination host.
    pub host: String,
    /// Destination port.
    pub port: u16,
    /// `allow` or `deny`.
    pub decision: String,
    /// Why it was refused or ended early.
    #[serde(default)]
    pub reason: Option<String>,
    /// Bytes from the room.
    #[serde(default)]
    pub bytes_up: u64,
    /// Bytes to the room.
    #[serde(default)]
    pub bytes_down: u64,
}

impl ProxyEvent {
    /// The job event this line becomes.
    pub fn into_job_event(self) -> JobEvent {
        if self.decision == "allow" {
            JobEvent::Request {
                method: self.method,
                host: self.host,
                port: self.port,
                bytes_up: self.bytes_up,
                bytes_down: self.bytes_down,
            }
        } else {
            JobEvent::Refused(Refusal {
                host: self.host,
                port: self.port,
                reason: self.reason.unwrap_or_else(|| "refused".into()),
            })
        }
    }
}

/// A running proxy.
#[derive(Debug)]
pub struct Proxy {
    child: Child,
}

impl Proxy {
    /// Starts a proxy for `job_id` on `listen` with `allow`, serving only `room`, and forwards
    /// its events to `tx`. Returns once the proxy accepts connections.
    pub async fn start(
        cfg: &EgressConfig,
        job_id: &str,
        listen: SocketAddr,
        room: Ipv4Addr,
        allow: &[String],
        tx: mpsc::Sender<JobEvent>,
    ) -> anyhow::Result<Self> {
        let mut cmd = match cfg.user {
            Some((uid, gid)) => {
                let mut c = Command::new(&cfg.setpriv);
                c.arg(format!("--reuid={uid}"))
                    .arg(format!("--regid={gid}"))
                    .arg("--clear-groups")
                    .arg("--inh-caps=-all")
                    .arg("--ambient-caps=-all")
                    .arg("--no-new-privs")
                    .arg("--")
                    .arg(&cfg.bin);
                c
            }
            None => Command::new(&cfg.bin),
        };
        cmd.arg("--job")
            .arg(job_id)
            .arg("--listen")
            .arg(listen.to_string())
            .arg("--max-bytes")
            .arg(cfg.max_bytes.to_string())
            .arg("--peer")
            .arg(room.to_string())
            .env_clear()
            .env("RUST_LOG", "info")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for origin in allow {
            cmd.arg("--allow").arg(origin);
        }
        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("egress proxy has no stdout"))?;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.len() > 16 * 1024 {
                    continue;
                }
                if let Ok(ev) = serde_json::from_str::<ProxyEvent>(&line)
                    && tx.send(ev.into_job_event()).await.is_err()
                {
                    break;
                }
            }
        });
        // Ready when the proxy says so on stderr. Probing its port instead would connect from
        // the host's own address, which the proxy rightly refuses and records as a refusal.
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("egress proxy has no stderr"))?;
        let mut lines = BufReader::new(stderr).lines();
        let ready = tokio::time::timeout(Duration::from_secs(3), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.contains("egress proxy ready") {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        // Keep draining, so the proxy never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let proxy = Self { child };
        anyhow::ensure!(
            ready,
            "egress proxy for {job_id} did not start listening on {listen}"
        );
        Ok(proxy)
    }

    /// Stops the proxy and confirms it exited.
    pub async fn stop(mut self) -> bool {
        let _ = self.child.start_kill();
        matches!(
            tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await,
            Ok(Ok(_))
        )
    }
}

/// The room's proxy environment: both spellings, since tools disagree. `NO_PROXY` is empty so
/// nothing bypasses it.
pub fn proxy_env(listen: SocketAddr) -> Vec<(String, String)> {
    let url = format!("http://{listen}");
    vec![
        ("HTTP_PROXY".into(), url.clone()),
        ("HTTPS_PROXY".into(), url.clone()),
        ("http_proxy".into(), url.clone()),
        ("https_proxy".into(), url),
        ("NO_PROXY".into(), String::new()),
        ("no_proxy".into(), String::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_lines_become_job_events() {
        let allow = r#"{"ts_ms":1,"job":"j","method":"CONNECT","host":"kilele.example","port":443,"decision":"allow","reason":null,"ip":"203.0.113.9","bytes_up":10,"bytes_down":2000,"ms":30}"#;
        let ev: ProxyEvent = serde_json::from_str(allow).unwrap();
        assert!(matches!(
            ev.into_job_event(),
            JobEvent::Request {
                port: 443,
                bytes_down: 2000,
                ..
            }
        ));
        let deny = r#"{"ts_ms":1,"job":"j","method":"GET","host":"169.254.169.254","port":80,"decision":"deny","reason":"non-public address","ip":null,"bytes_up":0,"bytes_down":0,"ms":0}"#;
        let ev: ProxyEvent = serde_json::from_str(deny).unwrap();
        assert_eq!(
            ev.into_job_event(),
            JobEvent::Refused(Refusal {
                host: "169.254.169.254".into(),
                port: 80,
                reason: "non-public address".into()
            })
        );
    }

    #[test]
    fn rooms_get_both_proxy_spellings_and_no_bypass() {
        let env = proxy_env("172.30.0.1:3128".parse().unwrap());
        assert!(env.contains(&("HTTPS_PROXY".into(), "http://172.30.0.1:3128".into())));
        assert!(env.contains(&("no_proxy".into(), String::new())));
    }
}
