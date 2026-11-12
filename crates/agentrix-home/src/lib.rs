//! The agentrix.website home page: "Upgrade your website into WebAgents."
//!
//! The page tells the submission deck's story (the problem, the Move, containment, go-live,
//! the site that answers people and their AI agents, the console, the architecture on Vultr,
//! the samples, plans and the trust promises) and leads into Assess through the paste box.
//! Every number on it is read from receipts and the database, through W8's metric catalogue
//! ([`store`]) with read-only queries for the details ([`facts`]); where a number
//! doesn't exist yet, the page says what will appear and when.
//!
//! Node mounts it through the host seam (contract C11):
//!
//! - [`mount`] adds every route except `/`: the page at `/home`, its assets under `/_home/`,
//!   `/robots.txt`, `/sitemap.xml`, `/privacy`, `/terms`, and the crawler's page at `/bot` (the
//!   address its User-Agent names) and `/.well-known/agentrix-crawler` (03 §4), and the
//!   living marks under `/_home/marks/` ([`marks`]).
//! - [`mount_with_front_door`] adds the same routes plus `GET /`. The coordinator swaps it in
//!   for [`mount`] once the first release's Start page leaves `/`.
//!
//! Server-rendered with minijinja in strict mode, one small stylesheet on the tokens in
//! `concept/styles.css`, light and dark. The only script is the hero film's play and pause
//! (`static/hero.js`); without it the hero shows its poster and everything else works.
//! The film itself is served from node's disk ([`film`]).

pub mod desks;
pub mod door;
pub mod facts;
pub mod film;
pub mod keep;
pub mod marks;
pub mod newsletter;
pub mod probe;
pub mod store;
pub mod view;

use std::sync::Arc;
use std::time::Duration;

use agentrix_host::{Mounted, NodeContext};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use minijinja::{Environment, UndefinedBehavior};
use serde::Deserialize;

pub use facts::{CachedFacts, FactSource, Facts, SqliteFacts};
pub use keep::migrate;
pub use probe::SampleProbe;
pub use store::StoreFacts;
use view::{Links, Plan, Sample, Wholesale};

/// A version stamp that changes whenever a static file changes; templates append it as `?v=…`.
pub const ASSETS_VERSION: &str = env!("AGENTRIX_HOME_ASSETS_VERSION");

/// Rendering failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A template is missing or broken, or the view lacks a field it prints.
    #[error("template: {0}")]
    Template(#[from] minijinja::Error),
    /// The catalogue in `data/` doesn't parse.
    #[error("catalogue: {0}")]
    Catalogue(#[from] serde_json::Error),
}

/// What the page needs besides the facts.
#[derive(Debug, Clone)]
pub struct HomeConfig {
    /// The platform's origin, e.g. `https://agentrix.website`.
    pub origin: String,
    /// Where the paste box posts: W4's Assess.
    pub assess_action: String,
    /// "Try it on Zavora's infected fixture", when node has a fixture configured.
    pub fixture_href: Option<String>,
    /// The submission video and the public repository, once published.
    pub links: Links,
    /// The platform's host, e.g. `agentrix.website`: the samples live at `{slug}.{host}`.
    pub platform_host: String,
    /// The hero's cut and the film, from `AGENTRIX_HOME_FILM_DIR` (see [`film`]).
    pub film: Option<film::Film>,
    /// "Rather talk to a person? WhatsApp us", once James has published a number and hours.
    pub contact: Option<view::Contact>,
}

impl HomeConfig {
    /// The defaults for `origin`, reading `AGENTRIX_FIXTURE_URL` (as Assess does),
    /// `AGENTRIX_HOME_VIDEO_URL`, `AGENTRIX_HOME_CODE_URL` and `AGENTRIX_HOME_FILM_DIR`.
    pub fn from_env(origin: &str) -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            origin: origin.trim_end_matches('/').to_owned(),
            assess_action: "/assess".into(),
            fixture_href: var("AGENTRIX_FIXTURE_URL").map(|_| "/assess/fixture".into()),
            links: Links {
                video: var("AGENTRIX_HOME_VIDEO_URL"),
                code: var("AGENTRIX_HOME_CODE_URL"),
            },
            platform_host: host_of(origin),
            film: film::Film::from_env(),
            contact: view::Contact::from_env(),
        }
    }
}

/// `agentrix.website` from `https://agentrix.website:443/`.
fn host_of(origin: &str) -> String {
    let rest = origin.split_once("://").map_or(origin, |(_, r)| r);
    let host_port = rest.split('/').next().unwrap_or(rest);
    host_port.split(':').next().unwrap_or(host_port).to_owned()
}

#[derive(Deserialize)]
struct SampleCatalogue {
    samples: Vec<Sample>,
}

/// The three samples, from `data/samples.json`.
fn sample_catalogue() -> Result<Vec<Sample>, Error> {
    let catalogue: SampleCatalogue = serde_json::from_str(include_str!("../data/samples.json"))?;
    Ok(catalogue.samples)
}

/// The home page and its neighbours.
#[derive(Clone)]
pub struct Home {
    inner: Arc<Inner>,
}

