//! The shell on sample data, for comparing pages with `concept/console/` in a browser.
//!
//! `scripts/wcargo run -p agentrix-shell --example shell-preview` then open
//! <http://localhost:18500/console/site_kilele_sample>. Set `AGENTRIX_SHELL_PORT` to move it,
//! and `AGENTRIX_SHELL_NO_PASSKEY=1` to see the authenticator-code path. The data is the
//! concept's sample bakery, marked Sample; nothing here talks to a real node. Orders and Customers
//! read sample rows in the tables a node keeps them in (an order from a customer's AI agent
//! waiting, one accepted and paid in PayPal's sandbox, one declined; a customer account; a
//! question passed with a contact), so those pages can be compared with the concept too.

use std::sync::Arc;

use agentrix_shell::{Shell, ShellState, router, sample::SamplePort, store::ShellStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("AGENTRIX_SHELL_PORT").unwrap_or_else(|_| "18500".into());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    let store = ShellStore::open(pool.clone()).await?;
    seed_commerce(&pool).await?;
    let sample = SamplePort {
        has_passkey: std::env::var("AGENTRIX_SHELL_NO_PASSKEY").is_err(),
        ..SamplePort::default()
    };
    let state = ShellState {
        shell: Arc::new(Shell::new()?),
        port: Arc::new(sample),
        store,
        apps: Vec::new(),
        lanes: Vec::new(),
        freeze_enabled: true,
    };
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    println!(
        "Shell preview on http://localhost:{port}/console/{}",
        agentrix_shell::sample::SITE
    );
    // Node mounts the kit's assets through agentrix-ui; the preview serves them itself.
    axum::serve(
        listener,
        router(state)
            .merge(agentrix_ui::assets())
            .merge(agentrix_kernel::site_routes()),
    )
    .await?;
    Ok(())
}

/// The tables Orders and Customers read, with the columns their owners' migrations make, and a
/// few sample rows from the concept's bakery.
async fn seed_commerce(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    use agentrix_types::{CardKind, DecisionCard, DecisionId, EffectRequest, SiteId};
    use chrono::{Duration, SecondsFormat, Utc};
    let site = agentrix_shell::sample::SITE;
    sqlx::raw_sql(
        "CREATE TABLE decisions (decision_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, kind TEXT NOT NULL, card TEXT NOT NULL, fingerprint TEXT NOT NULL, state TEXT NOT NULL, method TEXT, receipt TEXT, created_at TEXT NOT NULL, decided_at TEXT);
         CREATE TABLE receipts (digest TEXT PRIMARY KEY, site_id TEXT NOT NULL, kind TEXT NOT NULL, subject_id TEXT NOT NULL, issued_at TEXT NOT NULL);
         CREATE TABLE customer_payments (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, order_id TEXT NOT NULL, provider TEXT NOT NULL, currency TEXT NOT NULL, amount_minor INTEGER NOT NULL, state TEXT NOT NULL, sandbox INTEGER NOT NULL, created_at INTEGER NOT NULL);
         CREATE TABLE refunds (id TEXT PRIMARY KEY, payment_id TEXT NOT NULL, site_id TEXT NOT NULL, amount_minor INTEGER NOT NULL, currency TEXT NOT NULL, state TEXT NOT NULL, receipt TEXT, created_at INTEGER NOT NULL);
         CREATE TABLE visitor_accounts (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, email TEXT NOT NULL, name TEXT NOT NULL, phone TEXT, email_verified_at INTEGER, created_at INTEGER NOT NULL);
         CREATE TABLE consent_receipts (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, account_id TEXT NOT NULL, purpose TEXT NOT NULL, receipt TEXT, given_at INTEGER NOT NULL, withdrawn_at INTEGER);
         CREATE TABLE passed_questions (question_id TEXT PRIMARY KEY, site_id TEXT NOT NULL, question TEXT NOT NULL, contact TEXT, passed_at TEXT NOT NULL);",
    )
    .execute(pool)
    .await?;
    let now = Utc::now();
    let t = |at: chrono::DateTime<Utc>| at.to_rfc3339_opts(SecondsFormat::Micros, true);
    let mut ids = Vec::new();
    for (hours, name, email, what, state) in [
        (
            2,
            "Njeri",
            "njeri@example.com",
            "A 2-tier forest cake for Amina's birthday, no nuts, \"Happy 7th Amina\" on top",
            "waiting",
        ),
        (
            30,
            "Riverside Co.",
            "orders@riverside.example.com",
            "40 breakfast boxes for Friday's staff meeting",
            "approved",
        ),
        (
            54,
            "Amani W.",
            "amani@example.com",
            "A same-day wedding cake for 120",
            "declined",
        ),
    ] {
        let created = now - Duration::hours(hours);
        let effect = EffectRequest {
            site: SiteId::new(site),
            webagent: "Concierge".into(),
            tool: "door.request_custom_order".into(),
            args: serde_json::json!({ "capability": "request_custom_order", "idempotency_key": format!("preview-{hours}"),
                "input": { "description": what, "date": (now + Duration::days(6)).format("%Y-%m-%d").to_string(), "name": name, "email": email } }),
            amount: None,
            expiry: created + Duration::hours(48),
        };
        let card = DecisionCard::new(
            DecisionId::generate(),
            CardKind::OrderHeld,
            format!("Order from an AI customer: {what}"),
            "",
            effect,
            created,
        )?;
        let id = card.decision_id.as_str().to_owned();
        sqlx::query("INSERT INTO decisions VALUES (?, ?, 'order-held', ?, ?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(site)
            .bind(serde_json::to_string(&card)?)
            .bind(card.fingerprint.to_string())
            .bind(state)
            .bind((state == "approved").then_some("passkey"))
            .bind((state != "waiting").then(|| format!("sha256:{}", "3c".repeat(32))))
            .bind(t(created))
            .bind((state != "waiting").then(|| t(created + Duration::minutes(40))))
            .execute(pool)
            .await?;
        ids.push(id);
    }
    let secs = now.timestamp();
    sqlx::query("INSERT INTO receipts VALUES (?, ?, 'door.order-held', ?, ?)")
        .bind(format!("sha256:{}", "5a".repeat(32)))
        .bind(site)
        .bind(&ids[0])
        .bind(t(now - Duration::hours(2)))
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO customer_payments VALUES ('pay_preview1', ?, ?, 'paypal', 'KES', 1440000, 'captured', 1, ?)")
        .bind(site).bind(&ids[1]).bind(secs - 20 * 3600)
        .execute(pool).await?;
    sqlx::query("INSERT INTO visitor_accounts VALUES ('vac_preview_njeri', ?, 'njeri@example.com', 'Njeri', '+44 7700 900123', ?, ?)")
        .bind(site).bind(secs - 86_400 * 4).bind(secs - 86_400 * 4)
        .execute(pool).await?;
    sqlx::query("INSERT INTO consent_receipts VALUES ('cr_preview1', ?, 'vac_preview_njeri', 'order-updates', ?, ?, NULL)")
        .bind(site).bind(format!("sha256:{}", "7e".repeat(32))).bind(secs - 86_400 * 4)
        .execute(pool).await?;
    sqlx::query("INSERT INTO passed_questions VALUES ('pq_preview1', ?, 'Do you make gluten-free cakes?', 'mary.k@example.com', ?)")
        .bind(site).bind(t(now - Duration::hours(7)))
        .execute(pool).await?;
    Ok(())
}
