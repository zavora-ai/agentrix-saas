//! Approvals for `decisions/` (design §4.1, D9; R7.2). The move lane's
//! `POST /decisions/{id}/approve` calls [`Approvals::verify`] with the card's fingerprint:
//! - **passkey:** the console embeds [`Approvals::passkey_options`] in the card as
//!   `card.passkey_options`; the browser returns a webauthn-rs `PublicKeyCredential`, which is
//!   matched to its ceremony by the challenge it signed;
//! - **totp:** for an owner without a passkey, a fresh code from their authenticator app, used
//!   once, bound to this fingerprint by this call.
//!
//! Either way the result is an [`ApprovalProof`] for the decision's receipt, naming the method.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PublicKeyCredential, RequestChallengeResponse,
};

use super::passkey::{challenge_of, credential_id_text};
use super::pending::Pending;
use super::store::now;
use super::{AuthError, AuthState, totp};

/// A started passkey approval, waiting for the browser's assertion.
#[derive(Debug)]
struct PendingApproval {
    account: Uuid,
    fingerprint: [u8; 32],
    nonce: [u8; super::passkey::NONCE_BYTES],
    state: PasskeyAuthentication,
}

/// Live approval ceremonies, keyed by challenge. Ten minutes: the card is re-rendered after.
#[derive(Debug)]
pub(super) struct ApprovalCeremonies(Pending<PendingApproval>);

impl Default for ApprovalCeremonies {
    fn default() -> Self {
        Self(Pending::new(Duration::from_secs(600), 1024))
    }
}

/// What `POST /decisions/{id}/approve` received.
#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "lowercase")]
pub enum ApproveRequest {
    /// A passkey assertion (JSON body).
    Passkey {
        /// The browser's `PublicKeyCredential`, as webauthn-rs JSON.
        credential: Box<PublicKeyCredential>,
    },
    /// An authenticator code (form body; decisions/ checks the posted fingerprint is the card's).
    Totp {
        /// The six digits.
        code: String,
    },
}

/// Evidence for the decision's receipt (design §4.4: the receipt records the method).
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalProof {
    /// `passkey` or `totp`.
    pub method: &'static str,
    /// Who approved.
    pub account: Uuid,
    /// The card's fingerprint, hex.
    pub fingerprint: String,
    /// Unix seconds.
    pub at: i64,
    /// Passkey: the nonce, base64url. challenge = SHA-256(fingerprint ‖ nonce).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// Passkey: the credential id, base64url.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// Passkey: the full assertion (clientDataJSON, authenticatorData, signature), so anyone
    /// can verify it offline against the credential's public key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertion: Option<serde_json::Value>,
    /// TOTP: the time step the code belonged to. A code rests on the platform's attestation,
    /// not the owner's signature, and the receipt says so by its method.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totp_step: Option<i64>,
}

/// Passkey options for one card, for the page to embed.
#[derive(Debug, Serialize)]
pub struct PasskeyOptions {
    /// webauthn-rs `RequestChallengeResponse` JSON, for `navigator.credentials.get`.
    #[serde(flatten)]
    pub options: RequestChallengeResponse,
}

/// The approvals API, a view on [`AuthState`].
#[derive(Debug, Clone)]
pub struct Approvals {
    auth: AuthState,
}

impl AuthState {
    /// The approvals API for `decisions/` and the console.
    pub fn approvals(&self) -> Approvals {
        Approvals { auth: self.clone() }
    }

    pub(super) async fn passkeys_of(
        &self,
        account: Uuid,
    ) -> Result<Vec<(String, Passkey)>, AuthError> {
        self.store
            .credentials(account)
            .await?
            .into_iter()
            .map(|row| {
                serde_json::from_str::<Passkey>(&row.passkey_json)
                    .map(|p| (row.id.clone(), p))
                    .map_err(|e| AuthError::Internal(format!("stored passkey {}: {e}", row.id)))
            })
            .collect()
    }
}

