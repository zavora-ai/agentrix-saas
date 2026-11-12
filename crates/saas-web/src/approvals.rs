//! [`HostApprovals`]: the node's `agentrix_host::Approvals` over the owners' console passkeys,
//! so a ceremony another crate runs (saas-badge's Link PayPal desk, settle's mandate) asks the
//! same passkey the owner set up on their console and gets back proof anyone can check offline.
//!
//! An account is a console owner: `company`'s WebAuthn user handle on this console's host
//! ([`HostApprovals::account`]). The challenge is SHA-256(fingerprint ‖ nonce), as on every
//! moment, and the proof has agentrix-node's `ApprovalProof` shape (`method`, `account`,
//! `fingerprint` hex, `at` unix seconds, `nonce` and `credential_id` base64url, `assertion`).

use std::sync::Arc;

use agentrix_host::Approvals;
use anyhow::{anyhow, bail};
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use uuid::Uuid;
use webauthn_rs::prelude::PublicKeyCredential;

use crate::passkeys::{Party, Passkeys};

/// The owners' console passkeys as `agentrix_host::Approvals`, for one console host.
pub struct HostApprovals {
    passkeys: Arc<Passkeys>,
    host: String,
}

/// The decision key a fingerprint's ceremony is filed under.
fn decision(fingerprint: &[u8; 32]) -> String {
    format!("approval:{}", hex::encode(fingerprint))
}

impl HostApprovals {
    /// Approvals on `host`, the console's host as the browser sees it (`kilele.money.agentrix.website`,
    /// or `kilele.money.localhost:18183` locally): the passkey's relying party.
    pub fn new(passkeys: Arc<Passkeys>, host: &str) -> anyhow::Result<Self> {
        Party::for_host(host).map_err(|e| anyhow!("{host}: {e}"))?;
        Ok(Self {
            passkeys,
            host: host.to_owned(),
        })
    }

    fn party(&self) -> anyhow::Result<Party> {
        Party::for_host(&self.host).map_err(|e| anyhow!("{}: {e}", self.host))
    }

    /// The account of `company`'s owner on this console: pass it to `passkey_options` and
    /// `verify`.
    pub fn account(&self, company: &str) -> anyhow::Result<Uuid> {
        let party = self.party()?;
        Ok(self.passkeys.owner_account(company, &party.rp_id)?)
    }

    fn company(&self, account: Uuid, party: &Party) -> anyhow::Result<String> {
        self.passkeys
            .company_of(account, &party.rp_id)
            .ok_or_else(|| anyhow!("no owner with that account on {}", party.rp_id))
    }
}

/// The credential in an approve body: `{"method": "passkey", "credential": …}` or
/// `{"passkey": …}`. An authenticator code is refused: these consoles approve with passkeys.
fn credential_of(request: Value) -> anyhow::Result<PublicKeyCredential> {
    let credential = match request {
        Value::Object(mut o) => match o.get("method").and_then(Value::as_str) {
            Some("passkey") => o.remove("credential"),
            Some(other) => bail!("this console approves with a passkey, not {other}"),
            None if o.contains_key("totp") => {
                bail!("this console approves with a passkey, not an authenticator code")
            }
            None => o.remove("passkey"),
        },
        _ => None,
    }
    .ok_or_else(|| anyhow!("sign with your passkey"))?;
    serde_json::from_value(credential).map_err(|_| anyhow!("that isn't a passkey assertion"))
}

#[async_trait]
impl Approvals for HostApprovals {
    async fn passkey_options(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<Value>> {
        let party = self.party()?;
        let company = self.company(account, &party)?;
        if !self.passkeys.has(&company, &party) {
            return Ok(None);
        }
        let options = self.passkeys.start_approval(
            &company,
            &decision(&fingerprint),
            &format!("sha256:{}", hex::encode(fingerprint)),
            &party,
        )?;
        Ok(Some(serde_json::to_value(options)?))
    }

    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        request: Value,
    ) -> anyhow::Result<Value> {
        let party = self.party()?;
        let company = self.company(account, &party)?;
        let credential = credential_of(request)?;
        let evidence = self.passkeys.finish_approval(
            &company,
            &decision(&fingerprint),
            &credential,
            &party,
        )?;
        let want = format!("sha256:{}", hex::encode(fingerprint));
        if evidence.get("fingerprint").and_then(Value::as_str) != Some(want.as_str()) {
            bail!("the passkey signed something else");
        }
        let nonce = evidence
            .get("nonce")
            .and_then(Value::as_str)
            .and_then(|h| hex::decode(h).ok())
            .map(|b| URL_SAFE_NO_PAD.encode(b));
        Ok(json!({
            "method": "passkey",
            "account": account,
            "fingerprint": hex::encode(fingerprint),
            "at": chrono::Utc::now().timestamp(),
            "nonce": nonce,
            "credential_id": evidence.get("credentialId"),
            "assertion": evidence,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_owner_without_a_passkey_gets_no_options_and_nothing_verifies() {
        let passkeys = Arc::new(Passkeys::in_memory());
        let a = HostApprovals::new(Arc::clone(&passkeys), "kilele.money.localhost:18183").unwrap();
        let account = a.account("kilele").unwrap();
        assert_eq!(
            a.account("kilele").unwrap(),
            account,
            "the same account every time"
        );
        assert_ne!(a.account("agentrix").unwrap(), account);
        assert_eq!(a.passkey_options(account, [7; 32]).await.unwrap(), None);
        assert!(
            a.passkey_options(Uuid::new_v4(), [7; 32]).await.is_err(),
            "not an owner here"
        );
        let refused = a
            .verify(
                account,
                [7; 32],
                json!({"method": "totp", "code": "123456"}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("passkey"), "{refused}");
        assert!(
            a.verify(account, [7; 32], json!({"passkey": {"id": "x"}}))
                .await
                .is_err()
        );
        // Another host is another relying party: the same owner is someone else there.
        let public = HostApprovals::new(passkeys, "kilele.money.agentrix.website").unwrap();
        assert!(public.passkey_options(account, [7; 32]).await.is_err());
        assert!(HostApprovals::new(Arc::new(Passkeys::in_memory()), "127.0.0.1:8080").is_err());
    }

    #[test]
    fn approve_bodies_in_either_form() {
        assert!(credential_of(json!({"method": "passkey"})).is_err());
        assert!(
            credential_of(json!({"totp": "123456"}))
                .unwrap_err()
                .to_string()
                .contains("authenticator code")
        );
        assert!(credential_of(json!("x")).is_err());
    }
}
