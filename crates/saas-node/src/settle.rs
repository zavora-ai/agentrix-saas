//! Agent-to-agent settlement on the node: saas-settle's `SettleHost`, mounted on the pay and
//! Kilele hosts, and plugged into both companies' kernels.
//!
//! - **Routes.** Settle's door paths (`/a2a`, the key directory, the agent card) and the Vault
//!   pages (`/settle/vault/…`) are answered by saas-settle on the pay host (Agentrix's door) and
//!   the Kilele host (Kilele's door), in front of saas-web's pages ([`Overlay`]).
//! - **Start-up.** Each desk checks its owner's mandate against its own published key
//!   directory, so the node first serves the directories, then builds the host, then swaps its
//!   routers in.
//! - **Mandates.** Each owner's sealed mandate is read from the data dir (the console seals it
//!   after the owner's passkey approval). With none there, a local sandbox run seals the demo
//!   terms with the owner key file, marked `local-owner-key`, never as a passkey.
//! - **The kernels.** Kilele's Payables settles through [`SettleSlot`] (saas-company's
//!   `Settlement` port), once per governed call's key; Agentrix's Receivables collects
//!   instalments through it. An invoice Agentrix offers over A2A wakes Kilele's Payables.

use std::path::Path;
use std::sync::{Arc, RwLock};

use adk_auth::web_bot_auth::{HttpKeyDirectorySource, KeyDirectorySource};
use async_trait::async_trait;
use axum::Router;
use axum::extract::Request;
use axum::response::Response;
use saas_company::identity::PAYABLES;
use saas_company::rails::{Settled, Settlement};
use saas_company::runtime::Runtime;
use saas_core::{Books, CompanyId};
use saas_settle::events::SettleEvent;
use saas_settle::host::{
    CompanySetup, SettleHost, SettleHostConfig, company_keys, demo_collections_terms,
    demo_intent_terms, load_mandate, save_mandate,
};
use saas_settle::keys::{PAYABLES as SETTLE_PAYABLES, RECEIVABLES as SETTLE_RECEIVABLES};
use saas_settle::mandate::{LOCAL_OWNER_METHOD, PASSKEY_METHOD, seal_locally_approved};
use saas_settle::{BoundsNegotiator, LoopbackDirectorySource, Route, SealKind};
use saas_web::feed::Settlement as FeedSettlement;
use saas_web::hosts::{Hosts, Site};
use tower::ServiceExt;

use crate::feed::SettlementLog;

/// Where each company's door is reached from: loopback names on the node's port locally,
/// the public hosts on the VM.
#[derive(Debug, Clone)]
pub struct Origins {
    /// Agentrix's door (the pay host).
    pub agentrix: String,
    /// Kilele's door (the Kilele host).
    pub kilele: String,
    /// Whether these are public https origins.
    pub public: bool,
}

impl Origins {
    /// `http://pay.localhost:<port>` and `http://kilele.money.localhost:<port>`.
    pub fn local(port: u16) -> Self {
        Self {
            agentrix: format!("http://pay.localhost:{port}"),
            kilele: format!("http://kilele.money.localhost:{port}"),
            public: false,
        }
    }

    /// `https://<pay host>` and `https://<Kilele host>`.
    pub fn public(hosts: &Hosts) -> Self {
        Self {
            agentrix: format!("https://{}", hosts.pay),
            kilele: format!("https://{}", hosts.kilele),
            public: true,
        }
    }
}

/// The settlement host once built: what the kernels' `Settlement` port calls.
#[derive(Default)]
pub struct SettleSlot {
    host: RwLock<Option<Arc<SettleHost>>>,
}

impl std::fmt::Debug for SettleSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettleSlot")
            .field("mounted", &self.get().is_some())
            .finish()
    }
}

