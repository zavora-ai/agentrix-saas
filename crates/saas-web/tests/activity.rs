//! Operators' live activity: each source reads only real records, the page names what isn't
//! connected, and the stream sends new rows.

use std::collections::HashMap;
use std::sync::Arc;

use adk_core::audit_log::{
    AuditAppend, AuditHash, AuditLogEntry, AuditLogStore, InMemoryAuditLogStore,
};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chrono::{Duration, Utc};
use saas_web::Face;
use saas_web::activity::{Activity, AuditSource, GatewaySource, Source, SpendCap};
use saas_web::hosts::Hosts;
use serde_json::{Value, json};
use tower::ServiceExt;

fn record(seq: u64, decision: &str, ext: Value) -> String {
    json!({
        "v": 1, "logId": "kilele", "seq": seq, "ts": Utc::now().to_rfc3339(),
        "actor": {"agent": "kilele-bakery/payables"},
        "kind": "tool_call", "action": "pay_bill", "decision": decision, "outcome": "not_run",
        "ext": ext,
    })
    .to_string()
}

async fn append(store: &InMemoryAuditLogStore, from: u64, records: Vec<String>) {
    let entries = records
        .into_iter()
        .enumerate()
        .map(|(i, r)| AuditLogEntry {
            seq: from + i as u64,
            leaf_hash: AuditHash([i as u8; 32]),
            record: Some(r),
        })
        .collect();
    store
        .append(AuditAppend {
            log_id: "kilele".into(),
            entries,
            nodes: vec![],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn the_audit_source_reads_each_record_once() {
    let store = Arc::new(InMemoryAuditLogStore::new());
    append(&store, 0, vec![
        record(0, "allowed", json!({"woke_by": "schedule 06:00 bills", "paypal_id": "5PX1", "amount_minor": -7900, "currency": "USD", "era_journal": "JE-2026-0031"})),
        record(1, "denied", json!({"reason": "Unga Millers isn't a payee in the mandate"})),
    ])
    .await;
    let source = AuditSource::new(
        store.clone(),
        HashMap::from([("kilele".to_owned(), "kilele".to_owned())]),
    );
    let rows = source.poll().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].paypal_id.as_deref(), Some("5PX1"));
    assert_eq!(rows[0].era.as_deref(), Some("JE-2026-0031"));
    assert_eq!(
        rows[1].reason.as_deref(),
        Some("Unga Millers isn't a payee in the mandate")
    );
    assert!(source.poll().await.unwrap().is_empty(), "nothing new");
    append(&store, 2, vec![record(2, "approval_required", json!({}))]).await;
    let rows = source.poll().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "audit:kilele:2");
}

#[tokio::test]
async fn the_gateway_source_reads_control_db_usage() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE usage (usage_id INTEGER PRIMARY KEY, site_id TEXT NOT NULL, purpose TEXT NOT NULL, model TEXT NOT NULL, tokens_in INTEGER NOT NULL, tokens_out INTEGER NOT NULL, cost_micro_usd INTEGER NOT NULL, outcome TEXT NOT NULL, at TEXT NOT NULL, webagent TEXT, job TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    let now = Utc::now();
    // As the gateway writes it: RFC 3339, microseconds, Z.
    let micros = |t: chrono::DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let old = micros(now - Duration::days(2));
    for (site, cost, outcome, at) in [
        ("site_kilele", 410, "ok", micros(now)),
        ("site_kilele", 900, "over-budget", micros(now)),
        ("site_other", 5000, "ok", micros(now)),
        ("site_kilele", 7000, "ok", old),
    ] {
        sqlx::query("INSERT INTO usage (site_id, purpose, model, tokens_in, tokens_out, cost_micro_usd, outcome, at, webagent, job) VALUES (?, 'webagent.act', 'qwen3.8-27b', 800, 90, ?, ?, ?, 'payables', 'bills-0600')")
            .bind(site).bind(cost).bind(outcome).bind(at)
            .execute(&pool)
            .await
            .unwrap();
    }
    let source = GatewaySource::new(
        pool.clone(),
        HashMap::from([("site_kilele".to_owned(), "kilele".to_owned())]),
    );
    let rows = source.poll().await.unwrap();
    assert_eq!(rows.len(), 3, "only Kilele's site");
    assert!(
        rows.iter()
            .any(|r| r.reason.as_deref() == Some("over the model budget"))
    );
    assert!(rows.iter().all(
        |r| r.model.as_deref() == Some("qwen3.8-27b") && r.agent.as_deref() == Some("payables")
    ));
    assert!(source.poll().await.unwrap().is_empty());
    let since = now - Duration::hours(1);
    assert_eq!(
        source.spend_since(since).await,
        Some(1310),
        "today's Kilele calls only"
    );
    // A fresh read for the page: the source above has read everything already.
    let a = Activity::new()
        .with_source(Arc::new(GatewaySource::new(
            pool.clone(),
            HashMap::from([("site_kilele".to_owned(), "kilele".to_owned())]),
        )))
        .with_cap(Some(SpendCap {
            micro_usd: 1000,
            per: "day".into(),
        }));
    a.refresh().await;
    let t = a.totals(Utc::now()).await;
    assert_eq!(t["cap"], "$0.0010 a day");
    assert!(
        t["money"].is_null(),
        "no wallets connected: not zero, not connected"
    );
    assert_eq!(t["refusals"], 1);
}

