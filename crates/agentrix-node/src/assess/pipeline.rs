//! One assessment, start to finish (R1): robots.txt, then the site breadth-first in a few
//! `fetch` jobs, then the detectors and the report.
//!
//! Policy lives here, on control: only the site's hosts are on the allowlist, robots.txt is
//! read first and obeyed (and an unreachable robots.txt means "don't"), requests are spaced by
//! the crawl delay (at least [`AssessConfig::min_delay_ms`]), every request carries the crawler's
//! identity, and the crawl stops at [`AssessConfig::max_pages`].

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use agentrix_types::ui::{
    DoneEvent, DoneOutcome, ErrorEvent, FindingEvent, StepEvent, StepName, StepState,
};
use agentrix_types::{Digest, Finding, FindingId, UiEvent};
use chrono::Utc;
use reqwest::Url;
use tokio::sync::Semaphore;

use super::AssessConfig;
use super::detect::{self, AI_CRAWLERS, Crawl, Page};
use super::html;
use super::report::{Outcome, ReportDoc, StoredFinding, Totals};
use super::robots::Robots;
use super::store::{AssessStore, Assessment, AssessmentState, Emitter, Hub};
use crate::broker::fetch::{FetchArgs, FetchOutcome, FetchRequest};
use crate::broker::ui::UiMapper;
use crate::broker::{Broker, BrokerError, BrokerEvent, Parent};

/// What a running assessment needs.
pub struct Ctx {
    /// The broker client.
    pub broker: Broker,
    /// Persistence.
    pub store: Arc<dyn AssessStore>,
    /// Live assessments.
    pub hub: Hub,
    /// Settings.
    pub config: AssessConfig,
    /// Assessments that may run at once.
    pub slots: Arc<Semaphore>,
}

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// The hosts a site's crawl may reach: its own, and its `www.` twin.
pub fn allowlist(host: &str) -> Vec<String> {
    let host = host.to_ascii_lowercase();
    let twin = match host.strip_prefix("www.") {
        Some(bare) => bare.to_owned(),
        None => format!("www.{host}"),
    };
    vec![host, twin]
}

/// Paths a crawler of public pages has no business in.
fn skip_path(path: &str, query: Option<&str>) -> bool {
    let p = path.to_ascii_lowercase();
    const SKIP_EXT: &[&str] = &[
        ".jpg", ".jpeg", ".png", ".gif", ".webp", ".svg", ".ico", ".pdf", ".zip", ".gz", ".mp4",
        ".mp3", ".mov", ".avi", ".css", ".js", ".woff", ".woff2", ".ttf", ".xml", ".json", ".doc",
        ".docx", ".xls", ".xlsx", ".ppt", ".pptx", ".rss",
    ];
    SKIP_EXT.iter().any(|e| p.ends_with(e))
        || p.contains("/wp-admin")
        || p.contains("/wp-login")
        || p.contains("/wp-json")
        || p.contains("xmlrpc.php")
        || p.ends_with("/feed/")
        || p.ends_with("/feed")
        || p.contains("/cart")
        || p.contains("/checkout")
        || p.contains("/my-account")
        || query.is_some_and(|q| {
            q.contains("replytocom") || q.contains("add-to-cart") || q.contains("s=")
        })
}

/// `href` on `page` as a crawlable URL on the site, or `None`.
fn crawlable(page: &str, href: &str, hosts: &[String]) -> Option<String> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with('#')
        || href.starts_with("mailto:")
        || href.starts_with("tel:")
        || href.starts_with("javascript:")
    {
        return None;
    }
    let mut u = Url::parse(page).ok()?.join(href).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    if !hosts.contains(&host) || u.port().is_some() {
        return None;
    }
    if skip_path(u.path(), u.query()) {
        return None;
    }
    u.set_fragment(None);
    Some(u.to_string())
}

