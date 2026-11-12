//! Pages and their views, rendered with the agentrix-ui kit so every host looks like Agentrix.

use std::sync::OnceLock;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use minijinja::Environment;
use serde_json::{Value, json};

use crate::feed::{AGENTRIX, Feed, KILELE, money};

/// This crate's templates, as `(name, source)`.
const TEMPLATES: &[(&str, &str)] = &[
    (
        "saas/front.html",
        include_str!("../templates/saas/front.html"),
    ),
    (
        "saas/plans.html",
        include_str!("../templates/saas/plans.html"),
    ),
    (
        "saas/subscribe.html",
        include_str!("../templates/saas/subscribe.html"),
    ),
    (
        "saas/subscribe_return.html",
        include_str!("../templates/saas/subscribe_return.html"),
    ),
    (
        "saas/proofs.html",
        include_str!("../templates/saas/proofs.html"),
    ),
    (
        "saas/judges.html",
        include_str!("../templates/saas/judges.html"),
    ),
    (
        "saas/activity.html",
        include_str!("../templates/saas/activity.html"),
    ),
    (
        "saas/activity_summary.html",
        include_str!("../templates/saas/activity_summary.html"),
    ),
    (
        "saas/apps/base.html",
        include_str!("../templates/saas/apps/base.html"),
    ),
    (
        "saas/apps/passkey.html",
        include_str!("../templates/saas/apps/passkey.html"),
    ),
    (
        "saas/apps/moment.html",
        include_str!("../templates/saas/apps/moment.html"),
    ),
    (
        "saas/apps/badges.html",
        include_str!("../templates/saas/apps/badges.html"),
    ),
    (
        "saas/apps/verify_box.html",
        include_str!("../templates/saas/apps/verify_box.html"),
    ),
    (
        "saas/apps/agents.html",
        include_str!("../templates/saas/apps/agents.html"),
    ),
    (
        "saas/apps/wallet.html",
        include_str!("../templates/saas/apps/wallet.html"),
    ),
    (
        "saas/apps/link_paypal.html",
        include_str!("../templates/saas/apps/link_paypal.html"),
    ),
    (
        "saas/console.html",
        include_str!("../templates/saas/console.html"),
    ),
    (
        "saas/door.html",
        include_str!("../templates/saas/door.html"),
    ),
    (
        "saas/books.html",
        include_str!("../templates/saas/books.html"),
    ),
    (
        "saas/board_missing.html",
        include_str!("../templates/saas/board_missing.html"),
    ),
    (
        "saas/macros.html",
        include_str!("../templates/saas/macros.html"),
    ),
];

/// This crate's static files under `/static/saas/`, as `(name, bytes, content type)`.
const ASSETS: &[(&str, &[u8], &str)] = &[
    (
        "saas.css",
        include_bytes!("../static/saas.css"),
        "text/css; charset=utf-8",
    ),
    (
        "desk.js",
        include_bytes!("../static/desk.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "console.js",
        include_bytes!("../static/console.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "subscribe.js",
        include_bytes!("../static/subscribe.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "ceremony.js",
        include_bytes!("../static/ceremony.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "verify.js",
        include_bytes!("../static/verify.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "judge.js",
        include_bytes!("../static/judge.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "activity.js",
        include_bytes!("../static/activity.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "activity-summary.js",
        include_bytes!("../static/activity-summary.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "activity-summary.css",
        include_bytes!("../static/activity-summary.css"),
        "text/css; charset=utf-8",
    ),
    (
        "paypal-link.js",
        include_bytes!("../static/paypal-link.js"),
        "text/javascript; charset=utf-8",
    ),
];

/// A content version for `?v=` on this crate's static files.
pub fn assets_version() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        for (name, bytes, _) in ASSETS {
            name.hash(&mut h);
            bytes.hash(&mut h);
        }
        format!("{:08x}", h.finish() as u32)
    })
}

fn env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = agentrix_ui::environment();
        for (name, source) in TEMPLATES {
            env.add_template(name, source)
                .unwrap_or_else(|e| panic!("template {name}: {e}"));
        }
        // The living marks are served by agentrix-kernel, which this node does not run: the
        // kit's slots draw plain squircles instead.
        env.add_global("ui_marks", false);
        env.add_global("saas_v", assets_version());
        env
    })
}

/// The CSP every kit page is served with: no inline script or style anywhere.
pub const CSP: &str = "default-src 'self'; img-src 'self' data:; script-src 'self'; style-src 'self'; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// Renders `name` with `ctx` as an HTML response.
pub fn render(name: &str, ctx: Value) -> Response {
    render_status(StatusCode::OK, name, ctx)
}

/// [`render`] with a status.
pub fn render_status(status: StatusCode, name: &str, ctx: Value) -> Response {
    match env().get_template(name).and_then(|t| t.render(ctx)) {
        Ok(html) => (
            status,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CONTENT_SECURITY_POLICY, CSP),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::REFERRER_POLICY, "same-origin"),
            ],
            html,
        )
            .into_response(),
        Err(e) => {
            tracing::error!(template = name, error = %e, "rendering a page");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "This page failed to render.",
            )
                .into_response()
        }
    }
}

