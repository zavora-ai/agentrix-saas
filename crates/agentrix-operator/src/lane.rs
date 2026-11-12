//! Support-access requests and partners' role requests in the owner console's lane (W5's `LaneSource`, contract C9): each
//! request or live grant on the owner's site shows under "Needs you" and links to
//! `/support-access/{grant}`, where the owner reads the exact terms and decides. The shell never
//! approves it.

use std::sync::Arc;

use agentrix_shell::apps::{AppContext, LaneItem, LaneSource};
use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, Utc};
use sqlx::SqlitePool;

use crate::access;

/// The lane source node passes to `agentrix_shell::Plugins { lanes }`.
pub fn support_lane(ctx: &agentrix_host::NodeContext) -> Arc<dyn LaneSource> {
    Arc::new(SupportLane {
        pool: ctx.pool.clone(),
    })
}

struct SupportLane {
    pool: SqlitePool,
}

fn words_at(text: &str, offset_minutes: i32, format: &str) -> String {
    let zone = FixedOffset::east_opt(offset_minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"));
    DateTime::parse_from_rfc3339(text)
        .map(|t| {
            t.with_timezone(&Utc)
                .with_timezone(&zone)
                .format(format)
                .to_string()
        })
        .unwrap_or_default()
}

#[async_trait]
impl LaneSource for SupportLane {
    async fn needs_you(&self, ctx: &AppContext) -> anyhow::Result<Vec<LaneItem>> {
        access::expire(&self.pool).await?;
        let owner: Option<(Option<String>,)> =
            sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?")
                .bind(&ctx.site_id)
                .fetch_optional(&self.pool)
                .await?;
        if owner.and_then(|(o,)| o).as_deref() != Some(ctx.account.as_str()) {
            return Ok(Vec::new());
        }
        let mut items = Vec::new();
        for g in access::grants_for_site(&self.pool, &ctx.site_id).await? {
            let verb = if g.access == "read" {
                "read"
            } else {
                "read and change"
            };
            let hours = if g.hours == 1 {
                "1 hour".to_owned()
            } else {
                format!("{} hours", g.hours)
            };
            let (title, when_words) = match g.state.as_str() {
                "requested" => (
                    format!("Zavora support asks to {verb} your site for {hours}"),
                    format!(
                        "Asked {}",
                        words_at(&g.created_at, ctx.utc_offset_minutes, "%a %-d %b, %H:%M")
                    ),
                ),
                "active" => (
                    format!("Zavora support can {verb} your site now"),
                    format!(
                        "Ends {}",
                        words_at(
                            g.ends_at.as_deref().unwrap_or(""),
                            ctx.utc_offset_minutes,
                            "%H:%M"
                        )
                    ),
                ),
                _ => continue,
            };
            items.push(LaneItem {
                id: g.grant_id.clone(),
                title,
                summary: g.reason.clone(),
                from: "Zavora support".to_owned(),
                tone: if g.access == "read" { "wait" } else { "risk" }.to_owned(),
                chip: if g.access == "read" {
                    "Read only"
                } else {
                    "Read and change"
                }
                .to_owned(),
                when_words,
                href: format!("/support-access/{}", g.grant_id),
            });
        }
        // A partner asking for a charter role on this site (08 §2.5).
        let asks: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT c.partner_id, p.name, c.charter_role, c.invited_at FROM partner_clients c JOIN partners p ON p.partner_id = c.partner_id \
             WHERE c.site_id = ? AND c.state = 'invited' ORDER BY c.invited_at",
        )
        .bind(&ctx.site_id)
        .fetch_all(&self.pool)
        .await?;
        for (partner, name, role, at) in asks {
            items.push(LaneItem {
                id: format!("agency-{partner}"),
                title: format!("{name} asks to be your site's {role}"),
                summary: format!("{name} works on sites on Agentrix. You decide; you can take the role back at any time."),
                from: name,
                tone: if role == "viewer" { "wait" } else { "risk" }.to_owned(),
                chip: format!("{}{}", role[..1].to_uppercase(), &role[1..]),
                when_words: format!("Asked {}", words_at(&at, ctx.utc_offset_minutes, "%a %-d %b, %H:%M")),
                href: format!("/agency-access/{partner}/{}", ctx.site_id),
            });
        }
        Ok(items)
    }
}
