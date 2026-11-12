//! The contract exercised on [`FakeBooks`]: the posting rules, idempotency by PayPal id, and the
//! receivables, payables, debt and CRM flows the agents use.

use chrono::NaiveDate;

use crate::books::*;
use crate::fake::FakeBooks;
use crate::paypal::PayPalRef;
use crate::posting::{PayPalPosting, post_paypal};
use crate::{CompanyId, Counterparty, Currency, Money};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn line(description: &str, cents: i64) -> LineDraft {
    LineDraft {
        product: None,
        description: description.into(),
        quantity: 1,
        unit_price: Money::usd(cents),
    }
}

fn capture(id: &str) -> PayPalRef {
    PayPalRef::Capture(id.parse().unwrap())
}

async fn posted_invoice(
    books: &FakeBooks,
    customer: &CustomerId,
    cents: i64,
    issued: &str,
    due: &str,
) -> Invoice {
    let draft = InvoiceDraft {
        customer: customer.clone(),
        issue_date: d(issued),
        due_date: d(due),
        currency: Currency::USD,
        lines: vec![line("WebAgent Pro", cents)],
        reference: None,
        memo: None,
    };
    let inv = books.draft_invoice(&draft).await.unwrap();
    books.post_invoice(&inv.id).await.unwrap()
}

async fn kilele(books: &FakeBooks) -> Customer {
    let mut who = Counterparty::usd("Kilele Bakery", Some("ap@kilele.example"));
    who.company = Some(CompanyId::kilele());
    books
        .ensure_customer(&who, Some(Money::usd(500_000)))
        .await
        .unwrap()
}

#[tokio::test]
async fn a_capture_pays_the_invoice_and_books_the_fee_once() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    let inv = posted_invoice(&books, &c.id, 22_800, "2026-11-10", "2026-11-24").await;
    assert_eq!(inv.number.as_deref(), Some("INV-0001"));
    assert_eq!(books.balance("1100"), Money::usd(22_800));

    let posting = PayPalPosting::Received {
        paypal: capture("3C679366HH908993F"),
        customer: c.id.clone(),
        gross: Money::usd(22_800),
        fee: Money::usd(845),
        date: d("2026-11-10"),
        invoice: Some(inv.id.clone()),
    };
    let first = post_paypal(&books, &posting).await.unwrap();
    assert!(!first.already_posted);
    assert_eq!(
        first.applied,
        vec![(DocumentRef::Invoice(inv.id.clone()), Money::usd(22_800))]
    );
    assert!(first.fee_journal.is_some());

    let inv = books.invoice(&inv.id).await.unwrap();
    assert_eq!(inv.status, DocumentStatus::Paid);
    assert!(inv.balance_due.is_zero());
    assert_eq!(books.balance("1100"), Money::usd(0));
    assert_eq!(
        books.balance("1010"),
        Money::usd(22_800 - 845),
        "clearing holds the net, as PayPal does"
    );
    assert_eq!(books.balance("6150"), Money::usd(845));

    let journals = books
        .journal(&JournalFilter::default())
        .await
        .unwrap()
        .len();
    let again = post_paypal(&books, &posting).await.unwrap();
    assert!(again.already_posted);
    assert_eq!(again.payment, first.payment);
    assert_eq!(again.fee_journal, first.fee_journal);
    assert_eq!(
        books
            .journal(&JournalFilter::default())
            .await
            .unwrap()
            .len(),
        journals,
        "nothing posted twice"
    );
    assert_eq!(books.balance("1010"), Money::usd(22_800 - 845));
}

