//! Settlement on the SaaS node: both companies' desks and doors built once, the routers for
//! the hosts that serve them, and the calls each company's kernel makes.
//!
//! **Routers** (plain axum; the node composes them by `Host`, this crate runs no server):
//!
//! - [`SettleHost::pay_router`] for `pay.agentrix.website`: Agentrix's agent door (A2A v1
//!   `POST /a2a`, `/.well-known/http-message-signatures-directory`,
//!   `/.well-known/agent-card.json`) and the Vault pages (`/settle/vault/start`, the
//!   `/settle/vault/approved` page PayPal returns Kilele's owner to, and
//!   `/settle/vault/cancelled`).
//! - [`SettleHost::kilele_router`] for `kilele.money.agentrix.website`: Kilele's agent door
//!   (the same three paths).
//!
//! **Kernel calls** (each company's kernel wakes on [`crate::events::SettleEvent`]s, keeps the
//! due dates, and calls these; the desks keep no timers):
//!
//! - Agentrix's Receivables: [`SettleHost::offer_invoice`] (invoice offer → A2A → Kilele),
//!   [`SettleHost::agree_plan`] (its terms → A2A → Kilele accepts with cart mandates), and on
//!   each due date [`SettleHost::collect_instalment`]. [`SettleHost::uncollected`] lists the
//!   due dates to wake on, again after a restart.
//! - Kilele's Payables: [`SettleHost::settle_invoice`], once per governed call's key.
//!
//! `settle_invoice` and `collect_instalment` take saas-company's `Settlement` port's arguments
//! and return its `Settled` fields ([`SettledRef`]), so the node's adapter is a few lines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adk_auth::web_bot_auth::KeyDirectorySource;
use agentrix_effects::paypal::PayPalClient;
use agentrix_types::Money;
use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::{Duration, NaiveDate, Utc};
use saas_core::{Books, CompanyId};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::door::{Door, DoorAuth, DoorClient, Refusal};
use crate::era::{EraPayee, EraPayer, KnownCompany};
use crate::events::SettleEvent;
use crate::keys::{CompanyKeys, PAYABLES, RECEIVABLES};
use crate::mandate::{CollectionsMandate, InstalmentBounds, IntentMandate};
use crate::payee::{Clock, Negotiator, PayeeConfig, PayeeDesk};
use crate::payer::{PayError, PayerConfig, PayerDesk};
use crate::protocol::{DoorAnswer, Instalment, Route, SettlementReceipt};
use crate::rails::{PayoutRail, VaultRail};
use crate::seal::{SealKind, Sealed};
use crate::store::{ScheduledRow, Store};

/// Both companies' published agent keys, on the pay host.
pub const AGENT_KEYS_PATH: &str = "/.well-known/agentrix-saas/agent-keys.json";

/// Whether `path` is one this crate's routers answer (the node forwards these to
/// [`SettleHost::pay_router`] or [`SettleHost::kilele_router`] by host).
pub fn is_settle_path(path: &str) -> bool {
    path == crate::door::A2A_PATH
        || path == adk_auth::web_bot_auth::DIRECTORY_WELL_KNOWN_PATH
        || path == crate::door::AGENT_CARD_PATH
        || path == AGENT_KEYS_PATH
        || path.starts_with("/settle/")
}

fn company_entry(name: &str, keys: &CompanyKeys) -> serde_json::Value {
    serde_json::json!({
        "company": name,
        "origin": keys.origin,
        "key_directory": format!("{}{}", keys.origin, adk_auth::web_bot_auth::DIRECTORY_WELL_KNOWN_PATH),
        "agent_card": format!("{}{}", keys.origin, crate::door::AGENT_CARD_PATH),
        "a2a": format!("{}{}", keys.origin, crate::door::A2A_PATH),
        "keys": keys.directory().keys,
    })
}

