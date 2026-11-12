//! Sealed documents: Ed25519 over a context string and the RFC 8785 canonical JSON of a body,
//! made with a key the signer's company publishes in its Web Bot Auth key directory
//! (`/.well-known/http-message-signatures-directory`). One directory identifies a company's
//! agents on the wire (request signatures) and checks what they sign (mandates, invoices,
//! receipts), so the counterparty needs nothing configured out of band.
//!
//! On the wire: `{"body": {…}, "signer": "<origin>", "keyid": "<thumbprint>", "signature":
//! "<base64url>"}`. The body is kept as sent, so its canonical bytes are the signed ones.

use adk_auth::web_bot_auth::{DirectoryKey, Ed25519SigningKey, KeyDirectorySource};
use agentrix_types::{Digest, canonical_json};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a sealed document is. The kind is part of what is signed, so a signature made for one
/// kind never checks as another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealKind {
    /// The payer owner's intent mandate.
    IntentMandate,
    /// The payee owner's collections mandate.
    CollectionsMandate,
    /// A per-invoice (or per-instalment) cart mandate.
    CartMandate,
    /// An invoice offered by the payee's Receivables agent.
    Invoice,
    /// The payee's settlement receipt.
    SettlementReceipt,
    /// An instalment plan, as proposed, countered or agreed.
    InstalmentPlan,
    /// A door's or desk's refusal: what was refused, why, and that nothing moved.
    RefusalReceipt,
}

impl SealKind {
    fn context(self) -> &'static [u8] {
        match self {
            Self::IntentMandate => b"agentrix-saas ap2-style intent mandate v1\n",
            Self::CollectionsMandate => b"agentrix-saas collections mandate v1\n",
            Self::CartMandate => b"agentrix-saas ap2-style cart mandate v1\n",
            Self::Invoice => b"agentrix-saas invoice offer v1\n",
            Self::SettlementReceipt => b"agentrix-saas settlement receipt v1\n",
            Self::InstalmentPlan => b"agentrix-saas instalment plan v1\n",
            Self::RefusalReceipt => b"agentrix-saas refusal receipt v1\n",
        }
    }
}

/// Why a sealed document doesn't check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    /// It isn't the expected document.
    #[error("the {kind} isn't readable: {reason}")]
    Malformed {
        /// What was expected.
        kind: &'static str,
        /// Why not.
        reason: String,
    },
    /// The signature doesn't verify against the signer's published keys.
    #[error("the {kind}'s signature doesn't check against {signer}'s published keys: {reason}")]
    BadSignature {
        /// What was checked.
        kind: &'static str,
        /// The signer's directory origin.
        signer: String,
        /// Why not.
        reason: String,
    },
}

/// A document signed by a key in its signer's key directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sealed {
    /// The document, as signed.
    pub body: Value,
    /// The signer's key directory origin, for example `https://pay.agentrix.website`.
    pub signer: String,
    /// The JWK thumbprint of the signing key.
    pub keyid: String,
    /// Ed25519 over the kind's context and the canonical JSON of `body`, base64url.
    pub signature: String,
}

impl Sealed {
    /// Seals `body` as `kind` with `key`, published at `signer`.
    pub fn seal<T: Serialize>(
        kind: SealKind,
        body: &T,
        signer: &str,
        key: &Ed25519SigningKey,
    ) -> Result<Self, SealError> {
        let body = serde_json::to_value(body).map_err(|e| SealError::Malformed {
            kind: kind_name(kind),
            reason: e.to_string(),
        })?;
        let signature = key.sign(&signed_bytes(kind, &body)?);
        Ok(Self {
            body,
            signer: signer.trim_end_matches('/').to_owned(),
            keyid: key.thumbprint().to_owned(),
            signature: URL_SAFE_NO_PAD.encode(signature),
        })
    }

    /// The body as `T`.
    pub fn read<T: DeserializeOwned>(&self, kind: SealKind) -> Result<T, SealError> {
        serde_json::from_value(self.body.clone()).map_err(|e| SealError::Malformed {
            kind: kind_name(kind),
            reason: e.to_string(),
        })
    }

