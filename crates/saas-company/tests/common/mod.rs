//! A company on fakes: FakeBooks (saas-core), PayPal faked in memory, a manual clock, scripted
//! models, and a signed mandate.

#![allow(dead_code, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use adk_core::{Clock, ManualClock};
use agentrix_decisions::MandateTerms;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use saas_company::audit::SignedAudit;
use saas_company::company::{Company, CompanyConfig, ModelMix, Parts};
use saas_company::desk::RecordingOutbox;
use saas_company::identity::{Resource, StaffCredentials};
use saas_company::memory::CounterpartyMemory;
use saas_company::rails::{CaptureRecord, FakeRails, Rails};
use saas_company::scripted::ScriptedLlm;
use saas_core::books::{BillDraft, BillId, InvoiceDraft, InvoiceId, LineDraft, VendorId};
use saas_core::fake::FakeBooks;
use saas_core::{Books, CompanyId, Counterparty, Currency, Money};

pub const FOUNDER: &str = "founder@agentrix.website";

pub fn start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap()
}

pub fn day(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

/// What the seeded books hold.
#[derive(Debug, Clone)]
pub struct Seeded {
    pub vultr: VendorId,
    pub anthropic: VendorId,
    pub contractor: VendorId,
    pub vultr_bill: BillId,
    pub anthropic_bill: BillId,
    pub contractor_bill: BillId,
    pub kilele: saas_core::books::CustomerId,
    pub kilele_invoice: InvoiceId,
    pub acme: saas_core::books::CustomerId,
    pub acme_invoice: InvoiceId,
}

pub struct Harness {
    pub company: Arc<Company>,
    pub books: Arc<FakeBooks>,
    pub rails: Arc<FakeRails>,
    pub outbox: Arc<RecordingOutbox>,
    pub clock: ManualClock,
    pub audit: Arc<SignedAudit>,
    pub seeded: Seeded,
    pub models: BTreeMap<String, ScriptedLlm>,
}

fn vendor(name: &str, paypal: &str) -> Counterparty {
    let mut c = Counterparty::usd(name, Some(paypal));
    c.paypal_email = Some(paypal.to_owned());
    c
}

async fn bill(books: &FakeBooks, vendor: &VendorId, reference: &str, cents: i64) -> BillId {
    let b = books
        .draft_bill(&BillDraft {
            vendor: vendor.clone(),
            vendor_reference: Some(reference.into()),
            issue_date: day("2026-11-01"),
            due_date: day("2026-11-15"),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: reference.into(),
                quantity: 1,
                unit_price: Money::usd(cents),
            }],
            memo: None,
        })
        .await
        .unwrap();
    books.post_bill(&b.id).await.unwrap().id
}

async fn invoice(
    books: &FakeBooks,
    customer: &saas_core::books::CustomerId,
    reference: &str,
    cents: i64,
    due: &str,
) -> InvoiceId {
    let i = books
        .draft_invoice(&InvoiceDraft {
            customer: customer.clone(),
            issue_date: day("2026-10-01"),
            due_date: day(due),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: reference.into(),
                quantity: 1,
                unit_price: Money::usd(cents),
            }],
            reference: Some(reference.into()),
            memo: None,
        })
        .await
        .unwrap();
    books.submit_invoice(&i.id).await.unwrap();
    books.post_invoice(&i.id).await.unwrap().id
}

