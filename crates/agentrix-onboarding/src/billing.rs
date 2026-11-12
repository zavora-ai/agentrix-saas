//! Billing and ownership (O9.1 to O9.4), for the site's proven owner.
//!
//! | Route | What it does |
//! |---|---|
//! | `GET /billing?site=` | The plan, its state, AI spend against the budget, plans, cancel, export |
//! | `POST /billing/cancel` (JSON `{site, passkey}` or `{site, totp}`) | Cancels the PayPal subscription once the owner's approval verifies |
//! | `POST /billing/period` (form `site, period`) | Pays monthly or 12 months up front from the next renewal: saved, then PayPal's page to agree to the change on a live plan |
//! | `GET /billing/period/return?site=&subscription_id=` | PayPal sends the owner back; the plan is read back from PayPal |
//!
//! Cancelling is an owner decision (W6: "an owner decision … raises and approves first"): the
//! page shows exactly what is signed, the approval goes through node's frozen approve contract
//! (`NodeContext::approvals`, a passkey assertion or an authenticator code bound to the
//! fingerprint), and only then does W6's Effects Gateway cancel at PayPal.

use std::sync::Arc;

use agentrix_billing::Period;
use agentrix_effects::{EffectError, Effects};
use agentrix_host::{Approvals, NodeContext, OwnerSessions};
use agentrix_types::SiteId;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::store::{PlanChoice, plan_choice, save_plan_choice, set_plan_period};
use crate::view::{BillingPage, CancelCard, Chip, PeriodSwitch, PlanPage, encode, usd};
use crate::{OnboardingPage, Pages};

/// What the billing routes share.
#[derive(Clone)]
pub struct BillingRoutes {
    pages: Arc<Pages>,
    owners: Arc<dyn OwnerSessions>,
    approvals: Arc<dyn Approvals>,
    pool: sqlx::SqlitePool,
    effects: Effects,
    origin: url::Url,
}

impl BillingRoutes {
    /// The routes' state for node.
    pub fn new(ctx: &NodeContext, effects: Effects) -> anyhow::Result<Self> {
        Ok(Self {
            pages: Arc::new(Pages::new()?),
            owners: ctx.owners.clone(),
            approvals: ctx.approvals.clone(),
            pool: ctx.pool.clone(),
            effects,
            origin: ctx.origin.clone(),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct Q {
    site: Option<String>,
    /// Set by PayPal on the way back from agreeing to a switch.
    subscription_id: Option<String>,
    /// What just happened: `period` after a switch was saved.
    done: Option<String>,
}

fn safe_site(site: &str) -> Option<String> {
    let s = site.trim();
    (!s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then(|| s.to_owned())
}

/// What the owner's approval of a cancel signs: the site and the subscription, nothing else.
fn cancel_fingerprint(site: &str, subscription_id: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(format!(
        "agentrix.billing.plan_cancel\nsite={site}\nsubscription={subscription_id}\n"
    ));
    h.finalize().into()
}

async fn owns(pool: &sqlx::SqlitePool, site: &str, account: Uuid) -> anyhow::Result<bool> {
    let owner: Option<(Option<String>,)> =
        sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?1")
            .bind(site)
            .fetch_optional(pool)
            .await?;
    Ok(matches!(owner, Some((Some(id),)) if id == account.to_string()))
}

async fn owner(st: &BillingRoutes, headers: &HeaderMap, site: &str) -> Result<Uuid, Response> {
    match st.owners.owner(headers).await {
        Ok(Some(o)) => match owns(&st.pool, site, o.account).await {
            Ok(true) => Ok(o.account),
            Ok(false) => Err((
                StatusCode::FORBIDDEN,
                "Only the site's owner sees its billing, once the domain is proved theirs.",
            )
                .into_response()),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "checking the site's owner");
                Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "We couldn't check who owns this site. Nothing was changed.",
                )
                    .into_response())
            }
        },
        Ok(None) => Err(Redirect::to(&format!(
            "/account/signin?next={}",
            encode(&format!("/billing?site={site}"))
        ))
        .into_response()),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the owner session");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't check who's signed in. Nothing was changed.",
            )
                .into_response())
        }
    }
}

fn dollars_micro(micro: i64) -> String {
    let cents = (micro + 5_000) / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

fn day_words(at: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc).format("%a %-d %b").to_string())
}

