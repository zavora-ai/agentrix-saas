//! The owner's side of support access, on the platform host (08 §2.5): the owner reads exactly
//! what Zavora asks for, and grants it with a passkey or an authenticator code bound to the
//! grant's terms, declines it, or ends it early. Operators never approve this for an owner.

use std::sync::Arc;

use agentrix_host::{Approvals, OwnerSessions, Receipts};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use url::Url;
use uuid::Uuid;

use crate::access::{self, Grant, tell_owner};
use crate::identity::{account, approve};
use crate::ledger::{self, Entry, time};
use crate::render::Render;

/// The owner side's state.
#[derive(Clone)]
pub(crate) struct OwnerState {
    pool: SqlitePool,
    approvals: Arc<dyn Approvals>,
    owners: Arc<dyn OwnerSessions>,
    receipts: Option<Arc<dyn Receipts>>,
    render: Arc<Render>,
    #[allow(dead_code)]
    ops_origin: Url,
}

impl OwnerState {
    pub(crate) fn new(
        pool: SqlitePool,
        approvals: Arc<dyn Approvals>,
        owners: Arc<dyn OwnerSessions>,
        receipts: Option<Arc<dyn Receipts>>,
        ops_origin: Url,
    ) -> Result<Self, crate::Error> {
        Ok(Self {
            pool,
            approvals,
            owners,
            receipts,
            render: Arc::new(Render::new(180)?),
            ops_origin,
        })
    }
}

pub(crate) fn router(state: OwnerState) -> axum::Router {
    Router::new()
        .route("/support-access.json", get(waiting))
        .route("/support-access/{grant_id}", get(page))
        .route("/support-access/{grant_id}/approve", post(grant_it))
        .route("/support-access/{grant_id}/decline", post(decline))
        .route("/support-access/{grant_id}/end", post(end))
        .route("/agency-access/{partner}/{site}", get(agency_page))
        .route(
            "/agency-access/{partner}/{site}/approve",
            post(agency_grant),
        )
        .route(
            "/agency-access/{partner}/{site}/decline",
            post(agency_decline),
        )
        .route(
            "/agency-access/{partner}/{site}/revoke",
            post(agency_revoke),
        )
        .with_state(state)
        .merge(crate::assets::routes())
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

/// A refusal the owner reads in their browser: a page in words, never raw JSON. `words` is
/// always one of this file's fixed sentences.
fn refuse_page(status: StatusCode, words: &str) -> Response {
    (
        status,
        [("cache-control", "no-store")],
        Html(format!(
            "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>Agentrix</title>\
             <p style=\"font-family:system-ui;padding:24px;max-width:560px\">{words} <a href=\"/\">Go to Agentrix</a></p>"
        )),
    )
        .into_response()
}

/// `refuse` for the JSON islands, `refuse_page` for pages and form posts.
fn refusal(html: bool, status: StatusCode, words: &str) -> Response {
    if html {
        refuse_page(status, words)
    } else {
        refuse(status, words)
    }
}

fn fail(e: impl std::fmt::Display) -> Response {
    crate::pages::internal(e)
}

/// The signed-in owner of `grant`'s site, or the response to send instead.
async fn owner_of(
    s: &OwnerState,
    headers: &HeaderMap,
    grant_id: &str,
    html: bool,
) -> Result<(Uuid, String, Grant, String), Response> {
    let owner = s.owners.owner(headers).await.map_err(fail)?;
    let Some(owner) = owner else {
        return Err(if html {
            Redirect::to(&format!("/auth/signin?next=/support-access/{grant_id}")).into_response()
        } else {
            refuse(StatusCode::UNAUTHORIZED, "Sign in first.")
        });
    };
    let Some(g) = access::grant(&s.pool, grant_id).await.map_err(fail)? else {
        return Err(refusal(
            html,
            StatusCode::NOT_FOUND,
            "There's no support request with that link.",
        ));
    };
    let row: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT owner_account_id, domain FROM sites WHERE site_id = ?")
            .bind(&g.site_id)
            .fetch_optional(&s.pool)
            .await
            .map_err(fail)?;
    let Some((Some(site_owner), domain)) = row else {
        return Err(refusal(
            html,
            StatusCode::NOT_FOUND,
            "There's no support request with that link.",
        ));
    };
    if site_owner != owner.account.to_string() {
        return Err(refusal(
            html,
            StatusCode::FORBIDDEN,
            "Only the site's owner can answer this request. Sign in as the owner to see it.",
        ));
    }
    let name = account(&s.pool, owner.account)
        .await
        .map_err(fail)?
        .map(|a| a.name)
        .unwrap_or_default();
    Ok((owner.account, name, g, domain))
}

fn same_origin(headers: &HeaderMap) -> bool {
    // The platform's own CSRF stance: SameSite=Lax cookies plus a same-site Origin on writes.
    match (
        headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()),
        headers.get(header::HOST).and_then(|h| h.to_str().ok()),
    ) {
        (None, _) => true,
        (Some(origin), Some(host)) => origin.split("://").nth(1).is_some_and(|o| o == host),
        _ => false,
    }
}

