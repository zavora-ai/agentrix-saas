//! Ed25519 signing over RFC 8785 canonical JSON (design §4.4, D6).
//!
//! Only control holds a [`Signer`]. The runner and anyone checking a receipt hold the public
//! key, published as a [`KeySet`] at `/.well-known/agentrix/keys`.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::digest::Digest;
use crate::error::{Error, Result};

/// The RFC 8785 canonical JSON of `value`: the exact bytes every signature and fingerprint
/// covers.
pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_jcs::to_vec(value)?)
}

/// Names a signing key: `ed25519:` and the first 16 hex characters of the SHA-256 of its
/// public key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyId(String);

impl KeyId {
    /// The id of `key`.
    pub fn of(key: &VerifyingKey) -> Self {
        let hex = Digest::of(key.as_bytes()).hex();
        Self(format!("ed25519:{}", &hex[..16]))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A placeholder that [`crate::Receipt::sign`] always replaces.
    pub(crate) fn unset() -> Self {
        Self(String::new())
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An Ed25519 signature, written as 86 characters of unpadded base64url.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature(ed25519_dalek::Signature);

impl Signature {
    /// The raw 64 bytes.
    pub fn to_bytes(&self) -> [u8; 64] {
        self.0.to_bytes()
    }

    /// Wraps raw signature bytes.
    pub fn from_bytes(bytes: &[u8; 64]) -> Self {
        Self(ed25519_dalek::Signature::from_bytes(bytes))
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&URL_SAFE_NO_PAD.encode(self.0.to_bytes()))
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({self})")
    }
}

impl std::str::FromStr for Signature {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|_| Error::BadSignatureEncoding)?;
        let bytes: [u8; 64] = bytes.try_into().map_err(|_| Error::BadSignatureEncoding)?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Control's signing key. Its `Debug` output never shows the secret.
pub struct Signer {
    key: SigningKey,
    key_id: KeyId,
}

impl Signer {
    /// A new random key from the operating system's generator.
    pub fn generate() -> Self {
        Self::from_signing_key(SigningKey::generate(&mut rand::rngs::OsRng))
    }

    /// Loads a key from its 32 secret bytes.
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        Self::from_signing_key(SigningKey::from_bytes(bytes))
    }

    /// Wraps an existing key.
    pub fn from_signing_key(key: SigningKey) -> Self {
        let key_id = KeyId::of(&key.verifying_key());
        Self { key, key_id }
    }

    /// The 32 secret bytes, for sealing into storage. Never log them.
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    /// This key's id.
    pub fn key_id(&self) -> &KeyId {
        &self.key_id
    }

    /// The public half.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    /// The public half as published at `/.well-known/agentrix/keys`.
    pub fn published_key(&self) -> PublishedKey {
        PublishedKey::of(&self.verifying_key())
    }

    /// Signs raw bytes.
    pub fn sign_bytes(&self, message: &[u8]) -> Signature {
        Signature(self.key.sign(message))
    }

    /// Signs the canonical JSON of `value`.
    pub fn sign<T: Serialize + ?Sized>(&self, value: &T) -> Result<Signature> {
        Ok(self.sign_bytes(&canonical_json(value)?))
    }
}

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signer")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

/// Checks `signature` over the canonical JSON of `value`, recomputing the bytes from the value.
pub fn verify<T: Serialize + ?Sized>(
    key: &VerifyingKey,
    value: &T,
    signature: &Signature,
) -> Result<()> {
    let bytes = canonical_json(value)?;
    key.verify_strict(&bytes, &signature.0)
        .map_err(|_| Error::BadSignature)
}

/// One published verification key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedKey {
    /// The key's id, as receipts name it in `issuer`.
    pub key_id: KeyId,
    /// Always `Ed25519`.
    pub alg: String,
    /// The 32-byte public key, unpadded base64url.
    pub public_key: String,
}

impl PublishedKey {
    /// The published form of `key`.
    pub fn of(key: &VerifyingKey) -> Self {
        Self {
            key_id: KeyId::of(key),
            alg: "Ed25519".to_owned(),
            public_key: URL_SAFE_NO_PAD.encode(key.as_bytes()),
        }
    }

    /// Decodes the key and checks that it matches its id.
    pub fn verifying_key(&self) -> Result<VerifyingKey> {
        if self.alg != "Ed25519" {
            return Err(Error::BadKey(format!("unsupported alg {:?}", self.alg)));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.public_key)
            .map_err(|_| Error::BadKey("public_key is not base64url".to_owned()))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::BadKey("public_key is not 32 bytes".to_owned()))?;
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|_| Error::BadKey("public_key is not a valid point".to_owned()))?;
        if KeyId::of(&key) != self.key_id {
            return Err(Error::BadKey(format!(
                "{} does not match its key",
                self.key_id
            )));
        }
        Ok(key)
    }
}

/// The document at `/.well-known/agentrix/keys`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySet {
    /// Current and retired keys. Retired keys stay listed so old receipts still verify.
    pub keys: Vec<PublishedKey>,
}

impl KeySet {
    /// The verification key with this id.
    pub fn find(&self, key_id: &KeyId) -> Result<VerifyingKey> {
        self.keys
            .iter()
            .find(|k| &k.key_id == key_id)
            .ok_or_else(|| Error::UnknownKey(key_id.clone()))?
            .verifying_key()
    }
}
