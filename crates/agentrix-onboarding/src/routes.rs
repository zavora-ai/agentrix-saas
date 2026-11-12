//! The pages this crate serves itself on the platform host, through [`crate::mount`].
//!
//! | Route | Page |
//! |---|---|
//! | `GET /start` | The address bar on its own (O1.1); its form posts to node's `POST /assess` |
//! | `GET /account/new?next=&host=` | Create the account (O2.1) |
//! | `GET /account/signin?next=` | Sign in (O2.3) |
//! | `GET /account/recover?next=` | Recover with a code, or by email with a hold |
//! | `GET /setup?next=&host=` | The Chief WebAgent introduces itself (O2.4); signed in only |
//! | `GET /setup/plan?site=&host=&next=` | Choose a plan and the AI budget (O3.1); signed in only |
//! | `POST /setup/plan` (form `site, plan_id, period, ai_budget_usd, next, host`) | Saves the choice (`period` is `monthly` or `annual`, 12 months at the catalogue's discount), then `next` |
//!
//! The account pages are static apart from the query; `static/onboarding/account.js` drives
//! The first release's passkey and authenticator endpoints and W6's email-code and recovery endpoints.

use std::sync::Arc;

use agentrix_billing::Period;
use agentrix_host::OwnerSessions;
use axum::Router;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use serde::Deserialize;

use crate::store::{PlanChoice, plan_choice, save_plan_choice};
use crate::view::{
    AccountMode, AccountPage, PlanInput, PlanPage, SetupPage, SetupStep, StartPage, encode,
};
use crate::{OnboardingPage, Pages};

/// Where a signed-in owner goes when nothing else is asked for.
pub const DEFAULT_NEXT: &str = "/os";

/// `next` if it's a path on this host, otherwise [`DEFAULT_NEXT`]. Refuses other hosts
/// (`//evil.example`, `https://…`), backslashes and control characters, so a link can't send
/// a freshly signed-in owner somewhere else.
pub fn safe_next(next: Option<&str>) -> String {
    match next.map(str::trim) {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.contains('\\')
                && !n.chars().any(char::is_control)
                && n.len() <= 512 =>
        {
            n.to_owned()
        }
        _ => DEFAULT_NEXT.to_owned(),
    }
}

/// A host name as typed in a link: letters, digits, dots and dashes only.
fn safe_host(host: Option<&str>) -> Option<String> {
    let h = host?.trim().to_ascii_lowercase();
    let ok = !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    ok.then_some(h)
}

#[derive(Debug, Default, Deserialize)]
struct Q {
    next: Option<String>,
    host: Option<String>,
    site: Option<String>,
}

#[derive(Clone)]
struct RouteState {
    pages: Arc<Pages>,
    owners: Arc<dyn OwnerSessions>,
    pool: sqlx::SqlitePool,
    /// W3's plan catalogue; empty until it's published here.
    plans: Arc<Vec<PlanInput>>,
}

/// A site id as it appears in a link: letters, digits, `_` and `-`.
fn safe_site(site: Option<&str>) -> Option<String> {
    let s = site?.trim();
    (!s.is_empty()
        && s.len() <= 80
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    .then(|| s.to_owned())
}

/// The site id in a claim path, `/sites/{id}/claim`.
fn site_of_claim(next: &str) -> Option<String> {
    safe_site(next.strip_prefix("/sites/")?.strip_suffix("/claim"))
}

/// Sign-in first, then back to `back`.
fn sign_in_then(back: &str) -> Response {
    Redirect::to(&format!("/account/signin?next={}", encode(back))).into_response()
}

fn html(pages: &Pages, page: OnboardingPage, view: &impl serde::Serialize) -> Response {
    match pages.render(page, view) {
        Ok(body) => Html(body).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "rendering an onboarding page");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "This page didn't load on our side. Nothing was changed. Try again in a moment.",
            )
                .into_response()
        }
    }
}

/// "Try it on Zavora's fixture" when node has a fixture configured: the same two settings
/// node's Assess reads (`AGENTRIX_FIXTURE_URL`, or the node flag's `AGENTRIX_FIXTURE_DOMAIN`).
fn fixture_href() -> Option<String> {
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.trim().is_empty());
    (set("AGENTRIX_FIXTURE_URL") || set("AGENTRIX_FIXTURE_DOMAIN"))
        .then(|| "/assess/fixture".into())
}

async fn start(State(st): State<RouteState>, headers: HeaderMap) -> Response {
    // The console sends a signed-in owner with no site here; the page then says so.
    let signed_in = matches!(st.owners.owner(&headers).await, Ok(Some(_)));
    let view = StartPage {
        fixture_url: fixture_href(),
        signed_in,
        ..StartPage::default()
    };
    html(&st.pages, OnboardingPage::Start, &view)
}

