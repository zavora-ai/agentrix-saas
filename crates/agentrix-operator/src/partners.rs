//! Partners and the portfolio scope (08 §2.2, §2.5, §4.4; concept/control-plane/portfolio.html).
//!
//! Zavora's partner people (`op.partners`, `op.admin`) create agencies and resellers, add their
//! people, and invite client sites. A client becomes the partner's only when the site's owner
//! grants the charter role on `/agency-access/{partner}/{site}` with a passkey or code; the owner
//! can revoke it any time. The portfolio shows each client content-free: stage, plan, release,
//! decisions waiting, AI spend against the plan's included budget, the role and ring.

use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::identity::{Operator, Role};
use crate::ledger::{self, Entry};
use crate::notice;
use crate::pages::internal;
use crate::{OperatorState, reads};

/// Charter roles an owner can give a partner (08 §2.2).
pub const CHARTER_ROLES: [&str; 4] = ["editor", "approver", "publisher", "viewer"];
/// Partner platform roles (08 §2.3).
pub const PARTNER_ROLES: [&str; 5] = [
    "partner.admin",
    "partner.manager",
    "partner.operator",
    "partner.billing",
    "partner.viewer",
];

pub(crate) fn routes() -> Router<OperatorState> {
    Router::new()
        .route("/portfolio", get(own_portfolio))
        .route("/partners", get(list).post(create))
        .route("/partners/{id}", get(view))
        .route("/partners/{id}/clients", post(invite))
        .route("/partners/{id}/members", post(add_member))
}

/// A partner.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Partner {
    /// `pt_…`.
    pub partner_id: String,
    /// `agency`, `reseller`, `referral`.
    pub kind: String,
    /// Its name.
    pub name: String,
    /// Its white-label console host, once set.
    pub console_host: Option<String>,
    /// Its email sender, once set.
    pub email_sender: Option<String>,
    /// Made.
    pub created_at: String,
}

/// One client in a portfolio.
#[derive(Debug, Clone, Serialize)]
pub struct Client {
    /// The site's list row.
    pub site: reads::SiteRow,
    /// `sample`, `fixture`, `customer`.
    pub kind: &'static str,
    /// The lifecycle stage.
    pub stage: &'static str,
    /// The charter role the owner granted (or was asked for).
    pub role: String,
    /// `stable` or `canary`.
    pub ring: String,
    /// `invited`, `granted`, `declined`, `revoked`.
    pub state: String,
    /// The plan's name, if the site has a subscription.
    pub plan: Option<String>,
    /// The plan's included AI a month, micro-USD.
    pub included_ai: i64,
}

/// A partner's portfolio, content-free.
#[derive(Debug, Clone, Serialize)]
pub struct Portfolio {
    /// The partner.
    pub partner: Partner,
    /// Its people: `(name, email, role)`.
    pub members: Vec<(String, String, String)>,
    /// Clients, live first.
    pub clients: Vec<Client>,
    /// Granted clients whose site is live.
    pub live: i64,
    /// Granted clients mid-Move.
    pub moving: i64,
    /// Invitations waiting for the owner.
    pub invited: i64,
    /// Owner decisions waiting on granted clients' sites.
    pub waiting: i64,
    /// Granted clients' Moves by the step they're in.
    pub stages: Vec<reads::Stage>,
    /// Wholesale owed to Zavora this month, cents: live clients times the agency price, at the
    /// minimum number of sites.
    pub wholesale_cents: i64,
    /// The agency price per site, cents.
    pub wholesale_each_cents: i64,
    /// The minimum sites billed.
    pub wholesale_minimum: i64,
    /// Model spend across granted clients this month, micro-USD.
    pub spend_micro_usd: i64,
    /// AI spend above each plan's included amount this month, across granted clients with a
    /// plan, micro-USD (gateway estimates; not billed on this build).
    pub ai_over_micro_usd: i64,
    /// Granted clients with a plan, so the line above has something to rest on.
    pub clients_with_plan: i64,
    /// Granted clients in the canary ring.
    pub ring_canary: i64,
    /// Granted clients in the stable ring.
    pub ring_stable: i64,
}

