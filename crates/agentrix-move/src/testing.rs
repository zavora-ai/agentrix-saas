//! In-memory ports for tests and offline development (D10a): no sealed rooms, no model calls.
//! Nothing here is used by a deployment.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use agentrix_types::{
    ArtifactRef, Digest, JobId, JobKind, JobResult, KillReason, MoveId, Peaks, Receipt,
    ReceiptBody, Risk, Signer, SiteId, UiEvent,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::clean::agent_instructions;
use crate::ports::{
    Blobs, BuildReply, BuildRequest, CallContext, Events, Gateway, Receipts, RoomJob, RoomRun,
    Rooms, Screen, VisionRequest, VisionVerdict,
};
use crate::rebuild::plain_html;

/// A CAS in memory, with the per-site index.
#[derive(Debug, Default)]
pub struct MemoryBlobs {
    blobs: Mutex<HashMap<Digest, Vec<u8>>>,
    index: Mutex<BTreeSet<(String, Digest)>>,
}

#[async_trait]
impl Blobs for MemoryBlobs {
    async fn put(
        &self,
        site: &SiteId,
        _kind: &str,
        _ct: &str,
        bytes: &[u8],
    ) -> anyhow::Result<Digest> {
        let digest = Digest::of(bytes);
        self.blobs.lock().unwrap().insert(digest, bytes.to_vec());
        self.index
            .lock()
            .unwrap()
            .insert((site.to_string(), digest));
        Ok(digest)
    }

    async fn get(&self, site: &SiteId, digest: &Digest) -> anyhow::Result<Option<Vec<u8>>> {
        if !self
            .index
            .lock()
            .unwrap()
            .contains(&(site.to_string(), *digest))
        {
            return Ok(None);
        }
        Ok(self.blobs.lock().unwrap().get(digest).cloned())
    }
}

impl MemoryBlobs {
    /// Stores bytes for a site without going through the trait (test setup).
    pub fn insert(&self, site: &SiteId, bytes: &[u8]) -> Digest {
        let digest = Digest::of(bytes);
        self.blobs.lock().unwrap().insert(digest, bytes.to_vec());
        self.index
            .lock()
            .unwrap()
            .insert((site.to_string(), digest));
        digest
    }
}

/// Signs receipts with a fresh key and keeps them.
#[derive(Debug)]
pub struct MemoryReceipts {
    signer: Signer,
    /// Every receipt issued, oldest first.
    pub issued: Mutex<Vec<Receipt>>,
}

impl Default for MemoryReceipts {
    fn default() -> Self {
        Self {
            signer: Signer::generate(),
            issued: Mutex::new(Vec::new()),
        }
    }
}

impl MemoryReceipts {
    /// The receipts of one kind.
    pub fn of_kind(&self, kind: agentrix_types::ReceiptKind) -> Vec<Receipt> {
        self.issued
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.body.kind == kind)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl Receipts for MemoryReceipts {
    async fn issue(&self, body: ReceiptBody) -> anyhow::Result<Digest> {
        let receipt = Receipt::sign(body, &self.signer)?;
        let digest = receipt.digest()?;
        self.issued.lock().unwrap().push(receipt);
        Ok(digest)
    }
}

/// Keeps every UI event.
#[derive(Debug, Default)]
pub struct MemoryEvents {
    /// Every event, in order.
    pub events: Mutex<Vec<UiEvent>>,
}

impl MemoryEvents {
    /// The events' kebab-case names, in order.
    pub fn names(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.name().to_owned())
            .collect()
    }
}

