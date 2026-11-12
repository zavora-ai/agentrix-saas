//! Agentrix SaaS's face: what people and other agents see (slice D).
//!
//! One axum server answers four hosts, picked by the `Host` header ([`hosts`]):
//!
//! | Host | Serves |
//! |---|---|
//! | `money.agentrix.website` | The WebAgents front (plans, Sales and Support desks), the owner console, the treasury board at `/board/` and its agent at `/board/agent`, and `/feed.json` |
//! | `kilele.money.agentrix.website` | Kilele Bakery's console: its staff, its mandate, its bills; `/feed.json` with Kilele's rows |
//! | `pay.agentrix.website` | The agent door ([`door`]): AWP 2.0 manifest, agent keys under `/.well-known/`, and slice C's A2A routes |
//! | `books.agentrix.website` | Zavora ERA, forwarded when `SAAS_ERA_URL` is set ([`books`]) |
//!
//! The other slices plug in through seams: a [`feed::FeedSource`] (slices A and B), a
//! [`console::Console`] (freeze and decisions, slices B and C), a [`desk::Desk`] (slice B's
//! Revenue agent), and [`door::Door`]'s routes, keys and capabilities (slice C). [`Face::demo`]
//! fills every seam with the built-in story, so the face runs on its own.

pub mod activity;
pub mod agent;
pub mod approvals;
pub mod apps;
pub mod books;
pub mod books_feed;
pub mod checkout;
pub mod console;
pub mod demo;
pub mod desk;
pub mod door;
pub mod feed;
pub mod hosts;
pub mod os;
pub mod pages;
pub mod passkeys;
pub mod proofs;
pub mod seed;

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{FixedOffset, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::agent::Today;
use crate::console::Console;
use crate::desk::Desk;
use crate::door::Door;
use crate::feed::{AGENTRIX, FeedSource, KILELE};
use crate::hosts::{Hosts, Site};

/// Today in Nairobi (EAT, UTC+3), the day the books count ages from.
pub fn today_in_nairobi() -> NaiveDate {
    let eat = FixedOffset::east_opt(3 * 3600).expect("UTC+3 is a valid offset");
    Utc::now().with_timezone(&eat).date_naive()
}

/// Who may press the console's buttons.
#[derive(Debug, Clone, Default)]
pub struct Access {
    /// The owner's token: a request carrying it (cookie `saas_owner`, or `Authorization:
    /// Bearer`) may act. Visiting `/console?key=<token>` once sets the cookie.
    pub owner_token: Option<String>,
    /// Anyone may act (a local demo, or judging day). Loopback and `.localhost` hosts may
    /// always act.
    pub open: bool,
}

impl Access {
    pub(crate) fn allows(&self, headers: &HeaderMap) -> bool {
        if self.open {
            return true;
        }
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
        if name == "localhost"
            || name == "127.0.0.1"
            || name == "[::1]"
            || name.ends_with(".localhost")
        {
            return true;
        }
        let Some(token) = self.owner_token.as_deref().filter(|t| t.len() >= 16) else {
            return false;
        };
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let cookie = headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(';'))
            .filter_map(|kv| kv.trim().strip_prefix("saas_owner="))
            .next();
        [bearer, cookie]
            .into_iter()
            .flatten()
            .any(|given| constant_eq(given, token))
    }
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// The face, with every seam filled.
pub struct Face {
    hosts: Hosts,
    feed: Arc<dyn FeedSource>,
    console: Arc<dyn Console>,
    desk: Arc<dyn Desk>,
    door: Door,
    board_dist: Option<PathBuf>,
    era_url: Option<String>,
    today: Today,
    access: Access,
    money_extra: Option<Router>,
    kilele_extra: Option<Router>,
    proofs_dir: Option<PathBuf>,
    board_licence: Option<String>,
    checkout: Arc<dyn checkout::Checkout>,
    passkeys: Arc<passkeys::Passkeys>,
    app_sites: Vec<AppSite>,
    day_runner: Option<Arc<dyn apps::DayRunner>>,
    activity: Option<activity::Activity>,
    grid_js: Option<PathBuf>,
}

/// One site's v3 apps (passkey moments, badges, wallet) and the host they're served on:
/// nested at `/console/{site}` there, with the public `/agents` page.
#[derive(Clone)]
pub struct AppSite {
    /// `agentrix`, `kilele`.
    pub site: String,
    /// "Kilele Bakery".
    pub name: String,
    /// "Wanjiru".
    pub owner: String,
    /// Which host serves it.
    pub host: Site,
    /// The moments (settle).
    pub ceremonies: Arc<dyn apps::Ceremonies>,
    /// The badges (settle).
    pub badges: Arc<dyn apps::Badges>,
    /// The wallet (books).
    pub wallet: Arc<dyn apps::Wallets>,
    /// Where saas-badge's PayPal binding router is mounted for this site, when it isn't
    /// `/console/{site}/paypal-link` on the same host.
    pub paypal_link: Option<String>,
}

/// The demo apps for Agentrix (money host) and Kilele (Kilele host), from the story.
fn demo_app_sites(hosts: &Hosts) -> Vec<AppSite> {
    let story = demo::story(NaiveDate::default());
    let ceremonies: Arc<dyn apps::Ceremonies> = Arc::new(apps::demo::DemoCeremonies::story(
        &story
            .companies
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str(), c.owner.as_str()))
            .collect::<Vec<_>>(),
    ));
    let sites = story
        .companies
        .iter()
        .map(|c| {
            let host = if c.id == KILELE {
                hosts.kilele.clone()
            } else {
                hosts.money.clone()
            };
            (
                c.id.clone(),
                apps::demo::DemoSite {
                    host,
                    name: c.name.clone(),
                    owner: c.owner.clone(),
                    agents: c
                        .agents
                        .iter()
                        .map(|a| (a.id.clone(), a.name.clone()))
                        .collect(),
                },
            )
        })
        .collect();
    let badges: Arc<dyn apps::Badges> = Arc::new(apps::demo::DemoBadges::new(sites));
    story
        .companies
        .iter()
        .map(|c| AppSite {
            site: c.id.clone(),
            name: c.name.clone(),
            owner: c.owner.clone(),
            host: if c.id == KILELE {
                Site::Kilele
            } else {
                Site::Money
            },
            ceremonies: Arc::clone(&ceremonies),
            badges: Arc::clone(&badges),
            wallet: Arc::new(apps::demo::NotConnected),
            paypal_link: None,
        })
        .collect()
}

