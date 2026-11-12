//! The console's Messages page: what was sent, or would have been, to whom and why.
//!
//! Routes on the platform host, for a signed-in owner:
//! - `GET /console/messages` with `?status=` and `?channel=` filters;
//! - `GET /console/messages/{id}`, one message as it was sent;
//! - `GET /console/messages/{id}/view`, the stored email for a sandboxed frame;
//! - `GET /console/messages/catalogue`, every message the platform can send, with examples;
//! - `GET /console/messages/catalogue/{id}/view`, an example email.
//!
//! An owner sees messages to their account and messages about their sites, nothing else.

use std::sync::OnceLock;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::{DateTime, FixedOffset, Utc};
use minijinja::Environment;
use serde::{Deserialize, Serialize};

use agentrix_host::OwnerSessions;

use crate::catalogue::{self, Channel};
use crate::outbox::{Entry, Filter, Outbox, Status};
use crate::send::{Messages, Readiness};
use std::sync::Arc;

/// What the routes need.
#[derive(Clone)]
pub struct WebState {
    /// The messages service.
    pub messages: Messages,
    /// Who is signed in.
    pub owners: Arc<dyn OwnerSessions>,
}

pub(crate) fn env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = agentrix_ui::environment();
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        for (name, src) in [
            (
                "messages/base.html",
                include_str!("../templates/_console/base.html"),
            ),
            (
                "messages/list.html",
                include_str!("../templates/_console/messages.html"),
            ),
            (
                "messages/message.html",
                include_str!("../templates/_console/message.html"),
            ),
            (
                "messages/catalogue.html",
                include_str!("../templates/_console/catalogue.html"),
            ),
            (
                "messages/app_list.html",
                include_str!("../templates/_console/app_list.html"),
            ),
            (
                "messages/app_message.html",
                include_str!("../templates/_console/app_message.html"),
            ),
            (
                "messages/app_catalogue.html",
                include_str!("../templates/_console/app_catalogue.html"),
            ),
            (
                "messages/app_notifications.html",
                include_str!("../templates/_console/app_notifications.html"),
            ),
        ] {
            env.add_template(name, src)
                .expect("console template parses");
        }
        env
    })
}

const CSS: &str = include_str!("../templates/_console/messages.css");

/// The routes, ready to merge into the platform host's router.
pub fn routes(state: WebState) -> Router {
    Router::new()
        .route("/console/messages", get(list))
        .route("/console/messages/assets/messages.css", get(css))
        .route("/console/messages/catalogue", get(catalogue_page))
        .route("/console/messages/catalogue/{id}/view", get(example_email))
        .route("/console/messages/{id}", get(detail))
        .route("/console/messages/{id}/view", get(stored_email))
        .with_state(state)
}

fn page(name: &str, ctx: impl Serialize) -> Response {
    match env().get_template(name).and_then(|t| t.render(ctx)) {
        Ok(html) => {
            let mut resp = Html(html).into_response();
            let h = resp.headers_mut();
            h.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(
                    "default-src 'self'; frame-src 'self'; frame-ancestors 'self'; base-uri 'none'; form-action 'self'",
                ),
            );
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(e) => {
            tracing::error!(error = %e, template = name, "messages page failed to render");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't show this page. Your messages are safe; reload to try again.",
            )
                .into_response()
        }
    }
}

/// An email shown inside the console: sandboxed, no scripts, no network but images.
fn email_response(html: String) -> Response {
    let mut resp = Html(html).into_response();
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src https: data:; frame-ancestors 'self'",
        ),
    );
    h.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

async fn css() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=300"),
        ],
        CSS,
    )
        .into_response()
}

struct Scope {
    account: String,
    sites: Vec<crate::directory::SiteRef>,
}

impl Scope {
    fn site_ids(&self) -> Vec<String> {
        self.sites.iter().map(|s| s.id.clone()).collect()
    }

    fn offset(&self) -> FixedOffset {
        let minutes = self
            .sites
            .first()
            .map(|s| s.utc_offset_minutes)
            .unwrap_or(0);
        FixedOffset::east_opt(minutes * 60)
            .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"))
    }