impl SettleSlot {
    /// The host, when mounted.
    pub fn get(&self) -> Option<Arc<SettleHost>> {
        self.host.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Mounts `host`.
    pub fn mount(&self, host: Arc<SettleHost>) {
        *self.host.write().unwrap_or_else(|p| p.into_inner()) = Some(host);
    }

    fn need(&self) -> anyhow::Result<Arc<SettleHost>> {
        self.get().ok_or_else(|| {
            anyhow::anyhow!("the agent door isn't mounted on this node yet; nothing was paid")
        })
    }
}

fn settled(r: saas_settle::host::SettledRef) -> Settled {
    Settled {
        paypal_id: r.paypal_id,
        paypal_kind: r.paypal_kind,
        amount_minor: r.amount_minor,
        currency: r.currency,
        settled_on: r.settled_on,
    }
}

#[async_trait]
impl Settlement for SettleSlot {
    async fn settle_invoice(
        &self,
        key: &str,
        counterparty: &str,
        invoice_ref: &str,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<Settled> {
        let host = self.need()?;
        Ok(settled(
            host.settle_invoice(key, counterparty, invoice_ref, currency, amount_minor)
                .await?,
        ))
    }

    async fn agree_plan(
        &self,
        _key: &str,
        _counterparty: &str,
        plan_id: &str,
        invoice_ref: &str,
        _currency: &str,
        instalments: &[(chrono::NaiveDate, i64)],
    ) -> anyhow::Result<Vec<(chrono::NaiveDate, i64)>> {
        let host = self.need()?;
        let (plan, _sealed) = host
            .agree_plan(
                plan_id,
                invoice_ref,
                instalments,
                "Agreed by Agentrix's Receivables, inside its owner's mandate",
            )
            .await?;
        Ok(plan
            .instalments
            .iter()
            .map(|i| (i.due_on, i.amount.minor))
            .collect())
    }

    async fn collect_instalment(
        &self,
        key: &str,
        counterparty: &str,
        plan_id: &str,
        instalment: u32,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<Settled> {
        let host = self.need()?;
        Ok(settled(
            host.collect_instalment(
                key,
                counterparty,
                plan_id,
                instalment,
                currency,
                amount_minor,
            )
            .await?,
        ))
    }

    async fn offer_invoice(&self, invoice_ref: &str) -> anyhow::Result<String> {
        let offer = self.need()?.offer_invoice(invoice_ref).await?;
        Ok(if offer.note.is_empty() {
            offer.decision
        } else {
            format!("{} ({})", offer.decision, offer.note)
        })
    }
}

/// Whether `path` is one saas-settle's routers answer (its doors, key directories, the pay
/// host's key index and `/settle/`), as saas-settle defines it.
pub fn is_settle_path(path: &str) -> bool {
    saas_settle::host::is_settle_path(path)
}

/// saas-settle's routers in front of saas-web's, by Host and path; swappable once the host is
/// built.
#[derive(Clone)]
pub struct Overlay {
    hosts: Hosts,
    face: Router,
    settle: Arc<RwLock<(Router, Router)>>,
}

impl Overlay {
    /// `face` behind `pay` (the pay host's settle routes) and `kilele` (the Kilele host's).
    pub fn new(hosts: Hosts, face: Router, pay: Router, kilele: Router) -> Self {
        Self {
            hosts,
            face,
            settle: Arc::new(RwLock::new((pay, kilele))),
        }
    }

    /// Replaces the settle routes.
    pub fn swap(&self, pay: Router, kilele: Router) {
        *self.settle.write().unwrap_or_else(|p| p.into_inner()) = (pay, kilele);
    }

    /// The router.
    pub fn router(&self) -> Router {
        let me = self.clone();
        Router::new().fallback(move |request: Request| {
            let me = me.clone();
            async move { me.dispatch(request).await }
        })
    }

    async fn dispatch(&self, request: Request) -> Response {
        let site = self.hosts.site_of(request.headers());
        let router = if is_settle_path(request.uri().path()) {
            let routers = self.settle.read().unwrap_or_else(|p| p.into_inner());
            match site {
                Some(Site::Pay) => Some(routers.0.clone()),
                Some(Site::Kilele) => Some(routers.1.clone()),
                _ => None,
            }
        } else {
            None
        };
        let router = router.unwrap_or_else(|| self.face.clone());
        match router.oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }
}

/// A short tag for a pair of origins: the first 12 hex digits of their SHA-256.
fn origins_tag(origins: &Origins) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(format!("{}|{}", origins.agentrix, origins.kilele));
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// What the node needs to build the settlement host.
pub struct SettleSetup {
    agentrix: CompanySetup,
    kilele: CompanySetup,
    origins: Origins,
    data_dir: std::path::PathBuf,
    agentrix_paypal: Option<Arc<agentrix_effects::paypal::PayPalClient>>,
    kilele_cash_minor: Option<i64>,
}

async fn pool(path: &Path) -> anyhow::Result<sqlx::SqlitePool> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&format!("sqlite://{}?mode=rwc", path.display()))
        .await?;
    saas_settle::migrate(&pool).await?;
    Ok(pool)
}

impl SettleSetup {
    /// Both companies over their books, keeping keys and state under `<data_dir>/settle`.
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        data_dir: &Path,
        origins: Origins,
        agentrix_books: Arc<dyn Books>,
        kilele_books: Arc<dyn Books>,
        agentrix_email: Option<String>,
        kilele_email: Option<String>,
        agentrix_paypal: Option<Arc<agentrix_effects::paypal::PayPalClient>>,
        kilele_cash_minor: Option<i64>,
    ) -> anyhow::Result<Self> {
        let dir = data_dir.join("settle");
        Ok(Self {
            agentrix: CompanySetup {
                company: CompanyId::agentrix(),
                name: "Agentrix SaaS".into(),
                door_url: url::Url::parse(&origins.agentrix)?,
                origin: origins.agentrix.clone(),
                keys_dir: dir.join("keys").join("agentrix"),
                pool: pool(&dir.join("agentrix.db")).await?,
                books: agentrix_books,
                paypal_email: agentrix_email,
            },
            kilele: CompanySetup {
                company: CompanyId::kilele(),
                name: "Kilele Bakery".into(),
                door_url: url::Url::parse(&origins.kilele)?,
                origin: origins.kilele.clone(),
                keys_dir: dir.join("keys").join("kilele"),
                pool: pool(&dir.join("kilele.db")).await?,
                books: kilele_books,
                paypal_email: kilele_email,
            },
            origins,
            data_dir: dir,
            agentrix_paypal,
            kilele_cash_minor,
        })
    }

