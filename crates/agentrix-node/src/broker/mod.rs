//! Broker client: sends control-signed job specs to `agentrix-runner` on the sandbox VM and
//! folds what comes back into checked artifacts and a signed receipt (design §4.2, D6).
//!
//! Lane: `assess` (S1-05), handed over by the coordinator (design §3.0).
//!
//! One job, start to finish ([`Broker::run`]):
//! 1. Build the [`JobSpec`] (pinned image, inputs by digest, allowlist, caps, deadline, args)
//!    and sign it with control's Ed25519 key ([`SignedJobSpec::sign`]).
//! 2. Upload each input with `PUT /blobs/{digest}`.
//! 3. `POST /jobs` and read the NDJSON [`JobEvent`] stream to its `result` line, handing each
//!    event to the caller as a [`BrokerEvent`]. Frames are fetched as they arrive, checked
//!    against their digest and stored in control's CAS, so the browser only ever sees
//!    `/frames/{digest}`.
//! 4. Fetch every artifact with `GET /artifacts/{digest}`, check its digest and size, and store
//!    it in control's CAS.
//! 5. Write the job's receipt (`job.fetch`, `job.render`, …) through the receipts store, which
//!    signs it (design §4.4).
//!
//! The transport is behind [`RunnerApi`]: [`HttpRunner`] speaks to the real runner over the
//! VPC; [`fake::FakeRunner`] speaks the same protocol in tests. Storage is behind
//! [`ReceiptIssuer`] and [`BlobStore`], with in-memory versions in [`memory`].
//!
//! Callers: `assess/` (public crawl), `claim/` through [`fetch::Broker::probe_txt`] and
//! [`fetch::Broker::probe_file`] (R3.1), and the Move (move lane) for `build`, `render` and
//! `replay` jobs through [`Broker::run`].

pub mod claim_probe;
pub mod db;
pub mod fake;
pub mod fetch;
mod http;
pub mod memory;
pub mod ui;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use agentrix_types::{
    AssessmentId, Caps, Digest, ImageRef, JobEvent, JobId, JobKind, JobResult, JobSpec, MoveId,
    Observed, ReceiptBody, SignedJobSpec, Signer, SiteId,
};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, SubsecRound as _, Utc};
use futures::StreamExt as _;
use futures::stream::BoxStream;

pub use http::HttpRunner;

/// The runner's HTTP API (design §4.2), as the broker uses it.
#[async_trait]
pub trait RunnerApi: Send + Sync + 'static {
    /// `PUT /blobs/{digest}`: uploads one input. Uploading a blob the runner holds is a no-op.
    async fn put_blob(&self, digest: &Digest, bytes: Bytes) -> Result<(), BrokerError>;
    /// `POST /jobs`: starts a job and returns its event stream, which ends with `result`.
    async fn post_job(&self, signed: &SignedJobSpec) -> Result<EventStream, BrokerError>;
    /// `GET /artifacts/{digest}`: one artifact or frame, unchecked.
    async fn get_artifact(&self, digest: &Digest) -> Result<Bytes, BrokerError>;
}

/// A job's events, in the order the runner sent them.
pub type EventStream = BoxStream<'static, Result<JobEvent, BrokerError>>;

/// Signs and stores receipts: the data lane's receipts store in production
/// (`db::receipts::ReceiptStore`), [`memory::MemoryReceipts`] in tests.
#[async_trait]
pub trait ReceiptIssuer: Send + Sync + 'static {
    /// Signs `body` with control's key, stores it and returns its digest.
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Digest>;
}

/// Control's content-addressed store, with the per-site index that decides who may read a
/// blob: the data lane's `db::cas` in production, [`memory::MemoryBlobs`] in tests.
#[async_trait]
pub trait BlobStore: Send + Sync + 'static {
    /// Stores `bytes`, records that `site` holds them as `kind` (`page`, `frame`, `evidence`,
    /// …) with `content_type`, and returns their digest.
    async fn put(
        &self,
        site: &SiteId,
        kind: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest>;
    /// The bytes and content type of a blob `site` holds.
    async fn get(
        &self,
        site: &SiteId,
        digest: &Digest,
    ) -> anyhow::Result<Option<(Vec<u8>, String)>>;
}