impl Face {
    /// The face on the built-in story: demo books and console, rules desks, the plan-only door.
    pub fn demo(hosts: Hosts) -> Self {
        let story = demo::DemoFeed::new();
        let hosts_for_apps = hosts.clone();
        Self {
            hosts,
            feed: Arc::new(story.clone()),
            console: Arc::new(story),
            desk: Arc::new(desk::RulesDesk::new(&hosts_for_apps.pay)),
            door: Door::new(),
            board_dist: Some(default_board_dist()),
            era_url: None,
            today: Arc::new(today_in_nairobi),
            access: Access::default(),
            money_extra: None,
            kilele_extra: None,
            proofs_dir: Some(default_proofs_dir()),
            board_licence: None,
            checkout: Arc::new(checkout::NotConnected),
            passkeys: Arc::new(passkeys::Passkeys::in_memory()),
            app_sites: demo_app_sites(&hosts_for_apps),
            day_runner: None,
            activity: None,
            grid_js: Some(apps::default_grid_js()),
        }
    }

    /// Serves `site`'s v3 apps (replacing the stand-ins for that site id).
    pub fn with_app_site(mut self, site: AppSite) -> Self {
        self.app_sites.retain(|s| s.site != site.site);
        self.app_sites.push(site);
        self
    }

    /// The apps' sites, to change one (`saas-node` swaps in settle's and books' seams).
    pub fn app_sites(&self) -> &[AppSite] {
        &self.app_sites
    }

    /// Reads every site's wallet from `wallet`: books' saas-wallet in process, or
    /// [`apps::http::HttpWallet`] at its `GET /wallet/{site}.json`.
    pub fn with_wallets(mut self, wallet: Arc<dyn apps::Wallets>) -> Self {
        for site in &mut self.app_sites {
            site.wallet = Arc::clone(&wallet);
        }
        self
    }

    /// Operators' live activity (`/activity` on the money host when the face runs on its own;
    /// the node mounts its own on the ops host). Without one, it reads this node's wallets and
    /// names the sources it doesn't connect.
    pub fn with_activity(mut self, activity: Option<activity::Activity>) -> Self {
        self.activity = activity;
        self
    }

    /// "Run the day" on the judges' page (judge mode only).
    pub fn with_day_runner(mut self, runner: Option<Arc<dyn apps::DayRunner>>) -> Self {
        self.day_runner = runner;
        self
    }

    /// Where the built AG Grid wallet component is.
    pub fn with_grid_js(mut self, path: Option<PathBuf>) -> Self {
        self.grid_js = path;
        self
    }

    /// Subscribes visitors with PayPal through `checkout` (saas-node: agentrix-effects).
    pub fn with_checkout(mut self, checkout: Arc<dyn checkout::Checkout>) -> Self {
        self.checkout = checkout;
        self
    }

    /// Keeps the owners' passkeys in `passkeys` (a file on the node, so a restart keeps them).
    pub fn with_passkeys(mut self, passkeys: Arc<passkeys::Passkeys>) -> Self {
        self.passkeys = passkeys;
        self
    }

    /// AG Studio's licence key (`AG_STUDIO_LICENSE_KEY`), handed to the board at
    /// `/board/config.json`. A client-side licence key by AG Grid's design; without one the
    /// board runs with Studio's watermark.
    pub fn with_board_licence(mut self, key: Option<String>) -> Self {
        self.board_licence = key.filter(|k| !k.trim().is_empty());
        self
    }

    /// The face on saas-core's books: both companies' `FakeBooks`, seeded with the story as of
    /// `as_of` through the `Books` contract ([`seed`]), read by [`books_feed::BooksFeed`]. The
    /// story still fills what the books don't hold (staff, mandates, decisions, plans,
    /// settlements, spend), naming the books' invoice numbers.
    pub async fn fakebooks(hosts: Hosts, as_of: NaiveDate) -> anyhow::Result<Self> {
        let seeded = seed::seed(as_of).await?;
        let story = demo::DemoFeed::new().with_numbers(seeded.numbers.clone());
        let companies = vec![
            books_feed::CompanyBooks {
                company: AGENTRIX.into(),
                books: seeded.agentrix.clone(),
                paypal_mode: "sandbox".into(),
            },
            books_feed::CompanyBooks {
                company: KILELE.into(),
                books: seeded.kilele.clone(),
                paypal_mode: "sandbox".into(),
            },
        ];
        let feed = books_feed::BooksFeed::new(companies, Arc::new(story.clone()), "fakebooks");
        let mut face = Self::demo(hosts)
            .with_feed(Arc::new(feed))
            .with_console(Arc::new(story));
        let books: std::collections::HashMap<String, Arc<dyn saas_core::Books>> = [
            (
                AGENTRIX.to_owned(),
                seeded.agentrix.clone() as Arc<dyn saas_core::Books>,
            ),
            (
                KILELE.to_owned(),
                seeded.kilele.clone() as Arc<dyn saas_core::Books>,
            ),
        ]
        .into();
        let wallet: Arc<dyn apps::Wallets> = Arc::new(apps::demo::BooksWallet::new(books));
        for site in &mut face.app_sites {
            site.wallet = Arc::clone(&wallet);
        }
        Ok(face)
    }

