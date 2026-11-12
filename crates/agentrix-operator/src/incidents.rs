//! Incidents (08 §3.7): declared by an on-call person, worked on a timeline, mitigated, resolved
//! and closed with a blameless postmortem. Every change is an operator ledger entry.

use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;

use crate::OperatorState;
use crate::identity::{Operator, Role};
use crate::ledger::{self, Entry};
use crate::notice;
use crate::pages::internal;

pub(crate) fn routes() -> Router<OperatorState> {
    Router::new()
        .route("/incidents", get(list).post(declare))
        .route("/incidents/{id}", get(one))
        .route("/incidents/{id}/events", post(add_event))
        .route("/incidents/{id}/status", post(set_status))
        .route("/incidents/{id}/postmortem", post(postmortem))
}

/// An incident.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Incident {
    /// `INC-0001`.
    pub incident_id: String,
    /// `sev1` to `sev4`.
    pub severity: String,
    /// What's wrong, in a line.
    pub title: String,
    /// More words.
    pub summary: String,
    /// The cell.
    pub cell: String,
    /// One affected site, if it's about one.
    pub site_id: Option<String>,
    /// The commander's account id.
    pub commander: String,
    /// `open`, `mitigated`, `resolved`.
    pub state: String,
    /// Who declared it.
    pub opened_by: String,
    /// When.
    pub opened_at: String,
    /// Mitigated.
    pub mitigated_at: Option<String>,
    /// Resolved.
    pub resolved_at: Option<String>,
    /// The postmortem, once written.
    pub postmortem: Option<String>,
}

/// A timeline entry.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Event {
    /// Order.
    pub seq: i64,
    /// When.
    pub at: String,
    /// Who.
    pub actor_name: String,
    /// `note`, `evidence`, `status`, `comms`.
    pub kind: String,
    /// What.
    pub words: String,
}

macro_rules! inc_select {
    ($rest:literal) => {
        concat!(
            "SELECT incident_id, severity, title, summary, cell, site_id, commander, state, opened_by, opened_at, mitigated_at, resolved_at, postmortem FROM incidents ",
            $rest
        )
    };
}

/// Open and mitigated incidents, worst first.
pub async fn open(pool: &SqlitePool) -> Result<Vec<Incident>, sqlx::Error> {
    sqlx::query_as(inc_select!(
        "WHERE state <> 'resolved' ORDER BY severity, opened_at DESC"
    ))
    .fetch_all(pool)
    .await
}

async fn recent_resolved(pool: &SqlitePool) -> Result<Vec<Incident>, sqlx::Error> {
    sqlx::query_as(inc_select!(
        "WHERE state = 'resolved' ORDER BY resolved_at DESC LIMIT 30"
    ))
    .fetch_all(pool)
    .await
}

