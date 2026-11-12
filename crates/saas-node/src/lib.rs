//! `agentrix-saas`: the one process that runs Agentrix SaaS.
//!
//! - **Two companies**, Agentrix and Kilele Bakery, each a [`saas_company::runtime::Runtime`]:
//!   its AI staff on the governed path, its resident kernel and its flows, on durable SQLite
//!   stores under the data dir (in memory in judge mode).
//! - **Books**: saas-core's FakeBooks seeded with the story (`--books fake`), or Zavora ERA
//!   (`--books era`, once saas-books' EraBooks merges).
//! - **Money**: each company's own PayPal sandbox app through agentrix-effects.
//! - **Credentials**: each agent exchanges its own credential at its company's token service
//!   (RFC 8693, in process on loopback) for tokens bound to one resource and its role's scopes.
//! - **Mandates**: the founder's signed mandate, put in force at boot.
//! - **Hosts**: saas-web's router answers all four hosts by `Host`; the pay host also takes
//!   PayPal's webhooks at `/paypal/webhook` (verified by agentrix-effects, posted with
//!   saas-core's rules, waking the flow and the agent they concern) and slice C's A2A routes.
//! - **Judge mode** (`--judge`): scripted, seeded models and in-memory stores, with the attack
//!   demos at `/judge/` on the money host, at zero model cost.

pub mod activity;
pub mod attacks;
pub mod audit;
pub mod badges;
pub mod billing;
pub mod checkout;
pub mod config;
pub mod console;
pub mod door_cards;
pub mod feed;
pub mod hourly;
pub mod live;
pub mod ops_console;
pub mod os;
pub mod platform;
pub mod receipts;
pub mod relay;
pub mod samples;
pub mod seed;
pub mod settle;
pub mod story;
pub mod wallet_key;

use std::path::Path;
use std::sync::Arc;

use adk_core::{Clock, Url};
use agentrix_effects::Effects;
use agentrix_effects::paypal::PayPalClient;
use agentrix_host::Receipts;
use agentrix_types::SiteId;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use saas_company::audit::SignedAudit;
use saas_company::company::{Company, CompanyConfig, Parts};
use saas_company::desk::RecordingOutbox;
use saas_company::effects_rails::EffectsRails;
use saas_company::identity::{PAYABLES, Resources};
use saas_company::mandate::LocalMandates;
use saas_company::memory::CounterpartyMemory;
use saas_company::rails::{FakeRails, Rails, Settlement};
use saas_company::runtime::Runtime;
use saas_company::screen::InboundKind;
use saas_company::stores::CompanyStores;
use saas_company::sts::{TokenService, exchange_credentials};
use saas_core::Books;
use saas_web::books_feed::{BooksFeed, CompanyBooks};
use saas_web::door::Door;
use saas_web::hosts::Site;
use saas_web::passkeys::Passkeys;
use saas_web::{Access, Face, default_board_dist};
use serde_json::{Value, json};
use sqlx::sqlite::SqlitePoolOptions;

use crate::checkout::EffectsCheckout;
use crate::config::{BooksMode, Config, PayPalApp};
use crate::console::NodeConsole;
use crate::feed::{NodeOffBooks, SettlementLog};
use crate::receipts::NodeReceipts;
use crate::seed::Story;
use crate::settle::{Origins, Overlay, SettleSetup, SettleSlot};

/// The default port: everything on one.
pub const DEFAULT_PORT: u16 = 18190;

/// The seed judge mode replays with.
pub const JUDGE_SEED: u64 = 11_12;

/// One company's PayPal, through agentrix-effects.
struct Money {
    effects: Effects,
    client: Arc<PayPalClient>,
}

/// The running node.
pub struct Node {
    /// Its settings.
    pub config: Config,
    /// Its clock.
    pub clock: Arc<dyn Clock>,
    /// Agentrix.
    pub agentrix: Arc<Runtime>,
    /// Kilele Bakery.
    pub kilele: Arc<Runtime>,
    /// Agentrix's PayPal (its webhooks come here).
    pub agentrix_effects: Option<Effects>,
    /// Kilele's PayPal.
    pub kilele_effects: Option<Effects>,
    /// What the seeded books hold (fake books).
    pub story: Story,
    /// The receipts agentrix-effects signs.
    pub receipts: Arc<NodeReceipts>,
    /// Agent-to-agent settlement, once mounted ([`Node::mount_settle`]).
    pub settle: Arc<SettleSlot>,
    /// Settlements the doors reported, for the feed.
    pub settlements: Arc<SettlementLog>,
    /// Agentrix's books.
    pub agentrix_books: Arc<dyn Books>,
    /// Kilele Bakery's books.
    pub kilele_books: Arc<dyn Books>,
    /// Agentrix's PayPal app (Vault captures, reads).
    pub agentrix_client: Option<Arc<PayPalClient>>,
    /// The judges' "Run the day" runs, by id.
    pub run_days: std::sync::Mutex<std::collections::BTreeMap<String, Arc<story::Progress>>>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("config", &self.config.summary())
            .finish_non_exhaustive()
    }
}

