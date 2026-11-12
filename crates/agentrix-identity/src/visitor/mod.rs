//! Customer accounts on each site's own host (01a §2.4 A-01 to A-22, 06 §4.10).
//!
//! - **A passkey first**, with the site's host as the relying party: a passkey made on Kilele
//!   never works on a look-alike.
//! - **Without one, an email with a 6-digit code.** No magic links. A code works once, for 10
//!   minutes, with five tries.
//! - **Recovery codes** at sign-up, shown once (A-06); a lost device gets back in with one of
//!   them or with an email code (A-16), then adds a new passkey.
//! - **Sessions and devices**: a host-only cookie per site; the security page lists devices,
//!   sessions and sign-ins, warns about a new device, and signs out everywhere (A-09).
//! - **Consent receipts** for each optional use, signed as platform receipts (A-08).
//! - Big changes need a sign-in in the last 10 minutes (A-10, A-19).
//!
//! Desks never see any of this: W2's pages read [`Visitor`], which carries no credential.

mod consent;
mod passkeys;
pub mod rights;
mod routes;
mod session;
pub mod store;
mod views;

use std::sync::Arc;
use std::time::Duration;

use agentrix_types::tenancy::{SiteHost, VisitorAccountId};
use agentrix_types::{Digest, ReceiptBody, ReceiptKind, SiteId, Subject, canonical_json};
use axum::Json;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use rand::Rng;
use serde::Serialize;
use subtle::ConstantTimeEq;
use uuid::Uuid;
use webauthn_rs::prelude::{
    CreationChallengeResponse, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, RequestChallengeResponse,
};

pub(crate) use consent::when as consent_when;
pub use consent::{ConsentReceipt, Purpose, PurposeWords};
pub use routes::router;
pub use session::SESSION_SECONDS;

use crate::device;
use crate::mailer::{CodeMailer, CodePurpose, Mail, MailKind, is_reserved_example};
use crate::pending::Pending;
use crate::shell::SiteShell;
use crate::throttle::Throttle;
use store::{AccountRow, Store, StoreError};

/// A code works for this long.
pub const CODE_MINUTES: u32 = 10;
/// Wrong tries before a code stops working.
pub const CODE_TRIES: u8 = 5;
/// Codes sent to one email on one site in [`SEND_WINDOW`].
pub const SENDS_PER_WINDOW: usize = 5;
/// The window for [`SENDS_PER_WINDOW`] and failed sign-ins.
pub const SEND_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failed sign-ins (passkey or recovery code) per email per site in [`SEND_WINDOW`].
pub const FAILURES_PER_WINDOW: usize = 10;
/// Recovery codes made at sign-up.
pub const RECOVERY_CODES: usize = 8;
/// A sign-in this recent counts as fresh for big changes (seconds).
pub const FRESH_SECONDS: i64 = 10 * 60;

/// The customer signed in on this site, as W2's pages see them. No credential, no method.
#[derive(Debug, Clone, Serialize)]
pub struct Visitor {
    /// Their account on this site.
    pub account: VisitorAccountId,
    /// The site.
    pub site: SiteId,
    /// The name receipts use.
    pub name: String,
    /// The email for receipts.
    pub email: String,
    /// Proven by a code (ordering needs it, A-15).
    pub email_verified: bool,
    /// Phone for order updates, if given.
    pub phone: Option<String>,
    #[serde(skip)]
    pub(crate) session: SessionRef,
}

/// The session behind a [`Visitor`], kept for sign-out and freshness checks.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionRef {
    pub(crate) id: String,
    pub(crate) authenticated_at: i64,
}

impl Visitor {
    /// Whether they signed in within the last 10 minutes (big changes need it).
    pub fn is_fresh(&self) -> bool {
        crate::now() - self.session.authenticated_at <= FRESH_SECONDS
    }

    /// The first letter of their name, for the account chip.
    pub fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
    }
}

/// Errors, each answered with `{message}` in plain words (voice.md rule 5).
#[derive(Debug, thiserror::Error)]
pub enum VisitorError {
    /// Not a site host: no `SiteHost` extension on the request.
    #[error("That page isn't on this site.")]
    NotASite,
    /// Not signed in.
    #[error("Sign in first.")]
    SignedOut,
    /// Signed in too long ago for a big change.
    #[error("Sign in again to change this.")]
    SignInAgain,
    /// Bad input; the message says what.
    #[error("{0}")]
    BadRequest(String),
    /// Creating an account with an email that has one.
    #[error("An account with this email already exists. Sign in instead.")]
    EmailTaken,
    /// No account for this email on this site.
    #[error("There's no account for this email here yet.")]
    NoAccount,
    /// The account has no passkey.
    #[error("This account has no passkey yet. We can email you a code instead.")]
    NoPasskey,
    /// The browser's passkey didn't check out.
    #[error("That passkey didn't work. Try again, or get a code by email.")]
    PasskeyRefused,
    /// Wrong code.
    #[error("That code didn't match. Check the newest email and try again.")]
    WrongCode,
    /// Wrong recovery code.
    #[error("That recovery code didn't work. Each code works once.")]
    WrongRecoveryCode,
    /// The step expired or was used.
    #[error("That code has expired. Send a new one.")]
    Expired,
    /// Too many tries or sends.
    #[error("Too many tries. Wait 15 minutes, then try again.")]
    TooManyTries,
    /// The email couldn't be sent.
    #[error("We couldn't send the email just now. Try again in a minute, or use a passkey.")]
    MailFailed,
    /// A POST from another origin.
    #[error("This request came from another site, so it was refused.")]
    CrossOrigin,
    /// A fault on our side; the detail is logged, not shown.
    #[error("Something went wrong on our side. Nothing changed. Try again in a minute.")]
    Internal(String),
}

impl VisitorError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotASite | Self::NoAccount | Self::NoPasskey => StatusCode::NOT_FOUND,
            Self::SignedOut | Self::PasskeyRefused | Self::WrongCode | Self::WrongRecoveryCode => {
                StatusCode::UNAUTHORIZED
            }
            Self::SignInAgain | Self::CrossOrigin => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::EmailTaken => StatusCode::CONFLICT,
            Self::Expired => StatusCode::GONE,
            Self::TooManyTries => StatusCode::TOO_MANY_REQUESTS,
            Self::MailFailed => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// A short machine code for the page's script.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotASite => "not-a-site",
            Self::SignedOut => "signed-out",
            Self::SignInAgain => "sign-in-again",
            Self::BadRequest(_) => "bad-request",
            Self::EmailTaken => "email-taken",
            Self::NoAccount => "no-account",
            Self::NoPasskey => "no-passkey",
            Self::PasskeyRefused => "passkey-refused",
            Self::WrongCode => "wrong-code",
            Self::WrongRecoveryCode => "wrong-recovery-code",
            Self::Expired => "expired",
            Self::TooManyTries => "too-many-tries",
            Self::MailFailed => "mail-failed",
            Self::CrossOrigin => "cross-origin",
            Self::Internal(_) => "internal",
        }
    }
}

