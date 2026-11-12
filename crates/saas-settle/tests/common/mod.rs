//! Two companies on one machine: Agentrix (the payee) and Kilele Bakery (the payer), each with
//! its own keys, store, `FakeBooks` and agent door on a local port, a mock PayPal (wiremock),
//! and owner mandates sealed after a (test) passkey approval.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration as StdDuration;

use adk_auth::web_bot_auth::{InMemoryKeyDirectorySource, KeyDirectorySource};
use agentrix_effects::Effects;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use agentrix_host::Approvals;
use agentrix_types::{Money, SiteId};
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use saas_core::books::{InvoiceDraft, LineDraft};
use saas_core::fake::FakeBooks;
use saas_core::{Books, CompanyId, Counterparty, Currency};
use saas_settle::era::{EraPayee, EraPayer, KnownCompany};
use saas_settle::keys::{PAYABLES, RECEIVABLES};
use saas_settle::mandate::{PASSKEY_METHOD, seal_owner_approved};
use saas_settle::payee::Clock;
use saas_settle::rails::{PayoutRail, VaultRail};
use saas_settle::{
    BoundsNegotiator, CollectionsMandate, CompanyKeys, Door, DoorAuth, DoorClient,
    InstalmentBounds, IntentMandate, Negotiator, PayeeConfig, PayeeDesk, PayerConfig, PayerDesk,
    Route, SealKind, Sealed, Store,
};
use serde_json::{Value, json};
use url::Url;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub const AGENTRIX: &str = "https://pay.agentrix.example";
pub const KILELE: &str = "https://kilele.agentrix.example";
pub const AGENTRIX_PAYPAL: &str = "agentrix-saas-merchant@example.com";
pub const VAULT_ID: &str = "VAULT-KILELE-1";

pub fn usd(minor: i64) -> Money {
    Money {
        currency: "USD".into(),
        minor,
    }
}

/// A passkey ceremony that always verifies, recording what it approved. Stands in for node's
/// WebAuthn check (agentrix-host `Approvals`).
pub struct TestPasskey;

#[async_trait]
impl Approvals for TestPasskey {
    async fn passkey_options(
        &self,
        _account: Uuid,
        fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<Value>> {
        Ok(Some(json!({ "challenge": hex::encode(fingerprint) })))
    }

    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        _request: Value,
    ) -> anyhow::Result<Value> {
        Ok(json!({
            "method": PASSKEY_METHOD,
            "account": account.to_string(),
            "fingerprint": hex::encode(fingerprint),
            "credential_id": "test-credential",
        }))
    }
}

/// PayPal's Orders v2 for a saved wallet: a COMPLETED order with one capture for the amount
/// asked, answered once per `PayPal-Request-Id` (a repeat gets the first answer).
pub struct Orders {
    pub posts: AtomicUsize,
    /// Answer this many POSTs with a 503 first (the outcome unknown to the caller).
    pub fail_first: AtomicUsize,
    seen: std::sync::Mutex<HashMap<String, Value>>,
}

impl Orders {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            posts: AtomicUsize::new(0),
            fail_first: AtomicUsize::new(0),
            seen: std::sync::Mutex::new(HashMap::new()),
        })
    }
}

/// Answers Orders v2 for [`Orders`].
pub struct OrdersResponder(pub Arc<Orders>);

impl Respond for OrdersResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.answer(request)
    }
}

