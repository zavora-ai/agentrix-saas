//! Sign-in (R2, design D9, §4.1): passkeys first; without one, email and password, always with
//! an authenticator (TOTP) code; sessions in an HttpOnly, Secure, SameSite cookie. Also the
//! approvals API that `decisions/` uses for go-live and other cards ([`Approvals`]).
//!
//! Lane: `control-plane` (specs/first-release/design.md §3.0).
//!
//! Wiring (the integration lead, in `app.rs`):
//! ```ignore
//! let auth = auth::AuthState::new(auth::AuthConfig::for_origin(origin)?, pool, sealer)?
//!     .with_console(console.clone());          // Arc<agentrix_console::Console>: /auth/signin, /auth/register
//! let app = app.merge(auth::router(auth.clone()));
//! // decisions/ gets auth.approvals(); any handler takes `auth::Session` as an extractor;
//! // the console's session adapter uses auth.session_from(headers) and auth.account_summary(id).
//! ```
//! The tables are in `schema.sql`; the data lane's migrator includes it.

mod approvals;
mod passkey;
mod password;
mod pending;
mod routes;
mod seal;
mod session;
mod store;
mod totp;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::SqlitePool;
use url::Url;
use uuid::Uuid;

pub use approvals::{ApprovalProof, Approvals, ApproveRequest, PasskeyOptions};
pub use routes::router;
pub use seal::Sealer;
pub use session::Session;
pub use store::{NewAccount, Store};

/// The schema of the sign-in tables, for the data lane's migrator.
pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Sign-in settings.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// The platform's origin, exactly as browsers send it, e.g. `https://agentrix.example`.
    pub origin: Url,
    /// The passkey relying party id: the origin's host.
    pub rp_id: String,
    /// The name authenticators show.
    pub rp_name: String,
    /// The issuer authenticator apps show beside the code.
    pub issuer: String,
    /// `Secure` and `__Host-` cookies. True behind TLS; false only for local development.
    pub secure_cookies: bool,
    /// Where a sign-in goes when no `next` is given.
    pub after_sign_in: String,
}

impl AuthConfig {
    /// Settings for an origin: RP id from its host, secure cookies when it is `https`.
    pub fn for_origin(origin: Url) -> Result<Self, AuthError> {
        let rp_id = origin
            .host_str()
            .ok_or_else(|| AuthError::Internal(format!("origin {origin} has no host")))?
            .to_owned();
        let secure_cookies = origin.scheme() == "https";
        Ok(Self {
            origin,
            rp_id,
            rp_name: "Agentrix".into(),
            issuer: "Agentrix".into(),
            secure_cookies,
            after_sign_in: "/os".into(),
        })
    }
}

/// Everything the sign-in routes share. Cheap to clone.
#[derive(Clone)]
pub struct AuthState {
    pub(crate) config: Arc<AuthConfig>,
    pub(crate) store: Store,
    pub(crate) sealer: Arc<Sealer>,
    passkeys: Option<Arc<passkey::Passkeys>>,
    approvals: Arc<approvals::ApprovalCeremonies>,
    registrations: Arc<pending::Pending<routes::PendingRegistration>>,
    logins: Arc<pending::Pending<routes::PendingLogin>>,
    password_steps: Arc<pending::Pending<routes::PasswordStep>>,
    throttle: Arc<routes::Throttle>,
    console: Option<Arc<agentrix_console::Console>>,
}

