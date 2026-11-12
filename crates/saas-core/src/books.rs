//! The books: one company's double-entry ledger, receivables, payables and CRM, held in
//! Zavora ERA (hosted). [`Books`] is what every agent sees; `saas-books::EraBooks` implements it
//! over ERA, and [`crate::fake::FakeBooks`] in memory for tests.
//!
//! Every amount is [`Money`]; every PayPal posting carries its [`PayPalRef`] key as the
//! document's `reference`, so the same PayPal id never posts twice.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::ids::CompanyId;
use crate::money::{Currency, Money, MoneyError};

macro_rules! books_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// The id as the books wrote it.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }
    };
}

books_id!(
    /// A customer in the books.
    CustomerId
);
books_id!(
    /// A vendor (supplier) in the books.
    VendorId
);
books_id!(
    /// A product or service on the price list.
    ProductId
);
books_id!(
    /// A sales invoice (receivable).
    InvoiceId
);
books_id!(
    /// A recurring-invoice schedule.
    RecurringInvoiceId
);
books_id!(
    /// A vendor bill (payable).
    BillId
);
books_id!(
    /// A payment: a customer receipt or a vendor payment.
    PaymentId
);
books_id!(
    /// A customer credit note.
    CreditNoteId
);
books_id!(
    /// A journal entry.
    JournalId
);
books_id!(
    /// A CRM lead.
    LeadId
);
books_id!(
    /// A CRM opportunity.
    OpportunityId
);
books_id!(
    /// A support ticket.
    TicketId
);
books_id!(
    /// A general-ledger account code, for example `1010`.
    AccountCode
);

/// Why the books refused or failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BooksError {
    /// No such record.
    #[error("{what} {id} not found in the books")]
    NotFound {
        /// What was looked for, for example `"invoice"`.
        what: &'static str,
        /// The id or key looked for.
        id: String,
    },
    /// The request is malformed: an unbalanced journal, a mixed-currency invoice, no lines.
    #[error("invalid: {0}")]
    Invalid(String),
    /// The record is in the wrong state for this: posting a posted invoice, over-applying a payment.
    #[error("conflict: {0}")]
    Conflict(String),
    /// This backend has no such operation.
    #[error("not supported by these books: {0}")]
    Unsupported(&'static str),
    /// The books can't be reached right now; the same call may be retried.
    #[error("books unavailable: {0}")]
    Unavailable(String),
    /// The books answered with an error.
    #[error("books refused: {0}")]
    Backend(String),
    /// An amount the books returned or were given doesn't add up.
    #[error(transparent)]
    Money(#[from] MoneyError),
}

impl BooksError {
    /// Whether the same call may succeed if retried later.
    pub fn retryable(&self) -> bool {
        matches!(self, BooksError::Unavailable(_))
    }
}

/// Shorthand for a books result.
pub type BooksResult<T> = Result<T, BooksError>;

/// Who a counterparty is, independent of whether we sell to them or buy from them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counterparty {
    /// Legal or trading name. Customers and vendors are matched by this name, case-insensitively.
    pub name: String,
    /// Billing email.
    pub email: Option<String>,
    /// The currency we trade with them in.
    pub currency: Currency,
    /// Set when the counterparty is itself a company on this node (Kilele Bakery, to Agentrix).
    pub company: Option<CompanyId>,
    /// Their PayPal account email: where payouts go, or who pays us.
    pub paypal_email: Option<String>,
}

impl Counterparty {
    /// A counterparty trading in US dollars.
    pub fn usd(name: impl Into<String>, email: Option<&str>) -> Self {
        Self {
            name: name.into(),
            email: email.map(str::to_owned),
            currency: Currency::USD,
            company: None,
            paypal_email: None,
        }
    }
}

/// A customer: someone who owes us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Customer {
    /// The books' id.
    pub id: CustomerId,
    /// Who they are.
    pub counterparty: Counterparty,
    /// The most we let them owe; `None` is no limit set.
    pub credit_limit: Option<Money>,
    /// What they owe now (open receivables, net of unapplied credits).
    pub balance: Money,
}

/// A vendor: someone we owe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vendor {
    /// The books' id.
    pub id: VendorId,
    /// Who they are.
    pub counterparty: Counterparty,
    /// What we owe them now.
    pub balance: Money,
}