async fn names(pool: &SqlitePool, ids_json: &str) -> Vec<String> {
    let mut out = Vec::new();
    for id in serde_json::from_str::<Vec<String>>(ids_json).unwrap_or_default() {
        if let Ok(uuid) = Uuid::parse_str(&id) {
            if let Ok(Some(a)) = account(pool, uuid).await {
                out.push(a.name);
            }
        }
    }
    out
}

fn fingerprint(g: &Grant) -> Option<[u8; 32]> {
    hex::decode(&g.fingerprint).ok()?.try_into().ok()
}

async fn page(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
) -> Response {
    if let Err(e) = access::expire(&s.pool).await {
        return fail(e);
    }
    let (owner, owner_name, g, domain) = match owner_of(&s, &headers, &grant_id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let options = if g.state == "requested" {
        match fingerprint(&g) {
            Some(fp) => match s.approvals.passkey_options(owner, fp).await {
                Ok(o) => o,
                Err(e) => return fail(e),
            },
            None => None,
        }
    } else {
        None
    };
    let grantees = names(&s.pool, &g.grantees).await;
    let method = match (g.state.as_str(), options.is_some()) {
        ("requested", true) => "passkey",
        ("requested", false) => "totp",
        _ => "none",
    };
    let html = s.render.page(
        "owner_grant.html",
        &json!({
            "g": g,
            "areas": g.area_list(),
            "grantees": grantees,
            "domain": domain,
            "owner_name": owner_name,
            "options": options.map(|o| o.to_string()),
            "method": method,
        }),
    );
    match html {
        Ok(h) => (
            [
                ("cache-control", "no-store"),
                ("referrer-policy", "same-origin"),
            ],
            Html(h),
        )
            .into_response(),
        Err(e) => fail(e),
    }
}

async fn grant_it(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if !same_origin(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "This request came from another site, so it was refused.",
        );
    }
    let (owner, owner_name, g, domain) = match owner_of(&s, &headers, &grant_id, false).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if g.state != "requested" {
        return refuse(StatusCode::CONFLICT, "This request was already answered.");
    }
    let Some(fp) = fingerprint(&g) else {
        return fail("stored grant fingerprint is not 32 bytes");
    };
    let proof = match approve(s.approvals.as_ref(), owner, fp, &body).await {
        Ok(p) => p,
        Err(words) => return refuse(StatusCode::UNAUTHORIZED, &words),
    };
    let now = Utc::now();
    let ends = now + chrono::Duration::hours(g.hours);
    let done = sqlx::query(
        "UPDATE support_grants SET state = 'active', decided_by = ?, decided_method = ?, decided_at = ?, starts_at = ?, ends_at = ? \
         WHERE grant_id = ? AND state = 'requested'",
    )
    .bind(owner.to_string())
    .bind(proof.method)
    .bind(time(now))
    .bind(time(now))
    .bind(time(ends))
    .bind(&grant_id)
    .execute(&s.pool)
    .await;
    match done {
        Ok(d) if d.rows_affected() == 1 => {}
        Ok(_) => return refuse(StatusCode::CONFLICT, "This request was already answered."),
        Err(e) => return fail(e),
    }
    let grantees = names(&s.pool, &g.grantees).await.join(" and ");
    let access_words = if g.access == "read" {
        "read"
    } else {
        "read and change"
    };
    let how = if proof.method == "passkey" {
        "with your passkey"
    } else {
        "with an authenticator code"
    };
    let receipt = tell_owner(
        s.receipts.as_deref(),
        &g.site_id,
        "decision.approved",
        format!("You let {grantees} of Zavora {access_words} your site's {} for {}, {how}", g.area_list().join(", "), crate::notice::plural(g.hours, "hour", "hours")),
        json!({ "card": "support-grant", "grant": grant_id, "fingerprint": g.fingerprint, "method": proof.method, "approver": owner, "areas": g.area_list(), "access": g.access, "hours": g.hours, "ends_at": time(ends), "proof": proof.evidence }),
    )
    .await;
    if let Some(r) = &receipt {
        let _ = sqlx::query("UPDATE support_grants SET receipt = ? WHERE grant_id = ?")
            .bind(r)
            .bind(&grant_id)
            .execute(&s.pool)
            .await;
    }
    if let Err(e) = ledger::append(
        &s.pool,
        Entry::person(
            owner,
            &owner_name,
            "grant.approve",
            &format!("site:{}", g.site_id),
            &format!(
                "{owner_name}, owner of {domain}, granted support access to {} for {} {how}.",
                g.area_list().join(", "),
                crate::notice::plural(g.hours, "hour", "hours")
            ),
        )
        .approval(
            json!([{ "account": owner, "method": proof.method, "fingerprint": g.fingerprint }]),
        )
        .detail(json!({ "grant": grant_id, "receipt": receipt })),
    )
    .await
    {
        return fail(e);
    }
    Json(json!({ "next": format!("/support-access/{grant_id}") })).into_response()
}

