//! The apps' routes and pages.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use saas_core::PayPalRef;
use saas_core::identity::{Badge, BadgeStatus};
use saas_core::wallet::{Authority, Reconciled, WalletCause, WalletFeed, WalletSource};

use super::{AppError, Badges, DayRunner, Moment, SiteApps};
use crate::{host_of, pages, passkeys};

type Apps = Arc<SiteApps>;

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, axum::Json(json!({ "message": message.into() }))).into_response()
}

fn app_error(e: &AppError) -> Response {
    let status = match e {
        AppError::NotConnected => StatusCode::SERVICE_UNAVAILABLE,
        AppError::NotFound => StatusCode::NOT_FOUND,
        AppError::Changed => StatusCode::CONFLICT,
        AppError::Expired => StatusCode::GONE,
        AppError::Refused(_) => StatusCode::BAD_GATEWAY,
    };
    json_error(status, e.to_string())
}

/// One site's console apps, at paths relative to wherever it is nested (see the module docs).
///
/// Every path starts with its app's id (`passkey`, `badges`, `wallet`), so the same router
/// serves Agentrix OS's apps at `/console/{site}/apps/{id}/…` ([`crate::os`]). Every JSON read
/// also answers a POST, because a shell app's GET returns a page.
pub fn console_router(apps: SiteApps) -> Router {
    Router::new()
        .route("/passkey", get(passkey_page))
        .route(
            "/passkey/setup/options",
            get(setup_options).post(setup_options),
        )
        .route("/passkey/setup", post(setup_finish))
        .route("/passkey/link-paypal", get(link_paypal))
        .route("/passkey/{moment}/{id}", get(moment_page))
        .route(
            "/passkey/{moment}/{id}/challenge",
            get(moment_challenge).post(moment_challenge),
        )
        .route("/passkey/{moment}/{id}/sign", post(moment_sign))
        .route("/badges", get(badges_page))
        .route("/badges/verify", get(verify).post(verify_post))
        .route("/badges/verify-token", post(verify_token))
        .route("/wallet", get(wallet_page))
        .route("/wallet/grid.json", get(wallet_json).post(wallet_json))
        .route("/wallet/feed.json", get(wallet_feed))
        .with_state(Arc::new(apps))
}

/// `/static/saas/…`: the face's scripts and styles, and the built AG Grid wallet component at
/// `/static/saas/wallet-grid.js` (from `grid_js`, `web/saas-board/dist/assets/wallet-grid.js`).
pub fn saas_assets<S: Clone + Send + Sync + 'static>(grid_js: Option<PathBuf>) -> Router<S> {
    Router::new()
        .route(
            "/static/saas/wallet-grid.js",
            get(move || {
                let path = grid_js.clone();
                async move { grid_js_file(path.as_deref()) }
            }),
        )
        .route("/static/saas/{*path}", get(pages::asset))
}

/// A page of the apps: whole, or only its field when a host shell draws the rest.
fn render_app(apps: &SiteApps, status: StatusCode, name: &str, view: Value) -> Response {
    if apps.fragment {
        pages::render_fragment(status, name, view)
    } else {
        pages::render_status(status, name, view)
    }
}

/// What a sample site shows anyone: its agents' badges and the Verify box.
pub fn public_router(apps: SiteApps) -> Router {
    Router::new()
        .route("/agents", get(agents_page))
        .route("/agents/check", get(verify).post(verify_post))
        .route("/agents/check-token", post(verify_token))
        .with_state(Arc::new(apps))
}

fn shell_view(apps: &SiteApps, current: &str) -> Value {
    json!({
        "site": {"name": apps.name, "sub": "Agentrix OS", "monogram": apps.name.chars().next().unwrap_or('A').to_string(), "href": apps.base},
        "base": apps.base,
        "current": current,
        "owner": apps.owner,
        "site_id": apps.site,
    })
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
    a
}

// ---- the owner's passkey and its moments --------------------------------------------------

