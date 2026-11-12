//! The contract types slices P and K serve, from saas-core (`saas_core::identity` and
//! `saas_core::wallet`): plain bodies, signed here.

pub use saas_core::identity::{
    Badge, BadgeCredentials, BadgeKey, BadgeSite, BadgeStatus, BindingRef, BindingStatus,
    MandateRef, PayPalBinding, PayPalVaultIds, Sponsor,
};
pub use saas_core::wallet::{Reconciled, WalletCheckpointRef};
