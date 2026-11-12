//! The node's routers, assembled from every lane's module (integration lane, design §3.0, §4.1).
//!
//! Requests are dispatched by `Host`: the platform's own host gets the platform routes (start,
//! Assess, sign-in, claim, console, receipts); any host a signed release serves gets the
//! converted site and its agent door; anything else is refused.

use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tower::ServiceExt as _;

use agentrix_console::Console;
use agentrix_types::Signer;

use crate::auth::{AuthConfig, AuthState};
use crate::claim::{ClaimConfig, ClaimState, Claims};
use crate::db::cas::Cas;
use crate::db::receipts::ReceiptStore;
use crate::door::DoorState;
use crate::pages::PagesState;
use crate::pages::store::SqliteReads;
use crate::site::SiteState;
use crate::site::gateway::GatewayModel;
use crate::state::{Config, key_file};
use crate::wiring;

/// The assembled node: the public router and the loopback-only router for Caddy.
pub struct App {
    /// Served on [`Config::listen`].
    pub public: Router,
    /// Served on [`Config::internal_listen`] (loopback only).
    pub internal: Router,
}

#[derive(Clone)]
struct Dispatch {
    platform_host: String,
    platform: Router,
    sites: Router,
    site_state: SiteState,
    modules: wiring::host::ModuleHosts,
}

#[derive(Clone)]
struct Shared {
    signer: Arc<Signer>,
}