struct Inner {
    env: Environment<'static>,
    marks: marks::Marks,
    facts: CachedFacts,
    config: HomeConfig,
    samples: Vec<Sample>,
    plans: Vec<Plan>,
    agency: Option<Wholesale>,
    probe: SampleProbe,
    keep: keep::Keep,
    desks: desks::Desks,
    /// Node's database and CAS, for reading a check's own pages ("Talk about your site").
    node: Option<(sqlx::SqlitePool, Arc<dyn agentrix_host::Blobs>)>,
    /// Questions asked about each check, to keep the model's cost per address bounded.
    talks: std::sync::Mutex<std::collections::HashMap<String, u32>>,
    /// The weekly fix: the Editor drafts, James approves, readers who confirmed get it.
    newsletter: newsletter::Newsletter,
    /// Assess, for the agent door's `start_check`.
    checks: Arc<dyn door::Checks>,
}

fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    // The kit's formatter: `& < > " '` escaped, without `&#x2f;` in every URL.
    env.set_formatter(agentrix_ui::format);
    // A query value, for links that ask a desk a starter question without JavaScript.
    env.add_filter("urlencode", |v: String| -> String {
        v.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                b' ' => "+".to_owned(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    });
    // Indentation in the templates costs bytes on every load.
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    #[cfg(feature = "dev-reload")]
    env.set_loader(minijinja::path_loader(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/templates"
    )));
    #[cfg(not(feature = "dev-reload"))]
    minijinja_embed::load_templates!(&mut env);
    env
}

impl Home {
    /// The page, with facts from `source` cached for a minute.
    pub fn new(config: HomeConfig, source: Arc<dyn FactSource>) -> Result<Self, Error> {
        let samples = sample_catalogue()?;
        let (plans, agency) = view::plans(agentrix_billing::catalogue());
        Ok(Self {
            inner: Arc::new(Inner {
                env: environment(),
                marks: marks::Marks::build(),
                facts: CachedFacts::new(source, Duration::from_secs(60)),
                config,
                samples,
                plans,
                agency,
                probe: SampleProbe::new(),
                keep: keep::Keep::new(None, None),
                desks: desks::Desks::new(None, None),
                node: None,
                talks: Default::default(),
                newsletter: newsletter::Newsletter::new(
                    None,
                    None,
                    None,
                    None,
                    "https://agentrix.website",
                ),
                checks: Arc::new(door::NoChecks),
            }),
        })
    }