/// "Mon 7 Dec 2026".
fn date_words(at: DateTime<Utc>) -> String {
    at.format("%a %-d %b %Y").to_string()
}

/// "$149 a month" or "$1,430.40 for 12 months".
fn charge_words(period: Period, cents: i64) -> String {
    match period {
        Period::Monthly => format!("{} a month", usd(cents)),
        Period::Annual => format!("{} for 12 months", usd(cents)),
    }
}

/// "Monthly" or "12 months, 20% off".
fn period_words(period: Period) -> String {
    match period {
        Period::Monthly => "Monthly".into(),
        Period::Annual => format!(
            "12 months, {}% off",
            agentrix_billing::catalogue().annual_discount_percent
        ),
    }
}

/// The other way of paying for `plan`, and, when the owner already asked for it, when it
/// starts. `now` is the period the plan is paid by; `asked` the owner's latest choice.
fn period_switch(
    plan: &agentrix_billing::Plan,
    now: Period,
    asked: Period,
    renewal: Option<DateTime<Utc>>,
    live: bool,
) -> PeriodSwitch {
    let when = match (live, renewal) {
        (true, Some(at)) => format!("your next renewal, {}", date_words(at)),
        (true, None) => "your next renewal".to_owned(),
        (false, _) => "your first charge, the day you approve it on PayPal".to_owned(),
    };
    if asked != now {
        let other = if now == Period::Monthly {
            "monthly"
        } else {
            "for 12 months"
        };
        return PeriodSwitch {
            to: now.as_str().into(),
            words: format!(
                "From {when}, you pay {}.",
                charge_words(asked, plan.charge_cents(asked))
            ),
            button: format!("Keep paying {other}"),
            agree_to: live.then(|| asked.as_str().to_owned()),
            pending: Some(if live {
                "It starts once you agree to the change on PayPal's page. Until then nothing changes."
                    .into()
            } else {
                "Saved. PayPal charges it when you approve your plan.".into()
            }),
        };
    }
    match now {
        Period::Monthly => PeriodSwitch {
            to: Period::Annual.as_str().into(),
            words: format!(
                "Pay for 12 months at a time: {} once a year instead of {} paid monthly, so you save {}. It starts at {when}.",
                usd(plan.annual_cents()),
                usd(plan.price_cents * 12),
                usd(plan.annual_saving_cents())
            ),
            button: "Switch to 12 months".into(),
            agree_to: None,
            pending: None,
        },
        Period::Annual => PeriodSwitch {
            to: Period::Monthly.as_str().into(),
            words: format!(
                "Pay monthly instead: {} a month. It starts at {when}.",
                usd(plan.price_cents)
            ),
            button: "Switch to monthly".into(),
            agree_to: None,
            pending: None,
        },
    }
}

