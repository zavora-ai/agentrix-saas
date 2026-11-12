//! Settlement over saas-core's [`Books`] (Zavora ERA on the node, `FakeBooks` in tests).
//!
//! - The payee's invoice is the books' sales invoice, addressed to a customer whose
//!   counterparty is a company on this node; its open amount is the books' balance due.
//! - Money received posts with saas-core's posting rules as [`PayPalPosting::Received`]:
//!   a receipt into PayPal clearing applied to the invoice, and the payee's PayPal fee.
//! - The payer records the invoice as a vendor bill (once, by the payee's invoice number), and
//!   money paid posts as [`PayPalPosting::PaidOut`]: a bill payment out of PayPal clearing.
//!
//! Both posts carry the same [`PayPalRef`] key (`paypal:capture:<id>` on the Vault route,
//! `paypal:payout:<item id>` on the Payout route), and saas-core makes each idempotent by it.

use std::sync::Arc;

use agentrix_types::Money;
use async_trait::async_trait;
use saas_core::books::{
    Bill, BillDraft, BillId, CustomerId, DocumentFilter, Invoice as BooksInvoice, LineDraft,
    VendorId,
};
use saas_core::paypal::{PayPalCaptureId, PayPalPayoutItemId};
use saas_core::{Books, CompanyId, Counterparty, Currency, PayPalPosting, PayPalRef, post_paypal};

use crate::books::{PayeeBooks, PayerBooks, PostedRef};
use crate::protocol::{Invoice, InvoiceLine, Route, SettlementReceipt};

/// A company on this node: who it is in the books and where its agents' keys are published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownCompany {
    /// Its id, as counterparties in the books name it.
    pub company: CompanyId,
    /// Its name.
    pub name: String,
    /// Its key directory origin.
    pub origin: String,
    /// Its PayPal email, when known.
    pub paypal_email: Option<String>,
}

fn to_core(money: &Money) -> anyhow::Result<saas_core::Money> {
    Ok(saas_core::Money::new(
        money.minor,
        Currency::new(&money.currency)?,
    ))
}

fn from_core(money: &saas_core::Money) -> Money {
    Money {
        currency: money.currency.code().to_owned(),
        minor: money.minor,
    }
}

fn paypal_ref(receipt: &SettlementReceipt) -> anyhow::Result<PayPalRef> {
    Ok(match receipt.paypal.kind.as_str() {
        "capture" => PayPalRef::Capture(PayPalCaptureId::new(receipt.paypal.id.clone())?),
        "payout" => PayPalRef::Payout(PayPalPayoutItemId::new(receipt.paypal.id.clone())?),
        other => anyhow::bail!("no books posting for a PayPal {other}"),
    })
}

/// The fee each side carries: the payee pays PayPal's fee on a capture, the payer on a payout.
fn fee_for(receipt: &SettlementReceipt, payee: bool) -> anyhow::Result<saas_core::Money> {
    let currency = Currency::new(&receipt.amount.currency)?;
    let carries = matches!(
        (receipt.route, payee),
        (Route::Vault, true) | (Route::Payout, false)
    );
    match (&receipt.paypal.fee, carries) {
        (Some(fee), true) => to_core(fee),
        _ => Ok(saas_core::Money::zero(currency)),
    }
}

/// The payee's books.
pub struct EraPayee {
    books: Arc<dyn Books>,
    us: KnownCompany,
    counterparties: Vec<KnownCompany>,
}

impl std::fmt::Debug for EraPayee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EraPayee")
            .field("us", &self.us)
            .finish_non_exhaustive()
    }
}

impl EraPayee {
    /// The payee `us` over `books`, settling with `counterparties`.
    pub fn new(books: Arc<dyn Books>, us: KnownCompany, counterparties: Vec<KnownCompany>) -> Self {
        Self {
            books,
            us,
            counterparties,
        }
    }

    async fn find(&self, invoice_id: &str) -> anyhow::Result<Option<BooksInvoice>> {
        let all = self.books.invoices(&DocumentFilter::default()).await?;
        Ok(all
            .into_iter()
            .find(|i| i.number.as_deref() == Some(invoice_id) || i.id.0 == invoice_id))
    }
}

#[async_trait]
impl PayeeBooks for EraPayee {
    async fn books_id(&self, invoice_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self.find(invoice_id).await?.map(|i| i.id.0))
    }

    async fn invoice(&self, invoice_id: &str) -> anyhow::Result<Option<Invoice>> {
        let Some(invoice) = self.find(invoice_id).await? else {
            return Ok(None);
        };
        let customer = self.books.customer(&invoice.customer).await?;
        let Some(payer) = customer
            .counterparty
            .company
            .as_ref()
            .and_then(|c| self.counterparties.iter().find(|k| k.company == *c))
        else {
            return Ok(None);
        };
        let description = invoice
            .lines
            .iter()
            .map(|l| l.description.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        Ok(Some(Invoice {
            invoice_id: invoice
                .number
                .clone()
                .unwrap_or_else(|| invoice.id.0.clone()),
            payee: self.us.origin.clone(),
            payee_name: self.us.name.clone(),
            payer: payer.origin.clone(),
            payer_name: payer.name.clone(),
            total: from_core(&invoice.total),
            open: from_core(&invoice.balance_due),
            issued_on: invoice.issue_date,
            due_on: invoice.due_date,
            description,
            lines: invoice
                .lines
                .iter()
                .map(|l| InvoiceLine {
                    description: l.description.clone(),
                    amount: from_core(&l.amount),
                })
                .collect(),
        }))
    }

    async fn post_received(&self, receipt: &SettlementReceipt) -> anyhow::Result<PostedRef> {
        let invoice = self
            .find(&receipt.invoice_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no invoice {} in the books", receipt.invoice_id))?;
        let paypal = paypal_ref(receipt)?;
        let posted = post_paypal(
            self.books.as_ref(),
            &PayPalPosting::Received {
                paypal: paypal.clone(),
                customer: invoice.customer.clone(),
                gross: to_core(&receipt.amount)?,
                fee: fee_for(receipt, true)?,
                date: receipt.settled_at.date_naive(),
                invoice: Some(invoice.id.clone()),
            },
        )
        .await?;
        Ok(PostedRef {
            paypal_key: paypal.key(),
            payment_id: posted.payment.map(|p| p.0),
            already_posted: posted.already_posted,
        })
    }
}

/// The payer's books.
pub struct EraPayer {
    books: Arc<dyn Books>,
    counterparties: Vec<KnownCompany>,
}

impl std::fmt::Debug for EraPayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EraPayer").finish_non_exhaustive()
    }
}

