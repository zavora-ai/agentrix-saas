//! Operator access to one site (08 §2.4–2.5): support-grant requests, break-glass with two
//! people, and platform holds. Every step is an operator ledger entry; every step the owner must
//! know about is a receipt on their Trust tab.
//!
//! Consequential steps (opening break-glass, signing as its second person, placing or lifting a
//! hold) follow the decision contract: the page shows "If you approve, exactly this happens",
//! the person approves that exact effect with a passkey or an authenticator code bound to its
//! fingerprint, and only then does it run, once.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentrix_types::{ReceiptBody, ReceiptKind, SiteId, Subject};
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::identity::{Operator, Role, approve};
use crate::ledger::{self, Entry, time};
use crate::notice::{self, plural};
use crate::pages::internal;
use crate::{OperatorState, holds};

/// How long break-glass stays open (08 §2.5).
pub const BREAK_GLASS_HOURS: i64 = 1;
/// A break-glass request waits this long for its second person.
pub const BREAK_GLASS_WAIT_MINUTES: i64 = 30;
/// Areas a support grant can name (08 §2.5). Payment data is never one of them.
pub const AREAS: [&str; 4] = ["pages", "releases", "jobs", "receipts"];
/// One access receipt per person and grant in this window, so a support session is one line on
/// the owner's Trust tab rather than one per click.
const ACCESS_RECEIPT_MINUTES: i64 = 15;

pub(crate) fn routes() -> Router<OperatorState> {
    Router::new()
        .route("/sites/{site_id}/support-grants", post(request_grant))
        .route("/support-grants/{grant_id}/end", post(end_grant))
        .route("/break-glass/{bg_id}/close", post(close_break_glass))
        .route("/actions/prepare", post(prepare))
        .route("/actions/{id}/approve", post(approve_action))
}

// ---------- Receipts to the owner ----------

/// Writes an owner-facing receipt on `site`. Returns its digest, or `None` when this build has no
/// receipt kind of that name yet or no receipt signer (the ledger still records the action).
pub(crate) async fn tell_owner(
    receipts: Option<&dyn agentrix_host::Receipts>,
    site: &str,
    kind: &str,
    title: String,
    detail: Value,
) -> Option<String> {
    let receipts = receipts?;
    let Some(kind) = ReceiptKind::parse(kind) else {
        tracing::warn!(
            kind,
            site,
            "no receipt kind of this name on this build; the owner receipt waits"
        );
        return None;
    };
    let now = Utc::now();
    let site_id = SiteId::new(site.to_owned());
    let mut body = ReceiptBody::new(kind, site_id.clone(), Subject::Site(site_id), now, now);
    body.detail = Some(detail);
    match receipts.issue(body.with_title(title)).await {
        Ok(digest) => Some(digest),
        Err(e) => {
            tracing::error!(error = %e, site, "writing an owner receipt failed");
            None
        }
    }
}

// ---------- Fingerprints and pending actions ----------

fn fingerprint_of(value: &Value) -> [u8; 32] {
    Sha256::digest(value.to_string().as_bytes()).into()
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

/// A consequential operator action, waiting for its approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub(crate) enum Action {
    /// Open break-glass on a site: the first of two people.
    BreakGlass {
        /// The site.
        site: String,
        /// Why, recorded and told to the owner.
        reason: String,
        /// The incident it serves, if any.
        #[serde(default)]
        incident: Option<String>,
    },
    /// Sign a waiting break-glass request as the second person.
    BreakGlassSecond {
        /// The request.
        bg_id: String,
    },
    /// Place a platform hold on a site.
    Hold {
        /// The site.
        site: String,
        /// `read-only` or `notice`.
        mode: String,
        /// Shown to the owner.
        reason: String,
    },
    /// Lift a hold.
    Lift {
        /// The hold.
        hold_id: String,
        /// Why it's lifted.
        reason: String,
    },
    /// Issue a month's invoices (08 §5.3 `money.invoice_run`, op.finance).
    InvoiceRun {
        /// `2026-11`.
        period: String,
        /// The run's digest as shown; a changed run is refused.
        digest: String,
    },
}

struct PendingAction {
    account: Uuid,
    action: Action,
    fingerprint: [u8; 32],
    started: Instant,
}

/// Prepared actions, by id, for ten minutes.
#[derive(Default)]
pub(crate) struct Pending(Mutex<HashMap<String, PendingAction>>);

impl Pending {
    const TTL: Duration = Duration::from_secs(600);

    fn put(&self, account: Uuid, action: Action, fingerprint: [u8; 32]) -> String {
        let mut bytes = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let id = hex::encode(bytes);
        let mut map = self.0.lock().expect("pending lock");
        map.retain(|_, p| p.started.elapsed() < Self::TTL);
        if map.len() < 1024 {
            map.insert(
                id.clone(),
                PendingAction {
                    account,
                    action,
                    fingerprint,
                    started: Instant::now(),
                },
            );
        }
        id
    }

    fn take(&self, id: &str, account: Uuid) -> Option<PendingAction> {
        let mut map = self.0.lock().expect("pending lock");
        let found = map.remove(id)?;
        (found.account == account && found.started.elapsed() < Self::TTL).then_some(found)
    }
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

async fn operator(s: &OperatorState, headers: &HeaderMap) -> Result<Operator, Response> {
    if !s.same_origin(headers) {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            "This request came from another site, so it was refused.",
        ));
    }
    match s.operator_from(headers).await {
        Ok(Some(op)) => Ok(op),
        Ok(None) => Err(refuse(
            StatusCode::UNAUTHORIZED,
            "Sign in again: your operator session ended.",
        )),
        Err(e) => Err(internal(e)),
    }
}

/// The signed-in operator for a form post, or where the browser goes instead: sign-in (then
/// back to `back`), or a refusal for a post from another site.
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

async fn site_domain(pool: &SqlitePool, site: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT domain FROM sites WHERE site_id = ?")
        .bind(site)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(d,)| d))
}

// ---------- Break-glass records ----------