    /// The same page, starting the agent door's checks with `checks` (node passes Assess).
    pub fn with_checks(mut self, checks: Arc<dyn door::Checks>) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.checks = checks;
        }
        self
    }

    /// The same page, keeping reports with `keep` (node passes its database and W9's outbox).
    pub fn with_desks(mut self, desks: desks::Desks) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.desks = desks;
        }
        self
    }

    /// The front desks.
    pub fn desks(&self) -> &desks::Desks {
        &self.inner.desks
    }

    /// When the Chief last wrote James's brief for agentrix.website (W1's `briefs` rows).
    pub async fn chief_brief(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let (pool, _) = self.inner.node.as_ref()?;
        let at: Option<String> = sqlx::query_scalar(
            "SELECT written_at FROM briefs WHERE site_id = ? ORDER BY written_at DESC LIMIT 1",
        )
        .bind(desks::SITE)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        at.and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
    }

    /// The same page, with the newsletter agent.
    pub fn with_newsletter(mut self, n: newsletter::Newsletter) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.newsletter = n;
        }
        self
    }

    /// The newsletter agent.
    pub fn newsletter(&self) -> &newsletter::Newsletter {
        &self.inner.newsletter
    }

    /// The same page, reading checks' own pages from node's database and CAS.
    pub fn with_node(
        mut self,
        pool: sqlx::SqlitePool,
        blobs: Arc<dyn agentrix_host::Blobs>,
    ) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.node = Some((pool, blobs));
        }
        self
    }

    /// The same page, keeping reports with `keep` (node passes its database and W9's outbox).
    pub fn with_keep(mut self, keep: keep::Keep) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.keep = keep;
        }
        self
    }

    /// The page as node runs it: facts from `control.db`, read-only.
    pub fn from_context(ctx: &NodeContext) -> Result<Self, Error> {
        let metrics = agentrix_store::metrics::SqliteMetrics::from_env(ctx.pool.clone());
        let mut config = HomeConfig::from_env(ctx.origin.as_str());
        config.platform_host = ctx.platform_host.clone();
        Ok(Self::new(
            config,
            Arc::new(
                StoreFacts::new(Arc::new(metrics), SqliteFacts::new(ctx.pool.clone()))
                    .with_samples(sample_catalogue()?.into_iter().map(|s| s.slug).collect()),
            ),
        )?
        .with_keep(
            keep::Keep::new(
                Some(ctx.pool.clone()),
                Some(Arc::new(keep::OutboxSender(
                    agentrix_messages::from_context(ctx),
                ))),
            )
            .with_origin(ctx.origin.as_str()),
        )
        .with_desks(
            desks::Desks::new(
                Some(Arc::new(desks::GatewayModel::new(
                    &ctx.models_url,
                    ctx.http.clone(),
                ))),
                Some(Arc::new(desks::KernelDials(agentrix_kernel::Rosters::new(
                    ctx.pool.clone(),
                    ctx.blobs.clone(),
                )))),
            )
            .with_pool(ctx.pool.clone()),
        )
        .with_node(ctx.pool.clone(), ctx.blobs.clone())
        .with_newsletter(newsletter_agent(ctx))
        .with_checks(door::checks_for(ctx.origin.as_str(), ctx.http.clone())))
    }

    /// Each sample's host: `kilele-bakery-sample.agentrix.website`.
    pub fn sample_hosts(&self) -> Vec<String> {
        self.inner
            .samples
            .iter()
            .map(|s| format!("{}.{}", s.slug, self.inner.config.platform_host))
            .collect()
    }

    /// Which samples answer, checked by [`SampleProbe::task`].
    pub fn sample_probe(&self) -> &SampleProbe {
        &self.inner.probe
    }

    /// The background check that the samples answer, for node to spawn.
    fn probe_task(&self, ctx: &NodeContext) -> futures::future::BoxFuture<'static, ()> {
        self.inner.probe.clone().task(
            ctx.http.clone(),
            self.sample_hosts(),
            self.inner.config.origin.clone(),
        )
    }

    /// The home page's HTML, on the facts as they are now.
    pub async fn render_home(&self) -> Result<String, Error> {
        self.render_view("home.html").await
    }

    /// A page drawn from the same view as the home page: `home.html`, or one of the desks'
    /// pages (`how.html`, `samples.html`, `prices.html`).
    pub async fn render_view(&self, template: &str) -> Result<String, Error> {
        let facts = self.inner.facts.get().await;
        let c = &self.inner.config;
        let mut view = view::home(
            &facts,
            &view::Inputs {
                origin: &c.origin,
                assess_action: &c.assess_action,
                fixture_href: c.fixture_href.as_deref(),
                samples: &self.inner.samples,
                plans: &self.inner.plans,
                agency: self.inner.agency.as_ref(),
                links: &c.links,
                marks: &self.inner.marks,
                platform_host: &c.platform_host,
                samples_open: &self.inner.probe.open(),
                film: self.film_view(),
                contact: c.contact.as_ref(),
                kept: self.inner.keep.counts().await,
                desks_ai: self.inner.desks.has_model(),
                answered: self.inner.desks.answered_today().await,
                issues: self.inner.newsletter.issues().await,
                readers: self.inner.newsletter.readers().await,
                chief_brief: self.chief_brief().await,
            },
        );
        Ok(compact(&self.inner.env.get_template(template)?.render(
            {
                view.page = template.trim_end_matches(".html").to_owned();
                &view
            },
        )?))
    }

    fn film_view(&self) -> film::FilmView {
        self.inner
            .config
            .film
            .as_ref()
            .map(film::Film::view)
            .unwrap_or_default()
    }

    /// The film's page, when the whole film is on disk.
    pub fn render_film(&self) -> Option<Result<String, Error>> {
        let film = self.film_view();
        film.film.as_ref()?;
        let c = &self.inner.config;
        let render = || {
            Ok(compact(&self.inner.env.get_template("film.html")?.render(
                minijinja::context! {
                    origin => c.origin.clone(),
                    v => ASSETS_VERSION,
                    ui_v => agentrix_ui::VERSION,
                    assess_action => c.assess_action.clone(),
                    links => c.links.clone(),
                    film => film,
                },
            )?))
        };
        Some(render())
    }

    /// A plain page (privacy, terms, the crawler page).
    pub fn render_page(&self, template: &str) -> Result<String, Error> {
        let c = &self.inner.config;
        Ok(self
            .inner
            .env
            .get_template(template)?
            .render(minijinja::context! {
                origin => c.origin.clone(),
                v => ASSETS_VERSION,
                ui_v => agentrix_ui::VERSION,
                assess_action => c.assess_action.clone(),
                links => c.links.clone(),
            })
            .map(|html| compact(&html))?)
    }

    /// Every route except `/`.
    pub fn routes(&self) -> Router {
        self.common()
            .route("/home", get(home_page))
            .with_state(self.clone())
    }

    /// Every route with the page at `/`, and `/home` sending people there for good.
    pub fn routes_as_front_door(&self) -> Router {
        self.common()
            .route("/", get(home_page))
            .route("/home", get(|| async { Redirect::permanent("/") }))
            .with_state(self.clone())
    }

    /// The routes besides the page itself.
    fn common(&self) -> Router<Home> {
        Router::new()
            .route("/_home/{file}", get(asset))
            .route("/_home/marks/{file}", get(mark))
            .route("/_home/film/{file}", get(film_file))
            .route("/film", get(film_page))
            .route("/your-site", get(|s| desk_page(s, "yoursite.html")))
            .route("/how-it-works", get(|s| desk_page(s, "how.html")))
            .route("/samples", get(|s| desk_page(s, "samples.html")))
            .route("/prices", get(|s| desk_page(s, "prices.html")))
            .route("/_home/keep", axum::routing::post(keep_report))
            .route("/_home/ask", get(ask_page).post(ask_json))
            .route("/_home/talk", axum::routing::post(talk_json))
            .route("/_home/fix/confirm", get(fix_confirm))
            .route("/_home/fix/join", axum::routing::post(fix_join))
            .route("/_home/fix/stop", get(fix_stop))
            .route("/_home/peek/{desk}", get(peek))
            .route("/robots.txt", get(robots))
            .route("/sitemap.xml", get(sitemap))
            .route("/privacy", get(privacy))
            .route("/terms", get(terms))
            .route("/bot", get(crawler))
            .route("/.well-known/agentrix-crawler", get(crawler))
            .merge(door::routes())
    }

    /// `GET /`: the home page as the front door.
    pub fn front_door(&self) -> Router {
        Router::new()
            .route("/", get(home_page))
            .with_state(self.clone())
    }
}