async fn account(st: RouteState, mode: AccountMode, headers: HeaderMap, q: Q) -> Response {
    let next = safe_next(q.next.as_deref());
    let mut view = AccountPage::new(mode, &next, safe_host(q.host.as_deref()));
    // The first release's passkey "register" adds a passkey to whoever is signed in, so the create and
    // sign-in forms are never shown to a signed-in browser (W0, 8 Nov: a passkey landed on the
    // wrong account). Recovery keeps its page: it shows a signed-in owner's holds.
    view.signed_in =
        mode != AccountMode::Recover && matches!(st.owners.owner(&headers).await, Ok(Some(_)));
    html(&st.pages, OnboardingPage::Account, &view)
}

async fn setup(State(st): State<RouteState>, headers: HeaderMap, Query(q): Query<Q>) -> Response {
    let next = safe_next(q.next.as_deref());
    let host = safe_host(q.host.as_deref());
    match st.owners.owner(&headers).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            let back = format!(
                "/setup?next={}{}",
                encode(&next),
                host.as_deref()
                    .map(|h| format!("&host={}", encode(h)))
                    .unwrap_or_default()
            );
            return Redirect::to(&format!("/account/signin?next={}", encode(&back)))
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the owner session");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't check who's signed in. Nothing was changed. Try again in a moment.",
            )
                .into_response();
        }
    }
    let site = host.clone().unwrap_or_else(|| "your site".into());
    let claiming = next.ends_with("/claim");
    let mut steps = vec![SetupStep {
        n: 1,
        title: "Your account".into(),
        words: "Made, and signed in on this device.".into(),
        done: true,
    }];
    // The plan comes before the claim when plans are published here (owner-plan.html).
    let plan_href = site_of_claim(&next)
        .filter(|_| !st.plans.is_empty())
        .map(|id| {
            format!(
                "/setup/plan?site={}&next={}{}",
                encode(&id),
                encode(&next),
                host.as_deref()
                    .map(|h| format!("&host={}", encode(h)))
                    .unwrap_or_default()
            )
        });
    if plan_href.is_some() {
        steps.push(SetupStep {
            n: 2,
            title: "Choose a plan".into(),
            words: "In US dollars, with an AI budget you set as a limit. Nothing is charged until \
                    your site goes live."
                .into(),
            done: false,
        });
    }
    if claiming {
        steps.push(SetupStep {
            n: steps.len() as u32 + 1,
            title: format!("Prove {site} is yours"),
            words: "One DNS record or one file. The Caretaker checks it from the sandbox.".into(),
            done: false,
        });
    }
    steps.push(SetupStep {
        n: steps.len() as u32 + 1,
        title: "The Move".into(),
        words: "The Mover copies your site, leaves behind anything infected, rebuilds it and \
                checks every page against the old one."
            .into(),
        done: false,
    });
    steps.push(SetupStep {
        n: steps.len() as u32 + 1,
        title: "Go live".into(),
        words: "One card that says exactly what going live does. You approve it with your \
                passkey or a code from your authenticator app, and undo is one tap for 30 days."
            .into(),
        done: false,
    });
    let view = SetupPage {
        chief_words: match (&plan_href, claiming) {
            (Some(_), _) => format!(
                "Next, choose a plan, then prove {site} is yours, and the Mover will start."
            ),
            (None, true) => format!("Next, prove {site} is yours, and the Mover will start."),
            (None, false) => {
                "Your console shows what your WebAgents are doing and what needs you.".into()
            }
        },
        host,
        steps,
        next_words: match (&plan_href, claiming) {
            (Some(_), _) => "Next: choose a plan".into(),
            (None, true) => format!("Next: prove {site} is yours"),
            (None, false) => "Next: open your console".into(),
        },
        next_href: plan_href.unwrap_or(next),
    };
    html(&st.pages, OnboardingPage::Setup, &view)
}

async fn plan_page(
    State(st): State<RouteState>,
    headers: HeaderMap,
    Query(q): Query<Q>,
) -> Response {
    let next = safe_next(q.next.as_deref());
    let host = safe_host(q.host.as_deref());
    let Some(site) = safe_site(q.site.as_deref()) else {
        return Redirect::to(&next).into_response();
    };
    match st.owners.owner(&headers).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            let back = format!(
                "/setup/plan?site={}&next={}{}",
                encode(&site),
                encode(&next),
                host.as_deref()
                    .map(|h| format!("&host={}", encode(h)))
                    .unwrap_or_default()
            );
            return sign_in_then(&back);
        }
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the owner session");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't check who's signed in. Nothing was changed. Try again in a moment.",
            )
                .into_response();
        }
    }
    let mut view = PlanPage::new(&site, host, &st.plans, &next);
    if let Ok(Some(choice)) = plan_choice(&st.pool, &site).await
        && view.plans.iter().any(|p| p.id == choice.plan_id)
    {
        view.chosen = choice.plan_id;
        if view.annual {
            view.period = choice.period.as_str().to_owned();
        }
    }
    html(&st.pages, OnboardingPage::Plan, &view)
}

