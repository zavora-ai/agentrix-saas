//! `fetch` jobs under the room-fetch contract, and the claim probes built on them (R1.2, R3.1).
//!
//! The contract (agreed with the framework lane, notes `assess-room-fetch-contract.md`):
//! `/in/job.json` is a [`FetchArgs`]; the room fetches `urls` in order through the egress proxy
//! with `user_agent`, `delay_ms` apart, and leaves `/out/fetch.json` ([`FetchManifest`]) and
//! each body at `/out/bodies/<i>`. After each URL it prints the note `fetched <i> <status>` (or
//! `fetched <i> error`), then optionally a frame. Crawl policy (robots.txt, the page budget,
//! which links to follow) stays on control.

use agentrix_types::{Digest, JobKind, SiteId};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::{Broker, BrokerError, BrokerEvent, JobOutcome, JobRequest, Parent};

/// The crawler's default identity, sent as `User-Agent`.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; AgentrixBot/1.0; +https://agentrix.website/bot)";

/// The token robots.txt rules name the crawler by.
pub const ROBOTS_TOKEN: &str = "AgentrixBot";

/// DNS over HTTPS, reached from the sandbox for TXT lookups (R3.1).
pub const DOH_HOST: &str = "dns.google";

/// `/in/job.json` for a fetch room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchArgs {
    /// Always `fetch`.
    pub task: String,
    /// URLs to fetch, in order.
    pub urls: Vec<String>,
    /// The `User-Agent` on every request.
    pub user_agent: String,
    /// Pause between requests, in milliseconds (the rate limit).
    pub delay_ms: u64,
    /// Per-request timeout, in milliseconds.
    pub timeout_ms: u64,
    /// Most bytes kept of each body.
    pub max_bytes: u64,
    /// Most redirects followed per URL.
    pub max_redirects: u32,
    /// Send a sealed screenshot per HTML page, when the room has a browser.
    pub screenshots: bool,
}

impl FetchArgs {
    /// Arguments for `urls` with the crawler's defaults: 500 ms apart, 15 s timeout, 2 MiB
    /// bodies, 5 redirects, screenshots on.
    pub fn new(urls: Vec<String>, user_agent: impl Into<String>) -> Self {
        Self {
            task: "fetch".into(),
            urls,
            user_agent: user_agent.into(),
            delay_ms: 500,
            timeout_ms: 15_000,
            max_bytes: 2 * 1024 * 1024,
            max_redirects: 5,
            screenshots: true,
        }
    }
}

/// `/out/fetch.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchManifest {
    /// One per input URL, in order.
    pub results: Vec<FetchedUrl>,
}

/// What the room saw for one URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedUrl {
    /// Its index in `urls`.
    pub i: usize,
    /// The URL asked for.
    pub url: String,
    /// Where redirects ended.
    #[serde(default)]
    pub final_url: Option<String>,
    /// HTTP status; `None` when no response came.
    #[serde(default)]
    pub status: Option<u16>,
    /// `Content-Type`.
    #[serde(default)]
    pub content_type: Option<String>,
    /// Body bytes kept.
    #[serde(default)]
    pub bytes: u64,
    /// Whether the body was cut at `max_bytes`.
    #[serde(default)]
    pub truncated: bool,
    /// Time to the last byte.
    #[serde(default)]
    pub ms: Option<u64>,
    /// The body's path under `/out`.
    #[serde(default)]
    pub body: Option<String>,
    /// Why there is no response, in plain words.
    #[serde(default)]
    pub error: Option<String>,
    /// The DOM after scripts ran, under `/out` (rooms with a browser; conventionally
    /// `rendered/<i>.html`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered: Option<String>,
}

/// One fetched URL with its body, checked and stored in control's CAS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    /// What the room reported.
    pub seen: FetchedUrl,
    /// The body, if any.
    pub body: Option<Bytes>,
    /// The body's digest (evidence for findings).
    pub digest: Option<Digest>,
    /// The DOM after scripts ran, when the room had a browser: injected spam often exists
    /// only there.
    pub rendered: Option<Bytes>,
}

impl FetchedPage {
    /// The body as text (lossy UTF-8).
    pub fn text(&self) -> String {
        self.body
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    }

    /// Whether the response is HTML.
    pub fn is_html(&self) -> bool {
        self.seen
            .content_type
            .as_deref()
            .is_some_and(|ct| ct.to_ascii_lowercase().contains("html"))
    }
}

/// What a fetch job produced.
#[derive(Debug, Clone)]
pub struct FetchOutcome {
    /// The job, its receipt and raw artifacts.
    pub job: JobOutcome,
    /// One per URL, in order; empty if the room left no manifest.
    pub pages: Vec<FetchedPage>,
    /// Why the pages are missing or incomplete, if they are.
    pub problem: Option<String>,
}

