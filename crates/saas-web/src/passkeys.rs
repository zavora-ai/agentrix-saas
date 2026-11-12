//! The owners' passkeys: how James (money host) and Wanjiru (Kilele host) approve a decision.
//!
//! The relying party is the console's own host: the RP ID is `money.agentrix.website` on the
//! money host and `kilele.money.agentrix.website` on Kilele's, so each owner's passkey works
//! only on their own console (`money.localhost` and `kilele.money.localhost` locally).
//!
//! An approval's assertion commits to the decision itself: webauthn-rs picks a random
//! challenge, and the ceremony's challenge is replaced with SHA-256(fingerprint ‖ nonce), in the
//! state and in the options sent to the browser, so the signed assertion can be checked
//! offline against the exact call (the same step as agentrix-node's `auth/passkey.rs`, design
//! D9). Every one of webauthn-rs's own checks (origin, RP id, user verification, signature,
//! counter) still runs.
//!
//! Passkeys are public keys; they are kept in memory, or in a JSON file (`SAAS_PASSKEYS_FILE`)
//! so a restart doesn't forget them.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use webauthn_rs::prelude::{
    Base64UrlSafeData, COSEKey, COSEKeyType, CreationChallengeResponse, ECDSACurve, EDDSACurve,
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, RequestChallengeResponse, Url, Uuid, Webauthn, WebauthnBuilder,
};

/// Nonce bytes mixed into an approval challenge.
pub const NONCE_BYTES: usize = 16;

/// The challenge an approval's assertion signs: SHA-256(fingerprint ‖ nonce).
pub fn approval_challenge(fingerprint: &[u8; 32], nonce: &[u8; NONCE_BYTES]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(fingerprint);
    hasher.update(nonce);
    hasher.finalize().into()
}

/// A passkey's public key as a JWK (RFC 7517): EC P-256/384/521 with `x` and `y`, or OKP
/// Ed25519/Ed448 with `x`, all base64url. `Null` for a key a JWK verifier here can't use (RSA).
pub fn public_jwk(key: &COSEKey) -> Value {
    let b64 = |bytes: &[u8]| URL_SAFE_NO_PAD.encode(bytes);
    match &key.key {
        COSEKeyType::EC_EC2(k) => {
            let crv = match k.curve {
                ECDSACurve::SECP256R1 => "P-256",
                ECDSACurve::SECP384R1 => "P-384",
                ECDSACurve::SECP521R1 => "P-521",
            };
            json!({"kty": "EC", "crv": crv, "x": b64(k.x.as_ref()), "y": b64(k.y.as_ref())})
        }
        COSEKeyType::EC_OKP(k) => {
            let crv = match k.curve {
                EDDSACurve::ED25519 => "Ed25519",
                EDDSACurve::ED448 => "Ed448",
            };
            json!({"kty": "OKP", "crv": crv, "x": b64(k.x.as_ref())})
        }
        _ => Value::Null,
    }
}

/// `sha256:<64 hex>` (or bare hex) as 32 bytes.
pub fn fingerprint_bytes(fingerprint: &str) -> Option<[u8; 32]> {
    let hex_part = fingerprint.strip_prefix("sha256:").unwrap_or(fingerprint);
    let bytes = hex::decode(hex_part).ok()?;
    bytes.try_into().ok()
}