pub async fn seed(books: &FakeBooks) -> Seeded {
    let vultr = books
        .ensure_vendor(&vendor("Vultr", "billing@vultr.example"))
        .await
        .unwrap()
        .id;
    let anthropic = books
        .ensure_vendor(&vendor("Anthropic", "billing@anthropic.example"))
        .await
        .unwrap()
        .id;
    let contractor = books
        .ensure_vendor(&vendor("Wanjiru Designs", "wanjiru@designs.example"))
        .await
        .unwrap()
        .id;
    let vultr_bill = bill(books, &vultr, "VLT-881", 12_000).await;
    let anthropic_bill = bill(books, &anthropic, "ANT-2026-11", 90_000).await;
    let contractor_bill = bill(books, &contractor, "WD-14", 80_000).await;
    let mut k = Counterparty::usd("Kilele Bakery", Some("accounts@kilele.example"));
    k.company = Some(CompanyId::kilele());
    let kilele = books
        .ensure_customer(&k, Some(Money::usd(500_000)))
        .await
        .unwrap()
        .id;
    let kilele_invoice = invoice(books, &kilele, "INV-KIL-1", 120_000, "2026-11-05").await;
    let acme = books
        .ensure_customer(
            &Counterparty::usd("Acme Shop", Some("ap@acme.example")),
            Some(Money::usd(500_000)),
        )
        .await
        .unwrap()
        .id;
    let acme_invoice = invoice(books, &acme, "INV-ACM-1", 120_000, "2026-11-05").await;
    Seeded {
        vultr,
        anthropic,
        contractor,
        vultr_bill,
        anthropic_bill,
        contractor_bill,
        kilele,
        kilele_invoice,
        acme,
        acme_invoice,
    }
}

pub fn mandate(seeded: &Seeded, now: DateTime<Utc>) -> MandateTerms {
    MandateTerms {
        company: "Agentrix".into(),
        currency: "USD".into(),
        refund_max_each_minor: 5_000,
        payout_max_each_minor: 50_000,
        payout_max_per_day_minor: 150_000,
        payees: vec![
            seeded.vultr.to_string(),
            seeded.anthropic.to_string(),
            seeded.contractor.to_string(),
        ],
        debt_plan_max_instalments: 4,
        debt_plan_max_days: 120,
        inference_max_per_day_minor: 2_000,
        plans: vec!["business".into(), "essential".into()],
        suspend_on_failed_payment: true,
        valid_until: now + chrono::Duration::days(30),
    }
}

pub async fn agentrix(seed_value: u64) -> Harness {
    agentrix_wrapped(seed_value, |rails| rails).await
}

/// Agentrix with its PayPal rails wrapped (a crash in the middle of a payout, say).
pub async fn agentrix_wrapped(
    seed_value: u64,
    wrap: impl FnOnce(Arc<FakeRails>) -> Arc<dyn Rails>,
) -> Harness {
    let books = Arc::new(FakeBooks::new(CompanyId::agentrix()));
    let seeded = seed(&books).await;
    let clock = ManualClock::new(start());
    let rails = Arc::new(
        FakeRails::new()
            .with_clock(Arc::new(clock.clone()))
            .with_capture(CaptureRecord {
                capture_id: "3C679366HH908993F".into(),
                payer_email: Some("ana@customer.example".into()),
                currency: "USD".into(),
                amount_minor: 4_900,
                refunded_minor: 0,
                subscription: Some("I-BUSINESS0001".into()),
            }),
    );
    let outbox = Arc::new(RecordingOutbox::default());
    let audit = Arc::new(SignedAudit::seeded("agentrix", seed_value).await.unwrap());
    let creds = StaffCredentials::in_memory(
        "agentrix",
        FOUNDER,
        BTreeMap::from([
            (Resource::Books, "era-agentrix-key".to_owned()),
            (Resource::PayPal, "paypal-agentrix-key".to_owned()),
            (Resource::AgentDoor, "door-agentrix-key".to_owned()),
        ]),
    )
    .unwrap();
    let mut models = BTreeMap::new();
    let mut mix = ModelMix::all(Arc::new(ScriptedLlm::seeded("unused", seed_value)));
    for role in saas_company::identity::ROLES {
        let m = ScriptedLlm::seeded(format!("scripted-{role}"), seed_value);
        mix = mix.with(role, Arc::new(m.clone()));
        models.insert(role.to_owned(), m);
    }
    let company = Company::assemble(
        CompanyConfig::agentrix(FOUNDER),
        Parts {
            books: books.clone(),
            rails: wrap(rails.clone()),
            settlement: rails.clone(),
            outbox: outbox.clone(),
            clock: Arc::new(clock.clone()) as Arc<dyn Clock>,
            creds,
            memory: CounterpartyMemory::in_memory("agentrix"),
            audit: audit.clone(),
            models: mix,
        },
    )
    .unwrap();
    company
        .governance
        .set_mandate(Some(mandate(&seeded, start())), start());
    Harness {
        company: Arc::new(company),
        books,
        rails,
        outbox,
        clock,
        audit,
        seeded,
        models,
    }
}

