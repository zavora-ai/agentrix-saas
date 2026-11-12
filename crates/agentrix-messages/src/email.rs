//! Email delivery through Resend's HTTP API (`POST https://api.resend.com/emails`).
//!
//! Switched on by node's environment, read once at startup and never logged:
//! - `RESEND_API_KEY`: the send-only key for the verified domain agentrix.website (James set up
//!   DKIM, the send and rsend records and DMARC on Vultr DNS). Optional beside it:
//!   `AGENTRIX_EMAIL_FROM` (default `hello@agentrix.website`, shown as "Agentrix" for owner and
//!   prospect messages and as the site's name for its customers), `AGENTRIX_EMAIL_REPLY_TO` (an
//!   email address; a web page can't be a Reply-To) and `RESEND_WEBHOOK_SECRET`.
//! - or `AGENTRIX_EMAIL_FILE`, naming a JSON file only node's user can read:
//!
//! ```json
//! {"provider": "resend", "api_key": "re_…", "from": "hello@agentrix.website",
//!  "reply_to": null, "webhook_secret": "whsec_…"}
//! ```
//!
//! Neither set: the platform keeps [`crate::Undelivered`], and every message stays in the outbox
//! as "Not sent". Vultr blocks outbound port 25, so delivery is over HTTPS. Something people
//! signed up for (the weekly fix, the digest) carries List-Unsubscribe and
//! List-Unsubscribe-Post: One-Click.
//!
//! Bounces and complaints come back to `POST /messages/email/webhook` on the platform host,
//! signed by Resend's webhook service (Svix); unsigned or stale calls are refused.
//!
//! Horizon 2 moves this driver into W6's Effects Gateway process, the only holder of provider
//! keys (ADR-004); the [`Transport`] port stays the same.

use std::path::Path;

use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use sha2::Sha256;

use crate::catalogue::Channel;
use crate::outbox::{Outbox, Status};
use crate::send::{Delivery, Envelope, Readiness, Transport, Undelivered};

/// What the credential file holds.
#[derive(Clone, Deserialize)]
pub struct EmailConfig {
    /// Only `resend` for now.
    pub provider: String,
    /// The sending-only API key.
    pub api_key: String,
    /// The From address on the verified sending domain.
    #[serde(default = "default_from")]
    pub from: String,
    /// Where replies go, if anywhere: an email address (a web page can't be a Reply-To).
    #[serde(default)]
    pub reply_to: Option<String>,
    /// The webhook signing secret (`whsec_…`), for bounces and complaints.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// The API's base URL (tests point it at a local server).
    #[serde(default)]
    pub api_base: Option<String>,
}

/// The From address unless configured: `hello@agentrix.website` (Resend's verified domain).
pub const DEFAULT_FROM: &str = "hello@agentrix.website";

fn default_from() -> String {
    DEFAULT_FROM.to_string()
}

impl std::fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailConfig")
            .field("provider", &self.provider)
            .field("from", &self.from)
            .field("reply_to", &self.reply_to)
            .field("api_key", &"…")
            .finish()
    }
}

