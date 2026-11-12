//! The shell's routes. See the table in the crate docs.

use std::sync::Arc;

use agentrix_console::view as cv;
use agentrix_types::{Digest, EffectRequest, ReceiptBody, ReceiptKind, SiteId, Subject};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::apps::{AppContext, AppPage, AppReply, ConsoleApp};
use crate::port::{Access, Attempt, CardRow, Happened, Owner};
use crate::schedule::{self, Mode, QuietHours, TimePill};
use crate::view::*;
use crate::{ShellPage, ShellState};

pub(crate) fn router(state: ShellState) -> Router {
    Router::new()
        .route("/os", get(entry))
        .route("/os/{*page}", get(entry_page))
        .route("/d/{card}", get(short_link))
        .route("/console/{site}/apps", get(apps_home))
        .route("/console/{site}", get(home))
        .route("/console/{site}/decisions", get(decisions))
        .route("/console/{site}/decisions/{id}", get(decision))
        .route("/console/{site}/cards/{id}/sheet", get(sheet))
        .route("/console/{site}/webagents", get(webagents))
        .route("/console/{site}/webagents/{id}", get(webagent))
        .route("/console/{site}/orders", get(crate::commerce_pages::orders))
        .route(
            "/console/{site}/orders/{id}",
            get(crate::commerce_pages::order),
        )
        .route(
            "/console/{site}/customers",
            get(crate::commerce_pages::customers),
        )
        .route(
            "/console/{site}/customers/{key}",
            get(crate::commerce_pages::customer),
        )
        .route("/console/{site}/trust", get(trust))
        .route("/console/{site}/website", get(website))
        .route("/console/{site}/store", get(store))
        .route("/console/{site}/memory", get(memory))
        .route("/console/{site}/gestures", get(gestures))
        .route("/console/{site}/live", get(live_list))
        .route("/console/{site}/live/{thread}", get(live_thread))
        .route("/console/{site}/live/{thread}/turns", get(live_turns))
        .route("/console/{site}/live/{thread}/{act}", post(live_act))
        .route("/console/{site}/walk", get(walk))
        .route(
            "/console/{site}/missions",
            get(missions).post(mission_start),
        )
        .route("/console/{site}/missions/draft", post(mission_draft))
        .route("/console/{site}/missions/{id}", get(mission))
        .route("/console/{site}/missions/{id}/state", post(mission_state))
        .route("/console/{site}/store/add", post(store_add))
        .route(
            "/console/{site}/webagents/{id}/remove",
            post(webagent_remove),
        )
        .route("/console/{site}/quarantine", get(quarantine))
        .route("/console/{site}/settings", get(settings))
        .route(
            "/console/{site}/settings/quiet-hours",
            post(set_quiet_hours),
        )
        .route("/console/{site}/freeze", post(freeze))
        .route("/console/{site}/unfreeze/sheet", post(unfreeze_sheet))
        .route("/console/{site}/unfreeze", post(unfreeze))
        .route("/console/{site}/apps/{app}", get(app_get).post(app_post))
        .route(
            "/console/{site}/apps/{app}/{*path}",
            get(app_get_path).post(app_post_path),
        )
        .with_state(state)
}

// ---------- Errors ----------

/// Why a page didn't render.
pub(crate) enum PageError {
    Go(String),
    NotFound,
    Internal(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for PageError {
    fn from(e: E) -> Self {
        Self::Internal(e.into())
    }
}

const NOT_FOUND: &str = "<!doctype html><meta charset=utf-8><title>Not found · Agentrix</title><link rel=stylesheet href=\"/static/os/styles.css\"><main class=\"wrap section\"><h2>We couldn't find that</h2><p class=\"lead\">It may belong to another site, or it was never there. <a href=\"/os\">Back to your console</a></p></main>";
const BROKEN: &str = "<!doctype html><meta charset=utf-8><title>This page didn't load · Agentrix</title><link rel=stylesheet href=\"/static/os/styles.css\"><main class=\"wrap section\"><h2>This page didn't load</h2><p class=\"lead\">Nothing was changed. Try again in a moment; if it keeps happening, tell us and we'll look at the log.</p></main>";

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        match self {
            Self::Go(path) => Redirect::to(&path).into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, Html(NOT_FOUND)).into_response(),
            Self::Internal(e) => {
                tracing::error!(error = %format!("{e:#}"), "shell page failed");
                (StatusCode::INTERNAL_SERVER_ERROR, Html(BROKEN)).into_response()
            }
        }
    }
}

pub(crate) type PageResult = Result<Response, PageError>;

pub(crate) fn render(
    state: &ShellState,
    page: ShellPage,
    view: &impl serde::Serialize,
) -> PageResult {
    let html = state.shell.render(page, view)?;
    Ok(([(header::CACHE_CONTROL, "private, no-store")], Html(html)).into_response())
}

pub(crate) async fn owner(
    state: &ShellState,
    headers: &HeaderMap,
    site: Option<&str>,
    uri: &Uri,
) -> Result<Owner, PageError> {
    let back = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/os");
    match state.port.access(headers, site, back).await? {
        Access::Owner(o) => Ok(o),
        Access::Redirect(to) => Err(PageError::Go(to)),
        Access::NotFound => Err(PageError::NotFound),
    }
}

// ---------- Words ----------

pub(crate) fn base(o: &Owner) -> String {
    format!("/console/{}", o.site.id)
}

fn monogram(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "A".into())
}

fn risk_class(r: cv::Risk) -> &'static str {
    match r {
        cv::Risk::Low => "ok",
        cv::Risk::Medium => "wait",
        cv::Risk::High => "risk",
    }
}

fn expires_words(at: DateTime<Utc>, now: DateTime<Utc>, offset: i32) -> String {
    if at <= now {
        "Expired: nothing was done".to_owned()
    } else if at - now > Duration::days(60) {
        "Waits for you; nothing happens until you answer".to_owned()
    } else {
        format!(
            "Waits until {}; nothing happens if you don't answer",
            schedule::when(at, now, offset)
        )
    }
}

pub(crate) fn lane_card(row: &CardRow, o: &Owner, now: DateTime<Utc>) -> LaneCard {
    let base = base(o);
    let offset = o.site.utc_offset_minutes;
    LaneCard {
        id: row.id.clone(),
        kind: row.kind.clone(),
        title: row.title.clone(),
        summary: row.summary.clone(),
        asked_by: row.asked_by.clone(),
        from_words: format!("{} asks", row.asked_by.name),
        risk_class: risk_class(row.risk).to_owned(),
        risk_words: row.risk_words.clone(),
        created_words: format!("Raised {}", schedule::when(row.created_at, now, offset)),
        expires_words: expires_words(row.expires_at, now, offset),
        href: format!("{base}/decisions/{}", row.id),
        sheet_url: format!("{base}/cards/{}/sheet", row.id),
        decline_url: row.decline_url.clone(),
        outcome_words: row.outcome_words.clone(),
    }
}

fn timeline_item(h: &Happened, now: DateTime<Utc>, offset: i32) -> TimelineItem {
    let (tag, class) = match h.kind.as_str() {
        "decision.approved" | "decision.declined" => ("You decided", "ok"),
        "door.order-held" => ("Waiting for you", "warn"),
        "platform.hold" | "operator.access" => ("Zavora", "warn"),
        "payment.refunded" => ("Refunded", ""),
        "concierge.refusal" => ("Refused", ""),
        "release.rollback" => ("Undone", "warn"),
        _ if h.failed => ("Failed", "warn"),
        _ => ("Done", "ok"),
    };
    let t = if now - h.at < Duration::hours(20) {
        schedule::clock(h.at.with_timezone(&schedule::offset(offset)).time())
    } else {
        schedule::when(h.at, now, offset)
    };
    TimelineItem {
        t,
        title: h.title.clone(),
        tag: tag.to_owned(),
        class: class.to_owned(),
        href: format!("/receipts/{}", h.digest),
    }
}

// ---------- Chrome ----------

