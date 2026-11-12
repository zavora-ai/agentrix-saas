//! Console and public templates ported from `concept/` (design §8, D5).
//!
//! **Deprecated: the owner pages here (Today, Decisions, Decision, Trust, WebAgents,
//! Quarantine) are the first release's five-tab console, replaced by Agentrix OS at `/os`
//! (`agentrix-shell`).** Node answers their old routes with a 308 to the `/os` page (see
//! `agentrix_node::pages`). The receipt page stays (it is public), and so does the Move page
//! for a node without W4's Move (`agentrix-onboarding`). The [`view`] models are still the
//! shared types the shell's port returns.
//!
//! Server-rendered pages with small JS islands and SSE for live parts, on the tokens in
//! `concept/styles.css`. Phone first, light and dark, WCAG 2.2 AA. Task S1-08.
//!
//! `agentrix-node` fills a [`view`] model from the database and `agentrix-types`, then calls
//! [`Console::render`]. Templates run in minijinja's strict mode: a field a template expects
//! but the view lacks is an error, never a blank on screen (the honesty rule).
//!
//! ```no_run
//! use agentrix_console::{Console, Page, view};
//! let console = Console::new()?;
//! let html = console.render(Page::Start, &view::StartView::default())?;
//! # Ok::<(), agentrix_console::Error>(())
//! ```

mod assets;
pub mod view;

pub use assets::{ASSETS_VERSION, assets};

use minijinja::{AutoEscape, Environment, Output, State, UndefinedBehavior, Value};
use serde::Serialize;

/// Every page the console renders, one template each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    /// `GET /`: paste your URL (R1.1). View: [`view::StartView`].
    Start,
    /// `GET /assess/{id}`: live receipt, frames and the report (R1). View: [`view::AssessView`].
    Assess,
    /// Sign in, passkey first (R2). View: [`view::AuthView`]. Rendered by `auth/`.
    SignIn,
    /// Create an account, passkey first (R2). View: [`view::AuthView`]. Rendered by `auth/`.
    Register,
    /// DNS TXT or file proof (R3). View: [`view::ClaimView`].
    Claim,
    /// The Move, live (R4–R6, R6.5). View: [`view::MoveView`]. Deprecated: W4's Move page
    /// (`agentrix-onboarding`) serves `/console/move`; this one only without it.
    Move,
    /// The Chief WebAgent's brief (R10.1). View: [`view::TodayView`]. Deprecated: replaced
    /// by `/os` (Back office).
    Today,
    /// Cards waiting and decided (R10.1). View: [`view::DecisionsView`]. Deprecated:
    /// replaced by `/os/decisions`.
    Decisions,
    /// One card, including the go-live card (R7). View: [`view::DecisionView`]. Deprecated:
    /// replaced by the card in `/os` (`/d/{id}`).
    Decision,
    /// Receipts and the gauntlet score (R10.3, R13). View: [`view::TrustView`]. Deprecated:
    /// replaced by `/os/trust`.
    Trust,
    /// One receipt, with a check in the browser (R10.3). View: [`view::ReceiptView`].
    Receipt,
    /// Jobs and dial positions, read-only (R10.2). View: [`view::WebAgentsView`]. Deprecated:
    /// replaced by `/os/webagents`.
    WebAgents,
    /// Items left behind, with restore as a signed decision (S2-06). View: [`view::QuarantineView`].
    /// Deprecated: replaced by `/os/quarantine`.
    Quarantine,
}

impl Page {
    /// Every page, for checks that each template loads.
    pub const ALL: [Page; 13] = [
        Page::Start,
        Page::Assess,
        Page::SignIn,
        Page::Register,
        Page::Claim,
        Page::Move,
        Page::Today,
        Page::Decisions,
        Page::Decision,
        Page::Trust,
        Page::Receipt,
        Page::WebAgents,
        Page::Quarantine,
    ];

    /// The template that renders this page.
    pub fn template(self) -> &'static str {
        match self {
            Page::Start => "start.html",
            Page::Assess => "assess.html",
            Page::SignIn => "auth/signin.html",
            Page::Register => "auth/register.html",
            Page::Claim => "claim.html",
            Page::Move => "console/move.html",
            Page::Today => "console/today.html",
            Page::Decisions => "console/decisions.html",
            Page::Decision => "console/decision.html",
            Page::Trust => "console/trust.html",
            Page::Receipt => "console/receipt.html",
            Page::WebAgents => "console/webagents.html",
            Page::Quarantine => "console/quarantine.html",
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
        page: Page,
        /// What minijinja reported, with the template line.
        #[source]
        source: minijinja::Error,
    },
}

/// Writes a value into the page. Text is HTML-escaped (`& < > " '`), which is enough in text
/// and in quoted attributes. minijinja's default formatter also turns `/` into `&#x2f;`: safe,
/// but every URL in the page source then reads `&#x2f;console&#x2f;today`. Strict mode still
/// refuses an undefined value before this runs.
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

/// The template environment. Build it once at startup and share it.
#[derive(Debug)]
pub struct Console {
    env: Environment<'static>,
}

impl Console {
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
        env.add_global("asset_v", ASSETS_VERSION);
        env.add_global("theme", "auto");
        for page in Page::ALL {
            env.get_template(page.template())
                .map_err(|source| Error::Load {
                    template: page.template(),
                    source,
                })?;
        }
        Ok(Self { env })
    }

    /// Renders `page` with `view`, which must be the page's view model (see [`Page`]).
    pub fn render(&self, page: Page, view: &impl Serialize) -> Result<String, Error> {
        let template = self
            .env
            .get_template(page.template())
            .map_err(|source| Error::Render { page, source })?;
        template
            .render(view)
            .map_err(|source| Error::Render { page, source })
    }
}
