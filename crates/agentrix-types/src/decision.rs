//! Decision cards and approvals (R7, ADR-006, ADR-014).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::digest::Digest;
use crate::effect::EffectRequest;
use crate::error::Result;
use crate::ids::{DecisionId, SiteId};

/// Which card (design §4.3a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CardKind {
    /// Publish the proved candidate release.
    GoLive,
    /// Put a quarantined item back.
    QuarantineRestore,
    /// An order from an AI customer, held for the owner.
    OrderHeld,
}

/// A risk score from a decision model (`/v1/decide`, Vultr `systemone`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Risk {
    /// The decision model, for example `clef`.
    pub model: String,
    /// Risk from 0 (none) to 1 (high).
    pub score: f64,
    /// How concentrated the model's answer was, 0 to 1, when it reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Plain words, for the card.
    pub summary: String,
}

/// A decision the owner approves or declines, bound to one effect's fingerprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionCard {
    /// The decision.
    pub decision_id: DecisionId,
    /// The site.
    pub site: SiteId,
    /// Which card.
    pub kind: CardKind,
    /// Plain words: what will happen.
    pub title: String,
    /// Plain words: what changes, what it costs, how to undo it.
    pub body: String,
    /// The exact effect an approval allows.
    pub effect: EffectRequest,
    /// `effect.fingerprint()`, which the approval signs over.
    pub fingerprint: Digest,
    /// The decision model's view, when one was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<Risk>,
    /// When the card was raised.
    pub created_at: DateTime<Utc>,
}

impl DecisionCard {
    /// A card for `effect`, with its fingerprint computed and its site taken from the effect.
    pub fn new(
        decision_id: DecisionId,
        kind: CardKind,
        title: impl Into<String>,
        body: impl Into<String>,
        effect: EffectRequest,
        created_at: DateTime<Utc>,
    ) -> Result<Self> {
        Ok(Self {
            decision_id,
            site: effect.site.clone(),
            kind,
            title: title.into(),
            body: body.into(),
            fingerprint: effect.fingerprint()?,
            effect,
            risk: None,
            created_at,
        })
    }

    /// Whether the stored fingerprint still matches the effect. Check before honouring an
    /// approval.
    pub fn fingerprint_matches(&self) -> Result<bool> {
        Ok(self.effect.fingerprint()? == self.fingerprint)
    }
}

/// How the owner approved (ADR-006). Receipts always record it, written as the approve route
/// names it (design §4.1): `passkey` or `totp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApprovalMethod {
    /// A WebAuthn assertion whose challenge commits to the fingerprint (design D9).
    #[serde(rename = "passkey")]
    Passkey,
    /// An MFA step-up: a fresh single-use TOTP code bound to the fingerprint.
    #[serde(rename = "totp")]
    StepUp,
}

/// An approval, as recorded in a `decision.approved` receipt's `detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// The decision approved.
    pub decision_id: DecisionId,
    /// The fingerprint approved.
    pub fingerprint: Digest,
    /// How.
    pub method: ApprovalMethod,
    /// The approver's account id.
    pub approver: String,
    /// When.
    pub at: DateTime<Utc>,
}
