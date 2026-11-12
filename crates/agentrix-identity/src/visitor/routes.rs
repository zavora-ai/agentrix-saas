//! Customer account routes on every site host (agreed with W2, Sun 8 Nov). Pages are HTML in
//! the site's shell; the API is JSON for `/account/account.js`; the code path also works as
//! plain form posts, so a customer can sign in with JavaScript off (G-35).
//!
//! | Route | What |
//! |---|---|
//! | `GET /account/sign-in?next=&why=order&again=1` | A-01, A-14 |
//! | `GET /account/create?next=&f=` | A-03 (`f`: a server-side prefill, never the address itself) |
//! | `GET /account/code?p=` | A-15: type the code |
//! | `GET /account/recovery-codes?r=` | A-06, once |
//! | `GET /account/passkey-offer?next=` | A-07 |
//! | `GET /account/recover` | A-16 |
//! | `GET /account/security` | A-09, A-19 |
//! | `GET /account/choices?next=` | A-08 |
//! | `GET /account/keeps` | A-02 |
//! | `GET /account/signed-out` | A-18 |
//! | `POST /account/sign-out` | form; 303 to `/account/signed-out` |
//! | `POST /account/code/send`, `/account/code/check`, `/account/code/resend` | forms (no JS) |
//! | `GET /account/account.js`, `/account/account.css` | the island and the plain shell's CSS |
//! | `GET /account/api/session` | `{signed_in, account?}` |
//! | `POST /account/api/passkey/register/start` · `/finish` | make a passkey (new account, or another) |
//! | `POST /account/api/passkey/login/start` · `/finish` | sign in with a passkey |
//! | `POST /account/api/passkey/decline` | "Not now" on this device |
//! | `POST /account/api/code/start` · `/verify` · `/resend` | email codes |
//! | `POST /account/api/recovery/use` · `/new` | recovery codes |
//! | `POST /account/api/sessions/end` | `{id}` or `{all: true}` |
//! | `POST /account/api/passkeys/remove` · `/account/api/alerts/dismiss` | security page |
//! | `GET`, `POST /account/api/consents` | choices and receipts |
//!
//! Every POST must come from the site's own origin. Errors are `{message, code}`.

use agentrix_types::tenancy::SiteHost;
use axum::extract::{Query, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ORIGIN, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Extension, Form, Json, Router};
use serde::Deserialize;
use serde_json::json;
use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential};

use super::consent::{self, Purpose};
use super::views::{ACCOUNT_CSS, ACCOUNT_JS};
use super::{CODE_MINUTES, Done, Visitor, VisitorError, Visitors, enc, next_of, session};
use crate::mailer::CodePurpose;

/// The customer account routes. Needs the `SiteHost` extension on every request (W2's host
/// router adds it in a layer outside this router).
pub fn router(visitors: Visitors) -> Router {
    Router::new()
        .route("/account/sign-in", get(sign_in_page))
        .route("/account/create", get(create_page))
        .route("/account/code", get(code_page))
        .route("/account/recovery-codes", get(recovery_codes_page))
        .route("/account/passkey-offer", get(passkey_offer_page))
        .route("/account/recover", get(recover_page))
        .route("/account/security", get(security_page))
        .route("/account/choices", get(choices_page))
        .route("/account/keeps", get(keeps_page))
        .route("/account/signed-out", get(signed_out_page))
        .route("/account/data", get(data_page))
        .route("/account/erased", get(erased_page))
        .route("/account/api/export", get(api_export))
        .route("/account/api/delete", post(api_delete))
        .route("/account/sign-out", post(sign_out_form))
        .route("/account/code/send", post(code_send_form))
        .route("/account/code/check", post(code_check_form))
        .route("/account/code/resend", post(code_resend_form))
        .route("/account/account.js", get(account_js))
        .route("/account/account.css", get(account_css))
        .route("/account/api/session", get(api_session))
        .route(
            "/account/api/passkey/register/start",
            post(api_register_start),
        )
        .route(
            "/account/api/passkey/register/finish",
            post(api_register_finish),
        )
        .route("/account/api/passkey/login/start", post(api_login_start))
        .route("/account/api/passkey/login/finish", post(api_login_finish))
        .route("/account/api/passkey/decline", post(api_decline))
        .route("/account/api/code/start", post(api_code_start))
        .route("/account/api/code/verify", post(api_code_verify))
        .route("/account/api/code/resend", post(api_code_resend))
        .route("/account/api/recovery/use", post(api_recovery_use))
        .route("/account/api/recovery/new", post(api_recovery_new))
        .route("/account/api/sessions/end", post(api_sessions_end))
        .route("/account/api/passkeys/remove", post(api_passkey_remove))
        .route("/account/api/alerts/dismiss", post(api_alerts_dismiss))
        .route(
            "/account/api/consents",
            get(api_consents).post(api_consent_set),
        )
        .layer(middleware::from_fn(same_origin))
        .with_state(visitors)
}

