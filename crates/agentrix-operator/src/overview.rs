//! The ops host's front page (concept/control-plane/index.html): the three scopes side by side,
//! each with its real numbers from the reads the other pages already make.
//!
//! - **Platform** (operator scope): sites, Moves, open incidents and money this month.
//! - **Portfolios** (portfolio scope): each partner's clients, live sites, decisions waiting on
//!   owners, and AI spend.
//! - **Your lane**: what waits for the signed-in operator: second signatures they can give,
//!   support grants they asked for, break-glass they're part of, incidents they command.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;
use axum::routing::get;
use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::OperatorState;
use crate::pages::{internal, page, shell, signed_in};
use crate::{incidents, partners, reads};

pub(crate) fn routes() -> Router<OperatorState> {
    Router::new().route("/", get(overview))
}

/// Partners shown on the overview; the rest are on Partners.
const PARTNERS_SHOWN: usize = 12;

/// One partner's portfolio in a line.
#[derive(Debug, Serialize)]
struct PortfolioLine {
    partner_id: String,
    name: String,
    kind: String,
    clients: usize,
    live: i64,
    moving: i64,
    invited: i64,
    waiting: i64,
    spend_micro_usd: i64,
}

/// What the signed-in operator started or is part of.
#[derive(Debug, Default, Serialize)]
struct Mine {
    /// Support grants they asked for that wait for the owner.
    grants_waiting: i64,
    /// Support grants they asked for that are open now.
    grants_active: i64,
    /// Break-glass they started or signed, waiting or open.
    break_glass: i64,
    /// Open incidents they command: (id, severity, title, state).
    incidents: Vec<(String, String, String, String)>,
}

async fn mine(pool: &SqlitePool, account: &str) -> Result<Mine, sqlx::Error> {
    let (grants_waiting, grants_active): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(state = 'requested'), 0), COALESCE(SUM(state = 'active'), 0) \
         FROM support_grants WHERE requested_by = ?",
    )
    .bind(account)
    .fetch_one(pool)
    .await?;
    let (break_glass,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM break_glass WHERE state IN ('waiting', 'open') AND (first_by = ?1 OR second_by = ?1)",
    )
    .bind(account)
    .fetch_one(pool)
    .await?;
    let incidents = sqlx::query_as(
        "SELECT incident_id, severity, title, state FROM incidents WHERE commander = ? AND state <> 'resolved' ORDER BY opened_at DESC LIMIT 5",
    )
    .bind(account)
    .fetch_all(pool)
    .await?;
    Ok(Mine {
        grants_waiting,
        grants_active,
        break_glass,
        incidents,
    })
}

async fn overview(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let account = op.account.to_string();
    let shell = match shell(&s, op, "overview").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let now = Utc::now();
    let loaded = async {
        let fleet = reads::fleet(pool, now).await?;
        let open = incidents::open(pool).await?;
        let mine = mine(pool, &account).await?;
        let all = partners::all(pool).await?;
        let mut lines = Vec::new();
        for p in all.iter().take(PARTNERS_SHOWN).cloned() {
            let pf = partners::portfolio(pool, p, now).await?;
            lines.push(PortfolioLine {
                clients: pf.clients.iter().filter(|c| c.state == "granted").count(),
                partner_id: pf.partner.partner_id,
                name: pf.partner.name,
                kind: pf.partner.kind,
                live: pf.live,
                moving: pf.moving,
                invited: pf.invited,
                waiting: pf.waiting,
                spend_micro_usd: pf.spend_micro_usd,
            });
        }
        Ok::<_, sqlx::Error>((fleet, open, mine, all.len(), lines))
    }
    .await;
    let (fleet, open, mine, partners_total, lines) = match loaded {
        Ok(x) => x,
        Err(e) => return internal(e),
    };
    // Money: billing's own read, as the Money page shows it. A failure is said, not hidden.
    let money: Value = match agentrix_billing::Billing::new(pool.clone())
        .money(now)
        .await
    {
        Ok(m) => {
            json!({ "ok": true, "mrr_cents": m.mrr_cents, "collected_cents": m.collected_cents, "outstanding_cents": m.outstanding_cents, "outstanding_invoices": m.outstanding_invoices, "month": m.month })
        }
        Err(e) => {
            tracing::warn!(error = %e, "billing read for the overview");
            json!({ "ok": false })
        }
    };
    let worst = open
        .iter()
        .map(|i| i.severity.as_str())
        .min()
        .map(str::to_uppercase);
    page(
        &s,
        "overview.html",
        &json!({
            "shell": shell,
            "f": fleet,
            "incidents_open": open.len(),
            "incidents_worst": worst,
            "money": money,
            "portfolios": lines,
            "partners_total": partners_total,
            "mine": mine,
        }),
    )
}
