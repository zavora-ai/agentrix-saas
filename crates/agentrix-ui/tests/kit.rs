//! The kit's contract: every layout and macro renders in strict mode with the arguments the
//! README documents, and nothing on a page needs `'unsafe-inline'`.

use minijinja::context;

fn render(source: &str, ctx: minijinja::Value) -> String {
    let mut env = agentrix_ui::environment();
    env.add_template_owned("t.html", source.to_owned())
        .expect("test template parses");
    env.get_template("t.html")
        .expect("loads")
        .render(ctx)
        .unwrap_or_else(|e| panic!("{e:#}"))
}

fn agent() -> minijinja::Value {
    context! { id => "cashier", name => "Cashier", glyph => "Ca", color => "#11734a", state => "waiting" }
}

#[test]
fn every_template_parses() {
    let env = agentrix_ui::environment();
    let names: Vec<_> = agentrix_ui::templates().map(|(n, _)| n).collect();
    for name in [
        "ui/base.html",
        "ui/kit.html",
        "ui/error.html",
        "ui/layouts/page.html",
        "ui/layouts/shell.html",
        "ui/layouts/site.html",
    ] {
        assert!(names.contains(&name), "{name} is missing");
        env.get_template(name)
            .unwrap_or_else(|e| panic!("{name}: {e:#}"));
    }
}

#[test]
fn no_template_needs_unsafe_inline() {
    for (name, source) in agentrix_ui::templates() {
        assert!(
            !source.contains(" style=\""),
            "{name} has a style attribute"
        );
        assert!(
            !source.contains(" onclick="),
            "{name} has an inline handler"
        );
        for (i, _) in source.match_indices("<script") {
            let tag = &source[i..source[i..].find('>').map_or(source.len(), |e| i + e)];
            assert!(tag.contains(" src="), "{name} has an inline script: {tag}");
        }
    }
}

