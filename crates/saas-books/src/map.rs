//! ERA's JSON onto the books contract's types, and back.

use chrono::NaiveDate;
use saas_core::books::*;
use saas_core::{BooksError, CompanyId, Counterparty, Currency, Money};
use serde_json::{Value, json};

pub(crate) type R<T> = Result<T, BooksError>;

pub(crate) fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(crate) fn opt_s(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// ERA writes some enums JSON-quoted inside a string (`"\"Monthly\""`).
pub(crate) fn unquote(v: &Value) -> String {
    s(v).trim_matches('"').to_owned()
}

pub(crate) fn currency(v: &Value, fallback: Currency) -> Currency {
    v.as_str()
        .and_then(|c| Currency::new(c).ok())
        .unwrap_or(fallback)
}

/// An ERA amount (a decimal string, sometimes a number, null for nothing) in `currency`.
pub(crate) fn money(v: &Value, currency: Currency) -> R<Money> {
    match v {
        Value::Null => Ok(Money::zero(currency)),
        Value::String(t) => Ok(Money::parse_decimal(t, currency)?),
        Value::Number(n) => Ok(Money::parse_decimal(&n.to_string(), currency)?),
        other => Err(BooksError::Backend(format!(
            "ERA sent {other} where an amount belongs"
        ))),
    }
}

pub(crate) fn opt_money(v: &Value, currency: Currency) -> R<Option<Money>> {
    if v.is_null() {
        Ok(None)
    } else {
        money(v, currency).map(Some)
    }
}

/// An amount as ERA reads it on REST: a decimal string.
pub(crate) fn dec(m: Money) -> Value {
    Value::String(m.to_decimal())
}

/// An amount as mcp-erp's tools take it: a JSON number, from the exact decimal text, so the
/// number ERA reads back is the same decimal.
pub(crate) fn num(m: Money) -> Value {
    m.to_decimal()
        .parse::<f64>()
        .map(|f| json!(f))
        .unwrap_or(Value::Null)
}

pub(crate) fn date(v: &Value) -> R<NaiveDate> {
    let t = s(v);
    let day = t.get(..10).unwrap_or(&t);
    day.parse()
        .map_err(|_| BooksError::Backend(format!("ERA sent {t:?} where a date belongs")))
}

pub(crate) fn opt_date(v: &Value) -> R<Option<NaiveDate>> {
    if v.is_null() {
        Ok(None)
    } else {
        date(v).map(Some)
    }
}

fn quantity(v: &Value) -> u32 {
    s(v).parse::<f64>()
        .map(|q| q.round().max(0.0) as u32)
        .unwrap_or(0)
}

// ── Notes: where the contract's extra fields live in ERA ─────────────────────────────────────

const REF: &str = "[ref] ";

/// A document's notes: the memo, then `[ref] <reference>` on its own line.
pub(crate) fn notes_with_ref(memo: Option<&str>, reference: Option<&str>) -> Value {
    let mut lines: Vec<String> = memo
        .filter(|m| !m.is_empty())
        .map(|m| vec![m.to_owned()])
        .unwrap_or_default();
    if let Some(r) = reference {
        lines.push(format!("{REF}{r}"));
    }
    if lines.is_empty() {
        Value::Null
    } else {
        Value::String(lines.join("\n"))
    }
}

pub(crate) fn ref_from_notes(notes: &Value) -> Option<String> {
    notes
        .as_str()?
        .lines()
        .find_map(|l| l.strip_prefix(REF))
        .map(str::to_owned)
}

/// A party's notes: `agentrix.company: …` and `agentrix.paypal: …` lines (the seed writes these).
pub(crate) fn party_notes(who: &Counterparty) -> Value {
    let mut lines = Vec::new();
    if let Some(c) = &who.company {
        lines.push(format!("agentrix.company: {c}"));
    }
    if let Some(p) = &who.paypal_email {
        lines.push(format!("agentrix.paypal: {p}"));
    }
    if lines.is_empty() {
        Value::Null
    } else {
        Value::String(lines.join("\n"))
    }
}

pub(crate) fn note_value(notes: &Value, key: &str) -> Option<String> {
    let prefix = format!("{key}: ");
    notes
        .as_str()?
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim().to_owned())
}

