//! The money layer as an `agentrix-node` module (contract C11): node calls [`mount`] once at
//! startup, like every platform crate.
//!
//! Every name follows from node's one platform host `P` (`paypal.agentrix.website` for the
//! judged build, `localhost` locally):
//!
//! | Where | What the money layer serves there |
//! |---|---|
//! | `P` | Agentrix's agent door: `/a2a`, its key directory and agent card, the key index, `/settle/` (Vault pages) |
//! | `kilele-bakery-sample.P` | Kilele Bakery's agent door, beside the sample site's own pages ([`with_sample_doors`]) |
//! | `pay.P` | PayPal's webhooks (`/paypal/webhook`) |
//! | `books.P` | the read-only view of Zavora ERA |
//! | `money.P`, `kilele.money.P` | v2's fronts, the owner consoles and the judges' routes, until face's console apps replace them |
//!
//! The seam's mount is synchronous, and the money layer starts asynchronously: the books, the
//! two companies' kernels, and settlement, which checks each owner's mandate against the key
//! directory node itself serves. So these routes answer 503 until a background task has
//! started the money layer and put the real routers in place.
//!
//! - **Models:** node's Model Gateway (`NodeContext::models_url`), metering into node's
//!   `control.db`; the two companies are registered there as sites `saas-agentrix` and
//!   `saas-kilele`.
//! - **Data:** `SAAS_DATA_DIR`, else `<node data dir>/saas-node`.
//! - **Settings:** the process environment (and `SAAS_ENV_FILE` when set), as `agentrix-saas`
//!   reads them; `SAAS_JUDGE=1` for the scripted models. `SAAS_HOST`, `SAAS_KILELE_HOST`,
//!   `SAAS_AGENT_DOOR_HOST` and `SAAS_BOOKS_HOST` still override a name.
//! - **Off unless asked:** node mounts nothing here without `SAAS_MODULE=1`, so its own tests
//!   and a platform-only node never start the money layer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use adk_core::SystemClock;
use agentrix_host::{HostLookup, HostMatch, Mounted, NodeContext};
use async_trait::async_trait;
use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use saas_web::hosts::Hosts;
use tower::ServiceExt;

use crate::Node;
use crate::config::{BooksMode, Config, EnvFile};
use crate::settle::{Origins, is_settle_path};

/// The sample site whose agent door is Kilele Bakery's.
pub const KILELE_SAMPLE: &str = "kilele-bakery-sample";

/// Where saas-badge's PayPal link router is mounted for Kilele Bakery, on the platform host.
pub const PAYPAL_LINK_KILELE: &str = "/paypal-link/kilele";

/// Face's public badge pages on a sample host.
const PUBLIC_APP_PATHS: [&str; 3] = ["/agents", "/agents/check", "/agents/check-token"];

/// Where a router goes once the money layer has built it.
#[derive(Clone, Default)]
struct Slot(Arc<RwLock<Option<Router>>>);

fn starting() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "5")],
        "Agentrix's money layer is starting.",
    )
        .into_response()
}

impl Slot {
    fn set(&self, router: Router) {
        *self.0.write().unwrap_or_else(|p| p.into_inner()) = Some(router);
    }

    async fn serve(&self, request: Request) -> Response {
        let current = self.0.read().unwrap_or_else(|p| p.into_inner()).clone();
        match current {
            Some(router) => router.oneshot(request).await.into_response(),
            None => starting(),
        }
    }

    fn router(&self) -> Router {
        let slot = self.clone();
        Router::new().fallback(move |request: Request| {
            let slot = slot.clone();
            async move { slot.serve(request).await }
        })
    }
}

/// The two agent doors and the money hosts, shared by [`mount`] and [`with_sample_doors`].
#[derive(Default)]
struct Doors {
    agentrix: Slot,
    kilele: Slot,
    /// The other sample sites' badge routes, by host.
    samples: RwLock<std::collections::HashMap<String, Router>>,
    hosts: Slot,
    paypal_link: Slot,
    /// The operators' activity page, once the money layer has started.
    activity: RwLock<Option<saas_web::activity::Activity>>,
    /// The node, once started: the operator console's agents and askline.
    node: RwLock<Option<Arc<Node>>>,
}

fn doors() -> &'static Doors {
    static DOORS: OnceLock<Doors> = OnceLock::new();
    DOORS.get_or_init(Doors::default)
}

fn enabled() -> bool {
    std::env::var("SAAS_MODULE").as_deref() == Ok("1")
}

/// The money layer's own hosts, by saas-web's rule (production names and `.localhost`
/// aliases), never node's platform host or loopback names.
struct MoneyHosts {
    hosts: Hosts,
    platform: String,
}

