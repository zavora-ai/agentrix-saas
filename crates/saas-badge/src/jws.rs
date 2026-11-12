//! Signatures on documents: JWS (RFC 7515) with EdDSA (RFC 8037) over the RFC 8785 canonical
//! JSON of the document, detached (RFC 7515 Appendix F) so the document stays readable JSON.
//!
//! - **Documents** (badges, vouchers, bindings) carry a `proof`:
//!   `{"type": "JsonWebSignature2020", "created", "verificationMethod": "<DID URL>",
//!   "proofPurpose": "assertionMethod", "jws": "<header>..<signature>"}`; the signed payload is
//!   the document without `proof`.
//! - **A2A agent cards** carry `signatures: [{"protected", "signature"}]` (A2A v1's
//!   `AgentCardSignature`); the signed payload is the card without `signatures`. Cards are
//!   signed by adk-awp's card signer (`adk_awp::signing::card::sign_agent_card`) with the site's
//!   owner key; [`verify_card`] checks them against the key the `kid` resolves to.

use std::sync::Arc;

use adk_auth::web_bot_auth::Ed25519SigningKey;
use adk_awp::signing::SigningError;
use adk_awp::signing::card::sign_agent_card;
use adk_awp::signing::jws::{DocumentSigner, public_key_jwk};
use agentrix_types::canonical_json;
use async_trait::async_trait;
use awp_types::PublicKeyJwk;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};

/// Why a signature doesn't check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JwsError {
    /// The document has no proof, or the card no signature.
    #[error("it carries no signature")]
    Missing,
    /// The proof or JWS isn't well formed.
    #[error("the signature isn't well formed: {0}")]
    Malformed(String),
    /// Not EdDSA.
    #[error("the signature uses {0}, not EdDSA")]
    Algorithm(String),
    /// The signature doesn't match the content and key.
    #[error("the signature doesn't match the document and the key")]
    Mismatch,
}

fn malformed(e: impl std::fmt::Display) -> JwsError {
    JwsError::Malformed(e.to_string())
}

fn signing_input(header_b64: &str, payload: &Value) -> Result<Vec<u8>, JwsError> {
    let payload = URL_SAFE_NO_PAD.encode(canonical_json(payload).map_err(malformed)?);
    Ok(format!("{header_b64}.{payload}").into_bytes())
}

fn verify_ed25519(public_key: &[u8; 32], message: &[u8], signature: &[u8]) -> Result<(), JwsError> {
    let key = ed25519_dalek::VerifyingKey::from_bytes(public_key).map_err(malformed)?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| JwsError::Malformed("an Ed25519 signature is 64 bytes".into()))?;
    key.verify_strict(message, &ed25519_dalek::Signature::from_bytes(&signature))
        .map_err(|_| JwsError::Mismatch)
}

fn header(kid: &str) -> String {
    URL_SAFE_NO_PAD.encode(json!({ "alg": "EdDSA", "kid": kid }).to_string())
}

fn read_header(header_b64: &str) -> Result<Value, JwsError> {
    let bytes = URL_SAFE_NO_PAD.decode(header_b64).map_err(malformed)?;
    let header: Value = serde_json::from_slice(&bytes).map_err(malformed)?;
    match header["alg"].as_str() {
        Some("EdDSA") => Ok(header),
        Some(other) => Err(JwsError::Algorithm(other.to_owned())),
        None => Err(JwsError::Malformed("no alg".into())),
    }
}

/// Signs `document` (a JSON object) with `key`, named by `verification_method` (a DID URL
/// such as `did:web:<host>#owner`), and returns it with its `proof`.
pub fn sign_document(
    mut document: Value,
    key: &Ed25519SigningKey,
    verification_method: &str,
) -> Result<Value, JwsError> {
    let object = document
        .as_object_mut()
        .ok_or_else(|| JwsError::Malformed("only a JSON object can be signed".into()))?;
    object.remove("proof");
    let header_b64 = header(verification_method);
    let signature = key.sign(&signing_input(&header_b64, &document)?);
    let jws = format!("{header_b64}..{}", URL_SAFE_NO_PAD.encode(signature));
    document["proof"] = json!({
        "type": "JsonWebSignature2020",
        "created": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        "verificationMethod": verification_method,
        "proofPurpose": "assertionMethod",
        "jws": jws,
    });
    Ok(document)
}

/// The DID URL a document's proof names.
pub fn proof_method(document: &Value) -> Option<&str> {
    document["proof"]["verificationMethod"].as_str()
}

