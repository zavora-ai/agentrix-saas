//! The signed audit log (adk-auth `signed-audit`): every decision, verifiable offline.
//!
//! The governed path writes each call's decision here (allowed, denied, held), each approval,
//! and each freeze refusal; the company adds what it executed. Every record is checkpointed as
//! it is written (`checkpoint_every(1)`), so a killed process leaves no unsigned tail.
//!
//! - [`SignedAudit::receipt`] gives one record as a [`SignedReceipt`]: the record, its RFC 9162
//!   inclusion proof and the signed checkpoint it is under. Anyone with the verifier key checks
//!   it alone ([`SignedReceipt::verify`]), for example the denial of a payment to a changed
//!   payee.
//! - [`SignedAudit::write_export`] writes the whole log as JSON lines with the verifier key
//!   beside it, for `adk-rust audit verify audit.jsonl --key "$(cat audit.vkey)"`.
//!
//! The log is a runtime guarantee, not books: the books are Zavora ERA.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adk_auth::signed_audit::{
    AuditExport, AuditLog, AuditSigner, CheckpointBody, Ed25519Signer, Note, NoteVerifier,
    VerifyReport, merkle,
};
use adk_core::audit_log::{
    AuditCheckpoint, AuditDecision, AuditHash, AuditLogStore, AuditRecord, AuditRecordDraft,
    AuditTrail, InMemoryAuditLogStore,
};
use serde::{Deserialize, Serialize};

/// A company's signed audit log.
pub struct SignedAudit {
    log: Arc<AuditLog>,
    store: Arc<dyn AuditLogStore>,
    log_id: String,
    verifier: String,
}

impl std::fmt::Debug for SignedAudit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedAudit")
            .field("log_id", &self.log_id)
            .field("verifier", &self.verifier)
            .finish()
    }
}

/// The key name a company's audit checkpoints are signed under.
pub fn key_name(company: &str) -> String {
    format!("audit.{company}.money.agentrix.website")
}

impl SignedAudit {
    /// Opens log `log_id` in `store`, signing with `signer`.
    pub async fn open(
        store: Arc<dyn AuditLogStore>,
        log_id: &str,
        signer: Ed25519Signer,
    ) -> anyhow::Result<Self> {
        let verifier = signer.verifier_key();
        let log = AuditLog::builder(store.clone(), log_id, Arc::new(signer))
            .checkpoint_every(1)
            .open()
            .await?;
        Ok(Self {
            log: Arc::new(log),
            store,
            log_id: log_id.to_owned(),
            verifier,
        })
    }