/// Why a job could not be run or read, in words fit for logs; [`BrokerError::plain`] gives the
/// owner-facing line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    /// The runner could not be reached, or the connection broke.
    Unreachable(String),
    /// The runner refused the job before any room existed.
    Refused {
        /// The HTTP status.
        status: u16,
        /// The runner's reason.
        message: String,
    },
    /// The runner said something the protocol does not allow.
    Protocol(String),
    /// An artifact did not match its digest or size.
    Artifact(String),
    /// No room image is configured for this kind of job.
    NoImage(JobKind),
    /// Signing, storing or the receipt failed on control.
    Control(String),
}

impl BrokerError {
    /// One plain-words line for the owner (never a trace).
    pub fn plain(&self) -> String {
        match self {
            Self::Unreachable(_) => "Our sealed rooms aren't reachable right now.".into(),
            Self::Refused { status: 429, .. } => {
                "Every sealed room is busy right now; try again in a minute.".into()
            }
            Self::Refused { .. } => "The sandbox refused to start the room.".into(),
            Self::Protocol(_) | Self::Artifact(_) => {
                "The sealed room's results didn't check out, so we didn't use them.".into()
            }
            Self::NoImage(_) => "This kind of sealed room isn't set up on this server.".into(),
            Self::Control(_) => "Something went wrong on our side while recording the job.".into(),
        }
    }

    /// What the owner can do about it, in plain words. Waiting only helps when the rooms exist.
    pub fn advice(&self) -> String {
        match self {
            Self::Refused { status: 429, .. } => "Try again in a minute.".into(),
            Self::NoImage(_) => {
                "Waiting won't help: this server can't run the check until its sealed rooms are set up."
                    .into()
            }
            _ => "Try again in a few minutes.".into(),
        }
    }
}

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(m) => write!(f, "runner unreachable: {m}"),
            Self::Refused { status, message } => {
                write!(f, "runner refused the job ({status}): {message}")
            }
            Self::Protocol(m) => write!(f, "runner protocol: {m}"),
            Self::Artifact(m) => write!(f, "artifact check failed: {m}"),
            Self::NoImage(kind) => write!(f, "no room image configured for {kind:?} jobs"),
            Self::Control(m) => write!(f, "control: {m}"),
        }
    }
}

impl std::error::Error for BrokerError {}

/// The pinned room image for each kind of job (design §3.1).
#[derive(Debug, Clone, Default)]
pub struct Images {
    /// `room-fetch`.
    pub fetch: Option<ImageRef>,
    /// `room-replay`.
    pub replay: Option<ImageRef>,
    /// `room-render`.
    pub render: Option<ImageRef>,
    /// `room-build`.
    pub build: Option<ImageRef>,
}

impl Images {
    /// Reads `AGENTRIX_IMAGE_FETCH`, `_REPLAY`, `_RENDER` and `_BUILD` (each
    /// `name@sha256:<hex>`). An unset variable leaves that kind unavailable; a malformed one is
    /// an error, so a typo never runs an unpinned image.
    pub fn from_env() -> anyhow::Result<Self> {
        let read = |var: &str| -> anyhow::Result<Option<ImageRef>> {
            match std::env::var(var) {
                Ok(v) if !v.trim().is_empty() => ImageRef::parse(v.trim())
                    .map(Some)
                    .map_err(|e| anyhow::anyhow!("{var}: {e}")),
                _ => Ok(None),
            }
        };
        Ok(Self {
            fetch: read("AGENTRIX_IMAGE_FETCH")?,
            replay: read("AGENTRIX_IMAGE_REPLAY")?,
            render: read("AGENTRIX_IMAGE_RENDER")?,
            build: read("AGENTRIX_IMAGE_BUILD")?,
        })
    }

