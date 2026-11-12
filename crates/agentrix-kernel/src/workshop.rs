//! The Workshop's save: the owner's new look for one WebAgent or desk (ADR-031; 07 §3.4; 01b C13).
//!
//! The Workshop previews everything in the browser and saves nothing until the owner presses
//! Save. Then [`Rosters::save_mark`] checks what the platform keeps:
//! - the mark stays this WebAgent's (same site key and id, so its seed key can't be borrowed);
//! - the fill keeps 3:1 on the site's background and lets the glyph keep 3:1 with the pattern
//!   drawn (the refusal offers the nearest shade that passes);
//! - the shape stays a squircle in range, the glyph a role icon or one or two letters.
//!
//! The signature family belongs to the whole site, so the team reads as one: changing it on one
//! mark changes it on every WebAgent's and desk's mark. Each change is saved as a new version of
//! the spec it touches, with a receipt; the mark publishes with the site's next release.

use agentrix_host::Receipts;
use agentrix_marks::{MarkSpec, color};
use agentrix_types::{DecisionId, ReceiptBody, ReceiptKind, SiteId, Subject};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::policy::member_of;
use crate::roster::{Roster, mark_for, role_glyph};
use crate::store::Rosters;

/// Why a mark wasn't saved, in the owner's words.
#[derive(Debug, thiserror::Error)]
pub enum MarkSaveError {
    /// No such WebAgent or desk, or no team.
    #[error("{0}")]
    NotFound(String),
    /// The platform keeps this.
    #[error("{message}")]
    NotAllowed {
        /// What to say.
        message: String,
        /// The nearest fill that passes, when contrast was the problem.
        suggestion: Option<String>,
    },
    /// Something underneath failed.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// The mark this WebAgent or desk starts with on this site: what Reset goes back to.
pub fn default_mark(roster: &Roster, id: &str) -> Option<MarkSpec> {
    if id == "coordinator" {
        return Some(mark_for(&roster.look, "coordinator", "monogram"));
    }
    if roster.get(id).is_some() {
        return Some(mark_for(&roster.look, id, role_glyph(id)));
    }
    let desk = roster.desks().iter().find(|d| d.id == id)?;
    let glyph = match &desk.mark.glyph {
        agentrix_marks::Glyph::Role { id } => id.clone(),
        agentrix_marks::Glyph::Monogram { .. } => "monogram".to_string(),
    };
    Some(mark_for(&roster.look, &desk.mark.webagent, &glyph))
}

impl Rosters {
    /// Saves the owner's new mark for `id` (a WebAgent or a desk) and returns the receipt.
    pub async fn save_mark(
        &self,
        site: &str,
        id: &str,
        mark: MarkSpec,
        account: Uuid,
        receipts: &dyn Receipts,
    ) -> Result<String, MarkSaveError> {
        let roster = self
            .roster(site)
            .await?
            .ok_or_else(|| MarkSaveError::NotFound("There's no team for this site yet.".into()))?;
        let mark = mark.normalized();
        mark.validate().map_err(|e| MarkSaveError::NotAllowed {
            message: e.to_string(),
            suggestion: None,
        })?;
        let current = if let Some(m) = roster.get(id) {
            m.spec.mark.clone()
        } else if let Some(d) = roster.desks().iter().find(|d| d.id == id) {
            d.mark.clone()
        } else {
            return Err(MarkSaveError::NotFound(
                "There's no WebAgent or desk with that id.".into(),
            ));
        };
        if mark.site != current.site || mark.webagent != current.webagent {
            return Err(MarkSaveError::NotAllowed {
                message: "That mark belongs to another WebAgent. Start from this one's.".into(),
                suggestion: None,
            });
        }
        if !color::passes(&mark.color.fill, &roster.look.bg) {
            return Err(MarkSaveError::NotAllowed {
                message:
                    "That colour is too faint on your site. Use the nearest shade that passes."
                        .into(),
                suggestion: Some(color::nearest_passing(&mark.color.fill, &roster.look.bg)),
            });
        }
        let family_changed = mark.signature.family != current.signature.family;
        let change_id = format!("mark_{}", Uuid::now_v7().simple());
        let reason = "New look from the Workshop";
        // The member that holds this mark (desks live in the Concierge's spec).
        let holder = member_of(id);
        for m in &roster.members {
            let mut spec = m.spec.clone();
            let mut changed = false;
            if m.spec.id == holder {
                if id == holder {
                    spec.mark = mark.clone();
                } else if let Some(d) = spec.desks.iter_mut().find(|d| d.id == id) {
                    d.mark = mark.clone();
                }
                changed = true;
            }
            if family_changed {
                if spec.mark.signature.family != mark.signature.family {
                    spec.mark.signature.family = mark.signature.family;
                    changed = true;
                }
                for d in &mut spec.desks {
                    if d.mark.signature.family != mark.signature.family {
                        d.mark.signature.family = mark.signature.family;
                        changed = true;
                    }
                }
            }
            if changed {
                let why = if m.spec.id == holder {
                    reason.to_string()
                } else {
                    format!(
                        "Signature family changed to {} for the whole site",
                        mark.signature.family.label()
                    )
                };
                self.save_spec(site, &spec, &why, Some(&change_id)).await?;
            }
        }
        let now = Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionApproved,
            SiteId::new(site),
            Subject::Decision(DecisionId::new(change_id)),
            now,
            now,
        );
        body.detail = Some(json!({
            "card": "webagent.mark",
            "webagent": id,
            "mark": mark,
            "family_for_site": family_changed,
            "method": "signed-in",
            "account": account.to_string(),
        }));
        Ok(receipts.issue(body).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_marks::Family;

    async fn setup() -> (Rosters, agentrix_host::NodeContext) {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        rosters.seed_sample("kilele", "kilele").await.unwrap();
        (rosters, ctx)
    }

    #[tokio::test]
    async fn a_new_seed_and_colour_save_for_one_webagent() {
        let (rosters, ctx) = setup().await;
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        let mut mark = r.get("editor").unwrap().spec.mark.clone();
        mark.signature.seed = 4242;
        mark.color.fill = "#3b2416".into();
        let receipt = rosters
            .save_mark(
                "kilele",
                "editor",
                mark,
                Uuid::new_v4(),
                ctx.receipts.as_ref(),
            )
            .await
            .unwrap();
        assert!(receipt.starts_with("sha256:"));
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert_eq!(r.get("editor").unwrap().spec.mark.signature.seed, 4242);
        assert_eq!(
            r.get("guardian").unwrap().spec.mark.signature.seed,
            2,
            "others untouched"
        );
    }

    #[tokio::test]
    async fn a_desk_mark_saves_into_the_concierge() {
        let (rosters, ctx) = setup().await;
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        let mut mark = r
            .desks()
            .iter()
            .find(|d| d.id == "desk-cakes")
            .unwrap()
            .mark
            .clone();
        mark.glyph = MarkSpec::monogram("C");
        rosters
            .save_mark(
                "kilele",
                "desk-cakes",
                mark,
                Uuid::new_v4(),
                ctx.receipts.as_ref(),
            )
            .await
            .unwrap();
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        let cakes = r.desks().iter().find(|d| d.id == "desk-cakes").unwrap();
        assert_eq!(cakes.mark.glyph, MarkSpec::monogram("C"));
    }

    #[tokio::test]
    async fn the_family_is_the_whole_sites() {
        let (rosters, ctx) = setup().await;
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        let mut mark = r.get("editor").unwrap().spec.mark.clone();
        mark.signature.family = Family::Weave;
        rosters
            .save_mark(
                "kilele",
                "editor",
                mark,
                Uuid::new_v4(),
                ctx.receipts.as_ref(),
            )
            .await
            .unwrap();
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert!(
            r.members
                .iter()
                .all(|m| m.spec.mark.signature.family == Family::Weave)
        );
        assert!(
            r.desks()
                .iter()
                .all(|d| d.mark.signature.family == Family::Weave)
        );
    }

    #[tokio::test]
    async fn the_platform_keeps_contrast_and_identity() {
        let (rosters, ctx) = setup().await;
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        let mut faint = r.get("editor").unwrap().spec.mark.clone();
        faint.color.fill = "#f3ead9".into();
        match rosters
            .save_mark(
                "kilele",
                "editor",
                faint,
                Uuid::new_v4(),
                ctx.receipts.as_ref(),
            )
            .await
        {
            Err(MarkSaveError::NotAllowed {
                suggestion: Some(s),
                ..
            }) => assert!(color::passes(&s, &r.look.bg)),
            other => panic!("{other:?}"),
        }
        let borrowed = r.get("guardian").unwrap().spec.mark.clone();
        assert!(matches!(
            rosters
                .save_mark(
                    "kilele",
                    "editor",
                    borrowed,
                    Uuid::new_v4(),
                    ctx.receipts.as_ref()
                )
                .await,
            Err(MarkSaveError::NotAllowed { .. })
        ));
        assert_eq!(default_mark(&r, "editor").unwrap().signature.seed, 5);
        assert_eq!(
            default_mark(&r, "desk-cakes").unwrap().seed_key(),
            "kilele:cakes:7"
        );
    }
}