/// Builds the status bar, dock and lane for `active`.
pub(crate) async fn chrome(
    state: &ShellState,
    o: &Owner,
    active: &str,
    cards: &[CardRow],
) -> Result<Chrome, PageError> {
    let now = Utc::now();
    let base = base(o);
    let waiting: Vec<&CardRow> = cards
        .iter()
        .filter(|c| c.state == cv::CardState::Waiting)
        .collect();
    let hours = state.store.quiet_hours(&o.site.id).await?;
    let time = TimePill::at(&hours, now, o.site.utc_offset_minutes);
    let frozen = state.store.freeze(&o.site.id).await?;
    // Conversations with a turn in the last 10 minutes; a failed read shows none.
    let live_now = state
        .store
        .live()
        .threads(
            &o.site.id,
            now - Duration::minutes(crate::live::LIVE_FOR_MINUTES),
        )
        .await
        .map(|t| u32::try_from(t.len()).unwrap_or(u32::MAX))
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "live conversations unreadable");
            0
        });
    // Live is offered only once the site's desks have sent it a conversation.
    let live_on = live_now > 0
        || state
            .store
            .live()
            .connected(&o.site.id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "live conversations unreadable");
                false
            });

    // Spend is shown when it can be read; a failed read hides the pill rather than the page.
    let meter = state.port.meter(o).await.unwrap_or_else(|e| {
        tracing::warn!(error = %format!("{e:#}"), "spend read failed; the budget pill is hidden");
        None
    });
    let budget = meter.as_ref().map(|m| Budget {
        amount: m.spent.clone(),
        words: m.cap_words.clone(),
        pct: m.cap_pct.map(|p| p.min(100)),
    });
    let activity = meter.map(|m| m.by_purpose).unwrap_or_default();
    let gauntlet = state.port.gauntlet(o).await?;
    let trust = TrustPill {
        words: match &gauntlet {
            Some(g) if g.held == g.of && g.of > 0 => "Protections on".to_owned(),
            Some(g) => format!("{} of {} held", g.held, g.of),
            None => "Trust centre".to_owned(),
        },
        ok: gauntlet
            .as_ref()
            .is_some_and(|g| g.held == g.of && g.of > 0),
        waiting: u32::try_from(waiting.len()).unwrap_or(u32::MAX),
    };

    // The dock: each WebAgent with its mark. A WebAgent with a card waiting shows the amber dot
    // and a badge (its process is parked on the owner); a frozen site greys every mark. W1's
    // roster gives the living marks; without one, the charter's WebAgents draw letter marks.
    let roster = state.port.roster(o).await?;
    let asks_for = |name: &str, id: &str| {
        waiting
            .iter()
            .filter(|c| {
                c.asked_by.name.eq_ignore_ascii_case(name)
                    || c.asked_by.name.eq_ignore_ascii_case(id)
            })
            .count()
    };
    let presence = |asks: usize| -> (&'static str, &'static str) {
        if frozen.is_some() {
            ("paused", "Frozen")
        } else if asks > 0 {
            ("waiting", "Waiting for you")
        } else {
            ("idle", "")
        }
    };
    let dock: Vec<DockItem> = match &roster {
        Some(r) => r
            .on_duty()
            .filter(|m| m.spec.id != "coordinator")
            .map(|m| {
                let asks = asks_for(&m.spec.name, &m.spec.id);
                let (st, words) = presence(asks);
                DockItem {
                    id: m.spec.id.clone(),
                    name: m.spec.name.clone(),
                    title: if words.is_empty() { format!("{}: {}", m.spec.name, m.spec.role_words) } else { format!("{}: {}", m.spec.name, words.to_lowercase()) },
                    href: format!("{base}/webagents#agent-{}", m.spec.id),
                    mark: None,
                    agent: Some(serde_json::json!({ "id": m.spec.id, "name": m.spec.name, "spec": m.spec.mark, "state": st, "owner": o.name })),
                    icon: String::new(),
                    badge: u32::try_from(asks).unwrap_or(u32::MAX),
                    current: false,
                    opens: None,
                }
            })
            .collect(),
        None => state
            .port
            .webagents(o)
            .await?
            .agents
            .iter()
            .map(|a| {
                let asks = asks_for(&a.name, &a.name);
                let mut mark = a.mark.clone();
                let (st, words) = presence(asks);
                if st == "paused" {
                    mark.state = cv::MarkState::Paused;
                } else if st == "waiting" {
                    mark.state = cv::MarkState::Waiting;
                }
                if !words.is_empty() {
                    mark.state_words = words.into();
                }
                DockItem {
                    id: slug(&a.name),
                    name: a.name.clone(),
                    title: if words.is_empty() { format!("{}: {}", a.name, a.role_words) } else { format!("{}: {}", a.name, words.to_lowercase()) },
                    href: format!("{base}/webagents#agent-{}", slug(&a.name)),
                    mark: Some(mark),
                    agent: None,
                    icon: String::new(),
                    badge: u32::try_from(asks).unwrap_or(u32::MAX),
                    current: false,
                    opens: None,
                }
            })
            .collect(),
    };
    let team = roster
        .as_ref()
        .map(|r| serde_json::to_value(r.team()))
        .transpose()?;

    let sys = |id: &str,
               name: &str,
               title: &str,
               href: String,
               icon: &str,
               badge: u32,
               opens: Option<&str>| DockItem {
        id: id.to_owned(),
        name: name.to_owned(),
        title: title.to_owned(),
        href,
        mark: None,
        agent: None,
        icon: icon.to_owned(),
        badge,
        current: active == id,
        opens: opens.map(str::to_owned),
    };
    // Orders waiting for the owner's answer, for the Orders badge; a failed read shows none.
    let orders_waiting = state
        .store
        .commerce()
        .waiting_count(&o.site.id, now)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %format!("{e:#}"), "orders unreadable; the badge is hidden");
            0
        });
    let system = vec![
        sys(
            "decisions",
            "Decisions",
            "Every decision, waiting and decided",
            format!("{base}/decisions"),
            "decisions",
            trust.waiting,
            None,
        ),
        sys(
            "orders",
            "Orders",
            "Orders: what customers asked for, and what needs you",
            format!("{base}/orders"),
            "orders",
            orders_waiting,
            None,
        ),
        sys(
            "customers",
            "Customers",
            "Customers: the people your site served",
            format!("{base}/customers"),
            "people",
            0,
            None,
        ),
        sys(
            "activity",
            "Activity",
            "Activity monitor: what each job spent today",
            format!("{base}#activity"),
            "activity",
            0,
            Some("activity"),
        ),
        sys(
            "website",
            "Website",
            "Website: your releases, roll back, and what visitors asked",
            format!("{base}/website"),
            "website",
            0,
            None,
        ),
        sys(
            "store",
            "Store",
            "Store: add WebAgents to your team",
            format!("{base}/store"),
            "store",
            0,
            None,
        ),
        sys(
            "live",
            "Live",
            "Live: sit in on conversations, take over, hand back",
            format!("{base}/live"),
            "live",
            live_now,
            None,
        ),
        sys(
            "missions",
            "Missions",
            "Missions: goals you set, and their count",
            format!("{base}/missions"),
            "missions",
            0,
            None,
        ),
        sys(
            "memory",
            "Memory",
            "Memory and rules: what your WebAgents know and may do",
            format!("{base}/memory"),
            "memory",
            0,
            None,
        ),
        sys(
            "trust",
            "Trust",
            "Trust centre: receipts, protections and Freeze",
            format!("{base}/trust"),
            "trust",
            0,
            None,
        ),
        sys(
            "settings",
            "Settings",
            "Quiet hours and how the console behaves",
            format!("{base}/settings"),
            "settings",
            0,
            None,
        ),
    ];
    let system: Vec<DockItem> = system
        .into_iter()
        .filter(|d| live_on || d.id != "live")
        .collect();

    let ctx = app_context(o, "");
    let mut registered: Vec<(u32, DockItem)> = Vec::new();
    for app in &state.apps {
        let spec = app.spec();
        let ctx = AppContext {
            base: format!("{base}/apps/{}", spec.id),
            ..ctx.clone()
        };
        if !app.available(&ctx).await {
            continue;
        }
        let badge = app.badge(&ctx).await.unwrap_or_else(|e| {
            tracing::warn!(app = %spec.id, error = %format!("{e:#}"), "app badge failed");
            0
        });
        let id = format!("app:{}", spec.id);
        registered.push((
            spec.order,
            DockItem {
                current: active == id,
                id,
                name: spec.name,
                title: spec.title,
                href: ctx.base.clone(),
                mark: None,
                agent: None,
                icon: spec.icon,
                badge,
                opens: None,
            },
        ));
    }
    registered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));

    // Done for you: today's newest receipts that weren't the owner's own decisions.
    let midnight = local_midnight(now, o.site.utc_offset_minutes);
    let done_words: Vec<String> = state
        .port
        .happened(o, midnight, now + Duration::minutes(1))
        .await?
        .iter()
        .rev()
        .filter(|h| !h.kind.starts_with("decision.") && !h.failed)
        .take(3)
        .map(|h| h.title.clone())
        .collect();

    let apps: Vec<DockItem> = registered.into_iter().map(|(_, d)| d).collect();

    // Items other crates own (C9 lane sources): each links to the page where the owner decides.
    let mut others = Vec::new();
    for source in &state.lanes {
        match source.needs_you(&ctx).await {
            Ok(items) => others.extend(items),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "a lane source failed; the lane shows the rest")
            }
        }
    }
    let page = |label: &str, href: String| Command {
        label: label.to_owned(),
        kind: "Page".into(),
        href,
    };
    let mut commands = vec![
        page("Back office", base.clone()),
        page("Front of house", format!("{base}?world=front")),
        page("Decisions", format!("{base}/decisions")),
        page("Orders", format!("{base}/orders")),
        page("Customers", format!("{base}/customers")),
        page("WebAgents", format!("{base}/webagents")),
        page("Trust centre", format!("{base}/trust")),
        page("Website and releases", format!("{base}/website")),
        page("Store: add a WebAgent", format!("{base}/store")),
        page("Memory and rules", format!("{base}/memory")),
        page("Hand gestures", format!("{base}/gestures")),
        page("Missions", format!("{base}/missions")),
        page("Live conversations", format!("{base}/live")),
        page("Walk your site", format!("{base}/walk")),
        page("Receipts", format!("{base}/trust#receipts")),
        page("Left behind by the Move", format!("{base}/quarantine")),
        page("Quiet hours and Night Time", format!("{base}/settings")),
        page("The Move", "/console/move".to_owned()),
    ];
    if !live_on {
        commands.retain(|c| !c.href.ends_with("/live"));
    }
    commands.extend(apps.iter().map(|a| Command {
        label: a.name.clone(),
        kind: "App".into(),
        href: a.href.clone(),
    }));
    commands.extend(waiting.iter().map(|c| Command {
        label: c.title.clone(),
        kind: "Decision".into(),
        href: format!("{base}/decisions/{}", c.id),
    }));

    let commands_json = serde_json::to_string(&commands)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");

    let offset = o.site.utc_offset_minutes;
    let quiet_hours = (offset % 60 == 0).then(|| QuietHoursAttr {
        from: time.start.clone(),
        to: time.end.clone(),
        // IANA's Etc zones count the other way: UTC+3 is Etc/GMT-3.
        tz: match offset / 60 {
            0 => "UTC".to_owned(),
            h if h > 0 => format!("Etc/GMT-{h}"),
            h => format!("Etc/GMT+{}", -h),
        },
    });

    Ok(Chrome {
        team,
        presence_url: format!("/api/webagents/{}/presence", o.site.id),
        freeze_enabled: state.freeze_enabled,
        night: time.mode == Mode::Night,
        quiet_hours,
        commands_json,
        activity,
        monogram: monogram(&o.site.name),
        base,
        owner_name: o.name.clone(),
        active: active.to_owned(),
        world: if active == "live" { "front" } else { "back" }.to_owned(),
        live_now,
        live_on,
        time,
        budget,
        trust,
        freeze: frozen.map(|f| FreezeView {
            words: format!(
                "Frozen {} by {}",
                schedule::when(f.at, now, o.site.utc_offset_minutes),
                if f.by == o.account {
                    "you"
                } else {
                    "someone with access"
                }
            ),
        }),
        dock,
        system,
        apps,
        lane: Lane {
            others,
            needs_you: waiting.iter().map(|c| lane_card(c, o, now)).collect(),
            done_words,
        },
        has_passkey: o.has_passkey,
        site: o.site.clone(),
    })
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn local_midnight(now: DateTime<Utc>, offset: i32) -> DateTime<Utc> {
    let tz = schedule::offset(offset);
    now.with_timezone(&tz)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| d.and_local_timezone(tz).single())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::hours(24))
}

fn app_context(o: &Owner, base: &str) -> AppContext {
    AppContext {
        site_id: o.site.id.clone(),
        site_name: o.site.name.clone(),
        host: o.site.host.clone(),
        sample: o.site.sample,
        account: o.account.clone(),
        owner_name: o.name.clone(),
        utc_offset_minutes: o.site.utc_offset_minutes,
        base: base.to_owned(),
        console: format!("/console/{}", o.site.id),
    }
}

// ---------- Pages ----------

async fn entry(State(state): State<ShellState>, headers: HeaderMap, uri: Uri) -> PageResult {
    let o = owner(&state, &headers, None, &uri).await?;
    Ok(Redirect::to(&base(&o)).into_response())
}

/// `GET /os/{page}`: the same page of the signed-in owner's console, for links that don't
/// know the site (messages, the first release's old routes, other crates' pages): `/os/decisions` goes
/// to `/console/{site}/decisions`. Only plain path segments pass; the query string follows.
async fn entry_page(
    State(state): State<ShellState>,
    Path(page): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let page = page.trim_matches('/');
    let plain = page.len() <= 200
        && page.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        });
    if !page.is_empty() && !plain {
        return Err(PageError::NotFound);
    }
    let o = owner(&state, &headers, None, &uri).await?;
    let mut to = base(&o);
    if !page.is_empty() {
        to = format!("{to}/{page}");
    }
    if let Some(q) = uri.query() {
        to = format!("{to}?{q}");
    }
    Ok(Redirect::to(&to).into_response())
}

#[derive(Debug, Deserialize)]
struct HomeQuery {
    world: Option<String>,
}

/// How long the owner must have been gone before "while you were away" shows.
const AWAY_AFTER: Duration = Duration::hours(1);
/// The most lines a timeline shows.
const TIMELINE_MAX: usize = 12;

