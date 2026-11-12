//! The Store (01b C14, C15; concept/console/store.html): the WebAgents a site doesn't have yet,
//! from W1's roster, and adding one. Adding is a decision: the console raises a card v2
//! (`webagent.add`) the owner approves with their passkey or a code, and [`AddWebAgent`] puts
//! the WebAgent on duty once that approval's receipt is written. Packs and connections show
//! when their catalogues exist; nothing here invents a listing.
//!
//! Removing is the same decision the other way (`webagent.remove`, [`RemoveWebAgent`]): the
//! WebAgent goes back to the Store, W1's policy refuses anything it tries after that, and its
//! receipts and the owner's rules for it stay. The Chief WebAgent, the Concierge and the Guardian
//! stay on every team ([`stays`]).

use std::sync::Arc;

use agentrix_decisions::{DecisionCardV2, EffectRunner};
use agentrix_host::NodeContext;
use agentrix_kernel::{DeskDraft, Rosters, SiteLook};
use anyhow::Context;
use async_trait::async_trait;

/// The effect tool a Store card carries.
pub const TOOL: &str = "webagent.add";

/// The effect tool a card that removes a WebAgent carries.
pub const REMOVE_TOOL: &str = "webagent.remove";

/// Why a WebAgent can't be removed, in the owner's words, or `None` when it can: the Chief
/// WebAgent leads the team, the Concierge runs the desks visitors talk to (the sites' Ask doesn't
/// check the team), and the Guardian protects the site.
pub fn stays(id: &str) -> Option<&'static str> {
    match id {
        "coordinator" => Some("The Chief WebAgent leads your team, so it stays on it."),
        "concierge" => {
            Some("The Concierge runs the desks your visitors talk to, so it stays on your team.")
        }
        "guardian" => Some(
            "The Guardian protects your site and can put back your last release, so it stays on your team.",
        ),
        _ => None,
    }
}

/// What stops when the owner removes a WebAgent, and what stays: the card's effects and the
/// confirmation on the WebAgent's page say the same.
pub fn remove_effects(spec: &agentrix_kernel::WebAgentSpec, site_name: &str) -> Vec<String> {
    let mut effects = vec![format!(
        "{} stops working for {site_name}. Anything it tries after this is refused",
        spec.name
    )];
    effects.extend(spec.jobs.iter().map(|j| format!("\"{}\" stops", j.name)));
    effects.push(format!(
        "Its receipts stay on record, and so do your rules for {}",
        spec.name
    ));
    effects.push("You can add it back from the Store".into());
    effects
}

/// The site's recorded team (its look, who's on duty, the Concierge's desks), or for a moved
/// site with nothing recorded yet, the roster it already shows.
async fn team(
    rosters: &Rosters,
    site: &str,
    roster: &agentrix_kernel::Roster,
) -> anyhow::Result<(SiteLook, Vec<String>, Vec<DeskDraft>)> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT look, on_duty, desks FROM webagent_sites WHERE site_id = ?")
            .bind(site)
            .fetch_optional(rosters.pool())
            .await
            .unwrap_or(None);
    Ok(match row {
        Some((look, on, desks)) => (
            serde_json::from_str(&look)?,
            serde_json::from_str(&on)?,
            serde_json::from_str(&desks)?,
        ),
        None => (
            roster.look.clone(),
            roster.on_duty().map(|m| m.spec.id.clone()).collect(),
            Vec::new(),
        ),
    })
}

/// Puts a WebAgent on duty for a site after the owner approved its card.
pub struct AddWebAgent {
    rosters: Rosters,
}

impl AddWebAgent {
    /// Over node's roster tables.
    pub fn new(ctx: &NodeContext) -> Self {
        Self {
            rosters: Rosters::new(ctx.pool.clone(), ctx.blobs.clone()),
        }
    }
}

#[async_trait]
impl EffectRunner for AddWebAgent {
    fn tool(&self) -> &str {
        TOOL
    }

    async fn run(&self, card: &DecisionCardV2, _receipt: &str) -> anyhow::Result<Option<String>> {
        let site = card.site.as_str();
        let id = card.effect.args["webagent"]
            .as_str()
            .context("the card names no WebAgent")?;
        let roster = self
            .rosters
            .roster(site)
            .await?
            .context("this site has no WebAgents to add to")?;
        anyhow::ensure!(
            roster.get(id).is_some(),
            "{id} isn't a WebAgent this platform offers"
        );
        // Keep the site's recorded team and desks; a moved site with nothing recorded yet starts
        // from the roster it already shows.
        let (look, mut on_duty, desks) = team(&self.rosters, site, &roster).await?;
        if !on_duty.iter().any(|w| w == id) {
            on_duty.push(id.to_owned());
        }
        let on: Vec<&str> = on_duty.iter().map(String::as_str).collect();
        self.rosters.set_team(site, &look, &on, &desks).await?;
        Ok(Some(format!("/console/{site}/webagents#agent-{id}")))
    }
}

/// Takes a WebAgent off a site's team after the owner approved its card. It goes back to the
/// Store; its receipts, rules and grants stay as records.
pub struct RemoveWebAgent {
    rosters: Rosters,
}

impl RemoveWebAgent {
    /// Over node's roster tables.
    pub fn new(ctx: &NodeContext) -> Self {
        Self {
            rosters: Rosters::new(ctx.pool.clone(), ctx.blobs.clone()),
        }
    }
}

#[async_trait]
impl EffectRunner for RemoveWebAgent {
    fn tool(&self) -> &str {
        REMOVE_TOOL
    }

    async fn run(&self, card: &DecisionCardV2, _receipt: &str) -> anyhow::Result<Option<String>> {
        let site = card.site.as_str();
        let id = card.effect.args["webagent"]
            .as_str()
            .context("the card names no WebAgent")?;
        if let Some(why) = stays(id) {
            anyhow::bail!("{why}");
        }
        let roster = self
            .rosters
            .roster(site)
            .await?
            .context("this site has no team to remove a WebAgent from")?;
        anyhow::ensure!(
            roster.get(id).is_some(),
            "{id} isn't a WebAgent this platform offers"
        );
        let (look, mut on_duty, desks) = team(&self.rosters, site, &roster).await?;
        on_duty.retain(|w| w != id);
        let on: Vec<&str> = on_duty.iter().map(String::as_str).collect();
        self.rosters.set_team(site, &look, &on, &desks).await?;
        Ok(Some(format!("/console/{site}/webagents/{id}")))
    }
}

/// The effect runners the console registers with `agentrix_decisions::mount_with`.
pub fn runners(ctx: &NodeContext) -> Vec<Arc<dyn EffectRunner>> {
    vec![
        Arc::new(AddWebAgent::new(ctx)),
        Arc::new(RemoveWebAgent::new(ctx)),
        Arc::new(crate::missions::StartMission::new(ctx)),
    ]
}
