//! The samples' own knowledge for the Chief WebAgent (W1): the same sections and retrieval Ask
//! answers from, so the Chief and the desks never disagree about a site's facts.

use serde::Serialize;

use crate::App;
use crate::ports::SampleHost;
use crate::web::Ctx;

/// A piece of a sample site that answers a question, with where it is.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Passage {
    /// "Cakes · 2 tiers".
    pub label: String,
    /// The page and anchor on the site's own host.
    pub href: String,
    /// The text.
    pub text: String,
}

/// The `k` passages of a sample site that best match `query`, best first. Empty for a site
/// this crate doesn't serve.
///
/// # Errors
///
/// When the records store can't be read.
pub async fn knowledge_search(
    app: &App,
    site: &str,
    query: &str,
    k: usize,
) -> anyhow::Result<Vec<Passage>> {
    let records = app.records().await?;
    let Some(site) = records.site(site).await? else {
        return Ok(Vec::new());
    };
    let origin = app.config.origin_for(&site.host);
    let ctx = Ctx {
        host: SampleHost {
            site: site.site.clone(),
            host: site.host.clone(),
            origin: origin.clone(),
            name: site.name.clone(),
            sample: site.is_sample(),
        },
        visitor: None,
        path: "/".into(),
        cart: None,
        site,
    };
    let sections = match ctx.site.archetype.as_str() {
        "bakery" => crate::bakery::sections(app, &ctx).await?,
        "shop" => crate::classy::sections(app, &ctx).await?,
        "news" => crate::news::sections(app, &ctx).await?,
        _ => Vec::new(),
    };
    Ok(crate::ask::retrieve(&sections, query, k)
        .into_iter()
        .map(|s| Passage {
            label: s.label.clone(),
            href: format!("{origin}{}", s.href),
            text: s.text.clone(),
        })
        .collect())
}

/// The samples' pages as the Chief WebAgent's site knowledge (W1's `chief::SiteKnowledge`).
pub struct Knowledge(pub App);

#[async_trait::async_trait]
impl agentrix_kernel::chief::SiteKnowledge for Knowledge {
    async fn search(
        &self,
        site: &str,
        query: &str,
        k: usize,
    ) -> anyhow::Result<Vec<agentrix_kernel::chief::Passage>> {
        Ok(knowledge_search(&self.0, site, query, k)
            .await?
            .into_iter()
            .map(|p| agentrix_kernel::chief::Passage {
                label: p.label,
                href: p.href,
                text: p.text,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use crate::{App, Config};

    #[tokio::test]
    async fn the_chief_finds_kilele_s_hours_and_cake_prices() {
        let mut ctx = agentrix_host::testing::context().await.unwrap();
        ctx.platform_host = "localhost".into();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config = Config {
            data_dir: std::env::temp_dir()
                .join(format!("agentrix-samples-knowledge-{}", std::process::id())),
            platform_host: "localhost".into(),
            scheme: "http".into(),
            port: None,
            bundles_dir: Some(root.join("fixtures/samples")),
            templates_dir: None,
            reseed: true,
        };
        let app = App::new(config, &ctx).unwrap();
        let hours =
            super::knowledge_search(&app, "kilele-bakery-sample", "opening hours on Sunday", 3)
                .await
                .unwrap();
        assert!(
            hours.iter().any(|p| p.href.ends_with("/visit#visit-hours")),
            "{hours:?}"
        );
        let cakes = super::knowledge_search(&app, "kilele-bakery-sample", "2 tiers cake price", 3)
            .await
            .unwrap();
        assert!(
            cakes.iter().any(|p| p.text.contains("KSh 4,500")),
            "{cakes:?}"
        );
        assert!(
            super::knowledge_search(&app, "nowhere", "anything", 3)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