async fn decline(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
) -> Response {
    answer(s, headers, grant_id, "requested", "declined").await
}

async fn end(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
) -> Response {
    answer(s, headers, grant_id, "active", "revoked").await
}

async fn answer(
    s: OwnerState,
    headers: HeaderMap,
    grant_id: String,
    from: &str,
    to: &str,
) -> Response {
    if !same_origin(&headers) {
        return crate::notice::cross_site();
    }
    let (owner, owner_name, g, domain) = match owner_of(&s, &headers, &grant_id, true).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    if g.state != from {
        return Redirect::to(&format!("/support-access/{grant_id}")).into_response();
    }
    let now = ledger::now();
    let sql = if to == "declined" {
        "UPDATE support_grants SET state = 'declined', decided_by = ?, decided_at = ? WHERE grant_id = ? AND state = 'requested'"
    } else {
        "UPDATE support_grants SET state = 'revoked', revoked_by = ?, revoked_at = ? WHERE grant_id = ? AND state = 'active'"
    };
    if let Err(e) = sqlx::query(sql)
        .bind(owner.to_string())
        .bind(&now)
        .bind(&grant_id)
        .execute(&s.pool)
        .await
    {
        return fail(e);
    }
    let (title, summary) = if to == "declined" {
        (
            "You declined Zavora Support's request to see your site".to_owned(),
            format!("{owner_name}, owner of {domain}, declined the support request."),
        )
    } else {
        (
            "You ended Zavora Support's access to your site early".to_owned(),
            format!("{owner_name}, owner of {domain}, ended support access early."),
        )
    };
    tell_owner(
        s.receipts.as_deref(),
        &g.site_id,
        "decision.declined",
        title,
        json!({ "card": "support-grant", "grant": grant_id, "by": owner, "state": to }),
    )
    .await;
    if let Err(e) = ledger::append(
        &s.pool,
        Entry::person(
            owner,
            &owner_name,
            if to == "declined" {
                "grant.decline"
            } else {
                "grant.revoke"
            },
            &format!("site:{}", g.site_id),
            &summary,
        )
        .detail(json!({ "grant": grant_id })),
    )
    .await
    {
        return fail(e);
    }
    Redirect::to(&format!("/support-access/{grant_id}")).into_response()
}