    /// The key directories, to serve before [`SettleSetup::build`].
    pub fn directory_routers(&self) -> anyhow::Result<(Router, Router)> {
        SettleHost::directory_routers(&self.agentrix, &self.kilele)
    }

    /// Builds the host (the node must already serve the directories).
    pub async fn build(self) -> anyhow::Result<Arc<SettleHost>> {
        let operator = "the operator's key file (local sandbox)";
        let agentrix_keys = company_keys(&self.agentrix, SETTLE_RECEIVABLES)?;
        let kilele_keys = company_keys(&self.kilele, SETTLE_PAYABLES)?;
        // A mandate names the origins it was sealed for, so its file does too: when the doors
        // move (another host, another port), fresh mandates are sealed and the old files stay.
        let tag = origins_tag(&self.origins);
        let intent_file = self.data_dir.join(format!("kilele-intent-{tag}.json"));
        let intent = match load_mandate(&intent_file)? {
            Some(sealed) => sealed,
            None => {
                let sealed = seal_locally_approved(
                    SealKind::IntentMandate,
                    &demo_intent_terms(&kilele_keys, &self.origins.agentrix)?,
                    &kilele_keys,
                    operator,
                )?;
                save_mandate(&intent_file, &sealed)?;
                sealed
            }
        };
        let collections_file = self
            .data_dir
            .join(format!("agentrix-collections-{tag}.json"));
        let collections = match load_mandate(&collections_file)? {
            Some(sealed) => sealed,
            None => {
                let sealed = seal_locally_approved(
                    SealKind::CollectionsMandate,
                    &demo_collections_terms(&agentrix_keys, &self.origins.kilele),
                    &agentrix_keys,
                    operator,
                )?;
                save_mandate(&collections_file, &sealed)?;
                sealed
            }
        };
        let (directories, http): (Arc<dyn KeyDirectorySource>, reqwest::Client) =
            if self.origins.public {
                (
                    Arc::new(HttpKeyDirectorySource::new()?),
                    reqwest::Client::new(),
                )
            } else {
                (
                    Arc::new(LoopbackDirectorySource::new()),
                    saas_settle::keys::loopback_client(&[
                        &self.origins.agentrix,
                        &self.origins.kilele,
                    ])?,
                )
            };
        let host = SettleHost::build(SettleHostConfig {
            agentrix: self.agentrix,
            kilele: self.kilele,
            directories,
            http,
            agentrix_paypal: self.agentrix_paypal,
            kilele_payout: None,
            route: Route::Vault,
            intent,
            collections,
            owner_methods: vec![PASSKEY_METHOD.into(), LOCAL_OWNER_METHOD.into()],
            negotiator: Arc::new(BoundsNegotiator),
            cash_available: self.kilele_cash_minor.map(|minor| agentrix_types::Money {
                currency: "USD".into(),
                minor,
            }),
            clock: Arc::new(chrono::Utc::now),
        })
        .await?;
        Ok(Arc::new(host))
    }
}

