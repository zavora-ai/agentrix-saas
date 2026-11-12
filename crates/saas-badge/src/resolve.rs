//! Identity resolvers: one trait, [`IdentityResolver`], that turns an identifier into the keys
//! it controls. Three ship now:
//!
//! - [`DidWebResolver`]: `did:web:…`, fetched from the site's own domain.
//! - [`DidKeyResolver`]: `did:key:z6Mk…`, an Ed25519 key in the identifier itself.
//! - [`KeyDirectoryResolver`]: an origin (`https://kilele-sample.paypal.agentrix.website`), its Web
//!   Bot Auth key directory, keys named by `kid`.
//!
//! Other systems (Microsoft Entra Agent ID, Amazon Bedrock AgentCore Identity, Google's agent
//! identity, OpenAgents, OAI-1) come in through the same trait; `INTEROP.md` maps them.

use std::sync::Arc;
use std::time::Duration;

use adk_auth::web_bot_auth::KeyDirectorySource;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::did::{DidWeb, is_loopback_host};

/// One key an identity controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationKey {
    /// Its id: a DID URL (`did:web:<host>#owner`) or `<origin>#<kid>`.
    pub id: String,
    /// The Ed25519 public key.
    pub public_key: [u8; 32],
}

/// What an identifier resolved to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedIdentity {
    /// The identifier.
    pub id: String,
    /// The document it resolved to (a DID document, or a synthesized one).
    pub document: Value,
    /// Its keys.
    pub keys: Vec<VerificationKey>,
    /// Other names for it (`urn:adk:agent:…`).
    pub also_known_as: Vec<String>,
    /// Its services: `(type, endpoint)`.
    pub services: Vec<(String, String)>,
}

impl ResolvedIdentity {
    /// The key with this id: the full id (`did:web:a#owner`) or just its fragment (`owner`).
    pub fn key(&self, id: &str) -> Option<&VerificationKey> {
        let fragment = id.rsplit_once('#').map_or(id, |(_, f)| f);
        self.keys
            .iter()
            .find(|k| k.id == id || k.id.rsplit_once('#').is_some_and(|(_, f)| f == fragment))
    }

    /// The first service of `kind`.
    pub fn service(&self, kind: &str) -> Option<&str> {
        self.services
            .iter()
            .find(|(t, _)| t == kind)
            .map(|(_, e)| e.as_str())
    }
}

/// Why an identifier didn't resolve.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// No resolver here handles it.
    #[error("no resolver handles {0}")]
    Unsupported(String),
    /// It isn't well formed.
    #[error("{0} isn't a valid identifier")]
    Invalid(String),
    /// Its document couldn't be fetched.
    #[error("{id} didn't resolve: {reason}")]
    Unavailable {
        /// The identifier.
        id: String,
        /// Why.
        reason: String,
    },
    /// Its document doesn't hold together.
    #[error("{id}'s document is wrong: {reason}")]
    BadDocument {
        /// The identifier.
        id: String,
        /// Why.
        reason: String,
    },
}

/// Resolves identifiers of some method to the keys they control.
#[async_trait]
pub trait IdentityResolver: Send + Sync {
    /// Whether this resolver handles `id`.
    fn handles(&self, id: &str) -> bool;
    /// Resolves `id`.
    async fn resolve(&self, id: &str) -> Result<ResolvedIdentity, ResolveError>;
}

fn jwk_key(jwk: &Value) -> Option<[u8; 32]> {
    if jwk["kty"].as_str() != Some("OKP") || jwk["crv"].as_str() != Some("Ed25519") {
        return None;
    }
    URL_SAFE_NO_PAD
        .decode(jwk["x"].as_str()?)
        .ok()?
        .try_into()
        .ok()
}

/// Reads a DID document's Ed25519 keys, other names and services.
pub fn read_document(id: &str, document: Value) -> Result<ResolvedIdentity, ResolveError> {
    let bad = |reason: &str| ResolveError::BadDocument {
        id: id.to_owned(),
        reason: reason.to_owned(),
    };
    if document["id"].as_str() != Some(id) {
        return Err(bad("its id isn't the identifier asked for"));
    }
    let mut keys = Vec::new();
    for method in document["verificationMethod"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let Some(vm_id) = method["id"].as_str() else {
            continue;
        };
        let vm_id = if vm_id.starts_with('#') {
            format!("{id}{vm_id}")
        } else {
            vm_id.to_owned()
        };
        let key = match method["type"].as_str() {
            Some("JsonWebKey2020") => jwk_key(&method["publicKeyJwk"]),
            Some("Ed25519VerificationKey2020" | "Multikey") => method["publicKeyMultibase"]
                .as_str()
                .and_then(ed25519_from_multibase),
            _ => None,
        };
        if let Some(public_key) = key {
            keys.push(VerificationKey {
                id: vm_id,
                public_key,
            });
        }
    }
    let also_known_as = document["alsoKnownAs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let services = document["service"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some((
                s["type"].as_str()?.to_owned(),
                s["serviceEndpoint"].as_str()?.to_owned(),
            ))
        })
        .collect();
    Ok(ResolvedIdentity {
        id: id.to_owned(),
        document,
        keys,
        also_known_as,
        services,
    })
}

