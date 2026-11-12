//! What settlement needs from each company's books, and nothing more.
//!
//! The payee reads the open invoice and posts money received; the payer records the bill and
//! posts money paid. Both posts cite the same PayPal key ([`PayPalSettlement::key`]), and both
//! are idempotent by it, so a retry never posts twice. [`crate::era`] implements these over
//! saas-core's `Books` trait (Zavora ERA, or `FakeBooks` in tests).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::protocol::{Invoice, SettlementReceipt};

#[cfg(doc)]
use crate::protocol::PayPalSettlement;

/// What a posting left in the books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostedRef {
    /// The PayPal key the posting cites.
    pub paypal_key: String,
    /// The books' payment id.
    pub payment_id: Option<String>,
    /// True when an earlier run had already posted it.
    pub already_posted: bool,
}

/// The payee's books.
#[async_trait]
pub trait PayeeBooks: Send + Sync {
    /// The invoice with this number as the books hold it now (its open amount net of what was
    /// paid), addressed to a counterparty company; `None` if there is no such invoice.
    async fn invoice(&self, invoice_id: &str) -> anyhow::Result<Option<Invoice>>;
    /// The books' own id for the invoice with this number (or id), when the books have one.
    async fn books_id(&self, _invoice_id: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    /// Posts money received for `receipt.invoice_id`: Dr PayPal clearing, Cr receivables,
    /// and PayPal's fee. Idempotent by the receipt's PayPal key.
    async fn post_received(&self, receipt: &SettlementReceipt) -> anyhow::Result<PostedRef>;
}

/// The payer's books.
#[async_trait]
pub trait PayerBooks: Send + Sync {
    /// Records the payee's invoice as a bill (once per payee and invoice number), and returns
    /// the books' bill id.
    async fn record_bill(&self, invoice: &Invoice) -> anyhow::Result<String>;
    /// Posts money paid for `receipt.invoice_id`: Dr payables, Cr PayPal clearing.
    /// Idempotent by the receipt's PayPal key.
    async fn post_paid(&self, receipt: &SettlementReceipt) -> anyhow::Result<PostedRef>;
}
