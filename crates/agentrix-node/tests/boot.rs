//! The whole node starts. Every crate's tests mount that crate alone; this one builds the
//! router the binary serves, with every module mounted together, so two crates that claim the
//! same route fail here instead of at start-up (GET /samples did, on 8 Nov).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

fn config(dir: &std::path::Path) -> agentrix_node::state::Config {
    agentrix_node::state::Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        internal_listen: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.to_path_buf(),
        origin: "http://localhost:18999".parse().unwrap(),
        // Nothing listens here: pages must still render with the gateway down.
        models_url: "http://127.0.0.1:9".into(),
        runner: None,
        fixture_domain: None,
        gauntlet_token_file: None,
    }
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
    let kind = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    (status, kind)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_whole_node_builds_and_answers() {
    let dir = tempfile::tempdir().unwrap();
    let app = agentrix_node::app::build(&config(dir.path()))
        .await
        .expect("the node builds with every module mounted");

    let platform = "localhost:18999";
    for (path, want) in [
        ("/healthz", StatusCode::OK),
        // agentrix.website's own pages (agentrix-home).
        ("/", StatusCode::OK),
        ("/samples", StatusCode::OK),
        ("/your-site", StatusCode::OK),
        ("/how-it-works", StatusCode::OK),
        ("/prices", StatusCode::OK),
        // The sample owners' views (agentrix-samples) start one slash further on.
        ("/samples/", StatusCode::OK),
    ] {
        let (status, _) = get(&app.public, platform, path).await;
        assert_eq!(status, want, "GET {path} on the platform host");
    }

    // A sample site answers on its own host.
    let (status, kind) = get(&app.public, "kilele-bakery-sample.localhost:18999", "/").await;
    assert_eq!(status, StatusCode::OK, "Kilele's home page");
    assert!(
        kind.starts_with("text/html"),
        "Kilele's home page is HTML: {kind}"
    );
}