/// `did:web`, fetched over HTTPS from the site's own domain (plain HTTP for loopback hosts only,
/// and only when allowed).
#[derive(Debug, Clone)]
pub struct DidWebResolver {
    http: reqwest::Client,
    allow_loopback: bool,
}

impl DidWebResolver {
    /// HTTPS only.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            allow_loopback: false,
        }
    }

    /// Also resolves loopback hosts over plain HTTP (local runs and tests).
    pub fn allow_loopback(mut self) -> Self {
        self.allow_loopback = true;
        self
    }

    /// The client.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Fetches a JSON document from a site (a badge, a voucher): the same rules as DIDs.
    pub async fn fetch_json(&self, url: &str) -> Result<Value, ResolveError> {
        let unavailable = |reason: String| ResolveError::Unavailable {
            id: url.to_owned(),
            reason,
        };
        let parsed = url::Url::parse(url).map_err(|e| unavailable(e.to_string()))?;
        let host = parsed.host_str().unwrap_or_default();
        match parsed.scheme() {
            "https" => {}
            "http" if self.allow_loopback && is_loopback_host(host) => {}
            other => return Err(unavailable(format!("{other} isn't allowed here"))),
        }
        let response = self
            .http
            .get(parsed)
            .header("accept", "application/did+json, application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(unavailable(format!("HTTP {}", response.status())));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| unavailable(e.to_string()))?;
        if bytes.len() > 256 * 1024 {
            return Err(unavailable("the document is too large".into()));
        }
        serde_json::from_slice(&bytes).map_err(|e| unavailable(e.to_string()))
    }
}

#[async_trait]
impl IdentityResolver for DidWebResolver {
    fn handles(&self, id: &str) -> bool {
        id.starts_with("did:web:")
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedIdentity, ResolveError> {
        let did_url = id.split('#').next().unwrap_or(id);
        let parsed = DidWeb::parse(did_url).ok_or_else(|| ResolveError::Invalid(id.into()))?;
        if parsed.is_loopback() && !self.allow_loopback {
            return Err(ResolveError::Unavailable {
                id: id.into(),
                reason: "loopback hosts aren't resolved here".into(),
            });
        }
        let document = self.fetch_json(&parsed.document_url()).await?;
        read_document(did_url, document)
    }
}

/// `did:key` for Ed25519 (`did:key:z6Mk…`): the key is the identifier.
#[derive(Debug, Default, Clone, Copy)]
pub struct DidKeyResolver;

/// The multicodec prefix of an Ed25519 public key.
const ED25519_PUB: [u8; 2] = [0xed, 0x01];

/// `did:key:z…` for an Ed25519 public key.
pub fn did_key(public_key: &[u8; 32]) -> String {
    let mut bytes = ED25519_PUB.to_vec();
    bytes.extend_from_slice(public_key);
    format!("did:key:z{}", base58_encode(&bytes))
}

fn ed25519_from_multibase(value: &str) -> Option<[u8; 32]> {
    let bytes = base58_decode(value.strip_prefix('z')?)?;
    bytes.strip_prefix(&ED25519_PUB)?.try_into().ok()
}

#[async_trait]
impl IdentityResolver for DidKeyResolver {
    fn handles(&self, id: &str) -> bool {
        id.starts_with("did:key:")
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedIdentity, ResolveError> {
        let did = id.split('#').next().unwrap_or(id);
        let multibase = did
            .strip_prefix("did:key:")
            .ok_or_else(|| ResolveError::Invalid(id.into()))?;
        let public_key =
            ed25519_from_multibase(multibase).ok_or_else(|| ResolveError::Invalid(id.into()))?;
        let vm = format!("{did}#{multibase}");
        let document = serde_json::json!({
            "@context": crate::did::CONTEXTS,
            "id": did,
            "verificationMethod": [{
                "id": vm, "type": "Multikey", "controller": did, "publicKeyMultibase": multibase,
            }],
            "authentication": [vm], "assertionMethod": [vm],
        });
        Ok(ResolvedIdentity {
            id: did.to_owned(),
            document,
            keys: vec![VerificationKey { id: vm, public_key }],
            also_known_as: Vec::new(),
            services: Vec::new(),
        })
    }
}

/// An origin's Web Bot Auth key directory: the keys an agent door publishes, by `kid`
/// (`<origin>#agent:payables`, `<origin>#owner`).
#[derive(Clone)]
pub struct KeyDirectoryResolver {
    source: Arc<dyn KeyDirectorySource>,
}

impl std::fmt::Debug for KeyDirectoryResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyDirectoryResolver")
            .finish_non_exhaustive()
    }
}