/// A product or service to put on the price list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDraft {
    /// Name, for example "WebAgent Pro — monthly". Products are matched by this name.
    pub name: String,
    /// Stock-keeping code, for example `WA-PRO-M`.
    pub sku: Option<String>,
    /// List price per unit.
    pub price: Money,
    /// Description printed on invoices.
    pub description: Option<String>,
}

/// A product or service on the price list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Product {
    /// The books' id.
    pub id: ProductId,
    /// Name.
    pub name: String,
    /// Stock-keeping code.
    pub sku: Option<String>,
    /// List price per unit.
    pub price: Money,
}

/// One line to put on an invoice or bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDraft {
    /// The product, when the line is for one.
    pub product: Option<ProductId>,
    /// What the line is for.
    pub description: String,
    /// How many.
    pub quantity: u32,
    /// Price per unit.
    pub unit_price: Money,
}

/// One line on an invoice or bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// The product, when the line is for one.
    pub product: Option<ProductId>,
    /// What the line is for.
    pub description: String,
    /// How many.
    pub quantity: u32,
    /// Price per unit.
    pub unit_price: Money,
    /// `quantity × unit_price`, as the books computed it.
    pub amount: Money,
}

/// Where an invoice or bill is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    /// Being written; not in the ledger.
    Draft,
    /// Waiting on approval; not in the ledger.
    Submitted,
    /// In the ledger, nothing paid.
    Posted,
    /// In the ledger, part paid.
    PartiallyPaid,
    /// Paid in full.
    Paid,
    /// Cancelled before posting, or voided after.
    Void,
}

impl DocumentStatus {
    /// Whether the document is in the ledger with something still owed.
    pub fn is_open(&self) -> bool {
        matches!(self, DocumentStatus::Posted | DocumentStatus::PartiallyPaid)
    }
}

/// A sales invoice to draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceDraft {
    /// Who owes it.
    pub customer: CustomerId,
    /// The invoice date.
    pub issue_date: NaiveDate,
    /// When payment is due; ageing counts from here.
    pub due_date: NaiveDate,
    /// The invoice currency; every line must be in it.
    pub currency: Currency,
    /// What is billed.
    pub lines: Vec<LineDraft>,
    /// Our reference, for example the A2A request id or `paypal:subscription:I-…`.
    pub reference: Option<String>,
    /// A note printed on the invoice.
    pub memo: Option<String>,
}

/// A sales invoice: money a customer owes us.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    /// The books' id.
    pub id: InvoiceId,
    /// The human number, for example `INV-0007`, once the books assign one.
    pub number: Option<String>,
    /// Who owes it.
    pub customer: CustomerId,
    /// Where it is in its life.
    pub status: DocumentStatus,
    /// The invoice date.
    pub issue_date: NaiveDate,
    /// When payment is due.
    pub due_date: NaiveDate,
    /// What is billed.
    pub lines: Vec<Line>,
    /// The invoice total.
    pub total: Money,
    /// What is still owed.
    pub balance_due: Money,
    /// Our reference.
    pub reference: Option<String>,
}

impl Invoice {
    /// What has been paid on this invoice: `total − balance_due`.
    pub fn amount_paid(&self) -> Result<Money, MoneyError> {
        self.total.checked_sub(self.balance_due)
    }
}

/// Which invoices or bills to list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentFilter {
    /// Only this counterparty's (a customer id for invoices, a vendor id for bills).
    pub party: Option<String>,
    /// Only those in the ledger with something owed.
    pub open_only: bool,
    /// Only those with this reference.
    pub reference: Option<String>,
}

/// How often a recurring invoice is raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// Every week.
    Weekly,
    /// Every month.
    Monthly,
    /// Every three months.
    Quarterly,
    /// Every year.
    Yearly,
}