impl Orders {
    fn answer(&self, request: &Request) -> ResponseTemplate {
        let key = request
            .headers
            .get("paypal-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if self
            .fail_first
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return ResponseTemplate::new(503)
                .set_body_json(json!({"name": "SERVICE_UNAVAILABLE"}));
        }
        let mut seen = self.seen.lock().unwrap();
        if let Some(answer) = seen.get(&key) {
            return ResponseTemplate::new(201).set_body_json(answer.clone());
        }
        let n = self.posts.fetch_add(1, Ordering::SeqCst) + 1;
        let body: Value = request.body_json().unwrap();
        assert_eq!(
            body["payment_source"]["paypal"]["vault_id"], VAULT_ID,
            "a merchant-initiated order on the saved wallet"
        );
        let unit = &body["purchase_units"][0];
        let answer = json!({
            "id": format!("ORDER-{n}"),
            "status": "COMPLETED",
            "purchase_units": [{
                "reference_id": unit["reference_id"],
                "invoice_id": unit["invoice_id"],
                "payments": {"captures": [{
                    "id": format!("CAP-{n}"),
                    "status": "COMPLETED",
                    "amount": unit["amount"],
                    "seller_receivable_breakdown": {
                        "gross_amount": unit["amount"],
                        "paypal_fee": {"currency_code": "USD", "value": "3.50"},
                    },
                }]},
            }],
        });
        seen.insert(key, answer.clone());
        ResponseTemplate::new(201).set_body_json(answer)
    }
}

pub async fn paypal() -> (MockServer, Arc<Orders>) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/oauth2/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "access_token": "A21AA-test", "expires_in": 32400 })),
        )
        .mount(&server)
        .await;
    let orders = Orders::new();
    Mock::given(method("POST"))
        .and(path("/v2/checkout/orders"))
        .respond_with(OrdersResponder(orders.clone()))
        .mount(&server)
        .await;
    (server, orders)
}

pub fn client(server: &MockServer) -> Arc<PayPalClient> {
    Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: "id".into(),
            client_secret: "secret".into(),
            webhook_id: None,
            environment: Environment::Custom(server.uri()),
        },
        reqwest::Client::new(),
    ))
}

/// How many requests the mock PayPal saw on `path`.
pub async fn calls(server: &MockServer, on: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == on)
        .count()
}

async fn store() -> Store {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    saas_settle::migrate(&pool).await.unwrap();
    Store::new(pool)
}

/// The payee's books, whose first post may fail once.
pub struct FlakyPayee {
    inner: EraPayee,
    fail: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl saas_settle::books::PayeeBooks for FlakyPayee {
    async fn books_id(&self, invoice_id: &str) -> anyhow::Result<Option<String>> {
        self.inner.books_id(invoice_id).await
    }

    async fn invoice(&self, invoice_id: &str) -> anyhow::Result<Option<saas_settle::Invoice>> {
        self.inner.invoice(invoice_id).await
    }

    async fn post_received(
        &self,
        receipt: &saas_settle::SettlementReceipt,
    ) -> anyhow::Result<saas_settle::books::PostedRef> {
        if self.fail.swap(false, Ordering::SeqCst) {
            anyhow::bail!("ERA is down");
        }
        self.inner.post_received(receipt).await
    }
}

/// Settings a test may change.
pub struct Options {
    /// The payee's first books post fails (the money moved; the books lag).
    pub payee_post_fails_once: bool,
    pub route: Route,
    pub negotiator: Arc<dyn Negotiator>,
    pub invoice: i64,
    pub cash_available: Option<Money>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            payee_post_fails_once: false,
            route: Route::Vault,
            negotiator: Arc::new(BoundsNegotiator),
            invoice: 120_000,
            cash_available: None,
        }
    }
}

pub struct World {
    pub paypal: MockServer,
    pub orders: Arc<Orders>,
    pub directories: Arc<InMemoryKeyDirectorySource>,
    pub agentrix: CompanyKeys,
    pub kilele: CompanyKeys,
    pub agentrix_books: Arc<FakeBooks>,
    pub kilele_books: Arc<FakeBooks>,
    pub payee: Arc<PayeeDesk>,
    pub payer: Arc<PayerDesk>,
    pub agentrix_door: Url,
    pub kilele_door: Url,
    pub intent: Sealed,
    pub intent_terms: IntentMandate,
    pub invoice_id: String,
    pub offset_days: Arc<AtomicI64>,
    /// The agent tokens Agentrix's door accepts (issued for `AGENTRIX`).
    pub tokens: Arc<saas_settle::tokens::FixedTokens>,
}