/// The weekly fix's agent on node: the Editor drafts through the gateway, James approves on a
/// decision card, and W9's outbox sends.
fn newsletter_agent(ctx: &NodeContext) -> newsletter::Newsletter {
    newsletter::Newsletter::new(
        Some(ctx.pool.clone()),
        Some(Arc::new(
            desks::GatewayModel::new(&ctx.models_url, ctx.http.clone())
                .for_job("brief", "weekly-fix"),
        )),
        Some(agentrix_decisions::Decisions::new(ctx)),
        Some(Arc::new(keep::OutboxSender(
            agentrix_messages::from_context(ctx),
        ))),
        ctx.origin.as_str(),
    )
}

/// The runner for an approved weekly fix (`newsletter.send`). Node registers it with the
/// other effect runners: `runners.push(agentrix_home::newsletter_runner(ctx))`.
pub fn newsletter_runner(ctx: &NodeContext) -> Arc<dyn agentrix_decisions::EffectRunner> {
    Arc::new(newsletter::NewsletterRunner(newsletter_agent(ctx)))
}

/// Mounts every home-page route except `/` (contract C11). The page is at `/home` until the
/// coordinator swaps in [`mount_with_front_door`].
pub fn mount(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let home = Home::from_context(ctx)?;
    Ok(Mounted::default()
        .platform(home.routes())
        .task("home: the samples answer", home.probe_task(ctx))
        .task(
            "home: kept reports last 30 days",
            home.inner.keep.clone().task(),
        )
        .task(
            "home: the Editor drafts the weekly fix",
            home.inner.newsletter.clone().task(),
        ))
}

/// Mounts every home-page route including `GET /`, once the first release's Start page has left `/`
/// (`specs/platform/handover-wiring.md` §1.2). `/home` then redirects to `/` for good.
pub fn mount_with_front_door(ctx: &NodeContext) -> anyhow::Result<Mounted> {
    let home = Home::from_context(ctx)?;
    Ok(Mounted::default()
        .platform(home.routes_as_front_door())
        .task("home: the samples answer", home.probe_task(ctx))
        .task(
            "home: kept reports last 30 days",
            home.inner.keep.clone().task(),
        )
        .task(
            "home: the Editor drafts the weekly fix",
            home.inner.newsletter.clone().task(),
        ))
}

/// Drops each line's indentation and blank lines, outside `<pre>`: the templates stay readable
/// and the page stays under 60 KB of HTML and CSS (02 §9).
pub fn compact(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_pre = false;
    for line in html.lines() {
        let kept = if in_pre { line } else { line.trim_start() };
        if !kept.is_empty() || in_pre {
            out.push_str(kept);
            out.push('\n');
        }
        let opens = line.matches("<pre").count();
        let closes = line.matches("</pre>").count();
        if opens > closes {
            in_pre = true;
        } else if closes > opens {
            in_pre = false;
        }
    }
    out
}

/// Nothing from third parties and no framing: the scripts, the film, the check's live events and
/// every image come from this origin, and forms post only to it.
const CSP: &str = "default-src 'none'; style-src 'self'; img-src 'self' data:; font-src 'self'; \
                   script-src 'self'; media-src 'self'; connect-src 'self'; form-action 'self'; \
                   base-uri 'none'; frame-ancestors 'none'";

fn page(html: String, max_age: u32) -> Response {
    let cache = HeaderValue::from_str(&format!("public, max-age={max_age}"))
        .unwrap_or(HeaderValue::from_static("no-cache"));
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, cache),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(CSP),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("strict-origin-when-cross-origin"),
            ),
        ],
        Html(html),
    )
        .into_response()
}

fn failed(e: Error) -> Response {
    tracing::error!(error = %e, "home: rendering failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "This page didn't render. The rest of agentrix.website still works: try /assess.",
    )
        .into_response()
}

async fn home_page(State(home): State<Home>) -> Response {
    match home.render_home().await {
        // The facts are cached for a minute, so the page may be too.
        Ok(html) => page(html, 60),
        Err(e) => failed(e),
    }
}

async fn desk_page(State(home): State<Home>, template: &'static str) -> Response {
    match home.render_view(template).await {
        Ok(html) => page(html, 60),
        Err(e) => failed(e),
    }
}