/// The page's view, from W3's billing and W6's Effects.
async fn view_for(st: &BillingRoutes, site: &str, account: Uuid) -> anyhow::Result<BillingPage> {
    let billing = agentrix_billing::Billing::new(st.pool.clone());
    let sub = billing
        .for_site(site)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let choice = plan_choice(&st.pool, site).await?;
    let plan_id = sub
        .as_ref()
        .map(|s| s.plan_id.clone())
        .or_else(|| choice.as_ref().map(|c| c.plan_id.clone()));
    let plan = plan_id.as_deref().and_then(agentrix_billing::plan);
    let plans = PlanPage::new(site, None, &crate::plans(), "/").plans;
    let mut view = BillingPage {
        site_id: site.to_owned(),
        plans,
        current_plan: plan_id.clone().unwrap_or_default(),
        plan_href: format!(
            "/setup/plan?site={}&next={}",
            encode(site),
            encode(&format!("/billing?site={site}"))
        ),
        ..BillingPage::default()
    };
    let (chip_class, chip_words) = match sub.as_ref().map(|s| s.state.as_str()) {
        Some("active") => ("chip-ok", "Active"),
        Some("past_due") => ("chip-risk", "Payment overdue"),
        Some("pending_approval") => ("chip-wait", "Waiting for PayPal"),
        Some("cancelled") => ("", "Cancelled"),
        Some("suspended") => ("chip-risk", "Suspended"),
        Some(_) => ("", "Ended"),
        None => ("", "No plan yet"),
    };
    view.state_chip = Chip {
        class: chip_class.into(),
        words: chip_words.into(),
    };
    if let Some(p) = plan {
        view.has_plan = true;
        // The period the plan is paid by: the subscription's, or before one, the choice's.
        let live_sub = sub
            .as_ref()
            .filter(|s| matches!(s.state.as_str(), "active" | "past_due" | "pending_approval"));
        let period = live_sub
            .map(|s| s.period())
            .or_else(|| choice.as_ref().map(|c| c.period))
            .unwrap_or_default();
        let asked = choice.as_ref().map_or(period, |c| c.period);
        let price = live_sub.map_or(p.charge_cents(period), |s| s.price_cents);
        let mut summary = format!(
            "{} · {} · PayPal subscription",
            p.name,
            charge_words(period, price)
        );
        let active = sub.as_ref().filter(|s| s.state == "active");
        let renewal = active.and_then(|s| s.next_charge());
        if let Some(day) = active
            .and_then(|s| s.period_end.as_deref())
            .and_then(day_words)
        {
            summary.push_str(&format!(" · next charge {day}"));
        }
        view.summary = summary;
        view.period_words = period_words(period);
        view.next_charge = renewal.map(|at| format!("{}, {}", date_words(at), usd(price)));
        if p.annual_cents() > 0 {
            view.switch = Some(period_switch(p, period, asked, renewal, active.is_some()));
        }
        let budget_usd = choice
            .as_ref()
            .map_or(p.included_ai_micro_usd / 1_000_000, |c| c.ai_budget_usd);
        view.budget_words = Some(format!("${budget_usd}"));
        if let Ok(usage) = billing.usage(Utc::now()).await
            && let Some(u) = usage.iter().find(|u| u.site_id == site)
        {
            view.spent_words = Some(dollars_micro(u.spent_micro_usd));
            let cap = (budget_usd * 1_000_000).max(1);
            let pct = (u.spent_micro_usd * 100 / cap).clamp(0, 100);
            view.spent_pct = ((pct / 5) * 5) as u8;
        }
    }
    // The cancel card, for a subscription that can still be cancelled.
    if let Ok(subs) = st.effects.subscriptions_for_site(&SiteId::new(site)).await
        && let Some(live) = subs
            .iter()
            .filter(|s| matches!(s.state.as_str(), "active" | "pending_approval" | "past_due"))
            .max_by_key(|s| s.created_at)
    {
        let fp = cancel_fingerprint(site, &live.id);
        let options = st
            .approvals
            .passkey_options(account, fp)
            .await
            .ok()
            .flatten()
            .map(|v| v.to_string());
        let name = plan.map_or("your plan", |p| p.name.as_str());
        view.cancel = Some(CancelCard {
            title: format!("Cancel {name}?"),
            sign_words: format!(
                "Cancel {name} for this site: PayPal stops charging. Subscription {}.",
                live.id
            ),
            fingerprint: hex::encode(fp),
            passkey_options: options,
        });
    }
    Ok(view)
}

async fn page(State(st): State<BillingRoutes>, headers: HeaderMap, Query(q): Query<Q>) -> Response {
    let Some(site) = q.site.as_deref().and_then(safe_site) else {
        return Redirect::to("/os").into_response();
    };
    let account = match owner(&st, &headers, &site).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let view = view_for(&st, &site, account).await.map(|mut view| {
        if q.done.as_deref() == Some("period") {
            view.said = Some("Saved.".into());
        }
        view
    });
    match view {
        Ok(view) => match st.pages.render(OnboardingPage::Billing, &view) {
            Ok(body) => Html(body).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "rendering billing");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "This page didn't load on our side. Nothing was changed.",
                )
                    .into_response()
            }
        },
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading billing");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't read your billing. Nothing was changed. Try again in a moment.",
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
struct CancelBody {
    site: String,
    #[serde(default)]
    passkey: Option<Value>,
    #[serde(default)]
    totp: Option<String>,
}

