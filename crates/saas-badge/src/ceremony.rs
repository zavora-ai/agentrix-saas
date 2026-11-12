//! Owner ceremonies (specs/saas/v3.md, technology 2; face's `face-v3-contracts.md` §1): settle
//! says what a moment signs (its summary and fingerprint) and seals the record; the face runs
//! the passkey (WebAuthn, the challenge, the assertion) and posts back its evidence.
//!
//! `GET /owner/{site}/ceremonies/{moment}/{id}` returns the [`Subject`];
//! `POST` the same with `{"evidence": PasskeyEvidence}` seals it: 200 `{sealed, record,
//! auditSeq}`, 409 if the subject changed since its fingerprint was issued, 410 if it expired.
//!
//! [`PasskeyEvidence`] is checked here too, offline, before anything is sealed: the challenge
//! is SHA-256(fingerprint ‖ nonce), the client data names that challenge and the origin, the
//! authenticator data's RP ID hash is SHA-256(rpId), and the assertion's signature (ES256 or
//! EdDSA) verifies with the credential's public key. The same evidence goes into the sealed
//! record, so anyone can check it again later.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// The face's record of one passkey assertion, checkable offline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyEvidence {
    /// `passkey`.
    pub kind: String,
    /// The WebAuthn RP ID.
    pub rp_id: String,
    /// The page's origin.
    pub origin: String,
    /// The credential, base64url.
    pub credential_id: String,
    /// The credential's public key, a JWK (EC P-256 or OKP Ed25519).
    pub public_key: Value,
    /// `sha256:<hex>`: what the owner approved.
    pub fingerprint: String,
    /// Hex, 16 bytes.
    pub nonce: String,
    /// base64url(SHA-256(fingerprint bytes ‖ nonce bytes)).
    pub challenge: String,
    /// base64url.
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    /// base64url.
    pub authenticator_data: String,
    /// base64url (DER for ES256).
    pub signature: String,
    /// Whether the authenticator verified the user.
    #[serde(default)]
    pub user_verified: bool,
}

/// The evidence kind of a local run's stand-in (an Ed25519 key file on the operator's machine,
/// shaped like a passkey assertion). A desk accepts it only when told to; every record says it.
pub const LOCAL_KIND: &str = "local-owner-key";

/// Why evidence doesn't check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    /// It approves another fingerprint (the subject changed).
    #[error("the passkey approved another fingerprint than this subject's")]
    OtherFingerprint,
    /// Something in it doesn't hold.
    #[error("the passkey evidence doesn't check: {0}")]
    Invalid(String),
}

fn invalid(e: impl std::fmt::Display) -> EvidenceError {
    EvidenceError::Invalid(e.to_string())
}

fn b64(text: &str) -> Result<Vec<u8>, EvidenceError> {
    URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(invalid)
}

/// `sha256:<hex>` of 32 bytes.
pub fn fingerprint_text(fp: &[u8; 32]) -> String {
    format!("sha256:{}", hex::encode(fp))
}

/// The challenge for `fp` and `nonce`: base64url(SHA-256(fp ‖ nonce)).
pub fn challenge_for(fp: &[u8; 32], nonce: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fp);
    hasher.update(nonce);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Checks `evidence` approves `expected` (and, when given, comes from `rp_id`).
pub fn check_evidence(
    evidence: &PasskeyEvidence,
    expected: &[u8; 32],
    rp_id: Option<&str>,
) -> Result<(), EvidenceError> {
    if evidence.kind != "passkey" && evidence.kind != LOCAL_KIND {
        return Err(invalid(format!("{} isn't a passkey", evidence.kind)));
    }
    if evidence.fingerprint != fingerprint_text(expected) {
        return Err(EvidenceError::OtherFingerprint);
    }
    if let Some(rp_id) = rp_id {
        if evidence.rp_id != rp_id {
            return Err(invalid(format!("RP ID {} isn't {rp_id}", evidence.rp_id)));
        }
    }
    let nonce = hex::decode(&evidence.nonce).map_err(invalid)?;
    let challenge = challenge_for(expected, &nonce);
    if evidence.challenge != challenge {
        return Err(invalid("the challenge isn't SHA-256(fingerprint ‖ nonce)"));
    }
    let client_data = b64(&evidence.client_data_json)?;
    let client: Value = serde_json::from_slice(&client_data).map_err(invalid)?;
    if client["type"].as_str() != Some("webauthn.get") {
        return Err(invalid("the client data isn't an assertion"));
    }
    if client["challenge"]
        .as_str()
        .map(|c| c.trim_end_matches('='))
        != Some(challenge.as_str())
    {
        return Err(invalid("the client data names another challenge"));
    }
    if client["origin"].as_str() != Some(evidence.origin.as_str()) {
        return Err(invalid("the client data names another origin"));
    }
    if !origin_fits(&evidence.origin, &evidence.rp_id) {
        return Err(invalid(format!(
            "the origin {} isn't on RP ID {}",
            evidence.origin, evidence.rp_id
        )));
    }
    let authenticator = b64(&evidence.authenticator_data)?;
    if authenticator.len() < 37 {
        return Err(invalid("the authenticator data is too short"));
    }
    if authenticator[..32] != Sha256::digest(evidence.rp_id.as_bytes())[..] {
        return Err(invalid("the authenticator data is for another RP ID"));
    }
    if authenticator[32] & 0x01 == 0 {
        return Err(invalid("the user wasn't present"));
    }
    let mut signed = authenticator.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data));
    let signature = b64(&evidence.signature)?;
    verify_assertion(&evidence.public_key, &signed, &signature)
}

