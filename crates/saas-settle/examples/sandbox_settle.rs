//! Agentrix settles an invoice with Kilele Bakery on PayPal's **sandbox**, end to end on this
//! machine: both companies' agent doors on 127.0.0.1:18182 (`agentrix.localhost` and
//! `kilele.localhost`), each fetching the other's key directory over HTTP, every A2A request
//! signed with Web Bot Auth, both mandates checked before any money moves.
//!
//! ```text
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- vault-start
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- vault-token <setup token>
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- vault-finish
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- run vault [cents]
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- run payout [cents]
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- instalments [cents]
//! scripts/wcargo run -p saas-settle --example sandbox_settle -- status
//! ```
//!
//! - Keys: Agentrix's app from `PAYPAL_CLIENT_*` (Vault capture), Kilele's from
//!   `KILELE_PAYPAL_CLIENT_*` (Payout fallback), read from the repo's `.env` line by line and
//!   never printed. It refuses any `PAYPAL_ENV` but `sandbox`.
//! - State: `~/.config/agentrix-saas/settle/` (mode 0700): each company's agent keys (0600),
//!   its settlement store, Kilele's payout journal, and `evidence.jsonl`.
//! - Mandates: no phone is at hand here, so each owner's mandate is approved with the operator's
//!   owner key file (`local-owner-key`), which these desks accept by name for this run only. On
//!   the node the owner approves with a passkey and only `passkey` is accepted.
//! - Books: both companies' books are saas-core `FakeBooks` here, printed after the run; on the
//!   node the same desks post to Zavora ERA through saas-books.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use agentrix_effects::Effects;
use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use agentrix_types::{Money, SiteId};
use axum::extract::Request;
use axum::http::header::HOST;
use chrono::{Duration, Utc};
use saas_core::books::{DocumentFilter, InvoiceDraft, LineDraft, PaymentFilter};
use saas_core::fake::FakeBooks;
use saas_core::{Books, CompanyId, Counterparty, Currency};
use saas_settle::era::{EraPayee, EraPayer, KnownCompany, bill_of};
use saas_settle::keys::{PAYABLES, RECEIVABLES, loopback_client};
use saas_settle::mandate::{LOCAL_OWNER_METHOD, PASSKEY_METHOD, seal_locally_approved};
use saas_settle::payee::Clock;
use saas_settle::rails::{PayoutRail, VaultRail};
use saas_settle::{
    BoundsNegotiator, CollectionsMandate, CompanyKeys, Door, DoorAnswer, DoorAuth, DoorClient,
    InstalmentBounds, IntentMandate, LoopbackDirectorySource, PayeeConfig, PayeeDesk, PayerConfig,
    PayerDesk, Route, SealKind, SettlementReceipt, Store,
};
use serde_json::json;
use tower::ServiceExt;

const PORT: u16 = 18182;
const AGENTRIX: &str = "http://agentrix.localhost:18182";
const KILELE: &str = "http://kilele.localhost:18182";

fn env_file() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env")
}

/// The repo's `.env`, line by line; a value starting with `#` is empty.
fn read_env() -> anyhow::Result<HashMap<String, String>> {
    let mut path = env_file();
    if !path.exists() {
        path = dirs_home().join("Developer/hackathons/agentrix-saas/.env");
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("can't read {}: {e}", path.display()))?;
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let mut v = v.trim().to_owned();
        if v.len() >= 2
            && (v.starts_with('"') && v.ends_with('"') || v.starts_with('\'') && v.ends_with('\''))
        {
            v = v[1..v.len() - 1].to_owned();
        }
        if v.starts_with('#') {
            v.clear();
        }
        out.insert(k.trim().to_owned(), v);
    }
    Ok(out)
}

fn dirs_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn state_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs_home().join(".config/agentrix-saas/settle");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