/// Builds every lane's state and router from the configuration.
pub async fn build(config: &Config) -> anyhow::Result<App> {
    std::fs::create_dir_all(&config.data_dir)
        .with_context(|| format!("creating {}", config.data_dir.display()))?;
    let pool = crate::db::open(&config.db_path())
        .await
        .context("opening the database")?;
    let http = reqwest::Client::builder().build()?;
    // Releases and evidence on Vultr Object Storage (R14.4): when configured, the CAS reads
    // through to the bucket and a mirror copies blobs and receipts there.
    let objects = crate::objects::ObjectsConfig::from_env()
        .context("reading AGENTRIX_OBJECTS_FILE")?
        .map(|cfg| crate::objects::ObjectStore::new(http.clone(), cfg))
        .transpose()
        .context("the Object Storage settings")?;
    let mut cas_store = Cas::open(config.cas_dir())
        .await
        .context("opening the content store")?;
    if let Some(store) = &objects {
        cas_store = cas_store.with_remote(Arc::new(store.clone()));
    }
    let cas = Arc::new(cas_store);
    let signer = Arc::new(Signer::from_secret_bytes(&key_file(
        &config.signing_key_path(),
    )?));
    let sealer = crate::auth::Sealer::from_key(key_file(&config.seal_key_path())?);
    let receipts = Arc::new(ReceiptStore::new(pool.clone(), signer.clone()));
    let console = Arc::new(Console::new().context("loading the console templates")?);

    // Sign-in (control-plane lane).
    let auth = AuthState::new(
        AuthConfig::for_origin(config.origin.clone())?,
        pool.clone(),
        sealer,
    )?
    .with_console(console.clone());

    // Platform crates through the agentrix-host seam (contract C11): each runs its own
    // migrator, then mounts its routes, hosts and tasks; node lends them its receipts, sign-in,
    // approvals and content store.
    let ctx = agentrix_host::NodeContext {
        pool: pool.clone(),
        http: http.clone(),
        data_dir: config.data_dir.clone(),
        origin: config.origin.clone(),
        platform_host: config.platform_host(),
        models_url: config.models_url.clone(),
        receipts: Arc::new(wiring::host::NodeReceipts {
            store: receipts.clone(),
        }),
        owners: Arc::new(wiring::host::NodeOwners { auth: auth.clone() }),
        approvals: Arc::new(wiring::host::NodeApprovals {
            approvals: auth.approvals(),
        }),
        blobs: Arc::new(wiring::host::NodeBlobs { cas: cas.clone() }),
        owner_accounts: Arc::new(wiring::host::NodeOwnerAccounts { auth: auth.clone() }),
    };
    // What governed paths check before they run: the WebAgents' dials (W1), the owner's Freeze
    // (W5) and the operator's holds (W3). With Freeze enforced here, the shell may offer it
    // (AGENTRIX_FREEZE_ENFORCED).
    let guards = crate::guards::Guards::for_node(&ctx);

    // The broker (assess lane): signed jobs to the runner on the sandbox VM (design §4.2).
    let broker = crate::broker::Broker::for_node(
        config.runner.as_deref(),
        signer.clone(),
        receipts.clone(),
        &cas,
        pool.clone(),
    )?;

    // Claim (control-plane lane), checked from the sandbox through the broker (R3.1).
    let claims = Claims::new(
        pool.clone(),
        Arc::new(crate::broker::claim_probe::BrokerProbe::new(
            broker.clone(),
            pool.clone(),
        )),
        Arc::new(wiring::DbSites { pool: pool.clone() }),
        Arc::new(wiring::ClaimReceipts {
            store: receipts.clone(),
        }),
        ClaimConfig {
            fixture_domain: config.fixture_domain.clone(),
        },
    );

    // The Move's go-live gate and decisions (move lane, design §4.5, §6).
    let release_master = key_file(&config.data_dir.join("keys").join("release-master.key"))?;
    let logs = Arc::new(crate::launch::SiteLogs::new(
        crate::launch::ReleaseKeys::new(release_master),
    ));
    let move_events = Arc::new(crate::launch::ports::MoveEvents::new(pool.clone()));
    let gate = crate::launch::LaunchGate {
        store: agentrix_move::MoveStore::new(pool.clone()),
        cas: cas.clone(),
        receipts: receipts.clone(),
        events: move_events.clone(),
        logs,
        probes: (10, std::time::Duration::from_secs(30)),
        // After go-live, in the background: W3's billing records go-live day and W11 drafts
        // pictures for pages that had none (handover-wiring 1.3).
        after_live: Some(Arc::new(crate::launch::after_live::NodeAfterLive::new(
            ctx.clone(),
        ))),
    };
    let decisions = crate::decisions::DecisionsState {
        store: agentrix_move::MoveStore::new(pool.clone()),
        receipts: receipts.clone(),
        gate: gate.clone(),
        approver: Arc::new(auth.approvals()),
        sessions: Arc::new(auth.clone()),
        events: move_events.clone(),
        guards: guards.clone(),
    };

    // Converted sites and the agent door (experience lane).
    // The Mover and owner uploads (move lane): `PUT /sites/{id}/uploads/{file}`,
    // `POST /sites/{id}/moves`; interrupted Moves resume at startup (N9).
    let mover = crate::launch::ports::mover(
        pool.clone(),
        cas.clone(),
        receipts.clone(),
        move_events.clone(),
        broker.clone(),
        &config.models_url,
        http.clone(),
    );
    let moves = crate::decisions::moves::MovesState {
        mover: mover.clone(),
        blobs: crate::launch::ports::NodeBlobs {
            pool: pool.clone(),
            cas: cas.clone(),
        },
        sessions: Arc::new(auth.clone()),
        fixture_exports: std::env::var_os("AGENTRIX_FIXTURE_EXPORTS").map(std::path::PathBuf::from),
        guards: guards.clone(),
    };
    tokio::spawn({
        let mover = mover.clone();
        async move { mover.resume_all().await }
    });

    let site_state = SiteState::new(
        Arc::new(gate.releases()),
        Arc::new(GatewayModel::new(config.models_url.clone(), http.clone())),
        Arc::new(wiring::DbReceiptSink {
            store: receipts.clone(),
        }),
    )
    // "Pass your question to the owner" (R8.3): stored as data, shown on Today.
    .with_questions(Arc::new(crate::db::questions::QuestionStore::new(
        pool.clone(),
    )))
    // The platform's checks before Ask and the agent door act (handover-wiring §2a).
    .with_guards(guards.clone());
    let door = DoorState {
        site: site_state.clone(),
        held: Arc::new(crate::decisions::HeldOrders {
            store: agentrix_move::MoveStore::new(pool.clone()),
        }),
    };

    // The console (console lane).
    let reads = SqliteReads::new(
        pool.clone(),
        receipts.keys().clone(),
        wiring::NodeDelegates {
            auth: auth.clone(),
            pool: pool.clone(),
            cas: cas.clone(),
            decisions: decisions.clone(),
        },
    );
    let shared = Shared {
        signer: signer.clone(),
    };

    // W4's onboarding pages (agentrix-onboarding), drawn as the concepts: Assess, the claim and
    // the Move. Their styles and scripts come through agentrix_onboarding::mount (wiring/host.rs).
    // Once a site is live, the Move page asks where the WebAgents should reach the owner, offering
    // only the channels W13's agentrix-channels can connect for that site today.
    let onboarding = Arc::new(
        agentrix_onboarding::Pages::new()
            .context("loading the onboarding templates")?
            .with_channels({
                let ctx = ctx.clone();
                move |site: &str| agentrix_channels::connectable(&ctx, site)
            }),
    );
    let pages = PagesState {
        console: console.clone(),
        reads: Arc::new(reads),
        onboarding: Some(onboarding.clone()),
        concepts: Some(pool.clone()),
    };
    // The operator console's host (W3): a second sign-in context whose origin is the ops host
    // and whose passkey RP id stays the platform's, so an operator's passkey works there. Its own
    // sessions and cookie keep operator and owner sign-ins apart.
    let ops_host = agentrix_operator::ops_host(&config.platform_host());
    let ops_auth = {
        let mut cfg =
            AuthConfig::for_origin(agentrix_operator::ops_origin(&config.origin, &ops_host))?;
        cfg.rp_id = config.platform_host();
        AuthState::new(
            cfg,
            pool.clone(),
            crate::auth::Sealer::from_key(key_file(&config.seal_key_path())?),
        )?
    };
    let ops_ctx = agentrix_host::NodeContext {
        approvals: Arc::new(wiring::host::NodeApprovals {
            approvals: ops_auth.approvals(),
        }),
        ..ctx.clone()
    };
    wiring::host::run_migrations(&pool, wiring::host::MIGRATIONS).await?;
    let modules = wiring::host::mount_all(&[
        (&ctx, wiring::host::MODULES),
        (&ops_ctx, wiring::host::OPS_MODULES),
    ])?;

    // Assess (assess lane): `/`, `/assess`, `/assess/fixture`, `/assess/{id}`, its events.
    let assess = crate::assess::router(
        crate::assess::AssessState::for_node(
            broker.clone(),
            pool.clone(),
            console.clone(),
            crate::assess::AssessConfig::from_env()
                .with_fixture_domain(config.fixture_domain.as_deref()),
        )
        .with_pages(onboarding.clone()),
    );

    // The gauntlet (gauntlet lane): receipt listing and recorded runs, verdicts recomputed
    // from verified receipts (R13.2).
    let gauntlet = agentrix_gauntlet::server::router(Arc::new(wiring::gauntlet::GauntletBackend {
        receipts: receipts.clone(),
        pool: pool.clone(),
        auth: auth.clone(),
        token: config
            .gauntlet_token_file
            .as_deref()
            .and_then(wiring::gauntlet::GauntletBackend::read_token),
    }));

    // Platform routes.
    let platform = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/.well-known/agentrix/keys", get(keys))
        .with_state(shared)
        .merge(crate::auth::router(auth.clone()))
        .merge(crate::claim::router(ClaimState {
            claims,
            auth: auth.clone(),
            console: console.clone(),
            pages: Some(onboarding.clone()),
        }))
        .merge(crate::pages::router(pages))
        .merge(assess)
        .merge(crate::decisions::router(decisions))
        .merge(crate::decisions::moves::router(moves))
        .merge(crate::decisions::watch::router(
            crate::decisions::watch::WatchState {
                store: agentrix_move::MoveStore::new(pool.clone()),
                events: move_events.clone(),
                sessions: Arc::new(auth.clone()),
            },
        ))
        .merge(gauntlet)
        // The console's stylesheet and scripts (`/static/*`, immutable, versioned by hash).
        .merge(agentrix_console::assets())
        .merge(modules.platform);

    let sites = crate::site::router(site_state.clone())
        .merge(crate::door::router(door))
        // W1: every site host serves the living marks same-origin.
        .merge(agentrix_kernel::site_routes())
        // W8: visits counted without cookies (POST /_agx/visit, GET /_agx/visit.js); the
        // Move's releases carry the counter's tag (handover-wiring 3.1).
        .merge(agentrix_store::visits::beacon_routes(
            agentrix_store::visits::VisitStore::new(pool.clone()),
            Arc::new(wiring::ServedSites {
                releases: Arc::new(gate.releases()),
                pool: pool.clone(),
            }),
        ))
        // W3: platform holds, per request: a notice hold serves only the neutral notice
        // (handover-wiring 3.2).
        .layer(axum::middleware::from_fn_with_state(
            site_state.clone(),
            crate::site::holds,
        ));

    let dispatch = Dispatch {
        platform_host: config.platform_host(),
        platform,
        sites,
        site_state: site_state.clone(),
        modules: modules.hosts.clone(),
    };
    let public = Router::new().fallback(by_host).with_state(dispatch);
    let internal = wiring::host::internal_router(site_state, modules.hosts).merge(modules.internal);

    // The Chief WebAgent's brief for Today (S2-04): refreshed every 15 minutes; it writes nothing
    // and calls no model when nothing has changed. W2's records hold the posted rate its AI
    // spend line is shown in.
    let chief = Arc::new(
        crate::brief::ChiefBrief::new(
            pool.clone(),
            receipts.clone(),
            Arc::new(crate::brief::GatewayHeadline::new(
                config.models_url.clone(),
                http.clone(),
            )),
        )
        .with_guards(guards.clone())
        .with_records_at(&config.data_dir.join("agentrix-samples").join("records.db")),
    );
    chief.clone().spawn(std::time::Duration::from_secs(900));

    // routing.changed receipts (R11.4, R11.6): the gateway's routes, checked at startup and every
    // 15 minutes; a provider switch is configuration plus this receipt on every owned site.
    Arc::new(crate::routing::RoutingWatch::new(
        pool.clone(),
        receipts.clone(),
        http.clone(),
        &config.models_url,
        std::env::var("AGENTRIX_ROUTING_REASON").ok(),
    ))
    .spawn(std::time::Duration::from_secs(900));

    if let Some(store) = objects {
        tracing::info!(
            bucket = store.bucket(),
            "mirroring releases and evidence to Object Storage"
        );
        Arc::new(crate::objects::ObjectMirror::new(
            pool.clone(),
            cas.clone(),
            store,
        ))
        .spawn(std::time::Duration::from_secs(60));
    }

    for (port, owner) in wiring::pending() {
        tracing::warn!(port, owner, "port answered by a pending adapter");
    }
    Ok(App { public, internal })
}

