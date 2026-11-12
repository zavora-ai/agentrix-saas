//! Team members and roles for an owner's staff (01b §4.2, 07 §5.6). The owner is the site's
//! `owner_account_id` (the first release's `sites`, read only); everyone else is a member with one role:
//!
//! | Role | Can |
//! |---|---|
//! | Owner | Everything, including granting roles |
//! | Manager | Decides the cards their role covers; can't raise authority or grant roles |
//! | Viewer | Reads |
//! | Billing | Reads; pays invoices |
//!
//! Granting a role (an invite or a change) is a sensitive change: the owner proves it with a
//! passkey or an authenticator code through the frozen approvals contract, over a fingerprint
//! of the exact grant (site, email or member, role). An email-code session can't grant roles,
//! because it can't approve anything. Removing access needs no proof: it only ever lowers it.
//! Each change signs a `team.changed` receipt. Invitations last 7 days, work only for the
//! invited email, and never sign anyone in.
//!
//! | Route (platform host, JSON) | Who |
//! |---|---|
//! | `GET /team/api/sites/{site}/members` | owner and members |
//! | `POST /team/api/sites/{site}/invite/start` `{email, role}` → `{challenge, passkey}` | owner |
//! | `POST /team/api/sites/{site}/invite` `{challenge, passkey?, totp?}` → `{member}` | owner |
//! | `POST /team/api/sites/{site}/members/{id}/role/start` `{role}` → `{challenge, passkey}` | owner |
//! | `POST /team/api/sites/{site}/members/{id}/role` `{challenge, passkey?, totp?}` | owner |
//! | `POST /team/api/sites/{site}/members/{id}/remove` | owner |
//! | `GET /team/api/invites/{token}` → `{site_name, role, inviter, email_hint}` | anyone with the link |
//! | `POST /team/api/invites/accept` `{token}` | signed in with the invited email |

use std::sync::Arc;
use std::time::Duration;

use agentrix_host::{Approvals, OwnerAccounts, OwnerSessions, Receipts};
use agentrix_types::tenancy::SiteHost;
use agentrix_types::{ReceiptBody, ReceiptKind, SiteId, Subject};
use axum::extract::{Path, Request, State};
use axum::http::header::{CACHE_CONTROL, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::mailer::{CodeMailer, Mail, MailKind};
use crate::now;
use crate::pending::Pending;

/// How long an invitation works (seconds).
pub const INVITE_SECONDS: i64 = 7 * 24 * 3600;

/// A person's role on a site (07 §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The site's owner.
    Owner,
    /// Decides the cards their role covers.
    Manager,
    /// Reads.
    Viewer,
    /// Reads and pays invoices.
    Billing,
}

impl Role {
    /// The stored word.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Manager => "manager",
            Self::Viewer => "viewer",
            Self::Billing => "billing",
        }
    }

    /// A grantable role from its word (never `owner`: ownership moves by transfer, later).
    pub fn grantable(text: &str) -> Option<Self> {
        match text {
            "manager" => Some(Self::Manager),
            "viewer" => Some(Self::Viewer),
            "billing" => Some(Self::Billing),
            _ => None,
        }
    }

    /// Whether this role decides cards (W5 checks it before showing Approve).
    pub fn can_decide(self) -> bool {
        matches!(self, Self::Owner | Self::Manager)
    }

    /// Whether this role pays invoices.
    pub fn can_pay(self) -> bool {
        matches!(self, Self::Owner | Self::Billing)
    }

    /// The role in words for an invitation.
    pub fn words(self) -> &'static str {
        match self {
            Self::Owner => "the owner",
            Self::Manager => "a manager, who can approve decisions",
            Self::Viewer => "a viewer, who can see everything but change nothing",
            Self::Billing => "billing, who can see the site and pay invoices",
        }
    }
}

