//! JSON shapes stored in `control.db` and read by more than one lane, defined once here so the
//! writer and the console cannot drift.
//!
//! - `charters.body` is a [`Charter`] (move lane writes).
//! - `briefs.body` is a [`Brief`] (brief/ writes).
//! - `gauntlet_runs.detail` is a list of [`GauntletAttack`] (gauntlet lane writes).
//! - A Move's clean-step receipt lists what it set aside as `detail.quarantined`, a list of
//!   [`QuarantineEntry`] (move lane writes; the gauntlet scores from it).

use serde::{Deserialize, Serialize};

use crate::digest::Digest;
use crate::ids::QuarantineId;

/// How much a WebAgent may do on one job without asking (05 §3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// Watches and reports only.
    Watch,
    /// Suggests; the owner acts.
    Suggest,
    /// Prepares the action and asks first.
    AskFirst,
    /// Acts, then tells the owner.
    DoAndTell,
    /// Acts within an agreed budget.
    WithinBudget,
    /// Acts; the owner can only undo.
    UndoOnly,
}

/// One job's starting level and ceiling in a charter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobLevel {
    /// The WebAgent, for example `mover`.
    pub webagent: String,
    /// Its role, in plain words.
    pub role: String,
    /// The job, for example `go-live`.
    pub job: String,
    /// Where the dial starts.
    pub level: Level,
    /// The highest the dial may go.
    pub ceiling: Level,
    /// Why the ceiling is where it is, in plain words.
    pub ceiling_reason: String,
}

/// `charters.body`: the levels the owner signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Charter {
    /// Every job's level.
    pub jobs: Vec<JobLevel>,
}

/// One line of the Chief WebAgent's brief, resting on a receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefLine {
    /// Plain words.
    pub text: String,
    /// The receipt the line rests on, if one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Digest>,
}

/// `briefs.body`: the Chief WebAgent's brief for Today (R10.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brief {
    /// The opening line, if the brief has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    /// What happened, newest first.
    pub lines: Vec<BriefLine>,
    /// Model spend in plain words, for example "Models cost $0.42 today".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_words: Option<String>,
}

/// One attack in a gauntlet run (`gauntlet_runs.detail` is a list of these).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GauntletAttack {
    /// The attack's short name.
    pub name: String,
    /// What it tried, in plain words.
    pub plain: String,
    /// Whether the platform held.
    pub held: bool,
    /// The receipt that proves it, if one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Digest>,
}

/// One item a Move's clean step set aside, as listed in its receipt's `detail.quarantined`.
/// The same fields as the `quarantined` UI event, plus the detector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantineEntry {
    /// The quarantined item.
    pub item_id: QuarantineId,
    /// Plain words.
    pub title: String,
    /// Why, in plain words.
    pub reason: String,
    /// Where it was found.
    #[serde(rename = "where")]
    pub location: String,
    /// The detector that caught it, if one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_serialise_as_the_console_reads_them() {
        let level = serde_json::to_string(&Level::DoAndTell).unwrap();
        assert_eq!(level, r#""do-and-tell""#);
        assert!(Level::Watch < Level::UndoOnly);
        let entry = QuarantineEntry {
            item_id: QuarantineId::new("qtn_1"),
            title: "Hidden spam links".into(),
            reason: "Links injected into the footer".into(),
            location: "wp-content/cache.php".into(),
            detector: Some("spam-links".into()),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["where"], "wp-content/cache.php");
        let brief = Brief {
            headline: None,
            lines: vec![BriefLine {
                text: "Your site moved.".into(),
                receipt: None,
            }],
            spend_words: None,
        };
        assert_eq!(
            serde_json::to_string(&brief).unwrap(),
            r#"{"lines":[{"text":"Your site moved."}]}"#
        );
    }
}
