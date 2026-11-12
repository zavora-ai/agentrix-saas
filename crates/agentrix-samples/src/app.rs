//! The samples' shared state: the records store (opened and loaded on first use), templates,
//! and the ports.

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_host::{NodeContext, OwnerSessions, Receipts};
use agentrix_records::Records;
use anyhow::Context as _;
use minijinja::{Environment, UndefinedBehavior};
use tokio::sync::OnceCell;

use crate::ports::{AnswerModel, GatewayModel, Payments, RegisteredPayments, Visitors};
use crate::{SAMPLES, web};

include!(concat!(env!("OUT_DIR"), "/bundles.rs"));

/// Where the samples run.
#[derive(Debug, Clone)]
pub struct Config {
    /// `<node data>/agentrix-samples`: `records.db` and the unpacked bundles.
    pub data_dir: PathBuf,
    /// The platform's host; sample hosts are `<slug>.<platform host>`.
    pub platform_host: String,
    /// `https` in production, `http` in previews.
    pub scheme: String,
    /// The port browsers use, when not the default for the scheme.
    pub port: Option<u16>,
    /// Read bundles from this folder instead of the embedded copies (development).
    pub bundles_dir: Option<PathBuf>,
    /// Read templates from this folder on every render (development).
    pub templates_dir: Option<PathBuf>,
    /// Reload every bundle at startup even when its version is loaded.
    pub reseed: bool,
}

impl Config {
    /// The configuration node's context implies, with `AGENTRIX_SAMPLES_*` overrides.
    pub fn from_context(ctx: &NodeContext) -> Self {
        Self {
            data_dir: ctx.data_dir.join("agentrix-samples"),
            platform_host: ctx.platform_host.clone(),
            scheme: ctx.origin.scheme().to_owned(),
            port: ctx.origin.port(),
            bundles_dir: std::env::var_os("AGENTRIX_SAMPLES_BUNDLES").map(PathBuf::from),
            templates_dir: std::env::var_os("AGENTRIX_SAMPLES_TEMPLATES").map(PathBuf::from),
            reseed: std::env::var("AGENTRIX_SAMPLES_RESEED").is_ok_and(|v| v == "1"),
        }
    }

    /// Every sample host node should route here.
    pub fn sample_hosts(&self) -> Vec<String> {
        SAMPLES
            .iter()
            .map(|(slug, _)| format!("{slug}.{}", self.platform_host))
            .collect()
    }

    /// `https://kilele-bakery-sample.agentrix.website`.
    pub fn origin_for(&self, host: &str) -> String {
        match self.port {
            Some(p) => format!("{}://{host}:{p}", self.scheme),
            None => format!("{}://{host}", self.scheme),
        }
    }

    /// The platform's own origin.
    pub fn platform_origin(&self) -> String {
        self.origin_for(&self.platform_host)
    }
}

/// The samples' state.
pub struct Inner {
    /// Configuration.
    pub config: Config,
    records: Arc<OnceCell<Records>>,
    /// Sites by slug, cached when the bundles load (for the synchronous account shell).
    pub sites: Arc<std::sync::RwLock<std::collections::HashMap<String, agentrix_records::Site>>>,
    /// Templates.
    pub env: Environment<'static>,
    /// Customer sign-in.
    pub visitors: Arc<dyn Visitors>,
    /// Payments.
    pub payments: Arc<dyn Payments>,
    /// The Model Gateway.
    pub model: Arc<dyn AnswerModel>,
    /// Who is signed in as an owner on the platform host.
    pub owners: Arc<dyn OwnerSessions>,
    /// Signed receipts.
    pub receipts: Arc<dyn Receipts>,
    /// Node's `control.db` (W11's approved pictures live there).
    pub control: sqlx::SqlitePool,
    /// W9's messages: order and digest emails, recorded in the outbox first.
    pub messages: agentrix_messages::Messages,
    /// W11's pictures through node's gateway and content store: the cake studio's ideas.
    pub media: agentrix_media::routes::MediaState,
    /// The content store (W1's rosters keep marks there).
    pub blobs: Arc<dyn agentrix_host::Blobs>,
}

