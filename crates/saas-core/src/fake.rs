//! [`FakeBooks`]: the [`Books`] contract in memory, for tests. Double entry is real here: every
//! posted document writes its journal, so account balances and ageing come out of the same
//! entries a test can read.
//!
//! Its chart: `1010` PayPal clearing, `1100` receivables, `2000` payables, `4000` plan
//! revenue, `5000` operating expenses (every bill line), `6150` PayPal fees.

use std::sync::{Mutex, MutexGuard};

use chrono::{Months, NaiveDate};

use crate::books::*;
use crate::ids::CompanyId;
use crate::money::{Currency, Money};

const CLEARING: &str = "1010";
const RECEIVABLES: &str = "1100";
const PAYABLES: &str = "2000";
const REVENUE: &str = "4000";
const EXPENSES: &str = "5000";
const FEES: &str = "6150";

/// [`Books`] in memory.
pub struct FakeBooks {
    company: CompanyId,
    currency: Currency,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    seq: u64,
    fail_next: Option<BooksError>,
    customers: Vec<Customer>,
    vendors: Vec<Vendor>,
    products: Vec<Product>,
    invoices: Vec<Invoice>,
    recurring: Vec<(RecurringInvoice, RecurringInvoiceDraft)>,
    bills: Vec<Bill>,
    payments: Vec<Payment>,
    refunds: Vec<Refund>,
    journals: Vec<JournalEntry>,
    leads: Vec<Lead>,
    opportunities: Vec<Opportunity>,
    tickets: Vec<Ticket>,
    audit: Vec<(DocumentRef, AuditEntry)>,
}

impl FakeBooks {
    /// Empty books for `company`, in US dollars.
    pub fn new(company: CompanyId) -> Self {
        Self {
            company,
            currency: Currency::USD,
            state: Mutex::new(State::default()),
        }
    }

    /// The next call (any method) fails with `err`, once. For testing crash and retry paths.
    pub fn fail_next(&self, err: BooksError) {
        self.lock().fail_next = Some(err);
    }

    /// An account's balance as debits minus credits.
    pub fn balance(&self, code: &str) -> Money {
        let st = self.lock();
        balance_of(&st, code, self.currency)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Locks the state, failing first if a failure was queued.
    fn enter(&self) -> BooksResult<MutexGuard<'_, State>> {
        let mut st = self.lock();
        match st.fail_next.take() {
            Some(err) => Err(err),
            None => Ok(st),
        }
    }
}

fn balance_of(st: &State, code: &str, currency: Currency) -> Money {
    let minor = st
        .journals
        .iter()
        .flat_map(|j| &j.lines)
        .filter(|l| l.account.as_str() == code)
        .map(|l| l.debit.minor - l.credit.minor)
        .sum();
    Money::new(minor, currency)
}

fn not_found(what: &'static str, id: impl ToString) -> BooksError {
    BooksError::NotFound {
        what,
        id: id.to_string(),
    }
}

impl State {
    fn next(&mut self, prefix: &str) -> String {
        self.seq += 1;
        format!("{prefix}-{:04}", self.seq)
    }

    fn audit(&mut self, doc: DocumentRef, action: &str, details: serde_json::Value) {
        self.seq += 1;
        let at = format!("seq:{:06}", self.seq);
        self.audit.push((
            doc,
            AuditEntry {
                at,
                actor: Some("fake".into()),
                action: action.into(),
                details,
            },
        ));
    }

    fn journal(
        &mut self,
        date: NaiveDate,
        memo: String,
        reference: Option<String>,
        lines: Vec<(&str, Money, bool)>,
    ) -> JournalId {
        let id = JournalId(self.next("je"));
        let lines = lines
            .into_iter()
            .map(|(acct, amount, debit)| {
                let zero = Money::zero(amount.currency);
                JournalLine {
                    account: acct.into(),
                    debit: if debit { amount } else { zero },
                    credit: if debit { zero } else { amount },
                    memo: None,
                }
            })
            .collect();
        self.journals.push(JournalEntry {
            id: id.clone(),
            number: Some(id.0.to_uppercase()),
            date,
            memo,
            reference,
            lines,
        });
        self.audit(
            DocumentRef::Journal(id.clone()),
            "posted",
            serde_json::Value::Null,
        );
        id
    }

    fn customer_mut(&mut self, id: &CustomerId) -> BooksResult<&mut Customer> {
        self.customers
            .iter_mut()
            .find(|c| &c.id == id)
            .ok_or_else(|| not_found("customer", id))
    }

    fn customer_view(&self, c: &Customer) -> Customer {
        let owed = c.balance.currency;
        let open: i64 = self
            .invoices
            .iter()
            .filter(|i| i.customer == c.id && i.status.is_open())
            .map(|i| i.balance_due.minor)
            .sum();
        let unapplied: i64 = self
            .payments
            .iter()
            .filter(|p| {
                p.direction == PaymentDirection::Received
                    && p.party == DocumentRef::Customer(c.id.clone())
            })
            .map(|p| p.unapplied.minor)
            .sum();
        Customer {
            balance: Money::new(open - unapplied, owed),
            ..c.clone()
        }
    }