// ── Parties ──────────────────────────────────────────────────────────────────────────────────

fn first_email(v: &Value) -> Option<String> {
    v.as_array()?.iter().find_map(|e| opt_s(&e["email"]))
}

pub(crate) fn counterparty(row: &Value) -> Counterparty {
    let notes = &row["notes"];
    let bank_paypal = (row["bank_details"]["bank_name"].as_str() == Some("PayPal"))
        .then(|| opt_s(&row["bank_details"]["account_number"]))
        .flatten();
    Counterparty {
        name: s(&row["name"]),
        email: first_email(&row["email"]),
        currency: currency(&row["currency"], Currency::USD),
        company: note_value(notes, "agentrix.company").and_then(|c| CompanyId::new(c).ok()),
        paypal_email: note_value(notes, "agentrix.paypal").or(bank_paypal),
    }
}

pub(crate) fn customer(row: &Value, balance: Money) -> R<Customer> {
    let cp = counterparty(row);
    Ok(Customer {
        id: CustomerId(s(&row["id"])),
        credit_limit: opt_money(&row["credit_limit"], cp.currency)?,
        counterparty: cp,
        balance,
    })
}

pub(crate) fn vendor(row: &Value, balance: Money) -> Vendor {
    Vendor {
        id: VendorId(s(&row["id"])),
        counterparty: counterparty(row),
        balance,
    }
}

pub(crate) fn party_body(who: &Counterparty) -> Value {
    let email: Vec<Value> = who
        .email
        .iter()
        .map(|e| json!({"email": e, "label": null, "is_primary": true}))
        .collect();
    json!({
        "name": who.name,
        "email": email,
        "phone": [],
        "currency": who.currency.code(),
        "notes": party_notes(who),
    })
}

// ── Documents ────────────────────────────────────────────────────────────────────────────────

pub(crate) fn invoice_status(status: &str, paid: bool) -> DocumentStatus {
    match status {
        "draft" => DocumentStatus::Draft,
        "pending_approval" | "approved" => DocumentStatus::Submitted,
        "partially_paid" => DocumentStatus::PartiallyPaid,
        "paid" => DocumentStatus::Paid,
        "voided" | "written_off" | "cancelled" => DocumentStatus::Void,
        // posted, sent, viewed, overdue, disputed
        _ if paid => DocumentStatus::PartiallyPaid,
        _ => DocumentStatus::Posted,
    }
}

fn lines(v: &Value, cur: Currency) -> R<Vec<Line>> {
    let Some(rows) = v.as_array() else {
        return Ok(Vec::new());
    };
    rows.iter()
        .map(|l| {
            Ok(Line {
                product: opt_s(&l["product_id"]).map(ProductId),
                description: s(&l["description"]),
                quantity: quantity(&l["quantity"]),
                unit_price: money(&l["unit_price"], cur)?,
                amount: money(
                    if l["line_total"].is_null() {
                        &l["amount"]
                    } else {
                        &l["line_total"]
                    },
                    cur,
                )?,
            })
        })
        .collect()
}

/// `{invoice, lines}` (a read), the full invoice (a create), or a list row (no lines).
pub(crate) fn invoice(v: &Value) -> R<Invoice> {
    let (inv, line_rows) = if v["invoice"].is_object() {
        (&v["invoice"], &v["lines"])
    } else {
        (v, &v["lines"])
    };
    let cur = currency(&inv["currency"], Currency::USD);
    let paid = money(&inv["amount_paid"], cur)?;
    Ok(Invoice {
        id: InvoiceId(s(&inv["id"])),
        number: opt_s(&inv["number"]),
        customer: CustomerId(s(&inv["customer_id"])),
        status: invoice_status(&s(&inv["status"]), paid.is_positive()),
        issue_date: date(&inv["issue_date"])?,
        due_date: date(&inv["due_date"])?,
        lines: lines(line_rows, cur)?,
        total: money(&inv["gross_total"], cur)?,
        balance_due: money(&inv["balance_due"], cur)?,
        reference: ref_from_notes(&inv["notes"]),
    })
}

