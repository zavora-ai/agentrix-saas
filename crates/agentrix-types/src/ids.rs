//! Typed ids. Each is text on the wire; generated ids are a prefix and a UUIDv7, so they sort
//! by creation time.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("The prefix of generated ids: `", $prefix, "_…`.")]
            pub const PREFIX: &'static str = $prefix;

            /// A new id: the type's prefix and a UUIDv7.
            pub fn generate() -> Self {
                Self(format!("{}_{}", $prefix, uuid::Uuid::now_v7().simple()))
            }

            /// Wraps an id read from storage or a request.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// The id as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(
    /// A site: the unit of tenancy. Every row, key and receipt carries one.
    SiteId,
    "site"
);
id_type!(
    /// One Move of a site from its old host to Agentrix (design §6).
    MoveId,
    "move"
);
id_type!(
    /// One free assessment (R1).
    AssessmentId,
    "asmt"
);
id_type!(
    /// One sealed-room job (design §4.2).
    JobId,
    "job"
);
id_type!(
    /// One receipt (design §4.4). A signed receipt is also addressed by its digest.
    ReceiptId,
    "rcpt"
);
id_type!(
    /// One decision card that the owner approves or declines.
    DecisionId,
    "dec"
);
id_type!(
    /// One finding from a detector.
    FindingId,
    "fnd"
);
id_type!(
    /// One quarantined item.
    QuarantineId,
    "qtn"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_carry_their_prefix_and_sort_by_time() {
        let a = MoveId::generate();
        let b = MoveId::generate();
        assert!(a.as_str().starts_with("move_"));
        assert!(a < b, "UUIDv7 ids sort by creation time");
        assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{a}\""));
    }
}
