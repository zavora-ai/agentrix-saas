//! Out: a site as a small OIDC issuer for its own agents.
//!
//! The site serves `/.well-known/openid-configuration` and its key set, and mints short-lived
//! tokens: `iss` the site (`https://<host>`), `sub` the agent's DID
//! (`did:web:<host>:agents:<role>`). A cloud accepts them through its own federation (an AWS IAM
//! OIDC provider, a Google workload identity pool, an Entra federated credential), so no cloud
//! secret is stored anywhere.
//!
//! RS256, not the sites' Ed25519 Web Bot Auth keys: the clouds' federation takes RSA. The key is
//! RSA 2048, its `kid` the RFC 7638 thumbprint.

use std::path::Path;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rsa::{KeyPair, KeySize};
use aws_lc_rs::signature::KeyPair as _;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use saas_core::federation::{AgentTokenIssuer, IssuedToken};

use crate::FederationError;

/// The longest token a site mints: clouds want short-lived assertions.
pub const MAX_TTL: Duration = Duration::hours(1);

/// A site's host under the platform's: `<slug>.<platform_host>`, for example
/// `kilele-bakery-sample.paypal.agentrix.website`. `platform_host` is the node's setting.
pub fn site_host(slug: &str, platform_host: &str) -> String {
    format!(
        "{}.{}",
        slug.trim().to_ascii_lowercase(),
        platform_host
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase()
    )
}

/// `https://<host>`: an issuer URL: a site's (`issuer_url(&site_host(slug, platform_host))`), or
/// Agentrix's own (`issuer_url(platform_host)`). Hosts come from configuration, never from this
/// crate.
pub fn issuer_url(host: &str) -> String {
    format!(
        "https://{}",
        host.trim().trim_end_matches('/').to_ascii_lowercase()
    )
}

/// One site's issuer.
pub struct SiteIssuer {
    issuer: String,
    kid: String,
    key: EncodingKey,
    jwk: Value,
}

impl std::fmt::Debug for SiteIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiteIssuer")
            .field("issuer", &self.issuer)
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

fn pem_of(der: &[u8]) -> String {
    let b64 = STANDARD.encode(der);
    let lines: Vec<String> = b64
        .as_bytes()
        .chunks(64)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        lines.join("\n")
    )
}

fn der_of(pem: &str) -> Result<Vec<u8>, FederationError> {
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    STANDARD
        .decode(body.trim())
        .map_err(|e| FederationError::Key(format!("PEM: {e}")))
}

impl SiteIssuer {
    /// A new RSA 2048 key for `issuer` (see [`issuer_url`]).
    pub fn generate(issuer: &str) -> Result<Self, FederationError> {
        let pair =
            KeyPair::generate(KeySize::Rsa2048).map_err(|e| FederationError::Key(e.to_string()))?;
        let der = pair
            .as_der()
            .map_err(|e| FederationError::Key(e.to_string()))?;
        Self::from_pkcs8_pem(issuer, &pem_of(der.as_ref()))
    }

    /// The issuer with a PKCS#8 PEM RSA key.
    pub fn from_pkcs8_pem(issuer: &str, pem: &str) -> Result<Self, FederationError> {
        let der = der_of(pem)?;
        let pair = KeyPair::from_pkcs8(&der).map_err(|e| FederationError::Key(e.to_string()))?;
        let public = pair.public_key();
        let n = URL_SAFE_NO_PAD.encode(public.modulus().big_endian_without_leading_zero());
        let e = URL_SAFE_NO_PAD.encode(public.exponent().big_endian_without_leading_zero());
        // RFC 7638: the SHA-256 of the required members, in lexicographic order.
        let thumb_input = format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#);
        let kid = URL_SAFE_NO_PAD.encode(Sha256::digest(thumb_input.as_bytes()));
        let jwk = json!({"kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "n": n, "e": e});
        let key = EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| FederationError::Key(e.to_string()))?;
        Ok(Self {
            issuer: issuer.trim_end_matches('/').to_owned(),
            kid,
            key,
            jwk,
        })
    }

    /// The issuer with the key in `key_file`, made (PKCS#8 PEM, mode 0600) on first use. Keep it
    /// outside the repository.
    pub fn load_or_create(issuer: &str, key_file: &Path) -> Result<Self, FederationError> {
        match std::fs::read_to_string(key_file) {
            Ok(pem) => Self::from_pkcs8_pem(issuer, &pem),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let pair = KeyPair::generate(KeySize::Rsa2048)
                    .map_err(|e| FederationError::Key(e.to_string()))?;
                let der = pair
                    .as_der()
                    .map_err(|e| FederationError::Key(e.to_string()))?;
                let pem = pem_of(der.as_ref());
                write_secret(key_file, &pem)?;
                Self::from_pkcs8_pem(issuer, &pem)
            }
            Err(e) => Err(FederationError::Key(format!("{}: {e}", key_file.display()))),
        }
    }

