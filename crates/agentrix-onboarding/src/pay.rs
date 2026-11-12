//! The plan's PayPal subscription, approved just before go-live (O3.2).
//!
//! | Route | What it does |
//! |---|---|
//! | `GET /setup/pay?site=&next=` | Where the plan's payment stands, and the button to PayPal |
//! | `POST /setup/pay` (form `site, next`) | Starts the subscription through W6's Effects Gateway, then PayPal's page |
//! | `GET /setup/pay/return?site=&next=&subscription_id=` | PayPal sends the owner back here; the state is refreshed, then the page |
//!
//! Only the site's proven owner (`sites.owner_account_id`, set when the claim is verified) can
//! start a subscription. The price is W3's catalogue's, never the browser's. A subscription's
//! first charge is the day it's approved, so this step comes right before go-live.
//!
//! The owner's period comes from their plan choice (`/setup/plan`): monthly, or 12 months up
//! front, which PayPal charges once as the 12-month total ([`plan_price`]) and W3's billing
//! records with its period and amount (`Billing::start_for`).

use std::sync::Arc;

use agentrix_billing::Period;
use agentrix_effects::{EffectError, Effects, PlanPrice, SubscriptionStart};
use agentrix_host::{NodeContext, OwnerSessions};
use agentrix_types::{Money, SiteId};
use axum::Router;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

use crate::routes::safe_next;
use crate::store::plan_choice;
use crate::view::{PayPage, PayState, encode};
use crate::{OnboardingPage, Pages};

/// What the pay routes share.
#[derive(Clone)]
pub struct PayRoutes {
    pages: Arc<Pages>,
    owners: Arc<dyn OwnerSessions>,
    pool: sqlx::SqlitePool,
    effects: Effects,
    origin: Url,
}

