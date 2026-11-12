//! Every page renders in every sample state under strict mode, and no `Option` leaks as "none".

mod common;

use agentrix_console::{Console, Page};

#[test]
fn every_template_loads() {
    Console::new().expect("templates parse");
    assert_eq!(Page::ALL.len(), 13);
}

#[test]
fn every_page_renders_in_every_state() {
    let console = Console::new().expect("templates parse");
    for (name, page, view) in common::all() {
        let html = console
            .render(page, &view)
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert!(
            html.starts_with("<!doctype html>"),
            "{name}: not a full page"
        );
        // SVG icons legitimately say fill="none"; anything else that prints "none" is a leaked Option.
        let text = html.replace("fill=\"none\"", "");
        for leak in [">none<", " none ", "=\"none\"", "· none"] {
            assert!(
                !text.contains(leak),
                "{name}: an Option printed as \"none\" ({leak})"
            );
        }
        assert!(
            html.contains("<main id=\"main\""),
            "{name}: no main landmark"
        );
    }
}

#[test]
fn fixture_is_labelled_and_frames_say_what_they_are() {
    let console = Console::new().unwrap();
    let html = console
        .render(Page::Assess, &common::assess_running())
        .unwrap();
    assert!(
        html.contains("Zavora's fixture"),
        "the fixture must be labelled (R3.3, honesty rule)"
    );
    assert!(
        html.contains("Sealed screenshot, about 1 a second"),
        "a sealed screenshot is never called live"
    );
    assert!(!html.contains(">Live<"));
}

#[test]
fn go_live_shows_locked_positions_and_both_approval_methods() {
    let console = Console::new().unwrap();
    let html = console
        .render(
            Page::Decision,
            &common::go_live(agentrix_console::view::CardState::Waiting),
        )
        .unwrap();
    assert!(
        html.contains("class=\"locked\""),
        "positions above the ceiling are locked (ADR-012)"
    );
    assert!(html.contains("Messages customers, so it always asks first"));
    assert!(html.contains("Approve with your passkey"));
    assert!(
        html.contains("Approve with code"),
        "an authenticator code works without a passkey (D9)"
    );
    assert!(
        html.contains("data-options="),
        "the ceremony's options ride on the page"
    );
}

