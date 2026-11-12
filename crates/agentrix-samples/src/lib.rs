//! The three sample websites (W2): Kilele Bakery, Classy Perfumes and Tana Valley Times.
//!
//! Each runs on its own `-sample` host (`kilele-bakery-sample.agentrix.website`), marked Sample
//! on every page, on records loaded from a bundle in `fixtures/samples/` (embedded in the
//! binary). Pages are server-rendered from those records; Ask, the desks, the agent door, the
//! bag, checkout and orders are routes on the same host. The owner's apps for each sample are
//! on the platform host under `/samples/<site>/`.
//!
//! Node mounts it with one line: `agentrix_samples::mount(&ctx)?` (contract C11).
//!
//! What this crate needs from other workstreams comes through ports, each with a stand-in
//! that keeps the samples working until the real one is wired:
//! - [`ports::Visitors`]: customer sign-in on the site's own host (W6). Stand-in: an email
//!   one-time code shown on screen as the "Sample mailbox".
//! - [`ports::Payments`]: PayPal sandbox through the Effects Gateway (W6). Stand-in: a
//!   clearly labelled sample payment step that moves no money.
//! - [`ports::AnswerModel`]: the Model Gateway's `concierge` purpose. When it can't be
//!   reached, Ask answers from the site's own text, quoted and cited.

#![recursion_limit = "256"]

pub mod app;
mod ask;
mod bakery;
mod classy;
mod commerce;
mod common;
mod door;
pub mod fulfilment;
pub mod held;
mod identity;
mod knowledge;
mod news;
mod notify;
mod owner;
pub mod ports;
pub mod proof;
mod shell;
mod studio;
mod subject;
mod web;

use std::sync::Arc;

use agentrix_host::{HostMatch, Mounted, NodeContext};

pub use app::{App, Config};
pub use knowledge::{Knowledge, Passage, knowledge_search};
pub use ports::set_payments;
pub use proof::{
    AgentProof, DoorRefusal, ProofFailure, ProofRequest, ProvenAgent, set_agent_proof,
};

/// The sample sites: slug (the host's first label) and bundle folder.
pub const SAMPLES: [(&str, &str); 3] = [
    ("kilele-bakery-sample", "kilele-bakery"),
    ("classy-perfumes-sample", "classy-perfumes"),
    ("tana-valley-times-sample", "tana-valley-times"),
];

/// Mounts the sample sites on their hosts and the owner apps on the platform host.
///
/// # Errors
///
/// When the data directory can't be prepared.
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    mount_with(ctx, Config::from_context(ctx))
}

/// [`mount`] with an explicit configuration (the preview server). Customer sign-in is W6's
/// visitor accounts; their pages render inside each sample's own shell. Node runs
/// `agentrix_identity::migrate` on `control.db`.
///
/// # Errors
///
/// When the data directory can't be prepared.
pub fn mount_with(ctx: &NodeContext, config: Config) -> anyhow::Result<Mounted> {
    let (visitors, shell) = identity::visitors(ctx);
    mount_visitors(ctx, config, visitors, shell)
}

/// [`mount`] with W9's mailer, so customers' sign-in codes and notices go through the message
/// catalogue and the outbox instead of the log-only mailer.
///
/// # Errors
///
/// When the data directory can't be prepared.
pub fn mount_with_mailer(
    ctx: &NodeContext,
    mailer: Arc<dyn agentrix_identity::CodeMailer>,
) -> anyhow::Result<Mounted> {
    let (visitors, shell) = identity::visitors(ctx);
    mount_visitors(
        ctx,
        Config::from_context(ctx),
        visitors.with_mailer(mailer),
        shell,
    )
}

fn mount_visitors(
    ctx: &NodeContext,
    config: Config,
    visitors: agentrix_identity::Visitors,
    shell: Arc<identity::SamplesShell>,
) -> anyhow::Result<Mounted> {
    let app =
        App::new(config, ctx)?.with_visitors(Arc::new(identity::W6Visitors(visitors.clone())));
    shell.set(app.clone());
    // The Chief WebAgent searches the samples' own pages, the same sections Ask uses (W1). The
    // first registration wins, so node needs no line of its own.
    agentrix_kernel::chief::set_site_knowledge(Arc::new(knowledge::Knowledge(app.clone())));
    // "Download my data" and "Delete my account" reach W2's orders, receipts and addresses.
    let visitors = visitors.with_subject_store(Arc::new(subject::SampleRecords(app.clone())));
    // ...and control.db's questions passed on and orders placed by agents (W8).
    let visitors = visitors.with_subject_store(Arc::new(
        agentrix_store::subject::ControlSubjectData::new(ctx.pool.clone()),
    ));
    Ok(mounted(
        app,
        visitors
            .router()
            .merge(agentrix_media::public_router(ctx))
            // W1's mark generator (/_agentrix/marks.js, marks.css), the same one the console uses.
            .merge(agentrix_marks::assets::router()),
    ))
}

/// The routes and tasks for an [`App`]; `extra` is merged into the sample hosts' router.
pub fn mounted(app: App, extra: axum::Router) -> Mounted {
    held::register(&app);
    let hosts = app.config.sample_hosts();
    // Visit counts without trackers (06 §4.9, W8): POST /_agx/visit and /_agx/visit.js on
    // every sample host, into control.db's visit_counts under the site's slug. No cookie, no
    // identifier; Global Privacy Control and the footer's switch turn it off.
    let sites = app.sites.clone();
    let counter = agentrix_store::visits::beacon_routes(
        agentrix_store::visits::VisitStore::new(app.control.clone()),
        Arc::new(agentrix_store::visits::FnHostSites(move |host: &str| {
            let slug = host.split('.').next()?;
            let site = sites.read().ok()?.get(slug)?.clone();
            Some((agentrix_types::SiteId::new(site.site), site.utc_offset_min))
        })),
    );
    let extra = extra.merge(counter);
    let warm = app.clone();
    let daily = app.clone();
    Mounted::default()
        .host(
            HostMatch::AnyOf(hosts),
            web::site_router(app.clone(), extra),
        )
        .platform(owner::router(app))
        .task(
            "samples: load bundles",
            Box::pin(async move {
                if let Err(e) = warm.records().await {
                    tracing::error!(error = %format!("{e:#}"), "sample bundles failed to load");
                }
            }),
        )
        .task(
            "samples: lapse stale requests",
            Box::pin(async move { daily.lapse_loop().await }),
        )
}

/// A shared app handle.
pub type Shared = Arc<App>;