async fn home(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    Query(q): Query<HomeQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let offset = o.site.utc_offset_minutes;
    let cards = state.port.cards(&o).await?;
    let mut chrome = chrome(&state, &o, "home", &cards).await?;
    if q.world.as_deref() == Some("front") {
        chrome.world = "front".into();
    }
    let today = state.port.today(&o).await?;
    let work = state.port.work(&o).await?;
    let gauntlet = state.port.gauntlet(&o).await?;
    let waiting = chrome.lane.needs_you.len();

    // While you were away: since the last visit, if it was long enough ago.
    let last = state.store.visit(&o.account, &o.site.id, now).await?;
    let away = match last {
        Some(since) if now - since >= AWAY_AFTER => {
            let all = state.port.happened(&o, since, now).await?;
            let more = all.len().saturating_sub(TIMELINE_MAX);
            Some(Away {
                since_words: format!("Since {}", schedule::when(since, now, offset)),
                spent_words: match state.port.spend_since(&o, since).await {
                    Ok(Some(s)) => s,
                    Ok(None) => "Nothing".into(),
                    Err(_) => "Not available".into(),
                },
                waiting: u32::try_from(waiting).unwrap_or(u32::MAX),
                items: all
                    .iter()
                    .rev()
                    .take(TIMELINE_MAX)
                    .rev()
                    .map(|h| timeline_item(h, now, offset))
                    .collect(),
                more: u32::try_from(more).unwrap_or(u32::MAX),
            })
        }
        _ => None,
    };

    // Night Time: the last quiet window, as recorded.
    let hours = state.store.quiet_hours(&o.site.id).await?;
    let (from, to) = hours.last_window(now, offset);
    let to = to.min(now);
    let night_items = state.port.happened(&o, from, to).await?;
    let tonight = chrome.time.mode == Mode::Night;
    let night = Night {
        sub_words: format!(
            "Quiet hours {} · {}",
            hours.words(),
            if tonight {
                "tonight so far, as recorded"
            } else {
                "last night, as recorded"
            }
        ),
        items: night_items
            .iter()
            .rev()
            .take(TIMELINE_MAX)
            .rev()
            .map(|h| timeline_item(h, now, offset))
            .collect(),
        spend_words: state
            .port
            .spend_since(&o, from)
            .await
            .ok()
            .flatten()
            .filter(|_| !night_items.is_empty())
            .map(|s| format!("AI spend since {}: {s}", schedule::clock(hours.start))),
        waiting: u32::try_from(waiting).unwrap_or(u32::MAX),
    };

    let front = Front {
        passed: today.passed.clone(),
        passed_total: today.passed_total,
        held: cards
            .iter()
            .filter(|c| c.state == cv::CardState::Waiting && c.kind == "order-held")
            .map(|c| lane_card(c, &o, now))
            .collect(),
    };

    let state_line = if chrome.freeze.is_some() {
        StateLine {
            class: String::new(),
            words: "Frozen: your WebAgents are stopped".into(),
        }
    } else if let Some(w) = work.iter().find(|w| w.state == "working") {
        StateLine {
            class: "working".into(),
            words: w.state_words.clone(),
        }
    } else if waiting > 0 {
        StateLine {
            class: "waiting".into(),
            words: format!("{waiting} waiting for you"),
        }
    } else if let Some(b) = &today.brief {
        StateLine {
            class: "live".into(),
            words: format!("Brief {}", b.written_words),
        }
    } else {
        StateLine::default()
    };

    let mut sub = vec![schedule::long_date(now, offset)];
    sub.push(match waiting {
        0 => "nothing needs you".to_owned(),
        1 => "1 decision waiting".to_owned(),
        n => format!("{n} decisions waiting"),
    });
    // "Allow, and tell me": what standing grants approved in the last two days.
    let live_grants = state.port.grants(&o, None).await?;
    let granted = state
        .port
        .granted(&o, now - Duration::days(2))
        .await?
        .into_iter()
        .map(|r| {
            let grant = live_grants
                .iter()
                .find(|g| Some(&g.id) == r.grant_id.as_ref());
            let at = r
                .decided_at
                .map(|a| schedule::when(a, now, offset))
                .unwrap_or_default();
            GrantedNotice {
                href: format!("{}/decisions/{}", base(&o), r.card.id),
                words: format!(
                    "{} went ahead under your standing grant{} · {at}.",
                    start_upper(&agentrix_decisions::agent_words(&r.card.raised_by.webagent)),
                    grant
                        .map(|g| format!(" for {}", g.terms.words))
                        .unwrap_or_default()
                ),
                receipt: r.receipt.clone().unwrap_or_default(),
                revoke_url: grant.map(|g| format!("/decisions/v2/grants/{}/revoke", g.id)),
                title: r.card.title,
            }
        })
        .collect();
    let view = HomeView {
        granted,
        greeting: schedule::greeting(&o.name, now, offset),
        sub_words: sub.join(" · "),
        state_line,
        first_card: chrome.lane.needs_you.first().cloned(),
        brief: today.brief.clone(),
        first_brief_words: today.first_brief_words.clone(),
        work,
        passed: today.passed.clone(),
        passed_total: today.passed_total,
        meter: state.port.meter(&o).await.unwrap_or(None),
        missions: mission_rows(&state, &o, now)
            .await?
            .into_iter()
            .filter(|m| m.state == "active")
            .collect(),
        gauntlet,
        away,
        night,
        front,
        recent: today.recent.clone(),
        chrome,
    };
    render(&state, ShellPage::Home, &view)
}

async fn decisions(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "decisions", &cards).await?;
    let pick = |s: cv::CardState| {
        cards
            .iter()
            .filter(|c| c.state == s)
            .map(|c| lane_card(c, &o, now))
            .collect::<Vec<_>>()
    };
    let mut decided = pick(cv::CardState::Approved);
    decided.extend(pick(cv::CardState::Declined));
    let view = DecisionsView {
        waiting: pick(cv::CardState::Waiting),
        decided,
        expired: pick(cv::CardState::Expired),
        chrome,
    };
    render(&state, ShellPage::Decisions, &view)
}

async fn decision(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let cards = state.port.cards(&o).await?;
    let row = cards
        .iter()
        .find(|c| c.id == id)
        .ok_or(PageError::NotFound)?;
    let card = state
        .port
        .card(&o, &id, true)
        .await?
        .ok_or(PageError::NotFound)?;
    let chrome = chrome(&state, &o, "decisions", &cards).await?;
    let extras = state
        .port
        .card_extras(&o, &id)
        .await?
        .map(|x| extras_view(x, &id, &o, now));
    let view = DecisionView {
        row: lane_card(row, &o, now),
        card,
        extras,
        chrome,
    };
    render(&state, ShellPage::Decision, &view)
}

pub(crate) fn rule_view(r: &agentrix_decisions::Rule, o: &Owner, now: DateTime<Utc>) -> RuleView {
    RuleView {
        id: r.id.clone(),
        words: r.words.clone(),
        made_words: format!(
            "Made {} when you declined a card",
            schedule::when(r.created_at, now, o.site.utc_offset_minutes)
        ),
        card_href: format!("{}/decisions/{}", base(o), r.from_card),
        receipt: r.receipt.clone(),
        remove_url: format!("/decisions/v2/rules/{}/remove", r.id),
    }
}

/// "The Cashier" from "the Cashier".
fn start_upper(words: &str) -> String {
    let mut c = words.chars();
    c.next()
        .map(|f| format!("{}{}", f.to_uppercase(), c.as_str()))
        .unwrap_or_default()
}

pub(crate) fn grant_view(
    g: &agentrix_decisions::Grant,
    o: &Owner,
    now: DateTime<Utc>,
) -> GrantView {
    let offset = o.site.utc_offset_minutes;
    let used = match g.uses {
        0 => "Not used yet".to_owned(),
        1 => "Used once".to_owned(),
        2 => "Used twice".to_owned(),
        n => format!("Used {n} times"),
    };
    let ended = g.expires_at <= now;
    let agent_words = agentrix_decisions::agent_words(&g.terms.webagent);
    GrantView {
        id: g.id.clone(),
        sentence: g.terms.sentence(),
        who: start_upper(&agent_words),
        agent_words,
        state_words: if ended {
            format!(
                "{used} · ended {}",
                schedule::when(g.expires_at, now, offset)
            )
        } else {
            format!(
                "{used} · until {}",
                schedule::when(g.expires_at, now, offset)
            )
        },
        card_href: format!("{}/decisions/{}", base(o), g.from_card),
        receipt: g.receipt.clone(),
        revoke_url: format!("/decisions/v2/grants/{}/revoke", g.id),
        ended,
    }
}

fn extras_view(
    x: crate::port::CardExtras,
    id: &str,
    o: &Owner,
    now: DateTime<Utc>,
) -> CardExtrasView {
    use agentrix_decisions::EditKind;
    let fields = x
        .editable
        .iter()
        .map(|(f, value, words)| FieldView {
            name: format!("edit.{}", f.key),
            label: f.label.clone(),
            kind: match f.kind {
                EditKind::Text => "text",
                EditKind::Date => "date",
                EditKind::Time => "time",
                EditKind::AmountDown => "amount",
            }
            .to_owned(),
            multiline: f.kind == EditKind::Text && value.chars().count() > 60,
            hint: match f.kind {
                EditKind::AmountDown => format!("{words} now. You can lower it, not raise it."),
                EditKind::Text => "At most 2,000 characters.".into(),
                _ => String::new(),
            },
            value: value.clone(),
        })
        .collect();
    CardExtrasView {
        agent_words: agentrix_decisions::agent_words(&x.webagent),
        fields,
        sheet_url: format!("/decisions/v2/{id}/sheet"),
        signed: x.edit.as_ref().map(|e| e.after.to_string()),
        changes: x
            .edit
            .map(|e| {
                e.changes
                    .into_iter()
                    .map(|c| ChangeView {
                        label: c.label,
                        before: c.before,
                        after: c.after,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        allow_person: x.allow_person,
        allow_kind: x.allow_kind,
        allow_refusal: x.allow_refusal,
        grant: x.grant.as_ref().map(|g| grant_view(g, o, now)),
        by_grant: x.by_grant,
        reason_is_rule: x.rules.iter().any(|r| r.from_card == id),
        decline_reason: x.decline_reason,
        rules: x.rules.iter().map(|r| rule_view(r, o, now)).collect(),
        agent_href: format!("{}/webagents/{}", base(o), x.webagent),
    }
}

/// JSON errors in plain words: `{message}`.
fn message(status: StatusCode, words: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "message": words })),
    )
        .into_response()
}