async fn waiting(State(s): State<OwnerState>, headers: HeaderMap) -> Response {
    let owner = match s.owners.owner(&headers).await {
        Ok(Some(o)) => o,
        Ok(None) => return refuse(StatusCode::UNAUTHORIZED, "Sign in first."),
        Err(e) => return fail(e),
    };
    if let Err(e) = access::expire(&s.pool).await {
        return fail(e);
    }
    let grants = match access::open_grants_for_owner(&s.pool, owner.account).await {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let mut items = Vec::new();
    for g in grants {
        items.push(json!({
            "grant": g.grant_id,
            "site": g.site_id,
            "state": g.state,
            "who": names(&s.pool, &g.grantees).await,
            "areas": g.area_list(),
            "access": g.access,
            "hours": g.hours,
            "reason": g.reason,
            "ends_at": g.ends_at,
            "href": format!("/support-access/{}", g.grant_id),
        }));
    }
    Json(json!({ "support_access": items })).into_response()
}

// ---------- Agency roles (08 §2.5: the owner grants, and can revoke at any time) ----------

/// The partner's request on the signed-in owner's site, or the response to send instead.
async fn agency_of(
    s: &OwnerState,
    headers: &HeaderMap,
    partner: &str,
    site: &str,
    html: bool,
) -> Result<
    (
        Uuid,
        String,
        String,
        crate::partners::Partner,
        (String, String, String, String),
    ),
    Response,
> {
    let owner = s.owners.owner(headers).await.map_err(fail)?;
    let Some(owner) = owner else {
        return Err(if html {
            Redirect::to(&format!(
                "/auth/signin?next=/agency-access/{partner}/{site}"
            ))
            .into_response()
        } else {
            refuse(StatusCode::UNAUTHORIZED, "Sign in first.")
        });
    };
    let row: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT owner_account_id, domain FROM sites WHERE site_id = ?")
            .bind(site)
            .fetch_optional(&s.pool)
            .await
            .map_err(fail)?;
    let Some((Some(site_owner), domain)) = row else {
        return Err(refusal(
            html,
            StatusCode::NOT_FOUND,
            "There's no request with that link.",
        ));
    };
    if site_owner != owner.account.to_string() {
        return Err(refusal(
            html,
            StatusCode::FORBIDDEN,
            "Only the site's owner can answer this request. Sign in as the owner to see it.",
        ));
    }
    let Some(p) = crate::partners::get_partner(&s.pool, partner)
        .await
        .map_err(fail)?
    else {
        return Err(refusal(
            html,
            StatusCode::NOT_FOUND,
            "There's no request with that link.",
        ));
    };
    let link: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT charter_role, ring, state, fingerprint FROM partner_clients WHERE partner_id = ? AND site_id = ?",
    )
    .bind(partner)
    .bind(site)
    .fetch_optional(&s.pool)
    .await
    .map_err(fail)?;
    let Some(link) = link else {
        return Err(refusal(
            html,
            StatusCode::NOT_FOUND,
            "There's no request with that link.",
        ));
    };
    let name = account(&s.pool, owner.account)
        .await
        .map_err(fail)?
        .map(|a| a.name)
        .unwrap_or_default();
    Ok((owner.account, name, domain, p, link))
}

async fn agency_page(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path((partner, site)): Path<(String, String)>,
) -> Response {
    let (owner, owner_name, domain, p, (role, ring, state, fp_hex)) =
        match agency_of(&s, &headers, &partner, &site, true).await {
            Ok(x) => x,
            Err(r) => return r,
        };
    let options = if state == "invited" {
        let fp: Option<[u8; 32]> = hex::decode(&fp_hex).ok().and_then(|b| b.try_into().ok());
        match fp {
            Some(fp) => match s.approvals.passkey_options(owner, fp).await {
                Ok(o) => o,
                Err(e) => return fail(e),
            },
            None => None,
        }
    } else {
        None
    };
    let method = match (state.as_str(), options.is_some()) {
        ("invited", true) => "passkey",
        ("invited", false) => "totp",
        _ => "none",
    };
    let html = s.render.page(
        "owner_agency.html",
        &json!({ "p": p, "site": site, "domain": domain, "role": role, "ring": ring, "state": state, "owner_name": owner_name, "options": options.map(|o| o.to_string()), "method": method }),
    );
    match html {
        Ok(h) => (
            [
                ("cache-control", "no-store"),
                ("referrer-policy", "same-origin"),
            ],
            Html(h),
        )
            .into_response(),
        Err(e) => fail(e),
    }
}