/// Answers 404 off a site host, and refuses a POST unless it comes from the site's own origin
/// (or, without an `Origin` header, says `Sec-Fetch-Site: same-origin`).
async fn same_origin(request: Request, next: Next) -> Response {
    let Some(site) = request.extensions().get::<SiteHost>() else {
        return VisitorError::NotASite.into_response();
    };
    if request.method() == Method::POST {
        let headers = request.headers();
        let ok = match headers.get(ORIGIN).and_then(|v| v.to_str().ok()) {
            Some(origin) => site.is_own_origin(origin),
            None => {
                headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("same-origin")
            }
        };
        if !ok {
            return VisitorError::CrossOrigin.into_response();
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

type Site = Extension<SiteHost>;

// ---- pages ------------------------------------------------------------------------------

fn page(
    visitors: &Visitors,
    site: &SiteHost,
    title: &str,
    body: String,
    visitor: Option<&Visitor>,
) -> Response {
    let html = visitors
        .inner
        .shell
        .page(site, title, &body, visitor.map(|v| v.name.as_str()));
    Html(html).into_response()
}

fn render<S: serde::Serialize>(
    visitors: &Visitors,
    site: &SiteHost,
    name: &str,
    title: &str,
    ctx: S,
    visitor: Option<&Visitor>,
) -> Response {
    match visitors.inner.views.render(name, ctx) {
        Ok(body) => page(visitors, site, title, body, visitor),
        Err(e) => e.into_response(),
    }
}

#[derive(Debug, Default, Deserialize)]
struct PageQuery {
    next: Option<String>,
    why: Option<String>,
    again: Option<String>,
    expired: Option<String>,
    /// A server-side prefill id (the address never travels in a URL).
    f: Option<String>,
    error: Option<String>,
    p: Option<String>,
    r: Option<String>,
    recovered: Option<String>,
}

fn error_words(code: Option<&str>) -> String {
    match code {
        Some("no-account") => {
            "There's no account for this email here yet. Create one below.".into()
        }
        Some("email-taken") => VisitorError::EmailTaken.to_string(),
        Some("too-many-tries") => VisitorError::TooManyTries.to_string(),
        Some("mail-failed") => VisitorError::MailFailed.to_string(),
        Some("wrong-code") => VisitorError::WrongCode.to_string(),
        Some("bad-email") => "That doesn't look like an email address.".into(),
        Some("bad-request") => "Check the form and try again.".into(),
        Some(_) => "That didn't work. Try again.".into(),
        None => String::new(),
    }
}

async fn sign_in_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    let next = next_of(q.next.as_deref(), "/account");
    if visitor.is_some() && q.again.is_none() {
        return Redirect::to(&next).into_response();
    }
    let (heading, why) = match q.why.as_deref() {
        Some("order") => (
            "Sign in to order".to_owned(),
            "So you can follow your order and get a receipt.",
        ),
        Some("save") => (
            "Sign in to save this".to_owned(),
            "So it's here next time, on any device.",
        ),
        Some("track") => (
            "Sign in to see your orders".to_owned(),
            "Your orders and receipts are in your account.",
        ),
        _ => (
            format!("Sign in to {}", site.name),
            "To see your orders, receipts and choices.",
        ),
    };
    let email = visitor
        .as_ref()
        .map(|v| v.email.clone())
        .unwrap_or_else(|| v.prefilled(&site, q.f.as_deref()).email);
    let ctx = json!({
        "next_q": enc(&next), "next": next, "heading": heading, "why": why, "site": site.name, "email": email,
        "again": q.again.is_some(), "expired": q.expired.is_some(), "error": error_words(q.error.as_deref()),
    });
    render(&v, &site, "sign-in.html", "Sign in", ctx, visitor.as_ref())
}

async fn create_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    let next = next_of(q.next.as_deref(), "/account");
    if visitor.is_some() {
        return Redirect::to(&next).into_response();
    }
    let form = v.prefilled(&site, q.f.as_deref());
    let ctx = json!({
        "next_q": enc(&next), "next": next, "site": site.name,
        "name": form.name, "email": form.email, "phone": form.phone,
        "error": error_words(q.error.as_deref()),
    });
    render(&v, &site, "create.html", "Create your account", ctx, None)
}

async fn code_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    let pending = q.p.unwrap_or_default();
    let Some((sent_to, purpose, mailbox, test_agent)) = v.code_view(&site, &pending) else {
        return render(
            &v,
            &site,
            "code-expired.html",
            "Code expired",
            json!({ "minutes": CODE_MINUTES }),
            visitor.as_ref(),
        );
    };
    let button = match purpose {
        CodePurpose::Create => "Create my account",
        CodePurpose::Verify => "Confirm my email",
        CodePurpose::SignIn | CodePurpose::Recover => "Sign in",
    };
    let ctx = json!({
        "pending": pending, "sent_to": sent_to, "minutes": CODE_MINUTES, "mailbox": mailbox, "button": button,
        "test_agent": test_agent,
        "site": site.name, "error": error_words(q.error.as_deref()),
    });
    render(
        &v,
        &site,
        "code.html",
        "Check your email",
        ctx,
        visitor.as_ref(),
    )
}