/// `POST /_home/keep`: the one field after the check. The in-place check asks for JSON; a plain
/// form post gets a short page with the same words.
async fn keep_report(
    State(home): State<Home>,
    headers: axum::http::HeaderMap,
    axum::Form(form): axum::Form<keep::KeepForm>,
) -> Response {
    let kept = home.inner.keep.keep(&form, "/", chrono::Utc::now()).await;
    let code = if kept.status == "invalid" {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::OK
    };
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    if wants_json {
        return (code, axum::Json(kept)).into_response();
    }
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>Your report · Agentrix</title><link rel=stylesheet href=\"/_home/home.css?v={v}\"><main class=\"wrap section\"><h1>Your report</h1><p class=lead>{msg}</p><p><a class=\"btn btn-primary\" href=\"{url}\">Open your report</a></p></main>",
        v = ASSETS_VERSION,
        msg = minijinja::HtmlEscape(&kept.message),
        url = minijinja::HtmlEscape(&kept.report_url),
    );
    (code, page(html, 0)).into_response()
}

impl Home {
    /// What the desks know now: the price list, the samples and this site's records as facts,
    /// and this site's own pages as sections.
    pub async fn knowledge(&self) -> desks::Knowledge {
        use desks::Section;
        let facts_now = self.inner.facts.get().await;
        let c = &self.inner.config;
        let mut facts: Vec<(&'static str, Section)> = Vec::new();
        for p in &self.inner.plans {
            facts.push((
                "prices",
                Section {
                    label: format!("Price list · {}", p.name),
                    href: "/prices".into(),
                    text: format!(
                        "{} plan: {} a {}. It includes: {}.",
                        p.name,
                        p.price,
                        p.interval,
                        p.points.join("; ")
                    ),
                },
            ));
        }
        if let Some(a) = &self.inner.agency {
            facts.push((
                "prices",
                Section {
                    label: "Price list · Agencies".into(),
                    href: "/prices".into(),
                    text: format!(
                        "Agencies pay {} a {} for each client site.",
                        a.price, a.interval
                    ),
                },
            ));
        }
        // How plans are paid, as the price list says, naming the way people here ask about.
        facts.push((
            "prices",
            Section {
                label: "Price list · How to pay".into(),
                href: "/prices".into(),
                text: format!(
                    "Plans are paid annually or monthly in US dollars (USD), through PayPal, and a 12-month plan comes with a {}% discount. Moving your site is free. PayPal is the only way to pay for a plan, so you can't pay a plan with M-Pesa.",
                    agentrix_billing::catalogue().annual_discount_percent
                ),
            },
        ));
        let open = self.inner.probe.open();
        for s in &self.inner.samples {
            let host = format!("{}.{}", s.slug, c.platform_host);
            let open_now = open.contains(&host);
            facts.push((
                "samples",
                Section {
                    label: format!("Samples · {}", s.name),
                    href: if open_now {
                        view::site_url(&c.origin, &host)
                    } else {
                        "/samples".into()
                    },
                    text: format!(
                        "{} ({}) is a sample business we made up. {} {} {}{}",
                        s.name,
                        s.tag,
                        s.line,
                        s.try_this,
                        if open_now {
                            "It is open now."
                        } else {
                            "It opens soon."
                        },
                        if s.walk.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " A walkthrough: {} Then open the owner's side at /samples/{}/.",
                                s.walk
                                    .iter()
                                    .enumerate()
                                    .map(|(i, w)| format!("{}. {w}", i + 1))
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                s.slug
                            )
                        }
                    ),
                },
            ));
        }
        // The films' stories: sample stories made with AI, one per kind of business.
        for (label, text) in film::STORIES {
            facts.push((
                "samples",
                Section {
                    label: label.into(),
                    href: "/samples#films".into(),
                    text: text.into(),
                },
            ));
        }
        let reports = match facts_now.reports_written {
            Some(n) => format!("This site has written {n} free health reports so far."),
            None => "This site's count of health reports can't be read right now.".into(),
        };
        facts.push((
            "your-site",
            Section {
                label: "This site's records".into(),
                href: "/".into(),
                text: format!(
                    "{reports} The check is free, needs no account and reads only public pages. It never changes anything on your site."
                ),
            },
        ));
        // What the move changes, for the Your site desk's business conversation: what the
        // product does today, in plain words.
        for (label, href, text) in [
            (
                "What changes · For your customers",
                "/how-it-works#conversations",
                "On your new Agentrix Website, WebAgent desks answer your visitors at any hour, from your own pages, and show the page they used. They take orders and bookings, and pass anything that needs a person to you.",
            ),
            (
                "What changes · For you",
                "/how-it-works#conversations",
                "Your Chief WebAgent briefs you each morning on what happened and what needs you. Anything that costs money or makes a promise waits for your OK, with one tap on your phone. You can read any conversation and take it over.",
            ),
            (
                "What changes · AI assistants and agents",
                "/how-it-works#site",
                "Your new site has an agent door, so AI assistants and agents can read it, ask it questions and place orders over AWP and MCP. An order from an agent waits for your OK.",
            ),
            (
                "What stays · Your look and your address",
                "/how-it-works#move",
                "Your site keeps its look, its pages and its address. It is rebuilt on Agentrix's secure platform, and the old site's security problems are left behind.",
            ),
            (
                "How to start",
                "/",
                "Paste your address for the free check. It reads public pages only, changes nothing on your site, and shows what the move would change.",
            ),
        ] {
            facts.push((
                "your-site",
                Section {
                    label: label.into(),
                    href: href.into(),
                    text: text.into(),
                },
            ));
        }
        let mut sections = Vec::new();
        for (template, label, path) in [
            ("home.html", "Home", "/"),
            ("yoursite.html", "Your site", "/your-site"),
            ("how.html", "How it works", "/how-it-works"),
            ("samples.html", "Samples", "/samples"),
            ("prices.html", "Prices", "/prices"),
        ] {
            if let Ok(html) = self.render_view(template).await {
                sections.extend(desks::sections(label, path, &html));
            }
        }
        desks::Knowledge {
            facts,
            sections,
            person: c.contact.as_ref().map(|p| desks::Step {
                label: "Talk to a person".into(),
                href: p.href.clone(),
            }),
        }
    }

    /// One desk's answer, recorded as a count (never the words).
    pub async fn ask(&self, desk: &str, q: &str, history: &[desks::Turn]) -> Option<desks::Answer> {
        let desk = desks::desk(desk)?;
        let know = self.knowledge().await;
        let answer = self.inner.desks.answer(desk, q, history, &know).await;
        self.inner.desks.record(answer.desk, answer.ai).await;
        Some(answer)
    }
}