    /// Reads the feed from `feed` (slices A and B).
    pub fn with_feed(mut self, feed: Arc<dyn FeedSource>) -> Self {
        self.feed = feed;
        self
    }

    /// Sends the owner's actions to `console` (slices B and C).
    pub fn with_console(mut self, console: Arc<dyn Console>) -> Self {
        self.console = console;
        self
    }

    /// Answers the desks with `desk` (slice B).
    pub fn with_desk(mut self, desk: Arc<dyn Desk>) -> Self {
        self.desk = desk;
        self
    }

    /// The agent door's keys, routes and capabilities (slice C).
    pub fn with_door(mut self, door: Door) -> Self {
        self.door = door;
        self
    }

    /// Where the board's built files are (`web/saas-board/dist`); `None` serves a note.
    pub fn with_board_dist(mut self, dist: Option<PathBuf>) -> Self {
        self.board_dist = dist;
        self
    }

    /// Forwards the books host to ERA at `url`.
    pub fn with_era(mut self, url: Option<String>) -> Self {
        self.era_url = url;
        self
    }

    /// Counts ages from `today()` instead of the clock (tests, replays).
    pub fn with_today(mut self, today: Today) -> Self {
        self.today = today;
        self
    }

    /// Who may press the console's buttons.
    pub fn with_access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// More routes on the Kilele host: slice C's Kilele agent door (`SettleHost::kilele_router`:
    /// the key directory, the agent card, `POST /a2a`). They must not reuse `/`, `/console…`,
    /// `/feed.json` or `/static/…`.
    pub fn with_kilele_routes(mut self, routes: Router) -> Self {
        self.kilele_extra = Some(routes);
        self
    }