/// Wakes the kernels on what the doors did, and keeps the settlements for the feed.
pub fn bridge(
    host: &Arc<SettleHost>,
    agentrix: Arc<Runtime>,
    kilele: Arc<Runtime>,
    log: Arc<SettlementLog>,
) {
    let mut payer = host.payer_events();
    let mut payee = host.payee_events();
    let payer_log = log.clone();
    let payer_rts = (agentrix.clone(), kilele.clone());
    let kilele_rt = kilele.clone();
    tokio::spawn(async move {
        while let Ok(event) = payer.recv().await {
            match event {
                SettleEvent::BillOffered {
                    invoice_id,
                    bill_id,
                    amount,
                    decision,
                    note,
                    ..
                } if decision == "will_settle" => {
                    let text = format!(
                        "Agentrix offered invoice {invoice_id} over the agent door; it is bill {bill_id} in the books, {} {:.2}. Its door says: {decision} ({note}). Settle it with settle_invoice (bill_id {bill_id}) if it is inside the mandate.",
                        amount.currency,
                        amount.minor as f64 / 100.0,
                    );
                    if let Err(e) = kilele
                        .supervisor
                        .wake(
                            PAYABLES,
                            &format!("a2a-{bill_id}"),
                            &text,
                            "agent-door",
                            &format!("offer:{invoice_id}:{bill_id}"),
                        )
                        .await
                    {
                        tracing::error!(error = %e, "couldn't wake Kilele's Payables on an offer");
                    }
                }
                SettleEvent::ReceiptPosted { receipt } | SettleEvent::Settled { receipt } => {
                    payer_log.record(row(&receipt));
                }
                // An offer Kilele can't pay at once asks for instalments: Agentrix's
                // Receivables proposes a plan inside both owners' mandates.
                SettleEvent::BillOffered { .. } | SettleEvent::PlanAgreed { .. } => {}
                SettleEvent::Refused { receipt } => {
                    refused(&receipt, &payer_rts, &payer_log).await;
                }
            }
        }
    });
    tokio::spawn(async move {
        while let Ok(event) = payee.recv().await {
            match event {
                SettleEvent::Settled { receipt } | SettleEvent::ReceiptPosted { receipt } => {
                    log.record(row(&receipt));
                }
                // A plan agreed at the door becomes Agentrix's: its instalments wake the kernel
                // on their dates (a plan the desk started is already held, and kept as it is).
                SettleEvent::PlanAgreed {
                    plan_id,
                    invoice_id,
                    books_invoice_id,
                    instalments,
                    ..
                } => {
                    let invoice_id = books_invoice_id.unwrap_or(invoice_id);
                    let desk = &agentrix.company.desk;
                    let customer = match desk
                        .books
                        .invoice(&saas_core::books::InvoiceId::from(invoice_id.as_str()))
                        .await
                    {
                        Ok(invoice) => invoice.customer.to_string(),
                        Err(e) => {
                            tracing::error!(plan = %plan_id, error = %e, "an agreed plan names an invoice the books don't have");
                            continue;
                        }
                    };
                    let currency = instalments
                        .first()
                        .map(|i| i.amount.currency.clone())
                        .unwrap_or_else(|| "USD".into());
                    let plan = saas_company::desk::DebtPlan {
                        plan_id: plan_id.clone(),
                        customer,
                        invoice: invoice_id,
                        currency,
                        collected: vec![false; instalments.len()],
                        instalments: instalments
                            .iter()
                            .map(|i| saas_company::memory::Instalment {
                                due_date: i.due_on,
                                amount_minor: i.amount.minor,
                            })
                            .collect(),
                    };
                    if let Err(e) = desk.adopt_plan(plan).await {
                        tracing::error!(plan = %plan_id, error = %e, "couldn't adopt an agreed plan");
                    }
                }
                SettleEvent::BillOffered { .. } => {}
                SettleEvent::Refused { receipt } => {
                    refused(&receipt, &(agentrix.clone(), kilele_rt.clone()), &log).await;
                }
            }
        }
    });
}