async fn sheet(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let o = match owner(&state, &headers, Some(&site), &uri).await {
        Ok(o) => o,
        Err(PageError::Go(_)) => {
            return message(StatusCode::UNAUTHORIZED, "Sign in again, then approve.");
        }
        Err(PageError::NotFound) => {
            return message(StatusCode::NOT_FOUND, "We couldn't find that card.");
        }
        Err(e) => return e.into_response(),
    };
    match state.port.card(&o, &id, true).await {
        Ok(Some(card)) if card.state == cv::CardState::Waiting => {
            let sheet = Sheet {
                sign_words: if card.effects.is_empty() {
                    card.title.clone()
                } else {
                    card.effects.join(" · ")
                },
                title: card.title,
                fingerprint: card.fingerprint,
                fingerprint_short: card.fingerprint_short,
                approve_url: card.approve_url,
                has_passkey: card.has_passkey,
                passkey_options: card.passkey_options,
            };
            ([(header::CACHE_CONTROL, "no-store")], Json(sheet)).into_response()
        }
        Ok(Some(_)) => message(StatusCode::CONFLICT, "This card was already decided."),
        Ok(None) => message(StatusCode::NOT_FOUND, "We couldn't find that card."),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "approval sheet failed");
            message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The approval sheet didn't load. Nothing was changed; try again.",
            )
        }
    }
}

async fn webagents(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "webagents", &cards).await?;
    let (members, available, agents) = match state.port.roster(&o).await? {
        Some(r) => {
            let view =
                |m: &agentrix_kernel::Member| member_view(m, &cards, &o, chrome.freeze.is_some());
            let members = r
                .on_duty()
                .filter(|m| m.spec.id != "coordinator")
                .map(view)
                .collect();
            let available = r
                .members
                .iter()
                .filter(|m| m.status != agentrix_kernel::Status::OnDuty)
                .map(view)
                .collect();
            (members, available, Vec::new())
        }
        None => (
            Vec::new(),
            Vec::new(),
            state.port.webagents(&o).await?.agents,
        ),
    };
    render(
        &state,
        ShellPage::WebAgents,
        &WebAgentsView {
            chrome,
            agents,
            members,
            available,
        },
    )
}

async fn webagent(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "webagents", &cards).await?;
    let roster = state.port.roster(&o).await?.ok_or(PageError::NotFound)?;
    let m = roster
        .get(&id)
        .filter(|m| m.spec.id != "coordinator")
        .ok_or(PageError::NotFound)?;
    let member = member_view(m, &cards, &o, chrome.freeze.is_some());
    let now = Utc::now();
    let rules = state
        .port
        .rules(&o, Some(&m.spec.id))
        .await?
        .iter()
        .map(|r| rule_view(r, &o, now))
        .collect();
    let grants = state
        .port
        .grants(&o, Some(&m.spec.id))
        .await?
        .iter()
        .map(|g| grant_view(g, &o, now))
        .collect();
    let stays_words = (member.on_duty)
        .then(|| crate::store_app::stays(&m.spec.id))
        .flatten()
        .map(str::to_owned);
    let remove = (member.on_duty && stays_words.is_none()).then(|| RemoveView {
        url: format!("{}/webagents/{}/remove", base(&o), m.spec.id),
        effects: crate::store_app::remove_effects(&m.spec, &o.site.name),
        waiting_href: waiting_team_card(
            &cards,
            crate::store_app::REMOVE_TOOL,
            &remove_title(&m.spec),
        )
        .map(|c| c.href.clone()),
    });
    let view = WebAgentView {
        on_duty: member.on_duty,
        member,
        rules,
        grants,
        remove,
        stays_words,
        chrome,
    };
    render(&state, ShellPage::WebAgent, &view)
}

fn remove_title(spec: &agentrix_kernel::WebAgentSpec) -> String {
    format!("Remove {} from your team", spec.name)
}

/// A card that changes the team (`kind`, `title`) and still waits for the owner.
fn waiting_team_card<'a>(cards: &'a [CardRow], kind: &str, title: &str) -> Option<&'a CardRow> {
    cards
        .iter()
        .find(|c| c.kind == kind && c.title == title && c.state == cv::CardState::Waiting)
}

/// A dedupe key for a team change: the same while its card waits, and new once that card is
/// decided, so a WebAgent the owner removed can be added again (and the other way round).
fn team_change_key(cards: &[CardRow], kind: &str, title: &str, verb: &str, id: &str) -> String {
    let decided = cards
        .iter()
        .filter(|c| c.kind == kind && c.title == title && c.state != cv::CardState::Waiting)
        .count();
    format!("{verb}:{id}:{decided}")
}

/// "Remove {name}": raises a card the owner approves; the WebAgent leaves only after that.
async fn webagent_remove(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let back = format!("{}/webagents/{id}", base(&o));
    if let Some(why) = crate::store_app::stays(&id) {
        return Ok(form_error(StatusCode::CONFLICT, why, &back));
    }
    let Some(roster) = state.port.roster(&o).await? else {
        return Ok(form_error(
            StatusCode::CONFLICT,
            "Your site has no WebAgents yet. They start when your site goes live.",
            &back,
        ));
    };
    let Some(m) = roster
        .members
        .iter()
        .find(|m| m.spec.id == id && m.status == agentrix_kernel::Status::OnDuty)
    else {
        return Ok(form_error(
            StatusCode::NOT_FOUND,
            "That WebAgent isn't on your team, so there's nothing to remove.",
            &format!("{}/webagents", base(&o)),
        ));
    };
    let s = &m.spec;
    let title = remove_title(s);
    let cards = state.port.cards(&o).await?;
    if let Some(c) = waiting_team_card(&cards, crate::store_app::REMOVE_TOOL, &title) {
        return Ok(Redirect::to(&c.href).into_response());
    }
    let card = agentrix_decisions::NewCard {
        kind: crate::store_app::REMOVE_TOOL.into(),
        title: title.clone(),
        summary: s.role_words.clone(),
        effects: crate::store_app::remove_effects(s, &o.site.name),
        effect: agentrix_types::EffectRequest {
            site: SiteId::new(o.site.id.clone()),
            webagent: "chief".into(),
            tool: crate::store_app::REMOVE_TOOL.into(),
            args: json!({ "webagent": s.id }),
            amount: None,
            expiry: Utc::now() + Duration::days(7),
        },
        raised_by: agentrix_decisions::RaisedBy {
            webagent: "chief".into(),
            job: None,
        },
        risk: agentrix_decisions::Risk {
            level: agentrix_decisions::RiskLevel::Low,
            reasons: vec![
                format!("{} stops working, and nothing it did is undone", s.name),
                "You can add it back from the Store".into(),
            ],
        },
        approval: agentrix_decisions::ApprovalKind::Reviewed,
        back: Some(back),
        dedupe_key: Some(team_change_key(
            &cards,
            crate::store_app::REMOVE_TOOL,
            &title,
            "remove",
            &s.id,
        )),
    };
    let card_id = state.port.raise(&o, card).await?;
    Ok(Redirect::to(&format!("{}/decisions/{card_id}", base(&o))).into_response())
}

fn ladder_key(l: agentrix_types::Level) -> &'static str {
    use agentrix_types::Level;
    match l {
        Level::Watch => "watch",
        Level::Suggest => "suggest",
        Level::AskFirst => "ask",
        Level::DoAndTell => "tell",
        Level::WithinBudget => "budget",
        Level::UndoOnly => "undo",
    }
}

fn member_view(
    m: &agentrix_kernel::Member,
    cards: &[CardRow],
    o: &Owner,
    frozen: bool,
) -> MemberView {
    let s = &m.spec;
    let asks = cards
        .iter()
        .filter(|c| {
            c.state == cv::CardState::Waiting
                && (c.asked_by.name.eq_ignore_ascii_case(&s.name)
                    || c.asked_by.name.eq_ignore_ascii_case(&s.id))
        })
        .count();
    let state = if m.status != agentrix_kernel::Status::OnDuty {
        "resting"
    } else if frozen {
        "paused"
    } else if asks > 0 {
        "waiting"
    } else {
        "idle"
    };
    MemberView {
        id: s.id.clone(),
        on_duty: m.status == agentrix_kernel::Status::OnDuty,
        name: s.name.clone(),
        role_words: s.role_words.clone(),
        mandate: s.mandate.clone(),
        never: s.never.clone(),
        status_words: m.status.words().to_owned(),
        agent: serde_json::json!({ "id": s.id, "name": s.name, "spec": s.mark, "state": state, "owner": o.name }),
        dial: match s.dial {
            agentrix_kernel::DialKind::Ladder => "ladder",
            agentrix_kernel::DialKind::FixedRules => "fixed-rules",
            agentrix_kernel::DialKind::UndoOnly => "undo-only",
        }
        .to_owned(),
        jobs: s
            .jobs
            .iter()
            .map(|j| JobView {
                id: j.id.clone(),
                name: j.name.clone(),
                note: j.note.clone(),
                current: ladder_key(j.start).to_owned(),
                ceiling: ladder_key(j.ceiling).to_owned(),
                ceiling_reason: j.ceiling_reason.clone(),
                level_words: crate::map::level_name(j.start).to_owned(),
                can: j
                    .can
                    .iter()
                    .map(|c| CanView {
                        words: c.words.clone(),
                        tag: c.effect.tag().to_owned(),
                    })
                    .collect(),
            })
            .collect(),
        wakes: s.wakes.iter().map(|w| w.words.clone()).collect(),
        share_percent: s.budget.share_percent,
        asks: u32::try_from(asks).unwrap_or(u32::MAX),
    }
}

async fn trust(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "trust", &cards).await?;
    let t = state.port.trust(&o).await?;
    let left_behind = state
        .port
        .quarantine(&o)
        .await?
        .items
        .iter()
        .filter(|i| i.state != cv::QuarantineState::Restored)
        .count();
    let now = Utc::now();
    let grants = state
        .port
        .grants(&o, None)
        .await?
        .iter()
        .map(|g| grant_view(g, &o, now))
        .collect();
    let view = TrustView {
        grants,
        gauntlet: t.gauntlet,
        receipts: t.receipts,
        key_id: t.key_id,
        left_behind: u32::try_from(left_behind).unwrap_or(u32::MAX),
        chrome,
    };
    render(&state, ShellPage::Trust, &view)
}

async fn website(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let offset = o.site.utc_offset_minutes;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "website", &cards).await?;
    let releases = state.port.releases(&o).await?;
    let today = state.port.today(&o).await?;
    let versions = releases
        .iter()
        .map(|r| {
            let how = match r.method.as_deref() {
                Some("passkey") => "Approved by you with your passkey",
                Some("totp") => "Approved by you with an authenticator code",
                _ if r.state == "candidate" => "Not signed yet",
                _ => "Signed by the platform",
            };
            let when = r
                .signed_at
                .map(|t| format!(" · {}", schedule::when(t, now, offset)))
                .unwrap_or_default();
            let undone = if r.state == "rolled-back" {
                " · rolled back"
            } else {
                ""
            };
            VersionView {
                name: format!("v{}", r.seq),
                words: format!("{how}{when}{undone}"),
                live: r.live,
                state: r.state.clone(),
                href: r
                    .decision
                    .as_ref()
                    .map(|d| format!("{}/decisions/{d}", base(&o))),
            }
        })
        .collect();
    let live = releases.iter().find(|r| r.live);
    let can_roll_back = live.is_some_and(|l| {
        releases
            .iter()
            .any(|r| r.seq < l.seq && r.signed_at.is_some() && r.state != "candidate")
    });
    // The first release's undo goes back to the old site for 30 days after the first release went live.
    let first = releases.iter().filter_map(|r| r.signed_at).min();
    let old_site_words = first.filter(|t| now - *t < Duration::days(30)).map(|t| {
        format!(
            "Until {}",
            schedule::when(t + Duration::days(30), now, offset)
        )
    });
    let view = WebsiteView {
        versions,
        can_roll_back,
        old_site_words,
        rollback_url: format!("/sites/{}/rollback", o.site.id),
        passed: today.passed,
        passed_total: today.passed_total,
        chrome,
    };
    render(&state, ShellPage::Website, &view)
}

