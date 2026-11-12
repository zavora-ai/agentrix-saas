//! A local preview of the operator console, for comparing pages with `concept/control-plane/`.
//! Never part of node: it approves sign-ins with a fixed code.
//!
//! ```sh
//! cargo run -p agentrix-operator --example ops-preview -- <scratch dir> [--seed]
//! ```
//!
//! Serves http://127.0.0.1:18300 over `<scratch dir>/control.db`, migrated with node's own
//! schema files. `--seed` adds preview records (an operator `ops@preview.test` with op.admin and
//! op.sre, and three made-up sites). Sign in with the code in `PREVIEW_CODE` (default 246810).

use std::sync::Arc;

use agentrix_host::Approvals;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use uuid::Uuid;

const SCHEMA: [&str; 15] = [
    include_str!("../../agentrix-node/src/auth/schema.sql"),
    include_str!("../../agentrix-node/src/claim/schema.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0001_control.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0002_passed_questions.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0300_operator.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0310_billing.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0320_incidents.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0330_partners.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0340_registry_columns.sql"),
    include_str!("../../agentrix-node/src/db/migrations/0350_billing_period.sql"),
    include_str!("../../agentrix-messages/migrations/1000_message_outbox.sql"),
    include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
    include_str!("../../agentrix-node/src/db/migrations/1201_voice_notes.sql"),
    include_str!("../../agentrix-node/src/db/migrations/1202_media_ideas.sql"),
    include_str!("../../agentrix-store/migrations/control/0802_health_hours.sql"),
];

struct CodeApprovals(String);

#[async_trait]
impl Approvals for CodeApprovals {
    async fn passkey_options(
        &self,
        _account: Uuid,
        _fingerprint: [u8; 32],
    ) -> anyhow::Result<Option<Value>> {
        Ok(None)
    }
    async fn verify(
        &self,
        account: Uuid,
        fingerprint: [u8; 32],
        request: Value,
    ) -> anyhow::Result<Value> {
        if request.get("totp").and_then(Value::as_str) != Some(self.0.as_str()) {
            anyhow::bail!(
                "That code didn't work. Codes change every 30 seconds, so use the newest one."
            );
        }
        Ok(
            json!({ "method": "totp", "account": account, "fingerprint": hex::encode(fingerprint), "preview": true }),
        )
    }
}

fn t(mins: i64) -> String {
    agentrix_operator::ledger::time(Utc::now() - Duration::minutes(mins))
}

