//! The feed read from saas-core's books: the story seeded into both companies' `FakeBooks`
//! through the `Books` contract, then served by the face as `/feed.json`.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, header};
use chrono::NaiveDate;
use saas_core::Books;
use saas_core::books::PaymentFilter;
use saas_web::Face;
use saas_web::demo::story;
use saas_web::feed::Feed;
use saas_web::hosts::Hosts;
use tower::ServiceExt;

fn as_of() -> NaiveDate {
    "2026-11-10".parse().unwrap()
}

async fn feed(app: &Router, host: &str) -> Feed {
    let res = app
        .clone()
        .oneshot(
            Request::get("/feed.json")
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.status().is_success());
    let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn app() -> Router {
    Face::fakebooks(Hosts::default(), as_of())
        .await
        .unwrap()
        .with_today(Arc::new(as_of))
        .with_board_dist(None)
        .router()
        .unwrap()
}

#[tokio::test]
async fn the_books_answer_what_the_story_says_is_owed() {
    let f = feed(&app().await, "money.agentrix.website").await;
    let s = story(as_of());
    assert_eq!(f.source, "fakebooks");
    for company in ["agentrix", "kilele"] {
        let owed = |rows: &[saas_web::feed::Receivable]| {
            rows.iter()
                .filter(|r| r.company == company)
                .map(|r| r.open_minor)
                .sum::<i64>()
        };
        assert_eq!(
            owed(&f.receivables),
            owed(&s.receivables),
            "{company} receivables"
        );
        let aged: i64 = f
            .ageing
            .iter()
            .filter(|a| a.company == company)
            .map(|a| a.total_minor)
            .sum();
        assert_eq!(
            aged,
            owed(&s.receivables),
            "{company}: ERA's ageing adds up to what is owed"
        );
        for a in f.ageing.iter().filter(|a| a.company == company) {
            let mine: i64 = f
                .receivables
                .iter()
                .filter(|r| r.company == company && r.customer == a.customer)
                .map(|r| r.open_minor)
                .sum();
            assert_eq!(a.total_minor, mine, "{}", a.customer);
        }
    }
    // Kilele's credit limit comes from the books.
    let kilele = f
        .ageing
        .iter()
        .find(|a| a.company == "agentrix" && a.customer == "Kilele Bakery")
        .unwrap();
    assert_eq!(kilele.credit_limit_minor, Some(150_000));
    assert_eq!(kilele.d1_30_minor, 50_560);
}

#[tokio::test]
async fn the_plan_names_the_books_invoice_and_both_ledgers_share_one_paypal_id() {
    let seeded = saas_web::seed::seed(as_of()).await.unwrap();
    let f = feed(&app().await, "money.agentrix.website").await;
    let plan = &f.debt_plans[0];
    assert!(
        plan.invoice_number.starts_with("INV-0"),
        "the books' number: {}",
        plan.invoice_number
    );
    assert_eq!(plan.invoice_number, seeded.numbers.kilele_annual);

    let receivable = f
        .receivables
        .iter()
        .find(|r| r.number == plan.invoice_number)
        .unwrap();
    assert_eq!(receivable.customer, "Kilele Bakery");
    assert_eq!(receivable.status, "on_plan");
    assert_eq!(
        receivable.open_minor,
        plan.principal_minor - plan.paid_minor
    );
    assert_eq!(receivable.paypal_id.as_deref(), Some("DEMO-2PF93355"));

    let bill = f
        .payables
        .iter()
        .find(|p| p.company == "kilele" && p.number == plan.invoice_number)
        .unwrap();
    assert_eq!(bill.vendor, "Agentrix");
    assert_eq!(
        bill.open_minor, receivable.open_minor,
        "what Kilele owes is what Agentrix is owed"
    );
    assert_eq!(bill.status, "on plan");

    // One PayPal transaction, posted in both companies' books.
    let key = "paypal:capture:DEMO-2PF93355";
    let filter = PaymentFilter {
        reference: Some(key.into()),
        direction: None,
    };
    assert_eq!(seeded.agentrix.payments(&filter).await.unwrap().len(), 1);
    assert_eq!(seeded.kilele.payments(&filter).await.unwrap().len(), 1);
    assert!(
        f.settlements.iter().any(|s| s.paypal_id == "DEMO-2PF93355"
            && s.invoice_number.starts_with(&plan.invoice_number))
    );
}

#[tokio::test]
async fn cash_bills_and_mrr_come_from_the_books() {
    let f = feed(&app().await, "money.agentrix.website").await;
    let paypal = |company: &str| {
        f.cash
            .iter()
            .find(|c| c.company == company && c.rail == "PayPal")
            .unwrap()
    };
    // Received 617.80 (instalment 1, five paid invoices), paid out 44.10 (Vultr, September).
    assert_eq!(paypal("agentrix").balance_minor, 57_370);
    assert_eq!(paypal("agentrix").account, "PayPal clearing");
    // Received 1,530.00 from cafes, paid Agentrix 252.80 and 79.00.
    assert_eq!(paypal("kilele").balance_minor, 119_820);

    let held = f.payables.iter().find(|p| p.number == "AMN-0007").unwrap();
    assert_eq!((held.open_minor, held.mandate.as_str()), (36_000, "over"));
    let vultr = f
        .payables
        .iter()
        .find(|p| p.number == "VLT-2026-09")
        .unwrap();
    assert_eq!((vultr.open_minor, vultr.status.as_str()), (0, "paid"));

    // Six subscriptions: 79 + 29 + 29 + 149 + 29 a month, and 758.40 a year.
    assert_eq!(f.mrr.len(), 1);
    assert_eq!(f.mrr[0].mrr_minor, 37_820);
    assert_eq!(f.mrr[0].customers, 6);
}

#[tokio::test]
async fn a_bill_on_a_plan_is_due_as_the_plan_says() {
    let app = app().await;
    let res = app
        .clone()
        .oneshot(
            Request::get("/")
                .header(header::HOST, "kilele.money.agentrix.website")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&bytes);
    // Unga Millers' $420.00 is due in 6 days; instalment 2 of the plan only in 27.
    assert!(html.contains("$420.00"), "{html}");
    assert!(
        !html.contains("$925.60"),
        "the whole plan isn't due this week"
    );
}

#[tokio::test]
async fn kilele_sees_its_own_books() {
    let f = feed(&app().await, "kilele.money.agentrix.website").await;
    assert!(f.receivables.iter().all(|r| r.company == "kilele"));
    assert!(
        f.payables
            .iter()
            .any(|p| p.vendor == "Agentrix" && p.status == "on plan")
    );
    assert!(
        f.payables
            .iter()
            .any(|p| p.vendor == "Unga Millers" && p.mandate == "over")
    );
}
