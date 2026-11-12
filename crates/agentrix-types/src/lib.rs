//! Shared types for the first release (specs/first-release/design.md §3, §4). Task S0-08.
//!
//! - **Ids:** [`SiteId`], [`MoveId`], [`AssessmentId`], [`JobId`], [`ReceiptId`],
//!   [`DecisionId`], [`FindingId`], [`QuarantineId`].
//! - **Content addresses:** [`Digest`], written `sha256:<64 lowercase hex>`.
//! - **Signing:** Ed25519 over RFC 8785 canonical JSON ([`canonical_json`], [`Signer`],
//!   [`verify`]). Control publishes its keys as a [`KeySet`] at `/.well-known/agentrix/keys`.
//! - **Broker and runner** (§4.2): [`JobSpec`], [`SignedJobSpec`], [`JobEvent`],
//!   [`JobResult`]; the runner's NDJSON stream is one [`JobEvent`] per line.
//! - **Receipts** (§4.4): [`Receipt`], [`ReceiptBody`], [`ReceiptKind`].
//! - **The Move's outputs:** [`Finding`], [`DecisionCard`], [`Approval`], and
//!   [`EffectRequest`] with its fingerprint (ADR-006).
//! - **UI events for SSE** (§4.3a): [`UiEvent`] in a [`UiEnvelope`].
//! - **Stored shapes several lanes read:** [`Charter`], [`Brief`], [`GauntletAttack`] and
//!   [`QuarantineEntry`].
//!
//! Every signature and fingerprint covers the RFC 8785 canonical bytes of a typed value, and
//! every check recomputes those bytes from the value it was handed. Signer and verifier
//! therefore cannot drift. A third party can check a receipt without this crate: drop the
//! `signature` member, canonicalise the rest with any RFC 8785 library, and verify with the
//! published key named by `issuer`.

pub mod decision;
pub mod digest;
pub mod effect;
pub mod error;
pub mod finding;
pub mod ids;
pub mod job;
pub mod receipt;
pub mod records;
pub mod sign;
pub mod tenancy;
pub mod ui;

pub use decision::{Approval, ApprovalMethod, CardKind, DecisionCard, Risk};
pub use digest::Digest;
pub use effect::{EffectRequest, Money};
pub use error::{Error, Result};
pub use finding::{Category, Finding, Severity};
pub use ids::{
    AssessmentId, DecisionId, FindingId, JobId, MoveId, QuarantineId, ReceiptId, SiteId,
};
pub use job::{
    ArtifactRef, Caps, ImageRef, JobEvent, JobKind, JobResult, JobSpec, KillReason, NOTE_MAX,
    Peaks, Refusal, SignedJobSpec,
};
pub use receipt::{Observed, Policy, Receipt, ReceiptBody, ReceiptKind, Subject};
pub use records::{Brief, BriefLine, Charter, GauntletAttack, JobLevel, Level, QuarantineEntry};
pub use sign::{KeyId, KeySet, PublishedKey, Signature, Signer, canonical_json, verify};
pub use ui::{UiEnvelope, UiEvent};

/// Re-exported so callers name the same key types this crate verifies with.
pub use ed25519_dalek::{SigningKey, VerifyingKey};
