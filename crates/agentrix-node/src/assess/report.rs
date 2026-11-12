//! The health report (R1.5–R1.7): stored as a [`ReportDoc`], shown through the console's
//! `view::AssessView`.

use agentrix_console::view;
use agentrix_types::ui::FrameEvent;
use agentrix_types::{Category, Digest, FindingId, Severity};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::detect::{Found, bytes_words};
use super::store::{Assessment, AssessmentState, LiveState};

/// How an assessment ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// A report.
    Report,
    /// The site could not be reached.
    Unreachable,
    /// robots.txt asked us not to read it.
    Refused,
    /// The check itself could not run.
    Failed,
}

/// A stored finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredFinding {
    /// Its id.
    pub id: FindingId,
    /// What was found.
    #[serde(flatten)]
    pub found: Found,
}

/// The receipt's totals, as the page shows them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    /// Hosts contacted.
    pub hosts: u64,
    /// Requests allowed.
    pub requests: u64,
    /// Bytes received.
    pub bytes: u64,
    /// Requests refused.
    pub refusals: u64,
}

/// Everything the report page needs, stored in `assessments.report`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportDoc {
    /// How it ended.
    pub outcome: Outcome,
    /// Pages read as HTML.
    pub pages_read: u32,
    /// Pages tried.
    pub pages_tried: u32,
    /// Links within the site not fetched.
    pub unchecked_links: u32,
    /// The slowest page, in milliseconds.
    pub slowest_ms: Option<u64>,
    /// The summary, in plain words.
    pub summary: String,
    /// The findings.
    pub findings: Vec<StoredFinding>,
    /// The main fetch job's receipt.
    pub receipt: Option<Digest>,
    /// Every job's receipt.
    pub receipts: Vec<Digest>,
    /// What we saw, when there is no report (R1.6).
    pub said: Option<String>,
    /// What to do next, when there is no report.
    pub next_step: Option<String>,
    /// The receipt's totals.
    pub totals: Totals,
    /// The last frame.
    pub frame: Option<FrameEvent>,
    /// The HTML pages read, with their bodies' digests in the CAS: agentrix.website's "Your
    /// site" desk answers a visitor's questions from them (W10). Older reports have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<ReadPage>,
}

/// One HTML page the check read, and where its body is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadPage {
    /// The page's address.
    pub url: String,
    /// Its body in the CAS (kind as the crawl stored it).
    pub digest: Digest,
}

impl ReportDoc {
    /// A report with no findings or pages, for the outcomes without one.
    pub fn without_report(outcome: Outcome, said: String, next_step: String) -> Self {
        Self {
            outcome,
            pages_read: 0,
            pages_tried: 0,
            unchecked_links: 0,
            slowest_ms: None,
            summary: String::new(),
            findings: Vec::new(),
            receipt: None,
            receipts: Vec::new(),
            said: Some(said),
            next_step: Some(next_step),
            totals: Totals::default(),
            frame: None,
            pages: Vec::new(),
        }
    }

