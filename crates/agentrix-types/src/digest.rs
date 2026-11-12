//! SHA-256 content addresses.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::error::{Error, Result};

/// A SHA-256 content address, written `sha256:<64 lowercase hex>`.
///
/// Artifacts, frames, evidence, releases and signed receipts are all named by one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The text before the hex.
    pub const PREFIX: &'static str = "sha256:";

    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Wraps raw digest bytes, for example from a streaming hasher.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw 32 bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The 64 hex characters without the prefix, as used in CAS paths.
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", Self::PREFIX, self.hex())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

impl FromStr for Digest {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let bad = || Error::BadDigest(s.chars().take(80).collect());
        let hex_part = s.strip_prefix(Self::PREFIX).ok_or_else(bad)?;
        if hex_part.len() != 64
            || !hex_part
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        hex::decode_to_slice(hex_part, &mut out).map_err(|_| bad())?;
        Ok(Self(out))
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_parse_round_trip() {
        let d = Digest::of(b"hello");
        let text = d.to_string();
        assert_eq!(
            text,
            "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(text.parse::<Digest>().unwrap(), d);
        assert_eq!(serde_json::to_string(&d).unwrap(), format!("\"{text}\""));
    }

    #[test]
    fn rejects_anything_but_lowercase_sha256() {
        for bad in [
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            "sha256:2CF24DBA5FB0A30E26E83B2AC5B9E29E1B161E5C1FA7425E73043362938B9824",
            "sha256:2cf24d",
            "sha512:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            "sha256:../../etc/passwd",
        ] {
            assert!(bad.parse::<Digest>().is_err(), "{bad} should be refused");
        }
    }
}
