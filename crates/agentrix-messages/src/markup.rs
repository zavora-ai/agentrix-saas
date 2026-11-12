//! The small line format message bodies are written in, and its two renderings.
//!
//! A message is written once and sent as plain text and as HTML, so both always say the same
//! thing. Each line is one of:
//!
//! | Line | Meaning |
//! |---|---|
//! | `# Text` | The heading at the top of an email |
//! | text lines | A paragraph; a blank line ends it |
//! | `- Text` | A bullet; consecutive bullets form a list |
//! | `::code 482913` | A one-time code, large and spaced |
//! | `::button Label \| https://…` | The one main action |
//! | `::link Label \| https://…` | A secondary link on its own line |
//! | `::fact Label \| Value` | A row of facts; consecutive rows form a table |
//! | `::quote Text` | Someone's own words, quoted |
//! | `::note Text` | Small print |
//!
//! Values come from people (names, messages), so the renderer marks every interpolated value
//! with private-use sentinels (see [`crate::render`]). A line that starts with a value is always
//! a paragraph, and a `|` inside a value never splits a line, so nobody can turn their name
//! into a button.

/// Marks the start of an interpolated value.
pub(crate) const OPEN: char = '\u{E000}';
/// Marks the end of an interpolated value.
pub(crate) const CLOSE: char = '\u{E001}';

/// One block of a message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// The heading at the top.
    Heading(String),
    /// A paragraph.
    Para(String),
    /// A list of bullets.
    Bullets(Vec<String>),
    /// A one-time code.
    Code(String),
    /// The main action.
    Button {
        /// What the button says.
        label: String,
        /// Where it goes (http or https only).
        url: String,
    },
    /// A secondary link.
    Link {
        /// What the link says.
        label: String,
        /// Where it goes (http or https only).
        url: String,
    },
    /// A table of label and value rows.
    Facts(Vec<(String, String)>),
    /// Someone's own words.
    Quote(String),
    /// Small print.
    Note(String),
}

/// Why a body could not be parsed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MarkupError {
    /// A directive the format doesn't know.
    #[error("line {line}: unknown directive `::{name}`")]
    UnknownDirective {
        /// 1-based line number in the rendered body.
        line: usize,
        /// The directive's name.
        name: String,
    },
    /// A directive that needs `Label | value` got one part.
    #[error("line {line}: `::{name}` needs `label | value`")]
    MissingPart {
        /// 1-based line number in the rendered body.
        line: usize,
        /// The directive's name.
        name: String,
    },
    /// A link that isn't http or https.
    #[error("line {line}: link `{url}` must start with https:// or http://")]
    BadUrl {
        /// 1-based line number in the rendered body.
        line: usize,
        /// The offending link, sentinels removed.
        url: String,
    },
    /// A directive with nothing after it.
    #[error("line {line}: `::{name}` is empty")]
    Empty {
        /// 1-based line number in the rendered body.
        line: usize,
        /// The directive's name.
        name: String,
    },
    /// A directive in the middle of a line, usually because a template tag at the end of the
    /// line before swallowed its line break. Put a blank line after that tag.
    #[error("line {line}: a `::` directive is glued to the text before it")]
    Glued {
        /// 1-based line number in the rendered body.
        line: usize,
    },
}

