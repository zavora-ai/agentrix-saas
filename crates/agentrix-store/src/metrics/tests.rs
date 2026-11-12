//! The metric catalogue against control.db's real schema, seeded with rows like the first release's.

use agentrix_types::SiteId;
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

use super::*;
use crate::testing::{apply, control_db, first_release_db};
use crate::time::{parse, stamp};
use crate::visits::{Device, Visit, VisitStore};
use crate::words::{MicroUsd, Shown};

fn now() -> DateTime<Utc> {
    parse("2026-11-08T09:00:00Z").unwrap()
}

fn ago(minutes: i64) -> String {
    stamp(now() - Duration::minutes(minutes))
}

async fn exec(pool: &SqlitePool, sql: &'static str, binds: &[&str]) {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(b.to_string());
    }
    q.execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn site(pool: &SqlitePool, id: &str, domain: &str, fixture: &str, owner: Option<&str>) {
    let mut q = sqlx::query(
        "INSERT INTO sites (site_id, domain, name, old_origin, utc_offset_minutes, \
         owner_account_id, fixture, created_at) VALUES (?, ?, ?, ?, 180, ?, ?, ?)",
    )
    .bind(id)
    .bind(domain)
    .bind(domain)
    .bind(format!("https://{domain}"))
    .bind(owner)
    .bind(fixture.parse::<i64>().unwrap());
    q = q.bind(ago(60 * 24 * 3));
    q.execute(pool).await.unwrap();
}

#[allow(clippy::too_many_arguments)]
async fn usage(
    pool: &SqlitePool,
    site: &str,
    purpose: &str,
    cost: i64,
    outcome: &str,
    move_id: Option<&str>,
    at: &str,
    units: Option<(i64, &str)>,
) {
    sqlx::query(
        "INSERT INTO usage (site_id, purpose, provider, model, move_id, tokens_in, tokens_out, \
         cost_micro_usd, outcome, at, units, unit) VALUES (?, ?, 'vultr', 'm1', ?, 100, 50, ?, ?, ?, ?, ?)",
    )
    .bind(site)
    .bind(purpose)
    .bind(move_id)
    .bind(cost)
    .bind(outcome)
    .bind(at)
    .bind(units.map(|u| u.0))
    .bind(units.map(|u| u.1))
    .execute(pool)
    .await
    .unwrap();
}

async fn step(
    pool: &SqlitePool,
    move_id: &str,
    site: &str,
    step: &str,
    state: &str,
    start: &str,
    end: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO move_steps (move_id, seq, site_id, step, state, summary, started_at, ended_at) \
         VALUES (?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM move_steps WHERE move_id = ?), ?, ?, ?, ?, ?, ?)",
    )
    .bind(move_id)
    .bind(move_id)
    .bind(site)
    .bind(step)
    .bind(state)
    .bind(format!("{step} {state}"))
    .bind(start)
    .bind(end)
    .execute(pool)
    .await
    .unwrap();
}