    /// A log in memory signed with a key derived from `seed`: tests and judge-mode takes,
    /// where the same seed gives the same verifier key.
    pub async fn seeded(company: &str, seed: u64) -> anyhow::Result<Self> {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&seed.to_be_bytes());
        bytes[8..8 + company.len().min(24)]
            .copy_from_slice(&company.as_bytes()[..company.len().min(24)]);
        let signer = Ed25519Signer::from_seed(key_name(company), bytes)?;
        Self::open(
            Arc::new(InMemoryAuditLogStore::new()),
            &format!("{company}-money"),
            signer,
        )
        .await
    }

    /// Opens `log_id` in `store`, signing with the key in `key_file`, made (mode 0600) when it
    /// doesn't exist yet. Keep the key outside the repository.
    pub async fn with_key_file(
        store: Arc<dyn AuditLogStore>,
        company: &str,
        key_file: &Path,
    ) -> anyhow::Result<Self> {
        let signer = match std::fs::read_to_string(key_file) {
            Ok(text) => Ed25519Signer::from_note_key(text.trim())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let signer = Ed25519Signer::generate(key_name(company))?;
                write_secret(key_file, signer.to_note_key().as_str())?;
                signer
            }
            Err(e) => return Err(e.into()),
        };
        Self::open(store, &format!("{company}-money"), signer).await
    }

    /// The trail the governed path records through (`RunConfig::audit`).
    pub fn trail(&self) -> AuditTrail {
        AuditTrail::new(self.log.clone())
    }

    /// The log itself, for a runner's `audit_log`.
    pub fn log(&self) -> Arc<AuditLog> {
        self.log.clone()
    }

    /// The key that verifies the log's checkpoints, `<name>+<hash>+<base64>`.
    pub fn verifier_key(&self) -> &str {
        &self.verifier
    }

    /// The parsed verifier.
    pub fn verifier(&self) -> anyhow::Result<NoteVerifier> {
        self.verifier
            .parse()
            .map_err(|e| anyhow::anyhow!("verifier key: {e:?}"))
    }

    /// The store the log is kept in (for readers such as the operators' activity page).
    pub fn store(&self) -> Arc<dyn AuditLogStore> {
        self.store.clone()
    }

    /// The log's id.
    pub fn log_id(&self) -> &str {
        &self.log_id
    }

    /// Appends one record the company writes itself (executed, a founder's decision).
    pub async fn record(&self, draft: AuditRecordDraft) -> anyhow::Result<AuditRecord> {
        Ok(self.log.append(draft).await?)
    }

    /// A digest of `value`, as the log's records cite arguments.
    pub fn digest(&self, value: &serde_json::Value) -> String {
        self.trail().digest(value)
    }

    /// The log and its checkpoints, as `adk-rust audit verify` reads them.
    pub async fn export(&self) -> anyhow::Result<AuditExport> {
        if self.log.size().await > 0 {
            self.log.checkpoint().await?;
        }
        Ok(AuditExport::from_store(self.store.as_ref(), &self.log_id).await?)
    }

    /// Every record, oldest first.
    pub async fn records(&self) -> anyhow::Result<Vec<AuditRecord>> {
        let export = self.export().await?;
        export
            .entries
            .iter()
            .filter_map(|e| e.record.as_deref())
            .map(|text| Ok(serde_json::from_str(text)?))
            .collect()
    }

    /// The newest record of `action` with `decision`.
    pub async fn last(
        &self,
        action: &str,
        decision: AuditDecision,
    ) -> anyhow::Result<Option<AuditRecord>> {
        Ok(self
            .records()
            .await?
            .into_iter()
            .rev()
            .find(|r| r.action == action && r.decision == Some(decision)))
    }

    /// Verifies the whole log against its own key, as `adk-rust audit verify` would.
    pub async fn verify(&self) -> anyhow::Result<VerifyReport> {
        Ok(self.export().await?.verify(&[self.verifier()?]))
    }

    /// Record `seq` as a receipt anyone with the verifier key can check alone.
    pub async fn receipt(&self, seq: u64) -> anyhow::Result<SignedReceipt> {
        let export = self.export().await?;
        let entry = export
            .entries
            .iter()
            .find(|e| e.seq == seq)
            .ok_or_else(|| anyhow::anyhow!("no record {seq} in {}", self.log_id))?;
        let record = entry
            .record
            .clone()
            .ok_or_else(|| anyhow::anyhow!("record {seq} was redacted"))?;
        let checkpoint = export
            .checkpoints
            .iter()
            .filter(|c| c.size > seq)
            .max_by_key(|c| c.size)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no checkpoint covers record {seq}"))?;
        let proof = self.log.inclusion_proof(seq, checkpoint.size).await?;
        Ok(SignedReceipt {
            log_id: self.log_id.clone(),
            seq,
            record,
            leaf_hash: entry.leaf_hash,
            proof,
            checkpoint,
            verifier_key: self.verifier.clone(),
        })
    }

    /// Writes `audit.jsonl` and `audit.vkey` into `dir` and returns their paths.
    pub async fn write_export(&self, dir: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
        self.write_export_as(dir, "audit").await
    }

    /// Writes `<stem>.jsonl` and `<stem>.vkey` into `dir` and returns their paths.
    pub async fn write_export_as(
        &self,
        dir: &Path,
        stem: &str,
    ) -> anyhow::Result<(PathBuf, PathBuf)> {
        std::fs::create_dir_all(dir)?;
        let jsonl = dir.join(format!("{stem}.jsonl"));
        let vkey = dir.join(format!("{stem}.vkey"));
        std::fs::write(&jsonl, self.export().await?.to_jsonl())?;
        std::fs::write(&vkey, format!("{}\n", self.verifier))?;
        Ok((jsonl, vkey))
    }
}