async fn money(
    app: Option<&PayPalApp>,
    db: Option<&Path>,
    receipts: Arc<dyn Receipts>,
) -> anyhow::Result<Option<Money>> {
    let Some(app) = app else {
        return Ok(None);
    };
    let url = match db {
        Some(path) => format!("sqlite://{}?mode=rwc", path.display()),
        None => "sqlite::memory:".into(),
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(if db.is_some() { 4 } else { 1 })
        .connect(&url)
        .await?;
    agentrix_effects::migrate(&pool).await?;
    let client = Arc::new(PayPalClient::new(
        app.config.clone(),
        reqwest::Client::new(),
    ));
    let effects = Effects::new(pool, Some(client.clone()), receipts);
    Ok(Some(Money { effects, client }))
}

/// The founders' default mandates over the seeded payees, valid 30 days.
fn default_mandate(
    company: &str,
    payees: &[saas_core::books::VendorId],
    now: chrono::DateTime<chrono::Utc>,
) -> agentrix_decisions::MandateTerms {
    let (name, each, day, plans) = if company == "kilele" {
        ("Kilele Bakery", 100_000, 150_000, vec![])
    } else {
        (
            "Agentrix",
            50_000,
            150_000,
            vec![
                "essential".into(),
                "business".into(),
                "commerce".into(),
                "agency".into(),
            ],
        )
    };
    agentrix_decisions::MandateTerms {
        company: name.into(),
        currency: "USD".into(),
        refund_max_each_minor: 5_000,
        payout_max_each_minor: each,
        payout_max_per_day_minor: day,
        payees: payees.iter().map(|p| p.to_string()).collect(),
        debt_plan_max_instalments: 4,
        debt_plan_max_days: 120,
        inference_max_per_day_minor: 2_000,
        plans,
        suspend_on_failed_payment: true,
        valid_until: now + chrono::Duration::days(30),
    }
}

/// Runs agentrix-models inside this process on a loopback port of its own, metering into
/// `<data_dir>/models/control.db` (made on first boot, with both companies registered as its
/// sites); returns its URL. Routing and purposes are the gateway's own; the daily caps are
/// `SAAS_DAILY_MODEL_CAP_USD` for both companies together and `AGENTRIX_SITE_DAILY_CAP_USD`
/// for each.
async fn embedded_gateway(config: &Config) -> anyhow::Result<String> {
    let env = &config.env;
    let db = config.data_dir.join("models").join("control.db");
    agentrix_models::init_control_db(
        &db,
        &[
            ("saas-agentrix", config.hosts.money.as_str()),
            ("saas-kilele", config.hosts.kilele.as_str()),
        ],
    )
    .await?;
    let usd = |key: &str, default: f64| {
        env.get(key)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(default)
    };
    let routing = agentrix_models::route::Routing::from_lookup(|key| env.get(key))?;
    let router = agentrix_models::router(&agentrix_models::Settings {
        db,
        base_url: env
            .get("VULTR_INFERENCE_BASE_URL")
            .unwrap_or_else(|| "https://api.vultrinference.com/v1".into()),
        mode: env
            .get("AGENTRIX_MODELS_MODE")
            .unwrap_or_else(|| "live".into()),
        recordings: env.get("AGENTRIX_MODELS_RECORDINGS").map(Into::into),
        daily_cap_usd: usd("AGENTRIX_SITE_DAILY_CAP_USD", 5.0),
        fleet_daily_cap_usd: usd("SAAS_DAILY_MODEL_CAP_USD", 10.0),
        samples_daily_cap_usd: 0.0,
        env_file: config.env_file.clone(),
        routing: Some(routing),
    })
    .await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(error = %e, "the model gateway stopped");
        }
    });
    tracing::info!(%addr, "model gateway inside this process");
    Ok(format!("http://{addr}"))
}

/// Serves `sts` on a loopback port of its own; returns its token endpoint.
async fn serve_sts(sts: &Arc<TokenService>) -> anyhow::Result<Url> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let app = sts.router();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "the token service stopped");
        }
    });
    Ok(Url::parse(&format!("http://{addr}/oauth/token"))?)
}