pub(crate) fn bill(v: &Value) -> R<Bill> {
    let b = if v["bill"].is_object() { &v["bill"] } else { v };
    let cur = currency(&b["currency"], Currency::USD);
    let paid = money(&b["amount_paid"], cur)?;
    Ok(Bill {
        id: BillId(s(&b["id"])),
        number: opt_s(&b["number"]),
        vendor: VendorId(s(&b["vendor_id"])),
        vendor_reference: opt_s(&b["vendor_invoice_number"]),
        status: invoice_status(&s(&b["status"]), paid.is_positive()),
        issue_date: date(&b["issue_date"])?,
        due_date: date(&b["due_date"])?,
        total: money(&b["gross_total"], cur)?,
        balance_due: money(&b["balance_due"], cur)?,
    })
}

/// Lines for an invoice or bill, with every line out of scope for VAT (ERA adds 16% otherwise).
pub(crate) fn lines_body(lines: &[LineDraft], default_product: &str) -> Vec<Value> {
    lines
        .iter()
        .map(|l| {
            json!({
                "product_id": l.product.as_ref().map(|p| p.0.clone()).unwrap_or_else(|| default_product.to_owned()),
                "description": l.description,
                "quantity": l.quantity.to_string(),
                "unit_price": dec(l.unit_price),
                "vat_treatment": "OutOfScope",
            })
        })
        .collect()
}

pub(crate) fn payment(v: &Value) -> R<Payment> {
    let cur = currency(&v["currency"], Currency::USD);
    let received = s(&v["payment_type"]) == "customer_payment";
    let party_id = s(&v["party_id"]);
    let apps = match &v["applications"] {
        Value::String(t) => serde_json::from_str(t).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let mut applied = Vec::new();
    for a in apps.as_array().into_iter().flatten() {
        let id = s(&a["document_id"]);
        let doc = match a["document_type"].as_str() {
            Some("Bill") => DocumentRef::Bill(BillId(id)),
            _ => DocumentRef::Invoice(InvoiceId(id)),
        };
        let amount = if a["amount_applied"].is_null() {
            &a["amount"]
        } else {
            &a["amount_applied"]
        };
        applied.push((doc, money(amount, cur)?));
    }
    Ok(Payment {
        id: PaymentId(s(&v["id"])),
        number: opt_s(&v["number"]),
        direction: if received {
            PaymentDirection::Received
        } else {
            PaymentDirection::Paid
        },
        party: if received {
            DocumentRef::Customer(CustomerId(party_id))
        } else {
            DocumentRef::Vendor(VendorId(party_id))
        },
        date: date(&v["payment_date"])?,
        amount: money(&v["amount"], cur)?,
        reference: opt_s(&v["reference"]),
        applied,
        unapplied: money(&v["unapplied"], cur)?,
        journal: opt_s(&v["journal_entry_id"]).map(JournalId),
    })
}

/// `{entry, lines}` (a read) or `{entry: {..., lines}}` (a create).
pub(crate) fn journal(v: &Value) -> R<JournalEntry> {
    let e = if v["entry"].is_object() {
        &v["entry"]
    } else {
        v
    };
    let line_rows = if v["lines"].is_array() {
        &v["lines"]
    } else {
        &e["lines"]
    };
    let mut out = Vec::new();
    for l in line_rows.as_array().into_iter().flatten() {
        let cur = currency(&l["currency"], Currency::USD);
        out.push(JournalLine {
            account: AccountCode(s(&l["account_code"])),
            debit: money(&l["debit"], cur)?,
            credit: money(&l["credit"], cur)?,
            memo: opt_s(&l["description"]),
        });
    }
    Ok(JournalEntry {
        id: JournalId(s(&e["id"])),
        number: opt_s(&e["number"]),
        date: date(&e["date"])?,
        memo: s(&e["description"]),
        reference: opt_s(&e["reference"]),
        lines: out,
    })
}

pub(crate) fn frequency_name(f: Frequency) -> &'static str {
    match f {
        Frequency::Weekly => "Weekly",
        Frequency::Monthly => "Monthly",
        Frequency::Quarterly => "Quarterly",
        Frequency::Yearly => "Annual",
    }
}