/// Checks a document's proof with `public_key` (the key its `verificationMethod` resolves to).
pub fn verify_document(document: &Value, public_key: &[u8; 32]) -> Result<(), JwsError> {
    let jws = document["proof"]["jws"].as_str().ok_or(JwsError::Missing)?;
    let (header_b64, signature_b64) = jws
        .split_once("..")
        .ok_or_else(|| JwsError::Malformed("not a detached JWS".into()))?;
    let header = read_header(header_b64)?;
    if header["kid"].as_str() != proof_method(document) {
        return Err(JwsError::Malformed(
            "the JWS names another key than the proof".into(),
        ));
    }
    let mut payload = document.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("proof");
    }
    let signature = URL_SAFE_NO_PAD.decode(signature_b64).map_err(malformed)?;
    verify_ed25519(
        public_key,
        &signing_input(header_b64, &payload)?,
        &signature,
    )
}

/// The site's owner key as adk-awp's [`DocumentSigner`], with its DID URL as the `kid`.
struct OwnerSigner {
    key: Arc<Ed25519SigningKey>,
    kid: String,
}

#[async_trait]
impl DocumentSigner for OwnerSigner {
    fn kid(&self) -> &str {
        &self.kid
    }

    fn public_key(&self) -> PublicKeyJwk {
        public_key_jwk(self.key.public_key())
    }

    async fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SigningError> {
        Ok(self.key.sign(message).to_vec())
    }
}

/// Signs an A2A agent card with adk-awp's card signer and `key`, its signature's `kid` the
/// owner key's DID URL. Any earlier signature is replaced.
pub async fn sign_card(
    mut card: Value,
    key: Arc<Ed25519SigningKey>,
    kid: &str,
) -> Result<Value, JwsError> {
    card.as_object_mut()
        .ok_or_else(|| JwsError::Malformed("an agent card is a JSON object".into()))?
        .remove("signatures");
    let signer = OwnerSigner {
        key,
        kid: kid.to_owned(),
    };
    sign_agent_card(&card, &signer).await.map_err(malformed)
}

/// The `kid` of a card's first signature.
pub fn card_kid(card: &Value) -> Result<String, JwsError> {
    let protected = card["signatures"][0]["protected"]
        .as_str()
        .ok_or(JwsError::Missing)?;
    read_header(protected)?["kid"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| JwsError::Malformed("no kid".into()))
}

/// Checks a card's first signature with `public_key` (the key its `kid` resolves to).
pub fn verify_card(card: &Value, public_key: &[u8; 32]) -> Result<(), JwsError> {
    let entry = &card["signatures"][0];
    let protected = entry["protected"].as_str().ok_or(JwsError::Missing)?;
    read_header(protected)?;
    let signature = URL_SAFE_NO_PAD
        .decode(entry["signature"].as_str().ok_or(JwsError::Missing)?)
        .map_err(malformed)?;
    let mut payload = card.clone();
    if let Some(object) = payload.as_object_mut() {
        object.remove("signatures");
    }
    verify_ed25519(public_key, &signing_input(protected, &payload)?, &signature)
}

/// `sha256:<hex>` of the canonical JSON of `value`: what one document cites of another.
pub fn digest(value: &Value) -> Result<String, JwsError> {
    Ok(agentrix_types::Digest::of(&canonical_json(value).map_err(malformed)?).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_signs_and_checks_and_an_edit_does_not() {
        let key = Ed25519SigningKey::generate().unwrap();
        let signed = sign_document(
            json!({"id": "did:web:a.example:agents:cashier", "status": "active"}),
            &key,
            "did:web:a.example#owner",
        )
        .unwrap();
        assert_eq!(proof_method(&signed), Some("did:web:a.example#owner"));
        verify_document(&signed, key.public_key()).unwrap();
        let mut edited = signed.clone();
        edited["status"] = json!("frozen");
        assert_eq!(
            verify_document(&edited, key.public_key()),
            Err(JwsError::Mismatch)
        );
        let other = Ed25519SigningKey::generate().unwrap();
        assert!(verify_document(&signed, other.public_key()).is_err());
        assert_eq!(
            verify_document(&json!({"id": "x"}), key.public_key()),
            Err(JwsError::Missing)
        );
    }

    #[tokio::test]
    async fn a_card_signs_as_a2a_expects() {
        let key = Arc::new(Ed25519SigningKey::generate().unwrap());
        let card = json!({"name": "Kilele Bakery agent door", "url": "https://k.example/a2a", "signatures": [{"protected": "x", "signature": "y"}]});
        let signed = sign_card(card, key.clone(), "did:web:k.example#owner")
            .await
            .unwrap();
        assert_eq!(signed["signatures"].as_array().unwrap().len(), 1);
        assert_eq!(card_kid(&signed).unwrap(), "did:web:k.example#owner");
        verify_card(&signed, key.public_key()).unwrap();
        let typed: a2a_protocol_types::AgentCardSignature =
            serde_json::from_value(signed["signatures"][0].clone()).unwrap();
        assert!(!typed.protected.is_empty());
        let mut edited = signed.clone();
        edited["url"] = json!("https://evil.example/a2a");
        assert_eq!(
            verify_card(&edited, key.public_key()),
            Err(JwsError::Mismatch)
        );
    }
}