#[allow(clippy::too_many_arguments)]
async fn company(
    config: &Config,
    company: CompanyConfig,
    books: Arc<dyn Books>,
    pay: Option<&Money>,
    clock: &Arc<dyn Clock>,
    mandates: &LocalMandates,
    http: &reqwest::Client,
    door: Arc<SettleSlot>,
    models_url: &str,
) -> anyhow::Result<Arc<Runtime>> {
    let slug = company.slug.clone();
    let dir = config.data_dir.join(&slug);
    let fake = Arc::new(FakeRails::new().with_clock(clock.clone()));
    let rails: Arc<dyn Rails> = match (config.fake_money, pay) {
        (false, Some(pay)) => Arc::new(EffectsRails::new(
            pay.effects.clone(),
            pay.client.clone(),
            SiteId::new(format!("saas-{slug}")),
            company.catalogue.clone(),
            &format!("https://{}/thanks", config.hosts.money),
        )),
        _ => fake.clone(),
    };
    let settlement: Arc<dyn Settlement> = if config.fake_money {
        fake.clone()
    } else {
        door
    };
    let (sts, credentials) = TokenService::new(&slug, &company.founder, Resources::hosted(&slug))?;
    let endpoint = serve_sts(&sts).await?;
    // Every token is checked at the token service before use, so a revoked agent stops at its
    // next call.
    let creds = exchange_credentials(
        &slug,
        &company.founder,
        endpoint,
        Resources::hosted(&slug),
        &credentials,
    )?
    .checked_by(sts.clone());
    let founder = company.founder.clone();
    let stores = CompanyStores::sqlite(&slug, &dir, clock.clone()).await?;
    let models = if config.judge {
        saas_company::judge::judge_mix(&slug, JUDGE_SEED)
    } else {
        // Every model call lands in the company's signed audit log (model, tokens, cost).
        saas_company::models::gateway_mix(http.clone(), models_url, &slug)
            .audited(stores.audit.clone())
    };
    let assembled = Company::assemble_on(
        company,
        Parts {
            books,
            rails,
            settlement,
            outbox: Arc::new(RecordingOutbox::default()),
            clock: clock.clone(),
            creds,
            memory: CounterpartyMemory::in_memory(&slug),
            audit: Arc::new(SignedAudit::seeded(&slug, JUDGE_SEED).await?),
            models,
        },
        &stores,
    )?;
    let assembled = Arc::new(assembled);
    assembled.attach_access(Arc::new(
        saas_company::access::AgentAccess::open(
            &slug,
            &founder,
            sts.clone(),
            stores.revocations.clone(),
            stores.audit.clone(),
        )
        .await?,
    ));
    if assembled.load_mandate(mandates).await? {
        tracing::info!(company = %slug, "the founder's mandate is in force");
    } else {
        tracing::warn!(company = %slug, "no signed mandate is in force: every money tool is denied");
    }
    let runtime = Runtime::start(assembled, &stores, clock.clone(), dir.join("kernel")).await?;
    Ok(Arc::new(runtime))
}