async fn get_one(pool: &SqlitePool, id: &str) -> Result<Option<Incident>, sqlx::Error> {
    sqlx::query_as(inc_select!("WHERE incident_id = ?"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn events(pool: &SqlitePool, id: &str) -> Result<Vec<Event>, sqlx::Error> {
    sqlx::query_as("SELECT seq, at, actor_name, kind, words FROM incident_events WHERE incident_id = ? ORDER BY seq")
        .bind(id)
        .fetch_all(pool)
        .await
}

async fn append_event(
    pool: &SqlitePool,
    id: &str,
    op: &Operator,
    kind: &str,
    words: &str,
) -> Result<(), sqlx::Error> {
    let (next,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM incident_events WHERE incident_id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO incident_events (incident_id, seq, at, actor, actor_name, kind, words) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(id)
        .bind(next)
        .bind(ledger::now())
        .bind(op.actor())
        .bind(&op.name)
        .bind(kind)
        .bind(words)
        .execute(pool)
        .await?;
    Ok(())
}

/// Anyone with an operator role but read-only audit can work an incident.
fn can_work(op: &Operator) -> bool {
    op.roles.iter().any(|r| *r != Role::Auditor)
}

/// Declaring needs on-call authority: op.sre, op.security or op.admin.
fn can_declare(op: &Operator) -> bool {
    op.has(Role::Sre) || op.has(Role::Security) || op.has(Role::Admin)
}

fn sev_words(sev: &str) -> &'static str {
    match sev {
        "sev1" => "SEV1",
        "sev2" => "SEV2",
        "sev3" => "SEV3",
        _ => "SEV4",
    }
}

async fn signed(s: &OperatorState, headers: &HeaderMap, uri: &Uri) -> Result<Operator, Response> {
    match s.operator_from(headers).await {
        Ok(Some(op)) => Ok(op),
        Ok(None) => {
            let next: String =
                url::form_urlencoded::byte_serialize(uri.path().as_bytes()).collect();
            Err(Redirect::to(&format!("/signin?next={next}")).into_response())
        }
        Err(e) => Err(internal(e)),
    }
}

async fn list(State(s): State<OperatorState>, headers: HeaderMap, uri: Uri) -> Response {
    let op = match signed(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let can = can_declare(&op);
    let shell = match crate::pages::shell(&s, op, "incidents").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let loaded =
        async { Ok::<_, sqlx::Error>((open(pool).await?, recent_resolved(pool).await?)) }.await;
    let (open, resolved) = match loaded {
        Ok(x) => x,
        Err(e) => return internal(e),
    };
    crate::pages::page(
        &s,
        "incidents.html",
        &json!({ "shell": shell, "open": open, "resolved": resolved, "can_declare": can, "cell": s.inner.config.cell.id }),
    )
}

#[derive(Debug, Deserialize)]
struct Declare {
    severity: String,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    site: String,
}

/// The signed-in operator for a form post, or where the browser goes instead.
async fn form_operator(
    s: &OperatorState,
    headers: &HeaderMap,
    back: &str,
) -> Result<Operator, Response> {
    if !s.same_origin(headers) {
        return Err(notice::cross_site());
    }
    match s.operator_from(headers).await {
        Ok(Some(op)) => Ok(op),
        Ok(None) => Err(notice::sign_in_again(back)),
        Err(e) => Err(internal(e)),
    }
}

const READ_ONLY: &str = "Nothing changed: your role is read-only audit, so you can read this incident but not change it.";

async fn declare(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Form(f): Form<Declare>,
) -> Response {
    let back = "/incidents";
    let op = match form_operator(&s, &headers, back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_declare(&op) {
        return notice::problem(
            &s,
            back,
            "Nothing was declared: declaring an incident needs op.sre, op.security or op.admin. Ask the on-call SRE to declare, or an admin to add the role.",
        );
    }
    let title = f.title.trim();
    if !["sev1", "sev2", "sev3", "sev4"].contains(&f.severity.as_str())
        || title.is_empty()
        || title.chars().count() > 140
    {
        return notice::problem_keeping(
            &s,
            back,
            "Nothing was declared. Pick a severity and say what's wrong in a line.",
            &[
                ("title", &f.title),
                ("summary", &f.summary),
                ("site", &f.site),
            ],
        );
    }
    let pool = &s.inner.pool;
    let site = f.site.trim();
    let site_id = if site.is_empty() {
        None
    } else {
        let found: Option<(String,)> = match sqlx::query_as(
            "SELECT site_id FROM sites WHERE site_id = ?1 OR domain = ?1 COLLATE NOCASE",
        )
        .bind(site)
        .fetch_optional(pool)
        .await
        {
            Ok(f) => f,
            Err(e) => return internal(e),
        };
        match found {
            Some((id,)) => Some(id),
            None => {
                return notice::problem_keeping(
                    &s,
                    back,
                    format!(
                        "Nothing was declared: no site has the id or address {site}. Leave it empty if the incident isn't about one site."
                    ),
                    &[
                        ("title", &f.title),
                        ("summary", &f.summary),
                        ("site", &f.site),
                    ],
                );
            }
        }
    };
    let (n,): (i64,) = match sqlx::query_as("SELECT COUNT(*) FROM incidents")
        .fetch_one(pool)
        .await
    {
        Ok(n) => n,
        Err(e) => return internal(e),
    };
    let id = format!("INC-{:04}", n + 1);
    let now = ledger::now();
    let inserted = sqlx::query(
        "INSERT INTO incidents (incident_id, severity, title, summary, cell, site_id, commander, state, opened_by, opened_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'open', ?, ?)",
    )
    .bind(&id)
    .bind(&f.severity)
    .bind(title)
    .bind(f.summary.trim())
    .bind(&s.inner.config.cell.id)
    .bind(&site_id)
    .bind(op.account.to_string())
    .bind(op.account.to_string())
    .bind(&now)
    .execute(pool)
    .await;
    if let Err(e) = inserted {
        return internal(e);
    }
    let words = format!(
        "{} declared {} and took command.",
        op.name,
        sev_words(&f.severity)
    );
    if let Err(e) = append_event(pool, &id, &op, "status", &words).await {
        return internal(e);
    }
    let target = site_id
        .as_ref()
        .map(|s| format!("site:{s}"))
        .unwrap_or_else(|| format!("cell:{}", s.inner.config.cell.id));
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            "incident.open",
            &target,
            &format!(
                "{} opened {id}, {}: {title}",
                op.name,
                sev_words(&f.severity)
            ),
        )
        .detail(json!({ "incident": id, "severity": f.severity })),
    )
    .await
    {
        return internal(e);
    }
    notice::done(
        &s,
        &format!("/incidents/{id}"),
        format!(
            "You declared {id}, {}, and you're its commander. Add what you see to the timeline as you go.",
            sev_words(&f.severity)
        ),
    )
}

async fn one(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    uri: Uri,
    Path(id): Path<String>,
) -> Response {
    let op = match signed(&s, &headers, &uri).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    let can = can_work(&op);
    let shell = match crate::pages::shell(&s, op, "incidents").await {
        Ok(shell) => shell,
        Err(r) => return r,
    };
    let pool = &s.inner.pool;
    let Some(inc) = (match get_one(pool, &id).await {
        Ok(i) => i,
        Err(e) => return internal(e),
    }) else {
        return (
            StatusCode::NOT_FOUND,
            crate::pages::page(
                &s,
                "app.html",
                &json!({ "shell": shell, "which": "missing-incident", "site_id": id }),
            ),
        )
            .into_response();
    };
    let evs = match events(pool, &id).await {
        Ok(e) => e,
        Err(e) => return internal(e),
    };
    let commander = crate::identity::account(
        pool,
        uuid::Uuid::parse_str(&inc.commander).unwrap_or_default(),
    )
    .await
    .ok()
    .flatten()
    .map(|a| a.name)
    .unwrap_or_default();
    let domain: Option<(String,)> = match &inc.site_id {
        Some(site) => sqlx::query_as("SELECT domain FROM sites WHERE site_id = ?")
            .bind(site)
            .fetch_optional(pool)
            .await
            .unwrap_or(None),
        None => None,
    };
    // The status page's gateway check, from an hour before it opened to an hour after it ended.
    let opened = crate::reads::parse(&inc.opened_at).unwrap_or_else(chrono::Utc::now);
    let ended = inc
        .resolved_at
        .as_deref()
        .and_then(crate::reads::parse)
        .unwrap_or_else(chrono::Utc::now);
    let gateway_hours = match crate::reads::check_hours(
        pool,
        "ask",
        opened - chrono::Duration::hours(1),
        ended + chrono::Duration::hours(1),
    )
    .await
    {
        Ok(h) => h,
        Err(e) => return internal(e),
    };
    crate::pages::page(
        &s,
        "incident.html",
        &json!({ "shell": shell, "inc": inc, "events": evs, "commander": commander, "domain": domain.map(|d| d.0), "can_work": can, "sev": sev_words(&inc.severity), "gateway_hours": gateway_hours }),
    )
}

#[derive(Debug, Deserialize)]
struct Note {
    kind: String,
    words: String,
}

async fn add_event(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<Note>,
) -> Response {
    let back = format!("/incidents/{id}#timeline");
    let op = match form_operator(&s, &headers, &back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_work(&op) {
        return notice::problem(&s, &back, READ_ONLY);
    }
    let words = f.words.trim();
    if !["note", "evidence", "comms"].contains(&f.kind.as_str())
        || words.is_empty()
        || words.chars().count() > 2000
    {
        return notice::problem_keeping(
            &s,
            &back,
            "Nothing was added: the note is empty or over 2,000 characters. Shorten it and add it again.",
            &[("note", &f.words)],
        );
    }
    let pool = &s.inner.pool;
    match get_one(pool, &id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return notice::problem(
                &s,
                "/incidents",
                "No incident has that id, so nothing was added.",
            );
        }
        Err(e) => return internal(e),
    }
    if let Err(e) = append_event(pool, &id, &op, &f.kind, words).await {
        return internal(e);
    }
    let what = match f.kind.as_str() {
        "evidence" => "evidence",
        "comms" => "a message draft",
        _ => "a note",
    };
    notice::done(&s, &back, format!("You added {what} to the timeline."))
}

#[derive(Debug, Deserialize)]
struct Status {
    state: String,
}

async fn set_status(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<Status>,
) -> Response {
    let back = format!("/incidents/{id}");
    let op = match form_operator(&s, &headers, &back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_work(&op) {
        return notice::problem(&s, &back, READ_ONLY);
    }
    let pool = &s.inner.pool;
    let Some(inc) = (match get_one(pool, &id).await {
        Ok(i) => i,
        Err(e) => return internal(e),
    }) else {
        return notice::problem(
            &s,
            "/incidents",
            "No incident has that id, so nothing changed.",
        );
    };
    let sql = match (inc.state.as_str(), f.state.as_str()) {
        ("open", "mitigated") => {
            "UPDATE incidents SET state = 'mitigated', mitigated_at = ?1 WHERE incident_id = ?2 AND state = 'open'"
        }
        ("open" | "mitigated", "resolved") => {
            "UPDATE incidents SET state = 'resolved', resolved_at = ?1, mitigated_at = COALESCE(mitigated_at, ?1) WHERE incident_id = ?2 AND state <> 'resolved'"
        }
        ("resolved", "open") => {
            "UPDATE incidents SET state = 'open', resolved_at = NULL, mitigated_at = NULL WHERE incident_id = ?2 AND state = 'resolved' AND ?1 IS NOT NULL"
        }
        _ => {
            return notice::problem(
                &s,
                &back,
                format!(
                    "Nothing changed: {id} is {} and can't go to {} from there. Reload to see where it stands.",
                    inc.state, f.state
                ),
            );
        }
    };
    if let Err(e) = sqlx::query(sql)
        .bind(ledger::now())
        .bind(&id)
        .execute(pool)
        .await
    {
        return internal(e);
    }
    let words = match f.state.as_str() {
        "mitigated" => format!(
            "{} marked it mitigated: the harm has stopped, the cause may remain.",
            op.name
        ),
        "resolved" => format!("{} marked it resolved.", op.name),
        _ => format!("{} opened it again.", op.name),
    };
    if let Err(e) = append_event(pool, &id, &op, "status", &words).await {
        return internal(e);
    }
    let target = inc
        .site_id
        .as_ref()
        .map(|s| format!("site:{s}"))
        .unwrap_or_else(|| format!("cell:{}", inc.cell));
    if let Err(e) = ledger::append(
        pool,
        Entry::person(
            op.account,
            &op.name,
            &format!("incident.{}", f.state),
            &target,
            &format!("{id}: {words}"),
        ),
    )
    .await
    {
        return internal(e);
    }
    notice::done(
        &s,
        &back,
        match f.state.as_str() {
            "mitigated" => format!("{id} is mitigated. Resolve it once the cause is fixed."),
            "resolved" => format!("{id} is resolved. Write the postmortem below."),
            _ => format!("{id} is open again."),
        },
    )
}

#[derive(Debug, Deserialize)]
struct Postmortem {
    words: String,
}

async fn postmortem(
    State(s): State<OperatorState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<Postmortem>,
) -> Response {
    let back = format!("/incidents/{id}#postmortem");
    let op = match form_operator(&s, &headers, &back).await {
        Ok(op) => op,
        Err(r) => return r,
    };
    if !can_work(&op) {
        return notice::problem(&s, &back, READ_ONLY);
    }
    let words = f.words.trim();
    if words.is_empty() || words.chars().count() > 20_000 {
        return notice::problem_keeping(
            &s,
            &back,
            "Nothing was saved. Write what happened, why, and what changes now, in under 20,000 characters.",
            &[("postmortem", &f.words)],
        );
    }
    let pool = &s.inner.pool;
    let done = sqlx::query(
        "UPDATE incidents SET postmortem = ? WHERE incident_id = ? AND state = 'resolved'",
    )
    .bind(words)
    .bind(&id)
    .execute(pool)
    .await;
    match done {
        Ok(d) if d.rows_affected() == 1 => {}
        Ok(_) => {
            return notice::problem_keeping(
                &s,
                &back,
                "Nothing was saved: write the postmortem once the incident is resolved.",
                &[("postmortem", &f.words)],
            );
        }
        Err(e) => return internal(e),
    }
    if let Err(e) = append_event(
        pool,
        &id,
        &op,
        "note",
        &format!("{} saved the postmortem.", op.name),
    )
    .await
    {
        return internal(e);
    }
    notice::done(&s, &back, "You saved the postmortem.")
}