#[test]
fn page_layout_renders_with_no_variables() {
    let html = render(
        r###"{% extends "ui/layouts/page.html" %}{% block title %}Start · Agentrix{% endblock %}{% block main %}<h1>Paste your website address</h1>{% endblock %}"###,
        context! {},
    );
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("/static/ui/tokens.css?v="));
    assert!(html.contains("/static/ui/kit.css?v="));
    assert!(html.contains(r###"<script type="module" src="/static/ui/kit.js?v="###));
    assert!(html.contains("<title>Start · Agentrix</title>"));
    assert!(html.contains(r###"<a class="skip" href="#main">Skip to content</a>"###));
    assert!(!html.contains("data-theme"));
    assert!(html.contains(r###"<link rel="stylesheet" href="/_agentrix/marks.css?v="###));
    assert!(html.contains(r###"<script type="module" src="/_agentrix/marks.js?v="###));
}

#[test]
fn night_time_and_theme_reach_the_root() {
    let html = render(
        r###"{% extends "ui/layouts/shell.html" %}{% block field %}<div class="field-head"><h1>Today</h1></div>{% endblock %}"###,
        context! { theme => "dark", night => true, quiet_hours => context! { from => "22:00", to => "06:00", tz => "Africa/Nairobi" } },
    );
    assert!(html.contains(r###"data-theme="dark""###));
    assert!(html.contains(r###"data-time="night""###));
    assert!(
        html.contains(
            r###"data-night-from="22:00" data-night-to="06:00" data-tz="Africa/Nairobi""###
        )
    );
    assert!(html.contains("/static/ui/shell.css?v="));
    assert!(html.contains(r###"<div class="os " data-scope="site">"###));
}

#[test]
fn sample_sites_say_sample_on_every_page() {
    let html = render(
        r###"{% extends "ui/layouts/site.html" %}{% block main %}<h1>Bread</h1>{% endblock %}"###,
        context! { site => context! { key => "kilele", name => "Kilele Bakery", sample => true } },
    );
    assert!(html.contains(r###"data-host="kilele""###));
    assert!(html.contains(r###"<meta name="color-scheme" content="light">"###));
    assert!(html.contains("sample-band"));
    assert!(html.contains("Kilele Bakery isn&#x27;t a real business"));
    assert!(html.contains("The WebAgents on this site are AI."));
    assert!(html.contains("/static/ui/site.css?v="));

    let real = render(
        r###"{% extends "ui/layouts/site.html" %}{% block main %}{% endblock %}"###,
        context! { site => context! { key => "site", name => "A real shop", sample => false, made_with => false } },
    );
    assert!(!real.contains("sample-band"));
    assert!(!real.contains("Made with Agentrix"));
    assert!(
        real.contains("The WebAgents on this site are AI."),
        "the AI line is never removable"
    );
}

#[test]
fn error_pages_say_what_to_do() {
    for (status, needle) in [
        (404, "There&#x27;s no page at this address"),
        (403, "signed in as wanjiru@example.com"),
        (500, "quote reference r-7f3a to support"),
    ] {
        let ctx = minijinja::Value::from_serialize(serde_json::json!({
            "status": status, "who": "wanjiru@example.com", "ref": "r-7f3a"
        }));
        let html = render(r###"{% include "ui/error.html" %}"###, ctx);
        assert!(html.contains(needle), "{status}: {html}");
    }
}

#[test]
fn primitives_render() {
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{{ ui.button("Approve", kind="primary") }}
{{ ui.button("Open the report", href="/reports/1", size="sm") }}
{{ ui.button("Send", type="submit", name="go", value="1", attrs={"data-x": "y"}) }}
{{ ui.chip("Verified", "ok") }}{{ ui.badge(2, "waiting") }}{{ ui.status("held", "Waiting for Wanjiru's OK") }}
{{ ui.state_line("working", "Comparing 14 prices") }}{{ ui.source("Price list, 2 Nov") }}{{ ui.source("Delivery calendar", live=true) }}
{{ ui.meter(38, 1500, label="KSh 38 of KSh 1,500 this month") }}{{ ui.meter(9, 0) }}{{ ui.meter(120, 100, tone="over") }}
{{ ui.kv([{"k": "Release", "v": "v14"}]) }}
{{ ui.tabs([{"label": "Waiting", "href": "?w", "current": true}, {"label": "Decided", "href": "?d", "current": false}], "Decisions") }}
{{ ui.segmented([{"label": "Day", "href": "?d", "current": true}], "View") }}
{{ ui.person("Wanjiru") }}{{ ui.kbd("/") }}{{ ui.logo() }}{{ ui.note("Plain note") }}
{{ ui.line("ok", "Saved") }}{{ ui.line("error", "We couldn't reach PayPal. You weren't charged.") }}"###,
        context! {},
    );
    assert!(html.contains(r###"<button class="btn btn-primary" type="button">Approve</button>"###));
    assert!(html.contains(r###"<a class="btn btn-sm" href="/reports/1">Open the report</a>"###));
    assert!(html.contains(r###"name="go" value="1" data-x="y""###));
    assert!(html.contains(r###"class="chip chip-ok""###));
    assert!(html.contains(r###"aria-label="2 waiting""###));
    assert!(html.contains(r###"data-value="38" data-max="1500""###));
    assert!(
        html.contains(r###"<span class="p0">"###),
        "a zero maximum draws an empty bar"
    );
    assert!(
        html.contains(r###"<span class="p100">"###),
        "over the maximum is capped at a full bar"
    );
    assert!(html.contains(r###"aria-current="page">Waiting"###));
    assert!(html.contains(r###"<p class="errline" role="alert">"###));
    assert!(html.contains("Source · <b>Delivery calendar</b> (live)"));
}

#[test]
fn surfaces_and_forms_render() {
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{% call ui.card(title="Orders", tone="accent") %}<p>Inside</p>{% endcall %}
{% call ui.fcard(title="Editor", sub="Comparing prices", who=agent) %}<p>Body</p>{% endcall %}
{{ ui.field("email", "Email", type="email", hint="We send a one-time code here.", autocomplete="email") }}
{{ ui.field("name", "Name", error="Tell us your name.") }}
{{ ui.code_field("code", "6-digit code") }}
{{ ui.textarea("note", "Note for the bakery") }}
{{ ui.select("slot", "Collection", [{"value": "a", "label": "Sat 10:00"}, {"value": "b", "label": "Sat 12:00"}], value="b") }}
{{ ui.check("terms", "I'm 18 or over") }}{{ ui.switch("digest", "Weekly digest", on=true) }}{{ ui.or_line() }}"###,
        context! { agent => agent() },
    );
    assert!(html.contains(r###"<section class="card accent">"###));
    assert!(html.contains("<p>Inside</p>"));
    assert!(html.contains(r###"<article class="fcard">"###));
    assert!(html.contains(r###"aria-describedby="email-note""###));
    assert!(html.contains(r###"<label class="o-field" data-invalid>"###));
    assert!(html.contains(r###"aria-invalid="true""###));
    assert!(html.contains(r###"autocomplete="one-time-code" inputmode="numeric""###));
    assert!(html.contains(r###"<option value="b" selected>"###));
    assert!(html.contains(r###"role="switch" name="digest" checked"###));
}

#[test]
fn data_renders() {
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{{ ui.data_table([{"key": "who", "label": "Customer"}, {"key": "amt", "label": "Amount", "align": "r"}], [{"who": "Amani W.", "amt": "KSh 4,500"}], caption="Orders today") }}
{{ ui.data_table([{"key": "who", "label": "Customer"}], [], empty="No orders yet. They appear here the moment someone pays.") }}
{{ ui.kpi({"label": "Visitors", "value": "41", "window": "Overnight", "source": "Edge counts", "href": "/visits"}) }}
{{ ui.kpi({"label": "AI spend", "value": "38", "unit": "KSh"}) }}
{{ ui.timeline([{"when": "06:58", "title": "Price list received", "state": "done"}, {"when": "07:00", "title": "Brief written", "body": "from 31 events", "state": "now"}]) }}
{{ ui.steps([{"text": "Read the list", "state": "done"}, {"text": "Compare", "state": "now"}, {"text": "Draft v15", "state": ""}]) }}
<ul>{{ ui.receipt_row({"digest": "sha256:ab", "title": "Release v14", "verified": true, "kind_words": "Release", "at_words": "Sat 7 Nov, 06:58", "digest_short": "ab12…"}) }}</ul>
{{ ui.sealed_chip(["Sealed room", "No network but the proxy"]) }}"###,
        context! {},
    );
    assert!(html.contains(r###"<th scope="col" class="r">Amount</th>"###));
    assert!(html.contains(r###"<td class="r">KSh 4,500</td>"###));
    assert!(html.contains(r###"<td colspan="1">No orders yet."###));
    assert!(html.contains(r###"<a class="kpi" href="/visits">"###));
    assert!(html.contains("<small>KSh</small>"));
    assert!(html.contains(r###"<li class="now"><span class="t">07:00</span>"###));
    assert!(html.contains(r###"<span class="sep" aria-hidden="true">·</span>"###));
}

#[test]
fn marks_ladder_and_shell_parts_render() {
    let svg = minijinja::Value::from_safe_string(
        r###"<svg viewBox="0 0 28 28"><rect fill="#11734a"/></svg>"###.into(),
    );
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{{ ui.mark(agent) }}
{{ ui.mark(agent, audience="visitor", state="resting", size=40) }}
{{ ui.mark({"id": "cakes", "name": "Cakes", "svg": svg}, state="working", audience="visitor") }}
{{ ui.team_mark({"name": "Kilele Bakery", "monogram": "K", "ink": "#2b1d14"}, size=32, waiting=true) }}
{{ ui.ai_chip() }}
{{ ui.ladder("ask", ceiling="tell") }}
{{ ui.ladder("tell", name="level", lowered=true, why="Lowered after a refund dispute on Sat 7 Nov.") }}
{{ ui.site_chip({"name": "Kilele Bakery", "sub": "kilele-bakery-sample.agentrix.website · v14 live", "monogram": "K", "tile": "#7a3f1d"}) }}
{{ ui.scope_chip("operator", "Zavora Cloud", "Operator · 3 regions") }}
{{ ui.app_icon({"label": "Cashier", "href": "/console/cashier", "agent": agent, "badge": 1}) }}
{{ ui.pill("Budget", value="KSh 38") }}{{ ui.time_pill("Day Time · until 18:00", "Night Time · until 06:00") }}
{% call ui.lane_section("Needs you", 2) %}{{ ui.quiet("Nothing else needs you.") }}{% endcall %}
{{ ui.command_bar("/console/ask", "Ask anything, or tell your WebAgents what to do") }}"###,
        context! { agent => agent(), svg => svg },
    );
    assert!(html.contains(r###"<span class="mk s28" data-state="waiting" data-c="#11734a" aria-hidden="true">Ca</span>"###));
    assert!(html.contains(r###"<span class="said" role="status">Waiting for you</span>"###));
    assert!(
        html.contains(r###"<span class="ai-chip">AI</span>"###),
        "visitor marks carry the AI chip"
    );
    assert!(
        html.contains("Answers are paused right now."),
        "visitor words for resting"
    );
    assert!(html.contains(r###"<span class="mk s28 live" aria-hidden="true"><wa-mark webagent="cakes" state="working"><svg"###));
    assert!(html.contains(r###"data-orb-ink="#2b1d14" data-waiting"###));
    assert!(html.contains(r###"<span class="rung locked">Handle it within budget"###));
    assert!(html.contains(r###"aria-current="true">Ask me first"###));
    assert!(html.contains(r###"data-ceiling="Your max">Do it and tell me"###));
    assert!(html.contains(r###"value="tell" aria-pressed="true">Do it and tell me"###));
    assert!(html.contains(r###"class="ladder lowered""###));
    assert!(html.contains(r###"data-site-tile="#7a3f1d""###));
    assert!(html.contains(r###"class="scope-chip op""###));
    assert!(html.contains(r###"aria-label="1 waiting""###));
    assert!(html.contains(r###"data-day="Day Time · until 18:00""###));
}

#[test]
fn decision_card_and_approval_sheet_render() {
    let card = context! {
        id => "o-118",
        title => "Cake order: KSh 4,500, collect Sat 14 Nov 10:00",
        from => agent(),
        body => "Amani W. asked for a forest-green ombré cake for 12.",
        chip => context! { text => "Waiting for you", tone => "wait" },
        effects => vec!["Confirms order o-118 to Amani on WhatsApp", "Takes KSh 4,500 through PayPal"],
        risk => "Low risk: returning customer, usual amount",
        expires => "Expires Sat 7 Nov, 19:00",
        why => "Orders over KSh 3,000 need your OK (standing rule, 2 Nov).",
        decline_action => "/console/decisions/o-118/decline",
        detail_href => "/console/decisions/o-118",
    };
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{{ ui.decision_card(card) }}
{{ ui.decision_card(card, size="full") }}
{{ ui.decision_card({"id": "x", "title": "Old", "from": agent, "state": "expired"}) }}
{{ ui.decision_card({"id": "p", "title": "Message 31 customers", "from": agent, "from_words": "Sales suggests", "primary": {"label": "Plan it", "href": "/missions/m-3"}, "why_label": "Why am I seeing this?", "why": "From the Christmas cakes mission."}) }}
{{ ui.decision_card({"id": "y", "title": "Done", "from": agent, "state": "done", "done_words": "Approved with your passkey on iPhone"}) }}
{{ ui.approval_sheet({"id": "o-118", "summary": "Cake order o-118 · KSh 4,500 · collect Sat 14 Nov 10:00", "fingerprint": "4c19…e7", "passkey_action": "/console/decisions/o-118/approve", "code_action": "/console/decisions/o-118/approve-code"}) }}"###,
        context! { card => card, agent => agent() },
    );
    assert!(
        html.contains(
            r###"<article class="ldec" id="card-o-118" data-decision data-card="o-118">"###
        )
    );
    assert!(html.contains(
        r###"<article class="ldec full" id="card-o-118" data-decision data-card="o-118">"###
    ));
    assert!(html.contains(r###"<h2 class="t">"###));
    assert!(html.contains(r###"href="#approve-o-118" data-open="approve-o-118""###));
    assert!(html.contains("What happens if you approve"));
    assert!(
        html.contains(
            r###"<article class="ldec expired" id="card-x" data-decision data-card="x">"###
        )
    );
    assert!(html.contains("This card expired before anyone answered. Nothing was done."));
    assert!(html.contains("Approved with your passkey on iPhone"));
    assert!(html.contains(
        r###"<div class="overlay" id="approve-o-118" role="dialog" aria-modal="true""###
    ));
    assert!(html.contains(r###"data-passkey data-action="/console/decisions/o-118/approve""###));
    assert!(html.contains(r###"action="/console/decisions/o-118/approve-code""###));
    assert!(html.contains(r###"autocomplete="one-time-code""###));
    // Drawn as the concept's lane card and passkey sheet (concept/console/shell.html).
    assert!(
        html.contains("<span>Cashier asks</span>"),
        "the card says who asks"
    );
    assert!(html.contains("<span>Sales suggests</span>"));
    assert!(
        html.contains(r###"<a class="btn btn-primary btn-sm" href="/missions/m-3">Plan it</a>"###),
        "a proposal's own verb"
    );
    assert!(html.contains(">Why am I seeing this?</button>"));
    let why = html.find(">Why?</button>").expect("a Why? button");
    let tag = &html[html[..why].rfind("<button").expect("its tag")..why];
    for attr in [
        r#"data-why="""#,
        r#"aria-expanded="false""#,
        r#"aria-controls="why-o-118""#,
    ] {
        assert!(tag.contains(attr), "{tag} lacks {attr}");
    }
    assert!(html.contains(r###"<p class="whybox" id="why-o-118">"###));
    assert!(html.contains("Confirm it&#x27;s you"));
    assert!(html.contains("<b>Your passkey signs:</b> <span data-sign-fp>4c19…e7</span>"));
    assert!(html.contains("No passkey on this device? Use a code from your authenticator app"));
    assert!(html.contains(
        r###"<button class="btn" type="button" data-stepup-back>Back to passkey</button>"###
    ));
}

#[test]
fn states_banners_and_site_pieces_render() {
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{{ ui.empty("No orders yet", "They appear here the moment someone pays.", action={"label": "Share your shop", "href": "/share"}) }}
{{ ui.loading("Checking 50 pages against the old site (31 done)") }}
{{ ui.skeleton(4) }}
{{ ui.error_state("The Move stopped at the replay step", "The old site timed out. Start it again; it picks up where it stopped.", ref="r-1") }}
{{ ui.offline("06:58") }}
{{ ui.read_only("Your plan ended on Sat 7 Nov.") }}
{{ ui.banner("frozen", "Frozen. Every WebAgent stopped at its next safe point.", action={"label": "Unfreeze", "href": "/console/unfreeze"}) }}
{{ ui.banner("offline", "You're offline.") }}
{{ ui.toast("Saved", "ok") }}
{{ ui.sample_band({"name": "Classy Perfumes"}) }}
{{ ui.provider("PayPal", "PayPal opens here to take the payment.") }}
{{ ui.disclosure() }}"###,
        context! {},
    );
    assert!(html.contains(r###"<a class="btn btn-primary" href="/share">Share your shop</a>"###));
    assert!(html.contains(r###"<p class="loading" role="status""###));
    assert!(html.matches("<i></i>").count() == 4);
    assert!(html.contains(r###"<p class="ref">Reference r-1</p>"###));
    assert!(html.contains("You&#x27;re seeing what was saved at 06:58."));
    assert!(html.contains(
        "Read-only. Your plan ended on Sat 7 Nov. You can still read and export everything."
    ));
    assert!(html.contains(r###"<div class="banner frozen" role="alert">"###));
    assert!(html.contains(r###"data-offline-banner hidden"###));
    assert!(html.contains("Classy Perfumes isn&#x27;t a real business."));
    assert!(
        html.contains("You&#x27;re talking to AI. It never guesses prices, dates or allergens.")
    );
}

#[test]
fn assets_are_served_with_their_types() {
    for (path, kind) in [
        ("tokens.css", "text/css; charset=utf-8"),
        ("brand.css", "text/css; charset=utf-8"),
        ("kit.css", "text/css; charset=utf-8"),
        ("shell.css", "text/css; charset=utf-8"),
        ("site.css", "text/css; charset=utf-8"),
        ("kit.js", "text/javascript; charset=utf-8"),
        ("fonts/manrope-latin-wght-normal.woff2", "font/woff2"),
        ("fonts/OFL.txt", "text/plain; charset=utf-8"),
    ] {
        let (body, content_type) =
            agentrix_ui::asset(path).unwrap_or_else(|| panic!("{path} is missing"));
        assert!(!body.is_empty(), "{path} is empty");
        assert_eq!(content_type, kind, "{path}");
    }
    assert!(agentrix_ui::asset("../Cargo.toml").is_none());
}

#[test]
fn every_font_the_tokens_name_is_shipped() {
    let (css, _) = agentrix_ui::asset("tokens.css").expect("tokens.css ships");
    let css = std::str::from_utf8(css).expect("utf-8");
    let mut count = 0;
    for (i, _) in css.match_indices("url(fonts/") {
        let end = css[i..].find(')').expect("url( is closed") + i;
        let path = &css[i + 4..end];
        assert!(
            agentrix_ui::asset(path).is_some(),
            "{path} is named in tokens.css but not shipped"
        );
        count += 1;
    }
    assert!(count >= 12);
}

#[tokio::test]
async fn the_router_serves_assets_and_refuses_the_rest() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    let app: axum::Router = agentrix_ui::assets();
    let res = app
        .clone()
        .oneshot(
            Request::get("/static/ui/kit.css?v=1")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("served");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "text/css; charset=utf-8");
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    let missing = app
        .oneshot(
            Request::get("/static/ui/nope.css")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("served");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[test]
fn living_marks_render_from_a_spec() {
    let spec = serde_json::json!({
        "markSpec": 1, "site": "kilele", "webagent": "cakes",
        "silhouette": {"shape": "squircle", "squareness": 4.6},
        "color": {"fill": "#8e3b46", "glyph": "#ffffff"},
        "glyph": {"kind": "role", "id": "cake"},
        "signature": {"family": "grain", "seed": 7}
    });
    let team = serde_json::json!({
        "site": "kilele", "ink": "#2b1d14", "squareness": 4.6, "letters": "K",
        "members": [{"webagent": "cakes", "fill": "#8e3b46"}, {"webagent": "orders", "fill": "#3b2416"}]
    });
    let ctx = minijinja::Value::from_serialize(serde_json::json!({
        "cakes": {"id": "cakes", "name": "Cakes", "spec": spec, "state": "paused", "owner": "Wanjiru"},
        "orders": {"id": "orders", "name": "Orders", "spec": spec, "state": "waiting", "owner": "Wanjiru"},
        "site": {"name": "Kilele Bakery", "monogram": "K", "team": team, "busy": ["cakes"]}
    }));
    let html = render(
        r###"{% import "ui/kit.html" as ui %}
{{ ui.mark(cakes) }}|{{ ui.mark(cakes, audience="visitor") }}|{{ ui.mark(orders, audience="visitor", size=40) }}|{{ ui.team_mark(site, waiting=true) }}"###,
        ctx,
    );
    let parts: Vec<&str> = html.split('|').collect();
    assert_eq!(parts.len(), 4, "{html}");
    // The owner sees paused as paused; a visitor sees it as resting, never the reason.
    assert!(
        parts[0].contains(r#"<wa-mark webagent="cakes" state="paused"><svg"#),
        "{}",
        parts[0]
    );
    assert!(
        parts[0].contains(r#"role="status">Paused</span>"#),
        "{}",
        parts[0]
    );
    assert!(
        parts[1].contains(r#"<wa-mark webagent="cakes" state="resting"><svg"#),
        "{}",
        parts[1]
    );
    assert!(
        parts[1].contains("Answers are paused right now."),
        "{}",
        parts[1]
    );
    assert!(parts[1].contains(r#"<span class="ai-chip">AI</span>"#));
    assert!(
        parts[2].contains("Waiting for Wanjiru&#x27;s OK"),
        "{}",
        parts[2]
    );
    assert!(
        parts[3].contains(r#"<span class="orb live" data-waiting"#),
        "{}",
        parts[3]
    );
    assert!(parts[3].contains("<wa-mark team><svg"), "{}", parts[3]);
    // Two marks on one page never share SVG ids.
    let ids: Vec<&str> = html
        .match_indices("id=\"wm")
        .map(|(i, _)| &html[i..i + 12])
        .collect();
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(ids.len(), unique.len(), "duplicate SVG ids: {ids:?}");
}

#[test]
fn a_bad_mark_state_is_an_error_not_a_blank() {
    let mut env = agentrix_ui::environment();
    env.add_template(
        "t.html",
        r#"{% import "ui/kit.html" as ui %}{{ ui.mark({"name": "Cakes"}, state="dancing") }}"#,
    )
    .expect("parses");
    assert!(
        env.get_template("t.html")
            .expect("loads")
            .render(())
            .is_err()
    );
}

#[test]
fn brand_css_is_slim_and_tokens_css_is_brand_plus_hosts() {
    let (brand, _) = agentrix_ui::asset("brand.css").expect("brand.css ships");
    let (tokens, _) = agentrix_ui::asset("tokens.css").expect("tokens.css ships");
    let brand = std::str::from_utf8(brand).expect("utf-8");
    let tokens = std::str::from_utf8(tokens).expect("utf-8");
    assert!(
        brand.len() < 4_800,
        "brand.css grew to {} bytes",
        brand.len()
    );
    assert!(
        tokens.starts_with(brand),
        "tokens.css starts with brand.css"
    );
    for needle in [
        "--indigo:#38356d",
        "[data-time=\"night\"]",
        "font-family:\"Manrope\"",
        "font-family:\"DM Sans\"",
        "font-family:\"JetBrains Mono\"",
    ] {
        assert!(brand.contains(needle), "brand.css lacks {needle}");
    }
    for needle in [
        "Fraunces",
        "[data-host=\"kilele\"]",
        "--wa-accent",
        "--os-bg",
        "latin-ext",
    ] {
        assert!(
            !brand.contains(needle),
            "brand.css should not carry {needle}"
        );
        assert!(tokens.contains(needle), "tokens.css lacks {needle}");
    }
}

#[test]
fn a_frozen_desk_tells_visitors_they_can_still_read_the_site() {
    // The mark draws as resting for a visitor, but the words follow the real state (06 §4.3).
    let html = render(
        r###"{% import "ui/kit.html" as ui %}{{ ui.mark({"id": "cakes", "name": "Cakes", "glyph": "Ca", "color": "#8e3b46"}, state="frozen", audience="visitor") }}|{{ ui.mark({"id": "cakes", "name": "Cakes", "glyph": "Ca"}, state="paused", audience="visitor") }}"###,
        context! {},
    );
    let (frozen, paused) = html.split_once('|').expect("two marks");
    assert!(frozen.contains(r#"data-state="resting""#), "{frozen}");
    assert!(frozen.contains("You can still read the site."), "{frozen}");
    assert!(
        paused.contains("The page and the form below still work."),
        "{paused}"
    );
}