/// Whether `::name ` appears in literal text (outside values) after the start of a line.
fn has_glued_directive(line: &str) -> bool {
    let mut depth = 0usize;
    let chars: Vec<char> = line.chars().collect();
    for i in 0..chars.len() {
        match chars[i] {
            OPEN => depth += 1,
            CLOSE => depth = depth.saturating_sub(1),
            ':' if depth == 0 && i > 0 && chars.get(i + 1) == Some(&':') => {
                let rest: String = chars[i + 2..].iter().take(8).collect();
                if [
                    "code ", "button ", "link ", "fact ", "quote ", "note ", "reply ",
                ]
                .iter()
                .any(|d| rest.starts_with(d))
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Removes the sentinels around interpolated values.
pub(crate) fn plain(s: &str) -> String {
    s.chars().filter(|c| *c != OPEN && *c != CLOSE).collect()
}

/// Splits `label | value` at the first ` | ` that is not inside an interpolated value.
fn split_pipe(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let bytes: Vec<(usize, char)> = s.char_indices().collect();
    for (i, &(pos, c)) in bytes.iter().enumerate() {
        match c {
            OPEN => depth += 1,
            CLOSE => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                let before_space = i > 0 && bytes[i - 1].1 == ' ';
                let after_space = bytes.get(i + 1).is_some_and(|&(_, c)| c == ' ');
                if before_space && after_space {
                    return Some((s[..pos].trim_end(), s[pos + 1..].trim_start()));
                }
            }
            _ => {}
        }
    }
    None
}

fn check_url(line: usize, url: &str) -> Result<String, MarkupError> {
    let url = plain(url).trim().to_string();
    let lower = url.to_ascii_lowercase();
    if (lower.starts_with("https://") || lower.starts_with("http://"))
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '<' || c == '>')
    {
        Ok(url)
    } else {
        Err(MarkupError::BadUrl { line, url })
    }
}

/// Parses a rendered body into blocks.
pub fn parse(body: &str) -> Result<Vec<Block>, MarkupError> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Vec<String> = Vec::new();

    fn flush(para: &mut Vec<String>, blocks: &mut Vec<Block>) {
        if !para.is_empty() {
            blocks.push(Block::Para(para.join(" ")));
            para.clear();
        }
    }

    for (idx, raw) in body.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if plain(line).trim().is_empty() {
            flush(&mut para, &mut blocks);
            continue;
        }
        if has_glued_directive(line) {
            return Err(MarkupError::Glued { line: line_no });
        }
        if let Some(rest) = line.strip_prefix("::") {
            flush(&mut para, &mut blocks);
            let (name, arg) = match rest.find(' ') {
                Some(i) => (&rest[..i], rest[i + 1..].trim()),
                None => (rest, ""),
            };
            if plain(arg).trim().is_empty() {
                return Err(MarkupError::Empty {
                    line: line_no,
                    name: name.to_string(),
                });
            }
            let two = |name: &str| {
                split_pipe(arg).ok_or_else(|| MarkupError::MissingPart {
                    line: line_no,
                    name: name.to_string(),
                })
            };
            match name {
                "code" => blocks.push(Block::Code(arg.to_string())),
                "quote" => blocks.push(Block::Quote(arg.to_string())),
                "note" => blocks.push(Block::Note(arg.to_string())),
                "button" => {
                    let (label, url) = two(name)?;
                    blocks.push(Block::Button {
                        label: label.to_string(),
                        url: check_url(line_no, url)?,
                    });
                }
                "link" => {
                    let (label, url) = two(name)?;
                    blocks.push(Block::Link {
                        label: label.to_string(),
                        url: check_url(line_no, url)?,
                    });
                }
                "fact" => {
                    let (label, value) = two(name)?;
                    let row = (label.to_string(), value.to_string());
                    match blocks.last_mut() {
                        Some(Block::Facts(rows)) => rows.push(row),
                        _ => blocks.push(Block::Facts(vec![row])),
                    }
                }
                other => {
                    return Err(MarkupError::UnknownDirective {
                        line: line_no,
                        name: other.to_string(),
                    });
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(rest.to_string()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            flush(&mut para, &mut blocks);
            match blocks.last_mut() {
                Some(Block::Bullets(items)) => items.push(rest.to_string()),
                _ => blocks.push(Block::Bullets(vec![rest.to_string()])),
            }
            continue;
        }
        para.push(line.to_string());
    }
    flush(&mut para, &mut blocks);
    Ok(blocks)
}

/// Groups a one-time code for reading aloud: `482913` becomes `482 913`.
pub fn spaced_code(code: &str) -> String {
    let code = plain(code);
    let chars: Vec<char> = code.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.len() < 6 || !chars.len().is_multiple_of(3) {
        return code;
    }
    chars
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The plain-text rendering.
pub fn to_text(blocks: &[Block]) -> String {
    let mut out: Vec<String> = Vec::new();
    for block in blocks {
        out.push(match block {
            Block::Heading(t) => plain(t),
            Block::Para(t) => plain(t),
            Block::Bullets(items) => items
                .iter()
                .map(|i| format!("- {}", plain(i)))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Code(c) => format!("    {}", spaced_code(c)),
            Block::Button { label, url } | Block::Link { label, url } => {
                format!("{}: {}", plain(label), url)
            }
            Block::Facts(rows) => rows
                .iter()
                .map(|(l, v)| format!("{}: {}", plain(l), plain(v)))
                .collect::<Vec<_>>()
                .join("\n"),
            Block::Quote(t) => format!("> {}", plain(t)),
            Block::Note(t) => plain(t),
        });
    }
    out.join("\n\n")
}

/// Escapes text for HTML element content and attribute values.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            OPEN | CLOSE => {}
            c => out.push(c),
        }
    }
    out
}

/// Colours the HTML rendering uses. Email clients ignore stylesheets, so every element carries
/// inline styles.
#[derive(Debug, Clone)]
pub struct Palette {
    /// Headings and strong text.
    pub head: String,
    /// Body text.
    pub ink: String,
    /// Small print.
    pub muted: String,
    /// The button and links.
    pub accent: String,
    /// Text on the button.
    pub on_accent: String,
    /// Hairlines.
    pub line: String,
    /// The code box and quote background.
    pub soft: String,
}

impl Default for Palette {
    fn default() -> Self {
        // concept/styles.css tokens.
        Self {
            head: "#1f1d3d".into(),
            ink: "#23223a".into(),
            muted: "#656479".into(),
            accent: "#38356d".into(),
            on_accent: "#ffffff".into(),
            line: "#e3e2ec".into(),
            soft: "#f6f6fb".into(),
        }
    }
}

const TEXT_FONT: &str = "'DM Sans',-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";
const DISPLAY_FONT: &str = "Manrope,-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";
const MONO_FONT: &str = "'JetBrains Mono',ui-monospace,'SF Mono',Menlo,Consolas,monospace";

/// The HTML rendering of the body (the layout adds the frame, header and footer).
pub fn to_html(blocks: &[Block], p: &Palette) -> String {
    let mut out = String::new();
    for block in blocks {
        match block {
            Block::Heading(t) => out.push_str(&format!(
                "<h1 style=\"margin:0 0 16px;font-family:{DISPLAY_FONT};font-size:22px;line-height:1.3;font-weight:800;color:{};\">{}</h1>\n",
                p.head,
                escape(t)
            )),
            Block::Para(t) => out.push_str(&format!(
                "<p style=\"margin:0 0 14px;font-family:{TEXT_FONT};font-size:16px;line-height:1.55;color:{};\">{}</p>\n",
                p.ink,
                escape(t)
            )),
            Block::Bullets(items) => {
                out.push_str(&format!(
                    "<ul style=\"margin:0 0 14px;padding-left:20px;font-family:{TEXT_FONT};font-size:16px;line-height:1.55;color:{};\">\n",
                    p.ink
                ));
                for i in items {
                    out.push_str(&format!("<li style=\"margin:0 0 4px;\">{}</li>\n", escape(i)));
                }
                out.push_str("</ul>\n");
            }
            Block::Code(c) => out.push_str(&format!(
                "<p style=\"margin:4px 0 18px;\"><span style=\"display:inline-block;padding:12px 18px;border-radius:12px;background:{};border:1px solid {};font-family:{MONO_FONT};font-size:28px;line-height:1.2;font-weight:600;letter-spacing:4px;color:{};\">{}</span></p>\n",
                p.soft,
                p.line,
                p.head,
                escape(&spaced_code(c))
            )),
            Block::Button { label, url } => out.push_str(&format!(
                "<p style=\"margin:6px 0 18px;\"><a href=\"{}\" style=\"display:inline-block;padding:12px 20px;border-radius:12px;background:{};color:{};font-family:{TEXT_FONT};font-size:16px;font-weight:700;text-decoration:none;\">{}</a></p>\n",
                escape(url),
                p.accent,
                p.on_accent,
                escape(label)
            )),
            Block::Link { label, url } => out.push_str(&format!(
                "<p style=\"margin:0 0 14px;font-family:{TEXT_FONT};font-size:16px;line-height:1.55;\"><a href=\"{}\" style=\"color:{};font-weight:700;\">{}</a></p>\n",
                escape(url),
                p.accent,
                escape(label)
            )),
            Block::Facts(rows) => {
                out.push_str(&format!(
                    "<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" style=\"width:100%;margin:0 0 18px;border-collapse:collapse;font-family:{TEXT_FONT};font-size:15px;line-height:1.45;\">\n"
                ));
                for (l, v) in rows {
                    out.push_str(&format!(
                        "<tr><td style=\"padding:8px 12px 8px 0;border-top:1px solid {};color:{};vertical-align:top;width:38%;\">{}</td><td style=\"padding:8px 0;border-top:1px solid {};color:{};font-weight:600;vertical-align:top;\">{}</td></tr>\n",
                        p.line,
                        p.muted,
                        escape(l),
                        p.line,
                        p.ink,
                        escape(v)
                    ));
                }
                out.push_str("</table>\n");
            }
            Block::Quote(t) => out.push_str(&format!(
                "<p style=\"margin:0 0 14px;padding:10px 14px;border-left:3px solid {};background:{};font-family:{TEXT_FONT};font-size:16px;line-height:1.55;color:{};\">{}</p>\n",
                p.accent,
                p.soft,
                p.ink,
                escape(t)
            )),
            Block::Note(t) => out.push_str(&format!(
                "<p style=\"margin:0 0 12px;font-family:{TEXT_FONT};font-size:13px;line-height:1.5;color:{};\">{}</p>\n",
                p.muted,
                escape(t)
            )),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> String {
        format!("{OPEN}{s}{CLOSE}")
    }

    #[test]
    fn parses_every_line_kind() {
        let body = format!(
            "# Your code\n\nUse it within\n10 minutes.\n\n::code {}\n::button Open your order | {}\n::fact Total | KSh 4,500\n::fact Paid | USD 19.77\n- one\n- two\n::quote {}\n::note Small print.",
            v("482913"),
            v("https://kilele.example/orders/KB-1042"),
            v("Can it be ready by 9?")
        );
        let blocks = parse(&body).unwrap();
        assert_eq!(blocks.len(), 8);
        assert_eq!(blocks[1], Block::Para("Use it within 10 minutes.".into()));
        assert!(
            matches!(&blocks[3], Block::Button { url, .. } if url == "https://kilele.example/orders/KB-1042")
        );
        assert!(matches!(&blocks[4], Block::Facts(rows) if rows.len() == 2));
        assert!(matches!(&blocks[5], Block::Bullets(items) if items.len() == 2));
    }

    #[test]
    fn a_value_never_becomes_markup() {
        // A name that looks like a directive or a heading stays a paragraph.
        let body = format!(
            "{} wrote to you.\n\n{}",
            v("::button Pay | https://evil.example"),
            v("# Free money")
        );
        let blocks = parse(&body).unwrap();
        assert!(blocks.iter().all(|b| matches!(b, Block::Para(_))));
        // A pipe inside a value never splits a fact.
        let body = format!("::fact Name | {}", v("Amina | Admin"));
        let blocks = parse(&body).unwrap();
        assert_eq!(
            blocks,
            vec![Block::Facts(vec![("Name".into(), v("Amina | Admin"))])]
        );
        let body = format!("::button {} ", v("Pay | https://evil.example"));
        assert!(parse(&body).is_err());
    }

    #[test]
    fn links_must_be_web_links() {
        let body = format!("::button Open | {}", v("javascript:alert(1)"));
        assert!(matches!(parse(&body), Err(MarkupError::BadUrl { .. })));
    }

    #[test]
    fn html_escapes_values() {
        let blocks = parse(&format!("Hello {}", v("<script>x</script>"))).unwrap();
        let html = to_html(&blocks, &Palette::default());
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn codes_are_spaced_in_both_versions() {
        let blocks = parse(&format!("::code {}", v("482913"))).unwrap();
        assert_eq!(to_text(&blocks), "    482 913");
        assert!(to_html(&blocks, &Palette::default()).contains(">482 913<"));
    }
}
