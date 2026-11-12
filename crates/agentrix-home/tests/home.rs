//! The home page renders on real facts, says what's missing when they don't exist, reads
//! control.db correctly, and keeps to the words rules.

use std::sync::Arc;

use agentrix_home::{FactSource, Facts, Home, HomeConfig, SqliteFacts, StoreFacts};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

struct Fixed(Facts);

#[async_trait]
impl FactSource for Fixed {
    async fn read(&self) -> anyhow::Result<Facts> {
        Ok(self.0.clone())
    }
}

fn home(facts: Facts) -> Home {
    home_with_film(facts, None)
}

fn home_with_film(facts: Facts, film: Option<agentrix_home::film::Film>) -> Home {
    let config = HomeConfig {
        origin: "https://agentrix.website".into(),
        assess_action: "/assess".into(),
        fixture_href: None,
        links: Default::default(),
        platform_host: "agentrix.website".into(),
        film,
        contact: None,
    };
    Home::new(config, Arc::new(Fixed(facts))).expect("the home page builds")
}

/// The node's own schema, so the queries run against what production has.
async fn control_db() -> sqlx::SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for sql in [
        include_str!("../../agentrix-node/src/db/migrations/0001_control.sql"),
        include_str!("../../agentrix-node/src/db/migrations/0002_passed_questions.sql"),
        include_str!("../../agentrix-node/src/db/migrations/1200_media.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    // W8's own migrations run in node too (its usage table rebuild among them).
    agentrix_store::migrate(&pool).await.unwrap();
    pool
}

async fn exec(pool: &sqlx::SqlitePool, sql: &'static str) {
    sqlx::raw_sql(sql).execute(pool).await.unwrap();
}

#[tokio::test]
async fn with_no_facts_every_section_says_what_will_appear() {
    let page = home(Facts::default());
    let front = page.render_home().await.unwrap();
    // v3's one line: identified by anyone, a passkey, a PayPal wallet; and the judges' path.
    assert!(front.contains("Turn your website into a team of <span>AI agents.</span>"));
    assert!(
        front.contains(
            "answers to a human with a passkey, and keeps a PayPal wallet anyone can check"
        )
    );
    assert!(
        front.contains("/judges\"") && front.contains("/judges#verify\""),
        "the judges' page and its Verify box"
    );
    assert!(!front.contains("Vultr"), "the Vultr story is over");
    assert!(front.contains(r#"action="/assess""#));
    assert!(front.contains(r#"name="url""#));
    // The detail lives one click away, on the desks' pages.
    let html = page.render_view("how.html").await.unwrap()
        + &page.render_view("samples.html").await.unwrap();
    assert!(html.contains("It hasn't run on this server yet."));
    assert!(html.contains("No Move has gone live on this server yet, so how long one takes"));
    assert!(
        html.contains(
            "The target is your go-live card within 10 minutes for a 50-page WordPress site."
        ),
        "the Move's time target, from requirement N1, called a target"
    );
    assert!(html.contains("The receipt count can&#x27;t be read right now."));
    assert!(html.contains("Opening soon at kilele-bakery-sample.agentrix.website"));
    // No made-up score anywhere.
    assert!(!html.contains("/10<"));
}

/// A control.db with one fixture Move that went live, gauntlet runs, receipts, a customer and a
/// sample: the rows the home page's facts read.
async fn seeded_db() -> sqlx::SqlitePool {
    let pool = control_db().await;
    // Seed only the rows these facts read; accounts and the rest aren't needed.
    exec(&pool, "PRAGMA foreign_keys = OFF").await;
    exec(&pool, r#"
        INSERT INTO sites (site_id, domain, name, old_origin, fixture, created_at) VALUES
          ('site_fx', 'fixture-agentrix-website.agentrix.website', 'Zavora''s fixture', 'https://fixture.agentrix.website', 1, '2026-11-07T18:00:00Z'),
          ('site_c', 'customer.example', 'A customer', 'https://customer.example', 0, '2026-11-07T18:00:00Z'),
          ('site_k', 'kilele-bakery-sample.agentrix.website', 'Kilele Bakery', 'https://kilele.example', 0, '2026-11-07T18:00:00Z');
        INSERT INTO moves (move_id, site_id, state, created_at, updated_at) VALUES
          ('move_1', 'site_fx', 'live', '2026-11-07T18:00:00Z', '2026-11-07T19:00:00Z');
        INSERT INTO move_steps (move_id, seq, site_id, step, state, summary, receipt, started_at, ended_at) VALUES
          ('move_1', 1, 'site_fx', 'replay', 'running', '', NULL, '2026-11-07T18:00:00Z', NULL),
          ('move_1', 2, 'site_fx', 'replay', 'done', 'Read 50 pages and 31 files in a sealed replay', 'sha256:aa', '2026-11-07T18:00:00Z', '2026-11-07T18:00:12.400Z'),
          ('move_1', 3, 'site_fx', 'clean', 'done', '50 pages clean; 4 items left behind', 'sha256:bb', '2026-11-07T18:00:13Z', '2026-11-07T18:00:50.500Z'),
          ('move_1', 4, 'site_fx', 'prove', 'done', 'Parity 98.8%; release 2 is ready for you', 'sha256:cc', '2026-11-07T18:01:00Z', '2026-11-07T18:01:20Z'),
          ('move_1', 5, 'site_fx', 'golive', 'done', 'Live', 'sha256:dd', '2026-11-07T18:01:20Z', '2026-11-07T18:30:41Z');
        INSERT INTO decisions (decision_id, site_id, kind, card, fingerprint, state, method, signer, assertion, created_at, decided_at) VALUES
          ('dec_1', 'site_fx', 'go-live', '{}', 'fp', 'approved', 'passkey', 'acct_1', '{}', '2026-11-07T18:01:20Z', '2026-11-07T18:30:00Z');
        INSERT INTO quarantine (item_id, site_id, move_id, title, reason, location, state, created_at, updated_at) VALUES
          ('q1', 'site_fx', 'move_1', 'Webshell', 'A shell', 'uploads/x.php', 'held', '2026-11-07T18:00:20Z', '2026-11-07T18:00:20Z'),
          ('q2', 'site_fx', 'move_1', 'Spam links', 'Hidden links', 'footer', 'held', '2026-11-07T18:00:20Z', '2026-11-07T18:00:20Z');
        INSERT INTO usage (site_id, purpose, provider, model, move_id, tokens_in, tokens_out, cost_micro_usd, outcome, at) VALUES
          ('site_fx', 'build', 'anthropic', 'm', 'move_1', 10, 10, 300000, 'ok', '2026-11-07T18:00:30Z'),
          ('site_fx', 'vision-parity', 'anthropic', 'm', 'move_1', 10, 10, 70000, 'ok', '2026-11-07T18:01:10Z');
        INSERT INTO releases (site_id, seq, manifest_digest, checkpoint, state, created_at) VALUES
          ('site_fx', 1, 'sha256:m', 'cp', 'live', '2026-11-07T18:30:41Z'),
          ('site_k', 1, 'sha256:k', 'cp', 'live', '2026-11-07T18:30:41Z');
        INSERT INTO site_pointer (site_id, seq, manifest_digest, updated_at) VALUES
          ('site_fx', 1, 'sha256:m', '2026-11-07T18:30:41Z'),
          ('site_k', 1, 'sha256:k', '2026-11-07T18:30:41Z');
        INSERT INTO gauntlet_runs (run_id, site_id, score, out_of, receipt, detail, started_at, ended_at) VALUES
          ('gr_old', 'site_fx', 7, 10, 'sha256:g0', NULL, '2026-11-07T15:00:00Z', '2026-11-07T15:05:00Z'),
          ('gr_new', 'site_fx', 10, 10, 'sha256:g1', '[{"name":"Webshell in uploads","plain":"x","held":true}]', '2026-11-07T19:00:00Z', '2026-11-07T19:04:00Z'),
          ('gr_running', 'site_fx', 0, 10, NULL, NULL, '2026-11-07T20:00:00Z', NULL);
        INSERT INTO receipts (digest, receipt_id, site_id, kind, subject_type, subject_id, issuer, started_at, ended_at, issued_at, body) VALUES
          ('sha256:r1', 'rcpt_1', 'site_fx', 'routing.changed', 'site', 'site_fx', 'k', '2026-11-07T19:00:00Z', '2026-11-07T19:00:00Z', '2026-11-07T19:00:00Z',
           '{"detail":{"to":{"provider":"vultr","routes":{"build":{"provider":"vultr","model":"qwen3.8-27b"},"embed":{"provider":"vultr","model":"qwen3-embedding-4b"}}}}}'),
          ('sha256:r2', 'rcpt_2', 'site_fx', 'job.replay', 'move', 'move_1', 'k', '2026-11-07T18:00:00Z', '2026-11-07T18:00:12Z', '2026-11-07T18:00:12Z', '{}');
        INSERT INTO assessments (assessment_id, site_id, url, state, created_at) VALUES
          ('as_1', 'site_c', 'https://customer.example', 'report-ready', '2026-11-07T17:00:00Z'),
          ('as_2', 'site_c', 'https://customer.example', 'failed', '2026-11-07T17:00:00Z');
        INSERT INTO briefs (site_id, written_at, body) VALUES
          ('site_c', '2026-11-08T04:00:00Z', '{"headline":"A customer''s private brief","lines":[]}'),
          ('site_fx', '2026-11-08T03:00:00Z', '{"headline":"Your site moved.","lines":[{"text":"4 items were left behind."}]}');
    "#).await;
    pool
}

#[tokio::test]
async fn facts_from_control_db_reach_the_page() {
    let facts = SqliteFacts::new(seeded_db().await).read().await.unwrap();
    facts_read_right(facts).await;
}

#[tokio::test]
async fn sample_cards_show_w11s_pictures_with_their_label() {
    let pool = control_db().await;
    let blobs: Arc<dyn agentrix_host::Blobs> =
        Arc::new(agentrix_host::testing::MemoryBlobs::default());
    // The pictures agentrix-media embeds: nothing is drawn.
    agentrix_media::samples::seed(&pool, &blobs).await.unwrap();
    let metrics = agentrix_store::metrics::SqliteMetrics::new(pool.clone());
    let facts = StoreFacts::new(Arc::new(metrics), SqliteFacts::new(pool))
        .with_samples(vec!["kilele-bakery-sample".into(), "no-such-sample".into()])
        .read()
        .await
        .unwrap();
    let hero = facts
        .sample_pictures
        .get("kilele-bakery-sample")
        .cloned()
        .expect("Kilele has an approved hero picture");
    assert!(hero.url.starts_with("/media/"));
    assert_eq!(hero.label.as_deref(), Some("Sample image, made with AI"));
    assert!(!facts.sample_pictures.contains_key("no-such-sample"));

    let html = home(facts).render_view("samples.html").await.unwrap();
    assert!(html.contains(&format!(r#"src="{}""#, hero.url)));
    assert!(html.contains(r#"<figcaption class="rtag">Sample image, made with AI</figcaption>"#));
    // A sample whose picture wasn't read keeps its screenshot.
    assert!(html.contains(r#"src="/_home/shot-tana.webp"#));
}

#[tokio::test]
async fn facts_through_w8s_metrics_read_the_same() {
    let pool = seeded_db().await;
    let read = || async {
        let metrics = agentrix_store::metrics::SqliteMetrics::new(pool.clone());
        StoreFacts::new(Arc::new(metrics), SqliteFacts::new(pool.clone()))
            .read()
            .await
            .unwrap()
    };
    // A run is going: the last finished score still shows.
    facts_read_right(read().await).await;
    // No run going: W8's latest run is the finished one.
    exec(
        &pool,
        "DELETE FROM gauntlet_runs WHERE run_id = 'gr_running'",
    )
    .await;
    facts_read_right(read().await).await;
}

async fn facts_read_right(facts: Facts) {
    let g = facts.gauntlet.as_ref().expect("a finished run");
    assert_eq!(
        (g.score, g.out_of, g.receipt.as_deref()),
        (10, 10, Some("sha256:g1"))
    );
    let m = facts.first_live_move.as_ref().expect("a live Move");
    assert_eq!(m.site_name, "Zavora's fixture");
    assert_eq!(
        m.steps.iter().map(|s| s.step.as_str()).collect::<Vec<_>>(),
        ["replay", "clean", "prove", "golive"]
    );
    assert!((m.steps[0].seconds - 12.4).abs() < 1e-9);
    assert_eq!(m.approval_to_live_seconds, Some(41.0));
    assert_eq!(m.left_behind, Some(2));
    assert_eq!(m.cost_micro_usd, Some(370_000));
    assert_eq!(facts.sites_live, Some(1), "samples are not counted");
    assert_eq!(facts.receipts_issued, Some(2));
    assert_eq!(facts.reports_written, Some(1));
    assert_eq!(facts.routing.as_ref().unwrap().provider, "vultr");
    assert_eq!(
        facts.brief.as_ref().unwrap().headline.as_deref(),
        Some("Your site moved."),
        "only a fixture's brief is shown"
    );

    // The samples are W2's own hosts: they're linked while they answer.
    let page = home(facts);
    page.sample_probe()
        .set_open(vec!["kilele-bakery-sample.agentrix.website".into()]);
    let front = page.render_home().await.unwrap();
    assert!(
        front.contains("10 of 10 stopped."),
        "the Guardian's state comes from the run's record"
    );
    // Screen 5 is the owner console now; the attack test's result shows on the Guardian's card.
    assert!(front.contains("Your WebAgents work for you. You stay in charge."));
    assert!(!front.contains("If your site was hacked"));
    let html = page.render_view("how.html").await.unwrap()
        + &page.render_view("samples.html").await.unwrap();
    assert!(html.contains("10/10"));
    assert!(html.contains(r#"href="/receipts/sha256:g1""#));
    assert!(html.contains("12.4 s"));
    assert!(html.contains("98.8%, target 95%"));
    assert!(html.contains("2 items, with reasons"));
    assert!(html.contains("Live 41.0 s after approval."));
    assert!(html.contains("$0.37"));
    assert!(html.contains("qwen3-embedding-4b"));
    assert!(html.contains(r#"href="https://kilele-bakery-sample.agentrix.website/""#));
    assert!(html.contains("Opening soon at classy-perfumes-sample.agentrix.website"));
    assert!(!html.contains("private brief"));
}

#[tokio::test]
async fn the_routes_answer() {
    let home = home(Facts::default());
    let app = home.routes().merge(home.front_door());
    for (path, kind) in [
        ("/", "text/html"),
        ("/home", "text/html"),
        ("/how-it-works", "text/html"),
        ("/samples", "text/html"),
        ("/prices", "text/html"),
        ("/_home/check.js", "text/javascript"),
        ("/_home/play.js", "text/javascript"),
        ("/privacy", "text/html"),
        ("/terms", "text/html"),
        ("/bot", "text/html"),
        ("/.well-known/agentrix-crawler", "text/html"),
        ("/robots.txt", "text/plain"),
        ("/sitemap.xml", "application/xml"),
        ("/_home/home.css", "text/css"),
        ("/_home/mark.svg", "image/svg+xml"),
        ("/_home/og.png", "image/png"),
        ("/_home/shot-console.webp", "image/webp"),
        ("/_home/shot-kilele.webp", "image/webp"),
        ("/_home/owner-brief.webp", "image/webp"),
        ("/_home/owner-decide.webp", "image/webp"),
        ("/_home/owner-live.webp", "image/webp"),
        ("/_home/owner-ask.webp", "image/webp"),
        ("/_home/owner.js", "text/javascript"),
        ("/_home/guide.js", "text/javascript"),
    ] {
        let res = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let ct = res.headers()["content-type"].to_str().unwrap();
        assert!(ct.starts_with(kind), "{path}: {ct}");
    }
    let res = app
        .oneshot(Request::get("/_home/nope.css").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn as_the_front_door_home_redirects_to_the_root() {
    let home = home(Facts::default());
    let app = home.routes_as_front_door();
    let res = app
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .oneshot(Request::get("/home").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(res.headers()["location"], "/");
}

/// voice.md's "Never on screen" list, until W7's copy lint runs over every crate.
#[test]
fn the_words_keep_to_voice_md() {
    let banned = [
        "seamless",
        "effortless",
        "elevate",
        "empower",
        "unlock",
        "unleash",
        "supercharge",
        "revolution",
        "game-changer",
        "cutting-edge",
        "state-of-the-art",
        "next-generation",
        "world-class",
        "best-in-class",
        "robust",
        "leverage",
        "utilize",
        "delve",
        "journey",
        "landscape",
        "tapestry",
        "vibrant",
        "testament",
        "pivotal",
        "crucial",
        "foster",
        "harness",
        "streamline",
        "synergy",
        "holistic",
        "magic",
        "delightful",
        "stunning",
        "breathtaking",
        "nestled",
        "rest assured",
        "look no further",
        "at your fingertips",
        "chatbot",
        "oops",
        "\u{2014}",
    ];
    for (name, text) in [
        ("home.html", include_str!("../templates/home.html")),
        ("base.html", include_str!("../templates/base.html")),
        ("privacy.html", include_str!("../templates/privacy.html")),
        ("terms.html", include_str!("../templates/terms.html")),
        ("crawler.html", include_str!("../templates/crawler.html")),
        ("film.html", include_str!("../templates/film.html")),
        ("samples.json", include_str!("../data/samples.json")),
    ] {
        // `_source` notes name files, not words on screen.
        let lower: String = text
            .lines()
            .filter(|l| !l.contains("\"_source\""))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        for word in banned {
            assert!(!lower.contains(word), "{name} says {word:?}");
        }
        assert!(!text.contains("! "), "{name} has an exclamation mark");
    }
}

async fn get(app: &axum::Router, path: &str, range: Option<&str>) -> axum::response::Response {
    let mut req = Request::get(path);
    if let Some(range) = range {
        req = req.header("range", range);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn text(res: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn without_a_film_directory_the_hero_shows_its_poster() {
    let home = home(Facts::default());
    let app = home.routes().merge(home.front_door());
    let html = text(get(&app, "/", None).await).await;
    assert!(html.contains(r#"<img src="/_home/hero-poster.webp?v="#));
    assert!(html.contains("aria-label=\"Sample story, made with AI\""));
    assert!(!html.contains("<video"));
    assert!(
        !html.contains("hero.js"),
        "no script without a film to play"
    );
    assert_eq!(
        get(&app, "/film", None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/_home/film/hero.mp4", None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        !text(get(&app, "/sitemap.xml", None).await)
            .await
            .contains("/film")
    );
    for file in ["hero-poster.webp", "film-poster.webp", "hero.js"] {
        let path = format!("/_home/{file}");
        assert_eq!(
            get(&app, &path, None).await.status(),
            StatusCode::OK,
            "{file}"
        );
    }
}

#[tokio::test]
async fn with_the_film_on_disk_the_hero_plays_it_and_film_has_a_page() {
    let dir = std::env::temp_dir().join(format!("agentrix-home-film-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bytes: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    for f in ["hero.mp4", "hero.webm", "film.mp4"] {
        std::fs::write(dir.join(f), &bytes).unwrap();
    }
    std::fs::write(dir.join("secret.txt"), b"not for the web").unwrap();
    let film = agentrix_home::film::Film::open(&dir).expect("hero.mp4 is there");
    let home = home_with_film(Facts::default(), Some(film));
    let app = home.routes().merge(home.front_door());

    let res = get(&app, "/", None).await;
    let csp = res.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        csp.contains("media-src 'self'") && csp.contains("script-src 'self'"),
        "{csp}"
    );
    let html = text(res).await;
    assert!(html.contains("<video muted loop playsinline preload=\"none\""));
    assert!(
        !html.contains("autoplay"),
        "the script decides, after asking about motion"
    );
    assert!(html.contains(r#"<source src="/_home/film/hero.webm?v="#));
    assert!(html.contains(r#"<source src="/_home/film/hero.mp4?v="#));
    assert!(html.contains(r#"<button type="button" class="film-toggle" hidden>"#));
    assert!(html.contains(r#"<script src="/_home/hero.js?v="#));
    assert!(html.contains(r#"<a href="/film">Watch the whole film</a>"#));

    let page = text(get(&app, "/film", None).await).await;
    assert!(page.contains("<video controls"));
    assert!(page.contains(r#"<source src="/_home/film/film.mp4?v="#));
    assert!(!page.contains("film.webm"), "not on disk");
    assert!(page.contains("What happens"));
    assert!(page.contains("Then the Move. The old site's problems stay behind."));
    assert!(
        text(get(&app, "/sitemap.xml", None).await)
            .await
            .contains("/film</loc>")
    );

    // Byte ranges, which Safari needs to play a video at all.
    let res = get(&app, "/_home/film/hero.mp4?v=1", Some("bytes=0-99")).await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(res.headers()["content-type"], "video/mp4");
    assert!(
        res.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(text_len(res).await, 100);
    let res = get(&app, "/_home/film/hero.webm", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "video/webm");
    for path in [
        "/_home/film/secret.txt",
        "/_home/film/film.webm",
        "/_home/film/..%2Fsecret.txt",
    ] {
        assert_eq!(
            get(&app, path, None).await.status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

async fn text_len(res: axum::response::Response) -> usize {
    axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn the_guide_is_on_every_page_and_its_tours_stop_on_parts_that_exist() {
    let page = home(Facts::default());
    let app = page.routes().merge(page.front_door());
    let guide =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/static/guide.js")).unwrap();
    // Each stop in guide.js: its page, its selector, and what that part is in the page's HTML.
    let mut pages = std::collections::BTreeMap::new();
    for (path, sel, marker) in [
        ("/", "#check", r#"id="check""#),
        ("/how-it-works", "#conversations", r#"id="conversations""#),
        ("/how-it-works", "#kernel", r#"id="kernel""#),
        ("/prices", "#pricing", r#"id="pricing""#),
        ("/samples", ".sample.s-kilele", r#"class="sample s-kilele""#),
        ("/samples", ".sample.s-kilele .walk", r#"<ol class="walk""#),
        ("/your-site", "#changes", r#"id="changes""#),
        (
            "/your-site",
            r#"[aria-labelledby="cta-title"]"#,
            r#"aria-labelledby="cta-title""#,
        ),
        ("/", "#owner", r#"id="owner""#),
    ] {
        assert!(
            guide.contains(&format!(r#"path: "{path}", sel: "#)) && guide.contains(sel),
            "guide.js stops at {path} {sel}"
        );
        if !pages.contains_key(path) {
            let res = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            pages.insert(path, String::from_utf8(body.to_vec()).unwrap());
        }
        assert!(pages[path].contains(marker), "{path} has {sel}");
    }
    // Every page has the guide's script; without it the header's mark stays the home link.
    for (path, html) in &pages {
        assert!(
            html.contains(r#"<script src="/_home/guide.js?v="#),
            "{path} loads the guide"
        );
        assert!(
            html.contains(r#"<a class="logo" href="/" aria-label="Agentrix home"><span class="mark" aria-hidden="true"></span>Agentrix</a>"#),
            "{path}'s logo is the home link"
        );
    }
    assert!(guide.contains("Tours work without AI. Asking uses this site's WebAgents."));
    assert!(guide.contains(r#""aria-label": "Open the Agentrix guide""#));
}

#[tokio::test]
async fn the_header_is_the_team_and_the_page_has_one_button() {
    let page = home(Facts::default());
    let html = page.render_home().await.unwrap();
    for (href, name) in [
        ("/your-site", "Your site"),
        ("/how-it-works", "How it works"),
        ("/samples", "Samples"),
        ("/prices", "Prices"),
    ] {
        assert!(html.contains(&format!(r#"href="{href}""#)), "{name}");
        assert!(html.contains(name), "{name}");
    }
    assert!(!html.contains("top-nav"), "no menu bar");
    assert!(
        html.contains(r#"<details class="team-sheet">"#),
        "the Team sheet works without JavaScript"
    );
    assert!(html.contains("Check my site"));
    assert!(!html.contains("Show me"));
    assert!(html.contains(
        "Check a site you own or run for free. No account required. We only read public pages."
    ));
    assert!(html.contains(r#"placeholder="zavora.ai""#));
    assert!(html.contains(r#"<script src="/_home/check.js?v="#));
    assert!(
        !html.contains("Rather talk to a person?"),
        "no person link until James publishes one"
    );
    // No desk is called AI while rules answer there.
    assert!(!html.contains("chip-ai"));
    let how = page.render_view("how.html").await.unwrap();
    assert!(how.contains(r#"aria-current="page">"#));
}

#[tokio::test]
async fn the_person_link_shows_the_published_hours() {
    let config = HomeConfig {
        origin: "https://agentrix.website".into(),
        assess_action: "/assess".into(),
        fixture_href: None,
        links: Default::default(),
        platform_host: "agentrix.website".into(),
        film: None,
        contact: agentrix_home::view::Contact::new("+254721490933", "6 a.m. to 11 p.m. EAT"),
    };
    let page = Home::new(config, Arc::new(Fixed(Facts::default()))).unwrap();
    let html = page.render_home().await.unwrap();
    assert!(html.contains(r#"href="https://wa.me/254721490933?text=Hi%2C%20I%27d%20like%20to%20talk%20about%20my%20website.""#));
    assert!(html.contains("Talk to a person"));
    assert!(
        html.contains("6 a.m. to 11 p.m. EAT.") || html.contains("We reply from 6 a.m. EAT."),
        "the hours, written as the films write times"
    );
}

/// A stand-in for W9's outbox: says whether sending is on, and keeps what it was asked to send.
struct Sender(bool, std::sync::Mutex<Vec<(String, serde_json::Value)>>);

#[async_trait]
impl agentrix_home::keep::ReportSender for Sender {
    async fn send(
        &self,
        message: &str,
        _email: &str,
        context: serde_json::Value,
        _key: &str,
    ) -> anyhow::Result<bool> {
        if self.0 {
            self.1.lock().unwrap().push((message.to_owned(), context));
        }
        Ok(self.0)
    }
}

/// An email provider for tests: refuses everything when asked to, else keeps what it got.
#[derive(Default)]
struct Inbox {
    refuse: std::sync::atomic::AtomicBool,
    got: std::sync::Mutex<Vec<agentrix_messages::Envelope>>,
}

#[async_trait]
impl agentrix_messages::Transport for Inbox {
    fn readiness(&self, _channel: agentrix_messages::Channel) -> agentrix_messages::Readiness {
        agentrix_messages::Readiness::Ready
    }
    async fn deliver(&self, envelope: &agentrix_messages::Envelope) -> agentrix_messages::Delivery {
        if self.refuse.load(std::sync::atomic::Ordering::SeqCst) {
            return agentrix_messages::Delivery::Failed {
                reason: "refused".into(),
            };
        }
        self.got.lock().unwrap().push(envelope.clone());
        agentrix_messages::Delivery::Sent {
            provider: "Resend".into(),
            provider_ref: format!("re_{}", envelope.outbox_id),
        }
    }
}

/// The newsletter sign-up through W9's own outbox, as node wires it: with email off the
/// visitor is told so, and their confirm link goes once node starts with email on; with it on
/// they're told to check their inbox, and signing up twice never sends a second link.
#[tokio::test]
async fn the_newsletter_sign_up_sends_its_confirm_link_through_w9s_outbox() {
    use agentrix_home::keep::{Keep, OutboxSender};
    use agentrix_messages::{Messages, Outbox, directory::Fixed};
    let ctx = agentrix_host::testing::context().await.unwrap();
    let pool = ctx.pool.clone();
    agentrix_messages::migrate(&pool).await.unwrap();
    agentrix_home::migrate(&pool).await.unwrap();
    let now = chrono::Utc::now();
    let off = Messages::new(Outbox::new(pool.clone()), Arc::new(Fixed::default()));
    let keep =
        |m: &Messages| Keep::new(Some(pool.clone()), Some(Arc::new(OutboxSender(m.clone()))));

    let saved = keep(&off).join("amina@gmail.com", now).await;
    assert_eq!(saved.status, "saved");
    assert_eq!(
        saved.message,
        "Saved for amina@gmail.com. Email isn't switched on yet; we'll ask you to confirm when it is."
    );

    // Node restarts with Resend's key: the link that waited goes.
    let inbox = Arc::new(Inbox::default());
    let on = off.with_transport(inbox.clone());
    assert_eq!(on.send_waiting().await.unwrap(), 1);
    // Signing up again finds it sent: told to check their inbox, and nothing more goes.
    let again = keep(&on).join("Amina@gmail.com", now).await;
    assert_eq!(again.status, "sent");
    assert_eq!(again.message, "Check amina@gmail.com for a confirm link.");
    assert_eq!(inbox.got.lock().unwrap().len(), 1);

    // A new reader once email is on gets theirs straight away, with a working confirm link.
    let joined = keep(&on).join("baraka@gmail.com", now).await;
    assert_eq!(joined.status, "sent");
    let link = {
        let got = inbox.got.lock().unwrap();
        assert_eq!(got.len(), 2);
        let e = got.last().unwrap();
        assert_eq!(e.to, "baraka@gmail.com");
        assert_eq!(e.subject, "Confirm the weekly fix from Agentrix");
        let at = e
            .text
            .find("/_home/fix/confirm?token=")
            .expect("the confirm link");
        e.text[at..]
            .split_whitespace()
            .next()
            .unwrap()
            .trim_end_matches([')', '.'])
            .to_owned()
    };
    let token = link.split("token=").nth(1).unwrap();
    assert!(
        keep(&on).confirm(token, now).await,
        "the link confirms the reader"
    );

    // The provider refusing it: told to try again, never told to check their inbox.
    inbox
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let refused = keep(&on).join("chebet@gmail.com", now).await;
    assert_eq!(refused.status, "saved");
    assert!(refused.message.contains("Try again in a minute"));
    assert!(!refused.message.contains("Check"));
}

#[tokio::test]
async fn keeping_a_report_stores_the_words_agreed_to_and_says_what_happens() {
    use agentrix_home::keep::{Keep, KeepForm, REPORT_WORDS, WEEKLY_WORDS};
    use chrono::{Duration, TimeZone, Utc};
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    agentrix_home::migrate(&pool).await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 8, 10, 0, 0).unwrap();
    let form = |email: &str, weekly: bool| KeepForm {
        email: email.into(),
        weekly: weekly.then(|| "1".into()),
        assessment: "asmt_01TEST".into(),
    };

    // Sending isn't switched on yet: saved, and the visitor is told so, with the link.
    let off = Keep::new(
        Some(pool.clone()),
        Some(Arc::new(Sender(false, Default::default()))),
    );
    let kept = off
        .keep(&form("Wanjiru@Kilele.example", false), "/", now)
        .await;
    assert_eq!(kept.status, "saved");
    assert!(kept.message.contains("isn't switched on yet"));
    assert_eq!(kept.report_url, "/assess/asmt_01TEST");
    let (contact, words, weekly, weekly_words, created): (
        String,
        String,
        i64,
        Option<String>,
        String,
    ) = sqlx::query_as(
        "SELECT contact, report_words, weekly, weekly_words, created_at FROM home_contacts",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(contact, "wanjiru@kilele.example");
    assert_eq!(words, REPORT_WORDS);
    assert_eq!(
        (weekly, weekly_words),
        (0, None),
        "the weekly fix is never assumed"
    );
    assert_eq!(created, now.to_rfc3339());

    // Sent once W9's message exists; the separate tick is kept with its own words.
    let sender = Arc::new(Sender(true, Default::default()));
    let on =
        Keep::new(Some(pool.clone()), Some(sender.clone())).with_origin("https://agentrix.website");
    let kept = on.keep(&form("amani@example.org", true), "/", now).await;
    assert_eq!(kept.status, "sent");
    assert!(
        kept.message
            .contains("Check your email to confirm the weekly fix.")
    );
    let words: Option<String> = sqlx::query_scalar(
        "SELECT weekly_words FROM home_contacts WHERE contact = 'amani@example.org'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(words.as_deref(), Some(WEEKLY_WORDS));
    // W9's lead.report gets every key its example has, then the weekly fix's double opt-in.
    let token = {
        let sent = sender.1.lock().unwrap();
        assert_eq!(sent.len(), 2);
        // W9's own templates render what the keep step sends.
        for (message, ctx) in sent.iter() {
            agentrix_messages::render(message, ctx).unwrap_or_else(|e| panic!("{message}: {e}"));
        }
        let (message, ctx) = &sent[0];
        assert_eq!(message, "lead.report");
        for key in [
            "site",
            "checked_at",
            "headline",
            "findings",
            "report_url",
            "keep_url",
            "expires",
        ] {
            assert!(!ctx["check"][key].is_null(), "check.{key}");
        }
        assert_eq!(
            ctx["check"]["report_url"],
            "https://agentrix.website/assess/asmt_01TEST"
        );
        assert_eq!(ctx["consent"]["words"], REPORT_WORDS);
        let (message, ctx) = &sent[1];
        assert_eq!(message, "newsletter.confirm");
        assert_eq!(ctx["consent"]["words"], WEEKLY_WORDS);
        let url = ctx["confirm_url"].as_str().unwrap();
        assert!(url.starts_with("https://agentrix.website/_home/fix/confirm?token="));
        url.split("token=").nth(1).unwrap().to_owned()
    };
    assert!(on.confirm(&token, now).await, "the confirm link works");
    let kept: Option<String> =
        sqlx::query_scalar("SELECT delete_after FROM home_contacts WHERE token = ?")
            .bind(&token)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(kept.is_none(), "a confirmed reader is kept until they stop");
    assert!(
        on.stop(&token).await,
        "one tap stops it and deletes the details"
    );
    assert!(!on.stop(&token).await);

    assert_eq!(
        on.keep(&form("not an email", false), "/", now).await.status,
        "invalid"
    );

    // Kept 30 days, as the form says.
    assert_eq!(on.purge(now + Duration::days(29)).await.unwrap(), 0);
    assert_eq!(
        on.purge(now + Duration::days(31)).await.unwrap(),
        1,
        "the reader who stopped is gone already"
    );
}

#[tokio::test]
async fn the_seven_screens_show_only_what_is_true() {
    let page = home(Facts::default());
    let html = page.render_home().await.unwrap();
    // Screen 2: the team, each state from records; an idle team says so.
    for name in [
        "Your site",
        "How it works",
        "Samples",
        "Prices",
        "Guardian",
        "Sales",
        "Editor",
        "Chief",
    ] {
        assert!(
            html.contains(&format!(r#"<p class="team-name">{name} "#)),
            "{name}"
        );
    }
    assert!(html.contains("No sites checked yet."));
    assert!(html.contains("The Guardian hasn&#x27;t attacked this site yet."));
    assert!(html.contains("Idle. Nobody has asked for a report yet."));
    assert!(
        !html.contains("They are AI"),
        "no desk on this site uses a model yet"
    );
    // Screen 3: the film plays only on a tap; chapters as text.
    assert!(html.contains("How an Agentrix Website can transform your business"));
    assert!(html.contains(r#"data-at="54">0:54"#));
    // Screen 4: labelled samples, one thing to check on each, no outcome numbers.
    assert_eq!(
        html.matches(r#"aria-label="Sample, fictional business""#)
            .count(),
        3
    );
    assert!(html.contains("It says what&#x27;s over budget."));
    assert!(html.contains("The answer quotes the paper&#x27;s correction."));
    // Screen 5: the owner console, with a way to try it on a sample. Each point has its own
    // screenshot under it, each with alt text that says what it shows and a caption naming the
    // sample; owner.js turns them into a walkthrough.
    assert!(html.contains("The owner console"));
    assert!(html.contains(r#"href="/samples/">Try the owner's side of a sample"#));
    assert!(html.contains("<ul class=\"owner-ways\" data-walk>"));
    for (way, sample) in [
        ("brief", "Classy Perfumes"),
        ("decide", "Kilele Bakery"),
        ("live", "Kilele Bakery"),
        ("ask", "Classy Perfumes"),
    ] {
        let at = html
            .find(&format!(r#"<li class="owner-way" data-way="{way}">"#))
            .unwrap_or_else(|| panic!("the {way} point"));
        let item = &html[at..at + html[at..].find("</li>").unwrap()];
        assert!(
            item.contains(&format!(r#"<img src="/_home/owner-{way}.webp?v="#)),
            "{way}'s screenshot sits under it"
        );
        let shot = &item[item.find("owner-frame").unwrap()..];
        let alt = &shot[shot.find(r#"" alt=""#).unwrap() + 7..];
        let alt = &alt[..alt.find('"').unwrap()];
        assert!(alt.len() > 120 && alt.contains(sample), "{way}: {alt}");
        assert!(item.contains(&format!("On the {sample} sample.</figcaption>")));
    }
    assert!(
        !html.contains("shot-console.webp"),
        "the old single screenshot is gone"
    );
    assert!(html.contains(r#"<script src="/_home/owner.js?v="#));
    // Screen 6: the lowest plan, from W3's catalogue at render time, with hosting, the
    // first-month offer and every WebAgent on it (the Chief included) in the roster's words.
    assert!(html.contains("Plans start at USD "));
    assert!(html.contains("Web hosting included"));
    assert!(html.contains("USD 50 of AI in your first month"));
    assert_eq!(html.matches("<li><b>Chief</b>").count(), 3);
    // Screen 7: the second box runs the same check.
    assert_eq!(html.matches("Check my site").count(), 2);
    assert_eq!(html.matches("form class=\"paste\" data-check").count(), 2);
}

/// A stand-in for the Model Gateway: replies with the JSON it was given, and remembers the
/// WebAgent each call named.
struct Model {
    reply: String,
    asked_as: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl agentrix_home::desks::DeskModel for Model {
    async fn reply(
        &self,
        webagent: &str,
        _system: &str,
        user: &str,
        _schema: &serde_json::Value,
    ) -> anyhow::Result<String> {
        assert!(
            user.contains("Numbered sources:"),
            "the model reads numbered sources"
        );
        self.asked_as.lock().unwrap().push(webagent.to_owned());
        Ok(self.reply.clone())
    }
}

struct Refuse;

#[async_trait]
impl agentrix_home::desks::DeskDials for Refuse {
    async fn may_answer(&self, _webagent: &str) -> bool {
        false
    }
}

fn with_model(reply: serde_json::Value) -> (Home, Arc<Model>) {
    let model = Arc::new(Model {
        reply: reply.to_string(),
        asked_as: Default::default(),
    });
    let page = home(Facts::default())
        .with_desks(agentrix_home::desks::Desks::new(Some(model.clone()), None));
    (page, model)
}

#[tokio::test]
async fn a_desk_answers_through_the_model_and_cites_this_site() {
    let (page, model) = with_model(serde_json::json!({
        "desk": "how-it-works", "covered": true, "step": "check",
        "answer": "The four steps in dashed boxes run in **sealed rooms** on a separate Vultr machine that holds no keys [2].",
    }));
    let know = page.knowledge().await;
    assert!(
        know.facts.iter().any(|(d, s)| *d == "samples"
            && s.label.contains("a hair salon")
            && s.text.contains("illustration")),
        "the salon film is a Samples source, marked as an illustration"
    );
    assert!(
        know.sections.iter().any(|s| s.href == "/how-it-works#move"),
        "How it works is a source"
    );
    let a = page
        .ask(
            "how-it-works",
            "Where does my old site run during the move?",
            &[],
        )
        .await
        .unwrap();
    assert!(a.ai, "a model wrote it");
    assert!(a.covered);
    assert_eq!(
        a.footer,
        "AI WebAgent · answers only from this site's pages"
    );
    assert_eq!(a.sources.len(), 1, "only cited sources are kept");
    assert_eq!(a.sources[0].n, 1, "renumbered from 1");
    assert!(a.html.contains("<b>sealed rooms</b>"));
    assert!(a.html.contains(r#"<a class="cite""#));
    assert_eq!(a.step.as_ref().unwrap().href, "/#check");
    assert_eq!(
        model.asked_as.lock().unwrap().as_slice(),
        ["desk-how-it-works"],
        "the call names the desk"
    );
    // Only then does the page say the desks are AI.
    let html = page.render_home().await.unwrap();
    assert!(html.contains("The desks that talk to you are AI."));
    assert!(html.contains(r#"<span class="ai-chip">AI</span>"#));
}

#[tokio::test]
async fn a_number_no_source_holds_falls_back_to_the_rules() {
    let (page, _) = with_model(serde_json::json!({
        "desk": "prices", "covered": true, "step": "none",
        "answer": "Plans start at **USD 4,321** a month [1].",
    }));
    let a = page
        .ask("prices", "What's the cheapest plan?", &[])
        .await
        .unwrap();
    assert!(!a.ai, "the model's answer was dropped");
    assert_eq!(a.footer, "Quoted from this site's pages");
    assert!(
        a.sources[0].label.starts_with("Price list"),
        "{:?}",
        a.sources
    );
    // The page stops saying the desks are AI, and the model rests: the next answer is the rules' too.
    assert!(
        !page
            .render_home()
            .await
            .unwrap()
            .contains("The desks that talk to you are AI.")
    );
    let again = page.ask("prices", "How do I pay?", &[]).await.unwrap();
    assert!(!again.ai);
}

#[tokio::test]
async fn without_a_model_or_with_the_rules_refusing_the_desks_quote_this_site() {
    let page = home(Facts::default());
    let a = page
        .ask("how-it-works", "Can I undo it after going live?", &[])
        .await
        .unwrap();
    assert!(!a.ai);
    assert!(a.covered);
    assert!(!a.footer.contains("AI"), "rules never call themselves AI");
    assert!(
        a.sources[0].href.starts_with("/how-it-works") || a.sources[0].href == "/",
        "{:?}",
        a.sources
    );
    let page = home(Facts::default()).with_desks(agentrix_home::desks::Desks::new(
        Some(Arc::new(Model {
            reply: "{}".into(),
            asked_as: Default::default(),
        })),
        Some(Arc::new(Refuse)),
    ));
    let a = page.ask("prices", "Is the move free?", &[]).await.unwrap();
    assert!(!a.ai, "the desk's rules said no, so no model was called");
    // Nothing on this site answers: it says so and offers the next step.
    let a = page.ask("samples", "zzqx", &[]).await.unwrap();
    assert!(!a.covered);
    assert!(
        a.html
            .contains("The site doesn&#x27;t say, so I won&#x27;t guess.")
    );
    assert!(a.step.is_some());
    assert!(page.ask("no-such-desk", "hi", &[]).await.is_none());
}

#[tokio::test]
async fn payment_a_customer_s_question_and_a_move_s_time_get_real_answers() {
    // The rules alone.
    let page = home(Facts::default());
    let a = page
        .ask("prices", "Can I pay with M-Pesa?", &[])
        .await
        .unwrap();
    assert!(a.covered);
    assert_eq!(a.sources[0].label, "Price list · How to pay");
    assert!(a.html.contains("M-Pesa"), "{}", a.html);
    let a = page
        .ask("your-site", "Do you deliver on Sundays?", &[])
        .await
        .unwrap();
    assert_eq!(a.desk, "your-site");
    assert!(!a.covered);
    assert_eq!(a.phrases, [agentrix_home::desks::CHECK_FIRST]);
    assert_eq!(a.step.as_ref().unwrap().href, "/#check");
    let a = page
        .ask("your-site", "Will it change my site?", &[])
        .await
        .unwrap();
    assert!(a.covered, "a question about the check is still answered");
    let a = page
        .ask("how-it-works", "How long does a move take?", &[])
        .await
        .unwrap();
    assert!(a.covered);
    assert!(a.html.contains("within 10 minutes"), "{}", a.html);

    // Through the model: a customer's question at the Your site desk waits for the check, even
    // when the model found a sample's answer; at the Samples desk the same reply stands.
    let reply = serde_json::json!({
        "desk": "samples", "theirs": true, "covered": true, "step": "page",
        "answer": "Kilele Bakery delivers on **Saturday mornings** [1].",
    });
    let (page, _) = with_model(reply.clone());
    let a = page
        .ask("your-site", "Do you deliver on Sundays?", &[])
        .await
        .unwrap();
    assert!(a.ai);
    assert_eq!(a.desk, "your-site");
    assert_eq!(a.phrases, [agentrix_home::desks::CHECK_FIRST]);
    assert_eq!(a.step.as_ref().unwrap().label, "Check my site");
    let (page, _) = with_model(reply);
    let a = page
        .ask("samples", "Does Kilele deliver on Sundays?", &[])
        .await
        .unwrap();
    assert!(a.covered && a.ai);
    assert_eq!(a.desk, "samples");
}

#[tokio::test]
async fn the_desks_answer_as_json_as_a_page_and_in_a_peek() {
    let page = home(Facts::default());
    let app = page.routes().merge(page.front_door());
    let res = app
        .clone()
        .oneshot(
            Request::post("/_home/ask")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"desk":"prices","q":"How do I pay?","history":[{"who":"visitor","text":"hi"}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
    assert_eq!(json["desk"], "prices");
    assert_eq!(json["ai"], false);
    let res = get(&app, "/_home/ask?desk=prices&q=How+do+I+pay%3F", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["x-robots-tag"], "noindex");
    let html = text(res).await;
    assert!(html.contains("How do I pay?"));
    assert!(html.contains("Ask Prices something else"));
    let res = get(&app, "/_home/peek/prices", None).await;
    assert_eq!(res.status(), StatusCode::OK);
    let peek = text(res).await;
    assert!(peek.contains(r#"role="dialog""#));
    assert!(peek.contains("I run a very small business. What fits?"));
    assert!(peek.contains("Answers come from this site's pages."));
    assert_eq!(
        get(&app, "/_home/peek/nope", None).await.status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        text(get(&app, "/robots.txt", None).await)
            .await
            .contains("Disallow: /_home/ask")
    );
    // Each desk's page has its ask box.
    for (path, name) in [
        ("/how-it-works", "Ask How it works"),
        ("/samples", "Ask Samples"),
        ("/prices", "Ask Prices"),
    ] {
        assert!(
            text(get(&app, path, None).await).await.contains(name),
            "{path}"
        );
    }
}

/// A stand-in for node's CAS.
#[derive(Default)]
struct Cas(std::sync::Mutex<std::collections::HashMap<String, bytes::Bytes>>);

#[async_trait]
impl agentrix_host::Blobs for Cas {
    async fn put(&self, bytes: bytes::Bytes) -> anyhow::Result<String> {
        let d = format!("sha256:{:064x}", bytes.len());
        self.0.lock().unwrap().insert(d.clone(), bytes);
        Ok(d)
    }
    async fn get(&self, digest: &str) -> anyhow::Result<Option<bytes::Bytes>> {
        Ok(self.0.lock().unwrap().get(digest).cloned())
    }
}

#[tokio::test]
async fn after_the_check_the_your_site_desk_answers_from_the_visitor_s_own_pages() {
    use agentrix_host::Blobs as _;
    let pool = control_db().await;
    let cas = Arc::new(Cas::default());
    let page = cas
        .put(bytes::Bytes::from_static(
            b"<html><head><title>Delivery</title></head><body><h1>Delivery</h1><p>We deliver in Westlands and Kilimani every day except Sunday. Orders before 2 p.m. go out the same day.</p><script>steal()</script></body></html>",
        ))
        .await
        .unwrap();
    let report = serde_json::json!({
        "summary": "We read 12 pages and found 1 problem that needs attention now.",
        "pages": [{ "url": "https://kilele-bakery.example/delivery/", "digest": page }],
    });
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sites (site_id, domain, name, old_origin, created_at) VALUES ('site_x', 'kilele-bakery.example', 'Kilele', 'https://kilele-bakery.example', '2026-11-08T10:00:00Z')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO assessments (assessment_id, site_id, url, state, report, created_at, finished_at) VALUES ('asmt_talk', 'site_x', 'https://kilele-bakery.example/', 'report-ready', ?, '2026-11-08T10:00:00Z', '2026-11-08T10:02:00Z')")
        .bind(report.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let page = home(Facts::default()).with_node(pool.clone(), cas.clone());
    let src = page
        .talk_sources("asmt_talk")
        .await
        .expect("the check's sources");
    assert_eq!(src.site, "kilele-bakery.example");
    assert_eq!(src.pages.len(), 1);
    assert_eq!(src.pages[0].label, "Your page · Delivery");
    assert!(src.pages[0].text.contains("except Sunday"));
    assert!(
        !src.pages[0].text.contains("steal()"),
        "scripts are never read"
    );
    assert!(page.talk_sources("asmt_nope").await.is_none());

    // A customer's question, answered as the new site would, from the owner's own page.
    let model = Arc::new(Model {
        reply: serde_json::json!({ "covered": true, "step": "keep", "answer": "Not on Sundays: Kilele delivers in **Westlands and Kilimani** every other day [2]." }).to_string(),
        asked_as: Default::default(),
    });
    let page = page.with_desks(agentrix_home::desks::Desks::new(Some(model.clone()), None));
    let know = agentrix_home::desks::Knowledge {
        facts: vec![],
        sections: vec![],
        person: None,
    };
    let a = page
        .desks()
        .talk("Do you deliver on Sundays?", &[], &src, &know)
        .await;
    assert!(a.ai);
    assert_eq!(a.sources[0].href, "https://kilele-bakery.example/delivery/");
    assert_eq!(
        a.footer,
        "AI WebAgent · reads only your public pages and never guesses"
    );
    assert_eq!(a.step.unwrap().label, "Keep my new site");
    assert_eq!(
        model.asked_as.lock().unwrap().as_slice(),
        ["desk-your-site"]
    );

    // James's Chief: its card says when it last wrote his brief, from W1's briefs row, never what it said.
    sqlx::query("INSERT INTO briefs (site_id, written_at, body) VALUES ('agentrix-home', '2026-11-08T04:00:00.000000Z', '{\"headline\":\"private\"}')")
        .execute(&pool)
        .await
        .unwrap();
    let html = page.render_home().await.unwrap();
    assert!(
        html.contains("Wrote James&#x27;s brief Sun 8 Nov, 07:00 EAT."),
        "the Chief's state"
    );
    assert!(!html.contains("private"), "the brief's words stay James's");

    // Without a model, a question the pages don't answer gets the spec's words, not a guess.
    let plain = home(Facts::default()).with_node(pool.clone(), cas);
    let a = plain.desks().talk("zzqx", &[], &src, &know).await;
    assert!(!a.ai);
    assert!(
        a.html
            .contains("Your site doesn&#x27;t say, so your new site wouldn&#x27;t guess.")
    );
}

#[tokio::test]
async fn the_editor_drafts_the_weekly_fix_and_it_goes_out_only_when_james_approves() {
    use agentrix_decisions::EffectRunner as _;
    use agentrix_home::newsletter::{Newsletter, NewsletterRunner};
    use chrono::{TimeZone, Utc};
    let ctx = agentrix_host::testing::context().await.unwrap();
    let pool = ctx.pool.clone();
    for sql in [include_str!(
        "../../agentrix-node/src/db/migrations/0001_control.sql"
    )] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();
    agentrix_decisions::migrate(&pool).await.unwrap();
    agentrix_home::migrate(&pool).await.unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 12, 13, 0, 0).unwrap(); // Thursday 16:00 in Nairobi
    let at = (now - chrono::Duration::days(1)).to_rfc3339();
    for (id, title) in [
        ("asmt_a", "An old WordPress with a public login page"),
        ("asmt_b", "An old WordPress with a public login page"),
        ("asmt_c", "No structured data for your products"),
    ] {
        sqlx::query("INSERT INTO assessments (assessment_id, site_id, url, state, created_at) VALUES (?, 'site_x', 'https://x.example/', 'report-ready', ?)").bind(id).bind(&at).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO findings (finding_id, site_id, assessment_id, detector, severity, category, title, location, at) VALUES (?, 'site_x', ?, 'd', 'warn', 'security', ?, '/', ?)")
            .bind(format!("f_{id}")).bind(id).bind(title).bind(&at).execute(&pool).await.unwrap();
    }
    let sender = Arc::new(Sender(true, Default::default()));
    let keep = agentrix_home::keep::Keep::new(Some(pool.clone()), Some(sender.clone()));
    let model = |reply: serde_json::Value| -> Arc<dyn agentrix_home::desks::DeskModel> {
        Arc::new(Model {
            reply: reply.to_string(),
            asked_as: Default::default(),
        })
    };
    let editor = |reply| {
        Newsletter::new(
            Some(pool.clone()),
            Some(model(reply)),
            Some(agentrix_decisions::Decisions::new(&ctx)),
            Some(sender.clone()),
            "https://agentrix.website",
        )
    };

    // No confirmed reader yet: no draft.
    let draft = serde_json::json!({ "title": "Hide your WordPress login page", "lines": ["Change the login address from /wp-login.php.", "Turn on two-step sign-in."] });
    assert!(!editor(draft.clone()).draft_week(now).await.unwrap());
    // Screen 7's sign-up asks to confirm first; once confirmed, the reader counts.
    let joined = keep.join("reader@example.org", now).await;
    assert_eq!(joined.status, "sent");
    let token = {
        let sent = sender.1.lock().unwrap();
        let (message, ctx) = sent.last().unwrap();
        assert_eq!(message, "newsletter.confirm");
        agentrix_messages::render(message, ctx).unwrap();
        ctx["confirm_url"]
            .as_str()
            .unwrap()
            .split("token=")
            .nth(1)
            .unwrap()
            .to_owned()
    };
    assert!(keep.confirm(&token, now).await);

    // A draft that states a number no check found is dropped.
    let bad = serde_json::json!({ "title": "Fix 7 things", "lines": ["Do 7 things."] });
    assert!(editor(bad).draft_week(now).await.is_err());

    // The Editor drafts from the week's findings; it waits for James as a card.
    let n = editor(draft.clone());
    assert!(n.draft_week(now).await.unwrap());
    assert!(!n.draft_week(now).await.unwrap(), "one draft a week");
    let issues = n.issues().await;
    assert!(issues.waiting);
    let page = home(Facts::default()).with_newsletter(n.clone());
    let html = page.render_home().await.unwrap();
    // The page promises the newsletter; how an issue is approved stays off it.
    assert!(html.contains("Our Friday newsletter on what an ordinary website can become"));
    assert!(!html.contains("class=\"fix-state\""));
    let card: String =
        sqlx::query_scalar("SELECT card FROM decision_cards_v2 WHERE site_id = 'agentrix-home'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let card: agentrix_decisions::DecisionCardV2 = serde_json::from_str(&card).unwrap();
    assert_eq!(card.effect.tool, "newsletter.send");
    assert_eq!(card.title, "Send this week's website fix to 1 reader?");
    let before = sender.1.lock().unwrap().len();

    // James approves: it goes to the confirmed reader, with the one-tap stop.
    NewsletterRunner(n.clone())
        .run(&card, "sha256:r")
        .await
        .unwrap();
    let (count, (message, ctx)) = {
        let sent = sender.1.lock().unwrap();
        (sent.len(), sent.last().cloned().unwrap())
    };
    assert_eq!(count, before + 1);
    let (message, ctx) = (&message, &ctx);
    assert_eq!(message, "newsletter.issue");
    agentrix_messages::render(message, ctx).unwrap();
    assert_eq!(ctx["issue"]["title"], "Hide your WordPress login page");
    assert!(
        ctx["unsubscribe_url"]
            .as_str()
            .unwrap()
            .contains("/_home/fix/stop?token=")
    );
    let issues = n.issues().await;
    assert!(!issues.waiting);
    assert_eq!(
        issues.last_sent.as_deref(),
        Some("Hide your WordPress login page")
    );
}
