//! Sign-in routes (design §4.1, R2). JSON in and out, driven by the sign-in page's small
//! script, except the two page renders and sign-out:
//!
//! | Route | Body | Reply |
//! |---|---|---|
//! | `GET /auth/signin`, `GET /auth/register` | `?next=` | Deprecated: a redirect to the platform's `/account/signin` or `/account/new`, keeping `next` |
//! | `POST /auth/passkey/register/start` | `{email, display_name?, next?}`; signed in: adds a passkey | webauthn-rs `CreationChallengeResponse` |
//! | `POST /auth/passkey/register/finish` | webauthn-rs `RegisterPublicKeyCredential` | `{redirect}` and the session cookie |
//! | `POST /auth/passkey/login/start` | `{email, next?}` | webauthn-rs `RequestChallengeResponse` |
//! | `POST /auth/passkey/login/finish` | webauthn-rs `PublicKeyCredential` | `{redirect}` and the session cookie |
//! | `POST /auth/password` | `{action: "register" \| "login", email, password, display_name?, next?}` | `{next: "totp-enrol", pending, key, otpauth, qr_svg}` or `{next: "totp", pending}` |
//! | `POST /auth/totp` | `{pending, code}` | `{redirect}` and the session cookie |
//! | `POST /auth/signout` | — | 303 to `/`, cookie cleared |
//! | `GET /auth/session` | — | `{account, email, display_name, method, has_passkey, has_password}` or 401 |
//!
//! Ceremonies are matched by the challenge their credential signed, so the browser sends pure
//! webauthn-rs JSON. A password never signs in by itself. Errors are `{message}` in plain words.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::header::{ORIGIN, SET_COOKIE};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential,
};

use super::passkey::credential_id_text;
use super::store::NewAccount;
use super::{AuthError, AuthState, password, session, totp};

/// The sign-in routes, with `state`.
pub fn router(state: AuthState) -> Router {
    Router::new()
        .route("/auth/signin", get(signin_page))
        .route("/auth/register", get(register_page))
        .route("/auth/passkey/register/start", post(passkey_register_start))
        .route(
            "/auth/passkey/register/finish",
            post(passkey_register_finish),
        )
        .route("/auth/passkey/login/start", post(passkey_login_start))
        .route("/auth/passkey/login/finish", post(passkey_login_finish))
        .route("/auth/password", post(password_step))
        .route("/auth/totp", post(totp_step))
        .route("/auth/signout", post(signout))
        .route("/auth/session", get(current_session))
        .layer(middleware::from_fn_with_state(state.clone(), same_origin))
        .with_state(state)
}

// ---- state between steps ------------------------------------------------------------------

/// A passkey registration in progress.
#[derive(Debug)]
pub(super) struct PendingRegistration {
    user: Uuid,
    email: String,
    display_name: String,
    existing_account: bool,
    state: PasskeyRegistration,
    next: String,
}

/// A passkey sign-in in progress.
#[derive(Debug)]
pub(super) struct PendingLogin {
    account: Uuid,
    state: PasskeyAuthentication,
    next: String,
}

/// A password step waiting for its TOTP code.
#[derive(Debug)]
pub(super) enum PasswordStep {
    /// A new account: nothing is stored until the first code proves the app is set up.
    Signup {
        email: String,
        display_name: String,
        password_hash: String,
        seed: [u8; totp::SEED_BYTES],
        next: String,
        tries: u8,
    },
    /// A sign-in whose password was right.
    Login {
        account: Uuid,
        next: String,
        tries: u8,
    },
}

/// Failed attempts per email: at most `LIMIT` in `WINDOW`, then 429 until the window passes.
#[derive(Debug, Default)]
pub(super) struct Throttle(Mutex<HashMap<String, (u32, Instant)>>);

impl Throttle {
    const LIMIT: u32 = 8;
    const WINDOW: Duration = Duration::from_secs(15 * 60);

    fn check(&self, key: &str) -> Result<(), AuthError> {
        let map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(key) {
            Some((count, since)) if *count >= Self::LIMIT && since.elapsed() < Self::WINDOW => {
                Err(AuthError::TooManyTries)
            }
            _ => Ok(()),
        }
    }