/// A cheap handle on the samples' state.
#[derive(Clone)]
pub struct App(Arc<Inner>);

impl std::ops::Deref for App {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl App {
    /// The samples' state for node's context. Sign-in and payments start as stand-ins; node
    /// replaces them with W6's through [`App::with_visitors`] and [`App::with_payments`].
    ///
    /// # Errors
    ///
    /// When the data directory can't be created.
    pub fn new(config: Config, ctx: &NodeContext) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("creating {}", config.data_dir.display()))?;
        let env = templates(config.templates_dir.clone());
        let records = Arc::new(OnceCell::new());
        Ok(Self(Arc::new(Inner {
            visitors: Arc::new(web::StandInVisitors::new(records.clone())),
            payments: Arc::new(RegisteredPayments),
            model: Arc::new(GatewayModel::new(&ctx.models_url, ctx.http.clone())),
            owners: ctx.owners.clone(),
            receipts: ctx.receipts.clone(),
            control: ctx.pool.clone(),
            messages: agentrix_messages::from_context(ctx),
            media: agentrix_media::routes::MediaState::new(ctx),
            blobs: ctx.blobs.clone(),
            records,
            sites: Arc::default(),
            env,
            config,
        })))
    }

    /// Uses W6's customer sign-in.
    pub fn with_visitors(self, visitors: Arc<dyn Visitors>) -> Self {
        self.rebuild(|i| i.visitors = visitors)
    }

    /// Uses W6's payments.
    pub fn with_payments(self, payments: Arc<dyn Payments>) -> Self {
        self.rebuild(|i| i.payments = payments)
    }

    /// Uses another answer model (tests).
    pub fn with_model(self, model: Arc<dyn AnswerModel>) -> Self {
        self.rebuild(|i| i.model = model)
    }

    fn rebuild(self, change: impl FnOnce(&mut Inner)) -> Self {
        let inner = Arc::try_unwrap(self.0).unwrap_or_else(|arc| Inner {
            config: arc.config.clone(),
            records: arc.records.clone(),
            sites: arc.sites.clone(),
            env: templates(arc.config.templates_dir.clone()),
            visitors: arc.visitors.clone(),
            payments: arc.payments.clone(),
            model: arc.model.clone(),
            owners: arc.owners.clone(),
            receipts: arc.receipts.clone(),
            control: arc.control.clone(),
            messages: arc.messages.clone(),
            media: arc.media.clone(),
            blobs: arc.blobs.clone(),
        });
        let mut inner = inner;
        change(&mut inner);
        Self(Arc::new(inner))
    }

    /// The records store, opened and loaded with the bundles on first use.
    ///
    /// # Errors
    ///
    /// When the store can't be opened or a bundle fails to load.
    pub async fn records(&self) -> anyhow::Result<&Records> {
        self.records
            .get_or_try_init(|| async {
                let records = Records::open(&self.config.data_dir.join("records.db")).await?;
                let now = agentrix_records::time::now();
                for (slug, folder) in SAMPLES {
                    let dir = match &self.config.bundles_dir {
                        Some(d) => d.join(folder),
                        None => unpack(&self.config.data_dir, folder)?,
                    };
                    if !dir.join("manifest.json").exists() {
                        tracing::warn!(folder, "no sample bundle yet");
                        continue;
                    }
                    let host = format!("{slug}.{}", self.config.platform_host);
                    let loaded = records
                        .load_bundle(&dir, Some(&host), self.config.reseed, now)
                        .await
                        .with_context(|| format!("loading the {folder} bundle"))?;
                    tracing::info!(?loaded, "sample bundle");
                }
                let sites = records.sites().await?;
                // Each sample is a site the platform knows (control.db's `sites`), so the Model
                // Gateway serves its desks (it refuses sites it doesn't know) and the console and
                // the operator's fleet see it. Inserted once; an existing row stays as it is.
                for site in &sites {
                    let origin = self.config.origin_for(&site.host);
                    if let Err(e) = register_site(&self.control, site, &origin).await {
                        tracing::warn!(error = %format!("{e:#}"), site = site.site, "sample not registered in control.db");
                    }
                }
                // Each sample's team in W1's rosters (look, who's on duty, the section desks),
                // so the console's dials and the Workshop know it. Safe to run again; a node
                // without the kernel's tables just logs it.
                let rosters = agentrix_kernel::Rosters::new(self.control.clone(), self.blobs.clone());
                for site in &sites {
                    let key = site.fact("/mark/key");
                    if key.is_empty() {
                        continue;
                    }
                    if let Err(e) = rosters.seed_sample(&site.site, &key).await {
                        tracing::info!(error = %format!("{e:#}"), site = site.site, "sample roster not seeded");
                    }
                }
                if let Ok(mut cache) = self.sites.write() {
                    for site in sites {
                        cache.insert(site.site.clone(), site);
                    }
                }
                Ok::<_, anyhow::Error>(records)
            })
            .await
    }

    /// Renders a template with a view.
    ///
    /// # Errors
    ///
    /// When the template is missing or the view lacks a field it uses (strict mode).
    pub fn render(&self, name: &str, view: &impl serde::Serialize) -> anyhow::Result<String> {
        if let Some(dir) = &self.config.templates_dir {
            let env = templates(Some(dir.clone()));
            return Ok(env.get_template(name)?.render(view)?);
        }
        Ok(self.env.get_template(name)?.render(view)?)
    }

    /// The approved pictures for a site by slot (`hero`, `product:<sku>`), from W11's library.
    /// Empty when there are none or the library can't be read: pages fall back to drawn tones.
    pub async fn pictures(
        &self,
        site: &str,
    ) -> std::collections::HashMap<String, agentrix_media::library::Shown> {
        agentrix_media::library::shown(&self.control, site)
            .await
            .unwrap_or_else(|e| {
                tracing::debug!(error = %e, site, "no pictures");
                Default::default()
            })
    }

    /// Hourly: lapses held or unpaid requests older than two days and tells each customer, and
    /// sends Tana's Friday five when it's due.
    pub async fn lapse_loop(&self) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            let Ok(records) = self.records().await else {
                continue;
            };
            let Ok(sites) = records.sites().await else {
                continue;
            };
            for site in sites {
                let now = agentrix_records::time::now();
                match records.lapse_stale(&site, 48, now).await {
                    Ok(lapsed) => {
                        let (host, origin) = self.site_origin(&site);
                        for view in &lapsed {
                            let event = agentrix_messages::orders::OrderEvent::RequestLapsed {
                                expired_at: agentrix_records::time::fmt_day_clock(
                                    now,
                                    site.utc_offset_min,
                                ),
                            };
                            self.tell_customer(&site, &host, &origin, view, event).await;
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, site = site.site, "lapse check failed"),
                }
            }
            if let Err(e) = crate::news::friday_digest(self).await {
                tracing::warn!(error = %format!("{e:#}"), "Friday digest check failed");
            }
        }
    }
}

