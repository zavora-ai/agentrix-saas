//! Agentrix SaaS shared contracts (slice A owns them; slices B, C and D build on them).
//!
//! - [`ids`]: companies and their agents, as adk-rust identities.
//! - [`money`]: [`Money`] in integer minor units, with its [`Currency`].
//! - [`paypal`]: PayPal's ids, typed, and [`PayPalRef`], the key every posting is idempotent by.
//! - [`books`]: the [`Books`] trait over one company's books in Zavora ERA (hosted): customers
//!   and vendors, invoices and recurring invoices, bills, payments through PayPal clearing,
//!   journals and audit, ageing and credit limits, and the CRM.
//! - [`posting`]: the rules that turn one PayPal event into entries in one company's books.
//! - `fake` (feature `fake`): [`fake::FakeBooks`], the whole contract in memory, for tests.
//!
//! - [`identity`] (v3): an agent's [`identity::Badge`] and the owner's passkey-signed
//!   [`identity::PayPalBinding`].
//! - [`wallet`] (v3): each money-handling agent's wallet record: [`wallet::WalletEntry`],
//!   [`wallet::WalletCheckpoint`], the three-way match and the board's feed.
//!
//! - [`federation`] (v3): the traits behind the cloud providers' agent identities, out
//!   ([`federation::AgentTokenIssuer`]) and in ([`federation::AgentTokens`]).
//! - [`spend`] (v3): the platform's metered model spend as the providers' vendor bills, once per
//!   window ([`spend::bill_model_spend`]).
//!
//! Mandates and settlement types belong to slice C.

pub mod books;
#[cfg(any(test, feature = "fake"))]
pub mod fake;
pub mod federation;
pub mod identity;
pub mod ids;
pub mod money;
pub mod paypal;
pub mod posting;
pub mod spend;
#[cfg(test)]
mod tests;
pub mod wallet;

pub use books::{Books, BooksError, BooksResult, Counterparty};
pub use ids::{AgentId, CompanyId, IdError};
pub use money::{Currency, Money, MoneyError};
pub use paypal::PayPalRef;
pub use posting::{PayPalPosting, Posted, post_paypal, recurring_invoice_for};