macro_rules! partner_select {
    ($rest:literal) => {
        concat!(
            "SELECT partner_id, kind, name, console_host, email_sender, created_at FROM partners ",
            $rest
        )
    };
}

/// Every partner, newest first.
pub async fn all(pool: &SqlitePool) -> Result<Vec<Partner>, sqlx::Error> {
    sqlx::query_as(partner_select!("ORDER BY created_at DESC"))
        .fetch_all(pool)
        .await
}

/// One partner.
pub async fn get_partner(pool: &SqlitePool, id: &str) -> Result<Option<Partner>, sqlx::Error> {
    sqlx::query_as(partner_select!("WHERE partner_id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// The partners `account` belongs to, with their roles.
pub async fn memberships(
    pool: &SqlitePool,
    account: Uuid,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as("SELECT partner_id, role FROM partner_members WHERE account_id = ? AND removed_at IS NULL ORDER BY added_at")
        .bind(account.to_string())
        .fetch_all(pool)
        .await
}

/// The fingerprint an owner approves when granting a partner a role on their site.
pub fn grant_fingerprint(partner: &str, site: &str, role: &str) -> [u8; 32] {
    let canonical =
        json!({ "v": 1, "kind": "agency-role", "partner": partner, "site": site, "role": role })
            .to_string();
    Sha256::digest(canonical.as_bytes()).into()
}

/// Builds a partner's portfolio at `now`.
pub async fn portfolio(
    pool: &SqlitePool,
    partner: Partner,
    now: DateTime<Utc>,
) -> Result<Portfolio, sqlx::Error> {
    let members: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT a.display_name, a.email, m.role FROM partner_members m JOIN accounts a ON a.id = m.account_id \
         WHERE m.partner_id = ? AND m.removed_at IS NULL ORDER BY m.added_at",
    )
    .bind(&partner.partner_id)
    .fetch_all(pool)
    .await?;
    let links: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT site_id, charter_role, ring, state FROM partner_clients WHERE partner_id = ? AND state IN ('invited', 'granted') ORDER BY invited_at",
    )
    .bind(&partner.partner_id)
    .fetch_all(pool)
    .await?;
    let mut clients = Vec::new();
    for (site_id, role, ring, state) in links {
        let Some(site) = reads::site_row(pool, now, &site_id).await? else {
            continue;
        };
        let plan_id: Option<(String,)> = sqlx::query_as(
            "SELECT plan_id FROM subscriptions WHERE site_id = ? AND state IN ('active', 'past_due', 'pending_approval') ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&site_id)
        .fetch_optional(pool)
        .await?;
        let plan = plan_id.and_then(|(p,)| agentrix_billing::plan(&p));
        clients.push(Client {
            kind: site.kind(),
            stage: site.stage(),
            plan: plan.map(|p| p.name.clone()),
            included_ai: plan.map(|p| p.included_ai_micro_usd).unwrap_or(0),
            site,
            role,
            ring,
            state,
        });
    }
    clients.sort_by_key(|c| {
        (
            c.state != "granted",
            c.site.live_seq.is_none(),
            c.site.decisions_waiting == 0,
        )
    });
    let granted = || clients.iter().filter(|c| c.state == "granted");
    let live = granted().filter(|c| c.site.live_seq.is_some()).count() as i64;
    let moving = granted().filter(|c| c.stage == "Moving").count() as i64;
    let mut stages: Vec<reads::Stage> = reads::STAGES
        .iter()
        .map(|(label, _)| reads::Stage { label, count: 0 })
        .collect();
    for c in granted() {
        if let Some(state) = c.site.move_state.as_deref() {
            if let Some(i) = reads::STAGES
                .iter()
                .position(|(_, states)| states.contains(&state))
            {
                stages[i].count += 1;
            }
        }
    }
    let agency = agentrix_billing::plan("agency");
    let each = agency.map(|p| p.price_cents).unwrap_or(0);
    let minimum = agency
        .and_then(|p| p.minimum_sites)
        .map(i64::from)
        .unwrap_or(0);
    let wholesale = if live == 0 {
        0
    } else {
        live.max(minimum) * each
    };
    let planned = || granted().filter(|c| c.plan.is_some());
    Ok(Portfolio {
        ai_over_micro_usd: planned()
            .map(|c| (c.site.spend_month - c.included_ai).max(0))
            .sum(),
        clients_with_plan: planned().count() as i64,
        ring_canary: granted().filter(|c| c.ring == "canary").count() as i64,
        ring_stable: granted().filter(|c| c.ring != "canary").count() as i64,
        invited: clients.iter().filter(|c| c.state == "invited").count() as i64,
        waiting: granted().map(|c| c.site.decisions_waiting).sum(),
        spend_micro_usd: granted().map(|c| c.site.spend_month).sum(),
        live,
        moving,
        stages,
        wholesale_cents: wholesale,
        wholesale_each_cents: each,
        wholesale_minimum: minimum,
        partner,
        members,
        clients,
    })
}

fn can_manage(op: &Operator) -> bool {
    op.has(Role::Partners) || op.has(Role::Admin)
}

async fn signed(s: &OperatorState, headers: &HeaderMap, uri: &Uri) -> Result<Operator, Response> {
    match s.operator_from(headers).await {
        Ok(Some(op)) => Ok(op),
        Ok(None) => {
            let next: String =
                url::form_urlencoded::byte_serialize(uri.path().as_bytes()).collect();
            Err(Redirect::to(&format!("/signin?next={next}")).into_response())
        }
        Err(e) => Err(internal(e)),
    }
}

/// The signed-in operator for a form post, or where the browser goes instead.
async fn form_operator(
    s: &OperatorState,
    headers: &HeaderMap,
    back: &str,
) -> Result<Operator, Response> {
    if !s.same_origin(headers) {
        return Err(notice::cross_site());
    }
    match s.operator_from(headers).await {
        Ok(Some(op)) => Ok(op),
        Ok(None) => Err(notice::sign_in_again(back)),
        Err(e) => Err(internal(e)),
    }
}

async fn list(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let manage = can_manage(&op);
    let shell = match crate::pages::shell(&s, op, "partners").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let partners = match all(pool).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    let mut rows: Vec<Value> = Vec::new();
    for p in partners {
        let counts: (i64, i64, i64) = match sqlx::query_as(
            "SELECT COALESCE(SUM(state = 'granted'), 0), COALESCE(SUM(state = 'invited'), 0), \
             (SELECT COUNT(*) FROM partner_members m WHERE m.partner_id = ?1 AND m.removed_at IS NULL) \
             FROM partner_clients WHERE partner_id = ?1",
        )
        .bind(&p.partner_id)
        .fetch_one(pool)
        .await
        {
            Ok(c) => c,
            Err(e) => return internal(e),
        };
        let mut v = serde_json::to_value(&p).unwrap_or(Value::Null);
        v["clients"] = json!(counts.0);
        v["invited"] = json!(counts.1);
        v["members"] = json!(counts.2);
        rows.push(v);
    }
    crate::pages::page(
        &s,
        "partners.html",
        &json!({ "shell": shell, "partners": rows, "can_manage": manage }),
    )
}

#[derive(Debug, Deserialize)]
struct NewPartner {
    name: String,
    kind: String,
}

async fn create(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Form(f): Form<NewPartner>,
) -> Response {
    let back = "/partners";
    let op = match form_operator(&s, &headers, back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_manage(&op) {
        return notice::problem(
            &s,
            back,
            "No partner was added: adding one needs op.partners or op.admin.",
        );
    }
    let name = f.name.trim();
    if name.is_empty()
        || name.chars().count() > 120
        || !["agency", "reseller", "referral"].contains(&f.kind.as_str())
    {
        return notice::problem_keeping(
            &s,
            back,
            "No partner was added. Give its name and say whether it's an agency, a reseller or a referral partner.",
            &[("name", &f.name)],
        );
    }
    let id = format!("pt_{}", Uuid::now_v7().simple());
    let pool = &s.inner.pool;
    if let Err(e) = sqlx::query("INSERT INTO partners (partner_id, kind, name, created_by, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&f.kind)
        .bind(name)
        .bind(op.account.to_string())
        .bind(ledger::now())
        .execute(pool)
        .await
    {
        return internal(e);
    }
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            "partner.create",
            &format!("pt:{id}"),
            &format!(
                "{} added {name} as a{} {}.",
                op.name,
                if f.kind == "agency" { "n" } else { "" },
                f.kind
            ),
        ),
    )
    .await
    {
        return internal(e);
    }
    notice::done(
        &s,
        &format!("/partners/{id}"),
        format!(
            "You added {name}. Invite its first client below; it joins once the owner grants a role."
        ),
    )
}

async fn view(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    uri: Uri,
    Path(id): Path<String>,
) -> Response {
    let op = match signed(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let manage = can_manage(&op);
    let shell = match crate::pages::shell(&s, op, "partners").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let partner = match get_partner(pool, &id).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                crate::pages::page(
                    &s,
                    "app.html",
                    &json!({ "shell": shell, "which": "missing-partner", "site_id": id }),
                ),
            )
                .into_response();
        }
        Err(e) => return internal(e),
    };
    let p = match portfolio(pool, partner, Utc::now()).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    let platform = s
        .inner
        .config
        .platform_origin
        .as_ref()
        .map(|u| u.as_str().trim_end_matches('/').to_owned());
    crate::pages::page(
        &s,
        "portfolio.html",
        &json!({ "shell": shell, "p": p, "as_operator": true, "can_manage": manage, "owner_link": platform.map(|p| format!("{p}/agency-access/")) }),
    )
}

