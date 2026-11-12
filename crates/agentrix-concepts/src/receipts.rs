//! Signed receipts for the concept stage, through node's receipt store: `concept.page` for each
//! finished page (its sha256 as the output) and `concept.chosen` for the owner's choice.

use std::sync::Arc;

use agentrix_host::Receipts;
use agentrix_types::{ReceiptBody, ReceiptKind, SiteId, Subject};

/// Signs a receipt of `kind` about `site_id`, with `output` (a `sha256:` digest) and `detail`.
/// `None` when this build has no such kind, or signing failed (logged): the page still works.
pub async fn issue(
    receipts: &Arc<dyn Receipts>,
    kind: &str,
    site_id: &str,
    output: Option<&str>,
    detail: serde_json::Value,
) -> Option<String> {
    let Some(kind) = ReceiptKind::parse(kind) else {
        tracing::debug!(kind, "concepts: receipts of this kind aren't in this build");
        return None;
    };
    let now = chrono::Utc::now();
    let mut body = ReceiptBody::new(
        kind,
        SiteId::new(site_id),
        Subject::Site(SiteId::new(site_id)),
        now,
        now,
    );
    if let Some(digest) = output.and_then(|d| d.parse().ok()) {
        body.outputs.push(digest);
    }
    body.detail = Some(detail);
    match receipts.issue(body).await {
        Ok(digest) => Some(digest),
        Err(e) => {
            tracing::warn!(error = %e, "concepts: a receipt wasn't signed");
            None
        }
    }
}