impl World {
    /// Moves every desk's clock `days` ahead (request signatures keep real time).
    pub fn advance(&self, days: i64) {
        self.offset_days.fetch_add(days, Ordering::SeqCst);
    }

    pub fn today(&self) -> NaiveDate {
        Utc::now().date_naive() + Duration::days(self.offset_days.load(Ordering::SeqCst))
    }

    /// An agent client to Agentrix's door, signing as Kilele with `key`.
    pub fn kilele_client(&self, key: Arc<adk_auth::web_bot_auth::Ed25519SigningKey>) -> DoorClient {
        DoorClient::new(
            reqwest::Client::new(),
            self.agentrix_door.clone(),
            key,
            KILELE,
        )
        .unwrap()
    }

    /// Raises and posts another invoice to Kilele in Agentrix's books.
    pub async fn invoice(&self, minor: i64, description: &str) -> String {
        raise_invoice(&self.agentrix_books, minor, description, self.today()).await
    }
}

pub async fn raise_invoice(
    books: &FakeBooks,
    minor: i64,
    description: &str,
    today: NaiveDate,
) -> String {
    let customer = books
        .ensure_customer(
            &Counterparty {
                name: "Kilele Bakery".into(),
                email: Some("accounts@kilele.example".into()),
                currency: Currency::USD,
                company: Some(CompanyId::kilele()),
                paypal_email: None,
            },
            None,
        )
        .await
        .unwrap();
    let draft = books
        .draft_invoice(&InvoiceDraft {
            customer: customer.id,
            issue_date: today,
            due_date: today + Duration::days(14),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: description.into(),
                quantity: 1,
                unit_price: saas_core::Money::usd(minor),
            }],
            reference: None,
            memo: None,
        })
        .await
        .unwrap();
    books.post_invoice(&draft.id).await.unwrap().number.unwrap()
}

async fn listener() -> (tokio::net::TcpListener, Url) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    (listener, url)
}

/// Kilele's own Effects gateway for the Payout route, over the same mock PayPal.
pub async fn kilele_effects(server: &MockServer) -> Effects {
    let ctx = agentrix_host::testing::context().await.unwrap();
    agentrix_effects::migrate(&ctx.pool).await.unwrap();
    Effects::new(
        ctx.pool.clone(),
        Some(client(server)),
        Arc::new(agentrix_host::testing::MemoryReceipts::default()),
    )
}