/// A break-glass request or session.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BreakGlass {
    /// `bg_…`.
    pub bg_id: String,
    /// The site.
    pub site_id: String,
    /// Why.
    pub reason: String,
    /// The incident.
    pub incident: Option<String>,
    /// `waiting`, `open`, `closed`, `declined`, `expired`.
    pub state: String,
    /// First person.
    pub first_by: String,
    /// Their role.
    pub first_role: String,
    /// How they approved.
    pub first_method: String,
    /// When.
    pub first_at: String,
    /// Second person.
    pub second_by: Option<String>,
    /// Their role.
    pub second_role: Option<String>,
    /// How.
    pub second_method: Option<String>,
    /// When.
    pub second_at: Option<String>,
    /// Open until.
    pub ends_at: Option<String>,
    /// When the owner's receipt was written.
    pub owner_told_at: Option<String>,
    /// Closed.
    pub closed_at: Option<String>,
    /// The owner's receipt.
    pub receipt: Option<String>,
    /// Requested.
    pub created_at: String,
}

macro_rules! bg_select {
    ($rest:literal) => {
        concat!(
            "SELECT bg_id, site_id, reason, incident, state, first_by, first_role, first_method, first_at, second_by, second_role, \
             second_method, second_at, ends_at, owner_told_at, closed_at, receipt, created_at FROM break_glass ",
            $rest
        )
    };
}

/// Marks lapsed break-glass and support grants as expired.
pub(crate) async fn expire(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let now = ledger::now();
    let wait_from = time(Utc::now() - chrono::Duration::minutes(BREAK_GLASS_WAIT_MINUTES));
    sqlx::query("UPDATE break_glass SET state = 'expired' WHERE (state = 'open' AND ends_at < ?) OR (state = 'waiting' AND created_at < ?)")
        .bind(&now)
        .bind(&wait_from)
        .execute(pool)
        .await?;
    sqlx::query(
        "UPDATE support_grants SET state = 'expired' WHERE state = 'active' AND ends_at < ?",
    )
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Break-glass on a site, newest first.
pub async fn break_glass_for_site(
    pool: &SqlitePool,
    site: &str,
) -> Result<Vec<BreakGlass>, sqlx::Error> {
    sqlx::query_as(bg_select!(
        "WHERE site_id = ? ORDER BY created_at DESC LIMIT 10"
    ))
    .bind(site)
    .fetch_all(pool)
    .await
}

/// Requests waiting for a second person.
pub async fn break_glass_waiting(pool: &SqlitePool) -> Result<Vec<BreakGlass>, sqlx::Error> {
    sqlx::query_as(bg_select!(
        "WHERE state = 'waiting' ORDER BY created_at DESC"
    ))
    .fetch_all(pool)
    .await
}

/// The role a second person needs, given the first's: op.security and a director (08 §5.3).
fn second_role_for(first_role: &str) -> Role {
    if first_role == Role::Security.as_str() {
        Role::Director
    } else {
        Role::Security
    }
}

/// Whether `op` can sign `bg` as its second person.
pub(crate) fn can_second(op: &Operator, bg: &BreakGlass) -> bool {
    bg.state == "waiting"
        && bg.first_by != op.account.to_string()
        && op.has(second_role_for(&bg.first_role))
}

// ---------- Support grants ----------

/// A support grant.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Grant {
    /// `grant_…`.
    pub grant_id: String,
    /// The site.
    pub site_id: String,
    /// Who asked (account id).
    pub requested_by: String,
    /// JSON array of account ids.
    pub grantees: String,
    /// JSON array of areas.
    pub areas: String,
    /// `read` or `read-write`.
    pub access: String,
    /// How long, once granted.
    pub hours: i64,
    /// Why.
    pub reason: String,
    /// `requested`, `active`, `declined`, `expired`, `revoked`, `withdrawn`.
    pub state: String,
    /// The fingerprint the owner approves, hex.
    pub fingerprint: String,
    /// Who decided (the owner).
    pub decided_by: Option<String>,
    /// How.
    pub decided_method: Option<String>,
    /// When.
    pub decided_at: Option<String>,
    /// From.
    pub starts_at: Option<String>,
    /// Until.
    pub ends_at: Option<String>,
    /// The owner's receipt.
    pub receipt: Option<String>,
    /// Asked.
    pub created_at: String,
}

impl Grant {
    /// The areas as a list.
    pub fn area_list(&self) -> Vec<String> {
        serde_json::from_str(&self.areas).unwrap_or_default()
    }

    /// Whether `account` is named on it.
    pub fn names(&self, account: Uuid) -> bool {
        serde_json::from_str::<Vec<String>>(&self.grantees)
            .unwrap_or_default()
            .contains(&account.to_string())
    }
}

macro_rules! grant_select {
    ($rest:literal) => {
        concat!(
            "SELECT grant_id, site_id, requested_by, grantees, areas, access, hours, reason, state, fingerprint, decided_by, \
             decided_method, decided_at, starts_at, ends_at, receipt, created_at FROM support_grants ",
            $rest
        )
    };
}

/// A site's grants, newest first.
pub async fn grants_for_site(pool: &SqlitePool, site: &str) -> Result<Vec<Grant>, sqlx::Error> {
    sqlx::query_as(grant_select!(
        "WHERE site_id = ? ORDER BY created_at DESC LIMIT 20"
    ))
    .bind(site)
    .fetch_all(pool)
    .await
}

/// One grant.
pub async fn grant(pool: &SqlitePool, grant_id: &str) -> Result<Option<Grant>, sqlx::Error> {
    sqlx::query_as(grant_select!("WHERE grant_id = ?"))
        .bind(grant_id)
        .fetch_optional(pool)
        .await
}

/// Grants waiting for or active on any of these sites.
pub async fn open_grants_for_owner(
    pool: &SqlitePool,
    owner: Uuid,
) -> Result<Vec<Grant>, sqlx::Error> {
    sqlx::query_as(grant_select!(
        "WHERE state IN ('requested', 'active') AND site_id IN (SELECT site_id FROM sites WHERE owner_account_id = ?) ORDER BY created_at DESC"
    ))
    .bind(owner.to_string())
    .fetch_all(pool)
    .await
}

/// The fingerprint an owner approves for a grant: its exact terms.
pub fn grant_fingerprint(
    grant_id: &str,
    site: &str,
    grantees: &[String],
    areas: &[String],
    access: &str,
    hours: i64,
    reason: &str,
) -> [u8; 32] {
    fingerprint_of(&json!({
        "v": 1,
        "kind": "support-grant",
        "grant": grant_id,
        "site": site,
        "grantees": grantees,
        "areas": areas,
        "access": access,
        "hours": hours,
        "reason": reason,
    }))
}