/// Sends each request to the platform or to a converted site by its `Host`.
async fn by_host(State(d): State<Dispatch>, request: Request) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(crate::site::normalise_host)
        .unwrap_or_default();
    let is_platform =
        host == d.platform_host || host == "localhost" || host == "127.0.0.1" || host.is_empty();
    let module = if is_platform {
        None
    } else {
        match d.modules.router_for(&host).await {
            Ok(router) => router,
            Err(e) => {
                tracing::error!(error = %e, %host, "module host lookup failed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "This site can't be reached right now.",
                )
                    .into_response();
            }
        }
    };
    let router = if is_platform {
        d.platform
    } else if let Some(router) = module {
        router
    } else {
        match crate::site::serves_host(&d.site_state, &host).await {
            Ok(true) => d.sites,
            Ok(false) => {
                return (StatusCode::NOT_FOUND, "This site isn't served here.").into_response();
            }
            Err(e) => {
                tracing::error!(error = %e, %host, "release lookup failed");
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "This site can't be reached right now.",
                )
                    .into_response();
            }
        }
    };
    match router.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// `GET /.well-known/agentrix/keys`: the receipt key set (design §4.4).
async fn keys(State(s): State<Shared>) -> impl IntoResponse {
    Json(serde_json::json!({ "keys": [s.signer.published_key()] }))
}
