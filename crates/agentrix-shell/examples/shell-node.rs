//! The owner console as node mounts it, on an in-memory `control.db` built from the first release's schema
//! files and seeded with the Kilele Bakery sample (marked Sample), for walking every page and
//! action locally: the shell, W1's roster, dials and Workshop, card v2 and the Store.
//!
//! `scripts/wcargo run -p agentrix-shell --example shell-node`, then open
//! <http://localhost:18502/os>. It uses agentrix-host's test fakes: you are always signed in as
//! the sample's owner, and any approval code is accepted. Never point it at real data.

use std::sync::Arc;

use agentrix_decisions::{
    ApprovalKind, EditKind, EditableField, NewCard, RaisedBy, Risk, RiskLevel,
};
use agentrix_host::testing::{FixedOwner, context};
use agentrix_types::{CardKind, DecisionCard, DecisionId, EffectRequest, SiteId};
use chrono::{Duration, SecondsFormat, Utc};

const SITE: &str = "site_kilele_sample";

fn node_sql(path: &str) -> anyhow::Result<String> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../agentrix-node/src")
        .join(path);
    Ok(std::fs::read_to_string(&p)?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = std::env::var("AGENTRIX_SHELL_PORT").unwrap_or_else(|_| "18502".into());
    let account = uuid::Uuid::now_v7();
    let mut ctx = context().await?;
    ctx.owners = Arc::new(FixedOwner::passkey(account));
    // W1's routes refuse requests from another origin, so the preview names its own.
    ctx.origin = format!("http://localhost:{port}").parse()?;
    let pool = ctx.pool.clone();
    for f in [
        "auth/schema.sql",
        "claim/schema.sql",
        "db/migrations/0001_control.sql",
        "db/migrations/0002_passed_questions.sql",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(node_sql(f)?))
            .execute(&pool)
            .await?;
    }
    agentrix_shell::migrate(&pool).await?;
    agentrix_decisions::migrate(&pool).await?;
    agentrix_store::migrate(&pool).await?;
    agentrix_kernel::migrate(&pool).await?;

    let now = Utc::now();
    let t = |at: chrono::DateTime<Utc>| at.to_rfc3339_opts(SecondsFormat::Micros, true);
    sqlx::query("INSERT INTO accounts (id, email, display_name, created_at) VALUES (?, 'owner@kilele-bakery-sample.example', 'Wanjiru', 0)")
        .bind(account.to_string())
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, owner_account_id, created_at) VALUES (?, 'kilele-bakery-sample.agentrix.website', 'Kilele Bakery', 'https://kilele-bakery-sample.agentrix.website', 180, ?, ?)")
        .bind(SITE)
        .bind(account.to_string())
        .bind(t(now - Duration::days(1)))
        .execute(&pool)
        .await?;
    agentrix_kernel::Rosters::new(pool.clone(), ctx.blobs.clone())
        .seed_sample(SITE, "kilele")
        .await?;

    // A held order (the first release's card) and a passed question.
    let effect = EffectRequest {
        site: SiteId::new(SITE),
        webagent: "orders".into(),
        tool: "order.confirm".into(),
        args: serde_json::json!({ "order": "o-118", "customer": "Amani W. (sample)" }),
        amount: None,
        expiry: now + Duration::hours(10),
    };
    let card = DecisionCard::new(
        DecisionId::new("dec_sample_118"),
        CardKind::OrderHeld,
        "Approve cake order · KSh 4,500",
        "Amani W. · 2-tier vanilla · collect Sat 14 Nov, 10:00",
        effect,
        now - Duration::minutes(40),
    )?;
    sqlx::query("INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, created_at) VALUES (?, ?, 'order-held', ?, ?, 'waiting', ?)")
        .bind(card.decision_id.as_str())
        .bind(SITE)
        .bind(serde_json::to_string(&card)?)
        .bind(card.fingerprint.to_string())
        .bind(t(card.created_at))
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO passed_questions (question_id, site_id, question, contact, passed_at) VALUES ('q_sample_1', ?, 'Do you make gluten-free cakes?', NULL, ?)")
        .bind(SITE)
        .bind(t(now - Duration::hours(2)))
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO briefs (site_id, written_at, body) VALUES (?, ?, ?)")
        .bind(SITE)
        .bind(t(now - Duration::hours(1)))
        .bind(serde_json::json!({ "headline": "Sample data: one order is held for you and one question was passed to you.", "lines": [{ "text": "The Orders desk held a cake order for KSh 4,500 for your approval." }] }).to_string())
        .execute(&pool)
        .await?;
    // Card v2, so the lane shows both kinds: a review reply whose words the owner may edit, and
    // a cake order whose message, day and amount (lower only) the owner may change before
    // approving. A sample runner below records the approval and does nothing else.
    let d = agentrix_decisions::Decisions::new(&ctx);
    d.raise_editable(
        NewCard {
            kind: "reply.review".into(),
            title: "Reply to a 5-star review (sample)".into(),
            summary: "Reputation drafted a thank-you to Grace M.".into(),
            effects: vec!["Posts the reply below under Grace M.'s review".into()],
            effect: EffectRequest {
                site: SiteId::new(SITE),
                webagent: "reputation".into(),
                tool: "review.reply".into(),
                args: serde_json::json!({ "review": "r-12", "customer": "Grace M. (sample)", "reply": "Thank you, Grace. See you on Saturday." }),
                amount: None,
                expiry: now + Duration::days(2),
            },
            raised_by: RaisedBy { webagent: "reputation".into(), job: Some("reviews".into()) },
            risk: Risk { level: RiskLevel::Low, reasons: vec!["A thank-you in your own words".into(), "You can delete it any time".into()] },
            approval: ApprovalKind::OneTap,
            back: None,
            dedupe_key: Some("sample-review".into()),
        },
        vec![EditableField::new("reply", "Reply", EditKind::Text)],
    )
    .await?;
    d.raise_editable(
        NewCard {
            kind: "order.confirm".into(),
            title: "Approve cake order · KSh 4,500 (sample)".into(),
            summary: "Amani W. · 2-tier vanilla · collect Sat 14 Nov, 10:00".into(),
            effects: vec![
                "WhatsApp to Amani W. with the message below".into(),
                "PayPal payment request for the amount below".into(),
                "The collection slot is held for Amani".into(),
            ],
            effect: EffectRequest {
                site: SiteId::new(SITE),
                webagent: "cashier".into(),
                tool: "order.confirm".into(),
                args: serde_json::json!({ "order": "KB-1044", "customer": "Amani W. (sample)", "collect_on": "2026-11-14", "collect_at": "10:00", "message": "Hi Amani, Kilele here. Your cake is confirmed for Saturday at 10:00. Please pay the PayPal request to confirm it. Wanjiru" }),
                amount: Some(agentrix_types::Money { currency: "KES".into(), minor: 450_000 }),
                expiry: now + Duration::days(2),
            },
            raised_by: RaisedBy { webagent: "cashier".into(), job: Some("orders".into()) },
            risk: Risk { level: RiskLevel::Low, reasons: vec!["Returning customer".into(), "Usual amount".into(), "The slot is free".into()] },
            approval: ApprovalKind::Reviewed,
            back: None,
            dedupe_key: Some("sample-order-1044".into()),
        },
        vec![
            EditableField::new("message", "Message to Amani", EditKind::Text),
            EditableField::new("collect_on", "Collection day", EditKind::Date),
            EditableField::new("collect_at", "Collection time", EditKind::Time),
            EditableField::new("amount", "Amount", EditKind::AmountDown),
        ],
    )
    .await?;

    // A visitor talking to the Cakes desk (sample), as a site's Ask would record it.
    let live = agentrix_shell::live::Live::new(pool.clone());
    let thread = "lv_5a3b1e0c9d8f47a6b2c1d0e9f8a7b6c5";
    live.visitor_said(
        SITE,
        thread,
        "/cakes",
        "Cakes",
        "Do you do wedding cakes? About 120 guests, early January.",
    )
    .await?;
    live.desk_said(
        SITE,
        thread,
        "Yes. Three-tier wedding cakes serve 100 to 150 guests and need 3 weeks' notice.",
        &["/cakes#wedding".into()],
    )
    .await?;
    live.visitor_said(
        SITE,
        thread,
        "/cakes",
        "Cakes",
        "It has to be gluten-free. Is that something you do?",
    )
    .await?;
    live.desk_said(SITE, thread, "The bakery's pages don't say whether they make gluten-free tiers. I can ask the bakery now.", &[])
        .await?;

    let app = agentrix_shell::mount(&ctx)?
        .platform
        .merge(agentrix_kernel::mount(&ctx)?.platform)
        .merge(
            agentrix_decisions::mount_with(&ctx, {
                let mut runners = agentrix_shell::runners(&ctx);
                runners.push(Arc::new(SampleRunner("review.reply")));
                runners.push(Arc::new(SampleRunner("order.confirm")));
                runners
            })?
            .platform,
        )
        .merge(agentrix_ui::assets());
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    println!("Owner console (sample) on http://localhost:{port}/os");
    axum::serve(listener, app).await?;
    Ok(())
}

/// The sample's stand-in for the tools its cards name: it records nothing beyond the approval's
/// receipt, so approving a sample card sends no message and requests no payment.
struct SampleRunner(&'static str);

#[async_trait::async_trait]
impl agentrix_decisions::EffectRunner for SampleRunner {
    fn tool(&self) -> &str {
        self.0
    }
    async fn run(
        &self,
        card: &agentrix_decisions::DecisionCardV2,
        receipt: &str,
    ) -> anyhow::Result<Option<String>> {
        println!(
            "sample: approved {} ({}) · receipt {receipt}",
            card.title, card.effect.args
        );
        Ok(None)
    }
}