#[derive(Debug, Deserialize)]
struct GrantForm {
    #[serde(default)]
    area_pages: Option<String>,
    #[serde(default)]
    area_releases: Option<String>,
    #[serde(default)]
    area_jobs: Option<String>,
    #[serde(default)]
    area_receipts: Option<String>,
    access: String,
    hours: i64,
    reason: String,
    #[serde(default)]
    ticket: Option<String>,
}

async fn request_grant(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(site): Path<String>,
    Form(f): Form<GrantForm>,
) -> Response {
    let back = format!("/sites/{site}#access");
    let op = match form_operator(&s, &headers, &back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !(op.has(Role::Support) || op.has(Role::Sre) || op.has(Role::Admin)) {
        return notice::problem(
            &s,
            &back,
            "The owner wasn't asked: asking for support access needs op.support, op.sre or op.admin.",
        );
    }
    let pool = &s.inner.pool;
    let domain = match site_domain(pool, &site).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return notice::problem(
                &s,
                "/customers?show=all",
                "No site has that id, so no owner was asked. Find the site on the full list.",
            );
        }
        Err(e) => return internal(e),
    };
    let areas: Vec<String> = [
        ("pages", &f.area_pages),
        ("releases", &f.area_releases),
        ("jobs", &f.area_jobs),
        ("receipts", &f.area_receipts),
    ]
    .iter()
    .filter(|(_, v)| v.is_some())
    .map(|(a, _)| (*a).to_owned())
    .collect();
    let reason = f.reason.trim();
    if areas.is_empty()
        || reason.is_empty()
        || reason.len() > 500
        || !(1..=72).contains(&f.hours)
        || !["read", "read-write"].contains(&f.access.as_str())
    {
        return notice::problem_keeping(
            &s,
            &back,
            "The owner wasn't asked. Pick at least one area and 1 to 72 hours, and say why in a sentence.",
            &[("grant_reason", &f.reason)],
        );
    }
    let grant_id = new_id("grant");
    let grantees = vec![op.account.to_string()];
    let fp = grant_fingerprint(
        &grant_id, &site, &grantees, &areas, &f.access, f.hours, reason,
    );
    let inserted = sqlx::query(
        "INSERT INTO support_grants (grant_id, site_id, requested_by, grantees, areas, access, hours, reason, ticket, fingerprint, state, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'requested', ?)",
    )
    .bind(&grant_id)
    .bind(&site)
    .bind(op.account.to_string())
    .bind(json!(grantees).to_string())
    .bind(json!(areas).to_string())
    .bind(&f.access)
    .bind(f.hours)
    .bind(reason)
    .bind(f.ticket.as_deref().map(str::trim).filter(|t| !t.is_empty()))
    .bind(hex::encode(fp))
    .bind(ledger::now())
    .execute(pool)
    .await;
    if let Err(e) = inserted {
        return internal(e);
    }
    let words = if f.access == "read" {
        "read"
    } else {
        "read and change"
    };
    let entry = Entry::person(
        op.account,
        &op.name,
        "grant.request",
        &format!("site:{site}"),
        &format!(
            "{} asked the owner of {domain} for {words} access to {} for {}.",
            op.name,
            areas.join(", "),
            plural(f.hours, "hour", "hours")
        ),
    )
    .detail(json!({ "grant": grant_id, "areas": areas, "access": f.access, "hours": f.hours }));
    if let Err(e) = ledger::append(pool, entry).await {
        return internal(e);
    }
    notice::done(
        &s,
        &back,
        format!(
            "You asked the owner of {domain} for {words} access to {} for {}. Nothing opens until they approve it.",
            areas.join(", "),
            plural(f.hours, "hour", "hours")
        ),
    )
}

async fn end_grant(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(grant_id): Path<String>,
) -> Response {
    let op = match form_operator(&s, &headers, "/customers").await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let g = match grant(pool, &grant_id).await {
        Ok(Some(g)) => g,
        Ok(None) => {
            return notice::problem(
                &s,
                "/customers",
                "No support grant has that id, so nothing changed.",
            );
        }
        Err(e) => return internal(e),
    };
    let back = format!("/sites/{}#access", g.site_id);
    let new_state = match g.state.as_str() {
        "requested" => "withdrawn",
        "active" => "revoked",
        _ => {
            return notice::problem(
                &s,
                &back,
                format!(
                    "This grant had already ended ({}), so nothing changed.",
                    g.state
                ),
            );
        }
    };
    if g.requested_by != op.account.to_string() && !op.has(Role::Admin) {
        return notice::problem(
            &s,
            &back,
            "Nothing changed: only the person who asked, or an op.admin, can end this grant.",
        );
    }
    let done = sqlx::query("UPDATE support_grants SET state = ?, revoked_by = ?, revoked_at = ? WHERE grant_id = ? AND state = ?")
        .bind(new_state)
        .bind(op.account.to_string())
        .bind(ledger::now())
        .bind(&grant_id)
        .bind(&g.state)
        .execute(pool)
        .await;
    if let Err(e) = done {
        return internal(e);
    }
    let summary = if new_state == "withdrawn" {
        format!("{} withdrew the request for support access.", op.name)
    } else {
        format!("{} ended support access before it ran out.", op.name)
    };
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            "grant.end",
            &format!("site:{}", g.site_id),
            &summary,
        )
        .detail(json!({ "grant": grant_id })),
    )
    .await
    {
        return internal(e);
    }
    let told = new_state == "revoked"
        && tell_owner(
            s.inner.receipts.as_deref(),
            &g.site_id,
            "operator.access",
            "Zavora Support ended its access to your site early".to_owned(),
            json!({ "verb": "support-access-ended", "grant": grant_id, "by": op.account }),
        )
        .await
        .is_some();
    notice::done(
        &s,
        &back,
        match (new_state, told) {
            ("withdrawn", _) => "You withdrew the request. The owner can no longer approve it.",
            (_, true) => "Support access ended now. The owner has a receipt saying so.",
            (_, false) => {
                "Support access ended now. No owner receipt was written; the operator ledger has it."
            }
        },
    )
}

// ---------- What an operator may see under a grant or break-glass ----------