#[tokio::test]
async fn a_crash_between_receipt_and_fee_finishes_on_the_next_run() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    posted_invoice(&books, &c.id, 10_000, "2026-11-01", "2026-11-15").await;
    let posting = PayPalPosting::Received {
        paypal: capture("CRASH1"),
        customer: c.id.clone(),
        gross: Money::usd(10_000),
        fee: Money::usd(399),
        date: d("2026-11-10"),
        invoice: None,
    };
    // payments() for the idempotency check, invoices() for FIFO, record_receipt(), then the
    // fee lookup: fail there, after the receipt is in.
    let receipt_only = {
        let applications = vec![InvoiceApplication {
            invoice: books.invoices(&DocumentFilter::default()).await.unwrap()[0]
                .id
                .clone(),
            amount: Money::usd(10_000),
        }];
        books
            .record_receipt(&ReceiptDraft {
                customer: c.id.clone(),
                date: d("2026-11-10"),
                amount: Money::usd(10_000),
                reference: capture("CRASH1").key(),
                applications,
                memo: None,
            })
            .await
            .unwrap()
    };
    books.fail_next(BooksError::Unavailable("connection reset".into()));
    assert!(post_paypal(&books, &posting).await.unwrap_err().retryable());

    let done = post_paypal(&books, &posting).await.unwrap();
    assert_eq!(done.payment, Some(receipt_only.id));
    assert!(!done.already_posted, "the fee was new");
    assert_eq!(
        books
            .payments(&PaymentFilter::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(books.balance("1010"), Money::usd(10_000 - 399));
}

#[tokio::test]
async fn receipts_without_an_invoice_go_to_the_oldest_due_first() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    let late = posted_invoice(&books, &c.id, 5_000, "2026-09-01", "2026-09-15").await;
    let newer = posted_invoice(&books, &c.id, 5_000, "2026-10-01", "2026-10-15").await;
    let p = post_paypal(
        &books,
        &PayPalPosting::Received {
            paypal: PayPalRef::SubscriptionSale("SALE1".parse().unwrap()),
            customer: c.id.clone(),
            gross: Money::usd(7_000),
            fee: Money::zero(Currency::USD),
            date: d("2026-11-10"),
            invoice: None,
        },
    )
    .await
    .unwrap();
    assert!(p.fee_journal.is_none());
    assert_eq!(
        p.applied,
        vec![
            (DocumentRef::Invoice(late.id.clone()), Money::usd(5_000)),
            (DocumentRef::Invoice(newer.id.clone()), Money::usd(2_000)),
        ]
    );
    assert_eq!(
        books.invoice(&newer.id).await.unwrap().status,
        DocumentStatus::PartiallyPaid
    );
    let ageing = books.ar_ageing(d("2026-11-10")).await.unwrap();
    assert_eq!(ageing.rows.len(), 1);
    assert_eq!(
        ageing.totals.days_1_30,
        Money::usd(3_000),
        "the newer one, 26 days late, owes 30.00"
    );
    assert_eq!(ageing.totals.total().unwrap(), Money::usd(3_000));
    let credit = books.credit_position(&c.id).await.unwrap();
    assert_eq!(credit.exposure, Money::usd(3_000));
    assert_eq!(credit.available, Some(Money::usd(497_000)));
}

#[tokio::test]
async fn a_refund_becomes_a_credit_and_leaves_clearing() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = books
        .ensure_customer(&Counterparty::usd("Wanjiru Designs", None), None)
        .await
        .unwrap();
    let inv = posted_invoice(&books, &c.id, 1_900, "2026-11-10", "2026-11-10").await;
    post_paypal(
        &books,
        &PayPalPosting::Received {
            paypal: capture("CAP19"),
            customer: c.id.clone(),
            gross: Money::usd(1_900),
            fee: Money::usd(90),
            date: d("2026-11-10"),
            invoice: Some(inv.id.clone()),
        },
    )
    .await
    .unwrap();
    let refund = PayPalPosting::Refunded {
        paypal: PayPalRef::Refund("REF19".parse().unwrap()),
        customer: c.id.clone(),
        amount: Money::usd(1_900),
        fee_returned: Money::usd(41),
        date: d("2026-11-11"),
        invoice: Some(inv.id.clone()),
        reason: "Cancelled within the trial".into(),
    };
    let r = post_paypal(&books, &refund).await.unwrap();
    assert!(r.credit_note.is_some() && r.payment.is_some() && r.journal.is_none());
    assert_eq!(books.balance("1010"), Money::usd(1_900 - 90 - 1_900 + 41));
    assert_eq!(books.balance("4000"), Money::usd(0), "revenue reversed");
    assert_eq!(books.balance("1100"), Money::usd(0));
    assert_eq!(books.balance("6150"), Money::usd(90 - 41));
    assert!(post_paypal(&books, &refund).await.unwrap().already_posted);
    assert!(
        books
            .refund_by_reference("paypal:refund:REF19")
            .await
            .unwrap()
            .is_some()
    );

    let too_much = RefundDraft {
        customer: c.id.clone(),
        invoice: Some(inv.id.clone()),
        date: d("2026-11-11"),
        amount: Money::usd(2_000),
        reference: "paypal:refund:REF20".into(),
        reason: "x".into(),
    };
    assert!(matches!(
        books.record_refund(&too_much).await,
        Err(BooksError::Conflict(_))
    ));
}

