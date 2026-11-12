//! Owner recovery and sign-in by email code (01b §4.1, plan decision 8, James: "must have,
//! including email OTP"). JSON on the platform host; W4 draws the pages. The first release's `auth/` and
//! its approve contract stay untouched: accounts and sessions go through node's
//! [`OwnerAccounts`], step-up goes through the frozen [`Approvals`].
//!
//! The rules, and why:
//! - **Sign-up by email code** makes a new account with no password and signs it in.
//! - **Sign-in by email code** works only for an account with no passkey and no password: its
//!   email already is its only way in. An account with a passkey or an authenticator never gets
//!   a session from an email alone, because a signed-in owner can add a passkey (auth/'s
//!   register route) and a passkey approves cards.
//! - **A recovery code** (10 at sign-up, shown once, stored hashed, each good once) signs in.
//! - **Recovery by email** is held for 24 hours, with a notice; the owner can cancel it by
//!   signing in any strong way before then. After the hold, the same email code signs in.
//! - Every code sign-in records an event and an `account.code-sign-in` receipt and sends a
//!   notice (W9 routes it to the owner's other channels). No code ever approves anything:
//!   approvals stay passkey or authenticator, through the frozen contract.
//! - Codes: 6 digits, 10 minutes, once, hashed, five tries; sends limited per email and per IP.
//!
//! | Route | Body | Reply |
//! |---|---|---|
//! | `POST /auth/email/start` | `{email, purpose: signup\|signin\|recover, display_name?, next?}` | `{pending, sent_to}` |
//! | `POST /auth/email/verify` | `{pending, code}` | `{redirect, created?, recovery?}` + cookie, or `{held, hold, release_at}` |
//! | `GET /auth/api/recovery-codes/shown?r=` | | `{codes, next}` once, then 410 |
//! | `GET /auth/recovery-codes` | signed in | `{left}` |
//! | `POST /auth/recovery-codes/start` | signed in | `{challenge, passkey}` (passkey options or null) |
//! | `POST /auth/recovery-codes` | `{challenge, passkey?, totp?}` | `{codes}` |
//! | `POST /auth/recover/code` | `{email, code}` | `{redirect}` + cookie |
//! | `GET /auth/recover/holds` | signed in | `{holds: [{id, requested, release, device}]}` |
//! | `POST /auth/recover/cancel` | `{hold}` signed in | `{cancelled}` |

mod routes;
pub mod store;

use std::sync::Arc;
use std::time::Duration;

use agentrix_host::{Approvals, OwnerAccounts, OwnerSessions, Receipts};
use agentrix_types::tenancy::SiteHost;
use agentrix_types::{ReceiptBody, ReceiptKind, SiteId, Subject};
use axum::Json;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rand::Rng;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

pub use routes::router;

use crate::device;
use crate::mailer::{CodeMailer, CodePurpose, Mail, MailKind};
use crate::pending::Pending;
use crate::throttle::Throttle;
use store::OwnerStore;

/// Recovery codes made for an owner.
pub const OWNER_RECOVERY_CODES: usize = 10;
/// How long recovery by email is held (seconds).
pub const HOLD_SECONDS: i64 = 24 * 3600;
/// A code works for this long (minutes).
pub const CODE_MINUTES: u32 = 10;
/// Wrong tries before a code stops working.
pub const CODE_TRIES: u8 = 5;
/// Codes sent to one email in 15 minutes.
pub const SENDS_PER_EMAIL: usize = 5;
/// Codes sent from one IP in 15 minutes.
pub const SENDS_PER_IP: usize = 20;
/// Failed recovery codes per email in 15 minutes.
pub const FAILURES_PER_EMAIL: usize = 10;

const WINDOW: Duration = Duration::from_secs(15 * 60);

/// Why an owner asked for a code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerPurpose {
    /// A new account.
    Signup,
    /// Sign in (email-only accounts).
    Signin,
    /// Recovery by email (held).
    Recover,
}