    /// The image for `kind`.
    pub fn get(&self, kind: JobKind) -> Option<&ImageRef> {
        match kind {
            JobKind::Fetch => self.fetch.as_ref(),
            JobKind::Replay => self.replay.as_ref(),
            JobKind::Render => self.render.as_ref(),
            JobKind::Build => self.build.as_ref(),
        }
    }
}

/// What a job belongs to, recorded in its receipt's `detail`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parent {
    /// A free assessment.
    Assessment(AssessmentId),
    /// A Move.
    Move(MoveId),
    /// A claim check.
    Claim(String),
    /// Nothing in particular.
    None,
}

/// One job to run.
#[derive(Debug, Clone)]
pub struct JobRequest {
    /// The site the job is for; every receipt and blob carries it.
    pub site: SiteId,
    /// What the room does.
    pub kind: JobKind,
    /// Input blobs, uploaded first and mounted read-only at `/in/<hex digest>`.
    pub inputs: Vec<Bytes>,
    /// Hosts the room may reach (fetch jobs only).
    pub allowlist: Vec<String>,
    /// Non-secret arguments, written to `/in/job.json`.
    pub args: serde_json::Value,
    /// Caps; the kind's defaults (design §3.1) when `None`.
    pub caps: Option<Caps>,
    /// What the job belongs to.
    pub parent: Parent,
    /// The receipt's plain-words title, e.g. "Read 41 pages of kilele.example in a sealed room".
    pub title: String,
    /// The CAS kind its artifacts are recorded under (`page`, `release`, …).
    pub artifact_kind: &'static str,
}

impl JobRequest {
    /// A request with no inputs, the kind's default caps and no parent.
    pub fn new(
        site: SiteId,
        kind: JobKind,
        args: serde_json::Value,
        title: impl Into<String>,
    ) -> Self {
        Self {
            site,
            kind,
            inputs: Vec::new(),
            allowlist: Vec::new(),
            args,
            caps: None,
            parent: Parent::None,
            title: title.into(),
            artifact_kind: "artifact",
        }
    }
}

/// An artifact fetched from the runner, checked and stored in control's CAS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Its path under the room's `/out`.
    pub name: String,
    /// Its digest, checked.
    pub digest: Digest,
    /// Its bytes.
    pub bytes: Bytes,
}

/// A frame from the room, checked and stored in control's CAS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInfo {
    /// Its digest; the browser fetches it at `/frames/{digest}`.
    pub digest: Digest,
    /// `image/png` or `image/jpeg`.
    pub content_type: &'static str,
    /// Width in pixels, 0 if unknown.
    pub w: u32,
    /// Height in pixels, 0 if unknown.
    pub h: u32,
}

/// What the broker tells its caller while a job runs.
#[derive(Debug, Clone, PartialEq)]
pub enum BrokerEvent {
    /// The spec was signed and the job admitted.
    Admitted {
        /// The job.
        job_id: JobId,
    },
    /// An event from the runner's stream (frames arrive as [`BrokerEvent::Frame`] instead).
    Job(JobEvent),
    /// A frame, already stored in control's CAS.
    Frame(FrameInfo),
}

/// What a finished job produced.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    /// The signed spec's content.
    pub spec: JobSpec,
    /// The runner's result line.
    pub result: JobResult,
    /// Every artifact, checked and stored.
    pub artifacts: Vec<Fetched>,
    /// Every frame, checked and stored.
    pub frames: Vec<FrameInfo>,
    /// What the receipt says was observed.
    pub observed: Observed,
    /// The signed receipt's digest.
    pub receipt: Digest,
}

impl JobOutcome {
    /// The artifact at `name` under `/out`.
    pub fn artifact(&self, name: &str) -> Option<&Fetched> {
        self.artifacts.iter().find(|a| a.name == name)
    }

