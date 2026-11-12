//! Passkeys through webauthn-rs (design D9): user verification required, attestation `none`,
//! the platform's RP id and exact origin, and a counter that only moves forward.
//!
//! Approvals use one extra step. webauthn-rs always picks a random challenge, but an approval's
//! assertion must commit to the effect itself, so that a receipt can be checked offline
//! (02 §8). With `danger-allow-state-serialisation`, the ceremony's challenge is replaced with
//! SHA-256(fingerprint ‖ nonce), in the state and in the options sent to the browser. All of
//! webauthn-rs's own checks (origin, RP id, user verification, signature, counter) still run.
//! adk-surface's tests rebuild ceremonies the same way (`mobile/tests.rs:1327` at the pin).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};
use webauthn_rs::prelude::{
    AuthenticationResult, Base64UrlSafeData, CreationChallengeResponse, CredentialID, Passkey,
    PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url, Uuid, Webauthn, WebauthnBuilder, WebauthnError,
};

/// Nonce bytes mixed into an approval challenge.
pub const NONCE_BYTES: usize = 16;

/// The challenge an approval's assertion signs: SHA-256(fingerprint ‖ nonce) (design D9).
pub fn approval_challenge(fingerprint: &[u8; 32], nonce: &[u8; NONCE_BYTES]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(fingerprint);
    hasher.update(nonce);
    hasher.finalize().into()
}

/// The challenge inside a browser's assertion (`clientDataJSON.challenge`), base64url. Used to
/// find the ceremony an assertion answers; webauthn-rs checks it again against that ceremony.
pub fn challenge_of(credential: &PublicKeyCredential) -> Option<String> {
    let client_data: serde_json::Value =
        serde_json::from_slice(credential.response.client_data_json.as_ref()).ok()?;
    client_data.get("challenge")?.as_str().map(str::to_owned)
}

/// A credential id as text (base64url), for storage.
pub fn credential_id_text(id: &CredentialID) -> String {
    URL_SAFE_NO_PAD.encode(id.as_ref())
}

/// Errors from the passkey layer.
#[derive(Debug, thiserror::Error)]
pub enum PasskeyError {
    /// webauthn-rs refused the ceremony (wrong origin, bad signature, stale counter…).
    #[error("passkey check failed: {0}")]
    Webauthn(#[from] WebauthnError),
    /// The ceremony state could not be rebuilt around the approval challenge.
    #[error("approval ceremony could not be prepared: {0}")]
    State(String),
}

/// The relying party.
#[derive(Debug)]
pub struct Passkeys {
    webauthn: Webauthn,
}

impl Passkeys {
    /// For `rp_id` (the platform's host) and its exact `origin`.
    pub fn new(rp_id: &str, origin: &Url, rp_name: &str) -> Result<Self, PasskeyError> {
        let webauthn = WebauthnBuilder::new(rp_id, origin)?
            .rp_name(rp_name)
            .build()?;
        Ok(Self { webauthn })
    }

    /// Starts registering a passkey for `user`. `existing` are excluded, so one authenticator
    /// is not registered twice.
    pub fn start_registration(
        &self,
        user: Uuid,
        email: &str,
        display_name: &str,
        existing: &[Passkey],
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), PasskeyError> {
        let exclude: Vec<CredentialID> = existing.iter().map(|p| p.cred_id().clone()).collect();
        let exclude = (!exclude.is_empty()).then_some(exclude);
        Ok(self
            .webauthn
            .start_passkey_registration(user, email, display_name, exclude)?)
    }

    /// Finishes a registration.
    pub fn finish_registration(
        &self,
        credential: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> Result<Passkey, PasskeyError> {
        Ok(self
            .webauthn
            .finish_passkey_registration(credential, state)?)
    }

    /// Starts a sign-in with the account's passkeys.
    pub fn start_login(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), PasskeyError> {
        Ok(self.webauthn.start_passkey_authentication(passkeys)?)
    }

    /// Finishes a sign-in or an approval: checks origin, RP id, user verification, signature
    /// and counter, and that the credential is one of the ceremony's.
    pub fn finish(
        &self,
        credential: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> Result<AuthenticationResult, PasskeyError> {
        Ok(self
            .webauthn
            .finish_passkey_authentication(credential, state)?)
    }

    /// Starts an approval whose challenge commits to `fingerprint`. Returns the options for the
    /// page, the state to keep, the nonce for the receipt, and the challenge (base64url) that
    /// keys the pending ceremony.
    pub fn start_approval(
        &self,
        passkeys: &[Passkey],
        fingerprint: &[u8; 32],
    ) -> Result<
        (
            RequestChallengeResponse,
            PasskeyAuthentication,
            [u8; NONCE_BYTES],
            String,
        ),
        PasskeyError,
    > {
        let (mut options, state) = self.webauthn.start_passkey_authentication(passkeys)?;
        let mut nonce = [0u8; NONCE_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let challenge = approval_challenge(fingerprint, &nonce);
        let encoded = URL_SAFE_NO_PAD.encode(challenge);
        let state = with_challenge(&state, &encoded)?;
        options.public_key.challenge = Base64UrlSafeData::from(challenge.to_vec());
        Ok((options, state, nonce, encoded))
    }
}

/// Rebuilds an authentication state around `challenge` (base64url). Fails rather than add a
/// field if webauthn-rs's state ever stops having `ast.challenge`.
fn with_challenge(
    state: &PasskeyAuthentication,
    challenge: &str,
) -> Result<PasskeyAuthentication, PasskeyError> {
    let mut value = serde_json::to_value(state).map_err(|e| PasskeyError::State(e.to_string()))?;
    let slot = value
        .pointer_mut("/ast/challenge")
        .ok_or_else(|| PasskeyError::State("webauthn-rs state has no ast.challenge".into()))?;
    *slot = serde_json::Value::String(challenge.to_owned());
    let rebuilt: PasskeyAuthentication =
        serde_json::from_value(value).map_err(|e| PasskeyError::State(e.to_string()))?;
    // Read it back: the state must now carry exactly this challenge.
    let check = serde_json::to_value(&rebuilt).map_err(|e| PasskeyError::State(e.to_string()))?;
    if check.pointer("/ast/challenge").and_then(|v| v.as_str()) != Some(challenge) {
        return Err(PasskeyError::State("challenge did not round-trip".into()));
    }
    Ok(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_challenge_commits_to_both_parts() {
        let a = approval_challenge(&[1; 32], &[2; NONCE_BYTES]);
        assert_eq!(a, approval_challenge(&[1; 32], &[2; NONCE_BYTES]));
        assert_ne!(a, approval_challenge(&[1; 32], &[3; NONCE_BYTES]));
        assert_ne!(a, approval_challenge(&[9; 32], &[2; NONCE_BYTES]));
    }

    #[test]
    fn relying_party_builds_for_the_platform_origin() {
        let origin = Url::parse("https://agentrix.example").unwrap();
        assert!(Passkeys::new("agentrix.example", &origin, "Agentrix").is_ok());
        let wrong = Url::parse("https://elsewhere.example").unwrap();
        assert!(
            Passkeys::new("agentrix.example", &wrong, "Agentrix").is_err(),
            "origin must be within the RP id"
        );
    }
}