/// A fetch to run.
#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// The site.
    pub site: SiteId,
    /// Hosts the room may reach.
    pub allowlist: Vec<String>,
    /// The room's arguments.
    pub args: FetchArgs,
    /// What the job belongs to.
    pub parent: Parent,
    /// The receipt's title.
    pub title: String,
}

/// A parsed `fetched <i> <status|error>` note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchNote {
    /// The URL's index.
    pub i: usize,
    /// Its status, or `None` for an error.
    pub status: Option<u16>,
}

/// Parses a room's progress note.
pub fn parse_note(text: &str) -> Option<FetchNote> {
    let mut parts = text.split_whitespace();
    if parts.next()? != "fetched" {
        return None;
    }
    let i = parts.next()?.parse().ok()?;
    let status = match parts.next()? {
        "error" => None,
        s => Some(s.parse().ok()?),
    };
    Some(FetchNote { i, status })
}

impl Broker {
    /// Runs one fetch job and reads its manifest and bodies.
    pub async fn fetch(
        &self,
        req: FetchRequest,
        on_event: &mut (dyn FnMut(BrokerEvent) + Send),
    ) -> Result<FetchOutcome, BrokerError> {
        let urls = req.args.urls.clone();
        let mut job = JobRequest::new(
            req.site,
            JobKind::Fetch,
            serde_json::to_value(&req.args).map_err(|e| BrokerError::Control(e.to_string()))?,
            req.title,
        );
        job.allowlist = req.allowlist;
        job.parent = req.parent;
        job.artifact_kind = "page";
        let outcome = self.run(job, on_event).await?;

        let manifest = outcome
            .artifact("fetch.json")
            .map(|a| serde_json::from_slice::<FetchManifest>(&a.bytes));
        let (pages, problem) = match manifest {
            None => (
                Vec::new(),
                Some(room_problem(&outcome, "the room left no fetch.json")),
            ),
            Some(Err(e)) => (
                Vec::new(),
                Some(format!("the room's fetch.json does not parse: {e}")),
            ),
            Some(Ok(m)) => {
                let mut pages = Vec::with_capacity(urls.len());
                let mut problem = None;
                for (i, url) in urls.iter().enumerate() {
                    // One result per URL control asked for, by index; the URL is control's own
                    // (a room's HTTP library may re-spell it).
                    let seen = m.results.iter().find(|r| r.i == i).cloned();
                    let Some(mut seen) = seen else {
                        problem.get_or_insert_with(|| format!("the room did not report {url}"));
                        continue;
                    };
                    seen.url = url.clone();
                    let mut body = seen
                        .body
                        .as_deref()
                        .and_then(|name| outcome.artifact(name))
                        .map(|a| (a.bytes.clone(), a.digest));
                    // The egress proxy's own refusal is not the site's answer.
                    if let Some(refusal) = proxy_refusal(seen.status, body.as_ref().map(|(b, _)| b))
                    {
                        seen.status = None;
                        seen.error = Some(refusal);
                        seen.content_type = None;
                        body = None;
                    }
                    let rendered = body.as_ref().and_then(|_| {
                        let name = seen
                            .rendered
                            .clone()
                            .unwrap_or_else(|| format!("rendered/{i}.html"));
                        outcome.artifact(&name).map(|a| a.bytes.clone())
                    });
                    pages.push(FetchedPage {
                        seen,
                        digest: body.as_ref().map(|(_, d)| *d),
                        body: body.map(|(b, _)| b),
                        rendered,
                    });
                }
                (pages, problem)
            }
        };
        Ok(FetchOutcome {
            job: outcome,
            pages,
            problem,
        })
    }

    /// The TXT strings at `name`, looked up from the sandbox by DNS over HTTPS (R3.1). Each
    /// record's strings are joined. No such name, or no TXT records, is `Ok(vec![])`; `Err` is
    /// plain words saying why the lookup itself could not run.
    pub async fn probe_txt(&self, site: &SiteId, name: &str) -> Result<Vec<String>, String> {
        let name = name.trim().trim_end_matches('.');
        let valid = !name.is_empty()
            && name.len() <= 253
            && name.split('.').all(|l| {
                !l.is_empty()
                    && l.len() <= 63
                    && l.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            });
        if !valid {
            return Err(format!("{name:?} is not a DNS name"));
        }
        let url = format!("https://{DOH_HOST}/resolve?name={name}&type=TXT");
        let mut args = FetchArgs::new(vec![url], DEFAULT_USER_AGENT);
        args.screenshots = false;
        args.max_bytes = 64 * 1024;
        args.max_redirects = 0;
        args.delay_ms = 0;
        let out = self
            .fetch(
                FetchRequest {
                    site: site.clone(),
                    allowlist: vec![DOH_HOST.into()],
                    args,
                    parent: Parent::Claim(name.to_owned()),
                    title: format!("Looked up the TXT records of {name} from the sandbox"),
                },
                &mut |_| {},
            )
            .await
            .map_err(|e| e.plain())?;
        let page = out.pages.first().ok_or_else(|| {
            out.problem
                .clone()
                .unwrap_or_else(|| "the lookup returned nothing".into())
        })?;
        if let Some(error) = &page.seen.error {
            return Err(format!("the sandbox could not reach {DOH_HOST}: {error}"));
        }
        if page.seen.status != Some(200) {
            return Err(format!(
                "{DOH_HOST} answered HTTP {}",
                page.seen.status.unwrap_or_default()
            ));
        }
        parse_doh_txt(&page.text())
    }