    /// Whether the room ran to its own end and was destroyed.
    pub fn clean_exit(&self) -> bool {
        self.result.destroyed && self.result.killed.is_none() && self.result.exit_code == Some(0)
    }
}

/// Largest artifact the broker will fetch, in bytes.
pub const MAX_ARTIFACT: usize = 64 * 1024 * 1024;

/// The broker client.
#[derive(Clone)]
pub struct Broker {
    runner: Arc<dyn RunnerApi>,
    signer: Arc<Signer>,
    receipts: Arc<dyn ReceiptIssuer>,
    blobs: Arc<dyn BlobStore>,
    images: Images,
}

impl fmt::Debug for Broker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Broker")
            .field("signer", &self.signer)
            .field("images", &self.images)
            .finish_non_exhaustive()
    }
}

impl Broker {
    /// A broker that signs with `signer` (control's job key), talks to `runner`, stores blobs
    /// in `blobs` and receipts in `receipts`.
    pub fn new(
        runner: Arc<dyn RunnerApi>,
        signer: Arc<Signer>,
        receipts: Arc<dyn ReceiptIssuer>,
        blobs: Arc<dyn BlobStore>,
        images: Images,
    ) -> Self {
        Self {
            runner,
            signer,
            receipts,
            blobs,
            images,
        }
    }

    /// The blob store, for serving frames and evidence.
    pub fn blobs(&self) -> &Arc<dyn BlobStore> {
        &self.blobs
    }

    /// Whether jobs of `kind` can run here.
    pub fn can_run(&self, kind: JobKind) -> bool {
        self.images.get(kind).is_some()
    }

    /// Builds and signs the spec for `req`.
    pub fn sign(
        &self,
        req: &JobRequest,
        job_id: JobId,
        now: DateTime<Utc>,
    ) -> Result<SignedJobSpec, BrokerError> {
        let image = self
            .images
            .get(req.kind)
            .cloned()
            .ok_or(BrokerError::NoImage(req.kind))?;
        let caps = req.caps.unwrap_or_else(|| req.kind.default_caps());
        // Whole seconds, so the deadline's RFC 3339 form is the same after any round trip.
        let deadline =
            now.trunc_subsecs(0) + chrono::Duration::seconds(i64::from(caps.seconds) + 30);
        let spec = JobSpec {
            job_id,
            kind: req.kind,
            image,
            inputs: req.inputs.iter().map(|b| Digest::of(b)).collect(),
            allowlist: req.allowlist.clone(),
            caps,
            deadline,
            args: req.args.clone(),
        };
        SignedJobSpec::sign(spec, &self.signer).map_err(|e| BrokerError::Control(e.to_string()))
    }

    /// Runs one job to its signed receipt, handing events to `on_event` as they arrive.
    pub async fn run(
        &self,
        req: JobRequest,
        on_event: &mut (dyn FnMut(BrokerEvent) + Send),
    ) -> Result<JobOutcome, BrokerError> {
        let signed = self.sign(&req, JobId::generate(), Utc::now())?;
        for (bytes, digest) in req.inputs.iter().zip(&signed.spec.inputs) {
            self.runner.put_blob(digest, bytes.clone()).await?;
        }
        let mut stream = self.runner.post_job(&signed).await?;
        on_event(BrokerEvent::Admitted {
            job_id: signed.spec.job_id.clone(),
        });

        // The stream is bounded by the job's own deadline, plus a minute for collection.
        let wait = (signed.spec.deadline - Utc::now())
            .to_std()
            .unwrap_or_default()
            + Duration::from_secs(60);
        let read = async {
            let mut events = Vec::new();
            let mut frames = Vec::new();
            let mut result = None;
            while let Some(ev) = stream.next().await {
                let ev = ev?;
                match ev {
                    JobEvent::Result(r) => {
                        result = Some(*r);
                        break;
                    }
                    JobEvent::Frame { digest } => {
                        events.push(JobEvent::Frame { digest });
                        match self.frame(&req.site, &digest).await {
                            Ok(info) => {
                                frames.push(info.clone());
                                on_event(BrokerEvent::Frame(info));
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "dropped a frame that did not check out");
                            }
                        }
                    }
                    other => {
                        events.push(other.clone());
                        on_event(BrokerEvent::Job(other));
                    }
                }
            }
            Ok::<_, BrokerError>((events, frames, result))
        };
        let (events, frames, result) = tokio::time::timeout(wait, read).await.map_err(|_| {
            BrokerError::Protocol("the job's stream outlived its deadline".into())
        })??;
        let result = result.ok_or_else(|| {
            BrokerError::Protocol("the stream ended before the result line".into())
        })?;
        if result.job_id != signed.spec.job_id {
            return Err(BrokerError::Protocol(format!(
                "result is for job {}, not {}",
                result.job_id, signed.spec.job_id
            )));
        }