/// A recurring-invoice schedule to set up, for example a monthly plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringInvoiceDraft {
    /// Who is billed.
    pub customer: CustomerId,
    /// The currency of every invoice raised.
    pub currency: Currency,
    /// What each invoice bills.
    pub lines: Vec<LineDraft>,
    /// How often.
    pub frequency: Frequency,
    /// The first invoice date.
    pub start_date: NaiveDate,
    /// No invoices after this date; `None` runs until stopped.
    pub end_date: Option<NaiveDate>,
    /// Days from each invoice date to its due date.
    pub due_in_days: u32,
    /// Our reference, for example `paypal:subscription:I-J53UUW5DF0KV`.
    pub reference: Option<String>,
}

/// A recurring-invoice schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringInvoice {
    /// The books' id.
    pub id: RecurringInvoiceId,
    /// Who is billed.
    pub customer: CustomerId,
    /// How often.
    pub frequency: Frequency,
    /// The total of each invoice raised.
    pub amount: Money,
    /// The next invoice date; `None` once finished or stopped.
    pub next_date: Option<NaiveDate>,
    /// No invoices after this date.
    pub end_date: Option<NaiveDate>,
    /// Whether it is still raising invoices.
    pub active: bool,
    /// Our reference.
    pub reference: Option<String>,
}

/// A vendor bill to draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillDraft {
    /// Who we owe.
    pub vendor: VendorId,
    /// The vendor's own invoice number.
    pub vendor_reference: Option<String>,
    /// The bill date.
    pub issue_date: NaiveDate,
    /// When we must pay.
    pub due_date: NaiveDate,
    /// The bill currency; every line must be in it.
    pub currency: Currency,
    /// What is billed.
    pub lines: Vec<LineDraft>,
    /// A note.
    pub memo: Option<String>,
}

/// A vendor bill: money we owe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bill {
    /// The books' id.
    pub id: BillId,
    /// The human number, once the books assign one.
    pub number: Option<String>,
    /// Who we owe.
    pub vendor: VendorId,
    /// The vendor's own invoice number.
    pub vendor_reference: Option<String>,
    /// Where it is in its life.
    pub status: DocumentStatus,
    /// The bill date.
    pub issue_date: NaiveDate,
    /// When we must pay.
    pub due_date: NaiveDate,
    /// The bill total.
    pub total: Money,
    /// What is still owed.
    pub balance_due: Money,
}

/// A document in the books, for audit reads and to say what a payment was applied to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum DocumentRef {
    /// A customer.
    Customer(CustomerId),
    /// A vendor.
    Vendor(VendorId),
    /// A sales invoice.
    Invoice(InvoiceId),
    /// A vendor bill.
    Bill(BillId),
    /// A payment.
    Payment(PaymentId),
    /// A credit note.
    CreditNote(CreditNoteId),
    /// A journal entry.
    Journal(JournalId),
}

/// Part of a receipt applied to one invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceApplication {
    /// The invoice paid.
    pub invoice: InvoiceId,
    /// How much of it.
    pub amount: Money,
}

/// Part of a vendor payment applied to one bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillApplication {
    /// The bill paid.
    pub bill: BillId,
    /// How much of it.
    pub amount: Money,
}

/// Money received from a customer into the PayPal clearing account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptDraft {
    /// Who paid.
    pub customer: CustomerId,
    /// The date the money arrived.
    pub date: NaiveDate,
    /// The gross amount received (before PayPal's fee).
    pub amount: Money,
    /// The PayPal reference, stored as [`crate::PayPalRef::key`]: the receipt's idempotency key.
    pub reference: String,
    /// Which invoices it pays; any remainder stays unapplied on the customer's account.
    pub applications: Vec<InvoiceApplication>,
    /// A note.
    pub memo: Option<String>,
}

/// Money paid to a vendor out of the PayPal clearing account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillPaymentDraft {
    /// Who was paid.
    pub vendor: VendorId,
    /// The date it left.
    pub date: NaiveDate,
    /// The amount paid (before PayPal's fee).
    pub amount: Money,
    /// The PayPal reference: the payment's idempotency key.
    pub reference: String,
    /// Which bills it pays.
    pub applications: Vec<BillApplication>,
    /// A note.
    pub memo: Option<String>,
}

/// Which way a payment moved money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentDirection {
    /// From a customer to us.
    Received,
    /// From us to a vendor, or back to a customer as a refund.
    Paid,
}

