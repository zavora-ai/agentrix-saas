//! The brief the channels send (Telegram, the owner app's Now): this month's AI spend against the
//! site's monthly AI budget, in its currency, the figure the console's meter shows. Never today's
//! dollars against the gateway's daily limit.

use agentrix_channels::service::{Channels, Settings};
use agentrix_host::testing::context;
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};

const SITE: &str = "kilele-bakery-sample";
const HOST: &str = "kilele-bakery-sample.localhost";

async fn spend(channels: &Channels) -> Value {
    channels.brief_json(SITE).await["spend"].clone()
}

#[tokio::test]
async fn the_brief_says_this_months_ai_spend_against_the_monthly_budget_in_the_sites_currency() {
    let mut ctx = context().await.unwrap();
    ctx.pool = agentrix_store::testing::control_db().await;
    agentrix_decisions::migrate(&ctx.pool).await.unwrap();
    agentrix_shell::migrate(&ctx.pool).await.unwrap();
    agentrix_channels::migrate(&ctx.pool).await.unwrap();
    // Kilele on the Commerce plan (USD 30 of AI a month), in Kenya.
    sqlx::query("INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, created_at, plan) VALUES (?, ?, 'Kilele Bakery', ?, 180, '2026-10-01T00:00:00Z', 'commerce')")
        .bind(SITE)
        .bind(HOST)
        .bind(format!("http://{HOST}"))
        .execute(&ctx.pool)
        .await
        .unwrap();
    let channels = Channels::with_bots(
        &ctx,
        Vec::new(),
        agentrix_messages::from_context(&ctx),
        Settings::default(),
    );

    // Without W2's records there is no posted rate, so the amounts stay in dollars, the unit
    // the gateway meters in, rather than a guessed conversion.
    let s = spend(&channels).await;
    assert_eq!(s["words"], "$0.00 of $30.00 this month", "{s}");
    assert_eq!(
        (s["currency"].clone(), s["period"].clone()),
        (json!("USD"), json!("month"))
    );

    // Kilele's records post KSh 129.00 to the dollar on the live price list.
    let dir = ctx.data_dir.join("agentrix-samples");
    std::fs::create_dir_all(&dir).unwrap();
    let records = agentrix_records::Records::open(&dir.join("records.db"))
        .await
        .unwrap();
    let bundle = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/samples/kilele-bakery");
    records
        .load_bundle(&bundle, Some(HOST), false, agentrix_records::time::now())
        .await
        .unwrap();
    let channels = Channels::with_bots(
        &ctx,
        Vec::new(),
        agentrix_messages::from_context(&ctx),
        Settings::default(),
    );
    sqlx::query("INSERT INTO usage (site_id, purpose, provider, model, tokens_in, tokens_out, cost_micro_usd, outcome, at) VALUES (?, 'concierge', 'vultr', 'm1', 10, 5, 294000, 'ok', ?)")
        .bind(SITE)
        .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true))
        .execute(&ctx.pool)
        .await
        .unwrap();
    let s = spend(&channels).await;
    assert_eq!(
        s,
        json!({
            "used": 38.0,
            "budget": 3870.0,
            "currency": "KES",
            "period": "month",
            "words": "KSh 38 of KSh 3,870 this month",
        })
    );
    assert!(!s.to_string().contains("day"), "{s}");

    // What Telegram sends with it.
    let def = agentrix_messages::catalogue::get("owner.morning_brief").unwrap();
    let mut message = def.example_context();
    message["brief"] = channels.brief_json(SITE).await;
    let chat = agentrix_messages::render(def.id, &message)
        .unwrap()
        .chat
        .unwrap()
        .text;
    assert!(
        chat.contains("AI spend KSh 38 of KSh 3,870 this month."),
        "{chat}"
    );
}