/// Errors, answered as `{message, code}`.
#[derive(Debug, thiserror::Error)]
pub enum TeamError {
    /// Not signed in.
    #[error("Sign in first.")]
    SignedOut,
    /// Not the owner (or not on the team at all).
    #[error("Only the owner can change who helps run this site.")]
    NotOwner,
    /// No such site, member or invitation.
    #[error("That isn't here, or it has expired.")]
    NotFound,
    /// Bad input.
    #[error("{0}")]
    BadRequest(String),
    /// Step-up missing or refused.
    #[error(
        "Confirm it's you with your passkey or your authenticator code first. Signed in with an email code? Add a passkey, then try again."
    )]
    StepUpNeeded,
    /// Accepting with another account.
    #[error(
        "This invitation is for another email address. Sign in with the address it was sent to."
    )]
    WrongAccount,
    /// From another site.
    #[error("This request came from another site, so it was refused.")]
    CrossOrigin,
    /// Ours.
    #[error("Something went wrong on our side. Nothing changed. Try again in a minute.")]
    Internal(String),
}

impl TeamError {
    fn code(&self) -> &'static str {
        match self {
            Self::SignedOut => "signed-out",
            Self::NotOwner => "not-owner",
            Self::NotFound => "not-found",
            Self::BadRequest(_) => "bad-request",
            Self::StepUpNeeded => "step-up",
            Self::WrongAccount => "wrong-account",
            Self::CrossOrigin => "cross-origin",
            Self::Internal(_) => "internal",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::SignedOut => StatusCode::UNAUTHORIZED,
            Self::NotOwner | Self::StepUpNeeded | Self::WrongAccount | Self::CrossOrigin => {
                StatusCode::FORBIDDEN
            }
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for TeamError {
    fn into_response(self) -> Response {
        if let Self::Internal(detail) = &self {
            tracing::error!(%detail, "identity: team internal error");
        }
        (
            self.status(),
            Json(json!({ "message": self.to_string(), "code": self.code() })),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for TeamError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

impl From<anyhow::Error> for TeamError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(format!("{e:#}"))
    }
}

/// A member as the People page shows them.
#[derive(Debug, Clone, Serialize)]
pub struct Member {
    /// `mem_…`.
    pub id: String,
    /// The email invited.
    pub email: String,
    /// Their role.
    pub role: Role,
    /// invited | active.
    pub state: &'static str,
    /// Their account, once accepted.
    pub account: Option<String>,
    /// When invited, in words.
    pub invited: String,
    /// When accepted, in words.
    pub accepted: Option<String>,
}

#[derive(Debug, Clone)]
enum Grant {
    Invite { email: String, role: Role },
    Change { member: String, role: Role },
}

#[derive(Debug)]
struct StepUp {
    account: Uuid,
    site: SiteId,
    fingerprint: [u8; 32],
    grant: Grant,
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    id: String,
    email: String,
    account_id: Option<String>,
    role: String,
    invited_at: i64,
    expires_at: i64,
    accepted_at: Option<i64>,
}

impl MemberRow {
    fn view(&self) -> Member {
        Member {
            id: self.id.clone(),
            email: self.email.clone(),
            role: Role::grantable(&self.role).unwrap_or(Role::Viewer),
            state: if self.accepted_at.is_some() {
                "active"
            } else {
                "invited"
            },
            account: self.account_id.clone(),
            invited: crate::visitor::consent_when(self.invited_at),
            accepted: self.accepted_at.map(crate::visitor::consent_when),
        }
    }
}

/// Team members and roles. Cheap to clone.
#[derive(Clone)]
pub struct Team {
    inner: Arc<Inner>,
}

struct Inner {
    pool: sqlx::SqlitePool,
    accounts: Arc<dyn OwnerAccounts>,
    sessions: Arc<dyn OwnerSessions>,
    approvals: Arc<dyn Approvals>,
    receipts: Arc<dyn Receipts>,
    mailer: Arc<dyn CodeMailer>,
    platform: SiteHost,
    stepups: Pending<StepUp>,
}

impl std::fmt::Debug for Team {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Team").finish_non_exhaustive()
    }
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl Team {
    /// For node: control.db, owner accounts and sessions, approvals, receipts and `mailer`.
    pub fn for_node(ctx: &agentrix_host::NodeContext, mailer: Arc<dyn CodeMailer>) -> Self {
        let platform = SiteHost {
            site: SiteId::new("platform"),
            host: ctx.platform_host.clone(),
            origin: ctx.origin.as_str().trim_end_matches('/').to_owned(),
            name: "Agentrix".into(),
            sample: false,
        };
        Self {
            inner: Arc::new(Inner {
                pool: ctx.pool.clone(),
                accounts: ctx.owner_accounts.clone(),
                sessions: ctx.owners.clone(),
                approvals: ctx.approvals.clone(),
                receipts: ctx.receipts.clone(),
                mailer,
                platform,
                stepups: Pending::new(Duration::from_secs(300), 1024),
            }),
        }
    }

    /// The routes on the platform host.
    pub fn router(&self) -> Router {
        Router::new()
            .route("/team/api/sites/{site}/members", get(members))
            .route("/team/api/sites/{site}/invite/start", post(invite_start))
            .route("/team/api/sites/{site}/invite", post(invite))
            .route(
                "/team/api/sites/{site}/members/{id}/role/start",
                post(role_start),
            )
            .route(
                "/team/api/sites/{site}/members/{id}/role",
                post(role_change),
            )
            .route("/team/api/sites/{site}/members/{id}/remove", post(remove))
            .route("/team/api/invites/{token}", get(invite_view))
            .route("/team/api/invites/accept", post(accept))
            .route("/team/people.js", get(people_js))
            .layer(middleware::from_fn_with_state(self.clone(), same_origin))
            .with_state(self.clone())
    }

    /// The site's owner and business name, from the first release's `sites` (read only).
    async fn site(&self, site: &SiteId) -> Result<(Option<Uuid>, String), TeamError> {
        let row: Option<(Option<String>, String, String)> =
            sqlx::query_as("SELECT owner_account_id, name, domain FROM sites WHERE site_id = ?")
                .bind(site.as_str())
                .fetch_optional(&self.inner.pool)
                .await?;
        let (owner, name, domain) = row.ok_or(TeamError::NotFound)?;
        let owner = owner.and_then(|o| Uuid::parse_str(&o).ok());
        Ok((owner, if name.trim().is_empty() { domain } else { name }))
    }

    /// Someone's role on a site: owner, an accepted member's role, or none. W5 calls this
    /// before showing a card's Approve.
    pub async fn role_of(&self, site: &SiteId, account: Uuid) -> Result<Option<Role>, TeamError> {
        let (owner, _) = self.site(site).await?;
        if owner == Some(account) {
            return Ok(Some(Role::Owner));
        }
        let role: Option<(String,)> = sqlx::query_as(
            "SELECT role FROM site_members WHERE site_id = ? AND account_id = ? AND accepted_at IS NOT NULL AND removed_at IS NULL",
        )
        .bind(site.as_str())
        .bind(account.to_string())
        .fetch_optional(&self.inner.pool)
        .await?;
        Ok(role.and_then(|(r,)| Role::grantable(&r)))
    }

    /// Sites someone helps run as a member (not as owner), with the role: the account switcher.
    pub async fn memberships(&self, account: Uuid) -> Result<Vec<(SiteId, Role)>, TeamError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT site_id, role FROM site_members WHERE account_id = ? AND accepted_at IS NOT NULL AND removed_at IS NULL ORDER BY accepted_at",
        )
        .bind(account.to_string())
        .fetch_all(&self.inner.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(s, r)| Role::grantable(&r).map(|r| (SiteId::new(s), r)))
            .collect())
    }

