//! A fake runner that speaks the §4.2 protocol in-process, for tests until the sandbox VM is
//! up. It checks what the real runner checks (control's signature, the job id never reused,
//! inputs uploaded first) and runs a scripted "room" instead of a container.
//!
//! [`fetch_room`] scripts a `room-fetch` room over a map of URLs, following the room-fetch
//! contract (`/in/job.json` → `/out/fetch.json` and `/out/bodies/<i>`), with the egress
//! allowlist enforced as the proxy would.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use agentrix_types::{
    ArtifactRef, Digest, JobEvent, JobResult, JobSpec, Peaks, Refusal, SignedJobSpec, VerifyingKey,
};
use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt as _;

use super::fetch::{FetchArgs, FetchManifest, FetchedUrl};
use super::{BrokerError, EventStream, RunnerApi};

/// What a scripted room did.
#[derive(Debug, Clone, Default)]
pub struct RoomRun {
    /// Events while it ran: `request`, `refused`, `note`, … (frames go in `frames`).
    pub events: Vec<JobEvent>,
    /// Frames, in order, each emitted after the event at the same position in `events`
    /// (`(after_event_index, bytes)`).
    pub frames: Vec<(usize, Vec<u8>)>,
    /// Files left in `/out`, by path.
    pub out: BTreeMap<String, Vec<u8>>,
    /// Exit code.
    pub exit_code: i64,
}

/// A scripted room: given the verified spec and its inputs, what it did.
pub type Room = Arc<dyn Fn(&JobSpec, &HashMap<Digest, Bytes>) -> RoomRun + Send + Sync>;

/// The fake runner.
#[derive(Clone)]
pub struct FakeRunner {
    key: VerifyingKey,
    room: Room,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    blobs: HashMap<Digest, Bytes>,
    seen: HashSet<String>,
    jobs: Vec<JobSpec>,
    down: bool,
}

impl std::fmt::Debug for FakeRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeRunner").finish_non_exhaustive()
    }
}

impl FakeRunner {
    /// A runner that trusts `key` and runs `room` for every job.
    pub fn new(key: VerifyingKey, room: Room) -> Self {
        Self {
            key,
            room,
            state: Arc::default(),
        }
    }

    /// Every spec admitted so far.
    pub fn jobs(&self) -> Vec<JobSpec> {
        self.lock().jobs.clone()
    }