/// Site content an operator may read now, and under what.
#[derive(Debug, Clone, Serialize)]
pub struct SupportView {
    /// `grant` or `break-glass`.
    pub under: &'static str,
    /// The grant or break-glass id.
    pub id: String,
    /// Until.
    pub ends_at: String,
    /// The areas open.
    pub areas: Vec<String>,
    /// Receipt titles (area `receipts`).
    pub receipts: Vec<(String, String, String)>,
    /// Job results, trimmed (area `jobs`).
    pub jobs: Vec<(String, String, String)>,
    /// Move step summaries (area `releases`).
    pub steps: Vec<(String, String, String)>,
    /// Findings and items left behind (area `pages`).
    pub pages: Vec<(String, String, String)>,
}

/// What `op` may read on `site` right now, recording the access. `None` without a live grant
/// naming them or an open break-glass they signed.
pub async fn support_view(
    s: &OperatorState,
    op: &Operator,
    site: &str,
) -> Result<Option<SupportView>, sqlx::Error> {
    let pool = &s.inner.pool;
    expire(pool).await?;
    let mut found: Option<(&'static str, String, String, Vec<String>)> = None;
    for g in grants_for_site(pool, site).await? {
        if g.state == "active" && g.names(op.account) {
            found = Some((
                "grant",
                g.grant_id.clone(),
                g.ends_at.clone().unwrap_or_default(),
                g.area_list(),
            ));
            break;
        }
    }
    if found.is_none() {
        for bg in break_glass_for_site(pool, site).await? {
            let me = op.account.to_string();
            if bg.state == "open"
                && (bg.first_by == me || bg.second_by.as_deref() == Some(me.as_str()))
            {
                found = Some((
                    "break-glass",
                    bg.bg_id.clone(),
                    bg.ends_at.clone().unwrap_or_default(),
                    AREAS.iter().map(|a| (*a).to_owned()).collect(),
                ));
                break;
            }
        }
    }
    let Some((under, id, ends_at, areas)) = found else {
        return Ok(None);
    };
    let has = |a: &str| areas.iter().any(|x| x == a);
    let mut view = SupportView {
        under,
        id: id.clone(),
        ends_at,
        areas: areas.clone(),
        receipts: vec![],
        jobs: vec![],
        steps: vec![],
        pages: vec![],
    };
    if has("receipts") {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT issued_at, kind, body FROM receipts WHERE site_id = ? ORDER BY issued_at DESC LIMIT 15").bind(site).fetch_all(pool).await?;
        view.receipts = rows
            .into_iter()
            .map(|(at, kind, body)| {
                let title = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|b| {
                        b.pointer("/detail/title")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                (at, kind, title)
            })
            .collect();
    }
    if has("jobs") {
        let rows: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT created_at, kind, result FROM jobs WHERE site_id = ? ORDER BY created_at DESC LIMIT 8").bind(site).fetch_all(pool).await?;
        view.jobs = rows
            .into_iter()
            .map(|(at, kind, result)| {
                let mut text = result.unwrap_or_default();
                if text.len() > 600 {
                    let cut = (0..=600)
                        .rev()
                        .find(|i| text.is_char_boundary(*i))
                        .unwrap_or(0);
                    text.truncate(cut);
                    text.push('…');
                }
                (at, kind, text)
            })
            .collect();
    }
    if has("releases") {
        view.steps = sqlx::query_as("SELECT started_at, step, summary FROM move_steps WHERE site_id = ? AND summary <> '' ORDER BY started_at DESC LIMIT 15")
            .bind(site)
            .fetch_all(pool)
            .await?;
    }
    if has("pages") {
        let mut rows: Vec<(String, String, String)> = sqlx::query_as("SELECT location, severity, title FROM findings WHERE site_id = ? ORDER BY at DESC LIMIT 15")
            .bind(site)
            .fetch_all(pool)
            .await?;
        let left: Vec<(String, String, String)> = sqlx::query_as("SELECT location, 'left behind', title || ': ' || reason FROM quarantine WHERE site_id = ? ORDER BY created_at DESC LIMIT 10")
            .bind(site)
            .fetch_all(pool)
            .await?;
        rows.extend(left);
        view.pages = rows;
    }
    record_access(s, op, site, under, &id, &areas).await?;
    Ok(Some(view))
}

async fn record_access(
    s: &OperatorState,
    op: &Operator,
    site: &str,
    under: &str,
    id: &str,
    areas: &[String],
) -> Result<(), sqlx::Error> {
    let pool = &s.inner.pool;
    let since = time(Utc::now() - chrono::Duration::minutes(ACCESS_RECEIPT_MINUTES));
    let (recent,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM support_access WHERE grant_id = ? AND account_id = ? AND at >= ?",
    )
    .bind(id)
    .bind(op.account.to_string())
    .bind(&since)
    .fetch_one(pool)
    .await?;
    let mut receipt = None;
    if recent == 0 {
        let domain = site_domain(pool, site).await?.unwrap_or_default();
        let how = if under == "grant" {
            "under your support grant"
        } else {
            "under emergency access"
        };
        receipt = tell_owner(
            s.inner.receipts.as_deref(),
            site,
            "operator.access",
            format!("{} of Zavora read your site's {} {how}", op.name, areas.join(", ")),
            json!({ "verb": "support-access", "under": under, "id": id, "areas": areas, "by": op.account }),
        )
        .await;
        ledger::append(
            pool,
            Entry::person(
                op.account,
                &op.name,
                "support.access",
                &format!("site:{site}"),
                &format!(
                    "{} read {} on {domain} {}.",
                    op.name,
                    areas.join(", "),
                    if under == "grant" {
                        "under a support grant"
                    } else {
                        "under break-glass"
                    }
                ),
            )
            .detail(json!({ "under": under, "id": id, "areas": areas, "receipt": receipt })),
        )
        .await?;
    }
    let now = ledger::now();
    for area in areas {
        sqlx::query("INSERT INTO support_access (grant_id, site_id, account_id, area, at, receipt) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(id)
            .bind(site)
            .bind(op.account.to_string())
            .bind(area)
            .bind(&now)
            .bind(&receipt)
            .execute(pool)
            .await?;
    }
    Ok(())
}

// ---------- Prepare and approve ----------

#[derive(Debug, Serialize)]
struct Effect {
    action: Action,
    domain: String,
    site: String,
    operator: String,
    hours: i64,
    ends_words: String,
    first_name: Option<String>,
    first_role: Option<String>,
    second_role: String,
    holds_enforced: bool,
    run: Option<agentrix_billing::Run>,
}

async fn account_name(pool: &SqlitePool, id: &str) -> String {
    let Ok(uuid) = Uuid::parse_str(id) else {
        return id.to_owned();
    };
    crate::identity::account(pool, uuid)
        .await
        .ok()
        .flatten()
        .map(|a| a.name)
        .unwrap_or_else(|| id.to_owned())
}

/// Checks `action` for `op` and builds what the card shows.
async fn check(s: &OperatorState, op: &Operator, action: &Action) -> Result<Effect, Response> {
    let pool = &s.inner.pool;
    expire(pool).await.map_err(internal)?;
    let tz = chrono::FixedOffset::east_opt(s.inner.config.utc_offset_minutes * 60).expect("zone");
    let ends = |hours: i64| {
        (Utc::now() + chrono::Duration::hours(hours))
            .with_timezone(&tz)
            .format("%H:%M")
            .to_string()
    };
    let effect = |site: &str, domain: String, hours: i64| Effect {
        action: action.clone(),
        domain,
        site: site.to_owned(),
        operator: op.name.clone(),
        hours,
        ends_words: ends(hours),
        first_name: None,
        first_role: None,
        second_role: String::new(),
        holds_enforced: s.inner.config.holds_enforced,
        run: None,
    };
    let domain_of = |site: String| async move {
        match site_domain(pool, &site).await {
            Ok(Some(d)) => Ok(d),
            Ok(None) => Err(refuse(StatusCode::NOT_FOUND, "No site has that id.")),
            Err(e) => Err(internal(e)),
        }
    };
    match action {
        Action::BreakGlass { site, reason, .. } => {
            if !(op.has(Role::Security) || op.has(Role::Director)) {
                return Err(refuse(
                    StatusCode::FORBIDDEN,
                    "Break-glass needs op.security or a director to start it.",
                ));
            }
            if reason.trim().is_empty() || reason.len() > 500 {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "Say why in a sentence: the owner reads it.",
                ));
            }
            let domain = domain_of(site.clone()).await?;
            let first_role = if op.has(Role::Security) {
                Role::Security
            } else {
                Role::Director
            };
            let mut e = effect(site, domain, BREAK_GLASS_HOURS);
            e.second_role = second_role_for(first_role.as_str()).as_str().to_owned();
            Ok(e)
        }
        Action::BreakGlassSecond { bg_id } => {
            let bg: Option<BreakGlass> = sqlx::query_as(bg_select!("WHERE bg_id = ?"))
                .bind(bg_id)
                .fetch_optional(pool)
                .await
                .map_err(internal)?;
            let Some(bg) = bg else {
                return Err(refuse(
                    StatusCode::NOT_FOUND,
                    "No break-glass request has that id.",
                ));
            };
            if bg.state != "waiting" {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "This request isn't waiting any more. Reload the page.",
                ));
            }
            if !can_second(op, &bg) {
                return Err(refuse(
                    StatusCode::FORBIDDEN,
                    "The second signer must be a different person with the other role: break-glass needs one op.security and one director.",
                ));
            }
            let domain = domain_of(bg.site_id.clone()).await?;
            let mut e = effect(&bg.site_id, domain, BREAK_GLASS_HOURS);
            e.first_name = Some(account_name(pool, &bg.first_by).await);
            e.first_role = Some(bg.first_role.clone());
            e.second_role = second_role_for(&bg.first_role).as_str().to_owned();
            Ok(e)
        }
        Action::Hold { site, mode, reason } => {
            if !(op.has(Role::Security) || op.has(Role::Sre)) {
                return Err(refuse(
                    StatusCode::FORBIDDEN,
                    "A platform hold needs op.security or op.sre.",
                ));
            }
            if holds::Mode::parse(mode).is_none() || reason.trim().is_empty() || reason.len() > 500
            {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "Pick how the site is served and give the owner a reason.",
                ));
            }
            if holds::active(pool, site)
                .await
                .map_err(internal)?
                .is_some_and(|h| h.scope == "site")
            {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "This site is already on hold.",
                ));
            }
            let domain = domain_of(site.clone()).await?;
            Ok(effect(site, domain, 0))
        }
        Action::Lift { hold_id, reason } => {
            if !(op.has(Role::Security) || op.has(Role::Sre) || op.has(Role::Admin)) {
                return Err(refuse(
                    StatusCode::FORBIDDEN,
                    "Lifting a hold needs op.security, op.sre or op.admin.",
                ));
            }
            let Some(h) = holds::get(pool, hold_id).await.map_err(internal)? else {
                return Err(refuse(StatusCode::NOT_FOUND, "No hold has that id."));
            };
            if h.lifted_at.is_some() {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "This hold was already lifted.",
                ));
            }
            if reason.trim().is_empty() || reason.len() > 500 {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "Say why it's safe to lift.",
                ));
            }
            let domain = domain_of(h.target.clone()).await?;
            Ok(effect(&h.target, domain, 0))
        }
        Action::InvoiceRun { period, digest } => {
            if !op.has(Role::Finance) {
                return Err(refuse(
                    StatusCode::FORBIDDEN,
                    "Issuing invoices needs op.finance.",
                ));
            }
            let run = agentrix_billing::Billing::new(pool.clone())
                .draft_run(period)
                .await
                .map_err(internal)?;
            if run.issued || run.digest != *digest {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "The invoice run changed since this page loaded. Reload to see the new run.",
                ));
            }
            if run.drafts.is_empty() {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "There's nothing to invoice for this month.",
                ));
            }
            let mut e = effect("", String::new(), 0);
            e.run = Some(run);
            Ok(e)
        }
    }
}

