//! The owner's buttons: freeze a company's money tools, and approve or decline a held call.
//!
//! saas-web already decides who may press them ([`saas_web::Access`]). A decision's id is the
//! digest of the held call's fingerprint, as the feed lists it; approving records the decision
//! in the company's approval store and resumes the agent's session, so the call runs exactly as
//! held (or, if it was edited since, is held again). While a company is frozen nothing is
//! approved.

use std::sync::Arc;

use async_trait::async_trait;
use saas_company::runtime::Runtime;
use saas_web::console::{Console, ConsoleError, restore_fingerprint};
use serde_json::{Value, json};

/// The console over the node's companies.
pub struct NodeConsole {
    companies: Vec<Arc<Runtime>>,
}

impl NodeConsole {
    /// Over `companies`.
    pub fn new(companies: Vec<Arc<Runtime>>) -> Self {
        Self { companies }
    }

    fn company(&self, id: &str) -> Result<&Arc<Runtime>, ConsoleError> {
        self.companies
            .iter()
            .find(|r| r.company.config.slug == id)
            .ok_or(ConsoleError::NotFound)
    }
}

fn failed(e: impl std::fmt::Display) -> ConsoleError {
    ConsoleError::Failed(e.to_string())
}

#[async_trait]
impl Console for NodeConsole {
    async fn set_frozen(&self, company: &str, frozen: bool) -> Result<(), ConsoleError> {
        let rt = self.company(company)?;
        let governance = &rt.company.governance;
        if frozen {
            governance
                .freeze("the founder froze every money tool from the console")
                .await
                .map_err(failed)
        } else {
            governance.unfreeze().await.map_err(failed)
        }
    }

    async fn decide(
        &self,
        company: &str,
        decision: &str,
        approve: bool,
        proof: Option<Value>,
    ) -> Result<String, ConsoleError> {
        let rt = self.company(company)?;
        let governance = &rt.company.governance;
        if approve && governance.control.is_frozen() {
            return Err(ConsoleError::Frozen);
        }
        // A custom order an agent asked for at the company's sample site's door.
        if decision.starts_with(crate::door_cards::PREFIX) {
            return crate::door_cards::decide(company, decision, approve, proof.as_ref()).await;
        }
        let held = governance
            .held()
            .await
            .map_err(failed)?
            .into_iter()
            .find(|h| h.view()["fingerprint"] == json!(decision))
            .ok_or(ConsoleError::NotFound)?;
        if approve {
            governance.approve(&held).await.map_err(failed)?;
        } else {
            governance
                .decline(&held, "the founder declined it in the console")
                .await
                .map_err(failed)?;
        }
        if let Some(proof) = proof {
            tracing::info!(company, decision, method = %proof["method"], "the approval carried a proof");
        }
        // Resume where it was held: a flow's thread, or the agent's session.
        let (_, user, session) = &held.scope;
        let resumed = if session.starts_with("p2p-") || session.starts_with("i2c-") {
            rt.flows
                .wake_thread(session, "decided")
                .await
                .map(|_| ())
                .map_err(failed)
        } else {
            let role = user.rsplit('/').next().unwrap_or_default();
            rt.company
                .resume(role, session)
                .await
                .map(|_| ())
                .map_err(failed)
        };
        if let Err(e) = resumed {
            tracing::warn!(company, decision, error = %e, "the decision is recorded; the agent resumes on its next turn");
        }
        Ok(if approve { "approved" } else { "declined" }.into())
    }

    /// Revoke beside Freeze: switches `role` of `company` off (its tokens stop at the next
    /// check; revoking it again answers the same), or back on with `proof`, the passkey
    /// evidence the console checked over [`restore_fingerprint`].
    async fn set_revoked(
        &self,
        company: &str,
        role: &str,
        revoked: bool,
        proof: Option<Value>,
    ) -> Result<Value, ConsoleError> {
        let rt = self.company(company)?;
        let access = rt.company.access().ok_or(ConsoleError::NotConnected)?;
        if revoked {
            return access
                .revoke(role, "the founder revoked it from the console")
                .await
                .map_err(failed);
        }
        if !access.is_revoked(role) {
            return Err(ConsoleError::NotFound);
        }
        let proof = proof.ok_or_else(|| {
            ConsoleError::BadProof("switching an agent back on takes the passkey".into())
        })?;
        let expected = restore_fingerprint(company, role);
        if proof["fingerprint"].as_str() != Some(expected.as_str()) {
            return Err(ConsoleError::BadProof(
                "the passkey signed something other than switching this agent back on".into(),
            ));
        }
        access
            .restore(role, &proof)
            .await
            .map_err(|e| ConsoleError::BadProof(e.to_string()))
    }
}