#[test]
fn user_text_is_escaped() {
    let console = Console::new().unwrap();
    let mut view = common::start();
    view.error = Some("<script>alert(1)</script>".into());
    let html = console.render(Page::Start, &view).unwrap();
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&lt;script&gt;"));
    view.error = Some(r#"a "quoted" & 'single' </p>"#.into());
    let html = console.render(Page::Start, &view).unwrap();
    assert!(html.contains("a &quot;quoted&quot; &amp; &#x27;single&#x27; &lt;/p&gt;"));
}

#[test]
fn urls_keep_their_slashes() {
    let console = Console::new().unwrap();
    let html = console
        .render(
            Page::Decision,
            &common::go_live(agentrix_console::view::CardState::Waiting),
        )
        .unwrap();
    assert!(
        !html.contains("&#x2f;"),
        "a URL in the page source should read as a URL"
    );
    assert!(
        html.contains("action=\"/decisions/"),
        "the code form posts to the approve route"
    );
}

#[test]
fn a_missing_field_is_an_error_not_a_blank() {
    let console = Console::new().unwrap();
    let err = console.render(Page::Today, &serde_json::json!({ "greeting": "Hi" }));
    assert!(
        err.is_err(),
        "strict mode must refuse a view that lacks fields"
    );
}

#[test]
fn sign_in_offers_the_password_path_when_passkeys_are_off() {
    let console = Console::new().unwrap();
    let on = console.render(Page::SignIn, &common::auth(true)).unwrap();
    assert!(on.contains("Sign in with a passkey"));
    let off = console.render(Page::SignIn, &common::auth(false)).unwrap();
    assert!(!off.contains("Sign in with a passkey"));
    assert!(
        off.contains("<details open"),
        "without passkeys the password path is open"
    );
    let reg = console.render(Page::Register, &common::auth(true)).unwrap();
    assert!(reg.contains("Create with a passkey") && reg.contains("/static/auth.js"));
}

#[test]
fn a_flagged_page_can_be_accepted_as_it_is() {
    use agentrix_console::view::{ParityPage, ParityState};
    let console = Console::new().unwrap();
    let mut view = common::move_running();
    let html = console.render(Page::Move, &view).unwrap();
    assert!(
        !html.contains("Accept this page as it is"),
        "only flagged pages offer it"
    );
    view.parity.as_mut().unwrap().pages.push(ParityPage {
        url: "/journal/".into(),
        state: ParityState::Flagged,
        score_words: "Flagged for you".into(),
        verdict: "The post list is shorter.".into(),
    });
    let html = console.render(Page::Move, &view).unwrap();
    assert!(
        html.contains("action=\"/moves/mv_1/accept\""),
        "R6.4: the owner accepts a flagged page"
    );
    assert!(html.contains("name=\"page\" value=\"/journal/\""));
}

#[test]
fn today_lists_the_questions_visitors_passed() {
    let console = Console::new().expect("templates parse");
    let html = console.render(Page::Today, &common::today()).unwrap();
    assert!(html.contains("Questions visitors passed to you"), "{html}");
    assert!(html.contains("Reply to 0700 000 000"));
    assert!(html.contains("No way to reply was left"));
    // A visitor's words are text, never markup.
    assert!(html.contains("&lt;b&gt;sugar-free&lt;/b&gt;"), "escaped");
    assert!(!html.contains("<b>sugar-free</b>"));
    let first = console.render(Page::Today, &common::today_first()).unwrap();
    assert!(!first.contains("Questions visitors passed to you"));
}

#[test]
fn a_live_site_offers_undo_in_one_step() {
    use agentrix_console::view::MoveState;
    let console = Console::new().unwrap();
    let html = console.render(Page::Move, &common::move_running()).unwrap();
    assert!(
        !html.contains("/rollback"),
        "undo appears only once the site is live"
    );
    let mut live = common::move_running();
    live.run.state = MoveState::Live;
    let html = console.render(Page::Move, &live).unwrap();
    assert!(
        html.contains("/rollback\"><input type=\"hidden\" name=\"to\" value=\"previous\""),
        "R7.4: back to the previous release"
    );
    assert!(
        html.contains("name=\"to\" value=\"old-site\""),
        "R7.4: back to the old site"
    );
}

#[test]
fn a_stopped_move_can_start_again() {
    use agentrix_console::view::MoveState;
    let console = Console::new().unwrap();
    let mut stopped = common::move_running();
    stopped.run.state = MoveState::Stopped;
    let html = console.render(Page::Move, &stopped).unwrap();
    assert!(
        html.contains("The Move stopped"),
        "the heading says what happened"
    );
    assert!(
        html.contains("action=\"/sites/site_k1/moves\""),
        "the owner can start again"
    );
    assert!(
        !html.contains("Moving kilele"),
        "a stopped Move is not 'Moving'"
    );
}

#[test]
fn a_live_site_is_linked_from_the_console() {
    use agentrix_console::view::MoveState;
    let console = Console::new().unwrap();
    let html = console.render(Page::Move, &common::move_running()).unwrap();
    assert!(!html.contains("View your site"), "no link before go-live");
    let mut live = common::move_running();
    live.run.state = MoveState::Live;
    live.chrome.site.live_url = Some("https://kilele-bakery-example.agentrix.website/".into());
    let html = console.render(Page::Move, &live).unwrap();
    assert!(html.contains("View your site"));
    assert!(html.contains(
        "Your new site is live at <a href=\"https://kilele-bakery-example.agentrix.website/\""
    ));
}

#[test]
fn an_owner_uploads_their_wordpress_export_before_the_move() {
    use agentrix_console::view::UploadRow;
    let console = Console::new().unwrap();
    let html = console.render(Page::Move, &common::move_none()).unwrap();
    assert!(
        !html.contains("Your WordPress export"),
        "the fixture's exports are already on the platform"
    );
    let mut own = common::move_none();
    own.chrome.site.fixture = false;
    own.uploads = vec![UploadRow {
        name: "export.xml".into(),
        size_words: "28 KB".into(),
        when_words: "22:10".into(),
    }];
    let html = console.render(Page::Move, &own).unwrap();
    assert!(
        html.contains("Your WordPress export"),
        "R4.1: a real site needs its export"
    );
    assert!(html.contains("data-name=\"export.xml\""));
    assert!(
        html.contains("Uploaded 28 KB · 22:10"),
        "what's uploaded is shown"
    );
    assert!(
        html.contains("Not uploaded yet"),
        "the optional files say they're missing"
    );
    assert!(html.contains("/static/upload.js"));
}
