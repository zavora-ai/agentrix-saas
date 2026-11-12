//! One job from admission to result (design §4.2).
//!
//! Admission checks the signature, the rules, that the job id was never used, and that the
//! inputs are here. A run then: prepares `/in` and `/out`; for fetch and replay, starts the
//! job's egress proxy on its slot's port of the shared rooms network (`agx-rooms`, deploy
//! README), serving only the slot's room address; creates and starts the room at that address;
//! watches it against its caps; and, however it ends, destroys the room and the proxy and
//! confirms both are gone. Only then does it read `/out`. The stream is
//! `started`, then `request`/`refused`/`frame`/`note` while it runs, `killed` if the runner
//! ended it, `destroyed`, one `artifact` per file, and `result` last.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use chrono::Utc;
use ed25519_dalek::VerifyingKey;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::cas::Cas;
use crate::collect;
use crate::egress::{EgressConfig, Proxy, proxy_env};
use crate::room::{ROOM_UID, RoomPlan, RoomRuntime};
use crate::spec::{
    Digest, JobEvent, JobResult, JobSpec, KillReason, Peaks, Refusal, SignedJobSpec, SpecError,
    proxied, verify,
};

/// Seconds between usage samples.
const SAMPLE_EVERY: Duration = Duration::from_secs(1);
/// Consecutive samples at a cap before the runner kills the room for it.
const AT_CAP_SAMPLES: u32 = 3;
/// Longest line read from a room's stdout.
const MAX_LINE: usize = 4 * 1024 * 1024;
/// Most frames kept from one room.
const MAX_FRAMES: u32 = 900;
/// Most frame bytes kept from one room.
const MAX_FRAME_BYTES: u64 = 128 * 1024 * 1024;

/// The runner: its engine, store, key and limits.
#[derive(Debug)]
pub struct Runner<R> {
    /// The container engine.
    pub runtime: R,
    /// Blobs and artifacts.
    pub cas: Cas,
    /// Per-job directories live under `<work>/jobs`.
    pub work: PathBuf,
    /// Control's job-signing key.
    pub key: VerifyingKey,
    /// The OCI runtime rooms run under.
    pub oci_runtime: String,
    /// How to start egress proxies.
    pub egress: EgressConfig,
    /// Job ids already accepted, also kept in `<work>/seen-jobs`.
    pub seen: Mutex<HashSet<String>>,
    /// Room slots: each a room address and a proxy port on the shared rooms network.
    pub slots: SlotPool,
    /// The shared rooms network.
    pub net: RoomsNet,
}

/// The shared rooms network (deploy/README.md): `agx-rooms`, bridge `agx-r0`, gateway
/// `172.30.0.1`, proxies on ports 3128-3191. Slot `i` owns room address `first_ip + i` and
/// proxy port `first_port + i`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomsNet {
    /// The Docker network.
    pub network: String,
    /// The gateway, where proxies listen.
    pub gateway: Ipv4Addr,
    /// The first proxy port.
    pub first_port: u16,
    /// The first room address.
    pub first_ip: Ipv4Addr,
}

impl RoomsNet {
    /// Slot `i`'s proxy port.
    pub fn port(&self, i: usize) -> u16 {
        self.first_port + u16::try_from(i).unwrap_or(u16::MAX)
    }

    /// Slot `i`'s room address.
    pub fn room_ip(&self, i: usize) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.first_ip) + u32::try_from(i).unwrap_or(0))
    }
}

/// Room slots in use.
#[derive(Debug, Clone)]
pub struct SlotPool {
    used: Arc<Mutex<Vec<bool>>>,
}

impl SlotPool {
    /// `n` slots.
    pub fn new(n: usize) -> Self {
        Self {
            used: Arc::new(Mutex::new(vec![false; n])),
        }
    }

    /// Takes the lowest free slot.
    pub fn take(&self) -> Option<Slot> {
        let mut used = self
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = used.iter().position(|u| !u)?;
        used[index] = true;
        Some(Slot {
            index,
            used: Arc::clone(&self.used),
        })
    }