    fn failed(&self, key: &str) {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(key.to_owned()).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= Self::WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    fn succeeded(&self, key: &str) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).remove(key);
    }
}

// ---- helpers --------------------------------------------------------------------------------

/// Lower-cased, trimmed, and plausibly an address. Real proof of an email is out of scope for
/// The first release (no email is sent); the address is a sign-in name and a contact.
fn email_of(raw: &str) -> Result<String, AuthError> {
    let email = raw.trim().to_lowercase();
    let ok = email.len() <= 254
        && email.split('@').count() == 2
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
        && !email.chars().any(char::is_whitespace);
    if ok {
        Ok(email)
    } else {
        Err(AuthError::BadRequest(
            "That doesn't look like an email address.".into(),
        ))
    }
}

fn display_name_of(given: Option<&str>, email: &str) -> String {
    given
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(80).collect())
        .unwrap_or_else(|| email.split('@').next().unwrap_or(email).to_owned())
}

/// Only same-site paths: never an open redirect.
fn next_of(given: Option<&str>, default: &str) -> String {
    match given {
        Some(path)
            if path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains('\\')
                && path.len() <= 512 =>
        {
            path.to_owned()
        }
        _ => default.to_owned(),
    }
}

fn challenge_key(challenge: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(challenge)
}

/// The challenge in a credential's clientDataJSON, base64url.
fn signed_challenge(client_data_json: &[u8]) -> Result<String, AuthError> {
    let client_data: serde_json::Value = serde_json::from_slice(client_data_json)
        .map_err(|_| AuthError::BadRequest("The passkey's answer could not be read.".into()))?;
    client_data
        .get("challenge")
        .and_then(|c| c.as_str())
        .map(str::to_owned)
        .ok_or_else(|| AuthError::BadRequest("The passkey's answer has no challenge.".into()))
}

fn signed_in(cookie: axum::http::HeaderValue, redirect: &str) -> Response {
    let mut response = Json(json!({ "redirect": redirect })).into_response();
    response.headers_mut().append(SET_COOKIE, cookie);
    response
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| AuthError::Internal(format!("worker: {e}")))
}

/// Refuses cross-site state changes: a POST whose `Origin` is not the platform's. SameSite=Lax
/// cookies and JSON-only bodies already stop classic CSRF; this is the third layer.
async fn same_origin(State(auth): State<AuthState>, request: Request, next: Next) -> Response {
    if request.method() == Method::POST {
        if let Some(origin) = request.headers().get(ORIGIN).and_then(|o| o.to_str().ok()) {
            if origin != auth.config.origin.as_str().trim_end_matches('/') {
                return AuthError::CrossOrigin.into_response();
            }
        }
    }
    next.run(request).await
}

// ---- pages ----------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PageQuery {
    next: Option<String>,
}

// Deprecated: the first sign-in and register pages. The platform's own pages replaced them
// (agentrix-onboarding: `/account/signin` with passkey first and an email code, `/account/new`),
// so these two routes only send people there, keeping `next`. The JSON routes in this file stay:
// the new pages call them.

async fn signin_page(
    State(auth): State<AuthState>,
    axum::extract::Query(q): axum::extract::Query<PageQuery>,
) -> Redirect {
    Redirect::to(&account_page(
        "/account/signin",
        q.next.as_deref(),
        &auth.config.after_sign_in,
    ))
}

async fn register_page(
    State(auth): State<AuthState>,
    axum::extract::Query(q): axum::extract::Query<PageQuery>,
) -> Redirect {
    Redirect::to(&account_page(
        "/account/new",
        q.next.as_deref(),
        &auth.config.after_sign_in,
    ))
}

/// `page?next=…`, with `next` checked like every other `next` here and percent-encoded.
fn account_page(page: &str, next: Option<&str>, default: &str) -> String {
    let next = next_of(next, default);
    let encoded: String = url::form_urlencoded::byte_serialize(next.as_bytes()).collect();
    format!("{page}?next={encoded}")
}