async fn prepare(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Json(action): Json<Action>,
) -> Response {
    let op = match operator(&s, &headers).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let effect = match check(&s, &op, &action).await {
        Ok(e) => e,
        Err(r) => return r,
    };
    let mut nonce = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let fp = fingerprint_of(
        &json!({ "v": 1, "op": s.inner.config.origin.as_str(), "by": op.account, "action": action, "nonce": hex::encode(nonce) }),
    );
    let html = match s.inner.render.page("effect.html", &json!({ "e": effect })) {
        Ok(h) => h,
        Err(e) => return internal(e),
    };
    let options = match s.inner.approvals.passkey_options(op.account, fp).await {
        Ok(o) => o,
        Err(e) => return internal(e),
    };
    let id = s.inner.actions.put(op.account, action, fp);
    Json(json!({ "id": id, "effect": html, "method": if options.is_some() { "passkey" } else { "totp" }, "options": options, "fingerprint": hex::encode(fp) })).into_response()
}

#[derive(Debug, Deserialize)]
struct ApproveBody {
    proof: Value,
}

async fn approve_action(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ApproveBody>,
) -> Response {
    let op = match operator(&s, &headers).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let Some(pending) = s.inner.actions.take(&id, op.account) else {
        return refuse(
            StatusCode::GONE,
            "This approval expired. Start again; nothing was changed.",
        );
    };
    // Re-check: roles or state may have changed since the card was shown.
    if let Err(r) = check(&s, &op, &pending.action).await {
        return r;
    }
    let proof = match approve(
        s.inner.approvals.as_ref(),
        op.account,
        pending.fingerprint,
        &body.proof,
    )
    .await
    {
        Ok(p) => p,
        Err(words) => return refuse(StatusCode::UNAUTHORIZED, &words),
    };
    let approval = json!([{ "account": op.account, "role": null, "method": proof.method, "fingerprint": hex::encode(pending.fingerprint), "proof": proof.evidence }]);
    let result = run(
        &s,
        &op,
        pending.action,
        proof.method,
        pending.fingerprint,
        approval,
    )
    .await;
    match result {
        Ok((next, words)) => {
            Json(json!({ "next": notice::link(&s, &next, notice::Tone::Done, words) }))
                .into_response()
        }
        Err(r) => r,
    }
}