#[derive(Deserialize)]
struct AskJson {
    desk: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    history: Vec<desks::Turn>,
}

/// `POST /_home/ask`: one turn, for the desks on the page.
async fn ask_json(State(home): State<Home>, axum::Json(body): axum::Json<AskJson>) -> Response {
    match home.ask(&body.desk, &body.q, &body.history).await {
        Some(a) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(a)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct AskQuery {
    #[serde(default)]
    desk: String,
    #[serde(default)]
    q: String,
}

/// `GET /_home/ask?desk=…&q=…`: the same answer as a page, for a browser without JavaScript.
/// Never indexed.
async fn ask_page(
    State(home): State<Home>,
    axum::extract::Query(q): axum::extract::Query<AskQuery>,
) -> Response {
    let Some(answer) = home.ask(&q.desk, &q.q, &[]).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let c = &home.inner.config;
    let html = home.inner.env.get_template("ask.html").and_then(|t| {
        t.render(minijinja::context! {
            origin => c.origin.clone(),
            v => ASSETS_VERSION,
            ui_v => agentrix_ui::VERSION,
            assess_action => c.assess_action.clone(),
            links => c.links.clone(),
            contact => c.contact.as_ref().map(|p| p.at(chrono::Utc::now())),
            desk => desks::desk(&q.desk),
            q => q.q.chars().take(400).collect::<String>(),
            answer => answer,
        })
    });
    match html {
        Ok(html) => {
            let mut res = page(compact(&html), 0);
            res.headers_mut()
                .insert("x-robots-tag", HeaderValue::from_static("noindex"));
            res
        }
        Err(e) => failed(e.into()),
    }
}

/// `GET /_home/peek/{desk}`: a desk's peek for the header, fetched the first time it opens.
async fn peek(State(home): State<Home>, Path(id): Path<String>) -> Response {
    let Some(desk) = desks::desk(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let facts = home.inner.facts.get().await;
    let fact = match desk.id {
        "your-site" => facts
            .reports_written
            .map(|n| format!("{n} free reports written so far")),
        "samples" => Some(format!(
            "{} of 3 samples open now",
            home.inner.probe.open().len()
        )),
        "prices" => home
            .inner
            .plans
            .iter()
            .filter(|p| p.interval == "month")
            .min_by_key(|p| p.cents)
            .map(|p| format!("Plans from {} a month", p.price)),
        _ => Some("Seven steps, four of them in sealed rooms".into()),
    };
    let html = home.inner.env.get_template("peek.html").and_then(|t| {
        t.render(minijinja::context! {
            desk => desk,
            fact => fact,
            as_of => facts.read_at.map(view::date_time),
            desks_ai => home.inner.desks.has_model(),
        })
    });
    match html {
        Ok(html) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "public, max-age=60"),
            ],
            compact(&html),
        )
            .into_response(),
        Err(e) => failed(e.into()),
    }
}

/// At most this many questions about one check.
const TALKS_PER_CHECK: u32 = 12;

