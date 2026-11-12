//! `books.agentrix.website`: Zavora ERA, hosted. Slice A runs ERA; this host only forwards to
//! it when `SAAS_ERA_URL` names it, and otherwise answers with a stub page.

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Requests and responses larger than this are refused (ERA's own upload limit is lower).
const MAX_BODY: usize = 16 * 1024 * 1024;

/// Hop-by-hop headers a proxy must not forward (RFC 9110 §7.6.1), plus `host`.
const HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
];

#[derive(Clone)]
struct Era {
    base: String,
    http: reqwest::Client,
}

/// The books host: a stub, or a forwarder to `era_url`.
pub fn router(era_url: Option<String>) -> Router {
    match era_url {
        None => Router::new().fallback(stub),
        Some(base) => Router::new().fallback(forward).with_state(Era {
            base: base.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }),
    }
}

async fn stub(req: Request) -> Response {
    if req.uri().path() == "/" {
        let mut r = crate::pages::render_static("saas/books.html");
        *r.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
        return r;
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"era_not_connected","message":"Zavora ERA is not connected to this node yet."}"#,
    )
        .into_response()
}

async fn forward(State(era): State<Era>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Too large.").into_response();
    };
    let mut out = era
        .http
        .request(parts.method, format!("{}{path}", era.base));
    for (name, value) in &parts.headers {
        if !HOP.contains(&name.as_str()) {
            out = out.header(name, value);
        }
    }
    match out.body(body).send().await {
        Ok(res) => {
            let status = res.status();
            let headers = res.headers().clone();
            let bytes = res.bytes().await.unwrap_or_default();
            let mut r = Response::new(Body::from(bytes));
            *r.status_mut() = status;
            for (name, value) in &headers {
                if !HOP.contains(&name.as_str())
                    && name != HeaderName::from_static("content-length")
                {
                    r.headers_mut().append(name, value.clone());
                }
            }
            r
        }
        Err(e) => {
            tracing::warn!(error = %e, "books: ERA did not answer");
            (StatusCode::BAD_GATEWAY, "Zavora ERA did not answer.").into_response()
        }
    }
}