    /// Free slots.
    pub fn free(&self) -> usize {
        self.used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|u| !**u)
            .count()
    }
}

/// A slot, given back when dropped.
#[derive(Debug)]
pub struct Slot {
    index: usize,
    used: Arc<Mutex<Vec<bool>>>,
}

impl Slot {
    /// Its number.
    pub fn index(&self) -> usize {
        self.index
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(u) = self
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(self.index)
        {
            *u = false;
        }
    }
}

/// Why `POST /jobs` was refused before any room existed.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    /// Bad signature or rules.
    #[error(transparent)]
    Spec(#[from] SpecError),
    /// The job id was used before.
    #[error("job {0} was already run")]
    Replayed(String),
    /// Inputs that were never uploaded.
    #[error("inputs not uploaded: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))]
    MissingInputs(Vec<Digest>),
    /// Every room slot is busy.
    #[error("all rooms are busy; try again shortly")]
    Busy,
    /// Disk trouble on the runner.
    #[error("runner storage: {0}")]
    Io(#[from] std::io::Error),
}

/// A job that passed admission, holding its room slot.
#[derive(Debug)]
pub struct Admitted {
    spec: JobSpec,
    slot: Slot,
}

/// Docker names for one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    /// The room container.
    pub room: String,
}

impl Names {
    /// The names for `job_id`.
    pub fn for_job(job_id: &str) -> Self {
        Self {
            room: format!("agx-room-{job_id}"),
        }
    }
}

/// Counts that end up on the result.
#[derive(Debug, Default)]
struct Tally {
    requests: u64,
    refusals: Vec<Refusal>,
    peaks: Peaks,
}

/// Sends events to the caller and counts them for the result. A caller that hangs up does not
/// stop the job: the room is still destroyed.
#[derive(Debug, Clone)]
struct Recorder {
    tx: mpsc::Sender<JobEvent>,
    tally: Arc<Mutex<Tally>>,
}

impl Recorder {
    async fn emit(&self, ev: JobEvent) {
        {
            let mut t = self
                .tally
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &ev {
                JobEvent::Request { .. } => t.requests += 1,
                JobEvent::Refused(r) if t.refusals.len() < 10_000 => t.refusals.push(r.clone()),
                _ => {}
            }
        }
        let _ = self.tx.send(ev).await;
    }

    fn peaks(&self, f: impl FnOnce(&mut Peaks)) {
        f(&mut self
            .tally
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .peaks);
    }
}

/// What a run leaves to tear down.
#[derive(Debug, Default)]
struct Built {
    room: bool,
    proxy: Option<Proxy>,
}