    async fn signed_in(&self, headers: &HeaderMap) -> Result<agentrix_host::Owner, TeamError> {
        self.inner
            .sessions
            .owner(headers)
            .await?
            .ok_or(TeamError::SignedOut)
    }

    async fn owner_of(
        &self,
        site: &SiteId,
        headers: &HeaderMap,
    ) -> Result<(Uuid, String), TeamError> {
        let me = self.signed_in(headers).await?;
        let (owner, name) = self.site(site).await?;
        if owner != Some(me.account) {
            return Err(TeamError::NotOwner);
        }
        Ok((me.account, name))
    }

    /// The site's current members and open invitations, oldest first. No access check: the
    /// caller has checked the owner (the console shell does, for the People app).
    pub async fn members_of(&self, site: &SiteId) -> Result<Vec<Member>, TeamError> {
        let rows: Vec<MemberRow> = sqlx::query_as(
            "SELECT id, email, account_id, role, invited_at, expires_at, accepted_at FROM site_members WHERE site_id = ? AND removed_at IS NULL ORDER BY invited_at",
        )
        .bind(site.as_str())
        .fetch_all(&self.inner.pool)
        .await?;
        let at = now();
        Ok(rows
            .iter()
            .filter(|r| r.accepted_at.is_some() || r.expires_at > at)
            .map(MemberRow::view)
            .collect())
    }

