//! The home page's facts through W8's metric catalogue (`agentrix-store`), so the home page,
//! the status page and the operator console count the same things the same way.
//!
//! W8's [`Metrics`] gives the counts and the timings: sites and their origin, receipts issued,
//! health reports written, each site's latest finished gauntlet score, and every Move with its
//! step times and model cost. What the catalogue doesn't carry (each attack's outcome, the
//! owner's approval and the seconds from it to live, items left behind, model routing, the
//! fixture's brief) comes from [`SqliteFacts`]'s read-only queries.
//!
//! The rules stay the same as before: only Zavora's own fixture sites are ever named, and the
//! count of live sites leaves the samples out.

use std::collections::BTreeMap;
use std::sync::Arc;

use agentrix_store::metrics::{
    GauntletScore, Metrics, MoveQuery, MoveSummary, Origin, Scope, SiteSummary,
};
use agentrix_store::time::Window;
use async_trait::async_trait;
use chrono::Utc;

use crate::facts::{
    FactSource, Facts, GauntletRun, LiveMove, Picture, SqliteFacts, StepTime, soft, time,
};

/// [`Facts`] from W8's [`Metrics`], with [`SqliteFacts`] for the details.
pub struct StoreFacts {
    metrics: Arc<dyn Metrics>,
    details: SqliteFacts,
    samples: Vec<String>,
}

impl StoreFacts {
    /// Facts from `metrics`, with `details` for what the catalogue doesn't carry.
    pub fn new(metrics: Arc<dyn Metrics>, details: SqliteFacts) -> Self {
        Self {
            metrics,
            details,
            samples: Vec::new(),
        }
    }

    /// Also reads these samples' hero pictures (host labels, `kilele-bakery-sample`).
    pub fn with_samples(mut self, slugs: Vec<String>) -> Self {
        self.samples = slugs;
        self
    }

    /// Each sample's approved hero picture from W11's library. A sample without one, or a
    /// library that can't be read, keeps its card's screenshot.
    async fn sample_pictures(&self) -> BTreeMap<String, Picture> {
        let mut out = BTreeMap::new();
        for slug in &self.samples {
            match agentrix_media::shown(self.details.pool(), slug).await {
                Ok(mut slots) => {
                    if let Some(hero) = slots.remove("hero") {
                        out.insert(
                            slug.clone(),
                            Picture {
                                url: hero.url,
                                alt: hero.alt,
                                label: hero.label,
                                width: hero.width,
                                height: hero.height,
                            },
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, slug, "home: a sample's pictures could not be read")
                }
            }
        }
        out
    }
}

fn is_fixture(sites: &[SiteSummary], site: &agentrix_types::SiteId) -> bool {
    sites
        .iter()
        .any(|s| &s.site_id == site && s.origin == Origin::Fixture)
}

/// The latest finished run on a fixture site.
fn latest_run(sites: &[SiteSummary], runs: Vec<GauntletScore>) -> Option<GauntletScore> {
    runs.into_iter()
        .filter(|g| g.ended_at.is_some() && is_fixture(sites, &g.site_id))
        .max_by(|a, b| a.ended_at.cmp(&b.ended_at))
}

/// When a Move's go-live step finished, if it did.
fn live_at(m: &MoveSummary) -> Option<&str> {
    m.steps
        .iter()
        .find(|s| s.step == "golive" && s.state == "done")
        .and_then(|s| s.ended_at.as_deref())
}

impl StoreFacts {
    async fn gauntlet(&self, sites: &[SiteSummary]) -> anyhow::Result<Option<GauntletRun>> {
        // W8 gives each site's latest run that has its receipt, so a run still going (each
        // deploy starts one) never shows as a score of zero.
        let Some(run) = latest_run(sites, self.metrics.gauntlet(None).await?) else {
            return Ok(None);
        };
        let attacks = match &run.receipt {
            Some(r) => soft("gauntlet attacks", self.details.attacks(r).await),
            None => Vec::new(),
        };
        Ok(Some(GauntletRun {
            score: u32::try_from(run.score).unwrap_or(0),
            out_of: u32::try_from(run.out_of).unwrap_or(0),
            receipt: run.receipt,
            finished_at: run
                .ended_at
                .as_deref()
                .and_then(time)
                .or_else(|| time(&run.started_at))
                .unwrap_or_default(),
            attacks,
        }))
    }

    async fn first_live_move(&self, sites: &[SiteSummary]) -> anyhow::Result<Option<LiveMove>> {
        let moves = self
            .metrics
            .moves(&MoveQuery {
                limit: Some(200),
                ..Default::default()
            })
            .await?;
        // The first to go live: a later rollback doesn't undo that it went live.
        let Some((m, live_at)) = moves
            .iter()
            .filter(|m| m.origin == Origin::Fixture)
            .filter_map(|m| live_at(m).map(|at| (m, at.to_owned())))
            .min_by(|a, b| a.1.cmp(&b.1))
        else {
            return Ok(None);
        };
        let site_name = sites
            .iter()
            .find(|s| s.site_id == m.site_id && !s.name.is_empty())
            .map_or_else(|| m.domain.clone(), |s| s.name.clone());
        let steps = m
            .steps
            .iter()
            .filter(|s| s.state == "done")
            .filter_map(|s| {
                Some(StepTime {
                    step: s.step.clone(),
                    seconds: s.millis? as f64 / 1000.0,
                    summary: s.summary.clone(),
                    receipt: s.receipt.clone(),
                })
            })
            .collect();
        let detail = self
            .details
            .move_detail(&m.move_id, m.site_id.as_str(), &live_at)
            .await?;
        Ok(Some(LiveMove {
            site_name,
            steps,
            live_at: time(&live_at).unwrap_or_default(),
            approval_to_live_seconds: detail.approval_to_live_seconds,
            approved_with: detail.approved_with,
            left_behind: detail.left_behind,
            cost_micro_usd: Some(m.cost.0),
            providers: detail.providers,
        }))
    }
}

#[async_trait]
impl FactSource for StoreFacts {
    async fn read(&self) -> anyhow::Result<Facts> {
        let sites = self.metrics.sites().await;
        if let Err(e) = &sites {
            tracing::warn!(error = %e, "home: W8's site list could not be read");
        }
        let known = sites.as_deref().unwrap_or_default();
        let receipts = self
            .metrics
            .receipt_counts(&Scope::Everything, Window::All)
            .await;
        if let Err(e) = &receipts {
            tracing::warn!(error = %e, "home: W8's receipt count could not be read");
        }
        Ok(Facts {
            gauntlet: soft("gauntlet", self.gauntlet(known).await),
            first_live_move: soft("first live move", self.first_live_move(known).await),
            sites_live: sites.as_ref().ok().map(|all| {
                all.iter()
                    .filter(|s| s.origin != Origin::Sample && s.live_seq.is_some())
                    .count() as u64
            }),
            receipts_issued: receipts.ok().and_then(|c| u64::try_from(c.total).ok()),
            reports_written: match self.metrics.reports_written(Window::All).await {
                Ok(n) => u64::try_from(n).ok(),
                Err(e) => {
                    tracing::warn!(error = %e, "home: W8's health report count could not be read");
                    None
                }
            },
            routing: soft("routing", self.details.routing().await),
            brief: soft("brief", self.details.brief().await),
            sample_pictures: self.sample_pictures().await,
            read_at: Some(Utc::now()),
        })
    }
}
