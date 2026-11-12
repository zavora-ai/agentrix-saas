//! Card v2: raise, approve through the approvals port, decline, expiry, tampering, and the
//! limits on spoken answers. Sites come from the first release's `sites` table, created here.

use std::sync::{Arc, Mutex};

use agentrix_decisions::{
    Answer, ApprovalKind, Attempt, CardState, Decisions, EditKind, EditableField, EffectRunner,
    GrantScope, NewCard, RaisedBy, Risk, RiskLevel, Teach,
};
use agentrix_host::testing::{FixedOwner, MemoryReceipts, context};
use agentrix_types::{EffectRequest, SiteId};
use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use uuid::Uuid;

#[derive(Default)]
struct Confirm(
    Mutex<Vec<String>>,
    Mutex<Vec<agentrix_types::EffectRequest>>,
);

#[async_trait]
impl EffectRunner for Confirm {
    fn tool(&self) -> &str {
        "order.confirm"
    }
    async fn run(
        &self,
        card: &agentrix_decisions::DecisionCardV2,
        _receipt: &str,
    ) -> anyhow::Result<Option<String>> {
        self.0.lock().unwrap().push(card.id.clone());
        self.1.lock().unwrap().push(card.effect.clone());
        Ok(None)
    }
}

async fn setup() -> (Decisions, Arc<Confirm>, String) {
    let (d, runner, owner, _) = setup_with_receipts().await;
    (d, runner, owner)
}