    /// Removes a member for the signed-in owner (the People app's form post).
    pub async fn remove(
        &self,
        site: &SiteId,
        member: &str,
        headers: &HeaderMap,
    ) -> Result<bool, TeamError> {
        self.remove_member(site, member, headers).await
    }

    async fn list(&self, site: &SiteId, headers: &HeaderMap) -> Result<Value, TeamError> {
        let me = self.signed_in(headers).await?;
        let role = self
            .role_of(site, me.account)
            .await?
            .ok_or(TeamError::NotOwner)?;
        let (owner, name) = self.site(site).await?;
        let rows: Vec<MemberRow> = sqlx::query_as(
            "SELECT id, email, account_id, role, invited_at, expires_at, accepted_at FROM site_members WHERE site_id = ? AND removed_at IS NULL ORDER BY invited_at",
        )
        .bind(site.as_str())
        .fetch_all(&self.inner.pool)
        .await?;
        let at = now();
        let members: Vec<Member> = rows
            .iter()
            .filter(|r| r.accepted_at.is_some() || r.expires_at > at)
            .map(MemberRow::view)
            .collect();
        Ok(
            json!({ "site": name, "you": { "account": me.account, "role": role }, "owner": { "account": owner }, "members": members }),
        )
    }

    async fn start(
        &self,
        site: &SiteId,
        headers: &HeaderMap,
        grant: Grant,
    ) -> Result<Value, TeamError> {
        let (account, _) = self.owner_of(site, headers).await?;
        let mut nonce = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let what = match &grant {
            Grant::Invite { email, role } => {
                json!({ "kind": "team.invite", "email": email, "role": role })
            }
            Grant::Change { member, role } => {
                json!({ "kind": "team.role", "member": member, "role": role })
            }
        };
        let fingerprint = sha(&serde_json::to_vec(&json!({ "site": site, "account": account, "grant": what, "nonce": hex::encode(nonce) })).map_err(|e| TeamError::Internal(e.to_string()))?);
        let passkey = self
            .inner
            .approvals
            .passkey_options(account, fingerprint)
            .await?;
        let challenge = self.inner.stepups.put(StepUp {
            account,
            site: site.clone(),
            fingerprint,
            grant,
        });
        Ok(json!({ "challenge": challenge, "passkey": passkey }))
    }

    async fn proven(
        &self,
        site: &SiteId,
        headers: &HeaderMap,
        body: &Proof,
    ) -> Result<(Uuid, String, Grant), TeamError> {
        let (account, name) = self.owner_of(site, headers).await?;
        let step = body
            .challenge
            .as_deref()
            .and_then(|c| self.inner.stepups.take(c))
            .ok_or(TeamError::StepUpNeeded)?;
        if step.account != account || step.site != *site {
            return Err(TeamError::StepUpNeeded);
        }
        let proof = match (&body.passkey, &body.totp) {
            (Some(credential), _) => json!({ "passkey": credential }),
            (None, Some(code)) => json!({ "totp": code }),
            (None, None) => return Err(TeamError::StepUpNeeded),
        };
        self.inner
            .approvals
            .verify(account, step.fingerprint, proof)
            .await
            .map_err(|e| {
                tracing::info!(error = %e, "identity: team step-up refused");
                TeamError::StepUpNeeded
            })?;
        Ok((account, name, step.grant))
    }