async fn recovery_codes_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    match v.take_shown_codes(&site, visitor.as_ref(), q.r.as_deref().unwrap_or_default()) {
        Some((codes, next)) => render(
            &v,
            &site,
            "recovery-codes.html",
            "Recovery codes",
            json!({ "codes": codes, "next": next, "host": site.host }),
            visitor.as_ref(),
        ),
        None => render(
            &v,
            &site,
            "recovery-gone.html",
            "Recovery codes",
            json!({}),
            visitor.as_ref(),
        ),
    }
}

async fn passkey_offer_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let next = next_of(q.next.as_deref(), "/account");
    let Some(visitor) = v.visitor(&site, &headers).await.ok().flatten() else {
        return super::sign_in_redirect(&next).into_response();
    };
    if session::declined(&headers) {
        return Redirect::to(&next).into_response();
    }
    render(
        &v,
        &site,
        "passkey-offer.html",
        "A passkey for next time",
        json!({ "next": next, "host": site.host }),
        Some(&visitor),
    )
}

async fn recover_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    let next = next_of(q.next.as_deref(), "/account");
    render(
        &v,
        &site,
        "recover.html",
        "Get back in",
        json!({ "next": next, "site": site.name }),
        visitor.as_ref(),
    )
}

fn method_words(method: &str) -> &'static str {
    match method {
        "passkey" => "with your passkey",
        "email-code" => "with a code by email",
        "recovery-code" => "with a recovery code",
        _ => "signed in",
    }
}

fn day(unix: i64) -> String {
    consent::when(unix).replace(" Nairobi time", "")
}

