//! The operator pages: Fleet, Customers, a site, Trust, and the apps not built yet, each in the
//! scoped shell of `concept/control-plane/` (cp.js's status bar, dock, lane and command bar,
//! rendered on the server).

use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use chrono::{FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::identity::Operator;
use crate::{OperatorState, ledger, reads};

pub(crate) fn routes() -> Router<OperatorState> {
    Router::new()
        .route("/fleet", get(fleet))
        .route("/customers", get(customers))
        .route("/sites/{site_id}", get(site))
        .route("/trust", get(trust))
        .route("/cli", get(cli))
        .route("/rollouts", get(rollouts))
        .route("/money", get(money))
        .route("/holds", get(holds))
}

/// A fault on our side: logged with its detail, shown in plain words.
pub(crate) fn internal(error: impl std::fmt::Display) -> Response {
    tracing::error!(error = %error, "operator console");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html("<!doctype html><meta charset=utf-8><title>Control plane</title><p style=\"font-family:system-ui;padding:24px\">Something failed on our side. The error is in the node's log. Reload to see what was saved, then try again.</p>".to_owned()),
    )
        .into_response()
}

/// The signed-in operator, or a redirect to sign-in that comes back here.
pub(crate) async fn signed_in(
    s: &OperatorState,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Operator, Response> {
    match s.operator_from(headers).await {
        Ok(Some(op)) => Ok(op),
        Ok(None) => {
            let next = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
            let encoded: String = url::form_urlencoded::byte_serialize(next.as_bytes()).collect();
            Err(Redirect::to(&format!("/signin?next={encoded}")).into_response())
        }
        Err(e) => Err(internal(e)),
    }
}

/// The shell every page shares: who is signed in, the scope, the lane's count.
#[derive(Debug, Serialize)]
pub(crate) struct Shell {
    scope: &'static str,
    app: &'static str,
    operator: Operator,
    initials: String,
    first_name: String,
    roles: Vec<&'static str>,
    sites_live: i64,
    cells: usize,
    cell_id: String,
    now_words: String,
    zone: String,
    ledger_ok: bool,
    ledger_entries: i64,
    waiting: i64,
    lane: Vec<Value>,
    open_incidents: i64,
    platform_origin: Option<String>,
}

pub(crate) async fn shell(
    s: &OperatorState,
    op: Operator,
    app: &'static str,
) -> Result<Shell, Response> {
    let pool = &s.inner.pool;
    let (sites_live,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM site_pointer")
        .fetch_one(pool)
        .await
        .map_err(internal)?;
    let verified = ledger::verify(pool).await.map_err(internal)?;
    let lane = crate::access::lane(s, &op).await.map_err(internal)?;
    let (open_incidents,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM incidents WHERE state <> 'resolved'")
            .fetch_one(pool)
            .await
            .map_err(internal)?;
    let tz = FixedOffset::east_opt(s.inner.config.utc_offset_minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"));
    Ok(Shell {
        scope: "op",
        app,
        initials: op.initials(),
        first_name: op.first_name().to_owned(),
        roles: op.roles.iter().map(|r| r.as_str()).collect(),
        operator: op,
        sites_live,
        cells: 1,
        cell_id: s.inner.config.cell.id.clone(),
        now_words: Utc::now()
            .with_timezone(&tz)
            .format("%A %-d %b · %H:%M")
            .to_string(),
        zone: s.inner.config.zone_name.clone(),
        ledger_ok: verified.ok,
        ledger_entries: verified.entries,
        waiting: lane.len() as i64,
        lane,
        open_incidents,
        platform_origin: s
            .inner
            .config
            .platform_origin
            .as_ref()
            .map(|u| u.as_str().trim_end_matches('/').to_owned()),
    })
}

pub(crate) fn page(s: &OperatorState, template: &str, view: &Value) -> Response {
    // The line a form left for this page (POST, redirect, GET), shown once.
    let mut view = view.clone();
    if let Some(fields) = view.as_object_mut() {
        let notice = crate::notice::current(s);
        let kept = notice.as_ref().map(|n| n.kept.clone()).unwrap_or_default();
        fields.insert("notice".into(), json!(notice));
        fields.insert("kept".into(), json!(kept));
    }
    match s.inner.render.page(template, &view) {
        Ok(html) => (
            [
                ("content-security-policy", "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"),
                ("referrer-policy", "same-origin"),
                ("cache-control", "no-store"),
            ],
            Html(html),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

// ---------- The Model Gateway's status ----------

/// What the gateway reports at `GET /v1/status`: the provider, each purpose's route and the
/// daily cap per site. Shown as it is; nothing here can change a route.
#[derive(Debug, Clone, Serialize, Default)]
struct Gateway {
    reachable: bool,
    provider: String,
    mode: String,
    daily_cap_micro_usd: Option<i64>,
    routes: Vec<Route>,
    providers: Vec<(String, bool)>,
}

#[derive(Debug, Clone, Serialize)]
struct Route {
    purpose: String,
    provider: String,
    model: String,
    tier: Option<String>,
}

async fn gateway(s: &OperatorState) -> Gateway {
    let Some(url) = &s.inner.models_url else {
        return Gateway::default();
    };
    let url = format!("{}/v1/status", url.trim_end_matches('/'));
    let Ok(response) = s
        .inner
        .http
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
    else {
        return Gateway::default();
    };
    let Ok(status) = response.json::<Value>().await else {
        return Gateway::default();
    };
    let text = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let mut routes: Vec<Route> = status["routes"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(purpose, r)| Route {
                    purpose: purpose.clone(),
                    provider: text(&r["provider"]),
                    model: text(&r["model"]),
                    tier: r["tier"].as_str().map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    routes.sort_by(|a, b| a.purpose.cmp(&b.purpose));
    let providers = status["providers"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(name, p)| (name.clone(), p["ok"].as_bool().unwrap_or(false)))
                .collect()
        })
        .unwrap_or_default();
    Gateway {
        reachable: true,
        provider: text(&status["provider"]),
        mode: text(&status["mode"]),
        daily_cap_micro_usd: status["daily_cap_usd"]
            .as_f64()
            .map(|d| (d * 1_000_000.0).round() as i64),
        routes,
        providers,
    }
}

// ---------- Pages ----------

/// Brings in PayPal's side of each plan subscription (W6's table) so plans on screen are current.
/// A failure is logged; the page still shows what billing already knows.
async fn sync_billing(s: &OperatorState) {
    if let Err(e) = agentrix_billing::Billing::new(s.inner.pool.clone())
        .sync()
        .await
    {
        tracing::warn!(error = %e, "billing sync before a page");
    }
}

async fn fleet(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let shell = match shell(&s, op, "fleet").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let now = Utc::now();
    let fleet = match reads::fleet(&s.inner.pool, now).await {
        Ok(f) => f,
        Err(e) => return internal(e),
    };
    let ledger = match ledger::recent(&s.inner.pool, 6).await {
        Ok(l) => l,
        Err(e) => return internal(e),
    };
    let gateway = gateway(&s).await;
    let cell = &s.inner.config.cell;
    page(
        &s,
        "fleet.html",
        &json!({
            "shell": shell,
            "fleet": fleet,
            "ledger": ledger,
            "gateway": gateway,
            "cell": { "id": cell.id, "place": cell.place, "profile": cell.profile, "products": cell.products },
            "hour": now.with_timezone(&FixedOffset::east_opt(s.inner.config.utc_offset_minutes * 60).expect("zone")).format("%H").to_string().parse::<u32>().unwrap_or(9),
        }),
    )
}

#[derive(Debug, Deserialize)]
struct CustomersQuery {
    show: Option<String>,
}

async fn customers(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    uri: Uri,
    Query(q): Query<CustomersQuery>,
) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let shell = match shell(&s, op, "customers").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let (filter, show) = match q.show.as_deref() {
        Some("leads") => (reads::Filter::Leads, "leads"),
        Some("all") => (reads::Filter::All, "all"),
        _ => (reads::Filter::Customers, "customers"),
    };
    let now = Utc::now();
    sync_billing(&s).await;
    let rows = match reads::sites(&s.inner.pool, now, filter, 200).await {
        Ok(rows) => rows,
        Err(e) => return internal(e),
    };
    let counts = match reads::site_counts(&s.inner.pool).await {
        Ok(c) => c,
        Err(e) => return internal(e),
    };
    let sites: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
            v["kind"] = json!(r.kind());
            v["stage"] = json!(r.stage());
            v["plan"] = json!(r.plan_words());
            v
        })
        .collect();
    page(
        &s,
        "customers.html",
        &json!({ "shell": shell, "sites": sites, "show": show, "counts": counts }),
    )
}

async fn site(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    uri: Uri,
    Path(site_id): Path<String>,
) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let shell = match shell(&s, op, "customers").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    sync_billing(&s).await;
    let detail = match reads::site(&s.inner.pool, Utc::now(), &site_id).await {
        Ok(Some(d)) => d,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                page(
                    &s,
                    "app.html",
                    &json!({ "shell": shell, "which": "missing-site", "site_id": site_id }),
                ),
            )
                .into_response();
        }
        Err(e) => return internal(e),
    };
    let owned = detail.row.owner_name.is_some();
    let access = match crate::access::card(&s, &shell.operator, &site_id, owned).await {
        Ok(a) => a,
        Err(e) => return internal(e),
    };
    let ledger = match ledger::for_target(&s.inner.pool, &format!("site:{site_id}"), 10).await {
        Ok(l) => l,
        Err(e) => return internal(e),
    };
    page(
        &s,
        "site.html",
        &json!({ "shell": shell, "plan": detail.row.plan_words(), "site": detail, "ledger": ledger, "access": access }),
    )
}

async fn cli(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let shell = match shell(&s, op, "rollouts").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    // A coding agent's actions are ledger entries by an agent principal or under a cli.* verb.
    let sessions: Vec<ledger::Stored> = match ledger::recent(&s.inner.pool, 500).await {
        Ok(all) => all
            .into_iter()
            .filter(|e| e.actor.starts_with("agent:") || e.verb.starts_with("cli."))
            .take(30)
            .collect(),
        Err(e) => return internal(e),
    };
    page(
        &s,
        "cli.html",
        &json!({ "shell": shell, "sessions": sessions }),
    )
}

async fn trust(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let shell = match shell(&s, op, "trust").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let verified = match ledger::verify(&s.inner.pool).await {
        Ok(v) => v,
        Err(e) => return internal(e),
    };
    let entries = match ledger::recent(&s.inner.pool, 100).await {
        Ok(l) => l,
        Err(e) => return internal(e),
    };
    let gateway = gateway(&s).await;
    page(
        &s,
        "trust.html",
        &json!({ "shell": shell, "verified": verified, "entries": entries, "gateway": gateway }),
    )
}

async fn rollouts(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let shell = match shell(&s, op, "rollouts").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let r =
        match reads::rollouts(&s.inner.pool, Utc::now(), s.inner.config.utc_offset_minutes).await {
            Ok(r) => r,
            Err(e) => return internal(e),
        };
    let ceil = |n: i64, pct: i64| (n * pct + 99) / 100;
    page(
        &s,
        "rollouts.html",
        &json!({
            "shell": shell,
            "r": r,
            "ring2": ceil(r.customers, 10),
            "ring3": ceil(r.customers, 50),
        }),
    )
}

async fn money(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let can_finance = op.has(crate::Role::Finance);
    let shell = match shell(&s, op, "money").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let now = Utc::now();
    let billing = agentrix_billing::Billing::new(s.inner.pool.clone());
    let period = agentrix_billing::period_of(now);
    let loaded = async {
        // The provider's side first: PayPal's states and payments, as W6's gateway recorded them.
        billing.sync().await?;
        Ok::<_, agentrix_billing::Error>((
            billing.money(now).await?,
            billing.usage(now).await?,
            billing.draft_run(&period).await?,
            billing.runs().await?,
        ))
    }
    .await;
    let (money, usage, run, runs) = match loaded {
        Ok(x) => x,
        Err(e) => return internal(e),
    };
    // What the gateway itself sent to each provider this month, per site (its own estimates).
    let providers = match reads::provider_spend(&s.inner.pool, now).await {
        Ok(p) => p,
        Err(e) => return internal(e),
    };
    let vultr_of = |site: &str| {
        providers
            .get(site)
            .and_then(|p| p.get("vultr"))
            .copied()
            .unwrap_or((0, 0))
    };
    let vultr: Value = json!({
        "by_site": usage.iter().map(|u| (u.site_id.clone(), vultr_of(&u.site_id).0)).collect::<std::collections::BTreeMap<_, _>>(),
        "micro_usd": providers.values().filter_map(|p| p.get("vultr")).map(|(m, _)| m).sum::<i64>(),
        "calls": providers.values().filter_map(|p| p.get("vultr")).map(|(_, c)| c).sum::<i64>(),
    });
    page(
        &s,
        "money.html",
        &json!({
            "shell": shell,
            "money": money,
            "usage": usage,
            "vultr": vultr,
            "run": run,
            "runs": runs,
            "catalogue": agentrix_billing::catalogue(),
            "can_finance": can_finance,
        }),
    )
}

/// `/holds`: every platform hold in force, and the ones lifted lately, from `platform_holds`.
async fn holds(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed_in(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let can_hold = op.has(crate::Role::Security) || op.has(crate::Role::Sre);
    let shell = match shell(&s, op, "holds").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let loaded = async {
        let active = crate::holds::all_active(pool).await?;
        let lifted = crate::holds::recently_lifted(pool, 20).await?;
        let mut rows = Vec::new();
        for (on, h) in active
            .into_iter()
            .map(|h| (true, h))
            .chain(lifted.into_iter().map(|h| (false, h)))
        {
            let names: Option<(String, Option<String>)> = if h.scope == "site" {
                sqlx::query_as("SELECT domain, name FROM sites WHERE site_id = ?")
                    .bind(&h.target)
                    .fetch_optional(pool)
                    .await?
            } else {
                None
            };
            let placed_by = account_name(pool, &h.placed_by).await?;
            let lifted_by = match &h.lifted_by {
                Some(id) => Some(account_name(pool, id).await?),
                None => None,
            };
            let mut v = serde_json::to_value(&h).unwrap_or(Value::Null);
            v["on"] = json!(on);
            v["domain"] = json!(names.as_ref().map(|(d, _)| d));
            v["placed_by_name"] = json!(placed_by);
            v["lifted_by_name"] = json!(lifted_by);
            rows.push(v);
        }
        Ok::<_, sqlx::Error>(rows)
    }
    .await;
    let rows = match loaded {
        Ok(r) => r,
        Err(e) => return internal(e),
    };
    let active = rows.iter().filter(|r| r["on"] == json!(true)).count();
    page(
        &s,
        "holds.html",
        &json!({
            "shell": shell,
            "holds": rows,
            "active": active,
            "enforced": s.inner.config.holds_enforced,
            "can_hold": can_hold,
        }),
    )
}

/// An account's name for a stored id, or the id when there's no such account.
async fn account_name(pool: &sqlx::SqlitePool, id: &str) -> Result<String, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT display_name FROM accounts WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(n,)| n).unwrap_or_else(|| id.to_owned()))
}
