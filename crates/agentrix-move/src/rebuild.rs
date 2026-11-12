//! Rebuild: each clean page written again by the builder model, checked, and wrapped in the
//! site theme (R6.1, R6.3).
//!
//! The builder's HTML is untrusted output too. [`check_body`] accepts only an allowlist of
//! elements and attributes, links to the site's own pages or to links the clean page already
//! had, and images the clean page already showed; no scripts, styles, handlers, forms or
//! comments. A page that fails is sent back with the error, at most three tries in all; then
//! it is flagged for the owner and carries a plain rendering of its clean text, so nothing
//! unchecked is ever published.

use std::collections::BTreeSet;

use pulldown_cmark::{CowStr, Event, HeadingLevel, Parser, Tag, TagEnd, html};
use serde::{Deserialize, Serialize};

use crate::clean::CleanPage;
use crate::ports::{BuildRequest, CallContext, Gateway};

/// The most tries per page (R6.3).
pub const MAX_ATTEMPTS: u32 = 3;

/// Who wrote a page's body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "by")]
pub enum Builder {
    /// The builder model, through the gateway.
    Model {
        /// The model the gateway reported, if it did.
        model: Option<String>,
    },
    /// A plain rendering of the clean Markdown, after the model's tries ran out.
    Plain,
}

/// One rebuilt page body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuiltBody {
    /// The page's path.
    pub path: String,
    /// The checked body.
    pub body_html: String,
    /// Tries used so far, from 1.
    pub attempts: u32,
    /// Who wrote it.
    pub builder: Builder,
    /// Why the page is flagged for the owner, when its tries ran out.
    pub flagged: Option<String>,
}

/// Renders clean Markdown as HTML: headings get ids (the anchors Ask cites), and raw HTML
/// cannot appear because the sanitiser's Markdown holds none.
pub fn plain_html(markdown: &str) -> String {
    let events: Vec<Event<'_>> = Parser::new(markdown).collect();
    let mut out: Vec<Event<'_>> = Vec::with_capacity(events.len());
    let mut used = BTreeSet::new();
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading {
            level,
            classes,
            attrs,
            ..
        }) = &events[i]
        {
            let mut text = String::new();
            let mut j = i + 1;
            while j < events.len() && !matches!(events[j], Event::End(TagEnd::Heading(_))) {
                if let Event::Text(t) | Event::Code(t) = &events[j] {
                    text.push_str(t);
                }
                j += 1;
            }
            let id = (*level != HeadingLevel::H1).then(|| unique_slug(&text, &mut used));
            out.push(Event::Start(Tag::Heading {
                level: *level,
                id: id.map(CowStr::from),
                classes: classes.clone(),
                attrs: attrs.clone(),
            }));
            i += 1;
            continue;
        }
        if let Event::Html(raw) | Event::InlineHtml(raw) = &events[i] {
            out.push(Event::Text(raw.clone()));
            i += 1;
            continue;
        }
        out.push(events[i].clone());
        i += 1;
    }
    let mut html_out = String::new();
    html::push_html(&mut html_out, out.into_iter());
    html_out
}