async fn setup_with_receipts() -> (Decisions, Arc<Confirm>, String, Arc<MemoryReceipts>) {
    let owner = Uuid::now_v7();
    let mut ctx = context().await.unwrap();
    ctx.owners = Arc::new(FixedOwner::passkey(owner));
    let receipts = Arc::new(MemoryReceipts::default());
    ctx.receipts = receipts.clone();
    sqlx::raw_sql("CREATE TABLE sites (site_id TEXT PRIMARY KEY, owner_account_id TEXT); INSERT INTO sites VALUES ('s1', NULL);")
        .execute(&ctx.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE sites SET owner_account_id = ?")
        .bind(owner.to_string())
        .execute(&ctx.pool)
        .await
        .unwrap();
    agentrix_decisions::migrate(&ctx.pool).await.unwrap();
    let runner = Arc::new(Confirm::default());
    (
        Decisions::new(&ctx).with_runners(vec![runner.clone()]),
        runner,
        owner.to_string(),
        receipts,
    )
}

fn receipt_detail(receipts: &MemoryReceipts, digest: &str) -> (String, serde_json::Value) {
    let issued = receipts.issued.lock().unwrap();
    let (_, body) = issued.iter().find(|(d, _)| d == digest).expect("receipt");
    let v = serde_json::to_value(body).unwrap();
    (
        v["detail"]["title"].as_str().unwrap_or_default().to_owned(),
        v["detail"].clone(),
    )
}

fn card(expiry_hours: i64, risk: RiskLevel, approval: ApprovalKind, key: Option<&str>) -> NewCard {
    NewCard {
        kind: "order.confirm".into(),
        title: "Approve cake order · KSh 4,500".into(),
        summary: "Amani W. · collect Saturday".into(),
        effects: vec!["Amani gets a payment request for KSh 4,500".into()],
        effect: EffectRequest {
            site: SiteId::new("s1"),
            webagent: "cashier".into(),
            tool: "order.confirm".into(),
            args: serde_json::json!({ "order": "o-118" }),
            amount: None,
            expiry: Utc::now() + Duration::hours(expiry_hours),
        },
        raised_by: RaisedBy {
            webagent: "cashier".into(),
            job: Some("payments".into()),
        },
        risk: Risk {
            level: risk,
            reasons: vec!["Returning customer".into()],
        },
        approval,
        back: None,
        dedupe_key: key.map(str::to_owned),
    }
}

#[tokio::test]
async fn raise_once_then_approve_with_a_code_runs_the_effect() {
    let (d, runner, owner) = setup().await;
    let c = d
        .raise(card(
            8,
            RiskLevel::Low,
            ApprovalKind::Reviewed,
            Some("o-118"),
        ))
        .await
        .unwrap();
    let again = d
        .raise(card(
            8,
            RiskLevel::Low,
            ApprovalKind::Reviewed,
            Some("o-118"),
        ))
        .await
        .unwrap();
    assert_eq!(c.id, again.id, "the same key raises one card");
    assert_eq!(d.list("s1").await.unwrap().len(), 1);

    // A code must name the card's fingerprint.
    let r = d
        .approve(
            &c.id,
            &owner,
            Attempt::Totp("123456".into()),
            Some("sha256:00"),
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::CONFLICT);
    // Someone who doesn't own the site can't decide it.
    let r = d
        .approve(
            &c.id,
            &Uuid::now_v7().to_string(),
            Attempt::Totp("123456".into()),
            Some(&c.fingerprint.to_string()),
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    d.approve(
        &c.id,
        &owner,
        Attempt::Totp("123456".into()),
        Some(&c.fingerprint.to_string()),
    )
    .await
    .unwrap();
    assert_eq!(*runner.0.lock().unwrap(), vec![c.id.clone()]);
    let row = d.get("s1", &c.id).await.unwrap().unwrap();
    assert_eq!(row.state, CardState::Approved);
    assert_eq!(row.method.as_deref(), Some("totp"));
    assert!(row.receipt.is_some());
    let r = d
        .approve(
            &c.id,
            &owner,
            Attempt::Totp("123456".into()),
            Some(&c.fingerprint.to_string()),
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::CONFLICT, "a card runs once");
}

#[tokio::test]
async fn decline_and_expiry_never_run_anything() {
    let (d, runner, owner) = setup().await;
    let c = d
        .raise(card(8, RiskLevel::Low, ApprovalKind::Reviewed, None))
        .await
        .unwrap();
    d.decline(&c.id, &owner, "console").await.unwrap();
    assert_eq!(
        d.get("s1", &c.id).await.unwrap().unwrap().state,
        CardState::Declined
    );

    let old = d
        .raise(card(-1, RiskLevel::Low, ApprovalKind::Reviewed, None))
        .await
        .unwrap();
    assert_eq!(
        d.get("s1", &old.id).await.unwrap().unwrap().state,
        CardState::Expired
    );
    let r = d
        .approve(
            &old.id,
            &owner,
            Attempt::Passkey(serde_json::json!({})),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::GONE);
    assert!(runner.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn spoken_answers_allow_only_low_risk_one_tap_cards() {
    let (d, runner, owner) = setup().await;
    let reviewed = d
        .raise(card(8, RiskLevel::Low, ApprovalKind::Reviewed, None))
        .await
        .unwrap();
    let risky = d
        .raise(card(8, RiskLevel::High, ApprovalKind::OneTap, None))
        .await
        .unwrap();
    let everyday = d
        .raise(card(8, RiskLevel::Low, ApprovalKind::OneTap, None))
        .await
        .unwrap();
    for id in [&reviewed.id, &risky.id] {
        let r = d
            .decide_spoken("s1", &owner, id, true, "voice", "allow once")
            .await
            .unwrap_err();
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
    d.decide_spoken("s1", &owner, &everyday.id, true, "voice", "allow once")
        .await
        .unwrap();
    assert_eq!(
        d.get("s1", &everyday.id)
            .await
            .unwrap()
            .unwrap()
            .method
            .as_deref(),
        Some("spoken")
    );
    // Deny works on any card.
    d.decide_spoken("s1", &owner, &risky.id, false, "chat", "no")
        .await
        .unwrap();
    assert_eq!(
        d.get("s1", &risky.id).await.unwrap().unwrap().state,
        CardState::Declined
    );
    assert_eq!(*runner.0.lock().unwrap(), vec![everyday.id.clone()]);
}

#[tokio::test]
async fn a_card_nothing_can_carry_out_is_never_approved() {
    let (d, _runner, owner) = setup().await;
    let mut new = card(8, RiskLevel::Low, ApprovalKind::OneTap, None);
    new.effect.tool = "review.reply".into();
    let c = d.raise(new).await.unwrap();
    let r = d
        .approve(
            &c.id,
            &owner,
            Attempt::Totp("123456".into()),
            Some(&c.fingerprint.to_string()),
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = d
        .decide_spoken("s1", &owner, &c.id, true, "voice", "allow once")
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::CONFLICT);
    let row = d.get("s1", &c.id).await.unwrap().unwrap();
    assert_eq!(row.state, CardState::Waiting, "no approval was recorded");
    assert!(row.receipt.is_none());
    d.decline(&c.id, &owner, "console").await.unwrap();
}

fn order_card() -> NewCard {
    let mut new = card(8, RiskLevel::Low, ApprovalKind::Reviewed, None);
    new.effect.args = serde_json::json!({ "order": "o-118", "customer": "Amani W.", "message": "Hi Amani, your cake is confirmed for Saturday at 10:00.", "collect_on": "2026-11-14" });
    new.effect.amount = Some(agentrix_types::Money {
        currency: "KES".into(),
        minor: 450_000,
    });
    new
}

fn order_fields() -> Vec<EditableField> {
    vec![
        EditableField::new("message", "Message to Amani", EditKind::Text),
        EditableField::new("collect_on", "Collection day", EditKind::Date),
        EditableField::new("amount", "Amount", EditKind::AmountDown),
    ]
}

#[tokio::test]
async fn an_edited_approval_signs_and_records_the_before_and_after() {
    let (d, runner, owner, receipts) = setup_with_receipts().await;
    let c = d
        .raise_editable(order_card(), order_fields())
        .await
        .unwrap();
    let row = d.get("s1", &c.id).await.unwrap().unwrap();
    assert_eq!(row.editable.len(), 3);

    // Only the fields the raiser marked can change, and an amount only goes down.
    for (k, v) in [("customer", "Someone else"), ("amount", "5000")] {
        let answer = Answer {
            edits: [(k.to_owned(), v.to_owned())].into(),
            ..Default::default()
        };
        let r = d
            .approve_with(
                &c.id,
                &owner,
                Attempt::Totp("123456".into()),
                Some(&c.fingerprint.to_string()),
                &answer,
            )
            .await
            .unwrap_err();
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{k}");
    }

    let answer = Answer {
        edits: [
            (
                "message".to_owned(),
                "Hi Amani, your cake is ready on Saturday at 10:00.".to_owned(),
            ),
            ("amount".to_owned(), "4000".to_owned()),
        ]
        .into(),
        ..Default::default()
    };
    let sheet = d.sheet(&c.id, &owner, &answer).await.unwrap();
    let signed = sheet["fingerprint"].as_str().unwrap().to_owned();
    assert_ne!(
        signed,
        c.fingerprint.to_string(),
        "an edit changes what is signed"
    );
    assert!(
        sheet["sign_words"]
            .as_str()
            .unwrap()
            .contains("Amount \"KSh 4,000\" (was \"KSh 4,500\")"),
        "{sheet}"
    );

    d.approve_with(
        &c.id,
        &owner,
        Attempt::Totp("123456".into()),
        Some(&signed),
        &answer,
    )
    .await
    .unwrap();
    let row = d.get("s1", &c.id).await.unwrap().unwrap();
    assert_eq!(row.state, CardState::Approved);
    let edit = row.edit.expect("the edit is on the card");
    assert_eq!(edit.before, c.fingerprint);
    assert_eq!(edit.after.to_string(), signed);
    let amount = edit.changes.iter().find(|ch| ch.key == "amount").unwrap();
    assert_eq!(
        (amount.before.as_str(), amount.after.as_str()),
        ("KSh 4,500", "KSh 4,000")
    );

    // What ran is the edited effect, and the receipt says the owner edited it.
    let ran = runner.1.lock().unwrap().last().cloned().unwrap();
    assert_eq!(ran.amount.unwrap().minor, 400_000);
    assert_eq!(
        ran.args["customer"], "Amani W.",
        "fields the owner can't edit stay as raised"
    );
    let (title, detail) = receipt_detail(&receipts, row.receipt.as_deref().unwrap());
    assert!(title.contains("You edited and approved"), "{title}");
    assert_eq!(detail["fingerprint"], signed);
    assert_eq!(
        detail["evidence"]["fingerprint"],
        signed.trim_start_matches("sha256:")
    );
    assert_eq!(
        detail["edited_by_owner"]["before"],
        c.fingerprint.to_string()
    );
    assert_eq!(
        detail["edited_by_owner"]["changes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn decline_with_a_rule_keeps_it_for_the_webagent_until_removed() {
    let (d, runner, owner, receipts) = setup_with_receipts().await;
    let first = d
        .raise(card(8, RiskLevel::Low, ApprovalKind::Reviewed, None))
        .await
        .unwrap();
    // A rule needs words.
    let r = d
        .decline_with(
            &first.id,
            &owner,
            "console",
            &Teach {
                reason: None,
                rule: true,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    d.decline_with(
        &first.id,
        &owner,
        "console",
        &Teach {
            reason: Some("  Never take orders for the same day ".into()),
            rule: true,
        },
    )
    .await
    .unwrap();
    let row = d.get("s1", &first.id).await.unwrap().unwrap();
    assert_eq!(row.state, CardState::Declined);
    assert_eq!(
        row.decline_reason.as_deref(),
        Some("Never take orders for the same day")
    );
    let (title, detail) = receipt_detail(&receipts, row.receipt.as_deref().unwrap());
    assert!(title.contains("gave the Cashier a rule"), "{title}");
    assert_eq!(detail["rule"]["webagent"], "cashier");

    let rules = d.rules("s1", Some("cashier")).await.unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].words, "Never take orders for the same day");
    assert_eq!(rules[0].job.as_deref(), Some("payments"));
    assert!(d.rules("s1", Some("editor")).await.unwrap().is_empty());

    // A reason alone is kept on the card but makes no rule.
    let second = d
        .raise(card(8, RiskLevel::Low, ApprovalKind::Reviewed, None))
        .await
        .unwrap();
    d.decline_with(
        &second.id,
        &owner,
        "console",
        &Teach {
            reason: Some("Not this week".into()),
            rule: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(d.rules("s1", None).await.unwrap().len(), 1);

    // Only the owner removes it; then it is gone from the list but kept in the record.
    let r = d
        .remove_rule(&rules[0].id, &Uuid::now_v7().to_string())
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    d.remove_rule(&rules[0].id, &owner).await.unwrap();
    assert!(d.rules("s1", None).await.unwrap().is_empty());
    let r = d.remove_rule(&rules[0].id, &owner).await.unwrap_err();
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(
        runner.0.lock().unwrap().is_empty(),
        "declining runs nothing"
    );
}

fn order_for(customer: &str, minor: i64, risk: RiskLevel) -> NewCard {
    let mut new = card(8, risk, ApprovalKind::Reviewed, None);
    new.effect.args = serde_json::json!({ "order": format!("o-{minor}"), "customer": customer });
    new.effect.amount = Some(agentrix_types::Money {
        currency: "KES".into(),
        minor,
    });
    new
}

async fn approve_and_grant(d: &Decisions, owner: &str, id: &str, scope: GrantScope) -> String {
    let answer = Answer {
        grant: Some(scope),
        ..Default::default()
    };
    let sheet = d.sheet(id, owner, &answer).await.unwrap();
    let signed = sheet["fingerprint"].as_str().unwrap().to_owned();
    d.approve_with(
        id,
        owner,
        Attempt::Totp("123456".into()),
        Some(&signed),
        &answer,
    )
    .await
    .unwrap();
    signed
}

#[tokio::test]
async fn a_standing_grant_approves_matching_cards_within_its_limit_and_tells_the_owner() {
    let (d, runner, owner, receipts) = setup_with_receipts().await;
    let first = d
        .raise(order_for("Amani W.", 450_000, RiskLevel::Low))
        .await
        .unwrap();
    let answer = Answer {
        grant: Some(GrantScope::Person),
        ..Default::default()
    };
    let sheet = d.sheet(&first.id, &owner, &answer).await.unwrap();
    assert!(
        sheet["sign_words"]
            .as_str()
            .unwrap()
            .contains("Always allow orders for Amani W., up to KSh 4,500"),
        "{sheet}"
    );
    let signed = approve_and_grant(&d, &owner, &first.id, GrantScope::Person).await;
    assert_ne!(
        signed,
        first.fingerprint.to_string(),
        "the grant is part of what is signed"
    );
    let grants = d.grants("s1", None).await.unwrap();
    assert_eq!(grants.len(), 1);
    let g = &grants[0];
    assert_eq!(
        g.terms.limit.as_ref().unwrap().minor,
        450_000,
        "capped at the card's own amount"
    );
    let row = d.get("s1", &first.id).await.unwrap().unwrap();
    assert_eq!(row.grant_id.as_deref(), Some(g.id.as_str()));
    let (title, detail) = receipt_detail(&receipts, row.receipt.as_deref().unwrap());
    assert!(
        title.contains("Always allow orders for Amani W."),
        "{title}"
    );
    assert_eq!(detail["standing_grant"]["id"], g.id.as_str());
    assert_eq!(detail["fingerprint"], signed);

    // The same person under the limit: approved under the grant as it is raised, and run.
    let same = d
        .raise(order_for("Amani W.", 400_000, RiskLevel::Low))
        .await
        .unwrap();
    let row = d.get("s1", &same.id).await.unwrap().unwrap();
    assert_eq!(row.state, CardState::Approved);
    assert_eq!(row.method.as_deref(), Some("grant"));
    assert_eq!(row.grant_id.as_deref(), Some(g.id.as_str()));
    assert!(runner.0.lock().unwrap().contains(&same.id));
    let (title, detail) = receipt_detail(&receipts, row.receipt.as_deref().unwrap());
    assert!(title.contains("under your standing grant"), "{title}");
    assert_eq!(detail["counts_for_promotion"], false);

    // Over the limit, another person, or high risk: each waits for the owner.
    let over = d
        .raise(order_for("Amani W.", 500_000, RiskLevel::Low))
        .await
        .unwrap();
    let other = d
        .raise(order_for("Njeri", 100_000, RiskLevel::Low))
        .await
        .unwrap();
    let risky = d
        .raise(order_for("Amani W.", 100_000, RiskLevel::High))
        .await
        .unwrap();
    assert_eq!(d.sweep_grants().await.unwrap(), 0);
    for c in [&over, &other, &risky] {
        assert_eq!(
            d.get("s1", &c.id).await.unwrap().unwrap().state,
            CardState::Waiting
        );
    }
    assert_eq!(d.grants("s1", None).await.unwrap()[0].uses, 1);

    // Stopped, it approves nothing more.
    d.revoke_grant(&g.id, &owner).await.unwrap();
    assert!(d.grants("s1", None).await.unwrap().is_empty());
    let after = d
        .raise(order_for("Amani W.", 400_000, RiskLevel::Low))
        .await
        .unwrap();
    assert_eq!(
        d.get("s1", &after.id).await.unwrap().unwrap().state,
        CardState::Waiting
    );
}

#[tokio::test]
async fn high_risk_and_money_cards_refuse_standing_grants() {
    let (d, _runner, owner) = setup().await;
    let risky = d
        .raise(order_for("Amani W.", 450_000, RiskLevel::High))
        .await
        .unwrap();
    let mut refund = order_for("Amani W.", 120_000, RiskLevel::Low);
    refund.kind = "payment.refund".into();
    let refund = d.raise(refund).await.unwrap();
    for id in [&risky.id, &refund.id] {
        let answer = Answer {
            grant: Some(GrantScope::Kind),
            ..Default::default()
        };
        let r = d.sheet(id, &owner, &answer).await.unwrap_err();
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert!(r.words.contains("can't be allowed for good"), "{}", r.words);
        let r = d
            .approve_with(
                id,
                &owner,
                Attempt::Totp("123456".into()),
                Some("sha256:00"),
                &answer,
            )
            .await
            .unwrap_err();
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            d.get("s1", id).await.unwrap().unwrap().state,
            CardState::Waiting
        );
    }
    assert!(d.grants("s1", None).await.unwrap().is_empty());

    // A person grant needs a person on the card; a kind grant doesn't.
    let nobody = d
        .raise(card(8, RiskLevel::Low, ApprovalKind::Reviewed, None))
        .await
        .unwrap();
    let r = d
        .sheet(
            &nobody.id,
            &owner,
            &Answer {
                grant: Some(GrantScope::Person),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    approve_and_grant(&d, &owner, &nobody.id, GrantScope::Kind).await;
    let g = d.grants("s1", None).await.unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].terms.limit, None);
    // A kind grant made on a card without money never covers a card with money.
    let with_money = d
        .raise(order_for("Amani W.", 1_000, RiskLevel::Low))
        .await
        .unwrap();
    assert_eq!(
        d.get("s1", &with_money.id).await.unwrap().unwrap().state,
        CardState::Waiting
    );
}