    fn vendor_view(&self, v: &Vendor) -> Vendor {
        let open: i64 = self
            .bills
            .iter()
            .filter(|b| b.vendor == v.id && b.status.is_open())
            .map(|b| b.balance_due.minor)
            .sum();
        Vendor {
            balance: Money::new(open, v.balance.currency),
            ..v.clone()
        }
    }

    fn invoice_mut(&mut self, id: &InvoiceId) -> BooksResult<&mut Invoice> {
        self.invoices
            .iter_mut()
            .find(|i| &i.id == id)
            .ok_or_else(|| not_found("invoice", id))
    }

    fn bill_mut(&mut self, id: &BillId) -> BooksResult<&mut Bill> {
        self.bills
            .iter_mut()
            .find(|b| &b.id == id)
            .ok_or_else(|| not_found("bill", id))
    }

    /// Applies receipt amounts to invoices; checks everything before changing anything.
    fn apply_to_invoices(
        &mut self,
        customer: &CustomerId,
        apps: &[InvoiceApplication],
        available: Money,
    ) -> BooksResult<Vec<(DocumentRef, Money)>> {
        let mut total = Money::zero(available.currency);
        for a in apps {
            let inv = self
                .invoices
                .iter()
                .find(|i| i.id == a.invoice)
                .ok_or_else(|| not_found("invoice", &a.invoice))?;
            if &inv.customer != customer {
                return Err(BooksError::Invalid(format!(
                    "invoice {} is not this customer's",
                    inv.id
                )));
            }
            if !inv.status.is_open() {
                return Err(BooksError::Conflict(format!(
                    "invoice {} is not open",
                    inv.id
                )));
            }
            if !a.amount.is_positive() || a.amount.compare(&inv.balance_due)?.is_gt() {
                return Err(BooksError::Conflict(format!(
                    "{} does not fit invoice {} (owes {})",
                    a.amount, inv.id, inv.balance_due
                )));
            }
            total = total.checked_add(a.amount)?;
        }
        if total.compare(&available)?.is_gt() {
            return Err(BooksError::Conflict(format!(
                "applications {total} exceed the {available} available"
            )));
        }
        let mut applied = Vec::new();
        for a in apps {
            let inv = self.invoice_mut(&a.invoice)?;
            inv.balance_due = inv.balance_due.checked_sub(a.amount)?;
            inv.status = if inv.balance_due.is_zero() {
                DocumentStatus::Paid
            } else {
                DocumentStatus::PartiallyPaid
            };
            applied.push((DocumentRef::Invoice(a.invoice.clone()), a.amount));
        }
        Ok(applied)
    }

    fn raise_invoice(&mut self, draft: &InvoiceDraft) -> BooksResult<Invoice> {
        if !self.customers.iter().any(|c| c.id == draft.customer) {
            return Err(not_found("customer", &draft.customer));
        }
        let (lines, total) = priced(&draft.lines, draft.currency)?;
        let id = InvoiceId(self.next("inv"));
        let invoice = Invoice {
            id: id.clone(),
            number: None,
            customer: draft.customer.clone(),
            status: DocumentStatus::Draft,
            issue_date: draft.issue_date,
            due_date: draft.due_date,
            lines,
            total,
            balance_due: total,
            reference: draft.reference.clone(),
        };
        self.invoices.push(invoice.clone());
        self.audit(DocumentRef::Invoice(id), "drafted", serde_json::Value::Null);
        Ok(invoice)
    }

    fn post_invoice(&mut self, id: &InvoiceId) -> BooksResult<Invoice> {
        let inv = self.invoice_mut(id)?;
        if !matches!(
            inv.status,
            DocumentStatus::Draft | DocumentStatus::Submitted
        ) {
            return Err(BooksError::Conflict(format!(
                "invoice {id} is already {:?}",
                inv.status
            )));
        }
        inv.status = DocumentStatus::Posted;
        let (date, total) = (inv.issue_date, inv.total);
        let n = self.invoices.iter().filter(|i| i.number.is_some()).count() + 1;
        let inv = self.invoice_mut(id)?;
        inv.number = Some(format!("INV-{n:04}"));
        let out = inv.clone();
        self.journal(
            date,
            format!("Invoice {}", out.number.clone().unwrap_or_default()),
            Some(format!("invoice:{id}")),
            vec![(RECEIVABLES, total, true), (REVENUE, total, false)],
        );
        self.audit(
            DocumentRef::Invoice(id.clone()),
            "posted",
            serde_json::Value::Null,
        );
        Ok(out)
    }
}

