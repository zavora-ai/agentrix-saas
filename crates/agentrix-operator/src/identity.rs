//! Operator sign-in, sessions and platform roles (08 §2.3, §5.3).
//!
//! Sign-in proves presence the way an approval does (02 §8): the server picks a fresh sign-in
//! fingerprint, and the person approves it with a passkey (a WebAuthn assertion whose challenge
//! commits to the fingerprint) or, without a passkey, a fresh authenticator code. The check is
//! The first release's approvals API behind `agentrix_host::Approvals`, so this crate never handles a credential.
//!
//! Operator sessions live in `operator_sessions` with their own cookie, `__Host-agx_ops`
//! (HttpOnly, Secure, SameSite=Strict, 12 hours), on the ops host only.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use agentrix_host::Approvals;
use axum::extract::{Query, State};
use axum::http::header::{COOKIE, ORIGIN, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::ledger::{self, Entry};
use crate::{Error, OperatorState};

/// How long an operator session lasts.
pub const SESSION_HOURS: i64 = 12;
/// How long a started sign-in waits for its proof.
const CEREMONY: Duration = Duration::from_secs(300);

// ---------- Approvals ----------

/// What an approval proved: the method and the full proof for the ledger.
#[derive(Debug, Clone)]
pub(crate) struct Proof {
    /// `passkey` or `totp`.
    pub method: &'static str,
    /// The proof as node returned it.
    pub evidence: Value,
}

/// Turns the page's `{"method":"passkey","credential":…}` or `{"method":"totp","code":…}` into
/// the seam's request, asks node to verify it against `fingerprint`, and names the method.
pub(crate) async fn approve(
    approvals: &dyn Approvals,
    account: Uuid,
    fingerprint: [u8; 32],
    body: &Value,
) -> Result<Proof, String> {
    let request = match body.get("method").and_then(Value::as_str) {
        Some("passkey") => {
            // Both shapes: the seam documents {"passkey": cred}; node's adapter parses the first release's
            // ApproveRequest, {"method":"passkey","credential": cred}. Extra fields are ignored.
            let credential = body.get("credential").cloned().unwrap_or(Value::Null);
            json!({ "method": "passkey", "credential": credential.clone(), "passkey": credential })
        }
        Some("totp") => {
            let code: String = body
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            if code.len() != 6 {
                return Err("Enter the 6-digit code from your authenticator app.".into());
            }
            json!({ "method": "totp", "code": code.clone(), "totp": code })
        }
        _ => return Err("Approve with your passkey or an authenticator code.".into()),
    };
    match approvals.verify(account, fingerprint, request).await {
        Ok(evidence) => {
            let method = if evidence.get("method").and_then(Value::as_str) == Some("passkey") {
                "passkey"
            } else {
                "totp"
            };
            Ok(Proof { method, evidence })
        }
        Err(e) => {
            tracing::info!(error = %e, "approval refused");
            Err(refusal_words(&e.to_string()))
        }
    }
}

/// The first release's refusals are already plain words for people; anything else gets a safe line.
fn refusal_words(message: &str) -> String {
    let known = [
        "That code didn't work",
        "This approval expired",
        "This passkey can't approve",
        "This account has no authenticator",
        "Too many tries",
        "That passkey didn't work",
    ];
    if known.iter().any(|k| message.starts_with(k)) {
        message.to_owned()
    } else {
        "That didn't check out. Nothing was approved. Try again.".to_owned()
    }
}

// ---------- Roles ----------

/// A platform role on the operator side (08 §2.3), plus `director` (charter Owner, 08 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Role {
    /// Platform administration.
    #[serde(rename = "op.admin")]
    Admin,
    /// Site reliability.
    #[serde(rename = "op.sre")]
    Sre,
    /// Release trains.
    #[serde(rename = "op.release")]
    Release,
    /// Money.
    #[serde(rename = "op.finance")]
    Finance,
    /// Support.
    #[serde(rename = "op.support")]
    Support,
    /// Partner success.
    #[serde(rename = "op.partners")]
    Partners,
    /// Security and abuse.
    #[serde(rename = "op.security")]
    Security,
    /// Read-only audit.
    #[serde(rename = "op.auditor")]
    Auditor,
    /// A director: Owner on the operator charter.
    #[serde(rename = "director")]
    Director,
}

