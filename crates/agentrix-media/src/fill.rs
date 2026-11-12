//! Pictures the Move fills in where the old site had none (W11 with W4).
//!
//! The Move's rebuild carries only the images a page already had (parity first), so a moved
//! site keeps its bare pages. After go-live, the Editor drafts one picture for each page that
//! has none, from the page's own title and summary; each waits in Pictures for the owner and
//! reaches visitors only in a release the owner signs. The page's words are the old site's
//! content: they go to the model as a description only, the policy check runs on them, and the
//! safety check runs on what comes back.

use crate::library::{Image, Request};
use crate::pipeline::{Ask, MakeError};
use crate::policy::Kind;
use crate::routes::{MediaState, draw_and_keep, is_sample};

/// A page of the moved site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Its path, `/cakes`.
    pub path: String,
    /// Its title.
    pub title: String,
    /// One or two sentences of what it says.
    pub summary: String,
    /// Whether it already shows a picture.
    pub has_picture: bool,
}

/// What a fill did.
#[derive(Debug, Default)]
pub struct Filled {
    /// Drafts made, waiting for the owner.
    pub drafts: Vec<Image>,
    /// Pages skipped: a picture already, a draft already waiting, or a brief the policy refused.
    pub skipped: Vec<(String, String)>,
}

/// The slot a page's picture fills.
pub fn slot_for(path: &str) -> String {
    let clean: String = path
        .trim_matches('/')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("page:{}", if clean.is_empty() { "home" } else { &clean })
}

/// Drafts a picture for each page without one, at most `limit` per call (each costs about
/// $0.02 and ten seconds).
pub async fn fill(
    state: &MediaState,
    site: &str,
    look: &str,
    pages: &[Page],
    limit: usize,
) -> Result<Filled, MakeError> {
    let mut out = Filled::default();
    for page in pages {
        if out.drafts.len() >= limit {
            out.skipped
                .push((page.path.clone(), "over this run's limit".into()));
            continue;
        }
        if page.has_picture {
            out.skipped
                .push((page.path.clone(), "already has a picture".into()));
            continue;
        }
        let slot = slot_for(&page.path);
        let (waiting,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM media_images WHERE site_id = ? AND slot = ? AND status IN ('draft', 'approved')",
        )
        .bind(site)
        .bind(&slot)
        .fetch_one(&state.ctx().pool)
        .await
        .map_err(|e| MakeError::Gateway(crate::client::GatewayError { code: "store".into(), message: e.to_string() }))?;
        if waiting > 0 {
            out.skipped.push((
                page.path.clone(),
                "a picture is already there or waiting".into(),
            ));
            continue;
        }
        let title = page.title.trim();
        let summary: String = page.summary.trim().chars().take(300).collect();
        // The summary helps the picture fit; if it trips the policy ("our team", "customers"),
        // the title alone is the brief.
        let with_summary = format!("A picture for a page titled \"{title}\": {summary}");
        let brief = if crate::policy::check(Kind::Fill, &with_summary).is_ok() {
            with_summary
        } else {
            format!("A picture for a page titled \"{title}\"")
        };
        let request = Request {
            site,
            slot: &slot,
            kind: Kind::Fill,
            brief: &brief,
            asked_by: "move",
            replaces: None,
        };
        let ask = Ask {
            kind: Kind::Fill,
            brief: brief.clone(),
            look: look.to_owned(),
            context: format!(
                "A picture for the page {} of a small business website",
                page.path
            ),
            n: 1,
            seed: None,
            tier: "fast",
            sample: is_sample(site),
        };
        match draw_and_keep(state, &request, &ask).await {
            Ok((made, _blocked)) => out.drafts.extend(made),
            Err(MakeError::Refused(r)) => out.skipped.push((page.path.clone(), r.0)),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::slot_for;

    #[test]
    fn a_page_path_names_its_slot() {
        assert_eq!(slot_for("/"), "page:home");
        assert_eq!(slot_for("/cakes"), "page:cakes");
        assert_eq!(slot_for("/visit-us/"), "page:visit-us");
        assert_eq!(slot_for("/About Us"), "page:about-us");
    }
}