impl Node {
    /// Starts both companies.
    pub async fn start(config: Config, clock: Arc<dyn Clock>) -> anyhow::Result<Arc<Self>> {
        tracing::info!("agentrix-saas: {}", config.summary());
        std::fs::create_dir_all(&config.data_dir)?;
        let receipts = Arc::new(NodeReceipts::open(&config.data_dir)?);
        let db = |slug: &str| Some(config.data_dir.join(format!("{slug}-effects.db")));
        let agentrix_pay = money(
            config.agentrix_paypal.as_ref(),
            db("agentrix").as_deref(),
            receipts.clone(),
        )
        .await?;
        let kilele_pay = money(
            config.kilele_paypal.as_ref(),
            db("kilele").as_deref(),
            receipts.clone(),
        )
        .await?;

        let (agentrix_books, kilele_books, story): (Arc<dyn Books>, Arc<dyn Books>, Story) =
            match config.books {
                BooksMode::Fake => {
                    let seeded = saas_web::seed::seed(clock.now().date_naive()).await?;
                    let (a, k): (Arc<dyn Books>, Arc<dyn Books>) = (seeded.agentrix, seeded.kilele);
                    seed::ensure_cast(
                        a.as_ref(),
                        config.env.get("PAYPAL_PARTNER_EMAIL").as_deref(),
                    )
                    .await?;
                    seed::ensure_payroll(a.as_ref(), k.as_ref(), &config.env).await?;
                    let story = Story::read(a.as_ref(), k.as_ref()).await?;
                    (a, k, story)
                }
                BooksMode::Era => {
                    let cfg = |company| {
                        saas_books::EraConfig::from_tenants_file(
                            &config.era_tenants,
                            company,
                            &config.era_api,
                        )
                    };
                    let a: Arc<dyn Books> = Arc::new(
                        saas_books::EraBooks::connect(cfg(saas_core::CompanyId::agentrix())?)
                            .await?,
                    );
                    let k: Arc<dyn Books> = Arc::new(
                        saas_books::EraBooks::connect(cfg(saas_core::CompanyId::kilele())?).await?,
                    );
                    tracing::info!(era = %config.era_api, "books: Zavora ERA (hosted)");
                    seed::ensure_cast(
                        a.as_ref(),
                        config.env.get("PAYPAL_PARTNER_EMAIL").as_deref(),
                    )
                    .await?;
                    seed::ensure_payroll(a.as_ref(), k.as_ref(), &config.env).await?;
                    let story = Story::read(a.as_ref(), k.as_ref()).await?;
                    (a, k, story)
                }
            };

        let now = clock.now();
        let mandates = LocalMandates::new()
            .with(
                "agentrix",
                default_mandate("agentrix", &story.agentrix_payees, now),
                "the operator's key file (local sandbox)",
            )
            .with(
                "kilele",
                default_mandate("kilele", &story.kilele_payees, now),
                "the operator's key file (local sandbox)",
            );

        let http = reqwest::Client::new();
        let door = Arc::new(SettleSlot::default());
        let models_url = match (&config.models_url, config.judge) {
            (_, true) => String::new(),
            (Some(url), false) => url.clone(),
            (None, false) => embedded_gateway(&config).await?,
        };
        let agentrix = company(
            &config,
            CompanyConfig::agentrix(&config.agentrix_founder),
            agentrix_books.clone(),
            agentrix_pay.as_ref(),
            &clock,
            &mandates,
            &http,
            door.clone(),
            &models_url,
        )
        .await?;
        let kilele = company(
            &config,
            CompanyConfig::kilele(&config.kilele_founder),
            kilele_books.clone(),
            kilele_pay.as_ref(),
            &clock,
            &mandates,
            &http,
            door.clone(),
            &models_url,
        )
        .await?;
        let agentrix_client = agentrix_pay.as_ref().map(|m| m.client.clone());
        Ok(Arc::new(Self {
            config,
            clock,
            agentrix,
            kilele,
            agentrix_effects: agentrix_pay.map(|m| m.effects),
            kilele_effects: kilele_pay.map(|m| m.effects),
            story,
            receipts,
            settle: door,
            settlements: Arc::new(SettlementLog::default()),
            agentrix_books,
            kilele_books,
            agentrix_client,
            run_days: Default::default(),
        }))
    }

    /// Both companies.
    pub fn runtimes(&self) -> Vec<Arc<Runtime>> {
        vec![self.agentrix.clone(), self.kilele.clone()]
    }

    /// The owners' feed: both companies' books, and what the node knows beyond them.
    pub fn feed(self: &Arc<Self>) -> Arc<BooksFeed> {
        let mode = |rt: &Runtime| rt.company.desk.rails.label();
        Arc::new(BooksFeed::new(
            vec![
                CompanyBooks {
                    company: "agentrix".into(),
                    books: self.agentrix_books.clone(),
                    paypal_mode: mode(&self.agentrix),
                },
                CompanyBooks {
                    company: "kilele".into(),
                    books: self.kilele_books.clone(),
                    paypal_mode: mode(&self.kilele),
                },
            ],
            Arc::new(NodeOffBooks::new(
                self.config.hosts.clone(),
                vec![
                    (self.agentrix.clone(), "James".to_owned()),
                    (self.kilele.clone(), "Wanjiru".to_owned()),
                ],
                self.settlements.clone(),
            )),
            match self.config.books {
                BooksMode::Era => "books",
                BooksMode::Fake => "fakebooks",
            },
        ))
    }