    /// Where `/proofs` reads its files: `PROOFS.md`, `proofs/attacks.json` and
    /// `proofs/audit/<company>.jsonl|.vkey` (`SAAS_PROOFS_DIR` on the node).
    pub fn with_proofs_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.proofs_dir = dir;
        self
    }

    /// More routes on the money host. They must not reuse this crate's paths (`/`, `/plans`,
    /// `/subscribe…`, `/desk/…`, `/console…`, `/board…`, `/feed.json`, `/static/…`).
    pub fn with_money_routes(mut self, routes: Router) -> Self {
        self.money_extra = Some(routes);
        self
    }

    /// The server: one plain axum router for all four hosts, dispatching on `Host`. saas-node
    /// serves it on its own listener (and composes slice C's door routes through
    /// [`Face::with_door`]); `main.rs` serves it alone for local work.
    pub fn router(self) -> anyhow::Result<Router> {
        let shared = Arc::new(Shared {
            hosts: self.hosts.clone(),
            feed: Arc::clone(&self.feed),
            console: self.console,
            desk: self.desk,
            today: Arc::clone(&self.today),
            access: self.access,
            checkout: self.checkout,
            passkeys: self.passkeys,
            proofs_dir: self.proofs_dir,
            keys: self.door.published_keys().to_vec(),
            judge_mode: self.day_runner.is_some(),
        });
        let money_ctx = Ctx {
            shared: Arc::clone(&shared),
            company: AGENTRIX,
        };
        let kilele_ctx = Ctx {
            shared: Arc::clone(&shared),
            company: KILELE,
        };

        let mut money = Router::new()
            .route("/", get(front))
            .route("/plans", get(plans_page))
            .route("/desk/{desk}", post(desk_reply))
            .route("/feed.json", get(feed_json))
            .route("/subscribe", get(subscribe).post(subscribe_start))
            .route("/subscribe/return", get(subscribe_return))
            .route("/subscribe/status", get(subscribe_status))
            .route("/subscribe/cancelled", get(subscribe_cancelled))
            .route("/proofs", get(proofs::page))
            .route("/proofs/files/{*path}", get(proofs::file))
            .route("/judges", get(judges));
        money = console_routes(money);
        let mut money = money.with_state(money_ctx);
        if let Some(extra) = self.money_extra {
            money = money.merge(extra);
        }
        let mut money = money
            .nest(
                "/board/agent",
                agent::server(Arc::clone(&self.feed), Arc::clone(&self.today), &self.hosts),
            )
            .merge(board_routes(
                self.board_dist,
                self.board_licence,
                self.hosts.pay.clone(),
            ))
            .merge(statics(self.grid_js.clone()));

        let mut kilele = console_routes(
            Router::new()
                .route("/", get(console_page))
                .route("/feed.json", get(feed_json)),
        )
        .with_state(kilele_ctx)
        .merge(statics(self.grid_js.clone()));
        if let Some(extra) = self.kilele_extra {
            kilele = kilele.merge(extra);
        }

        // v3's apps: each site's at /console/{site} on its host, with the public /agents page.
        let judge_badges = self
            .app_sites
            .iter()
            .find(|a| a.host == Site::Money)
            .or_else(|| self.app_sites.first())
            .map(|a| Arc::clone(&a.badges));
        for site in &self.app_sites {
            let apps = apps::SiteApps {
                site: site.site.clone(),
                name: site.name.clone(),
                owner: site.owner.clone(),
                base: format!("/console/{}", site.site),
                ceremonies: Arc::clone(&site.ceremonies),
                badges: Arc::clone(&site.badges),
                wallet: Arc::clone(&site.wallet),
                paypal_link: site
                    .paypal_link
                    .clone()
                    .unwrap_or_else(|| format!("/console/{}/paypal-link", site.site)),
                passkeys: Arc::clone(&shared.passkeys),
                access: shared.access.clone(),
                fragment: false,
            };
            let base = apps.base.clone();
            let routes = Router::new()
                .nest(&base, apps::console_router(apps.clone()))
                .merge(apps::public_router(apps));
            match site.host {
                Site::Money => money = money.merge(routes),
                Site::Kilele => kilele = kilele.merge(routes),
                Site::Pay | Site::Books => {}
            }
        }
        if let Some(badges) = judge_badges {
            money = money.merge(apps::judge_routes(apps::JudgeKit {
                runner: self.day_runner.clone(),
                badges,
            }));
        }

        // Operators' live activity: this node's wallets, and the sources it doesn't connect
        // named as such. Local and owner-token only here; the node serves its own on the ops
        // host behind operator sign-in.
        let activity = self.activity.clone().unwrap_or_else(|| {
            let wallets: std::collections::HashMap<String, Arc<dyn apps::Wallets>> = self
                .app_sites
                .iter()
                .map(|s| (s.site.clone(), Arc::clone(&s.wallet)))
                .collect();
            let sites = self
                .app_sites
                .iter()
                .map(|s| (s.site.clone(), s.site.clone()))
                .collect();
            activity::Activity::new()
                .with_source(Arc::new(activity::WalletSource::new(
                    Arc::new(activity::PerSite(wallets)),
                    sites,
                )))
                .with_feed(Arc::clone(&shared.feed), Arc::clone(&shared.today))
                .not_connected("Signed audit logs")
                .not_connected("Model gateway usage (control.db)")
                .not_connected("Kernel events")
        });
        money = money.merge(activity::router(
            activity,
            Arc::new(activity::LocalGate(shared.access.clone())),
        ));

        let pay = self
            .door
            .router(
                &self.hosts.pay,
                Arc::clone(&self.feed),
                Arc::clone(&self.today),
            )?
            .merge(statics(self.grid_js.clone()));
        let books = books::router(self.era_url).merge(statics(None));

        let sites = Arc::new(Sites {
            hosts: self.hosts,
            money,
            kilele,
            pay,
            books,
        });
        Ok(Router::new()
            .fallback(dispatch)
            .with_state(sites)
            .layer(tower_http::trace::TraceLayer::new_for_http()))
    }
}

/// `<repo>/web/saas-board/dist`, where `npm run build` writes the board.
pub fn default_board_dist() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/saas-board/dist")
}

pub(crate) struct Shared {
    pub(crate) hosts: Hosts,
    pub(crate) feed: Arc<dyn FeedSource>,
    pub(crate) console: Arc<dyn Console>,
    pub(crate) desk: Arc<dyn Desk>,
    pub(crate) today: Today,
    pub(crate) access: Access,
    pub(crate) checkout: Arc<dyn checkout::Checkout>,
    pub(crate) passkeys: Arc<passkeys::Passkeys>,
    pub(crate) proofs_dir: Option<PathBuf>,
    pub(crate) keys: Vec<door::AgentKey>,
    pub(crate) judge_mode: bool,
}

/// Each host's address as a visitor should follow it from this request: the production names
/// over https, or the `.localhost` aliases on this port when the request came in locally.
pub(crate) fn site_links(headers: &HeaderMap, hosts: &Hosts) -> Value {
    let host = host_of(headers);
    let (name, port) = match host.rsplit_once(':') {
        Some((n, p)) if p.bytes().all(|b| b.is_ascii_digit()) => (n, Some(p)),
        _ => (host, None),
    };
    let local = name == "localhost" || name == "127.0.0.1" || name.ends_with(".localhost");
    let at = |site: Site, alias: &str| {
        if local {
            match port {
                Some(p) => format!("http://{alias}.localhost:{p}"),
                None => format!("http://{alias}.localhost"),
            }
        } else {
            format!("https://{}", hosts.name(site))
        }
    };
    json!({
        "money": at(Site::Money, "money"),
        "kilele": at(Site::Kilele, "kilele.money"),
        "pay": at(Site::Pay, "pay"),
        "books": at(Site::Books, "books"),
    })
}