/// Where the Vault approval returns Kilele's owner, under the pay host.
pub const VAULT_APPROVED_PATH: &str = "/settle/vault/approved";
/// Where a cancelled approval returns.
pub const VAULT_CANCELLED_PATH: &str = "/settle/vault/cancelled";
/// Starts saving the payer's wallet: redirects to PayPal's approval page.
pub const VAULT_START_PATH: &str = "/settle/vault/start";

/// One company on the node.
pub struct CompanySetup {
    /// Its id in the books (`agentrix`, `kilele`).
    pub company: CompanyId,
    /// Its name.
    pub name: String,
    /// Its key directory origin: `https://pay.agentrix.website`,
    /// `https://kilele.money.agentrix.website`.
    pub origin: String,
    /// Where its agent door is reached from the other company (usually `origin`).
    pub door_url: Url,
    /// Its agent and owner keys, as 0600 JWK files ([`CompanyKeys::load_or_create`]).
    pub keys_dir: PathBuf,
    /// Its own settlement store (never shared with the other company's).
    pub pool: sqlx::SqlitePool,
    /// Its books (Zavora ERA through saas-books, or `FakeBooks`).
    pub books: Arc<dyn Books>,
    /// Its PayPal email: where payouts to it go.
    pub paypal_email: Option<String>,
}

/// Everything settlement needs on the node.
pub struct SettleHostConfig {
    /// Agentrix, the payee.
    pub agentrix: CompanySetup,
    /// Kilele Bakery, the payer.
    pub kilele: CompanySetup,
    /// Where key directories are fetched (`HttpKeyDirectorySource` on the node).
    pub directories: Arc<dyn KeyDirectorySource>,
    /// The HTTP client the doors call each other with.
    pub http: reqwest::Client,
    /// Agentrix's PayPal app (Vault capture).
    pub agentrix_paypal: Option<Arc<PayPalClient>>,
    /// Kilele's Payout rail (the fallback route).
    pub kilele_payout: Option<PayoutRail>,
    /// How Kilele settles.
    pub route: Route,
    /// Kilele's owner's sealed intent mandate (see [`load_mandate`]).
    pub intent: Sealed,
    /// Agentrix's owner's sealed collections mandate.
    pub collections: Sealed,
    /// Approval methods accepted on owners' mandates: `passkey` on the node.
    pub owner_methods: Vec<String>,
    /// Drafts Agentrix's counter-plans (Claude, or `BoundsNegotiator`).
    pub negotiator: Arc<dyn Negotiator>,
    /// Kilele's cash on hand, when the demo sets one (an invoice over it asks for instalments).
    pub cash_available: Option<Money>,
    /// Now.
    pub clock: Clock,
}

/// What [`SettleHost::settle_invoice`] and [`SettleHost::collect_instalment`] return: the
/// fields of saas-company's `Settled`, and which PayPal object the id names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettledRef {
    /// The one PayPal id both companies' books post from.
    pub paypal_id: String,
    /// `capture` (Vault route) or `payout` (a payout item, Payout route).
    pub paypal_kind: String,
    /// Paid, in minor units.
    pub amount_minor: i64,
    /// ISO 4217.
    pub currency: String,
    /// When.
    pub settled_on: NaiveDate,
    /// The payee's sealed receipt's id.
    pub receipt_id: String,
}

impl SettledRef {
    fn of(receipt: &SettlementReceipt) -> Self {
        Self {
            paypal_id: receipt.paypal.id.clone(),
            paypal_kind: receipt.paypal.kind.clone(),
            amount_minor: receipt.amount.minor,
            currency: receipt.amount.currency.clone(),
            settled_on: receipt.settled_at.date_naive(),
            receipt_id: receipt.receipt_id.clone(),
        }
    }
}

/// A debt plan both companies agreed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreedPlanRef {
    /// The plan.
    pub plan_id: String,
    /// The payee's invoice number.
    pub invoice_id: String,
    /// The instalments, in order: (due date, minor units).
    pub instalments: Vec<(NaiveDate, i64)>,
}

/// What the payer's door said to an invoice offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferOutcome {
    /// The A2A task at the payer's door.
    pub task_id: String,
    /// `will_settle`, `instalments` or `held`.
    pub decision: String,
    /// Why, in the payer agent's words.
    pub note: String,
}

