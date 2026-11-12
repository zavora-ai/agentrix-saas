//! The converted site's theme: the experience lane's `templates/site/page.html` and
//! `static/site/site.css` (S1-09), filled with the site's tokens and each rebuilt page.
//!
//! The template and stylesheet are the experience lane's files, read at compile time, so the
//! Move and the edge agree on one theme.

use minijinja::Environment;
use serde::Serialize;

use crate::clean::NavLink;

const PAGE_TEMPLATE: &str = include_str!("../../agentrix-console/templates/site/page.html");

/// The theme's stylesheet, copied into every release as `assets/site.css`.
pub const SITE_CSS: &[u8] = include_bytes!("../../agentrix-console/static/site/site.css");

/// The site's look: colours and fonts (06 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Tokens {
    /// Page background.
    pub bg: String,
    /// A softer background for panels.
    pub soft: String,
    /// Body text.
    pub ink: String,
    /// Secondary text.
    pub ink2: String,
    /// Rules and borders.
    pub line: String,
    /// The brand colour, for links and buttons.
    pub accent: String,
    /// Text on the brand colour.
    pub on_accent: String,
    /// The display font stack.
    pub display: String,
    /// The text font stack.
    pub text: String,
}

impl Default for Tokens {
    fn default() -> Self {
        Self {
            bg: "#fffdf8".into(),
            soft: "#f6efe4".into(),
            ink: "#1f1a14".into(),
            ink2: "#5b4f42".into(),
            line: "#e6dccd".into(),
            accent: "#8a3b12".into(),
            on_accent: "#ffffff".into(),
            display: "Georgia, 'Times New Roman', serif".into(),
            text: "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif".into(),
        }
    }
}

fn hex_rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim_start_matches('#');
    let full = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h.to_owned(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some((
        f64::from((v >> 16) & 0xff) / 255.0,
        f64::from((v >> 8) & 0xff) / 255.0,
        f64::from(v & 0xff) / 255.0,
    ))
}

fn luminance(hex: &str) -> Option<f64> {
    let (r, g, b) = hex_rgb(hex)?;
    let lin = |c: f64| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b))
}

/// The WCAG contrast ratio of two hex colours.
pub fn contrast(a: &str, b: &str) -> f64 {
    match (luminance(a), luminance(b)) {
        (Some(x), Some(y)) => (x.max(y) + 0.05) / (x.min(y) + 0.05),
        _ => 1.0,
    }
}

impl Tokens {
    /// Tokens that keep the old site's brand colour, read from its stylesheet as text: the
    /// most used saturated colour becomes the accent, when it keeps AA contrast (4.5:1) on
    /// the background. Everything else stays at the accessible defaults.
    pub fn from_css(css: &str) -> Self {
        let mut tokens = Self::default();
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        let bytes = css.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'#' {
                let hex: String = css[i + 1..]
                    .chars()
                    .take_while(char::is_ascii_hexdigit)
                    .collect();
                if hex.len() == 6 || hex.len() == 3 {
                    *counts
                        .entry(format!("#{}", hex.to_ascii_lowercase()))
                        .or_default() += 1;
                }
                i += 1 + hex.len();
            } else {
                i += 1;
            }
        }
        let saturated = |hex: &str| {
            hex_rgb(hex).is_some_and(|(r, g, b)| {
                let max = r.max(g).max(b);
                let min = r.min(g).min(b);
                max - min > 0.25
            })
        };
        if let Some((accent, _)) = counts
            .iter()
            .filter(|(hex, _)| saturated(hex) && contrast(hex, &tokens.bg) >= 4.5)
            .max_by_key(|(_, n)| **n)
        {
            tokens.accent = accent.clone();
            tokens.on_accent = if contrast(accent, "#ffffff") >= 4.5 {
                "#ffffff".into()
            } else {
                tokens.ink.clone()
            };
        }
        tokens
    }
}