    fn names(&self) -> String {
        let names: Vec<&str> = self.sites.iter().map(|s| s.name.as_str()).collect();
        match names.as_slice() {
            [] => "your account".to_string(),
            [one] => (*one).to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    }
}

async fn scope(state: &WebState, headers: &HeaderMap, next: &str) -> Result<Scope, Response> {
    let owner = match state.owners.owner(headers).await {
        Ok(Some(o)) => o,
        Ok(None) => return Err(Redirect::to(&format!("/auth/signin?next={next}")).into_response()),
        Err(e) => {
            tracing::error!(error = %e, "owner session lookup failed");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "Sign-in is unavailable right now. Try again in a minute.",
            )
                .into_response());
        }
    };
    let account = owner.account.to_string();
    let sites = state
        .messages
        .directory()
        .sites_of(&account)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "sites lookup failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Your sites couldn't be read right now. Try again in a minute.",
            )
                .into_response()
        })?;
    Ok(Scope { account, sites })
}

/// "Sat 7 Nov, 06:58", or "06:58" today.
pub(crate) fn when(t: DateTime<Utc>, offset: FixedOffset, now: DateTime<Utc>) -> String {
    let local = t.with_timezone(&offset);
    let today = now.with_timezone(&offset).date_naive();
    let suffix = if offset.local_minus_utc() == 0 {
        " UTC"
    } else {
        ""
    };
    if local.date_naive() == today {
        format!("{}{suffix}", local.format("%H:%M"))
    } else {
        format!("{}{suffix}", local.format("%a %-d %b, %H:%M"))
    }
}

#[derive(Serialize)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) when: String,
    pub(crate) channel: &'static str,
    pub(crate) to: String,
    pub(crate) subject: String,
    pub(crate) reason: String,
    pub(crate) status: &'static str,
    pub(crate) status_tone: &'static str,
    pub(crate) status_reason: Option<String>,
    pub(crate) message_label: String,
}

fn status_tone(s: Status) -> &'static str {
    match s {
        Status::Sent => "ok",
        Status::Held | Status::Queued => "info",
        Status::NotSent | Status::Suppressed => "wait",
        Status::Failed => "risk",
    }
}

fn message_label(id: &str) -> String {
    catalogue::get(id)
        .map(|d| d.family.label().to_string())
        .unwrap_or_else(|| id.to_string())
}

pub(crate) fn row(e: &Entry, offset: FixedOffset, now: DateTime<Utc>) -> Row {
    Row {
        id: e.id.clone(),
        when: when(e.created_at, offset, now),
        channel: e.channel.label(),
        to: e.recipient_label.clone(),
        subject: e.subject.clone(),
        reason: e.reason.clone(),
        status: e.status.label(),
        status_tone: status_tone(e.status),
        status_reason: e.status_reason.clone(),
        message_label: message_label(&e.message),
    }
}

/// What sits behind Details on a message: the catalogue key and the provider's own ids.
pub(crate) fn details_for(e: &Entry) -> Vec<Fact> {
    let mut details = vec![Fact::new("Message", &e.message)];
    if let Some(p) = &e.provider {
        details.push(Fact::new("Provider", p));
    }
    if let Some(r) = &e.provider_ref {
        details.push(Fact::new("Provider's id", r));
    }
    if let Some(c) = e
        .consent
        .as_deref()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
    {
        let words = c.get("words").and_then(|w| w.as_str()).unwrap_or_default();
        let when = c
            .get("given_at")
            .and_then(|w| w.as_str())
            .unwrap_or_default();
        let source = c.get("source").and_then(|w| w.as_str()).unwrap_or_default();
        details.push(Fact::new(
            "They agreed to",
            &format!("\"{words}\" ({when}, {source})"),
        ));
    }
    details.push(Fact::new("Attempts", &e.attempts.to_string()));
    details
}

/// A row for the kit's `ui.kv`.
#[derive(Serialize)]
pub(crate) struct Fact {
    k: String,
    v: String,
}