/// A heading's anchor: lowercase words joined by `-`, unique on the page.
pub fn unique_slug(text: &str, used: &mut BTreeSet<String>) -> String {
    let mut slug = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let base = slug.trim_matches('-').to_owned();
    let base = if base.is_empty() {
        "section".to_owned()
    } else {
        base
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

const ELEMENTS: &[&str] = &[
    "nav",
    "article",
    "footer",
    "details",
    "summary",
    "u",
    "s",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "a",
    "img",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "em",
    "strong",
    "b",
    "i",
    "del",
    "br",
    "hr",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "caption",
    "section",
    "div",
    "span",
    "figure",
    "figcaption",
    "header",
    "aside",
    "small",
    "address",
    "time",
    "dl",
    "dt",
    "dd",
    "mark",
    "sup",
    "sub",
    "abbr",
    "cite",
    "q",
];

const VOID: &[&str] = &["br", "hr", "img"];

const ATTRIBUTES: &[&str] = &[
    "id", "class", "href", "src", "alt", "title", "width", "height", "loading", "decoding",
    "colspan", "rowspan", "scope", "datetime", "lang", "role", "rel", "target",
];

/// Checks a body the builder wrote. Returns every problem, in words the builder can act on;
/// empty means it may be published.
pub fn check_body(
    html: &str,
    allowed_links: &BTreeSet<String>,
    allowed_images: &BTreeSet<String>,
) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push = |p: String| {
        if seen.insert(p.clone()) {
            problems.push(p);
        }
    };
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            push("no HTML comments".into());
            i += 4;
            continue;
        }
        let Some(end) = html[i..].find('>').map(|e| i + e) else {
            push("a tag is not closed with '>'".into());
            break;
        };
        let inner = &html[i + 1..end];
        if inner.contains('<') {
            push(format!("malformed tag near '{}'", snippet(inner)));
            i = end + 1;
            continue;
        }
        let (closing, rest) = match inner.strip_prefix('/') {
            Some(r) => (true, r),
            None => (false, inner),
        };
        let name_len = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric())
            .count();
        let name = rest[..name_len].to_ascii_lowercase();
        if name.is_empty() {
            push(format!("malformed tag near '{}'", snippet(inner)));
            i = end + 1;
            continue;
        }
        if !ELEMENTS.contains(&name.as_str()) {
            push(format!("the <{name}> element is not allowed"));
            i = end + 1;
            continue;
        }
        if closing {
            if !rest[name_len..].trim().is_empty() {
                push(format!("malformed closing tag </{name}>"));
            }
            i = end + 1;
            continue;
        }
        match parse_attributes(rest[name_len..].trim_end_matches('/')) {
            Err(e) => push(format!("<{name}>: {e}")),
            Ok(attrs) => {
                for (attr, value) in attrs {
                    let attr_ok = ATTRIBUTES.contains(&attr.as_str()) || attr.starts_with("aria-");
                    if !attr_ok {
                        push(format!("the {attr} attribute is not allowed"));
                        continue;
                    }
                    match attr.as_str() {
                        "href" if !link_ok(&value, allowed_links) => {
                            push(format!(
                                "the link to '{}' is not on the page's list of links",
                                snippet(&value)
                            ));
                        }
                        "src" if !allowed_images.contains(value.as_str()) => {
                            push(format!(
                                "the image '{}' is not one of the page's images",
                                snippet(&value)
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
        let _ = VOID;
        i = end + 1;
    }
    problems
}

fn snippet(text: &str) -> String {
    text.chars().take(60).collect()
}

fn link_ok(href: &str, allowed: &BTreeSet<String>) -> bool {
    if href.starts_with('#') {
        return !href.contains(['"', '\'', ' ']);
    }
    if allowed.contains(href) {
        return true;
    }
    let path = href.split(['#', '?']).next().unwrap_or_default();
    if !href.starts_with('/') || href.starts_with("//") {
        return false;
    }
    // `/contact` and `/contact/` are the same page to a visitor.
    let other = match path.strip_suffix('/') {
        Some(bare) if !bare.is_empty() => bare.to_owned(),
        _ => format!("{path}/"),
    };
    allowed.contains(path) || allowed.contains(&other)
}

fn parse_attributes(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '-') {
            i += 1;
        }
        if start == i {
            return Err(format!("unexpected '{}' in a tag", chars[i]));
        }
        let name: String = chars[start..i]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            let quote = chars.get(i).copied();
            if !matches!(quote, Some('"' | '\'')) {
                return Err(format!("the {name} value must be in quotes"));
            }
            let quote = quote.unwrap_or('"');
            i += 1;
            let value_start = i;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            if i >= chars.len() {
                return Err(format!("the {name} value is not closed"));
            }
            let value: String = chars[value_start..i].iter().collect();
            i += 1;
            out.push((name, decode_entities(&value)));
        } else {
            out.push((name, String::new()));
        }
    }
    Ok(out)
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

/// Builds one page: up to [`MAX_ATTEMPTS`] tries through the builder model, each failure fed
/// back. `first_attempt` and `feedback` continue an earlier page's tries (a parity retry).
#[allow(clippy::too_many_arguments)]
pub async fn build_page(
    gateway: &dyn Gateway,
    base: &CallContext,
    site_name: &str,
    page: &CleanPage,
    site_paths: &BTreeSet<String>,
    first_attempt: u32,
    mut feedback: Option<String>,
    on_retry: &mut (dyn FnMut(u32, &str) + Send),
) -> BuiltBody {
    let mut allowed_links: BTreeSet<String> = site_paths.clone();
    allowed_links.extend(page.links.iter().cloned());
    let allowed_images: BTreeSet<String> = page.images.iter().cloned().collect();
    let sections =
        page.markdown.matches("\n## ").count() as u32 + u32::from(page.markdown.starts_with("## "));
    let mut attempt = first_attempt.max(1);
    while attempt <= MAX_ATTEMPTS {
        let request = BuildRequest {
            site_name: site_name.to_owned(),
            path: page.path.clone(),
            title: page.title.clone(),
            markdown: page.markdown.clone(),
            allowed_links: allowed_links.iter().cloned().collect(),
            allowed_images: allowed_images.iter().cloned().collect(),
            feedback: feedback.clone(),
            sections,
        };
        let ctx = CallContext {
            attempt,
            ..base.clone()
        };
        let problem = match gateway.build_page(&ctx, &request).await {
            Ok(reply) if reply.body_html.trim().is_empty() => "the page came back empty".to_owned(),
            Ok(reply) => {
                let problems = check_body(&reply.body_html, &allowed_links, &allowed_images);
                if problems.is_empty() {
                    return BuiltBody {
                        path: page.path.clone(),
                        body_html: reply.body_html,
                        attempts: attempt,
                        builder: Builder::Model { model: reply.model },
                        flagged: None,
                    };
                }
                format!("the page broke these rules: {}", problems.join("; "))
            }
            Err(e) => format!("the builder did not answer: {e}"),
        };
        if attempt < MAX_ATTEMPTS {
            on_retry(attempt, &problem);
        }
        feedback = Some(problem);
        attempt += 1;
    }
    BuiltBody {
        path: page.path.clone(),
        body_html: plain_html(&page.markdown),
        attempts: MAX_ATTEMPTS,
        builder: Builder::Plain,
        flagged: feedback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn plain_html_gives_headings_ids_and_escapes_raw_html() {
        let html = plain_html(
            "## Delivery\n\n### Kilimani\n\nSaturdays.\n\n### Kilimani\n\n<script>x</script>\n",
        );
        assert!(html.contains("<h2 id=\"delivery\">Delivery</h2>"), "{html}");
        assert!(html.contains("<h3 id=\"kilimani\">"));
        assert!(html.contains("<h3 id=\"kilimani-2\">"));
        assert!(!html.contains("<script>"));
        assert!(check_body(&html, &set(&[]), &set(&[])).is_empty(), "{html}");
    }

    #[test]
    fn the_check_refuses_scripts_handlers_styles_and_unknown_links() {
        let links = set(&["/", "/about/", "https://maps.example/kilele"]);
        let images = set(&["/wp-content/uploads/bread.jpg"]);
        assert!(
            check_body(
                r#"<p><a href="/about" rel="noopener" target="_blank">About</a></p>"#,
                &links,
                &images
            )
            .is_empty()
        );
        let ok = r#"<section class="hero"><h1>Kilele</h1><p><a href="/about/#story">About</a> <a href="https://maps.example/kilele">Map</a></p><img src="/wp-content/uploads/bread.jpg" alt="Bread"></section>"#;
        assert!(check_body(ok, &links, &images).is_empty());
        for bad in [
            "<script>alert(1)</script>",
            r#"<p onclick="x()">Hi</p>"#,
            r#"<p style="background:url(//x)">Hi</p>"#,
            r#"<a href="https://casino.example">Win</a>"#,
            r#"<a href="javascript:alert(1)">x</a>"#,
            r#"<img src="https://tracker.example/p.gif">"#,
            "<!-- hi --><p>x</p>",
            "<iframe src=\"/\"></iframe>",
            "<a href=/about/>x</a>",
            "<scr<p>ipt>",
        ] {
            assert!(
                !check_body(bad, &links, &images).is_empty(),
                "{bad} should fail"
            );
        }
    }
}