    /// The status and first 4 KiB of `url`, fetched from the sandbox with no redirects (R3.1).
    /// A non-200 answer is `Ok`, so the owner can be told what was seen; `Err` is plain words
    /// saying why the fetch could not run.
    pub async fn probe_file(
        &self,
        site: &SiteId,
        url: &reqwest::Url,
    ) -> Result<(u16, String), String> {
        if !matches!(url.scheme(), "https" | "http") {
            return Err("the proof file must be on http or https".into());
        }
        let host = url
            .host_str()
            .ok_or_else(|| "the proof URL has no host".to_owned())?
            .to_owned();
        let mut args = FetchArgs::new(vec![url.to_string()], DEFAULT_USER_AGENT);
        args.screenshots = false;
        args.max_bytes = 4096;
        args.max_redirects = 0;
        args.delay_ms = 0;
        let out = self
            .fetch(
                FetchRequest {
                    site: site.clone(),
                    allowlist: vec![host.clone()],
                    args,
                    parent: Parent::Claim(host.clone()),
                    title: format!("Checked the proof file on {host} from the sandbox"),
                },
                &mut |_| {},
            )
            .await
            .map_err(|e| e.plain())?;
        let page = out.pages.first().ok_or_else(|| {
            out.problem
                .clone()
                .unwrap_or_else(|| "the fetch returned nothing".into())
        })?;
        if let Some(error) = &page.seen.error {
            return Err(format!("the sandbox could not reach {host}: {error}"));
        }
        let mut body = page.text();
        if body.len() > 4096 {
            let mut cut = 4096;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
        }
        Ok((page.seen.status.unwrap_or_default(), body))
    }
}

/// The egress proxy refuses with `403` and a body starting `agentrix-egress refused
/// <host>:<port>: <reason>`. A room may report that as the site's answer; it is not.
fn proxy_refusal(status: Option<u16>, body: Option<&Bytes>) -> Option<String> {
    let body = body?;
    if status != Some(403) || !body.starts_with(b"agentrix-egress refused") {
        return None;
    }
    let line = String::from_utf8_lossy(body);
    let what = line
        .lines()
        .next()
        .unwrap_or_default()
        .trim_start_matches("agentrix-egress refused")
        .trim();
    Some(format!("refused by the egress proxy ({what})"))
}

fn room_problem(outcome: &JobOutcome, fallback: &str) -> String {
    let r = &outcome.result;
    if !r.destroyed {
        "the room could not be confirmed destroyed, so its output was not read".into()
    } else if let Some(k) = r.killed {
        format!("the room was stopped ({k:?})")
    } else if r.exit_code.is_some_and(|c| c != 0) {
        format!(
            "the room exited with code {}",
            r.exit_code.unwrap_or_default()
        )
    } else {
        fallback.into()
    }
}

/// TXT strings from a dns.google JSON answer: `Answer[].data` where `type == 16`, quotes
/// stripped and a record's split strings joined. NXDOMAIN or no answer is an empty list.
pub fn parse_doh_txt(body: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "the DNS answer was not JSON".to_owned())?;
    let status = v
        .get("Status")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if status == 3 {
        return Ok(Vec::new());
    }
    if status != 0 {
        return Err(format!("the DNS lookup failed (status {status})"));
    }
    let mut out = Vec::new();
    for a in v
        .get("Answer")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        if a.get("type").and_then(serde_json::Value::as_u64) != Some(16) {
            continue;
        }
        let Some(data) = a.get("data").and_then(|d| d.as_str()) else {
            continue;
        };
        out.push(join_txt(data));
    }
    Ok(out)
}

/// `"abc" "def"` → `abcdef`; an unquoted string is kept as is.
fn join_txt(data: &str) -> String {
    let data = data.trim();
    if !data.starts_with('"') {
        return data.to_owned();
    }
    let mut out = String::new();
    let mut inside = false;
    let mut chars = data.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => inside = !inside,
            '\\' if inside => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            c if inside => out.push(c),
            _ => {}
        }
    }
    out
}