impl Home {
    /// What the Your site desk answers from after a check: what it found, the pages it read
    /// (from node's CAS, as data) and this site's How it works.
    pub async fn talk_sources(&self, assessment: &str) -> Option<desks::TalkSources> {
        let (pool, blobs) = self.inner.node.as_ref()?;
        let facts = keep::check_facts(pool, assessment).await;
        if facts.site.is_empty() {
            return None;
        }
        let mut check_text = facts.headline.clone();
        for f in &facts.findings {
            check_text.push_str(&format!(" Found: {f}."));
        }
        let check = vec![desks::Section {
            label: "Your check".into(),
            href: format!("/assess/{assessment}"),
            text: check_text,
        }];
        let report: Option<String> =
            sqlx::query_scalar("SELECT report FROM assessments WHERE assessment_id = ?")
                .bind(assessment)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .flatten();
        let listed: Vec<(String, String)> = report
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
            .and_then(|r| r["pages"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|p| {
                Some((
                    p["url"].as_str()?.to_owned(),
                    p["digest"].as_str()?.to_owned(),
                ))
            })
            .take(25)
            .collect();
        let mut pages = Vec::new();
        for (url, digest) in listed {
            let Ok(Some(bytes)) = blobs.get(&digest).await else {
                continue;
            };
            let html = String::from_utf8_lossy(&bytes);
            let title = html
                .find("<title")
                .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
                .and_then(|a| {
                    html[a..]
                        .find("</title>")
                        .map(|b| desks::text_of(&html[a..a + b]))
                })
                .filter(|t| !t.is_empty());
            let path = url
                .split_once("://")
                .map_or(url.as_str(), |(_, r)| r)
                .split_once('/')
                .map_or("/".to_owned(), |(_, p)| format!("/{p}"));
            let text = desks::text_of(&html);
            if text.len() < 20 {
                continue;
            }
            pages.push(desks::Section {
                label: format!("Your page · {}", title.unwrap_or(path)),
                href: url,
                text: text.chars().take(1400).collect(),
            });
        }
        let how = match self.render_view("how.html").await {
            Ok(html) => desks::sections("How it works", "/how-it-works", &html),
            Err(_) => Vec::new(),
        };
        Some(desks::TalkSources {
            site: facts.site,
            check,
            pages,
            how,
        })
    }
}

#[derive(Deserialize)]
struct TalkJson {
    #[serde(default)]
    assessment: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    history: Vec<desks::Turn>,
}

/// `POST /_home/talk`: one question about the visitor's own site, after the check.
async fn talk_json(State(home): State<Home>, axum::Json(body): axum::Json<TalkJson>) -> Response {
    let valid = body.assessment.starts_with("asmt_")
        && body.assessment.len() <= 64
        && body
            .assessment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return StatusCode::NOT_FOUND.into_response();
    }
    {
        let mut talks = home
            .inner
            .talks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = talks.entry(body.assessment.clone()).or_default();
        if *n >= TALKS_PER_CHECK {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({ "message": "That's all the questions for this check. A person can answer more: WhatsApp us." })),
            )
                .into_response();
        }
        *n += 1;
    }
    let Some(sources) = home.talk_sources(&body.assessment).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let know = desks::Knowledge {
        facts: Vec::new(),
        sections: Vec::new(),
        person: home.inner.config.contact.as_ref().map(|p| desks::Step {
            label: "Talk to a person".into(),
            href: p.href.clone(),
        }),
    };
    let answer = home
        .inner
        .desks
        .talk(&body.q, &body.history, &sources, &know)
        .await;
    home.inner.desks.record(answer.desk, answer.ai).await;
    ([(header::CACHE_CONTROL, "no-store")], axum::Json(answer)).into_response()
}

#[derive(Deserialize)]
struct FixToken {
    #[serde(default)]
    token: String,
}

fn fix_page(title: &str, words: &str) -> Response {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><meta name=robots content=noindex><title>{title} · Agentrix</title><link rel=stylesheet href=\"/_home/home.css?v={v}\"><main class=\"wrap section\"><h1>{title}</h1><p class=lead>{words}</p><p><a class=btn href=\"/\">agentrix.website</a></p></main>",
        v = ASSETS_VERSION,
    );
    page(html, 0)
}

/// The confirm link in `newsletter.confirm`.
async fn fix_confirm(
    State(home): State<Home>,
    axum::extract::Query(q): axum::extract::Query<FixToken>,
) -> Response {
    if home.inner.keep.confirm(&q.token, chrono::Utc::now()).await {
        fix_page(
            "You're in",
            "The weekly fix comes on Fridays, five lines or fewer. Every email has a one-tap stop.",
        )
    } else {
        fix_page(
            "That link didn't work",
            "It may be old or already used. You can sign up again at the bottom of the home page.",
        )
    }
}

#[derive(Deserialize)]
struct JoinForm {
    #[serde(default)]
    email: String,
}

/// The sign-up on screen 7: the same double opt-in as the keep step's tick.
async fn fix_join(
    State(home): State<Home>,
    headers: axum::http::HeaderMap,
    axum::Form(form): axum::Form<JoinForm>,
) -> Response {
    let kept = home.inner.keep.join(&form.email, chrono::Utc::now()).await;
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    let code = if kept.status == "invalid" {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::OK
    };
    if wants_json {
        return (code, axum::Json(kept)).into_response();
    }
    let (title, words) = match kept.status {
        "invalid" => ("Please check that address", kept.message.as_str()),
        _ => ("Nearly there", kept.message.as_str()),
    };
    let mut res = fix_page(title, words);
    *res.status_mut() = code;
    res
}

/// The one-tap stop in every weekly fix: the reader's details are deleted.
async fn fix_stop(
    State(home): State<Home>,
    axum::extract::Query(q): axum::extract::Query<FixToken>,
) -> Response {
    home.inner.keep.stop(&q.token).await;
    fix_page(
        "Stopped",
        "We won't send you anything more, and we deleted your details.",
    )
}