    async fn receipt(
        &self,
        site: &SiteId,
        member: &str,
        subject: String,
        detail: Value,
    ) -> Option<String> {
        // `team.changed` is being added to agentrix-types (asked of W0, Sun 8 Nov); until it is
        // there, the change is recorded in site_members without a receipt.
        let Some(kind) = ReceiptKind::parse("team.changed") else {
            tracing::warn!("identity: team.changed receipts aren't available in this build yet");
            return None;
        };
        let now = chrono::Utc::now();
        let mut body = ReceiptBody::new(kind, site.clone(), Subject::Account(subject), now, now);
        body.detail = Some(detail);
        match self.inner.receipts.issue(body).await {
            Ok(digest) => {
                let _ = sqlx::query("UPDATE site_members SET receipt = ? WHERE id = ?")
                    .bind(&digest)
                    .bind(member)
                    .execute(&self.inner.pool)
                    .await;
                Some(digest)
            }
            Err(e) => {
                tracing::warn!(error = %e, "identity: team receipt not issued");
                None
            }
        }
    }

    async fn invite(
        &self,
        site: &SiteId,
        headers: &HeaderMap,
        body: Proof,
    ) -> Result<Member, TeamError> {
        let (owner, business, grant) = self.proven(site, headers, &body).await?;
        let Grant::Invite { email, role } = grant else {
            return Err(TeamError::StepUpNeeded);
        };
        if let Some(existing) = self.inner.accounts.by_email(&email).await? {
            if existing.account == owner {
                return Err(TeamError::BadRequest("You already own this site.".into()));
            }
        }
        let mut token = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut token);
        let token = URL_SAFE_NO_PAD.encode(token);
        let id = format!("mem_{}", Uuid::now_v7().simple());
        let at = now();
        // A fresh invite to the same address replaces an old, unaccepted one.
        sqlx::query("UPDATE site_members SET removed_at = ? WHERE site_id = ? AND email = ? AND accepted_at IS NULL AND removed_at IS NULL")
            .bind(at)
            .bind(site.as_str())
            .bind(&email)
            .execute(&self.inner.pool)
            .await?;
        let inserted = sqlx::query(
            "INSERT INTO site_members (id, site_id, email, role, invited_by, invite_hash, invited_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(site.as_str())
        .bind(&email)
        .bind(role.as_str())
        .bind(owner.to_string())
        .bind(hex::encode(sha(token.as_bytes())))
        .bind(at)
        .bind(at + INVITE_SECONDS)
        .execute(&self.inner.pool)
        .await;
        if let Err(sqlx::Error::Database(e)) = &inserted {
            if e.is_unique_violation() {
                return Err(TeamError::BadRequest(format!(
                    "{email} already helps run this site. Change their role instead."
                )));
            }
        }
        inserted?;
        let link = format!("{}/team/join?t={token}", self.inner.platform.origin);
        let mail = Mail {
            site: self.inner.platform.clone(),
            to: email.clone(),
            kind: MailKind::TeamInvite {
                business: business.clone(),
                inviter: format!("The owner of {business}"),
                role: role.words().into(),
                link,
            },
        };
        if let Err(e) = self.inner.mailer.send(mail).await {
            tracing::warn!(error = %e, "identity: team invitation not sent");
        }
        let email_hash = hex::encode(&sha(email.as_bytes())[..8]);
        self.receipt(
            site,
            &id,
            format!("email:{email_hash}"),
            json!({ "member": id, "role": role, "by": owner, "change": "invited" }),
        )
        .await;
        Ok(Member {
            id,
            email,
            role,
            state: "invited",
            account: None,
            invited: crate::visitor::consent_when(at),
            accepted: None,
        })
    }