#[derive(Debug, Deserialize)]
struct PlanForm {
    site: String,
    plan_id: String,
    ai_budget_usd: i64,
    /// `monthly` (the default, for a form without the choice) or `annual`.
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    next: Option<String>,
}

async fn plan_save(
    State(st): State<RouteState>,
    headers: HeaderMap,
    Form(f): Form<PlanForm>,
) -> Response {
    let next = safe_next(f.next.as_deref());
    let owner = match st.owners.owner(&headers).await {
        Ok(Some(o)) => o,
        Ok(None) => return sign_in_then(&next),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "reading the owner session");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't check who's signed in. Nothing was changed. Try again in a moment.",
            )
                .into_response();
        }
    };
    let Some(site) = safe_site(Some(&f.site)) else {
        return (
            StatusCode::BAD_REQUEST,
            "That isn't one of your sites. Nothing was saved.",
        )
            .into_response();
    };
    // Only a published site plan, and a budget the page could offer (up to four times what's included).
    let Some(plan) = st
        .plans
        .iter()
        .find(|p| p.id == f.plan_id && p.kind == "site")
    else {
        return (
            StatusCode::BAD_REQUEST,
            "That plan isn't offered. Nothing was saved.",
        )
            .into_response();
    };
    let period = match f.period.as_deref().map(str::trim) {
        None | Some("") => Period::Monthly,
        Some(p) => {
            match Period::parse(p).filter(|p| *p == Period::Monthly || plan.annual_cents > 0) {
                Some(p) => p,
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        "That way of paying isn't offered. Nothing was saved.",
                    )
                        .into_response();
                }
            }
        }
    };
    let most = (plan.included_ai_micro_usd / 1_000_000).max(5) * 4;
    if !(0..=most).contains(&f.ai_budget_usd) {
        return (
            StatusCode::BAD_REQUEST,
            "That AI budget is outside what the plan allows. Nothing was saved.",
        )
            .into_response();
    }
    let choice = PlanChoice {
        site_id: site,
        account: owner.account.to_string(),
        plan_id: plan.id.clone(),
        ai_budget_usd: f.ai_budget_usd,
        period,
        chosen_at: crate::store::now(),
    };
    if let Err(e) = save_plan_choice(&st.pool, &choice).await {
        tracing::error!(error = %format!("{e:#}"), "saving a plan choice");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "We couldn't save your plan. Nothing was charged. Try again in a moment.",
        )
            .into_response();
    }
    Redirect::to(&next).into_response()
}

/// The routes, for [`crate::mount`]. `plans` is W3's catalogue (empty until it's published).
pub fn router(
    pages: Arc<Pages>,
    owners: Arc<dyn OwnerSessions>,
    pool: sqlx::SqlitePool,
    plans: Vec<PlanInput>,
) -> Router {
    let st = RouteState {
        pages,
        owners,
        pool,
        plans: Arc::new(plans),
    };
    Router::new()
        .route("/start", get(start))
        .route(
            "/account/new",
            get(
                |State(st): State<RouteState>, headers: HeaderMap, Query(q): Query<Q>| {
                    account(st, AccountMode::New, headers, q)
                },
            ),
        )
        .route(
            "/account/signin",
            get(
                |State(st): State<RouteState>, headers: HeaderMap, Query(q): Query<Q>| {
                    account(st, AccountMode::Signin, headers, q)
                },
            ),
        )
        .route(
            "/account/recover",
            get(
                |State(st): State<RouteState>, headers: HeaderMap, Query(q): Query<Q>| {
                    account(st, AccountMode::Recover, headers, q)
                },
            ),
        )
        .route("/setup", get(setup))
        .route("/setup/plan", get(plan_page).post(plan_save))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_stays_on_this_host() {
        assert_eq!(safe_next(Some("/sites/s1/claim")), "/sites/s1/claim");
        assert_eq!(
            safe_next(Some("/setup?next=%2Fsites%2Fs1%2Fclaim")),
            "/setup?next=%2Fsites%2Fs1%2Fclaim"
        );
        for bad in [
            "//evil.example/",
            "https://evil.example/",
            "/\\evil",
            "javascript:alert(1)",
            "",
            "/a\nb",
        ] {
            assert_eq!(safe_next(Some(bad)), DEFAULT_NEXT, "{bad:?}");
        }
        assert_eq!(safe_next(None), DEFAULT_NEXT);
    }

    #[test]
    fn hosts_are_names_only() {
        assert_eq!(
            safe_host(Some("Kilele-Bakery.example")).as_deref(),
            Some("kilele-bakery.example")
        );
        assert_eq!(safe_host(Some("<script>")), None);
        assert_eq!(safe_host(Some("localhost")), None);
    }
}