fn said(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

async fn cancel(
    State(st): State<BillingRoutes>,
    headers: HeaderMap,
    Json(b): Json<CancelBody>,
) -> Response {
    let Some(site) = safe_site(&b.site) else {
        return said(
            StatusCode::BAD_REQUEST,
            "That isn't one of your sites. Nothing was changed.",
        );
    };
    let account = match owner(&st, &headers, &site).await {
        Ok(a) => a,
        Err(_) => {
            return said(
                StatusCode::FORBIDDEN,
                "Only the site's owner can cancel its plan. Nothing was changed.",
            );
        }
    };
    let subs = match st
        .effects
        .subscriptions_for_site(&SiteId::new(site.clone()))
        .await
    {
        Ok(s) => s,
        Err(e) => return said(StatusCode::BAD_GATEWAY, &e.to_string()),
    };
    let Some(live) = subs
        .iter()
        .filter(|s| matches!(s.state.as_str(), "active" | "pending_approval" | "past_due"))
        .max_by_key(|s| s.created_at)
    else {
        return said(
            StatusCode::CONFLICT,
            "There's no plan to cancel on this site. Nothing was changed.",
        );
    };
    let request = match (b.passkey, b.totp) {
        (Some(p), _) => json!({ "passkey": p }),
        (None, Some(code)) => json!({ "totp": code.trim() }),
        (None, None) => {
            return said(
                StatusCode::BAD_REQUEST,
                "Approve with your passkey or a code. Nothing was changed.",
            );
        }
    };
    let fp = cancel_fingerprint(&site, &live.id);
    if let Err(e) = st.approvals.verify(account, fp, request).await {
        tracing::info!(error = %format!("{e:#}"), "cancel approval refused");
        return said(
            StatusCode::FORBIDDEN,
            "That approval didn't check out, so nothing was cancelled. Try again.",
        );
    }
    match st
        .effects
        .cancel_subscription(&SiteId::new(site.clone()), &live.id, "The owner cancelled from Billing")
        .await
    {
        Ok(_) => Json(json!({ "redirect": format!("/billing?site={site}"), "said": "Cancelled. PayPal won't charge again; your site and data stay yours." })).into_response(),
        Err(e) => said(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct PeriodForm {
    site: String,
    period: String,
}

fn billing_href(site: &str) -> String {
    format!("/billing?site={}", encode(site))
}

/// Pays monthly or 12 months up front from the next renewal. The owner's choice is saved; on a
/// live PayPal subscription, PayPal's page then asks the owner to agree to the change (PayPal
/// moves it at the next billing cycle). With PayPal off there's no live subscription, so the
/// choice is what the plan is approved with later.
async fn switch_period(
    State(st): State<BillingRoutes>,
    headers: HeaderMap,
    Form(f): Form<PeriodForm>,
) -> Response {
    let Some(site) = safe_site(&f.site) else {
        return (
            StatusCode::BAD_REQUEST,
            "That isn't one of your sites. Nothing was changed.",
        )
            .into_response();
    };
    let account = match owner(&st, &headers, &site).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(period) = Period::parse(&f.period) else {
        return (
            StatusCode::BAD_REQUEST,
            "That way of paying isn't offered. Nothing was changed.",
        )
            .into_response();
    };
    let billing = agentrix_billing::Billing::new(st.pool.clone());
    let sub = billing.for_site(&site).await.ok().flatten();
    let choice = plan_choice(&st.pool, &site).await.ok().flatten();
    let Some(plan) = sub
        .as_ref()
        .map(|s| s.plan_id.clone())
        .or_else(|| choice.as_ref().map(|c| c.plan_id.clone()))
        .as_deref()
        .and_then(agentrix_billing::plan)
        .filter(|p| p.kind == "site")
    else {
        return (
            StatusCode::CONFLICT,
            "There's no plan on this site yet. Nothing was changed.",
        )
            .into_response();
    };
    let saved = match choice {
        Some(_) => set_plan_period(&st.pool, &site, period).await.map(|_| ()),
        None => {
            save_plan_choice(
                &st.pool,
                &PlanChoice {
                    site_id: site.clone(),
                    account: account.to_string(),
                    plan_id: plan.id.clone(),
                    ai_budget_usd: plan.included_ai_micro_usd / 1_000_000,
                    period,
                    chosen_at: crate::store::now(),
                },
            )
            .await
        }
    };
    if let Err(e) = saved {
        tracing::error!(error = %format!("{e:#}"), "saving the plan's period");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "We couldn't save the change. Nothing was charged. Try again in a moment.",
        )
            .into_response();
    }
    let done = format!("{}&done=period", billing_href(&site));
    // A live subscription paid the other way: PayPal's page to agree to the change.
    let live = match st
        .effects
        .subscriptions_for_site(&SiteId::new(site.clone()))
        .await
    {
        Ok(subs) => subs
            .into_iter()
            .filter(|s| s.state == "active")
            .max_by_key(|s| s.created_at),
        Err(_) => None,
    };
    let Some(live) = live.filter(|l| {
        agentrix_billing::plan_key_parts(&l.plan_key).2 != period && st.effects.enabled()
    }) else {
        return Redirect::to(&done).into_response();
    };
    let back = |path: &str| st.origin.join(path).map(|u| u.to_string());
    let (Ok(return_url), Ok(cancel_url)) = (
        back(&format!("/billing/period/return?site={}", encode(&site))),
        back(&billing_href(&site)),
    ) else {
        return Redirect::to(&done).into_response();
    };
    match st
        .effects
        .revise_subscription(
            &SiteId::new(site.clone()),
            &live.id,
            &crate::pay::plan_price(plan, period),
            &return_url,
            &cancel_url,
        )
        .await
    {
        Ok(approve) => Redirect::to(&approve).into_response(),
        Err(EffectError::NotConfigured) => Redirect::to(&done).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "asking PayPal to switch the period");
            Redirect::to(&done).into_response()
        }
    }
}

/// PayPal sends the owner back after agreeing to a switch: the plan is read back from PayPal.
async fn back_from_paypal(
    State(st): State<BillingRoutes>,
    headers: HeaderMap,
    Query(q): Query<Q>,
) -> Response {
    let Some(site) = q.site.as_deref().and_then(safe_site) else {
        return Redirect::to("/os").into_response();
    };
    if let Err(r) = owner(&st, &headers, &site).await {
        return r;
    }
    if let Some(id) = q.subscription_id.as_deref().filter(|i| i.len() <= 64) {
        if let Err(e) = st.effects.refresh_subscription(id).await {
            tracing::warn!(error = %e, "refreshing the subscription after a switch");
        }
        let _ = agentrix_billing::Billing::new(st.pool.clone()).sync().await;
    }
    Redirect::to(&billing_href(&site)).into_response()
}

/// The billing routes, for [`crate::mount`].
pub fn router(st: BillingRoutes) -> Router {
    Router::new()
        .route("/billing", get(page))
        .route("/billing/cancel", post(cancel))
        .route("/billing/period", post(switch_period))
        .route("/billing/period/return", get(back_from_paypal))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_signs_its_site_and_subscription_only() {
        assert_eq!(
            cancel_fingerprint("s1", "sub_1"),
            cancel_fingerprint("s1", "sub_1")
        );
        assert_ne!(
            cancel_fingerprint("s1", "sub_1"),
            cancel_fingerprint("s2", "sub_1")
        );
        assert_ne!(
            cancel_fingerprint("s1", "sub_1"),
            cancel_fingerprint("s1", "sub_2")
        );
    }

    #[test]
    fn switching_period_says_what_it_costs_and_when_it_starts() {
        let commerce = agentrix_billing::plan("commerce").unwrap();
        let renewal = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 12, 7, 9, 0, 0).single();
        let to_annual = period_switch(commerce, Period::Monthly, Period::Monthly, renewal, true);
        assert_eq!(to_annual.to, "annual");
        assert_eq!(to_annual.button, "Switch to 12 months");
        assert_eq!(
            to_annual.words,
            "Pay for 12 months at a time: $1,430.40 once a year instead of $1,788 paid monthly, so you save $357.60. It starts at your next renewal, Mon 7 Dec 2026."
        );
        let asked = period_switch(commerce, Period::Monthly, Period::Annual, renewal, true);
        assert_eq!(
            asked.words,
            "From your next renewal, Mon 7 Dec 2026, you pay $1,430.40 for 12 months."
        );
        assert_eq!(
            (asked.to.as_str(), asked.button.as_str()),
            ("monthly", "Keep paying monthly")
        );
        assert!(asked.pending.is_some());
        let to_monthly = period_switch(commerce, Period::Annual, Period::Annual, None, false);
        assert_eq!(
            to_monthly.words,
            "Pay monthly instead: $149 a month. It starts at your first charge, the day you approve it on PayPal."
        );
        assert_eq!(period_words(Period::Annual), "12 months, 20% off");
        assert_eq!(
            charge_words(Period::Annual, 143_040),
            "$1,430.40 for 12 months"
        );
    }

    #[test]
    fn spend_reads_in_dollars_and_cents() {
        assert_eq!(dollars_micro(3_400_000), "$3.40");
        assert_eq!(dollars_micro(0), "$0.00");
        assert_eq!(dollars_micro(214_999), "$0.21");
    }
}