impl OwnerPurpose {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "signup" => Some(Self::Signup),
            "signin" => Some(Self::Signin),
            "recover" => Some(Self::Recover),
            _ => None,
        }
    }
}

/// Errors, answered as `{message, code}` in plain words.
#[derive(Debug, thiserror::Error)]
pub enum OwnerError {
    /// Not signed in.
    #[error("Sign in first.")]
    SignedOut,
    /// Bad input.
    #[error("{0}")]
    BadRequest(String),
    /// Sign-up with an email that has an account.
    #[error("An account with this email already exists. Sign in instead.")]
    EmailTaken,
    /// No account for the email.
    #[error("There's no Agentrix account for this email. Check it for typos, or make an account.")]
    NoAccount,
    /// Email-code sign-in for an account that has a passkey or an authenticator.
    #[error(
        "This account signs in with a passkey or an authenticator app. Lost both? Use a recovery code, or recover by email: that waits 24 hours, so you can stop it if it isn't you."
    )]
    UseStrongerSignIn,
    /// Wrong code.
    #[error("That code didn't match. Check the newest email and try again.")]
    WrongCode,
    /// Wrong recovery code.
    #[error("That recovery code didn't work. Each code works once.")]
    WrongRecoveryCode,
    /// Expired or used.
    #[error("That code has expired. Send a new one.")]
    Expired,
    /// Too many.
    #[error("Too many tries. Wait 15 minutes, then try again.")]
    TooManyTries,
    /// Step-up proof missing or refused.
    #[error("Confirm it's you with your passkey or your authenticator code first.")]
    StepUpNeeded,
    /// The email didn't go out.
    #[error("We couldn't send the email just now. Try again in a minute.")]
    MailFailed,
    /// From another site.
    #[error("This request came from another site, so it was refused.")]
    CrossOrigin,
    /// Ours; logged, not shown.
    #[error("Something went wrong on our side. Nothing changed. Try again in a minute.")]
    Internal(String),
}

