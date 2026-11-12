//! The automatic check each finished page gets before the owner sees it: are the fundamentals
//! of an Agentrix Website all there? What's missing goes back to the same agent, in the same
//! room, to fix with `fs_write` patch (at most [`MAX_ROUNDS`] rounds).

use crate::facts::SiteFacts;

/// Fix rounds after the first write.
pub const MAX_ROUNDS: i64 = 2;

/// One fundamental, as the check names it to the agent and to the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fundamental {
    /// A short key for records.
    pub key: &'static str,
    /// What the agent is told is missing.
    pub missing: &'static str,
}

const COMPLETE: Fundamental = Fundamental {
    key: "complete",
    missing: "a complete page that ends with </html>",
};
const DESKS: Fundamental = Fundamental {
    key: "desks",
    missing: "WebAgent desks in the navigation, each with real questions",
};
const ASK: Fundamental = Fundamental {
    key: "ask",
    missing: "an Ask box (a text field to ask the site anything)",
};
const AI_LABELS: Fundamental = Fundamental {
    key: "ai-labels",
    missing: "the label \"Sample image, made with AI\" on images",
};
const AGENT_DOOR: Fundamental = Fundamental {
    key: "agent-door",
    missing: "a footer link to /.well-known/awp for AI agents",
};
const SIGN_IN: Fundamental = Fundamental {
    key: "sign-in",
    missing: "a Sign in link (passkey) to /account",
};
const REAL_FACTS: Fundamental = Fundamental {
    key: "real-facts",
    missing: "the business's real address, phone or opening hours from the site facts",
};
const RESPONSIVE: Fundamental = Fundamental {
    key: "responsive",
    missing: "responsive CSS (media queries or clamp())",
};
const NO_SCRIPTS: Fundamental = Fundamental {
    key: "no-scripts",
    missing: "no scripts: remove every <script>, iframe, javascript: link and on… event attribute",
};

/// Every fundamental, in check order.
pub const ALL: [Fundamental; 9] = [
    COMPLETE, DESKS, ASK, AI_LABELS, AGENT_DOOR, SIGN_IN, REAL_FACTS, RESPONSIVE, NO_SCRIPTS,
];

/// The plain words for a fundamental's key.
pub fn words_for(key: &str) -> Option<&'static str> {
    ALL.iter().find(|f| f.key == key).map(|f| f.missing)
}

/// What the page still lacks, in the order the agent should fix it. Empty when it's all there.
pub fn missing(raw: &str, facts: &SiteFacts) -> Vec<Fundamental> {
    let lower = raw.to_ascii_lowercase();
    let mut out = Vec::new();
    let trimmed = lower.trim_end();
    if !(lower.contains("<body") && trimmed.ends_with("</html>")) {
        out.push(COMPLETE);
    }
    if !(lower.contains("desk") || lower.contains("webagent")) {
        out.push(DESKS);
    }
    if !((lower.contains("<input") || lower.contains("<textarea")) && lower.contains("ask")) {
        out.push(ASK);
    }
    if !lower.contains("made with ai") && lower.contains("<img") {
        out.push(AI_LABELS);
    }
    if !lower.contains("/.well-known/awp") {
        out.push(AGENT_DOOR);
    }
    if !lower.contains("sign in") {
        out.push(SIGN_IN);
    }
    if !has_real_facts(raw, facts) {
        out.push(REAL_FACTS);
    }
    if !(lower.contains("@media") || lower.contains("clamp(")) {
        out.push(RESPONSIVE);
    }
    if has_scripts(&lower) {
        out.push(NO_SCRIPTS);
    }
    out
}

/// Whether the page uses something a customer needs from the facts: a phone number, an
/// opening time, or a street. True when the facts have none of them to check against.
fn has_real_facts(raw: &str, facts: &SiteFacts) -> bool {
    let wanted = fact_marks(facts);
    if wanted.is_empty() {
        return true;
    }
    let page = squash(raw);
    wanted.iter().any(|w| page.contains(w.as_str()))
}

/// Lower-cased, without spaces, dots or dashes, so "7 a.m." matches "7am".
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '.' | '-' | '\u{a0}'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Phone numbers (9+ digits), times ("7am", "7:30") and street names ("Ngong Road") in the facts.
fn fact_marks(facts: &SiteFacts) -> Vec<String> {
    let mut text = facts.tagline.clone();
    for p in &facts.pages {
        text.push(' ');
        text.push_str(&p.text);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut marks: Vec<String> = Vec::new();
    // Phone numbers: runs of digits (with spaces, + and dashes) of 9 or more digits.
    let mut digits = String::new();
    for w in &words {
        let d: String = w.chars().filter(char::is_ascii_digit).collect();
        let phoneish = !d.is_empty()
            && w.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '(' | ')' | '.' | ','));
        if phoneish {
            digits.push_str(&d);
        } else {
            if digits.len() >= 9 {
                marks.push(digits[digits.len() - 9..].to_owned());
            }
            digits.clear();
        }
    }
    if digits.len() >= 9 {
        marks.push(digits[digits.len() - 9..].to_owned());
    }
    for (i, w) in words.iter().enumerate() {
        let lw = squash(w);
        // Times: "7am", "7:30", "7" followed by "am"/"a.m."
        let next = words.get(i + 1).map(|n| squash(n)).unwrap_or_default();
        let digit_lead = lw.chars().next().is_some_and(|c| c.is_ascii_digit());
        if digit_lead && (lw.ends_with("am") || lw.ends_with("pm")) && lw.len() <= 7 {
            marks.push(lw.trim_end_matches([',', ';']).to_owned());
        } else if digit_lead && (next.starts_with("am") || next.starts_with("pm")) {
            marks.push(format!("{lw}{}", &next[..2]));
        } else if digit_lead && lw.contains(':') && lw.len() <= 6 {
            marks.push(lw.trim_end_matches([',', ';']).to_owned());
        }
        // Streets: the word before Road/Street/Avenue/Lane/Drive.
        let street = ["road", "street", "avenue", "lane", "drive", "rd", "st"];
        if i > 0 && street.contains(&lw.trim_end_matches([',', '.'])) {
            let name = squash(words[i - 1]);
            if name.len() >= 3 && name.chars().all(char::is_alphabetic) {
                marks.push(name);
            }
        }
    }
    marks.sort();
    marks.dedup();
    marks
}