#[derive(Debug, Deserialize)]
struct Invite {
    site: String,
    role: String,
    #[serde(default)]
    ring: Option<String>,
}

async fn invite(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<Invite>,
) -> Response {
    let back = format!("/partners/{id}#clients");
    let op = match form_operator(&s, &headers, &back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_manage(&op) {
        return notice::problem(
            &s,
            &back,
            "Nobody was invited: inviting a client needs op.partners or op.admin.",
        );
    }
    if !CHARTER_ROLES.contains(&f.role.as_str()) {
        return notice::problem(
            &s,
            &back,
            "Nobody was invited. Pick the role the owner is asked to grant.",
        );
    }
    let pool = &s.inner.pool;
    let Some(partner) = (match get_partner(pool, &id).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    }) else {
        return notice::problem(
            &s,
            "/partners",
            "No partner has that id, so nobody was invited.",
        );
    };
    let site: Option<(String, String, Option<String>)> = match sqlx::query_as(
        "SELECT site_id, domain, owner_account_id FROM sites WHERE site_id = ?1 OR domain = ?1 COLLATE NOCASE",
    )
    .bind(f.site.trim())
    .fetch_optional(pool)
    .await
    {
        Ok(x) => x,
        Err(e) => return internal(e),
    };
    let Some((site_id, domain, owner)) = site else {
        return notice::problem_keeping(
            &s,
            &back,
            format!(
                "No site has the address {}. The owner runs Assess and claims it first; then invite it again.",
                f.site.trim()
            ),
            &[("client_site", &f.site)],
        );
    };
    if owner.is_none() {
        return notice::problem_keeping(
            &s,
            &back,
            format!("Nobody has claimed {domain} yet, so there's no owner to grant the role."),
            &[("client_site", &f.site)],
        );
    }
    let ring = if f.ring.as_deref() == Some("canary") {
        "canary"
    } else {
        "stable"
    };
    let fp = grant_fingerprint(&partner.partner_id, &site_id, &f.role);
    let done = sqlx::query(
        "INSERT INTO partner_clients (partner_id, site_id, charter_role, ring, state, fingerprint, invited_by, invited_at) VALUES (?, ?, ?, ?, 'invited', ?, ?, ?) \
         ON CONFLICT (partner_id, site_id) DO UPDATE SET charter_role = excluded.charter_role, ring = excluded.ring, state = 'invited', \
           fingerprint = excluded.fingerprint, invited_by = excluded.invited_by, invited_at = excluded.invited_at, decided_by = NULL, decided_method = NULL, decided_at = NULL, revoked_at = NULL \
         WHERE partner_clients.state <> 'granted'",
    )
    .bind(&partner.partner_id)
    .bind(&site_id)
    .bind(&f.role)
    .bind(ring)
    .bind(hex::encode(fp))
    .bind(op.account.to_string())
    .bind(ledger::now())
    .execute(pool)
    .await;
    match done {
        Ok(d) if d.rows_affected() == 1 => {}
        Ok(_) => {
            return notice::problem(
                &s,
                &back,
                format!(
                    "{domain} is already {}'s client, so nothing changed.",
                    partner.name
                ),
            );
        }
        Err(e) => return internal(e),
    }
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            "partner.invite",
            &format!("site:{site_id}"),
            &format!(
                "{} asked the owner of {domain} to make {} its {}.",
                op.name, partner.name, f.role
            ),
        ),
    )
    .await
    {
        return internal(e);
    }
    notice::done(
        &s,
        &back,
        format!(
            "You asked the owner of {domain} to make {} its {}. The site joins the client list once they approve.",
            partner.name, f.role
        ),
    )
}

