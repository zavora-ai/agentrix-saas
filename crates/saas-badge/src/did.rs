//! `did:web` identifiers for a site and its agents (W3C DID identifiers with the did:web
//! method): the website's own domain is the root of trust, and nothing depends on a registry.
//!
//! - The site: `did:web:<site-host>`, its document at `https://<site-host>/.well-known/did.json`.
//! - An agent: `did:web:<site-host>:agents:<role>`, its document at
//!   `https://<site-host>/agents/<role>/did.json`.
//!
//! A port in the host is written `%3A` (`did:web:kilele.localhost%3A18182`), as the method
//! says. Plain `http` is only ever used for loopback hosts (`localhost`, `*.localhost`,
//! `127.0.0.1`), for local runs.

use serde_json::{Value, json};

/// The JSON-LD contexts every document here carries.
pub const CONTEXTS: [&str; 2] = [
    "https://www.w3.org/ns/did/v1",
    "https://w3id.org/security/suites/jws-2020/v1",
];

/// A parsed `did:web` identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DidWeb {
    /// The host, with any port (`kilele-sample.paypal.agentrix.website`, `kilele.localhost:18182`).
    pub host: String,
    /// The path segments after the host (`["agents", "cashier"]`), empty for the site.
    pub path: Vec<String>,
}

impl DidWeb {
    /// Parses `did:web:<host>[:<segment>…]`.
    pub fn parse(did: &str) -> Option<Self> {
        let rest = did.strip_prefix("did:web:")?;
        let rest = rest.split(['#', '?']).next()?;
        let mut parts = rest.split(':');
        let host = parts.next()?.replace("%3A", ":").replace("%3a", ":");
        if host.is_empty() || host.contains('/') {
            return None;
        }
        let path: Vec<String> = parts.map(str::to_owned).collect();
        if path
            .iter()
            .any(|p| p.is_empty() || p.contains('/') || p == "..")
        {
            return None;
        }
        Some(Self { host, path })
    }

    /// The identifier.
    pub fn to_did(&self) -> String {
        let mut did = format!("did:web:{}", self.host.replace(':', "%3A"));
        for segment in &self.path {
            did.push(':');
            did.push_str(segment);
        }
        did
    }

    /// Whether the host is a loopback name (local runs only).
    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }

    /// Where the document is: `https://<host>/.well-known/did.json` for the site,
    /// `https://<host>/<path>/did.json` otherwise (`http` for loopback hosts).
    pub fn document_url(&self) -> String {
        let scheme = if self.is_loopback() { "http" } else { "https" };
        if self.path.is_empty() {
            format!("{scheme}://{}/.well-known/did.json", self.host)
        } else {
            format!("{scheme}://{}/{}/did.json", self.host, self.path.join("/"))
        }
    }

    /// The site this identifier belongs to.
    pub fn site(&self) -> DidWeb {
        DidWeb {
            host: self.host.clone(),
            path: Vec::new(),
        }
    }

    /// The agent role, for `did:web:<host>:agents:<role>`.
    pub fn agent_role(&self) -> Option<&str> {
        match self.path.as_slice() {
            [agents, role] if agents == "agents" => Some(role),
            _ => None,
        }
    }

    /// The site's origin (`https://<host>`, or `http://` for loopback hosts).
    pub fn origin(&self) -> String {
        let scheme = if self.is_loopback() { "http" } else { "https" };
        format!("{scheme}://{}", self.host)
    }
}

/// Whether `host` (with or without a port) is a loopback name.
pub fn is_loopback_host(host: &str) -> bool {
    let name = host.rsplit_once(':').map_or(host, |(h, p)| {
        if p.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    let name = name.to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost") || name == "127.0.0.1"
}

/// `did:web:<host>`.
pub fn site_did(host: &str) -> String {
    DidWeb {
        host: host.to_owned(),
        path: Vec::new(),
    }
    .to_did()
}

/// `did:web:<host>:agents:<role>`.
pub fn agent_did(host: &str, role: &str) -> String {
    DidWeb {
        host: host.to_owned(),
        path: vec!["agents".into(), role.to_owned()],
    }
    .to_did()
}

/// A verification method entry: an Ed25519 key as a JWK.
pub fn verification_method(id: &str, controller: &str, public_key: &[u8; 32]) -> Value {
    use base64::Engine;
    json!({
        "id": id,
        "type": "JsonWebKey2020",
        "controller": controller,
        "publicKeyJwk": {
            "kty": "OKP",
            "crv": "Ed25519",
            "x": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public_key),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_their_documents() {
        let did = agent_did("kilele-sample.agentrix.website", "cashier");
        assert_eq!(did, "did:web:kilele-sample.agentrix.website:agents:cashier");
        let parsed = DidWeb::parse(&did).unwrap();
        assert_eq!(parsed.agent_role(), Some("cashier"));
        assert_eq!(
            parsed.document_url(),
            "https://kilele-sample.agentrix.website/agents/cashier/did.json"
        );
        assert_eq!(
            DidWeb::parse("did:web:agentrix.website")
                .unwrap()
                .document_url(),
            "https://agentrix.website/.well-known/did.json"
        );
        let local = agent_did("kilele.localhost:18182", "cashier");
        assert_eq!(local, "did:web:kilele.localhost%3A18182:agents:cashier");
        let parsed = DidWeb::parse(&local).unwrap();
        assert_eq!(parsed.host, "kilele.localhost:18182");
        assert_eq!(
            parsed.document_url(),
            "http://kilele.localhost:18182/agents/cashier/did.json"
        );
        assert_eq!(parsed.to_did(), local);
        assert!(DidWeb::parse("did:web:a.example:..:x").is_none());
        assert!(DidWeb::parse("did:key:z6Mk").is_none());
        assert!(!is_loopback_host("localhost.evil.example"));
        assert!(is_loopback_host("127.0.0.1:18182"));
    }
}
