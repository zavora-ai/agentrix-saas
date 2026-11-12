//! EraBooks over ERA's REST API, against a mocked ERA: what it sends and how it reads ERA back.
//! (mcp-erp is off here; the live test covers the MCP path.)

use chrono::NaiveDate;
use saas_books::{AccountMap, EraBooks, EraConfig};
use saas_core::books::*;
use saas_core::paypal::PayPalRef;
use saas_core::{Books, BooksError, CompanyId, Money, PayPalPosting, post_paypal};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn d(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

async fn era() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ok(
            json!({"access_token": "t1", "user": {"entity_id": "e1", "user_id": "u1"}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/bank-accounts"))
        .respond_with(ok(json!([
            {"id": "bank-kes", "gl_account": "1020", "is_active": true, "balance": "0", "currency": "KES"},
            {"id": "bank1", "gl_account": "1050", "is_active": true, "balance": "27.57", "currency": "USD"},
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/products"))
        .respond_with(ok(json!([
            {"id": "p-svc", "name": "General services", "product_type": "Service", "unit_price": null, "currency": "USD"},
            {"id": "p-exp", "name": "General expense", "product_type": "Expense", "unit_price": null, "currency": "USD"},
        ])))
        .mount(&server)
        .await;
    server
}

fn config(server: &MockServer) -> EraConfig {
    EraConfig {
        company: CompanyId::agentrix(),
        base_url: server.uri(),
        email: "books+agentrix@agentrix.website".into(),
        password: "not-a-real-password".into(),
        accounts: AccountMap::default(),
        #[cfg(feature = "mcp")]
        mcp: None,
    }
}

fn invoice_row(status: &str, paid: &str, due: &str) -> Value {
    json!({
        "id": "inv1", "number": "INV-2026-0001", "invoice_type": "invoice", "customer_id": "c1",
        "status": status, "issue_date": "2026-11-10", "due_date": "2026-11-24", "currency": "USD",
        "gross_total": "29.00", "amount_paid": paid, "balance_due": due,
        "notes": "Kilele's plan\n[ref] a2a:req-7",
    })
}

async fn requests_to(server: &MockServer, verb: &str, p: &str) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == p)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

#[tokio::test]
async fn a_capture_posts_a_receipt_into_clearing_and_a_fee_journal() {
    let server = era().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/payments"))
        .respond_with(ok(json!({"data": [], "has_more": false})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/invoices/inv1"))
        .respond_with(ok(
            json!({"invoice": invoice_row("posted", "0", "29.00"), "lines": []}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/payments"))
        .respond_with(ok(json!({"id": "pay1"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/payments/pay1"))
        .respond_with(ok(json!({
            "id": "pay1", "number": "PAY-2026-0001", "payment_type": "customer_payment", "party_id": "c1",
            "payment_date": "2026-11-10", "amount": "29.00", "currency": "USD", "reference": "paypal:capture:CAP1",
            "applications": [{"document_id": "inv1", "document_type": "Invoice", "amount_applied": "29"}],
            "unapplied": "0",
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/journal-entries"))
        .respond_with(ok(json!({"data": [], "has_more": false})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/accounts"))
        .respond_with(ok(json!([{"code": "1050", "name": "PayPal Clearing - USD"}, {"code": "8075", "name": "PayPal Fees"}])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/journal-entries"))
        .respond_with(ok(
            json!({"entry": {"id": "je1"}, "validation_report": {"is_valid": true}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/journal-entries/je1"))
        .respond_with(ok(json!({
            "entry": {"id": "je1", "number": "JE-2026-0001", "date": "2026-11-10", "description": "PayPal fee", "reference": "paypal:capture:CAP1:fee"},
            "lines": [
                {"account_code": "8075", "debit": "1.43", "credit": null, "currency": "USD"},
                {"account_code": "1050", "debit": null, "credit": "1.43", "currency": "USD"},
            ],
        })))
        .mount(&server)
        .await;

    let books = EraBooks::connect(config(&server)).await.unwrap();
    assert_eq!(books.via(), "rest");
    let posted = post_paypal(
        &books,
        &PayPalPosting::Received {
            paypal: PayPalRef::Capture("CAP1".parse().unwrap()),
            customer: "c1".into(),
            gross: Money::usd(2_900),
            fee: Money::usd(143),
            date: d("2026-11-10"),
            invoice: Some("inv1".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(posted.payment, Some("pay1".into()));
    assert_eq!(posted.fee_journal, Some("je1".into()));
    assert_eq!(
        posted.applied,
        vec![(DocumentRef::Invoice("inv1".into()), Money::usd(2_900))]
    );

    let payment = &requests_to(&server, "POST", "/api/v1/payments").await[0];
    assert_eq!(payment["payment_type"], "customer_payment");
    assert_eq!(
        payment["amount"], "29.00",
        "amounts go to ERA as decimal strings"
    );
    assert_eq!(
        payment["bank_account_id"], "bank1",
        "into the PayPal clearing bank account"
    );
    assert_eq!(payment["reference"], "paypal:capture:CAP1");
    assert_eq!(payment["method"]["Card"]["processor"], "PayPal");
    assert_eq!(
        payment["applications"],
        json!([{"document_id": "inv1", "amount": "29.00"}])
    );

    let journal = &requests_to(&server, "POST", "/api/v1/journal-entries").await[0];
    assert_eq!(journal["reference"], "paypal:capture:CAP1:fee");
    assert_eq!(journal["lines"][0]["account_code"], "8075");
    assert_eq!(journal["lines"][0]["debit"], "1.43");
    assert_eq!(journal["lines"][0]["credit"], Value::Null);
    assert_eq!(journal["lines"][1]["account_code"], "1050");
    assert_eq!(journal["lines"][1]["credit"], "1.43");
}

#[tokio::test]
async fn invoices_read_back_with_their_reference_and_a_credit_limit_refusal_is_a_conflict() {
    let server = era().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/invoices/inv1"))
        .respond_with(ok(json!({
            "invoice": invoice_row("draft", "0", "29.00"),
            "lines": [{"product_id": "p-plan", "description": "WebAgent Essential plan, monthly", "quantity": "1.0000", "unit_price": "29.0000", "line_total": "29.00"}],
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/invoices/inv1/post"))
        .respond_with(
            ResponseTemplate::new(409)
                .set_body_json(json!({"error": "credit limit exceeded for customer c1"})),
        )
        .mount(&server)
        .await;
    let books = EraBooks::connect(config(&server)).await.unwrap();
    let inv = books.invoice(&"inv1".into()).await.unwrap();
    assert_eq!(inv.status, DocumentStatus::Draft);
    assert_eq!(inv.reference.as_deref(), Some("a2a:req-7"));
    assert_eq!(inv.lines[0].quantity, 1);
    assert_eq!(inv.lines[0].unit_price, Money::usd(2_900));
    let err = books.post_invoice(&"inv1".into()).await.unwrap_err();
    assert!(
        matches!(err, BooksError::Conflict(ref m) if m.contains("credit limit")),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_expired_token_logs_in_again_once() {
    let server = era().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/customers/c1"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": "unauthorized: token expired"})),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/customers/c1"))
        .respond_with(ok(json!({
            "id": "c1", "name": "Kilele Bakery", "currency": "USD", "credit_limit": "800.00",
            "email": [{"email": "ap@kilele.example", "is_primary": true}],
            "notes": "agentrix.company: kilele-bakery",
        })))
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/invoices"))
        .respond_with(ok(
            json!({"data": [invoice_row("partially_paid", "9.00", "20.00")], "has_more": false}),
        ))
        .mount(&server)
        .await;
    let books = EraBooks::connect(config(&server)).await.unwrap();
    let credit = books.credit_position(&"c1".into()).await.unwrap();
    assert_eq!(credit.limit, Some(Money::usd(80_000)));
    assert_eq!(credit.exposure, Money::usd(2_000));
    assert_eq!(credit.available, Some(Money::usd(78_000)));
    assert_eq!(
        requests_to(&server, "POST", "/api/v1/auth/login")
            .await
            .len(),
        2,
        "connect, then once after the 401"
    );
}

#[tokio::test]
async fn ageing_reads_eras_report() {
    let server = era().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/reports"))
        .respond_with(ok(json!({
            "report_type": "ArAgeing",
            "content": {"ArAgeing": {
                "as_at": "2026-11-10",
                "lines": [{"party_id": "c1", "party_name": "Kilele Bakery", "current": "29.00", "days_1_30": "0", "days_31_60": "60.00", "days_61_90": "0", "over_90": "0", "total": "89.00"}],
                "totals": {"current": "29.00", "days_1_30": "0", "days_31_60": "60.00", "days_61_90": "0", "over_90": "0", "total": "89.00"},
            }},
        })))
        .mount(&server)
        .await;
    let books = EraBooks::connect(config(&server)).await.unwrap();
    let report = books.ar_ageing(d("2026-11-10")).await.unwrap();
    assert_eq!(report.rows[0].party, DocumentRef::Customer("c1".into()));
    assert_eq!(report.totals.days_31_60, Money::usd(6_000));
    assert_eq!(report.totals.total().unwrap(), Money::usd(8_900));
    let sent = &requests_to(&server, "POST", "/api/v1/reports").await[0];
    assert_eq!(sent["entity_id"], "e1");
    assert_eq!(sent["parameters"]["as_at"], "2026-11-10");
}

#[tokio::test]
async fn connect_needs_the_seeded_clearing_account() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(ok(
            json!({"access_token": "t1", "user": {"entity_id": "e1"}}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/bank-accounts"))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    let err = EraBooks::connect(config(&server)).await.unwrap_err();
    assert!(matches!(err, BooksError::NotFound { .. }), "{err:?}");
    let refused = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": "unauthorized: Invalid email or password"})),
        )
        .mount(&refused)
        .await;
    assert!(matches!(
        EraBooks::connect(config(&refused)).await.unwrap_err(),
        BooksError::Backend(_)
    ));
}