impl<R: RoomRuntime> Runner<R> {
    /// Checks a signed spec and takes a room slot.
    pub async fn admit(&self, signed: SignedJobSpec) -> Result<Admitted, Refused> {
        let spec = verify(&signed, &self.key, Utc::now())?.clone();
        let missing: Vec<Digest> = {
            let mut m = Vec::new();
            for d in &spec.inputs {
                if !self.cas.has(d).await {
                    m.push(*d);
                }
            }
            m
        };
        if !missing.is_empty() {
            return Err(Refused::MissingInputs(missing));
        }
        let slot = self.slots.take().ok_or(Refused::Busy)?;
        {
            let mut seen = self
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !seen.insert(spec.job_id.to_string()) {
                return Err(Refused::Replayed(spec.job_id.to_string()));
            }
        }
        use tokio::io::AsyncWriteExt as _;
        let mut journal = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.work.join("seen-jobs"))
            .await?;
        journal
            .write_all(format!("{}\n", spec.job_id).as_bytes())
            .await?;
        Ok(Admitted { spec, slot })
    }

    /// Runs an admitted job to its result. The result is always the last event.
    pub async fn run(self: Arc<Self>, job: Admitted, tx: mpsc::Sender<JobEvent>) {
        let Admitted { spec, slot } = job;
        let names = Names::for_job(spec.job_id.as_str());
        let rec = Recorder {
            tx,
            tally: Arc::default(),
        };
        let started_at = Utc::now();
        let dir = self.work.join("jobs").join(spec.job_id.as_str());
        let mut built = Built::default();

        let ended = self
            .run_room(&spec, &names, &slot, &dir, &rec, &mut built)
            .await;
        let (exit_code, killed) = match ended {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(job = %spec.job_id, error = %e, "job failed outside the room");
                rec.emit(JobEvent::Error {
                    message: format!("{e:#}"),
                })
                .await;
                (None, None)
            }
        };

        let destroyed = self.destroy(&names, built).await;
        rec.emit(JobEvent::Destroyed {
            confirmed: destroyed,
        })
        .await;

        let mut artifacts = Vec::new();
        if destroyed {
            match collect::collect(&dir.join("out"), &self.cas).await {
                Ok((arts, skipped)) => {
                    for a in &arts {
                        rec.emit(JobEvent::Artifact(a.clone())).await;
                    }
                    if skipped > 0 {
                        rec.emit(JobEvent::Note {
                            text: format!(
                                "{skipped} entries in /out were not regular files and were skipped"
                            ),
                        })
                        .await;
                    }
                    artifacts = arts;
                }
                Err(e) => {
                    rec.emit(JobEvent::Error {
                        message: format!("collecting /out: {e}"),
                    })
                    .await
                }
            }
        } else {
            rec.emit(JobEvent::Error {
                message: "the room could not be confirmed destroyed; /out was not read".into(),
            })
            .await;
        }
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            tracing::warn!(job = %spec.job_id, error = %e, "could not remove the job directory");
        }

        let (requests, refusals, peaks) = {
            let t = rec
                .tally
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (t.requests, t.refusals.clone(), t.peaks)
        };
        let result = JobResult {
            job_id: spec.job_id.clone(),
            runtime: self.oci_runtime.clone(),
            exit_code,
            killed,
            artifacts,
            requests,
            refusals,
            peaks,
            destroyed,
            started_at,
            ended_at: Utc::now(),
        };
        rec.emit(JobEvent::Result(Box::new(result))).await;
    }

    /// Everything up to the room's end. Returns its exit code and why the runner killed it.
    async fn run_room(
        &self,
        spec: &JobSpec,
        names: &Names,
        slot: &Slot,
        dir: &Path,
        rec: &Recorder,
        built: &mut Built,
    ) -> anyhow::Result<(Option<i64>, Option<KillReason>)> {
        let (in_dir, out_dir) = self.prepare_dirs(spec, dir).await?;

        let mut env = vec![("HOME".to_string(), "/tmp".to_string())];
        let (mut network, mut ip) = (None, None);
        if proxied(spec.kind) {
            let room_ip = self.net.room_ip(slot.index());
            let listen = SocketAddr::from((self.net.gateway, self.net.port(slot.index())));
            let (ptx, mut prx) = mpsc::channel(256);
            built.proxy = Some(
                Proxy::start(
                    &self.egress,
                    spec.job_id.as_str(),
                    listen,
                    room_ip,
                    &spec.allowlist,
                    ptx,
                )
                .await?,
            );
            let r = rec.clone();
            tokio::spawn(async move {
                while let Some(ev) = prx.recv().await {
                    r.emit(ev).await;
                }
            });
            env.extend(proxy_env(listen));
            network = Some(self.net.network.clone());
            ip = Some(room_ip);
        }

        let plan = RoomPlan {
            job_id: spec.job_id.to_string(),
            name: names.room.clone(),
            image: spec.image.as_str().to_string(),
            runtime: self.oci_runtime.clone(),
            cpus: spec.caps.cpus,
            memory_mb: spec.caps.memory_mb,
            pids: spec.caps.pids,
            tmpfs_mb: (spec.caps.memory_mb / 2).clamp(32, 1024),
            in_dir,
            out_dir: out_dir.clone(),
            network,
            ip,
            env,
        };
        self.runtime.create(&plan).await?;
        built.room = true;

        let wall = Duration::from_secs(u64::from(spec.caps.seconds));
        let until_deadline = (spec.deadline - Utc::now()).to_std().unwrap_or_default();
        let deadline = tokio::time::Instant::now() + wall.min(until_deadline);

        let mut child = self.runtime.start_attached(&names.room).await?;
        rec.emit(JobEvent::Started {
            job_id: spec.job_id.clone(),
            runtime: self.oci_runtime.clone(),
            at: Utc::now(),
        })
        .await;
        if let Some(stdout) = child.stdout.take() {
            let (r, cas) = (rec.clone(), self.cas.clone());
            tokio::spawn(async move { read_room_stdout(BufReader::new(stdout), &cas, &r).await });
        }
        // The room's stderr is read only for one thing: a process failing to start because a
        // cap is reached. Under gVisor the process cap shows as fork or exec failing with
        // ENOMEM ("sh: sleep: Out of memory"), never as a host-visible process count, so this
        // is how the runner learns a fork bomb hit its cap.
        let (cap_tx, mut cap_rx) = tokio::sync::watch::channel(false);
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut buf = Vec::new();
                while let Ok(Some(within)) =
                    read_bounded_line(&mut reader, &mut buf, 64 * 1024).await
                {
                    if within && start_failure(&buf) {
                        let _ = cap_tx.send(true);
                    }
                }
            });
        }
        let mut watch_stderr = true;

        let caps = spec.caps;
        let out_cap = u64::from(caps.out_mb) * 1024 * 1024;
        let mut tick = tokio::time::interval(SAMPLE_EVERY);
        let (mut at_pids, mut at_mem) = (0u32, 0u32);
        let mut kill = None;
        loop {
            tokio::select! {
                _ = child.wait() => break,
                changed = cap_rx.changed(), if watch_stderr => {
                    match changed {
                        Ok(()) if *cap_rx.borrow() => {
                            let memory = rec.tally.lock().unwrap_or_else(std::sync::PoisonError::into_inner).peaks.memory_mb;
                            kill = Some(if memory >= f64::from(caps.memory_mb) * 0.8 { KillReason::Memory } else { KillReason::Pids });
                            break;
                        }
                        Ok(()) => {}
                        Err(_) => watch_stderr = false,
                    }
                }
                () = tokio::time::sleep_until(deadline) => { kill = Some(KillReason::Deadline); break; }
                _ = tick.tick() => {
                    if let Ok(Some(u)) = self.runtime.stats(&names.room).await {
                        rec.peaks(|p| {
                            p.cpu_pct = p.cpu_pct.max(u.cpu_pct);
                            p.memory_mb = p.memory_mb.max(u.memory_mb);
                            p.pids = p.pids.max(u.pids);
                        });
                        at_pids = if u.pids >= u64::from(caps.pids) { at_pids + 1 } else { 0 };
                        at_mem = if u.memory_mb >= f64::from(caps.memory_mb) * 0.97 { at_mem + 1 } else { 0 };
                        if at_pids >= AT_CAP_SAMPLES { kill = Some(KillReason::Pids); break; }
                        if at_mem >= AT_CAP_SAMPLES { kill = Some(KillReason::Memory); break; }
                    }
                    let out = out_dir.clone();
                    let used = tokio::task::spawn_blocking(move || collect::size(&out)).await.unwrap_or(0);
                    if used > out_cap { kill = Some(KillReason::OutCap); break; }
                }
            }
        }
        // A room that exits right after a start failure can beat the signal: give its stderr
        // a moment to finish, then look again.
        if kill.is_none() && watch_stderr {
            let _ = tokio::time::timeout(Duration::from_millis(500), async {
                while !*cap_rx.borrow() && cap_rx.changed().await.is_ok() {}
            })
            .await;
            if *cap_rx.borrow() {
                let memory = rec
                    .tally
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .peaks
                    .memory_mb;
                kill = Some(if memory >= f64::from(caps.memory_mb) * 0.8 {
                    KillReason::Memory
                } else {
                    KillReason::Pids
                });
            }
        }
        if let Some(reason) = kill {
            self.runtime.kill(&names.room).await?;
            let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
            rec.emit(JobEvent::Killed { reason }).await;
        }
        let state = self
            .runtime
            .exit_state(&names.room)
            .await
            .unwrap_or_default();
        if kill.is_none() && state.oom_killed {
            kill = Some(KillReason::Memory);
            rec.emit(JobEvent::Killed {
                reason: KillReason::Memory,
            })
            .await;
        }
        Ok((if kill.is_some() { None } else { state.code }, kill))
    }

    /// `<dir>/in` (read-only to the room: `job.json` and each input by hex digest) and
    /// `<dir>/out` (owned by the room's user).
    async fn prepare_dirs(&self, spec: &JobSpec, dir: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
        let (in_dir, out_dir) = (dir.join("in"), dir.join("out"));
        tokio::fs::create_dir_all(&in_dir).await?;
        tokio::fs::create_dir_all(&out_dir).await?;
        tokio::fs::write(in_dir.join("job.json"), serde_json::to_vec(&spec.args)?).await?;
        for d in &spec.inputs {
            let dst = in_dir.join(d.hex());
            if tokio::fs::hard_link(self.cas.path(d), &dst).await.is_err() {
                tokio::fs::copy(self.cas.path(d), &dst).await?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            tokio::fs::set_permissions(&in_dir, std::fs::Permissions::from_mode(0o755)).await?;
            tokio::fs::set_permissions(&out_dir, std::fs::Permissions::from_mode(0o700)).await?;
            if let Err(e) = std::os::unix::fs::chown(&out_dir, Some(ROOM_UID), Some(ROOM_UID)) {
                tracing::warn!(error = %e, "could not give /out to the room's user (not running as root?)");
            }
        }
        Ok((in_dir, out_dir))
    }

    /// Removes the room and stops the proxy, then confirms both are gone.
    async fn destroy(&self, names: &Names, built: Built) -> bool {
        let mut room_gone = true;
        if built.room {
            for _ in 0..3 {
                let _ = self.runtime.remove(&names.room).await;
                room_gone = matches!(self.runtime.exists(&names.room).await, Ok(false));
                if room_gone {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        let proxy_gone = match built.proxy {
            Some(p) => p.stop().await,
            None => true,
        };
        room_gone && proxy_gone
    }
}

/// Reads the room's stdout: `{"event":"frame","b64":"…"}` (PNG or JPEG) becomes a stored frame,
/// `{"event":"note","text":"…"}` a note. Frames travel in the stream, never as paths, so the
/// runner never opens a file the room could have swapped for a link. Anything else is ignored.
async fn read_room_stdout<B: AsyncBufRead + Unpin>(mut reader: B, cas: &Cas, rec: &Recorder) {
    #[derive(serde::Deserialize)]
    #[serde(tag = "event", rename_all = "snake_case")]
    enum Line {
        Frame { b64: String },
        Note { text: String },
    }
    let (mut frames, mut frame_bytes) = (0u32, 0u64);
    let mut buf = Vec::new();
    loop {
        match read_bounded_line(&mut reader, &mut buf, MAX_LINE).await {
            Ok(None) => break,
            Ok(Some(false)) => continue,
            Ok(Some(true)) => {}
            Err(_) => break,
        }
        match serde_json::from_slice::<Line>(&buf) {
            Ok(Line::Frame { b64 }) if frames < MAX_FRAMES && frame_bytes < MAX_FRAME_BYTES => {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
                    && let Ok(digest) = cas.put_bytes(&bytes).await
                {
                    frames += 1;
                    frame_bytes += bytes.len() as u64;
                    rec.emit(JobEvent::Frame { digest }).await;
                }
            }
            Ok(Line::Note { text }) => {
                let text: String = text.chars().filter(|c| !c.is_control()).take(200).collect();
                rec.emit(JobEvent::Note { text }).await;
            }
            _ => {}
        }
    }
}

/// Whether a stderr line says a process could not start for want of a process slot or memory:
/// what shells print when a fork bomb meets the room's caps, under gVisor or not.
fn start_failure(line: &[u8]) -> bool {
    let line = String::from_utf8_lossy(line).to_ascii_lowercase();
    [
        "can't fork",
        "cannot fork",
        "fork: retry",
        "fork failed",
        "out of memory",
        "cannot allocate memory",
    ]
    .iter()
    .any(|p| line.contains(p))
}

/// Reads one line into `buf`. `Some(true)`: a line within `max`; `Some(false)`: a longer line,
/// skipped; `None`: end of stream.
async fn read_bounded_line<B: AsyncBufRead + Unpin>(
    reader: &mut B,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Option<bool>> {
    buf.clear();
    let mut too_long = false;
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(if buf.is_empty() && !too_long {
                None
            } else {
                Some(!too_long)
            });
        }
        let (part, done) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (&chunk[..i], Some(i + 1)),
            None => (chunk, None),
        };
        if !too_long {
            if buf.len() + part.len() > max {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(part);
            }
        }
        let used = done.unwrap_or(chunk.len());
        reader.consume(used);
        if done.is_some() {
            return Ok(Some(!too_long));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_give_each_room_its_own_address_and_proxy_port() {
        let net = RoomsNet {
            network: "agx-rooms".into(),
            gateway: Ipv4Addr::new(172, 30, 0, 1),
            first_port: 3128,
            first_ip: Ipv4Addr::new(172, 30, 0, 10),
        };
        let pool = SlotPool::new(2);
        let a = pool.take().unwrap();
        let b = pool.take().unwrap();
        assert!(pool.take().is_none(), "two slots, two rooms");
        assert_eq!(
            (net.port(a.index()), net.room_ip(a.index())),
            (3128, Ipv4Addr::new(172, 30, 0, 10))
        );
        assert_eq!(
            (net.port(b.index()), net.room_ip(b.index())),
            (3129, Ipv4Addr::new(172, 30, 0, 11))
        );
        drop(a);
        assert_eq!(pool.free(), 1);
        assert_eq!(pool.take().unwrap().index(), 0, "a freed slot is reused");
    }

    #[test]
    fn cap_failures_are_recognised_in_every_shell() {
        for line in [
            "sh: sleep: Out of memory",
            "sh: 1: sleep: Cannot allocate memory",
            "bash: fork: retry: Resource temporarily unavailable",
            "sh: can't fork: Resource temporarily unavailable",
        ] {
            assert!(start_failure(line.as_bytes()), "{line}");
        }
        assert!(!start_failure(b"GET /index.html 200"));
    }

    #[tokio::test]
    async fn long_lines_are_skipped_and_reading_goes_on() {
        let mut input = vec![b'x'; 100];
        input.push(b'\n');
        input.extend_from_slice(b"{\"event\":\"note\",\"text\":\"ok\"}\n");
        let mut r = BufReader::with_capacity(16, &input[..]);
        let mut buf = Vec::new();
        assert_eq!(
            read_bounded_line(&mut r, &mut buf, 50).await.unwrap(),
            Some(false)
        );
        assert_eq!(
            read_bounded_line(&mut r, &mut buf, 50).await.unwrap(),
            Some(true)
        );
        assert_eq!(buf, b"{\"event\":\"note\",\"text\":\"ok\"}");
        assert_eq!(read_bounded_line(&mut r, &mut buf, 50).await.unwrap(), None);
    }

    #[tokio::test]
    async fn frames_are_stored_and_notes_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let cas = Cas::open(dir.path()).await.unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let rec = Recorder {
            tx,
            tally: Arc::default(),
        };
        let jpeg = base64::engine::general_purpose::STANDARD.encode(b"\xff\xd8frame");
        let input = format!(
            "{{\"event\":\"frame\",\"b64\":\"{jpeg}\"}}\n{{\"event\":\"note\",\"text\":\"Read 3 pages\\u0007\"}}\nnoise\n"
        );
        read_room_stdout(BufReader::new(input.as_bytes()), &cas, &rec).await;
        drop(rec);
        let ev = rx.recv().await.unwrap();
        assert_eq!(
            ev,
            JobEvent::Frame {
                digest: Digest::of(b"\xff\xd8frame")
            }
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            JobEvent::Note {
                text: "Read 3 pages".into()
            }
        );
        assert!(rx.recv().await.is_none());
    }
}
