//! Broker ↔ runner (design §4.2): signed job specs, the event stream and the result.

use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::digest::Digest;
use crate::error::{Error, Result};
use crate::ids::JobId;
use crate::receipt::ReceiptKind;
use crate::sign::{KeyId, Signature, Signer, verify};

/// What a sealed room does (design §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    /// Crawl and screenshot public pages, through the egress proxy.
    Fetch,
    /// Replay WordPress from a WXR export or database dump, with no network.
    Replay,
    /// Render candidate pages and take screenshots, with no network.
    Render,
    /// Run the static generator on the clean inventory, with no network.
    Build,
}

impl JobKind {
    /// The receipt kind for a finished job of this kind.
    pub fn receipt_kind(self) -> ReceiptKind {
        match self {
            Self::Fetch => ReceiptKind::JobFetch,
            Self::Replay => ReceiptKind::JobReplay,
            Self::Render => ReceiptKind::JobRender,
            Self::Build => ReceiptKind::JobBuild,
        }
    }

    /// The default caps for this kind (design §3.1). `out_mb` is a the first release default to tune.
    pub fn default_caps(self) -> Caps {
        let (pids, seconds) = match self {
            Self::Replay => (512, 15 * 60),
            Self::Fetch | Self::Render | Self::Build => (256, 10 * 60),
        };
        Caps {
            cpus: 2.0,
            memory_mb: 2048,
            pids,
            seconds,
            out_mb: 1024,
        }
    }
}

/// A container image pinned by digest: `name@sha256:<64 hex>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ImageRef(String);

impl ImageRef {
    /// Parses `name@sha256:<64 hex>`, refusing tags and unpinned names.
    pub fn parse(text: &str) -> Result<Self> {
        let bad = || Error::BadImage(text.chars().take(200).collect());
        let (name, digest) = text.rsplit_once('@').ok_or_else(bad)?;
        if name.is_empty() || name.chars().any(char::is_whitespace) {
            return Err(bad());
        }
        digest.parse::<Digest>().map_err(|_| bad())?;
        Ok(Self(text.to_owned()))
    }

    /// The image name, without the digest.
    pub fn name(&self) -> &str {
        self.0.rsplit_once('@').map_or("", |(name, _)| name)
    }

    /// The pinned digest.
    pub fn digest(&self) -> Digest {
        self.0
            .rsplit_once('@')
            .and_then(|(_, d)| d.parse().ok())
            .expect("checked when parsed")
    }

    /// The full reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ImageRef {
    type Error = Error;

    fn try_from(text: String) -> Result<Self> {
        Self::parse(&text)
    }
}

impl From<ImageRef> for String {
    fn from(image: ImageRef) -> Self {
        image.0
    }
}

/// Resource caps for one room. The runner kills a room that passes any of them.
///
/// `cpus` is an `f64` so that its RFC 8785 form is the same in every language.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Caps {
    /// CPU cores, for example `2.0`.
    pub cpus: f64,
    /// Memory in MiB; swap is set equal, so there is none.
    pub memory_mb: u32,
    /// Maximum processes and threads.
    pub pids: u32,
    /// Wall-clock limit in seconds.
    pub seconds: u32,
    /// Maximum bytes the room may write to its output, in MiB.
    pub out_mb: u32,
}

/// A job for one sealed room (design §4.2). Control signs it; the runner verifies it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSpec {
    /// The job.
    pub job_id: JobId,
    /// What the room does.
    pub kind: JobKind,
    /// The room image, pinned by digest.
    pub image: ImageRef,
    /// Input blobs, uploaded with `PUT /blobs/{digest}` and mounted read-only at `/in/<digest>`.
    pub inputs: Vec<Digest>,
    /// Hosts the room may reach through the egress proxy: `host`, `host:port` or `*.suffix`.
    /// Empty for rooms with no network.
    pub allowlist: Vec<String>,
    /// Resource caps.
    pub caps: Caps,
    /// When the runner kills the room, whatever it is doing.
    pub deadline: DateTime<Utc>,
    /// Non-secret job arguments, written to `/in/job.json`.
    #[serde(default)]
    pub args: serde_json::Value,
}

impl JobSpec {
    /// Whether the deadline has passed at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.deadline
    }
}

/// A job spec with control's signature over its canonical JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedJobSpec {
    /// The spec.
    pub spec: JobSpec,
    /// Ed25519 over the RFC 8785 bytes of `spec`.
    pub signature: Signature,
    /// The key that signed it.
    pub key_id: KeyId,
}