async fn passkey_page(State(apps): State<Apps>, headers: HeaderMap) -> Response {
    let party = passkeys::Party::for_host(host_of(&headers)).ok();
    let ready = party
        .as_ref()
        .is_some_and(|p| apps.passkeys.has(&apps.site, p));
    let pending = apps.ceremonies.pending(&apps.site).await;
    let (moments, note) = match pending {
        Ok(list) => (list, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    let moments: Vec<Value> = moments
        .iter()
        .map(|s| json!({"moment": s.moment.as_str(), "words": s.moment.words(), "id": s.id, "title": s.title, "state": s.state, "reason": s.reason, "href": format!("{}/passkey/{}/{}", apps.base, s.moment.as_str(), s.id)}))
        .collect();
    let view = merge(
        shell_view(&apps, "passkey"),
        json!({
            "ready": ready,
            "rp_id": party.map(|p| p.rp_id),
            "may_act": apps.access.allows(&headers),
            "moments": moments,
            "note": note,
        }),
    );
    render_app(&apps, StatusCode::OK, "saas/apps/passkey.html", view)
}

async fn setup_options(State(apps): State<Apps>, headers: HeaderMap) -> Response {
    if !apps.access.allows(&headers) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Only the owner can set up this console's passkey.",
        );
    }
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    match apps
        .passkeys
        .start_registration(&apps.site, &apps.owner, &party)
    {
        Ok(options) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(options)).into_response(),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn setup_finish(
    State(apps): State<Apps>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !apps.access.allows(&headers) {
        return json_error(
            StatusCode::FORBIDDEN,
            "Only the owner can set up this console's passkey.",
        );
    }
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let Ok(credential) = serde_json::from_slice(&body) else {
        return json_error(StatusCode::BAD_REQUEST, "That isn't a passkey.");
    };
    match apps
        .passkeys
        .finish_registration(&apps.site, &credential, &party)
    {
        Ok(()) => axum::Json(json!({"message": format!("Passkey set up for {}.", party.rp_id)}))
            .into_response(),
        Err(e) => json_error(StatusCode::UNAUTHORIZED, e.to_string()),
    }
}

fn moment_of(m: &str) -> Result<Moment, Box<Response>> {
    Moment::parse(m).ok_or_else(|| Box::new(json_error(StatusCode::NOT_FOUND, "No such moment.")))
}

/// The ceremony's key in the passkey store: the moment and its subject.
fn ceremony_key(moment: Moment, id: &str) -> String {
    format!("{}:{id}", moment.as_str())
}

async fn moment_page(
    State(apps): State<Apps>,
    Path((moment, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Ok(moment) = moment_of(&moment) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let party = passkeys::Party::for_host(host_of(&headers)).ok();
    let ready = party
        .as_ref()
        .is_some_and(|p| apps.passkeys.has(&apps.site, p));
    let (subject, error) = match apps.ceremonies.subject(&apps.site, moment, &id).await {
        Ok(s) => (Some(s), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let here = format!("{}/passkey/{}/{}", apps.base, moment.as_str(), id);
    let view = merge(
        shell_view(&apps, "passkey"),
        json!({
            "moment": moment.as_str(),
            "moment_words": moment.words(),
            "subject": subject,
            "error": error,
            "ready": ready,
            "rp_id": party.map(|p| p.rp_id),
            "may_act": apps.access.allows(&headers),
            "challenge_url": format!("{here}/challenge"),
            "sign_url": format!("{here}/sign"),
        }),
    );
    let status = if error.is_some() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::OK
    };
    render_app(&apps, status, "saas/apps/moment.html", view)
}

async fn moment_challenge(
    State(apps): State<Apps>,
    Path((moment, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !apps.access.allows(&headers) {
        return json_error(StatusCode::FORBIDDEN, "Only the owner can sign this.");
    }
    let moment = match moment_of(&moment) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    if !apps.passkeys.has(&apps.site, &party) {
        return json_error(
            StatusCode::PRECONDITION_REQUIRED,
            "Set up your passkey first.",
        );
    }
    let subject = match apps.ceremonies.subject(&apps.site, moment, &id).await {
        Ok(s) if s.state == "pending" => s,
        Ok(s) => {
            return json_error(
                StatusCode::CONFLICT,
                format!("This is already {}.", s.state),
            );
        }
        Err(e) => return app_error(&e),
    };
    match apps.passkeys.start_approval(
        &apps.site,
        &ceremony_key(moment, &id),
        &subject.fingerprint,
        &party,
    ) {
        Ok(options) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(options)).into_response(),
        Err(e) => json_error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn moment_sign(
    State(apps): State<Apps>,
    Path((moment, id)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !apps.access.allows(&headers) {
        return json_error(StatusCode::FORBIDDEN, "Only the owner can sign this.");
    }
    let moment = match moment_of(&moment) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let party = match passkeys::Party::for_host(host_of(&headers)) {
        Ok(p) => p,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let credential = body.get("proof").cloned().unwrap_or(body);
    let Ok(credential) = serde_json::from_value(credential) else {
        return json_error(StatusCode::BAD_REQUEST, "Sign with your passkey.");
    };
    let evidence = match apps.passkeys.finish_approval(
        &apps.site,
        &ceremony_key(moment, &id),
        &credential,
        &party,
    ) {
        Ok(e) => e,
        Err(e) => return json_error(StatusCode::UNAUTHORIZED, e.to_string()),
    };
    // What was signed must still be what is sealed: an edit since voids the signature.
    match apps.ceremonies.subject(&apps.site, moment, &id).await {
        Ok(s)
            if Some(s.fingerprint.as_str())
                == evidence.get("fingerprint").and_then(Value::as_str) => {}
        Ok(_) => return app_error(&AppError::Changed),
        Err(e) => return app_error(&e),
    }
    match apps
        .ceremonies
        .seal(&apps.site, moment, &id, evidence)
        .await
    {
        Ok(sealed) => {
            let message = match sealed.audit_seq {
                Some(seq) => format!("Signed with your passkey and sealed: audit log entry {seq}."),
                None => {
                    "Signed with your passkey. Not sealed in an audit log on this node.".to_owned()
                }
            };
            axum::Json(json!({"sealed": sealed.sealed, "auditSeq": sealed.audit_seq, "record": sealed.record, "message": message})).into_response()
        }
        Err(e) => app_error(&e),
    }
}

// ---- badges and Verify -------------------------------------------------------------------

fn short(digest: &str) -> String {
    match digest.get(..23) {
        Some(head) if digest.len() > 23 => format!("{head}…"),
        _ => digest.to_owned(),
    }
}

/// A badge's own document address: https, or http for a local alias.
fn badge_url(host: &str, role: &str) -> String {
    let name = host.split(':').next().unwrap_or(host);
    let scheme = if name == "localhost" || name.ends_with(".localhost") {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host}/agents/{role}/badge.json")
}

/// One badge as the cards show it.
fn badge_row(p: &Badge) -> Value {
    let when = |t: &chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%d %H:%M UTC").to_string();
    json!({
        "role": p.role,
        "name": p.name,
        "did": p.id,
        "urn": p.also_known_as.join(", "),
        "site": p.site.name,
        "site_did": p.site.did,
        "sample": p.site.sample,
        "sponsor": p.sponsor.name.clone().unwrap_or_else(|| p.sponsor.owner.clone()),
        "passkey": p.sponsor.passkey.as_deref().map(|id| format!("passkey {}", short(id))),
        "approved_at": p.sponsor.approved_at.as_ref().map(when),
        "keys": p.keys.iter().map(|k| k.kid.clone()).collect::<Vec<_>>().join(", "),
        "paypal": p.paypal.as_ref().map(|b| json!({
            "binding": b.binding_id,
            "digest": short(&b.digest),
            "environment": b.environment,
            "tail": b.payment_token_tail,
        })),
        "mandate": p.mandate.as_ref().map(|m| json!({"summary": m.summary, "digest": short(&m.digest), "expires": when(&m.expires_at)})),
        "wallet": p.wallet.as_ref().map(|w| {
            let state = match w.reconciled {
                Reconciled::Matches => "matches PayPal",
                Reconciled::Drift => "drifts from PayPal",
                Reconciled::Unchecked => "not yet compared with PayPal",
            };
            format!("{} entries, {state} · root {}", w.sequence, short(&w.root))
        }),
        "voucher": p.voucher.as_deref().map(short),
        "frozen": p.status == BadgeStatus::Frozen,
        "expires": when(&p.expires_at),
        "badge_url": badge_url(&p.site.host, &p.role),
    })
}

async fn badges_view(apps: &SiteApps) -> Value {
    let (badges, error) = match apps.badges.badges(&apps.site).await {
        Ok(a) => (a.iter().map(badge_row).collect::<Vec<_>>(), None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    json!({"badges": badges, "compact": false, "error": error, "note": apps.badges.note(), "verify_url": format!("{}/badges/verify", apps.base), "token_url": format!("{}/badges/verify-token", apps.base)})
}

async fn badges_page(State(apps): State<Apps>) -> Response {
    let view = merge(shell_view(&apps, "badges"), badges_view(&apps).await);
    render_app(&apps, StatusCode::OK, "saas/apps/badges.html", view)
}

async fn agents_page(State(apps): State<Apps>) -> Response {
    let mut view = badges_view(&apps).await;
    view["verify_url"] = json!("/agents/check");
    view["token_url"] = json!("/agents/check-token");
    view["name"] = json!(apps.name);
    pages::render("saas/apps/agents.html", view)
}

#[derive(Deserialize)]
struct DidQuery {
    did: Option<String>,
}

fn did_of(q: &DidQuery) -> Option<&str> {
    q.did
        .as_deref()
        .map(str::trim)
        .filter(|d| d.starts_with("did:") && d.len() <= 512 && !d.chars().any(char::is_whitespace))
}

async fn verify_with(badges: &dyn Badges, q: &DidQuery) -> Response {
    let Some(did) = did_of(q) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "That isn't a DID. Agent DIDs start with did:web:",
        );
    };
    match badges.verify(did).await {
        Ok(v) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(v)).into_response(),
        Err(e) => app_error(&e),
    }
}

async fn verify(State(apps): State<Apps>, Query(q): Query<DidQuery>) -> Response {
    verify_with(apps.badges.as_ref(), &q).await
}

/// `{"did": …}` in a POST's body.
fn did_body(body: &[u8]) -> DidQuery {
    serde_json::from_slice(body).unwrap_or(DidQuery { did: None })
}

async fn verify_post(State(apps): State<Apps>, body: axum::body::Bytes) -> Response {
    verify_with(apps.badges.as_ref(), &did_body(&body)).await
}

#[derive(Deserialize)]
struct TokenBody {
    token: Option<String>,
}

/// `POST {"token": …}`: who an agent token names, and who vouched. The token is never logged.
async fn verify_token(State(apps): State<Apps>, body: axum::body::Bytes) -> Response {
    let token = serde_json::from_slice::<TokenBody>(&body)
        .ok()
        .and_then(|b| b.token)
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty() && t.len() <= 16 * 1024 && !t.chars().any(char::is_whitespace));
    let Some(token) = token else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "Paste the agent's token: one line, as it was issued.",
        );
    };
    match apps.badges.verify_token(&token).await {
        Ok(v) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(v)).into_response(),
        Err(e) => app_error(&e),
    }
}

// ---- the wallet -----------------------------------------------------------------------------

/// AG Grid themes the grid by injecting styles, so the wallet page allows inline styles (and
/// nothing else beyond the kit's CSP).
const WALLET_CSP: &str = "default-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; font-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

fn paypal_id(r: &PayPalRef) -> String {
    match r {
        PayPalRef::Capture(id) => id.to_string(),
        PayPalRef::SubscriptionSale(id) => id.to_string(),
        PayPalRef::Refund(id) => id.to_string(),
        PayPalRef::Payout(id) => id.to_string(),
    }
}

fn name_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn cause_text(c: &WalletCause) -> String {
    match c {
        WalletCause::Order { order, reference } => reference
            .clone()
            .unwrap_or_else(|| format!("order {order}")),
        WalletCause::Invoice { invoice, number } => {
            number.clone().unwrap_or_else(|| invoice.0.clone())
        }
        WalletCause::Bill { bill, number } => number.clone().unwrap_or_else(|| bill.0.clone()),
        WalletCause::Subscription { subscription } => format!("subscription {subscription}"),
    }
}

fn authority_text(a: &Authority) -> String {
    match a {
        Authority::Mandate { mandate_digest, .. } if mandate_digest.is_empty() => "mandate".into(),
        Authority::Mandate { mandate_digest, .. } => format!("mandate {}", short(mandate_digest)),
        Authority::Approval { approved_by, .. } => format!("approved by {approved_by}"),
        Authority::PayerApproved => "payer approved at PayPal".into(),
    }
}

fn source_text(s: &WalletSource) -> String {
    match s {
        WalletSource::Webhook {
            event_type,
            event_id,
        } => format!("webhook {event_type} {event_id}"),
        WalletSource::ReadBack { call } => format!("read back: {call}"),
    }
}

/// The wallet as the grid draws it: flat rows for the ledger and the three-way match.
pub fn grid_view(feed: &WalletFeed, note: Option<String>) -> Value {
    let entries: Vec<Value> = feed
        .entries
        .iter()
        .map(|e| {
            json!({
                "seq": e.seq,
                "at": e.at.format("%Y-%m-%d %H:%M").to_string(),
                "kind": name_of(&e.kind),
                "agent": e.acting.name(),
                "currency": e.amount.currency.code(),
                "amountMinor": e.amount.minor,
                "paypalId": paypal_id(&e.paypal),
                "paypalStatus": e.paypal_status,
                "counterparty": e.counterparty.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
                "cause": e.cause.as_ref().map(cause_text).unwrap_or_default(),
                "authority": authority_text(&e.authority),
                "source": source_text(&e.source),
            })
        })
        .collect();
    let matches: Vec<Value> = feed
        .matches
        .iter()
        .map(|m| {
            json!({
                "seq": m.seq,
                "kind": name_of(&m.kind),
                "currency": m.amount.currency.code(),
                "amountMinor": m.amount.minor,
                "paypalId": paypal_id(&m.paypal),
                "paypalStatus": m.paypal_status,
                "paypalAmountMinor": m.paypal_amount.map(|a| a.minor),
                "booksAmountMinor": m.books_amount.map(|a| a.minor),
                "payment": m.books.payment.as_ref().map(|p| p.0.clone()),
                "journal": m.books.journal.as_ref().map(|j| j.0.clone()),
                "feeJournal": m.books.fee_journal.as_ref().map(|j| j.0.clone()),
                "paymentNumber": m.books.payment_number,
                "journalNumber": m.books.journal_number,
                "feeJournalNumber": m.books.fee_journal_number,
                "state": name_of(&m.state),
            })
        })
        .collect();
    let checkpoint = |c: &saas_core::wallet::WalletCheckpointSummary| json!({"words": c.words, "state": name_of(&c.state), "size": c.size, "root": c.root_hash, "checkedAt": c.checked_at});
    json!({
        "wallet": feed.wallet.to_string(),
        "checkpoint": feed.checkpoint.as_ref().map(checkpoint),
        "history": feed.history.iter().rev().map(checkpoint).collect::<Vec<_>>(),
        "note": note,
        "entries": entries,
        "matches": matches,
    })
}

async fn wallet_page(State(apps): State<Apps>) -> Response {
    let (view, error) = match apps.wallet.wallet(&apps.site).await {
        Ok(w) => (grid_view(&w, apps.wallet.note()), None),
        Err(e) => (Value::Null, Some(e.to_string())),
    };
    let empty = Vec::new();
    let entries = view
        .get("entries")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let matches = view
        .get("matches")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let in_books = matches
        .iter()
        .filter(|m| m.get("state").and_then(Value::as_str) == Some("matched"))
        .count();
    let three_way = matches
        .iter()
        .filter(|m| {
            m.get("state").and_then(Value::as_str) == Some("matched")
                && m.get("paypalStatus").is_some_and(|s| !s.is_null())
        })
        .count();
    let rows: Vec<Value> = entries
        .iter()
        .map(|e| {
            let minor = e.get("amountMinor").and_then(Value::as_i64).unwrap_or(0);
            let currency = e.get("currency").and_then(Value::as_str).unwrap_or("USD");
            json!({
                "seq": e.get("seq").cloned().unwrap_or(Value::Null).to_string(),
                "at": e.get("at").and_then(Value::as_str).unwrap_or_default(),
                "kind": e.get("kind").and_then(Value::as_str).unwrap_or_default(),
                "amount": crate::feed::money(minor, currency),
                "paypal_status": e.get("paypalStatus").and_then(Value::as_str).unwrap_or_default(),
                "paypal": e.get("paypalId").and_then(Value::as_str).unwrap_or_default(),
                "counterparty": e.get("counterparty").and_then(Value::as_str).unwrap_or_default(),
                "cause": e.get("cause").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    let page = merge(
        shell_view(&apps, "wallet"),
        json!({
            "error": error,
            "wallet_id": view.get("wallet"),
            "note": view.get("note"),
            "entries": entries.len(),
            "rows": rows,
            "in_books": in_books,
            "three_way": three_way,
            "matches": matches.len(),
            "checkpoint": view.get("checkpoint"),
            "history": view.get("history").and_then(Value::as_array).map(|h| h.iter().map(|c| json!({
                "checked": c.get("checkedAt").and_then(Value::as_str).map(|t| t.get(..16).unwrap_or(t).replace('T', " ")).unwrap_or_default(),
                "size": c.get("size").cloned().unwrap_or(Value::Null).to_string(),
                "root": c.get("root").and_then(Value::as_str).map(short).unwrap_or_default(),
                "words": c.get("words").and_then(Value::as_str).unwrap_or_default(),
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "json_url": format!("{}/wallet/grid.json", apps.base),
            "feed_url": format!("{}/wallet/feed.json", apps.base),
            "grid_url": format!("/static/saas/wallet-grid.js?v={}", pages::assets_version()),
        }),
    );
    let mut r = render_app(&apps, StatusCode::OK, "saas/apps/wallet.html", page);
    if let Ok(v) = header::HeaderValue::from_str(WALLET_CSP) {
        r.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v);
    }
    r
}

/// The grid's view of the wallet.
async fn wallet_json(State(apps): State<Apps>) -> Response {
    match apps.wallet.wallet(&apps.site).await {
        Ok(w) => (
            [(header::CACHE_CONTROL, "no-store")],
            axum::Json(grid_view(&w, apps.wallet.note())),
        )
            .into_response(),
        Err(e) => app_error(&e),
    }
}

/// The wallet feed as the contract has it (`saas_core::wallet::WalletFeed`), for anyone.
async fn wallet_feed(State(apps): State<Apps>) -> Response {
    match apps.wallet.wallet(&apps.site).await {
        Ok(w) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(w)).into_response(),
        Err(e) => app_error(&e),
    }
}

/// The owner's Link PayPal page: saas-badge's binding desk returns them here with
/// `?binding=<id>`; the page reads the draft from the desk and asks for the passkey.
async fn link_paypal(
    State(apps): State<Apps>,
    Query(q): Query<BindingQuery>,
    headers: HeaderMap,
) -> Response {
    let binding = q.binding.filter(|b| {
        !b.is_empty()
            && b.len() <= 128
            && b.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    });
    let view = merge(
        shell_view(&apps, "passkey"),
        json!({
            "binding": binding,
            "api": apps.paypal_link,
            "may_act": apps.access.allows(&headers),
        }),
    );
    render_app(&apps, StatusCode::OK, "saas/apps/link_paypal.html", view)
}

#[derive(Deserialize)]
struct BindingQuery {
    binding: Option<String>,
}

fn grid_js_file(path: Option<&std::path::Path>) -> Response {
    match path.and_then(|p| std::fs::read(p).ok()) {
        Some(js) => (
            [
                (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            js,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ---- judges ---------------------------------------------------------------------------------

/// What the judges' page needs: the day's runner (judge mode only) and a verifier.
#[derive(Clone)]
pub struct JudgeKit {
    /// "Run the day", when judge mode is on.
    pub runner: Option<Arc<dyn DayRunner>>,
    /// The Verify box's verifier.
    pub badges: Arc<dyn Badges>,
}

/// `POST /judges/run-day`, `GET /judges/run-day/{run}`, `GET /judges/verify?did=`.
pub fn judge_routes(kit: JudgeKit) -> Router {
    Router::new()
        .route("/judges/run-day", post(run_day))
        .route("/judges/run-day/{run}", get(run_status))
        .route("/judges/verify", get(judge_verify).post(judge_verify_post))
        .with_state(Arc::new(kit))
}

async fn run_day(State(kit): State<Arc<JudgeKit>>) -> Response {
    let Some(runner) = kit.runner.as_ref() else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Judge mode isn't on for this node.",
        );
    };
    match runner.start().await {
        Ok(run) => (StatusCode::ACCEPTED, axum::Json(json!({"run": run}))).into_response(),
        Err(e) => app_error(&e),
    }
}

async fn run_status(State(kit): State<Arc<JudgeKit>>, Path(run): Path<String>) -> Response {
    let Some(runner) = kit.runner.as_ref() else {
        return json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Judge mode isn't on for this node.",
        );
    };
    if run.len() > 128
        || !run
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    match runner.status(&run).await {
        Ok(v) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(v)).into_response(),
        Err(e) => app_error(&e),
    }
}

async fn judge_verify(State(kit): State<Arc<JudgeKit>>, Query(q): Query<DidQuery>) -> Response {
    verify_with(kit.badges.as_ref(), &q).await
}

async fn judge_verify_post(State(kit): State<Arc<JudgeKit>>, body: axum::body::Bytes) -> Response {
    verify_with(kit.badges.as_ref(), &did_body(&body)).await
}