pub async fn world(options: Options) -> World {
    let (paypal, orders) = paypal().await;
    let agentrix = CompanyKeys::generate(AGENTRIX, &[RECEIVABLES]).unwrap();
    let kilele = CompanyKeys::generate(KILELE, &[PAYABLES]).unwrap();
    let directories = Arc::new(InMemoryKeyDirectorySource::new());
    directories
        .insert(AGENTRIX, agentrix.directory(), None)
        .await;
    directories.insert(KILELE, kilele.directory(), None).await;
    let source: Arc<dyn KeyDirectorySource> = directories.clone();

    let offset_days = Arc::new(AtomicI64::new(0));
    let clock: Clock = {
        let offset = offset_days.clone();
        Arc::new(move || Utc::now() + Duration::days(offset.load(Ordering::SeqCst)))
    };
    let now = Utc::now();

    // The owners' mandates, each approved with a passkey and sealed by the company's owner key.
    let intent_terms = IntentMandate {
        mandate_id: "im_kilele_1".into(),
        natural_language_description:
            "Pay Agentrix's WebAgent invoices: up to $1,500 each and $2,000 a month; instalments up to 4 within 90 days."
                .into(),
        payer: KILELE.into(),
        agent_keyid: kilele.agent(PAYABLES).unwrap().thumbprint().into(),
        payees: vec![AGENTRIX.into()],
        max_per_invoice: usd(150_000),
        max_per_month: usd(200_000),
        instalments: InstalmentBounds {
            max_count: 4,
            max_span_days: 90,
            min_first_percent: 20,
        },
        not_before: now - Duration::hours(1),
        intent_expiry: now + Duration::days(365),
    };
    let owner = Uuid::new_v4();
    let intent = seal_owner_approved(
        SealKind::IntentMandate,
        &intent_terms,
        &TestPasskey,
        owner,
        json!({"method": "passkey", "credential": {}}),
        &kilele,
    )
    .await
    .unwrap();
    let collections_terms = CollectionsMandate {
        mandate_id: "cm_agentrix_1".into(),
        natural_language_description:
            "Give Kilele Bakery terms up to $2,000 owed: at most 3 instalments within 60 days, the first at least 25%."
                .into(),
        payee: AGENTRIX.into(),
        counterparties: vec![KILELE.into()],
        credit_limit: usd(200_000),
        instalments: InstalmentBounds {
            max_count: 3,
            max_span_days: 60,
            min_first_percent: 25,
        },
        expiry: now + Duration::days(365),
    };
    let collections = seal_owner_approved(
        SealKind::CollectionsMandate,
        &collections_terms,
        &TestPasskey,
        Uuid::new_v4(),
        json!({"method": "passkey", "credential": {}}),
        &agentrix,
    )
    .await
    .unwrap();

    let agentrix_books = Arc::new(FakeBooks::new(CompanyId::agentrix()));
    let kilele_books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let invoice_id = raise_invoice(
        &agentrix_books,
        options.invoice,
        "WebAgent Pro, annual plan",
        now.date_naive(),
    )
    .await;
    let known_agentrix = KnownCompany {
        company: CompanyId::agentrix(),
        name: "Agentrix".into(),
        origin: AGENTRIX.into(),
        paypal_email: Some(AGENTRIX_PAYPAL.into()),
    };
    let known_kilele = KnownCompany {
        company: CompanyId::kilele(),
        name: "Kilele Bakery".into(),
        origin: KILELE.into(),
        paypal_email: None,
    };

    let (agentrix_listener, agentrix_door) = listener().await;
    let (kilele_listener, kilele_door) = listener().await;
    let methods = vec![PASSKEY_METHOD.to_owned()];

    let agentrix_store = store().await;
    let payee = PayeeDesk::new(PayeeConfig {
        name: "Agentrix".into(),
        keys: agentrix.clone(),
        directories: source.clone(),
        store: agentrix_store.clone(),
        books: Arc::new(FlakyPayee {
            inner: EraPayee::new(
                agentrix_books.clone(),
                known_agentrix.clone(),
                vec![known_kilele.clone()],
            ),
            fail: std::sync::atomic::AtomicBool::new(options.payee_post_fails_once),
        }),
        vault: Some(VaultRail::new(client(&paypal))),
        paypal_email: Some(AGENTRIX_PAYPAL.into()),
        collections,
        owner_methods: methods.clone(),
        negotiator: options.negotiator,
        payer_doors: HashMap::from([(
            KILELE.to_owned(),
            DoorClient::new(
                reqwest::Client::new(),
                kilele_door.clone(),
                agentrix.agent(RECEIVABLES).unwrap(),
                AGENTRIX,
            )
            .unwrap(),
        )]),
        clock: clock.clone(),
    })
    .await
    .unwrap();
    payee.use_vault(KILELE, VAULT_ID).await.unwrap();

    let kilele_store = store().await;
    let payout = match options.route {
        Route::Payout => Some(PayoutRail::new(
            kilele_effects(&paypal).await,
            SiteId::new("site_kilele_bakery"),
            StdDuration::from_secs(10),
        )),
        Route::Vault => None,
    };
    let payer = PayerDesk::new(PayerConfig {
        name: "Kilele Bakery".into(),
        keys: kilele.clone(),
        directories: source.clone(),
        store: kilele_store.clone(),
        books: Arc::new(EraPayer::new(kilele_books.clone(), vec![known_agentrix])),
        intent: intent.clone(),
        owner_methods: methods,
        payee_doors: HashMap::from([(
            AGENTRIX.to_owned(),
            DoorClient::new(
                reqwest::Client::new(),
                agentrix_door.clone(),
                kilele.agent(PAYABLES).unwrap(),
                KILELE,
            )
            .unwrap(),
        )]),
        route: options.route,
        payout,
        payee_emails: HashMap::from([(AGENTRIX.to_owned(), AGENTRIX_PAYPAL.to_owned())]),
        cash_available: options.cash_available,
        clock,
    })
    .await
    .unwrap();

    let agentrix_door_handle = Door::new(
        "Agentrix",
        agentrix.clone(),
        DoorAuth::new(source.clone(), agentrix_store, [KILELE.to_owned()]),
        payee.clone(),
        Some(payee.events()),
    );
    let tokens = Arc::new(saas_settle::tokens::FixedTokens::default());
    assert!(agentrix_door_handle.accept_tokens(tokens.clone(), AGENTRIX));
    let agentrix_router = agentrix_door_handle.router();
    let kilele_router = Door::new(
        "Kilele Bakery",
        kilele.clone(),
        DoorAuth::new(source.clone(), kilele_store, [AGENTRIX.to_owned()]),
        payer.clone(),
        Some(payer.events()),
    )
    .router();
    tokio::spawn(async move {
        axum::serve(agentrix_listener, agentrix_router)
            .await
            .unwrap();
    });
    tokio::spawn(async move {
        axum::serve(kilele_listener, kilele_router).await.unwrap();
    });

    World {
        paypal,
        orders,
        directories,
        agentrix,
        kilele,
        agentrix_books,
        kilele_books,
        payee,
        payer,
        agentrix_door,
        kilele_door,
        intent,
        intent_terms,
        invoice_id,
        offset_days,
        tokens,
    }
}

