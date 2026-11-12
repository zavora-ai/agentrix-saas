//! The owner's way in, from the address bar to a live site (W4, `specs/platform/00-plan.md`).
//!
//! Each page is drawn in `concept/` and runs on node's real data. This first slice brings
//! Start and Assess up to `concept/assess.html`: the address bar on its own at `GET /assess`
//! (the home page at `/` belongs to W10 and posts the same form), the pages read in sealed rooms
//! with the room's receipt ticking, the honest health report, and "Keep it".
//!
//! Server-rendered in minijinja's strict mode, like `agentrix-console`: a field a template
//! expects but the view lacks is an error, never a blank on screen (the honesty rule).
//!
//! ```no_run
//! use agentrix_onboarding::{Pages, view};
//! let pages = Pages::new()?;
//! let html = pages.render_start(&view::StartPage::default())?;
//! # Ok::<(), agentrix_onboarding::Error>(())
//! ```
//!
//! Node wires it in two places: [`mount`] in `app.rs` (the static files), and Assess's handlers
//! in `agentrix-node/src/assess/` rendering [`Pages::render_start`] and [`Pages::render_assess`]
//! from the views they already build, until Assess itself moves into this crate (02 §5.1).

mod assets;
pub mod billing;
pub mod pay;
pub mod routes;
pub mod store;
pub mod view;

pub use store::migrate;

pub use assets::{ONBOARDING_ASSETS_VERSION, assets};

use minijinja::{AutoEscape, Environment, Output, State, UndefinedBehavior, Value};
use serde::Serialize;

/// Mounts onboarding on the node (contract C11), on the platform host: the pages' stylesheet
/// and scripts, and the account and setup pages ([`routes`]). Assess's and the claim's routes
/// stay in node's `assess/` and `claim/` for now and render with [`Pages`].
pub fn mount(ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    let pages = std::sync::Arc::new(Pages::new()?);
    let effects = agentrix_effects::Effects::for_node(ctx)?;
    Ok(agentrix_host::Mounted::default()
        .platform(assets())
        .platform(routes::router(
            pages,
            ctx.owners.clone(),
            ctx.pool.clone(),
            plans(),
        ))
        .platform(pay::router(pay::PayRoutes::new(ctx, effects.clone())?))
        .platform(billing::router(billing::BillingRoutes::new(ctx, effects)?)))
}

/// The plans an owner can choose, read from W3's catalogue (`agentrix_billing::catalogue()`),
/// never typed here.
pub fn plans() -> Vec<view::PlanInput> {
    agentrix_billing::catalogue()
        .plans
        .iter()
        .map(|p| view::PlanInput {
            id: p.id.clone(),
            name: p.name.clone(),
            kind: p.kind.clone(),
            price_cents: p.price_cents,
            annual_cents: if p.kind == "site" {
                p.annual_cents()
            } else {
                0
            },
            included_ai_micro_usd: p.included_ai_micro_usd,
            webagents: p.webagents.clone(),
            includes: p.includes.clone(),
        })
        .collect()
}

/// Every page this crate renders, one template each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardingPage {
    /// `GET /assess`, and `POST /assess` when the address can't be checked (O1.1).
    /// View: [`view::StartPage`].
    Start,
    /// `GET /assess/{id}`: reading, the report and "Keep it", or why there's no report
    /// (O1.2 to O1.4, O1.8). View: [`view::AssessPage`].
    Assess,
    /// `GET /sites/{id}/claim`: prove the domain is yours by DNS record or file (O4.1).
    /// View: [`view::ClaimPage`].
    Claim,
    /// `GET /account/new|signin|recover`: create the account, sign in, recover (O2.1, O2.3).
    /// View: [`view::AccountPage`].
    Account,
    /// `GET /setup`: the Chief WebAgent introduces itself and says what's next (O2.4).
    /// View: [`view::SetupPage`].
    Setup,
    /// `GET /console/move`: the Move live, go-live and undo (O5.1 to O5.5, O6.2).
    /// View: [`view::MovePage`].
    Move,
    /// `GET /setup/plan`: choose a plan and the AI budget, then PayPal (O3.1, O3.2).
    /// View: [`view::PlanPage`].
    Plan,
    /// `GET /setup/pay`: the plan's PayPal subscription, just before go-live (O3.2).
    /// View: [`view::PayPage`].
    Pay,
    /// `GET /billing`: the plan, AI spend, plans, cancel and export (O9).
    /// View: [`view::BillingPage`].
    Billing,
}

impl OnboardingPage {
    /// Every page, for checks that each template loads.
    pub const ALL: [OnboardingPage; 9] = [
        OnboardingPage::Start,
        OnboardingPage::Assess,
        OnboardingPage::Claim,
        OnboardingPage::Account,
        OnboardingPage::Setup,
        OnboardingPage::Move,
        OnboardingPage::Plan,
        OnboardingPage::Pay,
        OnboardingPage::Billing,
    ];