impl EmailConfig {
    /// Reads the file `AGENTRIX_EMAIL_FILE` names, if any.
    ///
    /// Two ways to switch email on, checked in this order:
    /// - `AGENTRIX_EMAIL_FILE`: a JSON credential file (see the module docs);
    /// - `RESEND_API_KEY`: the send-only key itself, with `AGENTRIX_EMAIL_FROM` (default
    ///   [`DEFAULT_FROM`]), `AGENTRIX_EMAIL_REPLY_TO` and `RESEND_WEBHOOK_SECRET` optional.
    ///
    /// Neither set: `None`, and every message stays "Not sent".
    pub fn from_env() -> Option<Self> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// [`Self::from_env`] over any source of variables (tests pass a map).
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let set = |k: &str| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        if let Some(path) = set("AGENTRIX_EMAIL_FILE") {
            return Self::from_file(Path::new(&path));
        }
        let api_key = set("RESEND_API_KEY")?;
        let from = set("AGENTRIX_EMAIL_FROM").unwrap_or_else(default_from);
        if !from.contains('@') {
            tracing::warn!("AGENTRIX_EMAIL_FROM isn't an address; email stays off");
            return None;
        }
        let reply_to = set("AGENTRIX_EMAIL_REPLY_TO");
        if reply_to.as_deref().is_some_and(|r| !r.contains('@')) {
            tracing::warn!("AGENTRIX_EMAIL_REPLY_TO must be an email address; sending without one");
        }
        Some(Self {
            provider: "resend".into(),
            api_key,
            from,
            reply_to: reply_to.filter(|r| r.contains('@')),
            webhook_secret: set("RESEND_WEBHOOK_SECRET"),
            api_base: None,
        })
    }

    /// Reads one file. Logs why it couldn't (never its contents) and returns `None`.
    pub fn from_file(path: &Path) -> Option<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "email credentials unreadable; email stays off");
                return None;
            }
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(cfg)
                if cfg.provider == "resend"
                    && !cfg.api_key.trim().is_empty()
                    && cfg.from.contains('@') =>
            {
                Some(cfg)
            }
            Ok(_) => {
                tracing::warn!(
                    "email credentials incomplete (provider resend, api_key, from); email stays off"
                );
                None
            }
            Err(_) => {
                tracing::warn!("email credentials aren't valid JSON; email stays off");
                None
            }
        }
    }
}

/// Delivers email through Resend; push and chat stay as [`Undelivered`] says.
pub struct ResendTransport {
    http: reqwest::Client,
    cfg: EmailConfig,
}

impl ResendTransport {
    /// With node's shared HTTP client.
    pub fn new(http: reqwest::Client, cfg: EmailConfig) -> Self {
        Self { http, cfg }
    }

    fn endpoint(&self) -> String {
        format!(
            "{}/emails",
            self.cfg
                .api_base
                .as_deref()
                .unwrap_or("https://api.resend.com")
                .trim_end_matches('/')
        )
    }
}

/// Resend's `Idempotency-Key`: the outbox row, and the attempt after the first, so a second try
/// (the same thing asked for again) isn't answered from the first one's record.
fn idempotency_key(envelope: &Envelope) -> String {
    match envelope.attempt {
        0 => envelope.outbox_id.clone(),
        n => format!("{}-{n}", envelope.outbox_id),
    }
}

/// `"Kilele Bakery (sample)" <messages@…>`, with the name cleaned for a header.
fn from_header(name: &str, address: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '<' && *c != '>' && *c != '\\')
        .collect();
    format!("\"{}\" <{address}>", clean.trim())
}

#[async_trait]
impl Transport for ResendTransport {
    fn readiness(&self, channel: Channel) -> Readiness {
        match channel {
            Channel::Email => Readiness::Ready,
            other => Undelivered.readiness(other),
        }
    }