    /// Makes the runner unreachable (or reachable again).
    pub fn set_down(&self, down: bool) {
        self.lock().down = down;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn check_up(&self) -> Result<(), BrokerError> {
        if self.lock().down {
            Err(BrokerError::Unreachable(
                "connection refused (fake runner is down)".into(),
            ))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl RunnerApi for FakeRunner {
    async fn put_blob(&self, digest: &Digest, bytes: Bytes) -> Result<(), BrokerError> {
        self.check_up()?;
        if &Digest::of(&bytes) != digest {
            return Err(BrokerError::Refused {
                status: 400,
                message: "blob does not match its digest".into(),
            });
        }
        self.lock().blobs.insert(*digest, bytes);
        Ok(())
    }

    async fn post_job(&self, signed: &SignedJobSpec) -> Result<EventStream, BrokerError> {
        self.check_up()?;
        // The real runner reads JSON off the wire; so does the fake.
        let wire = serde_json::to_vec(signed).map_err(|e| BrokerError::Protocol(e.to_string()))?;
        let signed: SignedJobSpec =
            serde_json::from_slice(&wire).map_err(|e| BrokerError::Refused {
                status: 422,
                message: e.to_string(),
            })?;
        let spec = signed
            .verify(&self.key)
            .map_err(|e| BrokerError::Refused {
                status: 403,
                message: e.to_string(),
            })?
            .clone();
        let inputs = {
            let mut st = self.lock();
            if !st.seen.insert(spec.job_id.as_str().to_owned()) {
                return Err(BrokerError::Refused {
                    status: 409,
                    message: format!("job {} was already run", spec.job_id),
                });
            }
            let mut inputs = HashMap::new();
            for d in &spec.inputs {
                match st.blobs.get(d) {
                    Some(b) => {
                        inputs.insert(*d, b.clone());
                    }
                    None => {
                        return Err(BrokerError::Refused {
                            status: 409,
                            message: format!("inputs not uploaded: {d}"),
                        });
                    }
                }
            }
            st.jobs.push(spec.clone());
            inputs
        };

        let started_at = Utc::now();
        let run = (self.room)(&spec, &inputs);
        let mut events = vec![JobEvent::Started {
            job_id: spec.job_id.clone(),
            runtime: "fake".into(),
            at: started_at,
        }];
        let mut requests = 0;
        let mut refusals = Vec::new();
        let mut frames = run.frames.iter().peekable();
        for (i, ev) in run.events.iter().enumerate() {
            match ev {
                JobEvent::Request { .. } => requests += 1,
                JobEvent::Refused(r) => refusals.push(r.clone()),
                _ => {}
            }
            events.push(ev.clone());
            while let Some((_, bytes)) = frames.next_if(|(at, _)| *at == i) {
                let digest = Digest::of(bytes);
                self.lock().blobs.insert(digest, Bytes::from(bytes.clone()));
                events.push(JobEvent::Frame { digest });
            }
        }
        events.push(JobEvent::Destroyed { confirmed: true });
        let mut artifacts = Vec::new();
        for (name, bytes) in &run.out {
            let digest = Digest::of(bytes);
            self.lock().blobs.insert(digest, Bytes::from(bytes.clone()));
            let a = ArtifactRef {
                digest,
                name: name.clone(),
                bytes: bytes.len() as u64,
            };
            events.push(JobEvent::Artifact(a.clone()));
            artifacts.push(a);
        }
        events.push(JobEvent::Result(Box::new(JobResult {
            job_id: spec.job_id.clone(),
            runtime: "fake".into(),
            exit_code: Some(run.exit_code),
            killed: None,
            artifacts,
            requests,
            refusals,
            peaks: Peaks {
                cpu_pct: 12.0,
                memory_mb: 180.0,
                pids: 9,
            },
            destroyed: true,
            started_at,
            ended_at: Utc::now(),
        })));
        // Through NDJSON, as the real stream is.
        let mut text = Vec::new();
        for ev in &events {
            text.extend(serde_json::to_vec(ev).map_err(|e| BrokerError::Protocol(e.to_string()))?);
            text.push(b'\n');
        }
        let chunks = futures::stream::iter(
            text.chunks(97)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect::<Vec<_>>(),
        )
        .boxed();
        Ok(super::http::ndjson(chunks))
    }

    async fn get_artifact(&self, digest: &Digest) -> Result<Bytes, BrokerError> {
        self.check_up()?;
        self.lock()
            .blobs
            .get(digest)
            .cloned()
            .ok_or(BrokerError::Refused {
                status: 404,
                message: "no such artifact".into(),
            })
    }
}

/// One URL the fake web serves.
#[derive(Debug, Clone)]
pub struct FakePage {
    /// HTTP status.
    pub status: u16,
    /// `Content-Type`.
    pub content_type: String,
    /// Body.
    pub body: Vec<u8>,
    /// Milliseconds it "took".
    pub ms: u64,
    /// A redirect target (`Location`), followed by the room.
    pub location: Option<String>,
    /// The DOM after scripts ran, written to `rendered/<i>.html` when screenshots are on.
    pub rendered: Option<String>,
}

impl FakePage {
    /// A 200 HTML page.
    pub fn html(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
            ms: 120,
            location: None,
            rendered: None,
        }
    }

    /// A 200 plain-text file.
    pub fn text(body: &str) -> Self {
        Self {
            content_type: "text/plain; charset=utf-8".into(),
            ..Self::html(body)
        }
    }

    /// A response with this status and an HTML body.
    pub fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::html("<html><body>error</body></html>")
        }
    }

    /// A redirect to `to`.
    pub fn redirect(to: &str) -> Self {
        Self {
            location: Some(to.to_owned()),
            ..Self::status(301)
        }
    }

    /// Whose DOM after scripts ran is `html`.
    pub fn rendering(mut self, html: &str) -> Self {
        self.rendered = Some(html.to_owned());
        self
    }

    /// Takes this long.
    pub fn taking(mut self, ms: u64) -> Self {
        self.ms = ms;
        self
    }
}

/// A PNG header (1280×800) standing in for a sealed screenshot; enough for `image_info`.
pub const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x03, 0x20, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

fn host_allowed(host: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|a| {
        let a = a.split(':').next().unwrap_or(a);
        match a.strip_prefix("*.") {
            Some(suffix) => host.ends_with(&format!(".{suffix}")),
            None => a.eq_ignore_ascii_case(host),
        }
    })
}