async fn get(app: &axum::Router, host: &str, path: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::get(path)
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    if path.contains("stream") {
        // The first read of the stream: rows, then totals and sources.
        use futures::StreamExt;
        let mut body = res.into_body().into_data_stream();
        let mut text = String::new();
        while let Some(Ok(chunk)) = body.next().await {
            text.push_str(&String::from_utf8_lossy(&chunk));
            if text.contains("event: sources") {
                break;
            }
        }
        return (status, text);
    }
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn the_page_streams_the_wallets_and_names_what_is_not_connected() {
    let app = Face::fakebooks(Hosts::default(), "2026-11-10".parse().unwrap())
        .await
        .unwrap()
        .with_board_dist(None)
        .with_grid_js(None)
        .router()
        .unwrap();
    let local = "money.localhost:18183";
    let (status, page) = get(&app, local, "/activity").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    for want in [
        "What the agents are doing",
        "Signed audit logs: not connected on this node",
        "Model gateway usage (control.db): not connected on this node",
        "Kernel events: not connected on this node",
        "Wallets and ERA postings",
        "DEMO-2PF93355",
        "data-stream=\"/activity/stream\"",
    ] {
        assert!(page.contains(want), "activity lacks {want:?}");
    }
    let (_, json) = get(&app, local, "/activity/rows.json?company=kilele&money=1").await;
    let v: Value = serde_json::from_str(&json).unwrap();
    let rows = v["rows"].as_array().unwrap();
    assert!(
        !rows.is_empty()
            && rows
                .iter()
                .all(|r| r["company"] == "kilele" && r["money"] == true)
    );
    assert!(
        v["totals"]["held"].as_u64().is_some(),
        "held decisions from the feed"
    );
    let (status, events) = get(&app, local, "/activity/stream?after=0").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        events.contains("event: row") && events.contains("event: totals"),
        "{events}"
    );

    // Not for the public: the gate sends anyone else away.
    let (status, _) = get(&app, "money.agentrix.website", "/activity").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, _) = get(&app, "money.agentrix.website", "/activity/rows.json").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = get(&app, "money.agentrix.website", "/activity/stream").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_audit_logs_come_from_the_node_at_each_read_and_read_as_saas_node_writes_them() {
    // Empty until the money layer starts: the source says so instead of showing nothing.
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let store = Arc::new(InMemoryAuditLogStore::new());
    let (flag, s) = (Arc::clone(&started), Arc::clone(&store));
    let source = AuditSource::each(move || {
        if flag.load(std::sync::atomic::Ordering::Relaxed) {
            vec![(
                "kilele".to_owned(),
                "kilele".to_owned(),
                Arc::clone(&s) as Arc<dyn AuditLogStore>,
            )]
        } else {
            Vec::new()
        }
    });
    let err = source.poll().await.unwrap_err().to_string();
    assert!(err.contains("hasn't started"), "{err}");

    started.store(true, std::sync::atomic::Ordering::Relaxed);
    let ts = Utc::now().to_rfc3339();
    append(&store, 0, vec![
        json!({"v": 1, "ts": ts, "actor": {}, "kind": "model_call", "action": "model.call",
               "ext": {"agent": "kilele-bakery/payables", "purpose": "webagent.act", "model": "qwen3.8-27b", "tokens_in": 900, "tokens_out": 80, "cost_micro_usd": 450}}).to_string(),
        json!({"v": 1, "ts": ts, "actor": {}, "kind": "access", "action": "a2a.refused",
               "ext": {"code": "signature_invalid", "requester": "wba:https://impostor.example#k1", "nothing_moved": true}}).to_string(),
        json!({"v": 1, "ts": ts, "actor": {}, "kind": "governance", "action": "agent.revoke",
               "ext": {"agent": "kilele-bakery/payables", "why": "the owner switched it off", "tokens_dropped": 2, "how": "owner console"}}).to_string(),
    ])
    .await;
    let rows = source.poll().await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (rows[0].what.as_str(), rows[0].agent.as_deref()),
        ("model call: webagent act", Some("payables"))
    );
    assert_eq!(rows[0].cost_micro_usd, Some(450));
    assert_eq!(rows[1].what, "door refused wba:https://impostor.example#k1");
    assert_eq!(
        rows[1].reason.as_deref(),
        Some("signature_invalid; nothing moved")
    );
    assert_eq!(rows[1].woke_by.as_deref(), Some("agent door"));
    assert_eq!(rows[2].what, "revoked payables (2 tokens dropped)");
    assert_eq!(rows[2].reason.as_deref(), Some("the owner switched it off"));
}

