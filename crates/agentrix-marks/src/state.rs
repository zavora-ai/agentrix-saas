//! The ten states: one grammar on every site and in the console (06 §4.3, 07 §3.5, ADR-031).
//!
//! A state reaches a mark only from a presence frame (`data-state`), never from a timer or a
//! model's say-so. The words here are the owner's and the visitor's for each state, from the
//! Workshop prototype; W7's words catalogue may take them over, keyed the same way.

use serde::{Deserialize, Serialize};

/// What a WebAgent is doing, as its mark shows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkState {
    /// Between wakes: no lease, no model call. Console only.
    Asleep,
    /// Awake with nothing to do. The mark only.
    #[default]
    Idle,
    /// A person is typing or holding to talk: the halo breathes.
    Listening,
    /// Text is streaming: a steady ring.
    Answering,
    /// A tool is running: the trace line runs and the signature moves.
    Working,
    /// The voice is playing: the ring follows its level.
    Speaking,
    /// Held on the owner: the amber dot.
    Waiting,
    /// Paused by the owner or the platform. Console only; visitors see resting.
    Paused,
    /// Out of budget or unavailable: grey.
    Resting,
    /// The site is frozen. Console shows the banner; visitors see resting.
    Frozen,
}

/// Who is looking at the mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    /// The owner or their team, in the console.
    Owner,
    /// A visitor or customer on the site.
    Visitor,
}

impl MarkState {
    /// Every state, in the Workshop's order.
    pub const ALL: [MarkState; 10] = [
        MarkState::Idle,
        MarkState::Listening,
        MarkState::Working,
        MarkState::Answering,
        MarkState::Speaking,
        MarkState::Waiting,
        MarkState::Resting,
        MarkState::Asleep,
        MarkState::Paused,
        MarkState::Frozen,
    ];

    /// The value of `data-state`.
    pub fn key(self) -> &'static str {
        match self {
            MarkState::Asleep => "asleep",
            MarkState::Idle => "idle",
            MarkState::Listening => "listening",
            MarkState::Answering => "answering",
            MarkState::Working => "working",
            MarkState::Speaking => "speaking",
            MarkState::Waiting => "waiting",
            MarkState::Paused => "paused",
            MarkState::Resting => "resting",
            MarkState::Frozen => "frozen",
        }
    }

    /// Reads a `data-state` value.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The state's name, as the Workshop lists it.
    pub fn name(self) -> &'static str {
        match self {
            MarkState::Asleep => "Asleep",
            MarkState::Idle => "Idle",
            MarkState::Listening => "Listening",
            MarkState::Answering => "Answering",
            MarkState::Working => "Working",
            MarkState::Speaking => "Speaking",
            MarkState::Waiting => "Waiting on the owner",
            MarkState::Paused => "Paused",
            MarkState::Resting => "Resting",
            MarkState::Frozen => "Frozen",
        }
    }

    /// What a visitor's mark shows for this state. Console-only states never reach a visitor
    /// as themselves: asleep looks idle, paused and frozen look resting, never with the reason.
    pub fn for_visitor(self) -> MarkState {
        match self {
            MarkState::Asleep => MarkState::Idle,
            MarkState::Paused | MarkState::Frozen => MarkState::Resting,
            other => other,
        }
    }

    /// Whether the mark moves in this state (before it settles after 5 seconds).
    pub fn moves(self) -> bool {
        matches!(
            self,
            MarkState::Listening | MarkState::Working | MarkState::Speaking
        )
    }

    /// Whether a member of the team counts as busy on the Chief's team mark.
    pub fn busy(self) -> bool {
        matches!(self, MarkState::Working | MarkState::Answering)
    }
}

/// What the words around a mark need to know.
#[derive(Debug, Clone, Default)]
pub struct WordsContext<'a> {
    /// The running tool's label, for working ("Checking today's stock").
    pub label: Option<&'a str>,
    /// The owner's first name or the business name, for waiting.
    pub owner: Option<&'a str>,
    /// When it next wakes, for asleep ("07:15").
    pub next_wake: Option<&'a str>,
    /// Why it's paused, for the owner only.
    pub reason: Option<&'a str>,
}

