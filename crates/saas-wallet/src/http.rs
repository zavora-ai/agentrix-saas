//! `GET /wallet/{site}.json`: a site's wallet record (the Cashier's view: the whole site), or with
//! `?agent=<role>` that agent's view, for the board when the face runs in another process
//! (`SAAS_WALLET_URL`). In one binary the face calls [`Wallets::feed`] directly.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use saas_core::AgentId;

use crate::Wallets;

/// The router, for the face's site ids (`agentrix`, `kilele`) mapped to each company's wallets.
pub fn router(sites: HashMap<String, Arc<Wallets>>) -> Router {
    Router::new()
        .route("/wallet/{file}", get(feed))
        .with_state(Arc::new(sites))
}

/// `?agent=<role>`: that agent's view of the site's record instead of the whole record.
#[derive(serde::Deserialize)]
struct View {
    agent: Option<String>,
}

async fn feed(
    State(sites): State<Arc<HashMap<String, Arc<Wallets>>>>,
    Path(file): Path<String>,
    Query(view): Query<View>,
) -> Response {
    let Some(site) = file.strip_suffix(".json") else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let Some(wallets) = sites.get(site) else {
        return (StatusCode::NOT_FOUND, format!("no wallet for site {site}")).into_response();
    };
    let role = view.agent.as_deref().unwrap_or(AgentId::CASHIER);
    let agent = match AgentId::new(wallets.company().clone(), role) {
        Ok(a) => a,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    match wallets.feed(&agent).await {
        Ok(feed) => Json(feed).into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    }
}