fn mission_row(
    m: &crate::missions::Mission,
    count: Option<i64>,
    names: &std::collections::HashMap<String, String>,
    o: &Owner,
    now: DateTime<Utc>,
) -> MissionRow {
    let offset = o.site.utc_offset_minutes;
    let day = |t: DateTime<Utc>| {
        t.with_timezone(&schedule::offset(offset))
            .format("%a %-d %b")
            .to_string()
    };
    let name = |id: &str| {
        names
            .get(id)
            .cloned()
            .unwrap_or_else(|| crate::map::display_name(id))
    };
    let pct = match (count, m.target) {
        (Some(c), t) if t > 0 => (c * 100 / t).clamp(0, 100),
        _ => 0,
    };
    // A mission past its end date is over, whatever its stored state: it shows as ended, with
    // the count at its end and no forecast. A paused one shows its count and no forecast.
    let over = m.over_at(now);
    let state = if over.is_some() {
        "ended".to_owned()
    } else {
        m.state.clone()
    };
    let so_far = |c: i64| {
        if m.target > 0 {
            format!("{c} of {}", m.target)
        } else {
            c.to_string()
        }
    };
    let reached = |c: i64| m.target > 0 && c >= m.target;
    let pace_words = match (state.as_str(), count, m.started_at) {
        ("proposed", ..) => "Starts when you approve its contract.".to_owned(),
        ("ended", None, _) => format!(
            "Ended {}. Nothing was counted for it.",
            day(over.unwrap_or(m.ends_at))
        ),
        ("ended", Some(c), _) => format!(
            "Ended {} with {}.{}",
            day(over.unwrap_or(m.ends_at)),
            so_far(c),
            if reached(c) {
                " It reached its goal."
            } else {
                ""
            }
        ),
        ("paused", None, _) => "Paused. Nothing is counted for this mission yet.".to_owned(),
        ("paused", Some(c), _) => format!(
            "Paused at {}.{}",
            so_far(c),
            if reached(c) {
                " It reached its goal."
            } else {
                " There's no forecast while it's paused."
            }
        ),
        (_, None, _) => "Nothing is counted for this mission yet.".to_owned(),
        (_, Some(c), _) if reached(c) => format!("Reached: {}.", so_far(c)),
        (_, Some(c), Some(start)) if now - start >= Duration::days(1) => {
            let elapsed = (now - start).num_hours().max(1) as f64;
            let left = (m.ends_at - now).num_hours().max(0) as f64;
            let forecast = c as f64 + (c as f64 / elapsed) * left;
            format!(
                "At this pace, about {} by {}.",
                forecast.round() as i64,
                day(m.ends_at)
            )
        }
        _ => "No pace to show. It shows from the first full day until the end date.".to_owned(),
    };
    MissionRow {
        id: m.id.clone(),
        goal: m.goal.clone(),
        state_words: match state.as_str() {
            "proposed" => "Waiting for your approval",
            "active" => "Running",
            "paused" => "Paused",
            _ => "Ended",
        }
        .to_owned(),
        state,
        measure_words: m.measure.words().to_owned(),
        count,
        target: m.target,
        pct,
        pace_words,
        members: m.members.iter().map(|id| name(id)).collect(),
        lead: name(&m.lead),
        ends_words: day(m.ends_at),
        left_words: left_words(m, now),
        since_words: m
            .started_at
            .map(|t| format!("since {}", day(t)))
            .unwrap_or_default(),
        href: format!("{}/missions/{}", base(o), m.id),
        card_href: m
            .card_id
            .as_ref()
            .filter(|_| m.state == "proposed")
            .map(|c| format!("{}/decisions/{c}", base(o))),
    }
}

fn left_words(m: &crate::missions::Mission, now: DateTime<Utc>) -> String {
    if m.over_at(now).is_some() {
        return "Ended".into();
    }
    let days = (m.ends_at - now).num_days();
    match days {
        d if d < 0 => "Past its end date".into(),
        0 => "Ends today".into(),
        1 => "1 day left".into(),
        d if d < 14 => format!("{d} days left"),
        d => format!("{} weeks left", d / 7),
    }
}

async fn mission_rows(
    state: &ShellState,
    o: &Owner,
    now: DateTime<Utc>,
) -> Result<Vec<MissionRow>, PageError> {
    let missions = state.store.missions();
    let list = missions.list(&o.site.id).await?;
    if list.is_empty() {
        return Ok(Vec::new());
    }
    let names: std::collections::HashMap<String, String> = state
        .port
        .roster(o)
        .await?
        .map(|r| {
            r.members
                .iter()
                .map(|m| (m.spec.id.clone(), m.spec.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let mut rows = Vec::new();
    for m in &list {
        let count = missions.count(m, &o.site.host).await?;
        rows.push(mission_row(m, count, &names, o, now));
    }
    Ok(rows)
}

async fn missions(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    missions_page(&state, &o, None).await
}

/// A mission ends 30 days from now unless the owner says otherwise, `YYYY-MM-DD` on the site's
/// calendar.
fn default_end(o: &Owner, now: DateTime<Utc>) -> String {
    (now + Duration::days(30))
        .with_timezone(&schedule::offset(o.site.utc_offset_minutes))
        .format("%Y-%m-%d")
        .to_string()
}

/// The Missions page: the form's defaults, or a draft for the owner to check.
async fn missions_page(
    state: &ShellState,
    o: &Owner,
    draft: Option<(ContractForm, DraftNote)>,
) -> PageResult {
    let now = Utc::now();
    let cards = state.port.cards(o).await?;
    let chrome = chrome(state, o, "missions", &cards).await?;
    let mut workers: Vec<(String, String)> = match state.port.roster(o).await? {
        Some(r) => r
            .on_duty()
            .filter(|m| m.spec.id != "coordinator")
            .map(|m| (m.spec.id.clone(), m.spec.name.clone()))
            .collect(),
        None => Vec::new(),
    };
    let counts_orders = state.store.missions().takes_orders(&o.site.host).await?;
    let default_end = default_end(o, now);
    let (contract, checking) = match draft {
        Some((form, note)) => {
            // The first one ticked leads, so the draft's lead goes first.
            workers.sort_by_key(|(id, _)| {
                form.members
                    .iter()
                    .position(|m| m == id)
                    .unwrap_or(usize::MAX)
            });
            (form, Some(note))
        }
        None => (
            ContractForm {
                said: String::new(),
                drafted: false,
                goal: String::new(),
                measure: if counts_orders { "orders" } else { "none" }.into(),
                target: None,
                members: workers.first().map(|w| w.0.clone()).into_iter().collect(),
                ends: default_end.clone(),
            },
            None,
        ),
    };
    let view = MissionsView {
        missions: mission_rows(state, o, now).await?,
        workers,
        counts_orders,
        default_end,
        contract,
        checking,
        chrome,
    };
    render(state, ShellPage::Missions, &view)
}

/// "Draft the contract": the Chief WebAgent drafts it from the owner's words through the Model
/// Gateway, and the page shows the draft in the form for the owner to check and change. Nothing
/// is stored and nothing starts. When the Chief's dial refuses, no model is wired or the model
/// doesn't answer, the form holds the owner's words and says so.
async fn mission_draft(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let back = format!("{}/missions", base(&o));
    let said = form(&body)
        .get("said")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    if said.is_empty() || said.chars().count() > 300 {
        return Ok(form_error(
            StatusCode::BAD_REQUEST,
            "Say the goal in a sentence or two, up to 300 characters.",
            &back,
        ));
    }
    let Some(roster) = state.port.roster(&o).await? else {
        return Ok(form_error(
            StatusCode::CONFLICT,
            "Your WebAgents start when your site goes live; missions come after that.",
            &back,
        ));
    };
    let now = Utc::now();
    let ask = crate::missions::DraftAsk {
        said: said.clone(),
        site_name: o.site.name.clone(),
        today: now
            .with_timezone(&schedule::offset(o.site.utc_offset_minutes))
            .format("%Y-%m-%d")
            .to_string(),
        default_end: default_end(&o, now),
        counts_orders: state.store.missions().takes_orders(&o.site.host).await?,
        team: roster
            .on_duty()
            .filter(|m| m.spec.id != "coordinator")
            .map(|m| crate::missions::TeamMember {
                id: m.spec.id.clone(),
                name: m.spec.name.clone(),
                role: m.spec.role_words.clone(),
            })
            .collect(),
    };
    // The Chief drafts under its own dial for answering the owner, as it answers questions.
    let ruling = agentrix_kernel::policy::check(
        &roster,
        &agentrix_kernel::Action::new(
            "coordinator",
            "owner-questions",
            agentrix_kernel::Effect::Read,
        ),
    );
    let (raw, words) = if ruling.verdict != agentrix_kernel::Verdict::Act {
        (
            None,
            format!(
                "Your Chief WebAgent didn't draft this. {} Fill in the parts below yourself.",
                ruling.reason
            ),
        )
    } else {
        match state.port.draft_mission(&o, &ask).await {
            Ok(Some(raw)) => (
                Some(raw),
                "Your Chief WebAgent (AI) drafted this from your words. Check each part and change anything that's wrong.".to_owned(),
            ),
            Ok(None) => (
                None,
                "Drafting from your words isn't available on this console, so fill in the parts below yourself.".to_owned(),
            ),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), site = %o.site.id, "mission draft: the Chief couldn't draft");
                (
                    None,
                    "Your Chief WebAgent couldn't draft this just now, so fill in the parts below yourself. Your words are in the goal.".to_owned(),
                )
            }
        }
    };
    let drafted = raw.is_some();
    let d = crate::missions::settle(raw.unwrap_or_default(), &ask);
    let counts_orders = ask.counts_orders;
    let form = ContractForm {
        said,
        drafted,
        goal: d.goal,
        measure: if drafted {
            d.measure.as_str()
        } else if counts_orders {
            "orders"
        } else {
            "none"
        }
        .into(),
        target: d.target,
        members: d.members,
        ends: d.ends,
    };
    let note = DraftNote {
        drafted,
        words,
        question: d.question,
    };
    missions_page(&state, &o, Some((form, note))).await
}

async fn mission(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "missions", &cards).await?;
    let mission = mission_rows(&state, &o, now)
        .await?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or(PageError::NotFound)?;
    render(&state, ShellPage::Mission, &MissionView { chrome, mission })
}