async fn judges(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    let feed = match ctx.shared.feed.feed((ctx.shared.today)()).await {
        Ok(f) => f,
        Err(e) => return failed(e),
    };
    let plan = feed.debt_plans.first().map(|p| p.invoice_number.clone());
    // The Verify box's DIDs (the cards are on each site's /agents page).
    let badges: Vec<Value> = feed
        .companies
        .iter()
        .flat_map(|c| {
            let host = if c.id == KILELE { ctx.shared.hosts.kilele.clone() } else { ctx.shared.hosts.money.clone() };
            c.agents.iter().map(move |a| {
                json!({"role": format!("{} · {}", c.name, a.name), "did": apps::demo::DemoBadges::did(&host, &a.id)})
            })
        })
        .collect();
    pages::render(
        "saas/judges.html",
        json!({
            "links": site_links(&headers, &ctx.shared.hosts),
            "plan": plan,
            "source": feed.source,
            "run_day": ctx.shared.judge_mode,
            "badges": badges,
            "compact": true,
            "token_url": null,
            "note": null,
            "error": null,
            "verify_url": "/judges/verify",
        }),
    )
}

/// `<repo>/specs/saas`, where the company session commits `PROOFS.md` and `proofs/`.
pub fn default_proofs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../specs/saas")
}

/// `https://money.agentrix.website`, or `http://money.localhost:18183` locally: where this
/// request came in, for PayPal's return links.
fn origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
    let local = name == "localhost" || name == "127.0.0.1" || name.ends_with(".localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|p| *p == "http" || *p == "https")
        .unwrap_or(if local { "http" } else { "https" });
    format!("{scheme}://{host}")
}

#[derive(Clone)]
pub(crate) struct Ctx {
    pub(crate) shared: Arc<Shared>,
    pub(crate) company: &'static str,
}

struct Sites {
    hosts: Hosts,
    money: Router,
    kilele: Router,
    pay: Router,
    books: Router,
}

async fn dispatch(State(sites): State<Arc<Sites>>, req: Request) -> Response {
    let router = match sites.hosts.site_of(req.headers()) {
        Some(Site::Money) => sites.money.clone(),
        Some(Site::Kilele) => sites.kilele.clone(),
        Some(Site::Pay) => sites.pay.clone(),
        Some(Site::Books) => sites.books.clone(),
        None => {
            return (
                StatusCode::MISDIRECTED_REQUEST,
                "This server doesn't answer for that host.",
            )
                .into_response();
        }
    };
    match router.oneshot(req).await {
        Ok(r) => r,
        Err(never) => match never {},
    }
}

fn statics<S: Clone + Send + Sync + 'static>(grid_js: Option<PathBuf>) -> Router<S> {
    agentrix_ui::assets().merge(apps::saas_assets(grid_js))
}

fn console_routes(r: Router<Ctx>) -> Router<Ctx> {
    r.route("/console", get(console_page))
        .route("/console/freeze", post(freeze))
        .route("/console/agents/{agent}/revoke", post(revoke_agent))
        .route(
            "/console/agents/{agent}/restore/challenge",
            get(restore_challenge),
        )
        .route("/console/agents/{agent}/restore", post(restore_agent))
        .route(
            "/console/passkey/options",
            get(passkey_options).post(passkey_options),
        )
        .route("/console/passkey", post(passkey_register))
        .route(
            "/console/decisions/{id}/approve/challenge",
            get(approve_challenge),
        )
        .route("/console/decisions/{id}/{verb}", post(decide))
}

fn board_routes(dist: Option<PathBuf>, licence: Option<String>, door: String) -> Router {
    let config = json!({ "licenseKey": licence, "doorHost": door });
    let index = dist
        .as_ref()
        .map(|d| d.join("index.html"))
        .filter(|p| p.is_file());
    let page = move || {
        let index = index.clone();
        async move {
            match index.and_then(|p| std::fs::read(p).ok()) {
                Some(html) => (
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, "no-cache"),
                        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    ],
                    html,
                )
                    .into_response(),
                None => pages::render_status(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "saas/board_missing.html",
                    json!({}),
                ),
            }
        }
    };
    let mut r = Router::new()
        .route("/board", get(|| async { Redirect::permanent("/board/") }))
        .route("/board/", get(page))
        .route(
            "/board/config.json",
            get(move || {
                let config = config.clone();
                async move { ([(header::CACHE_CONTROL, "no-store")], axum::Json(config)) }
            }),
        );
    if let Some(dist) = dist {
        r = r.nest_service(
            "/board/assets",
            tower_http::services::ServeDir::new(dist.join("assets")),
        );
    }
    r
}

async fn front(State(ctx): State<Ctx>) -> Response {
    let feed = match ctx.shared.feed.feed((ctx.shared.today)()).await {
        Ok(f) => f,
        Err(e) => return failed(e),
    };
    let agents: Vec<Value> = feed
        .company(AGENTRIX)
        .map(|c| {
            pages::console_view(&feed, AGENTRIX, false, "")["agents"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .zip(&c.agents)
                .map(|(mut v, a)| {
                    v["job"] = json!(a.job);
                    v
                })
                .collect()
        })
        .unwrap_or_default();
    let mut ctx_json = pages::plans_view();
    ctx_json["desks"] = json!(desk::DESKS);
    ctx_json["agents"] = json!(agents);
    ctx_json["door"] = json!(ctx.shared.hosts.pay);
    ctx_json["host"] = json!(ctx.shared.hosts.money);
    pages::render("saas/front.html", ctx_json)
}

async fn plans_page() -> Response {
    pages::render("saas/plans.html", pages::plans_view())
}

#[derive(Deserialize)]
struct PlanQuery {
    plan: Option<String>,
    period: Option<String>,
}

fn plan_view(id: Option<&str>) -> Option<Value> {
    pages::plans_view()["plans"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["id"].as_str() == id))
        .cloned()
}