impl Role {
    /// Every role, in 08 §2.3's order.
    pub const ALL: [Role; 9] = [
        Role::Admin,
        Role::Sre,
        Role::Release,
        Role::Finance,
        Role::Support,
        Role::Partners,
        Role::Security,
        Role::Auditor,
        Role::Director,
    ];

    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "op.admin",
            Role::Sre => "op.sre",
            Role::Release => "op.release",
            Role::Finance => "op.finance",
            Role::Support => "op.support",
            Role::Partners => "op.partners",
            Role::Security => "op.security",
            Role::Auditor => "op.auditor",
            Role::Director => "director",
        }
    }

    /// The role for a stored name.
    pub fn parse(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.as_str() == name)
    }
}

/// A signed-in operator.
#[derive(Debug, Clone, Serialize)]
pub struct Operator {
    /// Account id.
    pub account: Uuid,
    /// Sign-in email.
    pub email: String,
    /// The name the console greets.
    pub name: String,
    /// Active platform roles.
    pub roles: Vec<Role>,
    /// How they proved presence at sign-in: `passkey` or `totp`.
    pub method: String,
}

impl Operator {
    /// Whether they hold `role`.
    pub fn has(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }

    /// The first name, for greetings.
    pub fn first_name(&self) -> &str {
        self.name.split_whitespace().next().unwrap_or(&self.name)
    }

    /// Initials for the signer chip.
    pub fn initials(&self) -> String {
        self.name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }

    /// The ledger's actor id.
    pub fn actor(&self) -> String {
        format!("account:{}", self.account)
    }
}

pub(crate) async fn active_roles(
    pool: &SqlitePool,
    account: Uuid,
) -> Result<Vec<Role>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT role FROM operator_roles WHERE account_id = ? AND revoked_at IS NULL",
    )
    .bind(account.to_string())
    .fetch_all(pool)
    .await?;
    let mut roles: Vec<Role> = rows.iter().filter_map(|(r,)| Role::parse(r)).collect();
    roles.sort_by_key(|r| Role::ALL.iter().position(|x| x == r));
    Ok(roles)
}

/// An account as the operator console sees it.
#[derive(Debug, Clone)]
pub(crate) struct Account {
    pub id: Uuid,
    pub email: String,
    pub name: String,
}

pub(crate) async fn account_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<Account>, sqlx::Error> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, email, display_name FROM accounts WHERE email = ? COLLATE NOCASE",
    )
    .bind(email.trim())
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(id, email, name)| {
        Some(Account {
            id: Uuid::parse_str(&id).ok()?,
            email,
            name,
        })
    }))
}

pub(crate) async fn account(pool: &SqlitePool, id: Uuid) -> Result<Option<Account>, sqlx::Error> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT email, display_name FROM accounts WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(email, name)| Account { id, email, name }))
}