        let mut artifacts = Vec::with_capacity(result.artifacts.len());
        for a in &result.artifacts {
            let bytes = self.checked(&a.digest, Some(a.bytes)).await?;
            let content_type = content_type_for(&a.name, &bytes);
            self.blobs
                .put(&req.site, req.artifact_kind, content_type, &bytes)
                .await
                .map_err(|e| BrokerError::Control(format!("storing {}: {e:#}", a.name)))?;
            artifacts.push(Fetched {
                name: a.name.clone(),
                digest: a.digest,
                bytes,
            });
        }

        let (receipt, observed) = self
            .receipt(&req, &signed.spec, &result, &events, &frames)
            .await?;
        Ok(JobOutcome {
            spec: signed.spec,
            result,
            artifacts,
            frames,
            observed,
            receipt,
        })
    }

    async fn receipt(
        &self,
        req: &JobRequest,
        spec: &JobSpec,
        result: &JobResult,
        events: &[JobEvent],
        frames: &[FrameInfo],
    ) -> Result<(Digest, Observed), BrokerError> {
        let mut body = ReceiptBody::for_job(req.site.clone(), spec, result, events);
        let observed = body.observed.clone().unwrap_or(Observed {
            hosts: Vec::new(),
            requests: result.requests,
            bytes: 0,
            refusals: result.refusals.clone(),
            peaks: result.peaks,
        });
        if let Some(serde_json::Value::Object(d)) = &mut body.detail {
            d.insert("image".into(), serde_json::json!(spec.image.as_str()));
            d.insert("frames".into(), serde_json::json!(frames.len()));
            match &req.parent {
                Parent::Assessment(id) => {
                    d.insert("assessment".into(), serde_json::json!(id));
                }
                Parent::Move(id) => {
                    d.insert("move".into(), serde_json::json!(id));
                }
                Parent::Claim(id) => {
                    d.insert("claim".into(), serde_json::json!(id));
                }
                Parent::None => {}
            }
        }
        let body = body.with_title(req.title.clone());
        let digest = self
            .receipts
            .issue(body)
            .await
            .map_err(|e| BrokerError::Control(format!("issuing the receipt: {e:#}")))?;
        Ok((digest, observed))
    }

    /// Fetches `digest` from the runner and checks it.
    async fn checked(&self, digest: &Digest, size: Option<u64>) -> Result<Bytes, BrokerError> {
        let bytes = self.runner.get_artifact(digest).await?;
        if bytes.len() > MAX_ARTIFACT {
            return Err(BrokerError::Artifact(format!(
                "{digest} is larger than {MAX_ARTIFACT} bytes"
            )));
        }
        if let Some(size) = size
            && bytes.len() as u64 != size
        {
            return Err(BrokerError::Artifact(format!(
                "{digest} is {} bytes, the result says {size}",
                bytes.len()
            )));
        }
        let actual = Digest::of(&bytes);
        if &actual != digest {
            return Err(BrokerError::Artifact(format!(
                "asked for {digest}, got {actual}"
            )));
        }
        Ok(bytes)
    }

    /// Fetches, checks and stores one frame.
    async fn frame(&self, site: &SiteId, digest: &Digest) -> Result<FrameInfo, BrokerError> {
        let bytes = self.checked(digest, None).await?;
        let (content_type, w, h) = image_info(&bytes)
            .ok_or_else(|| BrokerError::Artifact(format!("frame {digest} is not a PNG or JPEG")))?;
        self.blobs
            .put(site, "frame", content_type, &bytes)
            .await
            .map_err(|e| BrokerError::Control(format!("storing a frame: {e:#}")))?;
        Ok(FrameInfo {
            digest: *digest,
            content_type,
            w,
            h,
        })
    }
}

