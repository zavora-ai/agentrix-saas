//! The copy check every rendered message passes before it ships (`specs/platform/voice.md`).
//!
//! W7's `scripts/copy-lint` scans template sources across the repository; this check runs on
//! the rendered words of every example in the catalogue, so it also sees text that values and
//! filters produce. Both follow the same list.

/// Words and phrases that mark text as machine-written or as marketing filler.
const BANNED: &[&str] = &[
    "seamless",
    "seamlessly",
    "effortless",
    "effortlessly",
    "elevate",
    "empower",
    "unlock",
    "unleash",
    "supercharge",
    "revolutionize",
    "revolutionise",
    "game-changer",
    "cutting-edge",
    "state-of-the-art",
    "next-generation",
    "world-class",
    "best-in-class",
    "robust",
    "leverage",
    "utilize",
    "utilise",
    "delve",
    "journey",
    "landscape",
    "tapestry",
    "vibrant",
    "testament",
    "pivotal",
    "crucial",
    "foster",
    "harness",
    "streamline",
    "synergy",
    "holistic",
    "curated",
    "magic",
    "magical",
    "delightful",
    "stunning",
    "breathtaking",
    "nestled",
    "in today's world",
    "look no further",
    "rest assured",
    "we've got you covered",
    "at your fingertips",
    "take it to the next level",
    "great question",
    "absolutely!",
    "certainly!",
    "i hope this helps",
    "let me know if",
    "happy to help",
    "oops",
    "uh-oh",
    "chatbot",
    "please be advised",
    "kindly",
    "don't hesitate",
    "valued customer",
];

/// One problem the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// What rule it breaks.
    pub rule: &'static str,
    /// The offending piece of text.
    pub found: String,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '-'
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let hay: Vec<char> = haystack.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    if pat.is_empty() || pat.len() > hay.len() {
        return false;
    }
    for start in 0..=(hay.len() - pat.len()) {
        if hay[start..start + pat.len()] == pat[..] {
            let before_ok = start == 0 || !is_word_char(hay[start - 1]);
            let end = start + pat.len();
            let after_ok = end == hay.len()
                || !is_word_char(hay[end])
                || !pat[pat.len() - 1].is_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x1F000..=0x1F2FF | 0xFE0F)
}

/// Checks one piece of user-facing text.
pub fn check(text: &str) -> Vec<Problem> {
    let mut problems = Vec::new();
    let lower = text.to_lowercase();
    for word in BANNED {
        if contains_word(&lower, word) {
            problems.push(Problem {
                rule: "word on voice.md's never list",
                found: (*word).to_string(),
            });
        }
    }
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '—' => problems.push(Problem {
                rule: "em dash in prose",
                found: context(&chars, i),
            }),
            '–' => {
                let digit_before = i > 0 && chars[i - 1].is_ascii_digit();
                let digit_after = chars.get(i + 1).is_some_and(|c| c.is_ascii_digit());
                if !(digit_before && digit_after) {
                    problems.push(Problem {
                        rule: "en dash in prose",
                        found: context(&chars, i),
                    });
                }
            }
            '!' => problems.push(Problem {
                rule: "exclamation mark",
                found: context(&chars, i),
            }),
            c if is_emoji(c) => problems.push(Problem {
                rule: "emoji",
                found: context(&chars, i),
            }),
            _ => {}
        }
    }
    problems
}

fn context(chars: &[char], i: usize) -> String {
    let start = i.saturating_sub(20);
    let end = (i + 20).min(chars.len());
    chars[start..end].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_the_tells() {
        assert!(!check("Seamlessly move your site!").is_empty());
        assert!(!check("Your journey starts here").is_empty());
        assert!(!check("It works — mostly").is_empty());
        assert!(!check("Done 🎉").is_empty());
        assert!(check("Open 9–5, Monday to Saturday.").is_empty());
        assert!(check("Your order is in. Collect from 08:00.").is_empty());
        // Whole words only: "magical" is caught, "magician" isn't a listed word.
        assert!(check("The magician's cake").is_empty());
    }
}