#[test]
fn the_live_records_read_in_words() {
    use saas_web::activity::audit_row;
    // Shapes as the node writes them (ext keys from agentrix-money on the VM).
    let tool = json!({"ts": "2026-11-11T15:45:00Z", "actor": {"agent": "urn:adk:agent:agentrix/payables"},
        "kind": "tool_call", "action": "ap_bills_due", "decision": "allowed",
        "ext": {"sessionId": "cron-payables", "effect": "read_only", "toolSource": "builtin"}});
    let r = audit_row("agentrix", "agentrix-money", 114, &tool);
    assert_eq!(
        (
            r.agent.as_deref(),
            r.woke_by.as_deref(),
            r.reason.as_deref()
        ),
        (
            Some("payables"),
            Some("schedule: payables"),
            Some("read only")
        )
    );
    let wake = json!({"ts": "2026-11-11T15:30:00Z", "actor": {}, "kind": "kernel", "action": "kernel.process.spawned",
        "ext": {"kind": "scheduled", "wake": {"anyOf": [{"cron": {"expr": "0 30 */3 * * *", "missed": "fireOnce"}}], "onExpiry": "cancel"}}});
    let r = audit_row("agentrix", "agentrix-money", 90, &wake);
    assert_eq!(
        (r.what.as_str(), r.woke_by.as_deref()),
        (
            "woke on its schedule",
            Some("schedule: every 3 hours at :30")
        )
    );
    let hourly = json!({"ts": "2026-11-11T15:15:00Z", "kind": "kernel", "action": "kernel.process.spawned",
        "ext": {"kind": "scheduled", "wake": {"anyOf": [{"cron": {"expr": "0 15 * * * *"}}]}}});
    assert_eq!(
        audit_row("kilele", "kilele-money", 1, &hourly)
            .woke_by
            .as_deref(),
        Some("schedule: every hour at :15")
    );
    let sleep = json!({"ts": "2026-11-11T15:31:00Z", "kind": "kernel", "action": "kernel.process.suspended", "ext": {"reason": "asleep"}});
    assert_eq!(
        audit_row("kilele", "kilele-money", 2, &sleep).what,
        "went back to sleep"
    );
    let freeze = json!({"ts": "2026-11-10T18:36:24Z", "kind": "governance", "action": "governance.freeze",
        "ext": {"reason": "the founder froze every money tool from the console", "replicated": true}});
    let r = audit_row("agentrix", "agentrix-money", 4, &freeze);
    assert_eq!(
        (r.what.as_str(), r.reason.as_deref()),
        (
            "froze every money tool",
            Some("the founder froze every money tool from the console")
        )
    );
    // A posting's ERA numbers, document first, once company writes them.
    let bill = json!({"ts": "2026-11-11T15:40:00Z", "kind": "tool_call", "action": "bill_model_spend", "decision": "allowed",
        "ext": {"sessionId": "cron-spend-bill", "era_document": "BILL-2026-0001", "era_journal": "JE-2026-0001", "amount_minor": -12, "currency": "USD"}});
    let r = audit_row("agentrix", "agentrix-money", 109, &bill);
    assert_eq!(r.era.as_deref(), Some("BILL-2026-0001 · JE-2026-0001"));
    assert_eq!(r.woke_by.as_deref(), Some("schedule: spend bill"));
}