    async fn change_role(
        &self,
        site: &SiteId,
        member: &str,
        headers: &HeaderMap,
        body: Proof,
    ) -> Result<Member, TeamError> {
        let (owner, _, grant) = self.proven(site, headers, &body).await?;
        let Grant::Change {
            member: signed,
            role,
        } = grant
        else {
            return Err(TeamError::StepUpNeeded);
        };
        if signed != member {
            return Err(TeamError::StepUpNeeded);
        }
        let row = self
            .member(site, member)
            .await?
            .ok_or(TeamError::NotFound)?;
        sqlx::query("UPDATE site_members SET role = ? WHERE id = ? AND removed_at IS NULL")
            .bind(role.as_str())
            .bind(member)
            .execute(&self.inner.pool)
            .await?;
        let subject = row
            .account_id
            .clone()
            .unwrap_or_else(|| format!("email:{}", hex::encode(&sha(row.email.as_bytes())[..8])));
        self.receipt(site, member, subject, json!({ "member": member, "role": role, "from": row.role, "by": owner, "change": "role" })).await;
        let row = self
            .member(site, member)
            .await?
            .ok_or(TeamError::NotFound)?;
        Ok(row.view())
    }

    async fn remove_member(
        &self,
        site: &SiteId,
        member: &str,
        headers: &HeaderMap,
    ) -> Result<bool, TeamError> {
        let (owner, _) = self.owner_of(site, headers).await?;
        let row = self
            .member(site, member)
            .await?
            .ok_or(TeamError::NotFound)?;
        sqlx::query("UPDATE site_members SET removed_at = ?, invite_hash = NULL WHERE id = ? AND removed_at IS NULL").bind(now()).bind(member).execute(&self.inner.pool).await?;
        let subject = row
            .account_id
            .clone()
            .unwrap_or_else(|| format!("email:{}", hex::encode(&sha(row.email.as_bytes())[..8])));
        self.receipt(
            site,
            member,
            subject,
            json!({ "member": member, "from": row.role, "by": owner, "change": "removed" }),
        )
        .await;
        Ok(true)
    }

    async fn member(&self, site: &SiteId, member: &str) -> Result<Option<MemberRow>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, email, account_id, role, invited_at, expires_at, accepted_at FROM site_members WHERE site_id = ? AND id = ? AND removed_at IS NULL",
        )
        .bind(site.as_str())
        .bind(member)
        .fetch_optional(&self.inner.pool)
        .await
    }

    async fn invite_by_token(
        &self,
        token: &str,
    ) -> Result<(String, String, String, String, i64), TeamError> {
        let row: Option<(String, String, String, String, i64)> = sqlx::query_as(
            "SELECT id, site_id, email, role, expires_at FROM site_members WHERE invite_hash = ? AND accepted_at IS NULL AND removed_at IS NULL",
        )
        .bind(hex::encode(sha(token.as_bytes())))
        .fetch_optional(&self.inner.pool)
        .await?;
        let row = row.ok_or(TeamError::NotFound)?;
        if row.4 <= now() {
            return Err(TeamError::NotFound);
        }
        Ok(row)
    }

    async fn accept(&self, token: &str, headers: &HeaderMap) -> Result<Value, TeamError> {
        let me = self.signed_in(headers).await?;
        let (id, site, email, role, _) = self.invite_by_token(token).await?;
        let invited = self
            .inner
            .accounts
            .by_email(&email)
            .await?
            .ok_or(TeamError::WrongAccount)?;
        if invited.account != me.account {
            return Err(TeamError::WrongAccount);
        }
        sqlx::query("UPDATE site_members SET account_id = ?, accepted_at = ?, invite_hash = NULL WHERE id = ? AND accepted_at IS NULL")
            .bind(me.account.to_string())
            .bind(now())
            .bind(&id)
            .execute(&self.inner.pool)
            .await?;
        let site = SiteId::new(site);
        self.receipt(
            &site,
            &id,
            me.account.to_string(),
            json!({ "member": id, "role": role, "by": me.account, "change": "accepted" }),
        )
        .await;
        let (_, name) = self.site(&site).await?;
        Ok(json!({ "site": site, "site_name": name, "role": role, "redirect": "/os" }))
    }
}

async fn people_js() -> Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/javascript; charset=utf-8",
        )],
        include_str!("../static/people.js"),
    )
        .into_response()
}

