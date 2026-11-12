//! How a sample site's agent door learns who an agent is (v3: "an agent is known only when it
//! proves it"). A request proves its agent with a Web Bot Auth signature (RFC 9421) checked
//! against the key directory its `Signature-Agent` names, or with a bearer token from an issuer
//! the site trusts, addressed to this door. A name alone (`clientInfo.name`, a bare
//! `Signature-Agent` header) is only claimed and gives no rights.
//!
//! The checking lives outside this crate (saas-settle implements [`AgentProof`] over its
//! signature check and the federation's tokens); node registers it once with
//! [`set_agent_proof`]. Without one, nobody is proven and the known-only tools refuse.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::http::{HeaderMap, Method, Uri};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One request at the door, as a proof checker needs it.
#[derive(Debug, Clone, Copy)]
pub struct ProofRequest<'a> {
    /// This door's origin (`https://kilele-bakery-sample.paypal.agentrix.website`): the
    /// audience a token must be addressed to.
    pub door: &'a str,
    /// The request's method.
    pub method: &'a Method,
    /// The request's URI as the agent signed it.
    pub uri: &'a Uri,
    /// Its headers.
    pub headers: &'a HeaderMap,
    /// Its body.
    pub body: &'a [u8],
}

/// An agent that proved who it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenAgent {
    /// Its verified id: `wba:{origin}#{key}` for a signature, the token's principal otherwise.
    pub id: String,
    /// Who vouched: the key directory's origin, or the token's issuer in words.
    pub vouched_by: String,
    /// How: `web-bot-auth` or `token`.
    pub method: String,
}

/// A proof that was offered and didn't check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofFailure {
    /// A short code: `signature_invalid`, `replayed_request`, `token_refused`, …
    pub code: String,
    /// Why, in plain words.
    pub message: String,
}

/// A known-only call the door refused, for its signed receipt.
#[derive(Debug, Clone, Copy)]
pub struct DoorRefusal<'a> {
    /// The door's origin.
    pub door: &'a str,
    /// The site's name.
    pub site: &'a str,
    /// The tool refused.
    pub tool: &'a str,
    /// A short code, for example `agent_identity_required`.
    pub code: &'a str,
    /// Why, in plain words.
    pub reason: &'a str,
    /// The name the caller gave, if any (claimed, not proven).
    pub claimed: Option<&'a str>,
    /// The request body.
    pub body: &'a [u8],
}

/// Checks a request's proof of identity and signs the door's refusals.
#[async_trait]
pub trait AgentProof: Send + Sync {
    /// The agent the request proves, `Ok(None)` when it carries no proof, or why the proof it
    /// carries doesn't check.
    async fn prove(&self, request: ProofRequest<'_>) -> Result<Option<ProvenAgent>, ProofFailure>;

    /// A signed refusal receipt in the settlement door's shape (`nothing_moved: true`, the
    /// claimed agent, the code), kept by the checker; `None` if it can't sign.
    async fn refusal_receipt(&self, refusal: DoorRefusal<'_>) -> Option<Value>;
}

static PROOF: OnceLock<Arc<dyn AgentProof>> = OnceLock::new();

/// Registers the door's proof checker. The first registration wins; `false` if one was
/// already set.
pub fn set_agent_proof(proof: Arc<dyn AgentProof>) -> bool {
    PROOF.set(proof).is_ok()
}

pub(crate) fn agent_proof() -> Option<Arc<dyn AgentProof>> {
    PROOF.get().cloned()
}
