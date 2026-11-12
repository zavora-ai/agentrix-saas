//! The sample sites' agent door, where an agent is known only when it proves it (v3, "Verify at
//! sample doors"): saas-settle's Web Bot Auth check and the federation's agent tokens behind
//! agentrix-samples' [`AgentProof`]. Node registers it once:
//!
//! ```ignore
//! agentrix_samples::set_agent_proof(Arc::new(saas_settle::SampleDoorProof::new(
//!     DoorAuth::any_signed(directories, store),  // any agent whose own directory publishes its key
//!     Some(federation),                          // saas-federation's `Federation`, or `None`
//!     agentrix_keys,                             // seals the refusal receipts (Agentrix hosts the samples)
//!     Some(events),                              // `SettleEvent::Refused` to the kernel
//! )));
//! ```

use std::sync::Arc;

use agentrix_samples::{AgentProof, DoorRefusal, ProofFailure, ProofRequest, ProvenAgent};
use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::door::DoorAuth;
use crate::events::{EventSender, SettleEvent};
use crate::keys::CompanyKeys;
use crate::protocol::RefusalReceipt;
use crate::seal::{SealKind, Sealed};
use crate::tokens::{AgentTokens, bearer};

/// Proves agents at the sample doors and seals their refusals.
pub struct SampleDoorProof {
    auth: DoorAuth,
    tokens: Option<Arc<dyn AgentTokens>>,
    keys: CompanyKeys,
    events: Option<EventSender>,
}

impl std::fmt::Debug for SampleDoorProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleDoorProof")
            .field("signer", &self.keys.origin)
            .field("tokens", &self.tokens.is_some())
            .finish_non_exhaustive()
    }
}

impl SampleDoorProof {
    /// Checks signatures with `auth` (normally [`DoorAuth::any_signed`]), tokens with `tokens`
    /// for the door's own origin as audience, and seals refusals with `keys`' agent key.
    pub fn new(
        auth: DoorAuth,
        tokens: Option<Arc<dyn AgentTokens>>,
        keys: CompanyKeys,
        events: Option<EventSender>,
    ) -> Self {
        Self {
            auth,
            tokens,
            keys,
            events,
        }
    }
}

fn failure(code: &str, message: impl Into<String>) -> ProofFailure {
    ProofFailure {
        code: code.into(),
        message: message.into(),
    }
}

#[async_trait]
impl AgentProof for SampleDoorProof {
    async fn prove(&self, request: ProofRequest<'_>) -> Result<Option<ProvenAgent>, ProofFailure> {
        if let Some(token) = bearer(request.headers) {
            let Some(tokens) = &self.tokens else {
                return Err(failure(
                    "token_refused",
                    "this door trusts no token issuers yet.",
                ));
            };
            return match tokens.principal(token, request.door).await {
                Ok(p) => Ok(Some(ProvenAgent {
                    id: p.principal.id.to_string(),
                    vouched_by: p.vouched_by(),
                    method: "token".into(),
                })),
                Err(e) => Err(failure(
                    "token_refused",
                    format!("the token was refused: {e}."),
                )),
            };
        }
        // A bare Signature-Agent header is a claim, not a proof.
        if !request.headers.contains_key("signature")
            && !request.headers.contains_key("signature-input")
        {
            return Ok(None);
        }
        match self
            .auth
            .verify(request.method, request.uri, request.headers, request.body)
            .await
        {
            Ok(agent) => Ok(Some(ProvenAgent {
                id: agent.id,
                vouched_by: agent.origin,
                method: "web-bot-auth".into(),
            })),
            Err(refusal) => Err(failure(
                refusal.code,
                refusal.message.trim_end_matches(" Nothing was done."),
            )),
        }
    }

    async fn refusal_receipt(&self, refusal: DoorRefusal<'_>) -> Option<Value> {
        let receipt = RefusalReceipt {
            refusal_id: format!("rr_{}", uuid::Uuid::now_v7().simple()),
            refused_by: refusal.door.to_owned(),
            company: refusal.site.to_owned(),
            requester: None,
            claimed_agent: refusal.claimed.map(str::to_owned),
            message: Some(refusal.tool.to_owned()),
            code: refusal.code.to_owned(),
            reason: refusal.reason.to_owned(),
            request_digest: Some(format!(
                "sha256:{}",
                hex::encode(Sha256::digest(refusal.body))
            )),
            nothing_moved: true,
            at: chrono::Utc::now(),
            principal: None,
        };
        let key = self.keys.agents.values().next()?;
        let sealed =
            Sealed::seal(SealKind::RefusalReceipt, &receipt, &self.keys.origin, key).ok()?;
        if let Ok(json) = serde_json::to_string(&sealed) {
            if let Err(e) = self
                .auth
                .store
                .put_refusal(&receipt.refusal_id, &receipt.code, &json)
                .await
            {
                tracing::warn!(error = %e, "settle: sample door refusal receipt not kept");
            }
        }
        if let Some(events) = &self.events {
            let _ = events.send(SettleEvent::Refused { receipt });
        }
        serde_json::to_value(&sealed).ok()
    }
}