#[tokio::test]
async fn a_payout_pays_the_bill_and_its_fee() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let vultr = books
        .ensure_vendor(&Counterparty::usd("Vultr", Some("billing@vultr.example")))
        .await
        .unwrap();
    let bill = books
        .draft_bill(&BillDraft {
            vendor: vultr.id.clone(),
            vendor_reference: Some("VLT-2026-11".into()),
            issue_date: d("2026-11-01"),
            due_date: d("2026-11-15"),
            currency: Currency::USD,
            lines: vec![line("saas-node, November", 4_000)],
            memo: None,
        })
        .await
        .unwrap();
    let bill = books.post_bill(&bill.id).await.unwrap();
    assert_eq!(
        books.vendor(&vultr.id).await.unwrap().balance,
        Money::usd(4_000)
    );
    assert_eq!(
        books
            .ap_ageing(d("2026-11-10"))
            .await
            .unwrap()
            .totals
            .current,
        Money::usd(4_000)
    );

    let payout = PayPalPosting::PaidOut {
        paypal: PayPalRef::Payout("AN6FP42Y3464L".parse().unwrap()),
        vendor: vultr.id.clone(),
        amount: Money::usd(4_000),
        fee: Money::usd(25),
        date: d("2026-11-10"),
        bill: None,
    };
    let p = post_paypal(&books, &payout).await.unwrap();
    assert_eq!(
        p.applied,
        vec![(DocumentRef::Bill(bill.id.clone()), Money::usd(4_000))]
    );
    assert_eq!(
        books.bill(&bill.id).await.unwrap().status,
        DocumentStatus::Paid
    );
    assert_eq!(books.balance("2000"), Money::usd(0));
    assert_eq!(books.balance("1010"), Money::usd(-4_025));
    assert!(post_paypal(&books, &payout).await.unwrap().already_posted);
}

