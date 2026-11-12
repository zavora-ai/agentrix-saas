//! Every Kilele visitor page renders from the seeded records with the embedded templates
//! (strict mode, so a field a template uses but the view lacks fails here).

use agentrix_samples::{App, Config};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt as _;

async fn site(name: &str) -> anyhow::Result<Router> {
    let mut ctx = agentrix_host::testing::context().await?;
    ctx.platform_host = "localhost".into();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let config = Config {
        data_dir: std::env::temp_dir().join(format!(
            "agentrix-samples-kilele-{name}-{}",
            std::process::id()
        )),
        platform_host: "localhost".into(),
        scheme: "http".into(),
        port: None,
        bundles_dir: Some(root.join("fixtures/samples")),
        templates_dir: None,
        reseed: true,
    };
    let app = App::new(config, &ctx)?;
    let host = agentrix_samples::mounted(app, axum::Router::new())
        .hosts
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no sample host"))?;
    Ok(host.router)
}

async fn get(site: &Router, path: &str) -> anyhow::Result<(StatusCode, String)> {
    let req = Request::get(path)
        .header(header::HOST, "kilele-bakery-sample.localhost")
        .body(Body::empty())?;
    let res = site.clone().oneshot(req).await?;
    let status = res.status();
    let body = to_bytes(res.into_body(), usize::MAX).await?;
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

#[tokio::test]
async fn every_kilele_visitor_page_renders_from_records() -> anyhow::Result<()> {
    let site = site("pages").await?;
    let pages = [
        ("/bread", StatusCode::OK, "id=\"item-sourdough\""),
        ("/cakes", StatusCode::OK, "id=\"row-2tier\""),
        ("/visit", StatusCode::OK, "id=\"visit-hours\""),
        ("/delivery", StatusCode::OK, "id=\"fee-rule\""),
        ("/allergens", StatusCode::OK, "id=\"kitchen\""),
        ("/help", StatusCode::OK, "id=\"details\""),
        (
            "/shop/weekend-bake-box",
            StatusCode::OK,
            "action=\"/bag/add\"",
        ),
        (
            "/shop/no-such-bake",
            StatusCode::NOT_FOUND,
            "id=\"not-found\"",
        ),
        ("/no/such/page", StatusCode::NOT_FOUND, "id=\"dockForm\""),
    ];
    for (path, status, marker) in pages {
        let (got, body) = get(&site, path).await?;
        assert_eq!(got, status, "{path}: {body}");
        assert!(body.contains(marker), "{path} lacks {marker}");
        assert!(
            body.contains("class=\"s-band\""),
            "{path} lacks the Sample band"
        );
    }
    Ok(())
}

#[tokio::test]
async fn sold_out_bakes_have_no_bag_button() -> anyhow::Result<()> {
    let site = site("sold-out").await?;
    let (_, body) = get(&site, "/bread").await?;
    // The seed bakes no cinnamon rolls today.
    let card = body
        .split("id=\"item-cinnamon-roll\"")
        .nth(1)
        .and_then(|rest| rest.split("</article>").next())
        .unwrap_or_default();
    assert!(card.contains("Sold out"), "cinnamon roll card: {card}");
    assert!(
        !card.contains("/bag/add"),
        "a sold-out card offers Add to bag"
    );
    let sourdough = body
        .split("id=\"item-sourdough\"")
        .nth(1)
        .and_then(|rest| rest.split("</article>").next())
        .unwrap_or_default();
    assert!(sourdough.contains("name=\"back\" value=\"/bread#item-sourdough\""));
    Ok(())
}

/// W7's copy review (8 Nov): no release numbers, no "Pickup", sources in the visitor's words.
#[tokio::test]
async fn visitor_pages_follow_the_copy_review() -> anyhow::Result<()> {
    let site = site("copy").await?;
    for path in [
        "/",
        "/bread",
        "/cakes",
        "/visit",
        "/delivery",
        "/help",
        "/order-a-cake",
        "/cakes/studio",
    ] {
        let (status, body) = get(&site, path).await?;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        for gone in [
            "Release v",
            "price list v",
            "Price list v",
            "Pickup",
            "no AI model",
        ] {
            assert!(!body.contains(gone), "{path} still says {gone:?}");
        }
    }
    let (_, home) = get(&site, "/").await?;
    assert!(
        home.contains("Kilele&#x27;s kitchen uses wheat flour")
            || home.contains("Kilele's kitchen uses wheat flour"),
        "home renders the allergen fact"
    );
    assert!(home.contains("Collection slots: Mon–Sat 08:00–12:00 and 14:00–16:00"));
    let (_, cakes) = get(&site, "/cakes").await?;
    assert!(cakes.contains("from the price list of "), "{cakes}");
    let (_, help) = get(&site, "/help").await?;
    assert!(help.contains("The Front desk, Bread, Cakes, Orders and Visit are AI WebAgents."));
    assert!(
        help.contains("<b>kilele-bakery-sample.localhost</b>"),
        "a sample names the host the visitor is on"
    );
    assert!(help.contains("usually within 2 hours (7:00–20:00)"));
    let (_, studio) = get(&site, "/cakes/studio").await?;
    assert!(
        studio.contains("You&#x27;re talking to AI. It never guesses prices, dates or allergens.")
            || studio
                .contains("You're talking to AI. It never guesses prices, dates or allergens.")
    );
    Ok(())
}
