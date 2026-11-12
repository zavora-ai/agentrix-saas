//! Passwords (R2.2, R2.3): argon2id with the library's defaults (19 MiB, two passes, one lane,
//! as OWASP recommends), stored in PHC form. A password never signs anyone in by itself: every
//! password sign-in continues with a TOTP code.

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};

/// Shortest password accepted.
pub const MIN_CHARS: usize = 12;
/// Longest password accepted (hashing cost is bounded).
pub const MAX_CHARS: usize = 256;

/// Checks length only: long passphrases are encouraged and nothing else is imposed.
pub fn check_policy(password: &str) -> Result<(), &'static str> {
    let chars = password.chars().count();
    if chars < MIN_CHARS {
        Err("Use at least 12 characters. A few words together work well.")
    } else if chars > MAX_CHARS {
        Err("Use at most 256 characters.")
    } else {
        Ok(())
    }
}

/// Hashes a password for storage.
pub fn hash(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Checks a password against a stored hash.
pub fn verify(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Spends the same work as `verify` when the email is unknown or has no password, so the
/// response time does not tell an attacker which emails have accounts.
pub fn verify_nothing(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let stored = DUMMY.get_or_init(|| hash("agentrix-dummy-password").unwrap_or_default());
    let _ = verify(password, stored);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies() {
        let stored = hash("correct horse battery staple").unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert!(verify("correct horse battery staple", &stored));
        assert!(!verify("correct horse battery stapler", &stored));
        assert!(!verify("anything", "not a hash"));
    }

    #[test]
    fn policy_is_length_only() {
        assert!(check_policy("short").is_err());
        assert!(check_policy("twelve chars").is_ok());
        assert!(check_policy(&"x".repeat(257)).is_err());
    }
}