/// Applies a roles file's text (see [`OperatorState::apply_roles_file`]).
pub(crate) async fn apply_roles(pool: &SqlitePool, text: &str) -> Result<usize, Error> {
    let mut wanted: HashMap<Uuid, (Account, Vec<Role>)> = HashMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let Some(email) = words.next() else { continue };
        let roles: Vec<Role> = words
            .filter_map(|w| {
                let role = Role::parse(w);
                if role.is_none() {
                    tracing::warn!(line = n + 1, role = w, "roles file: unknown role, skipped");
                }
                role
            })
            .collect();
        match account_by_email(pool, email).await? {
            Some(account) => {
                wanted
                    .entry(account.id)
                    .or_insert_with(|| (account, Vec::new()))
                    .1
                    .extend(roles);
            }
            None => tracing::warn!(
                line = n + 1,
                "roles file: no account with that email yet; it needs to register on the platform first"
            ),
        }
    }
    let mut changes = 0;
    let now = ledger::now();
    // Grants.
    for (account, roles) in wanted.values() {
        for role in roles {
            let done = sqlx::query(
                "INSERT INTO operator_roles (account_id, role, granted_by, granted_at) VALUES (?, ?, 'roles-file', ?) \
                 ON CONFLICT (account_id, role) DO UPDATE SET granted_by = 'roles-file', granted_at = excluded.granted_at, revoked_at = NULL \
                 WHERE operator_roles.revoked_at IS NOT NULL",
            )
            .bind(account.id.to_string())
            .bind(role.as_str())
            .bind(&now)
            .execute(pool)
            .await?;
            if done.rows_affected() > 0 {
                changes += 1;
                ledger::append(
                    pool,
                    Entry::platform(
                        "role.grant",
                        &format!("account:{}", account.id),
                        &format!(
                            "{} was given {} from the roles file.",
                            account.name,
                            role.as_str()
                        ),
                    )
                    .detail(json!({ "role": role.as_str(), "source": "roles-file" })),
                )
                .await?;
            }
        }
    }
    // Revocations: file-granted roles the file no longer lists.
    let granted: Vec<(String, String)> = sqlx::query_as(
        "SELECT account_id, role FROM operator_roles WHERE granted_by = 'roles-file' AND revoked_at IS NULL",
    )
    .fetch_all(pool)
    .await?;
    for (account_id, role_name) in granted {
        let keep = Uuid::parse_str(&account_id)
            .ok()
            .and_then(|id| wanted.get(&id))
            .is_some_and(|(_, roles)| roles.iter().any(|r| r.as_str() == role_name));
        if keep {
            continue;
        }
        sqlx::query("UPDATE operator_roles SET revoked_at = ? WHERE account_id = ? AND role = ?")
            .bind(&now)
            .bind(&account_id)
            .bind(&role_name)
            .execute(pool)
            .await?;
        changes += 1;
        ledger::append(
            pool,
            Entry::platform(
                "role.revoke",
                &format!("account:{account_id}"),
                &format!("{role_name} was taken away: the roles file no longer lists it."),
            )
            .detail(json!({ "role": role_name, "source": "roles-file" })),
        )
        .await?;
    }
    Ok(changes)
}

// ---------- Ceremonies and throttle ----------

struct Pending {
    account: Uuid,
    fingerprint: [u8; 32],
    started: Instant,
}

/// Started sign-ins, by ceremony id.
#[derive(Default)]
pub(crate) struct Ceremonies(Mutex<HashMap<String, Pending>>);

impl Ceremonies {
    fn put(&self, account: Uuid, fingerprint: [u8; 32]) -> String {
        let id = random_token();
        let mut map = self.0.lock().expect("ceremonies lock");
        map.retain(|_, p| p.started.elapsed() < CEREMONY);
        if map.len() < 4096 {
            map.insert(
                id.clone(),
                Pending {
                    account,
                    fingerprint,
                    started: Instant::now(),
                },
            );
        }
        id
    }

    fn take(&self, id: &str) -> Option<Pending> {
        let mut map = self.0.lock().expect("ceremonies lock");
        map.remove(id).filter(|p| p.started.elapsed() < CEREMONY)
    }
}

/// Failed sign-ins per email: ten in 15 minutes, then a wait.
#[derive(Default)]
pub(crate) struct Throttle(Mutex<HashMap<String, (u32, Instant)>>);

impl Throttle {
    const LIMIT: u32 = 10;
    const WINDOW: Duration = Duration::from_secs(900);

    fn blocked(&self, email: &str) -> bool {
        let map = self.0.lock().expect("throttle lock");
        map.get(&email.to_lowercase())
            .is_some_and(|(n, at)| *n >= Self::LIMIT && at.elapsed() < Self::WINDOW)
    }

    fn fail(&self, email: &str) {
        let mut map = self.0.lock().expect("throttle lock");
        map.retain(|_, (_, at)| at.elapsed() < Self::WINDOW);
        let entry = map
            .entry(email.to_lowercase())
            .or_insert((0, Instant::now()));
        entry.0 += 1;
    }

