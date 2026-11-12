//! What an owner can do from a console: freeze their agents, revoke one agent, and approve or
//! decline a decision.
//!
//! Slice B (company) owns the kernel's freeze, the token service that revokes one agent, and the
//! decisions; slice C (settle) owns the passkey ceremony that signs an approval. They plug in by
//! implementing [`Console`]. Until then [`crate::demo::DemoFeed`] keeps the owner's choices in
//! memory.
//!
//! **Revoke** (v3) switches one agent off at once: its tokens stop working at its next call,
//! and no PayPal key is rotated. Switching it back on takes the owner's passkey over
//! [`restore_fingerprint`]. Both go through [`Console::set_revoked`].

use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// Why an owner's action was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConsoleError {
    /// No such company or decision.
    #[error("not found")]
    NotFound,
    /// The company is frozen: nothing that moves money is approved until the owner unfreezes.
    #[error("frozen")]
    Frozen,
    /// Already decided; carries the outcome.
    #[error("already {0}")]
    Decided(String),
    /// The approval's proof (the passkey assertion) did not check out.
    #[error("bad proof: {0}")]
    BadProof(String),
    /// Something else failed; the words are safe to show.
    #[error("{0}")]
    Failed(String),
    /// Nothing behind this action on this node yet.
    #[error("not connected")]
    NotConnected,
}

impl ConsoleError {
    /// The owner's words for it.
    pub fn words(&self) -> String {
        match self {
            ConsoleError::NotFound => "Nothing here by that name.".into(),
            ConsoleError::Frozen => {
                "Your agents are frozen. Unfreeze them before approving a payment.".into()
            }
            ConsoleError::Decided(outcome) => format!("Already {outcome}."),
            ConsoleError::BadProof(why) => format!("The passkey didn't check out: {why}"),
            ConsoleError::Failed(why) => why.clone(),
            ConsoleError::NotConnected => "This isn't connected on this node yet.".into(),
        }
    }
}

/// What the owner's passkey signs to switch `role` of `company` back on: `sha256:` and the hex
/// SHA-256 of the UTF-8 text `restore-agent\n<company>\n<role>` (`company` as the console
/// names it: `agentrix`, `kilele`).
pub fn restore_fingerprint(company: &str, role: &str) -> String {
    let text = format!("restore-agent\n{company}\n{role}");
    format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())))
}

/// The owner's actions, as the consoles call them.
#[async_trait]
pub trait Console: Send + Sync {
    /// Freezes (or unfreezes) every agent of `company` that moves money.
    async fn set_frozen(&self, company: &str, frozen: bool) -> Result<(), ConsoleError>;

    /// Approves or declines `decision`. `proof` is the passkey assertion the console's island
    /// collected over the decision's fingerprint (absent for a decline). Returns the outcome
    /// in words (`approved`, `declined`).
    async fn decide(
        &self,
        company: &str,
        decision: &str,
        approve: bool,
        proof: Option<serde_json::Value>,
    ) -> Result<String, ConsoleError>;

    /// Revokes (`revoked`) or restores one agent of `company` (slice B's token service).
    /// Revoking needs no proof: the agent's tokens stop at its next call, and no PayPal key is
    /// rotated. Restoring needs `proof`, the passkey evidence the console checked over
    /// [`restore_fingerprint`] (`kind`, `rpId`, `origin`, `credential`, `fingerprint`, `nonce`,
    /// `challenge`, `userVerified`, …). Returns the signed `agent.revoke` or `agent.restore`
    /// record: `{agent, revoked, tokens_dropped?, audit_seq}`. A revoked agent shows in the feed
    /// with state `revoked`.
    async fn set_revoked(
        &self,
        _company: &str,
        _role: &str,
        _revoked: bool,
        _proof: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ConsoleError> {
        Err(ConsoleError::NotConnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restore_names_its_agent() {
        let fp = restore_fingerprint("kilele", "cashier");
        let text = "restore-agent\nkilele\ncashier";
        assert_eq!(
            fp,
            format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())))
        );
        assert_ne!(restore_fingerprint("kilele", "payables"), fp);
        assert_ne!(restore_fingerprint("agentrix", "cashier"), fp);
    }
}
