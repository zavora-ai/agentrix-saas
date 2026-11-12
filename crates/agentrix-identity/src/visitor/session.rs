//! Customer sessions: a random 256-bit token in a host-only cookie, separate from the owner's
//! `__Host-agx_session`. Behind TLS the cookie is `__Host-agx_customer` (Secure, Path=/, no
//! Domain), so no other host, not even a sibling sample site, can set or read it. Only the
//! token's SHA-256 is stored.

use axum::http::header::COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// How long a customer session lasts, in seconds (30 days).
pub const SESSION_SECONDS: i64 = 30 * 24 * 3600;

/// The cookie that remembers "Not now" to the passkey offer on this device (A-07).
pub const DECLINED_COOKIE: &str = "agx_passkey_offer";

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
        "__Host-agx_customer"
    } else {
        "agx_customer"
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

/// `Set-Cookie` for "this device said Not now to the passkey offer" (a year).
pub fn declined_cookie(secure: bool) -> HeaderValue {
    let value = format!(
        "{DECLINED_COOKIE}=no; Path=/; SameSite=Lax; Max-Age=31536000{}",
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&value).expect("static cookie")
}

/// A cookie's value from the request.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

/// The session token from the request's cookies, if any.
pub fn token_from(headers: &HeaderMap, secure: bool) -> Option<String> {
    cookie(headers, cookie_name(secure)).filter(|v| !v.is_empty() && v.len() <= 64)
}

/// Whether this device said "Not now" to the passkey offer.
pub fn declined(headers: &HeaderMap) -> bool {
    cookie(headers, DECLINED_COOKIE).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_round_trip() {
        let (token, hash) = new_token();
        assert_eq!(hash, token_hash(&token));
        let set = set_cookie(true, &token);
        let set = set.to_str().unwrap();
        assert!(
            set.starts_with("__Host-agx_customer=")
                && set.contains("HttpOnly")
                && set.contains("Secure")
        );
        assert!(!set.contains("Domain"), "host-only");
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!(
                "__Host-agx_session=owner; __Host-agx_customer={token}"
            ))
            .unwrap(),
        );
        assert_eq!(token_from(&headers, true).as_deref(), Some(token.as_str()));
        assert_eq!(token_from(&headers, false), None);
        assert!(!declined(&headers));
    }
}