async fn run(
    s: &OperatorState,
    op: &Operator,
    action: Action,
    method: &str,
    fp: [u8; 32],
    approval: Value,
) -> Result<(String, String), Response> {
    let pool = &s.inner.pool;
    let now = Utc::now();
    let receipts = s.inner.receipts.as_deref();
    match action {
        Action::BreakGlass {
            site,
            reason,
            incident,
        } => {
            let bg_id = new_id("bg");
            let role = if op.has(Role::Security) {
                Role::Security
            } else {
                Role::Director
            };
            sqlx::query(
                "INSERT INTO break_glass (bg_id, site_id, reason, incident, fingerprint, state, first_by, first_role, first_method, first_at, created_at) \
                 VALUES (?, ?, ?, ?, ?, 'waiting', ?, ?, ?, ?, ?)",
            )
            .bind(&bg_id)
            .bind(&site)
            .bind(reason.trim())
            .bind(incident.as_deref().map(str::trim).filter(|i| !i.is_empty()))
            .bind(hex::encode(fp))
            .bind(op.account.to_string())
            .bind(role.as_str())
            .bind(method)
            .bind(time(now))
            .bind(time(now))
            .execute(pool)
            .await
            .map_err(internal)?;
            let domain = site_domain(pool, &site)
                .await
                .map_err(internal)?
                .unwrap_or_default();
            ledger::append(
                pool,
                Entry::person(
                    op.account,
                    &op.name,
                    "breakglass.request",
                    &format!("site:{site}"),
                    &format!(
                        "{} asked for break-glass on {domain} as {}; it waits for someone else who holds {}. Reason: {}",
                        op.name,
                        role.as_str(),
                        second_role_for(role.as_str()).as_str(),
                        reason.trim()
                    ),
                )
                .approval(approval)
                .detail(json!({ "bg": bg_id, "incident": incident })),
            )
            .await
            .map_err(internal)?;
            Ok((
                format!("/sites/{site}#access"),
                format!(
                    "Break-glass on {domain} waits for someone else who holds {}. It lapses in {} if nobody signs.",
                    second_role_for(role.as_str()).as_str(),
                    plural(BREAK_GLASS_WAIT_MINUTES, "minute", "minutes")
                ),
            ))
        }
        Action::BreakGlassSecond { bg_id } => {
            let bg: BreakGlass = sqlx::query_as(bg_select!("WHERE bg_id = ?"))
                .bind(&bg_id)
                .fetch_one(pool)
                .await
                .map_err(internal)?;
            let role = second_role_for(&bg.first_role);
            let ends = now + chrono::Duration::hours(BREAK_GLASS_HOURS);
            let done = sqlx::query(
                "UPDATE break_glass SET state = 'open', second_by = ?, second_role = ?, second_method = ?, second_at = ?, starts_at = ?, ends_at = ?, owner_told_at = ? \
                 WHERE bg_id = ? AND state = 'waiting' AND first_by <> ?",
            )
            .bind(op.account.to_string())
            .bind(role.as_str())
            .bind(method)
            .bind(time(now))
            .bind(time(now))
            .bind(time(ends))
            .bind(time(now))
            .bind(&bg_id)
            .bind(op.account.to_string())
            .execute(pool)
            .await
            .map_err(internal)?;
            if done.rows_affected() == 0 {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "This request isn't waiting any more. Reload the page.",
                ));
            }
            let first = account_name(pool, &bg.first_by).await;
            let receipt = tell_owner(
                receipts,
                &bg.site_id,
                "operator.access",
                format!("Zavora opened emergency read-only access to your site for {}. Reason: {}", plural(BREAK_GLASS_HOURS, "hour", "hours"), bg.reason),
                json!({ "verb": "break-glass-opened", "bg": bg_id, "approvers": [{ "account": bg.first_by, "role": bg.first_role, "method": bg.first_method }, { "account": op.account, "role": role.as_str(), "method": method }], "ends_at": time(ends) }),
            )
            .await;
            if let Some(r) = &receipt {
                sqlx::query("UPDATE break_glass SET receipt = ? WHERE bg_id = ?")
                    .bind(r)
                    .bind(&bg_id)
                    .execute(pool)
                    .await
                    .map_err(internal)?;
            }
            let domain = site_domain(pool, &bg.site_id)
                .await
                .map_err(internal)?
                .unwrap_or_default();
            ledger::append(
                pool,
                Entry::person(op.account, &op.name, "breakglass.open", &format!("site:{}", bg.site_id), &format!("{} signed as {} after {first}; break-glass on {domain} is open for {}, read only. {}", op.name, role.as_str(), plural(BREAK_GLASS_HOURS, "hour", "hours"), if receipt.is_some() { "The owner was told." } else { "No owner receipt was written." }))
                    .approval(approval)
                    .detail(json!({ "bg": bg_id, "receipt": receipt })),
            )
            .await
            .map_err(internal)?;
            let tz = chrono::FixedOffset::east_opt(s.inner.config.utc_offset_minutes * 60)
                .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("UTC"));
            Ok((
                format!("/sites/{}#access", bg.site_id),
                format!(
                    "Break-glass on {domain} is open until {}, read only. {}",
                    ends.with_timezone(&tz).format("%H:%M"),
                    if receipt.is_some() {
                        "The owner has a receipt."
                    } else {
                        "No owner receipt was written; the operator ledger has it."
                    }
                ),
            ))
        }
        Action::Hold { site, mode, reason } => {
            let hold_id = new_id("hold");
            sqlx::query(
                "INSERT INTO platform_holds (hold_id, scope, target, mode, reason, fingerprint, placed_by, placed_method, placed_at) VALUES (?, 'site', ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&hold_id)
            .bind(&site)
            .bind(&mode)
            .bind(reason.trim())
            .bind(hex::encode(fp))
            .bind(op.account.to_string())
            .bind(method)
            .bind(time(now))
            .execute(pool)
            .await
            .map_err(internal)?;
            let enforced = s.inner.config.holds_enforced;
            let receipt = tell_owner(
                receipts,
                &site,
                "platform.hold",
                format!(
                    "Zavora placed a platform hold on your site. {} Reason: {}",
                    hold_words(&mode, enforced),
                    reason.trim()
                ),
                json!({ "verb": "placed", "hold": hold_id, "mode": mode, "by": op.account, "method": method }),
            )
            .await;
            if let Some(r) = &receipt {
                sqlx::query("UPDATE platform_holds SET receipt = ? WHERE hold_id = ?")
                    .bind(r)
                    .bind(&hold_id)
                    .execute(pool)
                    .await
                    .map_err(internal)?;
            }
            let domain = site_domain(pool, &site)
                .await
                .map_err(internal)?
                .unwrap_or_default();
            ledger::append(
                pool,
                Entry::person(
                    op.account,
                    &op.name,
                    "hold.place",
                    &format!("site:{site}"),
                    &format!(
                        "{} placed a {mode} platform hold on {domain}.{} Reason: {}",
                        op.name,
                        if enforced {
                            ""
                        } else {
                            " Recorded only: the edge doesn't enforce holds on this build."
                        },
                        reason.trim()
                    ),
                )
                .approval(approval)
                .detail(json!({ "hold": hold_id, "receipt": receipt, "enforced": enforced })),
            )
            .await
            .map_err(internal)?;
            Ok((
                format!("/sites/{site}#access"),
                if enforced {
                    format!(
                        "{domain} is on platform hold. {}",
                        if mode == "notice" {
                            "Visitors see a neutral notice page instead of the site."
                        } else {
                            "Visitors can still read it; Ask, forms and the agent door are paused."
                        }
                    )
                } else {
                    format!(
                        "The hold on {domain} is recorded{}. The edge doesn't enforce holds on this build, so visitors still see the site as usual.",
                        if receipt.is_some() {
                            " and the owner has a receipt"
                        } else {
                            ""
                        }
                    )
                },
            ))
        }
        Action::InvoiceRun { period, .. } => {
            let billing = agentrix_billing::Billing::new(pool.clone());
            let draft = billing.draft_run(&period).await.map_err(internal)?;
            let n = match billing
                .issue_run(&draft, &op.account.to_string(), method)
                .await
            {
                Ok(n) => n,
                Err(agentrix_billing::Error::RunChanged) => {
                    return Err(refuse(
                        StatusCode::CONFLICT,
                        "The invoice run changed since you approved it. Nothing was issued; reload and approve the new run.",
                    ));
                }
                Err(e) => return Err(internal(e)),
            };
            let total = crate::render::usd(draft.total_cents * 10_000);
            ledger::append(
                pool,
                Entry::person(
                    op.account,
                    &op.name,
                    "money.invoice_run",
                    "fleet",
                    &format!("{} issued {n} invoice{} for {period}, {total} in all.", op.name, if n == 1 { "" } else { "s" }),
                )
                .approval(approval)
                .detail(json!({ "period": period, "digest": draft.digest, "invoices": n, "total_cents": draft.total_cents, "fingerprint": hex::encode(fp) })),
            )
            .await
            .map_err(internal)?;
            Ok((
                "/money".to_owned(),
                format!(
                    "You issued {} for {period}, {total} in all.",
                    plural(n as i64, "invoice", "invoices")
                ),
            ))
        }
        Action::Lift { hold_id, reason } => {
            let h = holds::get(pool, &hold_id)
                .await
                .map_err(internal)?
                .ok_or_else(|| refuse(StatusCode::NOT_FOUND, "No hold has that id."))?;
            let done = sqlx::query("UPDATE platform_holds SET lifted_by = ?, lifted_at = ?, lift_reason = ? WHERE hold_id = ? AND lifted_at IS NULL")
                .bind(op.account.to_string())
                .bind(time(now))
                .bind(reason.trim())
                .bind(&hold_id)
                .execute(pool)
                .await
                .map_err(internal)?;
            if done.rows_affected() == 0 {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "This hold was already lifted.",
                ));
            }
            let enforced = s.inner.config.holds_enforced;
            tell_owner(
                receipts,
                &h.target,
                "platform.hold",
                if enforced {
                    "Zavora lifted the platform hold on your site. Visitors see it as before."
                } else {
                    "Zavora lifted the platform hold on your site. Agentrix only recorded it, so nothing changes for visitors."
                }
                .to_owned(),
                json!({ "verb": "lifted", "hold": hold_id, "by": op.account, "method": method }),
            )
            .await;
            let domain = site_domain(pool, &h.target)
                .await
                .map_err(internal)?
                .unwrap_or_default();
            ledger::append(
                pool,
                Entry::person(
                    op.account,
                    &op.name,
                    "hold.lift",
                    &format!("site:{}", h.target),
                    &format!(
                        "{} lifted the platform hold on {domain}. Reason: {}",
                        op.name,
                        reason.trim()
                    ),
                )
                .approval(approval)
                .detail(json!({ "hold": hold_id })),
            )
            .await
            .map_err(internal)?;
            Ok((
                format!("/sites/{}#access", h.target),
                format!("The platform hold on {domain} is lifted."),
            ))
        }
    }
}