#[tokio::test]
async fn bad_postings_are_refused_before_anything_is_written() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    for (gross, fee) in [(0, 0), (100, 101), (100, -1)] {
        let p = PayPalPosting::Received {
            paypal: capture("BAD"),
            customer: c.id.clone(),
            gross: Money::usd(gross),
            fee: Money::usd(fee),
            date: d("2026-11-10"),
            invoice: None,
        };
        assert!(
            matches!(post_paypal(&books, &p).await, Err(BooksError::Invalid(_))),
            "{gross} {fee}"
        );
    }
    assert!(
        books
            .payments(&PaymentFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
    let unknown = JournalDraft {
        date: d("2026-11-10"),
        memo: "x".into(),
        reference: None,
        lines: vec![
            JournalLine {
                account: "9999".into(),
                debit: Money::usd(1),
                credit: Money::usd(0),
                memo: None,
            },
            JournalLine {
                account: "1010".into(),
                debit: Money::usd(0),
                credit: Money::usd(1),
                memo: None,
            },
        ],
    };
    assert!(matches!(
        books.post_journal(&unknown).await,
        Err(BooksError::NotFound { .. })
    ));
}

#[tokio::test]
async fn invoices_move_draft_submitted_posted_and_not_back() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    let draft = InvoiceDraft {
        customer: c.id.clone(),
        issue_date: d("2026-11-10"),
        due_date: d("2026-12-10"),
        currency: Currency::USD,
        lines: vec![
            line("WebAgent Pro, 12 months", 19_000),
            line("Setup", 3_800),
        ],
        reference: Some("a2a:req-1".into()),
        memo: None,
    };
    let inv = books.draft_invoice(&draft).await.unwrap();
    assert_eq!(inv.total, Money::usd(22_800));
    assert_eq!(
        books.submit_invoice(&inv.id).await.unwrap().status,
        DocumentStatus::Submitted
    );
    assert_eq!(
        books.post_invoice(&inv.id).await.unwrap().status,
        DocumentStatus::Posted
    );
    assert!(matches!(
        books.post_invoice(&inv.id).await,
        Err(BooksError::Conflict(_))
    ));
    let mixed = InvoiceDraft {
        lines: vec![LineDraft {
            unit_price: Money::new(100, Currency::KES),
            ..line("x", 0)
        }],
        ..draft.clone()
    };
    assert!(matches!(
        books.draft_invoice(&mixed).await,
        Err(BooksError::Invalid(_))
    ));
    let by_ref = DocumentFilter {
        reference: Some("a2a:req-1".into()),
        ..Default::default()
    };
    assert_eq!(books.invoices(&by_ref).await.unwrap().len(), 1);
    assert_eq!(
        books
            .audit_trail(&DocumentRef::Invoice(inv.id))
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn recurring_invoices_raise_each_due_cycle_until_stopped() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    let r = books
        .create_recurring_invoice(&RecurringInvoiceDraft {
            customer: c.id.clone(),
            currency: Currency::USD,
            lines: vec![line("WebAgent Starter, monthly", 1_900)],
            frequency: Frequency::Monthly,
            start_date: d("2026-09-10"),
            end_date: None,
            due_in_days: 7,
            reference: Some("paypal:subscription:I-J53UUW5DF0KV".into()),
        })
        .await
        .unwrap();
    let again = books
        .create_recurring_invoice(&RecurringInvoiceDraft {
            customer: c.id.clone(),
            currency: Currency::USD,
            lines: vec![line("dup", 1)],
            frequency: Frequency::Weekly,
            start_date: d("2026-09-10"),
            end_date: None,
            due_in_days: 0,
            reference: Some("paypal:subscription:I-J53UUW5DF0KV".into()),
        })
        .await
        .unwrap();
    assert_eq!(again.id, r.id, "idempotent by reference");
    let raised = books.raise_due_recurring(d("2026-11-10")).await.unwrap();
    assert_eq!(
        raised.iter().map(|i| i.issue_date).collect::<Vec<_>>(),
        vec![d("2026-09-10"), d("2026-10-10"), d("2026-11-10")]
    );
    assert!(
        raised
            .iter()
            .all(|i| i.status == DocumentStatus::Posted && i.total == Money::usd(1_900))
    );
    assert!(
        books
            .raise_due_recurring(d("2026-11-10"))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        books.recurring_invoices().await.unwrap()[0].next_date,
        Some(d("2026-12-10"))
    );
    books.stop_recurring_invoice(&r.id).await.unwrap();
    assert!(
        books
            .raise_due_recurring(d("2027-06-01"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn crm_lead_to_won_customer_and_tickets() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let lead = books
        .create_lead(&LeadDraft {
            contact_name: "Achieng Otieno".into(),
            company_name: Some("Kilele Bakery".into()),
            email: Some("achieng@kilele.example".into()),
            source: Some("sales-desk".into()),
            notes: None,
        })
        .await
        .unwrap();
    let c = kilele(&books).await;
    let opp = books
        .convert_lead(
            &lead.id,
            &OpportunityDraft {
                name: "WebAgent Pro, 12 months".into(),
                amount: Money::usd(19_000),
                expected_close: None,
                customer: Some(c.id.clone()),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        books
            .convert_lead(
                &lead.id,
                &OpportunityDraft {
                    name: "x".into(),
                    amount: Money::usd(1),
                    expected_close: None,
                    customer: None,
                }
            )
            .await,
        Err(BooksError::Conflict(_))
    ));
    let won = books.win_opportunity(&opp.id).await.unwrap();
    assert_eq!(won.stage, OpportunityStage::Won);
    assert_eq!(won.customer, Some(c.id.clone()));
    assert_eq!(
        books.leads().await.unwrap()[0].status,
        LeadStatus::Converted
    );

    let t = books
        .open_ticket(&TicketDraft {
            customer: Some(c.id.clone()),
            subject: "Cancel my monthly plan".into(),
            description: "We moved to annual.".into(),
            priority: TicketPriority::Normal,
        })
        .await
        .unwrap();
    books
        .update_ticket(
            &t.id,
            TicketStatus::Resolved,
            Some("Refunded the last month"),
        )
        .await
        .unwrap();
    assert!(
        books
            .tickets(Some(TicketStatus::Open))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(books.tickets(None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn customers_are_matched_by_name_and_keep_their_limit() {
    let books = FakeBooks::new(CompanyId::agentrix());
    let a = kilele(&books).await;
    let b = books
        .ensure_customer(&Counterparty::usd("  kilele bakery ", None), None)
        .await
        .unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(b.credit_limit, Some(Money::usd(500_000)));
    let c = books
        .set_credit_limit(&a.id, Some(Money::usd(100)))
        .await
        .unwrap();
    assert_eq!(c.credit_limit, Some(Money::usd(100)));
    assert_eq!(books.customers().await.unwrap().len(), 1);
    assert!(matches!(
        books.customer(&"nope".into()).await,
        Err(BooksError::NotFound { .. })
    ));
}

#[tokio::test]
async fn a_subscription_payment_pays_its_recurring_invoice_not_the_oldest_one() {
    use crate::paypal::PayPalSubscriptionId;
    use crate::posting::recurring_invoice_for;

    let books = FakeBooks::new(CompanyId::agentrix());
    let c = kilele(&books).await;
    // An older, unrelated open invoice: oldest-first would pay this one.
    let older = posted_invoice(&books, &c.id, 50_000, "2026-10-01", "2026-10-15").await;
    let sub: PayPalSubscriptionId = "I-J53UUW5DF0KV".parse().unwrap();
    books
        .create_recurring_invoice(&RecurringInvoiceDraft {
            customer: c.id.clone(),
            currency: Currency::USD,
            lines: vec![line("WebAgent Essential plan, monthly", 2_900)],
            frequency: Frequency::Monthly,
            start_date: d("2026-11-10"),
            end_date: None,
            due_in_days: 0,
            reference: Some(sub.reference()),
        })
        .await
        .unwrap();
    assert_eq!(
        recurring_invoice_for(&books, &sub.reference(), d("2026-11-09"))
            .await
            .unwrap(),
        None,
        "nothing due yet"
    );
    let invoice = recurring_invoice_for(&books, &sub.reference(), d("2026-11-10"))
        .await
        .unwrap()
        .expect("raised and open");
    let posted = post_paypal(
        &books,
        &PayPalPosting::Received {
            paypal: PayPalRef::SubscriptionSale("8X1234567A1234567".parse().unwrap()),
            customer: c.id.clone(),
            gross: Money::usd(2_900),
            fee: Money::usd(118),
            date: d("2026-11-10"),
            invoice: Some(invoice.clone()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        posted.applied,
        vec![(DocumentRef::Invoice(invoice.clone()), Money::usd(2_900))]
    );
    assert_eq!(
        books.invoice(&invoice).await.unwrap().status,
        DocumentStatus::Paid
    );
    assert_eq!(
        books.invoice(&older.id).await.unwrap().status,
        DocumentStatus::Posted,
        "the unrelated invoice is untouched"
    );
    assert_eq!(
        recurring_invoice_for(&books, &sub.reference(), d("2026-11-10"))
            .await
            .unwrap(),
        None,
        "this cycle is paid"
    );
}