/// Writes an embedded bundle under the data directory and returns its folder.
fn unpack(data_dir: &std::path::Path, folder: &str) -> anyhow::Result<PathBuf> {
    let dir = data_dir.join("bundles").join(folder);
    for (bundle, path, text) in BUNDLES {
        if *bundle != folder {
            continue;
        }
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, text).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(dir)
}

/// Each sample's plan (`specs/platform/facts.md`): Kilele Bakery and Classy Perfumes are on
/// Commerce, with USD 30 of AI included a month, so the console shows their AI budget as
/// KSh 3,870 a month. The sheet records no plan for Tana Valley Times, so it has none here.
pub const SAMPLE_PLANS: [(&str, &str); 2] = [
    ("kilele-bakery-sample", "commerce"),
    ("classy-perfumes-sample", "commerce"),
];

/// Adds a sample to control.db's `sites` if it isn't there. The host's first label ends in
/// `-sample`, which is how the platform's counts tell a sample from a real business; a sample
/// has no old site, so its own origin stands in for one. Its currency and plan fill the
/// registry columns (W3's `sites.currency` and `sites.plan`) where they are empty.
pub(crate) async fn register_site(
    control: &sqlx::SqlitePool,
    site: &agentrix_records::Site,
    origin: &str,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, fixture, created_at) \
         VALUES (?, ?, ?, ?, ?, 0, ?) ON CONFLICT (site_id) DO NOTHING",
    )
    .bind(&site.site)
    .bind(&site.host)
    .bind(&site.name)
    .bind(origin)
    .bind(site.utc_offset_min)
    .bind(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    .execute(control)
    .await?;
    let plan = SAMPLE_PLANS
        .iter()
        .find(|(slug, _)| *slug == site.site)
        .map(|(_, plan)| *plan);
    // A database without W3's registry columns (an older schema) keeps working without them.
    if let Err(e) = sqlx::query(
        "UPDATE sites SET currency = COALESCE(currency, ?), plan = COALESCE(plan, ?) WHERE site_id = ?",
    )
    .bind(&site.currency)
    .bind(plan)
    .bind(&site.site)
    .execute(control)
    .await
    {
        tracing::info!(error = %e, site = site.site, "sample's currency and plan not recorded");
    }
    Ok(done.rows_affected() > 0)
}

