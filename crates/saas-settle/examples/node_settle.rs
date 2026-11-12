//! Settlement on PayPal's **sandbox** the way the SaaS node runs it: `SettleHost` built once,
//! its `pay_router` and `kilele_router` composed by `Host` on one listener (127.0.0.1:18182,
//! `agentrix.localhost` and `kilele.localhost` standing in for `pay.agentrix.website` and
//! `kilele.money.agentrix.website`), and every step driven by the calls the companies'
//! kernels make:
//!
//! 1. Agentrix's Receivables: `offer_invoice` (A2A → Kilele's door; Kilele's desk emits
//!    `BillOffered`).
//! 2. Kilele's Payables: `settle_invoice` with a governed call's key (and again with the same
//!    key: the same settlement).
//! 3. A second invoice on terms: `agree_plan` (Agentrix's terms → A2A → Kilele accepts inside
//!    its owner's mandate), then `collect_instalment` on the first due date's wake.
//!
//! ```text
//! scripts/wcargo run -p saas-settle --example node_settle -- [cents]
//! ```
//!
//! Keys and the saved wallet come from `~/.config/agentrix-saas/settle/` (see
//! `sandbox_settle`; run its `vault-start`/`vault-finish` once). Mandates are approved with
//! the owner key file here (`local-owner-key`); on the node, with a passkey. Books are
//! saas-core `FakeBooks` unless ERA is wired in by the node.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentrix_effects::paypal::{Environment, PayPalClient, PayPalConfig};
use axum::extract::Request;
use axum::http::header::HOST;
use chrono::{Duration, Utc};
use saas_core::books::{DocumentFilter, InvoiceDraft, LineDraft, PaymentFilter};
use saas_core::fake::FakeBooks;
use saas_core::{Books, CompanyId, Counterparty, Currency};
use saas_settle::events::SettleEvent;
use saas_settle::host::{
    CompanySetup, SettleHost, SettleHostConfig, company_keys, demo_collections_terms,
    demo_intent_terms,
};
use saas_settle::keys::{PAYABLES, RECEIVABLES, loopback_client};
use saas_settle::mandate::{LOCAL_OWNER_METHOD, PASSKEY_METHOD, seal_locally_approved};
use saas_settle::payee::Clock;
use saas_settle::{BoundsNegotiator, LoopbackDirectorySource, Route, SealKind, Store};
use serde_json::json;
use tower::ServiceExt;

const PORT: u16 = 18182;
const AGENTRIX: &str = "http://agentrix.localhost:18182";
const KILELE: &str = "http://kilele.localhost:18182";

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// The repo's `.env`, line by line; a value starting with `#` is empty. Never printed.
fn read_env() -> anyhow::Result<HashMap<String, String>> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    if !path.exists() {
        path = home().join("Developer/hackathons/agentrix-saas/.env");
    }
    let mut out = HashMap::new();
    for line in std::fs::read_to_string(&path)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let mut v = v.trim().trim_matches('"').trim_matches('\'').to_owned();
        if v.starts_with('#') {
            v.clear();
        }
        out.insert(k.trim().to_owned(), v);
    }
    Ok(out)
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