#[tokio::test]
async fn the_test_agents_visits_are_read_once_and_whole() {
    use saas_web::activity::VisitsSource;
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity.jsonl");
    let line = |at: &str, agent: &str, kind: &str, ok: bool, detail: &str| {
        json!({"at": at, "agent": agent, "kind": kind, "ok": ok, "site": "https://kilele-bakery-sample.paypal.agentrix.website", "detail": detail}).to_string()
    };
    let source = VisitsSource::new(&path);
    assert!(
        source
            .poll()
            .await
            .unwrap_err()
            .to_string()
            .contains("no visits")
    );
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(
        f,
        "{}",
        line(
            "2026-11-11T13:40:26Z",
            "baraka",
            "quote",
            true,
            "quote red-velvet x1 = KSh 350 (USD 2.71)"
        )
    )
    .unwrap();
    // The next line is half written when the page reads.
    let half = line(
        "2026-11-11T13:54:16Z",
        "chiku",
        "pay",
        false,
        "not paid: no saved sandbox buyer yet",
    );
    write!(f, "{}", &half[..20]).unwrap();
    f.flush().unwrap();
    let rows = source.poll().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].company.as_str(),
            rows[0].agent.as_deref(),
            rows[0].woke_by.as_deref()
        ),
        (
            "kilele-bakery-sample",
            Some("baraka"),
            Some("test customer agent")
        )
    );
    assert_eq!(
        rows[0].what,
        "quote: quote red-velvet x1 = KSh 350 (USD 2.71)"
    );
    writeln!(f, "{}", &half[20..]).unwrap();
    f.flush().unwrap();
    let rows = source.poll().await.unwrap();
    assert_eq!(rows.len(), 1, "the finished line, once");
    assert_eq!(
        rows[0].what,
        "pay: not paid: no saved sandbox buyer yet (not ok)"
    );
    assert!(source.poll().await.unwrap().is_empty());
    // Rotated: a shorter file starts again.
    std::fs::write(
        &path,
        format!(
            "{}\n",
            line(
                "2026-11-11T16:00:00Z",
                "amani",
                "browse",
                true,
                "catalog.search \"oud\": 10 items"
            )
        ),
    )
    .unwrap();
    assert_eq!(source.poll().await.unwrap().len(), 1);
}

#[tokio::test]
async fn the_console_panel_shows_the_totals_and_the_newest_ten_linked_to_their_place() {
    let app = Face::fakebooks(Hosts::default(), "2026-11-10".parse().unwrap())
        .await
        .unwrap()
        .with_board_dist(None)
        .with_grid_js(None)
        .router()
        .unwrap();
    let local = "money.localhost:18183";
    let (status, html) = get(&app, local, "/activity/summary").await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(
        !html.contains("<html"),
        "a fragment for the console to embed"
    );
    for want in [
        "Live activity",
        "Open full activity",
        "Money moved today",
        "Waiting for owners",
        "Refusals today",
        "Model spend today",
        "Last reconciliation",
        "/static/saas/activity-summary.css",
    ] {
        assert!(html.contains(want), "the panel lacks {want:?}");
    }
    let rows = html.matches("<li class=").count();
    assert!((1..=10).contains(&rows), "the newest ten at most: {rows}");
    assert!(
        html.contains("href=\"/activity?company="),
        "each row links to its place"
    );
    // The full page carries the anchors the panel links to.
    let (_, page) = get(&app, local, "/activity").await;
    assert!(page.contains("id=\"row-wallet-"), "rows have anchors");
    // The same gate.
    let (status, _) = get(&app, "money.agentrix.website", "/activity/summary").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, js) = get(&app, local, "/static/saas/activity-summary.js").await;
    assert!(status.is_success() && js.contains("data-activity-summary"));
}

struct OneWallet(saas_core::wallet::WalletFeed);

#[async_trait::async_trait]
impl saas_web::apps::Wallets for OneWallet {
    async fn wallet(
        &self,
        _site: &str,
    ) -> Result<saas_core::wallet::WalletFeed, saas_web::apps::AppError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn each_wallet_is_read_by_the_id_its_log_uses() {
    use saas_web::activity::{PerSite, WalletSource};
    // Kilele's wallet log is wallet:kilele-bakery: the wallets know it as kilele-bakery, and the
    // page shows it as kilele. A guessed short id finds nothing and says so.
    let feed: saas_core::wallet::WalletFeed =
        serde_json::from_str(include_str!("data/wallet-feed-sample.json")).unwrap();
    let wallets = Arc::new(PerSite(HashMap::from([(
        "kilele-bakery".to_owned(),
        Arc::new(OneWallet(feed)) as Arc<dyn saas_web::apps::Wallets>,
    )])));
    let right = WalletSource::new(
        wallets.clone(),
        vec![("kilele-bakery".into(), "kilele".into())],
    );
    let rows = right.poll().await.unwrap();
    assert!(!rows.is_empty() && rows.iter().all(|r| r.company == "kilele"));
    assert!(
        rows.iter()
            .any(|r| r.paypal_id.as_deref() == Some("3KT78856M2504654G"))
    );
    let guessed = WalletSource::new(wallets, vec![("kilele".into(), "kilele".into())]);
    assert!(
        guessed.poll().await.is_err(),
        "a wrong id shows as not connected, never as empty"
    );
}
