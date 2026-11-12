//! W6's customer accounts on the sample hosts: the [`Visitors`] port over
//! `agentrix_identity::Visitors`, and the shell W6's account pages render inside.

use std::sync::{Arc, OnceLock};

use agentrix_types::tenancy::SiteHost;
use async_trait::async_trait;
use axum::http::HeaderMap;

use crate::App;
use crate::ports::{SampleHost, Visitor, Visitors};

/// `SiteHost` (W6's type) for a sample request.
pub fn site_host(h: &SampleHost) -> SiteHost {
    SiteHost {
        site: agentrix_types::ids::SiteId::new(h.site.clone()),
        host: h.host.clone(),
        origin: h.origin.clone(),
        name: h.name.clone(),
        sample: h.sample,
    }
}

/// The [`Visitors`] port backed by W6's visitor accounts.
pub struct W6Visitors(pub agentrix_identity::Visitors);

#[async_trait]
impl Visitors for W6Visitors {
    async fn visitor(
        &self,
        site: &SampleHost,
        headers: &HeaderMap,
    ) -> anyhow::Result<Option<Visitor>> {
        Ok(self
            .0
            .visitor(&site_host(site), headers)
            .await?
            .map(|v| Visitor {
                account: v.account.to_string(),
                name: v.name,
                email: v.email,
                email_verified: v.email_verified,
                phone: v.phone,
            }))
    }
}

/// The site shell around W6's account pages: the sample's own header, desks, Sample band and
/// footer. Built without live desk facts, since the port is synchronous.
#[derive(Default)]
pub struct SamplesShell {
    app: OnceLock<App>,
}

impl SamplesShell {
    /// Points the shell at the app once it exists.
    pub fn set(&self, app: App) {
        let _ = self.app.set(app);
    }
}

impl agentrix_identity::SiteShell for SamplesShell {
    fn page(&self, site: &SiteHost, title: &str, body: &str, signed_in: Option<&str>) -> String {
        let Some(app) = self.app.get() else {
            return agentrix_identity::PlainShell.page(site, title, body, signed_in);
        };
        match app.account_shell(site.site.as_str(), &site.host, title, signed_in) {
            Some(shell) => app
                .render(
                    "account/page.html",
                    &serde_json::json!({ "shell": shell, "body": body }),
                )
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "account shell failed");
                    agentrix_identity::PlainShell.page(site, title, body, signed_in)
                }),
            None => agentrix_identity::PlainShell.page(site, title, body, signed_in),
        }
    }

    fn offers_line(&self, site: &SiteHost) -> Option<String> {
        Some(match site.site.as_str() {
            "kilele-bakery-sample" => "New bakes and seasonal offers.".into(),
            "classy-perfumes-sample" => "New scents and gift ideas.".into(),
            _ => "News from the editor and subscriber offers.".into(),
        })
    }
}

/// W6's visitors for node's context, with the samples' shell.
pub fn visitors(
    ctx: &agentrix_host::NodeContext,
) -> (agentrix_identity::Visitors, Arc<SamplesShell>) {
    let shell = Arc::new(SamplesShell::default());
    (
        agentrix_identity::Visitors::for_node(ctx, shell.clone()),
        shell,
    )
}