/// A door's refusal: a `refused` row in the feed's settlements, and a Denied record in the
/// refusing company's signed audit log (once per refusal).
async fn refused(
    receipt: &saas_settle::protocol::RefusalReceipt,
    (agentrix, kilele): &(Arc<Runtime>, Arc<Runtime>),
    log: &SettlementLog,
) {
    let refuser = company_of(&receipt.refused_by);
    let row = FeedSettlement {
        at: receipt.at.date_naive(),
        payer: receipt
            .requester
            .as_deref()
            .map_or_else(|| "unknown".into(), company_of),
        payee: refuser.clone(),
        invoice_number: String::new(),
        currency: "USD".into(),
        amount_minor: 0,
        paypal_id: String::new(),
        status: "refused".into(),
        route: format!("A2A · refused: {}", receipt.code),
    };
    if !log.record_refusal(&receipt.refusal_id, row) {
        return;
    }
    let rt = if refuser == "kilele" {
        kilele
    } else {
        agentrix
    };
    let draft = adk_core::audit_log::AuditRecordDraft::new(
        adk_core::audit_log::AuditRecordKind::Access,
        "a2a.refused",
    )
    .decision(adk_core::audit_log::AuditDecision::Denied)
    .outcome(adk_core::audit_log::AuditRecordOutcome::NotRun)
    .ext("code", serde_json::json!(receipt.code))
    .ext("refusal_id", serde_json::json!(receipt.refusal_id))
    .ext("requester", serde_json::json!(receipt.requester))
    .ext("nothing_moved", serde_json::json!(receipt.nothing_moved));
    if let Err(e) = rt.company.governance.audit.record(draft).await {
        tracing::warn!(error = %e, "a door's refusal didn't reach the signed audit log");
    }
}

fn company_of(origin: &str) -> String {
    if origin.contains("kilele") {
        "kilele".into()
    } else {
        "agentrix".into()
    }
}

fn row(receipt: &saas_settle::SettlementReceipt) -> FeedSettlement {
    FeedSettlement {
        at: receipt.settled_at.date_naive(),
        payer: company_of(&receipt.payer),
        payee: company_of(&receipt.payee),
        invoice_number: receipt.invoice_id.clone(),
        currency: receipt.amount.currency.clone(),
        amount_minor: receipt.amount.minor,
        paypal_id: receipt.paypal.id.clone(),
        status: "settled".into(),
        route: receipt.paypal.kind.clone(),
    }
}