impl Tokens {
    /// The look the owner chose before the Move, kept only where it stays readable: body text
    /// and secondary text at AA (4.5:1) on the background, the accent at AA on the background,
    /// and text on the accent at AA. Anything else falls back to the accessible defaults.
    /// Colours must be `#hex` and font stacks plain names, so nothing else reaches the
    /// stylesheet.
    pub fn chosen(look: &Tokens) -> Self {
        let d = Self::default();
        let hex = |v: &str| {
            let h = v.trim().to_ascii_lowercase();
            (h.starts_with('#')
                && matches!(h.len(), 4 | 7)
                && h[1..].chars().all(|c| c.is_ascii_hexdigit()))
            .then_some(h)
        };
        // Family names unquoted (`Cormorant Garamond, Georgia, serif` is valid CSS): the page
        // template escapes quotes, which would break the declaration inside its <style>.
        let font = |v: &str| {
            let f = v.trim();
            (!f.is_empty()
                && f.len() <= 200
                && f.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, ' ' | ',' | '\'' | '"' | '-')
                }))
            .then(|| f.replace(['\'', '"'], ""))
        };
        let mut t = Self::default();
        if let (Some(bg), Some(ink)) = (hex(&look.bg), hex(&look.ink))
            && contrast(&ink, &bg) >= 4.5
        {
            t.bg = bg;
            t.ink = ink;
            t.soft = hex(&look.soft)
                .filter(|s| contrast(&t.ink, s) >= 4.5)
                .unwrap_or_else(|| t.bg.clone());
            t.line = hex(&look.line).unwrap_or(d.line);
            t.ink2 = hex(&look.ink2)
                .filter(|s| contrast(s, &t.bg) >= 4.5)
                .unwrap_or_else(|| t.ink.clone());
        }
        let accent = hex(&look.accent).filter(|a| contrast(a, &t.bg) >= 4.5);
        if let Some(accent) = accent {
            t.on_accent = [
                hex(&look.on_accent),
                Some("#ffffff".into()),
                Some(t.ink.clone()),
            ]
            .into_iter()
            .flatten()
            .find(|c| contrast(c, &accent) >= 4.5)
            .unwrap_or_else(|| "#ffffff".into());
            t.accent = accent;
        } else if contrast(&t.accent, &t.bg) < 4.5 {
            t.accent = t.ink.clone();
            t.on_accent = t.bg.clone();
        }
        t.display = font(&look.display).unwrap_or(d.display);
        t.text = font(&look.text).unwrap_or(d.text);
        t
    }
}

/// The site, as the template names it.
#[derive(Debug, Clone, Serialize)]
pub struct SiteView {
    /// The business name.
    pub name: String,
    /// The domain.
    pub domain: String,
    /// The language.
    pub lang: String,
    /// The look.
    pub tokens: Tokens,
    /// The menu.
    pub nav: Vec<NavLink>,
    /// The contact page, when there is one.
    pub contact_href: Option<String>,
    /// Zavora's fixture site (R3.3).
    pub fixture: bool,
    /// Whether the site counts visits: the footer says so and offers the visitor's switch.
    pub counting: bool,
    /// The visit counter's script tag (W8's `visits::script_tag`), when the site counts visits.
    pub counter: Option<String>,
}

/// One page, as the template names it.
#[derive(Debug, Clone, Serialize)]
pub struct PageView<'a> {
    /// Its title.
    pub title: &'a str,
    /// Its description.
    pub description: Option<&'a str>,
    /// Its path.
    pub path: &'a str,
    /// Its Markdown twin.
    pub md_href: String,
    /// The rebuilt body, checked by [`crate::rebuild::check_body`].
    pub body_html: &'a str,
    /// When it was last changed, in words.
    pub updated: Option<String>,
}

/// Where a page's files go in the release: `/` is `index.html`, `/about/` is
/// `about/index.html`.
pub fn page_file(path: &str) -> String {
    let trimmed = path
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_matches('/');
    if trimmed.is_empty() {
        "index.html".into()
    } else {
        format!("{trimmed}/index.html")
    }
}

/// The page's Markdown twin, beside its HTML.
pub fn twin_file(path: &str) -> String {
    page_file(path).trim_end_matches(".html").to_owned() + ".md"
}