    /// The template that renders this page.
    pub fn template(self) -> &'static str {
        match self {
            OnboardingPage::Start => "onboarding/start.html",
            OnboardingPage::Assess => "onboarding/assess.html",
            OnboardingPage::Claim => "onboarding/claim.html",
            OnboardingPage::Account => "onboarding/account.html",
            OnboardingPage::Setup => "onboarding/setup.html",
            OnboardingPage::Move => "onboarding/move.html",
            OnboardingPage::Plan => "onboarding/plan.html",
            OnboardingPage::Pay => "onboarding/pay.html",
            OnboardingPage::Billing => "onboarding/billing.html",
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
        page: OnboardingPage,
        /// What minijinja reported, with the template line.
        #[source]
        source: minijinja::Error,
    },
}

/// HTML-escapes text (`& < > " '`) without minijinja's `/` → `&#x2f;`, so URLs in the page
/// source stay readable. Strict mode still refuses an undefined value before this runs.
fn format(out: &mut Output, state: &State, value: &Value) -> Result<(), minijinja::Error> {
    match (state.auto_escape(), value.as_str()) {
        (AutoEscape::Html, Some(text)) if !value.is_safe() => {
            let mut done = 0;
            for (i, byte) in text.bytes().enumerate() {
                let entity = match byte {
                    b'&' => "&amp;",
                    b'<' => "&lt;",
                    b'>' => "&gt;",
                    b'"' => "&quot;",
                    b'\'' => "&#x27;",
                    _ => continue,
                };
                out.write_str(&text[done..i])?;
                out.write_str(entity)?;
                done = i + 1;
            }
            Ok(out.write_str(&text[done..])?)
        }
        _ => minijinja::escape_formatter(out, state, value),
    }
}

/// The onboarding templates. Build once at startup and share.
pub struct Pages {
    env: Environment<'static>,
    /// Which channels the Channels app can connect for a site, when it's registered.
    connectable: Option<Connectable>,
}

/// For a site id, the channel ids W13's Channels app can connect there today.
type Connectable = std::sync::Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

impl std::fmt::Debug for Pages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pages")
            .field("channels", &self.connectable.is_some())
            .finish_non_exhaustive()
    }
}

impl Pages {
    /// Loads every template and fails fast if any does not parse.
    pub fn new() -> Result<Self, Error> {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_formatter(format);
        #[cfg(feature = "dev-reload")]
        env.set_loader(minijinja::path_loader(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates"
        )));
        #[cfg(not(feature = "dev-reload"))]
        minijinja_embed::load_templates!(&mut env);
        env.add_global("onboarding_v", ONBOARDING_ASSETS_VERSION);
        // W7's brand.css: the self-hosted Manrope, DM Sans and JetBrains Mono, and the tokens.
        env.add_global("ui_v", agentrix_ui::VERSION);
        // Off until node says W13's Channels app is registered ([`Pages::with_channels`]).
        env.add_global("channels_on", false);
        env.add_global("reach_channels", Value::from(Vec::<Value>::new()));
        for page in OnboardingPage::ALL {
            env.get_template(page.template())
                .map_err(|source| Error::Load {
                    template: page.template(),
                    source,
                })?;
        }
        Ok(Self {
            env,
            connectable: None,
        })
    }

    /// Shows the optional "Where should your WebAgents reach you?" step once a site is live.
    /// Node calls this when W13's Channels app (id `channels`) is registered, the same check
    /// W5's Settings card makes, with W13's per-site check, for example
    /// `move |site| agentrix_channels::connectable(&ctx, site)`: the channels it returns get a
    /// Connect button (the app's `?connect=<channel>`); the rest say "Not available yet". Email is
    /// always on.
    pub fn with_channels<F, S>(mut self, connectable: F) -> Self
    where
        F: Fn(&str) -> Vec<S> + Send + Sync + 'static,
        S: Into<String>,
    {
        self.connectable = Some(std::sync::Arc::new(move |site| {
            connectable(site).into_iter().map(Into::into).collect()
        }));
        self.env.add_global("channels_on", true);
        self
    }

    /// Renders `page` with `view`, which must be the page's view model (see [`OnboardingPage`]).
    pub fn render(&self, page: OnboardingPage, view: &impl Serialize) -> Result<String, Error> {
        let template = self
            .env
            .get_template(page.template())
            .map_err(|source| Error::Render { page, source })?;
        template
            .render(view)
            .map_err(|source| Error::Render { page, source })
    }

    /// The address bar page (O1.1).
    pub fn render_start(&self, view: &view::StartPage) -> Result<String, Error> {
        self.render(OnboardingPage::Start, view)
    }

    /// The Assess page in whatever state the run is in (O1.2 to O1.4, O1.8).
    pub fn render_assess(&self, view: &view::AssessPage) -> Result<String, Error> {
        self.render(OnboardingPage::Assess, view)
    }

    /// The Move page (O5, O6.2).
    pub fn render_move(&self, view: &view::MovePage) -> Result<String, Error> {
        let Some(connectable) = &self.connectable else {
            return self.render(OnboardingPage::Move, view);
        };
        let ready = connectable(&view.site_id);
        let channels = view::ChannelOffer::from_ready(&ready).channels();
        let ctx = minijinja::context! {
            reach_channels => Value::from_serialize(channels),
            ..Value::from_serialize(view)
        };
        self.render(OnboardingPage::Move, &ctx)
    }

    /// The claim page (O4.1).
    pub fn render_claim(&self, view: &view::ClaimPage) -> Result<String, Error> {
        self.render(OnboardingPage::Claim, view)
    }
}