impl fmt::Debug for AuthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthState")
            .field("origin", &self.config.origin.as_str())
            .field("passkeys", &self.passkeys.is_some())
            .field("console", &self.console.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthState {
    /// Sign-in over `pool` (whose schema includes [`SCHEMA_SQL`]), sealing TOTP seeds with
    /// `sealer`. Fails if the origin and RP id do not fit together.
    pub fn new(config: AuthConfig, pool: SqlitePool, sealer: Sealer) -> Result<Self, AuthError> {
        let passkeys = passkey::Passkeys::new(&config.rp_id, &config.origin, &config.rp_name)
            .map_err(|e| AuthError::Internal(format!("passkey relying party: {e}")))?;
        let ten_minutes = Duration::from_secs(600);
        Ok(Self {
            config: Arc::new(config),
            store: Store::new(pool),
            sealer: Arc::new(sealer),
            passkeys: Some(Arc::new(passkeys)),
            approvals: Arc::default(),
            registrations: Arc::new(pending::Pending::new(ten_minutes, 4096)),
            logins: Arc::new(pending::Pending::new(Duration::from_secs(300), 4096)),
            password_steps: Arc::new(pending::Pending::new(ten_minutes, 4096)),
            throttle: Arc::default(),
            console: None,
        })
    }

    /// The console, which renders `/auth/signin` and `/auth/register`.
    pub fn with_console(mut self, console: Arc<agentrix_console::Console>) -> Self {
        self.console = Some(console);
        self
    }

    /// What the console needs about an owner: `has_passkey` decides whether a card offers the
    /// passkey or the authenticator code.
    pub async fn account_summary(
        &self,
        account: Uuid,
    ) -> Result<Option<AccountSummary>, AuthError> {
        let Some(found) = self.store.account(account).await? else {
            return Ok(None);
        };
        let has_passkey = !self.store.credentials(account).await?.is_empty();
        Ok(Some(AccountSummary {
            id: found.id,
            email: found.email,
            display_name: found.display_name,
            has_passkey,
            has_password: found.password_hash.is_some(),
        }))
    }

    /// The store, for tests and for the integration lead's schema check.
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn passkeys(&self) -> Result<&passkey::Passkeys, AuthError> {
        self.passkeys.as_deref().ok_or(AuthError::PasskeysOff)
    }
}

/// An owner, as the console sees them.
#[derive(Debug, Clone, Serialize)]
pub struct AccountSummary {
    /// Account id.
    pub id: Uuid,
    /// Sign-in email.
    pub email: String,
    /// The name the console greets.
    pub display_name: String,
    /// At least one passkey: cards offer the passkey; otherwise the authenticator code.
    pub has_passkey: bool,
    /// Has a password (and so an authenticator app).
    pub has_password: bool,
}

/// Sign-in errors. Each answers with `{message}` in plain words (design §4.1).
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Not signed in.
    #[error("Sign in first.")]
    SignedOut,
    /// The request was malformed; the message says how.
    #[error("{0}")]
    BadRequest(String),
    /// Registration with an email that has an account.
    #[error("An account with this email already exists. Sign in instead.")]
    EmailTaken,
    /// Email and password do not match (or the email has no password).
    #[error("That email and password don't match.")]
    WrongCredentials,
    /// The TOTP code was wrong, old or already used.
    #[error("That code didn't work. Codes change every 30 seconds, so use the newest one.")]
    WrongCode,
    /// The account has no authenticator app set up.
    #[error("This account has no authenticator app set up.")]
    NoTotp,
    /// No passkey is registered for the email.
    #[error("No passkey is registered for this email. Sign in with your password and code.")]
    NoPasskey,
    /// webauthn-rs refused a sign-in or registration.
    #[error("That passkey didn't work. Try again, or use your password and code.")]
    PasskeyRefused,
    /// The step took too long, or was already used.
    #[error("That took too long. Start again.")]
    Expired,
    /// The approval's ceremony is unknown or expired.
    #[error("This approval expired. Reload the card and approve again.")]
    ApprovalExpired,
    /// The assertion does not approve this card for this owner.
    #[error("This passkey can't approve this card.")]
    ApprovalRefused,
    /// Passkeys are not available on this server.
    #[error("Passkeys aren't available here. Use your password and code.")]
    PasskeysOff,
    /// Too many failed attempts for this email.
    #[error("Too many tries. Wait 15 minutes, then try again.")]
    TooManyTries,
    /// A state-changing request from another site.
    #[error("This request came from another site, so it was refused.")]
    CrossOrigin,
    /// A fault on our side; the detail is logged, not shown.
    #[error("Something went wrong on our side. Nothing was changed. Try again in a moment.")]
    Internal(String),
}

impl AuthError {
    fn status(&self) -> StatusCode {
        match self {
            Self::SignedOut | Self::WrongCredentials | Self::WrongCode | Self::PasskeyRefused => {
                StatusCode::UNAUTHORIZED
            }
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::EmailTaken | Self::NoTotp => StatusCode::CONFLICT,
            Self::NoPasskey => StatusCode::NOT_FOUND,
            Self::Expired | Self::ApprovalExpired => StatusCode::GONE,
            Self::ApprovalRefused | Self::CrossOrigin => StatusCode::FORBIDDEN,
            Self::PasskeysOff => StatusCode::SERVICE_UNAVAILABLE,
            Self::TooManyTries => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        if let Self::Internal(detail) = &self {
            tracing::error!(%detail, "auth: internal error");
        }
        (
            self.status(),
            Json(serde_json::json!({ "message": self.to_string() })),
        )
            .into_response()
    }
}

impl From<store::StoreError> for AuthError {
    fn from(error: store::StoreError) -> Self {
        match error {
            store::StoreError::EmailTaken => Self::EmailTaken,
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<passkey::PasskeyError> for AuthError {
    fn from(error: passkey::PasskeyError) -> Self {
        Self::Internal(error.to_string())
    }
}