impl Fact {
    pub(crate) fn new(k: &str, v: &str) -> Self {
        Self {
            k: k.to_string(),
            v: v.to_string(),
        }
    }
}

#[derive(Deserialize, Default)]
struct ListQuery {
    status: Option<String>,
    channel: Option<String>,
    before: Option<String>,
}

#[derive(Serialize)]
struct FilterChip {
    label: String,
    href: String,
    current: bool,
}

async fn list(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Response {
    let scope = match scope(&state, &headers, "/console/messages").await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let status = q.status.as_deref().and_then(Status::parse);
    let channel = q.channel.as_deref().and_then(Channel::parse);
    let before = q
        .before
        .as_deref()
        .and_then(|b| DateTime::parse_from_rfc3339(b).ok())
        .map(|t| t.with_timezone(&Utc));
    let filter = Filter {
        status,
        channel,
        before,
        limit: Some(50),
    };
    let outbox: &Outbox = state.messages.outbox();
    let site_ids = scope.site_ids();
    let (entries, counts) = match tokio::try_join!(
        outbox.list_for_owner(&scope.account, &site_ids, &filter),
        outbox.counts_for_owner(&scope.account, &site_ids)
    ) {
        Ok(x) => x,
        Err(e) => {
            tracing::error!(error = %e, "outbox read failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Messages couldn't be read right now. Try again in a minute.",
            )
                .into_response();
        }
    };
    let now = Utc::now();
    let offset = scope.offset();
    let rows: Vec<Row> = entries.iter().map(|e| row(e, offset, now)).collect();
    let older = (entries.len() == 50).then(|| {
        let last = entries.last().expect("50 rows").created_at.to_rfc3339();
        let mut href = format!("/console/messages?before={}", urlencode(&last));
        if let Some(s) = status {
            href.push_str(&format!("&status={}", s.as_str()));
        }
        if let Some(c) = channel {
            href.push_str(&format!("&channel={}", c.as_str()));
        }
        href
    });
    let chip = |label: &str, n: i64, s: Option<Status>| FilterChip {
        label: format!("{label} · {n}"),
        href: match s {
            Some(s) => format!("/console/messages?status={}", s.as_str()),
            None => "/console/messages".to_string(),
        },
        current: s == status,
    };
    let mut chips = vec![
        chip("All", counts.all, None),
        chip("Sent", counts.sent, Some(Status::Sent)),
        chip("Not sent", counts.not_sent, Some(Status::NotSent)),
    ];
    if counts.held > 0 || status == Some(Status::Held) {
        chips.push(chip(
            "Held for quiet hours",
            counts.held,
            Some(Status::Held),
        ));
    }
    if counts.failed > 0 || status == Some(Status::Failed) {
        chips.push(chip("Failed", counts.failed, Some(Status::Failed)));
    }
    let email_ready = state.messages.readiness(Channel::Email) == Readiness::Ready;
    page(
        "messages/list.html",
        minijinja::context! {
            title => "Messages",
            scope_names => scope.names(),
            rows => rows,
            chips => chips,
            filtered => status.is_some() || channel.is_some(),
            older => older,
            email_ready => email_ready,
        },
    )
}

pub(crate) fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn load_visible(
    state: &WebState,
    headers: &HeaderMap,
    id: &str,
) -> Result<(Scope, Entry), Response> {
    let scope = scope(
        state,
        headers,
        &format!("/console/messages/{}", urlencode(id)),
    )
    .await?;
    let entry = state.messages.outbox().get(id).await.map_err(|e| {
        tracing::error!(error = %e, "outbox read failed");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "This message couldn't be read right now. Try again in a minute.",
        )
            .into_response()
    })?;
    match entry {
        Some(e) if Outbox::visible_to(&e, &scope.account, &scope.site_ids()) => Ok((scope, e)),
        _ => Err((StatusCode::NOT_FOUND, Html(not_found())).into_response()),
    }
}