fn paypal(
    env: &HashMap<String, String>,
    id: &str,
    secret: &str,
) -> anyhow::Result<Arc<PayPalClient>> {
    anyhow::ensure!(
        env.get("PAYPAL_ENV").map(String::as_str) == Some("sandbox"),
        "PAYPAL_ENV must be sandbox"
    );
    let client_id = env.get(id).filter(|v| !v.is_empty());
    let client_secret = env.get(secret).filter(|v| !v.is_empty());
    let (Some(client_id), Some(client_secret)) = (client_id, client_secret) else {
        anyhow::bail!("{id} and {secret} must be set in .env");
    };
    Ok(Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            webhook_id: None,
            environment: Environment::Sandbox,
        },
        reqwest::Client::new(),
    )))
}

async fn sqlite(path: &Path) -> anyhow::Result<sqlx::SqlitePool> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    Ok(sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

fn usd(minor: i64) -> Money {
    Money {
        currency: "USD".into(),
        minor,
    }
}

struct Node {
    payee: Arc<PayeeDesk>,
    payer: Arc<PayerDesk>,
    agentrix_books: Arc<FakeBooks>,
    kilele_books: Arc<FakeBooks>,
    agentrix_store: Store,
}

async fn node(route: Route, cash_available: Option<Money>, per_run: bool) -> anyhow::Result<Node> {
    let env = read_env()?;
    let dir = state_dir()?;
    let agentrix =
        CompanyKeys::load_or_create(AGENTRIX, &[RECEIVABLES], &dir.join("keys/agentrix"))?;
    let kilele = CompanyKeys::load_or_create(KILELE, &[PAYABLES], &dir.join("keys/kilele"))?;
    let directories = Arc::new(LoopbackDirectorySource::new());
    let http = loopback_client(&[AGENTRIX, KILELE])?;

    // Agentrix's saved-wallet record persists in agentrix.db. The books here are re-seeded
    // each run (invoice numbers restart at INV-0001), so a settlement run keeps both companies'
    // settlement records in fresh per-run files, with the saved wallet copied in.
    let wallet_store = Store::new(sqlite(&dir.join("agentrix.db")).await?);
    saas_settle::migrate(wallet_store.pool()).await?;
    std::fs::create_dir_all(dir.join("runs"))?;
    let run = Utc::now().format("%Y%m%dT%H%M%S");
    let agentrix_store = if per_run {
        let store = Store::new(sqlite(&dir.join(format!("runs/agentrix-{run}.db"))).await?);
        saas_settle::migrate(store.pool()).await?;
        if let Some(row) = wallet_store.vault(KILELE).await? {
            store.put_vault(&row).await?;
        }
        store
    } else {
        wallet_store
    };
    let kilele_store = Store::new(sqlite(&dir.join(format!("runs/kilele-{run}.db"))).await?);
    saas_settle::migrate(kilele_store.pool()).await?;

    // Serve both doors first: each desk checks its owner's mandate against its own directory.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await?;
    let (agentrix_tx, agentrix_rx) = tokio::sync::oneshot::channel::<axum::Router>();
    let (kilele_tx, kilele_rx) = tokio::sync::oneshot::channel::<axum::Router>();
    let directory_only = (agentrix.directory_router(), kilele.directory_router());
    tokio::spawn(serve(listener, directory_only, agentrix_rx, kilele_rx));

    let now = Utc::now();
    let methods = vec![PASSKEY_METHOD.to_owned(), LOCAL_OWNER_METHOD.to_owned()];
    let operator = "operator (sandbox run)";
    let intent_terms = IntentMandate {
        mandate_id: "im_kilele_sandbox".into(),
        natural_language_description: "Pay Agentrix SaaS's WebAgent invoices: up to $500 each and $1,000 a month; instalments up to 4 within 90 days, the first at least 20%.".into(),
        payer: KILELE.into(),
        agent_keyid: kilele.agent(PAYABLES)?.thumbprint().into(),
        payees: vec![AGENTRIX.into()],
        max_per_invoice: usd(50_000),
        max_per_month: usd(100_000),
        instalments: InstalmentBounds { max_count: 4, max_span_days: 90, min_first_percent: 20 },
        not_before: now - Duration::hours(1),
        intent_expiry: now + Duration::days(180),
    };
    let intent = seal_locally_approved(SealKind::IntentMandate, &intent_terms, &kilele, operator)?;
    let collections_terms = CollectionsMandate {
        mandate_id: "cm_agentrix_sandbox".into(),
        natural_language_description: "Give Kilele Bakery terms up to $1,000 owed: at most 3 instalments within 60 days, the first at least 25%.".into(),
        payee: AGENTRIX.into(),
        counterparties: vec![KILELE.into()],
        credit_limit: usd(100_000),
        instalments: InstalmentBounds { max_count: 3, max_span_days: 60, min_first_percent: 25 },
        expiry: now + Duration::days(180),
    };
    let collections = seal_locally_approved(
        SealKind::CollectionsMandate,
        &collections_terms,
        &agentrix,
        operator,
    )?;

    let agentrix_books = Arc::new(FakeBooks::new(CompanyId::agentrix()));
    let kilele_books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let agentrix_email = env
        .get("PAYPAL_MERCHANT_EMAIL")
        .cloned()
        .filter(|v| !v.is_empty());
    let known_agentrix = KnownCompany {
        company: CompanyId::agentrix(),
        name: "Agentrix SaaS".into(),
        origin: AGENTRIX.into(),
        paypal_email: agentrix_email.clone(),
    };
    let known_kilele = KnownCompany {
        company: CompanyId::kilele(),
        name: "Kilele Bakery".into(),
        origin: KILELE.into(),
        paypal_email: env.get("KILELE_PAYPAL_MERCHANT_EMAIL").cloned(),
    };
    let clock: Clock = Arc::new(Utc::now);

    let payee = PayeeDesk::new(PayeeConfig {
        name: "Agentrix SaaS".into(),
        keys: agentrix.clone(),
        directories: directories.clone(),
        store: agentrix_store.clone(),
        books: Arc::new(EraPayee::new(
            agentrix_books.clone(),
            known_agentrix.clone(),
            vec![known_kilele],
        )),
        vault: Some(VaultRail::new(paypal(
            &env,
            "PAYPAL_CLIENT_ID",
            "PAYPAL_CLIENT_SECRET",
        )?)),
        paypal_email: agentrix_email.clone(),
        collections,
        owner_methods: methods.clone(),
        negotiator: Arc::new(BoundsNegotiator),
        payer_doors: HashMap::from([(
            KILELE.to_owned(),
            DoorClient::new(
                http.clone(),
                url::Url::parse(KILELE)?,
                agentrix.agent(RECEIVABLES)?,
                AGENTRIX,
            )?,
        )]),
        clock: clock.clone(),
    })
    .await?;

    let payout = match route {
        Route::Payout => {
            let pool = sqlite(&dir.join("kilele-effects.db")).await?;
            agentrix_effects::migrate(&pool).await?;
            let effects = Effects::new(
                pool,
                Some(paypal(
                    &env,
                    "KILELE_PAYPAL_CLIENT_ID",
                    "KILELE_PAYPAL_CLIENT_SECRET",
                )?),
                Arc::new(agentrix_host::testing::MemoryReceipts::default()),
            );
            Some(PayoutRail::new(
                effects,
                SiteId::new("site_kilele_bakery"),
                StdDuration::from_secs(90),
            ))
        }
        Route::Vault => None,
    };
    let payer = PayerDesk::new(PayerConfig {
        name: "Kilele Bakery".into(),
        keys: kilele.clone(),
        directories: directories.clone(),
        store: kilele_store.clone(),
        books: Arc::new(EraPayer::new(kilele_books.clone(), vec![known_agentrix])),
        intent,
        owner_methods: methods,
        payee_doors: HashMap::from([(
            AGENTRIX.to_owned(),
            DoorClient::new(
                http.clone(),
                url::Url::parse(AGENTRIX)?,
                kilele.agent(PAYABLES)?,
                KILELE,
            )?,
        )]),
        route,
        payout,
        payee_emails: agentrix_email
            .map(|e| HashMap::from([(AGENTRIX.to_owned(), e)]))
            .unwrap_or_default(),
        cash_available,
        clock,
    })
    .await?;

    let agentrix_router = Door::new(
        "Agentrix SaaS",
        agentrix,
        DoorAuth::new(
            directories.clone(),
            agentrix_store.clone(),
            [KILELE.to_owned()],
        ),
        payee.clone(),
        Some(payee.events()),
    )
    .router();
    let kilele_router = Door::new(
        "Kilele Bakery",
        kilele,
        DoorAuth::new(directories.clone(), kilele_store, [AGENTRIX.to_owned()]),
        payer.clone(),
        Some(payer.events()),
    )
    .router();
    agentrix_tx.send(agentrix_router).ok();
    kilele_tx.send(kilele_router).ok();
    Ok(Node {
        payee,
        payer,
        agentrix_books,
        kilele_books,
        agentrix_store,
    })
}

/// One listener, two doors by `Host`: directories first, the full doors once the desks exist.
async fn serve(
    listener: tokio::net::TcpListener,
    directories: (axum::Router, axum::Router),
    agentrix: tokio::sync::oneshot::Receiver<axum::Router>,
    kilele: tokio::sync::oneshot::Receiver<axum::Router>,
) {
    let doors = Arc::new(tokio::sync::RwLock::new(directories));
    {
        let doors = doors.clone();
        tokio::spawn(async move {
            let (Ok(a), Ok(k)) = (agentrix.await, kilele.await) else {
                return;
            };
            *doors.write().await = (a, k);
        });
    }
    let app = axum::Router::new().fallback(move |request: Request| {
        let doors = doors.clone();
        async move {
            let host = request
                .headers()
                .get(HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let router = {
                let doors = doors.read().await;
                if host.starts_with("kilele.") {
                    doors.1.clone()
                } else {
                    doors.0.clone()
                }
            };
            router.oneshot(request).await.unwrap_or_else(|e| match e {})
        }
    });
    axum::serve(listener, app).await.ok();
}

async fn raise_invoice(books: &FakeBooks, minor: i64, due_in_days: i64) -> anyhow::Result<String> {
    let customer = books
        .ensure_customer(
            &Counterparty {
                name: "Kilele Bakery".into(),
                email: None,
                currency: Currency::USD,
                company: Some(CompanyId::kilele()),
                paypal_email: None,
            },
            None,
        )
        .await?;
    let today = Utc::now().date_naive();
    let draft = books
        .draft_invoice(&InvoiceDraft {
            customer: customer.id,
            issue_date: today,
            due_date: today + Duration::days(due_in_days),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: "Agentrix SaaS: WebAgent Starter, sandbox settlement run".into(),
                quantity: 1,
                unit_price: saas_core::Money::usd(minor),
            }],
            reference: None,
            memo: None,
        })
        .await?;
    Ok(books
        .post_invoice(&draft.id)
        .await?
        .number
        .unwrap_or(draft.id.0))
}