async fn agency_grant(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path((partner, site)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    if !same_origin(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "This request came from another site, so it was refused.",
        );
    }
    let (owner, owner_name, domain, p, (role, _, state, fp_hex)) =
        match agency_of(&s, &headers, &partner, &site, false).await {
            Ok(x) => x,
            Err(r) => return r,
        };
    if state != "invited" {
        return refuse(StatusCode::CONFLICT, "This request was already answered.");
    }
    let Some(fp) = hex::decode(&fp_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b).ok())
    else {
        return fail("stored agency fingerprint is not 32 bytes");
    };
    let proof = match approve(s.approvals.as_ref(), owner, fp, &body).await {
        Ok(p) => p,
        Err(words) => return refuse(StatusCode::UNAUTHORIZED, &words),
    };
    let now = time(Utc::now());
    let done = sqlx::query(
        "UPDATE partner_clients SET state = 'granted', decided_by = ?, decided_method = ?, decided_at = ? WHERE partner_id = ? AND site_id = ? AND state = 'invited'",
    )
    .bind(owner.to_string())
    .bind(proof.method)
    .bind(&now)
    .bind(&partner)
    .bind(&site)
    .execute(&s.pool)
    .await;
    match done {
        Ok(d) if d.rows_affected() == 1 => {}
        Ok(_) => return refuse(StatusCode::CONFLICT, "This request was already answered."),
        Err(e) => return fail(e),
    }
    let how = if proof.method == "passkey" {
        "with your passkey"
    } else {
        "with an authenticator code"
    };
    let receipt = tell_owner(
        s.receipts.as_deref(),
        &site,
        "decision.approved",
        format!("You made {} your site's {role}, {how}. You can take it back at any time.", p.name),
        json!({ "card": "agency-role", "partner": partner, "role": role, "fingerprint": fp_hex, "method": proof.method, "approver": owner, "proof": proof.evidence }),
    )
    .await;
    if let Some(r) = &receipt {
        let _ = sqlx::query(
            "UPDATE partner_clients SET receipt = ? WHERE partner_id = ? AND site_id = ?",
        )
        .bind(r)
        .bind(&partner)
        .bind(&site)
        .execute(&s.pool)
        .await;
    }
    if let Err(e) = ledger::append(
        &s.pool,
        Entry::person(
            owner,
            &owner_name,
            "partner.grant",
            &format!("site:{site}"),
            &format!(
                "{owner_name}, owner of {domain}, made {} the site's {role} {how}.",
                p.name
            ),
        )
        .approval(json!([{ "account": owner, "method": proof.method, "fingerprint": fp_hex }])),
    )
    .await
    {
        return fail(e);
    }
    Json(json!({ "next": format!("/agency-access/{partner}/{site}") })).into_response()
}

async fn agency_decline(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path((partner, site)): Path<(String, String)>,
) -> Response {
    agency_answer(s, headers, partner, site, "invited", "declined").await
}

async fn agency_revoke(
    State(s): State<OwnerState>,
    headers: HeaderMap,
    Path((partner, site)): Path<(String, String)>,
) -> Response {
    agency_answer(s, headers, partner, site, "granted", "revoked").await
}

async fn agency_answer(
    s: OwnerState,
    headers: HeaderMap,
    partner: String,
    site: String,
    from: &str,
    to: &str,
) -> Response {
    if !same_origin(&headers) {
        return crate::notice::cross_site();
    }
    let (owner, owner_name, domain, p, (role, _, state, _)) =
        match agency_of(&s, &headers, &partner, &site, true).await {
            Ok(x) => x,
            Err(r) => return r,
        };
    if state != from {
        return Redirect::to(&format!("/agency-access/{partner}/{site}")).into_response();
    }
    let now = time(Utc::now());
    if let Err(e) = sqlx::query("UPDATE partner_clients SET state = ?, decided_by = COALESCE(decided_by, ?), revoked_at = CASE WHEN ? = 'revoked' THEN ? ELSE revoked_at END WHERE partner_id = ? AND site_id = ? AND state = ?")
        .bind(to)
        .bind(owner.to_string())
        .bind(to)
        .bind(&now)
        .bind(&partner)
        .bind(&site)
        .bind(from)
        .execute(&s.pool)
        .await
    {
        return fail(e);
    }
    let (title, summary) = if to == "declined" {
        (
            format!("You declined {}'s request to be your site's {role}", p.name),
            format!(
                "{owner_name}, owner of {domain}, declined {}'s request.",
                p.name
            ),
        )
    } else {
        (
            format!("You took back {}'s {role} role on your site", p.name),
            format!(
                "{owner_name}, owner of {domain}, revoked {}'s {role} role.",
                p.name
            ),
        )
    };
    tell_owner(s.receipts.as_deref(), &site, "decision.declined", title, json!({ "card": "agency-role", "partner": partner, "role": role, "by": owner, "state": to })).await;
    if let Err(e) = ledger::append(
        &s.pool,
        Entry::person(
            owner,
            &owner_name,
            if to == "declined" {
                "partner.decline"
            } else {
                "partner.revoke"
            },
            &format!("site:{site}"),
            &summary,
        ),
    )
    .await
    {
        return fail(e);
    }
    Redirect::to(&format!("/agency-access/{partner}/{site}")).into_response()
}