/// Settlement on the node.
pub struct SettleHost {
    payee: Arc<PayeeDesk>,
    payer: Arc<PayerDesk>,
    agentrix_door: Arc<Door>,
    kilele_door: Arc<Door>,
    agentrix_keys: CompanyKeys,
    kilele_keys: CompanyKeys,
    agentrix_name: String,
    kilele_name: String,
    companies: HashMap<String, String>,
    pub(crate) http: reqwest::Client,
    pub(crate) agentrix_door_url: Url,
    pub(crate) kilele_door_url: Url,
    pub(crate) directories: Arc<dyn KeyDirectorySource>,
    pub(crate) agentrix_store: Store,
    pub(crate) kilele_store: Store,
    pub(crate) intent: Sealed,
}

impl std::fmt::Debug for SettleHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettleHost")
            .field("companies", &self.companies)
            .finish_non_exhaustive()
    }
}

fn error(e: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

/// The keys of a company on the node (the console uses the same files to seal its owner's
/// mandate after the passkey approval).
pub fn company_keys(setup: &CompanySetup, agent: &str) -> anyhow::Result<CompanyKeys> {
    CompanyKeys::load_or_create(&setup.origin, &[agent], &setup.keys_dir)
}

/// Reads a sealed mandate the console saved; `None` when there is none yet.
pub fn load_mandate(path: &Path) -> anyhow::Result<Option<Sealed>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(serde_json::from_str(&text)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Saves a sealed mandate (mode 0600).
pub fn save_mandate(path: &Path, sealed: &Sealed) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(sealed)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn usd(minor: i64) -> Money {
    Money {
        currency: "USD".into(),
        minor,
    }
}

/// The demo's intent mandate for Kilele's owner to approve: pay Agentrix's invoices up to
/// $1,500 each and $2,000 a month, instalments up to 4 within 90 days (the first at least
/// 20%), for 180 days.
pub fn demo_intent_terms(
    kilele: &CompanyKeys,
    agentrix_origin: &str,
) -> anyhow::Result<IntentMandate> {
    let now = Utc::now();
    Ok(IntentMandate {
        mandate_id: format!("im_kilele_{}", now.format("%Y%m%d")),
        natural_language_description: "Pay Agentrix SaaS's WebAgent invoices: up to $1,500 each and $2,000 a month; instalments up to 4 within 90 days, the first at least 20%.".into(),
        payer: kilele.origin.clone(),
        agent_keyid: kilele.agent(PAYABLES)?.thumbprint().into(),
        payees: vec![agentrix_origin.trim_end_matches('/').to_owned()],
        max_per_invoice: usd(150_000),
        max_per_month: usd(200_000),
        instalments: InstalmentBounds {
            max_count: 4,
            max_span_days: 90,
            min_first_percent: 20,
        },
        not_before: now - Duration::hours(1),
        intent_expiry: now + Duration::days(180),
    })
}

/// The demo's collections mandate for Agentrix's owner to approve: terms for Kilele up to
/// $2,000 owed, at most 3 instalments within 60 days (the first at least 25%), for 180 days.
pub fn demo_collections_terms(agentrix: &CompanyKeys, kilele_origin: &str) -> CollectionsMandate {
    let now = Utc::now();
    CollectionsMandate {
        mandate_id: format!("cm_agentrix_{}", now.format("%Y%m%d")),
        natural_language_description: "Give Kilele Bakery terms up to $2,000 owed: at most 3 instalments within 60 days, the first at least 25%.".into(),
        payee: agentrix.origin.clone(),
        counterparties: vec![kilele_origin.trim_end_matches('/').to_owned()],
        credit_limit: usd(200_000),
        instalments: InstalmentBounds {
            max_count: 3,
            max_span_days: 60,
            min_first_percent: 25,
        },
        expiry: now + Duration::days(180),
    }
}

impl SettleHost {
    /// Builds both desks and doors. Each desk checks its owner's mandate against its
    /// company's published directory, so the directories must be reachable (on the node:
    /// serve the routers before or while building; locally the tests use in-memory ones).
    pub async fn build(config: SettleHostConfig) -> anyhow::Result<Self> {
        let SettleHostConfig {
            agentrix,
            kilele,
            directories,
            http,
            agentrix_paypal,
            kilele_payout,
            route,
            intent,
            collections,
            owner_methods,
            negotiator,
            cash_available,
            clock,
        } = config;
        let agentrix_keys = company_keys(&agentrix, RECEIVABLES)?;
        let kilele_keys = company_keys(&kilele, PAYABLES)?;
        let agentrix_store = Store::new(agentrix.pool.clone());
        crate::migrate(agentrix_store.pool()).await?;
        let kilele_store = Store::new(kilele.pool.clone());
        crate::migrate(kilele_store.pool()).await?;
        let known = |c: &CompanySetup| KnownCompany {
            company: c.company.clone(),
            name: c.name.clone(),
            origin: c.origin.clone(),
            paypal_email: c.paypal_email.clone(),
        };
        let (known_agentrix, known_kilele) = (known(&agentrix), known(&kilele));
        let payee = PayeeDesk::new(PayeeConfig {
            name: agentrix.name.clone(),
            keys: agentrix_keys.clone(),
            directories: directories.clone(),
            store: agentrix_store.clone(),
            books: Arc::new(EraPayee::new(
                agentrix.books.clone(),
                known_agentrix.clone(),
                vec![known_kilele.clone()],
            )),
            vault: agentrix_paypal.map(VaultRail::new),
            paypal_email: agentrix.paypal_email.clone(),
            collections,
            owner_methods: owner_methods.clone(),
            negotiator,
            payer_doors: HashMap::from([(
                kilele.origin.clone(),
                DoorClient::new(
                    http.clone(),
                    kilele.door_url.clone(),
                    agentrix_keys.agent(RECEIVABLES)?,
                    &agentrix.origin,
                )?,
            )]),
            clock: clock.clone(),
        })
        .await?;
        let payer = PayerDesk::new(PayerConfig {
            name: kilele.name.clone(),
            keys: kilele_keys.clone(),
            directories: directories.clone(),
            store: kilele_store.clone(),
            books: Arc::new(EraPayer::new(kilele.books.clone(), vec![known_agentrix])),
            intent: intent.clone(),
            owner_methods,
            payee_doors: HashMap::from([(
                agentrix.origin.clone(),
                DoorClient::new(
                    http.clone(),
                    agentrix.door_url.clone(),
                    kilele_keys.agent(PAYABLES)?,
                    &kilele.origin,
                )?,
            )]),
            route,
            payout: kilele_payout,
            payee_emails: agentrix
                .paypal_email
                .clone()
                .map(|e| HashMap::from([(agentrix.origin.clone(), e)]))
                .unwrap_or_default(),
            cash_available,
            clock,
        })
        .await?;
        let agentrix_door = Door::new(
            agentrix.name.clone(),
            agentrix_keys.clone(),
            DoorAuth::new(
                directories.clone(),
                agentrix_store.clone(),
                [kilele.origin.clone()],
            ),
            payee.clone(),
            Some(payee.events()),
        );
        let kilele_door = Door::new(
            kilele.name.clone(),
            kilele_keys.clone(),
            DoorAuth::new(
                directories.clone(),
                kilele_store.clone(),
                [agentrix.origin.clone()],
            ),
            payer.clone(),
            Some(payer.events()),
        );
        Ok(Self {
            payee,
            payer,
            agentrix_door,
            kilele_door,
            agentrix_keys,
            kilele_keys,
            agentrix_name: agentrix.name.clone(),
            kilele_name: kilele.name.clone(),
            http,
            agentrix_door_url: agentrix.door_url.clone(),
            kilele_door_url: kilele.door_url.clone(),
            directories,
            agentrix_store,
            kilele_store,
            intent,
            companies: HashMap::from([
                (
                    agentrix.company.as_str().to_owned(),
                    agentrix.origin.clone(),
                ),
                (kilele.company.as_str().to_owned(), kilele.origin.clone()),
            ]),
        })
    }

    /// Only the key directories, for serving before [`SettleHost::build`] (each desk checks
    /// its owner's mandate against its own published directory when it starts).
    pub fn directory_routers(
        agentrix: &CompanySetup,
        kilele: &CompanySetup,
    ) -> anyhow::Result<(Router, Router)> {
        Ok((
            company_keys(agentrix, RECEIVABLES)?.directory_router(),
            company_keys(kilele, PAYABLES)?.directory_router(),
        ))
    }

    /// The payee's desk (Agentrix).
    pub fn payee(&self) -> &Arc<PayeeDesk> {
        &self.payee
    }

    /// The payer's desk (Kilele Bakery).
    pub fn payer(&self) -> &Arc<PayerDesk> {
        &self.payer
    }

    /// Agentrix's keys.
    pub fn agentrix_keys(&self) -> &CompanyKeys {
        &self.agentrix_keys
    }

    /// The sample sites' agent door checker (v3, "Verify at sample doors"), on Agentrix's keys,
    /// store and events: Agentrix hosts the samples, so it seals their doors' refusals. Register
    /// it with `agentrix_samples::set_agent_proof`. `directories` reads any agent's key
    /// directory (`HttpKeyDirectorySource` on the node); `tokens` (saas-federation's
    /// `Federation`) checks bearer tokens addressed to each sample door's origin.
    pub fn sample_door_proof(
        &self,
        directories: Arc<dyn KeyDirectorySource>,
        tokens: Option<Arc<dyn crate::tokens::AgentTokens>>,
    ) -> crate::SampleDoorProof {
        crate::SampleDoorProof::new(
            DoorAuth::any_signed(directories, self.agentrix_store.clone()),
            tokens,
            self.agentrix_keys.clone(),
            Some(self.payee.events()),
        )
    }

    /// Kilele's keys.
    pub fn kilele_keys(&self) -> &CompanyKeys {
        &self.kilele_keys
    }

    /// Routes for `pay.agentrix.website`: Agentrix's agent door and the Vault pages.
    pub fn pay_router(&self) -> Router {
        let vault = Router::new()
            .route(VAULT_START_PATH, get(vault_start))
            .route(VAULT_APPROVED_PATH, get(vault_approved))
            .route(VAULT_CANCELLED_PATH, get(vault_cancelled))
            .with_state(VaultPages {
                payee: self.payee.clone(),
                payer: self.kilele_keys.origin.clone(),
                payer_name: self.kilele_name.clone(),
                payee_name: self.payee.name().to_owned(),
                pay_origin: self.agentrix_keys.origin.clone(),
            });
        let index = serde_json::json!({
            "about": "Both agent-run companies settling on this node, and where each publishes its agents' keys. Each company's key directory, at its own origin, is what its agents' Web Bot Auth signatures and sealed documents are checked against.",
            "companies": [
                company_entry(&self.agentrix_name, &self.agentrix_keys),
                company_entry(&self.kilele_name, &self.kilele_keys),
            ],
        });
        let keys = Router::new().route(
            AGENT_KEYS_PATH,
            get(move || {
                let index = index.clone();
                async move { axum::Json(index) }
            }),
        );
        self.agentrix_door.router().merge(vault).merge(keys)
    }

    /// Routes for `kilele.money.agentrix.website`: Kilele's agent door.
    pub fn kilele_router(&self) -> Router {
        self.kilele_door.router()
    }

    /// Events from Agentrix's desk (settlements, agreed plans).
    pub fn payee_events(&self) -> tokio::sync::broadcast::Receiver<SettleEvent> {
        self.payee.subscribe()
    }

    /// Events from Kilele's desk (bills offered, receipts posted, plans agreed).
    pub fn payer_events(&self) -> tokio::sync::broadcast::Receiver<SettleEvent> {
        self.payer.subscribe()
    }

    fn origin_of(&self, company: &str) -> anyhow::Result<&str> {
        self.companies
            .get(company)
            .map(String::as_str)
            .or_else(|| {
                self.companies
                    .values()
                    .find(|o| *o == company)
                    .map(String::as_str)
            })
            .ok_or_else(|| anyhow::anyhow!("{company} isn't a company on this node"))
    }

    /// Agentrix's Receivables agent offers `invoice_id` (its invoice number or books id) to
    /// the payer's door over A2A. Kilele's door records the bill, decides, and its desk emits
    /// `BillOffered` for Kilele's kernel.
    pub async fn offer_invoice(&self, invoice_id: &str) -> anyhow::Result<OfferOutcome> {
        let reply = self.payee.send_offer(invoice_id).await?;
        match reply.answer {
            Some(DoorAnswer::InvoiceRecorded { decision, note, .. }) => Ok(OfferOutcome {
                task_id: reply.task_id,
                decision,
                note,
            }),
            _ => anyhow::bail!(
                "{}",
                reply
                    .refusal
                    .unwrap_or_else(|| "the payer's door didn't record the invoice".into())
            ),
        }
    }

    /// Kilele's Payables agent settles `invoice_ref` (the payee's invoice number, the bill's
    /// vendor reference) from `counterparty` (a company id such as `agentrix`, or an origin),
    /// once per `key`. `currency` and `amount_minor` must be what is open on it.
    pub async fn settle_invoice(
        &self,
        key: &str,
        counterparty: &str,
        invoice_ref: &str,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<SettledRef> {
        let payee = self.origin_of(counterparty)?.to_owned();
        let settled = self
            .payer
            .settle_with_key(&payee, invoice_ref, Some(key))
            .await
            .map_err(|e: PayError| error(e))?;
        anyhow::ensure!(
            settled.terms.amount.currency.eq_ignore_ascii_case(currency)
                && settled.terms.amount.minor == amount_minor,
            "settled {} {}, not the {currency} {amount_minor} asked",
            settled.terms.amount.currency,
            settled.terms.amount.minor
        );
        Ok(SettledRef::of(&settled.terms))
    }

    /// Agentrix's Receivables agent offers Kilele an instalment plan for `invoice_id`
    /// (instalments as `(due date, minor units)`); Kilele's Payables agent accepts over A2A
    /// when it is inside Kilele's owner's mandate. Agreed plans are scheduled here; the
    /// kernel wakes on each due date (`PlanAgreed`, [`SettleHost::uncollected`]).
    pub async fn agree_plan(
        &self,
        plan_id: &str,
        invoice_id: &str,
        instalments: &[(NaiveDate, i64)],
        note: &str,
    ) -> anyhow::Result<(crate::protocol::InstalmentPlan, Sealed)> {
        let currency = self.payee.offer(invoice_id).await?.0.open.currency;
        let instalments = instalments
            .iter()
            .enumerate()
            .map(|(i, (due_on, minor))| Instalment {
                index: i as u32 + 1,
                due_on: *due_on,
                amount: Money {
                    currency: currency.clone(),
                    minor: *minor,
                },
            })
            .collect();
        self.payee
            .agree_plan(Some(plan_id.to_owned()), invoice_id, instalments, note)
            .await
            .map_err(|r: Refusal| error(r.message))
    }

    /// On the kernel's wake for its due date, Agentrix's Receivables agent collects one
    /// instalment (`position` counts from 0, as saas-company's plans do). `currency` and
    /// `amount_minor` must be the instalment's. Collected once; asking again returns the same.
    pub async fn collect_instalment(
        &self,
        _key: &str,
        _counterparty: &str,
        plan_id: &str,
        position: u32,
        currency: &str,
        amount_minor: i64,
    ) -> anyhow::Result<SettledRef> {
        let sealed = self
            .payee
            .collect_instalment(plan_id, position)
            .await
            .map_err(|r: Refusal| error(r.message))?;
        let receipt: SettlementReceipt = sealed.read(SealKind::SettlementReceipt)?;
        anyhow::ensure!(
            receipt.amount.currency.eq_ignore_ascii_case(currency)
                && receipt.amount.minor == amount_minor,
            "collected {} {}, not the {currency} {amount_minor} asked",
            receipt.amount.currency,
            receipt.amount.minor
        );
        Ok(SettledRef::of(&receipt))
    }

    /// saas-company's `Settlement::agree_plan` shape: Agentrix's Receivables agrees a debt plan
    /// for `invoice_ref` (books id or number) with `counterparty` over A2A, once per `plan_id`.
    /// `currency` must be the invoice's. Errors (with the reason) when Kilele's owner's mandate
    /// or Agentrix's own refuses it; nothing is scheduled then.
    #[allow(clippy::too_many_arguments)]
    pub async fn agree_debt_plan(
        &self,
        _key: &str,
        counterparty: &str,
        plan_id: &str,
        invoice_ref: &str,
        currency: &str,
        instalments: &[(NaiveDate, i64)],
        note: &str,
    ) -> anyhow::Result<AgreedPlanRef> {
        let payer = self.origin_of(counterparty).ok().map(str::to_owned);
        let (invoice, _) = self.payee.offer(invoice_ref).await?;
        anyhow::ensure!(
            payer.is_none_or(|p| p == invoice.payer),
            "invoice {invoice_ref} isn't {counterparty}'s"
        );
        anyhow::ensure!(
            invoice.open.currency.eq_ignore_ascii_case(currency),
            "invoice {invoice_ref} is in {}, not {currency}",
            invoice.open.currency
        );
        let (plan, _) = self
            .agree_plan(plan_id, invoice_ref, instalments, note)
            .await?;
        Ok(AgreedPlanRef {
            plan_id: plan.plan_id,
            invoice_id: plan.invoice_id,
            instalments: plan
                .instalments
                .iter()
                .map(|i| (i.due_on, i.amount.minor))
                .collect(),
        })
    }

    /// For the node's PayPal webhook, after agentrix-effects verified PayPal's signature:
    /// books a `PAYMENT.CAPTURE.COMPLETED` for a capture of Agentrix's cart mandates (finishing
    /// one whose answer was lost or pending, through both books). Anything else is `None`.
    pub async fn on_paypal_event(
        &self,
        event: &serde_json::Value,
    ) -> anyhow::Result<Option<crate::payee::CaptureBooked>> {
        if event["event_type"].as_str() != Some("PAYMENT.CAPTURE.COMPLETED") {
            return Ok(None);
        }
        self.payee.complete_capture(&event["resource"]).await
    }

    /// The most recent sealed refusal receipts from both companies' doors, newest first.
    pub async fn refusals(&self, limit: i64) -> anyhow::Result<Vec<Sealed>> {
        let mut out = Vec::new();
        for store in [&self.agentrix_store, &self.kilele_store] {
            for json in store.refusals(limit).await? {
                out.push(serde_json::from_str::<Sealed>(&json)?);
            }
        }
        out.sort_by(|a, b| b.body["at"].as_str().cmp(&a.body["at"].as_str()));
        out.truncate(usize::try_from(limit).unwrap_or(0));
        Ok(out)
    }

    /// Every instalment not yet collected (1-based `index`, due date, amount): the wake list.
    pub async fn uncollected(&self) -> anyhow::Result<Vec<ScheduledRow>> {
        self.payee.uncollected().await
    }
}

#[derive(Clone)]
struct VaultPages {
    payee: Arc<PayeeDesk>,
    payer: String,
    payer_name: String,
    payee_name: String,
    pay_origin: String,
}

#[derive(Deserialize)]
struct Returned {
    approval_token_id: Option<String>,
    approval_session_id: Option<String>,
}

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ --bg:#f7f6f2; --fg:#1d1d1b; --muted:#5b5b57; --card:#ffffff; --line:#e3e1da; --ok:#1f7a4d; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#141413; --fg:#f2f1ec; --muted:#a8a69f; --card:#1e1e1c; --line:#33332f; --ok:#58c48d; }} }}
body {{ margin:0; background:var(--bg); color:var(--fg); font:16px/1.55 system-ui, -apple-system, "Segoe UI", sans-serif; }}
main {{ max-width:36rem; margin:12vh auto; padding:0 16px; }}
.card {{ background:var(--card); border:1px solid var(--line); border-radius:14px; padding:28px; }}
h1 {{ font-size:1.35rem; margin:0 0 12px; }}
p {{ margin:0 0 12px; color:var(--muted); }}
p.lead {{ color:var(--fg); }}
code {{ font-size:.9em; }}
.ok {{ color:var(--ok); font-weight:600; }}
</style></head>
<body><main><div class="card">{body}</div></main></body></html>"#
    );
    (status, Html(html)).into_response()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn vault_start(State(pages): State<VaultPages>) -> Response {
    let approved = format!("{}{VAULT_APPROVED_PATH}", pages.pay_origin);
    let cancelled = format!("{}{VAULT_CANCELLED_PATH}", pages.pay_origin);
    match pages
        .payee
        .start_vault(&pages.payer, &pages.payer_name, &approved, &cancelled)
        .await
    {
        Ok(row) if row.state == "saved" => page(
            StatusCode::OK,
            "PayPal already saved",
            &format!(
                "<h1 class=\"ok\">{} already saved its PayPal with {}</h1><p>Nothing to do.</p>",
                escape(&pages.payer_name),
                escape(&pages.payee_name)
            ),
        ),
        Ok(row) => match row.approve_url {
            Some(url) => Redirect::to(&url).into_response(),
            None => page(
                StatusCode::BAD_GATEWAY,
                "PayPal didn't answer",
                "<h1>PayPal didn't give an approval link</h1><p>Nothing was saved. Try again in a minute.</p>",
            ),
        },
        Err(e) => page(
            StatusCode::BAD_GATEWAY,
            "PayPal didn't answer",
            &format!(
                "<h1>We couldn't start saving PayPal</h1><p>{}</p><p>Nothing was saved.</p>",
                escape(&e.to_string())
            ),
        ),
    }
}

async fn vault_approved(
    State(pages): State<VaultPages>,
    Query(returned): Query<Returned>,
) -> Response {
    let Some(token) = returned
        .approval_token_id
        .or(returned.approval_session_id)
        .filter(|t| !t.is_empty() && t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric()))
    else {
        return page(
            StatusCode::BAD_REQUEST,
            "Nothing to save",
            "<h1>This link didn't come from PayPal's approval</h1><p>Nothing was saved.</p>",
        );
    };
    match pages.payee.finish_vault_by_token(&token).await {
        Ok(row) => {
            let vault = row.vault_id.unwrap_or_default();
            let tail: String = vault
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            page(
                StatusCode::OK,
                "PayPal saved",
                &format!(
                    "<h1 class=\"ok\">{payer}'s PayPal is saved with {payee}</h1>\
                     <p class=\"lead\">{payee}'s agents can now settle {payer}'s invoices from it, each one only with the cart mandate {payer}'s Payables agent signs for that invoice, inside the limits {payer}'s owner approved.</p>\
                     <p>Saved payment method ending <code>…{tail}</code>. You can close this page.</p>",
                    payer = escape(&pages.payer_name),
                    payee = escape(&pages.payee_name),
                    tail = escape(&tail),
                ),
            )
        }
        Err(e) => page(
            StatusCode::CONFLICT,
            "Not saved yet",
            &format!(
                "<h1>PayPal isn't saved yet</h1><p>{}</p><p>Nothing was saved. Open the approval link again.</p>",
                escape(&e.to_string())
            ),
        ),
    }
}

async fn vault_cancelled(State(pages): State<VaultPages>) -> Response {
    page(
        StatusCode::OK,
        "Not saved",
        &format!(
            "<h1>{} didn't save its PayPal</h1><p>Nothing was saved, and nothing was charged.</p>",
            escape(&pages.payer_name)
        ),
    )
}