async fn security_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let Some(visitor) = v.visitor(&site, &headers).await.ok().flatten() else {
        return super::sign_in_redirect("/account/security").into_response();
    };
    match security_view(&v, &site, &visitor, q.recovered.is_some()).await {
        Ok(ctx) => render(
            &v,
            &site,
            "security.html",
            "Sign-in and security",
            ctx,
            Some(&visitor),
        ),
        Err(e) => e.into_response(),
    }
}

async fn security_view(
    v: &Visitors,
    site: &SiteHost,
    visitor: &Visitor,
    recovered: bool,
) -> Result<serde_json::Value, VisitorError> {
    let store = &v.inner.store;
    let events = store.sign_ins(&site.site, &visitor.account, 20).await?;
    let new_devices: Vec<&str> = events
        .iter()
        .filter(|e| e.new_device && e.dismissed_at.is_none())
        .map(|e| e.device.as_str())
        .collect();
    let sessions: Vec<_> = store
        .sessions(&site.site, &visitor.account)
        .await?
        .into_iter()
        .map(|s| {
            let current = s.id == visitor.session.id;
            json!({
                "id": s.id, "device": s.device, "current": current, "method_words": method_words(&s.method),
                "seen": day(s.last_seen_at), "new": !current && new_devices.contains(&s.device.as_str()),
            })
        })
        .collect();
    let passkeys: Vec<_> = store
        .credentials(&site.site, &visitor.account)
        .await?
        .into_iter()
        .map(|c| json!({ "id": c.id, "label": c.label, "made": day(c.created_at), "used": c.last_used_at.map(day) }))
        .collect();
    let alert = events
        .iter()
        .find(|e| e.new_device && e.dismissed_at.is_none())
        .map(|e| json!({ "device": e.device, "when": day(e.at) }));
    let events: Vec<_> = events
        .iter()
        .take(10)
        .map(|e| json!({ "device": e.device, "when": day(e.at), "method_words": method_words(&e.method) }))
        .collect();
    let codes_left = store
        .recovery_codes_left(&site.site, &visitor.account)
        .await?;
    Ok(json!({
        "recovered": recovered, "alert": alert, "sessions": sessions, "passkeys": passkeys, "events": events,
        "codes_left": codes_left, "email": visitor.email, "verified": visitor.email_verified,
        "host": site.host, "site": site.name,
    }))
}

async fn choices_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let Some(visitor) = v.visitor(&site, &headers).await.ok().flatten() else {
        return super::sign_in_redirect("/account/choices").into_response();
    };
    let next = q.next.as_deref().map(|n| next_of(Some(n), "/account"));
    match choices_view(&v, &site, &visitor).await {
        Ok(mut ctx) => {
            ctx["next"] = json!(next);
            render(
                &v,
                &site,
                "choices.html",
                "Your choices",
                ctx,
                Some(&visitor),
            )
        }
        Err(e) => e.into_response(),
    }
}

async fn choices_view(
    v: &Visitors,
    site: &SiteHost,
    visitor: &Visitor,
) -> Result<serde_json::Value, VisitorError> {
    let rows = v.inner.store.consents(&site.site, &visitor.account).await?;
    let offers = v.inner.shell.offers_line(site);
    let choices: Vec<_> = Purpose::ALL
        .into_iter()
        .map(|p| {
            let w = consent::words(p, site, visitor.phone.as_deref(), offers.as_deref());
            let on = rows.iter().any(|r| r.purpose == p.key() && r.withdrawn_at.is_none());
            let blocked = p == Purpose::OrderUpdates && visitor.phone.is_none() && !on;
            json!({ "key": w.key, "title": w.title, "line": w.line, "needs_adult": w.needs_adult, "on": on, "blocked": blocked })
        })
        .collect();
    let receipts: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            let body: consent::ConsentReceipt = serde_json::from_str(&r.body).ok()?;
            let short = Purpose::from_key(&r.purpose).map(|p| consent::words(p, site, None, None).short).unwrap_or("Consent");
            Some(json!({
                "id": r.id, "short": short, "purpose": body.purpose_text, "what": body.what, "shared_with": body.shared_with,
                "kept_for": body.kept_for, "to_withdraw": body.to_withdraw, "given": consent::when(r.given_at),
                "withdrawn": r.withdrawn_at.map(consent::when), "adult": body.adult_confirmed.unwrap_or(false),
                "short_id": consent::short_id(&r.id), "signed": r.receipt.is_some(),
                "digest": r.receipt.clone().unwrap_or_else(|| r.digest.clone()),
            }))
        })
        .collect();
    Ok(json!({ "choices": choices, "receipts": receipts, "host": site.host, "site": site.name }))
}

