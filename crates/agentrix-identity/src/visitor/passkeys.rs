//! One passkey relying party per site host (06 §4.10): the RP id is the host the customer
//! sees, so a passkey made on one site can't be used on a look-alike or on another site of the
//! platform. Built on first use and kept for the life of the process.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agentrix_types::tenancy::SiteHost;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use webauthn_rs::prelude::{
    AuthenticationResult, CreationChallengeResponse, CredentialID, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url, Uuid, Webauthn, WebauthnBuilder, WebauthnError,
};

use super::VisitorError;

/// A credential id as text (base64url), for storage.
pub fn credential_id_text(id: &CredentialID) -> String {
    URL_SAFE_NO_PAD.encode(id.as_ref())
}

/// The relying party for one site host.
#[derive(Debug)]
pub struct SiteRp {
    webauthn: Webauthn,
}

impl SiteRp {
    fn new(site: &SiteHost) -> Result<Self, WebauthnError> {
        let origin = Url::parse(&site.origin).map_err(|_| WebauthnError::Configuration)?;
        if origin.host_str() != Some(site.host.as_str()) {
            return Err(WebauthnError::Configuration);
        }
        let webauthn = WebauthnBuilder::new(&site.host, &origin)?
            .rp_name(&site.name)
            .build()?;
        Ok(Self { webauthn })
    }

    /// Starts registering a passkey; `existing` are excluded so one authenticator isn't added twice.
    pub fn start_registration(
        &self,
        user: Uuid,
        email: &str,
        name: &str,
        existing: &[Passkey],
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), VisitorError> {
        let exclude: Vec<CredentialID> = existing.iter().map(|p| p.cred_id().clone()).collect();
        let exclude = (!exclude.is_empty()).then_some(exclude);
        self.webauthn
            .start_passkey_registration(user, email, name, exclude)
            .map_err(internal)
    }

    /// Finishes a registration.
    pub fn finish_registration(
        &self,
        credential: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> Result<Passkey, WebauthnError> {
        self.webauthn.finish_passkey_registration(credential, state)
    }

    /// Starts a sign-in with the account's passkeys.
    pub fn start_login(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), VisitorError> {
        self.webauthn
            .start_passkey_authentication(passkeys)
            .map_err(internal)
    }

    /// Finishes a sign-in: origin, RP id, user verification, signature and counter.
    pub fn finish(
        &self,
        credential: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> Result<AuthenticationResult, WebauthnError> {
        self.webauthn
            .finish_passkey_authentication(credential, state)
    }
}

fn internal(e: WebauthnError) -> VisitorError {
    VisitorError::Internal(format!("passkey: {e}"))
}

/// Relying parties by host.
#[derive(Debug, Default)]
pub struct RelyingParties {
    by_host: Mutex<HashMap<String, Arc<SiteRp>>>,
}

impl RelyingParties {
    /// The relying party for `site`, built on first use. A site whose origin doesn't match its
    /// host gets none.
    pub fn for_site(&self, site: &SiteHost) -> Result<Arc<SiteRp>, VisitorError> {
        let key = format!("{}\0{}", site.host, site.origin);
        let mut map = self.by_host.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(rp) = map.get(&key) {
            return Ok(rp.clone());
        }
        let rp = Arc::new(SiteRp::new(site).map_err(|e| {
            VisitorError::Internal(format!("relying party for {}: {e}", site.host))
        })?);
        map.insert(key, rp.clone());
        Ok(rp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::SiteId;

    fn site(host: &str, origin: &str) -> SiteHost {
        SiteHost {
            site: SiteId::new("site_k"),
            host: host.into(),
            origin: origin.into(),
            name: "Kilele Bakery".into(),
            sample: true,
        }
    }

    #[test]
    fn one_rp_per_host_and_origin_must_match() {
        let rps = RelyingParties::default();
        let a = rps
            .for_site(&site(
                "kilele-bakery-sample.agentrix.website",
                "https://kilele-bakery-sample.agentrix.website",
            ))
            .unwrap();
        let b = rps
            .for_site(&site(
                "kilele-bakery-sample.agentrix.website",
                "https://kilele-bakery-sample.agentrix.website",
            ))
            .unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(
            rps.for_site(&site("kilele.localhost", "http://kilele.localhost:18600"))
                .is_ok(),
            "local development"
        );
        assert!(
            rps.for_site(&site(
                "kilele-bakery-sample.agentrix.website",
                "https://evil.example"
            ))
            .is_err()
        );
    }
}
