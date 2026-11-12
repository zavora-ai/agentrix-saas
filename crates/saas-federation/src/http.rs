//! `/.well-known/openid-configuration` and `/.well-known/jwks.json` for each site, chosen by the
//! request's `Host`: what a cloud's federation fetches to trust a site's agents.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};

use crate::SiteIssuer;

type Sites = Arc<HashMap<String, Arc<SiteIssuer>>>;

/// The router, for each site's host (from configuration) and its issuer.
pub fn router(sites: HashMap<String, Arc<SiteIssuer>>) -> Router {
    let sites: HashMap<String, Arc<SiteIssuer>> = sites
        .into_iter()
        .map(|(h, i)| (h.to_ascii_lowercase(), i))
        .collect();
    Router::new()
        .route(saas_core::federation::DISCOVERY_PATH, get(discovery))
        .route(saas_core::federation::JWKS_PATH, get(jwks))
        .with_state(Arc::new(sites))
}

fn site<'a>(sites: &'a Sites, headers: &HeaderMap) -> Option<&'a Arc<SiteIssuer>> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    sites.get(&host)
}

fn cached(body: serde_json::Value) -> Response {
    ([(header::CACHE_CONTROL, "public, max-age=300")], Json(body)).into_response()
}

async fn discovery(State(sites): State<Sites>, headers: HeaderMap) -> Response {
    match site(&sites, &headers) {
        Some(i) => cached(i.discovery()),
        None => (StatusCode::NOT_FOUND, "no issuer for this host").into_response(),
    }
}

async fn jwks(State(sites): State<Sites>, headers: HeaderMap) -> Response {
    match site(&sites, &headers) {
        Some(i) => cached(i.jwks()),
        None => (StatusCode::NOT_FOUND, "no issuer for this host").into_response(),
    }
}
