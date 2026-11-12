//! What the owner meant, from what they said: a fixed grammar, never a model, so a spoken
//! decision is the same every time (adk-surface's voice rules; 07 §3.6).
//!
//! By voice the owner may allow once, deny, stop everything, hear the brief, hear what's next,
//! or ask the Chief WebAgent anything else. "Stop everything" acts at once; a spoken decision
//! waits 3 seconds for a cancel; a serious one goes to a screen.

use serde::Serialize;

/// The owner's intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// "Stop everything": freeze every WebAgent, at once.
    Stop,
    /// "Cancel" during the 3-second window.
    Cancel,
    /// "Allow once" (or "approve that").
    AllowOnce,
    /// "Deny", "decline".
    Deny,
    /// "Read my brief".
    Brief,
    /// "What's next?", "what's waiting?"
    Next,
    /// Anything else: a question for the Chief WebAgent.
    Ask,
}

fn words(said: &str) -> Vec<String> {
    said.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.replace('\u{2019}', "'"))
        .collect()
}

fn has(words: &[String], phrase: &[&str]) -> bool {
    words
        .windows(phrase.len())
        .any(|w| w.iter().zip(phrase).all(|(a, b)| a == b))
}

/// Reads one utterance. Short commands win over questions: "deny it" is a decision, but a
/// long sentence that merely contains "no" is a question for the Chief.
pub fn read(said: &str) -> Intent {
    let w = words(said);
    if w.is_empty() {
        return Intent::Ask;
    }
    if has(&w, &["stop", "everything"])
        || has(&w, &["freeze", "everything"])
        || has(&w, &["stop", "all"])
    {
        return Intent::Stop;
    }
    let short = w.len() <= 6;
    if short
        && (w[0] == "cancel" || has(&w, &["don't", "do", "it"]) || has(&w, &["wait", "cancel"]))
    {
        return Intent::Cancel;
    }
    if short
        && (has(&w, &["allow", "once"])
            || has(&w, &["allow", "it", "once"])
            || has(&w, &["allow", "that", "once"])
            || has(&w, &["yes", "once"])
            || has(&w, &["approve", "it"])
            || has(&w, &["approve", "that"])
            || w == ["approve"]
            || w == ["allow"])
    {
        return Intent::AllowOnce;
    }
    if short
        && ["deny", "decline", "reject", "refuse"]
            .iter()
            .any(|v| w.iter().any(|x| x == v))
    {
        return Intent::Deny;
    }
    if short && (w == ["no"] || has(&w, &["no", "thanks"])) {
        return Intent::Deny;
    }
    if has(&w, &["brief"]) || has(&w, &["what", "happened"]) {
        return Intent::Brief;
    }
    if short
        && (w.iter().any(|x| x == "next")
            || has(&w, &["what's", "waiting"])
            || has(&w, &["what", "is", "waiting"])
            || has(&w, &["what", "needs", "me"]))
    {
        return Intent::Next;
    }
    Intent::Ask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_are_read_by_a_fixed_grammar() {
        for (said, want) in [
            ("Allow once.", Intent::AllowOnce),
            ("allow it once", Intent::AllowOnce),
            ("Yes, once", Intent::AllowOnce),
            ("Approve that", Intent::AllowOnce),
            ("Deny.", Intent::Deny),
            ("Decline it please", Intent::Deny),
            ("No.", Intent::Deny),
            ("Stop everything!", Intent::Stop),
            ("please freeze everything now", Intent::Stop),
            ("Cancel", Intent::Cancel),
            ("Wait, cancel that", Intent::Cancel),
            ("Read me my brief", Intent::Brief),
            ("What happened overnight?", Intent::Brief),
            ("What's next?", Intent::Next),
            ("What's waiting", Intent::Next),
            ("How many croissants did we sell yesterday?", Intent::Ask),
            ("", Intent::Ask),
        ] {
            assert_eq!(read(said), want, "{said:?}");
        }
    }

    #[test]
    fn a_long_sentence_is_a_question_not_a_decision() {
        assert_eq!(
            read("Why is there no answer about the gluten free cakes on the site"),
            Intent::Ask
        );
        assert_eq!(
            read("can you tell me whether I should deny the refund for the order from yesterday"),
            Intent::Ask
        );
    }
}