#[async_trait]
impl Events for MemoryEvents {
    async fn emit(&self, _site: &SiteId, _move_id: &MoveId, event: UiEvent) -> anyhow::Result<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// A 1×1 PNG.
pub const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f,
    0x00, 0x05, 0xfe, 0x02, 0xfe, 0xa7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Rooms that answer at once: `render` writes a screenshot per page; `build` runs the legacy
/// theme hook as attack 9 does (killed by the pids cap when `kill_legacy` is set) and the
/// clean retry exits 0. `replay` is unsupported unless listed.
pub struct FakeRooms {
    /// Kinds this deployment runs.
    pub kinds: HashSet<JobKind>,
    /// The legacy theme build fork-bombs.
    pub kill_legacy: bool,
    /// Where results go.
    pub blobs: std::sync::Arc<dyn Blobs>,
    /// Every job run, in order.
    pub runs: Mutex<Vec<(RoomJob, Option<KillReason>)>>,
    /// Pages a `replay` room records as room-replay does: (path, browser HTML, crawler HTML).
    pub replay_pages: Vec<(String, String, String)>,
    /// The render room fails (a sandbox fault).
    pub fail_render: bool,
    /// The most inputs a room takes, as the runner enforces.
    pub max_inputs: usize,
}

impl FakeRooms {
    /// Render and build rooms, with attack 9 armed.
    pub fn new(blobs: std::sync::Arc<dyn Blobs>) -> Self {
        Self {
            kinds: [JobKind::Render, JobKind::Build].into_iter().collect(),
            kill_legacy: true,
            blobs,
            runs: Mutex::new(Vec::new()),
            replay_pages: Vec::new(),
            fail_render: false,
            max_inputs: crate::pipeline::MAX_ROOM_INPUTS,
        }
    }
}

#[async_trait]
impl Rooms for FakeRooms {
    fn supports(&self, kind: JobKind) -> bool {
        self.kinds.contains(&kind)
    }

