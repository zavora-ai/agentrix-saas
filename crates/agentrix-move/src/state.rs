//! The Move's states and steps (design §6, schema `moves.state` and `move_steps.step`).

use std::fmt;

use agentrix_types::ui::StepName;
use serde::{Deserialize, Serialize};

/// Where a Move is. The text forms are the `moves.state` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MoveState {
    /// The site was assessed; ownership is not proved yet.
    Assessed,
    /// Ownership is proved (R3); the Move can read the old site.
    Claimed,
    /// The old site's inventory is read (R4).
    Replayed,
    /// Sanitised, detected and quarantined (R5).
    Cleaned,
    /// Every page rebuilt (R6.1).
    Rebuilt,
    /// Rendered and proved at parity; a candidate release exists (R6.2, R6.4).
    Proved,
    /// The go-live card waits for the owner (R7.1).
    AwaitingOwner,
    /// The owner approved; the release is signed and served (R7.3).
    Live,
    /// The owner undid the go-live (R7.4).
    RolledBack,
    /// A step failed and needs a person.
    Failed,
    /// The owner or the platform stopped the Move.
    Stopped,
}

impl MoveState {
    /// Every state.
    pub const ALL: [Self; 11] = [
        Self::Assessed,
        Self::Claimed,
        Self::Replayed,
        Self::Cleaned,
        Self::Rebuilt,
        Self::Proved,
        Self::AwaitingOwner,
        Self::Live,
        Self::RolledBack,
        Self::Failed,
        Self::Stopped,
    ];

    /// The stored text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assessed => "assessed",
            Self::Claimed => "claimed",
            Self::Replayed => "replayed",
            Self::Cleaned => "cleaned",
            Self::Rebuilt => "rebuilt",
            Self::Proved => "proved",
            Self::AwaitingOwner => "awaiting-owner",
            Self::Live => "live",
            Self::RolledBack => "rolled-back",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    /// Parses the stored text.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == text)
    }

    /// The step that moves the Move on from here, if the Mover may take it by itself.
    /// `None` when the Move waits for a person or has ended: the Mover never goes live by
    /// itself (R7).
    pub fn next_step(self) -> Option<StepName> {
        match self {
            Self::Claimed => Some(StepName::Replay),
            Self::Replayed => Some(StepName::Clean),
            Self::Cleaned => Some(StepName::Rebuild),
            Self::Rebuilt => Some(StepName::Prove),
            Self::Proved => Some(StepName::GoLive),
            _ => None,
        }
    }

    /// The state a finished step leaves the Move in.
    pub fn after(step: StepName) -> Self {
        match step {
            StepName::Assess => Self::Assessed,
            StepName::Claim => Self::Claimed,
            StepName::Replay => Self::Replayed,
            StepName::Clean => Self::Cleaned,
            StepName::Rebuild => Self::Rebuilt,
            StepName::Prove => Self::Proved,
            StepName::GoLive => Self::AwaitingOwner,
        }
    }
}

impl fmt::Display for MoveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stored text of a step (`move_steps.step`).
pub fn step_str(step: StepName) -> &'static str {
    match step {
        StepName::Assess => "assess",
        StepName::Claim => "claim",
        StepName::Replay => "replay",
        StepName::Clean => "clean",
        StepName::Rebuild => "rebuild",
        StepName::Prove => "prove",
        StepName::GoLive => "golive",
    }
}

/// Parses a stored step.
pub fn parse_step(text: &str) -> Option<StepName> {
    STEPS.into_iter().find(|s| step_str(*s) == text)
}

/// The steps in the order the Move screen shows them (R6.5).
pub const STEPS: [StepName; 7] = [
    StepName::Assess,
    StepName::Claim,
    StepName::Replay,
    StepName::Clean,
    StepName::Rebuild,
    StepName::Prove,
    StepName::GoLive,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_round_trip_and_the_mover_stops_before_going_live() {
        for state in MoveState::ALL {
            assert_eq!(MoveState::parse(state.as_str()), Some(state));
        }
        assert_eq!(MoveState::Proved.next_step(), Some(StepName::GoLive));
        assert_eq!(MoveState::after(StepName::GoLive), MoveState::AwaitingOwner);
        assert_eq!(MoveState::AwaitingOwner.next_step(), None);
        assert_eq!(MoveState::Live.next_step(), None);
        for step in STEPS {
            assert_eq!(parse_step(step_str(step)), Some(step));
        }
    }
}