impl AuthState {
    /// Deprecated with the pages above: renders the first sign-in page. Kept, unused, until the
    /// old console crate goes. With `passkeys: false` the page hides the passkey button.
    #[allow(dead_code)]
    fn render_page(
        &self,
        page: agentrix_console::Page,
        next: Option<&str>,
    ) -> Result<Response, AuthError> {
        let console = self
            .console
            .as_ref()
            .ok_or_else(|| AuthError::Internal("the console is not wired into auth".into()))?;
        let view = agentrix_console::view::AuthView {
            next: next_of(next, &self.config.after_sign_in),
            passkeys: self.passkeys().is_ok(),
        };
        let html = console
            .render(page, &view)
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(Html(html).into_response())
    }
}

// ---- passkeys -------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RegisterStart {
    email: Option<String>,
    display_name: Option<String>,
    next: Option<String>,
}

async fn passkey_register_start(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<RegisterStart>,
) -> Result<Response, AuthError> {
    let passkeys = auth.passkeys()?;
    let next = next_of(body.next.as_deref(), &auth.config.after_sign_in);
    let (user, email, display_name, existing_account, existing) =
        match auth.session_from(&headers).await? {
            // Signed in: add a passkey to this account.
            Some(session) => {
                let account = auth
                    .store
                    .account(session.account)
                    .await?
                    .ok_or(AuthError::SignedOut)?;
                let existing: Vec<Passkey> = auth
                    .passkeys_of(account.id)
                    .await?
                    .into_iter()
                    .map(|(_, p)| p)
                    .collect();
                (
                    account.id,
                    account.email,
                    account.display_name,
                    true,
                    existing,
                )
            }
            // A new account, created when the passkey is registered.
            None => {
                let email = email_of(body.email.as_deref().unwrap_or_default())?;
                if auth.store.account_by_email(&email).await?.is_some() {
                    return Err(AuthError::EmailTaken);
                }
                let display_name = display_name_of(body.display_name.as_deref(), &email);
                (Uuid::now_v7(), email, display_name, false, Vec::new())
            }
        };
    let (options, state) = passkeys.start_registration(user, &email, &display_name, &existing)?;
    let key = challenge_key(options.public_key.challenge.as_ref());
    auth.registrations.put_as(
        key,
        PendingRegistration {
            user,
            email,
            display_name,
            existing_account,
            state,
            next,
        },
    );
    Ok(Json(options).into_response())
}

async fn passkey_register_finish(
    State(auth): State<AuthState>,
    Json(credential): Json<RegisterPublicKeyCredential>,
) -> Result<Response, AuthError> {
    let passkeys = auth.passkeys()?;
    let key = signed_challenge(credential.response.client_data_json.as_ref())?;
    let pending = auth.registrations.take(&key).ok_or(AuthError::Expired)?;
    let passkey = passkeys
        .finish_registration(&credential, &pending.state)
        .map_err(|_| AuthError::PasskeyRefused)?;
    let cred_id = credential_id_text(passkey.cred_id());
    let json = serde_json::to_string(&passkey).map_err(|e| AuthError::Internal(e.to_string()))?;
    let label = format!("Passkey added {}", chrono::Utc::now().format("%-d %b %Y"));
    if pending.existing_account {
        auth.store
            .add_credential(pending.user, &cred_id, &json, &label)
            .await?;
    } else {
        auth.store
            .create_account(
                pending.user,
                NewAccount {
                    email: &pending.email,
                    display_name: &pending.display_name,
                    password_hash: None,
                    passkey: Some((&cred_id, &json, &label)),
                    totp: None,
                },
            )
            .await?;
    }
    let cookie = auth.start_session(pending.user, "passkey").await?;
    Ok(signed_in(cookie, &pending.next))
}

#[derive(Debug, Deserialize)]
struct LoginStart {
    email: String,
    next: Option<String>,
}

async fn passkey_login_start(
    State(auth): State<AuthState>,
    Json(body): Json<LoginStart>,
) -> Result<Response, AuthError> {
    let passkeys = auth.passkeys()?;
    let email = email_of(&body.email)?;
    auth.throttle.check(&email)?;
    let Some(account) = auth.store.account_by_email(&email).await? else {
        return Err(AuthError::NoPasskey);
    };
    let keys: Vec<Passkey> = auth
        .passkeys_of(account.id)
        .await?
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    if keys.is_empty() {
        return Err(AuthError::NoPasskey);
    }
    let (options, state) = passkeys.start_login(&keys)?;
    let key = challenge_key(options.public_key.challenge.as_ref());
    let next = next_of(body.next.as_deref(), &auth.config.after_sign_in);
    auth.logins.put_as(
        key,
        PendingLogin {
            account: account.id,
            state,
            next,
        },
    );
    Ok(Json(options).into_response())
}