    /// `sha256:` of the canonical JSON of the whole sealed document: what other documents and
    /// receipts cite.
    pub fn digest(&self) -> Result<Digest, SealError> {
        Ok(Digest::of(&canonical_json(self).map_err(|e| {
            SealError::Malformed {
                kind: "sealed document",
                reason: e.to_string(),
            }
        })?))
    }

    /// Checks the signature with the key `signer`'s directory publishes under `keyid`, and
    /// returns that key (its `kid` says which role the company gave it).
    pub async fn verify(
        &self,
        kind: SealKind,
        directories: &dyn KeyDirectorySource,
    ) -> Result<DirectoryKey, SealError> {
        let bad = |reason: String| SealError::BadSignature {
            kind: kind_name(kind),
            signer: self.signer.clone(),
            reason,
        };
        let fetched = directories
            .fetch(&self.signer)
            .await
            .map_err(|e| bad(e.to_string()))?;
        let key = fetched
            .directory
            .find(&self.keyid)
            .cloned()
            .ok_or_else(|| bad(format!("no key {} is published", self.keyid)))?;
        if !key.is_current(Utc::now().timestamp()) {
            return Err(bad(format!("key {} isn't current", self.keyid)));
        }
        let public = key.public_key().map_err(|e| bad(e.to_string()))?;
        let verifying = ed25519_dalek::VerifyingKey::from_bytes(&public)
            .map_err(|_| bad("the published key isn't a valid Ed25519 point".into()))?;
        let signature: [u8; 64] = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| bad("the signature isn't 64 bytes of base64url".into()))?;
        verifying
            .verify_strict(
                &signed_bytes(kind, &self.body)?,
                &ed25519_dalek::Signature::from_bytes(&signature),
            )
            .map_err(|_| bad("the signature doesn't match the document".into()))?;
        Ok(key)
    }
}

fn kind_name(kind: SealKind) -> &'static str {
    match kind {
        SealKind::IntentMandate => "intent mandate",
        SealKind::CollectionsMandate => "collections mandate",
        SealKind::CartMandate => "cart mandate",
        SealKind::Invoice => "invoice",
        SealKind::SettlementReceipt => "settlement receipt",
        SealKind::InstalmentPlan => "instalment plan",
        SealKind::RefusalReceipt => "refusal receipt",
    }
}

fn signed_bytes(kind: SealKind, body: &Value) -> Result<Vec<u8>, SealError> {
    let mut bytes = kind.context().to_vec();
    bytes.extend(canonical_json(body).map_err(|e| SealError::Malformed {
        kind: kind_name(kind),
        reason: e.to_string(),
    })?);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use adk_auth::web_bot_auth::{InMemoryKeyDirectorySource, KeyDirectory};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn a_seal_checks_and_an_edit_or_another_kind_does_not() {
        let key = Ed25519SigningKey::generate().unwrap();
        let directories = InMemoryKeyDirectorySource::new();
        directories
            .insert(
                "https://kilele.example",
                KeyDirectory::from_keys([&key]),
                None,
            )
            .await;
        let sealed = Sealed::seal(
            SealKind::CartMandate,
            &json!({"invoice_id": "INV-1", "amount": {"currency": "USD", "minor": 100}}),
            "https://kilele.example/",
            &key,
        )
        .unwrap();
        assert_eq!(sealed.signer, "https://kilele.example");
        sealed
            .verify(SealKind::CartMandate, &directories)
            .await
            .unwrap();
        assert!(
            sealed
                .verify(SealKind::Invoice, &directories)
                .await
                .is_err(),
            "a cart mandate's signature never checks as an invoice's"
        );
        let mut edited = sealed.clone();
        edited.body["amount"]["minor"] = 99_999.into();
        assert!(matches!(
            edited.verify(SealKind::CartMandate, &directories).await,
            Err(SealError::BadSignature { .. })
        ));
        let stranger = Ed25519SigningKey::generate().unwrap();
        let forged = Sealed::seal(
            SealKind::CartMandate,
            &sealed.body,
            "https://kilele.example",
            &stranger,
        )
        .unwrap();
        assert!(
            forged
                .verify(SealKind::CartMandate, &directories)
                .await
                .is_err()
        );
    }
}