/// Renders one page with the site theme.
pub fn render_page(site: &SiteView, page: &PageView<'_>) -> anyhow::Result<String> {
    let mut env = Environment::new();
    env.add_template("page.html", PAGE_TEMPLATE)?;
    let html = env
        .get_template("page.html")?
        .render(minijinja::context! { site => site, page => page })?;
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_files_and_twins() {
        assert_eq!(page_file("/"), "index.html");
        assert_eq!(page_file("/about/"), "about/index.html");
        assert_eq!(
            page_file("/2026/10/fresh-bread"),
            "2026/10/fresh-bread/index.html"
        );
        assert_eq!(twin_file("/about/"), "about/index.md");
    }

    #[test]
    fn a_chosen_look_is_kept_only_where_it_stays_readable() {
        let look = Tokens {
            bg: "#f6eedf".into(),
            soft: "#f1e4cf".into(),
            ink: "#2a1d12".into(),
            ink2: "#5b4f42".into(),
            line: "#e0d3bd".into(),
            accent: "#9a4a1f".into(),
            on_accent: "#ffffff".into(),
            display: "'Fraunces', Georgia, serif".into(),
            text: "'Nunito Sans', system-ui, sans-serif".into(),
        };
        let t = Tokens::chosen(&look);
        assert_eq!(
            t,
            Tokens {
                display: "Fraunces, Georgia, serif".into(),
                text: "Nunito Sans, system-ui, sans-serif".into(),
                ..look.clone()
            }
        );
        let pale = Tokens {
            accent: "#f0c8a0".into(),
            display: "x;}</style><script>".into(),
            ..look.clone()
        };
        let t = Tokens::chosen(&pale);
        assert_ne!(t.accent, "#f0c8a0");
        assert!(contrast(&t.accent, &t.bg) >= 4.5);
        assert_eq!(t.display, Tokens::default().display);
        let unreadable = Tokens {
            ink: "#eeeeee".into(),
            ..look
        };
        assert_eq!(Tokens::chosen(&unreadable).bg, Tokens::default().bg);
    }

    #[test]
    fn the_brand_colour_is_kept_only_with_enough_contrast() {
        let css = "a { color: #1d4ed8 } .btn { background: #1d4ed8 } .hint { color: #ffff00 }";
        assert_eq!(Tokens::from_css(css).accent, "#1d4ed8");
        assert_eq!(
            Tokens::from_css(".x { color: #ffff00 }").accent,
            Tokens::default().accent
        );
    }

    #[test]
    fn renders_the_experience_lanes_template_with_escaping() {
        let site = SiteView {
            name: "Kilele <Bakery>".into(),
            domain: "kilele-bakery.example".into(),
            lang: "en".into(),
            tokens: Tokens::default(),
            nav: vec![NavLink {
                label: "Home".into(),
                href: "/".into(),
            }],
            contact_href: None,
            fixture: true,
            counting: false,
            counter: None,
        };
        let page = PageView {
            title: "Home",
            description: None,
            path: "/",
            md_href: "/index.md".into(),
            body_html: "<h1>Kilele</h1>",
            updated: None,
        };
        let html = render_page(&site, &page).unwrap();
        assert!(html.contains("<h1>Kilele</h1>"));
        assert!(html.contains("Kilele &lt;Bakery&gt;"));
        assert!(html.contains("Zavora&#x27;s fixture") || html.contains("Zavora's fixture"));
        assert!(!html.contains("/_agx/visit.js") && !html.contains("counts visits"));
    }

    #[test]
    fn a_counting_site_carries_the_counter_and_the_visitors_switch() {
        let site = SiteView {
            name: "Kilele".into(),
            domain: "kilele-bakery.example".into(),
            lang: "en".into(),
            tokens: Tokens::default(),
            nav: vec![],
            contact_href: None,
            fixture: false,
            counting: true,
            counter: Some(agentrix_store::visits::script_tag(Some(3))),
        };
        let page = PageView {
            title: "Home",
            description: None,
            path: "/",
            md_href: "/index.md".into(),
            body_html: "<h1>Kilele</h1>",
            updated: None,
        };
        let html = render_page(&site, &page).unwrap();
        assert!(
            html.contains(r#"<script src="/_agx/visit.js" data-seq="3" defer></script>"#),
            "{html}"
        );
        assert!(html.contains(r#"data-agx-count="off""#));
        assert!(html.contains(r#"data-agx-count="on""#));
        assert!(html.contains(r#"data-visits="gpc""#));
    }
}