/// A payment recorded in the books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payment {
    /// The books' id.
    pub id: PaymentId,
    /// The human number, once the books assign one.
    pub number: Option<String>,
    /// Which way the money moved.
    pub direction: PaymentDirection,
    /// The customer or vendor.
    pub party: DocumentRef,
    /// The payment date.
    pub date: NaiveDate,
    /// The amount.
    pub amount: Money,
    /// The idempotency key (a PayPal reference key).
    pub reference: Option<String>,
    /// What it was applied to, and how much to each.
    pub applied: Vec<(DocumentRef, Money)>,
    /// What is left unapplied.
    pub unapplied: Money,
    /// The journal entry the payment posted, when the books report it.
    pub journal: Option<JournalId>,
}

/// Which payments to list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentFilter {
    /// Only this reference (a PayPal reference key).
    pub reference: Option<String>,
    /// Only this direction.
    pub direction: Option<PaymentDirection>,
}

/// Money given back to a customer: a credit note, and the refund out of PayPal clearing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefundDraft {
    /// Who is refunded.
    pub customer: CustomerId,
    /// The invoice the refund is against, when there is one.
    pub invoice: Option<InvoiceId>,
    /// The date of the refund.
    pub date: NaiveDate,
    /// The amount given back.
    pub amount: Money,
    /// The PayPal refund's reference key: the refund's idempotency key.
    pub reference: String,
    /// Why, in plain words.
    pub reason: String,
}

/// A refund in the books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refund {
    /// The credit note that reduces the customer's balance and the revenue, when the books
    /// record the refund as one.
    pub credit_note: Option<CreditNoteId>,
    /// The payment back to the customer out of PayPal clearing, when the books record one.
    pub payment: Option<PaymentId>,
    /// The journal that records the refund, when the books record it as a journal (ERA: Dr
    /// sales returns, Cr PayPal clearing, because a paid invoice takes no credit note).
    pub journal: Option<JournalId>,
    /// The amount given back.
    pub amount: Money,
    /// The idempotency key.
    pub reference: String,
}

/// The accounts the posting rules need, mapped onto each company's own chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    /// The USD bank account "PayPal clearing": PayPal's balance as the books see it.
    PayPalClearing,
    /// Trade receivables.
    Receivables,
    /// Trade payables.
    Payables,
    /// Sales of plans.
    Revenue,
    /// PayPal's fees, an expense.
    PayPalFees,
}

/// A general-ledger account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// The account code.
    pub code: AccountCode,
    /// The account name.
    pub name: String,
    /// Its balance as debits minus credits, when the books report it.
    pub balance: Option<Money>,
}

/// One line of a journal entry to post. Exactly one of `debit` and `credit` is non-zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLine {
    /// The account.
    pub account: AccountCode,
    /// The debit.
    pub debit: Money,
    /// The credit.
    pub credit: Money,
    /// A note for the line.
    pub memo: Option<String>,
}

/// A manual journal entry to post, for example a PayPal fee. Debits must equal credits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalDraft {
    /// The posting date.
    pub date: NaiveDate,
    /// What it is, in plain words.
    pub memo: String,
    /// The idempotency key, for example `paypal:capture:…:fee`.
    pub reference: Option<String>,
    /// The lines.
    pub lines: Vec<JournalLine>,
}

impl JournalDraft {
    /// Checks the lines: at least two, one currency, each one-sided and non-negative, and
    /// balanced.
    pub fn check(&self) -> BooksResult<Money> {
        let first = self
            .lines
            .first()
            .ok_or_else(|| BooksError::Invalid("a journal needs lines".into()))?;
        let currency = first.debit.currency;
        let (mut dr, mut cr) = (Money::zero(currency), Money::zero(currency));
        if self.lines.len() < 2 {
            return Err(BooksError::Invalid(
                "a journal needs at least two lines".into(),
            ));
        }
        for l in &self.lines {
            if l.debit.minor < 0
                || l.credit.minor < 0
                || (l.debit.minor == 0) == (l.credit.minor == 0)
            {
                return Err(BooksError::Invalid(format!(
                    "line on {} must be a debit or a credit, above zero",
                    l.account
                )));
            }
            dr = dr.checked_add(l.debit)?;
            cr = cr.checked_add(l.credit)?;
        }
        if dr != cr {
            return Err(BooksError::Invalid(format!(
                "debits {dr} do not equal credits {cr}"
            )));
        }
        Ok(dr)
    }
}