/// A template rendered to a string (a fragment another page embeds); `None` if it failed, which
/// is logged.
pub fn render_to_string(name: &str, ctx: Value) -> Option<String> {
    match env().get_template(name).and_then(|t| t.render(ctx)) {
        Ok(html) => Some(html),
        Err(e) => {
            tracing::error!(template = name, error = %e, "rendering a fragment");
            None
        }
    }
}

/// The title of a page rendered as a fragment, carried on the response for the host shell.
#[derive(Debug, Clone)]
pub struct FragmentTitle(pub String);

/// Only the field of a page (its styles, field and scripts blocks), for a host shell that draws
/// its own chrome around it (Agentrix OS's console apps, contract C9). The page's title rides
/// on the response as [`FragmentTitle`].
pub fn render_fragment(status: StatusCode, name: &str, ctx: Value) -> Response {
    let blocks = env().get_template(name).and_then(|t| {
        let mut captured = t.render_captured(&ctx)?;
        captured.with_state_mut(|state| {
            let title = state.render_block("title")?;
            let mut html = String::new();
            for block in ["shell_styles", "field", "scripts"] {
                html.push_str(&state.render_block(block)?);
                html.push('\n');
            }
            Ok((title, html))
        })
    });
    match blocks {
        Ok((title, html)) => {
            let mut r = (
                status,
                [
                    (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                html,
            )
                .into_response();
            r.extensions_mut()
                .insert(FragmentTitle(title.trim().to_owned()));
            r
        }
        Err(e) => {
            tracing::error!(template = name, error = %e, "rendering a fragment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "This page failed to render.",
            )
                .into_response()
        }
    }
}

/// A page with no data of its own.
pub fn render_static(name: &str) -> Response {
    render(name, json!({}))
}

/// `/static/saas/{file}`.
pub async fn asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match ASSETS.iter().find(|(name, _, _)| *name == path) {
        Some((_, bytes, ct)) => (
            [
                (header::CONTENT_TYPE, *ct),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            *bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The plans as the front and plans pages show them.
pub fn plans_view() -> Value {
    let cat = agentrix_billing::catalogue();
    let plans: Vec<Value> = cat
        .plans
        .iter()
        .filter(|p| p.kind == "site")
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "monthly": money(p.price_cents, "USD"),
                "annual": money(p.annual_cents(), "USD"),
                "saving": money(p.annual_saving_cents(), "USD"),
                "includes": p.includes,
                "webagents": p.webagents.join(", "),
                "featured": p.id == "business",
            })
        })
        .collect();
    json!({ "plans": plans, "discount": cat.annual_discount_percent })
}

/// A door's refusal code in plain words, for the console's list of refused requests.
fn refusal_words(code: &str) -> &'static str {
    match code {
        "signature_required" => "An unsigned request was refused",
        "signature_invalid" => "A request whose signature didn't verify was refused",
        "content_digest_required" => "A request without a body digest was refused",
        "mandate_used" => "A cart mandate that had already been used was refused",
        "token_invalid" | "token_refused" => "A request with a bad agent token was refused",
        _ => "A request was refused",
    }
}

fn agent_view(company: &str, a: &crate::feed::Agent) -> Value {
    let color = match a.id.as_str() {
        "chief" => "#0F8A8A",
        "revenue" => "#C2410C",
        "receivables" => "#1D4ED8",
        "payables" => "#7C3AED",
        _ => "#334155",
    };
    // A revoked agent (the node's feed says `revoked`, which outranks frozen) draws as stopped:
    // the marks have no revoked state, and the console says "Revoked" beside it.
    let revoked = a.state == "revoked";
    json!({
        "id": format!("{company}-{}", a.id),
        "role": a.id,
        "name": a.name,
        "glyph": a.name.chars().take(2).collect::<String>(),
        "color": color,
        "state": if revoked { "frozen" } else { a.state.as_str() },
        "revoked": revoked,
        "state_words": a.doing,
        "job": a.job,
    })
}

