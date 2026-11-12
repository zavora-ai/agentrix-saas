//! EraBooks against the local Zavora ERA (hosted) instance. Ignored by default; run with
//!
//! ```text
//! deploy/saas/era/era.sh up && deploy/saas/era/era.sh seed
//! scripts/wcargo test -p saas-books --test era_live -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Ledger writes go through mcp-erp when its binary is built (see `McpConfig::from_env`), over
//! REST otherwise; the test prints which. Each run uses fresh PayPal-shaped ids (`LIVE…`), so
//! runs don't collide, and posts in November 2026, the open period the seed's tenants have.

use chrono::NaiveDate;
use saas_books::{EraBooks, EraConfig};
use saas_core::books::*;
use saas_core::paypal::PayPalRef;
use saas_core::{Books, CompanyId, Counterparty, Currency, Money, PayPalPosting, post_paypal};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// A fresh tag per run, for ids that must not repeat across runs.
fn tag() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:X}", n % 0xFFFF_FFFF_FFFF)
}

async fn books(company: CompanyId) -> EraBooks {
    let books = EraBooks::connect(
        EraConfig::from_env(company).expect("ERA tenants file (run era.sh seed)"),
    )
    .await
    .expect("local ERA on :18180 (run era.sh up)");
    println!("{} books via {}", books.company(), books.via());
    books
}

async fn invoice_for(
    books: &EraBooks,
    customer: &CustomerId,
    product: &str,
    reference: &str,
) -> Invoice {
    let products = books.products().await.unwrap();
    let plan = products
        .iter()
        .find(|p| p.name == product)
        .unwrap_or_else(|| panic!("seeded product {product}"));
    let draft = InvoiceDraft {
        customer: customer.clone(),
        issue_date: d("2026-11-10"),
        due_date: d("2026-11-24"),
        currency: Currency::USD,
        lines: vec![LineDraft {
            product: Some(plan.id.clone()),
            description: plan.name.clone(),
            quantity: 1,
            unit_price: plan.price,
        }],
        reference: Some(reference.to_owned()),
        memo: Some("saas-books live test".into()),
    };
    let inv = books.draft_invoice(&draft).await.unwrap();
    assert_eq!(inv.status, DocumentStatus::Draft);
    assert_eq!(inv.total, plan.price, "out of scope for VAT: no 16% added");
    assert_eq!(inv.reference.as_deref(), Some(reference));
    let inv = books.post_invoice(&inv.id).await.unwrap();
    assert_eq!(inv.status, DocumentStatus::Posted);
    inv
}