/// A journal entry in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// The books' id.
    pub id: JournalId,
    /// The human number.
    pub number: Option<String>,
    /// The posting date.
    pub date: NaiveDate,
    /// What it is.
    pub memo: String,
    /// The reference it carries.
    pub reference: Option<String>,
    /// The lines.
    pub lines: Vec<JournalLine>,
}

/// Which journal entries to read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalFilter {
    /// From this date, inclusive.
    pub from: Option<NaiveDate>,
    /// To this date, inclusive.
    pub to: Option<NaiveDate>,
    /// Only entries carrying this reference.
    pub reference: Option<String>,
}

/// One change the books recorded about a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// When, as the books wrote it (RFC 3339).
    pub at: String,
    /// Who: the books' user, which for agents is the company's service login or the human
    /// whose token the call carried.
    pub actor: Option<String>,
    /// What was done, for example `posted`.
    pub action: String,
    /// The books' own detail.
    pub details: serde_json::Value,
}

/// Amounts owed, by how far past due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeingBuckets {
    /// Not yet due.
    pub current: Money,
    /// 1 to 30 days past due.
    pub days_1_30: Money,
    /// 31 to 60 days past due.
    pub days_31_60: Money,
    /// 61 to 90 days past due.
    pub days_61_90: Money,
    /// More than 90 days past due.
    pub over_90: Money,
}

impl AgeingBuckets {
    /// Nothing owed, in `currency`.
    pub fn zero(currency: Currency) -> Self {
        let z = Money::zero(currency);
        Self {
            current: z,
            days_1_30: z,
            days_31_60: z,
            days_61_90: z,
            over_90: z,
        }
    }

    /// Adds `amount`, due on `due`, as seen on `as_of`.
    pub fn add(
        &mut self,
        amount: Money,
        due: NaiveDate,
        as_of: NaiveDate,
    ) -> Result<(), MoneyError> {
        let late = (as_of - due).num_days();
        let bucket = match late {
            i64::MIN..=0 => &mut self.current,
            1..=30 => &mut self.days_1_30,
            31..=60 => &mut self.days_31_60,
            61..=90 => &mut self.days_61_90,
            _ => &mut self.over_90,
        };
        *bucket = bucket.checked_add(amount)?;
        Ok(())
    }

    /// The total across buckets.
    pub fn total(&self) -> Result<Money, MoneyError> {
        self.current
            .checked_add(self.days_1_30)?
            .checked_add(self.days_31_60)?
            .checked_add(self.days_61_90)?
            .checked_add(self.over_90)
    }
}

/// One counterparty's line in an ageing report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeingRow {
    /// The customer or vendor.
    pub party: DocumentRef,
    /// Their name.
    pub name: String,
    /// What they owe (or we owe them), by age.
    pub buckets: AgeingBuckets,
}

/// Receivables or payables by age, as of one date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeingReport {
    /// The date ages are counted to.
    pub as_of: NaiveDate,
    /// One row per counterparty with something open.
    pub rows: Vec<AgeingRow>,
    /// The column totals.
    pub totals: AgeingBuckets,
}

/// A customer's credit: limit, what they owe, and what is left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditPosition {
    /// The customer.
    pub customer: CustomerId,
    /// The limit; `None` is no limit set.
    pub limit: Option<Money>,
    /// What they owe on posted invoices now.
    pub exposure: Money,
    /// `limit − exposure`, when there is a limit; negative when over.
    pub available: Option<Money>,
}

impl CreditPosition {
    /// Whether a further `amount` of credit fits under the limit (always, with no limit).
    pub fn allows(&self, amount: Money) -> bool {
        match self.available {
            None => true,
            Some(a) => a.compare(&amount).map(|o| o.is_ge()).unwrap_or(false),
        }
    }
}

/// A sales lead to record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeadDraft {
    /// The person.
    pub contact_name: String,
    /// Their company, if any.
    pub company_name: Option<String>,
    /// Their email.
    pub email: Option<String>,
    /// Where they came from, for example `sales-desk`.
    pub source: Option<String>,
    /// What they want, in plain words.
    pub notes: Option<String>,
}