pub(crate) fn frequency(v: &Value) -> Frequency {
    match unquote(v).as_str() {
        "Weekly" | "Biweekly" => Frequency::Weekly,
        "Quarterly" | "SemiAnnual" => Frequency::Quarterly,
        "Annual" => Frequency::Yearly,
        _ => Frequency::Monthly,
    }
}

pub(crate) fn recurring(row: &Value) -> R<RecurringInvoice> {
    let template = match &row["template"] {
        Value::String(t) => serde_json::from_str(t).unwrap_or(Value::Null),
        other => other.clone(),
    };
    let cur = currency(&template["currency"], Currency::USD);
    let mut amount = Money::zero(cur);
    for l in template["lines"].as_array().into_iter().flatten() {
        let price = money(&l["unit_price"], cur)?;
        amount =
            amount.checked_add(price.checked_mul(i64::from(quantity(&l["quantity"]).max(1)))?)?;
    }
    let active = row["is_active"].as_bool().unwrap_or(true);
    let end_date = opt_date(&row["end_date"])?;
    let next = opt_date(&row["next_run"])?.filter(|n| active && end_date.is_none_or(|e| *n <= e));
    Ok(RecurringInvoice {
        id: RecurringInvoiceId(s(&row["id"])),
        customer: CustomerId(s(&row["customer_id"])),
        frequency: frequency(&row["frequency"]),
        amount,
        next_date: next,
        end_date,
        active: next.is_some(),
        reference: ref_from_notes(&template["notes"]),
    })
}

pub(crate) fn ageing(
    report: &Value,
    kind: &str,
    as_of: NaiveDate,
    cur: Currency,
) -> R<AgeingReport> {
    let content = &report["content"][kind];
    let buckets = |v: &Value| -> R<AgeingBuckets> {
        Ok(AgeingBuckets {
            current: money(&v["current"], cur)?,
            days_1_30: money(&v["days_1_30"], cur)?,
            days_31_60: money(&v["days_31_60"], cur)?,
            days_61_90: money(&v["days_61_90"], cur)?,
            over_90: money(&v["over_90"], cur)?,
        })
    };
    let mut rows = Vec::new();
    for l in content["lines"].as_array().into_iter().flatten() {
        let id = s(&l["party_id"]);
        rows.push(AgeingRow {
            party: if kind == "ApAgeing" {
                DocumentRef::Vendor(VendorId(id))
            } else {
                DocumentRef::Customer(CustomerId(id))
            },
            name: s(&l["party_name"]),
            buckets: buckets(l)?,
        });
    }
    let totals = if content["totals"].is_object() {
        buckets(&content["totals"])?
    } else {
        AgeingBuckets::zero(cur)
    };
    Ok(AgeingReport {
        as_of,
        rows,
        totals,
    })
}

pub(crate) fn lead(v: &Value) -> Lead {
    let status = match s(&v["status"]).to_ascii_lowercase().as_str() {
        "converted" => LeadStatus::Converted,
        "working" | "qualified" => LeadStatus::Qualified,
        "unqualified" | "lost" => LeadStatus::Lost,
        _ => LeadStatus::New,
    };
    Lead {
        id: LeadId(s(&v["id"])),
        contact_name: s(&v["name"]),
        company_name: opt_s(&v["company"]),
        email: opt_s(&v["email"]),
        status,
    }
}

pub(crate) fn opportunity(v: &Value) -> R<Opportunity> {
    let cur = currency(&v["currency"], Currency::USD);
    let stage = match s(&v["status"]).as_str() {
        "Won" => OpportunityStage::Won,
        "Lost" => OpportunityStage::Lost,
        _ => OpportunityStage::Open,
    };
    Ok(Opportunity {
        id: OpportunityId(s(&v["id"])),
        name: s(&v["name"]),
        lead: opt_s(&v["lead_id"]).map(LeadId),
        customer: opt_s(&v["customer_id"]).map(CustomerId),
        amount: money(&v["amount"], cur)?,
        stage,
    })
}

pub(crate) fn ticket_status_name(s: TicketStatus) -> &'static str {
    match s {
        TicketStatus::Open => "Open",
        TicketStatus::Pending => "Pending",
        TicketStatus::Resolved => "Resolved",
        TicketStatus::Closed => "Closed",
    }
}

