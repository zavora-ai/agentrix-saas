//! Agentrix SaaS books: [`EraBooks`], the `saas-core` [`Books`](saas_core::Books) contract over
//! Zavora ERA (hosted), one login per company.
//!
//! ```no_run
//! # async fn demo() -> Result<(), saas_core::BooksError> {
//! use saas_books::{EraBooks, EraConfig};
//! use saas_core::{Books, CompanyId};
//!
//! let books = EraBooks::connect(EraConfig::from_env(CompanyId::agentrix())?).await?;
//! let clearing = books.account(saas_core::books::AccountRole::PayPalClearing).await?;
//! # let _ = clearing; Ok(()) }
//! ```
//!
//! Posting PayPal events uses the shared rules in `saas_core::posting` over these books.
//! The local instance and its seed are in `deploy/saas/era/`.

mod era;
mod map;
#[cfg(feature = "mcp")]
mod mcp;
mod rest;

pub use era::{AccountMap, EraBooks, EraConfig};
#[cfg(feature = "mcp")]
pub use mcp::McpConfig;
