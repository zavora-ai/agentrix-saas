//! Sealing small secrets at rest (R2.3, 09 §2.3): TOTP seeds are encrypted with
//! ChaCha20-Poly1305 under the node's sealing key before they reach SQLite. The key lives in
//! `/etc/agentrix/secrets/seal.key` (32 random bytes, root-only, written once by the deploy
//! script) and never in the database, logs or receipts.

use std::fmt;
use std::path::Path;

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use rand::RngCore;

const NONCE_BYTES: usize = 12;

/// Encrypts and decrypts with one key. The associated data binds a sealed value to its owner,
/// so a seed copied to another account's row does not open.
pub struct Sealer {
    cipher: ChaCha20Poly1305,
}

impl fmt::Debug for Sealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sealer(<key withheld>)")
    }
}

impl Sealer {
    /// From raw key bytes.
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(&key)),
        }
    }

    /// From a key file holding exactly 32 bytes.
    pub fn from_file(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} must hold exactly 32 bytes", path.display()),
            )
        })?;
        Ok(Self::from_key(key))
    }

    /// A random key that lives only as long as the process. For tests and local development:
    /// seeds sealed with it cannot be opened after a restart.
    pub fn ephemeral() -> Self {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self::from_key(key)
    }

    /// `nonce ‖ ciphertext`.
    pub fn seal(&self, plaintext: &[u8], associated: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: associated,
                },
            )
            .expect("ChaCha20-Poly1305 encryption cannot fail for in-memory input");
        let mut out = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        out
    }

    /// Opens a sealed value, or `None` if it was altered, sealed under another key or for
    /// another owner.
    pub fn open(&self, sealed: &[u8], associated: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < NONCE_BYTES {
            return None;
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_BYTES);
        self.cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: associated,
                },
            )
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_binding() {
        let sealer = Sealer::from_key([7; 32]);
        let sealed = sealer.seal(b"seed", b"account-a");
        assert_ne!(&sealed[NONCE_BYTES..], b"seed");
        assert_eq!(
            sealer.open(&sealed, b"account-a").as_deref(),
            Some(&b"seed"[..])
        );
        assert_eq!(
            sealer.open(&sealed, b"account-b"),
            None,
            "bound to its owner"
        );
        assert_eq!(
            Sealer::from_key([8; 32]).open(&sealed, b"account-a"),
            None,
            "other key"
        );
        let mut altered = sealed.clone();
        *altered.last_mut().unwrap() ^= 1;
        assert_eq!(sealer.open(&altered, b"account-a"), None, "tamper-evident");
        assert_eq!(format!("{sealer:?}"), "Sealer(<key withheld>)");
    }
}