/// WebAuthn's rule: the page's host is the RP ID or a subdomain of it, over HTTPS (HTTP only for
/// `localhost` names). Each console registers its owner's passkeys under its own host, so
/// `kilele-sample.paypal.agentrix.website` (and `kilele.money.localhost` on a laptop).
fn origin_fits(origin: &str, rp_id: &str) -> bool {
    let (secure, rest) = match origin.split_once("://") {
        Some(("https", rest)) => (true, rest),
        Some(("http", rest)) => (false, rest),
        _ => return false,
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    }
    .to_ascii_lowercase();
    let local = host == "localhost" || host.ends_with(".localhost");
    (secure || local)
        && !rp_id.is_empty()
        && (host == rp_id || host.ends_with(&format!(".{rp_id}")))
}

fn verify_assertion(jwk: &Value, message: &[u8], signature: &[u8]) -> Result<(), EvidenceError> {
    match (jwk["kty"].as_str(), jwk["crv"].as_str()) {
        (Some("EC"), Some("P-256")) => {
            let x = b64(jwk["x"].as_str().unwrap_or_default())?;
            let y = b64(jwk["y"].as_str().unwrap_or_default())?;
            if x.len() != 32 || y.len() != 32 {
                return Err(invalid("the P-256 key isn't 32-byte coordinates"));
            }
            let mut point = vec![0x04];
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            aws_lc_rs::signature::UnparsedPublicKey::new(
                &aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1,
                point,
            )
            .verify(message, signature)
            .map_err(|_| invalid("the assertion's signature doesn't verify"))
        }
        (Some("OKP"), Some("Ed25519")) => {
            let x: [u8; 32] = b64(jwk["x"].as_str().unwrap_or_default())?
                .try_into()
                .map_err(|_| invalid("the Ed25519 key isn't 32 bytes"))?;
            let key = ed25519_dalek::VerifyingKey::from_bytes(&x).map_err(invalid)?;
            let sig: [u8; 64] = signature
                .try_into()
                .map_err(|_| invalid("the Ed25519 signature isn't 64 bytes"))?;
            key.verify_strict(message, &ed25519_dalek::Signature::from_bytes(&sig))
                .map_err(|_| invalid("the assertion's signature doesn't verify"))
        }
        _ => Err(invalid("the credential's key isn't ES256 or EdDSA")),
    }
}

/// What a moment asks the owner to approve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subject {
    /// `link-paypal`, `mandate` or `step-up`.
    pub moment: String,
    /// The subject's id (`bind_…`).
    pub id: String,
    /// The site.
    pub site: String,
    /// `pending`, `sealed` or `expired`.
    pub state: String,
    /// One line for the page's heading.
    pub title: String,
    /// What is approved, line by line.
    pub lines: Vec<String>,
    /// `sha256:<hex>`.
    pub fingerprint: String,
    /// After this it can't be approved.
    pub expires_at: DateTime<Utc>,
}

/// A sealed record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedRecord {
    /// Always true.
    pub sealed: bool,
    /// The record, signed with the site's owner key.
    pub record: Value,
    /// Its place in the signed audit log.
    pub audit_seq: u64,
}