/// The owner console's view of `company` in `feed`.
pub fn console_view(feed: &Feed, company: &str, may_act: bool, base: &str) -> Value {
    let f = feed.for_company(company);
    let Some(c) = f.company(company) else {
        return json!({});
    };
    let usd = |m: i64| money(m, "USD");
    let cash_usd: i64 = f
        .cash
        .iter()
        .filter(|x| x.currency == "USD")
        .map(|x| x.balance_minor)
        .sum();
    let ar_open: i64 = f.receivables.iter().map(|r| r.open_minor).sum();
    let ar_late: i64 = f
        .receivables
        .iter()
        .filter(|r| r.days_overdue > 0)
        .map(|r| r.open_minor)
        .sum();
    // A bill on an instalment plan is due as its plan says, not all at once.
    let week_end = feed.as_of + chrono::Days::new(7);
    let ap_week: i64 = f
        .payables
        .iter()
        .filter(|p| p.open_minor > 0)
        .map(|p| {
            match feed.debt_plans.iter().find(|d| {
                d.debtor == company && d.invoice_number == p.number && p.status == "on plan"
            }) {
                Some(plan) => plan
                    .instalments
                    .iter()
                    .filter(|i| i.status != "paid" && i.due <= week_end)
                    .map(|i| i.amount_minor)
                    .sum(),
                None if p.days_to_due <= 7 => p.open_minor,
                None => 0,
            }
        })
        .sum();
    let mut kpis = vec![
        json!({"label": "Cash on PayPal", "value": usd(cash_usd), "window": "now", "source": "ERA · PayPal clearing"}),
        json!({"label": "Owed to us", "value": usd(ar_open), "window": format!("{} past due", usd(ar_late)), "source": "ERA ageing"}),
        json!({"label": "Bills due", "value": usd(ap_week), "window": "next 7 days", "source": "ERA bills"}),
    ];
    if let Some(m) = f.mrr.last() {
        kpis.push(json!({"label": "MRR", "value": usd(m.mrr_minor), "window": format!("{} customers", m.customers), "source": "PayPal Subscriptions"}));
    } else if let Some(p) = feed.debt_plans.iter().find(|p| p.debtor == company) {
        kpis.push(json!({"label": "Our instalment plan", "value": usd(p.principal_minor - p.paid_minor), "window": format!("left on {}", p.invoice_number), "source": "agreed by both agents"}));
    }

    let agents: Vec<Value> = c.agents.iter().map(|a| agent_view(company, a)).collect();
    let agent_of = |id: &str| {
        c.agents
            .iter()
            .find(|a| a.id == id)
            .map(|a| agent_view(company, a))
            .unwrap_or_else(|| json!({"name": id, "glyph": "Ag", "color": "#334155"}))
    };

    let mandates: Vec<Value> = f
        .mandates
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "agent": agent_of(&m.agent),
                "scope": m.scope,
                "per_payment": usd(m.per_payment_minor),
                "daily": usd(m.daily_minor),
                "used": usd(m.used_today_minor),
                "used_minor": m.used_today_minor,
                "daily_minor": m.daily_minor,
                "payees": m.payees.join(", "),
                "signed": format!("Signed by {} with a {} on {}", m.owner, m.signed_with, m.signed),
                "expires": format!("Good until {}", m.expires),
                "status": m.status,
                "frozen": m.status == "frozen",
            })
        })
        .collect();

    let decisions: Vec<Value> = f
        .decisions
        .iter()
        .map(|d| {
            let done_words = match d.state.as_str() {
                "approved" => "Approved. Payables pays it once.".to_string(),
                "declined" => "Declined. Nothing was paid.".to_string(),
                _ => String::new(),
            };
            let from = agent_of(&d.agent);
            let mut card = json!({
                "id": d.id,
                "title": d.title,
                "from": from,
                "body": d.body,
                "effects": d.effects,
                "why": d.why,
                "chip": d.amount_minor.map(|m| json!({"text": usd(m), "tone": "wait"})),
                "state": if d.state == "open" { "open" } else { "done" },
                "done_words": done_words,
                "approve_href": format!("{base}/console#approve-{}", d.id),
                "decline_action": if may_act { json!(format!("{base}/console/decisions/{}/decline", d.id)) } else { Value::Null },
                "sheet": {
                    "id": d.id,
                    "summary": d.title,
                    "fingerprint": d.fingerprint,
                    "passkey_action": format!("{base}/console/decisions/{}/approve", d.id),
                    "code_action": format!("{base}/console/decisions/{}/code", d.id),
                },
            });
            // For a proven agent: who it is and who vouched for it, beside its id.
            // A hold the agent door raised (agent `door`) is the requester's own ask.
            if let Some(r) = &d.requested_by {
                card["from_words"] = json!(if d.agent == "door" {
                    format!(
                        "{} asks at the agent door · vouched for by {} ({})",
                        r.id, r.vouched_by, r.method
                    )
                } else {
                    format!(
                        "{} asks for {} · vouched for by {} ({})",
                        from["name"].as_str().unwrap_or("An agent"),
                        r.id,
                        r.vouched_by,
                        r.method
                    )
                });
            }
            card
        })
        .collect();
    let open = f.decisions.iter().filter(|d| d.state == "open").count();
    let (open_decisions, done_decisions): (Vec<Value>, Vec<Value>) =
        decisions.into_iter().partition(|d| d["state"] == "open");

    let receivables: Vec<Value> = f
        .receivables
        .iter()
        .filter(|r| r.open_minor > 0)
        .map(|r| json!({"number": r.number, "customer": r.customer, "due": r.due.to_string(), "open": usd(r.open_minor), "age": if r.days_overdue > 0 { format!("{} days late", r.days_overdue) } else { "not due".into() }, "status": r.status.replace('_', " ")}))
        .collect();
    let payables: Vec<Value> = f
        .payables
        .iter()
        .filter(|p| p.open_minor > 0)
        .map(|p| json!({"number": p.number, "vendor": p.vendor, "due": p.due.to_string(), "open": usd(p.open_minor), "mandate": if p.mandate == "over" { "needs your OK" } else { "inside the mandate" }, "status": p.status}))
        .collect();

    let plans: Vec<Value> = feed
        .debt_plans
        .iter()
        .filter(|p| p.creditor == company || p.debtor == company)
        .map(|p| {
            let other = if p.creditor == company { &p.debtor } else { &p.creditor };
            let other = feed.company(other).map_or(other.clone(), |c| c.name.clone());
            json!({
                "title": format!("{} · {} {}", p.invoice_number, if p.creditor == company { "owed by" } else { "owed to" }, other),
                "summary": format!("{} of {} paid · agreed {}", usd(p.paid_minor), usd(p.principal_minor), p.agreed),
                "paid_minor": p.paid_minor,
                "principal_minor": p.principal_minor,
                "items": p.instalments.iter().map(|i| json!({
                    "when": i.due.to_string(),
                    "title": format!("Instalment {} · {}", i.n, usd(i.amount_minor)),
                    "body": match &i.paypal_id { Some(id) => format!("paid · PayPal {id}"), None => i.status.clone() },
                    "state": match i.status.as_str() { "paid" => "done", "due" => "now", "late" => "risk", _ => "next" },
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let settlements: Vec<Value> = f
        .settlements
        .iter()
        .filter(|s| s.status != "refused")
        .map(|s| json!({"when": s.at.to_string(), "title": format!("{} · {}", s.invoice_number, usd(s.amount_minor)), "body": format!("{} · PayPal {}", s.route, s.paypal_id), "state": "ok"}))
        .collect();
    // What the agent door refused: no invoice, no amount and no PayPal id, because nothing moved.
    let refused: Vec<&crate::feed::Settlement> = f
        .settlements
        .iter()
        .filter(|s| s.status == "refused")
        .collect();
    let refusals: Vec<Value> = refused
        .iter()
        .take(5)
        .map(|s| {
            let code = s.route.rsplit("refused: ").next().unwrap_or("refused");
            json!({"when": s.at.to_string(), "title": refusal_words(code), "body": format!("{code} · nothing moved"), "state": "risk"})
        })
        .collect();

    let ink = if company == KILELE {
        "#B45309"
    } else {
        "#0F8A8A"
    };
    json!({
        "company": company,
        "base": base,
        "site": {"name": c.name, "sub": if company == AGENTRIX { "Owner console" } else { "Bakery console" }, "monogram": c.name.chars().next().unwrap_or('A').to_string(), "tile": ink, "href": format!("{base}/console")},
        "owner": c.owner,
        "frozen": c.frozen,
        "may_act": may_act,
        "kpis": kpis,
        "agents": agents,
        "mandates": mandates,
        "open_decisions": open_decisions,
        "done_decisions": done_decisions,
        "open": open,
        "receivables": receivables,
        "payables": payables,
        "plans": plans,
        "settlements": settlements,
        "refusals": refusals,
        "refusals_total": refused.len(),
        "audit": {"verified": feed.audit.verified, "entries": feed.audit.entries, "checkpoint": feed.audit.checkpoint.chars().take(23).collect::<String>(), "checked": feed.audit.checked},
        "as_of": feed.as_of.to_string(),
        "source": feed.source,
        "is_agentrix": company == AGENTRIX,
    })
}