/// "Start the mission": the contract becomes a card v2; the mission starts when it's approved.
async fn mission_start(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let back = format!("{}/missions", base(&o));
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(&body).into_owned().collect();
    let one = |k: &str| {
        pairs
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.trim().to_owned())
            .unwrap_or_default()
    };
    let goal = one("goal");
    if goal.is_empty() || goal.chars().count() > 200 {
        return Ok(form_error(
            StatusCode::BAD_REQUEST,
            "Say the goal in a sentence, up to 200 characters.",
            &back,
        ));
    }
    let measure = crate::missions::Measure::parse(&one("measure"));
    let target: i64 = one("target").parse().unwrap_or(0);
    if measure != crate::missions::Measure::None && !(1..=100_000).contains(&target) {
        return Ok(form_error(
            StatusCode::BAD_REQUEST,
            "Give a number to reach, such as 60.",
            &back,
        ));
    }
    let Some(roster) = state.port.roster(&o).await? else {
        return Ok(form_error(
            StatusCode::CONFLICT,
            "Your WebAgents start when your site goes live; missions come after that.",
            &back,
        ));
    };
    let on_duty: Vec<String> = roster
        .on_duty()
        .filter(|m| m.spec.id != "coordinator")
        .map(|m| m.spec.id.clone())
        .collect();
    let members: Vec<String> = pairs
        .iter()
        .filter(|(n, v)| n == "member" && on_duty.contains(v))
        .map(|(_, v)| v.clone())
        .collect();
    if members.is_empty() {
        return Ok(form_error(
            StatusCode::BAD_REQUEST,
            "Choose at least one WebAgent to work on it.",
            &back,
        ));
    }
    let tz = schedule::offset(o.site.utc_offset_minutes);
    let Some(ends_at) = chrono::NaiveDate::parse_from_str(&one("ends"), "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(23, 59, 0))
        .and_then(|d| d.and_local_timezone(tz).single())
        .map(|d| d.with_timezone(&Utc))
        .filter(|d| *d > Utc::now())
    else {
        return Ok(form_error(
            StatusCode::BAD_REQUEST,
            "Choose an end date after today.",
            &back,
        ));
    };
    let names: std::collections::HashMap<String, String> = roster
        .members
        .iter()
        .map(|m| (m.spec.id.clone(), m.spec.name.clone()))
        .collect();
    let who: Vec<String> = members
        .iter()
        .map(|id| names.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect();
    let missions = state.store.missions();
    let m = missions
        .propose(
            &o.site.id,
            &goal,
            measure,
            if measure == crate::missions::Measure::None {
                0
            } else {
                target
            },
            &members,
            &members[0],
            ends_at,
        )
        .await?;
    let ends_words = ends_at.with_timezone(&tz).format("%a %-d %b").to_string();
    // Who wrote the terms: the Chief WebAgent's draft from the owner's words (checked by the
    // owner on the form), or the owner alone. The card says which.
    let said: String = one("said").chars().take(300).collect();
    let drafted = one("drafted") == "chief" && !said.is_empty();
    let mut effects = Vec::new();
    if drafted {
        effects.push(format!(
            "Drafted by your Chief WebAgent (AI) from your words: “{said}”"
        ));
    }
    effects.extend([
        format!("Goal: {goal}"),
        if measure == crate::missions::Measure::None {
            format!("Counts: {}", measure.words())
        } else {
            format!("Counts: {}, aiming for {target}", measure.words())
        },
        format!(
            "Who: {} (lead), with {}",
            who[0],
            if who.len() > 1 {
                who[1..].join(", ")
            } else {
                "nobody else".into()
            }
        ),
        format!("Ends: {ends_words}, or when you end it"),
        "Authority: each WebAgent keeps its own levels. A mission can't raise any".into(),
    ]);
    let mut args = json!({ "mission": m.id, "goal": goal, "measure": measure, "target": m.target, "members": members, "ends": ends_at });
    args["drafted_by"] = json!(if drafted { "chief" } else { "owner" });
    if drafted {
        args["said"] = json!(said);
    }
    let card = agentrix_decisions::NewCard {
        kind: crate::missions::TOOL.into(),
        title: format!("Start the mission: {goal}"),
        summary: format!(
            "{} · ends {ends_words} · {}",
            who.join(", "),
            if drafted {
                "drafted by your Chief WebAgent"
            } else {
                "written by you"
            }
        ),
        effects,
        effect: agentrix_types::EffectRequest {
            site: SiteId::new(o.site.id.clone()),
            webagent: "chief".into(),
            tool: crate::missions::TOOL.into(),
            args,
            amount: None,
            expiry: Utc::now() + Duration::days(7),
        },
        // The Chief routes every card the owner starts from the console (as the Store's "Add"
        // does); the summary and `drafted_by` say who wrote the terms.
        raised_by: agentrix_decisions::RaisedBy {
            webagent: "chief".into(),
            job: drafted.then(|| "owner-questions".into()),
        },
        risk: agentrix_decisions::Risk {
            level: agentrix_decisions::RiskLevel::Low,
            reasons: vec![
                "A mission changes no one's permissions".into(),
                "You can pause or end it at any time".into(),
            ],
        },
        approval: agentrix_decisions::ApprovalKind::Reviewed,
        back: Some(format!("{}/missions/{}", base(&o), m.id)),
        dedupe_key: Some(format!("mission:{}", m.id)),
    };
    let card_id = state.port.raise(&o, card).await?;
    missions.set_card(&m.id, &card_id).await?;
    Ok(Redirect::to(&format!("{}/decisions/{card_id}", base(&o))).into_response())
}

/// Pause, resume or end: these only stop or restart work, so they need no signature.
async fn mission_state(
    State(state): State<ShellState>,
    Path((site, id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let to = form(&body).get("to").cloned().unwrap_or_default();
    let back = format!("{}/missions/{id}", base(&o));
    if !state
        .store
        .missions()
        .set_state(&o.site.id, &id, &to)
        .await?
    {
        return Ok(form_error(
            StatusCode::CONFLICT,
            "The mission didn't change: it may already be paused, running or ended. Go back to see where it stands.",
            &back,
        ));
    }
    Ok(Redirect::to(&back).into_response())
}

/// The name visitors see when the owner replies: the first name, or the business.
fn person_name(o: &Owner) -> String {
    if o.name.trim().is_empty() {
        o.site.name.clone()
    } else {
        o.name.clone()
    }
}

fn live_row(t: &crate::live::Thread, last_text: String, o: &Owner, now: DateTime<Utc>) -> LiveRow {
    let live = now - t.last_at < Duration::minutes(crate::live::LIVE_FOR_MINUTES);
    let mins = (now - t.last_at).num_minutes();
    LiveRow {
        id: t.id.clone(),
        href: format!("{}/live/{}", base(o), t.id),
        who: format!("{} on {}", t.visitor, t.page),
        desk: t.desk.clone(),
        live,
        owner: t.owner,
        state_words: match (t.owner, live) {
            (true, _) => "You have it".to_owned(),
            (false, true) => format!("{} is answering", t.desk),
            (false, false) => format!(
                "Quiet since {}",
                schedule::when(t.last_at, now, o.site.utc_offset_minutes)
            ),
        },
        last_words: match mins {
            0 => "just now".to_owned(),
            m if m < 60 => format!("{m} min ago"),
            _ => schedule::when(t.last_at, now, o.site.utc_offset_minutes),
        },
        last_text,
    }
}

fn turn_view(t: &crate::live::Turn, o: &Owner, now: DateTime<Utc>) -> LiveTurnView {
    LiveTurnView {
        seq: t.seq,
        who: t.who.clone(),
        by: match t.who.as_str() {
            "owner" => format!("You · {}", t.label()),
            "note" => String::new(),
            _ => t.label(),
        },
        text: t.text.clone(),
        at_words: schedule::when(t.at, now, o.site.utc_offset_minutes),
        cites: t.cites.clone(),
    }
}

async fn live_list(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "live", &cards).await?;
    let live = state.store.live();
    let mut threads = Vec::new();
    for t in live.threads(&o.site.id, now - Duration::hours(24)).await? {
        let last = live
            .turns(&o.site.id, &t.id, 0)
            .await?
            .into_iter()
            .rev()
            .find(|x| x.who == "visitor")
            .map(|x| x.text)
            .unwrap_or_default();
        threads.push(live_row(&t, last, &o, now));
    }
    let live_count = threads.iter().filter(|t| t.live).count();
    render(
        &state,
        ShellPage::Live,
        &LiveView {
            chrome,
            threads,
            live_count,
        },
    )
}

async fn live_thread(
    State(state): State<ShellState>,
    Path((site, thread)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let live = state.store.live();
    live.owner_has_it(&o.site.id, &thread)
        .await
        .map_err(anyhow::Error::from)?;
    let t = live
        .thread(&o.site.id, &thread)
        .await
        .map_err(anyhow::Error::from)?
        .ok_or(PageError::NotFound)?;
    let turns = live
        .turns(&o.site.id, &thread, 0)
        .await
        .map_err(anyhow::Error::from)?;
    let last = turns
        .iter()
        .rev()
        .find(|x| x.who == "visitor")
        .map(|x| x.text.clone())
        .unwrap_or_default();
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "live", &cards).await?;
    let view = LiveThreadView {
        thread: live_row(&t, last, &o, now),
        after: turns.last().map(|x| x.seq).unwrap_or(0),
        turns: turns.iter().map(|x| turn_view(x, &o, now)).collect(),
        owner_name: person_name(&o),
        turns_url: format!("{}/live/{}/turns", base(&o), t.id),
        act_url: format!("{}/live/{}", base(&o), t.id),
        chrome,
    };
    render(&state, ShellPage::LiveThread, &view)
}

#[derive(Deserialize)]
struct AfterQuery {
    #[serde(default)]
    after: i64,
}

/// The turns after `after`, for the open conversation's page to append.
async fn live_turns(
    State(state): State<ShellState>,
    Path((site, thread)): Path<(String, String)>,
    Query(q): Query<AfterQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let now = Utc::now();
    let live = state.store.live();
    let owner_has_it = live
        .owner_has_it(&o.site.id, &thread)
        .await
        .map_err(anyhow::Error::from)?;
    let Some(t) = live
        .thread(&o.site.id, &thread)
        .await
        .map_err(anyhow::Error::from)?
    else {
        return Err(PageError::NotFound);
    };
    let turns = live
        .turns(&o.site.id, &thread, q.after)
        .await
        .map_err(anyhow::Error::from)?;
    let row = live_row(&t, String::new(), &o, now);
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "owner": owner_has_it,
            "state_words": row.state_words,
            "turns": turns.iter().map(|x| turn_view(x, &o, now)).collect::<Vec<_>>(),
        })),
    )
        .into_response())
}