/// Where a lead is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadStatus {
    /// Not yet worked.
    New,
    /// Being worked.
    Qualified,
    /// Turned into an opportunity.
    Converted,
    /// Dropped.
    Lost,
}

/// A sales lead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lead {
    /// The books' id.
    pub id: LeadId,
    /// The person.
    pub contact_name: String,
    /// Their company.
    pub company_name: Option<String>,
    /// Their email.
    pub email: Option<String>,
    /// Where it is.
    pub status: LeadStatus,
}

/// An opportunity to open from a lead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunityDraft {
    /// What is being sold, for example "WebAgent Pro, 12 months".
    pub name: String,
    /// Its expected value.
    pub amount: Money,
    /// When it is expected to close.
    pub expected_close: Option<NaiveDate>,
    /// The customer, when they are already in the books (ERA links the customer when the
    /// opportunity is opened, not when it is won).
    pub customer: Option<CustomerId>,
}

/// Where an opportunity is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityStage {
    /// Still being worked.
    Open,
    /// Won: there is a customer.
    Won,
    /// Lost.
    Lost,
}

/// A sales opportunity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opportunity {
    /// The books' id.
    pub id: OpportunityId,
    /// What is being sold.
    pub name: String,
    /// The lead it came from.
    pub lead: Option<LeadId>,
    /// The customer, once won.
    pub customer: Option<CustomerId>,
    /// Its value.
    pub amount: Money,
    /// Where it is.
    pub stage: OpportunityStage,
}

/// How urgent a ticket is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketPriority {
    /// Whenever.
    Low,
    /// The usual.
    Normal,
    /// Soon.
    High,
    /// Now.
    Urgent,
}

/// Where a ticket is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    /// Waiting on us.
    Open,
    /// Waiting on the customer.
    Pending,
    /// Answered.
    Resolved,
    /// Done.
    Closed,
}

/// A support ticket to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketDraft {
    /// The customer, when known.
    pub customer: Option<CustomerId>,
    /// The subject line.
    pub subject: String,
    /// What they asked, in their words.
    pub description: String,
    /// How urgent.
    pub priority: TicketPriority,
}

/// A support ticket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    /// The books' id.
    pub id: TicketId,
    /// The human number.
    pub number: Option<String>,
    /// The customer.
    pub customer: Option<CustomerId>,
    /// The subject line.
    pub subject: String,
    /// Where it is.
    pub status: TicketStatus,
    /// How urgent.
    pub priority: TicketPriority,
}

/// One company's books.
///
/// Writes that carry a `reference` are idempotent by it where noted: calling them again with
/// the same reference returns the first record and posts nothing.
#[async_trait::async_trait]
pub trait Books: Send + Sync {
    /// The company these books belong to.
    fn company(&self) -> &CompanyId;

    // ── Customers and vendors ───────────────────────────────────────────────────────────

    /// The customer named `who.name`, created if there is none. Sets the credit limit when given.
    async fn ensure_customer(
        &self,
        who: &Counterparty,
        credit_limit: Option<Money>,
    ) -> BooksResult<Customer>;
    /// One customer.
    async fn customer(&self, id: &CustomerId) -> BooksResult<Customer>;
    /// Every customer.
    async fn customers(&self) -> BooksResult<Vec<Customer>>;
    /// The vendor named `who.name`, created if there is none.
    async fn ensure_vendor(&self, who: &Counterparty) -> BooksResult<Vendor>;
    /// One vendor.
    async fn vendor(&self, id: &VendorId) -> BooksResult<Vendor>;
    /// Every vendor.
    async fn vendors(&self) -> BooksResult<Vec<Vendor>>;

    // ── Price list ──────────────────────────────────────────────────────────────────────

    /// The product named `draft.name`, created if there is none.
    async fn ensure_product(&self, draft: &ProductDraft) -> BooksResult<Product>;
    /// Every product.
    async fn products(&self) -> BooksResult<Vec<Product>>;

    // ── Receivables ─────────────────────────────────────────────────────────────────────