fn not_found() -> String {
    env()
        .get_template("messages/base.html")
        .and_then(|t| {
            t.render(minijinja::context! {
                title => "Not found",
                missing => true,
            })
        })
        .unwrap_or_else(|_| "No message with that id belongs to your sites.".into())
}

async fn detail(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let (scope, e) = match load_visible(&state, &headers, &id).await {
        Ok(x) => x,
        Err(r) => return r,
    };
    let now = Utc::now();
    let offset = scope.offset();
    let def = catalogue::get(&e.message);
    let replies: Vec<String> = e
        .payload
        .as_ref()
        .and_then(|p| p.get("replies"))
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let m = row(&e, offset, now);
    let mut facts: Vec<Fact> = vec![
        Fact::new("To", &m.to),
        Fact::new("Channel", m.channel),
        Fact::new("From", &e.from_name),
        Fact::new("Why", &e.reason),
    ];
    if let Some(d) = def {
        facts.push(Fact::new("Sent when", d.trigger));
        facts.push(Fact::new("Kind", d.kind.label()));
    }
    facts.push(Fact::new("Recorded", &when(e.created_at, offset, now)));
    if let Some(t) = e.hold_until {
        facts.push(Fact::new("Held until", &when(t, offset, now)));
    }
    if let Some(t) = e.sent_at {
        let by = e
            .provider
            .as_deref()
            .map(|p| format!(" by {p}"))
            .unwrap_or_default();
        facts.push(Fact::new("Sent", &format!("{}{by}", when(t, offset, now))));
    }
    let details = details_for(&e);
    page(
        "messages/message.html",
        minijinja::context! {
            title => e.subject.clone(),
            m => m,
            facts => facts,
            details => details,
            channel_key => e.channel.as_str(),
            from_name => e.from_name.clone(),
            body_text => e.body_text.clone(),
            has_html => e.body_html.is_some(),
            replies => replies,
        },
    )
}

async fn stored_email(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    match load_visible(&state, &headers, &id).await {
        Ok((_, e)) => match e.body_html {
            Some(html) => email_response(html),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Err(r) => r,
    }
}

#[derive(Serialize)]
pub(crate) struct CatalogueItem {
    id: &'static str,
    subject: String,
    audience: &'static str,
    kind: &'static str,
    channels: Vec<&'static str>,
    trigger: &'static str,
    push: Option<String>,
    chat: Option<String>,
    has_email: bool,
}

#[derive(Serialize)]
pub(crate) struct CatalogueGroup {
    label: &'static str,
    items: Vec<CatalogueItem>,
}

/// Every message, grouped by family, rendered from its example (stored copy, codes as dots).
pub(crate) fn catalogue_groups() -> Vec<CatalogueGroup> {
    let mut groups: Vec<CatalogueGroup> = Vec::new();
    for def in catalogue::all() {
        let rendered = match crate::render::render_stored(def.id, &def.example_context()) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "catalogue example failed");
                continue;
            }
        };
        let item = CatalogueItem {
            id: def.id,
            subject: rendered.summary(),
            audience: def.audience.label(),
            kind: def.kind.label(),
            channels: def.channels.iter().map(|c| c.label()).collect(),
            trigger: def.trigger,
            push: rendered.push.as_ref().map(|p| p.body.clone()),
            chat: rendered.chat.as_ref().map(|c| c.text.clone()),
            has_email: rendered.email.is_some(),
        };
        match groups.iter_mut().find(|g| g.label == def.family.label()) {
            Some(g) => g.items.push(item),
            None => groups.push(CatalogueGroup {
                label: def.family.label(),
                items: vec![item],
            }),
        }
    }
    groups
}

async fn catalogue_page(State(state): State<WebState>, headers: HeaderMap) -> Response {
    if let Err(r) = scope(&state, &headers, "/console/messages/catalogue").await {
        return r;
    }
    let groups = catalogue_groups();
    page(
        "messages/catalogue.html",
        minijinja::context! {
            title => "Every message",
            groups => groups,
            count => catalogue::all().len(),
        },
    )
}