impl IntoResponse for VisitorError {
    fn into_response(self) -> Response {
        if let Self::Internal(detail) = &self {
            tracing::error!(%detail, "identity: visitor internal error");
        }
        (
            self.status(),
            Json(serde_json::json!({ "message": self.to_string(), "code": self.code() })),
        )
            .into_response()
    }
}

impl From<StoreError> for VisitorError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::EmailTaken => Self::EmailTaken,
            other => Self::Internal(other.to_string()),
        }
    }
}

// ---- state between steps ----------------------------------------------------------------

#[derive(Debug)]
struct NewAccount {
    email: String,
    name: String,
    phone: Option<String>,
    user_handle: Uuid,
}

#[derive(Debug)]
struct PendingRegistration {
    site: SiteId,
    /// Adding a passkey to this account; `None` creates `new`.
    account: Option<VisitorAccountId>,
    new: Option<NewAccount>,
    state: PasskeyRegistration,
    next: String,
}

#[derive(Debug)]
struct PendingLogin {
    site: SiteId,
    account: VisitorAccountId,
    email: String,
    state: PasskeyAuthentication,
    next: String,
}

#[derive(Debug)]
struct PendingCode {
    site: SiteId,
    email: String,
    purpose: CodePurpose,
    name: Option<String>,
    phone: Option<String>,
    code_hash: [u8; 32],
    tries: u8,
    next: String,
    /// The code itself, only for a reserved example address on a sample site whose mailer
    /// doesn't reach inboxes: the code page shows it as "Sample mailbox".
    sample_mailbox: Option<String>,
}

/// What a form had in it, kept on the server for a redirect so the address never travels in a
/// URL (logs, history, Referer). The page reads it by an unguessable id, for 10 minutes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Prefill {
    pub(crate) site: Option<SiteId>,
    pub(crate) email: String,
    pub(crate) name: String,
    pub(crate) phone: String,
}

#[derive(Debug)]
struct ShownCodes {
    site: SiteId,
    account: VisitorAccountId,
    codes: Vec<String>,
    next: String,
}

/// What a finished step tells the browser.
#[derive(Debug, Serialize)]
pub struct Done {
    /// Where to go now.
    pub redirect: String,
    /// An account was created.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub created: bool,
}

/// A step that starts a session: the reply plus the cookie to set and the cookie it replaces.
pub(crate) struct SignedIn {
    pub(crate) done: Done,
    pub(crate) cookie: HeaderValue,
}

/// A code was sent.
#[derive(Debug, Serialize)]
pub struct CodeSent {
    /// The pending step, for `/account/code?p=`.
    pub pending: String,
    /// The address, partly hidden: "nj***@example.com".
    pub sent_to: String,
    /// Where the code page is.
    pub redirect: String,
}

/// Customer accounts for every site host. Cheap to clone.
#[derive(Clone)]
pub struct Visitors {
    inner: Arc<Inner>,
}

struct Inner {
    store: Store,
    mailer: Arc<dyn CodeMailer>,
    shell: Arc<dyn SiteShell>,
    receipts: Option<Arc<dyn agentrix_host::Receipts>>,
    subject_stores: std::sync::RwLock<Vec<Arc<dyn adk_core::subject::SubjectDataStore>>>,
    rps: passkeys::RelyingParties,
    registrations: Pending<PendingRegistration>,
    logins: Pending<PendingLogin>,
    codes: Pending<PendingCode>,
    shown: Pending<ShownCodes>,
    prefills: Pending<Prefill>,
    sends: Throttle,
    failures: Throttle,
    views: views::Views,
}

impl std::fmt::Debug for Visitors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Visitors")
            .field("receipts", &self.inner.receipts.is_some())
            .finish_non_exhaustive()
    }
}

impl Visitors {
    /// Over `pool` (migrated with [`crate::migrate`]), sending codes with `mailer` and wrapping
    /// pages in `shell`.
    pub fn new(
        pool: sqlx::SqlitePool,
        mailer: Arc<dyn CodeMailer>,
        shell: Arc<dyn SiteShell>,
    ) -> Self {
        let ten = Duration::from_secs(u64::from(CODE_MINUTES) * 60);
        Self {
            inner: Arc::new(Inner {
                store: Store::new(pool),
                mailer,
                shell,
                receipts: None,
                subject_stores: std::sync::RwLock::new(Vec::new()),
                rps: passkeys::RelyingParties::default(),
                registrations: Pending::new(ten, 4096),
                logins: Pending::new(Duration::from_secs(300), 4096),
                codes: Pending::new(ten, 8192),
                shown: Pending::new(Duration::from_secs(30 * 60), 1024),
                prefills: Pending::new(Duration::from_secs(10 * 60), 4096),
                sends: Throttle::new(SENDS_PER_WINDOW, SEND_WINDOW),
                failures: Throttle::new(FAILURES_PER_WINDOW, SEND_WINDOW),
                views: views::Views::new(),
            }),
        }
    }

    /// For a node module: `control.db`, node's receipts (they sign consent receipts with
    /// control's key), the log-only mailer until W9's is wired with [`Visitors::with_mailer`],
    /// and `shell` for the site's look.
    pub fn for_node(ctx: &agentrix_host::NodeContext, shell: Arc<dyn SiteShell>) -> Self {
        Self::new(ctx.pool.clone(), Arc::new(crate::LogMailer), shell)
            .with_receipts(ctx.receipts.clone())
    }

    /// Issues platform receipts for consents through `receipts`.
    pub fn with_receipts(self, receipts: Arc<dyn agentrix_host::Receipts>) -> Self {
        self.rebuild(|inner| inner.receipts = Some(receipts))
    }

    /// Sends codes and notices with `mailer` (W9's driver).
    pub fn with_mailer(self, mailer: Arc<dyn CodeMailer>) -> Self {
        self.rebuild(|inner| inner.mailer = mailer)
    }