/// Why a ceremony didn't seal.
#[derive(Debug, thiserror::Error)]
pub enum CeremonyError {
    /// No such subject.
    #[error("no such subject")]
    NotFound,
    /// The subject changed since its fingerprint was issued (an edit voids it).
    #[error("the subject changed since it was shown; open it again")]
    Changed,
    /// Past its time.
    #[error("the subject expired; start again")]
    Expired,
    /// The evidence doesn't check.
    #[error(transparent)]
    Evidence(EvidenceError),
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<EvidenceError> for CeremonyError {
    fn from(e: EvidenceError) -> Self {
        match e {
            EvidenceError::OtherFingerprint => Self::Changed,
            other => Self::Evidence(other),
        }
    }
}

/// A moment's subjects and their sealing.
#[async_trait]
pub trait Ceremony: Send + Sync {
    /// The subject `id`.
    async fn subject(&self, id: &str) -> Result<Subject, CeremonyError>;
    /// Seals `id` with the owner's evidence.
    async fn seal(
        &self,
        id: &str,
        evidence: PasskeyEvidence,
    ) -> Result<SealedRecord, CeremonyError>;
}

/// Every ceremony on the node, by `(site, moment)`.
#[derive(Clone, Default)]
pub struct Ceremonies {
    by_site: HashMap<(String, String), Arc<dyn Ceremony>>,
}

impl std::fmt::Debug for Ceremonies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ceremonies")
            .field("moments", &self.by_site.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Ceremonies {
    /// None yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// `site`'s `moment` (`link-paypal`, `mandate`, `step-up`) is run by `ceremony`.
    #[must_use]
    pub fn with(mut self, site: &str, moment: &str, ceremony: Arc<dyn Ceremony>) -> Self {
        self.by_site
            .insert((site.to_owned(), moment.to_owned()), ceremony);
        self
    }

    /// The ceremony for `site` and `moment`.
    pub fn get(&self, site: &str, moment: &str) -> Option<Arc<dyn Ceremony>> {
        self.by_site
            .get(&(site.to_owned(), moment.to_owned()))
            .cloned()
    }

    /// `GET`/`POST /owner/{site}/ceremonies/{moment}/{id}`.
    pub fn router(self) -> Router {
        Router::new()
            .route(
                "/owner/{site}/ceremonies/{moment}/{id}",
                get(subject_route).post(seal_route),
            )
            .with_state(Arc::new(self))
    }
}

fn error(status: StatusCode, message: impl std::fmt::Display) -> Response {
    (
        status,
        axum::Json(json!({ "message": message.to_string() })),
    )
        .into_response()
}

fn answer(e: CeremonyError) -> Response {
    let status = match &e {
        CeremonyError::NotFound => StatusCode::NOT_FOUND,
        CeremonyError::Changed => StatusCode::CONFLICT,
        CeremonyError::Expired => StatusCode::GONE,
        CeremonyError::Evidence(_) => StatusCode::FORBIDDEN,
        CeremonyError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error(status, e)
}

async fn subject_route(
    State(ceremonies): State<Arc<Ceremonies>>,
    Path((site, moment, id)): Path<(String, String, String)>,
) -> Response {
    let Some(ceremony) = ceremonies.get(&site, &moment) else {
        return error(StatusCode::NOT_FOUND, "No such ceremony.");
    };
    match ceremony.subject(&id).await {
        Ok(subject) => axum::Json(subject).into_response(),
        Err(e) => answer(e),
    }
}

#[derive(Deserialize)]
struct Posted {
    evidence: PasskeyEvidence,
}

async fn seal_route(
    State(ceremonies): State<Arc<Ceremonies>>,
    Path((site, moment, id)): Path<(String, String, String)>,
    axum::Json(posted): axum::Json<Posted>,
) -> Response {
    let Some(ceremony) = ceremonies.get(&site, &moment) else {
        return error(StatusCode::NOT_FOUND, "No such ceremony.");
    };
    match ceremony.seal(&id, posted.evidence).await {
        Ok(sealed) => axum::Json(sealed).into_response(),
        Err(e) => answer(e),
    }
}

/// Test support: evidence made with an Ed25519 "passkey", the way a browser's assertion is
/// shaped. Only for tests and local runs; the node takes the face's evidence.
pub fn ed25519_evidence(
    key: &adk_auth::web_bot_auth::Ed25519SigningKey,
    fp: &[u8; 32],
    rp_id: &str,
    origin: &str,
    kind: &str,
) -> PasskeyEvidence {
    let nonce = [7u8; 16];
    let challenge = challenge_for(fp, &nonce);
    let client =
        json!({"type": "webauthn.get", "challenge": challenge, "origin": origin}).to_string();
    let mut authenticator = Sha256::digest(rp_id.as_bytes()).to_vec();
    authenticator.push(0x05);
    authenticator.extend_from_slice(&[0, 0, 0, 1]);
    let mut signed = authenticator.clone();
    signed.extend_from_slice(&Sha256::digest(client.as_bytes()));
    let signature = key.sign(&signed);
    PasskeyEvidence {
        kind: kind.into(),
        rp_id: rp_id.into(),
        origin: origin.into(),
        credential_id: URL_SAFE_NO_PAD.encode(&key.public_key()[..16]),
        public_key: json!({"kty": "OKP", "crv": "Ed25519", "x": URL_SAFE_NO_PAD.encode(key.public_key())}),
        fingerprint: fingerprint_text(fp),
        nonce: hex::encode(nonce),
        challenge,
        client_data_json: URL_SAFE_NO_PAD.encode(client),
        authenticator_data: URL_SAFE_NO_PAD.encode(authenticator),
        signature: URL_SAFE_NO_PAD.encode(signature),
        user_verified: true,
    }
}

#[cfg(test)]
mod tests {
    use adk_auth::web_bot_auth::Ed25519SigningKey;

    use super::*;

    #[test]
    fn evidence_checks_offline_and_any_change_breaks_it() {
        let key = Ed25519SigningKey::generate().unwrap();
        let fp = [9u8; 32];
        let evidence = ed25519_evidence(
            &key,
            &fp,
            "agentrix.website",
            "https://agentrix.website",
            "passkey",
        );
        check_evidence(&evidence, &fp, Some("agentrix.website")).unwrap();
        assert_eq!(
            check_evidence(&evidence, &[8u8; 32], None),
            Err(EvidenceError::OtherFingerprint)
        );
        assert!(check_evidence(&evidence, &fp, Some("evil.example")).is_err());
        let mut forged = evidence.clone();
        forged.origin = "https://evil.example".into();
        assert!(check_evidence(&forged, &fp, None).is_err());
        let other = Ed25519SigningKey::generate().unwrap();
        let mut swapped = evidence.clone();
        swapped.public_key = json!({"kty": "OKP", "crv": "Ed25519", "x": URL_SAFE_NO_PAD.encode(other.public_key())});
        assert!(check_evidence(&swapped, &fp, None).is_err());
    }

    #[test]
    fn the_origin_must_be_on_the_rp_id() {
        let key = Ed25519SigningKey::generate().unwrap();
        let fp = [4u8; 32];
        let check = |rp_id: &str, origin: &str| {
            check_evidence(
                &ed25519_evidence(&key, &fp, rp_id, origin, "passkey"),
                &fp,
                None,
            )
        };
        // Each console's own host is its owner's RP ID.
        check(
            "kilele.money.agentrix.website",
            "https://kilele.money.agentrix.website",
        )
        .unwrap();
        check(
            "kilele.money.localhost",
            "http://kilele.money.localhost:18182",
        )
        .unwrap();
        check("agentrix.website", "https://money.agentrix.website").unwrap();
        // Signed correctly, but by a page the RP ID doesn't cover.
        assert!(check("agentrix.website", "https://evil.example").is_err());
        assert!(check("agentrix.website", "https://notagentrix.website").is_err());
        assert!(check("money.agentrix.website", "https://agentrix.website").is_err());
        assert!(check("agentrix.website", "http://agentrix.website").is_err());
        assert!(check("agentrix.website", "agentrix.website").is_err());
    }

    #[test]
    fn an_es256_assertion_verifies() {
        use aws_lc_rs::rand::SystemRandom;
        use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref()).unwrap();
        let public = pair.public_key().as_ref().to_vec();
        let fp = [3u8; 32];
        let nonce = [1u8; 16];
        let challenge = challenge_for(&fp, &nonce);
        let client = json!({"type": "webauthn.get", "challenge": challenge, "origin": "https://agentrix.website"}).to_string();
        let mut authenticator = Sha256::digest(b"agentrix.website").to_vec();
        authenticator.extend_from_slice(&[0x05, 0, 0, 0, 2]);
        let mut signed = authenticator.clone();
        signed.extend_from_slice(&Sha256::digest(client.as_bytes()));
        let signature = pair.sign(&rng, &signed).unwrap();
        let evidence = PasskeyEvidence {
            kind: "passkey".into(),
            rp_id: "agentrix.website".into(),
            origin: "https://agentrix.website".into(),
            credential_id: "cred".into(),
            public_key: json!({"kty": "EC", "crv": "P-256", "x": URL_SAFE_NO_PAD.encode(&public[1..33]), "y": URL_SAFE_NO_PAD.encode(&public[33..65])}),
            fingerprint: fingerprint_text(&fp),
            nonce: hex::encode(nonce),
            challenge,
            client_data_json: URL_SAFE_NO_PAD.encode(&client),
            authenticator_data: URL_SAFE_NO_PAD.encode(&authenticator),
            signature: URL_SAFE_NO_PAD.encode(signature.as_ref()),
            user_verified: true,
        };
        check_evidence(&evidence, &fp, Some("agentrix.website")).unwrap();
    }
}