/// Take over, reply, hand back. None needs a signature: the owner speaks as a person, and the
/// visitor sees that they joined.
async fn live_act(
    State(state): State<ShellState>,
    Path((site, thread, act)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let back = format!("{}/live/{thread}", base(&o));
    let live = state.store.live();
    let name = person_name(&o);
    let done = match act.as_str() {
        "take" => live.take_over(&o.site.id, &thread, &name).await,
        "back" => live.hand_back(&o.site.id, &thread).await,
        "say" => {
            let text = form(&body).get("text").cloned().unwrap_or_default();
            live.owner_said(&o.site.id, &thread, &name, &text)
                .await
                .map(|_| ())
        }
        _ => return Err(PageError::NotFound),
    };
    match done {
        Ok(()) if wants_json(&headers) => Ok(Json(json!({ "ok": true })).into_response()),
        Ok(()) => Ok(Redirect::to(&back).into_response()),
        Err(crate::live::LiveError::NotFound | crate::live::LiveError::BadId) => {
            Err(PageError::NotFound)
        }
        Err(e @ (crate::live::LiveError::NotTaken | crate::live::LiveError::BadText)) => {
            let words = match e {
                crate::live::LiveError::NotTaken => {
                    "Tap Take over first. Until then the desk answers."
                }
                _ => "Write a reply, up to 2,000 characters.",
            };
            if wants_json(&headers) {
                Ok((StatusCode::CONFLICT, Json(json!({ "message": words }))).into_response())
            } else {
                Ok(form_error(StatusCode::CONFLICT, words, &back))
            }
        }
        Err(e) => Err(PageError::Internal(e.into())),
    }
}

/// Walk your site with the Chief WebAgent: the live site beside W11's Talk panel.
async fn walk(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "live", &cards).await?;
    let mut talk_html = None;
    if let Some(app) = state.apps.iter().find(|a| a.spec().id == "talk") {
        let ctx = app_context(&o, &format!("{}/apps/talk", base(&o)));
        match app.page(&ctx, "", "").await {
            Ok(AppPage::Field { html, .. }) => talk_html = Some(html),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %format!("{e:#}"), "walk: the Talk panel didn't load"),
        }
    }
    render(
        &state,
        ShellPage::Walk,
        &WalkView {
            chrome,
            site_url: o.site.live_url.clone(),
            talk_html,
        },
    )
}

/// `/d/{card}`: the short link WhatsApp, email and CLI QR codes carry. After sign-in it opens the
/// card on the owner's site, where the passkey or code approves it.
async fn short_link(
    State(state): State<ShellState>,
    Path(card): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, None, &uri).await?;
    if card.len() > 80
        || !card
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(PageError::NotFound);
    }
    match state.port.card(&o, &card, false).await? {
        Some(_) => Ok(Redirect::to(&format!("{}/decisions/{card}", base(&o))).into_response()),
        None => Err(PageError::NotFound),
    }
}

/// The phone's Apps tab: every WebAgent and app as a home screen, talk and Freeze.
async fn apps_home(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "apps", &cards).await?;
    let talk_href = state
        .apps
        .iter()
        .any(|a| a.spec().id == "talk")
        .then(|| format!("{}/apps/talk", base(&o)));
    render(&state, ShellPage::Apps, &AppsView { chrome, talk_href })
}

async fn gestures(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "settings", &cards).await?;
    render(&state, ShellPage::Gestures, &GesturesView { chrome })
}

async fn memory(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "memory", &cards).await?;
    let members = match state.port.roster(&o).await? {
        Some(r) => r
            .on_duty()
            .filter(|m| m.spec.id != "coordinator")
            .map(|m| member_view(m, &cards, &o, chrome.freeze.is_some()))
            .collect(),
        None => Vec::new(),
    };
    let trust = state.port.trust(&o).await?;
    let receipts_total = u32::try_from(trust.receipts.len()).unwrap_or(u32::MAX);
    let view = MemoryView {
        members,
        recent: trust.receipts.into_iter().take(3).collect(),
        receipts_total,
        chrome,
    };
    render(&state, ShellPage::Memory, &view)
}

#[derive(Debug, Deserialize)]
struct StoreQuery {
    add: Option<String>,
}

async fn store(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    Query(q): Query<StoreQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "store", &cards).await?;
    let listings: Vec<MemberView> = match state.port.roster(&o).await? {
        Some(r) => r
            .members
            .iter()
            .filter(|m| m.status == agentrix_kernel::Status::Available)
            .map(|m| member_view(m, &cards, &o, false))
            .collect(),
        None => Vec::new(),
    };
    let selected = q
        .add
        .and_then(|id| listings.iter().find(|m| m.id == id).cloned());
    let view = StoreView {
        add_url: format!("{}/store/add", base(&o)),
        listings,
        selected,
        chrome,
    };
    render(&state, ShellPage::Store, &view)
}

/// "Add {name}": raises a card the owner approves; the WebAgent starts only after that.
async fn store_add(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let id = form(&body).get("webagent").cloned().unwrap_or_default();
    let back = format!("{}/store", base(&o));
    let Some(roster) = state.port.roster(&o).await? else {
        return Ok(form_error(
            StatusCode::CONFLICT,
            "Your site has no WebAgents yet. They start when your site goes live.",
            &back,
        ));
    };
    let Some(m) = roster
        .members
        .iter()
        .find(|m| m.spec.id == id && m.status == agentrix_kernel::Status::Available)
    else {
        return Ok(form_error(
            StatusCode::NOT_FOUND,
            "That WebAgent isn't in your Store.",
            &back,
        ));
    };
    let s = &m.spec;
    let title = format!("Add {} to your team", s.name);
    let cards = state.port.cards(&o).await?;
    if let Some(c) = waiting_team_card(&cards, crate::store_app::TOOL, &title) {
        return Ok(Redirect::to(&c.href).into_response());
    }
    let mut effects = vec![format!(
        "{} starts working for {}: {}",
        s.name, o.site.name, s.mandate
    )];
    effects.extend(s.jobs.iter().map(|j| {
        format!(
            "\"{}\" starts at {}",
            j.name,
            crate::map::level_name(j.start)
        )
    }));
    if s.budget.share_percent > 0 {
        effects.push(format!(
            "It may use up to {}% of your AI budget",
            s.budget.share_percent
        ));
    }
    effects.push("Anything it can't undo, such as money and messages, asks you first unless you later choose \"Allow, and tell me\" for it".into());
    let card = agentrix_decisions::NewCard {
        kind: crate::store_app::TOOL.into(),
        title: title.clone(),
        summary: s.role_words.clone(),
        effects,
        effect: agentrix_types::EffectRequest {
            site: SiteId::new(o.site.id.clone()),
            webagent: "chief".into(),
            tool: crate::store_app::TOOL.into(),
            args: json!({ "webagent": s.id, "jobs": s.jobs.iter().map(|j| json!({ "job": j.id, "start": j.start })).collect::<Vec<_>>(), "budget_share_percent": s.budget.share_percent }),
            amount: None,
            expiry: Utc::now() + Duration::days(7),
        },
        raised_by: agentrix_decisions::RaisedBy {
            webagent: "chief".into(),
            job: None,
        },
        risk: agentrix_decisions::Risk {
            level: agentrix_decisions::RiskLevel::Medium,
            reasons: vec![
                "A new WebAgent changes who works on your site".into(),
                "It starts at the levels shown and can't raise them itself".into(),
            ],
        },
        approval: agentrix_decisions::ApprovalKind::Reviewed,
        back: Some(format!("{}/webagents#agent-{}", base(&o), s.id)),
        dedupe_key: Some(team_change_key(
            &cards,
            crate::store_app::TOOL,
            &title,
            "add",
            &s.id,
        )),
    };
    let card_id = state.port.raise(&o, card).await?;
    Ok(Redirect::to(&format!("{}/decisions/{card_id}", base(&o))).into_response())
}

async fn quarantine(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "trust", &cards).await?;
    let items = state.port.quarantine(&o).await?.items;
    render(
        &state,
        ShellPage::Quarantine,
        &QuarantineView { chrome, items },
    )
}

#[derive(Debug, Deserialize)]
struct SettingsQuery {
    saved: Option<String>,
}

async fn settings(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    Query(q): Query<SettingsQuery>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let cards = state.port.cards(&o).await?;
    let chrome = chrome(&state, &o, "settings", &cards).await?;
    let saved_words = q
        .saved
        .is_some()
        .then(|| format!("Saved. Night Time now runs {}.", chrome.time.quiet_words));
    render(
        &state,
        ShellPage::Settings,
        &SettingsView {
            chrome,
            saved_words,
            surfaces: agentrix_decisions::surface::TABLE.to_vec(),
            channels_href: state
                .apps
                .iter()
                .any(|a| a.spec().id == "channels")
                .then(|| format!("{}/apps/channels", base(&o))),
        },
    )
}

fn form(body: &[u8]) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

fn wants_json(headers: &HeaderMap) -> bool {
    [header::CONTENT_TYPE, header::ACCEPT].into_iter().any(|h| {
        headers
            .get(h)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("application/json"))
    })
}

/// A plain page that says what went wrong, with a way back.
fn form_error(status: StatusCode, words: &str, back: &str) -> Response {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width, initial-scale=1\"><title>That didn't work · Agentrix</title><link rel=stylesheet href=\"/static/os/styles.css\"><main class=\"wrap section\"><h2>That didn't work</h2><p class=\"lead\">{}</p><p style=\"margin-top:16px\"><a class=\"btn\" href=\"{}\">Go back</a></p></main>",
        esc(words),
        esc(back)
    );
    (status, Html(html)).into_response()
}

async fn set_quiet_hours(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let f = form(&body);
    let back = format!("{}/settings", base(&o));
    let Some(hours) = QuietHours::parse(
        f.get("start").map(String::as_str).unwrap_or(""),
        f.get("end").map(String::as_str).unwrap_or(""),
    ) else {
        return Ok(form_error(
            StatusCode::BAD_REQUEST,
            "Quiet hours need a start and an end time, such as 22:00 and 06:00, and they can't be the same.",
            &back,
        ));
    };
    state
        .store
        .set_quiet_hours(&o.site.id, hours, &o.account, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("{back}?saved=1")).into_response())
}

// ---------- Freeze ----------

async fn freeze(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    if !state.freeze_enabled {
        return Err(PageError::NotFound);
    }
    let how = match form(&body).get("how").map(String::as_str) {
        Some("voice") => "voice",
        Some("gesture") => "gesture",
        _ => "console",
    };
    let f = state
        .store
        .set_frozen(&o.site.id, &o.account, how, Utc::now())
        .await?;
    tracing::info!(site = %o.site.id, freeze = %f.id, how, "site frozen by its owner");
    let back = form(&body)
        .get("back")
        .filter(|b| b.starts_with(&base(&o)))
        .cloned()
        .unwrap_or_else(|| base(&o));
    if wants_json(&headers) {
        return Ok(Json(json!({ "redirect": back })).into_response());
    }
    Ok(Redirect::to(&back).into_response())
}

/// How long an unfreeze sheet stays valid.
const UNFREEZE_WINDOW: Duration = Duration::minutes(10);

async fn unfreeze_sheet(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let o = match owner(&state, &headers, Some(&site), &uri).await {
        Ok(o) => o,
        Err(PageError::Go(_)) => {
            return message(StatusCode::UNAUTHORIZED, "Sign in again, then unfreeze.");
        }
        Err(e) => return e.into_response(),
    };
    let result: anyhow::Result<Response> = async {
        let Some(f) = state.store.freeze(&o.site.id).await? else {
            return Ok(message(StatusCode::CONFLICT, "Your site isn't frozen."));
        };
        let now = Utc::now();
        let effect = EffectRequest {
            site: SiteId::new(o.site.id.clone()),
            webagent: "owner".into(),
            tool: "site.unfreeze".into(),
            args: json!({ "freeze": f.id }),
            amount: None,
            expiry: now + UNFREEZE_WINDOW,
        };
        let fingerprint = effect.fingerprint()?;
        state.store.ask_unfreeze(&f.id, &effect).await?;
        let options = state.port.passkey_options(&o, &fingerprint).await?;
        let fp = fingerprint.to_string();
        let sheet = Sheet {
            title: format!("Unfreeze {}", o.site.name),
            sign_words: format!(
                "Unfreeze {}: your WebAgents start working again",
                o.site.name
            ),
            fingerprint_short: short(&fp),
            fingerprint: fp,
            approve_url: format!("{}/unfreeze", base(&o)),
            has_passkey: o.has_passkey,
            passkey_options: options,
        };
        Ok(([(header::CACHE_CONTROL, "no-store")], Json(sheet)).into_response())
    }
    .await;
    result.unwrap_or_else(|e| {
        tracing::error!(error = %format!("{e:#}"), "unfreeze sheet failed");
        message(
            StatusCode::INTERNAL_SERVER_ERROR,
            "The approval sheet didn't load. Your site stays frozen; try again.",
        )
    })
}