impl Harness {
    pub fn model(&self, role: &str) -> &ScriptedLlm {
        &self.models[role]
    }
}

/// Kilele Bakery: its own books (Agentrix as a vendor, with Agentrix's annual-plan invoice as a
/// bill), its own policy, identities and signed log; `settlement` is the agent-to-agent rail.
pub struct Kilele {
    pub company: Arc<Company>,
    pub books: Arc<FakeBooks>,
    pub audit: Arc<SignedAudit>,
    pub agentrix_vendor: VendorId,
    pub bill: BillId,
    pub payables: ScriptedLlm,
}

pub async fn kilele(
    seed_value: u64,
    clock: ManualClock,
    settlement: Arc<dyn saas_company::rails::Settlement>,
) -> Kilele {
    let founder = "wanjiku@kilele.example";
    let books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let mut agentrix = Counterparty::usd("Agentrix", Some("billing@agentrix.website"));
    agentrix.company = Some(CompanyId::agentrix());
    agentrix.paypal_email = Some("billing@agentrix.website".into());
    let agentrix_vendor = books.ensure_vendor(&agentrix).await.unwrap().id;
    let bill = bill(&books, &agentrix_vendor, "INV-KIL-1", 120_000).await;
    let audit = Arc::new(SignedAudit::seeded("kilele", seed_value).await.unwrap());
    let payables = ScriptedLlm::seeded("kilele-payables", seed_value);
    let models = ModelMix::all(Arc::new(ScriptedLlm::seeded("kilele", seed_value)))
        .with(saas_company::identity::PAYABLES, Arc::new(payables.clone()));
    let creds = StaffCredentials::in_memory(
        "kilele",
        founder,
        BTreeMap::from([
            (Resource::Books, "era-kilele-key".to_owned()),
            (Resource::PayPal, "paypal-kilele-key".to_owned()),
            (Resource::AgentDoor, "door-kilele-key".to_owned()),
        ]),
    )
    .unwrap();
    let company = Company::assemble(
        CompanyConfig::kilele(founder),
        Parts {
            books: books.clone(),
            rails: Arc::new(FakeRails::new().with_clock(Arc::new(clock.clone()))),
            settlement,
            outbox: Arc::new(RecordingOutbox::default()),
            clock: Arc::new(clock.clone()) as Arc<dyn Clock>,
            creds,
            memory: CounterpartyMemory::in_memory("kilele"),
            audit: audit.clone(),
            models,
        },
    )
    .unwrap();
    let mut terms = mandate_for("Kilele Bakery", vec![agentrix_vendor.to_string()], start());
    terms.payout_max_each_minor = 200_000;
    terms.payout_max_per_day_minor = 300_000;
    company.governance.set_mandate(Some(terms), start());
    Kilele {
        company: Arc::new(company),
        books,
        audit,
        agentrix_vendor,
        bill,
        payables,
    }
}

pub fn mandate_for(company: &str, payees: Vec<String>, now: DateTime<Utc>) -> MandateTerms {
    MandateTerms {
        company: company.into(),
        currency: "USD".into(),
        refund_max_each_minor: 5_000,
        payout_max_each_minor: 50_000,
        payout_max_per_day_minor: 150_000,
        payees,
        debt_plan_max_instalments: 4,
        debt_plan_max_days: 120,
        inference_max_per_day_minor: 2_000,
        plans: vec![],
        suspend_on_failed_payment: false,
        valid_until: now + chrono::Duration::days(30),
    }
}