impl EraPayer {
    /// The payer over `books`, settling with `counterparties`.
    pub fn new(books: Arc<dyn Books>, counterparties: Vec<KnownCompany>) -> Self {
        Self {
            books,
            counterparties,
        }
    }

    fn payee(&self, origin: &str) -> anyhow::Result<&KnownCompany> {
        self.counterparties
            .iter()
            .find(|k| k.origin == origin)
            .ok_or_else(|| anyhow::anyhow!("{origin} isn't a known company"))
    }

    async fn vendor(&self, payee: &KnownCompany, currency: Currency) -> anyhow::Result<VendorId> {
        let vendor = self
            .books
            .ensure_vendor(&Counterparty {
                name: payee.name.clone(),
                email: None,
                currency,
                company: Some(payee.company.clone()),
                paypal_email: payee.paypal_email.clone(),
            })
            .await?;
        Ok(vendor.id)
    }

    async fn bill(&self, vendor: &VendorId, invoice_id: &str) -> anyhow::Result<Option<Bill>> {
        let bills = self
            .books
            .bills(&DocumentFilter {
                party: Some(vendor.0.clone()),
                ..DocumentFilter::default()
            })
            .await?;
        Ok(bills
            .into_iter()
            .find(|b| b.vendor_reference.as_deref() == Some(invoice_id)))
    }
}

#[async_trait]
impl PayerBooks for EraPayer {
    async fn record_bill(&self, invoice: &Invoice) -> anyhow::Result<String> {
        let payee = self.payee(&invoice.payee)?;
        let currency = Currency::new(&invoice.total.currency)?;
        let vendor = self.vendor(payee, currency).await?;
        if let Some(bill) = self.bill(&vendor, &invoice.invoice_id).await? {
            return Ok(bill.id.0);
        }
        let mut lines: Vec<LineDraft> = invoice
            .lines
            .iter()
            .map(|l| {
                Ok(LineDraft {
                    product: None,
                    description: l.description.clone(),
                    quantity: 1,
                    unit_price: to_core(&l.amount)?,
                })
            })
            .collect::<anyhow::Result<_>>()?;
        let listed: i64 = invoice.lines.iter().map(|l| l.amount.minor).sum();
        if lines.is_empty() || listed != invoice.total.minor {
            lines = vec![LineDraft {
                product: None,
                description: invoice.description.clone(),
                quantity: 1,
                unit_price: to_core(&invoice.total)?,
            }];
        }
        let draft = self
            .books
            .draft_bill(&BillDraft {
                vendor,
                vendor_reference: Some(invoice.invoice_id.clone()),
                issue_date: invoice.issued_on,
                due_date: invoice.due_on,
                currency,
                lines,
                memo: Some(format!(
                    "{} invoice {}, offered by its Receivables agent",
                    invoice.payee_name, invoice.invoice_id
                )),
            })
            .await?;
        let bill = self.books.post_bill(&draft.id).await?;
        Ok(bill.id.0)
    }

    async fn post_paid(&self, receipt: &SettlementReceipt) -> anyhow::Result<PostedRef> {
        let payee = self.payee(&receipt.payee)?;
        let currency = Currency::new(&receipt.amount.currency)?;
        let vendor = self.vendor(payee, currency).await?;
        let bill = self.bill(&vendor, &receipt.invoice_id).await?;
        let paypal = paypal_ref(receipt)?;
        let posted = post_paypal(
            self.books.as_ref(),
            &PayPalPosting::PaidOut {
                paypal: paypal.clone(),
                vendor,
                amount: to_core(&receipt.amount)?,
                fee: fee_for(receipt, false)?,
                date: receipt.settled_at.date_naive(),
                bill: bill.map(|b| b.id),
            },
        )
        .await?;
        Ok(PostedRef {
            paypal_key: paypal.key(),
            payment_id: posted.payment.map(|p| p.0),
            already_posted: posted.already_posted,
        })
    }
}

/// The customer id behind an invoice number (for tests and the board).
pub async fn customer_of(
    books: &dyn Books,
    invoice_id: &str,
) -> anyhow::Result<Option<CustomerId>> {
    let all = books.invoices(&DocumentFilter::default()).await?;
    Ok(all
        .into_iter()
        .find(|i| i.number.as_deref() == Some(invoice_id) || i.id.0 == invoice_id)
        .map(|i| i.customer))
}

/// The bill id the payer recorded for a payee's invoice (for tests and the board).
pub async fn bill_of(
    books: &dyn Books,
    invoice_id: &str,
) -> anyhow::Result<Option<(BillId, saas_core::Money)>> {
    let all = books.bills(&DocumentFilter::default()).await?;
    Ok(all
        .into_iter()
        .find(|b| b.vendor_reference.as_deref() == Some(invoice_id))
        .map(|b| (b.id, b.balance_due)))
}
