//! The read a WebAgent's prompt uses: one WebAgent's active rules, newest first and capped, and
//! its grants in force.

use agentrix_decisions::taught;
use agentrix_host::testing::context;
use chrono::{Duration, SecondsFormat, Utc};

fn at(d: chrono::DateTime<Utc>) -> String {
    d.to_rfc3339_opts(SecondsFormat::Micros, true)
}

#[tokio::test]
async fn a_webagents_active_rules_newest_first_and_its_grants_in_force() {
    let ctx = context().await.unwrap();
    let pool = &ctx.pool;
    let now = Utc::now();

    // Before the migrations: none, not an error.
    assert_eq!(
        taught::rules(pool, "s1", Some("cashier"), 12)
            .await
            .unwrap()
            .1,
        0
    );
    assert_eq!(taught::grants(pool, "s1", None, now, 6).await.unwrap().1, 0);

    agentrix_decisions::migrate(pool).await.unwrap();
    let rule = |id: &str, webagent: &str, words: &str, mins_ago: i64, removed: bool| {
        let created = at(now - Duration::minutes(mins_ago));
        let removed = removed.then(|| at(now));
        let (id, webagent, words) = (id.to_owned(), webagent.to_owned(), words.to_owned());
        async move {
            sqlx::query("INSERT INTO decision_rules (id, site_id, webagent, job, kind, words, from_card, created_by, created_at, receipt, removed_at) VALUES (?, 's1', ?, 'payments', 'order.confirm', ?, 'dec2_x', 'owner', ?, 'r', ?)")
                .bind(id).bind(webagent).bind(words).bind(created).bind(removed)
                .execute(pool).await.unwrap();
        }
    };
    rule("rule_a", "cashier", "Oldest", 30, false).await;
    rule("rule_b", "cashier", "Removed", 20, true).await;
    rule("rule_c", "cashier", "Newest", 10, false).await;
    rule("rule_d", "cashier", "Middle", 15, false).await;
    rule("rule_e", "editor", "Not the Cashier's", 5, false).await;

    let (rules, total) = taught::rules(pool, "s1", Some("cashier"), 2).await.unwrap();
    assert_eq!(total, 3, "removed and other WebAgents' rules don't count");
    let words: Vec<&str> = rules.iter().map(|r| r.words.as_str()).collect();
    assert_eq!(words, ["Newest", "Middle"]);
    assert_eq!(rules[0].job.as_deref(), Some("payments"));
    assert_eq!(taught::rules(pool, "s1", None, 12).await.unwrap().1, 4);
    assert_eq!(
        taught::rules(pool, "s2", Some("cashier"), 12)
            .await
            .unwrap()
            .1,
        0
    );

    let grant = |id: &str, ends_in_days: i64, revoked: bool| {
        let terms = serde_json::json!({ "scope": "kind", "webagent": "cashier", "job": "payments", "kind": "order.confirm", "tool": "order.confirm", "person": null, "limit": null, "days": 90, "words": "orders" });
        let (id, revoked) = (id.to_owned(), revoked.then(|| at(now)));
        async move {
            sqlx::query("INSERT INTO decision_grants (id, site_id, webagent, kind, terms, from_card, created_by, created_at, expires_at, receipt, revoked_at) VALUES (?, 's1', 'cashier', 'order.confirm', ?, 'dec2_x', 'owner', ?, ?, 'r', ?)")
                .bind(id).bind(terms.to_string()).bind(at(now - Duration::days(1))).bind(at(now + Duration::days(ends_in_days))).bind(revoked)
                .execute(pool).await.unwrap();
        }
    };
    grant("grant_live", 30, false).await;
    grant("grant_ended", -1, false).await;
    grant("grant_stopped", 30, true).await;
    let (grants, total) = taught::grants(pool, "s1", Some("cashier"), now, 6)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(grants[0].id, "grant_live");
    assert_eq!(grants[0].terms.words, "orders");
    assert!(
        taught::grants(pool, "s1", Some("editor"), now, 6)
            .await
            .unwrap()
            .0
            .is_empty()
    );
}