fn templates(dir: Option<PathBuf>) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
    match dir {
        Some(dir) => env.set_loader(minijinja::path_loader(dir)),
        None => minijinja_embed::load_templates!(&mut env),
    }
    web::filters(&mut env);
    env
}

#[cfg(test)]
mod tests {
    use super::{App, Config};

    #[tokio::test]
    async fn a_sample_is_registered_with_its_currency_and_plan() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE sites (site_id TEXT PRIMARY KEY, domain TEXT, name TEXT, old_origin TEXT, \
             utc_offset_minutes INTEGER, fixture INTEGER, created_at TEXT, plan TEXT, currency TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let site = |slug: &str| agentrix_records::Site {
            site: slug.into(),
            host: format!("{slug}.localhost"),
            name: "Sample".into(),
            archetype: "shop".into(),
            currency: "KES".into(),
            utc_offset_min: 180,
            origin: "sample".into(),
            facts: serde_json::json!({}),
            seed_version: "1".into(),
            anchor_at: 0,
        };
        for slug in ["kilele-bakery-sample", "tana-valley-times-sample"] {
            assert!(
                super::register_site(&pool, &site(slug), "http://x")
                    .await
                    .unwrap()
            );
        }
        let rows: Vec<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT site_id, plan, currency FROM sites ORDER BY site_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "kilele-bakery-sample".into(),
                    Some("commerce".into()),
                    Some("KES".into())
                ),
                ("tana-valley-times-sample".into(), None, Some("KES".into())),
            ]
        );
        // A plan someone set stays; registering again changes nothing.
        sqlx::query("UPDATE sites SET plan = 'business' WHERE site_id = 'kilele-bakery-sample'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !super::register_site(&pool, &site("kilele-bakery-sample"), "http://x")
                .await
                .unwrap()
        );
        let (plan,): (String,) =
            sqlx::query_as("SELECT plan FROM sites WHERE site_id = 'kilele-bakery-sample'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(plan, "business");
    }

    #[tokio::test]
    async fn loading_the_bundles_seeds_each_samples_team_in_the_rosters() {
        let mut ctx = agentrix_host::testing::context().await.unwrap();
        ctx.platform_host = "localhost".into();
        agentrix_kernel::migrate(&ctx.pool).await.unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config = Config {
            data_dir: std::env::temp_dir()
                .join(format!("agentrix-samples-rosters-{}", std::process::id())),
            platform_host: "localhost".into(),
            scheme: "http".into(),
            port: None,
            bundles_dir: Some(root.join("fixtures/samples")),
            templates_dir: None,
            reseed: true,
        };
        let app = App::new(config, &ctx).unwrap();
        app.records().await.unwrap();
        let rosters = agentrix_kernel::Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        for site in [
            "kilele-bakery-sample",
            "classy-perfumes-sample",
            "tana-valley-times-sample",
        ] {
            let roster = rosters.roster(site).await.unwrap();
            assert!(
                roster.is_some_and(|r| !r.desks().is_empty()),
                "{site} has its desks"
            );
        }
    }
}
