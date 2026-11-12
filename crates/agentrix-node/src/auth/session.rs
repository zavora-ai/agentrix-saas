//! Sessions (R2.3): a random 256-bit token in an HttpOnly, Secure, SameSite=Lax cookie. Only
//! the token's SHA-256 is stored. Behind TLS the cookie is `__Host-agx_session` (Secure, Path=/,
//! no Domain), so no subdomain can set or read it.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{AuthError, AuthState};

/// How long a session lasts, in seconds (14 days).
pub const SESSION_SECONDS: i64 = 14 * 24 * 3600;

/// A new token for the cookie, and the hash to store.
pub fn new_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let hash = token_hash(&token);
    (token, hash)
}

/// The stored form of a token.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The cookie's name: with the `__Host-` prefix behind TLS.
pub fn cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-agx_session"
    } else {
        "agx_session"
    }
}

/// `Set-Cookie` for a new session.
pub fn set_cookie(secure: bool, token: &str) -> HeaderValue {
    let value = format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_SECONDS}{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&value).expect("token is base64url")
}

/// `Set-Cookie` that removes the session cookie.
pub fn clear_cookie(secure: bool) -> HeaderValue {
    let value = format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&value).expect("static cookie")
}

/// The session token from the request's cookies, if any.
pub fn token_from(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = cookie_name(secure);
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty() && v.len() <= 64)
}

/// The signed-in owner. As an extractor it answers 401 when nobody is signed in.
#[derive(Debug, Clone)]
pub struct Session {
    /// The account.
    pub account: Uuid,
    /// `passkey` or `password+totp`.
    pub method: String,
    token_hash: String,
}

impl Session {
    /// The stored hash of this session's token (for signing out).
    pub fn token_hash(&self) -> &str {
        &self.token_hash
    }
}

impl AuthState {
    /// The session behind these headers, if any and still live.
    pub async fn session_from(&self, headers: &HeaderMap) -> Result<Option<Session>, AuthError> {
        let Some(token) = token_from(headers, self.config.secure_cookies) else {
            return Ok(None);
        };
        let hash = token_hash(&token);
        Ok(self.store.session(&hash).await?.map(|row| Session {
            account: row.account,
            method: row.method,
            token_hash: hash,
        }))
    }

    /// Starts a session and returns the cookie to set.
    pub async fn start_session(
        &self,
        account: Uuid,
        method: &str,
    ) -> Result<HeaderValue, AuthError> {
        let (token, hash) = new_token();
        self.store
            .create_session(
                &hash,
                account,
                method,
                super::store::now() + SESSION_SECONDS,
            )
            .await?;
        Ok(set_cookie(self.config.secure_cookies, &token))
    }
}

impl<S> FromRequestParts<S> for Session
where
    AuthState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth = AuthState::from_ref(state);
        auth.session_from(&parts.headers)
            .await?
            .ok_or(AuthError::SignedOut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_round_trip() {
        let (token, hash) = new_token();
        assert_eq!(token.len(), 43);
        assert_eq!(hash, token_hash(&token));
        let set = set_cookie(true, &token);
        let set = set.to_str().unwrap();
        assert!(set.starts_with("__Host-agx_session="));
        assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax") && set.contains("Secure"));
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("other=1; __Host-agx_session={token}")).unwrap(),
        );
        assert_eq!(token_from(&headers, true).as_deref(), Some(token.as_str()));
        assert_eq!(
            token_from(&headers, false),
            None,
            "the dev cookie name differs"
        );
        assert!(!clear_cookie(false).to_str().unwrap().contains("Secure"));
    }
}