async fn receipt(pool: &SqlitePool, n: &str, site: &str, kind: &str, at: &str) {
    sqlx::query(
        "INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, \
         started_at, ended_at, issued_at, body) VALUES (?, ?, ?, ?, 'site', ?, 'control-1', ?, ?, ?, ?)",
    )
    .bind(format!("sha256:{n}"))
    .bind(format!("rcpt_{n}"))
    .bind(site)
    .bind(kind)
    .bind(site)
    .bind(at)
    .bind(at)
    .bind(at)
    .bind(format!(r#"{{"detail":{{"title":"Receipt {n}"}}}}"#))
    .execute(pool)
    .await
    .unwrap();
}

/// Kilele (real, live), a sample, a fixture, a site mid-Move and one only assessed.
async fn seeded() -> (SqlitePool, SqliteMetrics) {
    let pool = control_db().await;
    exec(&pool, "INSERT INTO accounts (id, email, display_name, created_at) VALUES ('acct_1', 'w@example.com', 'W', 0)", &[]).await;
    site(&pool, "site_bake", "kilele.example", "0", Some("acct_1")).await;
    site(
        &pool,
        "site_sample",
        "kilele-bakery-sample.agentrix.website",
        "0",
        None,
    )
    .await;
    site(&pool, "site_fix", "fixture.example", "1", None).await;
    site(&pool, "site_moving", "cafe.example", "0", Some("acct_1")).await;
    site(&pool, "site_new", "new.example", "0", None).await;

    // Kilele's Move: assess 2 min, replay 10 min, then live 2 hours ago.
    exec(&pool, "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES ('move_1', 'site_bake', 'live', ?, ?)", &[&ago(300), &ago(120)]).await;
    step(
        &pool,
        "move_1",
        "site_bake",
        "assess",
        "running",
        &ago(300),
        None,
    )
    .await;
    step(
        &pool,
        "move_1",
        "site_bake",
        "assess",
        "done",
        &ago(300),
        Some(&ago(298)),
    )
    .await;
    step(
        &pool,
        "move_1",
        "site_bake",
        "replay",
        "running",
        &ago(298),
        None,
    )
    .await;
    step(
        &pool,
        "move_1",
        "site_bake",
        "replay",
        "done",
        &ago(298),
        Some(&ago(288)),
    )
    .await;
    step(
        &pool,
        "move_1",
        "site_bake",
        "golive",
        "done",
        &ago(121),
        Some(&ago(120)),
    )
    .await;
    exec(&pool, "INSERT INTO releases (site_id, seq, manifest_digest, checkpoint, state, created_at, signed_at) VALUES ('site_bake', 1, 'sha256:m', 'cp', 'live', ?, ?)", &[&ago(125), &ago(120)]).await;
    exec(&pool, "INSERT INTO site_pointer (site_id, seq, manifest_digest, updated_at) VALUES ('site_bake', 1, 'sha256:m', ?)", &[&ago(120)]).await;

    // Café's Move is replaying.
    exec(&pool, "INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES ('move_2', 'site_moving', 'claimed', ?, ?)", &[&ago(20), &ago(5)]).await;
    step(
        &pool,
        "move_2",
        "site_moving",
        "assess",
        "done",
        &ago(20),
        Some(&ago(18)),
    )
    .await;
    step(
        &pool,
        "move_2",
        "site_moving",
        "replay",
        "running",
        &ago(5),
        None,
    )
    .await;

    // Sealed rooms.
    for (id, kind, state, start, end) in [
        ("job_1", "fetch", "done", ago(299), Some(ago(299))),
        ("job_2", "replay", "done", ago(298), Some(ago(297))),
        ("job_3", "build", "failed", ago(200), Some(ago(199))),
        ("job_4", "render", "running", ago(2), None),
    ] {
        sqlx::query("INSERT INTO jobs (job_id, site_id, kind, spec, state, created_at, started_at, ended_at) VALUES (?, 'site_bake', ?, '{}', ?, ?, ?, ?)")
            .bind(id).bind(kind).bind(state).bind(&start).bind(&start).bind(end)
            .execute(&pool).await.unwrap();
    }
    exec(&pool, "INSERT INTO jobs (job_id, site_id, kind, spec, state, created_at) VALUES ('job_5', 'site_moving', 'replay', '{}', 'queued', ?)", &[&ago(1)]).await;

    // Model spend: today in Nairobi started at 21:00 UTC yesterday (720 minutes ago).
    usage(
        &pool,
        "site_bake",
        "build",
        300_000,
        "ok",
        Some("move_1"),
        &ago(290),
        None,
    )
    .await;
    usage(
        &pool,
        "site_bake",
        "concierge",
        120_000,
        "ok",
        None,
        &ago(30),
        None,
    )
    .await;
    usage(
        &pool,
        "site_bake",
        "image",
        50_000,
        "ok",
        None,
        &ago(10),
        Some((2, "image")),
    )
    .await;
    usage(
        &pool,
        "site_bake",
        "concierge",
        0,
        "over-budget",
        None,
        &ago(5),
        None,
    )
    .await;
    usage(
        &pool,
        "site_bake",
        "concierge",
        0,
        "replayed",
        None,
        &ago(4),
        None,
    )
    .await;
    usage(
        &pool,
        "site_bake",
        "concierge",
        999_000,
        "ok",
        None,
        &ago(60 * 24 * 10),
        None,
    )
    .await;
    usage(
        &pool,
        "site_sample",
        "concierge",
        1_000_000,
        "ok",
        None,
        &ago(15),
        None,
    )
    .await;
    usage(&pool, "site_fix", "build", 10, "ok", None, &ago(15), None).await;
    exec(&pool, "INSERT INTO budgets (site_id, purpose, period, limit_micro_usd) VALUES ('site_bake', '*', 'day', 1000000)", &[]).await;

    // Receipts: one before go-live, two after.
    receipt(&pool, "a1", "site_bake", "job.fetch", &ago(299)).await;
    receipt(&pool, "a2", "site_bake", "release.signed", &ago(119)).await;
    receipt(&pool, "a3", "site_bake", "gauntlet.run", &ago(60)).await;
    receipt(&pool, "s1", "site_sample", "release.signed", &ago(60)).await;

    // Decisions: one waiting order, one approved; a waiting sample card.
    exec(&pool, r#"INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, created_at) VALUES ('dec_1', 'site_bake', 'order-held', '{"title":"Hold a cake order for Sat 14 Nov","body":"From Njeri (njeri@example.com)"}', 'sha256:f', 'waiting', ?)"#, &[&ago(40)]).await;
    exec(&pool, r#"INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, method, signer, assertion, created_at, decided_at) VALUES ('dec_2', 'site_bake', 'go-live', '{"title":"Go live"}', 'sha256:g', 'approved', 'passkey', 'acct_1', 'a', ?, ?)"#, &[&ago(130), &ago(120)]).await;
    exec(&pool, r#"INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, created_at) VALUES ('dec_3', 'site_sample', 'order-held', '{"title":"Sample order"}', 'sha256:h', 'waiting', ?)"#, &[&ago(10)]).await;

    // Gauntlet: 8/10 yesterday, 10/10 an hour ago.
    exec(&pool, "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, started_at, ended_at) VALUES ('g1', 'site_bake', 8, 10, ?, ?)", &[&ago(60 * 24), &ago(60 * 24 - 5)]).await;
    exec(&pool, "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, receipt, started_at, ended_at) VALUES ('g2', 'site_bake', 10, 10, 'sha256:a3', ?, ?)", &[&ago(65), &ago(60)]).await;

    exec(&pool, "INSERT INTO passed_questions (question_id, site_id, question, passed_at) VALUES ('q1', 'site_bake', 'Do you deliver to Karen?', ?)", &[&ago(50)]).await;
    exec(&pool, "INSERT INTO passed_questions (question_id, site_id, question, passed_at) VALUES ('q2', 'site_bake', 'Gluten-free?', ?)", &[&ago(60 * 24 * 2)]).await;

    let metrics = SqliteMetrics::new(pool.clone()).with_clock(now);
    (pool, metrics)
}

fn bake() -> SiteId {
    SiteId::new("site_bake")
}

#[tokio::test]
async fn sites_and_fleet_counts() {
    let (_pool, m) = seeded().await;
    let sites = m.sites().await.unwrap();
    assert_eq!(sites.len(), 5);
    let kilele = m.site(&bake()).await.unwrap().unwrap();
    assert_eq!(
        (kilele.state, kilele.origin, kilele.live_seq),
        (SiteState::Live, Origin::Real, Some(1))
    );
    assert_eq!(kilele.live_since, parse(&ago(120)));
    let sample = sites
        .iter()
        .find(|s| s.site_id.as_str() == "site_sample")
        .unwrap();
    assert_eq!(sample.origin, Origin::Sample);
    let fleet = m.fleet(Window::Today).await.unwrap();
    assert_eq!(
        fleet,
        FleetCounts {
            live: 1,
            moving: 1,
            awaiting_owner: 0,
            claimed: 0,
            assessed: 1,
            stopped: 0,
            samples: 1,
            fixtures: 1,
            went_live_in_window: 1,
        }
    );
}

#[tokio::test]
async fn spend_for_a_site_with_its_limits() {
    let (pool, m) = seeded().await;
    let today = m.spend(&Scope::Site(bake()), Window::Today).await.unwrap();
    assert_eq!(today.cost, MicroUsd(470_000));
    assert_eq!((today.calls, today.replayed, today.refused), (4, 1, 1));
    assert_eq!(today.by_purpose[0].key, "build");
    assert_eq!(
        today
            .by_family
            .iter()
            .map(|l| l.key.as_str())
            .collect::<Vec<_>>(),
        vec!["chat", "image"]
    );
    assert_eq!(today.units, vec![("image".to_owned(), 2)]);
    assert_eq!(
        today.words(),
        "Models cost $0.47 today across 4 calls, 1 of them replayed at no cost; 1 stopped by the budget."
    );
    // The budget row and the gateway's cap both count the UTC day (since 00:00 UTC, 540 min ago).
    assert_eq!(today.limits.len(), 2);
    assert_eq!(
        (today.limits[0].spent, today.limits[0].hard),
        (MicroUsd(470_000), false)
    );
    assert_eq!(today.limits[0].words(), "$0.47 of $1.00 today");
    assert_eq!(today.limits[0].percent(), 47);
    assert!(today.limits[1].hard);
    assert_eq!(today.limits[1].limit, MicroUsd(5_000_000));

    assert!(today.by_webagent.is_empty(), "no call named a WebAgent yet");
    sqlx::query(
        "INSERT INTO usage (site_id, purpose, provider, model, tokens_in, tokens_out, \
         cost_micro_usd, outcome, at, webagent, job) VALUES ('site_bake', 'webagent.act', \
         'vultr', 'm1', 10, 5, 7000, 'ok', ?, 'cashier', 'payments')",
    )
    .bind(ago(3))
    .execute(&pool)
    .await
    .unwrap();
    let today = m.spend(&Scope::Site(bake()), Window::Today).await.unwrap();
    assert_eq!(
        (today.by_webagent[0].key.as_str(), today.by_webagent[0].cost),
        ("cashier", MicroUsd(7_000))
    );
    assert_eq!(today.by_job[0].key, "payments");

    let month = m
        .spend(&Scope::Site(bake()), Window::ThisMonth)
        .await
        .unwrap();
    assert_eq!(month.cost, MicroUsd(477_000));
    let all = m.spend(&Scope::Site(bake()), Window::All).await.unwrap();
    assert_eq!(all.cost, MicroUsd(1_476_000));
}

#[tokio::test]
async fn fleet_spend_leaves_samples_and_fixtures_out() {
    let (_pool, m) = seeded().await;
    let fleet = m.spend(&Scope::Fleet, Window::Last24Hours).await.unwrap();
    assert_eq!(fleet.cost, MicroUsd(470_000));
    assert!(fleet.limits.is_empty());
    let by_site = m.spend_by_site(Window::Last24Hours).await.unwrap();
    let keys: Vec<_> = by_site
        .iter()
        .map(|s| (s.site_id.as_str(), s.origin, s.cost))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("site_sample", Origin::Sample, MicroUsd(1_000_000)),
            ("site_bake", Origin::Real, MicroUsd(470_000)),
            ("site_fix", Origin::Fixture, MicroUsd(10)),
        ]
    );
    assert_eq!(by_site[1].today.spent, MicroUsd(470_000));
}

#[tokio::test]
async fn receipts_list_count_and_page() {
    let (_pool, m) = seeded().await;
    let all = m.receipts(&bake(), &ReceiptQuery::default()).await.unwrap();
    assert_eq!(
        all.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
        vec!["gauntlet.run", "release.signed", "job.fetch"]
    );
    assert_eq!(all[0].title.as_deref(), Some("Receipt a3"));
    let page = m
        .receipts(
            &bake(),
            &ReceiptQuery {
                before: Some(all[0].issued_at.clone()),
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].kind, "release.signed");
    let jobs = m
        .receipts(
            &bake(),
            &ReceiptQuery {
                kinds: vec!["job.fetch".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    let counts = m
        .receipt_counts(&Scope::Site(bake()), Window::All)
        .await
        .unwrap();
    assert_eq!((counts.total, counts.since_go_live), (3, Some(2)));
    let fleet = m.receipt_counts(&Scope::Fleet, Window::All).await.unwrap();
    assert_eq!((fleet.total, fleet.since_go_live), (3, None));
}

#[tokio::test]
async fn moves_with_step_times() {
    let (_pool, m) = seeded().await;
    let moves = m.moves(&MoveQuery::default()).await.unwrap();
    assert_eq!(moves.len(), 2);
    let flight = m
        .moves(&MoveQuery {
            in_flight: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(flight.len(), 1);
    assert_eq!(flight[0].current_step.as_deref(), Some("replay"));
    assert_eq!(flight[0].steps[1].millis, Some(5 * 60_000));
    let kilele = m
        .moves(&MoveQuery {
            site: Some(bake()),
            ..Default::default()
        })
        .await
        .unwrap();
    let k = &kilele[0];
    assert_eq!(
        (k.in_flight, k.cost, k.current_step.clone()),
        (false, MicroUsd(300_000), None)
    );
    assert_eq!(
        k.steps
            .iter()
            .map(|s| (s.step.as_str(), s.state.as_str(), s.millis))
            .collect::<Vec<_>>(),
        vec![
            ("assess", "done", Some(120_000)),
            ("replay", "done", Some(600_000)),
            ("golive", "done", Some(60_000))
        ]
    );
    let times = m.step_times(Window::Last24Hours).await.unwrap();
    assert_eq!(
        times
            .iter()
            .map(|t| (t.step.as_str(), t.runs, t.p50_ms))
            .collect::<Vec<_>>(),
        vec![
            ("assess", 2, 120_000),
            ("replay", 1, 600_000),
            ("golive", 1, 60_000)
        ]
    );
}

#[tokio::test]
async fn rooms_gauntlet_decisions_questions() {
    let (pool, m) = seeded().await;
    let rooms = m.rooms(&Scope::Fleet, Window::Last24Hours).await.unwrap();
    assert_eq!(
        (rooms.running, rooms.queued, rooms.done, rooms.failed),
        (1, 1, 2, 1)
    );
    let replay = rooms.by_kind.iter().find(|k| k.kind == "replay").unwrap();
    assert_eq!((replay.jobs, replay.p50_ms), (2, Some(60_000)));

    let minutes = m
        .room_minutes(&Scope::Everything, Window::Last24Hours)
        .await
        .unwrap();
    assert_eq!(minutes.len(), 1);
    // fetch 0 s, replay 1 min, build 1 min, render running for 2 min.
    assert_eq!((minutes[0].jobs, minutes[0].millis), (3, 4 * 60_000));
    assert_eq!(minutes[0].minutes(), 4);
    let clipped = m
        .room_minutes(
            &Scope::Site(bake()),
            Window::Since(now() - Duration::seconds(30)),
        )
        .await
        .unwrap();
    assert_eq!(
        clipped[0].millis, 30_000,
        "only the part inside the window counts"
    );

    let scores = m.gauntlet(None).await.unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!((scores[0].score, scores[0].out_of), (10, 10));

    let waiting = m.decisions_waiting(&Scope::Site(bake())).await.unwrap();
    assert_eq!(waiting.count, 1);
    assert_eq!(waiting.cards[0].title, "Hold a cake order for Sat 14 Nov");
    let fleet = m.decisions_waiting(&Scope::Fleet).await.unwrap();
    assert_eq!(fleet.count, 1, "the sample's card is left out");

    assert_eq!(m.reports_written(Window::All).await.unwrap(), 0);
    sqlx::query(
        "INSERT INTO assessments (assessment_id, site_id, url, state, created_at, finished_at) \
         VALUES ('asmt_1', 'site_bake', 'https://kilele.example', 'report-ready', ?, ?)",
    )
    .bind(ago(30))
    .bind(ago(29))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(m.reports_written(Window::Today).await.unwrap(), 1);

    assert_eq!(m.questions_passed(&bake(), Window::Today).await.unwrap(), 1);
    assert_eq!(m.questions_passed(&bake(), Window::All).await.unwrap(), 2);
}

#[tokio::test]
async fn the_operators_reads_label_samples_and_the_fixture_and_stay_content_free() {
    let (pool, m) = seeded().await;
    let fleet = m.fleet_by_origin(Window::Last7Days).await.unwrap();
    assert_eq!(
        fleet.real,
        OriginCounts {
            sites: 3,
            claimed: 2,
            live: 1,
            went_live_in_window: 1,
            moves_in_flight: 1,
            moves_waiting_owner: 0,
            moves_live: 1,
        }
    );
    assert_eq!((fleet.sample.sites, fleet.sample.live), (1, 0));
    assert_eq!(fleet.fixture.sites, 1);

    let factory = m.move_factory(Window::Last30Days).await.unwrap();
    let replay = factory.at_step.iter().find(|(s, _)| s == "replay").unwrap();
    assert_eq!(replay.1, 1, "the cafe's Move is replaying");
    assert_eq!(factory.at_step.len(), 7);
    // Kilele's Move started 300 minutes ago and its go-live card came 120 minutes ago.
    assert_eq!(factory.to_go_live.n, 1);
    assert_eq!(factory.to_go_live.p90_ms, Some(180 * 60_000));

    let rooms = m
        .rooms(&Scope::Everything, Window::Last7Days)
        .await
        .unwrap();
    let replay = rooms.by_kind.iter().find(|k| k.kind == "replay").unwrap();
    assert_eq!(
        (replay.runs, replay.waits, replay.wait_p95_ms),
        (1, 1, Some(0))
    );

    let scores = m.gauntlet(None).await.unwrap();
    assert!(scores[0].live);
    assert_eq!(m.gauntlet_runs(Window::Last7Days).await.unwrap(), 2);
    assert_eq!(
        m.gauntlet_runs(Window::Since(now() - Duration::hours(2)))
            .await
            .unwrap(),
        1
    );

    let waiting = m.waiting_by_site().await.unwrap();
    assert_eq!(
        waiting
            .iter()
            .map(|w| (w.domain.as_str(), w.origin, w.count))
            .collect::<Vec<_>>(),
        vec![
            ("kilele-bakery-sample.agentrix.website", Origin::Sample, 1),
            ("kilele.example", Origin::Real, 1),
        ]
    );

    let by_site = m.spend_by_site(Window::Last24Hours).await.unwrap();
    let kilele = by_site
        .iter()
        .find(|s| s.site_id.as_str() == "site_bake")
        .unwrap();
    assert_eq!((kilele.replayed, kilele.errors), (1, 0));
    assert_eq!(kilele.budgets.len(), 1);
    assert_eq!(kilele.budgets[0].words(), "$0.47 of $1.00 today");
    let spend = m.spend(&Scope::Site(bake()), Window::Today).await.unwrap();
    assert_eq!(spend.ok, 3);

    // A run in progress (no receipt yet) never hides the last finished score.
    sqlx::query(
        "INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, started_at) \
         VALUES ('g3', 'site_bake', 0, 10, ?)",
    )
    .bind(ago(1))
    .execute(&pool)
    .await
    .unwrap();
    let scores = m.gauntlet(Some(&bake())).await.unwrap();
    assert_eq!(
        (scores[0].score, scores[0].receipt.as_deref()),
        (10, Some("sha256:a3"))
    );

    // The owner console's reads.
    let today = m.spend(&Scope::Site(bake()), Window::Today).await.unwrap();
    let day = today.day_limit().unwrap();
    assert_eq!(
        (day.limit, day.hard),
        (MicroUsd(1_000_000), false),
        "own budget under the cap"
    );
    let concierge = today
        .by_purpose
        .iter()
        .find(|l| l.key == "concierge")
        .unwrap();
    assert_eq!(concierge.label.as_deref(), Some("Answering visitors"));
    assert_eq!(today.by_provider[0].label, None);
    let waiting = m.decisions_waiting(&Scope::Site(bake())).await.unwrap();
    assert_eq!(waiting.oldest, parse(&ago(40)));
    sqlx::query(
        "INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, \
         started_at, ended_at, issued_at, body) VALUES ('sha256:f1', 'rcpt_f1', 'site_bake', \
         'move.step', 'move', 'move_1', 'control-1', ?, ?, ?, ?)",
    )
    .bind(ago(25))
    .bind(ago(20))
    .bind(ago(20))
    .bind(r#"{"detail":{"title":"Replay stopped","state":"failed"}}"#)
    .execute(&pool)
    .await
    .unwrap();
    let night = m
        .receipts(
            &bake(),
            &ReceiptQuery {
                ended_from: Some(now() - Duration::minutes(90)),
                ended_before: Some(now() - Duration::minutes(10)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        night
            .iter()
            .map(|r| (r.kind.as_str(), r.failed))
            .collect::<Vec<_>>(),
        vec![("move.step", true), ("gauntlet.run", false)]
    );

    let quiet = m
        .receipts(
            &bake(),
            &ReceiptQuery {
                content_free: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(quiet.iter().all(|r| r.title.is_none()));
}

#[tokio::test]
async fn health_scores_count_only_the_signals_with_a_reading() {
    let (pool, m) = seeded().await;
    let h = m.site_health(&bake()).await.unwrap().unwrap();
    // Decisions 1 handled of 1 (20/20), questions not rising (15/15), one call stopped by a
    // limit this week (5/15), no incidents declared (10/10); no owner sign-in on record, so
    // that signal doesn't count.
    assert_eq!(h.counted, 4);
    assert_eq!((h.score, h.band), (Some(83), Band::Healthy));
    assert_eq!(h.signals.len(), 9);
    assert_eq!(h.words(), "Healthy (83), from 4 of 9 signals.");
    let owner = h.signals.iter().find(|s| s.id == "owner_recency").unwrap();
    assert_eq!(owner.reading.as_deref(), Some("No sign-in on record."));
    let tickets = h.signals.iter().find(|s| s.id == "tickets").unwrap();
    assert_eq!(
        (tickets.reading.clone(), tickets.pending_owner),
        (None, Some("W3"))
    );

    // A sign-in ten days ago adds 12 of 20.
    sqlx::query(
        "INSERT INTO sessions (token_hash, account_id, method, created_at, expires_at) \
         VALUES ('t1', 'acct_1', 'passkey', ?, ?)",
    )
    .bind((now() - Duration::days(10)).timestamp())
    .bind(now().timestamp())
    .execute(&pool)
    .await
    .unwrap();
    let h = m.site_health(&bake()).await.unwrap().unwrap();
    assert_eq!((h.counted, h.score), (5, Some(78)));
    let payment = h.signals.iter().find(|s| s.id == "payment").unwrap();
    assert_eq!(
        payment.pending_owner,
        Some("W6"),
        "no subscriptions table here"
    );

    // With W6's subscriptions, a late payment counts 3 of 10.
    sqlx::raw_sql(
        "CREATE TABLE plan_subscriptions (id TEXT PRIMARY KEY, site_id TEXT NOT NULL, \
         state TEXT NOT NULL, updated_at INTEGER NOT NULL);
         INSERT INTO plan_subscriptions VALUES ('sub_1', 'site_bake', 'active', 1);
         INSERT INTO plan_subscriptions VALUES ('sub_2', 'site_bake', 'past_due', 2);",
    )
    .execute(&pool)
    .await
    .unwrap();
    let h = m.site_health(&bake()).await.unwrap().unwrap();
    let payment = h.signals.iter().find(|s| s.id == "payment").unwrap();
    assert_eq!(
        (payment.reading.as_deref(), payment.points),
        (Some("Payment is late."), Some(3))
    );
    assert_eq!((h.counted, h.score), (6, Some(72)));

    // An open SEV2 on the site takes the incidents signal to 0 of 10.
    sqlx::query(
        "INSERT INTO incidents (incident_id, severity, title, cell, site_id, commander, state, \
         opened_by, opened_at) VALUES ('INC-1', 'sev2', 'Ask slow', 'atl-0', 'site_bake', 'x', \
         'open', 'x', ?)",
    )
    .bind(ago(30))
    .execute(&pool)
    .await
    .unwrap();
    let h = m.site_health(&bake()).await.unwrap().unwrap();
    let slo = h.signals.iter().find(|s| s.id == "slo").unwrap();
    assert_eq!(
        (slo.reading.as_deref(), slo.points),
        (Some("1 serious incident (SEV1 or SEV2) open."), Some(0))
    );
    assert_eq!((h.score, h.band), (Some(61), Band::Watch));

    // A site that isn't live isn't scored: "nothing went wrong" only means nothing happened.
    let new = m
        .site_health(&SiteId::new("site_new"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (new.score, new.band, new.live),
        (None, Band::NotEnough, false)
    );
    assert_eq!(new.words(), "Not scored until the site is live.");
    assert!(m.site_health(&SiteId::new("nope")).await.unwrap().is_none());

    let live = m.health_by_site().await.unwrap();
    assert_eq!(live.len(), 1, "live sites only");
}

#[tokio::test]
async fn visits_hide_small_cells_and_say_when_nothing_counts() {
    let (pool, m) = seeded().await;
    let none = m.visits(&bake(), Window::Today).await.unwrap();
    assert!(matches!(
        none,
        Reading::NoSource {
            metric: "site.visits",
            ..
        }
    ));
    let store = VisitStore::new(pool.clone());
    let visit = |route: &str, device| Visit {
        site: bake(),
        route: route.to_owned(),
        referrer_host: String::new(),
        device,
        country: None,
        release_seq: Some(1),
    };
    for _ in 0..6 {
        store
            .record(&visit("/", Device::Phone), 180, now())
            .await
            .unwrap();
    }
    for _ in 0..2 {
        store
            .record(&visit("/cakes", Device::Desktop), 180, now())
            .await
            .unwrap();
    }
    let Reading::Value { value } = m.visits(&bake(), Window::Today).await.unwrap() else {
        panic!("counted");
    };
    assert_eq!(value.total, Shown::Exact(8));
    assert_eq!(value.by_route[0].visits, Shown::Exact(6));
    assert_eq!(value.by_route[1].visits, Shown::FewerThan5);
    assert_eq!(value.by_device[1].visits, Shown::FewerThan5);
    assert_eq!(value.words(), "8 visits today.");
    assert_eq!(store.prune(now()).await.unwrap(), 0);
}

#[tokio::test]
async fn usage_rebuild_keeps_first_release_rows_and_writers() {
    // The first release's shape, with rows as the gateway writes them.
    let pool = first_release_db().await;
    sqlx::query("INSERT INTO sites (site_id, domain, old_origin, created_at) VALUES ('s', 'a.example', 'https://a.example', ?)")
        .bind(ago(10)).execute(&pool).await.unwrap();
    // The gateway's own INSERT, word for word (crates/agentrix-models/src/meter.rs).
    const GATEWAY_INSERT: &str = "INSERT INTO usage (site_id, purpose, provider, model, tier, tier_reason, move_id, \
         tokens_in, tokens_out, reasoning_tokens, cost_micro_usd, outcome, at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
    for i in 0..372i64 {
        sqlx::query(GATEWAY_INSERT)
            .bind("s")
            .bind(if i % 2 == 0 { "build" } else { "concierge" })
            .bind("anthropic")
            .bind("claude")
            .bind(Some("standard"))
            .bind("")
            .bind(None::<String>)
            .bind(i)
            .bind(i)
            .bind(0i64)
            .bind(1_586i64)
            .bind("ok")
            .bind(ago(i))
            .execute(&pool)
            .await
            .unwrap();
    }
    let image = sqlx::query(GATEWAY_INSERT)
        .bind("s")
        .bind("image")
        .bind("vultr")
        .bind("qwen-image")
        .bind(None::<String>)
        .bind("")
        .bind(None::<String>)
        .bind(0i64)
        .bind(0i64)
        .bind(0i64)
        .bind(0i64)
        .bind("ok")
        .bind(ago(0))
        .execute(&pool)
        .await;
    assert!(image.is_err(), "the first release's CHECK refuses image");

    apply(&pool, "0800_visit_counts.sql").await;
    apply(&pool, "0801_usage_open.sql").await;

    let (rows, cost): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), SUM(cost_micro_usd) FROM usage")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((rows, cost), (372, 372 * 1_586));
    let (max_id,): (i64,) = sqlx::query_as("SELECT MAX(usage_id) FROM usage")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(max_id, 372, "ids kept");
    // The old writer still works, and image now fits.
    sqlx::query(GATEWAY_INSERT)
        .bind("s")
        .bind("image")
        .bind("vultr")
        .bind("qwen-image")
        .bind(None::<String>)
        .bind("")
        .bind(None::<String>)
        .bind(0i64)
        .bind(0i64)
        .bind(0i64)
        .bind(40_000i64)
        .bind("ok")
        .bind(ago(0))
        .execute(&pool)
        .await
        .unwrap();
    // Values outside the old lists that must still be refused.
    let bad = sqlx::query(GATEWAY_INSERT)
        .bind("s")
        .bind("build")
        .bind("vultr")
        .bind("m")
        .bind(Some("huge"))
        .bind("")
        .bind(None::<String>)
        .bind(0i64)
        .bind(0i64)
        .bind(0i64)
        .bind(0i64)
        .bind("ok")
        .bind(ago(0))
        .execute(&pool)
        .await;
    assert!(bad.is_err(), "tier is still checked");
    // The first release's reader (agentrix-node db::usage) still runs.
    let (sum,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(cost_micro_usd), 0) FROM usage WHERE site_id = ? AND at >= ?",
    )
    .bind("s")
    .bind("")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sum, 372 * 1_586 + 40_000);
    let (purposes,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM usage_purposes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(purposes >= 17);
}
