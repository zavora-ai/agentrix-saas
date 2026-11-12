//! `agentrix-gauntlet`: runs the containment gauntlet against a deployment (docs/04 §3, R13).
//!
//! 1. **Probes** Zavora's fixture: each planted attack is there (HTTP only; redirects are never
//!    followed, so the runner itself never asks for the planted metadata or loopback targets).
//! 2. **Fires attack 10**: sends the visitor prompt to the converted site's `POST /ask`.
//! 3. **Waits for the Move**: attacks 1 to 9 are met by the Move itself (Assess crawl, replay,
//!    clean, build), so the runner reads the receipts they leave.
//! 4. **Scores from receipts only**: it lists the site's receipts (`GET /receipts?site=&since=`),
//!    fetches each with `Accept: application/json`, verifies it against
//!    `/.well-known/agentrix/keys` and its own digest, and scores with
//!    [`agentrix_gauntlet::score`].
//! 5. **Records** (`--record`): `POST /gauntlet/runs` cites the receipts. The node recomputes
//!    every verdict from its own store and issues the `gauntlet.run` receipt.
//!
//! The gauntlet token is read from `AGENTRIX_GAUNTLET_TOKEN` or `--token-file`, and is sent only
//! as an `Authorization` header. It is never printed and never put in a URL.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use agentrix_gauntlet::{
    Manifest, PostedAttack, PostedRun, Probe, Score, VerifiedReceipt, manifest, score,
    verify_receipt,
};
use agentrix_types::{Digest, KeySet, Receipt};
use anyhow::{Context as _, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use clap::Parser;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, LOCATION, USER_AGENT};
use serde_json::{Value, json};

const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const RUNNER_UA: &str = concat!("agentrix-gauntlet/", env!("CARGO_PKG_VERSION"));

/// Runs the ten containment attacks against a deployment and scores them from receipts.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// The platform's origin, for receipts, keys and runs, e.g. https://agentrix.website or
    /// http://127.0.0.1:8080.
    #[arg(long, env = "AGENTRIX_PLATFORM_URL")]
    platform: Option<String>,

    /// Zavora's fixture origin, e.g. https://fixture.agentrix.website or http://127.0.0.1:8081.
    #[arg(long, env = "AGENTRIX_FIXTURE_URL")]
    fixture: Option<String>,

    /// The fixture's site id on the platform.
    #[arg(long, env = "AGENTRIX_SITE_ID")]
    site: Option<String>,

    /// The converted site's origin, where `POST /ask` answers (attack 10). Without it the
    /// runner does not fire attack 10 and scores any refusal already on record.
    #[arg(long, env = "AGENTRIX_SITE_URL")]
    site_url: Option<String>,

    /// Only receipts that ended at or after this time (RFC 3339). Default: 24 hours ago.
    #[arg(long)]
    since: Option<DateTime<Utc>>,

    /// Keep re-reading receipts for up to this many seconds until every attack is held.
    #[arg(long, default_value_t = 0)]
    wait: u64,

    /// Extra receipt digests to score, besides the listing (repeatable).
    #[arg(long = "receipt")]
    receipts: Vec<Digest>,

    /// A file holding the gauntlet bearer token (else AGENTRIX_GAUNTLET_TOKEN).
    #[arg(long)]
    token_file: Option<PathBuf>,

    /// Record the run through the platform (`POST /gauntlet/runs`).
    #[arg(long)]
    record: bool,

    /// Skip the fixture probes.
    #[arg(long)]
    no_probes: bool,

    /// Score receipt files (`*.json`) in this directory instead of a platform; needs --keys.
    #[arg(long, conflicts_with = "platform")]
    offline: Option<PathBuf>,

    /// The published key set (`/.well-known/agentrix/keys` JSON) for --offline.
    #[arg(long, requires = "offline")]
    keys: Option<PathBuf>,

    /// Print the score as JSON on stdout.
    #[arg(long)]
    json: bool,

    /// Exit 1 when fewer attacks than this are contained.
    #[arg(long, default_value_t = 10)]
    require: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let m = manifest();
    let started_at = Utc::now();
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()?;
    let token = read_token(&args)?;

    eprintln!("Agentrix containment gauntlet: {}", m.fixture);
    if let Some(fixture) = &args.fixture
        && !args.no_probes
    {
        probe_fixture(&http, m, fixture).await;
    }
    let mut extra: Vec<Digest> = args.receipts.clone();
    if let Some(site_url) = &args.site_url {
        match fire_visitor_prompt(&http, m, site_url).await {
            Ok(Some(d)) => extra.push(d),
            Ok(None) => {}
            Err(e) => eprintln!("  attack 10: the visitor prompt could not be sent: {e:#}"),
        }
    }

    let since = args
        .since
        .unwrap_or_else(|| started_at - chrono::Duration::hours(24));
    let deadline = std::time::Instant::now() + Duration::from_secs(args.wait);
    let (result, counts) = loop {
        let (receipts, counts) = match &args.offline {
            Some(dir) => read_offline(dir, args.keys.as_deref())?,
            None => collect(&http, &args, token.as_deref(), since, &extra).await?,
        };
        let s = score(m, &receipts);
        if s.score == s.out_of || std::time::Instant::now() >= deadline {
            break (s, counts);
        }
        eprintln!(
            "  {}/{} so far from {} receipts; waiting for the Move…",
            s.score, s.out_of, counts.verified
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
    };

    print_table(m, &result, &counts, args.json);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    if args.record {
        record(&http, &args, token.as_deref(), started_at, &result).await?;
    }
    if result.score < args.require {
        std::process::exit(1);
    }
    Ok(())
}