    async fn run(&self, site: &SiteId, _move_id: &MoveId, job: RoomJob) -> anyhow::Result<RoomRun> {
        if job.kind == JobKind::Render && self.fail_render {
            anyhow::bail!("Our sealed rooms aren't reachable right now.");
        }
        // The runner refuses a spec with more inputs than it allows (agentrix-runner MAX_INPUTS).
        if job.inputs.len() > self.max_inputs {
            anyhow::bail!("The sandbox refused to start the room.");
        }
        let now = Utc::now();
        let mut artifacts = Vec::new();
        let mut killed = None;
        match job.kind {
            JobKind::Render => {
                for page in job
                    .args
                    .get("pages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let name = page
                        .get("shot")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let digest = self.blobs.put(site, "frame", "image/png", TINY_PNG).await?;
                    artifacts.push(ArtifactRef {
                        digest,
                        name,
                        bytes: TINY_PNG.len() as u64,
                    });
                }
            }
            JobKind::Replay => {
                let mut recording = Vec::new();
                for (n, (path, browser, crawler)) in self.replay_pages.iter().enumerate() {
                    let (b, c) = (
                        format!("site/browser/{n:04}.html"),
                        format!("site/crawler/{n:04}.html"),
                    );
                    for (name, html) in [(&b, browser), (&c, crawler)] {
                        let digest = self
                            .blobs
                            .put(site, "replay", "text/html", html.as_bytes())
                            .await?;
                        artifacts.push(ArtifactRef {
                            digest,
                            name: name.clone(),
                            bytes: html.len() as u64,
                        });
                    }
                    recording.push(serde_json::json!({
                        "path": path, "public_url": format!("https://kilele-bakery.example{path}"),
                        "browser": {"file": b, "status": 200}, "crawler": {"file": c, "status": 200},
                        "differs": browser != crawler, "forms": 0,
                    }));
                }
                for (name, value) in [
                    ("recording.json", Value::Array(recording)),
                    ("inventory.json", serde_json::json!({"counts": {}})),
                ] {
                    let bytes = serde_json::to_vec(&value)?;
                    let digest = self
                        .blobs
                        .put(site, "replay", "application/json", &bytes)
                        .await?;
                    artifacts.push(ArtifactRef {
                        digest,
                        name: name.into(),
                        bytes: bytes.len() as u64,
                    });
                }
            }
            JobKind::Build
                if job.args.get("step").and_then(Value::as_str) == Some("legacy-theme")
                    && self.kill_legacy =>
            {
                killed = Some(KillReason::Pids);
            }
            _ => {}
        }
        self.runs.lock().unwrap().push((job.clone(), killed));
        let result = JobResult {
            job_id: JobId::generate(),
            runtime: "fake".into(),
            exit_code: if killed.is_some() { None } else { Some(0) },
            killed,
            artifacts: artifacts.clone(),
            requests: 0,
            refusals: vec![],
            peaks: Peaks::default(),
            destroyed: true,
            started_at: now,
            ended_at: now,
        };
        let receipt = Digest::of(serde_json::to_string(&result)?.as_bytes());
        Ok(RoomRun {
            result,
            artifacts: artifacts
                .into_iter()
                .map(|a| crate::ports::RoomArtifact {
                    name: a.name,
                    digest: a.digest,
                })
                .collect(),
            receipt,
        })
    }
}

/// A gateway that never calls a model: `build` lays the clean Markdown out as HTML (failing
/// the first try for paths in `fail_first`, to exercise R6.3); `screen` flags instructions to
/// agents; `vision-parity` answers a fixed score.
#[derive(Debug, Default)]
pub struct FakeGateway {
    /// Paths whose first build try returns a script, which the check refuses.
    pub fail_first: BTreeSet<String>,
    /// Paths whose every build try drops half the text, so parity flags them.
    pub always_short: BTreeSet<String>,
    /// The vision score.
    pub vision: Option<f64>,
    /// Make every build call fail (a broken gateway).
    pub broken: bool,
    /// Make the safety screen fail (the gateway is down during Clean).
    pub broken_screen: bool,
    /// Build calls.
    pub builds: AtomicU32,
    /// Screen calls.
    pub screens: AtomicU32,
    /// Feedback received on retries, by path.
    pub feedback: Mutex<BTreeMap<String, Vec<String>>>,
}

#[async_trait]
impl Gateway for FakeGateway {
    async fn build_page(
        &self,
        ctx: &CallContext,
        request: &BuildRequest,
    ) -> anyhow::Result<BuildReply> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        if self.broken {
            anyhow::bail!("the Model Gateway is not answering");
        }
        if let Some(f) = &request.feedback {
            self.feedback
                .lock()
                .unwrap()
                .entry(request.path.clone())
                .or_default()
                .push(f.clone());
        }
        let title = request.title.replace('&', "&amp;").replace('<', "&lt;");
        if ctx.attempt == 1 && self.fail_first.contains(&request.path) {
            return Ok(BuildReply {
                body_html: format!("<h1>{title}</h1><script>steal()</script>"),
                model: Some("fake".into()),
            });
        }
        let markdown = if self.always_short.contains(&request.path) {
            let words: Vec<&str> = request.markdown.split_whitespace().collect();
            words[..words.len() / 3].join(" ")
        } else {
            request.markdown.clone()
        };
        Ok(BuildReply {
            body_html: format!("<h1>{title}</h1>\n{}", plain_html(&markdown)),
            model: Some("fake".into()),
        })
    }

    async fn screen(&self, _ctx: &CallContext, text: &str) -> anyhow::Result<Screen> {
        self.screens.fetch_add(1, Ordering::SeqCst);
        if self.broken_screen {
            anyhow::bail!("The safety screen is not answering; the Move will try again.");
        }
        let flagged = !agent_instructions(text).is_empty() || text.contains("UNSAFE");
        Ok(Screen {
            safe: !flagged,
            reason: if flagged {
                "User Safety: unsafe".into()
            } else {
                "User Safety: safe".into()
            },
        })
    }

    async fn vision_parity(
        &self,
        _ctx: &CallContext,
        _request: &VisionRequest,
    ) -> anyhow::Result<VisionVerdict> {
        Ok(VisionVerdict {
            score: self.vision.unwrap_or(0.97),
            verdict: "Same content, new layout".into(),
        })
    }

    async fn decide(&self, _ctx: &CallContext, _state: &str) -> anyhow::Result<Option<Risk>> {
        Ok(Some(Risk {
            model: "fake".into(),
            score: 0.12,
            confidence: Some(0.9),
            summary: "Low risk: reversible for 30 days".into(),
        }))
    }
}
