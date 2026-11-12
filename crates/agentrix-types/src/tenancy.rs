//! Tenancy (ADR-018, contract C1 in specs/platform/02-architecture.md §6.1): who a request is
//! for and who is acting. Owner: W6 (identity, payments and trust).
//!
//! - [`SiteHost`]: the site a request on a site's own host is for. The edge inserts one per
//!   request; visitor sign-in uses its host as the passkey relying party, so a passkey made on
//!   one site never works on a look-alike (06 §4.10).
//! - [`TenantPath`]: operator → partner (optional) → organisation → site, the key every
//!   token, row, metric and log line carries in production (02 §8).
//! - [`Principal`]: a person, a customer, a WebAgent or an operator, as receipts name them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::SiteId;

macro_rules! tenant_id {
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

tenant_id!(
    /// The operator that runs a cell: Zavora, or a partner running a dedicated cell (`op_…`).
    OperatorId,
    "op"
);
tenant_id!(
    /// An agency, reseller or referral partner (`ptn_…`).
    PartnerId,
    "ptn"
);
tenant_id!(
    /// The business that owns sites (`org_…`).
    OrgId,
    "org"
);
tenant_id!(
    /// A customer's account on one site (`vac_…`). Never valid on another site.
    VisitorAccountId,
    "vac"
);

/// The site a request on a site's own host is for. The edge's host router builds it once per
/// request and inserts it as a request extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteHost {
    /// The site.
    pub site: SiteId,
    /// The host exactly as served, for example `kilele-bakery-sample.agentrix.website`. It is
    /// the passkey relying party id for the site's customers.
    pub host: String,
    /// The exact origin browsers send, for example `https://kilele-bakery-sample.agentrix.website`
    /// (`http://kilele-bakery-sample.localhost:18600` in development).
    pub origin: String,
    /// The business name visitors read, for example `Kilele Bakery`.
    pub name: String,
    /// A sample site on a `-sample` host: invented people, sandbox payments, marked "Sample".
    pub sample: bool,
}

impl SiteHost {
    /// Whether `origin` (an `Origin` header) is this site's own origin.
    pub fn is_own_origin(&self, origin: &str) -> bool {
        origin.trim_end_matches('/') == self.origin.trim_end_matches('/')
    }

    /// Whether cookies for this host must be `Secure` (`__Host-` prefixed).
    pub fn is_https(&self) -> bool {
        self.origin.starts_with("https://")
    }
}

/// Where a site sits: operator → partner (when one manages it) → organisation → site.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantPath {
    /// The operator running the cell.
    pub operator: OperatorId,
    /// The partner managing the organisation, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partner: Option<PartnerId>,
    /// The organisation that owns the site.
    pub org: OrgId,
    /// The site, when the path names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteId>,
}

impl TenantPath {
    /// Whether `other` is this path or below it (a site under this org, an org under this
    /// partner). An operator path covers everything it runs.
    pub fn covers(&self, other: &TenantPath) -> bool {
        self.operator == other.operator
            && (self.partner.is_none() || self.partner == other.partner)
            && self.org == other.org
            && (self.site.is_none() || self.site == other.site)
    }
}

/// Who acted, as receipts and tokens name them (05 §3.1, ADR-017).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Principal {
    /// A person with a platform account: an owner, a team member or a partner's staff.
    Person {
        /// The account id (UUID text).
        account: String,
    },
    /// A customer signed in on one site.
    Customer {
        /// The site.
        site: SiteId,
        /// Their account on that site.
        account: VisitorAccountId,
    },
    /// A WebAgent of a site, for example `cashier`.
    WebAgent {
        /// The site.
        site: SiteId,
        /// The WebAgent's role name.
        webagent: String,
    },
    /// An operator's person or operator WebAgent.
    Operator {
        /// The operator.
        operator: OperatorId,
        /// The person's account id, or `op:<webagent>`.
        actor: String,
    },
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Person { account } => write!(f, "person:{account}"),
            Self::Customer { site, account } => write!(f, "customer:{site}/{account}"),
            Self::WebAgent { site, webagent } => write!(f, "urn:adk:agent:{site}/{webagent}"),
            Self::Operator { operator, actor } => write!(f, "operator:{operator}/{actor}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kilele() -> SiteHost {
        SiteHost {
            site: SiteId::new("site_kilele"),
            host: "kilele-bakery-sample.agentrix.website".into(),
            origin: "https://kilele-bakery-sample.agentrix.website".into(),
            name: "Kilele Bakery".into(),
            sample: true,
        }
    }

    #[test]
    fn own_origin_only() {
        let site = kilele();
        assert!(site.is_own_origin("https://kilele-bakery-sample.agentrix.website"));
        assert!(site.is_own_origin("https://kilele-bakery-sample.agentrix.website/"));
        assert!(!site.is_own_origin("https://kilele-bakery-sample.agentrix.website.evil.example"));
        assert!(!site.is_own_origin("http://kilele-bakery-sample.agentrix.website"));
        assert!(site.is_https());
    }

    #[test]
    fn paths_cover_what_is_under_them() {
        let org = TenantPath {
            operator: "op_z".into(),
            partner: None,
            org: "org_k".into(),
            site: None,
        };
        let site = TenantPath {
            site: Some(SiteId::new("site_1")),
            ..org.clone()
        };
        assert!(org.covers(&site));
        assert!(!site.covers(&org));
        let other = TenantPath {
            org: "org_x".into(),
            ..site.clone()
        };
        assert!(!org.covers(&other));
    }

    #[test]
    fn ids_and_principals_read_as_text() {
        assert!(VisitorAccountId::generate().as_str().starts_with("vac_"));
        let p = Principal::WebAgent {
            site: SiteId::new("site_1"),
            webagent: "cashier".into(),
        };
        assert_eq!(p.to_string(), "urn:adk:agent:site_1/cashier");
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "web_agent");
    }
}