/// What a hold does to visitors, for the owner's receipt: what this build actually does.
fn hold_words(mode: &str, enforced: bool) -> String {
    let asked = if mode == "notice" {
        "show visitors a notice page instead of your site"
    } else {
        "pause answers and forms while visitors can still read your site"
    };
    match (enforced, mode) {
        (true, "notice") => "Visitors see a neutral notice page instead of your site.".to_owned(),
        (true, _) => "Visitors can still read your site; answers and forms are paused.".to_owned(),
        (false, _) => format!(
            "The hold would {asked}, but for now Agentrix only records holds, so visitors still see your site as usual."
        ),
    }
}

async fn close_break_glass(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(bg_id): Path<String>,
) -> Response {
    let op = match form_operator(&s, &headers, "/customers").await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let bg: Option<BreakGlass> = match sqlx::query_as(bg_select!("WHERE bg_id = ?"))
        .bind(&bg_id)
        .fetch_optional(pool)
        .await
    {
        Ok(b) => b,
        Err(e) => return internal(e),
    };
    let Some(bg) = bg else {
        return notice::problem(
            &s,
            "/customers",
            "No break-glass has that id, so nothing changed.",
        );
    };
    let back = format!("/sites/{}#access", bg.site_id);
    let me = op.account.to_string();
    if !(bg.first_by == me
        || bg.second_by.as_deref() == Some(me.as_str())
        || op.has(Role::Security)
        || op.has(Role::Director))
    {
        return notice::problem(
            &s,
            &back,
            "Nothing changed: only its two approvers, op.security or a director can close break-glass.",
        );
    }
    if !["waiting", "open"].contains(&bg.state.as_str()) {
        return notice::problem(
            &s,
            &back,
            format!(
                "This break-glass had already ended ({}), so nothing changed.",
                bg.state
            ),
        );
    }
    let was_open = bg.state == "open";
    if let Err(e) = sqlx::query("UPDATE break_glass SET state = 'closed', closed_by = ?, closed_at = ? WHERE bg_id = ? AND state IN ('waiting', 'open')")
        .bind(&me)
        .bind(ledger::now())
        .bind(&bg_id)
        .execute(pool)
        .await
    {
        return internal(e);
    }
    let told = was_open
        && tell_owner(
            s.inner.receipts.as_deref(),
            &bg.site_id,
            "operator.access",
            "Zavora closed its emergency access to your site".to_owned(),
            json!({ "verb": "break-glass-closed", "bg": bg_id, "by": op.account }),
        )
        .await
        .is_some();
    let words = if was_open {
        "closed break-glass"
    } else {
        "withdrew the break-glass request"
    };
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            "breakglass.close",
            &format!("site:{}", bg.site_id),
            &format!("{} {words}.", op.name),
        )
        .detail(json!({ "bg": bg_id })),
    )
    .await
    {
        return internal(e);
    }
    notice::done(
        &s,
        &back,
        match (was_open, told) {
            (false, _) => "You withdrew the break-glass request. Nothing was opened.",
            (true, true) => {
                "Break-glass is closed: nobody can read this site under it now. The owner has a receipt."
            }
            (true, false) => {
                "Break-glass is closed: nobody can read this site under it now. No owner receipt was written; the operator ledger has it."
            }
        },
    )
}