    async fn deliver(&self, envelope: &Envelope) -> Delivery {
        if envelope.channel != Channel::Email {
            return Undelivered.deliver(envelope).await;
        }
        let headers: serde_json::Map<String, Json> = envelope
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), Json::String(v.clone())))
            .collect();
        let mut body = json!({
            "from": from_header(&envelope.from_name, &self.cfg.from),
            "to": [envelope.to],
            "subject": envelope.subject,
            "text": envelope.text,
            "html": envelope.html,
            "headers": headers,
            "tags": [
                { "name": "message", "value": envelope.message.replace('.', "_") },
            ],
        });
        if let Some(reply_to) = &self.cfg.reply_to {
            body["reply_to"] = json!(reply_to);
        }
        let resp = self
            .http
            .post(self.endpoint())
            .bearer_auth(&self.cfg.api_key)
            .header("Idempotency-Key", idempotency_key(envelope))
            .json(&body)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, outbox = %envelope.outbox_id, "email provider unreachable");
                return Delivery::Failed {
                    reason: "The email provider didn't answer. The message is kept here.".into(),
                };
            }
        };
        let status = resp.status();
        let json: Json = resp.json().await.unwrap_or(Json::Null);
        if status.is_success() {
            match json.get("id").and_then(Json::as_str) {
                Some(id) => Delivery::Sent {
                    provider: "Resend".into(),
                    provider_ref: id.to_string(),
                },
                None => Delivery::Failed {
                    reason:
                        "The email provider took it but sent back no id, so we can't tell whether it was delivered."
                            .into(),
                },
            }
        } else {
            let why = json
                .get("message")
                .and_then(Json::as_str)
                .map(|m| m.chars().take(200).collect::<String>())
                .unwrap_or_else(|| status.to_string());
            tracing::warn!(status = status.as_u16(), provider_said = %why, outbox = %envelope.outbox_id, "email provider refused a message");
            Delivery::Failed {
                reason: "The email provider refused it, so it wasn't delivered. Asking for it again tries once more."
                    .into(),
            }
        }
    }
}

type HmacSha256 = Hmac<Sha256>;