/// Refuses a POST from any origin but the platform's.
async fn same_origin(State(team): State<Team>, request: Request, next: Next) -> Response {
    if request.method() == Method::POST {
        let headers = request.headers();
        let ok = match headers.get(ORIGIN).and_then(|v| v.to_str().ok()) {
            Some(origin) => origin.trim_end_matches('/') == team.inner.platform.origin,
            None => {
                headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("same-origin")
            }
        };
        if !ok {
            return TeamError::CrossOrigin.into_response();
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Debug, Default, Deserialize)]
struct Proof {
    challenge: Option<String>,
    passkey: Option<Value>,
    totp: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InviteStart {
    email: String,
    role: String,
}

#[derive(Debug, Deserialize)]
struct RoleStart {
    role: String,
}

#[derive(Debug, Deserialize)]
struct Accept {
    token: String,
}

fn role_of(text: &str) -> Result<Role, TeamError> {
    Role::grantable(text)
        .ok_or_else(|| TeamError::BadRequest("Choose manager, viewer or billing.".into()))
}

async fn members(
    State(t): State<Team>,
    Path(site): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, TeamError> {
    Ok(Json(t.list(&SiteId::new(site), &headers).await?))
}

async fn invite_start(
    State(t): State<Team>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(b): Json<InviteStart>,
) -> Result<Json<Value>, TeamError> {
    let email = crate::visitor::email_of(&b.email)
        .map_err(|_| TeamError::BadRequest("That doesn't look like an email address.".into()))?;
    let role = role_of(&b.role)?;
    Ok(Json(
        t.start(&SiteId::new(site), &headers, Grant::Invite { email, role })
            .await?,
    ))
}

async fn invite(
    State(t): State<Team>,
    Path(site): Path<String>,
    headers: HeaderMap,
    Json(b): Json<Proof>,
) -> Result<Json<Value>, TeamError> {
    Ok(Json(
        json!({ "member": t.invite(&SiteId::new(site), &headers, b).await? }),
    ))
}

async fn role_start(
    State(t): State<Team>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(b): Json<RoleStart>,
) -> Result<Json<Value>, TeamError> {
    let site = SiteId::new(site);
    t.member(&site, &id).await?.ok_or(TeamError::NotFound)?;
    Ok(Json(
        t.start(
            &site,
            &headers,
            Grant::Change {
                member: id,
                role: role_of(&b.role)?,
            },
        )
        .await?,
    ))
}

async fn role_change(
    State(t): State<Team>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(b): Json<Proof>,
) -> Result<Json<Value>, TeamError> {
    Ok(Json(
        json!({ "member": t.change_role(&SiteId::new(site), &id, &headers, b).await? }),
    ))
}

async fn remove(
    State(t): State<Team>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, TeamError> {
    Ok(Json(
        json!({ "removed": t.remove_member(&SiteId::new(site), &id, &headers).await? }),
    ))
}

async fn invite_view(
    State(t): State<Team>,
    Path(token): Path<String>,
) -> Result<Json<Value>, TeamError> {
    let (_, site, email, role, _) = t.invite_by_token(&token).await?;
    let (_, name) = t.site(&SiteId::new(site)).await?;
    let role = Role::grantable(&role).unwrap_or(Role::Viewer);
    Ok(Json(json!({
        "site_name": name, "role": role, "role_words": role.words(), "inviter": format!("The owner of {name}"),
        "email_hint": crate::visitor::mask_email(&email),
    })))
}

async fn accept(
    State(t): State<Team>,
    headers: HeaderMap,
    Json(b): Json<Accept>,
) -> Result<Json<Value>, TeamError> {
    Ok(Json(t.accept(&b.token, &headers).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_words_keep_the_shape_messages_reads() {
        // agentrix-messages (identity.rs, role_facts) reads the part before ", who " to say what
        // each role can and can't do in the invitation. Change both together.
        assert!(Role::Manager.words().starts_with("a manager, who "));
        assert!(Role::Viewer.words().starts_with("a viewer, who "));
        assert!(Role::Billing.words().starts_with("billing, who "));
    }

    #[test]
    fn roles() {
        assert_eq!(Role::grantable("owner"), None, "ownership isn't granted");
        assert!(
            Role::Manager.can_decide() && !Role::Viewer.can_decide() && !Role::Billing.can_decide()
        );
        assert!(Role::Billing.can_pay() && !Role::Manager.can_pay());
    }
}