/// One audit record, provable on its own: the record, its inclusion proof, and the signed
/// checkpoint whose root the proof reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedReceipt {
    /// The log.
    pub log_id: String,
    /// The record's position.
    pub seq: u64,
    /// The record, exactly as hashed into the log.
    pub record: String,
    /// Its leaf hash.
    pub leaf_hash: AuditHash,
    /// The RFC 9162 inclusion proof from the leaf to the checkpoint's root.
    pub proof: Vec<AuditHash>,
    /// The signed checkpoint.
    pub checkpoint: AuditCheckpoint,
    /// The key the checkpoint is signed with, as the log publishes it.
    pub verifier_key: String,
}

impl SignedReceipt {
    /// The record.
    pub fn record(&self) -> anyhow::Result<AuditRecord> {
        Ok(serde_json::from_str(&self.record)?)
    }

    /// Checks the receipt against `key` (the key the log publishes, not the one the receipt
    /// carries): the record hashes to its leaf, the leaf is in the tree, and the tree's root is
    /// the one `key` signed.
    pub fn verify(&self, key: &NoteVerifier) -> anyhow::Result<()> {
        anyhow::ensure!(
            merkle::leaf_hash(self.record.as_bytes()) == self.leaf_hash,
            "the record doesn't hash to its leaf"
        );
        merkle::verify_inclusion(
            self.seq,
            self.checkpoint.size,
            &self.leaf_hash,
            &self.proof,
            &self.checkpoint.root_hash,
        )
        .map_err(|e| anyhow::anyhow!("inclusion proof: {e:?}"))?;
        let note = Note::parse(&self.checkpoint.note).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        anyhow::ensure!(
            !note.verified_by(std::slice::from_ref(key)).is_empty(),
            "the checkpoint isn't signed by that key"
        );
        let body = CheckpointBody::parse(&note.text).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        anyhow::ensure!(
            body.size == self.checkpoint.size && body.root_hash == self.checkpoint.root_hash,
            "the signed checkpoint is for another tree"
        );
        Ok(())
    }
}

fn write_secret(path: &Path, contents: &str) -> anyhow::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Keeps [`AuditSigner`] in scope for callers that bring their own signer.
pub type DynSigner = Arc<dyn AuditSigner>;

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::audit_log::{AuditActor, AuditRecordKind};
    use adk_core::{ActingContext, AgentId, AgentPrincipal};

    #[tokio::test]
    async fn a_receipt_verifies_alone_and_a_flipped_byte_fails_it() {
        let audit = SignedAudit::seeded("agentrix", 7).await.unwrap();
        let acting = ActingContext::new(AgentPrincipal::new(
            AgentId::new("urn:adk:agent:agentrix/payables").unwrap(),
        ));
        for n in 0..3 {
            audit
                .record(
                    AuditRecordDraft::new(AuditRecordKind::ToolCall, format!("step_{n}"))
                        .actor(AuditActor::new(&acting, "inv-1"))
                        .decision(AuditDecision::Allowed),
                )
                .await
                .unwrap();
        }
        let denied = audit
            .record(
                AuditRecordDraft::new(AuditRecordKind::ToolCall, "pay_bill")
                    .actor(AuditActor::new(&acting, "inv-1"))
                    .decision(AuditDecision::Denied),
            )
            .await
            .unwrap();
        let receipt = audit.receipt(denied.seq).await.unwrap();
        let key = audit.verifier().unwrap();
        receipt.verify(&key).unwrap();
        assert_eq!(receipt.record().unwrap().action, "pay_bill");
        assert!(audit.verify().await.unwrap().is_valid());

        let mut forged = receipt.clone();
        forged.record = forged
            .record
            .replace("Denied", "Allowed")
            .replace("denied", "allowed");
        assert!(forged.verify(&key).is_err());
        let other = SignedAudit::seeded("kilele", 7).await.unwrap();
        assert!(receipt.verify(&other.verifier().unwrap()).is_err());
    }
}