fn subscribe_page(
    ctx: &Ctx,
    plan: Option<&str>,
    period: checkout::Period,
    error: Option<String>,
) -> Response {
    let status = if error.is_some() {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::OK
    };
    pages::render_status(
        status,
        "saas/subscribe.html",
        json!({
            "plan": plan_view(plan),
            "period": period.as_str(),
            "connected": ctx.shared.checkout.connected(),
            "error": error,
        }),
    )
}

async fn subscribe(State(ctx): State<Ctx>, Query(q): Query<PlanQuery>) -> Response {
    subscribe_page(
        &ctx,
        q.plan.as_deref(),
        checkout::Period::parse(q.period.as_deref()),
        None,
    )
}

#[derive(Deserialize)]
struct SubscribeForm {
    plan: String,
    period: Option<String>,
    email: Option<String>,
}

async fn subscribe_start(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    form: axum::Form<SubscribeForm>,
) -> Response {
    let period = checkout::Period::parse(form.period.as_deref());
    if plan_view(Some(&form.plan)).is_none() {
        return subscribe_page(&ctx, None, period, None);
    }
    let base = origin(&headers);
    let email = form
        .email
        .as_deref()
        .map(str::trim)
        .filter(|e| e.contains('@') && e.len() <= 254)
        .map(str::to_owned);
    let started = ctx
        .shared
        .checkout
        .start(checkout::Start {
            plan: form.plan.clone(),
            period,
            email,
            return_url: format!("{base}/subscribe/return"),
            cancel_url: format!("{base}/subscribe/cancelled?plan={}", form.plan),
        })
        .await;
    match started {
        // Only PayPal's own pages: a checkout that hands back anything else is refused.
        Ok(s)
            if s.approve_url.starts_with("https://www.sandbox.paypal.com/")
                || s.approve_url.starts_with("https://www.paypal.com/") =>
        {
            Redirect::to(&s.approve_url).into_response()
        }
        Ok(_) => subscribe_page(
            &ctx,
            Some(&form.plan),
            period,
            Some("PayPal's approval page didn't come back. Nothing was charged.".into()),
        ),
        Err(e) => subscribe_page(&ctx, Some(&form.plan), period, Some(e.to_string())),
    }
}

#[derive(Deserialize)]
struct ReturnQuery {
    subscription_id: Option<String>,
}

fn paypal_subscription_id(q: &ReturnQuery) -> Option<&str> {
    q.subscription_id.as_deref().filter(|id| {
        !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

async fn subscribe_return(State(ctx): State<Ctx>, Query(q): Query<ReturnQuery>) -> Response {
    let Some(id) = paypal_subscription_id(&q) else {
        return pages::render_status(
            StatusCode::BAD_REQUEST,
            "saas/subscribe_return.html",
            json!({"status": null, "message": "PayPal didn't say which subscription this is.", "id": ""}),
        );
    };
    let (status, words) = match ctx.shared.checkout.status(id).await {
        Ok(s) => {
            let w = checkout::words(&s);
            (Some(s), w)
        }
        Err(e) => (None, e.to_string()),
    };
    let done = status.as_ref().is_some_and(checkout::is_done);
    pages::render(
        "saas/subscribe_return.html",
        json!({"status": status, "message": words, "id": id, "done": done}),
    )
}

async fn subscribe_status(State(ctx): State<Ctx>, Query(q): Query<ReturnQuery>) -> Response {
    let Some(id) = paypal_subscription_id(&q) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match ctx.shared.checkout.status(id).await {
        Ok(s) => {
            let body =
                json!({"status": s, "words": checkout::words(&s), "done": checkout::is_done(&s)});
            ([(header::CACHE_CONTROL, "no-store")], axum::Json(body)).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({"words": e.to_string(), "done": false})),
        )
            .into_response(),
    }
}

async fn subscribe_cancelled(Query(q): Query<PlanQuery>) -> Response {
    pages::render(
        "saas/subscribe_return.html",
        json!({"status": null, "message": "You cancelled on PayPal. Nothing was charged.", "id": "", "plan": plan_view(q.plan.as_deref()), "cancelled": true}),
    )
}

async fn desk_reply(
    State(ctx): State<Ctx>,
    Path(desk_id): Path<String>,
    body: axum::Json<desk::Ask>,
) -> Response {
    if !desk::DESKS.iter().any(|d| d.id == desk_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let q: String = body.q.chars().take(500).collect();
    if q.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "Ask something.").into_response();
    }
    axum::Json(ctx.shared.desk.reply(&desk_id, q.trim()).await).into_response()
}

async fn feed_json(State(ctx): State<Ctx>) -> Response {
    match ctx.shared.feed.feed((ctx.shared.today)()).await {
        Ok(f) => {
            let f = if ctx.company == KILELE {
                f.for_company(KILELE)
            } else {
                f
            };
            ([(header::CACHE_CONTROL, "no-store")], axum::Json(f)).into_response()
        }
        Err(e) => failed(e),
    }
}

#[derive(Deserialize)]
struct KeyQuery {
    key: Option<String>,
}

async fn console_page(
    State(ctx): State<Ctx>,
    Query(q): Query<KeyQuery>,
    headers: HeaderMap,
) -> Response {
    if let (Some(given), Some(token)) = (q.key.as_deref(), ctx.shared.access.owner_token.as_deref())
    {
        if token.len() >= 16 && constant_eq(given, token) {
            let cookie = format!(
                "saas_owner={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=2592000"
            );
            return ([(header::SET_COOKIE, cookie)], Redirect::to("/console")).into_response();
        }
    }
    let feed = match ctx.shared.feed.feed((ctx.shared.today)()).await {
        Ok(f) => f,
        Err(e) => return failed(e),
    };
    let may_act = ctx.shared.access.allows(&headers);
    let mut view = pages::console_view(&feed, ctx.company, may_act, "");
    view["passkey"] = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(party) => {
            json!({"ready": ctx.shared.passkeys.has(ctx.company, &party), "rp_id": party.rp_id, "can_set_up": may_act})
        }
        Err(_) => json!({"ready": false, "rp_id": null, "can_set_up": false}),
    };
    pages::render("saas/console.html", view)
}

