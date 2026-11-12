//! Posting rules: what one PayPal event means in one company's books.
//!
//! | PayPal | Books |
//! |---|---|
//! | capture, or a subscription's sale | a receipt into PayPal clearing, applied to the invoice (or the customer's oldest open invoices) |
//! | refund | a credit note and the money back out of clearing; any fee PayPal returns comes back off the fee expense |
//! | payout | a bill payment out of clearing, applied to the bill (or the vendor's oldest open bills) |
//! | PayPal's fee on any of them | Dr PayPal fees, Cr PayPal clearing |
//!
//! Every rule is idempotent by the PayPal id: the receipt, payment or refund carries
//! [`PayPalRef::key`] and the fee journal [`PayPalRef::fee_key`], and a second run finds them and
//! posts nothing. That is what lets both companies post the same settlement from one PayPal id,
//! and a crash between the two halves (payment, then fee) finish on the next run.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::books::{
    AccountRole, BillApplication, BillId, BillPaymentDraft, Books, BooksError, BooksResult,
    CustomerId, DocumentFilter, DocumentRef, InvoiceApplication, InvoiceId, JournalDraft,
    JournalFilter, JournalId, JournalLine, PaymentFilter, PaymentId, ReceiptDraft, RefundDraft,
    VendorId,
};
use crate::money::Money;
use crate::paypal::PayPalRef;

/// One PayPal event to post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PayPalPosting {
    /// Money in: an order capture or a subscription's billing-cycle sale.
    Received {
        /// The capture or sale.
        paypal: PayPalRef,
        /// Who paid.
        customer: CustomerId,
        /// The gross amount the customer paid.
        gross: Money,
        /// PayPal's fee, taken from the gross (zero if none).
        fee: Money,
        /// The date PayPal reports.
        date: NaiveDate,
        /// The invoice it pays; `None` applies it to the customer's oldest open invoices.
        invoice: Option<InvoiceId>,
    },
    /// Money given back to a customer.
    Refunded {
        /// The refund.
        paypal: PayPalRef,
        /// Who is refunded.
        customer: CustomerId,
        /// The amount given back.
        amount: Money,
        /// Any part of the original fee PayPal gave back (zero if none).
        fee_returned: Money,
        /// The date PayPal reports.
        date: NaiveDate,
        /// The invoice the refund is against.
        invoice: Option<InvoiceId>,
        /// Why.
        reason: String,
    },
    /// Money out: one payout item to a vendor.
    PaidOut {
        /// The payout item.
        paypal: PayPalRef,
        /// Who was paid.
        vendor: VendorId,
        /// The amount paid.
        amount: Money,
        /// PayPal's fee on the payout, charged to us (zero if none).
        fee: Money,
        /// The date PayPal reports.
        date: NaiveDate,
        /// The bill it pays; `None` applies it to the vendor's oldest open bills.
        bill: Option<BillId>,
    },
}

impl PayPalPosting {
    /// The PayPal object this posting comes from.
    pub fn paypal(&self) -> &PayPalRef {
        match self {
            PayPalPosting::Received { paypal, .. }
            | PayPalPosting::Refunded { paypal, .. }
            | PayPalPosting::PaidOut { paypal, .. } => paypal,
        }
    }
}

/// What a posting left in the books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Posted {
    /// The PayPal object.
    pub paypal: PayPalRef,
    /// The receipt, vendor payment or refund payment.
    pub payment: Option<PaymentId>,
    /// The credit note, for a refund.
    pub credit_note: Option<crate::books::CreditNoteId>,
    /// The journal that records the refund, for books that record refunds as journals.
    pub journal: Option<JournalId>,
    /// The fee journal, when there was a fee.
    pub fee_journal: Option<JournalId>,
    /// The invoices or bills it was applied to.
    pub applied: Vec<(DocumentRef, Money)>,
    /// Whether everything was already in the books from an earlier run (nothing new posted).
    pub already_posted: bool,
}