/// Everything the site page's access card shows.
#[derive(Debug, Serialize)]
pub(crate) struct AccessCard {
    pub grants: Vec<Value>,
    pub break_glass: Vec<Value>,
    pub holds: Vec<holds::Hold>,
    pub active_hold: Option<holds::Hold>,
    pub view: Option<SupportView>,
    pub can_request: bool,
    pub can_break_glass: bool,
    pub can_hold: bool,
    pub holds_enforced: bool,
    pub owner_link: Option<String>,
    pub receipts_on: bool,
}

/// Builds the access card for `site`, recording the access when it shows content.
pub(crate) async fn card(
    s: &OperatorState,
    op: &Operator,
    site: &str,
    owned: bool,
) -> Result<AccessCard, sqlx::Error> {
    let pool = &s.inner.pool;
    expire(pool).await?;
    let me = op.account.to_string();
    let mut grants = Vec::new();
    for g in grants_for_site(pool, site).await? {
        let who = account_name(pool, &g.requested_by).await;
        let mut v = serde_json::to_value(&g).unwrap_or(Value::Null);
        v["who"] = json!(who);
        v["area_list"] = json!(g.area_list());
        v["mine"] = json!(g.requested_by == me);
        grants.push(v);
    }
    let mut break_glass = Vec::new();
    for bg in break_glass_for_site(pool, site).await? {
        let mut v = serde_json::to_value(&bg).unwrap_or(Value::Null);
        v["first_name"] = json!(account_name(pool, &bg.first_by).await);
        v["second_name"] = json!(match &bg.second_by {
            Some(id) => Some(account_name(pool, id).await),
            None => None,
        });
        v["can_second"] = json!(can_second(op, &bg));
        v["mine"] = json!(bg.first_by == me || bg.second_by.as_deref() == Some(me.as_str()));
        break_glass.push(v);
    }
    let view = support_view(s, op, site).await?;
    let platform = s
        .inner
        .config
        .platform_origin
        .as_ref()
        .map(|u| u.as_str().trim_end_matches('/').to_owned());
    Ok(AccessCard {
        grants,
        break_glass,
        holds: holds::for_site(pool, site).await?,
        active_hold: holds::active(pool, site).await?,
        view,
        can_request: owned && (op.has(Role::Support) || op.has(Role::Sre) || op.has(Role::Admin)),
        can_break_glass: op.has(Role::Security) || op.has(Role::Director),
        can_hold: op.has(Role::Security) || op.has(Role::Sre),
        holds_enforced: s.inner.config.holds_enforced,
        owner_link: platform.map(|p| format!("{p}/support-access/")),
        receipts_on: s.inner.receipts.is_some() && ReceiptKind::parse("operator.access").is_some(),
    })
}

/// Lane cards for `op`: break-glass requests they can sign as the second person.
pub(crate) async fn lane(s: &OperatorState, op: &Operator) -> Result<Vec<Value>, sqlx::Error> {
    let pool = &s.inner.pool;
    expire(pool).await?;
    let mut cards = Vec::new();
    for bg in break_glass_waiting(pool).await? {
        if !can_second(op, &bg) {
            continue;
        }
        let domain = site_domain(pool, &bg.site_id).await?.unwrap_or_default();
        let mut v = serde_json::to_value(&bg).unwrap_or(Value::Null);
        v["domain"] = json!(domain);
        v["first_name"] = json!(account_name(pool, &bg.first_by).await);
        v["second_role"] = json!(second_role_for(&bg.first_role).as_str());
        cards.push(v);
    }
    Ok(cards)
}