async fn keeps_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    render(
        &v,
        &site,
        "keeps.html",
        &format!("What {} keeps", site.name),
        json!({ "site": site.name, "host": site.host }),
        visitor.as_ref(),
    )
}

async fn data_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Response {
    let Some(visitor) = v.visitor(&site, &headers).await.ok().flatten() else {
        return super::sign_in_redirect("/account/data").into_response();
    };
    let others = v.other_store_names();
    render(
        &v,
        &site,
        "data.html",
        "Your data",
        json!({ "site": site.name, "other_stores": others }),
        Some(&visitor),
    )
}

async fn erased_page(
    State(v): State<Visitors>,
    Extension(site): Site,
    Query(q): Query<PageQuery>,
) -> Response {
    match v.erasure(&site, q.r.as_deref().unwrap_or_default()).await {
        Ok(Some((report, digest, _))) => {
            let stores = report["stores"].clone();
            let complete = stores
                .as_array()
                .is_some_and(|l| l.iter().all(|s| s["ok"] == true));
            let id = q.r.unwrap_or_default();
            let short = format!(
                "er-{}…{}",
                &id.trim_start_matches("er_")
                    .chars()
                    .take(4)
                    .collect::<String>(),
                id.chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            );
            let ctx = json!({
                "stores": stores, "complete": complete, "requested": report["requested_at"], "digest": digest,
                "short_id": short, "host": site.host, "site": site.name,
            });
            render(&v, &site, "erased.html", "Erasure receipt", ctx, None)
        }
        Ok(None) => render(
            &v,
            &site,
            "code-expired.html",
            "Not found",
            json!({ "minutes": CODE_MINUTES }),
            None,
        ),
        Err(e) => e.into_response(),
    }
}

async fn api_export(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    let bundle = v.export_bundle(&site, &visitor).await?;
    let name = format!("{}-my-data.json", site.host);
    let body =
        serde_json::to_string_pretty(&bundle).map_err(|e| VisitorError::Internal(e.to_string()))?;
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
        .map_err(|e| VisitorError::Internal(e.to_string()))?;
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response())
}

async fn api_delete(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    let id = v.delete_account(&site, &visitor).await?;
    let mut response =
        Json(json!({ "redirect": format!("/account/erased?r={}", enc(&id)) })).into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, session::clear_cookie(site.is_https()));
    Ok(response)
}

async fn signed_out_page(State(v): State<Visitors>, Extension(site): Site) -> Response {
    render(
        &v,
        &site,
        "signed-out.html",
        "Signed out",
        json!({ "site": site.name }),
        None,
    )
}

async fn sign_out_form(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = v.sign_out(&site, &headers).await {
        return e.into_response();
    }
    let mut response = Redirect::to("/account/signed-out").into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, session::clear_cookie(site.is_https()));
    response
}

// ---- forms (JavaScript off) -------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CodeSendForm {
    email: Option<String>,
    purpose: Option<String>,
    name: Option<String>,
    phone: Option<String>,
    next: Option<String>,
}

fn purpose_of(text: Option<&str>) -> CodePurpose {
    match text {
        Some("create") => CodePurpose::Create,
        Some("recover") => CodePurpose::Recover,
        Some("verify") => CodePurpose::Verify,
        _ => CodePurpose::SignIn,
    }
}