impl Approvals {
    /// Passkey options bound to `fingerprint`, or `None` when the owner has no passkey (the
    /// card then asks for an authenticator code instead).
    pub async fn passkey_options(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
    ) -> Result<Option<PasskeyOptions>, AuthError> {
        let passkeys: Vec<Passkey> = self
            .auth
            .passkeys_of(account)
            .await?
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        if passkeys.is_empty() {
            return Ok(None);
        }
        let (options, state, nonce, challenge) = self
            .auth
            .passkeys()?
            .start_approval(&passkeys, &fingerprint)?;
        self.auth.approvals.0.put_as(
            challenge,
            PendingApproval {
                account,
                fingerprint,
                nonce,
                state,
            },
        );
        Ok(Some(PasskeyOptions { options }))
    }

    /// Checks an approval of the card with `fingerprint` by `account`.
    pub async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        request: ApproveRequest,
    ) -> Result<ApprovalProof, AuthError> {
        match request {
            ApproveRequest::Passkey { credential } => {
                self.verify_passkey(account, fingerprint, &credential).await
            }
            ApproveRequest::Totp { code } => self.verify_totp(account, fingerprint, &code).await,
        }
    }

    async fn verify_passkey(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        credential: &PublicKeyCredential,
    ) -> Result<ApprovalProof, AuthError> {
        let challenge = challenge_of(credential).ok_or(AuthError::ApprovalRefused)?;
        let pending = self
            .auth
            .approvals
            .0
            .take(&challenge)
            .ok_or(AuthError::ApprovalExpired)?;
        if pending.account != account || pending.fingerprint != fingerprint {
            return Err(AuthError::ApprovalRefused);
        }
        let result = self
            .auth
            .passkeys()?
            .finish(credential, &pending.state)
            .map_err(|_| AuthError::ApprovalRefused)?;
        self.auth.record_use(account, &result).await?;
        Ok(ApprovalProof {
            method: "passkey",
            account,
            fingerprint: hex::encode(fingerprint),
            at: now(),
            nonce: Some(base64::Engine::encode(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                pending.nonce,
            )),
            credential_id: Some(credential_id_text(result.cred_id())),
            assertion: serde_json::to_value(credential).ok(),
            totp_step: None,
        })
    }

    async fn verify_totp(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        code: &str,
    ) -> Result<ApprovalProof, AuthError> {
        let step = self.auth.check_totp(account, code).await?;
        Ok(ApprovalProof {
            method: "totp",
            account,
            fingerprint: hex::encode(fingerprint),
            at: now(),
            nonce: None,
            credential_id: None,
            assertion: None,
            totp_step: Some(step),
        })
    }
}

impl AuthState {
    /// Checks a TOTP code for `account` and uses it up. Returns its step.
    pub(super) async fn check_totp(&self, account: Uuid, code: &str) -> Result<i64, AuthError> {
        let (sealed, last_step) = self
            .store
            .totp_seed(account)
            .await?
            .ok_or(AuthError::NoTotp)?;
        let seed = self
            .sealer
            .open(&sealed, account.to_string().as_bytes())
            .ok_or_else(|| {
                AuthError::Internal("TOTP seed does not open with this node's sealing key".into())
            })?;
        let now_unix = u64::try_from(now()).unwrap_or_default();
        let step = totp::verify(
            &seed,
            code,
            now_unix,
            u64::try_from(last_step).unwrap_or_default(),
        )
        .ok_or(AuthError::WrongCode)?;
        let step = i64::try_from(step).map_err(|_| AuthError::WrongCode)?;
        if !self.store.advance_totp(account, step).await? {
            return Err(AuthError::WrongCode);
        }
        Ok(step)
    }

    /// After a passkey check: saves the moved counter and stamps last use.
    pub(super) async fn record_use(
        &self,
        account: Uuid,
        result: &webauthn_rs::prelude::AuthenticationResult,
    ) -> Result<(), AuthError> {
        for (id, mut passkey) in self.passkeys_of(account).await? {
            if passkey.cred_id() == result.cred_id() {
                let changed = passkey.update_credential(result).unwrap_or(false);
                let json = if changed {
                    Some(
                        serde_json::to_string(&passkey)
                            .map_err(|e| AuthError::Internal(e.to_string()))?,
                    )
                } else {
                    None
                };
                self.store.touch_credential(&id, json.as_deref()).await?;
            }
        }
        Ok(())
    }
}