/// `<loc>` URLs in a sitemap.
fn sitemap_locs(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(p) = rest.find("<loc>") {
        rest = &rest[p + 5..];
        let Some(e) = rest.find("</loc>") else { break };
        out.push(html::decode(rest[..e].trim()));
        rest = &rest[e + 6..];
        if out.len() >= 500 {
            break;
        }
    }
    out
}

/// Plain words for why the start page could not be read. `refusals` are what the egress proxy
/// refused in that job: its reason tells a redirect off the site (policy) from a network
/// failure on the site's own host (DNS, connect, timeout), which the room sees the same way.
fn unreachable_words(
    host: &str,
    page: &Page,
    refusals: &[agentrix_types::Refusal],
    hosts: &[String],
) -> (String, String) {
    if let Some(error) = &page.error {
        let e = error.to_ascii_lowercase();
        if e.contains("refused") {
            let elsewhere = refusals.iter().find(|r| {
                detect::policy_refusal(&r.reason) && !hosts.contains(&r.host.to_ascii_lowercase())
            });
            let own = refusals
                .iter()
                .find(|r| hosts.contains(&r.host.to_ascii_lowercase()));
            match (elsewhere, own) {
                (Some(r), _) => {
                    let to = &r.host;
                    return (
                        format!(
                            "{host} sends visitors on to {to}, and this check may only visit {host}."
                        ),
                        format!("If your site has moved, check the new address instead: {to}."),
                    );
                }
                (None, Some(r)) => {
                    let why = r.reason.to_ascii_lowercase();
                    if why.starts_with("dns for") || why.ends_with("has no addresses") {
                        return (
                            format!("We couldn't find {host}: its name doesn't lead to a server."),
                            "Check the address for typos, or ask whoever manages your domain whether it's set up.".into(),
                        );
                    }
                    if why.contains("timed out") {
                        return (
                            format!("{host} didn't answer: connecting to it timed out."),
                            "Your site may be down. Try again in a few minutes; if it keeps happening, ask your hosting company.".into(),
                        );
                    }
                    if why.starts_with("connect to") {
                        return (
                            format!("{host}'s server didn't accept a connection."),
                            "Your site may be down. Try again in a few minutes; if it keeps happening, ask your hosting company.".into(),
                        );
                    }
                    if why.contains("non-public") {
                        return (
                            format!(
                                "{host} points to a private network address, which a public check can't visit."
                            ),
                            "Ask whoever manages your domain to point it at your public web server."
                                .into(),
                        );
                    }
                }
                (None, None) => {}
            }
            return (
                format!("{host} sends visitors somewhere this check may not go."),
                "If your site has moved, check its new address instead.".into(),
            );
        }
        if e.contains("dns") || e.contains("resolve") {
            return (
                format!("We couldn't find {host}: its name doesn't lead to a server."),
                "Check the address for typos, or ask whoever manages your domain whether it's set up.".into(),
            );
        }
        if e.contains("tls") || e.contains("certificate") {
            return (
                format!(
                    "We couldn't make a secure (HTTPS) connection to {host}: its certificate or \
                     its setup has a problem."
                ),
                "Ask your hosting company to check or renew the certificate, then try again."
                    .into(),
            );
        }
        if e.contains("time") {
            return (
                format!("{host} didn't answer within 15 seconds."),
                "Your site may be down or very slow. Try again in a few minutes; if it keeps happening, ask your hosting company.".into(),
            );
        }
        return (
            format!("We couldn't reach {host} ({error})."),
            "Check the address, then try again in a few minutes.".into(),
        );
    }
    match page.status {
        Some(s @ (401 | 403)) => (
            format!("{host} refused our crawler (HTTP {s})."),
            "Some security plugins and firewalls block crawlers. Allow AgentrixBot, then try again.".into(),
        ),
        Some(404) => (
            format!("{host} answered \"not found\" (HTTP 404) for that address."),
            "Check the address, or try your home page.".into(),
        ),
        Some(s) if s >= 500 => (
            format!("{host} answered with a server error (HTTP {s})."),
            "Your site may be having trouble. Try again in a few minutes; if it keeps happening, ask your hosting company.".into(),
        ),
        Some(s) if s >= 400 => (
            format!("{host} answered with an error (HTTP {s})."),
            "Check the address, then try again.".into(),
        ),
        _ => (
            format!("{host} didn't send a web page."),
            "Check that the address is your website's, then try again.".into(),
        ),
    }
}