    /// Findings of `severity`.
    pub fn count(&self, severity: Severity) -> u32 {
        self.findings
            .iter()
            .filter(|f| f.found.severity == severity)
            .count() as u32
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The summary line: what we read and what matters most.
pub fn summary(host: &str, pages_read: u32, findings: &[StoredFinding]) -> String {
    let urgent: Vec<&StoredFinding> = findings
        .iter()
        .filter(|f| f.found.severity == Severity::Urgent)
        .collect();
    let warn = findings
        .iter()
        .filter(|f| f.found.severity == Severity::Warn)
        .count() as u32;
    let read = format!(
        "We read {} of {host} in a sealed room",
        plural(pages_read, "page", "pages")
    );
    if !urgent.is_empty() {
        let titles: Vec<String> = urgent
            .iter()
            .take(3)
            .map(|f| {
                let mut t = f.found.title.clone();
                if let Some(c) = t.get(..1) {
                    t.replace_range(..1, &c.to_lowercase());
                }
                t
            })
            .collect();
        let more = if urgent.len() > titles.len() {
            format!("; and {} more", urgent.len() - titles.len())
        } else {
            String::new()
        };
        format!(
            "{read} and found {} attention now: {}{more}. {}",
            plural(
                urgent.len() as u32,
                "problem that needs",
                "problems that need"
            ),
            titles.join("; "),
            if warn == 1 {
                "1 more thing is worth fixing.".to_owned()
            } else if warn > 1 {
                format!("{warn} more things are worth fixing.")
            } else {
                "Nothing else needs fixing.".into()
            }
        )
    } else if warn > 0 {
        format!(
            "{read} and found nothing urgent. {} worth fixing.",
            plural(warn, "thing is", "things are")
        )
    } else {
        format!("{read} and found nothing that needs fixing.")
    }
}

fn severity(s: Severity) -> view::Severity {
    match s {
        Severity::Urgent => view::Severity::Urgent,
        Severity::Warn => view::Severity::Warn,
        Severity::Info => view::Severity::Info,
    }
}

fn finding_view(f: &StoredFinding) -> view::Finding {
    view::Finding {
        id: f.id.to_string(),
        severity: severity(f.found.severity),
        title: f.found.title.clone(),
        plain: f.found.plain.clone(),
        evidence: f
            .found
            .evidence
            .iter()
            .map(|e| view::Evidence {
                label: e.label.clone(),
                excerpt: e.excerpt.clone(),
                receipt: e.receipt.map(|d| d.to_string()),
            })
            .collect(),
    }
}

/// The report's four sections, in order.
pub fn sections(doc: &ReportDoc) -> Vec<view::ReportSection> {
    let n = doc.pages_read;
    let pages = plural(n, "page", "pages");
    let of = |c: Category| -> Vec<view::Finding> {
        let mut v: Vec<&StoredFinding> = doc
            .findings
            .iter()
            .filter(|f| f.found.category == c)
            .collect();
        v.sort_by_key(|f| match f.found.severity {
            Severity::Urgent => 0,
            Severity::Warn => 1,
            Severity::Info => 2,
        });
        v.into_iter().map(finding_view).collect()
    };
    let unchecked = if doc.unchecked_links > 0 {
        format!(
            " We didn't open {} within your site. A check reads at most 50 pages, follows \
             links only a few steps from the first page, and skips any that your robots.txt \
             file asks crawlers to leave alone.",
            plural(doc.unchecked_links, "more link", "more links")
        )
    } else {
        String::new()
    };
    vec![
        view::ReportSection {
            title: "Security".into(),
            empty_words: format!(
                "No signs of tampering on the {pages} we read: no spam links, disguised scripts or \
                 hidden instructions aimed at AI assistants."
            ),
            findings: of(Category::Security),
        },
        view::ReportSection {
            title: "Speed".into(),
            empty_words: match doc.slowest_ms {
                Some(ms) => format!(
                    "Every page we read sent its HTML within 3 seconds (the slowest took {:.1} s).",
                    ms as f64 / 1000.0
                ),
                None => format!("Nothing slow on the {pages} we read."),
            },
            findings: of(Category::Speed),
        },
        view::ReportSection {
            title: "Broken links".into(),
            empty_words: format!(
                "No broken links among the {pages} we read.{unchecked} We don't follow links to \
                 other sites."
            ),
            findings: of(Category::Links),
        },
        view::ReportSection {
            title: "Can AI assistants use your site?".into(),
            empty_words: "Every page we read has a title and a description, and your site has \
                          structured data (business details written for machines to read)."
                .into(),
            findings: of(Category::AiReadiness),
        },
    ]
}

/// "7 Nov 2026, 21:04 UTC".
pub fn when_words(at: DateTime<Utc>) -> String {
    at.format("%-d %b %Y, %H:%M UTC").to_string()
}

/// Where "Move this site" leads: sign-in, then the claim (R1.7).
pub fn move_href(a: &Assessment) -> String {
    format!("/auth/signin?next=/sites/{}/claim", a.site)
}

fn tick_view(t: Totals) -> view::Tick {
    let small = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    view::Tick {
        hosts: small(t.hosts),
        requests: small(t.requests),
        bytes_words: bytes_words(t.bytes),
        refusals: small(t.refusals),
    }
}

fn frame_view(f: &FrameEvent) -> view::Frame {
    view::Frame {
        kind: match f.kind {
            agentrix_types::ui::FrameKind::Live => view::FrameKind::Live,
            agentrix_types::ui::FrameKind::SealedScreenshot => view::FrameKind::SealedScreenshot,
        },
        src: f.src.clone(),
        page_url: f.page_url.clone(),
    }
}

/// The page's view model: from the live state while running, from the report once done.
pub fn view(a: &Assessment, live: Option<&LiveState>) -> view::AssessView {
    let mut v = view::AssessView {
        id: a.id.to_string(),
        url: a.url.clone(),
        host: a.host.clone(),
        fixture: a.fixture,
        state: view::AssessState::Running,
        progress_words: String::new(),
        started_words: when_words(a.created_at),
        tick: view::Tick {
            bytes_words: bytes_words(0),
            ..view::Tick::default()
        },
        frame: None,
        report: None,
        unreachable: None,
        move_href: move_href(a),
    };
    match (&a.report, a.state) {
        (Some(doc), state) if state != AssessmentState::Running => {
            v.tick = tick_view(doc.totals);
            v.frame = doc.frame.as_ref().map(frame_view);
            v.progress_words = format!("{} read", plural(doc.pages_read, "page", "pages"));
            match doc.outcome {
                Outcome::Report => {
                    v.state = view::AssessState::Done;
                    v.report = Some(view::Report {
                        summary_words: doc.summary.clone(),
                        counts: view::Counts {
                            urgent: doc.count(Severity::Urgent),
                            warn: doc.count(Severity::Warn),
                            info: doc.count(Severity::Info),
                        },
                        sections: sections(doc),
                        pages_read: doc.pages_read,
                        receipt: doc.receipt.map(|d| d.to_string()),
                    });
                }
                Outcome::Refused => {
                    v.state = view::AssessState::Refused;
                    v.unreachable = Some(view::Unreachable {
                        said: doc.said.clone().unwrap_or_default(),
                        next_step: doc.next_step.clone().unwrap_or_default(),
                    });
                }
                Outcome::Unreachable | Outcome::Failed => {
                    v.state = view::AssessState::Unreachable;
                    v.unreachable = Some(view::Unreachable {
                        said: doc.said.clone().unwrap_or_default(),
                        next_step: doc.next_step.clone().unwrap_or_default(),
                    });
                }
            }
        }
        _ => {
            if let Some(live) = live {
                v.tick = tick_view(Totals {
                    hosts: live.tick.hosts,
                    requests: live.tick.requests,
                    bytes: live.tick.bytes,
                    refusals: live.tick.refusals,
                });
                v.frame = live.frame.as_ref().map(frame_view);
                v.progress_words = if live.pages_of == 0 {
                    "Opening a sealed room".into()
                } else {
                    format!("{} of {} pages read", live.pages_done, live.pages_of)
                };
            } else {
                v.progress_words = "Starting".into();
            }
        }
    }
    v
}
