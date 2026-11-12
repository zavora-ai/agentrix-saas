//! Badges (v3, technology 1) on the node: who is this agent, checkable from its DID alone.
//!
//! Each agent door's site gets a saas-badge identity on the same keys its door already signs
//! with (settlement's key directories under `<data_dir>/settle/keys`), so an agent's DID
//! document, its badge and its signed requests all name one key:
//!
//! - Agentrix at node's own origin (`did:web:paypal.agentrix.website`), which also vouches, as
//!   the operator, for every site it hosts, and serves the verifier (`/agents/verify`).
//! - Kilele Bakery at its sample site (`did:web:kilele-bakery-sample.paypal.agentrix.website`).
//!
//! [`Badges::door`] puts a site's badge routes in front of its agent door: the DID documents,
//! badges, voucher, signed agent card and key directory are saas-badge's; `/a2a` and
//! `/settle/` stay settlement's.

use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::header;
use axum::response::IntoResponse;
use saas_badge::did::site_did;
use saas_badge::resolve::{DidKeyResolver, DidWebResolver, KeyDirectoryResolver, Resolvers};
use saas_badge::site::{self, AgentSpec, Operator, SiteIdentity, SiteState, is_badge_path};
use saas_badge::verify::{self, Verifier};
use saas_core::CompanyId;
use saas_core::identity::Sponsor;
use tower::ServiceExt;

use crate::settle::Origins;

/// The two sites' badges and the verifier.
pub struct Badges {
    /// Agentrix, the operator.
    pub agentrix: Arc<SiteState>,
    /// Kilele Bakery.
    pub kilele: Arc<SiteState>,
    /// Checks any agent's DID.
    pub verifier: Arc<Verifier>,
    /// The other sample sites (no agent door of their own yet): their badges and vouchers.
    pub samples: Vec<Arc<SiteState>>,
    operator: Operator,
}

impl std::fmt::Debug for Badges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Badges")
            .field("agentrix", &self.agentrix.identity.did())
            .field("kilele", &self.kilele.identity.did())
            .finish_non_exhaustive()
    }
}

/// `https://host[:port]` → `host[:port]`.
fn host_of(origin: &str) -> String {
    origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .trim_end_matches('/')
        .to_owned()
}

impl Badges {
    /// Both sites over settlement's keys in `<data_dir>/settle/keys`.
    pub fn build(
        origins: &Origins,
        data_dir: &Path,
        agentrix_owner: &str,
        kilele_owner: &str,
    ) -> anyhow::Result<Self> {
        let keys = data_dir.join("settle").join("keys");
        let agentrix_company = CompanyId::agentrix();
        let kilele_company = CompanyId::kilele();
        let agentrix = SiteIdentity::load(
            &host_of(&origins.agentrix),
            "Agentrix",
            agentrix_company.clone(),
            false,
            vec![AgentSpec::cashier(
                &agentrix_company,
                saas_settle::keys::RECEIVABLES,
            )],
            &keys.join("agentrix"),
        )?;
        let kilele = SiteIdentity::load(
            &host_of(&origins.kilele),
            "Kilele Bakery",
            kilele_company.clone(),
            true,
            vec![AgentSpec::cashier(
                &kilele_company,
                saas_settle::keys::PAYABLES,
            )],
            &keys.join("kilele"),
        )?;
        let sponsor = |owner: &str, name: &str| Sponsor {
            owner: owner.to_owned(),
            name: Some(name.to_owned()),
            passkey: None,
            approved_at: None,
        };
        let operator = Operator::new(agentrix.clone());
        let operator_did = site_did(&agentrix.host);
        let agentrix = SiteState::new(agentrix, sponsor(agentrix_owner, "Agentrix's founder"));
        let kilele = SiteState::new(kilele, sponsor(kilele_owner, "Kilele Bakery's owner"));
        let verifier = if origins.public {
            let web = DidWebResolver::new(reqwest::Client::new());
            let resolvers = Resolvers::new()
                .with(Arc::new(web.clone()))
                .with(Arc::new(DidKeyResolver))
                .with(Arc::new(KeyDirectoryResolver::new(Arc::new(
                    adk_auth::web_bot_auth::HttpKeyDirectorySource::new()?,
                ))));
            Verifier::new(web, resolvers, operator_did)
        } else {
            let http = saas_settle::keys::loopback_client(&[&origins.agentrix, &origins.kilele])?;
            let web = DidWebResolver::new(http).allow_loopback();
            let resolvers = Resolvers::new()
                .with(Arc::new(web.clone()))
                .with(Arc::new(DidKeyResolver))
                .with(Arc::new(KeyDirectoryResolver::new(Arc::new(
                    saas_settle::LoopbackDirectorySource::new(),
                ))));
            Verifier::new(web, resolvers, operator_did).accept_local()
        };
        // Every other sample site on the same platform host: its Cashier on a key of its own.
        let platform = host_of(&origins.agentrix);
        let (bare, port) = match platform.split_once(':') {
            Some((h, p)) => (h.to_owned(), Some(p.to_owned())),
            None => (platform.clone(), None),
        };
        let mut samples = Vec::new();
        for (slug, name) in crate::platform::SAMPLE_SITES {
            if slug == crate::platform::KILELE_SAMPLE {
                continue;
            }
            let company = CompanyId::new(slug.trim_end_matches("-sample"))?;
            let host = match &port {
                Some(p) => format!("{slug}.{bare}:{p}"),
                None => format!("{slug}.{bare}"),
            };
            let identity = SiteIdentity::load(
                &host,
                name,
                company.clone(),
                true,
                vec![AgentSpec::cashier(&company, "cashier")],
                &data_dir.join("badges").join("keys").join(slug),
            )?;
            samples.push(SiteState::new(
                identity,
                sponsor(&format!("owner@{slug}.example"), &format!("{name}'s owner")),
            ));
        }
        Ok(Self {
            agentrix,
            kilele,
            verifier: Arc::new(verifier),
            samples,
            operator,
        })
    }

    /// Agentrix vouches for both sites (again after a site's PayPal link changes).
    pub fn vouch(&self) -> anyhow::Result<()> {
        self.operator.vouch(&self.agentrix)?;
        self.operator.vouch(&self.kilele)?;
        for site in &self.samples {
            self.operator.vouch(site)?;
        }
        Ok(())
    }

    /// Signs `door`'s own A2A agent card for `site` (the badge serves the signed one).
    pub async fn adopt_card(site: &SiteState, door: &Router) -> anyhow::Result<()> {
        let response = door
            .clone()
            .oneshot(
                Request::get(saas_settle::door::AGENT_CARD_PATH)
                    .header(header::HOST, site.identity.host.as_str())
                    .body(Body::empty())?,
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
        site.set_card(serde_json::from_slice(&bytes)?).await
    }

    /// `site`'s badge routes in front of `door` (and the verifier on the operator's site).
    pub fn door(&self, site: &Arc<SiteState>, door: Router) -> Router {
        let mut badges = site::router(site.clone());
        if Arc::ptr_eq(site, &self.agentrix) {
            badges = badges.merge(verify::router(self.verifier.clone()));
        }
        Router::new().fallback(move |request: Request| {
            let badges = badges.clone();
            let door = door.clone();
            async move {
                if is_badge_path(request.uri().path()) {
                    badges.oneshot(request).await.into_response()
                } else {
                    door.oneshot(request).await.into_response()
                }
            }
        })
    }
}