async fn film_page(State(home): State<Home>) -> Response {
    match home.render_film() {
        Some(Ok(html)) => page(html, 3600),
        Some(Err(e)) => failed(e),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// One of the film's files from node's disk, with byte ranges (Safari asks for them).
async fn film_file(
    State(home): State<Home>,
    Path(file): Path<String>,
    request: axum::extract::Request,
) -> Response {
    use tower::ServiceExt;
    let Some(path) = home.inner.config.film.as_ref().and_then(|f| f.path(&file)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(response) = tower_http::services::ServeFile::new(path)
        .oneshot(request)
        .await;
    let mut response = response.map(axum::body::Body::new);
    let headers = response.headers_mut();
    // The URLs carry the files' version, so a new cut gets a new URL.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn privacy(State(home): State<Home>) -> Response {
    home.render_page("privacy.html")
        .map_or_else(failed, |h| page(h, 3600))
}

async fn terms(State(home): State<Home>) -> Response {
    home.render_page("terms.html")
        .map_or_else(failed, |h| page(h, 3600))
}

async fn crawler(State(home): State<Home>) -> Response {
    home.render_page("crawler.html")
        .map_or_else(failed, |h| page(h, 3600))
}

async fn robots(State(home): State<Home>) -> Response {
    let body = format!(
        "# agentrix.website\nUser-agent: *\nAllow: /\nDisallow: /console\nDisallow: /assess/\nDisallow: /auth/\nDisallow: /decisions/\nDisallow: /moves/\nDisallow: /sites/\nDisallow: /_home/ask\nDisallow: /_home/fix/\n\nSitemap: {}/sitemap.xml\n",
        home.inner.config.origin
    );
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

async fn sitemap(State(home): State<Home>) -> Response {
    let origin = &home.inner.config.origin;
    let film = home.film_view().film.is_some().then_some("/film");
    let urls = [
        "/",
        "/your-site",
        "/how-it-works",
        "/samples",
        "/prices",
        "/privacy",
        "/terms",
        "/bot",
    ]
    .into_iter()
    .chain(film)
    .map(|p| format!("  <url><loc>{origin}{p}</loc></url>\n"))
    .collect::<String>();
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{urls}</urlset>\n"
    );
    (
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}

/// The static files, compiled in.
fn static_file(name: &str) -> Option<(&'static str, &'static [u8])> {
    Some(match name {
        // Minified by build.rs; under dev-reload `asset` reads the readable file instead.
        "home.css" => (
            "text/css; charset=utf-8",
            include_bytes!(concat!(env!("OUT_DIR"), "/home.min.css")),
        ),
        "mark.svg" => ("image/svg+xml", include_bytes!("../static/mark.svg")),
        "og.png" => ("image/png", include_bytes!("../static/og.png")),
        // The hero film's play and pause, and the posters shown before (or instead of) it.
        "hero.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/hero.js"),
        ),
        // The desks: the header's peeks and every ask form, answered in place.
        "desks.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/desks.js"),
        ),
        // The Agentrix guide on every page: the header's mark opens tours and Ask.
        "guide.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/guide.js"),
        ),
        // Screen 3's film, played only when someone taps Play.
        "play.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/play.js"),
        ),
        // The free check in place on screen 1, from W4's Assess events.
        "check.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/check.js"),
        ),
        "hero-poster.webp" => ("image/webp", include_bytes!("../static/hero-poster.webp")),
        "film-poster.webp" => ("image/webp", include_bytes!("../static/film-poster.webp")),
        // Screenshots taken Sun 8 Nov from the samples and the console on a local node.
        "shot-kilele.webp" => ("image/webp", include_bytes!("../static/shot-kilele.webp")),
        "shot-classy.webp" => ("image/webp", include_bytes!("../static/shot-classy.webp")),
        "shot-tana.webp" => ("image/webp", include_bytes!("../static/shot-tana.webp")),
        "shot-console.webp" => ("image/webp", include_bytes!("../static/shot-console.webp")),
        // Screen 5's walkthrough, captured Sun 8 Nov: the brief and the Chief's answer from the
        // Classy Perfumes sample on a local node, the held order and Live from W5's shell-node
        // (film/console.py).
        "owner-brief.webp" => ("image/webp", include_bytes!("../static/owner-brief.webp")),
        "owner-decide.webp" => ("image/webp", include_bytes!("../static/owner-decide.webp")),
        "owner-live.webp" => ("image/webp", include_bytes!("../static/owner-live.webp")),
        "owner-ask.webp" => ("image/webp", include_bytes!("../static/owner-ask.webp")),
        "owner.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/owner.js"),
        ),
        _ => return None,
    })
}

async fn mark(State(home): State<Home>, Path(file): Path<String>) -> Response {
    match home.inner.marks.file(&file) {
        Some(svg) => (
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            svg.to_owned(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn asset(Path(file): Path<String>) -> Response {
    let Some((content_type, body)) = static_file(&file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // While editing, serve the file as it is on disk now.
    #[cfg(feature = "dev-reload")]
    let body: Vec<u8> = std::fs::read(format!("{}/static/{file}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|_| body.to_vec());
    let cache = if cfg!(feature = "dev-reload") {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_platform_host_comes_from_the_origin() {
        assert_eq!(
            super::host_of("https://agentrix.website"),
            "agentrix.website"
        );
        assert_eq!(
            super::host_of("https://agentrix.website:443/"),
            "agentrix.website"
        );
        assert_eq!(super::host_of("http://localhost:18080"), "localhost");
    }
}