pub(crate) fn host_of(headers: &HeaderMap) -> &str {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
}

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, axum::Json(json!({"message": message.into()}))).into_response()
}

async fn passkey_options(State(ctx): State<Ctx>, headers: HeaderMap) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Only the owner can set up the console's passkey.",
        );
    }
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let owner = match ctx.shared.feed.feed((ctx.shared.today)()).await {
        Ok(f) => f
            .company(ctx.company)
            .map_or_else(|| "Owner".to_owned(), |c| c.owner.clone()),
        Err(e) => return failed(e),
    };
    match ctx
        .shared
        .passkeys
        .start_registration(ctx.company, &owner, &party)
    {
        Ok(options) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(options)).into_response(),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn passkey_register(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Only the owner can set up the console's passkey.",
        );
    }
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let Ok(credential) = serde_json::from_slice(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "That isn't a passkey.");
    };
    match ctx.shared.passkeys.finish_registration(ctx.company, &credential, &party) {
        Ok(()) => axum::Json(json!({"message": format!("Passkey set up for {}. Approvals here now need it.", party.rp_id)})).into_response(),
        Err(e) => json_error(StatusCode::UNAUTHORIZED, e.to_string()),
    }
}

/// The open decision `id` of this console's company, from the feed.
async fn open_decision(ctx: &Ctx, id: &str) -> Result<Option<feed::Decision>, anyhow::Error> {
    let f = ctx.shared.feed.feed((ctx.shared.today)()).await?;
    Ok(f.decisions
        .into_iter()
        .find(|d| d.id == id && d.company == ctx.company && d.state == "open"))
}

async fn approve_challenge(
    State(ctx): State<Ctx>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return json_error(StatusCode::FORBIDDEN, "Only the owner can decide.");
    }
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::NOT_FOUND, e.to_string()),
    };
    if !ctx.shared.passkeys.has(ctx.company, &party) {
        return json_error(
            StatusCode::NOT_FOUND,
            "No passkey is set up for this console yet.",
        );
    }
    let decision = match open_decision(&ctx, &id).await {
        Ok(Some(d)) => d,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "Nothing waits here by that name."),
        Err(e) => return failed(e),
    };
    match ctx.shared.passkeys.start_approval(
        ctx.company,
        &decision.id,
        &decision.fingerprint,
        &party,
    ) {
        Ok(options) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(options)).into_response(),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

#[derive(Deserialize)]
struct FreezeForm {
    frozen: String,
}

async fn freeze(
    State(ctx): State<Ctx>,
    headers: HeaderMap,
    form: axum::Form<FreezeForm>,
) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return (
            StatusCode::FORBIDDEN,
            "Only the owner can freeze the agents.",
        )
            .into_response();
    }
    let frozen = form.frozen == "1";
    match ctx.shared.console.set_frozen(ctx.company, frozen).await {
        Ok(()) => Redirect::to("/console").into_response(),
        Err(e) => (StatusCode::CONFLICT, e.words()).into_response(),
    }
}

/// An agent's role in a path: what the story and the kernel use (`cashier`, `payables`).
fn agent_of(agent: &str) -> Option<&str> {
    (!agent.is_empty()
        && agent.len() <= 64
        && agent
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'))
    .then_some(agent)
}

fn console_failed(e: &console::ConsoleError) -> Response {
    let status = match e {
        console::ConsoleError::NotFound => StatusCode::NOT_FOUND,
        console::ConsoleError::BadProof(_) => StatusCode::UNAUTHORIZED,
        console::ConsoleError::NotConnected => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::CONFLICT,
    };
    json_error(status, e.words())
}

/// The ceremony key a restore is filed under in the passkey store.
fn restore_key(agent: &str) -> String {
    format!("restore:{agent}")
}

/// `POST /console/agents/{agent}/revoke`: switches one agent off now. No passkey: stopping an
/// agent never waits.
async fn revoke_agent(
    State(ctx): State<Ctx>,
    Path(agent): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return json_error(StatusCode::FORBIDDEN, "Only the owner can revoke an agent.");
    }
    let Some(agent) = agent_of(&agent) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    match ctx
        .shared
        .console
        .set_revoked(ctx.company, agent, true, None)
        .await
    {
        Ok(record) if wants_json => axum::Json(json!({
            "agent": agent,
            "revoked": true,
            "record": record,
            "message": "Revoked. Its tokens stop working at its next call; no PayPal key was rotated.",
        }))
        .into_response(),
        Ok(_) => Redirect::to("/console#staff").into_response(),
        Err(e) => console_failed(&e),
    }
}