    /// Drafts an invoice (not yet in the ledger).
    async fn draft_invoice(&self, draft: &InvoiceDraft) -> BooksResult<Invoice>;
    /// Sends a draft for approval, where the books have an approval step.
    async fn submit_invoice(&self, id: &InvoiceId) -> BooksResult<Invoice>;
    /// Posts an invoice to the ledger: Dr receivables, Cr revenue.
    async fn post_invoice(&self, id: &InvoiceId) -> BooksResult<Invoice>;
    /// One invoice.
    async fn invoice(&self, id: &InvoiceId) -> BooksResult<Invoice>;
    /// Invoices, filtered.
    async fn invoices(&self, filter: &DocumentFilter) -> BooksResult<Vec<Invoice>>;

    /// Sets up a recurring invoice. Idempotent by `draft.reference` when given.
    async fn create_recurring_invoice(
        &self,
        draft: &RecurringInvoiceDraft,
    ) -> BooksResult<RecurringInvoice>;
    /// Every recurring-invoice schedule.
    async fn recurring_invoices(&self) -> BooksResult<Vec<RecurringInvoice>>;
    /// Stops a schedule (a cancelled subscription); invoices already raised stay.
    async fn stop_recurring_invoice(
        &self,
        id: &RecurringInvoiceId,
    ) -> BooksResult<RecurringInvoice>;
    /// Raises and posts every recurring invoice due on or before `as_of`, and returns them.
    /// The books' own scheduler does this daily; an agent woken on a due date calls it to
    /// have the invoice now.
    async fn raise_due_recurring(&self, as_of: NaiveDate) -> BooksResult<Vec<Invoice>>;

    // ── Payables ────────────────────────────────────────────────────────────────────────

    /// Drafts a vendor bill.
    async fn draft_bill(&self, draft: &BillDraft) -> BooksResult<Bill>;
    /// Posts a bill to the ledger: Dr expense, Cr payables.
    async fn post_bill(&self, id: &BillId) -> BooksResult<Bill>;
    /// One bill.
    async fn bill(&self, id: &BillId) -> BooksResult<Bill>;
    /// Bills, filtered.
    async fn bills(&self, filter: &DocumentFilter) -> BooksResult<Vec<Bill>>;

    // ── Money through PayPal clearing ───────────────────────────────────────────────────

    /// Records money received into PayPal clearing and applies it to invoices: Dr clearing,
    /// Cr receivables. Idempotent by `draft.reference`.
    async fn record_receipt(&self, draft: &ReceiptDraft) -> BooksResult<Payment>;
    /// Applies more of an existing receipt's unapplied amount to invoices.
    async fn apply_receipt(
        &self,
        id: &PaymentId,
        applications: &[InvoiceApplication],
    ) -> BooksResult<Payment>;
    /// Records money paid to a vendor out of PayPal clearing and applies it to bills: Dr
    /// payables, Cr clearing. Idempotent by `draft.reference`.
    async fn record_bill_payment(&self, draft: &BillPaymentDraft) -> BooksResult<Payment>;
    /// Records a refund: a credit note against the customer, and the money back out of
    /// PayPal clearing. Idempotent by `draft.reference`.
    async fn record_refund(&self, draft: &RefundDraft) -> BooksResult<Refund>;
    /// One payment.
    async fn payment(&self, id: &PaymentId) -> BooksResult<Payment>;
    /// Payments, filtered.
    async fn payments(&self, filter: &PaymentFilter) -> BooksResult<Vec<Payment>>;
    /// The refund recorded under a reference key, if any.
    async fn refund_by_reference(&self, reference: &str) -> BooksResult<Option<Refund>>;

    // ── Ledger ──────────────────────────────────────────────────────────────────────────

    /// The account the company uses for a role.
    async fn account(&self, role: AccountRole) -> BooksResult<Account>;
    /// Posts a manual journal entry. Idempotent by `draft.reference` when given.
    async fn post_journal(&self, draft: &JournalDraft) -> BooksResult<JournalEntry>;
    /// Journal entries, filtered.
    async fn journal(&self, filter: &JournalFilter) -> BooksResult<Vec<JournalEntry>>;
    /// One journal entry.
    async fn journal_entry(&self, id: &JournalId) -> BooksResult<JournalEntry>;
    /// The audit trail of one document.
    async fn audit_trail(&self, doc: &DocumentRef) -> BooksResult<Vec<AuditEntry>>;