async fn example_email(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = scope(&state, &headers, "/console/messages/catalogue").await {
        return r;
    }
    let Some(def) = catalogue::get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match crate::render::render_stored(def.id, &def.example_context()) {
        Ok(r) => match r.email {
            Some(e) => email_response(e.html),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{Contact, Fixed, SiteRef};
    use crate::send::{Outgoing, Recipient};
    use agentrix_host::testing::FixedOwner;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn app(signed_in: Option<uuid::Uuid>, owner_of_site: uuid::Uuid) -> (Router, Messages) {
        let directory = Arc::new(Fixed {
            sites: vec![SiteRef {
                id: "site_k".into(),
                name: "Kilele Bakery".into(),
                domain: "kilele-bakery-sample.agentrix.website".into(),
                owner_account: Some(owner_of_site.to_string()),
                utc_offset_minutes: 180,
            }],
            owners: vec![Contact {
                account: owner_of_site.to_string(),
                email: "wanjiru@example.com".into(),
                name: "Wanjiru".into(),
            }],
        });
        let messages = Messages::new(Outbox::new(crate::outbox::tests::pool().await), directory);
        let owners: Arc<dyn OwnerSessions> = Arc::new(match signed_in {
            Some(a) => FixedOwner::passkey(a),
            None => FixedOwner::nobody(),
        });
        let router = routes(WebState {
            messages: messages.clone(),
            owners,
        });
        (router, messages)
    }

    async fn get(router: &Router, path: &str) -> (StatusCode, String, HeaderMap) {
        let resp = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&body).into_owned(), headers)
    }

    async fn seed(messages: &Messages) -> String {
        let def = catalogue::get("customer.signin_code").unwrap();
        let sent = messages
            .send(Outgoing {
                message: def.id.into(),
                site_id: Some("site_k".into()),
                to: Recipient::email("amina@gmail.com", Some("Amina")),
                context: def.example_context(),
                reason: "Amina asked for a sign-in code".into(),
                dedupe_key: None,
            })
            .await
            .unwrap();
        sent.entries[0].id.clone()
    }

    #[tokio::test]
    async fn signed_out_people_go_to_sign_in() {
        let (router, _) = app(None, uuid::Uuid::now_v7()).await;
        let (status, _, headers) = get(&router, "/console/messages").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(
            headers[header::LOCATION],
            "/auth/signin?next=/console/messages"
        );
    }

    #[tokio::test]
    async fn the_owner_sees_the_message_without_its_code() {
        let owner = uuid::Uuid::now_v7();
        let (router, messages) = app(Some(owner), owner).await;
        let (status, body, _) = get(&router, "/console/messages").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Email isn"), "the not-switched-on line shows");
        assert!(body.contains("No messages yet"));
        let id = seed(&messages).await;
        let (_, body, _) = get(&router, "/console/messages").await;
        assert!(body.contains("Your sign-in code for Kilele Bakery"));
        assert!(body.contains("t switched on yet"));
        let (status, body, _) = get(&router, &format!("/console/messages/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        // The whole code, plain and spaced: "482" alone also turns up in random message ids.
        assert!(
            !body.contains("482913") && !body.contains("482 913"),
            "the code itself never shows"
        );
        let (status, html, headers) = get(&router, &format!("/console/messages/{id}/view")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            headers[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .starts_with("sandbox")
        );
        // The whole code, plain and spaced: "482" alone also turns up in random message ids.
        assert!(
            !html.contains("482913") && !html.contains("482 913"),
            "the code itself never shows"
        );
        let (status, body, _) = get(&router, "/console/messages/catalogue").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Every message Agentrix can send"));
    }

    #[tokio::test]
    async fn another_owner_cant_open_it() {
        let owner = uuid::Uuid::now_v7();
        let stranger = uuid::Uuid::now_v7();
        let (router, messages) = app(Some(stranger), owner).await;
        let id = seed(&messages).await;
        let (status, _, _) = get(&router, &format!("/console/messages/{id}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = get(&router, &format!("/console/messages/{id}/view")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, body, _) = get(&router, "/console/messages").await;
        assert!(!body.contains("Your sign-in code"));
    }
}
