//! The owner console as Agentrix OS (07 §4.2, `concept/console/shell.html`; W5).
//!
//! One frame for everything the owner does: the status bar (site chip, worlds, the time pill,
//! the budget and trust pills, Freeze), the dock (WebAgents with their marks, the system apps,
//! then registered apps), the field, the lane (what needs you) and the command bar. Day Time and
//! Night Time follow the site's quiet hours; Night Time is the one dark theme. Below 900 px the
//! same shell reflows into the phone's order (status bar, lane, field, command bar, dock).
//!
//! Pages read through [`port::ShellPort`] (the first release's reads, implemented in `agentrix-node`), and
//! the shell keeps its own rows in [`store::ShellStore`]. Other workstreams add pages with
//! [`apps::ConsoleApp`] (contract C9). Routes, all under `/console/{site}`:
//!
//! | Route | Page |
//! |---|---|
//! | `GET /console/{site}` | Back office (Day Time), Night Time and Front of house |
//! | `GET /console/{site}/decisions`, `…/decisions/{id}` | Every card; one card |
//! | `GET /console/{site}/cards/{id}/sheet` | The approval sheet's JSON, with a fresh ceremony |
//! | `GET /console/{site}/webagents`, `…/trust`, `…/quarantine`, `…/settings` | Pages in the shell |
//! | `GET /console/{site}/orders`, `…/orders/{id}` | Orders: every order, by state; one order ([`commerce`]) |
//! | `GET /console/{site}/customers`, `…/customers/{key}` | Customers: everyone the site served, with search; one customer |
//! | `POST /console/{site}/freeze` | Freeze: no signature needed |
//! | `POST /console/{site}/unfreeze/sheet`, `POST /console/{site}/unfreeze` | Unfreeze: an approval |
//! | `POST /console/{site}/settings/quiet-hours` | Change quiet hours |
//! | `GET, POST /console/{site}/apps/{app}/…` | Registered apps (C9) |
//! | `GET /os` | The signed-in owner's shell |
//! | `GET /os/{page}` | The same page of the signed-in owner's shell (`/os/decisions`), for links that don't know the site, such as the first release's old routes and messages |
//! | `GET /static/os/*` | Stylesheets and scripts |
//!
//! Approving a card posts to the card's own approve URL (the first release's frozen contract, design
//! §4.1). The shell never approves anything itself.

pub mod apps;
mod assets;
pub mod budget;
pub mod commerce;
mod commerce_pages;
pub mod control;
pub mod live;
pub mod map;
pub mod missions;
pub mod port;
mod routes;
pub mod sample;
pub mod schedule;
pub mod store;
pub mod store_app;

pub use store_app::runners;
pub mod view;

use std::sync::Arc;

pub use assets::ASSETS_VERSION;

use minijinja::Environment;
use serde::Serialize;

/// Every template the shell renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellPage {
    /// [`view::HomeView`].
    Home,
    /// [`view::DecisionsView`].
    Decisions,
    /// [`view::DecisionView`].
    Decision,
    /// [`view::WebAgentsView`].
    WebAgents,
    /// [`view::TrustView`].
    Trust,
    /// [`view::QuarantineView`].
    Quarantine,
    /// [`view::SettingsView`].
    Settings,
    /// [`view::WebsiteView`].
    Website,
    /// [`view::StoreView`].
    Store,
    /// [`view::WebAgentView`].
    WebAgent,
    /// [`view::MemoryView`].
    Memory,
    /// [`view::GesturesView`].
    Gestures,
    /// [`view::MissionsView`].
    Missions,
    /// [`view::MissionView`].
    Mission,
    /// [`view::LiveView`].
    Live,
    /// [`view::LiveThreadView`].
    LiveThread,
    /// [`view::WalkView`].
    Walk,
    /// [`view::AppsView`].
    Apps,
    /// [`view::AppView`].
    App,
    /// [`view::OrdersView`].
    Orders,
    /// [`view::OrderView`].
    Order,
    /// [`view::CustomersView`].
    Customers,
    /// [`view::CustomerView`].
    Customer,
}

impl ShellPage {
    /// Every page, for the check that each template loads.
    pub const ALL: [ShellPage; 23] = [
        ShellPage::Orders,
        ShellPage::Order,
        ShellPage::Customers,
        ShellPage::Customer,
        ShellPage::Apps,
        ShellPage::Live,
        ShellPage::LiveThread,
        ShellPage::Walk,
        ShellPage::Missions,
        ShellPage::Mission,
        ShellPage::Gestures,
        ShellPage::Memory,
        ShellPage::WebAgent,
        ShellPage::Store,
        ShellPage::Website,
        ShellPage::Home,
        ShellPage::Decisions,
        ShellPage::Decision,
        ShellPage::WebAgents,
        ShellPage::Trust,
        ShellPage::Quarantine,
        ShellPage::Settings,
        ShellPage::App,
    ];

    /// The template that renders this page.
    pub fn template(self) -> &'static str {
        match self {
            ShellPage::Home => "os/home.html",
            ShellPage::Decisions => "os/decisions.html",
            ShellPage::Decision => "os/decision.html",
            ShellPage::WebAgents => "os/webagents.html",
            ShellPage::Trust => "os/trust.html",
            ShellPage::Quarantine => "os/quarantine.html",
            ShellPage::Settings => "os/settings.html",
            ShellPage::Website => "os/website.html",
            ShellPage::Store => "os/store.html",
            ShellPage::WebAgent => "os/webagent.html",
            ShellPage::Memory => "os/memory.html",
            ShellPage::Gestures => "os/gestures.html",
            ShellPage::Missions => "os/missions.html",
            ShellPage::Mission => "os/mission.html",
            ShellPage::Live => "os/live.html",
            ShellPage::LiveThread => "os/live-thread.html",
            ShellPage::Walk => "os/walk.html",
            ShellPage::Apps => "os/apps.html",
            ShellPage::App => "os/app.html",
            ShellPage::Orders => "os/orders.html",
            ShellPage::Order => "os/order.html",
            ShellPage::Customers => "os/customers.html",
            ShellPage::Customer => "os/customer.html",
        }
    }
}

