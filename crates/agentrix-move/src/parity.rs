//! Prove: parity between each rebuilt page and the clean content it came from (R6.2, R6.4).
//!
//! Per page:
//! - **text:** word-level similarity (adk-site's `text_similarity`) between the clean
//!   source's words and the words the rebuilt page shows. The source is the *clean* text, so
//!   leaving the infection behind never counts against parity;
//! - **vision:** when the `render` room produced a screenshot, the `vision-parity` purpose
//!   compares it with the old page's screenshot (from the crawl) or, without one, with the
//!   text it should show, and answers a score and one line.
//!
//! A page's score is `0.7 × text + 0.3 × vision`, or the text score alone when no screenshot
//! exists. The site's parity is the mean over its pages.

use adk_site::migrate::parity::text_similarity;
use adk_site::migrate::sanitize::Sanitiser;
use agentrix_types::Digest;
use agentrix_types::ui::ParityState;
use serde::{Deserialize, Serialize};

/// The bars a candidate must clear (R6.4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParityPolicy {
    /// A page below this after its tries is flagged for the owner.
    pub page_pass: f64,
    /// The site's mean must reach this.
    pub site_pass: f64,
    /// The most inputs one render room may take (the runner's `MAX_INPUTS`); pages go in
    /// batches under it.
    #[serde(default = "default_room_inputs")]
    pub room_inputs: usize,
}

fn default_room_inputs() -> usize {
    crate::pipeline::MAX_ROOM_INPUTS
}

impl Default for ParityPolicy {
    fn default() -> Self {
        Self {
            page_pass: 0.9,
            site_pass: 0.95,
            room_inputs: default_room_inputs(),
        }
    }
}

/// One page's parity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageParity {
    /// The page.
    pub path: String,
    /// The score, 0 to 1.
    pub score: f64,
    /// Text similarity.
    pub text: f64,
    /// The vision model's score, when there was a screenshot.
    pub vision: Option<f64>,
    /// One plain line.
    pub verdict: String,
    /// Pass, retry or flagged.
    pub state: ParityState,
    /// Which try this is.
    pub attempt: u32,
    /// The rebuilt page's screenshot, when the room made one.
    pub screenshot: Option<Digest>,
    /// The owner accepted the page as it is.
    #[serde(default)]
    pub accepted: bool,
}

/// The site's parity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityReport {
    /// Every page's latest parity.
    pub pages: Vec<PageParity>,
    /// The mean score.
    pub site: f64,
    /// True when the pages were rendered in a sealed room with no network.
    pub rendered_in_room: bool,
    /// The render jobs' receipts.
    pub render_receipts: Vec<Digest>,
    /// Why the pages were not rendered, when they weren't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_note: Option<String>,
}

impl ParityReport {
    /// Pages flagged and not accepted by the owner.
    pub fn open_flags(&self) -> Vec<&PageParity> {
        self.pages
            .iter()
            .filter(|p| p.state == ParityState::Flagged && !p.accepted)
            .collect()
    }

    /// Whether a candidate may be raised: site parity at the bar and no open flags (R6.4).
    pub fn candidate_ready(&self, policy: &ParityPolicy) -> bool {
        self.site >= policy.site_pass && self.open_flags().is_empty()
    }

    /// Recomputes the site score.
    pub fn recompute(&mut self) {
        self.site = if self.pages.is_empty() {
            0.0
        } else {
            self.pages.iter().map(|p| p.score).sum::<f64>() / self.pages.len() as f64
        };
    }
}

/// The words a page shows, from HTML: the sanitiser's Markdown, reduced to words.
pub fn words_of_html(html: &str) -> String {
    words_of_markdown(&Sanitiser::new().sanitise(html, None).markdown)
}

/// The words of Markdown: link targets and syntax removed, lowercase.
pub fn words_of_markdown(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut chars = markdown.chars().peekable();
    let mut prev = ' ';
    while let Some(c) = chars.next() {
        if c == ']' && chars.peek() == Some(&'(') {
            // Skip the link target.
            let mut depth = 0;
            for t in chars.by_ref() {
                match t {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            prev = ' ';
            out.push(' ');
            continue;
        }
        if c == '\\' {
            continue;
        }
        let keep = c.is_alphanumeric() || (c == '\'' && prev.is_alphanumeric());
        if keep {
            out.extend(c.to_lowercase());
            prev = c;
        } else if prev != ' ' {
            out.push(' ');
            prev = ' ';
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text similarity between the clean Markdown and a rebuilt body.
pub fn text_parity(clean_markdown: &str, body_html: &str) -> f64 {
    text_similarity(
        &words_of_markdown(clean_markdown),
        &words_of_html(body_html),
    )
}

/// A page's score from its parts.
pub fn page_score(text: f64, vision: Option<f64>) -> f64 {
    match vision {
        Some(v) => 0.7 * text + 0.3 * v.clamp(0.0, 1.0),
        None => text,
    }
}

/// What a parity shortfall tells the builder on its next try (R6.3).
pub fn feedback(parity: &PageParity) -> String {
    let mut line = format!(
        "parity was {:.0}%: the page kept {:.0}% of the source text",
        parity.score * 100.0,
        parity.text * 100.0
    );
    if parity.vision.is_some() {
        line.push_str(&format!("; the visual check said: {}", parity.verdict));
    }
    line.push_str(". Keep every sentence of the source, in order, word for word.");
    line
}

/// Where a render room writes a page's screenshot.
pub fn shot_name(file: &str) -> String {
    format!("shots/{}.png", file.replace('/', "__"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_ignore_markup_and_link_targets() {
        assert_eq!(
            words_of_markdown("## Delivery\n\nOrder at [our shop](/shop/) by **Friday**.\n"),
            "delivery order at our shop by friday"
        );
        assert_eq!(
            words_of_html(
                "<h2 id=\"d\">Delivery</h2><p>Order at <a href=\"/shop/\">our shop</a> by <strong>Friday</strong>.</p>"
            ),
            "delivery order at our shop by friday"
        );
        assert!(text_parity("Fresh bread daily.", "<p>Fresh bread daily.</p>") > 0.99);
        assert!(text_parity("Fresh bread daily, cakes on order.", "<p>Fresh bread.</p>") < 0.9);
    }

    #[test]
    fn the_site_needs_the_bar_and_no_open_flags() {
        let page = |score: f64, state| PageParity {
            path: "/".into(),
            score,
            text: score,
            vision: None,
            verdict: String::new(),
            state,
            attempt: 1,
            screenshot: None,
            accepted: false,
        };
        let policy = ParityPolicy::default();
        let mut report = ParityReport {
            pages: vec![page(0.99, ParityState::Pass), page(0.97, ParityState::Pass)],
            site: 0.0,
            rendered_in_room: true,
            render_receipts: vec![],
            render_note: None,
        };
        report.recompute();
        assert!(report.candidate_ready(&policy));
        report.pages.push(page(0.6, ParityState::Flagged));
        report.recompute();
        assert!(!report.candidate_ready(&policy));
        report.pages[2].accepted = true;
        assert!(report.open_flags().is_empty());
        assert!(
            !report.candidate_ready(&policy),
            "accepted, but the mean is still below 0.95"
        );
    }
}