    /// Adds another workstream's subject-data store (W2's orders and customers, W1's account
    /// memory), so "Download my data" and "Delete account" reach it. Stores run in the order
    /// added, and this crate's own store runs last, because it holds the account.
    /// Works on a shared `Visitors` too: every clone sees the store.
    pub fn with_subject_store(self, store: Arc<dyn adk_core::subject::SubjectDataStore>) -> Self {
        self.inner
            .subject_stores
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .push(store);
        self
    }

    /// The names of the other workstreams' stores, for the data page.
    pub(crate) fn other_store_names(&self) -> Vec<String> {
        self.inner
            .subject_stores
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|s| s.name().to_owned())
            .collect()
    }

    fn rebuild(self, change: impl FnOnce(&mut Inner)) -> Self {
        let mut inner = Arc::into_inner(self.inner)
            .expect("Visitors is configured once, while building, before it is cloned");
        change(&mut inner);
        Self {
            inner: Arc::new(inner),
        }
    }

    /// The routes to merge into every site host's router (see [`router`]).
    pub fn router(&self) -> axum::Router {
        routes::router(self.clone())
    }

    /// The store, for tests and the coordinator's checks.
    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    // ---- who is signed in -----------------------------------------------------------------

    /// The customer signed in on `site` behind these headers, if any.
    pub async fn visitor(
        &self,
        site: &SiteHost,
        headers: &HeaderMap,
    ) -> Result<Option<Visitor>, VisitorError> {
        let Some(token) = session::token_from(headers, site.is_https()) else {
            return Ok(None);
        };
        let hash = session::token_hash(&token);
        let Some(row) = self.inner.store.session(&site.site, &hash).await? else {
            return Ok(None);
        };
        let Some(account) = self.inner.store.account(&site.site, &row.account).await? else {
            return Ok(None);
        };
        Ok(Some(Visitor {
            account: account.id,
            site: account.site,
            name: account.name,
            email: account.email,
            email_verified: account.email_verified_at.is_some(),
            phone: account.phone,
            session: SessionRef {
                id: row.id,
                authenticated_at: row.authenticated_at,
            },
        }))
    }

    /// An account by id on a site (for W2's order pages and receipts).
    pub async fn account(
        &self,
        site: &SiteId,
        id: &VisitorAccountId,
    ) -> Result<Option<AccountRow>, VisitorError> {
        Ok(self.inner.store.account(site, id).await?)
    }

    async fn start_session(
        &self,
        site: &SiteHost,
        account: &AccountRow,
        method: &str,
        headers: &HeaderMap,
    ) -> Result<HeaderValue, VisitorError> {
        // A sign-in on a browser that already holds a session replaces it.
        if let Some(old) = session::token_from(headers, site.is_https()) {
            self.inner
                .store
                .end_session_by_token(&site.site, &session::token_hash(&old))
                .await?;
        }
        let device = device::describe(
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
        );
        let (token, hash) = session::new_token();
        self.inner
            .store
            .create_session(
                &site.site,
                &account.id,
                &hash,
                method,
                &device,
                crate::now() + session::SESSION_SECONDS,
            )
            .await?;
        let new_device = self
            .inner
            .store
            .record_sign_in(&site.site, &account.id, method, &device)
            .await?;
        if new_device {
            let mail = Mail {
                site: site.clone(),
                to: account.email.clone(),
                kind: MailKind::NewSignIn {
                    device: device.clone(),
                    when: consent::when(crate::now()),
                },
            };
            if let Err(e) = self.inner.mailer.send(mail).await {
                tracing::warn!(error = %e, "identity: new sign-in notice not sent");
            }
        }
        Ok(session::set_cookie(site.is_https(), &token))
    }

    // ---- passkeys -------------------------------------------------------------------------

    /// Starts making a passkey: a new account from `new`, or another passkey for `signed_in`.
    pub(crate) async fn passkey_register_start(
        &self,
        site: &SiteHost,
        signed_in: Option<&Visitor>,
        new: Option<(String, String, Option<String>)>,
        next: String,
    ) -> Result<(String, CreationChallengeResponse), VisitorError> {
        let rp = self.inner.rps.for_site(site)?;
        let (pending, options) = match signed_in {
            Some(visitor) => {
                let account = self
                    .inner
                    .store
                    .account(&site.site, &visitor.account)
                    .await?
                    .ok_or(VisitorError::SignedOut)?;
                let existing: Vec<_> = self
                    .inner
                    .store
                    .credentials(&site.site, &account.id)
                    .await?
                    .into_iter()
                    .map(|c| c.passkey)
                    .collect();
                let (options, state) = rp.start_registration(
                    account.user_handle,
                    &account.email,
                    &account.name,
                    &existing,
                )?;
                (
                    PendingRegistration {
                        site: site.site.clone(),
                        account: Some(account.id),
                        new: None,
                        state,
                        next,
                    },
                    options,
                )
            }
            None => {
                let (name, email, phone) = new.ok_or_else(|| {
                    VisitorError::BadRequest("Add your name and email first.".into())
                })?;
                let email = email_of(&email)?;
                let name = name_of(&name)?;
                let phone = phone_of(phone.as_deref())?;
                if self
                    .inner
                    .store
                    .account_by_email(&site.site, &email)
                    .await?
                    .is_some()
                {
                    return Err(VisitorError::EmailTaken);
                }
                let user_handle = Uuid::new_v4();
                let (options, state) = rp.start_registration(user_handle, &email, &name, &[])?;
                let new = NewAccount {
                    email,
                    name,
                    phone,
                    user_handle,
                };
                (
                    PendingRegistration {
                        site: site.site.clone(),
                        account: None,
                        new: Some(new),
                        state,
                        next,
                    },
                    options,
                )
            }
        };
        Ok((self.inner.registrations.put(pending), options))
    }

    /// Finishes making a passkey. A new account signs in and goes to its recovery codes.
    pub(crate) async fn passkey_register_finish(
        &self,
        site: &SiteHost,
        pending: &str,
        credential: &RegisterPublicKeyCredential,
        headers: &HeaderMap,
    ) -> Result<(Done, Option<HeaderValue>), VisitorError> {
        let reg = self
            .inner
            .registrations
            .take(pending)
            .ok_or(VisitorError::Expired)?;
        if reg.site != site.site {
            return Err(VisitorError::Expired);
        }
        let rp = self.inner.rps.for_site(site)?;
        let passkey = rp
            .finish_registration(credential, &reg.state)
            .map_err(|e| {
                tracing::info!(error = %e, "identity: passkey registration refused");
                VisitorError::PasskeyRefused
            })?;
        let label = device::describe(
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
        );
        let cred_id = passkeys::credential_id_text(passkey.cred_id());
        match (reg.account, reg.new) {
            (Some(account), _) => {
                self.inner
                    .store
                    .add_credential(&site.site, &account, &cred_id, &passkey, &label)
                    .await?;
                Ok((
                    Done {
                        redirect: reg.next,
                        created: false,
                    },
                    None,
                ))
            }
            (None, Some(new)) => {
                let account = self
                    .inner
                    .store
                    .create_account(
                        &site.site,
                        &new.email,
                        &new.name,
                        new.phone.as_deref(),
                        new.user_handle,
                        false,
                    )
                    .await?;
                self.inner
                    .store
                    .add_credential(&site.site, &account.id, &cred_id, &passkey, &label)
                    .await?;
                let cookie = self
                    .start_session(site, &account, "passkey", headers)
                    .await?;
                let after = format!("/account/choices?next={}", enc(&reg.next));
                let redirect = self.issue_recovery_codes(site, &account.id, after).await?;
                Ok((
                    Done {
                        redirect,
                        created: true,
                    },
                    Some(cookie),
                ))
            }
            (None, None) => Err(VisitorError::Internal(
                "registration without account or new account".into(),
            )),
        }
    }

    /// Starts a passkey sign-in for `email`.
    pub(crate) async fn passkey_login_start(
        &self,
        site: &SiteHost,
        email: &str,
        next: String,
    ) -> Result<(String, RequestChallengeResponse), VisitorError> {
        let email = email_of(email)?;
        let key = throttle_key(site, &email);
        if !self.inner.failures.allowed(&key) {
            return Err(VisitorError::TooManyTries);
        }
        let account = self
            .inner
            .store
            .account_by_email(&site.site, &email)
            .await?
            .ok_or(VisitorError::NoAccount)?;
        let passkeys: Vec<_> = self
            .inner
            .store
            .credentials(&site.site, &account.id)
            .await?
            .into_iter()
            .map(|c| c.passkey)
            .collect();
        if passkeys.is_empty() {
            return Err(VisitorError::NoPasskey);
        }
        let rp = self.inner.rps.for_site(site)?;
        let (options, state) = rp.start_login(&passkeys)?;
        let pending = self.inner.logins.put(PendingLogin {
            site: site.site.clone(),
            account: account.id,
            email,
            state,
            next,
        });
        Ok((pending, options))
    }

    /// Finishes a passkey sign-in.
    pub(crate) async fn passkey_login_finish(
        &self,
        site: &SiteHost,
        pending: &str,
        credential: &PublicKeyCredential,
        headers: &HeaderMap,
    ) -> Result<SignedIn, VisitorError> {
        let login = self
            .inner
            .logins
            .take(pending)
            .ok_or(VisitorError::Expired)?;
        if login.site != site.site {
            return Err(VisitorError::Expired);
        }
        let key = throttle_key(site, &login.email);
        let rp = self.inner.rps.for_site(site)?;
        let result = match rp.finish(credential, &login.state) {
            Ok(result) => result,
            Err(e) => {
                tracing::info!(error = %e, "identity: passkey sign-in refused");
                self.inner.failures.record(&key);
                return Err(VisitorError::PasskeyRefused);
            }
        };
        self.inner.failures.clear(&key);
        let cred_id = passkeys::credential_id_text(result.cred_id());
        let mut stored = self
            .inner
            .store
            .credentials(&site.site, &login.account)
            .await?
            .into_iter()
            .find(|c| c.id == cred_id)
            .ok_or(VisitorError::PasskeyRefused)?;
        let changed = stored.passkey.update_credential(&result).unwrap_or(false);
        self.inner
            .store
            .touch_credential(&site.site, &cred_id, changed.then_some(&stored.passkey))
            .await?;
        let account = self
            .inner
            .store
            .account(&site.site, &login.account)
            .await?
            .ok_or(VisitorError::NoAccount)?;
        let cookie = self
            .start_session(site, &account, "passkey", headers)
            .await?;
        Ok(SignedIn {
            done: Done {
                redirect: login.next,
                created: false,
            },
            cookie,
        })
    }

    // ---- email codes ----------------------------------------------------------------------

    /// Sends a 6-digit code. `purpose` decides what a right code does.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn code_start(
        &self,
        site: &SiteHost,
        signed_in: Option<&Visitor>,
        email: &str,
        purpose: CodePurpose,
        name: Option<&str>,
        phone: Option<&str>,
        next: String,
    ) -> Result<CodeSent, VisitorError> {
        let email = match (purpose, signed_in) {
            (CodePurpose::Verify, Some(v)) => v.email.clone(),
            (CodePurpose::Verify, None) => return Err(VisitorError::SignedOut),
            _ => email_of(email)?,
        };
        let (name, phone) = match purpose {
            CodePurpose::Create => (Some(name_of(name.unwrap_or_default())?), phone_of(phone)?),
            _ => (None, None),
        };
        let exists = self
            .inner
            .store
            .account_by_email(&site.site, &email)
            .await?
            .is_some();
        match purpose {
            CodePurpose::Create if exists => return Err(VisitorError::EmailTaken),
            CodePurpose::SignIn | CodePurpose::Recover if !exists => {
                return Err(VisitorError::NoAccount);
            }
            _ => {}
        }
        let key = throttle_key(site, &email);
        if !self.inner.sends.allowed(&key) {
            return Err(VisitorError::TooManyTries);
        }
        self.inner.sends.record(&key);
        let code = format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000u32));
        let mail = Mail {
            site: site.clone(),
            to: email.clone(),
            kind: MailKind::Code {
                code: code.clone(),
                purpose,
                minutes: CODE_MINUTES,
            },
        };
        // A test customer agent on a sample site reads its code on the page; nothing is mailed
        // to its reserved address.
        let test_agent = site.sample && crate::mailer::is_test_agent(&email);
        if !test_agent {
            self.inner.mailer.send(mail).await.map_err(|e| {
                tracing::warn!(error = %e, "identity: code email failed");
                VisitorError::MailFailed
            })?;
        }
        let sample_mailbox = (test_agent
            || (site.sample && !self.inner.mailer.delivers() && is_reserved_example(&email)))
        .then(|| code.clone());
        let pending = self.inner.codes.put(PendingCode {
            site: site.site.clone(),
            email: email.clone(),
            purpose,
            name,
            phone,
            code_hash: code_hash(&code),
            tries: 0,
            next: next.clone(),
            sample_mailbox,
        });
        Ok(CodeSent {
            redirect: format!("/account/code?p={}", enc(&pending)),
            sent_to: mask_email(&email),
            pending,
        })
    }

    /// Sends a fresh code for the same step (the old one stops working).
    pub(crate) async fn code_resend(
        &self,
        site: &SiteHost,
        pending: &str,
    ) -> Result<CodeSent, VisitorError> {
        let old = self
            .inner
            .codes
            .take(pending)
            .ok_or(VisitorError::Expired)?;
        if old.site != site.site {
            return Err(VisitorError::Expired);
        }
        let key = throttle_key(site, &old.email);
        if !self.inner.sends.allowed(&key) {
            return Err(VisitorError::TooManyTries);
        }
        self.inner.sends.record(&key);
        let code = format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000u32));
        let mail = Mail {
            site: site.clone(),
            to: old.email.clone(),
            kind: MailKind::Code {
                code: code.clone(),
                purpose: old.purpose,
                minutes: CODE_MINUTES,
            },
        };
        self.inner
            .mailer
            .send(mail)
            .await
            .map_err(|_| VisitorError::MailFailed)?;
        let sample_mailbox = old.sample_mailbox.is_some().then(|| code.clone());
        let email = old.email.clone();
        let pending = self.inner.codes.put(PendingCode {
            code_hash: code_hash(&code),
            tries: 0,
            sample_mailbox,
            ..old
        });
        Ok(CodeSent {
            redirect: format!("/account/code?p={}", enc(&pending)),
            sent_to: mask_email(&email),
            pending,
        })
    }

    /// What the code page shows about a pending code: the masked address, why, and the sample
    /// mailbox line if any. `None` if the step expired.
    pub(crate) fn code_view(
        &self,
        site: &SiteHost,
        pending: &str,
    ) -> Option<(String, CodePurpose, Option<String>, bool)> {
        self.inner
            .codes
            .with(pending, |p| {
                (p.site == site.site).then(|| {
                    (
                        mask_email(&p.email),
                        p.purpose,
                        p.sample_mailbox.clone(),
                        crate::mailer::is_test_agent(&p.email),
                    )
                })
            })
            .flatten()
    }

    /// Checks a code. Right: signs in (or creates the account, or proves the email).
    pub(crate) async fn code_verify(
        &self,
        site: &SiteHost,
        signed_in: Option<&Visitor>,
        pending: &str,
        code: &str,
        headers: &HeaderMap,
        passkey_declined: bool,
    ) -> Result<(Done, Option<HeaderValue>), VisitorError> {
        let typed: String = code.chars().filter(char::is_ascii_digit).collect();
        if typed.len() != 6 {
            return Err(VisitorError::BadRequest(
                "Type the 6 digits from the email.".into(),
            ));
        }
        let checked = self.inner.codes.with(pending, |p| {
            if p.site != site.site {
                return Err(VisitorError::Expired);
            }
            if p.tries >= CODE_TRIES {
                return Err(VisitorError::TooManyTries);
            }
            if bool::from(p.code_hash.ct_eq(&code_hash(&typed))) {
                Ok(())
            } else {
                p.tries += 1;
                Err(if p.tries >= CODE_TRIES {
                    VisitorError::TooManyTries
                } else {
                    VisitorError::WrongCode
                })
            }
        });
        match checked {
            None => return Err(VisitorError::Expired),
            Some(Err(e)) => {
                if matches!(e, VisitorError::TooManyTries) {
                    self.inner.codes.remove(pending);
                }
                return Err(e);
            }
            Some(Ok(())) => {}
        }
        let step = self
            .inner
            .codes
            .take(pending)
            .ok_or(VisitorError::Expired)?;
        self.inner.sends.clear(&throttle_key(site, &step.email));
        match step.purpose {
            CodePurpose::Create => {
                let name = step
                    .name
                    .clone()
                    .unwrap_or_else(|| step.email.split('@').next().unwrap_or_default().to_owned());
                let account = self
                    .inner
                    .store
                    .create_account(
                        &site.site,
                        &step.email,
                        &name,
                        step.phone.as_deref(),
                        Uuid::new_v4(),
                        true,
                    )
                    .await?;
                let cookie = self
                    .start_session(site, &account, "email-code", headers)
                    .await?;
                let after_choices = format!("/account/choices?next={}", enc(&step.next));
                let after = if passkey_declined {
                    after_choices
                } else {
                    format!("/account/passkey-offer?next={}", enc(&after_choices))
                };
                let redirect = self.issue_recovery_codes(site, &account.id, after).await?;
                Ok((
                    Done {
                        redirect,
                        created: true,
                    },
                    Some(cookie),
                ))
            }
            CodePurpose::Verify => {
                let visitor = signed_in.ok_or(VisitorError::SignedOut)?;
                if !visitor.email.eq_ignore_ascii_case(&step.email) {
                    return Err(VisitorError::Expired);
                }
                self.inner
                    .store
                    .verify_email(&site.site, &visitor.account)
                    .await?;
                Ok((
                    Done {
                        redirect: step.next,
                        created: false,
                    },
                    None,
                ))
            }
            CodePurpose::SignIn | CodePurpose::Recover => {
                let account = self
                    .inner
                    .store
                    .account_by_email(&site.site, &step.email)
                    .await?
                    .ok_or(VisitorError::NoAccount)?;
                self.inner
                    .store
                    .verify_email(&site.site, &account.id)
                    .await?;
                let cookie = self
                    .start_session(site, &account, "email-code", headers)
                    .await?;
                let has_passkey = !self
                    .inner
                    .store
                    .credentials(&site.site, &account.id)
                    .await?
                    .is_empty();
                let redirect = if step.purpose == CodePurpose::Recover {
                    "/account/security?recovered=1".to_owned()
                } else if !has_passkey && !passkey_declined {
                    format!("/account/passkey-offer?next={}", enc(&step.next))
                } else {
                    step.next
                };
                Ok((
                    Done {
                        redirect,
                        created: false,
                    },
                    Some(cookie),
                ))
            }
        }
    }

    // ---- recovery codes -------------------------------------------------------------------

    async fn issue_recovery_codes(
        &self,
        site: &SiteHost,
        account: &VisitorAccountId,
        next: String,
    ) -> Result<String, VisitorError> {
        let codes = new_recovery_codes();
        self.inner
            .store
            .set_recovery_codes(&site.site, account, &codes)
            .await?;
        let id = self.inner.shown.put(ShownCodes {
            site: site.site.clone(),
            account: account.clone(),
            codes,
            next,
        });
        Ok(format!("/account/recovery-codes?r={}", enc(&id)))
    }

    /// New recovery codes for a freshly signed-in customer; the old ones stop working.
    pub(crate) async fn recovery_new(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
    ) -> Result<Done, VisitorError> {
        if !visitor.is_fresh() {
            return Err(VisitorError::SignInAgain);
        }
        let redirect = self
            .issue_recovery_codes(site, &visitor.account, "/account/security".into())
            .await?;
        Ok(Done {
            redirect,
            created: false,
        })
    }

    /// Takes the codes to show once.
    pub(crate) fn take_shown_codes(
        &self,
        site: &SiteHost,
        visitor: Option<&Visitor>,
        id: &str,
    ) -> Option<(Vec<String>, String)> {
        let shown = self.inner.shown.take(id)?;
        let mine = shown.site == site.site && visitor.is_some_and(|v| v.account == shown.account);
        mine.then_some((shown.codes, shown.next))
    }

    /// Signs in with a recovery code (A-16).
    pub(crate) async fn recovery_use(
        &self,
        site: &SiteHost,
        email: &str,
        code: &str,
        headers: &HeaderMap,
    ) -> Result<SignedIn, VisitorError> {
        let email = email_of(email)?;
        let key = throttle_key(site, &email);
        if !self.inner.failures.allowed(&key) {
            return Err(VisitorError::TooManyTries);
        }
        let normal = normalise_recovery(code);
        let Some(account) = self
            .inner
            .store
            .account_by_email(&site.site, &email)
            .await?
        else {
            self.inner.failures.record(&key);
            return Err(VisitorError::WrongRecoveryCode);
        };
        if !self
            .inner
            .store
            .use_recovery_code(&site.site, &account.id, &normal)
            .await?
        {
            self.inner.failures.record(&key);
            return Err(VisitorError::WrongRecoveryCode);
        }
        self.inner.failures.clear(&key);
        let cookie = self
            .start_session(site, &account, "recovery-code", headers)
            .await?;
        let remaining = self
            .inner
            .store
            .recovery_codes_left(&site.site, &account.id)
            .await?;
        let device = device::describe(
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
        );
        let mail = Mail {
            site: site.clone(),
            to: account.email.clone(),
            kind: MailKind::RecoveryCodeUsed { device, remaining },
        };
        if let Err(e) = self.inner.mailer.send(mail).await {
            tracing::warn!(error = %e, "identity: recovery notice not sent");
        }
        Ok(SignedIn {
            done: Done {
                redirect: "/account/security?recovered=1".into(),
                created: false,
            },
            cookie,
        })
    }

    // ---- sessions, passkeys, alerts -------------------------------------------------------

    /// Ends one session, or every session (this one too).
    pub(crate) async fn end_sessions(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
        id: Option<&str>,
    ) -> Result<u64, VisitorError> {
        match id {
            Some(id) => Ok(u64::from(
                self.inner
                    .store
                    .end_session(&site.site, &visitor.account, id)
                    .await?,
            )),
            None => {
                let ended = self
                    .inner
                    .store
                    .end_all_sessions(&site.site, &visitor.account)
                    .await?;
                self.inner
                    .store
                    .dismiss_alerts(&site.site, &visitor.account)
                    .await?;
                Ok(ended)
            }
        }
    }

    /// Signs this browser out.
    pub(crate) async fn sign_out(
        &self,
        site: &SiteHost,
        headers: &HeaderMap,
    ) -> Result<(), VisitorError> {
        if let Some(token) = session::token_from(headers, site.is_https()) {
            self.inner
                .store
                .end_session_by_token(&site.site, &session::token_hash(&token))
                .await?;
        }
        Ok(())
    }

    /// Removes a passkey (needs a fresh sign-in). An email code always still works.
    pub(crate) async fn remove_passkey(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
        id: &str,
    ) -> Result<bool, VisitorError> {
        if !visitor.is_fresh() {
            return Err(VisitorError::SignInAgain);
        }
        Ok(self
            .inner
            .store
            .remove_credential(&site.site, &visitor.account, id)
            .await?)
    }

    /// "It was me".
    pub(crate) async fn dismiss_alerts(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
    ) -> Result<(), VisitorError> {
        Ok(self
            .inner
            .store
            .dismiss_alerts(&site.site, &visitor.account)
            .await?)
    }

    // ---- consent receipts -----------------------------------------------------------------

    /// Turns a purpose on (a new receipt) or off (the live receipt is withdrawn and kept).
    pub(crate) async fn set_consent(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
        purpose: Purpose,
        on: bool,
        adult: bool,
    ) -> Result<Option<ConsentReceipt>, VisitorError> {
        if !on {
            if let Some(id) = self
                .inner
                .store
                .live_consent(&site.site, &visitor.account, purpose.key())
                .await?
            {
                let receipt = self
                    .issue_consent_receipt(
                        site,
                        ReceiptKind::ConsentWithdrawn,
                        &visitor.account,
                        &id,
                        purpose,
                        None,
                    )
                    .await;
                self.inner
                    .store
                    .withdraw_consent(
                        &site.site,
                        &visitor.account,
                        purpose.key(),
                        receipt.as_deref(),
                    )
                    .await?;
            }
            return Ok(None);
        }
        if purpose.needs_adult() && !adult {
            return Err(VisitorError::BadRequest(
                "Account memory is for people 18 or over. Tick the box to say you are.".into(),
            ));
        }
        if purpose == Purpose::OrderUpdates && visitor.phone.is_none() {
            return Err(VisitorError::BadRequest(
                "Add a phone number in your details first.".into(),
            ));
        }
        // Already on: keep the live receipt.
        let live = self
            .inner
            .store
            .consents(&site.site, &visitor.account)
            .await?;
        if let Some(row) = live
            .iter()
            .find(|r| r.purpose == purpose.key() && r.withdrawn_at.is_none())
        {
            return serde_json::from_str(&row.body)
                .map(Some)
                .map_err(|e| VisitorError::Internal(e.to_string()));
        }
        let words = consent::words(
            purpose,
            site,
            visitor.phone.as_deref(),
            self.inner.shell.offers_line(site).as_deref(),
        );
        let given = crate::now();
        let receipt = ConsentReceipt {
            kind: "agentrix.consent.v1".into(),
            id: format!("cr_{}", Uuid::now_v7().simple()),
            site: site.site.to_string(),
            host: site.host.clone(),
            subject: visitor.account.clone(),
            purpose: purpose.key().into(),
            purpose_text: words.purpose,
            what: words.what,
            shared_with: words.shared_with,
            kept_for: words.kept_for,
            to_withdraw: words.to_withdraw,
            given_at: chrono::DateTime::from_timestamp(given, 0)
                .unwrap_or_default()
                .to_rfc3339(),
            adult_confirmed: purpose.needs_adult().then_some(true),
        };
        let body = canonical_json(&receipt).map_err(|e| VisitorError::Internal(e.to_string()))?;
        let body_text =
            String::from_utf8(body.clone()).map_err(|e| VisitorError::Internal(e.to_string()))?;
        let body_digest = Digest::of(&body);
        let digest = body_digest.to_string();
        let receipt_digest = self
            .issue_consent_receipt(
                site,
                ReceiptKind::ConsentGiven,
                &visitor.account,
                &receipt.id,
                purpose,
                Some(body_digest),
            )
            .await;
        self.inner
            .store
            .add_consent(
                &site.site,
                &visitor.account,
                &receipt.id,
                purpose.key(),
                &body_text,
                &digest,
                receipt_digest.as_deref(),
                given,
            )
            .await?;
        Ok(Some(receipt))
    }

    /// Signs a `consent.given` or `consent.withdrawn` platform receipt with control's key. A
    /// failure is logged and the consent still stands, recorded by its body digest.
    async fn issue_consent_receipt(
        &self,
        site: &SiteHost,
        kind: ReceiptKind,
        account: &VisitorAccountId,
        consent_id: &str,
        purpose: Purpose,
        body: Option<Digest>,
    ) -> Option<String> {
        let receipts = self.inner.receipts.as_ref()?;
        let now = chrono::Utc::now();
        let mut receipt = ReceiptBody::new(
            kind,
            site.site.clone(),
            Subject::Site(site.site.clone()),
            now,
            now,
        );
        receipt.inputs = body.into_iter().collect();
        receipt.detail = Some(serde_json::json!({
            "consent": consent_id, "purpose": purpose.key(), "customer": account, "host": site.host,
        }));
        match receipts.issue(receipt).await {
            Ok(digest) => Some(digest),
            Err(e) => {
                tracing::warn!(error = %e, "identity: consent receipt not issued");
                None
            }
        }
    }

    // ---- form prefills (never an address in a URL) ----------------------------------------

    /// Keeps what a form had in it for the next page; returns the id for `?f=`. Empty forms
    /// get no id.
    pub(crate) fn prefill(
        &self,
        site: &SiteHost,
        email: &str,
        name: &str,
        phone: &str,
    ) -> Option<String> {
        if email.trim().is_empty() && name.trim().is_empty() && phone.trim().is_empty() {
            return None;
        }
        Some(self.inner.prefills.put(Prefill {
            site: Some(site.site.clone()),
            email: email.trim().chars().take(254).collect(),
            name: name.trim().chars().take(80).collect(),
            phone: phone.trim().chars().take(32).collect(),
        }))
    }

    /// What a form had in it, by `?f=` id, on this site only. Read, not taken, so a reload
    /// still shows it.
    pub(crate) fn prefilled(&self, site: &SiteHost, id: Option<&str>) -> Prefill {
        id.and_then(|id| {
            self.inner
                .prefills
                .with(id, |p| {
                    (p.site.as_ref() == Some(&site.site)).then(|| p.clone())
                })
                .flatten()
        })
        .unwrap_or_default()
    }

    // ---- export and erasure (A-11 to A-13) -------------------------------------------------

    /// Every store, this crate's last: other stores (W8's ControlSubjectData, W2's records)
    /// read the account's email and phone from `visitor_accounts` while they erase, so the
    /// account must still exist when they run. Keep this order.
    fn all_stores(&self) -> Vec<Arc<dyn adk_core::subject::SubjectDataStore>> {
        let mut stores = self
            .inner
            .subject_stores
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        stores.push(Arc::new(rights::VisitorSubjectData::new(
            self.inner.store.clone(),
        )));
        stores
    }

    /// Everything every registered store holds about this customer, as one JSON bundle.
    pub async fn export_bundle(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
    ) -> Result<serde_json::Value, VisitorError> {
        let scope = rights::scope(&site.site, &visitor.account);
        let mut stores = serde_json::Map::new();
        for store in self.all_stores() {
            let value = store.export(&scope).await.map_err(|e| {
                VisitorError::Internal(format!("export from {}: {e}", store.name()))
            })?;
            stores.insert(store.name().to_owned(), value);
        }
        Ok(serde_json::json!({
            "type": "agentrix.subject-export.v1",
            "site": site.site, "host": site.host, "subject": visitor.account,
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "stores": stores,
        }))
    }

    /// Erases this customer from every registered store (needs a fresh sign-in). A store that
    /// refuses (an open order) stops it before anything is erased. Returns the receipt's id.
    pub async fn delete_account(
        &self,
        site: &SiteHost,
        visitor: &Visitor,
    ) -> Result<String, VisitorError> {
        if !visitor.is_fresh() {
            return Err(VisitorError::SignInAgain);
        }
        let scope = rights::scope(&site.site, &visitor.account);
        let requested = crate::now();
        let mut lines = Vec::new();
        for store in self.all_stores() {
            let at = crate::now();
            let report = match store.erase(&scope).await {
                Ok(done) => adk_core::subject::StoreReport {
                    store: store.name().to_owned(),
                    outcome: match done.completes_by {
                        Some(completes_by) => adk_core::subject::StoreOutcome::Scheduled {
                            deleted: done.deleted,
                            retained: done.retained,
                            completes_by,
                        },
                        None => adk_core::subject::StoreOutcome::Erased {
                            deleted: done.deleted,
                            retained: done.retained,
                        },
                    },
                },
                Err(e) if e.code == rights::OPEN_ORDERS_CODE => {
                    return Err(VisitorError::BadRequest(
                        "You have an open order, so your account stays until it's delivered, collected or cancelled. Nothing was erased.".into(),
                    ));
                }
                Err(e) => adk_core::subject::StoreReport {
                    store: store.name().to_owned(),
                    outcome: adk_core::subject::StoreOutcome::Failed {
                        code: e.code.to_owned(),
                        message: e.message.clone(),
                    },
                },
            };
            let mut line = rights::line(&report);
            line["at"] = serde_json::json!(consent::when(at));
            lines.push(line);
        }
        let subject_hash = {
            use sha2::{Digest as _, Sha256};
            hex::encode(
                &Sha256::digest(format!("{}\0{}", site.site, visitor.account).as_bytes())[..12],
            )
        };
        let report = serde_json::json!({
            "type": "agentrix.erasure.v1", "site": site.site, "host": site.host, "subject": subject_hash,
            "requested_at": consent::when(requested), "stores": lines,
        });
        let digest = Digest::of(
            &canonical_json(&report).map_err(|e| VisitorError::Internal(e.to_string()))?,
        )
        .to_string();
        self.inner
            .store
            .record_erasure(&site.site, &subject_hash, &report, &digest)
            .await
            .map_err(|e| VisitorError::Internal(e.to_string()))
    }

    /// An erasure receipt by id.
    pub(crate) async fn erasure(
        &self,
        site: &SiteHost,
        id: &str,
    ) -> Result<Option<(serde_json::Value, String, i64)>, VisitorError> {
        self.inner
            .store
            .erasure(&site.site, id)
            .await
            .map_err(|e| VisitorError::Internal(e.to_string()))
    }

    /// Whether a purpose is on for this customer (W1's desks ask before using account memory).
    pub async fn has_consent(
        &self,
        site: &SiteId,
        account: &VisitorAccountId,
        purpose: Purpose,
    ) -> Result<bool, VisitorError> {
        let rows = self.inner.store.consents(site, account).await?;
        Ok(rows
            .iter()
            .any(|r| r.purpose == purpose.key() && r.withdrawn_at.is_none()))
    }
}