async fn passkey_login_finish(
    State(auth): State<AuthState>,
    Json(credential): Json<PublicKeyCredential>,
) -> Result<Response, AuthError> {
    let passkeys = auth.passkeys()?;
    let key = signed_challenge(credential.response.client_data_json.as_ref())?;
    let pending = auth.logins.take(&key).ok_or(AuthError::Expired)?;
    let result = passkeys
        .finish(&credential, &pending.state)
        .map_err(|_| AuthError::PasskeyRefused)?;
    auth.record_use(pending.account, &result).await?;
    let cookie = auth.start_session(pending.account, "passkey").await?;
    Ok(signed_in(cookie, &pending.next))
}

// ---- password and TOTP ----------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PasswordAction {
    Register,
    Login,
}

#[derive(Debug, Deserialize)]
struct PasswordBody {
    action: PasswordAction,
    email: String,
    password: String,
    display_name: Option<String>,
    next: Option<String>,
}

async fn password_step(
    State(auth): State<AuthState>,
    Json(body): Json<PasswordBody>,
) -> Result<Response, AuthError> {
    let email = email_of(&body.email)?;
    let next = next_of(body.next.as_deref(), &auth.config.after_sign_in);
    match body.action {
        PasswordAction::Register => {
            password::check_policy(&body.password).map_err(|m| AuthError::BadRequest(m.into()))?;
            if auth.store.account_by_email(&email).await?.is_some() {
                return Err(AuthError::EmailTaken);
            }
            let plain = body.password;
            let password_hash = blocking(move || password::hash(&plain))
                .await?
                .map_err(|e| AuthError::Internal(e.to_string()))?;
            let seed = totp::new_seed();
            let uri = totp::otpauth_uri(&auth.config.issuer, &email, &seed);
            let display_name = display_name_of(body.display_name.as_deref(), &email);
            let pending = auth.password_steps.put(PasswordStep::Signup {
                email,
                display_name,
                password_hash,
                seed,
                next,
                tries: 0,
            });
            Ok(Json(json!({
                "next": "totp-enrol",
                "pending": pending,
                "key": totp::grouped(&totp::base32(&seed)),
                "otpauth": uri,
                "qr_svg": totp::qr_svg(&uri),
            }))
            .into_response())
        }
        PasswordAction::Login => {
            auth.throttle.check(&email)?;
            let account = auth.store.account_by_email(&email).await?;
            let plain = body.password;
            let stored = account.as_ref().and_then(|a| a.password_hash.clone());
            let ok = blocking(move || match stored {
                Some(hash) => password::verify(&plain, &hash),
                None => {
                    password::verify_nothing(&plain);
                    false
                }
            })
            .await?;
            let Some(account) = account.filter(|_| ok) else {
                auth.throttle.failed(&email);
                return Err(AuthError::WrongCredentials);
            };
            let pending = auth.password_steps.put(PasswordStep::Login {
                account: account.id,
                next,
                tries: 0,
            });
            Ok(Json(json!({ "next": "totp", "pending": pending })).into_response())
        }
    }
}

#[derive(Debug, Deserialize)]
struct TotpBody {
    pending: String,
    code: String,
}

/// Wrong codes per pending step before it must start again.
const MAX_CODE_TRIES: u8 = 5;