struct Run {
    ctx: Arc<Ctx>,
    a: Assessment,
    emit: Arc<Emitter>,
    mapper: UiMapper,
    started: Instant,
    receipts: Vec<Digest>,
    main_receipt: Option<(usize, Digest)>,
    crawl: Crawl,
}

impl Run {
    fn step(&self, state: StepState, summary: &str) -> UiEvent {
        let ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        UiEvent::Step(StepEvent::new(
            StepName::Assess,
            state,
            ms,
            summary,
            Utc::now(),
        ))
    }

    /// Runs one fetch job, streaming its events, and records its receipt.
    async fn batch(
        &mut self,
        urls: Vec<(String, Option<u32>)>,
        of: u32,
        delay_ms: u64,
        what: &str,
    ) -> Result<FetchOutcome, BrokerError> {
        self.mapper.fetch_job(&urls, of);
        self.emit.plan_pages(of);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<UiEvent>();
        let emit = self.emit.clone();
        let forward = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                emit.emit(ev).await;
            }
        });
        let mut args = FetchArgs::new(
            urls.iter().map(|(u, _)| u.clone()).collect(),
            self.ctx.config.user_agent.clone(),
        );
        args.delay_ms = delay_ms;
        args.timeout_ms = 15_000;
        args.screenshots = urls.iter().any(|(_, n)| n.is_some());
        let n_pages = urls.iter().filter(|(_, n)| n.is_some()).count();
        let req = FetchRequest {
            site: self.a.site.clone(),
            allowlist: self.crawl.hosts.clone(),
            args,
            parent: Parent::Assessment(self.a.id.clone()),
            title: if n_pages > 0 {
                format!(
                    "Read {} of {} in a sealed room",
                    plural(n_pages, "page", "pages"),
                    self.a.host
                )
            } else {
                format!("Read {what} of {} in a sealed room", self.a.host)
            },
        };
        let mapper = &mut self.mapper;
        let result = self
            .ctx
            .broker
            .fetch(req, &mut |ev: BrokerEvent| {
                for ui in mapper.on(&ev, Utc::now()) {
                    let _ = tx.send(ui);
                }
            })
            .await;
        drop(tx);
        let _ = forward.await;
        if let Ok(out) = &result {
            self.receipts.push(out.job.receipt);
            if self.main_receipt.is_none_or(|(n, _)| n_pages > n) {
                self.main_receipt = Some((n_pages, out.job.receipt));
            }
            for r in &out.job.result.refusals {
                self.crawl.refusals.push((r.clone(), Some(out.job.receipt)));
            }
        }
        result
    }

    fn page_from(&self, n: u32, f: &crate::broker::fetch::FetchedPage, receipt: Digest) -> Page {
        let body = f.body.as_ref().map(|b| b.to_vec()).unwrap_or_default();
        let facts = (f.is_html() && f.seen.status.is_some_and(|s| s < 400)).then(|| {
            let mut facts = html::read(&String::from_utf8_lossy(&body));
            if let Some(dom) = &f.rendered {
                facts.merge_rendered(html::read(&String::from_utf8_lossy(dom)));
            }
            facts
        });
        Page {
            n,
            url: f.seen.url.clone(),
            final_url: f.seen.final_url.clone(),
            status: f.seen.status,
            error: f.seen.error.clone(),
            ms: f.seen.ms,
            bytes: f.seen.bytes,
            truncated: f.seen.truncated,
            content_type: f.seen.content_type.clone(),
            digest: f.digest,
            receipt: Some(receipt),
            facts,
            body,
            referrers: Vec::new(),
        }
    }

    async fn end(
        &mut self,
        state: AssessmentState,
        doc: ReportDoc,
        outcome: DoneOutcome,
        summary: &str,
    ) {
        let at = Utc::now();
        if let Err(e) = self.ctx.store.finish(&self.a.id, state, &doc, at).await {
            tracing::error!(assessment = %self.a.id, error = %e, "could not store the report");
        }
        let mut step = self.step(
            if state == AssessmentState::Failed {
                StepState::Failed
            } else {
                StepState::Done
            },
            summary,
        );
        if let (UiEvent::Step(s), Some(r)) = (&mut step, doc.receipt) {
            s.receipt = Some(r);
        }
        self.emit.emit(step).await;
        self.emit
            .emit(UiEvent::Done(DoneEvent {
                outcome,
                href: Some(format!("/assess/{}", self.a.id)),
                at,
            }))
            .await;
    }

    fn totals(&self) -> Totals {
        let t = self.mapper.tick(Utc::now());
        Totals {
            hosts: t.hosts,
            requests: t.requests,
            bytes: t.bytes,
            refusals: t.refusals,
        }
    }

    async fn fail(&mut self, e: &BrokerError) {
        tracing::warn!(assessment = %self.a.id, error = %e, "assessment failed");
        self.emit
            .emit(UiEvent::Error(ErrorEvent {
                step: Some(StepName::Assess),
                message: e.plain(),
                retrying: false,
                at: Utc::now(),
            }))
            .await;
        let mut doc = ReportDoc::without_report(
            Outcome::Failed,
            format!(
                "{} So we didn't read {}, and we haven't guessed at any findings.",
                e.plain(),
                self.a.host
            ),
            e.advice(),
        );
        doc.receipts = self.receipts.clone();
        doc.totals = self.totals();
        self.end(
            AssessmentState::Failed,
            doc,
            DoneOutcome::Stopped,
            "The check couldn't run",
        )
        .await;
    }

    async fn unreachable(&mut self, outcome: Outcome, said: String, next: String) {
        let mut doc = ReportDoc::without_report(outcome, said, next);
        doc.receipts = self.receipts.clone();
        doc.receipt = self.receipts.last().copied();
        doc.totals = self.totals();
        let summary = if outcome == Outcome::Refused {
            format!("{} asked us not to read it", self.a.host)
        } else {
            format!("We couldn't reach {}", self.a.host)
        };
        self.end(
            AssessmentState::Unreachable,
            doc,
            DoneOutcome::Unreachable,
            &summary,
        )
        .await;
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Runs one assessment to its report. Never panics; every ending is stored and announced.
pub async fn run(ctx: Arc<Ctx>, a: Assessment, emit: Arc<Emitter>) {
    let hosts = allowlist(&a.host);
    let mut run = Run {
        ctx: ctx.clone(),
        crawl: Crawl {
            hosts,
            ..Crawl::default()
        },
        a,
        emit,
        mapper: UiMapper::new(),
        started: Instant::now(),
        receipts: Vec::new(),
        main_receipt: None,
    };
    let summary = format!("Opening a sealed room that can reach only {}", run.a.host);
    let step = run.step(StepState::Running, &summary);
    run.emit.emit(step).await;

    let _permit = match ctx.slots.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return,
    };
    let limit = std::time::Duration::from_secs(20 * 60);
    if tokio::time::timeout(limit, crawl_and_report(&mut run))
        .await
        .is_err()
    {
        run.fail(&BrokerError::Protocol(
            "the assessment ran out of time".into(),
        ))
        .await;
    }
    let id = run.a.id.clone();
    let hub = ctx.hub.clone();
    // Late subscribers still get the end from the store; the live state goes after a while.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        hub.forget(&id);
    });
}