/// The `Visitor` extractor: 401 `{message}` when nobody is signed in on this site. Needs the
/// `SiteHost` extension and `Visitors` in the router state.
impl<S> FromRequestParts<S> for Visitor
where
    Visitors: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = VisitorError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let visitors = Visitors::from_ref(state);
        let site = parts
            .extensions
            .get::<SiteHost>()
            .cloned()
            .ok_or(VisitorError::NotASite)?;
        visitors
            .visitor(&site, &parts.headers)
            .await?
            .ok_or(VisitorError::SignedOut)
    }
}

/// A redirect to the site's sign-in page that comes back to `next`.
pub fn sign_in_redirect(next: &str) -> Redirect {
    Redirect::to(&format!("/account/sign-in?next={}", enc(next)))
}

// ---- small helpers ----------------------------------------------------------------------

/// SHA-256 of a code.
fn code_hash(code: &str) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    Sha256::digest(code.as_bytes()).into()
}

fn throttle_key(site: &SiteHost, email: &str) -> String {
    format!("{}\0{email}", site.site)
}

/// Lower-cased, trimmed and plausibly an address.
pub(crate) fn email_of(raw: &str) -> Result<String, VisitorError> {
    let email = raw.trim().to_lowercase();
    let ok = email.len() <= 254
        && email.split('@').count() == 2
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
        && !email.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(email)
    } else {
        Err(VisitorError::BadRequest(
            "That doesn't look like an email address.".into(),
        ))
    }
}