/// Posts one PayPal event to `books`, once.
pub async fn post_paypal<B: Books + ?Sized>(
    books: &B,
    posting: &PayPalPosting,
) -> BooksResult<Posted> {
    match posting {
        PayPalPosting::Received {
            paypal,
            customer,
            gross,
            fee,
            date,
            invoice,
        } => {
            check_amounts(*gross, *fee)?;
            let key = paypal.key();
            let existing = books
                .payments(&PaymentFilter {
                    reference: Some(key.clone()),
                    direction: None,
                })
                .await?;
            let (payment, fresh) = match existing.into_iter().next() {
                Some(p) => (p, false),
                None => {
                    let applications =
                        invoice_applications(books, customer, *gross, invoice.as_ref()).await?;
                    let draft = ReceiptDraft {
                        customer: customer.clone(),
                        date: *date,
                        amount: *gross,
                        reference: key.clone(),
                        applications,
                        memo: Some(format!("PayPal {key}")),
                    };
                    (books.record_receipt(&draft).await?, true)
                }
            };
            let (fee_journal, fee_fresh) = post_fee(books, paypal, *fee, *date, false).await?;
            Ok(Posted {
                paypal: paypal.clone(),
                payment: Some(payment.id),
                credit_note: None,
                journal: None,
                fee_journal,
                applied: payment.applied,
                already_posted: !fresh && !fee_fresh,
            })
        }
        PayPalPosting::Refunded {
            paypal,
            customer,
            amount,
            fee_returned,
            date,
            invoice,
            reason,
        } => {
            check_amounts(*amount, *fee_returned)?;
            let key = paypal.key();
            let (refund, fresh) = match books.refund_by_reference(&key).await? {
                Some(r) => (r, false),
                None => {
                    let draft = RefundDraft {
                        customer: customer.clone(),
                        invoice: invoice.clone(),
                        date: *date,
                        amount: *amount,
                        reference: key.clone(),
                        reason: reason.clone(),
                    };
                    (books.record_refund(&draft).await?, true)
                }
            };
            let (fee_journal, fee_fresh) =
                post_fee(books, paypal, *fee_returned, *date, true).await?;
            Ok(Posted {
                paypal: paypal.clone(),
                payment: refund.payment,
                credit_note: refund.credit_note,
                journal: refund.journal,
                fee_journal,
                applied: invoice
                    .iter()
                    .map(|i| (DocumentRef::Invoice(i.clone()), *amount))
                    .collect(),
                already_posted: !fresh && !fee_fresh,
            })
        }
        PayPalPosting::PaidOut {
            paypal,
            vendor,
            amount,
            fee,
            date,
            bill,
        } => {
            check_amounts(*amount, *fee)?;
            let key = paypal.key();
            let existing = books
                .payments(&PaymentFilter {
                    reference: Some(key.clone()),
                    direction: None,
                })
                .await?;
            let (payment, fresh) = match existing.into_iter().next() {
                Some(p) => (p, false),
                None => {
                    let applications =
                        bill_applications(books, vendor, *amount, bill.as_ref()).await?;
                    let draft = BillPaymentDraft {
                        vendor: vendor.clone(),
                        date: *date,
                        amount: *amount,
                        reference: key.clone(),
                        applications,
                        memo: Some(format!("PayPal {key}")),
                    };
                    (books.record_bill_payment(&draft).await?, true)
                }
            };
            let (fee_journal, fee_fresh) = post_fee(books, paypal, *fee, *date, false).await?;
            Ok(Posted {
                paypal: paypal.clone(),
                payment: Some(payment.id),
                credit_note: None,
                journal: None,
                fee_journal,
                applied: payment.applied,
                already_posted: !fresh && !fee_fresh,
            })
        }
    }
}

/// The invoice a recurring payment pays, for [`PayPalPosting::Received`]'s `invoice`.
///
/// Raises whatever recurring invoices are due on `as_of` first, then takes the oldest open
/// invoice carrying `reference` (a subscription's [`crate::paypal::PayPalSubscriptionId::reference`],
/// or the reference a recurring invoice was set up with). `None` when nothing of it is open: the
/// payment then stays unapplied on the customer's account rather than paying another invoice.
pub async fn recurring_invoice_for<B: Books + ?Sized>(
    books: &B,
    reference: &str,
    as_of: NaiveDate,
) -> BooksResult<Option<InvoiceId>> {
    books.raise_due_recurring(as_of).await?;
    let filter = DocumentFilter {
        party: None,
        open_only: true,
        reference: Some(reference.to_owned()),
    };
    let mut open = books.invoices(&filter).await?;
    open.retain(|i| i.status.is_open() && i.reference.as_deref() == Some(reference));
    open.sort_by_key(|i| (i.due_date, i.issue_date));
    Ok(open.into_iter().next().map(|i| i.id))
}