#[async_trait]
impl HostLookup for MoneyHosts {
    async fn serves(&self, host: &str) -> anyhow::Result<bool> {
        if host == self.platform
            || host == "localhost"
            || host.parse::<std::net::IpAddr>().is_ok()
            || host
                .split('.')
                .next()
                .is_some_and(|l| l.ends_with("-sample"))
        {
            return Ok(false);
        }
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host)?);
        Ok(self.hosts.site_of(&headers).is_some())
    }
}

/// The money layer's names under `platform`, unless the environment names them.
fn hosts_under(env: &EnvFile, platform: &str) -> Hosts {
    let name = |key: &str, label: &str| {
        env.get(key)
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_else(|| format!("{label}.{platform}"))
    };
    Hosts {
        money: name("SAAS_HOST", "money"),
        kilele: name("SAAS_KILELE_HOST", "kilele.money"),
        pay: name("SAAS_AGENT_DOOR_HOST", "pay"),
        books: name("SAAS_BOOKS_HOST", "books"),
    }
}

/// The settings `agentrix-saas` would read, for a node at `ctx`.
fn config(ctx: &NodeContext) -> anyhow::Result<Config> {
    let env = match std::env::var_os("SAAS_ENV_FILE") {
        Some(path) => EnvFile::read(&PathBuf::from(path))?,
        None => EnvFile::default(),
    };
    let books = match env.get("SAAS_BOOKS").as_deref() {
        Some("era") => BooksMode::Era,
        Some("fake") => BooksMode::Fake,
        _ => BooksMode::detect(&env),
    };
    let judge = env.get("SAAS_JUDGE").as_deref() == Some("1");
    let mut config = Config::from_env(&env, books, judge);
    config.hosts = hosts_under(&env, &ctx.platform_host);
    config.platform_host = Some(ctx.platform_host.clone());
    if env.get("SAAS_DATA_DIR").is_none() {
        config.data_dir = ctx.data_dir.join("saas-node");
    }
    config.models_url = Some(if judge {
        String::new()
    } else {
        ctx.models_url.clone()
    });
    Ok(config)
}

/// Agentrix's door at node's own origin; Kilele's at its sample site's.
fn origins(ctx: &NodeContext) -> Origins {
    let agentrix = ctx.origin.as_str().trim_end_matches('/').to_owned();
    let kilele = match ctx.origin.port() {
        Some(port) => format!(
            "{}://{KILELE_SAMPLE}.{}:{port}",
            ctx.origin.scheme(),
            ctx.platform_host
        ),
        None => format!(
            "{}://{KILELE_SAMPLE}.{}",
            ctx.origin.scheme(),
            ctx.platform_host
        ),
    };
    Origins {
        agentrix,
        kilele,
        public: ctx.origin.scheme() == "https",
    }
}