async fn exec(pool: &SqlitePool, sql: &'static str, binds: &[&str]) {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn seed(pool: &SqlitePool) {
    let op = Uuid::now_v7().to_string();
    let owner = Uuid::now_v7().to_string();
    exec(pool, "INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, 'ops@preview.test', 'Preview Operator', 0)", &[&op]).await;
    exec(pool, "INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, 'owner@preview.test', 'Preview Owner', 0)", &[&owner]).await;
    for role in ["op.admin", "op.sre", "op.finance", "op.security"] {
        exec(pool, "INSERT INTO operator_roles (account_id, role, granted_by, granted_at) VALUES (?, ?, 'preview', ?)", &[&op, role, &t(0)]).await;
    }
    let digest = format!("sha256:{}", "7c1e".repeat(16));
    // A live site with a finished Move.
    let live = "site_preview_live";
    exec(pool, "INSERT INTO sites (site_id, domain, name, old_origin, owner_account_id, created_at) VALUES (?, 'bakery.preview.test', 'Preview Bakery', 'https://old.bakery.preview.test', ?, ?)", &[live, &owner, &t(3000)]).await;
    exec(pool, "INSERT INTO assessments (assessment_id, site_id, url, state, created_at) VALUES ('asmt_p1', ?, 'https://bakery.preview.test', 'report-ready', ?)", &[live, &t(3000)]).await;
    exec(pool, "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES ('move_p1', ?, 'live', ?, ?)", &[live, &t(2900), &t(2880)]).await;
    for (seq, step, start) in [
        ("1", "replay", 2900),
        ("2", "clean", 2897),
        ("3", "rebuild", 2895),
        ("4", "prove", 2893),
        ("5", "golive", 2892),
    ] {
        exec(pool, "INSERT INTO move_steps (move_id, seq, site_id, step, state, started_at, ended_at, receipt) VALUES ('move_p1', ?, ?, ?, 'done', ?, ?, ?)", &[seq, live, step, &t(start), &t(start - 1), &digest]).await;
    }
    exec(pool, "INSERT INTO releases (site_id, seq, manifest_digest, checkpoint, state, created_at, signed_at) VALUES (?, 1, ?, '{}', 'live', ?, ?)", &[live, &digest, &t(2885), &t(2884)]).await;
    exec(
        pool,
        "INSERT INTO site_pointer (site_id, seq, manifest_digest, updated_at) VALUES (?, 1, ?, ?)",
        &[live, &digest, &t(2880)],
    )
    .await;
    exec(pool, "INSERT INTO budgets (site_id, purpose, period, limit_micro_usd) VALUES (?, '*', 'month', 20000000)", &[live]).await;
    exec(pool, "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, receipt, started_at, ended_at) VALUES ('run_p1', ?, 10, 10, ?, ?, ?)", &[live, &digest, &t(60), &t(59)]).await;
    exec(pool, "INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, created_at) VALUES ('dec_p1', ?, 'order-held', '{}', 'f', 'waiting', ?)", &[live, &t(25)]).await;
    for (i, (purpose, provider, cost)) in [
        ("build", "vultr", "1200000"),
        ("concierge", "vultr", "310000"),
        ("vision-parity", "vultr", "2450000"),
        ("brief", "vultr", "90000"),
        ("embed", "vultr", "4000"),
    ]
    .into_iter()
    .enumerate()
    {
        let at = t(2890 - i as i64 * 300);
        exec(pool, "INSERT INTO usage (site_id, purpose, provider, model, move_id, tokens_in, tokens_out, cost_micro_usd, outcome, at) VALUES (?, ?, ?, 'qwen3.8-27b', 'move_p1', 42000, 6100, ?, 'ok', ?)", &[live, purpose, provider, cost, &at]).await;
    }
    for (id, kind, state, made) in [
        ("job_p1", "fetch", "done", "2990"),
        ("job_p2", "replay", "done", "2899"),
        ("job_p3", "render", "done", "2894"),
        ("job_p4", "build", "done", "2893"),
    ] {
        let made: i64 = made.parse().unwrap();
        exec(pool, "INSERT INTO jobs (job_id, site_id, move_id, kind, spec, state, created_at, started_at, ended_at) VALUES (?, ?, 'move_p1', ?, '{}', ?, ?, ?, ?)", &[id, live, kind, state, &t(made), &t(made - 1), &t(made - 2)]).await;
    }
    exec(pool, "INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, started_at, ended_at, issued_at, body) VALUES (?, 'rcpt_p1', ?, 'release.signed', 'release', '1', 'k', ?, ?, ?, '{}')", &[&digest, live, &t(2884), &t(2884), &t(2884)]).await;
    // A site mid-Move.
    let moving = "site_preview_moving";
    exec(pool, "INSERT INTO sites (site_id, domain, name, old_origin, owner_account_id, created_at) VALUES (?, 'clinic.preview.test', 'Preview Clinic', 'https://clinic.preview.test', ?, ?)", &[moving, &owner, &t(200)]).await;
    exec(pool, "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES ('move_p2', ?, 'cleaned', ?, ?)", &[moving, &t(30), &t(4)]).await;
    exec(pool, "INSERT INTO jobs (job_id, site_id, move_id, kind, spec, state, created_at, started_at) VALUES ('job_p5', ?, 'move_p2', 'build', '{}', 'running', ?, ?)", &[moving, &t(3), &t(2)]).await;
    // A lead.
    exec(pool, "INSERT INTO sites (site_id, domain, name, old_origin, created_at) VALUES ('site_preview_lead', 'shop.preview.test', '', 'https://shop.preview.test', ?)", &[&t(90)]).await;
    exec(pool, "INSERT INTO assessments (assessment_id, site_id, url, state, created_at) VALUES ('asmt_p3', 'site_preview_lead', 'https://shop.preview.test', 'report-ready', ?)", &[&t(90)]).await;
    let billing = agentrix_billing::Billing::new(pool.clone());
    billing
        .start(live, "business", "I-PREVIEW", None)
        .await
        .unwrap();
    billing
        .on_event(&agentrix_billing::SubscriptionEvent {
            provider_ref: "I-PREVIEW".into(),
            kind: agentrix_billing::EventKind::Activated,
            amount_cents: None,
            provider_payment_id: None,
            at: Utc::now() - Duration::days(2),
            period_end: None,
            receipt: None,
        })
        .await
        .unwrap();
    billing
        .went_live(live, Utc::now() - Duration::days(2))
        .await
        .unwrap();
    let agency = Uuid::now_v7().to_string();
    exec(pool, "INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, 'agency@preview.test', 'Preview Agency Person', 0)", &[&agency]).await;
    exec(pool, "INSERT INTO partners (partner_id, kind, name, created_by, created_at) VALUES ('pt_preview', 'agency', 'Preview Studio', ?, ?)", &[&op, &t(500)]).await;
    exec(pool, "INSERT INTO partner_members (partner_id, account_id, role, added_by, added_at) VALUES ('pt_preview', ?, 'partner.manager', ?, ?)", &[&agency, &op, &t(400)]).await;
    exec(pool, "INSERT INTO partner_clients (partner_id, site_id, charter_role, ring, state, fingerprint, invited_by, invited_at, decided_by, decided_method, decided_at) VALUES ('pt_preview', ?, 'editor', 'canary', 'granted', 'f', ?, ?, ?, 'passkey', ?)", &[live, &op, &t(300), &owner, &t(290)]).await;
    exec(pool, "INSERT INTO partner_clients (partner_id, site_id, charter_role, ring, state, fingerprint, invited_by, invited_at) VALUES ('pt_preview', ?, 'approver', 'stable', 'invited', 'f', ?, ?)", &[moving, &op, &t(30)]).await;
    println!("seeded: sign in as ops@preview.test");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber_init();
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("usage: ops-preview <dir> [--seed]"));
    let seed_it = args.any(|a| a == "--seed");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("control.db");
    let fresh = !path.exists();
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    if fresh {
        for sql in SCHEMA {
            sqlx::raw_sql(sql).execute(&pool).await?;
        }
        if seed_it {
            seed(&pool).await;
        }
    }
    let code = std::env::var("PREVIEW_CODE").unwrap_or_else(|_| "246810".into());
    let config = agentrix_operator::OperatorConfig::new("http://127.0.0.1:18300".parse()?);
    let mut state =
        agentrix_operator::OperatorState::new(config, pool, Arc::new(CodeApprovals(code)))?;
    if let Ok(models) = std::env::var("AGENTRIX_MODELS_URL") {
        state = state.with_gateway(reqwest::Client::new(), models);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18300").await?;
    println!("operator console preview on http://127.0.0.1:18300");
    axum::serve(listener, agentrix_operator::router(state)).await?;
    Ok(())
}

fn tracing_subscriber_init() {}
