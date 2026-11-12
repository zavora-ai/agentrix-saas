//! The custom orders held for a sample site's owner, for another console to show and decide too
//! (saas-node's decision cards in Agentrix OS): the same decision as the owner's Custom orders
//! page, with the customer told the same way.

use std::sync::RwLock;

use agentrix_records::orders::{OrderError, OrderView};
use agentrix_records::{Site, time};

use crate::App;

static APP: RwLock<Option<App>> = RwLock::new(None);

/// The samples mounted last on this node.
pub(crate) fn register(app: &App) {
    *APP.write().unwrap_or_else(|p| p.into_inner()) = Some(app.clone());
}

pub(crate) fn app() -> anyhow::Result<App> {
    APP.read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .ok_or_else(|| anyhow::anyhow!("the sample sites aren't mounted on this node"))
}

/// `slug`'s custom orders waiting for the owner, newest first (at most `limit`); none when the
/// site isn't there.
///
/// # Errors
///
/// When the samples aren't mounted or the records can't be read.
pub async fn held(slug: &str, limit: i64) -> anyhow::Result<Vec<OrderView>> {
    let app = app()?;
    let records = app.records().await?;
    let Some(site) = records.site(slug).await? else {
        return Ok(Vec::new());
    };
    Ok(records
        .orders_for_owner(&site, Some("custom"), limit)
        .await?
        .into_iter()
        .filter(|v| v.order.state == "held")
        .collect())
}

/// Keeps PayPal's verified confirmation of one of `slug`'s sales (a capture or a refund taken
/// through Agentrix's checkout) on the order it confirms, once per PayPal event id. Returns
/// whether it was new; `false` too when the site isn't there.
///
/// # Errors
///
/// When the samples aren't mounted or the records can't be written.
pub async fn confirm_paypal(
    slug: &str,
    confirmation: &agentrix_records::confirmations::PayPalConfirmation,
) -> anyhow::Result<bool> {
    let app = app()?;
    let records = app.records().await?;
    if records.site(slug).await?.is_none() {
        return Ok(false);
    }
    records.confirm_paypal(slug, confirmation).await
}

/// One of `slug`'s orders as it stands now.
///
/// # Errors
///
/// When the samples aren't mounted or the records can't be read.
pub async fn order(slug: &str, order_id: &str) -> anyhow::Result<Option<OrderView>> {
    let app = app()?;
    let records = app.records().await?;
    let Some(site) = records.site(slug).await? else {
        return Ok(None);
    };
    records.order(&site, order_id).await
}

/// The owner approves or declines `slug`'s held order `order_id`, with an optional note for
/// the customer, as the owner's Custom orders page does: the customer is told.
///
/// # Errors
///
/// [`OrderError::Refused`] when the order isn't held (any more) or the site isn't there.
pub async fn decide(
    slug: &str,
    order_id: &str,
    approve: bool,
    note: Option<&str>,
) -> Result<OrderView, OrderError> {
    let app = app()?;
    let records = app.records().await?;
    let Some(site) = records.site(slug).await? else {
        return Err(OrderError::Refused("There's no such site.".into()));
    };
    let owner = crate::owner::owner_first(&site);
    decide_and_tell(&app, &site, order_id, approve, &owner, note).await
}

/// Decides a held order as `owner` and tells the customer; what both owner consoles call.
pub(crate) async fn decide_and_tell(
    app: &App,
    site: &Site,
    order_id: &str,
    approve: bool,
    owner: &str,
    note: Option<&str>,
) -> Result<OrderView, OrderError> {
    let records = app.records().await?;
    let v = records
        .decide(site, order_id, approve, owner, note, time::now())
        .await?;
    let (host, origin) = app.site_origin(site);
    let event = if approve {
        agentrix_messages::orders::OrderEvent::RequestApproved {
            pay_by: crate::notify::two_days_on(site, time::now()),
        }
    } else {
        agentrix_messages::orders::OrderEvent::RequestDeclined {
            reason: note.map(str::to_owned),
        }
    };
    app.tell_customer(site, &host, &origin, &v, event).await;
    Ok(v)
}