async fn crawl_and_report(run: &mut Run) {
    let cfg = run.ctx.config.clone();
    let start = match Url::parse(&run.a.url) {
        Ok(u) => u,
        Err(_) => {
            run.unreachable(
                Outcome::Unreachable,
                "That address doesn't look like a website.".into(),
                "Check it and try again.".into(),
            )
            .await;
            return;
        }
    };
    let origin = start.origin().ascii_serialization();

    // 1. robots.txt, before anything else.
    let robots_url = format!("{origin}/robots.txt");
    let out = match run
        .batch(vec![(robots_url, None)], 0, 0, "the robots.txt file")
        .await
    {
        Ok(o) => o,
        Err(e) => return run.fail(&e).await,
    };
    let Some(robots_page) = out.pages.first() else {
        let e = BrokerError::Protocol(out.problem.unwrap_or_else(|| "no robots.txt result".into()));
        return run.fail(&e).await;
    };
    let robots_seen = run.page_from(0, robots_page, out.job.receipt);
    let robots = match (robots_seen.status, &robots_seen.error) {
        (Some(200), _) => {
            let text = String::from_utf8_lossy(&robots_seen.body).into_owned();
            if text.trim_start().starts_with('<') {
                Robots::allow_all()
            } else {
                Robots::parse(&text)
            }
        }
        (Some(s), _) if (400..500).contains(&s) => Robots::allow_all(),
        (Some(_), _) => Robots::disallow_all(),
        (None, _) => {
            let (said, next) = unreachable_words(
                &run.a.host,
                &robots_seen,
                &out.job.result.refusals,
                &run.crawl.hosts,
            );
            return run.unreachable(Outcome::Unreachable, said, next).await;
        }
    };
    let token = cfg.robots_token.as_str();
    let start_path = match start.query() {
        Some(q) => format!("{}?{q}", start.path()),
        None => start.path().to_owned(),
    };
    if !robots.allows(token, &start_path) {
        let said = if robots_seen.status.is_some_and(|s| s >= 500) {
            format!(
                "{}'s robots.txt file, which tells crawlers what they may read, answered with a server error. The rules for crawlers say to stay away until it works, so we did.",
                run.a.host
            )
        } else {
            format!(
                "{}'s robots.txt file, which tells crawlers what they may read, asks crawlers like ours ({token}) not to read it, so we didn't.",
                run.a.host
            )
        };
        return run
            .unreachable(
                Outcome::Refused,
                said,
                format!("If you'd like a report, allow {token} in robots.txt and try again. Or sign in and prove the site is yours: the Move can read it with your permission."),
            )
            .await;
    }
    run.crawl.ai_blocked = AI_CRAWLERS
        .iter()
        .filter(|c| robots.blocks_site(c))
        .map(|c| (*c).to_owned())
        .collect();
    let delay_ms = robots
        .crawl_delay(token)
        .map_or(cfg.min_delay_ms, |s| (s * 1000.0) as u64)
        .clamp(cfg.min_delay_ms, 10_000);
    // Keep each room inside its 10-minute cap.
    let budget = cfg
        .max_pages
        .min(u32::try_from(480_000 / (delay_ms + 2_000)).unwrap_or(u32::MAX))
        .max(1);

    // 2. The start page, the sitemap and llms.txt.
    let sitemap_url = robots
        .sitemaps
        .iter()
        .find(|s| {
            Url::parse(s)
                .ok()
                .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
                .is_some_and(|h| run.crawl.hosts.contains(&h))
        })
        .cloned()
        .unwrap_or_else(|| format!("{origin}/sitemap.xml"));
    let mut first = vec![(start.to_string(), Some(1))];
    for extra in [sitemap_url.clone(), format!("{origin}/llms.txt")] {
        if let Ok(u) = Url::parse(&extra)
            && robots.allows(token, u.path())
        {
            first.push((extra, None));
        }
    }
    let out = match run.batch(first.clone(), 1, delay_ms, "the home page").await {
        Ok(o) => o,
        Err(e) => return run.fail(&e).await,
    };
    if out.pages.is_empty() {
        let e = BrokerError::Protocol(out.problem.unwrap_or_else(|| "no pages".into()));
        return run.fail(&e).await;
    }
    let home = run.page_from(1, &out.pages[0], out.job.receipt);
    if home.facts.is_none() {
        let (said, next) = unreachable_words(
            &run.a.host,
            &home,
            &out.job.result.refusals,
            &run.crawl.hosts,
        );
        return run.unreachable(Outcome::Unreachable, said, next).await;
    }
    for (i, f) in out.pages.iter().enumerate().skip(1) {
        let ok = f.seen.status == Some(200);
        if first.get(i).is_some_and(|(u, _)| u.ends_with("/llms.txt")) {
            run.crawl.llms_txt = Some(ok && !f.is_html());
        } else {
            run.crawl.sitemap = Some(ok);
        }
    }
    let sitemap: Vec<String> = out
        .pages
        .iter()
        .skip(1)
        .filter(|f| f.seen.url == sitemap_url && f.seen.status == Some(200))
        .flat_map(|f| sitemap_locs(&f.text()))
        .collect();

    // 3. Breadth-first over the site's own links, within the budget.
    let mut pages: Vec<Page> = vec![home];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    seen.insert(start.to_string());
    if let Some(f) = &pages[0].final_url {
        seen.insert(f.clone());
    }
    let mut referrers: HashMap<String, Vec<String>> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let hosts = run.crawl.hosts.clone();
    let discover = |page: &Page,
                    queue: &mut VecDeque<String>,
                    seen: &mut BTreeSet<String>,
                    referrers: &mut HashMap<String, Vec<String>>| {
        let Some(f) = &page.facts else { return };
        let base = page.final_url.clone().unwrap_or_else(|| page.url.clone());
        for a in &f.anchors {
            if a.hidden.is_some() {
                continue;
            }
            let Some(u) = crawlable(&base, &a.href, &hosts) else {
                continue;
            };
            let refs = referrers.entry(u.clone()).or_default();
            if !refs.contains(&page.url) && refs.len() < 10 {
                refs.push(page.url.clone());
            }
            if seen.insert(u.clone()) {
                queue.push_back(u);
            }
        }
    };
    discover(&pages[0], &mut queue, &mut seen, &mut referrers);
    for loc in sitemap {
        if let Some(u) = crawlable(&loc, &loc, &run.crawl.hosts)
            && seen.insert(u.clone())
        {
            queue.push_back(u);
        }
    }
    let mut blocked_by_robots = 0usize;
    for _round in 0..4 {
        let room = budget.saturating_sub(pages.len() as u32) as usize;
        if room == 0 || queue.is_empty() {
            break;
        }
        let mut next = Vec::new();
        while next.len() < room
            && let Some(u) = queue.pop_front()
        {
            let path = Url::parse(&u).map(|x| match x.query() {
                Some(q) => format!("{}?{q}", x.path()),
                None => x.path().to_owned(),
            });
            if path.is_ok_and(|p| robots.allows(token, &p)) {
                next.push(u);
            } else {
                blocked_by_robots += 1;
            }
        }
        if next.is_empty() {
            break;
        }
        let first_n = pages.len() as u32 + 1;
        let of = (pages.len() + next.len() + queue.len()).min(budget as usize) as u32;
        let urls: Vec<(String, Option<u32>)> = next
            .iter()
            .enumerate()
            .map(|(i, u)| (u.clone(), Some(first_n + i as u32)))
            .collect();
        let out = match run.batch(urls, of, delay_ms, "pages").await {
            Ok(o) => o,
            Err(e) => {
                // Pages already read still make a report; say what stopped.
                tracing::warn!(assessment = %run.a.id, error = %e, "a crawl batch failed");
                run.emit
                    .emit(UiEvent::Error(ErrorEvent {
                        step: Some(StepName::Assess),
                        message: format!("{} We'll report on the pages we read.", e.plain()),
                        retrying: false,
                        at: Utc::now(),
                    }))
                    .await;
                break;
            }
        };
        for (i, f) in out.pages.iter().enumerate() {
            let page = run.page_from(first_n + i as u32, f, out.job.receipt);
            discover(&page, &mut queue, &mut seen, &mut referrers);
            pages.push(page);
        }
    }
    for p in &mut pages {
        p.referrers = referrers.get(&p.url).cloned().unwrap_or_default();
    }
    run.crawl.unchecked_links = queue.len() + blocked_by_robots;
    run.crawl.pages = pages;

    // 4. Findings, with evidence in the CAS.
    let found = detect::detect(&run.crawl);
    let mut stored = Vec::with_capacity(found.len());
    let mut rows = Vec::with_capacity(found.len());
    for f in found {
        let id = FindingId::generate();
        let at = Utc::now();
        let evidence_json = serde_json::to_vec(&serde_json::json!({
            "finding": id,
            "detector": f.detector,
            "title": f.title,
            "evidence": f.evidence,
        }))
        .unwrap_or_default();
        let mut evidence: Vec<Digest> = Vec::new();
        match run
            .ctx
            .broker
            .blobs()
            .put(&run.a.site, "evidence", "application/json", &evidence_json)
            .await
        {
            Ok(d) => evidence.push(d),
            Err(e) => tracing::warn!(error = %e, "could not store a finding's evidence"),
        }
        for p in f.evidence.iter().filter_map(|e| e.page) {
            if !evidence.contains(&p) {
                evidence.push(p);
            }
        }
        let row = Finding {
            id: id.clone(),
            site: run.a.site.clone(),
            detector: f.detector.clone(),
            severity: f.severity,
            category: f.category,
            title: f.title.clone(),
            explanation: f.plain.clone(),
            location: f.location.clone(),
            evidence,
            at,
        };
        run.emit
            .emit(UiEvent::Finding(FindingEvent::from(&row)))
            .await;
        rows.push(row);
        stored.push(StoredFinding { id, found: f });
    }
    if let Err(e) = run.ctx.store.save_findings(&run.a.id, &rows).await {
        tracing::error!(assessment = %run.a.id, error = %e, "could not store findings");
    }

    // 5. The report.
    let read: Vec<&Page> = run.crawl.pages.iter().filter(|p| p.is_read()).collect();
    let pages_read = read.len() as u32;
    let doc = ReportDoc {
        outcome: Outcome::Report,
        pages_read,
        pages_tried: run.crawl.pages.len() as u32,
        unchecked_links: run.crawl.unchecked_links as u32,
        slowest_ms: read.iter().filter_map(|p| p.ms).max(),
        summary: super::report::summary(&run.a.host, pages_read, &stored),
        findings: stored,
        receipt: run.main_receipt.map(|(_, d)| d),
        receipts: run.receipts.clone(),
        said: None,
        next_step: None,
        totals: run.totals(),
        frame: run.emit.live().state().frame,
        pages: read
            .iter()
            .filter(|p| {
                p.content_type
                    .as_deref()
                    .is_some_and(|t| t.contains("html"))
            })
            .filter_map(|p| {
                p.digest.map(|digest| super::report::ReadPage {
                    url: p.final_url.clone().unwrap_or_else(|| p.url.clone()),
                    digest,
                })
            })
            .collect(),
    };
    let words = format!(
        "Read {} and wrote the report",
        plural(pages_read as usize, "page", "pages")
    );
    run.end(
        AssessmentState::ReportReady,
        doc,
        DoneOutcome::ReportReady,
        &words,
    )
    .await;
}