/// What switching `agent` back on signs, while the feed shows it revoked.
async fn restore_subject(ctx: &Ctx, agent: &str) -> Result<String, Response> {
    let feed = ctx
        .shared
        .feed
        .feed((ctx.shared.today)())
        .await
        .map_err(failed)?;
    let revoked = feed
        .companies
        .iter()
        .filter(|c| c.id == ctx.company)
        .flat_map(|c| c.agents.iter())
        .any(|a| a.id == agent && a.state == "revoked");
    if !revoked {
        return Err(json_error(
            StatusCode::NOT_FOUND,
            "That agent isn't revoked.",
        ));
    }
    Ok(console::restore_fingerprint(ctx.company, agent))
}

/// `GET /console/agents/{agent}/restore/challenge`: passkey options over the restore's
/// fingerprint. There is no other way back on.
async fn restore_challenge(
    State(ctx): State<Ctx>,
    Path(agent): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Only the owner can switch an agent back on.",
        );
    }
    let Some(agent) = agent_of(&agent) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    if !ctx.shared.passkeys.has(ctx.company, &party) {
        return json_error(
            StatusCode::PRECONDITION_REQUIRED,
            "Switching an agent back on takes your passkey. Set it up first, then try again.",
        );
    }
    let fp = match restore_subject(&ctx, agent).await {
        Ok(fp) => fp,
        Err(r) => return r,
    };
    match ctx
        .shared
        .passkeys
        .start_approval(ctx.company, &restore_key(agent), &fp, &party)
    {
        Ok(options) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(options)).into_response(),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

/// `POST /console/agents/{agent}/restore` with `{proof: <PublicKeyCredential>}`: checks the
/// owner's passkey over the restore, then switches the agent back on.
async fn restore_agent(
    State(ctx): State<Ctx>,
    Path(agent): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !ctx.shared.access.allows(&headers) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Only the owner can switch an agent back on.",
        );
    }
    let Some(agent) = agent_of(&agent) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let parsed: DecideBody = serde_json::from_slice(&body).unwrap_or_default();
    let Some(credential) = parsed.proof.and_then(|p| serde_json::from_value(p).ok()) else {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "Switch it back on with your passkey.",
        );
    };
    let evidence = match ctx.shared.passkeys.finish_approval(
        ctx.company,
        &restore_key(agent),
        &credential,
        &party,
    ) {
        Ok(e) => e,
        Err(e) => return json_error(StatusCode::UNAUTHORIZED, e.to_string()),
    };
    // What was signed must be this agent's restore, and it must still be revoked.
    match restore_subject(&ctx, agent).await {
        Ok(fp) if evidence.get("fingerprint").and_then(Value::as_str) == Some(fp.as_str()) => {}
        Ok(_) => {
            return json_error(
                StatusCode::CONFLICT,
                "Your passkey signed something else. Nothing changed; try again.",
            );
        }
        Err(r) => return r,
    }
    match ctx
        .shared
        .console
        .set_revoked(ctx.company, agent, false, Some(evidence))
        .await
    {
        Ok(record) => axum::Json(json!({
            "record": record,
            "message": "Switched back on with your passkey. It gets a new token at its next call.",
        }))
        .into_response(),
        Err(e) => console_failed(&e),
    }
}

#[derive(Deserialize, Default)]
struct DecideBody {
    proof: Option<Value>,
}

async fn decide(
    State(ctx): State<Ctx>,
    Path((id, verb)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"));
    let approve = match verb.as_str() {
        "approve" => true,
        "decline" => false,
        // The approval sheet's authenticator-code fallback: this console has no codes, so it
        // never approves on one.
        "code" => {
            return (
                StatusCode::BAD_REQUEST,
                "Authenticator codes aren't set up on this console. Go back and approve with your passkey. Nothing was approved.",
            )
                .into_response();
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if !ctx.shared.access.allows(&headers) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(json!({"message": "Only the owner can decide."})),
        )
            .into_response();
    }
    let parsed: DecideBody = serde_json::from_slice(&body).unwrap_or_default();
    // With a passkey set up for this console, an approval must carry the owner's assertion
    // over this decision's fingerprint; the proof the console keeps says which passkey signed.
    let mut proof = None;
    if approve {
        if let Ok(party) = passkeys::Party::for_host(host_of(&headers)) {
            if ctx.shared.passkeys.has(ctx.company, &party) {
                let credential = parsed
                    .proof
                    .as_ref()
                    .and_then(|p| serde_json::from_value(p.clone()).ok());
                let Some(credential) = credential else {
                    return json_error(StatusCode::UNAUTHORIZED, "Approve with your passkey.");
                };
                match ctx
                    .shared
                    .passkeys
                    .finish_approval(ctx.company, &id, &credential, &party)
                {
                    Ok(p) => proof = Some(p),
                    Err(e) => return json_error(StatusCode::UNAUTHORIZED, e.to_string()),
                }
            }
        }
    }
    let signed = proof.is_some();
    let result = ctx
        .shared
        .console
        .decide(ctx.company, &id, approve, proof)
        .await;
    match (result, wants_json) {
        (Ok(outcome), true) => {
            let message = match (approve, signed) {
                (true, true) => "Approved with your passkey. Payables pays it once.",
                (true, false) => "Approved. Payables pays it once.",
                (false, _) => "Declined. Nothing was paid.",
            };
            axum::Json(json!({"outcome": outcome, "message": message})).into_response()
        }
        (Ok(_), false) => Redirect::to("/console").into_response(),
        (Err(e), _) => console_failed(&e),
    }
}

pub(crate) fn failed(e: anyhow::Error) -> Response {
    tracing::error!(error = %e, "reading the feed");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "The books didn't answer. Try again in a moment.",
    )
        .into_response()
}