fn short(fp: &str) -> String {
    let hex = fp.strip_prefix(Digest::PREFIX).unwrap_or(fp);
    format!(
        "{}…{}",
        &hex[..hex.len().min(4)],
        &hex[hex.len().saturating_sub(2)..]
    )
}

/// The approval body of the frozen contract: JSON `{method: "passkey", credential}` or
/// `{method: "totp", code, fingerprint}`, or a form with `method=totp`.
fn attempt(headers: &HeaderMap, body: &[u8]) -> Result<(Attempt, Option<String>), &'static str> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"))
    {
        let v: Value =
            serde_json::from_slice(body).map_err(|_| "That approval couldn't be read.")?;
        match v.get("method").and_then(Value::as_str) {
            Some("passkey") => Ok((
                Attempt::Passkey(v.get("credential").cloned().unwrap_or(Value::Null)),
                None,
            )),
            Some("totp") => Ok((
                Attempt::Totp(
                    v.get("code")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                ),
                v.get("fingerprint")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )),
            _ => Err("Approve with your passkey or a code from your authenticator app."),
        }
    } else {
        let f = form(body);
        match f.get("method").map(String::as_str) {
            Some("totp") => Ok((
                Attempt::Totp(f.get("code").cloned().unwrap_or_default()),
                f.get("fingerprint").cloned(),
            )),
            _ => Err("Approve with your passkey or a code from your authenticator app."),
        }
    }
}

async fn unfreeze(
    State(state): State<ShellState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Response {
    let json_reply = wants_json(&headers);
    let o = match owner(&state, &headers, Some(&site), &uri).await {
        Ok(o) => o,
        Err(PageError::Go(_)) if json_reply => {
            return message(StatusCode::UNAUTHORIZED, "Sign in again, then unfreeze.");
        }
        Err(e) => return e.into_response(),
    };
    let back = base(&o);
    let fail = |status: StatusCode, words: &str| {
        if json_reply {
            message(status, words)
        } else {
            form_error(status, words, &back)
        }
    };
    let (attempt, posted) = match attempt(&headers, &body) {
        Ok(a) => a,
        Err(words) => return fail(StatusCode::BAD_REQUEST, words),
    };
    let now = Utc::now();
    let f = match state.store.freeze(&o.site.id).await {
        Ok(Some(f)) => f,
        Ok(None) => return fail(StatusCode::CONFLICT, "Your site isn't frozen."),
        Err(e) => {
            tracing::error!(error = %e, "freeze read failed");
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't finish that on our side. Your site stays frozen. Try again in a minute.",
            );
        }
    };
    let Some(effect) = f.unfreeze_effect.clone() else {
        return fail(
            StatusCode::CONFLICT,
            "Open Unfreeze again to get a fresh approval.",
        );
    };
    if effect.is_expired(now) {
        return fail(
            StatusCode::GONE,
            "That approval expired, so nothing changed. Open Unfreeze again.",
        );
    }
    let Ok(fingerprint) = effect.fingerprint() else {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "We couldn't finish that on our side. Your site stays frozen. Try again in a minute.",
        );
    };
    if matches!(attempt, Attempt::Totp(_)) {
        let ours = fingerprint.to_string();
        let ok = posted
            .as_deref()
            .is_some_and(|p| p == ours || Some(p) == ours.strip_prefix(Digest::PREFIX));
        if !ok {
            return fail(
                StatusCode::CONFLICT,
                "This approval changed since you opened it. Open Unfreeze again.",
            );
        }
    }
    let proof = match state.port.verify(&o, &fingerprint, attempt).await {
        Ok(p) => p,
        Err(words) => return fail(StatusCode::FORBIDDEN, &words),
    };
    let method = serde_json::to_value(proof.method)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut receipt = ReceiptBody::new(
        ReceiptKind::DecisionApproved,
        effect.site.clone(),
        Subject::Site(effect.site.clone()),
        f.at,
        now,
    );
    receipt.detail = Some(json!({
        "kind": "site.unfreeze",
        "tool": effect.tool,
        "freeze": f.id,
        "frozen_at": f.at,
        "fingerprint": fingerprint,
        "method": method,
        "approver": o.account,
        "evidence": proof.evidence,
    }));
    let how = if method == "passkey" {
        "your passkey"
    } else {
        "a code from your authenticator app"
    };
    let digest = match state
        .port
        .issue(receipt, format!("You unfroze {} with {how}", o.site.name))
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "unfreeze receipt failed");
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Your approval was checked, but the receipt couldn't be written, so your site stays frozen. Try again.",
            );
        }
    };
    match state
        .store
        .unfreeze(&f.id, &o.account, &method, &digest.to_string(), now)
        .await
    {
        Ok(true) => {}
        Ok(false) => return fail(StatusCode::CONFLICT, "Your site was already unfrozen."),
        Err(e) => {
            tracing::error!(error = %e, "unfreeze write failed");
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn't finish that on our side. Your site stays frozen. Try again in a minute.",
            );
        }
    }
    tracing::info!(site = %o.site.id, freeze = %f.id, method, "site unfrozen by its owner");
    if json_reply {
        Json(json!({ "redirect": back })).into_response()
    } else {
        Redirect::to(&back).into_response()
    }
}

// ---------- Apps (C9) ----------

fn find_app(state: &ShellState, id: &str) -> Option<Arc<dyn ConsoleApp>> {
    state.apps.iter().find(|a| a.spec().id == id).cloned()
}

async fn app_page(
    state: ShellState,
    site: String,
    app: String,
    path: String,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let a = find_app(&state, &app).ok_or(PageError::NotFound)?;
    let ctx = app_context(&o, &format!("{}/apps/{app}", base(&o)));
    if !a.available(&ctx).await {
        return Err(PageError::NotFound);
    }
    match a
        .page(
            &ctx,
            path.trim_start_matches('/'),
            uri.query().unwrap_or(""),
        )
        .await?
    {
        AppPage::Field { title, html } => {
            let cards = state.port.cards(&o).await?;
            let chrome = chrome(&state, &o, &format!("app:{app}"), &cards).await?;
            render(
                &state,
                ShellPage::App,
                &AppView {
                    chrome,
                    title,
                    html,
                },
            )
        }
        AppPage::Redirect(to) => Ok(Redirect::to(&to).into_response()),
        AppPage::NotFound => Err(PageError::NotFound),
    }
}

async fn app_get(
    State(state): State<ShellState>,
    Path((site, app)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    app_page(state, site, app, String::new(), headers, uri).await
}

async fn app_get_path(
    State(state): State<ShellState>,
    Path((site, app, path)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> PageResult {
    app_page(state, site, app, path, headers, uri).await
}

async fn app_action(
    state: ShellState,
    site: String,
    app: String,
    path: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    let o = owner(&state, &headers, Some(&site), &uri).await?;
    let a = find_app(&state, &app).ok_or(PageError::NotFound)?;
    let ctx = app_context(&o, &format!("{}/apps/{app}", base(&o)));
    if !a.available(&ctx).await {
        return Err(PageError::NotFound);
    }
    Ok(
        match a
            .post(&ctx, path.trim_start_matches('/'), &headers, body)
            .await?
        {
            AppReply::Redirect(to) => Redirect::to(&to).into_response(),
            AppReply::Json(status, v) => (status, Json(v)).into_response(),
            AppReply::Error(status, words) => {
                if wants_json(&headers) {
                    message(status, &words)
                } else {
                    form_error(status, &words, &ctx.base)
                }
            }
            AppReply::NotFound => return Err(PageError::NotFound),
        },
    )
}

async fn app_post(
    State(state): State<ShellState>,
    Path((site, app)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    app_action(state, site, app, String::new(), headers, uri, body).await
}

async fn app_post_path(
    State(state): State<ShellState>,
    Path((site, app, path)): Path<(String, String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> PageResult {
    app_action(state, site, app, path, headers, uri, body).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn mission(state: &str, ends_at: DateTime<Utc>) -> crate::missions::Mission {
        let start = Utc.with_ymd_and_hms(2026, 11, 1, 6, 0, 0).unwrap();
        crate::missions::Mission {
            id: "msn_1".into(),
            site_id: "site_1".into(),
            goal: "Christmas cakes".into(),
            measure: crate::missions::Measure::Orders,
            target: 60,
            members: vec!["sales".into()],
            lead: "sales".into(),
            ends_at,
            state: state.into(),
            card_id: None,
            created_at: start,
            started_at: Some(start),
            ended_at: None,
        }
    }

    fn owner() -> Owner {
        Owner {
            site: crate::port::SiteInfo {
                id: "site_1".into(),
                utc_offset_minutes: 180,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_running_mission_forecasts_and_a_finished_one_shows_its_final_count() {
        let now = Utc.with_ymd_and_hms(2026, 11, 9, 6, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 12, 18, 18, 0, 0).unwrap();
        let names = std::collections::HashMap::new();
        let row =
            |m: &crate::missions::Mission, count| mission_row(m, count, &names, &owner(), now);

        let running = row(&mission("active", end), Some(23));
        assert_eq!(running.state_words, "Running");
        assert!(
            running.pace_words.starts_with("At this pace, about "),
            "{}",
            running.pace_words
        );

        // Past its end date: ended, with the count at its end, and no forecast.
        let past = row(&mission("active", now - Duration::days(2)), Some(41));
        assert_eq!(past.state, "ended");
        assert_eq!(past.state_words, "Ended");
        assert_eq!(past.left_words, "Ended");
        assert_eq!(past.pace_words, "Ended Sat 7 Nov with 41 of 60.");

        // Ended early by the owner: the day they ended it.
        let mut ended = mission("ended", end);
        ended.ended_at = Some(now - Duration::days(1));
        let ended = row(&ended, Some(64));
        assert_eq!(
            ended.pace_words,
            "Ended Sun 8 Nov with 64 of 60. It reached its goal."
        );

        // Paused: its count so far, and no forecast.
        let paused = row(&mission("paused", end), Some(23));
        assert_eq!(paused.state_words, "Paused");
        assert_eq!(
            paused.pace_words,
            "Paused at 23 of 60. There's no forecast while it's paused."
        );
        let paused_past = row(&mission("paused", now - Duration::days(2)), Some(23));
        assert_eq!(paused_past.state_words, "Ended");

        for r in [&past, &paused, &paused_past] {
            assert!(!r.pace_words.contains("At this pace"), "{}", r.pace_words);
        }
    }
}
