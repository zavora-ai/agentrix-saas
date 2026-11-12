//! A recording relay in front of PayPal, for the attack suite: the node's PayPal base URL points
//! here, every request is forwarded to PayPal (the sandbox, or a stand-in in CI) unchanged, and
//! each exchange is kept with PayPal's own answer. The suite's PayPal checks read this record
//! and PayPal's API, never the node's own idea of what it sent.
//!
//! Nothing secret is kept: request headers (the Basic and Bearer credentials) are not recorded,
//! and the token endpoint's answer is replaced by `{"redacted": true}`.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

/// One request PayPal received from the node, and what PayPal answered.
#[derive(Debug, Clone, Serialize)]
pub struct Exchange {
    /// When PayPal answered.
    pub at: DateTime<Utc>,
    /// The method.
    pub method: String,
    /// The path.
    pub path: String,
    /// The query, if any.
    pub query: Option<String>,
    /// The request body (JSON, or the text as a string).
    pub request: Value,
    /// PayPal's status; 0 when PayPal couldn't be reached.
    pub status: u16,
    /// PayPal's answer.
    pub response: Value,
}

impl Exchange {
    /// Whether this was `method` on a path starting with `prefix` and ending with `suffix`.
    pub fn is(&self, method: &str, prefix: &str, suffix: &str) -> bool {
        self.method == method && self.path.starts_with(prefix) && self.path.ends_with(suffix)
    }

    /// A payout batch request.
    pub fn is_payout(&self) -> bool {
        self.is("POST", "/v1/payments/payouts", "") && self.path == "/v1/payments/payouts"
    }

    /// A refund of a capture.
    pub fn is_refund(&self) -> bool {
        self.is("POST", "/v2/payments/captures/", "/refund")
    }

    /// Anything that can move money: an order, a capture, a payout, a refund.
    pub fn moves_money(&self) -> bool {
        self.method == "POST"
            && (self.path.starts_with("/v2/checkout/orders")
                || self.path.starts_with("/v2/payments/")
                || self.path == "/v1/payments/payouts")
    }
}

type Hook = Box<dyn Fn(&Exchange) + Send + Sync>;

struct Inner {
    upstream: String,
    http: reqwest::Client,
    log: Mutex<Vec<Exchange>>,
    hook: Mutex<Option<Hook>>,
}

/// The relay.
#[derive(Clone)]
pub struct Relay {
    url: String,
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relay")
            .field("url", &self.url)
            .field("upstream", &self.inner.upstream)
            .finish_non_exhaustive()
    }
}

impl Relay {
    /// Serves a relay to `upstream` on a loopback port.
    pub async fn start(upstream: &str) -> anyhow::Result<Self> {
        let inner = Arc::new(Inner {
            upstream: upstream.trim_end_matches('/').to_owned(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?,
            log: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let app = Router::new().fallback(forward).with_state(inner.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Ok(Self { url, inner })
    }

    /// Where the node should send PayPal requests.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Where the relay forwards them.
    pub fn upstream(&self) -> &str {
        &self.inner.upstream
    }

    /// How many exchanges so far: a mark for [`Relay::since`].
    pub fn mark(&self) -> usize {
        self.inner
            .log
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// The exchanges after `mark`.
    pub fn since(&self, mark: usize) -> Vec<Exchange> {
        self.inner
            .log
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .skip(mark)
            .cloned()
            .collect()
    }

    /// Runs `hook` on each exchange once PayPal has answered and before the answer goes back to
    /// the node (the kill -9 attack kills the node there).
    pub fn on_answer(&self, hook: impl Fn(&Exchange) + Send + Sync + 'static) {
        *self.inner.hook.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(hook));
    }

    /// Stops running the hook.
    pub fn clear_hook(&self) {
        *self.inner.hook.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

fn body_value(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
}

async fn forward(
    State(inner): State<Arc<Inner>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    let query = uri.query().map(str::to_owned);
    let target = match &query {
        Some(q) => format!("{}{path}?{q}", inner.upstream),
        None => format!("{}{path}", inner.upstream),
    };
    let mut request = inner.http.request(method.clone(), &target);
    for (name, value) in &headers {
        if name == header::HOST || name == header::CONTENT_LENGTH || name == header::CONNECTION {
            continue;
        }
        request = request.header(name, value);
    }
    let (status, content_type, answer) = match request.body(body.clone()).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let bytes = response.bytes().await.unwrap_or_default();
            (status, content_type, bytes)
        }
        Err(e) => (
            0,
            Some("application/json".into()),
            Bytes::from(json!({ "relay_error": e.to_string() }).to_string()),
        ),
    };
    let secret = path == "/v1/oauth2/token" || path.starts_with("/v1/identity/");
    let exchange = Exchange {
        at: Utc::now(),
        method: method.to_string(),
        path,
        query,
        request: if secret {
            json!({ "redacted": true })
        } else {
            body_value(&body)
        },
        status,
        response: if secret {
            json!({ "redacted": true })
        } else {
            body_value(&answer)
        },
    };
    inner
        .log
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(exchange.clone());
    if let Some(hook) = inner
        .hook
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        hook(&exchange);
    }
    let mut response = Response::builder().status(if status == 0 {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY)
    });
    if let Some(ct) = content_type {
        response = response.header(header::CONTENT_TYPE, ct);
    }
    response
        .body(Body::from(answer))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}