async fn raise_invoice(books: &FakeBooks, minor: i64, what: &str) -> anyhow::Result<String> {
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
            due_date: today,
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: what.into(),
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
    let path = home().join(".config/agentrix-saas/settle/evidence.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

async fn posted(books: &FakeBooks, key: &str) -> anyhow::Result<Option<String>> {
    Ok(books
        .payments(&PaymentFilter {
            reference: Some(key.into()),
            direction: None,
        })
        .await?
        .first()
        .map(|p| p.id.0.clone()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,saas_settle=info".into()),
        )
        .init();
    let cents: i64 = std::env::args()
        .nth(1)
        .map(|c| c.parse())
        .transpose()?
        .unwrap_or(4_800);
    let env = read_env()?;
    anyhow::ensure!(
        env.get("PAYPAL_ENV").map(String::as_str) == Some("sandbox"),
        "PAYPAL_ENV must be sandbox"
    );
    let (Some(id), Some(secret)) = (
        env.get("PAYPAL_CLIENT_ID").filter(|v| !v.is_empty()),
        env.get("PAYPAL_CLIENT_SECRET").filter(|v| !v.is_empty()),
    ) else {
        anyhow::bail!("PAYPAL_CLIENT_ID and PAYPAL_CLIENT_SECRET must be set in .env");
    };
    let paypal = Arc::new(PayPalClient::new(
        PayPalConfig {
            client_id: id.clone(),
            client_secret: secret.clone(),
            webhook_id: None,
            environment: Environment::Sandbox,
        },
        reqwest::Client::new(),
    ));

    let state = home().join(".config/agentrix-saas/settle");
    let run = Utc::now().format("%Y%m%dT%H%M%S");
    std::fs::create_dir_all(state.join("runs"))?;
    let agentrix_books = Arc::new(FakeBooks::new(CompanyId::agentrix()));
    let kilele_books = Arc::new(FakeBooks::new(CompanyId::kilele()));
    let agentrix = CompanySetup {
        company: CompanyId::agentrix(),
        name: "Agentrix SaaS".into(),
        origin: AGENTRIX.into(),
        door_url: url::Url::parse(AGENTRIX)?,
        keys_dir: state.join("keys/agentrix"),
        pool: sqlite(&state.join(format!("runs/node-agentrix-{run}.db"))).await?,
        books: agentrix_books.clone(),
        paypal_email: env.get("PAYPAL_MERCHANT_EMAIL").cloned(),
    };
    let kilele = CompanySetup {
        company: CompanyId::kilele(),
        name: "Kilele Bakery".into(),
        origin: KILELE.into(),
        door_url: url::Url::parse(KILELE)?,
        keys_dir: state.join("keys/kilele"),
        pool: sqlite(&state.join(format!("runs/node-kilele-{run}.db"))).await?,
        books: kilele_books.clone(),
        paypal_email: env.get("KILELE_PAYPAL_MERCHANT_EMAIL").cloned(),
    };

    // As the node: serve the key directories first, then the full routers once built.
    let (agentrix_dir, kilele_dir) = SettleHost::directory_routers(&agentrix, &kilele)?;
    let routers = Arc::new(tokio::sync::RwLock::new((agentrix_dir, kilele_dir)));
    let app = {
        let routers = routers.clone();
        axum::Router::new().fallback(move |request: Request| {
            let routers = routers.clone();
            async move {
                let host = request
                    .headers()
                    .get(HOST)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let router = {
                    let r = routers.read().await;
                    if host.starts_with("kilele.") {
                        r.1.clone()
                    } else {
                        r.0.clone()
                    }
                };
                router.oneshot(request).await.unwrap_or_else(|e| match e {})
            }
        })
    };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await?;
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });

    let operator = "operator (sandbox run through the node's routers)";
    let agentrix_keys = company_keys(&agentrix, RECEIVABLES)?;
    let kilele_keys = company_keys(&kilele, PAYABLES)?;
    let intent = seal_locally_approved(
        SealKind::IntentMandate,
        &demo_intent_terms(&kilele_keys, AGENTRIX)?,
        &kilele_keys,
        operator,
    )?;
    let collections = seal_locally_approved(
        SealKind::CollectionsMandate,
        &demo_collections_terms(&agentrix_keys, KILELE),
        &agentrix_keys,
        operator,
    )?;
    let clock: Clock = Arc::new(Utc::now);
    let host = SettleHost::build(SettleHostConfig {
        agentrix,
        kilele,
        directories: Arc::new(LoopbackDirectorySource::new()),
        http: loopback_client(&[AGENTRIX, KILELE])?,
        agentrix_paypal: Some(paypal),
        kilele_payout: None,
        route: Route::Vault,
        intent,
        collections,
        owner_methods: vec![PASSKEY_METHOD.into(), LOCAL_OWNER_METHOD.into()],
        negotiator: Arc::new(BoundsNegotiator),
        cash_available: None,
        clock,
    })
    .await?;
    *routers.write().await = (host.pay_router(), host.kilele_router());

    // Kilele's wallet, saved once with Agentrix's app (sandbox_settle vault-finish).
    let wallets = Store::new(sqlite(&state.join("agentrix.db")).await?);
    saas_settle::migrate(wallets.pool()).await?;
    let wallet = wallets
        .vault(KILELE)
        .await?
        .and_then(|row| row.vault_id)
        .ok_or_else(|| {
            anyhow::anyhow!("no saved wallet: run sandbox_settle vault-start and vault-finish once")
        })?;
    host.payee().use_vault(KILELE, &wallet).await?;
    let mut kilele_events = host.payer_events();
    let mut agentrix_events = host.payee_events();

    // 1. Receivables offers the invoice; Kilele's kernel hears of the bill.
    let invoice = raise_invoice(
        &agentrix_books,
        cents,
        "Agentrix SaaS: WebAgent Pro, monthly",
    )
    .await?;
    let offer = host.offer_invoice(&invoice).await?;
    println!(
        "1. offer_invoice({invoice}): Kilele's door task {} says {}: {}",
        offer.task_id, offer.decision, offer.note
    );
    if let Ok(SettleEvent::BillOffered {
        bill_id, decision, ..
    }) = kilele_events.try_recv()
    {
        println!("   Kilele's kernel event: bill_offered (bill {bill_id}, {decision})");
    }

    // 2. Kilele's Payables settles on its governed path, once per call key.
    let key = format!("node-run-{run}");
    let settled = host
        .settle_invoice(&key, "agentrix", &invoice, "USD", cents)
        .await?;
    println!(
        "2. settle_invoice: {} {} on PayPal sandbox, {} USD {}.{:02}, receipt {}",
        settled.paypal_kind,
        settled.paypal_id,
        settled.settled_on,
        cents / 100,
        cents % 100,
        settled.receipt_id
    );
    let again = host
        .settle_invoice(&key, "agentrix", &invoice, "USD", cents)
        .await?;
    println!(
        "   the same call key again: {} (no second capture)",
        again.paypal_id
    );
    let paypal_key = format!("paypal:{}:{}", settled.paypal_kind, settled.paypal_id);
    let a = posted(&agentrix_books, &paypal_key).await?;
    let k = posted(&kilele_books, &paypal_key).await?;
    println!("   both books from {paypal_key}: Agentrix receipt {a:?}, Kilele payment {k:?}");
    let mut order = None;
    while let Ok(event) = agentrix_events.try_recv() {
        if let SettleEvent::Settled { receipt } = event {
            order = receipt.paypal.parent_id.clone();
            println!(
                "   Agentrix's kernel event: settled (order {:?}, fee {:?})",
                receipt.paypal.parent_id, receipt.paypal.fee
            );
        }
    }
    evidence(json!({
        "at": Utc::now(), "what": "node settlement", "via": "SettleHost pay_router/kilele_router",
        "invoice": invoice, "amount_minor": cents, "paypal": paypal_key, "order": order,
        "receipt_id": settled.receipt_id, "agentrix_payment": a, "kilele_payment": k, "offer_task": offer.task_id,
    }))?;

    // 3. A second invoice on Agentrix's terms, collected on the first due date's wake.
    let second = raise_invoice(
        &agentrix_books,
        2 * cents,
        "Agentrix SaaS: Commerce add-on, annual",
    )
    .await?;
    host.offer_invoice(&second).await?;
    let today = Utc::now().date_naive();
    let plan_id = format!("plan-{second}-{run}");
    let (plan, _) = host
        .agree_plan(
            &plan_id,
            &second,
            &[(today, cents), (today + Duration::days(30), cents)],
            "Half now, half in thirty days",
        )
        .await?;
    println!(
        "3. agree_plan({plan_id}): Kilele's Payables accepted {} instalments inside its owner's mandate",
        plan.instalments.len()
    );
    for row in host.uncollected().await? {
        println!(
            "   wake list: instalment {} due {} for USD {}.{:02}",
            row.index,
            row.due_on,
            row.amount.minor / 100,
            row.amount.minor % 100
        );
    }
    let first = host
        .collect_instalment(
            &format!("wake-{plan_id}-0"),
            "kilele",
            &plan_id,
            0,
            "USD",
            cents,
        )
        .await?;
    let instalment_key = format!("paypal:{}:{}", first.paypal_kind, first.paypal_id);
    println!(
        "   collect_instalment(0): {instalment_key}; Agentrix receipt {:?}, Kilele payment {:?}",
        posted(&agentrix_books, &instalment_key).await?,
        posted(&kilele_books, &instalment_key).await?
    );
    let mut instalment_order = None;
    while let Ok(event) = agentrix_events.try_recv() {
        if let SettleEvent::Settled { receipt } = event {
            instalment_order = receipt.paypal.parent_id.clone();
        }
    }
    let invoices = agentrix_books.invoices(&DocumentFilter::default()).await?;
    for i in invoices {
        println!(
            "   Agentrix's books: {} open {}",
            i.number.unwrap_or_default(),
            i.balance_due
        );
    }
    evidence(json!({
        "at": Utc::now(), "what": "node instalment collected", "via": "SettleHost agree_plan/collect_instalment",
        "invoice": second, "plan": plan_id, "position": 0, "amount_minor": cents,
        "paypal": instalment_key, "order": instalment_order, "receipt_id": first.receipt_id,
    }))?;

    // 4. The eval suite's attacks against the live doors: each refused with a sealed receipt.
    for attack in saas_settle::attacks::Attack::ALL {
        let outcome = host.attack(attack).await?;
        let receipt = outcome.receipt_terms.as_ref();
        println!(
            "4. attack {attack:?}: refused={} code={} money_moved={} receipt {} sealed by {} (checks: {})",
            outcome.refused,
            outcome.code,
            outcome.money_moved,
            receipt.map(|r| r.refusal_id.as_str()).unwrap_or("none"),
            outcome
                .receipt
                .as_ref()
                .map(|r| r.signer.as_str())
                .unwrap_or("nobody"),
            receipt.is_some()
        );
        evidence(json!({
            "at": Utc::now(), "what": "attack refused", "attack": attack, "refused": outcome.refused,
            "code": outcome.code, "money_moved": outcome.money_moved,
            "refusal_id": receipt.map(|r| r.refusal_id.clone()),
            "refused_by": receipt.map(|r| r.refused_by.clone()),
        }))?;
    }
    Ok(())
}
