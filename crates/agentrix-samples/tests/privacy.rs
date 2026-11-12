//! Each sample has a privacy page in its own look, linked from its footer, with a way to report
//! a site pretending to be it (04-ui-parity "Fix first" 8).

use agentrix_samples::{App, Config};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

async fn samples() -> anyhow::Result<Router> {
    let mut ctx = agentrix_host::testing::context().await?;
    ctx.platform_host = "localhost".into();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = Config {
        data_dir: std::env::temp_dir()
            .join(format!("agentrix-samples-privacy-{}", std::process::id())),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: None,
        reseed: true,
    };
    let app = App::new(config, &ctx)?;
    let host = agentrix_samples::mounted(app, Router::new())
        .hosts
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no sample host"))?;
    Ok(host.router)
}

async fn get(router: &Router, host: &str, path: &str) -> anyhow::Result<(StatusCode, String)> {
    let req = Request::get(path)
        .header(header::HOST, host)
        .body(Body::empty())?;
    let res = router.clone().oneshot(req).await?;
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await?;
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

#[tokio::test]
async fn every_sample_has_a_privacy_page_in_its_own_look() -> anyhow::Result<()> {
    let router = samples().await?;
    for (host, look, keeps) in [
        (
            "kilele-bakery-sample.localhost",
            "class=\"b-pagehead\"",
            "A hold keeps only the name you give it.",
        ),
        (
            "classy-perfumes-sample.localhost",
            "class=\"cp-h1\"",
            "A gift plan lives only in its link",
        ),
        (
            "tana-valley-times-sample.localhost",
            "class=\"front-head\"",
            "If you ask for the Friday five, the site keeps your email and the topics you pick.",
        ),
    ] {
        let (status, body) = get(&router, host, "/privacy").await?;
        assert_eq!(status, StatusCode::OK, "{host}: {body}");
        assert!(body.contains(look), "{host} lacks its own look");
        assert!(body.contains(keeps), "{host} lacks {keeps:?}");
        assert!(body.contains("<title>Privacy · "), "{host} names the page");
        assert!(
            body.contains("class=\"s-band\""),
            "{host} lacks the Sample band"
        );
        // What the site keeps, with figures from the code that keeps them.
        assert!(
            body.contains(&format!("works on <b>{host}</b> only")),
            "{host}: an account works on this host only"
        );
        for words in [
            "It works once, for 10 minutes, and stops after 5 wrong tries.",
            "The site counts visits without cookies.",
            "deletes the totals after 25 months",
            "Global Privacy Control",
            "keeps you signed in for 30 days",
            "remembers if you said Not now to a passkey",
            "payments run in PayPal's sandbox",
        ] {
            assert!(body.contains(words), "{host} lacks {words:?}");
        }
        // The report: a mailto to the platform's address, naming this site.
        assert!(body.contains("id=\"report\""), "{host} has no report");
        assert!(
            body.contains(
                "href=\"mailto:hello@agentrix.website?subject=A%20site%20pretending%20to%20be%20"
            ),
            "{host} has no report link"
        );
        assert!(
            body.contains(&format!("The%20real%20site%20is%20http%3A%2F%2F{host}")),
            "{host}'s report names the real site"
        );
        // Every page's footer links the page and the report.
        let (_, home) = get(&router, host, "/").await?;
        assert!(
            home.contains("<a href=\"/privacy\">Privacy</a>"),
            "{host} footer"
        );
        assert!(
            home.contains(
                "<a href=\"/privacy#report\">Report a site pretending to be this one</a>"
            ),
            "{host} footer lacks the report link"
        );
    }
    let (_, kilele) = get(&router, "kilele-bakery-sample.localhost", "/privacy").await?;
    assert!(kilele.contains("One keeps your bag for 30 days."));
    let (_, tana) = get(&router, "tana-valley-times-sample.localhost", "/privacy").await?;
    assert!(!tana.contains("keeps your bag"), "Tana has no bag");
    Ok(())
}