fn name_of(raw: &str) -> Result<String, VisitorError> {
    let name: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    if name.is_empty() {
        Err(VisitorError::BadRequest(
            "Add your name, so receipts say who they're for.".into(),
        ))
    } else {
        Ok(name)
    }
}

fn phone_of(raw: Option<&str>) -> Result<Option<String>, VisitorError> {
    let Some(raw) = raw.map(str::trim).filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let digits = raw.chars().filter(char::is_ascii_digit).count();
    let ok = (7..=15).contains(&digits)
        && raw
            .chars()
            .all(|c| c.is_ascii_digit() || " +-()".contains(c));
    if ok {
        Ok(Some(raw.to_owned()))
    } else {
        Err(VisitorError::BadRequest(
            "That phone number doesn't look right. Use digits, with + and the country code.".into(),
        ))
    }
}

/// "nj***@example.com".
pub(crate) fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let shown: String = local.chars().take(2).collect();
            format!("{shown}***@{domain}")
        }
        None => "your email".into(),
    }
}

/// Percent-encodes a path or token for a query string.
pub(crate) fn enc(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

const RECOVERY_ALPHABET: &[u8] = b"23456789abcdefghjkmnpqrstuvwxyz";

/// Eight codes like `4kx7-q2mp`: 8 characters from a 31-letter alphabet without look-alikes.
fn new_recovery_codes() -> Vec<String> {
    let mut rng = rand::rngs::OsRng;
    (0..RECOVERY_CODES)
        .map(|_| {
            let chars: String = (0..8)
                .map(|_| RECOVERY_ALPHABET[rng.gen_range(0..RECOVERY_ALPHABET.len())] as char)
                .collect();
            format!("{}-{}", &chars[..4], &chars[4..])
        })
        .collect()
}

/// Lower case, no spaces, one dash in the middle: what a person types, as stored.
fn normalise_recovery(code: &str) -> String {
    let chars: String = code
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if chars.len() == 8 {
        format!("{}-{}", &chars[..4], &chars[4..])
    } else {
        chars
    }
}

/// Only same-site paths: never an open redirect.
pub(crate) fn next_of(given: Option<&str>, default: &str) -> String {
    match given.map(str::trim) {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.starts_with("/\\")
                && !n.contains(['\r', '\n'])
                && n.len() <= 512 =>
        {
            n.to_owned()
        }
        _ => default.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_checked() {
        assert_eq!(
            email_of(" Njeri@Example.com ").unwrap(),
            "njeri@example.com"
        );
        assert!(email_of("njeri").is_err());
        assert!(name_of("  ").is_err());
        assert_eq!(
            phone_of(Some("+44 7700 900418")).unwrap().as_deref(),
            Some("+44 7700 900418")
        );
        assert!(phone_of(Some("call me")).is_err());
        assert_eq!(phone_of(Some("")).unwrap(), None);
        assert_eq!(mask_email("njeri@example.com"), "nj***@example.com");
    }

    #[test]
    fn next_is_same_site_only() {
        assert_eq!(next_of(Some("/cakes?thread=t1"), "/"), "/cakes?thread=t1");
        assert_eq!(next_of(Some("//evil.example"), "/"), "/");
        assert_eq!(next_of(Some("https://evil.example"), "/"), "/");
        assert_eq!(next_of(Some("/\\evil.example"), "/"), "/");
        assert_eq!(next_of(None, "/account"), "/account");
    }

    #[test]
    fn recovery_codes_look_right() {
        let codes = new_recovery_codes();
        assert_eq!(codes.len(), RECOVERY_CODES);
        for code in &codes {
            assert_eq!(code.len(), 9);
            assert_eq!(
                normalise_recovery(&code.to_uppercase().replace('-', " ")),
                *code
            );
        }
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
    }
}
