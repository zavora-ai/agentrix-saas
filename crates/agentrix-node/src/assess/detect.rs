//! Detectors: what the crawl saw, turned into findings in plain words with evidence (R1.5).
//!
//! Every finding names the pages it was seen on, quotes what was seen, and links the receipt of
//! the sealed-room job that fetched it. Nothing here guesses: a detector that cannot see a thing
//! from public pages does not report it (the honesty rule).
//!
//! Detector ids match the gauntlet's (`spam-links`, `hidden-instructions`,
//! `obfuscated-script`, `executable-upload`) so the score can be read from findings.

use std::collections::{BTreeMap, BTreeSet};

use agentrix_types::{Category, Digest, Refusal, Severity};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use super::html::PageFacts;

/// One page the crawl tried.
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// Its number in the crawl, from 1.
    pub n: u32,
    /// The URL asked for.
    pub url: String,
    /// Where redirects ended.
    pub final_url: Option<String>,
    /// HTTP status, if a response came.
    pub status: Option<u16>,
    /// Why no response came.
    pub error: Option<String>,
    /// Time to the last byte, in the sealed room.
    pub ms: Option<u64>,
    /// Body bytes kept.
    pub bytes: u64,
    /// Whether the body was cut short.
    pub truncated: bool,
    /// `Content-Type`.
    pub content_type: Option<String>,
    /// The body's digest.
    pub digest: Option<Digest>,
    /// The receipt of the job that fetched it.
    pub receipt: Option<Digest>,
    /// What the HTML says, for HTML pages.
    pub facts: Option<PageFacts>,
    /// The body, for content checks.
    pub body: Vec<u8>,
    /// Pages that link here.
    pub referrers: Vec<String>,
}

impl Page {
    /// Whether it was read as HTML.
    pub fn is_read(&self) -> bool {
        self.facts.is_some() && self.status.is_some_and(|s| s < 400)
    }
}

/// Everything the crawl learned.
#[derive(Debug, Clone, Default)]
pub struct Crawl {
    /// The site's hosts (the allowlist).
    pub hosts: Vec<String>,
    /// Pages tried, in order.
    pub pages: Vec<Page>,
    /// AI crawlers robots.txt turns away.
    pub ai_blocked: Vec<String>,
    /// Whether `/llms.txt` exists; `None` if not checked.
    pub llms_txt: Option<bool>,
    /// Whether a sitemap was found; `None` if not checked.
    pub sitemap: Option<bool>,
    /// Links within the site that weren't fetched (the 50-page budget, or robots.txt).
    pub unchecked_links: usize,
    /// Every request the egress proxy refused.
    pub refusals: Vec<(Refusal, Option<Digest>)>,
}

/// One piece of evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proof {
    /// Where and what, in plain words.
    pub label: String,
    /// What was seen, quoted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// The page body it was seen in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Digest>,
    /// The receipt of the sealed-room job that saw it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Digest>,
}

/// A finding, before it is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// Which detector raised it.
    pub detector: String,
    /// How much it matters.
    pub severity: Severity,
    /// What it is about.
    pub category: Category,
    /// Plain words.
    pub title: String,
    /// Why it matters and what the Move does about it.
    pub plain: String,
    /// Where: a page URL, or "3 pages".
    pub location: String,
    /// What was seen.
    pub evidence: Vec<Proof>,
}

/// Words of the link spam injected into compromised sites (after adk-site's
/// `DEFAULT_SPAM_KEYWORDS`, with a few the fixture and the wild use).
pub const SPAM_WORDS: &[&str] = &[
    "casino",
    "poker",
    "sports betting",
    "betting tips",
    "viagra",
    "cialis",
    "levitra",
    "payday loan",
    "payday loans",
    "instant loans",
    "fast loans",
    "replica watches",
    "essay writing service",
    "porn",
    "xxx",
    "escort",
    "escorts",
    "buy followers",
    "free spins",
    "slot gacor",
    "judi online",
    "togel",
    "crypto giveaway",
    "online pharmacy",
    "pills",
    "weight loss pills",
];

/// Script hosts common enough on small-business sites not to be worth a finding.
const KNOWN_SCRIPT_HOSTS: &[&str] = &[
    "googletagmanager.com",
    "google-analytics.com",
    "gstatic.com",
    "googleapis.com",
    "google.com",
    "jsdelivr.net",
    "cdnjs.cloudflare.com",
    "unpkg.com",
    "code.jquery.com",
    "wp.com",
    "wordpress.com",
    "facebook.net",
    "twitter.com",
    "youtube.com",
    "vimeo.com",
    "stripe.com",
    "paypal.com",
    "cloudflare.com",
    "cloudflareinsights.com",
    "hotjar.com",
    "hs-scripts.com",
    "chimpstatic.com",
    "gravatar.com",
    "bootstrapcdn.com",
    "fontawesome.com",
    "typekit.net",
    "intercom.io",
    "crisp.chat",
    "tawk.to",
    "calendly.com",
    "recaptcha.net",
    "jquery.com",
    "shopify.com",
    "squarespace.com",
    "wixstatic.com",
    "elfsight.com",
    "tiktok.com",
    "instagram.com",
    "linkedin.com",
    "pinterest.com",
    "clarity.ms",
    "bing.com",
];

/// AI crawlers whose robots.txt rules the report mentions.
pub const AI_CRAWLERS: &[&str] = &[
    "GPTBot",
    "ChatGPT-User",
    "OAI-SearchBot",
    "ClaudeBot",
    "Claude-User",
    "anthropic-ai",
    "Google-Extended",
    "PerplexityBot",
    "CCBot",
    "Applebot-Extended",
    "Bytespider",
];