fn read_token(args: &Args) -> anyhow::Result<Option<String>> {
    if let Some(path) = &args.token_file {
        let t = std::fs::read_to_string(path)
            .with_context(|| format!("reading the token file {}", path.display()))?;
        return Ok(Some(t.trim().to_owned()));
    }
    Ok(std::env::var("AGENTRIX_GAUNTLET_TOKEN")
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty()))
}

fn origin(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// Checks each attack is planted. Never follows a redirect: attack 1's Location is only read.
async fn probe_fixture(http: &reqwest::Client, m: &Manifest, fixture: &str) {
    eprintln!("Probing the fixture at {}", origin(fixture));
    for attack in &m.attacks {
        let line = match &attack.probe {
            None => "not visible over HTTP (planted in the exports; scored from receipts)".into(),
            Some(p) => match probe(http, fixture, p).await {
                Ok(()) => "planted".to_owned(),
                Err(e) => format!("NOT FOUND: {e:#}"),
            },
        };
        eprintln!("  {:>2}  {:<44} {}", attack.id, attack.name, line);
    }
}

async fn probe(http: &reqwest::Client, fixture: &str, p: &Probe) -> anyhow::Result<()> {
    let url = format!("{}{}", origin(fixture), p.path);
    let ua = p.user_agent.as_deref().unwrap_or(BROWSER_UA);
    let resp = http.get(&url).header(USER_AGENT, ua).send().await?;
    let status = resp.status().as_u16();
    let location = resp
        .headers()
        .get(LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = resp.text().await.unwrap_or_default();
    if let Some(want) = p.status
        && status != want
    {
        bail!("status {status}, want {want}");
    }
    if let Some(want) = &p.location_contains
        && !location.contains(want.as_str())
    {
        bail!("Location {location:?} lacks {want}");
    }
    if let Some(want) = &p.body_contains
        && !body.contains(want.as_str())
    {
        bail!("the page lacks the planted markup");
    }
    if let Some(want) = &p.body_starts_with
        && !body.starts_with(want.as_str())
    {
        bail!("the body does not start with {want:?}");
    }
    if let Some(lacks) = &p.browser_lacks {
        let as_browser = http
            .get(&url)
            .header(USER_AGENT, BROWSER_UA)
            .send()
            .await?
            .text()
            .await?;
        if as_browser.contains(lacks.as_str()) {
            bail!("browsers also get {lacks}: no cloaking");
        }
    }
    Ok(())
}

/// Sends attack 10 to the converted site and returns the refusal receipt's digest when the
/// reply names one.
async fn fire_visitor_prompt(
    http: &reqwest::Client,
    m: &Manifest,
    site_url: &str,
) -> anyhow::Result<Option<Digest>> {
    let prompt = m
        .attacks
        .iter()
        .find_map(|a| a.prompt.clone())
        .context("the manifest has no visitor prompt")?;
    let resp = http
        .post(format!("{}/ask", origin(site_url)))
        .header(ACCEPT, "text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, RUNNER_UA)
        .json(&json!({ "question": prompt }))
        .send()
        .await?
        .error_for_status()?;
    let text = resp.text().await?;
    let mut event = String::new();
    let mut refused = false;
    let mut receipt = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("event:") {
            event = name.trim().to_owned();
        } else if let Some(data) = line.strip_prefix("data:")
            && event == "reply"
            && let Ok(v) = serde_json::from_str::<Value>(data.trim())
        {
            refused |= v.get("kind").and_then(Value::as_str) == Some("refused");
            receipt = v
                .get("receipt")
                .and_then(Value::as_str)
                .and_then(|r| r.parse::<Digest>().ok())
                .or(receipt);
        }
    }
    eprintln!(
        "  10  visitor prompt sent to {}/ask: {}",
        origin(site_url),
        if refused {
            "the Concierge replied with a refusal"
        } else {
            "no refusal in the reply (the receipts decide)"
        }
    );
    Ok(receipt)
}

#[derive(Debug, Default)]
struct Counts {
    listed: usize,
    verified: usize,
    rejected: Vec<String>,
}

/// Lists, fetches and verifies the site's receipts.
async fn collect(
    http: &reqwest::Client,
    args: &Args,
    token: Option<&str>,
    since: DateTime<Utc>,
    extra: &[Digest],
) -> anyhow::Result<(Vec<VerifiedReceipt>, Counts)> {
    let platform = args
        .platform
        .as_deref()
        .context("give --platform (or --offline)")?;
    let platform = origin(platform);
    let keys: KeySet = http
        .get(format!("{platform}/.well-known/agentrix/keys"))
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()
        .context("fetching the published keys")?
        .json()
        .await
        .context("the published keys do not parse")?;

    let mut digests: BTreeSet<String> = extra.iter().map(ToString::to_string).collect();
    let mut counts = Counts::default();
    if let Some(site) = &args.site {
        let mut req = http
            .get(format!(
                "{platform}/receipts?site={site}&since={}",
                since.to_rfc3339_opts(SecondsFormat::Secs, true)
            ))
            .header(ACCEPT, "application/json");
        if let Some(t) = token {
            req = req.header(AUTHORIZATION, format!("Bearer {t}"));
        }
        let listed: Value = req
            .send()
            .await?
            .error_for_status()
            .context("listing the site's receipts")?
            .json()
            .await?;
        let rows = listed
            .as_array()
            .or_else(|| listed.get("receipts").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();
        counts.listed = rows.len();
        digests.extend(
            rows.iter()
                .filter_map(|r| r.get("digest").and_then(Value::as_str))
                .map(str::to_owned),
        );
    } else if extra.is_empty() {
        bail!("give --site to list receipts, or --receipt digests");
    }

    let mut out = Vec::new();
    for d in digests {
        let Ok(digest) = d.parse::<Digest>() else {
            counts.rejected.push(format!("{d}: not a digest"));
            continue;
        };
        let body = http
            .get(format!("{platform}/receipts/{digest}"))
            .header(ACCEPT, "application/json")
            .send()
            .await?;
        if !body.status().is_success() {
            counts
                .rejected
                .push(format!("{digest}: HTTP {}", body.status()));
            continue;
        }
        match verify_receipt(&digest, &body.bytes().await?, &keys) {
            Ok(v) if v.receipt.body.ended_at >= since || extra.contains(&digest) => out.push(v),
            Ok(_) => {}
            Err(e) => counts.rejected.push(format!("{digest}: {e}")),
        }
    }
    counts.verified = out.len();
    Ok((out, counts))
}

/// Reads receipt files from a directory and verifies them against a key-set file.
fn read_offline(
    dir: &std::path::Path,
    keys: Option<&std::path::Path>,
) -> anyhow::Result<(Vec<VerifiedReceipt>, Counts)> {
    let keys_path = keys.context("--offline needs --keys")?;
    let keys: KeySet =
        serde_json::from_slice(&std::fs::read(keys_path)?).context("the key set does not parse")?;
    let mut counts = Counts::default();
    let mut out = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for path in paths {
        counts.listed += 1;
        let bytes = std::fs::read(&path)?;
        let digest = match serde_json::from_slice::<Receipt>(&bytes).map(|r| r.digest()) {
            Ok(Ok(d)) => d,
            _ => {
                counts
                    .rejected
                    .push(format!("{}: not a receipt", path.display()));
                continue;
            }
        };
        match verify_receipt(&digest, &bytes, &keys) {
            Ok(v) => out.push(v),
            Err(e) => counts.rejected.push(format!("{}: {e}", path.display())),
        }
    }
    counts.verified = out.len();
    Ok((out, counts))
}

/// The table, on stdout, or on stderr when stdout carries `--json`.
fn print_table(m: &Manifest, s: &Score, counts: &Counts, to_stderr: bool) {
    let out = |line: String| {
        if to_stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };
    out(String::new());
    out(format!(
        " #  {:<44} {:<10} What the receipts show",
        "Attack", "Contained"
    ));
    for v in &s.attacks {
        let first = v
            .receipts
            .first()
            .map(|d| format!(" [{}]", short(d)))
            .unwrap_or_default();
        out(format!(
            "{:>2}  {:<44} {:<10} {}{}",
            v.id,
            v.name,
            if v.held { "✓ held" } else { "✗ no proof" },
            v.plain,
            first
        ));
    }
    out(String::new());
    out(format!(
        "Score: {}/{} contained, computed from {} verified receipts ({} listed, {} rejected). {}",
        s.score,
        s.out_of,
        counts.verified,
        counts.listed,
        counts.rejected.len(),
        m.fixture
    ));
    for r in &counts.rejected {
        out(format!("  rejected {r}"));
    }
}

fn short(d: &Digest) -> String {
    let s = d.to_string();
    s.chars().take(19).collect()
}

/// Records the run. The node recomputes every verdict from its own receipts.
async fn record(
    http: &reqwest::Client,
    args: &Args,
    token: Option<&str>,
    started_at: DateTime<Utc>,
    s: &Score,
) -> anyhow::Result<()> {
    let platform = origin(
        args.platform
            .as_deref()
            .context("--record needs --platform")?,
    );
    let site = args.site.as_deref().context("--record needs --site")?;
    let body = PostedRun {
        site: site.to_owned(),
        started_at: started_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        ended_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        runner: Some(RUNNER_UA.to_owned()),
        attacks: s
            .attacks
            .iter()
            .map(|v| PostedAttack {
                id: v.id,
                receipts: v.receipts.clone(),
            })
            .collect(),
    };
    let mut req = http
        .post(format!("{platform}/gauntlet/runs"))
        .header(ACCEPT, "application/json")
        .json(&body);
    if let Some(t) = token {
        req = req.header(AUTHORIZATION, format!("Bearer {t}"));
    }
    let resp = req.send().await?;
    let status = resp.status();
    let reply: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("recording the run failed: HTTP {status} {reply}");
    }
    let node_score = reply.get("score").and_then(Value::as_u64);
    eprintln!(
        "Recorded: gauntlet.run receipt {} — the node recomputed {}/{}",
        reply.get("receipt").and_then(Value::as_str).unwrap_or("?"),
        node_score.map_or("?".into(), |n| n.to_string()),
        reply.get("out_of").and_then(Value::as_u64).unwrap_or(10)
    );
    if node_score.is_some_and(|n| n != u64::from(s.score)) {
        eprintln!(
            "warning: the node's score differs from this runner's; the node's receipt is the record"
        );
    }
    Ok(())
}