/// A room-fetch room over `web` (URL → page), honouring the spec's allowlist as the egress
/// proxy does. A URL `web` lacks is a 404. The hosts `unresolvable.example` (DNS) and
/// `down.example` (connect timeout) fail as the real proxy reports them: a refusal with the
/// network reason.
pub fn fetch_room(web: HashMap<String, FakePage>) -> Room {
    Arc::new(move |spec: &JobSpec, _inputs: &HashMap<Digest, Bytes>| {
        let args: FetchArgs = match serde_json::from_value(spec.args.clone()) {
            Ok(a) => a,
            Err(_) => {
                return RoomRun {
                    exit_code: 2,
                    ..RoomRun::default()
                };
            }
        };
        let mut run = RoomRun::default();
        let mut results = Vec::new();
        for (i, url) in args.urls.iter().enumerate() {
            let mut current = url.clone();
            let mut outcome: Result<(FakePage, String), String> = Err("no response".into());
            for _hop in 0..=args.max_redirects {
                let host = reqwest::Url::parse(&current)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_default();
                if !host_allowed(&host, &spec.allowlist) {
                    run.events.push(JobEvent::Refused(Refusal {
                        host: host.clone(),
                        port: 443,
                        reason: "not on the allowlist".into(),
                    }));
                    outcome = Err(format!("refused by the egress proxy ({host})"));
                    break;
                }
                // The real proxy refuses an allowed host it cannot reach, with the reason.
                let unreachable = match host.as_str() {
                    "unresolvable.example" => Some(format!(
                        "DNS for {host} failed: failed to lookup address information"
                    )),
                    "down.example" => Some("connect to 203.0.113.9:443 timed out".to_owned()),
                    _ => None,
                };
                if let Some(reason) = unreachable {
                    run.events.push(JobEvent::Refused(Refusal {
                        host: host.clone(),
                        port: 443,
                        reason,
                    }));
                    outcome = Err(format!("refused by the egress proxy ({host})"));
                    break;
                }
                let page = web
                    .get(&current)
                    .cloned()
                    .unwrap_or_else(|| FakePage::status(404));
                run.events.push(JobEvent::Request {
                    method: "CONNECT".into(),
                    host: host.clone(),
                    port: 443,
                    bytes_up: 300,
                    bytes_down: page.body.len() as u64 + 400,
                });
                match &page.location {
                    Some(to) if (300..400).contains(&page.status) => {
                        current = reqwest::Url::parse(&current)
                            .and_then(|base| base.join(to))
                            .map(|u| u.to_string())
                            .unwrap_or_else(|_| to.clone());
                        outcome = Ok((page, current.clone()));
                    }
                    _ => {
                        outcome = Ok((page, current.clone()));
                        break;
                    }
                }
            }
            let entry = match outcome {
                Ok((page, final_url)) => {
                    let keep = page.body.len().min(args.max_bytes as usize);
                    let name = format!("bodies/{i}");
                    run.out.insert(name.clone(), page.body[..keep].to_vec());
                    run.events.push(JobEvent::Note {
                        text: format!("fetched {i} {}", page.status),
                    });
                    let mut rendered = None;
                    if args.screenshots && page.content_type.starts_with("text/html") {
                        run.frames.push((run.events.len() - 1, TINY_PNG.to_vec()));
                        if let Some(dom) = &page.rendered {
                            let name = format!("rendered/{i}.html");
                            run.out.insert(name.clone(), dom.as_bytes().to_vec());
                            rendered = Some(name);
                        }
                    }
                    FetchedUrl {
                        i,
                        url: url.clone(),
                        final_url: Some(final_url),
                        status: Some(page.status),
                        content_type: Some(page.content_type.clone()),
                        bytes: keep as u64,
                        truncated: keep < page.body.len(),
                        ms: Some(page.ms),
                        body: Some(name),
                        error: None,
                        rendered,
                    }
                }
                Err(error) => {
                    run.events.push(JobEvent::Note {
                        text: format!("fetched {i} error"),
                    });
                    FetchedUrl {
                        i,
                        url: url.clone(),
                        final_url: None,
                        status: None,
                        content_type: None,
                        bytes: 0,
                        truncated: false,
                        ms: None,
                        body: None,
                        error: Some(error),
                        rendered: None,
                    }
                }
            };
            results.push(entry);
        }
        run.out.insert(
            "fetch.json".into(),
            serde_json::to_vec(&FetchManifest { results }).unwrap_or_default(),
        );
        run
    })
}