/// Checks a Svix signature: `v1,<base64>` entries over `"{id}.{timestamp}.{body}"`, signed with
/// the base64 secret after `whsec_`, within five minutes of `now`.
pub fn verify_svix(secret: &str, headers: &HeaderMap, body: &[u8], now: i64) -> bool {
    let get = |k: &str| headers.get(k).and_then(|v| v.to_str().ok());
    let (Some(id), Some(ts), Some(sigs)) =
        (get("svix-id"), get("svix-timestamp"), get("svix-signature"))
    else {
        return false;
    };
    let Ok(ts_num) = ts.parse::<i64>() else {
        return false;
    };
    if (now - ts_num).abs() > 300 {
        return false;
    }
    let Ok(key) = B64.decode(secret.trim_start_matches("whsec_")) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(&key) else {
        return false;
    };
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(ts.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = mac.finalize().into_bytes();
    sigs.split_whitespace().any(|entry| {
        entry
            .strip_prefix("v1,")
            .and_then(|b| B64.decode(b).ok())
            .is_some_and(|sig| {
                subtle::ConstantTimeEq::ct_eq(sig.as_slice(), expected.as_slice()).into()
            })
    })
}

#[derive(Clone)]
struct HookState {
    outbox: Outbox,
    secret: String,
}

/// `POST /messages/email/webhook`: bounces and complaints, signed by the provider.
pub fn webhook_routes(outbox: Outbox, secret: String) -> Router {
    Router::new()
        .route("/messages/email/webhook", post(webhook))
        .with_state(HookState { outbox, secret })
}

async fn webhook(State(state): State<HookState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    if !verify_svix(&state.secret, &headers, &body, Utc::now().timestamp()) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(event) = serde_json::from_slice::<Json>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    let kind = event.get("type").and_then(Json::as_str).unwrap_or("");
    let Some(id) = event.pointer("/data/email_id").and_then(Json::as_str) else {
        return StatusCode::OK;
    };
    let update = match kind {
        "email.bounced" => Some((Status::Failed, "Bounced: the address doesn't take mail")),
        "email.complained" => Some((
            Status::Sent,
            "Delivered, then the recipient marked it as spam",
        )),
        _ => None,
    };
    if let Some((status, reason)) = update
        && let Err(e) = state
            .outbox
            .mark_by_provider_ref(id, status, reason, Utc::now())
            .await
    {
        tracing::error!(error = %e, "couldn't record a bounce");
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue;
    use crate::directory::Fixed;
    use crate::send::{Messages, Outgoing, Recipient};
    use axum::http::HeaderValue;
    use std::sync::{Arc, Mutex};

    #[test]
    fn signatures_are_checked() {
        let secret = format!("whsec_{}", B64.encode(b"0123456789abcdef0123456789abcdef"));
        let body = br#"{"type":"email.bounced"}"#;
        let ts = "1762580000";
        let mut mac = HmacSha256::new_from_slice(b"0123456789abcdef0123456789abcdef").unwrap();
        mac.update(format!("msg_1.{ts}.").as_bytes());
        mac.update(body);
        let sig = B64.encode(mac.finalize().into_bytes());
        let mut h = HeaderMap::new();
        h.insert("svix-id", HeaderValue::from_static("msg_1"));
        h.insert("svix-timestamp", HeaderValue::from_static(ts));
        h.insert(
            "svix-signature",
            HeaderValue::from_str(&format!("v1,bogus v1,{sig}")).unwrap(),
        );
        assert!(verify_svix(&secret, &h, body, 1_762_580_010));
        assert!(!verify_svix(&secret, &h, body, 1_762_590_000), "stale");
        assert!(
            !verify_svix(&secret, &h, b"{}", 1_762_580_010),
            "changed body"
        );
    }

    #[test]
    fn names_cant_break_the_from_header() {
        assert_eq!(
            from_header("Kilele \"Bakery\" <x>\n", "messages@mail.agentrix.website"),
            "\"Kilele Bakery x\" <messages@mail.agentrix.website>"
        );
    }

    #[test]
    fn the_key_never_prints() {
        let cfg = EmailConfig {
            provider: "resend".into(),
            api_key: "re_secret".into(),
            from: "messages@mail.agentrix.website".into(),
            reply_to: None,
            webhook_secret: None,
            api_base: None,
        };
        assert!(!format!("{cfg:?}").contains("re_secret"));
    }

    #[tokio::test]
    async fn a_message_goes_to_the_provider_and_its_bounce_comes_back() {
        // A stand-in for Resend's API on a local port.
        type Seen = Arc<Mutex<Vec<(Option<String>, Json)>>>;
        let seen: Seen = Arc::default();
        let seen2 = seen.clone();
        let api = Router::new().route(
            "/emails",
            post(move |headers: HeaderMap, body: axum::Json<Json>| {
                let seen = seen2.clone();
                async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    seen.lock().unwrap().push((auth, body.0));
                    axum::Json(json!({ "id": "em_123" }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });

        let pool = crate::outbox::tests::pool().await;
        let outbox = Outbox::new(pool);
        let transport = ResendTransport::new(
            reqwest::Client::new(),
            EmailConfig {
                provider: "resend".into(),
                api_key: "re_test".into(),
                from: DEFAULT_FROM.into(),
                reply_to: Some("support@zavora.example".into()),
                webhook_secret: None,
                api_base: Some(format!("http://{addr}")),
            },
        );
        let messages = Messages::new(outbox.clone(), Arc::new(Fixed::default()))
            .with_transport(Arc::new(transport));
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
        let e = &sent.entries[0];
        assert_eq!(e.status, Status::Sent);
        assert_eq!(e.provider_ref.as_deref(), Some("em_123"));
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen[0].0.as_deref(), Some("Bearer re_test"));
            assert_eq!(
                seen[0].1["from"],
                "\"Kilele Bakery (sample)\" <hello@agentrix.website>"
            );
            assert_eq!(seen[0].1["reply_to"], "support@zavora.example");
            assert!(
                seen[0].1["text"].as_str().unwrap().contains("482 913"),
                "the live copy carries the code"
            );
        }
        assert!(!e.body_text.contains("482 913"), "the stored copy doesn't");

        // The bounce, signed.
        let key = b"0123456789abcdef0123456789abcdef";
        let secret = format!("whsec_{}", B64.encode(key));
        let hooks = webhook_routes(outbox.clone(), secret);
        let body = br#"{"type":"email.bounced","data":{"email_id":"em_123"}}"#.to_vec();
        let ts = Utc::now().timestamp().to_string();
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(format!("msg_9.{ts}.").as_bytes());
        mac.update(&body);
        let sig = B64.encode(mac.finalize().into_bytes());
        let req = axum::http::Request::post("/messages/email/webhook")
            .header("svix-id", "msg_9")
            .header("svix-timestamp", ts)
            .header("svix-signature", format!("v1,{sig}"))
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(hooks, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let after = outbox.get(&e.id).await.unwrap().unwrap();
        assert_eq!(after.status, Status::Failed);
        assert_eq!(
            after.status_reason.as_deref(),
            Some("Bounced: the address doesn't take mail")
        );
    }

    #[test]
    fn the_key_comes_from_the_environment_or_a_file_and_nothing_means_off() {
        use std::collections::HashMap;
        let vars = |pairs: &[(&str, &str)]| {
            let m: HashMap<String, String> = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            move |k: &str| m.get(k).cloned()
        };
        assert!(
            EmailConfig::from_vars(vars(&[])).is_none(),
            "no key: email stays off"
        );
        assert!(EmailConfig::from_vars(vars(&[("RESEND_API_KEY", "  ")])).is_none());
        let cfg = EmailConfig::from_vars(vars(&[("RESEND_API_KEY", "re_x")])).unwrap();
        assert_eq!(cfg.from, "hello@agentrix.website");
        assert_eq!(cfg.reply_to, None);
        let cfg = EmailConfig::from_vars(vars(&[
            ("RESEND_API_KEY", "re_x"),
            (
                "AGENTRIX_EMAIL_REPLY_TO",
                "https://agentrix.website/contact",
            ),
        ]))
        .unwrap();
        assert_eq!(cfg.reply_to, None, "a page can't be a Reply-To");
        assert!(!format!("{cfg:?}").contains("re_x"));
    }

    #[tokio::test]
    async fn the_weekly_fix_reaches_resend_with_one_click_unsubscribe() {
        let seen: Arc<Mutex<Vec<Json>>> = Arc::default();
        let seen2 = seen.clone();
        let api = Router::new().route(
            "/emails",
            post(move |body: axum::Json<Json>| {
                let seen = seen2.clone();
                async move {
                    seen.lock().unwrap().push(body.0);
                    axum::Json(json!({ "id": "em_9" }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });
        let transport = ResendTransport::new(
            reqwest::Client::new(),
            EmailConfig {
                provider: "resend".into(),
                api_key: "re_test".into(),
                from: DEFAULT_FROM.into(),
                reply_to: None,
                webhook_secret: None,
                api_base: Some(format!("http://{addr}")),
            },
        );
        let messages = Messages::new(
            Outbox::new(crate::outbox::tests::pool().await),
            Arc::new(Fixed::default()),
        )
        .with_transport(Arc::new(transport));
        let consent = crate::prospects::Consent {
            words: "Send me the weekly fix".into(),
            given_at: "Sun 8 Nov, 10:20".into(),
            source: "agentrix.website footer".into(),
        };
        let issue = crate::prospects::Issue {
            title: "Put your opening hours where Google can read them".into(),
            lines: vec!["Write your hours as text.".into()],
            link: None,
        };
        let out = crate::prospects::newsletter_issue(
            "reader@gmail.com",
            &issue,
            "2026-46",
            "https://agentrix.website/fix/stop?token=t",
            &consent,
        )
        .unwrap();
        let sent = messages.send(out).await.unwrap();
        assert_eq!(sent.entries[0].status, Status::Sent);
        let body = &seen.lock().unwrap()[0];
        assert_eq!(body["from"], "\"Agentrix\" <hello@agentrix.website>");
        assert_eq!(
            body["headers"]["List-Unsubscribe"],
            "<https://agentrix.website/fix/stop?token=t>"
        );
        assert_eq!(
            body["headers"]["List-Unsubscribe-Post"],
            "List-Unsubscribe=One-Click"
        );
        assert!(body.get("reply_to").is_none());
    }
}