impl PayRoutes {
    /// The routes' state for node: its sessions, `control.db` and W6's Effects Gateway.
    pub fn new(ctx: &NodeContext, effects: Effects) -> anyhow::Result<Self> {
        Ok(Self {
            pages: Arc::new(Pages::new()?),
            owners: ctx.owners.clone(),
            pool: ctx.pool.clone(),
            effects,
            origin: ctx.origin.clone(),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct Q {
    site: Option<String>,
    next: Option<String>,
    subscription_id: Option<String>,
    cancelled: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PayForm {
    site: String,
    #[serde(default)]
    next: Option<String>,
}

fn safe_site(site: &str) -> Option<String> {
    let s = site.trim();
    (!s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then(|| s.to_owned())
}

fn here(site: &str, next: &str) -> String {
    format!("/setup/pay?site={}&next={}", encode(site), encode(next))
}

fn render(st: &PayRoutes, view: &PayPage, status: StatusCode) -> Response {
    match st.pages.render(OnboardingPage::Pay, view) {
        Ok(body) => (status, Html(body)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "rendering the pay page");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "This page didn't load on our side. Nothing was charged. Try again in a moment.",
            )
                .into_response()
        }
    }
}

/// Whether `account` is the site's proven owner.
async fn owns(pool: &sqlx::SqlitePool, site: &str, account: Uuid) -> anyhow::Result<bool> {
    let owner: Option<(Option<String>,)> =
        sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?1")
            .bind(site)
            .fetch_optional(pool)
            .await?;
    Ok(matches!(owner, Some((Some(id),)) if id == account.to_string()))
}

/// The page's view: the owner's chosen plan and where its subscription stands.
async fn view_for(st: &PayRoutes, site: &str, next: &str) -> anyhow::Result<PayPage> {
    let mut view = PayPage {
        site_id: site.to_owned(),
        plan_href: format!(
            "/setup/plan?site={}&next={}",
            encode(site),
            encode(&here(site, next))
        ),
        next: next.to_owned(),
        ..PayPage::default()
    };
    let Some(choice) = plan_choice(&st.pool, site).await? else {
        return Ok(view);
    };
    let Some(plan) = agentrix_billing::plan(&choice.plan_id).filter(|p| p.kind == "site") else {
        return Ok(view);
    };
    view.plan_name = plan.name.clone();
    view.price_words = dollars(plan.price_cents);
    view.period = choice.period.as_str().to_owned();
    view.charge_words = charge_words(plan, choice.period);
    view.budget_words = format!("${}", choice.ai_budget_usd);
    if !st.effects.enabled() {
        view.state = PayState::Off;
        return Ok(view);
    }
    view.state = PayState::Ready;
    let mut subs = st
        .effects
        .subscriptions_for_site(&SiteId::new(site))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    subs.sort_by_key(|s| s.created_at);
    if let Some(latest) = subs.last() {
        match latest.state.as_str() {
            "active" => view.state = PayState::Active,
            "pending_approval" => {
                view.state = PayState::Pending;
                view.approve_url = latest.approve_url.clone();
            }
            _ => {}
        }
    }
    Ok(view)
}

fn dollars(cents: i64) -> String {
    crate::view::usd(cents)
}

/// "$149 a month" or "$1,430.40 for 12 months".
pub fn charge_words(plan: &agentrix_billing::Plan, period: Period) -> String {
    match period {
        Period::Monthly => format!("{} a month", dollars(plan.price_cents)),
        Period::Annual => format!("{} for 12 months", dollars(plan.annual_cents())),
    }
}

/// What W6's gateway asks PayPal for: the monthly price, and for 12 months up front the
/// 12-month total, charged once and renewed a year later. Always W3's catalogue's numbers.
pub fn plan_price(plan: &agentrix_billing::Plan, period: Period) -> PlanPrice {
    let usd = |minor| Money {
        currency: "USD".into(),
        minor,
    };
    PlanPrice {
        plan_id: plan.id.clone(),
        version: agentrix_billing::catalogue().version,
        name: format!("Agentrix {}", plan.name),
        monthly: usd(plan.price_cents),
        annual: (period == Period::Annual).then(|| usd(plan.annual_cents())),
    }
}

/// The signed-in owner of `site`, or the response to send instead.
async fn owner_of(
    st: &PayRoutes,
    headers: &HeaderMap,
    site: &str,
    next: &str,
) -> Result<Uuid, Response> {
    let owner = match st.owners.owner(headers).await {
        Ok(Some(o)) => o,
        Ok(None) => {
            return Err(Redirect::to(&format!(
                "/account/signin?next={}",
                encode(&here(site, next))
            ))
            .into_response());
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the owner session");
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't check who's signed in. Nothing was charged. Try again in a moment.",
            )
                .into_response());
        }
    };
    match owns(&st.pool, site, owner.account).await {
        Ok(true) => Ok(owner.account),
        Ok(false) => Err((
            StatusCode::FORBIDDEN,
            "Only the site's owner can set up its payment, once the domain is proved theirs. Nothing was charged.",
        )
            .into_response()),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "checking the site's owner");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't check who owns this site. Nothing was charged. Try again in a moment.",
            )
                .into_response())
        }
    }
}

async fn page(State(st): State<PayRoutes>, headers: HeaderMap, Query(q): Query<Q>) -> Response {
    let next = safe_next(q.next.as_deref());
    let Some(site) = q.site.as_deref().and_then(safe_site) else {
        return Redirect::to(&next).into_response();
    };
    if let Err(r) = owner_of(&st, &headers, &site, &next).await {
        return r;
    }
    match view_for(&st, &site, &next).await {
        Ok(mut view) => {
            if q.cancelled.is_some() && view.state != PayState::Active {
                view.said = Some("You left PayPal without agreeing. Nothing was charged.".into());
            }
            render(&st, &view, StatusCode::OK)
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the plan's payment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't read your plan's payment. Nothing was charged. Try again in a moment.",
            )
                .into_response()
        }
    }
}