    fn clear(&self, email: &str) {
        self.0
            .lock()
            .expect("throttle lock")
            .remove(&email.to_lowercase());
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The fingerprint a sign-in approves: SHA-256 over a fixed label, the ops origin, the account,
/// a fresh nonce and the time, so a sign-in proof can't be replayed as anything else.
fn signin_fingerprint(origin: &str, account: Uuid) -> [u8; 32] {
    let mut nonce = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut h = Sha256::new();
    h.update(b"agentrix/ops-signin/v1\n");
    h.update(origin.as_bytes());
    h.update(b"\n");
    h.update(account.as_bytes());
    h.update(nonce);
    h.update(Utc::now().timestamp().to_be_bytes());
    h.finalize().into()
}

// ---------- Cookies ----------

pub(crate) fn cookie_name(secure: bool) -> &'static str {
    if secure { "__Host-agx_ops" } else { "agx_ops" }
}

fn set_cookie(secure: bool, token: &str) -> HeaderValue {
    let value = format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        cookie_name(secure),
        SESSION_HOURS * 3600,
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&value).expect("token is base64url")
}

fn clear_cookie(secure: bool) -> HeaderValue {
    let value = format!(
        "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&value).expect("static cookie")
}

fn token_from(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = cookie_name(secure);
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
        .filter(|v| !v.is_empty() && v.len() <= 64)
}

// ---------- Sessions ----------

impl OperatorState {
    /// The signed-in operator behind these headers: a live session and at least one active role.
    /// Anyone signed in on the ops host, operator or partner person, as `(account, name, email,
    /// method)`. Operator pages use [`OperatorState::operator_from`], which also needs a role.
    pub async fn person_from(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<(Uuid, String, String, String)>, sqlx::Error> {
        let Some(token) = token_from(headers, self.inner.config.secure_cookies) else {
            return Ok(None);
        };
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT account_id, method FROM operator_sessions WHERE token_hash = ? AND ended_at IS NULL AND expires_at > ?",
        )
        .bind(token_hash(&token))
        .bind(ledger::now())
        .fetch_optional(&self.inner.pool)
        .await?;
        let Some((account_id, method)) = row else {
            return Ok(None);
        };
        let Ok(id) = Uuid::parse_str(&account_id) else {
            return Ok(None);
        };
        Ok(account(&self.inner.pool, id)
            .await?
            .map(|a| (id, a.name, a.email, method)))
    }

    /// The signed-in operator behind these headers: a live session and an operator role.
    pub async fn operator_from(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<Operator>, sqlx::Error> {
        let Some(token) = token_from(headers, self.inner.config.secure_cookies) else {
            return Ok(None);
        };
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT account_id, method FROM operator_sessions WHERE token_hash = ? AND ended_at IS NULL AND expires_at > ?",
        )
        .bind(token_hash(&token))
        .bind(ledger::now())
        .fetch_optional(&self.inner.pool)
        .await?;
        let Some((account_id, method)) = row else {
            return Ok(None);
        };
        let Ok(id) = Uuid::parse_str(&account_id) else {
            return Ok(None);
        };
        let roles = active_roles(&self.inner.pool, id).await?;
        if roles.is_empty() {
            return Ok(None);
        }
        let Some(acct) = account(&self.inner.pool, id).await? else {
            return Ok(None);
        };
        Ok(Some(Operator {
            account: id,
            email: acct.email,
            name: acct.name,
            roles,
            method,
        }))
    }

    /// Starts a session for `account` and returns the cookie.
    pub(crate) async fn start_session(
        &self,
        account: Uuid,
        method: &str,
    ) -> Result<HeaderValue, sqlx::Error> {
        let token = random_token();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO operator_sessions (token_hash, account_id, method, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(token_hash(&token))
        .bind(account.to_string())
        .bind(method)
        .bind(ledger::time(now))
        .bind(ledger::time(now + chrono::Duration::hours(SESSION_HOURS)))
        .execute(&self.inner.pool)
        .await?;
        Ok(set_cookie(self.inner.config.secure_cookies, &token))
    }

    /// Refuses a state-changing request from another origin. SameSite=Strict already keeps the
    /// cookie off cross-site requests; this also refuses ones that carry no cookie.
    pub(crate) fn same_origin(&self, headers: &HeaderMap) -> bool {
        match headers.get(ORIGIN).and_then(|o| o.to_str().ok()) {
            None => true,
            Some(origin) => {
                origin.trim_end_matches('/')
                    == self.inner.config.origin.as_str().trim_end_matches('/')
            }
        }
    }
}

// ---------- Routes ----------

pub(crate) fn routes() -> Router<OperatorState> {
    Router::new()
        .route("/signin", get(signin_page))
        .route("/signin/start", post(signin_start))
        .route("/signin/finish", post(signin_finish))
        .route("/signout", post(signout))
}

#[derive(Debug, Deserialize)]
struct NextQuery {
    next: Option<String>,
}

/// Only same-host paths, so a sign-in can't bounce someone to another site.
pub(crate) fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") && !n.contains('\\') => n.to_owned(),
        _ => "/".to_owned(),
    }
}