/// Above this, a page is slow (milliseconds in the sealed room).
pub const SLOW_MS: u64 = 3000;
/// Above this, a page's HTML is heavy.
pub const HEAVY_BYTES: u64 = 1024 * 1024;

/// Whether the egress proxy refused by policy (off the allowlist, a private address, a
/// smuggled request) rather than because the network failed it (DNS, connect, timeout). Both
/// come back to the room as a refusal; only the reason tells them apart.
pub fn policy_refusal(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    !(r.starts_with("dns for")
        || r.starts_with("connect to")
        || r.ends_with("has no addresses")
        || r.contains("no address to connect to"))
}

/// Whether page `p` was refused by policy: its error says refused and the job that fetched it
/// recorded a policy refusal.
fn refused_by_policy(p: &Page, crawl: &Crawl) -> bool {
    p.error.as_deref().is_some_and(|e| e.contains("refused"))
        && crawl
            .refusals
            .iter()
            .any(|(r, receipt)| receipt == &p.receipt && policy_refusal(&r.reason))
}

/// The host in a room's "refused by the egress proxy (host…)" error.
fn refused_host(error: &str) -> Option<String> {
    let inside = error.split_once("refused by the egress proxy (")?.1;
    let host: String = inside
        .chars()
        .take_while(|c| !matches!(c, ':' | ')' | ' '))
        .collect();
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The path and query of `url`, for labels.
pub fn path_of(url: &str) -> String {
    Url::parse(url)
        .map(|u| {
            let mut p = u.path().to_owned();
            if let Some(q) = u.query() {
                p.push('?');
                p.push_str(q);
            }
            p
        })
        .unwrap_or_else(|_| url.to_owned())
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Whether `text` contains `word` as a whole word or phrase (letters and digits on either side
/// break it).
fn has_word(text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(p) = text[from..].find(word) {
        let at = from + p;
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        if edge(before) && edge(after) {
            return true;
        }
        from = at + word.len();
    }
    false
}

/// The spam words in `text`.
pub fn spam_words(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    SPAM_WORDS
        .iter()
        .copied()
        .filter(|w| has_word(&lower, w))
        .collect()
}

/// Whether `host` is one of the site's.
fn on_site(host: &str, hosts: &[String]) -> bool {
    hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
}

/// The absolute URL `href` points to from `page`.
fn resolve(page: &str, href: &str) -> Option<Url> {
    Url::parse(page).ok()?.join(href).ok()
}

/// Loopback, private, link-local and metadata addresses, as a URL's host names them.
pub fn private_host(host: &str) -> bool {
    let h = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if h == "localhost"
        || h.ends_with(".localhost")
        || h == "::1"
        || h.starts_with("fe80:")
        || h.starts_with("fc")
        || h.starts_with("fd") && h.contains(':')
    {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        return ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified();
    }
    false
}

/// What an obfuscated script looks like: the markers found, or `None`.
pub fn obfuscation_markers(script: &str) -> Option<Vec<String>> {
    let lower = script.to_ascii_lowercase();
    let mut markers = Vec::new();
    let evals = lower.contains("eval(")
        || lower.contains("'\\x65\\x76\\x61\\x6c'")
        || lower.contains("[\"eval\"]")
        || lower.contains("['eval']")
        || lower.contains("\\x65\\x76\\x61\\x6c");
    let decodes = lower.contains("atob(")
        || lower.contains("\\x61\\x74\\x6f\\x62")
        || lower.contains("unescape(")
        || lower.contains("fromcharcode")
        || lower.contains("decodeuricomponent(");
    if evals && decodes {
        markers.push("runs code it decodes at load time (eval over decoded text)".to_owned());
    }
    if lower.contains("document.write(unescape") {
        markers
            .push("writes decoded markup into the page (document.write(unescape(…)))".to_owned());
    }
    let hex_ids = lower
        .match_indices("_0x")
        .filter(|(i, _)| {
            lower[i + 3..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .count()
                >= 2
        })
        .count();
    if hex_ids >= 2 {
        markers.push("uses machine-mangled names (_0x…)".to_owned());
    }
    let escapes = lower.matches("\\x").count();
    if escapes >= 8 {
        markers.push(format!("hides words as {escapes} hex escapes (\\x…)"));
    }
    if longest_base64(script).is_some_and(|b| b.len() >= 120) {
        markers.push("carries a long encoded string".to_owned());
    }
    // A long encoded string alone is common (fonts, images); require two signs.
    (markers.len() >= 2 || (markers.len() == 1 && evals && decodes)).then_some(markers)
}

/// The longest run of base64 characters in quotes.
fn longest_base64(script: &str) -> Option<&str> {
    let mut best: Option<&str> = None;
    for q in ['\'', '"'] {
        for (k, part) in script.split(q).enumerate() {
            if k % 2 == 1
                && part.len() >= 40
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
                && best.is_none_or(|b| part.len() > b.len())
            {
                best = Some(part);
            }
        }
    }
    best
}

/// Decodes standard base64, for showing what a disguised script carries (never to run it).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let clean: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len().max(1)]);
    }
    Some(out)
}

/// What a disguised script's encoded string says, if it decodes to readable text.
fn decoded_preview(script: &str) -> Option<String> {
    let b = longest_base64(script)?;
    let bytes = base64_decode(b)?;
    let text = String::from_utf8(bytes).ok()?;
    let printable = text
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .count();
    (printable * 10 >= text.chars().count() * 9).then(|| clip(&text, 240))
}

fn jquery_version(src: &str) -> Option<(u32, u32, u32)> {
    let lower = src.to_ascii_lowercase();
    if !lower.contains("jquery")
        || lower.contains("jquery-migrate")
        || lower.contains("jquery-ui")
        || lower.contains("jquery.ui")
    {
        return None;
    }
    let candidates = [
        lower.split("ver=").nth(1),
        lower.split("jquery-").nth(1),
        lower.split("/jquery/").nth(1),
    ];
    for c in candidates.into_iter().flatten() {
        let v: String = c
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
            .collect();
        let mut parts = v.split('.').map(|p| p.parse::<u32>().ok());
        if let (Some(Some(a)), Some(Some(b))) = (parts.next(), parts.next()) {
            let c = parts.next().flatten().unwrap_or(0);
            return Some((a, b, c));
        }
    }
    None
}

/// Runs every detector over the crawl.
pub fn detect(crawl: &Crawl) -> Vec<Found> {
    let mut out = Vec::new();
    security(crawl, &mut out);
    speed(crawl, &mut out);
    links(crawl, &mut out);
    ai_readiness(crawl, &mut out);
    out
}

fn proof(page: &Page, label: String, excerpt: Option<String>) -> Proof {
    Proof {
        label,
        excerpt,
        page: page.digest,
        receipt: page.receipt,
    }
}

fn where_(pages: &BTreeSet<String>) -> String {
    if pages.len() == 1 {
        pages.iter().next().cloned().unwrap_or_default()
    } else {
        format!("{} pages", pages.len())
    }
}

fn security(crawl: &Crawl, out: &mut Vec<Found>) {
    let read: Vec<&Page> = crawl.pages.iter().filter(|p| p.is_read()).collect();

    // Spam links, hidden or not, and hidden links to other sites.
    let (mut spam, mut spam_pages, mut any_hidden_spam) = (Vec::new(), BTreeSet::new(), false);
    let (mut hidden, mut hidden_pages) = (Vec::new(), BTreeSet::new());
    for p in &read {
        let Some(f) = &p.facts else { continue };
        let base = p.final_url.as_deref().unwrap_or(&p.url);
        let mut spam_here = Vec::new();
        let mut hidden_here = Vec::new();
        let mut how_hidden = BTreeSet::new();
        for a in &f.anchors {
            let Some(target) = resolve(base, &a.href) else {
                continue;
            };
            let external = target.host_str().is_some_and(|h| !on_site(h, &crawl.hosts));
            if !external || !matches!(target.scheme(), "http" | "https") {
                continue;
            }
            let words = spam_words(&format!("{} {}", a.text, target));
            let line = format!("“{}” → {}", clip(&a.text, 60), target);
            if !words.is_empty() {
                if let Some(how) = &a.hidden {
                    how_hidden.insert(how.clone());
                    any_hidden_spam = true;
                }
                spam_here.push(line);
            } else if let Some(how) = &a.hidden {
                how_hidden.insert(how.clone());
                hidden_here.push(line);
            }
        }
        let how = how_hidden.into_iter().collect::<Vec<_>>().join(", ");
        if !spam_here.is_empty() {
            spam_pages.insert(p.url.clone());
            let label = if how.is_empty() {
                format!(
                    "On {}: {}",
                    path_of(&p.url),
                    plural(spam_here.len(), "spam link", "spam links")
                )
            } else {
                format!(
                    "On {}: {}, hidden by {how}",
                    path_of(&p.url),
                    plural(spam_here.len(), "spam link", "spam links")
                )
            };
            spam.push(proof(p, label, Some(spam_here.join("\n"))));
        }
        if !hidden_here.is_empty() {
            hidden_pages.insert(p.url.clone());
            hidden.push(proof(
                p,
                format!(
                    "On {}: {} hidden by {how}",
                    path_of(&p.url),
                    plural(hidden_here.len(), "link", "links")
                ),
                Some(hidden_here.join("\n")),
            ));
        }
    }
    if !spam.is_empty() {
        let n = spam_pages.len();
        out.push(Found {
            detector: "spam-links".into(),
            severity: Severity::Urgent,
            category: Category::Security,
            title: if any_hidden_spam {
                format!("Hidden spam links on {}", plural(n, "page", "pages"))
            } else {
                format!("Spam links on {}", plural(n, "page", "pages"))
            },
            plain: format!(
                "We found links to spam sites (gambling, pills, loans and the like) on {}{}. \
                 Attackers often break into sites to plant paid links like these, and search \
                 engines can rank the whole site lower for them. The Move leaves them behind.",
                plural(n, "of your pages", "of your pages"),
                if any_hidden_spam {
                    ", hidden where visitors can't see them"
                } else {
                    ""
                }
            ),
            location: where_(&spam_pages),
            evidence: spam,
        });
    }
    if !hidden.is_empty() {
        out.push(Found {
            detector: "hidden-links".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: format!(
                "Hidden links to other sites on {}",
                plural(hidden_pages.len(), "page", "pages")
            ),
            plain: "Some links to other sites are hidden from visitors. Sometimes that's a menu \
                    waiting to open; on a hacked site it's paid link spam. If you don't recognise \
                    these sites, treat it as urgent."
                .into(),
            location: where_(&hidden_pages),
            evidence: hidden,
        });
    }

    // Hidden text that tries to instruct AI agents (prompt injection).
    let (mut inject, mut inject_pages) = (Vec::new(), BTreeSet::new());
    for p in &read {
        let Some(f) = &p.facts else { continue };
        for h in &f.hidden_texts {
            let lower = h.text.to_lowercase();
            let to_ai = [
                "ai agent",
                "ai assistant",
                "assistant",
                "llm",
                "language model",
                "chatgpt",
                "claude",
                "gpt",
                "ai model",
                "agents",
            ]
            .iter()
            .any(|w| has_word(&lower, w));
            let orders = [
                "ignore",
                "disregard",
                "instruction",
                "password",
                "email",
                "send",
                "add this",
                "execute",
                "run this",
                "api key",
                "credentials",
                "system prompt",
            ]
            .iter()
            .any(|w| lower.contains(w));
            if to_ai && orders {
                inject_pages.insert(p.url.clone());
                inject.push(proof(
                    p,
                    format!("On {}: text hidden by {}", path_of(&p.url), h.how),
                    Some(clip(&h.text, 300)),
                ));
            }
        }
    }
    if !inject.is_empty() {
        out.push(Found {
            detector: "hidden-instructions".into(),
            severity: Severity::Urgent,
            category: Category::Security,
            title: "Hidden instructions aimed at AI assistants".into(),
            plain: "Text hidden from visitors tries to give orders to AI assistants that read \
                    your site. This attack is called prompt injection. Our WebAgents are built to \
                    treat page text as information, not orders, and the Move leaves this text \
                    behind."
                .into(),
            location: where_(&inject_pages),
            evidence: inject,
        });
    }

    // Disguised scripts, grouped by content so one script on every page is one finding.
    // (markers, script, in the footer, evidence, pages)
    type Seen = (Vec<String>, String, bool, Vec<Proof>, BTreeSet<String>);
    let mut scripts: BTreeMap<Digest, Seen> = BTreeMap::new();
    for p in &read {
        let Some(f) = &p.facts else { continue };
        for s in f.scripts.iter().filter(|s| s.src.is_none()) {
            let Some(markers) = obfuscation_markers(&s.inline) else {
                continue;
            };
            let key = Digest::of(s.inline.split_whitespace().collect::<String>().as_bytes());
            let e = scripts.entry(key).or_insert_with(|| {
                (
                    markers,
                    s.inline.clone(),
                    false,
                    Vec::new(),
                    BTreeSet::new(),
                )
            });
            e.2 |= s.in_footer;
            e.4.insert(p.url.clone());
            if e.3.len() < 5 {
                let at = if s.in_footer {
                    "in the footer"
                } else if s.in_head {
                    "in the page head"
                } else {
                    "in the page"
                };
                e.3.push(proof(
                    p,
                    format!("On {}, {at}", path_of(&p.url)),
                    Some(clip(&s.inline, 400)),
                ));
            }
        }
    }
    let html_pages = read.len();
    for (_, (markers, script, footer, mut evidence, pages)) in scripts {
        if let Some(text) = decoded_preview(&script)
            && let Some(first) = evidence.first().cloned()
        {
            evidence.push(Proof {
                label: "What its encoded text says, decoded by us (we didn't run it)".into(),
                excerpt: Some(text),
                ..first
            });
        }
        let everywhere = pages.len() == html_pages && html_pages > 1;
        out.push(Found {
            detector: "obfuscated-script".into(),
            severity: Severity::Urgent,
            category: Category::Security,
            title: format!(
                "A disguised script {}{}",
                if footer {
                    "in your footer"
                } else {
                    "in your pages"
                },
                if everywhere {
                    ", on every page".to_owned()
                } else {
                    format!(", on {}", plural(pages.len(), "page", "pages"))
                }
            ),
            plain: format!(
                "A script hides what it does: it {}. Ordinary scripts rarely hide like this. \
                 Malware that attackers add to a site usually does, to mine cryptocurrency, send \
                 visitors elsewhere or copy card details. The Move leaves it behind.",
                markers.join("; it ")
            ),
            location: where_(&pages),
            evidence,
        });
    }

    // Scripts from unknown hosts.
    let mut foreign: BTreeMap<String, (bool, Vec<Proof>, BTreeSet<String>)> = BTreeMap::new();
    for p in &read {
        let Some(f) = &p.facts else { continue };
        let base = p.final_url.as_deref().unwrap_or(&p.url);
        for s in &f.scripts {
            let Some(src) = &s.src else { continue };
            let Some(u) = resolve(base, src) else {
                continue;
            };
            let Some(host) = u.host_str().map(str::to_ascii_lowercase) else {
                continue;
            };
            if on_site(&host, &crawl.hosts) || private_host(&host) {
                continue;
            }
            if KNOWN_SCRIPT_HOSTS
                .iter()
                .any(|k| host == *k || host.ends_with(&format!(".{k}")))
            {
                continue;
            }
            let e = foreign.entry(host).or_default();
            e.0 |= s.in_footer;
            e.2.insert(p.url.clone());
            if e.1.len() < 5 {
                let at = if s.in_footer {
                    "in the footer"
                } else if s.in_head {
                    "in the page head"
                } else {
                    "in the page"
                };
                e.1.push(proof(
                    p,
                    format!("On {}, {at}", path_of(&p.url)),
                    Some(s.tag.clone()),
                ));
            }
        }
    }
    for (host, (footer, evidence, pages)) in foreign {
        out.push(Found {
            detector: "foreign-script".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: format!(
                "A script loaded from {host}{}",
                if footer { ", in your footer" } else { "" }
            ),
            plain: format!(
                "Your pages run a script from {host}, which isn't on our list of common \
                 providers. If you didn't add it, treat it as urgent: hacked sites load malware \
                 this way. The rebuilt site runs only scripts you approve."
            ),
            location: where_(&pages),
            evidence,
        });
    }

    // Links and resources pointing into private networks.
    let (mut private, mut private_pages) = (Vec::new(), BTreeSet::new());
    for p in &read {
        let Some(f) = &p.facts else { continue };
        let base = p.final_url.as_deref().unwrap_or(&p.url);
        let mut seen = Vec::new();
        let targets = f
            .anchors
            .iter()
            .map(|a| a.href.as_str())
            .chain(f.resources.iter().map(String::as_str))
            .chain(f.scripts.iter().filter_map(|s| s.src.as_deref()))
            .chain(f.iframes.iter().map(|i| i.src.as_str()));
        for t in targets {
            if let Some(u) = resolve(base, t)
                && u.host_str().is_some_and(private_host)
                && !seen.contains(&u.to_string())
            {
                seen.push(u.to_string());
            }
        }
        if !seen.is_empty() {
            private_pages.insert(p.url.clone());
            private.push(proof(
                p,
                format!("On {}", path_of(&p.url)),
                Some(seen.join("\n")),
            ));
        }
    }
    if !private.is_empty() {
        out.push(Found {
            detector: "private-address".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: "Links into private network addresses".into(),
            plain:
                "Pages point at addresses inside a private network, such as 127.0.0.1. A public \
                    site has no reason to; it's a way to trick tools that read the site into \
                    reaching places they shouldn't. Our sealed room can't reach them."
                    .into(),
            location: where_(&private_pages),
            evidence: private,
        });
    }

    // Pages that sent the room somewhere the egress proxy refused.
    let mut refused = Vec::new();
    let mut refused_pages = BTreeSet::new();
    for p in crawl.pages.iter().filter(|p| refused_by_policy(p, crawl)) {
        refused_pages.insert(p.url.clone());
        let linked = if p.referrers.is_empty() {
            String::new()
        } else {
            format!(
                ", linked from {}",
                p.referrers
                    .iter()
                    .map(|r| path_of(r))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        refused.push(proof(
            p,
            format!("{}{linked}", path_of(&p.url)),
            p.error.clone(),
        ));
    }
    if !refused.is_empty() {
        // Where each failed page was going, when its error names it ("refused by the egress
        // proxy (169.254.169.254:80: …)"). A browser room's jobs also hold refusals of
        // subresources (fonts, trackers), which are not where a link led.
        let named: BTreeSet<String> = crawl
            .pages
            .iter()
            .filter(|p| refused_by_policy(p, crawl))
            .filter_map(|p| refused_host(p.error.as_deref()?))
            .filter(|h| !on_site(h, &crawl.hosts))
            .collect();
        let policy: Vec<&(Refusal, Option<Digest>)> = crawl
            .refusals
            .iter()
            .filter(|(r, _)| policy_refusal(&r.reason))
            .filter(|(r, _)| named.is_empty() || named.contains(&r.host.to_ascii_lowercase()))
            .collect();
        let hosts: BTreeSet<String> = if named.is_empty() {
            policy.iter().map(|(r, _)| r.host.clone()).collect()
        } else {
            named
        };
        if let Some(first) = refused.first().cloned() {
            for (r, receipt) in policy.into_iter().take(5) {
                refused.push(Proof {
                    label: format!(
                        "The sealed room refused {}:{} ({})",
                        r.host, r.port, r.reason
                    ),
                    excerpt: None,
                    page: None,
                    receipt: receipt.or(first.receipt),
                });
            }
        }
        out.push(Found {
            detector: "redirect-refused".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: format!(
                "{} sent our sealed room somewhere it isn't allowed to go",
                plural(refused_pages.len(), "page", "pages")
            ),
            plain: format!(
                "Following {} led off your site{}. Our sealed room refuses every address that \
                 isn't your site, so it stopped there, and the refusal is on the receipt. If you \
                 know that site, this is likely harmless. A redirect to an internal address, one \
                 only a server itself should reach, is a known way to attack tools that read \
                 websites.",
                if refused_pages.len() == 1 {
                    "a link"
                } else {
                    "these links"
                },
                if hosts.is_empty() {
                    String::new()
                } else {
                    format!(" (to {})", hosts.into_iter().collect::<Vec<_>>().join(", "))
                }
            ),
            location: where_(&refused_pages),
            evidence: refused,
        });
    }

    // Hidden iframes.
    let (mut frames, mut frame_pages) = (Vec::new(), BTreeSet::new());
    for p in &read {
        let Some(f) = &p.facts else { continue };
        for i in f.iframes.iter().filter(|i| i.hidden.is_some()) {
            frame_pages.insert(p.url.clone());
            frames.push(proof(
                p,
                format!(
                    "On {}: a frame (iframe) hidden by {}",
                    path_of(&p.url),
                    i.hidden.clone().unwrap_or_default()
                ),
                Some(i.tag.clone()),
            ));
        }
    }
    if !frames.is_empty() {
        out.push(Found {
            detector: "hidden-iframe".into(),
            severity: Severity::Urgent,
            category: Category::Security,
            title: "A hidden frame loading another page".into(),
            plain: "An invisible frame loads another page that your visitors can't see. Attackers \
                    use hidden frames to load ads, cryptocurrency miners or attacks on visitors' \
                    browsers. The Move leaves it behind."
                .into(),
            location: where_(&frame_pages),
            evidence: frames,
        });
    }

    // Program files in uploads, and server code showing in pages (adk-site's A-I1 rules).
    let (mut uploads, mut upload_pages) = (Vec::new(), BTreeSet::new());
    let mut leaks = Vec::new();
    let mut leak_pages = BTreeSet::new();
    for p in &read {
        let Some(f) = &p.facts else { continue };
        let base = p.final_url.as_deref().unwrap_or(&p.url);
        let mut seen = BTreeSet::new();
        for t in f
            .anchors
            .iter()
            .map(|a| a.href.as_str())
            .chain(f.resources.iter().map(String::as_str))
        {
            let Some(u) = resolve(base, t) else { continue };
            if !u.host_str().is_some_and(|h| on_site(h, &crawl.hosts))
                || !u.path().contains("/uploads/")
            {
                continue;
            }
            if !adk_site::scan::scan_file(u.path().trim_start_matches('/'), b"").is_empty() {
                seen.insert(u.path().to_owned());
            }
        }
        if !seen.is_empty() {
            upload_pages.insert(p.url.clone());
            uploads.push(proof(
                p,
                format!("Linked from {}", path_of(&p.url)),
                Some(seen.into_iter().collect::<Vec<_>>().join("\n")),
            ));
        }
        let code = adk_site::scan::scan_file("page.html", &p.body);
        if !code.is_empty() {
            leak_pages.insert(p.url.clone());
            leaks.push(proof(
                p,
                format!("On {}", path_of(&p.url)),
                Some(
                    code.iter()
                        .map(|c| format!("{:?}", c.kind))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ));
        }
    }
    if !uploads.is_empty() {
        out.push(Found {
            detector: "executable-upload".into(),
            severity: Severity::Urgent,
            category: Category::Security,
            title: "A program file in your uploads folder".into(),
            plain: "Your pages link to a file in your uploads folder whose name marks it as a \
                    program (such as .php). That folder should hold only images and documents. \
                    Attackers hide programs there to keep a way back into a site. The Move leaves \
                    it behind."
                .into(),
            location: where_(&upload_pages),
            evidence: uploads,
        });
    }
    if !leaks.is_empty() {
        out.push(Found {
            detector: "server-code".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: "Server code showing in your pages".into(),
            plain: "Pages contain server instructions (PHP or server-side includes) that should \
                    never reach a visitor. It can leak how the site works."
                .into(),
            location: where_(&leak_pages),
            evidence: leaks,
        });
    }

    // Components that announce themselves.
    if let Some(p) = read.first()
        && let Some(f) = &p.facts
        && let Some(generator) = f.meta("generator").filter(|g| !g.trim().is_empty())
    {
        {
            out.push(Found {
                detector: "version-announced".into(),
                severity: Severity::Info,
                category: Category::Security,
                title: format!("Your site announces what it runs: {}", clip(generator, 60)),
                plain: "Your home page says which software and version it runs. That tells \
                        attackers which known weaknesses to try. The rebuilt site serves \
                        ready-made pages, with no such software behind them to attack."
                    .into(),
                location: p.url.clone(),
                evidence: vec![proof(
                    p,
                    format!("On {}", path_of(&p.url)),
                    Some(format!("<meta name=\"generator\" content=\"{generator}\">")),
                )],
            });
        }
    }
    let mut old_jq: BTreeMap<String, (Vec<Proof>, BTreeSet<String>)> = BTreeMap::new();
    for p in &read {
        let Some(f) = &p.facts else { continue };
        for s in &f.scripts {
            let Some(src) = &s.src else { continue };
            if let Some((a, b, c)) = jquery_version(src)
                && (a, b) < (3, 5)
            {
                let e = old_jq.entry(format!("{a}.{b}.{c}")).or_default();
                e.1.insert(p.url.clone());
                if e.0.is_empty() {
                    e.0.push(proof(
                        p,
                        format!("On {}", path_of(&p.url)),
                        Some(s.tag.clone()),
                    ));
                }
            }
        }
    }
    for (v, (evidence, pages)) in old_jq {
        out.push(Found {
            detector: "outdated-component".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: format!("An outdated jQuery ({v}) with known security holes"),
            plain: "jQuery is a common script library. Versions before 3.5.0 have published flaws \
                    (cross-site scripting) that can let an attacker run their own script on your \
                    pages. The rebuilt site doesn't need it."
                .into(),
            location: where_(&pages),
            evidence,
        });
    }

    if let Some(p) = crawl.pages.iter().find(|p| p.is_read())
        && p.final_url
            .as_deref()
            .unwrap_or(&p.url)
            .starts_with("http://")
    {
        out.push(Found {
            detector: "no-https".into(),
            severity: Severity::Warn,
            category: Category::Security,
            title: "Your site doesn't use a secure connection".into(),
            plain:
                "Your home page loads over plain http, so anything typed into it can be read on \
                    the way and browsers mark it \"Not secure\". The rebuilt site is https only."
                    .into(),
            location: p.url.clone(),
            evidence: vec![proof(
                p,
                format!(
                    "{} ended at {}",
                    p.url,
                    p.final_url.clone().unwrap_or_default()
                ),
                None,
            )],
        });
    }
}

fn speed(crawl: &Crawl, out: &mut Vec<Found>) {
    let read: Vec<&Page> = crawl.pages.iter().filter(|p| p.is_read()).collect();
    let slow: Vec<&&Page> = read
        .iter()
        .filter(|p| p.ms.is_some_and(|ms| ms > SLOW_MS))
        .collect();
    if !slow.is_empty() {
        let pages = slow.iter().map(|p| p.url.clone()).collect();
        out.push(Found {
            detector: "slow-pages".into(),
            severity: Severity::Warn,
            category: Category::Speed,
            title: format!(
                "{} took more than 3 seconds to load",
                plural(slow.len(), "page", "pages")
            ),
            plain:
                "Visitors leave slow pages, and search engines rank them lower. These times are \
                    measured from our sealed room, for the page's HTML alone."
                    .into(),
            location: where_(&pages),
            evidence: slow
                .iter()
                .take(10)
                .map(|p| {
                    proof(
                        p,
                        format!(
                            "{}: {:.1} s",
                            path_of(&p.url),
                            p.ms.unwrap_or_default() as f64 / 1000.0
                        ),
                        None,
                    )
                })
                .collect(),
        });
    }
    let heavy: Vec<&&Page> = read
        .iter()
        .filter(|p| p.bytes > HEAVY_BYTES || p.truncated)
        .collect();
    if !heavy.is_empty() {
        let pages = heavy.iter().map(|p| p.url.clone()).collect();
        out.push(Found {
            detector: "heavy-pages".into(),
            severity: Severity::Warn,
            category: Category::Speed,
            title: format!(
                "{} with very large HTML",
                plural(heavy.len(), "page", "pages")
            ),
            plain: "These pages send more than 1 MB of HTML before any image or script, which is \
                    slow on a phone connection."
                .into(),
            location: where_(&pages),
            evidence: heavy
                .iter()
                .take(10)
                .map(|p| {
                    proof(
                        p,
                        format!(
                            "{}: {}{}",
                            path_of(&p.url),
                            bytes_words(p.bytes),
                            if p.truncated {
                                " (we stopped reading)"
                            } else {
                                ""
                            }
                        ),
                        None,
                    )
                })
                .collect(),
        });
    }
    let busy: Vec<&&Page> = read
        .iter()
        .filter(|p| {
            p.facts
                .as_ref()
                .is_some_and(|f| f.scripts.iter().filter(|s| s.src.is_some()).count() > 25)
        })
        .collect();
    if !busy.is_empty() {
        let pages = busy.iter().map(|p| p.url.clone()).collect();
        out.push(Found {
            detector: "many-scripts".into(),
            severity: Severity::Info,
            category: Category::Speed,
            title: format!(
                "{} more than 25 scripts",
                plural(busy.len(), "page loads", "pages load")
            ),
            plain:
                "Each script is another download before the page settles. The rebuilt site keeps \
                    only the ones it needs."
                    .into(),
            location: where_(&pages),
            evidence: busy
                .iter()
                .take(5)
                .map(|p| {
                    let n = p
                        .facts
                        .as_ref()
                        .map_or(0, |f| f.scripts.iter().filter(|s| s.src.is_some()).count());
                    proof(p, format!("{}: {n} scripts", path_of(&p.url)), None)
                })
                .collect(),
        });
    }
}

fn links(crawl: &Crawl, out: &mut Vec<Found>) {
    let broken: Vec<&Page> = crawl
        .pages
        .iter()
        .filter(|p| p.n > 1 && p.status.is_some_and(|s| s >= 400))
        .collect();
    if !broken.is_empty() {
        let pages = broken
            .iter()
            .flat_map(|p| p.referrers.iter().cloned())
            .collect::<BTreeSet<_>>();
        out.push(Found {
            detector: "broken-links".into(),
            severity: Severity::Warn,
            category: Category::Links,
            title: plural(broken.len(), "broken link", "broken links"),
            plain:
                "Links on your site lead to pages that answer with an error. Visitors hit a dead \
                    end, and search engines notice. The Move maps old addresses to new ones."
                    .into(),
            location: if pages.is_empty() {
                "your site".into()
            } else {
                where_(&pages)
            },
            evidence: broken
                .iter()
                .take(15)
                .map(|p| {
                    let from = if p.referrers.is_empty() {
                        String::new()
                    } else {
                        format!(
                            ", linked from {}",
                            p.referrers
                                .iter()
                                .take(3)
                                .map(|r| path_of(r))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    proof(
                        p,
                        format!(
                            "{} answers HTTP {}{from}",
                            path_of(&p.url),
                            p.status.unwrap_or_default()
                        ),
                        None,
                    )
                })
                .collect(),
        });
    }
    let silent: Vec<&Page> = crawl
        .pages
        .iter()
        .filter(|p| p.n > 1 && p.status.is_none() && !refused_by_policy(p, crawl))
        .collect();
    if !silent.is_empty() {
        out.push(Found {
            detector: "unanswered-links".into(),
            severity: Severity::Warn,
            category: Category::Links,
            title: format!("{} didn't answer", plural(silent.len(), "page", "pages")),
            plain:
                "Some pages on your site didn't answer our sealed room at all. They may be down \
                    or very slow."
                    .into(),
            location: where_(&silent.iter().map(|p| p.url.clone()).collect()),
            evidence: silent
                .iter()
                .take(10)
                .map(|p| {
                    proof(
                        p,
                        format!(
                            "{}: {}",
                            path_of(&p.url),
                            p.error.clone().unwrap_or_default()
                        ),
                        None,
                    )
                })
                .collect(),
        });
    }
}

fn ai_readiness(crawl: &Crawl, out: &mut Vec<Found>) {
    let read: Vec<&Page> = crawl.pages.iter().filter(|p| p.is_read()).collect();
    let Some(home) = read.first() else { return };
    if read
        .iter()
        .all(|p| p.facts.as_ref().is_some_and(|f| f.json_ld == 0))
    {
        out.push(Found {
            detector: "no-structured-data".into(),
            severity: Severity::Warn,
            category: Category::AiReadiness,
            title: "AI assistants have to guess your business details".into(),
            plain: format!(
                "We found no structured data on the {} we read. Structured data (schema.org) \
                 states your opening hours, address, prices and products in a form machines can \
                 read; without it, assistants guess them from the text. The rebuilt site adds it, \
                 plus a way for AI agents to ask questions and place orders.",
                plural(read.len(), "page", "pages")
            ),
            location: home.url.clone(),
            evidence: vec![proof(
                home,
                format!(
                    "No <script type=\"application/ld+json\"> on {}",
                    plural(read.len(), "page", "pages")
                ),
                None,
            )],
        });
    }
    if crawl.llms_txt == Some(false) {
        out.push(Found {
            detector: "no-llms-txt".into(),
            severity: Severity::Info,
            category: Category::AiReadiness,
            title: "No llms.txt guide for AI assistants".into(),
            plain: "Some assistants look for /llms.txt, a short guide to a site written for them. \
                    Yours has none; the rebuilt site writes one from your content."
                .into(),
            location: "/llms.txt".into(),
            evidence: vec![proof(home, "/llms.txt was not found".into(), None)],
        });
    }
    if !crawl.ai_blocked.is_empty() {
        out.push(Found {
            detector: "ai-crawlers-blocked".into(),
            severity: Severity::Info,
            category: Category::AiReadiness,
            title: format!("Your robots.txt turns away {}", crawl.ai_blocked.join(", ")),
            plain: "Your robots.txt file tells crawlers what they may read, and it asks these AI \
                    companies' crawlers to stay away. That's your choice. Assistants that can't \
                    read your pages can't answer questions from them or recommend them."
                .into(),
            location: "/robots.txt".into(),
            evidence: vec![proof(
                home,
                format!("robots.txt disallows {}", crawl.ai_blocked.join(", ")),
                None,
            )],
        });
    }
    let no_desc: Vec<&&Page> = read
        .iter()
        .filter(|p| {
            p.facts
                .as_ref()
                .is_some_and(|f| f.meta("description").is_none_or(|d| d.trim().is_empty()))
        })
        .collect();
    if !no_desc.is_empty() {
        out.push(Found {
            detector: "missing-descriptions".into(),
            severity: Severity::Info,
            category: Category::AiReadiness,
            title: format!(
                "{} of {} {} no description",
                no_desc.len(),
                plural(read.len(), "page", "pages"),
                if no_desc.len() == 1 { "has" } else { "have" }
            ),
            plain: "A page's description is the line search engines and assistants show for it. \
                    Without one they make their own."
                .into(),
            location: where_(&no_desc.iter().map(|p| p.url.clone()).collect()),
            evidence: no_desc
                .iter()
                .take(10)
                .map(|p| proof(p, path_of(&p.url), None))
                .collect(),
        });
    }
    let no_title: Vec<&&Page> = read
        .iter()
        .filter(|p| {
            p.facts
                .as_ref()
                .is_some_and(|f| f.title.as_deref().is_none_or(|t| t.trim().is_empty()))
        })
        .collect();
    if !no_title.is_empty() {
        out.push(Found {
            detector: "missing-titles".into(),
            severity: Severity::Warn,
            category: Category::AiReadiness,
            title: format!(
                "{} without a title",
                plural(no_title.len(), "page", "pages")
            ),
            plain: "Without a title, search results and assistants can't say what a page is."
                .into(),
            location: where_(&no_title.iter().map(|p| p.url.clone()).collect()),
            evidence: no_title
                .iter()
                .take(10)
                .map(|p| proof(p, path_of(&p.url), None))
                .collect(),
        });
    }
    let images: usize = read
        .iter()
        .filter_map(|p| p.facts.as_ref())
        .map(|f| f.images)
        .sum();
    let no_alt: usize = read
        .iter()
        .filter_map(|p| p.facts.as_ref())
        .map(|f| f.images_without_alt)
        .sum();
    if no_alt > 0 {
        out.push(Found {
            detector: "images-without-alt".into(),
            severity: Severity::Info,
            category: Category::AiReadiness,
            title: format!(
                "{no_alt} of {} {} no text description (alt text)",
                plural(images, "image", "images"),
                if no_alt == 1 { "has" } else { "have" }
            ),
            plain: "Screen readers and assistants can't tell what these images show. The rebuilt \
                    site asks you to describe the ones that matter."
                .into(),
            location: home.url.clone(),
            evidence: read
                .iter()
                .filter(|p| p.facts.as_ref().is_some_and(|f| f.images_without_alt > 0))
                .take(10)
                .map(|p| {
                    proof(
                        p,
                        format!(
                            "{}: {}",
                            path_of(&p.url),
                            plural(
                                p.facts.as_ref().map_or(0, |f| f.images_without_alt),
                                "image",
                                "images"
                            )
                        ),
                        None,
                    )
                })
                .collect(),
        });
    }
    let script_only: Vec<&&Page> = read
        .iter()
        .filter(|p| {
            p.facts
                .as_ref()
                .is_some_and(|f| f.text_chars < 200 && f.scripts.len() >= 5)
        })
        .collect();
    if !script_only.is_empty() {
        out.push(Found {
            detector: "script-only-pages".into(),
            severity: Severity::Warn,
            category: Category::AiReadiness,
            title: format!(
                "{} almost nothing without JavaScript",
                plural(script_only.len(), "page shows", "pages show")
            ),
            plain: "Many AI agents read pages without running scripts. To them these pages are \
                    nearly empty."
                .into(),
            location: where_(&script_only.iter().map(|p| p.url.clone()).collect()),
            evidence: script_only
                .iter()
                .take(10)
                .map(|p| {
                    proof(
                        p,
                        format!(
                            "{}: {} characters of text",
                            path_of(&p.url),
                            p.facts.as_ref().map_or(0, |f| f.text_chars)
                        ),
                        None,
                    )
                })
                .collect(),
        });
    }
}

/// `1.4 MB`, `820 KB`, `312 bytes`.
pub fn bytes_words(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{} KB", n / 1_000)
    } else {
        format!("{n} bytes")
    }
}