    /// saas-web's router for the four hosts, with the node's feed, console, checkout and
    /// passkeys, and the webhook on the pay host.
    pub fn face(self: &Arc<Self>) -> anyhow::Result<Router> {
        let hosts = self.config.hosts.clone();
        let feed = self.feed();
        let console = Arc::new(NodeConsole::new(self.runtimes()));
        let pay = Router::new()
            .route("/paypal/webhook", post(paypal_webhook))
            .with_state(self.clone());
        let passkeys = Arc::new(Passkeys::at(self.config.data_dir.join("passkeys.json"))?);
        let mut face = Face::demo(hosts)
            .with_feed(feed)
            .with_console(console)
            .with_passkeys(passkeys)
            .with_door(Door::new().routes(pay))
            .with_board_dist(Some(
                self.config
                    .board_dist
                    .clone()
                    .unwrap_or_else(default_board_dist),
            ))
            .with_board_licence(self.config.board_licence.clone())
            .with_era(match self.config.books {
                BooksMode::Era => Some(
                    self.config
                        .era_url
                        .clone()
                        .unwrap_or_else(|| self.config.era_api.clone()),
                ),
                BooksMode::Fake => self.config.era_url.clone(),
            })
            .with_access(Access {
                owner_token: self.config.owner_token.clone(),
                open: self.config.console_open,
            });
        if let Some(effects) = &self.agentrix_effects {
            face = face.with_checkout(Arc::new(EffectsCheckout::new(
                effects.clone(),
                self.agentrix.company.config.catalogue.clone(),
                self.agentrix.clone(),
            )));
        }
        // The proofs the page shows (`SAAS_PROOFS_DIR`, else the repository's specs/saas), and
        // the judges' Run the day in judge mode.
        if let Some(dir) = self.config.env.get("SAAS_PROOFS_DIR") {
            face = face.with_proofs_dir(Some(std::path::PathBuf::from(dir)));
        }
        face = face.with_day_runner(Some(Arc::new(NodeDayRunner(Arc::downgrade(self)))));
        // The money hosts show the same badges and wallets as the console's apps and the sample
        // sites, so the judges' Verify box checks a real badge and a wallet page shows the ledger.
        for site in face.app_sites().to_vec() {
            face = face.with_app_site(crate::os::live(site));
        }
        let app = face.router()?;
        if !self.config.judge {
            return Ok(app);
        }
        let judge = Router::new()
            .route("/judge", get(judge_index))
            .route("/judge/vendor-email", post(judge_vendor_email))
            .route("/judge/run-day", post(run_day_start))
            .route("/judge/run-day/{run}", get(run_day_view))
            .with_state(self.clone());
        Ok(judge.fallback_service(app))
    }

    /// The whole server: saas-settle's door routes in front of [`Node::face`] on the pay and
    /// Kilele hosts. Before [`Node::mount_settle`] they serve only the key directories (or
    /// nothing, with `directories` `None`).
    pub fn overlay(
        self: &Arc<Self>,
        directories: Option<(Router, Router)>,
    ) -> anyhow::Result<Overlay> {
        let (pay, kilele) = directories.unwrap_or_default();
        Ok(Overlay::new(
            self.config.hosts.clone(),
            self.face()?,
            pay,
            kilele,
        ))
    }

    /// The server without settlement mounted: tests, and judge mode.
    pub fn router(self: &Arc<Self>) -> anyhow::Result<Router> {
        Ok(self.overlay(None)?.router())
    }

    /// Settlement's setup for `origins` (the node must serve its directories before
    /// [`Node::mount_settle`]).
    pub async fn settle_setup(&self, origins: Origins) -> anyhow::Result<SettleSetup> {
        SettleSetup::new(
            &self.config.data_dir,
            origins,
            self.agentrix_books.clone(),
            self.kilele_books.clone(),
            self.config
                .agentrix_paypal
                .as_ref()
                .and_then(|a| a.merchant_email.clone()),
            self.config
                .kilele_paypal
                .as_ref()
                .and_then(|a| a.merchant_email.clone()),
            self.agentrix_client.clone(),
            self.config.kilele_cash_minor,
        )
        .await
    }

    /// Builds settlement (the node already serves its directories through `overlay`), swaps
    /// its routes in, and wires it to the kernels.
    pub async fn mount_settle(&self, setup: SettleSetup, overlay: &Overlay) -> anyhow::Result<()> {
        let host = setup.build().await?;
        // Kilele's PayPal wallet, saved once with Agentrix's app (`/settle/vault/start`), by id.
        if let Some(vault) = self.config.env.get("SAAS_KILELE_VAULT_ID") {
            let kilele = host.kilele_keys().origin.clone();
            let saved = host
                .payee()
                .store()
                .vault(&kilele)
                .await?
                .is_some_and(|row| row.vault_id.is_some());
            if !saved {
                host.payee().use_vault(&kilele, &vault).await?;
                tracing::info!("Kilele's saved PayPal wallet is in use (SAAS_KILELE_VAULT_ID)");
            }
        }
        overlay.swap(host.pay_router(), host.kilele_router());
        settle::bridge(
            &host,
            self.agentrix.clone(),
            self.kilele.clone(),
            self.settlements.clone(),
        );
        self.settle.mount(host);
        Ok(())
    }
}