/// The words beside a mark: the status line, read by a `role="status"` element. Pass the real
/// state, not [`MarkState::for_visitor`]'s: a frozen desk's mark rests, but it says "You can
/// still read the site." Empty means
/// the mark shows on its own. From 06 §4.3's table: visitors see the mark alone while it
/// listens, answers or speaks (captions carry the voice), the real tool label while it works,
/// and never the reason it rests; owners see the reason.
pub fn words(state: MarkState, audience: Audience, cx: &WordsContext<'_>) -> String {
    match (audience, state) {
        (_, MarkState::Idle) => String::new(),
        (_, MarkState::Working) => cx.label.unwrap_or("").to_string(),
        (Audience::Owner, MarkState::Listening) => "Listening".into(),
        (Audience::Owner, MarkState::Answering) => "Answering".into(),
        (Audience::Owner, MarkState::Speaking) => "Speaking".into(),
        (
            Audience::Visitor,
            MarkState::Listening | MarkState::Answering | MarkState::Speaking | MarkState::Asleep,
        ) => String::new(),
        (Audience::Owner, MarkState::Waiting) => "Waiting for you".into(),
        (Audience::Visitor, MarkState::Waiting) => match cx.owner {
            Some(o) => format!("Waiting for {o}'s OK"),
            None => "Waiting for the owner's OK".into(),
        },
        (Audience::Owner, MarkState::Resting) => match cx.reason {
            Some(r) => format!("Resting · {r}"),
            None => "Resting".into(),
        },
        (Audience::Visitor, MarkState::Resting | MarkState::Paused) => {
            "Answers are paused right now. The page and the form below still work.".into()
        }
        (Audience::Owner, MarkState::Asleep) => match cx.next_wake {
            Some(t) => format!("Asleep · next {t}"),
            None => "Asleep".into(),
        },
        (Audience::Owner, MarkState::Paused) => match cx.reason {
            Some(r) => format!("Paused · {r}"),
            None => "Paused".into(),
        },
        (Audience::Owner, MarkState::Frozen) => "Frozen".into(),
        (Audience::Visitor, MarkState::Frozen) => {
            "Answers are paused right now. You can still read the site.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_match_serde() {
        for s in MarkState::ALL {
            assert_eq!(MarkState::from_key(s.key()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.key());
        }
    }

    #[test]
    fn visitors_never_see_console_states() {
        for s in MarkState::ALL {
            let v = s.for_visitor();
            assert!(!matches!(
                v,
                MarkState::Asleep | MarkState::Paused | MarkState::Frozen
            ));
        }
        let paused = words(
            MarkState::Paused,
            Audience::Visitor,
            &WordsContext {
                reason: Some("owner stopped it"),
                ..Default::default()
            },
        );
        assert!(!paused.contains("owner stopped it"));
        assert_eq!(
            words(
                MarkState::Frozen,
                Audience::Visitor,
                &WordsContext::default()
            ),
            "Answers are paused right now. You can still read the site."
        );
    }

    #[test]
    fn words_say_who() {
        let cx = WordsContext {
            owner: Some("Wanjiru"),
            label: Some("Checking today's stock"),
            ..Default::default()
        };
        assert_eq!(
            words(MarkState::Waiting, Audience::Visitor, &cx),
            "Waiting for Wanjiru's OK"
        );
        assert_eq!(
            words(MarkState::Working, Audience::Visitor, &cx),
            "Checking today's stock"
        );
        assert_eq!(words(MarkState::Idle, Audience::Owner, &cx), "");
        assert_eq!(words(MarkState::Answering, Audience::Visitor, &cx), "");
        assert_eq!(
            words(
                MarkState::Working,
                Audience::Visitor,
                &WordsContext::default()
            ),
            ""
        );
        let budget = WordsContext {
            reason: Some("budget reached"),
            ..Default::default()
        };
        assert_eq!(
            words(MarkState::Resting, Audience::Owner, &budget),
            "Resting · budget reached"
        );
        assert!(!words(MarkState::Resting, Audience::Visitor, &budget).contains("budget"));
    }
}
