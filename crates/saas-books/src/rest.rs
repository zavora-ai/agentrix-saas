//! ERA's REST API (`/api/v1/...`) as one company: a login, a bearer token kept under ERA's
//! 15-minute lifetime, one re-login on a 401, and ERA's errors mapped onto [`BooksError`].

use std::time::{Duration, Instant};

use reqwest::{Method, StatusCode};
use saas_core::BooksError;
use serde_json::{Value, json};
use tokio::sync::Mutex;

/// Log in again this long after the last login: inside ERA's 15-minute access tokens.
const TOKEN_MAX_AGE: Duration = Duration::from_secs(12 * 60);

/// Pages read from ERA's list endpoints (its maximum).
const PAGE: usize = 500;

#[derive(Clone)]
pub(crate) struct Session {
    pub token: String,
    pub entity_id: String,
    issued: Instant,
}

pub(crate) struct Rest {
    http: reqwest::Client,
    base: String,
    email: String,
    password: String,
    session: Mutex<Option<Session>>,
}

impl Rest {
    pub fn new(base_url: &str, email: String, password: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            http,
            base: format!("{}/api/v1", base_url.trim_end_matches('/')),
            email,
            password,
            session: Mutex::new(None),
        }
    }

    pub async fn login(&self) -> Result<Session, BooksError> {
        let resp = self
            .http
            .post(format!("{}/auth/login", self.base))
            .json(&json!({"email": self.email, "password": self.password}))
            .send()
            .await
            .map_err(unreachable)?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(map_status(status, "auth/login", &body));
        }
        let token = body["access_token"]
            .as_str()
            .ok_or_else(|| BooksError::Backend("login returned no token".into()))?;
        let session = Session {
            token: token.to_owned(),
            entity_id: body["user"]["entity_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            issued: Instant::now(),
        };
        *self.session.lock().await = Some(session.clone());
        Ok(session)
    }

    pub async fn session(&self) -> Result<Session, BooksError> {
        if let Some(s) = self.session.lock().await.as_ref() {
            if s.issued.elapsed() < TOKEN_MAX_AGE {
                return Ok(s.clone());
            }
        }
        self.login().await
    }

    /// One call; a 401 logs in again and retries once.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, BooksError> {
        let mut session = self.session().await?;
        for attempt in 0..2 {
            let mut req = self
                .http
                .request(method.clone(), format!("{}/{path}", self.base))
                .bearer_auth(&session.token);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await.map_err(unreachable)?;
            let status = resp.status();
            let text = resp.text().await.map_err(unreachable)?;
            let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                session = self.login().await?;
                continue;
            }
            if !status.is_success() {
                return Err(map_status(status, path, &value));
            }
            return Ok(value);
        }
        Err(BooksError::Backend(format!(
            "ERA refused the login for {path}"
        )))
    }

    /// One call as someone else (ERA's customer portal), or as nobody: no session, no retry.
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        body: &Value,
        bearer: Option<&str>,
    ) -> Result<Value, BooksError> {
        let mut req = self
            .http
            .request(method, format!("{}/{path}", self.base))
            .json(body);
        if let Some(t) = bearer {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(unreachable)?;
        let status = resp.status();
        let text = resp.text().await.map_err(unreachable)?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if status.is_success() {
            Ok(value)
        } else {
            Err(map_status(status, path, &value))
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value, BooksError> {
        self.send(Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, BooksError> {
        self.send(Method::POST, path, Some(body)).await
    }

    pub async fn put(&self, path: &str, body: &Value) -> Result<Value, BooksError> {
        self.send(Method::PUT, path, Some(body)).await
    }

    /// Every row of a paged list (`{data, has_more}`) or a plain array.
    pub async fn get_all(&self, path: &str) -> Result<Vec<Value>, BooksError> {
        let mut out = Vec::new();
        let sep = if path.contains('?') { '&' } else { '?' };
        loop {
            let page = self
                .get(&format!("{path}{sep}limit={PAGE}&offset={}", out.len()))
                .await?;
            match page {
                Value::Array(rows) => return Ok(rows),
                Value::Object(mut o) => {
                    let rows = match o.remove("data") {
                        Some(Value::Array(rows)) => rows,
                        _ => Vec::new(),
                    };
                    let more = o.get("has_more").and_then(Value::as_bool).unwrap_or(false);
                    let n = rows.len();
                    out.extend(rows);
                    if !more || n == 0 {
                        return Ok(out);
                    }
                }
                _ => return Ok(out),
            }
        }
    }
}

fn unreachable(e: reqwest::Error) -> BooksError {
    let mut msg = format!("ERA: {e}");
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        msg.push_str(&format!(": {s}"));
        source = s.source();
    }
    BooksError::Unavailable(msg)
}

/// ERA's error text: `{"error": "..."}`, or the plain text axum answers before a handler runs.
pub(crate) fn error_text(body: &Value) -> String {
    match body {
        Value::Object(o) => o
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| body.to_string()),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// ERA's statuses onto the books' errors: 404 not found, 400 and 422 invalid, 409 conflict
/// (closed period, duplicate, over the credit limit), 429 and 5xx-from-a-proxy unavailable.
pub(crate) fn map_status(status: StatusCode, path: &str, body: &Value) -> BooksError {
    let msg = format!("{path}: {}", error_text(body));
    match status.as_u16() {
        404 => BooksError::NotFound {
            what: "record",
            id: msg,
        },
        400 | 422 => BooksError::Invalid(msg),
        409 => BooksError::Conflict(msg),
        429 | 502 | 503 | 504 => BooksError::Unavailable(msg),
        _ => BooksError::Backend(format!("{status} {msg}")),
    }
}