/// Back to the form with an error. What was typed travels as a server-side prefill id, never
/// in the URL.
fn back_with_error(path: &str, next: &str, prefill: Option<&str>, e: &VisitorError) -> Response {
    let code = match e {
        VisitorError::BadRequest(m) if m.contains("email") => "bad-email",
        other => other.code(),
    };
    let f = prefill
        .map(|f| format!("&f={}", enc(f)))
        .unwrap_or_default();
    Redirect::to(&format!("{path}?next={}{f}&error={code}", enc(next))).into_response()
}

async fn code_send_form(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Form(f): Form<CodeSendForm>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    let purpose = purpose_of(f.purpose.as_deref());
    let next = next_of(f.next.as_deref(), "/account");
    let email = f.email.unwrap_or_default();
    let (name, phone) = (f.name.clone(), f.phone.clone());
    match v
        .code_start(
            &site,
            visitor.as_ref(),
            &email,
            purpose,
            f.name.as_deref(),
            f.phone.as_deref(),
            next.clone(),
        )
        .await
    {
        Ok(sent) => Redirect::to(&sent.redirect).into_response(),
        Err(VisitorError::NoAccount) => {
            let f = v
                .prefill(&site, &email, "", "")
                .map(|f| format!("&f={}", enc(&f)))
                .unwrap_or_default();
            Redirect::to(&format!("/account/create?next={}{f}", enc(&next))).into_response()
        }
        Err(e) => {
            let back = match purpose {
                CodePurpose::Create => "/account/create",
                CodePurpose::Recover => "/account/recover",
                CodePurpose::Verify => "/account/security",
                CodePurpose::SignIn => "/account/sign-in",
            };
            let prefill = v.prefill(
                &site,
                &email,
                name.as_deref().unwrap_or_default(),
                phone.as_deref().unwrap_or_default(),
            );
            back_with_error(back, &next, prefill.as_deref(), &e)
        }
    }
}

#[derive(Debug, Deserialize)]
struct CodeCheckForm {
    pending: String,
    code: Option<String>,
}

fn with_cookie(mut response: Response, cookie: Option<HeaderValue>) -> Response {
    if let Some(cookie) = cookie {
        response.headers_mut().append(SET_COOKIE, cookie);
    }
    response
}

