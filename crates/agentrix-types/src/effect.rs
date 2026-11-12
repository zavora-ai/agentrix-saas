//! Effect requests and their fingerprints (ADR-006, design D2).
//!
//! No effect runs in the first release; approvals end at a signed decision. The type and its fingerprint
//! ship now so the Effects Gateway's seam is fixed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::digest::Digest;
use crate::error::Result;
use crate::ids::SiteId;
use crate::sign::canonical_json;

/// An amount of money in minor units, for example `{"currency": "KES", "minor": 150000}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    /// ISO 4217 code.
    pub currency: String,
    /// Amount in the currency's minor unit.
    pub minor: i64,
}

/// One consequential effect a WebAgent asks for, waiting on the owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectRequest {
    /// The site.
    pub site: SiteId,
    /// The WebAgent asking, for example `mover`.
    pub webagent: String,
    /// The tool that would run, for example `release.go_live`.
    pub tool: String,
    /// The tool's arguments.
    pub args: serde_json::Value,
    /// Money involved, if any.
    pub amount: Option<Money>,
    /// After this, the request cannot be approved.
    pub expiry: DateTime<Utc>,
}

impl EffectRequest {
    /// The fingerprint `H(site, webagent, tool, canonical_args, amount, expiry)`: SHA-256 of
    /// this request's RFC 8785 canonical JSON. Its 32 bytes are the WebAuthn challenge for a
    /// passkey approval, and the Effects Gateway's idempotency key.
    pub fn fingerprint(&self) -> Result<Digest> {
        Ok(Digest::of(&canonical_json(self)?))
    }

    /// Whether the request has expired at `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expiry
    }
}