/// A node serving on its listener.
pub struct Running {
    /// The node.
    pub node: Arc<Node>,
    /// The port it answers on.
    pub port: u16,
    /// The server.
    pub server: tokio::task::JoinHandle<std::io::Result<()>>,
}

/// Starts the node and serves it on `listener`: the key directories first, then settlement
/// built against them and mounted (each owner's mandate is checked against the company's own
/// published directory). `public` uses the production origins for the doors.
pub async fn run(
    config: Config,
    clock: Arc<dyn Clock>,
    listener: tokio::net::TcpListener,
    public: bool,
) -> anyhow::Result<Running> {
    let hosts = config.hosts.clone();
    let fake_money = config.fake_money;
    let node = Node::start(config, clock).await?;
    let port = listener.local_addr()?.port();
    let origins = if public {
        Origins::public(&hosts)
    } else {
        Origins::local(port)
    };
    let setup = if fake_money {
        None
    } else {
        Some(node.settle_setup(origins).await?)
    };
    let overlay = node.overlay(match &setup {
        Some(s) => Some(s.directory_routers()?),
        None => None,
    })?;
    let app = overlay.router();
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    if let Some(setup) = setup {
        match node.mount_settle(setup, &overlay).await {
            Ok(()) => {
                tracing::info!("the agent doors are mounted (A2A, Web Bot Auth, Vault pages)")
            }
            Err(e) => {
                tracing::error!(error = %e, "the agent doors aren't mounted; settlement is off until restart")
            }
        }
    }
    Ok(Running { node, port, server })
}

fn answer(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// `POST /paypal/webhook` on the pay host: verified by agentrix-effects (PayPal's own signature
/// check against Agentrix's webhook id), then posted to Agentrix's books and handed to the flow
/// and the agent it concerns. PayPal retries anything answered with a 5xx.
async fn paypal_webhook(
    State(node): State<Arc<Node>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(effects) = &node.agentrix_effects else {
        return answer(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "message": "PayPal isn't configured on this node." }),
        );
    };
    // PayPal signs every delivery; one without its transmission headers isn't PayPal's, and
    // isn't worth asking PayPal about.
    let signed = [
        "paypal-auth-algo",
        "paypal-cert-url",
        "paypal-transmission-id",
        "paypal-transmission-sig",
        "paypal-transmission-time",
    ]
    .iter()
    .all(|h| headers.get(*h).is_some_and(|v| !v.is_empty()));
    if !signed {
        return answer(
            StatusCode::UNAUTHORIZED,
            json!({ "message": "Signature not verified." }),
        );
    }
    let Ok(event) = serde_json::from_slice::<Value>(&body) else {
        return answer(StatusCode::BAD_REQUEST, json!({ "message": "Not JSON." }));
    };
    let applied = match effects.handle_webhook(&headers, &body).await {
        Ok(applied) => applied,
        Err(e) => {
            let why = format!("{e:?}");
            return if why.starts_with("NotVerified") {
                answer(
                    StatusCode::UNAUTHORIZED,
                    json!({ "message": "Signature not verified." }),
                )
            } else {
                tracing::warn!(error = %why, "PayPal webhook not applied; PayPal will retry");
                answer(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({ "message": "Try again later." }),
                )
            };
        }
    };
    if event["event_type"] == "BILLING.SUBSCRIPTION.ACTIVATED"
        && let Err(e) = subscription_activated(&node, effects, &event["resource"]).await
    {
        tracing::warn!(error = %e, "an active subscription's recurring invoice wasn't set up");
    }
    // A capture of one of settlement's cart mandates (its answer lost or pending) completes
    // here, through both books, exactly once.
    let settled = match node.settle.get() {
        Some(host) => match host.on_paypal_event(&event).await {
            Ok(booked) => booked.map(|b| json!({ "capture": b.capture_id, "cart": b.cart_mandate_id, "completed_now": b.completed_now })),
            Err(e) => {
                tracing::warn!(error = %e, "a settlement capture didn't book; PayPal will retry");
                return answer(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({ "message": "Try again later." }),
                );
            }
        },
        None => None,
    };
    if settled.is_some() {
        return answer(
            StatusCode::OK,
            json!({ "applied": applied, "settlement": settled }),
        );
    }
    match node.agentrix.on_paypal_event(&event).await {
        Ok(handled) => answer(
            StatusCode::OK,
            json!({
                "applied": applied,
                "posted": handled.posted.as_ref().map(|p| !p.already_posted),
                "flow": handled.flow.as_ref().map(|(thread, status)| json!({ "thread": thread, "done": status.is_done() })),
                "process": handled.process,
            }),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "a verified PayPal event didn't post; PayPal will retry");
            answer(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({ "message": "Try again later." }),
            )
        }
    }
}