/// Why a page could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A template failed to load or parse.
    #[error("loading template {template}: {source}")]
    Load {
        /// The template's name.
        template: &'static str,
        /// What minijinja reported.
        #[source]
        source: minijinja::Error,
    },
    /// Rendering failed, usually because the view lacks a field the template uses.
    #[error("rendering {page:?}: {source}")]
    Render {
        /// The page being rendered.
        page: ShellPage,
        /// What minijinja reported, with the template line.
        #[source]
        source: minijinja::Error,
    },
}

/// The shell's templates. Build once at startup and share.
#[derive(Debug)]
pub struct Shell {
    env: Environment<'static>,
}

impl Shell {
    /// Loads every template and fails fast if any does not parse.
    pub fn new() -> Result<Self, Error> {
        // agentrix-ui's environment: strict undefined, the escaping formatter, the kit's
        // templates (ui/…), its words and asset globals. The shell's own templates live under os/.
        let mut env = agentrix_ui::environment();
        #[cfg(feature = "dev-reload")]
        env.set_loader(minijinja::path_loader(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates"
        )));
        #[cfg(not(feature = "dev-reload"))]
        minijinja_embed::load_templates!(&mut env);
        env.add_global("asset_v", ASSETS_VERSION);
        for page in ShellPage::ALL {
            env.get_template(page.template())
                .map_err(|source| Error::Load {
                    template: page.template(),
                    source,
                })?;
        }
        Ok(Self { env })
    }

    /// Renders `page` with `view`, which must be the page's view model (see [`ShellPage`]).
    pub fn render(&self, page: ShellPage, view: &impl Serialize) -> Result<String, Error> {
        let template = self
            .env
            .get_template(page.template())
            .map_err(|source| Error::Render { page, source })?;
        template
            .render(view)
            .map_err(|source| Error::Render { page, source })
    }
}

/// What the shell's routes need.
#[derive(Clone)]
pub struct ShellState {
    /// The templates.
    pub shell: Arc<Shell>,
    /// Reads and approvals, from the platform.
    pub port: Arc<dyn port::ShellPort>,
    /// The shell's own rows.
    pub store: store::ShellStore,
    /// Apps registered by other workstreams (C9), in any order; the dock sorts them.
    pub apps: Vec<Arc<dyn apps::ConsoleApp>>,
    /// Lane items other workstreams own (C9), such as support-access requests.
    pub lanes: Vec<Arc<dyn apps::LaneSource>>,
    /// Whether Freeze is offered. Only once the paths it stops check it (handover-wiring.md
    /// §2b); `mount` reads `AGENTRIX_FREEZE_ENFORCED=1`, which W0 sets then.
    pub freeze_enabled: bool,
}

/// Whether Freeze is switched on (`AGENTRIX_FREEZE_ENFORCED` is 1, true or yes): only then do
/// the console, gestures and any other surface (a chat's STOP) offer it, because only then do
/// the paths it stops check it.
pub fn freeze_enforced() -> bool {
    std::env::var("AGENTRIX_FREEZE_ENFORCED")
        .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
}

/// What other workstreams plug into the shell (contract C9).
#[derive(Default, Clone)]
pub struct Plugins {
    /// Apps in the dock, each with pages inside the shell.
    pub apps: Vec<Arc<dyn apps::ConsoleApp>>,
    /// Items in the lane's "Needs you" that link to another crate's page.
    pub lanes: Vec<Arc<dyn apps::LaneSource>>,
}

/// The shell's routes and its static assets.
pub fn router(state: ShellState) -> axum::Router {
    routes::router(state).merge(assets::assets())
}

pub use store::migrate;

/// Mounts the owner console on the platform host (contract C11), with nothing plugged in.
pub fn mount(ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    mount_with(ctx, Plugins::default())
}

/// Mounts the owner console with what other workstreams plug in (C9): node builds each app and
/// lane source from its crate and passes them here.
pub fn mount_with(
    ctx: &agentrix_host::NodeContext,
    plugins: Plugins,
) -> anyhow::Result<agentrix_host::Mounted> {
    // W2's sample sites keep their orders, currency and price lists in
    // `<data>/agentrix-samples/records.db` (agentrix-samples `Config::from_context`): missions
    // count orders from there, and the AI budget is shown in the sample's currency.
    let store = store::ShellStore::new(ctx.pool.clone())
        .with_records_at(&ctx.data_dir.join("agentrix-samples").join("records.db"));
    let state = ShellState {
        shell: Arc::new(Shell::new()?),
        port: Arc::new(
            control::ControlPort::new(ctx.clone()).with_records(store.records().cloned()),
        ),
        store,
        apps: plugins.apps,
        lanes: plugins.lanes,
        freeze_enabled: freeze_enforced(),
    };
    Ok(agentrix_host::Mounted::default().platform(router(state)))
}

/// Whether a site is frozen, for the paths that must stop while it is (the brief, Ask, the
/// agent door, the Move). Node hands one to each of them.
pub fn freeze_gate(ctx: &agentrix_host::NodeContext) -> store::FreezeGate {
    store::FreezeGate::new(store::ShellStore::new(ctx.pool.clone()))
}