fn check_amounts(amount: Money, fee: Money) -> BooksResult<()> {
    if !amount.is_positive() {
        return Err(BooksError::Invalid(format!(
            "a PayPal posting needs an amount above zero, not {amount}"
        )));
    }
    if fee.minor < 0 || fee.currency != amount.currency {
        return Err(BooksError::Invalid(format!(
            "the fee {fee} must be zero or more, in {}",
            amount.currency
        )));
    }
    if fee.minor > amount.minor {
        return Err(BooksError::Invalid(format!(
            "the fee {fee} is more than the amount {amount}"
        )));
    }
    Ok(())
}

/// The fee journal, once: Dr fees / Cr clearing, or the other way round when PayPal returns a fee.
async fn post_fee<B: Books + ?Sized>(
    books: &B,
    paypal: &PayPalRef,
    fee: Money,
    date: NaiveDate,
    returned: bool,
) -> BooksResult<(Option<JournalId>, bool)> {
    if fee.is_zero() {
        return Ok((None, false));
    }
    let key = paypal.fee_key();
    let found = books
        .journal(&JournalFilter {
            reference: Some(key.clone()),
            ..Default::default()
        })
        .await?;
    if let Some(j) = found.into_iter().next() {
        return Ok((Some(j.id), false));
    }
    let fees = books.account(AccountRole::PayPalFees).await?.code;
    let clearing = books.account(AccountRole::PayPalClearing).await?.code;
    let zero = Money::zero(fee.currency);
    let (dr, cr) = if returned {
        (clearing, fees)
    } else {
        (fees, clearing)
    };
    let memo = if returned {
        format!("PayPal fee returned on {paypal}")
    } else {
        format!("PayPal fee on {paypal}")
    };
    let draft = JournalDraft {
        date,
        memo,
        reference: Some(key),
        lines: vec![
            JournalLine {
                account: dr,
                debit: fee,
                credit: zero,
                memo: None,
            },
            JournalLine {
                account: cr,
                debit: zero,
                credit: fee,
                memo: None,
            },
        ],
    };
    Ok((Some(books.post_journal(&draft).await?.id), true))
}

/// Applies `amount` to the named invoice, or to the customer's open invoices, oldest due first.
async fn invoice_applications<B: Books + ?Sized>(
    books: &B,
    customer: &CustomerId,
    amount: Money,
    invoice: Option<&InvoiceId>,
) -> BooksResult<Vec<InvoiceApplication>> {
    let mut open = match invoice {
        Some(id) => vec![books.invoice(id).await?],
        None => {
            let filter = DocumentFilter {
                party: Some(customer.0.clone()),
                open_only: true,
                reference: None,
            };
            books.invoices(&filter).await?
        }
    };
    open.retain(|i| {
        i.status.is_open() && &i.customer == customer && i.balance_due.currency == amount.currency
    });
    open.sort_by_key(|d| (d.due_date, d.issue_date));
    let mut left = amount;
    let mut out = Vec::new();
    for inv in open {
        if !left.is_positive() {
            break;
        }
        let take = if inv.balance_due.minor < left.minor {
            inv.balance_due
        } else {
            left
        };
        if take.is_positive() {
            out.push(InvoiceApplication {
                invoice: inv.id,
                amount: take,
            });
            left = left.checked_sub(take)?;
        }
    }
    Ok(out)
}

/// Applies `amount` to the named bill, or to the vendor's open bills, oldest due first.
async fn bill_applications<B: Books + ?Sized>(
    books: &B,
    vendor: &VendorId,
    amount: Money,
    bill: Option<&BillId>,
) -> BooksResult<Vec<BillApplication>> {
    let mut open = match bill {
        Some(id) => vec![books.bill(id).await?],
        None => {
            let filter = DocumentFilter {
                party: Some(vendor.0.clone()),
                open_only: true,
                reference: None,
            };
            books.bills(&filter).await?
        }
    };
    open.retain(|b| {
        b.status.is_open() && &b.vendor == vendor && b.balance_due.currency == amount.currency
    });
    open.sort_by_key(|d| (d.due_date, d.issue_date));
    let mut left = amount;
    let mut out = Vec::new();
    for b in open {
        if !left.is_positive() {
            break;
        }
        let take = if b.balance_due.minor < left.minor {
            b.balance_due
        } else {
            left
        };
        if take.is_positive() {
            out.push(BillApplication {
                bill: b.id,
                amount: take,
            });
            left = left.checked_sub(take)?;
        }
    }
    Ok(out)
}