fn evidence(line: serde_json::Value) -> anyhow::Result<()> {
    use std::io::Write;
    let path = state_dir()?.join("evidence.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn,hyper=warn,reqwest=warn".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("status");
    match command {
        "vault-start" => {
            let node = node(Route::Vault, None, false).await?;
            let row = node
                .payee
                .start_vault(
                    KILELE,
                    "Kilele Bakery",
                    "https://pay.agentrix.website/settle/vault/approved",
                    "https://pay.agentrix.website/settle/vault/cancelled",
                )
                .await?;
            println!("setup token {:?}, state {}", row.setup_token, row.state);
            println!(
                "Kilele's owner approves once, signed in to the PayPal sandbox as Kilele Bakery:"
            );
            println!("{}", row.approve_url.unwrap_or_default());
        }
        "vault-token" => {
            let token = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("vault-token <setup token>"))?;
            let node = node(Route::Vault, None, false).await?;
            node.agentrix_store
                .put_vault(&saas_settle::store::VaultRow {
                    payer: KILELE.into(),
                    vault_id: None,
                    customer_id: None,
                    setup_token: Some(token.clone()),
                    approve_url: Some(format!(
                        "https://www.sandbox.paypal.com/agreements/approve?approval_session_id={token}"
                    )),
                    state: "pending".into(),
                })
                .await?;
            println!("adopted setup token {token}");
        }
        "vault-finish" => {
            let node = node(Route::Vault, None, false).await?;
            let row = node.payee.finish_vault(KILELE).await?;
            println!(
                "Kilele's wallet is saved: vault_id {:?}, customer {:?}",
                row.vault_id, row.customer_id
            );
            evidence(
                json!({"at": Utc::now(), "what": "vault saved", "vault_id": row.vault_id, "customer_id": row.customer_id, "setup_token": row.setup_token}),
            )?;
        }
        "status" => {
            let node = node(Route::Vault, None, false).await?;
            println!("{:?}", node.agentrix_store.vault(KILELE).await?);
        }
        "run" => {
            let route = match args.get(1).map(String::as_str) {
                Some("payout") => Route::Payout,
                _ => Route::Vault,
            };
            let cents: i64 = args.get(2).map(|c| c.parse()).transpose()?.unwrap_or(2_400);
            let node = node(route, None, true).await?;
            let invoice = raise_invoice(&node.agentrix_books, cents, 14).await?;
            println!(
                "1. Agentrix's books: invoice {invoice} to Kilele Bakery for USD {}.{:02}",
                cents / 100,
                cents % 100
            );
            let offer = node.payee.send_offer(&invoice).await?;
            println!(
                "2. Invoice offered over A2A (signed): Kilele's door task {} answered {:?}",
                offer.task_id, offer.answer
            );
            let started = Utc::now();
            let settled = node.payer.settle(AGENTRIX, &invoice).await?;
            let receipt: SettlementReceipt = settled.receipt.read(SealKind::SettlementReceipt)?;
            println!(
                "3. Settled on PayPal sandbox by {:?}: {} ({} {:?}), transaction {:?}",
                receipt.route,
                receipt.paypal.key(),
                receipt.paypal.status,
                receipt.paypal.parent_id,
                receipt.paypal.transaction_id
            );
            println!(
                "   receipt {} sealed by Agentrix's Receivables agent; A2A task {}",
                receipt.receipt_id, settled.task_id
            );
            let key = receipt.paypal.key();
            let filter = PaymentFilter {
                reference: Some(key.clone()),
                direction: None,
            };
            let a_pay = node.agentrix_books.payments(&filter).await?;
            let k_pay = node.kilele_books.payments(&filter).await?;
            let a_inv = node
                .agentrix_books
                .invoices(&DocumentFilter::default())
                .await?
                .into_iter()
                .find(|i| i.number.as_deref() == Some(invoice.as_str()));
            let k_bill = bill_of(node.kilele_books.as_ref(), &invoice).await?;
            println!(
                "4. Agentrix's books: receipt {:?} for {key}, invoice open {:?}",
                a_pay.first().map(|p| &p.id),
                a_inv.map(|i| i.balance_due.to_string())
            );
            println!(
                "   Kilele's books: payment {:?} for {key}, bill open {:?}",
                k_pay.first().map(|p| &p.id),
                k_bill.map(|(_, open)| open.to_string())
            );
            evidence(json!({
                "at": started,
                "what": "settlement",
                "route": receipt.route,
                "invoice": invoice,
                "amount": receipt.amount,
                "paypal": receipt.paypal,
                "cart_mandate_id": receipt.cart_mandate_id,
                "intent_mandate_id": receipt.intent_mandate_id,
                "receipt_id": receipt.receipt_id,
                "a2a_task": settled.task_id,
                "offer_task": offer.task_id,
                "agentrix_payment": a_pay.first().map(|p| p.id.0.clone()),
                "kilele_payment": k_pay.first().map(|p| p.id.0.clone()),
            }))?;
            // The same cart mandate again: refused, nothing moves.
            if let Some(DoorAnswer::InvoiceRecorded { .. }) = offer.answer {
                println!(
                    "5. Replay check: the same invoice again is refused: {:?}",
                    node.payer
                        .settle(AGENTRIX, &invoice)
                        .await
                        .err()
                        .map(|e| e.to_string())
                );
            }
        }
        "instalments" => {
            let cents: i64 = args.get(1).map(|c| c.parse()).transpose()?.unwrap_or(6_000);
            // Kilele has a third of it on hand: its Payables agent asks for instalments.
            let node = node(Route::Vault, Some(usd(cents / 3)), true).await?;
            let invoice = raise_invoice(&node.agentrix_books, cents, 0).await?;
            println!(
                "1. Agentrix's books: invoice {invoice} to Kilele Bakery for USD {}.{:02}, due today",
                cents / 100,
                cents % 100
            );
            let offer = node.payee.send_offer(&invoice).await?;
            println!("2. Offered over A2A; Kilele answered {:?}", offer.answer);
            let today = Utc::now().date_naive();
            let quarter = cents / 4;
            let proposal: Vec<saas_settle::protocol::Instalment> = [0i64, 25, 50, 80]
                .iter()
                .enumerate()
                .map(|(i, days)| saas_settle::protocol::Instalment {
                    index: i as u32 + 1,
                    due_on: today + Duration::days(*days),
                    amount: usd(if i == 3 { cents - 3 * quarter } else { quarter }),
                })
                .collect();
            let (plan, sealed, countered, reason) = node
                .payer
                .propose_instalments(
                    AGENTRIX,
                    &invoice,
                    proposal,
                    "Short until the holiday orders pay; four instalments over 80 days?",
                )
                .await?;
            println!(
                "3. Kilele proposed 4 over 80 days; Agentrix {}: {reason}",
                if countered { "countered" } else { "accepted" }
            );
            for i in &plan.instalments {
                println!(
                    "   {} of {}: {} due {}",
                    i.index,
                    plan.instalments.len(),
                    agentrix_effects::words(&i.amount),
                    i.due_on
                );
            }
            let (agreement, _) = node.payer.accept_plan(AGENTRIX, &sealed).await?;
            println!(
                "4. Kilele accepted with {} pre-signed cart mandates; plan {} agreed and scheduled",
                agreement.cart_digests.len(),
                plan.plan_id
            );
            let collected = node.payee.collect_due().await?;
            for (plan_id, index, result) in &collected {
                match result {
                    Ok(receipt) => {
                        let r: SettlementReceipt = receipt.read(SealKind::SettlementReceipt)?;
                        println!(
                            "5. Due today: instalment {index} collected on PayPal: {} (order {:?}), {} left open",
                            r.paypal.key(),
                            r.paypal.parent_id,
                            agentrix_effects::words(&r.invoice_open_after)
                        );
                        let filter = PaymentFilter {
                            reference: Some(r.paypal.key()),
                            direction: None,
                        };
                        println!(
                            "   both books posted it: Agentrix {:?}, Kilele {:?}",
                            node.agentrix_books
                                .payments(&filter)
                                .await?
                                .first()
                                .map(|p| &p.id),
                            node.kilele_books
                                .payments(&filter)
                                .await?
                                .first()
                                .map(|p| &p.id)
                        );
                        evidence(json!({
                            "at": Utc::now(), "what": "instalment collected", "plan": plan_id, "index": index,
                            "invoice": invoice, "amount": r.amount, "paypal": r.paypal, "receipt_id": r.receipt_id,
                            "cart_mandate_id": r.cart_mandate_id, "open_after": r.invoice_open_after,
                        }))?;
                    }
                    Err(refusal) => {
                        println!("5. Instalment {index} not collected: {}", refusal.message)
                    }
                }
            }
            let rest = node.agentrix_store.schedule(&plan.plan_id).await?;
            for row in rest.iter().filter(|r| r.state == "scheduled") {
                println!(
                    "   scheduled: instalment {} of {} due {}",
                    row.index,
                    rest.len(),
                    row.due_on
                );
            }
        }
        other => anyhow::bail!("unknown command {other}"),
    }
    Ok(())
}