impl SignedJobSpec {
    /// Signs `spec` with control's key.
    pub fn sign(spec: JobSpec, signer: &Signer) -> Result<Self> {
        let signature = signer.sign(&spec)?;
        Ok(Self {
            spec,
            signature,
            key_id: signer.key_id().clone(),
        })
    }

    /// Checks the signature against control's public key, canonicalising the spec itself,
    /// and returns the spec only if it verifies.
    pub fn verify(&self, key: &VerifyingKey) -> Result<&JobSpec> {
        let offered = KeyId::of(key);
        if offered != self.key_id {
            return Err(Error::WrongKey {
                signed_by: self.key_id.clone(),
                offered,
            });
        }
        verify(key, &self.spec, &self.signature)?;
        Ok(&self.spec)
    }
}

/// Why the runner killed a room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillReason {
    /// The deadline passed.
    Deadline,
    /// The memory cap was reached.
    Memory,
    /// The process cap was reached.
    Pids,
    /// The room wrote more than `out_mb`.
    OutCap,
}

/// One line of the runner's NDJSON stream, tagged by `event`. The last line is
/// `{"event": "result", …}` with the [`JobResult`]'s members. These are the runner's own wire
/// shapes (agentrix-runner, S1-01).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JobEvent {
    /// The room started.
    Started {
        /// The job.
        job_id: JobId,
        /// The OCI runtime, for example `runsc`.
        runtime: String,
        /// When.
        at: DateTime<Utc>,
    },
    /// The egress proxy allowed a request. A `CONNECT` tunnel has no HTTP status to report.
    Request {
        /// HTTP method, or `CONNECT`.
        method: String,
        /// Host requested.
        host: String,
        /// Port requested.
        port: u16,
        /// Bytes sent.
        bytes_up: u64,
        /// Bytes received.
        bytes_down: u64,
    },
    /// The egress proxy refused a request.
    Refused(Refusal),
    /// A screenshot frame was stored.
    Frame {
        /// The frame's digest.
        digest: Digest,
    },
    /// A room status line, at most [`NOTE_MAX`] characters.
    Note {
        /// The line.
        text: String,
    },
    /// The runner killed the room.
    Killed {
        /// Why.
        reason: KillReason,
    },
    /// The room was destroyed.
    Destroyed {
        /// Whether the runner confirmed the container and its scratch are gone.
        confirmed: bool,
    },
    /// The room produced an artifact.
    Artifact(ArtifactRef),
    /// A runner failure outside the room.
    Error {
        /// What failed, in plain words.
        message: String,
    },
    /// The last line: what the room produced and whether it is gone.
    Result(Box<JobResult>),
}

/// The longest a `note` event's text may be, in characters.
pub const NOTE_MAX: usize = 200;

impl JobEvent {
    /// Parses one NDJSON line.
    pub fn parse(line: &str) -> Result<Self> {
        Ok(serde_json::from_str(line)?)
    }
}

/// One refused request, as summarised on the result and the receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// Host requested.
    pub host: String,
    /// Port requested.
    pub port: u16,
    /// The proxy's reason.
    pub reason: String,
}

/// Peak resource use in a room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Peaks {
    /// Peak CPU, as a percentage of one core.
    pub cpu_pct: f64,
    /// Peak memory in MiB.
    pub memory_mb: f64,
    /// Peak process count.
    pub pids: u64,
}

/// One artifact a room produced, fetched by control with `GET /artifacts/{digest}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Its digest.
    pub digest: Digest,
    /// Its name inside the room's output.
    pub name: String,
    /// Its size.
    pub bytes: u64,
}

/// What the room produced and whether it is gone: the stream's `result` line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobResult {
    /// The job.
    pub job_id: JobId,
    /// The OCI runtime the room ran under, for example `runsc`.
    pub runtime: String,
    /// The room's exit code, if it exited rather than being killed.
    pub exit_code: Option<i64>,
    /// Why the runner killed the room, if it did.
    pub killed: Option<KillReason>,
    /// The artifacts.
    pub artifacts: Vec<ArtifactRef>,
    /// Requests the egress proxy allowed.
    pub requests: u64,
    /// Every refused request.
    pub refusals: Vec<Refusal>,
    /// Peak resource use.
    pub peaks: Peaks,
    /// Whether the room was destroyed and the runner confirmed it.
    pub destroyed: bool,
    /// When the room started.
    pub started_at: DateTime<Utc>,
    /// When the job ended.
    pub ended_at: DateTime<Utc>,
}