    /// The issuer for a site's configured `host` (`https://<host>`), its key in
    /// `<key_dir>/oidc-<host>.pem` (made, 0600, on first use).
    pub fn for_host(host: &str, key_dir: &Path) -> Result<Self, FederationError> {
        let file = key_dir.join(format!("oidc-{}.pem", host.trim().to_ascii_lowercase()));
        Self::load_or_create(&issuer_url(host), &file)
    }

    /// `https://<host>`.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The signing key's id (its RFC 7638 thumbprint).
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// Where the key set is served.
    pub fn jwks_uri(&self) -> String {
        format!("{}/.well-known/jwks.json", self.issuer)
    }

    /// `/.well-known/openid-configuration`: what the clouds' federation reads.
    pub fn discovery(&self) -> Value {
        json!({
            "issuer": self.issuer,
            "jwks_uri": self.jwks_uri(),
            "response_types_supported": ["id_token"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "claims_supported": ["iss", "sub", "aud", "exp", "iat", "nbf", "jti", "agent", "site"],
        })
    }

    /// The public key set.
    pub fn jwks(&self) -> Value {
        json!({"keys": [self.jwk]})
    }

    /// A token for the agent with DID `agent_did`, for `audience` (the cloud's: for example
    /// `sts.amazonaws.com`, a Google workload identity provider's resource name, or Entra's
    /// `api://AzureADTokenExchange`), valid for `ttl` (at most [`MAX_TTL`]) from `now`. `extra`
    /// adds claims; it can't replace the standard ones.
    pub fn issue_at(
        &self,
        agent_did: &str,
        audience: &str,
        ttl: Duration,
        now: DateTime<Utc>,
        extra: Map<String, Value>,
    ) -> Result<IssuedToken, FederationError> {
        if ttl <= Duration::zero() || ttl > MAX_TTL {
            return Err(FederationError::Invalid(format!(
                "a token lives between 1 s and {} min",
                MAX_TTL.num_minutes()
            )));
        }
        if !agent_did.starts_with("did:") {
            return Err(FederationError::Invalid(format!(
                "the subject must be the agent's DID, not {agent_did:?}"
            )));
        }
        let mut claims = extra;
        for k in ["iss", "sub", "aud", "exp", "iat", "nbf", "jti"] {
            claims.remove(k);
        }
        claims.insert("iss".into(), json!(self.issuer));
        claims.insert("sub".into(), json!(agent_did));
        claims.insert("aud".into(), json!(audience));
        claims.insert("iat".into(), json!(now.timestamp()));
        claims.insert("nbf".into(), json!(now.timestamp()));
        claims.insert("exp".into(), json!((now + ttl).timestamp()));
        claims.insert("jti".into(), json!(uuid::Uuid::new_v4().to_string()));
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        header.typ = Some("JWT".into());
        let token = jsonwebtoken::encode(&header, &Value::Object(claims), &self.key)
            .map_err(|e| FederationError::Key(e.to_string()))?;
        Ok(IssuedToken {
            token,
            expires_at: now + ttl,
            kid: self.kid.clone(),
        })
    }
}

impl SiteIssuer {
    /// Signs a whole claim set as this issuer (judge mode's stand-ins for the clouds' keys).
    pub(crate) fn sign_claims(
        &self,
        mut claims: Map<String, Value>,
    ) -> Result<String, FederationError> {
        claims.insert("iss".into(), json!(self.issuer));
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        header.typ = Some("JWT".into());
        jsonwebtoken::encode(&header, &Value::Object(claims), &self.key)
            .map_err(|e| FederationError::Key(e.to_string()))
    }
}

fn write_secret(path: &Path, text: &str) -> Result<(), FederationError> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| FederationError::Key(e.to_string()))?;
    }
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    .map_err(|e| FederationError::Key(e.to_string()))?;
    #[cfg(not(unix))]
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| FederationError::Key(e.to_string()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| FederationError::Key(e.to_string()))
}

#[async_trait::async_trait]
impl AgentTokenIssuer for SiteIssuer {
    fn issuer(&self) -> String {
        SiteIssuer::issuer(self).to_owned()
    }

    fn discovery(&self) -> Value {
        SiteIssuer::discovery(self)
    }

    fn jwks(&self) -> Value {
        SiteIssuer::jwks(self)
    }

    async fn issue(
        &self,
        agent_did: &str,
        audience: &str,
        ttl: Duration,
    ) -> anyhow::Result<IssuedToken> {
        Ok(self.issue_at(agent_did, audience, ttl, Utc::now(), Map::new())?)
    }
}