async fn signin_page(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Query(q): Query<NextQuery>,
) -> Response {
    let next = safe_next(q.next.as_deref());
    if let Ok(Some(_)) = s.operator_from(&headers).await {
        return Redirect::to(&next).into_response();
    }
    if let Ok(Some(_)) = s.person_from(&headers).await {
        return Redirect::to("/portfolio").into_response();
    }
    match s.inner.render.page("signin.html", &json!({ "next": next })) {
        Ok(html) => Html(html).into_response(),
        Err(e) => crate::pages::internal(e),
    }
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message }))).into_response()
}

const NOT_AN_OPERATOR: &str =
    "This email can't sign in to the control plane. Operators are added by an op.admin.";

#[derive(Debug, Deserialize)]
struct StartRequest {
    email: String,
}

async fn signin_start(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Json(req): Json<StartRequest>,
) -> Response {
    if !s.same_origin(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "This request came from another site, so it was refused.",
        );
    }
    let email = req.email.trim();
    if email.is_empty() || email.len() > 254 {
        return refuse(
            StatusCode::BAD_REQUEST,
            "Enter the email you sign in to Agentrix with.",
        );
    }
    if s.inner.throttle.blocked(email) {
        return refuse(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tries. Wait 15 minutes, then try again.",
        );
    }
    let found = match account_by_email(&s.inner.pool, email).await {
        Ok(found) => found,
        Err(e) => return crate::pages::internal(e),
    };
    let Some(acct) = found else {
        s.inner.throttle.fail(email);
        return refuse(StatusCode::FORBIDDEN, NOT_AN_OPERATOR);
    };
    // Zavora's people (op.* roles) and partners' people (partner.* roles) sign in here; nobody else.
    let allowed = async {
        Ok::<_, sqlx::Error>(
            !active_roles(&s.inner.pool, acct.id).await?.is_empty()
                || !crate::partners::memberships(&s.inner.pool, acct.id)
                    .await?
                    .is_empty(),
        )
    }
    .await;
    match allowed {
        Ok(true) => {}
        Ok(false) => {
            s.inner.throttle.fail(email);
            return refuse(StatusCode::FORBIDDEN, NOT_AN_OPERATOR);
        }
        Err(e) => return crate::pages::internal(e),
    }
    let fingerprint = signin_fingerprint(s.inner.config.origin.as_str(), acct.id);
    let options = match s
        .inner
        .approvals
        .passkey_options(acct.id, fingerprint)
        .await
    {
        Ok(options) => options,
        Err(e) => return crate::pages::internal(e),
    };
    let ceremony = s.inner.signins.put(acct.id, fingerprint);
    Json(json!({
        "ceremony": ceremony,
        "method": if options.is_some() { "passkey" } else { "totp" },
        "options": options,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct FinishRequest {
    ceremony: String,
    email: String,
    #[serde(default)]
    next: Option<String>,
    /// The approval: `{"method":"passkey","credential":…}` or `{"method":"totp","code":…}`.
    proof: Value,
}

async fn signin_finish(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Json(req): Json<FinishRequest>,
) -> Response {
    if !s.same_origin(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "This request came from another site, so it was refused.",
        );
    }
    if s.inner.throttle.blocked(&req.email) {
        return refuse(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tries. Wait 15 minutes, then try again.",
        );
    }
    let Some(pending) = s.inner.signins.take(&req.ceremony) else {
        return refuse(StatusCode::GONE, "That took too long. Start again.");
    };
    let proof = match approve(
        s.inner.approvals.as_ref(),
        pending.account,
        pending.fingerprint,
        &req.proof,
    )
    .await
    {
        Ok(proof) => proof,
        Err(words) => {
            s.inner.throttle.fail(&req.email);
            return refuse(StatusCode::UNAUTHORIZED, &words);
        }
    };
    s.inner.throttle.clear(&req.email);
    let method = proof.method;
    let cookie = match s.start_session(pending.account, method).await {
        Ok(cookie) => cookie,
        Err(e) => return crate::pages::internal(e),
    };
    let name = account(&s.inner.pool, pending.account)
        .await
        .ok()
        .flatten()
        .map(|a| a.name)
        .unwrap_or_default();
    let words = if method == "passkey" {
        "with a passkey"
    } else {
        "with an authenticator code"
    };
    if let Err(e) = ledger::append(
        &s.inner.pool,
        Entry::person(
            pending.account,
            &name,
            "operator.signin",
            "fleet",
            &format!("{name} signed in to the control plane {words}."),
        )
        .approval(
            json!([{ "account": pending.account, "method": method, "proof": proof.evidence }]),
        ),
    )
    .await
    {
        return crate::pages::internal(e);
    }
    let next = match active_roles(&s.inner.pool, pending.account).await {
        Ok(roles) if roles.is_empty() => "/portfolio".to_owned(),
        Ok(_) => safe_next(req.next.as_deref()),
        Err(e) => return crate::pages::internal(e),
    };
    let mut response = Json(json!({ "next": next })).into_response();
    response.headers_mut().insert(SET_COOKIE, cookie);
    response
}

async fn signout(State(s): State<OperatorState>, headers: HeaderMap) -> Response {
    if !s.same_origin(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "This request came from another site, so it was refused.",
        );
    }
    if let Some(token) = token_from(&headers, s.inner.config.secure_cookies) {
        let _ = sqlx::query(
            "UPDATE operator_sessions SET ended_at = ? WHERE token_hash = ? AND ended_at IS NULL",
        )
        .bind(ledger::now())
        .bind(token_hash(&token))
        .execute(&s.inner.pool)
        .await;
    }
    let mut response = Redirect::to("/signin").into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, clear_cookie(s.inner.config.secure_cookies));
    response
}