/// A runner that isn't configured: every job fails, in plain words, as unreachable. Used when
/// `AGENTRIX_RUNNER` is unset, so nothing pretends to have read a site (the honesty rule).
#[derive(Debug, Clone, Default)]
pub struct NoRunner;

#[async_trait]
impl RunnerApi for NoRunner {
    async fn put_blob(&self, _: &Digest, _: Bytes) -> Result<(), BrokerError> {
        Err(BrokerError::Unreachable(
            "no runner is configured (AGENTRIX_RUNNER)".into(),
        ))
    }
    async fn post_job(&self, _: &SignedJobSpec) -> Result<EventStream, BrokerError> {
        Err(BrokerError::Unreachable(
            "no runner is configured (AGENTRIX_RUNNER)".into(),
        ))
    }
    async fn get_artifact(&self, _: &Digest) -> Result<Bytes, BrokerError> {
        Err(BrokerError::Unreachable(
            "no runner is configured (AGENTRIX_RUNNER)".into(),
        ))
    }
}

impl Broker {
    /// The node's broker: the runner at `runner` (e.g. `http://10.42.0.4:7443`) or
    /// [`NoRunner`] when unset; receipts through the data lane's store; blobs in the CAS;
    /// room images from `AGENTRIX_IMAGE_*` ([`Images::from_env`]).
    pub fn for_node(
        runner: Option<&str>,
        signer: Arc<Signer>,
        receipts: Arc<crate::db::receipts::ReceiptStore>,
        cas: &crate::db::cas::Cas,
        pool: sqlx::SqlitePool,
    ) -> anyhow::Result<Self> {
        let runner: Arc<dyn RunnerApi> = match runner.map(str::trim).filter(|r| !r.is_empty()) {
            Some(url) => Arc::new(HttpRunner::new(url)?),
            None => {
                tracing::warn!(
                    "AGENTRIX_RUNNER is unset: sealed-room jobs will fail as unreachable"
                );
                Arc::new(NoRunner)
            }
        };
        Ok(Self::new(
            runner,
            signer,
            receipts,
            Arc::new(db::DbBlobs::new(cas.clone(), pool)),
            Images::from_env()?,
        ))
    }
}

/// The content type to record for an artifact, from its first bytes and then its name.
pub fn content_type_for(name: &str, bytes: &[u8]) -> &'static str {
    if let Some((ct, _, _)) = image_info(bytes) {
        return ct;
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".txt") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

/// A PNG's or JPEG's content type and size in pixels, from its header. `None` for anything
/// else: the room cannot pass off another format as a frame.
pub fn image_info(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") && &bytes[12..16] == b"IHDR" {
        let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some(("image/png", w, h));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        // Walk the segments to the first start-of-frame marker.
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let h = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
                let w = u32::from(u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]));
                return Some(("image/jpeg", w, h));
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
            i += 2 + len;
        }
        return Some(("image/jpeg", 0, 0));
    }
    None
}

#[cfg(test)]
mod tests;