/// Registers the two companies as sites in node's `control.db`, so node's gateway meters
/// their model calls (it refuses callers it doesn't know).
async fn register_sites(pool: &sqlx::SqlitePool, hosts: &Hosts) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    for (site, domain) in [
        ("saas-agentrix", &hosts.pay),
        ("saas-kilele", &hosts.kilele),
    ] {
        sqlx::query(
            "INSERT INTO sites (site_id, domain, name, old_origin, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (site_id) DO NOTHING",
        )
        .bind(site)
        .bind(domain)
        .bind(site)
        .bind(format!("https://{domain}"))
        .bind(&now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn start(
    config: Config,
    pool: sqlx::SqlitePool,
    origins: Origins,
    issuers: Option<Arc<Issuers>>,
    receipts: Arc<dyn agentrix_host::Receipts>,
) -> anyhow::Result<Arc<Node>> {
    let doors = doors();
    register_sites(&pool, &config.hosts).await?;
    let fake_money = config.fake_money;
    let origins_now = origins.clone();
    let node = Node::start(config, Arc::new(SystemClock)).await?;
    // The sample sites take real PayPal (the sandbox) through Agentrix's app: each payment is a
    // customer_payments row in node's control.db, which `samples::follow` reads into the
    // company's wallet and books.
    if let Some(client) = node.agentrix_client.clone()
        && agentrix_samples::set_payments(Arc::new(saas_settle::SamplePayPal::new(
            agentrix_effects::Effects::new(pool.clone(), Some(client), receipts),
        )))
    {
        tracing::info!("the sample sites take PayPal (sandbox) through Agentrix's app");
    }
    // Their capture and refund webhooks reach Agentrix's runtime; each confirms its site's sale.
    node.agentrix
        .set_site_sales(Arc::new(crate::samples::SiteSales::new(pool.clone())));
    // The join: each sample site that is a company has its sales followed into that company's
    // Cashier wallet and books. The sample checkout takes payments in Agentrix's PayPal app, so
    // the wallet reads captures back through it.
    let mut company_wallets: std::collections::HashMap<String, Arc<saas_wallet::Wallets>> =
        std::collections::HashMap::new();
    // Each wallet's checkpoints are signed under a key named for this node's platform host.
    if let Some(platform_host) = &node.config.platform_host {
        let dir = node.config.data_dir.join("wallets");
        for company in [
            saas_core::CompanyId::agentrix(),
            saas_core::CompanyId::kilele(),
        ] {
            crate::wallet_key::name_for_host(&dir, &company, platform_host)?;
        }
    }
    if let Some(client) = node.agentrix_client.clone() {
        // Agentrix's own wallet record: its hourly reconciliation reads it through the shared
        // wallets, as do its wallet app and the operators' activity page.
        let wallets = Arc::new(
            saas_wallet::Wallets::sqlite(
                saas_core::CompanyId::agentrix(),
                &node.config.data_dir.join("wallets"),
                client.clone(),
                node.agentrix_books.clone(),
            )
            .await?,
        );
        crate::os::shared().add_wallets("agentrix", wallets.clone());
        company_wallets.insert("agentrix".into(), wallets);
        for (site, company) in crate::samples::joined() {
            let books = if company == saas_core::CompanyId::kilele() {
                node.kilele_books.clone()
            } else {
                node.agentrix_books.clone()
            };
            let face_site = if company == saas_core::CompanyId::kilele() {
                "kilele"
            } else {
                "agentrix"
            };
            let wallets = Arc::new(
                saas_wallet::Wallets::sqlite(
                    company,
                    &node.config.data_dir.join("wallets"),
                    client.clone(),
                    books,
                )
                .await?,
            );
            crate::os::shared().add_wallets(face_site, wallets.clone());
            company_wallets.insert(
                if face_site == "kilele" {
                    "kilele"
                } else {
                    "agentrix"
                }
                .into(),
                wallets.clone(),
            );
            tokio::spawn(crate::samples::follow(
                pool.clone(),
                site,
                wallets,
                std::time::Duration::from_secs(10),
            ));
        }
    }
    // After the sale: the shops' agents move each paid order on in its slot (agentrix-samples).
    tokio::spawn(agentrix_samples::fulfilment::follow(
        std::time::Duration::from_secs(60),
    ));
    {
        let mut logs = audit_registry().write().unwrap_or_else(|p| p.into_inner());
        logs.clear();
        for rt in [&node.agentrix, &node.kilele] {
            let audit = &rt.company.governance.audit;
            logs.push((
                rt.company.config.slug.clone(),
                audit.log_id().to_owned(),
                audit.store(),
            ));
        }
    }
    *doors.activity.write().unwrap_or_else(|p| p.into_inner()) =
        Some(crate::activity::activity(&node, pool.clone()));
    *doors.node.write().unwrap_or_else(|p| p.into_inner()) = Some(node.clone());
    // The live run: each desk reaches its wallet and the owner; a live node's kernels keep their
    // standing schedules, and the owner hears of holds, refused money calls and failures.
    let owner = Arc::new(crate::live::OwnerAlerts::from_env(&node.config.env));
    for (rt, slug) in [(&node.agentrix, "agentrix"), (&node.kilele, "kilele")] {
        rt.company
            .desk
            .use_outside(Arc::new(crate::live::NodeOutside {
                // The books' and wallets' id (`kilele-bakery`), not the slug (`kilele`): with
                // the slug, Kilele's hourly checkpoint reconciled Agentrix's wallet.
                company: if slug == "kilele" {
                    saas_core::CompanyId::kilele()
                } else {
                    saas_core::CompanyId::agentrix()
                },
                wallets: company_wallets.get(slug).cloned(),
                owner: owner.clone(),
                control: Some(pool.clone()),
                books: rt.company.desk.books.clone(),
                env: node.config.env.clone(),
                audit: Some(rt.company.governance.audit.clone()),
                slug: slug.to_owned(),
            }));
        if !node.config.judge {
            rt.supervisor
                .schedule_standing(&saas_company::supervisor::live_schedules(
                    slug == "agentrix",
                ))
                .await?;
            tokio::spawn(crate::live::follow_alerts(
                rt.clone(),
                owner.clone(),
                std::time::Duration::from_secs(20),
            ));
        }
    }
    let setup = if fake_money {
        None
    } else {
        Some(node.settle_setup(origins).await?)
    };
    let directories = match &setup {
        Some(s) => Some(s.directory_routers()?),
        None => None,
    };
    // Node is serving by now: the key directories answer at both doors before settlement
    // checks the owners' mandates against them.
    if let Some((agentrix, kilele)) = &directories {
        doors.agentrix.set(agentrix.clone());
        doors.kilele.set(kilele.clone());
    }
    let overlay = node.overlay(directories)?;
    doors.hosts.set(overlay.router());
    if let Some(setup) = setup {
        node.mount_settle(setup, &overlay).await?;
        if let Some(host) = node.settle.get() {
            doors.agentrix.set(host.pay_router());
            doors.kilele.set(host.kilele_router());
            // The sample sites' agent door: an agent is known only when it proves it (a Web Bot
            // Auth signature checked against the directory it names, or a token from a trusted
            // issuer addressed to the door). Settlement seals the refusals, as Agentrix.
            let directories: Arc<dyn adk_auth::web_bot_auth::KeyDirectorySource> =
                if origins_now.public {
                    Arc::new(adk_auth::web_bot_auth::HttpKeyDirectorySource::new()?)
                } else {
                    Arc::new(saas_settle::LoopbackDirectorySource::new())
                };
            let tokens: Option<Arc<dyn saas_settle::tokens::AgentTokens>> = match &issuers {
                Some(issuers) => match agentrix_tokens(
                    issuers,
                    node.config
                        .env
                        .get("SAAS_TRUSTED_ISSUERS_FILE")
                        .map(PathBuf::from)
                        .as_deref(),
                )
                .await
                {
                    Ok(federation) => Some(Arc::new(federation)),
                    Err(e) => {
                        tracing::error!(error = %e, "the sample doors take signatures only: no trusted issuers");
                        None
                    }
                },
                None => None,
            };
            if agentrix_samples::set_agent_proof(Arc::new(
                host.sample_door_proof(directories, tokens),
            )) {
                tracing::info!(
                    "the sample sites' agent door checks proof for its known-only tools"
                );
            }
            // Badges on the same keys the doors sign with, in front of each door.
            let badges = Arc::new(crate::badges::Badges::build(
                &origins_now,
                &node.config.data_dir,
                &node.config.agentrix_founder,
                &node.config.kilele_founder,
            )?);
            crate::badges::Badges::adopt_card(&badges.agentrix, &host.pay_router()).await?;
            crate::badges::Badges::adopt_card(&badges.kilele, &host.kilele_router()).await?;
            badges.vouch()?;
            crate::os::shared().set_badges(badges.clone());
            {
                let mut samples = doors.samples.write().unwrap_or_else(|p| p.into_inner());
                for site in &badges.samples {
                    let host = site
                        .identity
                        .host
                        .split(':')
                        .next()
                        .unwrap_or_default()
                        .to_owned();
                    samples.insert(host, saas_badge::site::router(site.clone()));
                }
            }
            // The owner's PayPal link for Kilele Bakery: saas-badge's desk over Vault on
            // Agentrix's app, sealed with the owner's passkey in Agentrix OS.
            if let Some(client) = node.agentrix_client.clone() {
                let dir = node.config.data_dir.join("badges");
                std::fs::create_dir_all(&dir)?;
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(2)
                    .connect_with(
                        sqlx::sqlite::SqliteConnectOptions::new()
                            .filename(dir.join("kilele-binding.db"))
                            .create_if_missing(true),
                    )
                    .await?;
                saas_badge::binding::migrate(&pool).await?;
                let rp_id = origins_now
                    .agentrix
                    .split_once("://")
                    .map_or(origins_now.agentrix.as_str(), |(_, h)| h)
                    .split(':')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let desk =
                    saas_badge::binding::BindingDesk::new(saas_badge::binding::BindingConfig {
                        site: badges.kilele.clone(),
                        vault: saas_settle::rails::VaultRail::new(client),
                        owner: node.config.kilele_founder.clone(),
                        rp_id,
                        audit: Arc::new(crate::os::CompanyAuditSink(
                            node.kilele.company.governance.audit.clone(),
                        )),
                        pool,
                        public_base: format!("{}{PAYPAL_LINK_KILELE}", origins_now.agentrix),
                        ceremony_url: Some(format!(
                            "/console/{KILELE_SAMPLE}/apps/passkey/link-paypal/{{id}}"
                        )),
                        approvals: None,
                        owner_account: None,
                        agents: vec![saas_badge::did::agent_did(
                            &badges.kilele.identity.host,
                            "cashier",
                        )],
                        accept_local: !origins_now.public,
                    })
                    .await?;
                doors.paypal_link.set(Router::new().nest(
                    PAYPAL_LINK_KILELE,
                    saas_badge::binding::router(desk.clone()),
                ));
                crate::os::shared().set_ceremonies(saas_badge::ceremony::Ceremonies::new().with(
                    "kilele",
                    "link-paypal",
                    desk,
                ));
            }
            doors
                .agentrix
                .set(badges.door(&badges.agentrix, host.pay_router()));
            doors
                .kilele
                .set(badges.door(&badges.kilele, host.kilele_router()));
            tokio::spawn(keep_badges_current(node.clone(), badges));
        }
        tracing::info!("the money layer's agent doors and badges are mounted");
    }
    Ok(node)
}

/// Keeps what the badges cite current: each company's freeze, and Kilele's Cashier wallet's
/// latest signed checkpoint.
async fn keep_badges_current(node: Arc<Node>, badges: Arc<crate::badges::Badges>) {
    let wallets = node.config.data_dir.join("wallets");
    loop {
        badges
            .agentrix
            .set_frozen(node.agentrix.company.governance.control.is_frozen());
        badges
            .kilele
            .set_frozen(node.kilele.company.governance.control.is_frozen());
        // A PayPal link sealed since is in the voucher from now on.
        if let Err(e) = badges.vouch() {
            tracing::warn!(error = %e, "Agentrix couldn't vouch for its sites");
        }
        if let Some(client) = node.agentrix_client.clone()
            && let Ok(w) = saas_wallet::Wallets::sqlite(
                saas_core::CompanyId::kilele(),
                &wallets,
                client,
                node.kilele_books.clone(),
            )
            .await
            && let Ok(cashier) = crate::samples::cashier(&saas_core::CompanyId::kilele())
            && let Ok(latest) = w.latest_ref(&cashier).await
        {
            badges.kilele.set_wallet(latest);
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

/// The judges' path on the platform host: `/judges` (Run the day, the Verify box), `/proofs` and
/// judge mode's `/judge`, answered by the money layer's front as if asked on its own host.
fn judges_routes(money_host: &str) -> Router {
    let mut router = Router::new();
    for path in [
        "/judges",
        "/judges/{*rest}",
        "/proofs",
        "/proofs/{*rest}",
        "/judge",
        "/judge/{*rest}",
    ] {
        let host = money_host.to_owned();
        router = router.route(
            path,
            any(move |request: Request| {
                let host = host.clone();
                async move {
                    let (mut parts, body) = request.into_parts();
                    parts.extensions = axum::http::Extensions::new();
                    if let Ok(value) = HeaderValue::from_str(&host) {
                        parts.headers.insert(header::HOST, value);
                    }
                    doors().hosts.serve(Request::from_parts(parts, body)).await
                }
            }),
        );
    }
    router
}

type AuditLogs = Vec<(String, String, Arc<dyn adk_core::audit_log::AuditLogStore>)>;

fn audit_registry() -> &'static RwLock<AuditLogs> {
    static LOGS: OnceLock<RwLock<AuditLogs>> = OnceLock::new();
    LOGS.get_or_init(|| RwLock::new(Vec::new()))
}

/// Each company's signed audit log: (company, log id, store). Empty until the money layer has
/// started, so ask per request. For the operators' activity page.
pub fn audit_logs() -> AuditLogs {
    audit_registry()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// The door's paths on `slot`, for a host shared with other routes.
fn door_routes(slot: &Slot) -> Router {
    let paths = [
        saas_settle::door::A2A_PATH,
        adk_auth::web_bot_auth::DIRECTORY_WELL_KNOWN_PATH,
        saas_settle::door::AGENT_CARD_PATH,
        saas_settle::host::AGENT_KEYS_PATH,
        "/settle/{*rest}",
        // The operator's badges and the verifier (saas-badge).
        saas_badge::site::SITE_DID_PATH,
        saas_badge::site::VOUCHER_PATH,
        saas_badge::site::BINDING_PATH,
        saas_badge::site::INDEX_PATH,
        "/agents/{role}/did.json",
        "/agents/{role}/badge.json",
        saas_badge::verify::VERIFY_PATH,
        saas_badge::verify::BADGE_VERIFY_PATH,
        saas_badge::verify::VERIFY_TOKEN_PATH,
    ];
    let mut router = Router::new();
    for path in paths {
        let slot = slot.clone();
        router = router.route(
            path,
            any(move |request: Request| {
                let slot = slot.clone();
                async move {
                    // The router behind the slot matches the path again; the parameters this
                    // route matched would otherwise be seen twice.
                    let (mut parts, body) = request.into_parts();
                    parts.extensions = axum::http::Extensions::new();
                    slot.serve(Request::from_parts(parts, body)).await
                }
            }),
        );
    }
    router
}

/// Mounts the money layer on node (contract C11).
///
/// # Errors
///
/// When the settings can't be read.
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    if !enabled() {
        return Ok(Mounted::default());
    }
    let config = config(ctx)?;
    let hosts = config.hosts.clone();
    let origins = origins(ctx);
    let pool = ctx.pool.clone();
    let issuers = issuers(ctx);
    let receipts = ctx.receipts.clone();
    let doors = doors();
    let link = doors.paypal_link.clone();
    let link_routes = Router::new().route(
        &format!("{PAYPAL_LINK_KILELE}/{{*rest}}"),
        any(move |request: Request| {
            let link = link.clone();
            async move { link.serve(request).await }
        }),
    );
    let mut mounted = Mounted::default();
    if let Some(federation) = federation(ctx) {
        mounted = mounted.platform(federation);
    }
    Ok(mounted
        .platform(judges_routes(&config.hosts.money))
        .platform(door_routes(&doors.agentrix))
        .platform(link_routes)
        // Face's assets for its console apps (styles, the wallet's grid).
        .platform(saas_web::os::assets(Some(grid_js())))
        .host(
            HostMatch::Lookup(Arc::new(MoneyHosts {
                hosts,
                platform: ctx.platform_host.clone(),
            })),
            doors.hosts.router(),
        )
        .task(
            "saas-node",
            Box::pin(async move {
                match start(config, pool, origins, issuers, receipts).await {
                    Ok(node) => {
                        tracing::info!("the money layer is serving: {}", node.config.summary());
                        // The node lives as long as the process.
                        std::future::pending::<()>().await;
                    }
                    Err(e) => {
                        tracing::error!(error = %format!("{e:#}"), "the money layer didn't start");
                    }
                }
            }),
        ))
}

/// The wallet grid's script for face's assets: `SAAS_WALLET_GRID_JS`, else face's default.
fn grid_js() -> PathBuf {
    std::env::var_os("SAAS_WALLET_GRID_JS")
        .map(PathBuf::from)
        .unwrap_or_else(saas_web::apps::default_grid_js)
}

/// Who may read the activity page (node's ops host: its signed-in operators).
pub use saas_web::activity::OperatorGate;

pub use crate::ops_console::{OperatorId, Operators};

/// Face's gate over the console's sign-in: any signed-in operator.
struct SignedIn(Arc<dyn Operators>);

#[async_trait]
impl OperatorGate for SignedIn {
    async fn allows(&self, headers: &HeaderMap) -> bool {
        self.0.operator(headers).await.is_some()
    }
}

/// The started node, or `None` while the money layer starts.
fn started() -> Option<Arc<Node>> {
    doors()
        .node
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// The operators' activity page ([`crate::activity`]) and Agentrix's real agents
/// ([`crate::ops_console`]) in front of `mounted`'s routes, which are node's operator console on
/// the ops host. For the operators `operators` says are signed in: `/activity`,
/// `/activity/rows.json`, `/activity/stream` and `/activity/summary` (the panel the console's
/// home embeds); `/activity/agents.json` and `/activity/ask`
/// (the dock's agents and the askline); and `/activity/console.js`, with face's styles and
/// scripts under `/static/saas/`. Everything else is the console's. Each answers 503 until the
/// money layer has started.
pub fn with_activity(mut mounted: Mounted, operators: Arc<dyn Operators>) -> Mounted {
    if !enabled() {
        return mounted;
    }
    let gate: Arc<dyn OperatorGate> = Arc::new(SignedIn(operators.clone()));
    let page: Arc<OnceLock<Router>> = Arc::new(OnceLock::new());
    let serve = move |request: Request| {
        let page = page.clone();
        let gate = gate.clone();
        async move {
            let router = match page.get() {
                Some(router) => router.clone(),
                None => {
                    let built = doors()
                        .activity
                        .read()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone();
                    let Some(activity) = built else {
                        return starting();
                    };
                    page.get_or_init(|| saas_web::activity::router(activity, gate))
                        .clone()
                }
            };
            router.oneshot(request).await.into_response()
        }
    };
    let who = operators.clone();
    let agents = move |headers: HeaderMap| {
        let who = who.clone();
        async move {
            if who.operator(&headers).await.is_none() {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let Some(node) = started() else {
                return starting();
            };
            (
                [(header::CACHE_CONTROL, "no-store")],
                axum::Json(crate::ops_console::agents(&node).await),
            )
                .into_response()
        }
    };
    let who = operators.clone();
    let ask = move |request: Request| {
        let who = who.clone();
        async move {
            let Some(operator) = who.operator(request.headers()).await else {
                return StatusCode::UNAUTHORIZED.into_response();
            };
            let Some(node) = started() else {
                return starting();
            };
            crate::ops_console::ask(node, operator, request).await
        }
    };
    let assets = saas_web::os::assets(Some(grid_js()));
    for routes in &mut mounted.hosts {
        let console = routes.router.clone();
        routes.router = Router::new()
            .route("/activity", any(serve.clone()))
            .route("/activity/rows.json", any(serve.clone()))
            .route("/activity/stream", any(serve.clone()))
            .route("/activity/summary", any(serve.clone()))
            .route("/activity/agents.json", axum::routing::get(agents.clone()))
            .route("/activity/ask", axum::routing::post(ask.clone()))
            .route(
                "/activity/console.js",
                axum::routing::get(|| async { crate::ops_console::script() }),
            )
            .merge(assets.clone())
            .fallback_service(console);
    }
    mounted
}

/// The sample sites on node's platform host (agentrix-samples' slugs, the host's first label),
/// with their names.
pub const SAMPLE_SITES: [(&str, &str); 3] = [
    (KILELE_SAMPLE, "Kilele Bakery"),
    ("classy-perfumes-sample", "Classy Perfumes"),
    ("tana-valley-times-sample", "Tana Valley Times"),
];

/// Each site as an OIDC issuer for its own agents (saas-federation): the platform host and
/// every sample host serve `/.well-known/openid-configuration` and `/.well-known/jwks.json`,
/// with an RS256 key per host kept in `<data dir>/federation`. Built once, on first use, so it
/// answers even while the rest of the money layer starts.
type Issuers = std::collections::HashMap<String, Arc<saas_federation::SiteIssuer>>;

/// Each site's issuer, made once.
fn issuers(ctx: &NodeContext) -> Option<Arc<Issuers>> {
    static ISSUERS: OnceLock<Option<Arc<Issuers>>> = OnceLock::new();
    ISSUERS
        .get_or_init(|| {
            let dir = match config(ctx) {
                Ok(c) => c.data_dir.join("federation"),
                Err(e) => {
                    tracing::error!(error = %e, "no settings for the sites' token issuers");
                    return None;
                }
            };
            let mut issuers = Issuers::new();
            let hosts = std::iter::once(ctx.platform_host.clone()).chain(
                SAMPLE_SITES
                    .iter()
                    .map(|(slug, _)| saas_federation::site_host(slug, &ctx.platform_host)),
            );
            for host in hosts {
                match saas_federation::SiteIssuer::for_host(&host, &dir) {
                    Ok(issuer) => {
                        issuers.insert(host, Arc::new(issuer));
                    }
                    Err(e) => {
                        tracing::error!(host = %host, error = %e, "a site's token issuer isn't up")
                    }
                }
            }
            Some(Arc::new(issuers))
        })
        .clone()
}

fn federation(ctx: &NodeContext) -> Option<Router> {
    issuers(ctx).map(|issuers| saas_federation::router((*issuers).clone()))
}

/// The tokens a sample door accepts: every Agentrix site's issuer, its keys pinned from the
/// issuer in this process (no fetch of our own discovery documents at boot).
async fn agentrix_tokens(
    issuers: &Issuers,
    extra: Option<&Path>,
) -> anyhow::Result<saas_federation::Federation> {
    let ours: Vec<saas_federation::TrustedIssuer> = issuers
        .iter()
        .map(|(host, issuer)| saas_federation::TrustedIssuer {
            kind: saas_federation::CloudKind::Agentrix,
            issuer: issuer.issuer().to_owned(),
            keys: saas_federation::Keys::Pinned {
                jwks: issuer.jwks(),
            },
            label: format!("Agentrix ({host})"),
            audiences: Vec::new(),
        })
        .collect();
    let others = match extra {
        Some(file) => match trusted_issuers_file(file) {
            Ok(others) => others,
            Err(e) => {
                tracing::error!(file = %file.display(), error = %e, "the trusted issuers file isn't read; Agentrix's own issuers only");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    if others.is_empty() {
        return Ok(saas_federation::Federation::connect(ours).await?);
    }
    let names: Vec<String> = others.iter().map(|t| t.label.clone()).collect();
    let mut all = ours.clone();
    all.extend(others);
    match saas_federation::Federation::connect(all).await {
        Ok(federation) => {
            tracing::info!(issuers = ?names, "the sample doors also trust these issuers");
            Ok(federation)
        }
        Err(e) => {
            tracing::error!(error = %e, "a trusted issuer from the file isn't reachable; Agentrix's own issuers only");
            Ok(saas_federation::Federation::connect(ours).await?)
        }
    }
}

/// Issuers to trust beside Agentrix's own: a JSON array of `saas_federation::TrustedIssuer` in
/// `file` (`SAAS_TRUSTED_ISSUERS_FILE`; `/etc/agentrix-saas/` on the server, never the
/// repository). A missing or empty file is none.
fn trusted_issuers_file(file: &Path) -> anyhow::Result<Vec<saas_federation::TrustedIssuer>> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&text)?)
}

fn is_federation_path(path: &str) -> bool {
    path == saas_federation::DISCOVERY_PATH || path == saas_federation::JWKS_PATH
}

/// Face's three apps (Passkey, Badges, Wallet) for node's shell: Kilele Bakery's sample site
/// has them. Empty unless the money layer is mounted.
pub fn console_apps(ctx: &NodeContext) -> Vec<Arc<dyn agentrix_shell::apps::ConsoleApp>> {
    if !enabled() {
        return Vec::new();
    }
    match kilele_apps(ctx) {
        Ok(apps) => crate::os::console_apps(apps, &ctx.platform_host),
        Err(e) => {
            tracing::error!(error = %e, "the money layer's console apps aren't mounted");
            Vec::new()
        }
    }
}

fn kilele_apps(ctx: &NodeContext) -> anyhow::Result<saas_web::apps::SiteApps> {
    let config = config(ctx)?;
    crate::os::kilele_apps(
        &config.kilele_founder,
        config.data_dir.join("passkeys.json"),
    )
}

/// Puts Kilele Bakery's agent door on its sample site's host, in front of the sample's own
/// pages: node's wiring passes the samples' [`Mounted`] through this. Other hosts, and every
/// other path, reach the samples' routers unchanged.
pub fn with_sample_doors(ctx: &NodeContext, mut mounted: Mounted) -> Mounted {
    if !enabled() {
        return mounted;
    }
    let kilele_host = format!("{KILELE_SAMPLE}.{}", ctx.platform_host);
    let public = match kilele_apps(ctx) {
        Ok(apps) => Some(crate::os::public_router(apps)),
        Err(e) => {
            tracing::error!(error = %e, "Kilele's public badge pages aren't mounted");
            None
        }
    };
    let federation = federation(ctx);
    // The public badge pages load face's scripts and styles from `/static/saas/…`.
    let assets = saas_web::os::assets(Some(grid_js()));
    for routes in &mut mounted.hosts {
        let public = public.clone();
        let samples = routes.router.clone();
        let host = kilele_host.clone();
        let federation = federation.clone();
        let assets = assets.clone();
        routes.router = Router::new().fallback(move |request: Request| {
            let samples = samples.clone();
            let host = host.clone();
            let public = public.clone();
            let federation = federation.clone();
            let assets = assets.clone();
            async move {
                let asked = request
                    .headers()
                    .get(header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .map(|h| h.split(':').next().unwrap_or(h).to_ascii_lowercase());
                let path = request.uri().path().to_owned();
                let ours = asked.as_deref() == Some(host.as_str());
                if ours && path.starts_with("/static/saas/") {
                    return assets.oneshot(request).await.into_response();
                }
                if is_federation_path(&path)
                    && let Some(federation) = federation
                {
                    return federation.oneshot(request).await.into_response();
                }
                if !ours
                    && saas_badge::site::is_badge_path(&path)
                    && let Some(badges) = asked.as_deref().and_then(|h| {
                        doors()
                            .samples
                            .read()
                            .unwrap_or_else(|p| p.into_inner())
                            .get(h)
                            .cloned()
                    })
                {
                    return badges.oneshot(request).await.into_response();
                }
                if ours
                    && PUBLIC_APP_PATHS.contains(&path.as_str())
                    && let Some(public) = public
                {
                    public.oneshot(request).await.into_response()
                } else if ours && (is_settle_path(&path) || saas_badge::site::is_badge_path(&path))
                {
                    doors().kilele.serve(request).await
                } else {
                    samples.oneshot(request).await.into_response()
                }
            }
        });
    }
    mounted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_empty_issuers_file_trusts_no_one_more() {
        let dir = std::env::temp_dir().join(format!("saas-issuers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            trusted_issuers_file(&dir.join("none.json"))
                .unwrap()
                .is_empty()
        );
        let empty = dir.join("empty.json");
        std::fs::write(&empty, "  ").unwrap();
        assert!(trusted_issuers_file(&empty).unwrap().is_empty());
        let one = dir.join("one.json");
        std::fs::write(
            &one,
            r#"[{"kind":"entra_agent_id","issuer":"https://login.microsoftonline.com/t/v2.0","keys":{"keys":"discovery"},"label":"Microsoft Entra (tenant t)"}]"#,
        )
        .unwrap();
        let read = trusted_issuers_file(&one).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].kind, saas_federation::CloudKind::EntraAgentId);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_name_follows_from_the_platform_host() {
        let hosts = hosts_under(&EnvFile::default(), "paypal.agentrix.website");
        assert_eq!(hosts.pay, "pay.paypal.agentrix.website");
        assert_eq!(hosts.books, "books.paypal.agentrix.website");
        assert_eq!(hosts.money, "money.paypal.agentrix.website");
        let env = EnvFile::parse("SAAS_AGENT_DOOR_HOST=Pay.Example\n");
        assert_eq!(hosts_under(&env, "x.example").pay, "pay.example");
    }

    #[tokio::test]
    async fn node_keeps_its_own_hosts_and_the_samples() {
        let lookup = MoneyHosts {
            hosts: hosts_under(&EnvFile::default(), "paypal.agentrix.website"),
            platform: "paypal.agentrix.website".into(),
        };
        for (host, ours) in [
            ("pay.paypal.agentrix.website", true),
            ("books.paypal.agentrix.website", true),
            ("money.paypal.agentrix.website", true),
            ("paypal.agentrix.website", false),
            ("ops.paypal.agentrix.website", false),
            ("kilele-bakery-sample.paypal.agentrix.website", false),
            ("localhost", false),
            ("127.0.0.1", false),
        ] {
            assert_eq!(lookup.serves(host).await.unwrap(), ours, "{host}");
        }
    }
}