    // ── Debt ────────────────────────────────────────────────────────────────────────────

    /// Receivables by age.
    async fn ar_ageing(&self, as_of: NaiveDate) -> BooksResult<AgeingReport>;
    /// Payables by age.
    async fn ap_ageing(&self, as_of: NaiveDate) -> BooksResult<AgeingReport>;
    /// Sets or clears a customer's credit limit.
    async fn set_credit_limit(
        &self,
        id: &CustomerId,
        limit: Option<Money>,
    ) -> BooksResult<Customer>;
    /// A customer's limit, exposure and headroom.
    async fn credit_position(&self, id: &CustomerId) -> BooksResult<CreditPosition>;

    // ── CRM ─────────────────────────────────────────────────────────────────────────────

    /// Records a lead.
    async fn create_lead(&self, draft: &LeadDraft) -> BooksResult<Lead>;
    /// Every lead.
    async fn leads(&self) -> BooksResult<Vec<Lead>>;
    /// Turns a lead into an opportunity.
    async fn convert_lead(&self, id: &LeadId, draft: &OpportunityDraft)
    -> BooksResult<Opportunity>;
    /// Marks an opportunity won.
    async fn win_opportunity(&self, id: &OpportunityId) -> BooksResult<Opportunity>;
    /// Marks an opportunity lost.
    async fn lose_opportunity(&self, id: &OpportunityId, reason: &str) -> BooksResult<Opportunity>;
    /// Every opportunity.
    async fn opportunities(&self) -> BooksResult<Vec<Opportunity>>;
    /// Opens a support ticket.
    async fn open_ticket(&self, draft: &TicketDraft) -> BooksResult<Ticket>;
    /// Moves a ticket on, with a note.
    async fn update_ticket(
        &self,
        id: &TicketId,
        status: TicketStatus,
        note: Option<&str>,
    ) -> BooksResult<Ticket>;
    /// Tickets, optionally only those in one status.
    async fn tickets(&self, status: Option<TicketStatus>) -> BooksResult<Vec<Ticket>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn ageing_buckets_by_days_late() {
        let mut b = AgeingBuckets::zero(Currency::USD);
        let as_of = d("2026-11-10");
        b.add(Money::usd(100), d("2026-11-10"), as_of).unwrap();
        b.add(Money::usd(200), d("2026-11-09"), as_of).unwrap();
        b.add(Money::usd(300), d("2026-10-10"), as_of).unwrap(); // 31 days
        b.add(Money::usd(400), d("2026-08-01"), as_of).unwrap(); // 101 days
        assert_eq!(b.current, Money::usd(100));
        assert_eq!(b.days_1_30, Money::usd(200));
        assert_eq!(b.days_31_60, Money::usd(300));
        assert_eq!(b.over_90, Money::usd(400));
        assert_eq!(b.total().unwrap(), Money::usd(1000));
    }

    #[test]
    fn journals_must_balance() {
        let line = |acct: &str, dr: i64, cr: i64| JournalLine {
            account: acct.into(),
            debit: Money::usd(dr),
            credit: Money::usd(cr),
            memo: None,
        };
        let mut j = JournalDraft {
            date: d("2026-11-10"),
            memo: "fee".into(),
            reference: None,
            lines: vec![line("6100", 59, 0), line("1010", 0, 59)],
        };
        assert_eq!(j.check().unwrap(), Money::usd(59));
        j.lines[1].credit = Money::usd(58);
        assert!(j.check().is_err());
        j.lines[1] = line("1010", 59, 59);
        assert!(j.check().is_err());
        j.lines.truncate(1);
        assert!(j.check().is_err());
    }

    #[test]
    fn credit_headroom() {
        let p = CreditPosition {
            customer: "c".into(),
            limit: Some(Money::usd(1000)),
            exposure: Money::usd(700),
            available: Some(Money::usd(300)),
        };
        assert!(p.allows(Money::usd(300)));
        assert!(!p.allows(Money::usd(301)));
        assert!(
            CreditPosition {
                limit: None,
                available: None,
                ..p
            }
            .allows(Money::usd(1_000_000))
        );
    }
}
