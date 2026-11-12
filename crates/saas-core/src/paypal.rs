//! PayPal's ids, typed, so an order id is never posted where a capture id belongs.
//!
//! [`PayPalRef`] names the one PayPal object a posting in the books came from. Its
//! [`PayPalRef::key`] is what the books store as the document's reference, which is how every
//! posting is idempotent by PayPal id.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Why a PayPal id was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a PayPal {kind} id: {value:?}")]
pub struct PayPalIdError {
    /// Which id.
    pub kind: &'static str,
    /// The value refused.
    pub value: String,
}

macro_rules! paypal_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The id as PayPal wrote it. PayPal ids are ASCII letters, digits, `-` and `_`.
            pub fn new(value: impl Into<String>) -> Result<Self, PayPalIdError> {
                let value = value.into();
                let ok = !value.is_empty()
                    && value.len() <= 128
                    && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
                if ok { Ok(Self(value)) } else { Err(PayPalIdError { kind: $kind, value }) }
            }

            /// The id.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = PayPalIdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::new(s)
            }
        }

        impl TryFrom<String> for $name {
            type Error = PayPalIdError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

paypal_id!(
    /// An Orders v2 order, for example `1EF274999Y578383Y`.
    PayPalOrderId, "order"
);
paypal_id!(
    /// A captured payment (Payments v2 capture), the id money settles under.
    PayPalCaptureId, "capture"
);
paypal_id!(
    /// A Subscriptions v1 subscription, for example `I-J53UUW5DF0KV`.
    PayPalSubscriptionId, "subscription"
);
paypal_id!(
    /// One sale (transaction) within a subscription: each billing cycle's payment.
    PayPalSaleId, "sale"
);
paypal_id!(
    /// A Payouts batch, for example `UGQN4VVND4DZJ`.
    PayPalPayoutBatchId, "payout batch"
);
paypal_id!(
    /// One item within a Payouts batch, for example `AN6FP42Y3464L`.
    PayPalPayoutItemId, "payout item"
);
paypal_id!(
    /// A refund of a capture.
    PayPalRefundId, "refund"
);
paypal_id!(
    /// A webhook event, for example `WH-…`.
    PayPalWebhookEventId, "webhook event"
);

impl PayPalSubscriptionId {
    /// The reference a recurring invoice carries for this subscription, and so every invoice it
    /// raises: `paypal:subscription:<id>`. Its payments book against those invoices
    /// ([`crate::posting::recurring_invoice_for`]).
    pub fn reference(&self) -> String {
        format!("paypal:subscription:{}", self.0)
    }
}

/// The one PayPal object a posting in the books came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum PayPalRef {
    /// Money captured on an order.
    Capture(PayPalCaptureId),
    /// One billing cycle's payment on a subscription.
    SubscriptionSale(PayPalSaleId),
    /// Money given back.
    Refund(PayPalRefundId),
    /// Money paid out to a payee (one payout item).
    Payout(PayPalPayoutItemId),
}

impl PayPalRef {
    /// The reference the books store for this object: `paypal:<kind>:<id>`, for example
    /// `paypal:capture:3C679366HH908993F`. Postings are idempotent by this key.
    pub fn key(&self) -> String {
        let (kind, id) = match self {
            PayPalRef::Capture(id) => ("capture", id.as_str()),
            PayPalRef::SubscriptionSale(id) => ("sale", id.as_str()),
            PayPalRef::Refund(id) => ("refund", id.as_str()),
            PayPalRef::Payout(id) => ("payout", id.as_str()),
        };
        format!("paypal:{kind}:{id}")
    }

    /// The key of the fee journal that goes with this object: `<key>:fee`.
    pub fn fee_key(&self) -> String {
        format!("{}:fee", self.key())
    }

    /// Reads a key written by [`PayPalRef::key`] back.
    pub fn from_key(key: &str) -> Option<Self> {
        let rest = key.strip_prefix("paypal:")?;
        let (kind, id) = rest.split_once(':')?;
        Some(match kind {
            "capture" => PayPalRef::Capture(id.parse().ok()?),
            "sale" => PayPalRef::SubscriptionSale(id.parse().ok()?),
            "refund" => PayPalRef::Refund(id.parse().ok()?),
            "payout" => PayPalRef::Payout(id.parse().ok()?),
            _ => return None,
        })
    }
}

impl fmt::Display for PayPalRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip() {
        let r = PayPalRef::Capture("3C679366HH908993F".parse().unwrap());
        assert_eq!(r.key(), "paypal:capture:3C679366HH908993F");
        assert_eq!(r.fee_key(), "paypal:capture:3C679366HH908993F:fee");
        assert_eq!(PayPalRef::from_key(&r.key()), Some(r.clone()));
        assert_eq!(
            PayPalRef::from_key("paypal:capture:3C679366HH908993F:fee"),
            None
        );
        assert_eq!(PayPalRef::from_key("INV-0001"), None);
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "capture", "id": "3C679366HH908993F"})
        );
    }

    #[test]
    fn ids_are_checked() {
        assert!(PayPalSubscriptionId::new("I-J53UUW5DF0KV").is_ok());
        assert!(PayPalCaptureId::new("").is_err());
        assert!(PayPalCaptureId::new("abc def").is_err());
        assert!(PayPalCaptureId::new("a:b").is_err());
    }
}
