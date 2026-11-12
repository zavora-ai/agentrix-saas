//! What follows a go-live in the background (`specs/platform/handover-wiring.md` 1.3).
//!
//! Once the LaunchGate has moved the pointer and the first probe passed, it hands the new
//! release's pages to an [`AfterLive`] in a spawned task, so go-live never waits on it and a
//! page view never starts it. On the node ([`NodeAfterLive`]):
//! 1. W3's billing records go-live day: a subscription's first charge is then
//!    (`agentrix_billing::Billing::went_live`);
//! 2. W11's Editor drafts a picture for each page that had none, at most [`FILL_LIMIT`] per
//!    go-live (`agentrix_media::fill::fill`). Each waits in Pictures for the owner and reaches
//!    visitors only in a release the owner signs.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use agentrix_host::NodeContext;
use agentrix_media::fill::Page;
use agentrix_types::{Digest, SiteId};
use async_trait::async_trait;
use awp_types::FileEntry;
use chrono::{DateTime, Utc};

use crate::db::cas::Cas;

/// At most this many pictures are drafted for a site per go-live (about $0.02 each, inside the
/// site's budget and the fleet's daily caps).
pub const FILL_LIMIT: usize = 5;

/// The most pages read from a release for the fill.
const MAX_PAGES: usize = 200;

/// What runs once a release is live. Go-live never waits on it.
#[async_trait]
pub trait AfterLive: Send + Sync {
    /// `site` went live at `at`, serving `pages`.
    async fn went_live(&self, site: &SiteId, at: DateTime<Utc>, pages: Vec<Page>);
}

/// Billing and pictures on the node, through the platform crates.
pub struct NodeAfterLive {
    ctx: NodeContext,
}

impl NodeAfterLive {
    /// Over node's context: `control.db` and the Model Gateway.
    pub fn new(ctx: NodeContext) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl AfterLive for NodeAfterLive {
    async fn went_live(&self, site: &SiteId, at: DateTime<Utc>, pages: Vec<Page>) {
        if let Err(e) = agentrix_billing::Billing::new(self.ctx.pool.clone())
            .went_live(site.as_str(), at)
            .await
        {
            tracing::warn!(site = %site, error = %e, "go-live day was not recorded for billing");
        }
        let media = agentrix_media::routes::MediaState::new(&self.ctx);
        match agentrix_media::fill::fill(&media, site.as_str(), "", &pages, FILL_LIMIT).await {
            Ok(filled) => tracing::info!(
                site = %site,
                drafts = filled.drafts.len(),
                skipped = filled.skipped.len(),
                "pictures drafted for pages that had none"
            ),
            Err(e) => {
                tracing::warn!(site = %site, error = %e, "no pictures were drafted after go-live");
            }
        }
    }
}

/// A release file's page path: `index.html` is `/`, `about/index.html` is `/about/`,
/// `menu.html` is `/menu`.
pub fn page_path(file: &str) -> Option<String> {
    let stem = file.strip_suffix(".html")?;
    if file == "404.html" {
        return None;
    }
    Some(match stem.strip_suffix("index") {
        Some("") => "/".to_owned(),
        Some(dir) if dir.ends_with('/') => format!("/{dir}"),
        _ => format!("/{stem}"),
    })
}

/// The page's title and a sentence or two of what it says, from its Markdown twin
/// (`# Title`, a blank line, then the page as Markdown).
pub fn title_and_summary(markdown: &str) -> (String, String) {
    let mut lines = markdown.lines();
    let title = lines
        .by_ref()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim().to_owned())
        .unwrap_or_default();
    let text: Vec<String> = lines
        .map(str::trim)
        .filter(|l| {
            !l.is_empty()
                && !l.starts_with('#')
                && !l.starts_with("![")
                && !l.starts_with('|')
                && !l.starts_with('<')
                && !l.starts_with("```")
        })
        .map(plain_markdown)
        .filter(|l| !l.is_empty())
        .take(6)
        .collect();
    let joined = text.join(" ");
    let mut summary = String::new();
    for sentence in joined.split_inclusive(". ").take(2) {
        summary.push_str(sentence);
    }
    let summary: String = summary.trim().chars().take(300).collect();
    (title, summary)
}