/// Why a ceremony failed. The words are safe to show.
#[derive(Debug, thiserror::Error)]
pub enum PasskeyError {
    /// The host can't be a relying party (no host, or an IP address).
    #[error("passkeys need the console's own host name")]
    Host,
    /// webauthn-rs refused (wrong origin, bad signature, stale counter …).
    #[error("the passkey didn't check out")]
    Webauthn(#[from] webauthn_rs::prelude::WebauthnError),
    /// No ceremony waiting for this answer.
    #[error("that passkey request has expired; try again")]
    NoCeremony,
    /// No passkey registered for this console yet.
    #[error("no passkey is set up for this console yet")]
    NoPasskey,
    /// The decision's fingerprint isn't 32 bytes of hex.
    #[error("this decision has no fingerprint to sign")]
    Fingerprint,
    /// The ceremony state couldn't be rebuilt around the approval challenge, or saved.
    #[error("{0}")]
    State(String),
}

/// The relying party for one console host, rebuilt per request from `Host`.
pub struct Party {
    webauthn: Webauthn,
    /// The RP ID: the host without its port.
    pub rp_id: String,
    /// The exact origin the browser reports (`https://host` or `http://host:port` locally).
    pub origin: String,
}

impl Party {
    /// For a request to `host` (the `Host` header). Loopback names and `.localhost` aliases are
    /// served over http with any port; everything else over https.
    pub fn for_host(host: &str) -> Result<Self, PasskeyError> {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        let (name, port) = match host.rsplit_once(':') {
            Some((n, p)) if p.bytes().all(|b| b.is_ascii_digit()) => {
                (n.to_owned(), Some(p.to_owned()))
            }
            _ => (host.clone(), None),
        };
        if name.is_empty() || name.parse::<std::net::IpAddr>().is_ok() || name.starts_with('[') {
            return Err(PasskeyError::Host);
        }
        let local = name == "localhost" || name.ends_with(".localhost");
        let origin = match (local, &port) {
            (true, Some(p)) => format!("http://{name}:{p}"),
            (true, None) => format!("http://{name}"),
            (false, Some(p)) => format!("https://{name}:{p}"),
            (false, None) => format!("https://{name}"),
        };
        let origin_text = origin.clone();
        let origin = Url::parse(&origin).map_err(|_| PasskeyError::Host)?;
        let webauthn = WebauthnBuilder::new(&name, &origin)?
            .rp_name("Agentrix")
            .allow_any_port(local)
            .build()?;
        Ok(Self {
            webauthn,
            rp_id: name,
            origin: origin_text,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Owner {
    user: Option<Uuid>,
    passkeys: Vec<Passkey>,
}

struct Pending {
    company: String,
    decision: String,
    fingerprint: String,
    nonce: [u8; NONCE_BYTES],
    rp_id: String,
    state: PasskeyAuthentication,
}

#[derive(Default)]
struct State {
    /// Per company, keyed by RP ID: a passkey belongs to the host it was made on.
    owners: BTreeMap<String, Owner>,
    registering: HashMap<String, PasskeyRegistration>,
    approving: HashMap<String, Pending>,
}

/// Every console's owner passkeys and the ceremonies in flight.
#[derive(Default)]
pub struct Passkeys {
    state: Mutex<State>,
    file: Option<PathBuf>,
}

fn key(company: &str, rp_id: &str) -> String {
    format!("{company}@{rp_id}")
}

impl Passkeys {
    /// Passkeys kept in memory only.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Passkeys kept in `file` (JSON of public keys), loaded now if it exists.
    pub fn at(file: PathBuf) -> anyhow::Result<Self> {
        let owners = match std::fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            state: Mutex::new(State {
                owners,
                ..State::default()
            }),
            file: Some(file),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, st: &State) -> Result<(), PasskeyError> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let body = serde_json::to_vec_pretty(&st.owners)
            .map_err(|e| PasskeyError::State(e.to_string()))?;
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, body)
            .map_err(|e| PasskeyError::State(format!("saving passkeys: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, file)
            .map_err(|e| PasskeyError::State(format!("saving passkeys: {e}")))
    }

    /// The account of `company`'s owner on this host: the WebAuthn user handle their passkeys
    /// are made under, the same before and after they set one up (made, and kept, on first ask).
    pub fn owner_account(&self, company: &str, rp_id: &str) -> Result<Uuid, PasskeyError> {
        let mut st = self.lock();
        let entry = st.owners.entry(key(company, rp_id)).or_default();
        if let Some(user) = entry.user {
            return Ok(user);
        }
        let user = Uuid::new_v4();
        entry.user = Some(user);
        self.save(&st)?;
        Ok(user)
    }

    /// Whose account `account` is on host `rp_id`: the company.
    pub fn company_of(&self, account: Uuid, rp_id: &str) -> Option<String> {
        self.lock().owners.iter().find_map(|(k, o)| {
            let (company, rp) = k.rsplit_once('@')?;
            (rp == rp_id && o.user == Some(account)).then(|| company.to_owned())
        })
    }

    /// True when `company`'s owner has a passkey on this host.
    pub fn has(&self, company: &str, party: &Party) -> bool {
        self.lock()
            .owners
            .get(&key(company, &party.rp_id))
            .is_some_and(|o| !o.passkeys.is_empty())
    }

    /// Starts registering a passkey for `company`'s owner on this host.
    pub fn start_registration(
        &self,
        company: &str,
        owner: &str,
        party: &Party,
    ) -> Result<CreationChallengeResponse, PasskeyError> {
        let mut st = self.lock();
        let k = key(company, &party.rp_id);
        let entry = st.owners.entry(k.clone()).or_default();
        let user = *entry.user.get_or_insert_with(Uuid::new_v4);
        let exclude: Vec<_> = entry.passkeys.iter().map(|p| p.cred_id().clone()).collect();
        let exclude = (!exclude.is_empty()).then_some(exclude);
        let (options, state) = party.webauthn.start_passkey_registration(
            user,
            &format!("{owner} ({company})"),
            &format!("{owner}, owner"),
            exclude,
        )?;
        st.registering.insert(k, state);
        Ok(options)
    }

    /// Finishes a registration: the owner's passkey is kept for this host.
    pub fn finish_registration(
        &self,
        company: &str,
        credential: &RegisterPublicKeyCredential,
        party: &Party,
    ) -> Result<(), PasskeyError> {
        let mut st = self.lock();
        let k = key(company, &party.rp_id);
        let state = st.registering.remove(&k).ok_or(PasskeyError::NoCeremony)?;
        let passkey = party
            .webauthn
            .finish_passkey_registration(credential, &state)?;
        st.owners.entry(k).or_default().passkeys.push(passkey);
        self.save(&st)
    }

    /// Starts an approval of `decision`, whose assertion will commit to `fingerprint`.
    pub fn start_approval(
        &self,
        company: &str,
        decision: &str,
        fingerprint: &str,
        party: &Party,
    ) -> Result<RequestChallengeResponse, PasskeyError> {
        let fp = fingerprint_bytes(fingerprint).ok_or(PasskeyError::Fingerprint)?;
        let mut st = self.lock();
        let passkeys = st
            .owners
            .get(&key(company, &party.rp_id))
            .map(|o| o.passkeys.clone())
            .filter(|p| !p.is_empty())
            .ok_or(PasskeyError::NoPasskey)?;
        let (mut options, state) = party.webauthn.start_passkey_authentication(&passkeys)?;
        let mut nonce = [0u8; NONCE_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let challenge = approval_challenge(&fp, &nonce);
        let encoded = URL_SAFE_NO_PAD.encode(challenge);
        let state = with_challenge(&state, &encoded)?;
        options.public_key.challenge = Base64UrlSafeData::from(challenge.to_vec());
        st.approving.insert(
            encoded,
            Pending {
                company: company.to_owned(),
                decision: decision.to_owned(),
                fingerprint: fingerprint.to_owned(),
                nonce,
                rp_id: party.rp_id.clone(),
                state,
            },
        );
        Ok(options)
    }

    /// Checks the owner's assertion for `decision` and returns the proof the console keeps:
    /// which passkey signed, over which fingerprint and nonce.
    pub fn finish_approval(
        &self,
        company: &str,
        decision: &str,
        credential: &PublicKeyCredential,
        party: &Party,
    ) -> Result<Value, PasskeyError> {
        let challenge = challenge_of(credential).ok_or(PasskeyError::NoCeremony)?;
        let mut st = self.lock();
        let pending = st
            .approving
            .remove(&challenge)
            .ok_or(PasskeyError::NoCeremony)?;
        if pending.company != company
            || pending.decision != decision
            || pending.rp_id != party.rp_id
        {
            return Err(PasskeyError::NoCeremony);
        }
        let result = party
            .webauthn
            .finish_passkey_authentication(credential, &pending.state)?;
        let mut public_key = Value::Null;
        if let Some(owner) = st.owners.get_mut(&key(company, &party.rp_id)) {
            for p in &mut owner.passkeys {
                p.update_credential(&result);
                if p.cred_id() == result.cred_id() {
                    public_key = public_jwk(p.get_public_key());
                }
            }
        }
        self.save(&st)?;
        let b64 = |bytes: &[u8]| URL_SAFE_NO_PAD.encode(bytes);
        // Everything an outsider needs to check the approval offline (saas-badge's
        // `PasskeyEvidence`): the challenge is SHA-256(fingerprint bytes ‖ nonce),
        // clientDataJSON names it and the origin, the authenticator data's rpIdHash is
        // SHA-256(rpId), and the signature over authenticatorData ‖ SHA-256(clientDataJSON)
        // verifies with the public key (a JWK).
        Ok(json!({
            "kind": "passkey",
            "rpId": pending.rp_id,
            // The origin the browser signed, which webauthn-rs has just checked against this
            // relying party (any port on a `.localhost` alias): an offline check compares the
            // evidence's origin with clientDataJSON's exactly.
            "origin": client_data_field(credential, "origin").unwrap_or_else(|| party.origin.clone()),
            "credential": b64(result.cred_id().as_ref()),
            "credentialId": b64(result.cred_id().as_ref()),
            "publicKey": public_key,
            "fingerprint": pending.fingerprint,
            "nonce": hex::encode(pending.nonce),
            "challenge": challenge,
            "clientDataJSON": b64(credential.response.client_data_json.as_ref()),
            "authenticatorData": b64(credential.response.authenticator_data.as_ref()),
            "signature": b64(credential.response.signature.as_ref()),
            "userVerified": result.user_verified(),
        }))
    }
}

/// The challenge inside a browser's assertion (`clientDataJSON.challenge`), base64url.
fn challenge_of(credential: &PublicKeyCredential) -> Option<String> {
    client_data_field(credential, "challenge")
}

/// One string field of the browser's `clientDataJSON`.
fn client_data_field(credential: &PublicKeyCredential, key: &str) -> Option<String> {
    let client_data: Value =
        serde_json::from_slice(credential.response.client_data_json.as_ref()).ok()?;
    client_data.get(key)?.as_str().map(str::to_owned)
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
    *slot = Value::String(challenge.to_owned());
    let rebuilt: PasskeyAuthentication =
        serde_json::from_value(value).map_err(|e| PasskeyError::State(e.to_string()))?;
    let check = serde_json::to_value(&rebuilt).map_err(|e| PasskeyError::State(e.to_string()))?;
    if check.pointer("/ast/challenge").and_then(Value::as_str) != Some(challenge) {
        return Err(PasskeyError::State("challenge did not round-trip".into()));
    }
    Ok(rebuilt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_evidence_names_the_origin_the_browser_signed() {
        let client = json!({"type": "webauthn.get", "challenge": "Y2g", "origin": "http://kilele.money.localhost:5173"});
        let credential: PublicKeyCredential = serde_json::from_value(json!({
            "id": "AQ",
            "rawId": "AQ",
            "type": "public-key",
            "response": {
                "authenticatorData": "AQ",
                "clientDataJSON": URL_SAFE_NO_PAD.encode(client.to_string()),
                "signature": "AQ",
                "userHandle": null
            },
            "extensions": {}
        }))
        .unwrap();
        assert_eq!(
            client_data_field(&credential, "origin").as_deref(),
            Some("http://kilele.money.localhost:5173")
        );
        assert_eq!(challenge_of(&credential).as_deref(), Some("Y2g"));
    }

    #[test]
    fn public_keys_are_jwks() {
        use webauthn_rs::prelude::{COSEAlgorithm, COSEEC2Key, COSEOKPKey};
        let ec = COSEKey {
            type_: COSEAlgorithm::ES256,
            key: COSEKeyType::EC_EC2(COSEEC2Key {
                curve: ECDSACurve::SECP256R1,
                x: vec![1u8; 32].into(),
                y: vec![2u8; 32].into(),
            }),
        };
        let jwk = public_jwk(&ec);
        assert_eq!(
            (jwk["kty"].as_str(), jwk["crv"].as_str()),
            (Some("EC"), Some("P-256"))
        );
        assert_eq!(
            URL_SAFE_NO_PAD.decode(jwk["y"].as_str().unwrap()).unwrap(),
            vec![2u8; 32]
        );
        let ed = COSEKey {
            type_: COSEAlgorithm::EDDSA,
            key: COSEKeyType::EC_OKP(COSEOKPKey {
                curve: EDDSACurve::ED25519,
                x: vec![3u8; 32].into(),
            }),
        };
        assert_eq!(
            public_jwk(&ed),
            json!({"kty": "OKP", "crv": "Ed25519", "x": URL_SAFE_NO_PAD.encode([3u8; 32])})
        );
    }

    #[test]
    fn the_rp_id_is_the_console_host() {
        assert_eq!(
            Party::for_host("money.agentrix.website").unwrap().rp_id,
            "money.agentrix.website"
        );
        assert_eq!(
            Party::for_host("kilele.money.agentrix.website:443")
                .unwrap()
                .rp_id,
            "kilele.money.agentrix.website"
        );
        assert_eq!(
            Party::for_host("kilele.money.localhost:18183")
                .unwrap()
                .rp_id,
            "kilele.money.localhost"
        );
        assert!(Party::for_host("127.0.0.1:18183").is_err());
        assert!(Party::for_host("").is_err());
    }

    #[test]
    fn an_approval_challenge_commits_to_the_fingerprint_and_the_nonce() {
        let a = approval_challenge(&[1; 32], &[2; NONCE_BYTES]);
        assert_eq!(a, approval_challenge(&[1; 32], &[2; NONCE_BYTES]));
        assert_ne!(a, approval_challenge(&[1; 32], &[3; NONCE_BYTES]));
        assert_ne!(a, approval_challenge(&[9; 32], &[2; NONCE_BYTES]));
        assert!(fingerprint_bytes(&format!("sha256:{}", "ab".repeat(32))).is_some());
        assert!(fingerprint_bytes("sha256:abc").is_none());
    }

    #[test]
    fn registration_options_carry_the_host_and_approvals_need_a_passkey() {
        let party = Party::for_host("money.localhost:18183").unwrap();
        let keys = Passkeys::in_memory();
        let options = serde_json::to_value(
            keys.start_registration("agentrix", "James", &party)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(options["publicKey"]["rp"]["id"], "money.localhost");
        assert!(!keys.has("agentrix", &party));
        let fp = format!("sha256:{}", "cd".repeat(32));
        assert!(matches!(
            keys.start_approval("agentrix", "dec-1", &fp, &party),
            Err(PasskeyError::NoPasskey)
        ));
    }
}