/// The payee's open balance on an invoice, and the payer's on the matching bill.
pub async fn balances(world: &World, invoice_id: &str) -> (i64, i64) {
    let invoices = world
        .agentrix_books
        .invoices(&saas_core::books::DocumentFilter::default())
        .await
        .unwrap();
    let invoice = invoices
        .iter()
        .find(|i| i.number.as_deref() == Some(invoice_id))
        .unwrap();
    let bill = saas_settle::era::bill_of(world.kilele_books.as_ref(), invoice_id)
        .await
        .unwrap()
        .map(|(_, open)| open.minor)
        .unwrap_or(-1);
    (invoice.balance_due.minor, bill)
}

/// The payments each side's books hold under a PayPal key.
pub async fn payments(books: &FakeBooks, key: &str) -> usize {
    books
        .payments(&saas_core::books::PaymentFilter {
            reference: Some(key.into()),
            direction: None,
        })
        .await
        .unwrap()
        .len()
}

/// A cart mandate Kilele's Payables agent signs for `invoice_id` (for the raw-message tests).
pub fn cart(world: &World, invoice_id: &str, minor: i64, due: NaiveDate) -> Sealed {
    let now = Utc::now();
    let terms = saas_settle::CartMandate {
        cart_mandate_id: format!("cart_{}", Uuid::now_v7().simple()),
        intent_mandate_id: world.intent_terms.mandate_id.clone(),
        intent_digest: world.intent.digest().unwrap().to_string(),
        invoice_id: invoice_id.into(),
        payer: KILELE.into(),
        payee: AGENTRIX.into(),
        amount: usd(minor),
        due_date: due,
        instalment: None,
        created_at: now,
        expires_at: now + Duration::days(7),
    };
    Sealed::seal(
        SealKind::CartMandate,
        &terms,
        KILELE,
        &world.kilele.agent(PAYABLES).unwrap(),
    )
    .unwrap()
}

pub fn later(days: i64) -> DateTime<Utc> {
    Utc::now() + Duration::days(days)
}