async fn start(
    State(st): State<PayRoutes>,
    headers: HeaderMap,
    Form(f): Form<PayForm>,
) -> Response {
    let next = safe_next(f.next.as_deref());
    let Some(site) = safe_site(&f.site) else {
        return (
            StatusCode::BAD_REQUEST,
            "That isn't one of your sites. Nothing was charged.",
        )
            .into_response();
    };
    if let Err(r) = owner_of(&st, &headers, &site, &next).await {
        return r;
    }
    let mut view = match view_for(&st, &site, &next).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the plan's payment");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't read your plan. Nothing was charged. Try again in a moment.",
            )
                .into_response();
        }
    };
    if view.state != PayState::Ready {
        return Redirect::to(&here(&site, &next)).into_response();
    }
    let Some((plan, period)) = plan_choice(&st.pool, &site)
        .await
        .ok()
        .flatten()
        .and_then(|c| agentrix_billing::plan(&c.plan_id).map(|p| (p, c.period)))
    else {
        return Redirect::to(&here(&site, &next)).into_response();
    };
    let back = |path: &str| st.origin.join(path).map(|u| u.to_string());
    let (Ok(return_url), Ok(cancel_url)) = (
        back(&format!(
            "/setup/pay/return?site={}&next={}",
            encode(&site),
            encode(&next)
        )),
        back(&format!("{}&cancelled=1", here(&site, &next))),
    ) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "We couldn't build PayPal's return address. Nothing was charged.",
        )
            .into_response();
    };
    let started = st
        .effects
        .start_subscription(SubscriptionStart {
            site: SiteId::new(site.clone()),
            plan: plan_price(plan, period),
            subscriber_email: None,
            return_url,
            cancel_url,
            // The first charge on approval, which is right before go-live.
            starts_at: None,
        })
        .await;
    match started {
        Ok(s) => {
            // W3's billing records it as waiting for approval, with its period and what one
            // charge is; went_live sets its start.
            if let Err(e) = agentrix_billing::Billing::new(st.pool.clone())
                .start_for(&site, &plan.id, period, &s.provider_ref, None)
                .await
            {
                tracing::warn!(error = %e, "recording the subscription in billing");
            }
            Redirect::to(&s.approve_url).into_response()
        }
        Err(EffectError::NotConfigured) => {
            view.state = PayState::Off;
            render(&st, &view, StatusCode::OK)
        }
        Err(e) => {
            view.said = Some(e.to_string());
            render(&st, &view, StatusCode::BAD_GATEWAY)
        }
    }
}

async fn back_from_paypal(
    State(st): State<PayRoutes>,
    headers: HeaderMap,
    Query(q): Query<Q>,
) -> Response {
    let next = safe_next(q.next.as_deref());
    let Some(site) = q.site.as_deref().and_then(safe_site) else {
        return Redirect::to(&next).into_response();
    };
    if let Err(r) = owner_of(&st, &headers, &site, &next).await {
        return r;
    }
    if let Some(id) = q.subscription_id.as_deref().filter(|i| i.len() <= 64) {
        if let Err(e) = st.effects.refresh_subscription(id).await {
            tracing::warn!(error = %e, "refreshing the subscription after PayPal");
        }
    }
    Redirect::to(&here(&site, &next)).into_response()
}

/// The pay routes, for [`crate::mount`].
pub fn router(st: PayRoutes) -> Router {
    Router::new()
        .route("/setup/pay", get(page).post(start))
        .route("/setup/pay/return", get(back_from_paypal))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The amount PayPal is asked for is the catalogue's: monthly, or 12 months less the one
    /// discount, charged once.
    #[test]
    fn paypal_is_asked_for_the_period_the_owner_chose() {
        let commerce = agentrix_billing::plan("commerce").unwrap();
        let monthly = plan_price(commerce, Period::Monthly);
        assert_eq!(
            (monthly.monthly.minor, monthly.annual.as_ref()),
            (14_900, None)
        );
        let annual = plan_price(commerce, Period::Annual);
        assert_eq!(annual.monthly.minor, 14_900);
        assert_eq!(annual.annual.as_ref().map(|m| m.minor), Some(143_040));
        assert_eq!(annual.charge().minor, 143_040);
        assert_eq!(annual.annual.unwrap().currency, "USD");
        assert_eq!(charge_words(commerce, Period::Monthly), "$149 a month");
        assert_eq!(
            charge_words(commerce, Period::Annual),
            "$1,430.40 for 12 months"
        );
    }
}