impl OwnerError {
    /// A short machine code for the page.
    pub fn code(&self) -> &'static str {
        match self {
            Self::SignedOut => "signed-out",
            Self::BadRequest(_) => "bad-request",
            Self::EmailTaken => "email-taken",
            Self::NoAccount => "no-account",
            Self::UseStrongerSignIn => "use-passkey",
            Self::WrongCode => "wrong-code",
            Self::WrongRecoveryCode => "wrong-recovery-code",
            Self::Expired => "expired",
            Self::TooManyTries => "too-many-tries",
            Self::StepUpNeeded => "step-up",
            Self::MailFailed => "mail-failed",
            Self::CrossOrigin => "cross-origin",
            Self::Internal(_) => "internal",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::SignedOut | Self::WrongCode | Self::WrongRecoveryCode => StatusCode::UNAUTHORIZED,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::EmailTaken | Self::UseStrongerSignIn => StatusCode::CONFLICT,
            Self::NoAccount => StatusCode::NOT_FOUND,
            Self::Expired => StatusCode::GONE,
            Self::TooManyTries => StatusCode::TOO_MANY_REQUESTS,
            Self::StepUpNeeded | Self::CrossOrigin => StatusCode::FORBIDDEN,
            Self::MailFailed => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for OwnerError {
    fn into_response(self) -> Response {
        if let Self::Internal(detail) = &self {
            tracing::error!(%detail, "identity: owner internal error");
        }
        (
            self.status(),
            Json(serde_json::json!({ "message": self.to_string(), "code": self.code() })),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for OwnerError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

impl From<anyhow::Error> for OwnerError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(format!("{e:#}"))
    }
}

#[derive(Debug)]
struct OwnerCode {
    email: String,
    purpose: OwnerPurpose,
    display_name: Option<String>,
    code_hash: [u8; 32],
    tries: u8,
    next: String,
}

#[derive(Debug)]
struct ShownCodes {
    codes: Vec<String>,
    next: String,
}

#[derive(Debug)]
struct StepUp {
    account: Uuid,
    fingerprint: [u8; 32],
}

/// What a verified code led to.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Verified {
    /// Signed in.
    SignedIn {
        /// Where to go.
        redirect: String,
        /// A new account was made.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        created: bool,
        /// For `GET /auth/api/recovery-codes/shown?r=`, after sign-up.
        #[serde(skip_serializing_if = "Option::is_none")]
        recovery: Option<String>,
    },
    /// Recovery by email is held.
    Held {
        /// Always true.
        held: bool,
        /// `rh_…`.
        hold: String,
        /// When it ends, in words.
        release_at: String,
    },
}

/// Owner email codes, recovery codes and recovery holds. Cheap to clone.
#[derive(Clone)]
pub struct Owners {
    inner: Arc<Inner>,
}

struct Inner {
    store: OwnerStore,
    accounts: Arc<dyn OwnerAccounts>,
    sessions: Arc<dyn OwnerSessions>,
    approvals: Arc<dyn Approvals>,
    receipts: Arc<dyn Receipts>,
    mailer: Arc<dyn CodeMailer>,
    platform: SiteHost,
    codes: Pending<OwnerCode>,
    shown: Pending<ShownCodes>,
    stepups: Pending<StepUp>,
    sends: Throttle,
    ip_sends: Throttle,
    failures: Throttle,
}

impl std::fmt::Debug for Owners {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Owners")
            .field("platform", &self.inner.platform.host)
            .finish_non_exhaustive()
    }
}

impl Owners {
    /// For node: control.db, owner accounts and sessions, approvals and receipts from `ctx`,
    /// and `mailer` for codes and notices.
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
                store: OwnerStore::new(ctx.pool.clone()),
                accounts: ctx.owner_accounts.clone(),
                sessions: ctx.owners.clone(),
                approvals: ctx.approvals.clone(),
                receipts: ctx.receipts.clone(),
                mailer,
                platform,
                codes: Pending::new(Duration::from_secs(u64::from(CODE_MINUTES) * 60), 4096),
                shown: Pending::new(Duration::from_secs(30 * 60), 1024),
                stepups: Pending::new(Duration::from_secs(300), 1024),
                sends: Throttle::new(SENDS_PER_EMAIL, WINDOW),
                ip_sends: Throttle::new(SENDS_PER_IP, WINDOW),
                failures: Throttle::new(FAILURES_PER_EMAIL, WINDOW),
            }),
        }
    }

    /// The routes on the platform host.
    pub fn router(&self) -> axum::Router {
        routes::router(self.clone())
    }

    /// The platform's origin, for the same-origin check.
    fn origin(&self) -> &str {
        &self.inner.platform.origin
    }

    async fn owner(&self, headers: &HeaderMap) -> Result<agentrix_host::Owner, OwnerError> {
        self.inner
            .sessions
            .owner(headers)
            .await?
            .ok_or(OwnerError::SignedOut)
    }

    // ---- codes ----------------------------------------------------------------------------

    /// Sends a code.
    pub(crate) async fn start(
        &self,
        email: &str,
        purpose: OwnerPurpose,
        display_name: Option<&str>,
        next: String,
        ip: &str,
    ) -> Result<(String, String), OwnerError> {
        let email = crate::visitor::email_of(email).map_err(|_| {
            OwnerError::BadRequest("That doesn't look like an email address.".into())
        })?;
        let account = self.inner.accounts.by_email(&email).await?;
        let display_name = match purpose {
            OwnerPurpose::Signup => {
                if account.is_some() {
                    return Err(OwnerError::EmailTaken);
                }
                let name: String = display_name
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(80)
                    .collect();
                Some(if name.is_empty() {
                    email.split('@').next().unwrap_or_default().to_owned()
                } else {
                    name
                })
            }
            OwnerPurpose::Signin => {
                let account = account.ok_or(OwnerError::NoAccount)?;
                if account.has_passkey || account.has_password {
                    return Err(OwnerError::UseStrongerSignIn);
                }
                None
            }
            OwnerPurpose::Recover => {
                account.ok_or(OwnerError::NoAccount)?;
                None
            }
        };
        if !self.inner.sends.allowed(&email) || !self.inner.ip_sends.allowed(ip) {
            return Err(OwnerError::TooManyTries);
        }
        self.inner.sends.record(&email);
        self.inner.ip_sends.record(ip);
        let code = format!("{:06}", rand::rngs::OsRng.gen_range(0..1_000_000u32));
        let mail_purpose = match purpose {
            OwnerPurpose::Signup => CodePurpose::Create,
            OwnerPurpose::Signin => CodePurpose::SignIn,
            OwnerPurpose::Recover => CodePurpose::Recover,
        };
        let mail = Mail {
            site: self.inner.platform.clone(),
            to: email.clone(),
            kind: MailKind::Code {
                code: code.clone(),
                purpose: mail_purpose,
                minutes: CODE_MINUTES,
            },
        };
        self.inner.mailer.send(mail).await.map_err(|e| {
            tracing::warn!(error = %e, "identity: owner code email failed");
            OwnerError::MailFailed
        })?;
        let pending = self.inner.codes.put(OwnerCode {
            email: email.clone(),
            purpose,
            display_name,
            code_hash: sha(code.as_bytes()),
            tries: 0,
            next,
        });
        Ok((pending, crate::visitor::mask_email(&email)))
    }

    /// Checks a code and does what its purpose says.
    pub(crate) async fn verify(
        &self,
        pending: &str,
        code: &str,
        headers: &HeaderMap,
        ip: &str,
    ) -> Result<(Verified, Option<HeaderValue>), OwnerError> {
        let typed: String = code.chars().filter(char::is_ascii_digit).collect();
        if typed.len() != 6 {
            return Err(OwnerError::BadRequest(
                "Type the 6 digits from the email.".into(),
            ));
        }
        let checked = self.inner.codes.with(pending, |p| {
            if p.tries >= CODE_TRIES {
                return Err(OwnerError::TooManyTries);
            }
            if bool::from(p.code_hash.ct_eq(&sha(typed.as_bytes()))) {
                Ok(())
            } else {
                p.tries += 1;
                Err(if p.tries >= CODE_TRIES {
                    OwnerError::TooManyTries
                } else {
                    OwnerError::WrongCode
                })
            }
        });
        match checked {
            None => return Err(OwnerError::Expired),
            Some(Err(e)) => {
                if matches!(e, OwnerError::TooManyTries) {
                    self.inner.codes.remove(pending);
                }
                return Err(e);
            }
            Some(Ok(())) => {}
        }
        let step = self.inner.codes.take(pending).ok_or(OwnerError::Expired)?;
        self.inner.sends.clear(&step.email);
        let device = device_of(headers);
        match step.purpose {
            OwnerPurpose::Signup => {
                let name = step.display_name.clone().unwrap_or_default();
                let account = self
                    .inner
                    .accounts
                    .create(&step.email, &name)
                    .await
                    .map_err(|e| {
                        tracing::info!(error = %e, "identity: owner sign-up refused");
                        OwnerError::EmailTaken
                    })?;
                let codes = new_codes(OWNER_RECOVERY_CODES);
                self.inner.store.set_recovery_codes(account, &codes).await?;
                let recovery = self.inner.shown.put(ShownCodes {
                    codes,
                    next: step.next.clone(),
                });
                let cookie = self
                    .code_session(account, None, "email-code", &device, ip)
                    .await?;
                Ok((
                    Verified::SignedIn {
                        redirect: step.next,
                        created: true,
                        recovery: Some(recovery),
                    },
                    Some(cookie),
                ))
            }
            OwnerPurpose::Signin => {
                let account = self
                    .inner
                    .accounts
                    .by_email(&step.email)
                    .await?
                    .ok_or(OwnerError::NoAccount)?;
                // Checked again: a passkey may have been added since the code was sent.
                if account.has_passkey || account.has_password {
                    return Err(OwnerError::UseStrongerSignIn);
                }
                let cookie = self
                    .code_session(
                        account.account,
                        Some(&account.email),
                        "email-code",
                        &device,
                        ip,
                    )
                    .await?;
                Ok((
                    Verified::SignedIn {
                        redirect: step.next,
                        created: false,
                        recovery: None,
                    },
                    Some(cookie),
                ))
            }
            OwnerPurpose::Recover => {
                let account = self
                    .inner
                    .accounts
                    .by_email(&step.email)
                    .await?
                    .ok_or(OwnerError::NoAccount)?;
                if let Some(hold) = self.inner.store.released_hold(account.account).await? {
                    self.inner.store.end_hold(&hold, "completed").await?;
                    self.hold_receipt(account.account, &hold, "completed", None)
                        .await;
                    let cookie = self
                        .code_session(
                            account.account,
                            Some(&account.email),
                            "recovery-hold",
                            &device,
                            ip,
                        )
                        .await?;
                    return Ok((
                        Verified::SignedIn {
                            redirect: "/console/settings/sign-in?recovered=1".into(),
                            created: false,
                            recovery: None,
                        },
                        Some(cookie),
                    ));
                }
                let (hold, release_at) = match self.inner.store.open_hold(account.account).await? {
                    Some(open) => open,
                    None => {
                        let release_at = crate::now() + HOLD_SECONDS;
                        let hold = self
                            .inner
                            .store
                            .start_hold(account.account, release_at, &device)
                            .await?;
                        self.hold_receipt(account.account, &hold, "held", Some(release_at))
                            .await;
                        let mail = Mail {
                            site: self.inner.platform.clone(),
                            to: account.email.clone(),
                            kind: MailKind::RecoveryHold {
                                release: crate::visitor::consent_when(release_at),
                            },
                        };
                        if let Err(e) = self.inner.mailer.send(mail).await {
                            tracing::warn!(error = %e, "identity: recovery hold notice not sent");
                        }
                        (hold, release_at)
                    }
                };
                Ok((
                    Verified::Held {
                        held: true,
                        hold,
                        release_at: crate::visitor::consent_when(release_at),
                    },
                    None,
                ))
            }
        }
    }

    /// Starts an owner session for a code sign-in: event, receipt and, when `notify` names the
    /// owner's email (every sign-in but a brand-new account's), a notice.
    async fn code_session(
        &self,
        account: Uuid,
        notify: Option<&str>,
        method: &str,
        device: &str,
        ip: &str,
    ) -> Result<HeaderValue, OwnerError> {
        let cookie = self.inner.accounts.start_session(account, method).await?;
        let ip_hash = hex::encode(&sha(ip.as_bytes())[..8]);
        let event = self
            .inner
            .store
            .record_sign_in(account, method, device, &ip_hash)
            .await?;
        let now = chrono::Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::AccountCodeSignIn,
            SiteId::new("platform"),
            Subject::Account(account.to_string()),
            now,
            now,
        );
        body.detail = Some(
            serde_json::json!({ "account": account, "method": method, "device": device, "ip_hash": ip_hash, "event": event }),
        );
        match self.inner.receipts.issue(body).await {
            Ok(digest) => {
                self.inner
                    .store
                    .set_sign_in_receipt(&event, &digest)
                    .await?
            }
            Err(e) => tracing::warn!(error = %e, "identity: code sign-in receipt not issued"),
        }
        if let Some(email) = notify {
            {
                let words = match method {
                    "recovery-code" => "a recovery code",
                    "recovery-hold" => "an email code, after the 24-hour recovery hold",
                    _ => "an email code",
                };
                let mail = Mail {
                    site: self.inner.platform.clone(),
                    to: email.to_owned(),
                    kind: MailKind::CodeSignIn {
                        device: device.to_owned(),
                        when: crate::visitor::consent_when(crate::now()),
                        method: words.into(),
                    },
                };
                if let Err(e) = self.inner.mailer.send(mail).await {
                    tracing::warn!(error = %e, "identity: code sign-in notice not sent");
                }
            }
        }
        Ok(cookie)
    }

    async fn hold_receipt(&self, account: Uuid, hold: &str, state: &str, release_at: Option<i64>) {
        let now = chrono::Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::AccountRecoveryHold,
            SiteId::new("platform"),
            Subject::Account(account.to_string()),
            now,
            now,
        );
        body.detail = Some(
            serde_json::json!({ "account": account, "hold": hold, "state": state, "release_at": release_at }),
        );
        if let Err(e) = self.inner.receipts.issue(body).await {
            tracing::warn!(error = %e, "identity: recovery hold receipt not issued");
        }
    }

    // ---- recovery codes -------------------------------------------------------------------

    /// The codes to show once after sign-up.
    pub(crate) fn take_shown(&self, id: &str) -> Option<(Vec<String>, String)> {
        self.inner.shown.take(id).map(|s| (s.codes, s.next))
    }

    /// Unused codes left.
    pub(crate) async fn codes_left(&self, headers: &HeaderMap) -> Result<usize, OwnerError> {
        let owner = self.owner(headers).await?;
        Ok(self.inner.store.recovery_codes_left(owner.account).await?)
    }

    /// Starts a step-up before new codes: passkey options bound to a one-off fingerprint, or
    /// `None` when the account has no passkey (then an authenticator code, if it has one).
    pub(crate) async fn stepup_start(
        &self,
        headers: &HeaderMap,
    ) -> Result<(String, Option<serde_json::Value>), OwnerError> {
        let owner = self.owner(headers).await?;
        let nonce: [u8; 16] = rand::rngs::OsRng.r#gen();
        let fingerprint = sha(&serde_json::to_vec(&serde_json::json!({
            "kind": "account.recovery-codes.regenerate", "account": owner.account, "nonce": hex::encode(nonce),
        })).map_err(|e| OwnerError::Internal(e.to_string()))?);
        let options = self
            .inner
            .approvals
            .passkey_options(owner.account, fingerprint)
            .await?;
        let challenge = self.inner.stepups.put(StepUp {
            account: owner.account,
            fingerprint,
        });
        Ok((challenge, options))
    }

    /// New recovery codes. A passkey or password session must prove it's the owner now
    /// (through the frozen approvals); a session that came from a recovery code, a finished
    /// hold or an email-only account already proved it moments ago or has nothing stronger.
    pub(crate) async fn regenerate(
        &self,
        headers: &HeaderMap,
        challenge: Option<&str>,
        proof: Option<serde_json::Value>,
    ) -> Result<Vec<String>, OwnerError> {
        let owner = self.owner(headers).await?;
        let strong_session = matches!(owner.method.as_str(), "passkey" | "password+totp");
        if strong_session {
            let step = challenge
                .and_then(|c| self.inner.stepups.take(c))
                .ok_or(OwnerError::StepUpNeeded)?;
            if step.account != owner.account {
                return Err(OwnerError::StepUpNeeded);
            }
            let proof = proof.ok_or(OwnerError::StepUpNeeded)?;
            self.inner
                .approvals
                .verify(owner.account, step.fingerprint, proof)
                .await
                .map_err(|e| {
                    tracing::info!(error = %e, "identity: step-up refused");
                    OwnerError::StepUpNeeded
                })?;
        }
        let codes = new_codes(OWNER_RECOVERY_CODES);
        self.inner
            .store
            .set_recovery_codes(owner.account, &codes)
            .await?;
        Ok(codes)
    }

    /// Signs in with a recovery code.
    pub(crate) async fn recover_with_code(
        &self,
        email: &str,
        code: &str,
        headers: &HeaderMap,
        ip: &str,
    ) -> Result<HeaderValue, OwnerError> {
        let email = crate::visitor::email_of(email).map_err(|_| {
            OwnerError::BadRequest("That doesn't look like an email address.".into())
        })?;
        if !self.inner.failures.allowed(&email) || !self.inner.ip_sends.allowed(ip) {
            return Err(OwnerError::TooManyTries);
        }
        let Some(account) = self.inner.accounts.by_email(&email).await? else {
            self.inner.failures.record(&email);
            self.inner.ip_sends.record(ip);
            return Err(OwnerError::WrongRecoveryCode);
        };
        if !self
            .inner
            .store
            .use_recovery_code(account.account, &normalise(code))
            .await?
        {
            self.inner.failures.record(&email);
            self.inner.ip_sends.record(ip);
            return Err(OwnerError::WrongRecoveryCode);
        }
        self.inner.failures.clear(&email);
        self.code_session(
            account.account,
            Some(&account.email),
            "recovery-code",
            &device_of(headers),
            ip,
        )
        .await
    }

    // ---- holds ----------------------------------------------------------------------------

    /// Holds waiting on the signed-in owner's account.
    pub(crate) async fn holds(
        &self,
        headers: &HeaderMap,
    ) -> Result<Vec<serde_json::Value>, OwnerError> {
        let owner = self.owner(headers).await?;
        let rows = self.inner.store.holds(owner.account).await?;
        Ok(rows
            .into_iter()
            .map(|h| {
                serde_json::json!({
                    "id": h.id, "device": h.device,
                    "requested": crate::visitor::consent_when(h.requested_at),
                    "release": crate::visitor::consent_when(h.release_at),
                })
            })
            .collect())
    }

    /// Cancels a hold (the owner signed in some other way, so it wasn't needed or wasn't them).
    pub(crate) async fn cancel_hold(
        &self,
        headers: &HeaderMap,
        hold: &str,
    ) -> Result<bool, OwnerError> {
        let owner = self.owner(headers).await?;
        let cancelled = self.inner.store.cancel_hold(owner.account, hold).await?;
        if cancelled {
            self.hold_receipt(owner.account, hold, "cancelled", None)
                .await;
        }
        Ok(cancelled)
    }
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn device_of(headers: &HeaderMap) -> String {
    device::describe(
        headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok()),
    )
}

const ALPHABET: &[u8] = b"23456789abcdefghjkmnpqrstuvwxyz";

fn new_codes(n: usize) -> Vec<String> {
    let mut rng = rand::rngs::OsRng;
    (0..n)
        .map(|_| {
            let chars: String = (0..8)
                .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
                .collect();
            format!("{}-{}", &chars[..4], &chars[4..])
        })
        .collect()
}

fn normalise(code: &str) -> String {
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

/// The client address as Caddy saw it: the last `X-Forwarded-For` entry, else "unknown".
pub(crate) fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map(str::trim)
        .filter(|ip| !ip.is_empty() && ip.len() <= 64)
        .unwrap_or("unknown")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_ips() {
        let codes = new_codes(OWNER_RECOVERY_CODES);
        assert_eq!(codes.len(), 10);
        assert_eq!(
            normalise(&codes[0].to_uppercase().replace('-', "")),
            codes[0]
        );
        let mut h = HeaderMap::new();
        assert_eq!(client_ip(&h), "unknown");
        h.insert(
            "x-forwarded-for",
            "203.0.113.9, 198.51.100.4".parse().unwrap(),
        );
        assert_eq!(client_ip(&h), "198.51.100.4", "the entry Caddy appended");
    }
}