#[derive(Debug, Deserialize)]
struct Member {
    email: String,
    role: String,
}

async fn add_member(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<Member>,
) -> Response {
    let back = format!("/partners/{id}#team");
    let op = match form_operator(&s, &headers, &back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_manage(&op) {
        return notice::problem(
            &s,
            &back,
            "Nobody was added: adding a partner's people needs op.partners or op.admin.",
        );
    }
    if !PARTNER_ROLES.contains(&f.role.as_str()) {
        return notice::problem(&s, &back, "Nobody was added. Pick a partner role.");
    }
    let pool = &s.inner.pool;
    let Some(partner) = (match get_partner(pool, &id).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    }) else {
        return notice::problem(
            &s,
            "/partners",
            "No partner has that id, so nobody was added.",
        );
    };
    let Some(acct) = (match crate::identity::account_by_email(pool, &f.email).await {
        Ok(a) => a,
        Err(e) => return internal(e),
    }) else {
        return notice::problem_keeping(
            &s,
            &back,
            "Nobody was added: no account has that email. They register on the platform first.",
            &[("member_email", &f.email)],
        );
    };
    if let Err(e) = sqlx::query(
        "INSERT INTO partner_members (partner_id, account_id, role, added_by, added_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (partner_id, account_id, role) DO UPDATE SET removed_at = NULL, added_by = excluded.added_by, added_at = excluded.added_at",
    )
    .bind(&partner.partner_id)
    .bind(acct.id.to_string())
    .bind(&f.role)
    .bind(op.account.to_string())
    .bind(ledger::now())
    .execute(pool)
    .await
    {
        return internal(e);
    }
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            "partner.member",
            &format!("pt:{}", partner.partner_id),
            &format!(
                "{} added {} to {} as {}.",
                op.name, acct.name, partner.name, f.role
            ),
        ),
    )
    .await
    {
        return internal(e);
    }
    notice::done(
        &s,
        &back,
        format!("You added {} to {} as {}.", acct.name, partner.name, f.role),
    )
}