async fn totp_step(
    State(auth): State<AuthState>,
    Json(body): Json<TotpBody>,
) -> Result<Response, AuthError> {
    let step = auth
        .password_steps
        .take(&body.pending)
        .ok_or(AuthError::Expired)?;
    match step {
        PasswordStep::Signup {
            email,
            display_name,
            password_hash,
            seed,
            next,
            tries,
        } => {
            let now_unix = u64::try_from(super::store::now()).unwrap_or_default();
            let Some(used) = totp::verify(&seed, &body.code, now_unix, 0) else {
                if tries + 1 < MAX_CODE_TRIES {
                    auth.password_steps.put_as(
                        body.pending,
                        PasswordStep::Signup {
                            email,
                            display_name,
                            password_hash,
                            seed,
                            next,
                            tries: tries + 1,
                        },
                    );
                }
                return Err(AuthError::WrongCode);
            };
            let id = Uuid::now_v7();
            let sealed = auth.sealer.seal(&seed, id.to_string().as_bytes());
            let used = i64::try_from(used).map_err(|_| AuthError::WrongCode)?;
            auth.store
                .create_account(
                    id,
                    NewAccount {
                        email: &email,
                        display_name: &display_name,
                        password_hash: Some(&password_hash),
                        passkey: None,
                        totp: Some((&sealed, used)),
                    },
                )
                .await?;
            let cookie = auth.start_session(id, "password+totp").await?;
            Ok(signed_in(cookie, &next))
        }
        PasswordStep::Login {
            account,
            next,
            tries,
        } => match auth.check_totp(account, &body.code).await {
            Ok(_) => {
                if let Some(a) = auth.store.account(account).await? {
                    auth.throttle.succeeded(&a.email);
                }
                let cookie = auth.start_session(account, "password+totp").await?;
                Ok(signed_in(cookie, &next))
            }
            Err(AuthError::WrongCode) => {
                if let Some(a) = auth.store.account(account).await? {
                    auth.throttle.failed(&a.email);
                }
                if tries + 1 < MAX_CODE_TRIES {
                    auth.password_steps.put_as(
                        body.pending,
                        PasswordStep::Login {
                            account,
                            next,
                            tries: tries + 1,
                        },
                    );
                }
                Err(AuthError::WrongCode)
            }
            Err(other) => Err(other),
        },
    }
}

// ---- session --------------------------------------------------------------------------------

async fn signout(State(auth): State<AuthState>, headers: HeaderMap) -> Result<Response, AuthError> {
    if let Some(current) = auth.session_from(&headers).await? {
        auth.store.delete_session(current.token_hash()).await?;
    }
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().append(
        SET_COOKIE,
        session::clear_cookie(auth.config.secure_cookies),
    );
    Ok(response)
}

async fn current_session(
    State(auth): State<AuthState>,
    current: session::Session,
) -> Result<Response, AuthError> {
    let summary = auth
        .account_summary(current.account)
        .await?
        .ok_or(AuthError::SignedOut)?;
    Ok((
        StatusCode::OK,
        Json(json!({
            "account": summary.id,
            "email": summary.email,
            "display_name": summary.display_name,
            "method": current.method,
            "has_passkey": summary.has_passkey,
            "has_password": summary.has_password,
        })),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_pages_send_people_to_the_platforms_own() {
        assert_eq!(
            account_page("/account/signin", Some("/sites/s1/claim"), "/os"),
            "/account/signin?next=%2Fsites%2Fs1%2Fclaim"
        );
        assert_eq!(
            account_page("/account/new", None, "/os"),
            "/account/new?next=%2Fos"
        );
        assert_eq!(
            account_page("/account/signin", Some("//evil.example"), "/os"),
            "/account/signin?next=%2Fos"
        );
    }

    #[test]
    fn emails_and_redirects() {
        assert_eq!(
            email_of("  Wanjiru@Kilele.Example ").unwrap(),
            "wanjiru@kilele.example"
        );
        for bad in ["", "no-at", "a@b", "a@@b.c", "@b.c", "a@.b.c", "a b@c.d"] {
            assert!(email_of(bad).is_err(), "{bad}");
        }
        assert_eq!(next_of(Some("/console/move"), "/os"), "/console/move");
        for bad in [
            "https://evil.example",
            "//evil.example",
            "/\\evil",
            "console",
        ] {
            assert_eq!(next_of(Some(bad), "/os"), "/os", "{bad}");
        }
        assert_eq!(display_name_of(None, "wanjiru@kilele.example"), "wanjiru");
    }

    #[test]
    fn throttle_blocks_after_the_limit() {
        let t = Throttle::default();
        for _ in 0..Throttle::LIMIT {
            assert!(t.check("a@b.c").is_ok());
            t.failed("a@b.c");
        }
        assert!(matches!(t.check("a@b.c"), Err(AuthError::TooManyTries)));
        t.succeeded("a@b.c");
        assert!(t.check("a@b.c").is_ok());
    }
}