async fn code_check_form(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Form(f): Form<CodeCheckForm>,
) -> Response {
    let visitor = v.visitor(&site, &headers).await.ok().flatten();
    let declined = session::declined(&headers);
    match v
        .code_verify(
            &site,
            visitor.as_ref(),
            &f.pending,
            f.code.as_deref().unwrap_or_default(),
            &headers,
            declined,
        )
        .await
    {
        Ok((done, cookie)) => with_cookie(Redirect::to(&done.redirect).into_response(), cookie),
        Err(VisitorError::Expired) => {
            Redirect::to(&format!("/account/code?p={}", enc(&f.pending))).into_response()
        }
        Err(e) => Redirect::to(&format!(
            "/account/code?p={}&error={}",
            enc(&f.pending),
            e.code()
        ))
        .into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct PendingForm {
    pending: String,
}

async fn code_resend_form(
    State(v): State<Visitors>,
    Extension(site): Site,
    Form(f): Form<PendingForm>,
) -> Response {
    match v.code_resend(&site, &f.pending).await {
        Ok(sent) => Redirect::to(&sent.redirect).into_response(),
        Err(e) => Redirect::to(&format!(
            "/account/code?p={}&error={}",
            enc(&f.pending),
            e.code()
        ))
        .into_response(),
    }
}

// ---- static -----------------------------------------------------------------------------

async fn account_js() -> Response {
    (
        [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
        ACCOUNT_JS,
    )
        .into_response()
}

async fn account_css() -> Response {
    ([(CONTENT_TYPE, "text/css; charset=utf-8")], ACCOUNT_CSS).into_response()
}

// ---- JSON API ---------------------------------------------------------------------------

async fn api_session(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Result<Response, VisitorError> {
    let visitor = v.visitor(&site, &headers).await?;
    let body = match visitor {
        Some(visitor) => {
            let has_passkey = !v
                .inner
                .store
                .credentials(&site.site, &visitor.account)
                .await?
                .is_empty();
            json!({ "signed_in": true, "account": {
                "id": visitor.account, "name": visitor.name, "email": visitor.email,
                "email_verified": visitor.email_verified, "has_passkey": has_passkey,
            }})
        }
        None => json!({ "signed_in": false }),
    };
    Ok(Json(body).into_response())
}

#[derive(Debug, Default, Deserialize)]
struct RegisterStart {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    next: Option<String>,
}

async fn api_register_start(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<RegisterStart>,
) -> Result<Response, VisitorError> {
    let visitor = v.visitor(&site, &headers).await?;
    let next = next_of(
        body.next.as_deref(),
        if visitor.is_some() {
            "/account/security"
        } else {
            "/account"
        },
    );
    let new = match (&visitor, body.name, body.email) {
        (None, Some(name), Some(email)) => Some((name, email, body.phone)),
        _ => None,
    };
    let (pending, options) = v
        .passkey_register_start(&site, visitor.as_ref(), new, next)
        .await?;
    Ok(Json(json!({ "pending": pending, "options": options })).into_response())
}

#[derive(Debug, Deserialize)]
struct RegisterFinish {
    pending: String,
    credential: RegisterPublicKeyCredential,
}

async fn api_register_finish(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<RegisterFinish>,
) -> Result<Response, VisitorError> {
    let (done, cookie) = v
        .passkey_register_finish(&site, &body.pending, &body.credential, &headers)
        .await?;
    Ok(with_cookie(Json(done).into_response(), cookie))
}

#[derive(Debug, Deserialize)]
struct LoginStart {
    email: String,
    next: Option<String>,
}

async fn api_login_start(
    State(v): State<Visitors>,
    Extension(site): Site,
    Json(body): Json<LoginStart>,
) -> Result<Response, VisitorError> {
    let next = next_of(body.next.as_deref(), "/account");
    match v
        .passkey_login_start(&site, &body.email, next.clone())
        .await
    {
        Ok((pending, options)) => {
            Ok(Json(json!({ "pending": pending, "options": options })).into_response())
        }
        // New here: the same button leads to creating an account, with the address kept on
        // the server rather than in the URL.
        Err(VisitorError::NoAccount) => {
            let f = v
                .prefill(&site, &body.email, "", "")
                .map(|f| format!("&f={}", enc(&f)))
                .unwrap_or_default();
            let e = VisitorError::NoAccount;
            Ok((
                StatusCode::NOT_FOUND,
                Json(json!({ "message": e.to_string(), "code": e.code(), "redirect": format!("/account/create?next={}{f}", enc(&next)) })),
            )
                .into_response())
        }
        Err(e) => Err(e),
    }
}

#[derive(Debug, Deserialize)]
struct LoginFinish {
    pending: String,
    credential: PublicKeyCredential,
}

async fn api_login_finish(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<LoginFinish>,
) -> Result<Response, VisitorError> {
    let signed = v
        .passkey_login_finish(&site, &body.pending, &body.credential, &headers)
        .await?;
    Ok(with_cookie(
        Json(signed.done).into_response(),
        Some(signed.cookie),
    ))
}

async fn api_decline(Extension(site): Site) -> Response {
    let mut response = Json(json!({ "declined": true })).into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, session::declined_cookie(site.is_https()));
    response
}

#[derive(Debug, Deserialize)]
struct CodeStart {
    email: Option<String>,
    purpose: Option<String>,
    name: Option<String>,
    phone: Option<String>,
    next: Option<String>,
}

async fn api_code_start(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<CodeStart>,
) -> Result<Response, VisitorError> {
    let visitor = v.visitor(&site, &headers).await?;
    let next = next_of(body.next.as_deref(), "/account");
    let sent = v
        .code_start(
            &site,
            visitor.as_ref(),
            body.email.as_deref().unwrap_or_default(),
            purpose_of(body.purpose.as_deref()),
            body.name.as_deref(),
            body.phone.as_deref(),
            next,
        )
        .await?;
    Ok(Json(sent).into_response())
}

#[derive(Debug, Deserialize)]
struct CodeVerify {
    pending: String,
    code: String,
}

async fn api_code_verify(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<CodeVerify>,
) -> Result<Response, VisitorError> {
    let visitor = v.visitor(&site, &headers).await?;
    let declined = session::declined(&headers);
    let (done, cookie) = v
        .code_verify(
            &site,
            visitor.as_ref(),
            &body.pending,
            &body.code,
            &headers,
            declined,
        )
        .await?;
    Ok(with_cookie(Json(done).into_response(), cookie))
}

#[derive(Debug, Deserialize)]
struct Pending {
    pending: String,
}

async fn api_code_resend(
    State(v): State<Visitors>,
    Extension(site): Site,
    Json(body): Json<Pending>,
) -> Result<Response, VisitorError> {
    Ok(Json(v.code_resend(&site, &body.pending).await?).into_response())
}

#[derive(Debug, Deserialize)]
struct RecoveryUse {
    email: String,
    code: String,
}

async fn api_recovery_use(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<RecoveryUse>,
) -> Result<Response, VisitorError> {
    let signed = v
        .recovery_use(&site, &body.email, &body.code, &headers)
        .await?;
    Ok(with_cookie(
        Json(signed.done).into_response(),
        Some(signed.cookie),
    ))
}

async fn signed_in(
    v: &Visitors,
    site: &SiteHost,
    headers: &HeaderMap,
) -> Result<Visitor, VisitorError> {
    v.visitor(site, headers)
        .await?
        .ok_or(VisitorError::SignedOut)
}

async fn api_recovery_new(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    let done: Done = v.recovery_new(&site, &visitor).await?;
    Ok(Json(done).into_response())
}

#[derive(Debug, Deserialize)]
struct SessionsEnd {
    id: Option<String>,
    #[serde(default)]
    all: bool,
}

async fn api_sessions_end(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<SessionsEnd>,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    if body.all {
        let ended = v.end_sessions(&site, &visitor, None).await?;
        let mut response = Json(
            json!({ "ended": ended, "redirect": "/account/sign-in?next=%2Faccount%2Fsecurity" }),
        )
        .into_response();
        response
            .headers_mut()
            .insert(SET_COOKIE, session::clear_cookie(site.is_https()));
        return Ok(response);
    }
    let id = body
        .id
        .ok_or_else(|| VisitorError::BadRequest("Say which session to end.".into()))?;
    let ended = v.end_sessions(&site, &visitor, Some(&id)).await?;
    Ok(Json(json!({ "ended": ended })).into_response())
}

#[derive(Debug, Deserialize)]
struct PasskeyRemove {
    id: String,
}

async fn api_passkey_remove(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<PasskeyRemove>,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    let removed = v.remove_passkey(&site, &visitor, &body.id).await?;
    Ok(Json(json!({ "removed": removed })).into_response())
}

async fn api_alerts_dismiss(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    v.dismiss_alerts(&site, &visitor).await?;
    Ok(Json(json!({ "dismissed": true })).into_response())
}

async fn api_consents(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    Ok(Json(choices_view(&v, &site, &visitor).await?).into_response())
}

#[derive(Debug, Deserialize)]
struct ConsentSet {
    purpose: String,
    on: bool,
    #[serde(default)]
    adult: bool,
}

async fn api_consent_set(
    State(v): State<Visitors>,
    Extension(site): Site,
    headers: HeaderMap,
    Json(body): Json<ConsentSet>,
) -> Result<Response, VisitorError> {
    let visitor = signed_in(&v, &site, &headers).await?;
    let purpose = Purpose::from_key(&body.purpose)
        .ok_or_else(|| VisitorError::BadRequest("That choice doesn't exist.".into()))?;
    let receipt = v
        .set_consent(&site, &visitor, purpose, body.on, body.adult)
        .await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "receipt": receipt, "on": body.on })),
    )
        .into_response())
}