#[tokio::test]
#[ignore = "needs the local ERA: deploy/saas/era/era.sh up && seed"]
async fn a_capture_pays_an_agentrix_invoice_once_and_a_refund_leaves_clearing() {
    let books = books(CompanyId::agentrix()).await;
    let t = tag();
    let kilele = books
        .ensure_customer(&Counterparty::usd("Kilele Bakery", None), None)
        .await
        .unwrap();
    assert_eq!(
        kilele.counterparty.company,
        Some(CompanyId::kilele()),
        "the seed's notes link the company"
    );
    let inv = invoice_for(
        &books,
        &kilele.id,
        "WebAgent Business plan, monthly",
        &format!("live:{t}"),
    )
    .await;
    let clearing_before = books
        .account(AccountRole::PayPalClearing)
        .await
        .unwrap()
        .balance
        .unwrap();

    let capture = PayPalRef::Capture(format!("LIVE{t}").parse().unwrap());
    let posting = PayPalPosting::Received {
        paypal: capture.clone(),
        customer: kilele.id.clone(),
        gross: inv.total,
        fee: Money::usd(325),
        date: d("2026-11-10"),
        invoice: Some(inv.id.clone()),
    };
    let first = post_paypal(&books, &posting).await.unwrap();
    println!(
        "capture {capture}: payment {:?}, fee journal {:?}",
        first.payment, first.fee_journal
    );
    assert!(!first.already_posted);
    assert_eq!(
        first.applied,
        vec![(DocumentRef::Invoice(inv.id.clone()), inv.total)]
    );
    let paid = books.invoice(&inv.id).await.unwrap();
    assert_eq!(paid.status, DocumentStatus::Paid);
    assert!(paid.balance_due.is_zero());

    let again = post_paypal(&books, &posting).await.unwrap();
    assert!(again.already_posted, "the same PayPal id posts once");
    assert_eq!(
        (again.payment, again.fee_journal),
        (first.payment.clone(), first.fee_journal.clone())
    );

    let fee = books
        .journal(&JournalFilter {
            reference: Some(capture.fee_key()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(fee.len(), 1);
    assert_eq!(
        fee[0]
            .lines
            .iter()
            .find(|l| l.account.as_str() == "8075")
            .map(|l| l.debit),
        Some(Money::usd(325))
    );
    let clearing = books
        .account(AccountRole::PayPalClearing)
        .await
        .unwrap()
        .balance
        .unwrap();
    assert_eq!(
        clearing.checked_sub(clearing_before).unwrap(),
        inv.total.checked_sub(Money::usd(325)).unwrap(),
        "clearing holds the net"
    );

    let refund = PayPalPosting::Refunded {
        paypal: PayPalRef::Refund(format!("LIVEREF{t}").parse().unwrap()),
        customer: kilele.id.clone(),
        amount: inv.total,
        fee_returned: Money::zero(Currency::USD),
        date: d("2026-11-10"),
        invoice: Some(inv.id.clone()),
        reason: "live test refund".into(),
    };
    let r = post_paypal(&books, &refund).await.unwrap();
    println!("refund: journal {:?}", r.journal);
    assert!(r.journal.is_some());
    assert!(post_paypal(&books, &refund).await.unwrap().already_posted);
    let after = books
        .account(AccountRole::PayPalClearing)
        .await
        .unwrap()
        .balance
        .unwrap();
    assert_eq!(
        after,
        clearing_before.checked_sub(Money::usd(325)).unwrap(),
        "the refund left clearing; the fee stays spent"
    );
}

#[tokio::test]
#[ignore = "needs the local ERA: deploy/saas/era/era.sh up && seed"]
async fn one_paypal_id_clears_both_companies_ledgers() {
    let agentrix = books(CompanyId::agentrix()).await;
    let kilele = books(CompanyId::kilele()).await;
    let t = tag();
    let capture = PayPalRef::Capture(format!("LIVEA2A{t}").parse().unwrap());

    // Agentrix's receivable.
    let customer = agentrix
        .ensure_customer(&Counterparty::usd("Kilele Bakery", None), None)
        .await
        .unwrap();
    let inv = invoice_for(
        &agentrix,
        &customer.id,
        "WebAgent Essential plan, monthly",
        &format!("a2a:{t}"),
    )
    .await;
    // Kilele's payable for the same invoice.
    let vendor = kilele
        .ensure_vendor(&Counterparty::usd("Agentrix Ltd", None))
        .await
        .unwrap();
    let bill = kilele
        .draft_bill(&BillDraft {
            vendor: vendor.id.clone(),
            vendor_reference: inv.number.clone(),
            issue_date: inv.issue_date,
            due_date: inv.due_date,
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: "WebAgent Essential plan, monthly".into(),
                quantity: 1,
                unit_price: inv.total,
            }],
            memo: Some(format!(
                "Agentrix invoice {}",
                inv.number.clone().unwrap_or_default()
            )),
        })
        .await
        .unwrap();
    assert_eq!(
        bill.total, inv.total,
        "out of scope for VAT on the bill too"
    );
    let bill = kilele.post_bill(&bill.id).await.unwrap();
    assert_eq!(bill.status, DocumentStatus::Posted);

    let received = post_paypal(
        &agentrix,
        &PayPalPosting::Received {
            paypal: capture.clone(),
            customer: customer.id.clone(),
            gross: inv.total,
            fee: Money::usd(118),
            date: d("2026-11-10"),
            invoice: Some(inv.id.clone()),
        },
    )
    .await
    .unwrap();
    let paid = post_paypal(
        &kilele,
        &PayPalPosting::PaidOut {
            paypal: capture.clone(),
            vendor: vendor.id.clone(),
            amount: inv.total,
            fee: Money::zero(Currency::USD),
            date: d("2026-11-10"),
            bill: Some(bill.id.clone()),
        },
    )
    .await
    .unwrap();
    println!(
        "{capture}: Agentrix receipt {:?}, Kilele payment {:?}",
        received.payment, paid.payment
    );
    assert_eq!(
        agentrix.invoice(&inv.id).await.unwrap().status,
        DocumentStatus::Paid
    );
    assert_eq!(
        kilele.bill(&bill.id).await.unwrap().status,
        DocumentStatus::Paid
    );
    let key = Some(capture.key());
    assert_eq!(
        agentrix
            .payments(&PaymentFilter {
                reference: key.clone(),
                direction: None
            })
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        kilele
            .payments(&PaymentFilter {
                reference: key,
                direction: None
            })
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
#[ignore = "needs the local ERA: deploy/saas/era/era.sh up && seed"]
async fn payouts_crm_recurring_ageing_and_tickets() {
    let books = books(CompanyId::agentrix()).await;
    let t = tag();

    // A payout pays a Vultr bill.
    let vultr = books
        .ensure_vendor(&Counterparty::usd("Vultr", None))
        .await
        .unwrap();
    let bill = books
        .draft_bill(&BillDraft {
            vendor: vultr.id.clone(),
            vendor_reference: Some(format!("VLT-{t}")),
            issue_date: d("2026-11-01"),
            due_date: d("2026-11-15"),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: "saas-node, November".into(),
                quantity: 1,
                unit_price: Money::usd(4_000),
            }],
            memo: None,
        })
        .await
        .unwrap();
    books.post_bill(&bill.id).await.unwrap();
    let ap = books.ap_ageing(d("2026-11-10")).await.unwrap();
    assert!(
        ap.rows
            .iter()
            .any(|r| r.party == DocumentRef::Vendor(vultr.id.clone()))
    );
    let p = post_paypal(
        &books,
        &PayPalPosting::PaidOut {
            paypal: PayPalRef::Payout(format!("LIVEPO{t}").parse().unwrap()),
            vendor: vultr.id.clone(),
            amount: Money::usd(4_000),
            fee: Money::usd(25),
            date: d("2026-11-10"),
            bill: Some(bill.id.clone()),
        },
    )
    .await
    .unwrap();
    assert!(p.fee_journal.is_some());
    assert_eq!(
        books.bill(&bill.id).await.unwrap().status,
        DocumentStatus::Paid
    );

    // AR ageing and credit.
    let kilele = books
        .ensure_customer(&Counterparty::usd("Kilele Bakery", None), None)
        .await
        .unwrap();
    books.ar_ageing(d("2026-11-10")).await.unwrap();
    let credit = books.credit_position(&kilele.id).await.unwrap();
    assert_eq!(credit.limit, Some(Money::usd(80_000)));

    // CRM: lead → opportunity → won.
    let lead = books
        .create_lead(&LeadDraft {
            contact_name: format!("Live Lead {t}"),
            company_name: Some("Kilele Bakery".into()),
            email: Some(format!("lead-{t}@example.com").to_lowercase()),
            source: Some("sales-desk".into()),
            notes: None,
        })
        .await
        .unwrap();
    let opp = books
        .convert_lead(
            &lead.id,
            &OpportunityDraft {
                name: "WebAgent Business plan, 12 months".into(),
                amount: Money::usd(75_840),
                expected_close: Some(d("2026-11-30")),
                customer: Some(kilele.id.clone()),
            },
        )
        .await
        .unwrap();
    assert_eq!(opp.amount, Money::usd(75_840));
    assert_eq!(opp.customer, Some(kilele.id.clone()));
    let won = books.win_opportunity(&opp.id).await.unwrap();
    assert_eq!(won.stage, OpportunityStage::Won);
    assert!(
        books
            .leads()
            .await
            .unwrap()
            .iter()
            .any(|l| l.id == lead.id && l.status == LeadStatus::Converted)
    );

    // A recurring invoice, idempotent by reference, then stopped.
    let products = books.products().await.unwrap();
    let monthly = products
        .iter()
        .find(|p| p.name == "WebAgent Essential plan, monthly")
        .unwrap();
    let draft = RecurringInvoiceDraft {
        customer: kilele.id.clone(),
        currency: Currency::USD,
        lines: vec![LineDraft {
            product: Some(monthly.id.clone()),
            description: monthly.name.clone(),
            quantity: 1,
            unit_price: monthly.price,
        }],
        frequency: Frequency::Monthly,
        start_date: d("2026-12-10"),
        end_date: None,
        due_in_days: 14,
        reference: Some(format!("paypal:subscription:I-LIVE{t}")),
    };
    let r = books.create_recurring_invoice(&draft).await.unwrap();
    assert_eq!(r.amount, monthly.price);
    assert_eq!(r.next_date, Some(d("2026-12-10")));
    assert_eq!(
        books.create_recurring_invoice(&draft).await.unwrap().id,
        r.id
    );
    let stopped = books.stop_recurring_invoice(&r.id).await.unwrap();
    assert!(!stopped.active);
    let listed = books.recurring_invoices().await.unwrap();
    assert!(
        listed.iter().all(|x| x.id != r.id || !x.active),
        "a stopped schedule raises nothing: {listed:?}"
    );

    // A support ticket through ERA's customer portal (it serves Agentrix's tenant).
    let ticket = books
        .open_ticket(&TicketDraft {
            customer: Some(kilele.id.clone()),
            subject: format!("Cancel the monthly plan ({t})"),
            description: "We moved to the 12-month plan.".into(),
            priority: TicketPriority::Normal,
        })
        .await
        .unwrap();
    let ticket = books
        .update_ticket(
            &ticket.id,
            TicketStatus::Resolved,
            Some("Done, and the last month refunded."),
        )
        .await
        .unwrap();
    assert_eq!(ticket.status, TicketStatus::Resolved);
    assert!(
        books
            .tickets(Some(TicketStatus::Resolved))
            .await
            .unwrap()
            .iter()
            .any(|x| x.id == ticket.id)
    );
}