/// `/portfolio`: a partner person's own portfolio, in the portfolio scope. Operators go to
/// Partners; anyone else signs in.
async fn own_portfolio(State(s): State<OperatorState>, headers: HeaderMap) -> Response {
    let pool = &s.inner.pool;
    let person = match s.person_from(&headers).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    let Some((account, name, email, method)) = person else {
        return Redirect::to("/signin?next=%2Fportfolio").into_response();
    };
    let member = match memberships(pool, account).await {
        Ok(m) => m,
        Err(e) => return internal(e),
    };
    let Some((partner_id, _)) = member.first().cloned() else {
        return Redirect::to("/partners").into_response();
    };
    let roles: Vec<String> = member
        .iter()
        .filter(|(p, _)| *p == partner_id)
        .map(|(_, r)| r.clone())
        .collect();
    let partner = match get_partner(pool, &partner_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return Redirect::to("/signin").into_response(),
        Err(e) => return internal(e),
    };
    let p = match portfolio(pool, partner.clone(), Utc::now()).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    let shell = json!({
        "scope": "pf",
        "app": "clients",
        "partner": { "name": partner.name, "kind": partner.kind },
        "operator": { "name": name, "email": email, "method": method },
        "roles": roles,
        "waiting": 0,
        "lane": [],
        "open_incidents": 0,
    });
    crate::pages::page(
        &s,
        "portfolio.html",
        &json!({ "shell": shell, "p": p, "as_operator": false, "can_manage": false, "owner_link": Value::Null }),
    )
}