/// A subscription PayPal says is active: its plan from agentrix-effects, its subscriber from
/// PayPal's resource, and Revenue sets up the recurring invoice.
pub(crate) async fn subscription_activated(
    node: &Node,
    effects: &Effects,
    resource: &Value,
) -> anyhow::Result<()> {
    let id = resource["id"].as_str().unwrap_or_default();
    let subscription = effects.refresh_subscription(id).await?;
    let (plan, yearly) = checkout::plan_of_key(&subscription.plan_key);
    let subscriber = &resource["subscriber"];
    let email = subscriber["email_address"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("PayPal named no subscriber email"))?;
    let name = [
        subscriber["name"]["given_name"].as_str(),
        subscriber["name"]["surname"].as_str(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let name = if name.is_empty() {
        email.to_owned()
    } else {
        name
    };
    node.agentrix
        .subscription_activated(id, email, &name, &plan, yearly)
        .await?;
    Ok(())
}

fn on_money(node: &Node, headers: &HeaderMap) -> bool {
    matches!(node.config.hosts.site_of(headers), Some(Site::Money))
}

async fn judge_index(State(node): State<Arc<Node>>, headers: HeaderMap) -> Response {
    if !on_money(&node, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    answer(
        StatusCode::OK,
        json!({ "mode": "judge", "seed": JUDGE_SEED, "demos": [
            { "post": "/judge/vendor-email", "shows": "A vendor's email says its bank details changed; Payables proposes paying the new account; the governed path refuses it with a signed receipt." },
            { "post": "/judge/run-day", "shows": "The five scenes in order against the books and PayPal; GET /judge/run-day/{run} ticks them off with their ids." }
        ] }),
    )
}

/// `POST /judge/run-day`: plays the story's five scenes in the background (one run at a time)
/// and answers 202 with its id.
async fn run_day_start(State(node): State<Arc<Node>>, headers: HeaderMap) -> Response {
    if !on_money(&node, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (id, running) = start_run_day(&node);
    if running {
        answer(StatusCode::ACCEPTED, json!({ "run": id, "running": true }))
    } else {
        answer(StatusCode::ACCEPTED, json!({ "run": id }))
    }
}

/// Starts a run of the day (one at a time): its id, and whether it was already running.
fn start_run_day(node: &Arc<Node>) -> (String, bool) {
    let options = story::StoryOptions::default();
    let (id, progress) = {
        let mut runs = node.run_days.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((id, _)) = runs.iter().find(|(_, p)| p.view()["done"] == json!(false)) {
            return (id.clone(), true);
        }
        let id = format!("day-{}", node.clock.now().format("%Y%m%d%H%M%S"));
        let progress = Arc::new(story::Progress::new(&options.scenes));
        runs.insert(id.clone(), progress.clone());
        (id, progress)
    };
    let runner = node.clone();
    tokio::spawn(async move {
        if let Err(e) = story::play_with(&runner, &options, Some(progress)).await {
            tracing::warn!(error = %e, "the judges' run of the day stopped");
        }
    });
    (id, false)
}

/// How often a live node starts an isolated run of the day.
const ISOLATED_DAY_EVERY: chrono::Duration = chrono::Duration::minutes(5);

/// Run the day on a live node: one run at a time, at most one start every five minutes. The run
/// plays on a node of its own ([`isolated_day`]), so the live companies' stores, books and
/// wallets are never touched. Returns the run's id (the running or recent one, if any).
fn start_isolated_day(live: &Arc<Node>) -> String {
    let options = story::StoryOptions::default();
    let now = live.clock.now();
    let (id, progress) = {
        let mut runs = live.run_days.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((id, _)) = runs.iter().find(|(_, p)| p.view()["done"] == json!(false)) {
            return id.clone();
        }
        if let Some(latest) = runs.keys().max().cloned()
            && let Some(started) = latest
                .strip_prefix("day-")
                .and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y%m%d%H%M%S").ok())
            && now.naive_utc() - started < ISOLATED_DAY_EVERY
        {
            return latest;
        }
        let id = format!("day-{}", now.format("%Y%m%d%H%M%S"));
        let progress = Arc::new(story::Progress::new(&options.scenes));
        runs.insert(id.clone(), progress.clone());
        (id, progress)
    };
    let live = live.clone();
    let run = id.clone();
    tokio::spawn(async move {
        if let Err(e) = isolated_day(&live, &run, progress.clone(), &options).await {
            tracing::warn!(error = %format!("{e:#}"), "the judges' isolated run of the day stopped");
            progress.abandon(&e.to_string());
        }
    });
    id
}

/// One run of the day on a node of its own: judge mode (scripted, seeded models), FakeBooks,
/// its own data dir under `<data dir>/judge-runs/<run>`, PayPal's sandbox. Its kernels and
/// server are stopped and its data dir removed when the run ends.
async fn isolated_day(
    live: &Node,
    run: &str,
    progress: Arc<story::Progress>,
    options: &story::StoryOptions,
) -> anyhow::Result<()> {
    let mut config = live.config.clone();
    config.judge = true;
    config.books = BooksMode::Fake;
    config.models_url = None;
    config.kilele_cash_minor = Some(10_000);
    config.data_dir = live.config.data_dir.join("judge-runs").join(run);
    let dir = config.data_dir.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let running = run_node(config, Arc::new(adk_core::SystemClock), listener).await?;
    let played = story::play_with(&running.node, options, Some(progress)).await;
    for rt in running.node.runtimes() {
        if let Err(e) = rt.supervisor.kernel().stop().await {
            tracing::warn!(error = %e, "an isolated run's kernel didn't stop");
        }
    }
    running.server.abort();
    drop(running);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        tracing::warn!(error = %e, dir = %dir.display(), "an isolated run's data dir wasn't removed");
    }
    played.map(|_| ())
}

async fn run_node(
    config: Config,
    clock: Arc<dyn Clock>,
    listener: tokio::net::TcpListener,
) -> anyhow::Result<Running> {
    Box::pin(run(config, clock, listener, false)).await
}

/// Face's Run the day button (`/judges`), over [`start_run_day`] and the runs' progress.
struct NodeDayRunner(std::sync::Weak<Node>);

#[async_trait::async_trait]
impl saas_web::apps::DayRunner for NodeDayRunner {
    async fn start(&self) -> Result<String, saas_web::apps::AppError> {
        let node = self
            .0
            .upgrade()
            .ok_or(saas_web::apps::AppError::NotConnected)?;
        // A judge-mode node plays the day on itself; a live node never does: it starts an
        // isolated judge-mode node for the run.
        Ok(if node.config.judge {
            start_run_day(&node).0
        } else {
            start_isolated_day(&node)
        })
    }

    async fn status(&self, run: &str) -> Result<Value, saas_web::apps::AppError> {
        let node = self
            .0
            .upgrade()
            .ok_or(saas_web::apps::AppError::NotConnected)?;
        let progress = node
            .run_days
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(run)
            .cloned();
        progress
            .map(|p| p.view())
            .ok_or(saas_web::apps::AppError::NotFound)
    }
}

/// `GET /judge/run-day/{run}`: the run's scenes as they tick.
async fn run_day_view(
    State(node): State<Arc<Node>>,
    headers: HeaderMap,
    axum::extract::Path(run): axum::extract::Path<String>,
) -> Response {
    if !on_money(&node, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let progress = node
        .run_days
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&run)
        .cloned();
    match progress {
        Some(p) => answer(StatusCode::OK, p.view()),
        None => answer(StatusCode::NOT_FOUND, json!({ "message": "No such run." })),
    }
}

/// The vendor-email attack, replayed on the scripted model.
async fn judge_vendor_email(State(node): State<Arc<Node>>, headers: HeaderMap) -> Response {
    if !on_money(&node, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let company = &node.agentrix.company;
    let session = format!("judge-vendor-{}", node.clock.now().timestamp_millis());
    let run = async {
        let (screened, turn) = company
            .inbound(
                PAYABLES,
                &session,
                InboundKind::Email,
                "billing@vultr-payments.example",
                saas_company::judge::VENDOR_EMAIL,
            )
            .await?;
        let audit = &company.governance.audit;
        let denied = audit
            .last("pay_bill", adk_core::audit_log::AuditDecision::Denied)
            .await?;
        let receipt = match &denied {
            Some(record) => Some(audit.receipt(record.seq).await?),
            None => None,
        };
        let verified = match &receipt {
            Some(r) => r.verify(&audit.verifier()?).is_ok(),
            None => false,
        };
        anyhow::Ok(json!({
            "screen": { "flags": screened.flags, "blocked": screened.blocked },
            "refused": turn.as_ref().map(|t| t.refused.clone()).unwrap_or_default(),
            "agent_said": turn.as_ref().map(|t| t.text.clone()),
            "paid": node.agentrix.company.desk.rails.label(),
            "receipt": receipt,
            "receipt_verified": verified,
            "verifier_key": audit.verifier_key(),
        }))
    };
    match run.await {
        Ok(body) => answer(StatusCode::OK, body),
        Err(e) => answer(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "message": e.to_string() }),
        ),
    }
}