/// Session timestamps parse back from storage.
#[allow(dead_code)]
pub(crate) fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn the_roles_file_grants_and_revokes_with_ledger_entries() {
        let pool = testing::pool().await;
        let a = testing::account(&pool, "achieng@zavora.example", "Achieng Otieno").await;
        let b = testing::account(&pool, "otieno@zavora.example", "Otieno Kamau").await;
        let n = apply_roles(&pool, "# ops\nachieng@zavora.example op.sre op.admin\notieno@zavora.example op.release bogus\nnobody@x.example op.sre\n")
            .await
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(
            active_roles(&pool, a).await.unwrap(),
            vec![Role::Admin, Role::Sre]
        );
        assert_eq!(active_roles(&pool, b).await.unwrap(), vec![Role::Release]);
        // Same file again: nothing changes.
        assert_eq!(
            apply_roles(
                &pool,
                "achieng@zavora.example op.sre op.admin\notieno@zavora.example op.release\n"
            )
            .await
            .unwrap(),
            0
        );
        // Otieno leaves the file: revoked.
        assert_eq!(
            apply_roles(&pool, "achieng@zavora.example op.sre op.admin\n")
                .await
                .unwrap(),
            1
        );
        assert!(active_roles(&pool, b).await.unwrap().is_empty());
        let entries = ledger::recent(&pool, 10).await.unwrap();
        assert_eq!(entries.len(), 4);
        assert!(ledger::verify(&pool).await.unwrap().ok);
    }

    #[test]
    fn next_stays_on_this_host() {
        assert_eq!(safe_next(Some("/sites/x")), "/sites/x");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(None), "/");
    }
}