impl KeyDirectoryResolver {
    /// Over `source` (`HttpKeyDirectorySource` on the node).
    pub fn new(source: Arc<dyn KeyDirectorySource>) -> Self {
        Self { source }
    }
}

#[async_trait]
impl IdentityResolver for KeyDirectoryResolver {
    fn handles(&self, id: &str) -> bool {
        id.starts_with("https://") || id.starts_with("http://")
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedIdentity, ResolveError> {
        let origin = id.split('#').next().unwrap_or(id).trim_end_matches('/');
        let fetched = self
            .source
            .fetch(origin)
            .await
            .map_err(|e| ResolveError::Unavailable {
                id: id.into(),
                reason: e.to_string(),
            })?;
        let mut keys = Vec::new();
        for key in &fetched.directory.keys {
            let (Ok(public_key), Ok(thumbprint)) = (key.public_key(), key.thumbprint()) else {
                continue;
            };
            let name = key.kid.clone().unwrap_or(thumbprint);
            keys.push(VerificationKey {
                id: format!("{origin}#{name}"),
                public_key,
            });
        }
        Ok(ResolvedIdentity {
            id: origin.to_owned(),
            document: serde_json::to_value(&fetched.directory.keys).unwrap_or(Value::Null),
            keys,
            also_known_as: Vec::new(),
            services: vec![(
                "HttpMessageSignaturesDirectory".into(),
                format!(
                    "{origin}{}",
                    adk_auth::web_bot_auth::DIRECTORY_WELL_KNOWN_PATH
                ),
            )],
        })
    }
}

/// Several resolvers: the first that handles an identifier resolves it.
#[derive(Clone, Default)]
pub struct Resolvers {
    resolvers: Vec<Arc<dyn IdentityResolver>>,
}

impl std::fmt::Debug for Resolvers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolvers")
            .field("count", &self.resolvers.len())
            .finish()
    }
}

impl Resolvers {
    /// None yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a resolver.
    #[must_use]
    pub fn with(mut self, resolver: Arc<dyn IdentityResolver>) -> Self {
        self.resolvers.push(resolver);
        self
    }

    /// `did:web` (HTTPS), `did:key` and key directories (`source`).
    pub fn standard(http: reqwest::Client, source: Arc<dyn KeyDirectorySource>) -> Self {
        Self::new()
            .with(Arc::new(DidWebResolver::new(http)))
            .with(Arc::new(DidKeyResolver))
            .with(Arc::new(KeyDirectoryResolver::new(source)))
    }
}

#[async_trait]
impl IdentityResolver for Resolvers {
    fn handles(&self, id: &str) -> bool {
        self.resolvers.iter().any(|r| r.handles(id))
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedIdentity, ResolveError> {
        match self.resolvers.iter().find(|r| r.handles(id)) {
            Some(resolver) => resolver.resolve(id).await,
            None => Err(ResolveError::Unsupported(id.into())),
        }
    }
}

const ALPHABET: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Base58 (Bitcoin alphabet), as `did:key`'s multibase `z` uses.
pub fn base58_encode(bytes: &[u8]) -> String {
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    let mut digits: Vec<u8> = Vec::new();
    for &byte in &bytes[zeros..] {
        let mut carry = u32::from(byte);
        for digit in &mut digits {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(digits.iter().rev().map(|d| ALPHABET[*d as usize] as char));
    out
}

/// The reverse of [`base58_encode`].
pub fn base58_decode(text: &str) -> Option<Vec<u8>> {
    let zeros = text.bytes().take_while(|b| *b == b'1').count();
    let mut bytes: Vec<u8> = Vec::new();
    for c in text.bytes().skip(zeros) {
        let mut carry = ALPHABET.iter().position(|a| *a == c)? as u32;
        for byte in &mut bytes {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let mut out = vec![0u8; zeros];
    out.extend(bytes.iter().rev());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base58_round_trips() {
        for bytes in [
            &b""[..],
            &[0, 0, 1][..],
            &[0xed, 0x01, 0xff, 0x00, 0x7f][..],
        ] {
            assert_eq!(base58_decode(&base58_encode(bytes)).unwrap(), bytes);
        }
        assert_eq!(base58_encode(b"hello world"), "StV1DL6CwTryKyV");
    }

    #[tokio::test]
    async fn did_key_resolves_to_its_key() {
        // The did:key spec's Ed25519 example.
        let did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let resolved = DidKeyResolver.resolve(did).await.unwrap();
        assert_eq!(resolved.keys.len(), 1);
        assert_eq!(did_key(&resolved.keys[0].public_key), did);
        let key = adk_auth::web_bot_auth::Ed25519SigningKey::generate().unwrap();
        let mine = did_key(key.public_key());
        assert!(mine.starts_with("did:key:z6Mk"));
        assert_eq!(
            &DidKeyResolver.resolve(&mine).await.unwrap().keys[0].public_key,
            key.public_key()
        );
        assert!(DidKeyResolver.resolve("did:key:zNotAKey").await.is_err());
    }
}