/// Markdown links become their words; emphasis and code marks go.
fn plain_markdown(line: &str) -> String {
    let line = line.trim_start_matches(['-', '*', '>', ' ']);
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '[' => {}
            ']' if chars.peek() == Some(&'(') => {
                for d in chars.by_ref() {
                    if d == ')' {
                        break;
                    }
                }
            }
            '*' | '_' | '`' | ']' => {}
            other => out.push(other),
        }
    }
    out.trim().to_owned()
}

/// The pages of a release, home page first, then by depth: each with its title and summary from
/// its Markdown twin, and whether it already shows a picture.
pub async fn release_pages(cas: &Arc<Cas>, files: &BTreeMap<String, FileEntry>) -> Vec<Page> {
    let mut paths: Vec<(&String, String)> = files
        .keys()
        .filter_map(|f| page_path(f).map(|p| (f, p)))
        .collect();
    paths.sort_by(|(_, a), (_, b)| {
        let depth = |p: &str| {
            p.trim_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .count()
        };
        depth(a).cmp(&depth(b)).then_with(|| a.cmp(b))
    });
    let read = |file: &str| {
        let entry = files.get(file).cloned();
        let cas = cas.clone();
        async move {
            let entry = entry?;
            let digest = Digest::from_str(entry.sha256.as_str()).ok()?;
            let bytes = cas.get(&digest).await.ok()??;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
    };
    let mut pages = Vec::new();
    for (file, path) in paths.into_iter().take(MAX_PAGES) {
        let Some(html) = read(file).await else {
            continue;
        };
        let twin = format!("{}.md", file.trim_end_matches(".html"));
        let (title, summary) = match read(&twin).await {
            Some(md) => title_and_summary(&md),
            None => (html_title(&html), String::new()),
        };
        if title.is_empty() {
            continue;
        }
        pages.push(Page {
            path,
            title,
            summary,
            has_picture: html.to_ascii_lowercase().contains("<img"),
        });
    }
    pages
}

/// The page's `<title>`, before the site name the theme adds (" · Kilele Bakery").
fn html_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title>").map(|i| i + "<title>".len()) else {
        return String::new();
    };
    let Some(len) = lower[start..].find("</title>") else {
        return String::new();
    };
    let title = &html[start..start + len];
    title.split(" · ").next().unwrap_or(title).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_files_name_their_pages() {
        assert_eq!(page_path("index.html").as_deref(), Some("/"));
        assert_eq!(page_path("about/index.html").as_deref(), Some("/about/"));
        assert_eq!(page_path("menu.html").as_deref(), Some("/menu"));
        assert_eq!(page_path("404.html"), None);
        assert_eq!(page_path("about/index.md"), None);
        assert_eq!(page_path("assets/site.css"), None);
    }

    #[test]
    fn a_twin_gives_the_title_and_two_plain_sentences() {
        let md = "# Delivery\n\n## Kilimani\n\nSaturday deliveries to **Kilimani** run 10:00-12:00. \
                  They cost KSh 300. See [our menu](/menu/) first.\n\n![A cake](/cake.jpg)\n";
        let (title, summary) = title_and_summary(md);
        assert_eq!(title, "Delivery");
        assert_eq!(
            summary,
            "Saturday deliveries to Kilimani run 10:00-12:00. They cost KSh 300."
        );
        assert_eq!(title_and_summary("# Home\n").1, "");
    }

    #[test]
    fn the_html_title_drops_the_site_name() {
        assert_eq!(
            html_title("<html><head><title>Cakes · Kilele</title></head></html>"),
            "Cakes"
        );
        assert_eq!(html_title("<p>no title</p>"), "");
    }
}
