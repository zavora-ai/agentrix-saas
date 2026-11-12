//! Errors from building, signing and checking the shared types.

use crate::sign::KeyId;

/// Everything that can go wrong building, signing or checking these types.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A value could not be written or read as JSON (including RFC 8785 canonical JSON).
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Not a `sha256:<64 lowercase hex>` digest.
    #[error("not a sha256 digest: {0:?}")]
    BadDigest(String),
    /// Not a container image pinned by digest (`name@sha256:<64 hex>`).
    #[error("image is not pinned by digest: {0:?}")]
    BadImage(String),
    /// A published key could not be decoded, or does not match its id.
    #[error("bad key: {0}")]
    BadKey(String),
    /// A signature is not 64 bytes of unpadded base64url.
    #[error("bad signature encoding")]
    BadSignatureEncoding,
    /// The signature does not match the canonical bytes.
    #[error("signature does not verify")]
    BadSignature,
    /// The value names a different signing key from the one offered.
    #[error("signed by {signed_by}, but the key offered is {offered}")]
    WrongKey {
        /// The key the value says signed it.
        signed_by: KeyId,
        /// The key the caller offered.
        offered: KeyId,
    },
    /// No published key has this id.
    #[error("no published key {0}")]
    UnknownKey(KeyId),
}

/// A result with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