/// Prices draft lines; every line in `currency`, at least one line, quantities above zero.
fn priced(lines: &[LineDraft], currency: Currency) -> BooksResult<(Vec<Line>, Money)> {
    if lines.is_empty() {
        return Err(BooksError::Invalid(
            "a document needs at least one line".into(),
        ));
    }
    let mut total = Money::zero(currency);
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        if l.unit_price.currency != currency {
            return Err(BooksError::Invalid(format!(
                "line {:?} is in {}, not {currency}",
                l.description, l.unit_price.currency
            )));
        }
        if l.quantity == 0 || l.unit_price.minor < 0 {
            return Err(BooksError::Invalid(format!(
                "line {:?} needs a quantity and a price",
                l.description
            )));
        }
        let amount = l.unit_price.checked_mul(i64::from(l.quantity))?;
        total = total.checked_add(amount)?;
        out.push(Line {
            product: l.product.clone(),
            description: l.description.clone(),
            quantity: l.quantity,
            unit_price: l.unit_price,
            amount,
        });
    }
    Ok((out, total))
}

fn step(date: NaiveDate, f: Frequency) -> Option<NaiveDate> {
    match f {
        Frequency::Weekly => date.checked_add_days(chrono::Days::new(7)),
        Frequency::Monthly => date.checked_add_months(Months::new(1)),
        Frequency::Quarterly => date.checked_add_months(Months::new(3)),
        Frequency::Yearly => date.checked_add_months(Months::new(12)),
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[async_trait::async_trait]
impl Books for FakeBooks {
    fn company(&self) -> &CompanyId {
        &self.company
    }

    async fn ensure_customer(
        &self,
        who: &Counterparty,
        credit_limit: Option<Money>,
    ) -> BooksResult<Customer> {
        let mut st = self.enter()?;
        if let Some(c) = st
            .customers
            .iter_mut()
            .find(|c| same_name(&c.counterparty.name, &who.name))
        {
            if credit_limit.is_some() {
                c.credit_limit = credit_limit;
            }
            let c = c.clone();
            return Ok(st.customer_view(&c));
        }
        let id = CustomerId(st.next("cus"));
        let c = Customer {
            id: id.clone(),
            counterparty: who.clone(),
            credit_limit,
            balance: Money::zero(who.currency),
        };
        st.customers.push(c.clone());
        st.audit(
            DocumentRef::Customer(id),
            "created",
            serde_json::Value::Null,
        );
        Ok(c)
    }

    async fn customer(&self, id: &CustomerId) -> BooksResult<Customer> {
        let st = self.enter()?;
        let c = st
            .customers
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| not_found("customer", id))?;
        Ok(st.customer_view(c))
    }

    async fn customers(&self) -> BooksResult<Vec<Customer>> {
        let st = self.enter()?;
        Ok(st.customers.iter().map(|c| st.customer_view(c)).collect())
    }

    async fn ensure_vendor(&self, who: &Counterparty) -> BooksResult<Vendor> {
        let mut st = self.enter()?;
        if let Some(v) = st
            .vendors
            .iter()
            .find(|v| same_name(&v.counterparty.name, &who.name))
        {
            return Ok(st.vendor_view(v));
        }
        let id = VendorId(st.next("ven"));
        let v = Vendor {
            id: id.clone(),
            counterparty: who.clone(),
            balance: Money::zero(who.currency),
        };
        st.vendors.push(v.clone());
        st.audit(DocumentRef::Vendor(id), "created", serde_json::Value::Null);
        Ok(v)
    }

    async fn vendor(&self, id: &VendorId) -> BooksResult<Vendor> {
        let st = self.enter()?;
        let v = st
            .vendors
            .iter()
            .find(|v| &v.id == id)
            .ok_or_else(|| not_found("vendor", id))?;
        Ok(st.vendor_view(v))
    }

    async fn vendors(&self) -> BooksResult<Vec<Vendor>> {
        let st = self.enter()?;
        Ok(st.vendors.iter().map(|v| st.vendor_view(v)).collect())
    }

    async fn ensure_product(&self, draft: &ProductDraft) -> BooksResult<Product> {
        let mut st = self.enter()?;
        if let Some(p) = st.products.iter().find(|p| same_name(&p.name, &draft.name)) {
            return Ok(p.clone());
        }
        let p = Product {
            id: ProductId(st.next("prd")),
            name: draft.name.clone(),
            sku: draft.sku.clone(),
            price: draft.price,
        };
        st.products.push(p.clone());
        Ok(p)
    }

    async fn products(&self) -> BooksResult<Vec<Product>> {
        Ok(self.enter()?.products.clone())
    }

    async fn draft_invoice(&self, draft: &InvoiceDraft) -> BooksResult<Invoice> {
        self.enter()?.raise_invoice(draft)
    }

    async fn submit_invoice(&self, id: &InvoiceId) -> BooksResult<Invoice> {
        let mut st = self.enter()?;
        let inv = st.invoice_mut(id)?;
        if inv.status != DocumentStatus::Draft {
            return Err(BooksError::Conflict(format!("invoice {id} is not a draft")));
        }
        inv.status = DocumentStatus::Submitted;
        let out = inv.clone();
        st.audit(
            DocumentRef::Invoice(id.clone()),
            "submitted",
            serde_json::Value::Null,
        );
        Ok(out)
    }

    async fn post_invoice(&self, id: &InvoiceId) -> BooksResult<Invoice> {
        self.enter()?.post_invoice(id)
    }

    async fn invoice(&self, id: &InvoiceId) -> BooksResult<Invoice> {
        let st = self.enter()?;
        st.invoices
            .iter()
            .find(|i| &i.id == id)
            .cloned()
            .ok_or_else(|| not_found("invoice", id))
    }

    async fn invoices(&self, filter: &DocumentFilter) -> BooksResult<Vec<Invoice>> {
        let st = self.enter()?;
        Ok(st
            .invoices
            .iter()
            .filter(|i| {
                filter
                    .party
                    .as_deref()
                    .is_none_or(|p| i.customer.as_str() == p)
            })
            .filter(|i| !filter.open_only || i.status.is_open())
            .filter(|i| filter.reference.is_none() || i.reference == filter.reference)
            .cloned()
            .collect())
    }

    async fn create_recurring_invoice(
        &self,
        draft: &RecurringInvoiceDraft,
    ) -> BooksResult<RecurringInvoice> {
        let mut st = self.enter()?;
        if draft.reference.is_some() {
            if let Some((r, _)) = st
                .recurring
                .iter()
                .find(|(r, _)| r.reference == draft.reference)
            {
                return Ok(r.clone());
            }
        }
        if !st.customers.iter().any(|c| c.id == draft.customer) {
            return Err(not_found("customer", &draft.customer));
        }
        let (_, amount) = priced(&draft.lines, draft.currency)?;
        let r = RecurringInvoice {
            id: RecurringInvoiceId(st.next("rec")),
            customer: draft.customer.clone(),
            frequency: draft.frequency,
            amount,
            next_date: Some(draft.start_date),
            end_date: draft.end_date,
            active: true,
            reference: draft.reference.clone(),
        };
        st.recurring.push((r.clone(), draft.clone()));
        Ok(r)
    }

    async fn recurring_invoices(&self) -> BooksResult<Vec<RecurringInvoice>> {
        Ok(self
            .enter()?
            .recurring
            .iter()
            .map(|(r, _)| r.clone())
            .collect())
    }

    async fn stop_recurring_invoice(
        &self,
        id: &RecurringInvoiceId,
    ) -> BooksResult<RecurringInvoice> {
        let mut st = self.enter()?;
        let (r, _) = st
            .recurring
            .iter_mut()
            .find(|(r, _)| &r.id == id)
            .ok_or_else(|| not_found("recurring invoice", id))?;
        r.active = false;
        r.next_date = None;
        Ok(r.clone())
    }

    async fn raise_due_recurring(&self, as_of: NaiveDate) -> BooksResult<Vec<Invoice>> {
        let mut st = self.enter()?;
        let mut raised = Vec::new();
        for i in 0..st.recurring.len() {
            loop {
                let (r, draft) = &st.recurring[i];
                let Some(date) = r.next_date.filter(|d| r.active && *d <= as_of) else {
                    break;
                };
                if r.end_date.is_some_and(|end| date > end) {
                    st.recurring[i].0.next_date = None;
                    break;
                }
                let inv = InvoiceDraft {
                    customer: draft.customer.clone(),
                    issue_date: date,
                    due_date: date + chrono::Days::new(u64::from(draft.due_in_days)),
                    currency: draft.currency,
                    lines: draft.lines.clone(),
                    reference: draft.reference.clone(),
                    memo: None,
                };
                let frequency = r.frequency;
                let inv = st.raise_invoice(&inv)?;
                raised.push(st.post_invoice(&inv.id)?);
                st.recurring[i].0.next_date = step(date, frequency);
            }
        }
        Ok(raised)
    }

    async fn draft_bill(&self, draft: &BillDraft) -> BooksResult<Bill> {
        let mut st = self.enter()?;
        if !st.vendors.iter().any(|v| v.id == draft.vendor) {
            return Err(not_found("vendor", &draft.vendor));
        }
        let (_, total) = priced(&draft.lines, draft.currency)?;
        let id = BillId(st.next("bill"));
        let bill = Bill {
            id: id.clone(),
            number: None,
            vendor: draft.vendor.clone(),
            vendor_reference: draft.vendor_reference.clone(),
            status: DocumentStatus::Draft,
            issue_date: draft.issue_date,
            due_date: draft.due_date,
            total,
            balance_due: total,
        };
        st.bills.push(bill.clone());
        st.audit(DocumentRef::Bill(id), "drafted", serde_json::Value::Null);
        Ok(bill)
    }

    async fn post_bill(&self, id: &BillId) -> BooksResult<Bill> {
        let mut st = self.enter()?;
        let n = st.bills.iter().filter(|b| b.number.is_some()).count() + 1;
        let bill = st.bill_mut(id)?;
        if !matches!(
            bill.status,
            DocumentStatus::Draft | DocumentStatus::Submitted
        ) {
            return Err(BooksError::Conflict(format!(
                "bill {id} is already {:?}",
                bill.status
            )));
        }
        bill.status = DocumentStatus::Posted;
        bill.number = Some(format!("BILL-{n:04}"));
        let out = bill.clone();
        st.journal(
            out.issue_date,
            format!("Bill {}", out.number.clone().unwrap_or_default()),
            Some(format!("bill:{id}")),
            vec![(EXPENSES, out.total, true), (PAYABLES, out.total, false)],
        );
        st.audit(
            DocumentRef::Bill(id.clone()),
            "posted",
            serde_json::Value::Null,
        );
        Ok(out)
    }

    async fn bill(&self, id: &BillId) -> BooksResult<Bill> {
        let st = self.enter()?;
        st.bills
            .iter()
            .find(|b| &b.id == id)
            .cloned()
            .ok_or_else(|| not_found("bill", id))
    }

    async fn bills(&self, filter: &DocumentFilter) -> BooksResult<Vec<Bill>> {
        let st = self.enter()?;
        Ok(st
            .bills
            .iter()
            .filter(|b| {
                filter
                    .party
                    .as_deref()
                    .is_none_or(|p| b.vendor.as_str() == p)
            })
            .filter(|b| !filter.open_only || b.status.is_open())
            .filter(|b| filter.reference.is_none() || b.vendor_reference == filter.reference)
            .cloned()
            .collect())
    }

    async fn record_receipt(&self, draft: &ReceiptDraft) -> BooksResult<Payment> {
        let mut st = self.enter()?;
        if let Some(p) = st
            .payments
            .iter()
            .find(|p| p.reference.as_deref() == Some(&draft.reference))
        {
            return Ok(p.clone());
        }
        if !draft.amount.is_positive() {
            return Err(BooksError::Invalid(
                "a receipt needs an amount above zero".into(),
            ));
        }
        st.customer_mut(&draft.customer)?;
        let applied = st.apply_to_invoices(&draft.customer, &draft.applications, draft.amount)?;
        let used = applied
            .iter()
            .try_fold(Money::zero(draft.amount.currency), |a, (_, m)| {
                a.checked_add(*m)
            })?;
        let id = PaymentId(st.next("pay"));
        let p = Payment {
            id: id.clone(),
            number: Some(id.0.to_uppercase()),
            direction: PaymentDirection::Received,
            party: DocumentRef::Customer(draft.customer.clone()),
            date: draft.date,
            amount: draft.amount,
            reference: Some(draft.reference.clone()),
            applied,
            unapplied: draft.amount.checked_sub(used)?,
            journal: None,
        };
        let journal = st.journal(
            draft.date,
            format!("Receipt {}", draft.reference),
            Some(draft.reference.clone()),
            vec![
                (CLEARING, draft.amount, true),
                (RECEIVABLES, draft.amount, false),
            ],
        );
        let mut p = p;
        p.journal = Some(journal);
        st.payments.push(p.clone());
        st.audit(
            DocumentRef::Payment(id),
            "recorded",
            serde_json::json!({"reference": draft.reference}),
        );
        Ok(p)
    }

    async fn apply_receipt(
        &self,
        id: &PaymentId,
        applications: &[InvoiceApplication],
    ) -> BooksResult<Payment> {
        let mut st = self.enter()?;
        let p = st
            .payments
            .iter()
            .find(|p| &p.id == id)
            .cloned()
            .ok_or_else(|| not_found("payment", id))?;
        let DocumentRef::Customer(customer) = &p.party else {
            return Err(BooksError::Invalid(format!(
                "payment {id} is not a customer receipt"
            )));
        };
        let applied = st.apply_to_invoices(customer, applications, p.unapplied)?;
        let p = st
            .payments
            .iter_mut()
            .find(|p| &p.id == id)
            .ok_or_else(|| not_found("payment", id))?;
        for (doc, amount) in applied {
            p.unapplied = p.unapplied.checked_sub(amount)?;
            p.applied.push((doc, amount));
        }
        let out = p.clone();
        st.audit(
            DocumentRef::Payment(id.clone()),
            "applied",
            serde_json::Value::Null,
        );
        Ok(out)
    }

    async fn record_bill_payment(&self, draft: &BillPaymentDraft) -> BooksResult<Payment> {
        let mut st = self.enter()?;
        if let Some(p) = st
            .payments
            .iter()
            .find(|p| p.reference.as_deref() == Some(&draft.reference))
        {
            return Ok(p.clone());
        }
        if !draft.amount.is_positive() {
            return Err(BooksError::Invalid(
                "a payment needs an amount above zero".into(),
            ));
        }
        if !st.vendors.iter().any(|v| v.id == draft.vendor) {
            return Err(not_found("vendor", &draft.vendor));
        }
        let mut total = Money::zero(draft.amount.currency);
        for a in &draft.applications {
            let b = st
                .bills
                .iter()
                .find(|b| b.id == a.bill)
                .ok_or_else(|| not_found("bill", &a.bill))?;
            if b.vendor != draft.vendor
                || !b.status.is_open()
                || a.amount.compare(&b.balance_due)?.is_gt()
                || !a.amount.is_positive()
            {
                return Err(BooksError::Conflict(format!(
                    "{} does not fit bill {}",
                    a.amount, b.id
                )));
            }
            total = total.checked_add(a.amount)?;
        }
        if total.compare(&draft.amount)?.is_gt() {
            return Err(BooksError::Conflict(format!(
                "applications {total} exceed the payment {}",
                draft.amount
            )));
        }
        let mut applied = Vec::new();
        for a in &draft.applications {
            let b = st.bill_mut(&a.bill)?;
            b.balance_due = b.balance_due.checked_sub(a.amount)?;
            b.status = if b.balance_due.is_zero() {
                DocumentStatus::Paid
            } else {
                DocumentStatus::PartiallyPaid
            };
            applied.push((DocumentRef::Bill(a.bill.clone()), a.amount));
        }
        let id = PaymentId(st.next("pay"));
        let p = Payment {
            id: id.clone(),
            number: Some(id.0.to_uppercase()),
            direction: PaymentDirection::Paid,
            party: DocumentRef::Vendor(draft.vendor.clone()),
            date: draft.date,
            amount: draft.amount,
            reference: Some(draft.reference.clone()),
            applied,
            unapplied: draft.amount.checked_sub(total)?,
            journal: None,
        };
        let journal = st.journal(
            draft.date,
            format!("Payment {}", draft.reference),
            Some(draft.reference.clone()),
            vec![
                (PAYABLES, draft.amount, true),
                (CLEARING, draft.amount, false),
            ],
        );
        let mut p = p;
        p.journal = Some(journal);
        st.payments.push(p.clone());
        st.audit(
            DocumentRef::Payment(id),
            "recorded",
            serde_json::json!({"reference": draft.reference}),
        );
        Ok(p)
    }

    async fn record_refund(&self, draft: &RefundDraft) -> BooksResult<Refund> {
        let mut st = self.enter()?;
        if let Some(r) = st.refunds.iter().find(|r| r.reference == draft.reference) {
            return Ok(r.clone());
        }
        if !draft.amount.is_positive() {
            return Err(BooksError::Invalid(
                "a refund needs an amount above zero".into(),
            ));
        }
        st.customer_mut(&draft.customer)?;
        if let Some(inv) = &draft.invoice {
            let inv = st
                .invoices
                .iter()
                .find(|i| &i.id == inv)
                .ok_or_else(|| not_found("invoice", inv))?;
            if inv.amount_paid()?.compare(&draft.amount)?.is_lt() {
                return Err(BooksError::Conflict(format!(
                    "refund {} is more than was paid on {}",
                    draft.amount, inv.id
                )));
            }
        }
        let credit_note = CreditNoteId(st.next("cn"));
        let payment = PaymentId(st.next("pay"));
        st.journal(
            draft.date,
            format!("Credit note: {}", draft.reason),
            Some(draft.reference.clone()),
            vec![
                (REVENUE, draft.amount, true),
                (RECEIVABLES, draft.amount, false),
            ],
        );
        let refund_journal = st.journal(
            draft.date,
            format!("Refund {}", draft.reference),
            Some(draft.reference.clone()),
            vec![
                (RECEIVABLES, draft.amount, true),
                (CLEARING, draft.amount, false),
            ],
        );
        st.payments.push(Payment {
            id: payment.clone(),
            number: Some(payment.0.to_uppercase()),
            direction: PaymentDirection::Paid,
            party: DocumentRef::Customer(draft.customer.clone()),
            date: draft.date,
            amount: draft.amount,
            reference: Some(draft.reference.clone()),
            applied: vec![(DocumentRef::CreditNote(credit_note.clone()), draft.amount)],
            unapplied: Money::zero(draft.amount.currency),
            journal: Some(refund_journal),
        });
        let r = Refund {
            credit_note: Some(credit_note.clone()),
            payment: Some(payment),
            journal: None,
            amount: draft.amount,
            reference: draft.reference.clone(),
        };
        st.refunds.push(r.clone());
        st.audit(
            DocumentRef::CreditNote(credit_note),
            "refunded",
            serde_json::json!({"reason": draft.reason}),
        );
        Ok(r)
    }

    async fn payment(&self, id: &PaymentId) -> BooksResult<Payment> {
        let st = self.enter()?;
        st.payments
            .iter()
            .find(|p| &p.id == id)
            .cloned()
            .ok_or_else(|| not_found("payment", id))
    }

    async fn payments(&self, filter: &PaymentFilter) -> BooksResult<Vec<Payment>> {
        let st = self.enter()?;
        Ok(st
            .payments
            .iter()
            .filter(|p| filter.reference.is_none() || p.reference == filter.reference)
            .filter(|p| filter.direction.is_none_or(|d| p.direction == d))
            .cloned()
            .collect())
    }

    async fn refund_by_reference(&self, reference: &str) -> BooksResult<Option<Refund>> {
        Ok(self
            .enter()?
            .refunds
            .iter()
            .find(|r| r.reference == reference)
            .cloned())
    }

    async fn account(&self, role: AccountRole) -> BooksResult<Account> {
        let st = self.enter()?;
        let (code, name) = match role {
            AccountRole::PayPalClearing => (CLEARING, "PayPal clearing"),
            AccountRole::Receivables => (RECEIVABLES, "Accounts receivable"),
            AccountRole::Payables => (PAYABLES, "Accounts payable"),
            AccountRole::Revenue => (REVENUE, "Plan revenue"),
            AccountRole::PayPalFees => (FEES, "PayPal fees"),
        };
        Ok(Account {
            code: code.into(),
            name: name.into(),
            balance: Some(balance_of(&st, code, self.currency)),
        })
    }

    async fn post_journal(&self, draft: &JournalDraft) -> BooksResult<JournalEntry> {
        let mut st = self.enter()?;
        if let Some(r) = &draft.reference {
            if let Some(j) = st.journals.iter().find(|j| j.reference.as_ref() == Some(r)) {
                return Ok(j.clone());
            }
        }
        draft.check()?;
        let known = [CLEARING, RECEIVABLES, PAYABLES, REVENUE, EXPENSES, FEES];
        if let Some(l) = draft
            .lines
            .iter()
            .find(|l| !known.contains(&l.account.as_str()))
        {
            return Err(not_found("account", &l.account));
        }
        let id = JournalId(st.next("je"));
        let entry = JournalEntry {
            id: id.clone(),
            number: Some(id.0.to_uppercase()),
            date: draft.date,
            memo: draft.memo.clone(),
            reference: draft.reference.clone(),
            lines: draft.lines.clone(),
        };
        st.journals.push(entry.clone());
        st.audit(DocumentRef::Journal(id), "posted", serde_json::Value::Null);
        Ok(entry)
    }

    async fn journal_entry(&self, id: &JournalId) -> BooksResult<JournalEntry> {
        let st = self.enter()?;
        st.journals
            .iter()
            .find(|j| &j.id == id)
            .cloned()
            .ok_or_else(|| not_found("journal entry", id))
    }

    async fn journal(&self, filter: &JournalFilter) -> BooksResult<Vec<JournalEntry>> {
        let st = self.enter()?;
        Ok(st
            .journals
            .iter()
            .filter(|j| filter.from.is_none_or(|f| j.date >= f))
            .filter(|j| filter.to.is_none_or(|t| j.date <= t))
            .filter(|j| filter.reference.is_none() || j.reference == filter.reference)
            .cloned()
            .collect())
    }

    async fn audit_trail(&self, doc: &DocumentRef) -> BooksResult<Vec<AuditEntry>> {
        let st = self.enter()?;
        Ok(st
            .audit
            .iter()
            .filter(|(d, _)| d == doc)
            .map(|(_, e)| e.clone())
            .collect())
    }

    async fn ar_ageing(&self, as_of: NaiveDate) -> BooksResult<AgeingReport> {
        let st = self.enter()?;
        let mut totals = AgeingBuckets::zero(self.currency);
        let mut rows = Vec::new();
        for c in &st.customers {
            let mut b = AgeingBuckets::zero(c.counterparty.currency);
            let mut any = false;
            for i in st
                .invoices
                .iter()
                .filter(|i| i.customer == c.id && i.status.is_open() && i.issue_date <= as_of)
            {
                b.add(i.balance_due, i.due_date, as_of)?;
                totals.add(i.balance_due, i.due_date, as_of)?;
                any = true;
            }
            if any {
                rows.push(AgeingRow {
                    party: DocumentRef::Customer(c.id.clone()),
                    name: c.counterparty.name.clone(),
                    buckets: b,
                });
            }
        }
        Ok(AgeingReport {
            as_of,
            rows,
            totals,
        })
    }

    async fn ap_ageing(&self, as_of: NaiveDate) -> BooksResult<AgeingReport> {
        let st = self.enter()?;
        let mut totals = AgeingBuckets::zero(self.currency);
        let mut rows = Vec::new();
        for v in &st.vendors {
            let mut b = AgeingBuckets::zero(v.counterparty.currency);
            let mut any = false;
            for bill in st
                .bills
                .iter()
                .filter(|b| b.vendor == v.id && b.status.is_open() && b.issue_date <= as_of)
            {
                b.add(bill.balance_due, bill.due_date, as_of)?;
                totals.add(bill.balance_due, bill.due_date, as_of)?;
                any = true;
            }
            if any {
                rows.push(AgeingRow {
                    party: DocumentRef::Vendor(v.id.clone()),
                    name: v.counterparty.name.clone(),
                    buckets: b,
                });
            }
        }
        Ok(AgeingReport {
            as_of,
            rows,
            totals,
        })
    }

    async fn set_credit_limit(
        &self,
        id: &CustomerId,
        limit: Option<Money>,
    ) -> BooksResult<Customer> {
        let mut st = self.enter()?;
        let c = st.customer_mut(id)?;
        c.credit_limit = limit;
        let c = c.clone();
        st.audit(
            DocumentRef::Customer(id.clone()),
            "credit_limit",
            serde_json::json!({"limit": limit}),
        );
        Ok(st.customer_view(&c))
    }

    async fn credit_position(&self, id: &CustomerId) -> BooksResult<CreditPosition> {
        let st = self.enter()?;
        let c = st
            .customers
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| not_found("customer", id))?;
        let exposure = st.customer_view(c).balance;
        let available = match c.credit_limit {
            Some(l) => Some(l.checked_sub(exposure)?),
            None => None,
        };
        Ok(CreditPosition {
            customer: id.clone(),
            limit: c.credit_limit,
            exposure,
            available,
        })
    }

    async fn create_lead(&self, draft: &LeadDraft) -> BooksResult<Lead> {
        let mut st = self.enter()?;
        let lead = Lead {
            id: LeadId(st.next("lead")),
            contact_name: draft.contact_name.clone(),
            company_name: draft.company_name.clone(),
            email: draft.email.clone(),
            status: LeadStatus::New,
        };
        st.leads.push(lead.clone());
        Ok(lead)
    }

    async fn leads(&self) -> BooksResult<Vec<Lead>> {
        Ok(self.enter()?.leads.clone())
    }

    async fn convert_lead(
        &self,
        id: &LeadId,
        draft: &OpportunityDraft,
    ) -> BooksResult<Opportunity> {
        let mut st = self.enter()?;
        let lead = st
            .leads
            .iter_mut()
            .find(|l| &l.id == id)
            .ok_or_else(|| not_found("lead", id))?;
        if matches!(lead.status, LeadStatus::Converted | LeadStatus::Lost) {
            return Err(BooksError::Conflict(format!(
                "lead {id} is already {:?}",
                lead.status
            )));
        }
        lead.status = LeadStatus::Converted;
        let opp = Opportunity {
            id: OpportunityId(st.next("opp")),
            name: draft.name.clone(),
            lead: Some(id.clone()),
            customer: draft.customer.clone(),
            amount: draft.amount,
            stage: OpportunityStage::Open,
        };
        st.opportunities.push(opp.clone());
        Ok(opp)
    }

    async fn win_opportunity(&self, id: &OpportunityId) -> BooksResult<Opportunity> {
        let mut st = self.enter()?;
        let opp = st
            .opportunities
            .iter_mut()
            .find(|o| &o.id == id)
            .ok_or_else(|| not_found("opportunity", id))?;
        if opp.stage != OpportunityStage::Open {
            return Err(BooksError::Conflict(format!(
                "opportunity {id} is already {:?}",
                opp.stage
            )));
        }
        opp.stage = OpportunityStage::Won;
        Ok(opp.clone())
    }

    async fn lose_opportunity(
        &self,
        id: &OpportunityId,
        _reason: &str,
    ) -> BooksResult<Opportunity> {
        let mut st = self.enter()?;
        let opp = st
            .opportunities
            .iter_mut()
            .find(|o| &o.id == id)
            .ok_or_else(|| not_found("opportunity", id))?;
        if opp.stage != OpportunityStage::Open {
            return Err(BooksError::Conflict(format!(
                "opportunity {id} is already {:?}",
                opp.stage
            )));
        }
        opp.stage = OpportunityStage::Lost;
        Ok(opp.clone())
    }

    async fn opportunities(&self) -> BooksResult<Vec<Opportunity>> {
        Ok(self.enter()?.opportunities.clone())
    }

    async fn open_ticket(&self, draft: &TicketDraft) -> BooksResult<Ticket> {
        let mut st = self.enter()?;
        if let Some(c) = &draft.customer {
            st.customer_mut(c)?;
        }
        let id = TicketId(st.next("tkt"));
        let t = Ticket {
            id: id.clone(),
            number: Some(id.0.to_uppercase()),
            customer: draft.customer.clone(),
            subject: draft.subject.clone(),
            status: TicketStatus::Open,
            priority: draft.priority,
        };
        st.tickets.push(t.clone());
        Ok(t)
    }

    async fn update_ticket(
        &self,
        id: &TicketId,
        status: TicketStatus,
        _note: Option<&str>,
    ) -> BooksResult<Ticket> {
        let mut st = self.enter()?;
        let t = st
            .tickets
            .iter_mut()
            .find(|t| &t.id == id)
            .ok_or_else(|| not_found("ticket", id))?;
        t.status = status;
        Ok(t.clone())
    }

    async fn tickets(&self, status: Option<TicketStatus>) -> BooksResult<Vec<Ticket>> {
        let st = self.enter()?;
        Ok(st
            .tickets
            .iter()
            .filter(|t| status.is_none_or(|s| t.status == s))
            .cloned()
            .collect())
    }
}