pub(crate) fn priority_name(p: TicketPriority) -> &'static str {
    match p {
        TicketPriority::Low => "Low",
        TicketPriority::Normal => "Normal",
        TicketPriority::High => "High",
        TicketPriority::Urgent => "Urgent",
    }
}

pub(crate) fn ticket(v: &Value) -> Ticket {
    let t = if v["ticket"].is_object() {
        &v["ticket"]
    } else {
        v
    };
    let status = match s(&t["status"]).to_ascii_lowercase().as_str() {
        "pending" => TicketStatus::Pending,
        "resolved" => TicketStatus::Resolved,
        "closed" => TicketStatus::Closed,
        _ => TicketStatus::Open,
    };
    let priority = match s(&t["priority"]).to_ascii_lowercase().as_str() {
        "low" => TicketPriority::Low,
        "high" => TicketPriority::High,
        "urgent" => TicketPriority::Urgent,
        _ => TicketPriority::Normal,
    };
    Ticket {
        id: TicketId(s(&t["id"])),
        number: None,
        customer: opt_s(&t["customer_id"]).map(CustomerId),
        subject: s(&t["subject"]),
        status,
        priority,
    }
}

pub(crate) fn audit(v: &Value) -> AuditEntry {
    let actor = opt_s(&v["actor_email"]).or_else(|| opt_s(&v["actor"]["id"]));
    AuditEntry {
        at: s(&v["timestamp"]),
        actor,
        action: s(&v["event_type"]),
        details: json!({"object_type": v["object_type"], "after": v["after"], "metadata": v["metadata"]}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_carry_the_reference() {
        let n = notes_with_ref(Some("Annual plan"), Some("paypal:capture:ABC"));
        assert_eq!(n, json!("Annual plan\n[ref] paypal:capture:ABC"));
        assert_eq!(ref_from_notes(&n).as_deref(), Some("paypal:capture:ABC"));
        assert_eq!(notes_with_ref(None, None), Value::Null);
        assert_eq!(ref_from_notes(&json!("no ref here")), None);
    }

    #[test]
    fn amounts_round_trip_exactly() {
        assert_eq!(
            money(&json!("228.0000"), Currency::USD).unwrap(),
            Money::usd(22_800)
        );
        assert_eq!(money(&json!(8.45), Currency::USD).unwrap(), Money::usd(845));
        assert_eq!(money(&Value::Null, Currency::USD).unwrap(), Money::usd(0));
        assert_eq!(num(Money::usd(845)), json!(8.45));
        assert_eq!(dec(Money::usd(845)), json!("8.45"));
        assert!(money(&json!("8.449999"), Currency::USD).is_err());
    }

    #[test]
    fn statuses_and_frequencies() {
        assert_eq!(
            invoice_status("overdue", true),
            DocumentStatus::PartiallyPaid
        );
        assert_eq!(invoice_status("overdue", false), DocumentStatus::Posted);
        assert_eq!(invoice_status("approved", false), DocumentStatus::Submitted);
        assert_eq!(frequency(&json!("\"Annual\"")), Frequency::Yearly);
        assert_eq!(frequency_name(Frequency::Yearly), "Annual");
    }

    #[test]
    fn parties_read_their_agentrix_notes() {
        let row = json!({
            "id": "c1", "name": "Kilele Bakery", "currency": "USD", "credit_limit": "800.00",
            "email": [{"email": "ap@kilele.example", "is_primary": true}],
            "notes": "agentrix.company: kilele-bakery\nagentrix.paypal: sb@business.example.com",
        });
        let c = customer(&row, Money::usd(0)).unwrap();
        assert_eq!(c.counterparty.company, Some(CompanyId::kilele()));
        assert_eq!(
            c.counterparty.paypal_email.as_deref(),
            Some("sb@business.example.com")
        );
        assert_eq!(c.counterparty.email.as_deref(), Some("ap@kilele.example"));
        assert_eq!(c.credit_limit, Some(Money::usd(80_000)));
        assert_eq!(party_notes(&c.counterparty), row["notes"]);
    }
}