/// Scripts, frames, script URLs or inline event handlers in the agent's own file.
fn has_scripts(lower: &str) -> bool {
    if lower.contains("<script") || lower.contains("<iframe") || lower.contains("javascript:") {
        return true;
    }
    // ` on…=` inside a tag.
    let b = lower.as_bytes();
    let mut in_tag = false;
    for i in 0..b.len() {
        match b[i] {
            b'<' => in_tag = true,
            b'>' => in_tag = false,
            c if in_tag
                && c.is_ascii_whitespace()
                && b.get(i + 1) == Some(&b'o')
                && b.get(i + 2) == Some(&b'n') =>
            {
                let rest = &b[i + 3..];
                let name_len = rest.iter().take_while(|c| c.is_ascii_alphabetic()).count();
                if name_len >= 3 {
                    let after = &rest[name_len..];
                    let eq = after.iter().find(|c| !c.is_ascii_whitespace());
                    if eq == Some(&b'=') {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// The fix request for what's missing, one line each, for [`crate::prompts::revise_prompt`].
pub fn fix_request(missing: &[Fundamental]) -> String {
    missing
        .iter()
        .map(|m| format!("- {}", m.missing))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::PageFacts;

    fn facts() -> SiteFacts {
        SiteFacts {
            name: "Kilele Bakery".into(),
            pages: vec![PageFacts {
                url: "https://kilele.example/".into(),
                title: "Kilele".into(),
                text: "Open Tuesday to Sunday, 7 a.m. to 6 p.m. Opposite Adams Arcade on Ngong Road. Call +254 712 345 678.".into(),
            }],
            ..SiteFacts::default()
        }
    }

    const GOOD: &str = r#"<!doctype html><html><head><style>@media (min-width:600px){.a{}}</style></head><body>
<nav>Bread desk · Cakes desk</nav><form action="/ask" method="get"><input name="q"></form>
<figure><img src="/img/a.webp" alt="Bread"><figcaption>Sample image, made with AI</figcaption></figure>
<p>Open 7am to 6pm on Ngong Road.</p><a href="/account">Sign in</a><footer><a href="/.well-known/awp">For AI agents</a></footer></body></html>"#;

    #[test]
    fn a_complete_page_passes() {
        assert_eq!(missing(GOOD, &facts()), vec![]);
    }

    #[test]
    fn each_gap_is_named() {
        let keys =
            |html: &str| -> Vec<&str> { missing(html, &facts()).iter().map(|m| m.key).collect() };
        assert_eq!(keys(&GOOD.replace("</html>", "")), ["complete"]);
        assert_eq!(keys(&GOOD.replace("made with AI", "")), ["ai-labels"]);
        // No images, no label needed.
        assert_eq!(
            keys(
                &GOOD
                    .replace(r#"<img src="/img/a.webp" alt="Bread">"#, "")
                    .replace("made with AI", "")
            ),
            Vec::<&str>::new()
        );
        assert_eq!(
            keys(&GOOD.replace("/.well-known/awp", "/agents")),
            ["agent-door"]
        );
        assert_eq!(keys(&GOOD.replace(r#"<input name="q">"#, "")), ["ask"]);
        assert_eq!(keys(&GOOD.replace("@media", "@supports")), ["responsive"]);
        assert_eq!(
            keys(&GOOD.replace("Open 7am to 6pm on Ngong Road.", "Come by soon.")),
            ["real-facts"]
        );
        assert_eq!(
            keys(&GOOD.replace("<nav>", "<nav onclick = \"x()\">")),
            ["no-scripts"]
        );
        assert_eq!(
            keys(&GOOD.replace("<body>", "<body><script>x</script>")),
            ["no-scripts"]
        );
        // A phone number counts as a real fact too.
        assert_eq!(
            keys(&GOOD.replace("Open 7am to 6pm on Ngong Road.", "Call 0712 345 678.")),
            Vec::<&str>::new()
        );
        // A plain word starting with "on" isn't a handler.
        assert_eq!(
            keys(&GOOD.replace("<nav>", "<nav one=\"1\" class=\"online\">")),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn the_fix_request_lists_what_is_missing() {
        let p = fix_request(&[AI_LABELS, AGENT_DOOR]);
        assert!(p.contains("made with AI") && p.contains("/.well-known/awp"));
        assert_eq!(p.lines().count(), 2);
    }
}
