//! Every onboarding page renders in every state, in strict mode, with the concept's pieces.

mod common;

use agentrix_onboarding::{OnboardingPage, Pages};

fn pages() -> Pages {
    Pages::new().expect("templates load")
}

#[test]
fn every_template_loads() {
    let p = pages();
    for page in OnboardingPage::ALL {
        assert!(!page.template().is_empty());
    }
    drop(p);
}

#[test]
fn start_is_the_address_bar_with_its_three_promises() {
    let html = pages().render_start(&common::start()).unwrap();
    assert!(html.contains(r#"<form class="addr-form" method="post" action="/assess""#));
    assert!(html.contains(r#"name="url""#));
    assert!(html.contains(">Check my site</button>"));
    for promise in ["No account", "No passwords", "Only what's already public"] {
        assert!(html.contains(promise), "missing {promise}");
    }
    assert!(html.contains(r#"href="/assess/fixture""#));
    assert!(!html.contains("addr-error"), "no error on a fresh page");
    assert!(html.contains("/static/onboarding/onboarding.css?v="));
}

#[test]
fn start_shows_the_error_and_what_was_typed() {
    let html = pages().render_start(&common::start_error()).unwrap();
    assert!(html.contains(r#"role="alert""#));
    assert!(html.contains("That address isn&#x27;t on the public internet."));
    assert!(html.contains(r#"value="bakery.local""#));
    assert!(html.contains(r#"aria-invalid="true""#));
    assert!(
        html.contains("Kilele Bakery</a>, a sample business"),
        "a sample says so"
    );
}

#[test]
fn reading_shows_the_room_and_waits_for_the_first_frame() {
    let html = pages().render_assess(&common::running()).unwrap();
    assert!(html.contains("Reading fixture.example"));
    assert!(html.contains("Step 1 of 3"));
    assert!(html.contains(r#"data-state="running""#));
    assert!(html.contains(r#"data-events="/assess/asmt_0193test/events""#));
    assert!(html.contains("The sealed browser hasn't sent a frame yet."));
    assert!(html.contains(r#"id="tick-requests">9<"#));
    assert!(html.contains(r#"id="tick-bytes">412 KB<"#));
    assert!(html.contains("Zavora's fixture"), "the fixture is labelled");
    assert!(html.contains("/static/onboarding/assess.js?v="));
}

#[test]
fn reading_with_a_frame_labels_it_honestly() {
    let html = pages()
        .render_assess(&common::running_with_frame())
        .unwrap();
    assert!(html.contains(r#"src="/frames/sha256-test""#));
    assert!(html.contains("Sealed screenshot, about 1 a second"));
    assert!(
        !html.contains(">Live<"),
        "a sealed screenshot is never called live"
    );
}

#[test]
fn the_report_puts_urgent_first_and_says_what_looks_fine() {
    let html = pages().render_assess(&common::done()).unwrap();
    assert!(html.contains("Health report for fixture.example</h1>"));
    assert!(html.contains("Step 2 of 3"));
    assert!(html.contains("1 urgent"));
    assert!(html.contains("3 findings"));
    assert!(html.contains("public evidence only"));
    let urgent = html.find("Hidden links you didn&#x27;t put there").unwrap();
    let warn = html
        .find("An old WordPress with a public login page")
        .unwrap();
    let info = html.find("No structured data for your products").unwrap();
    assert!(
        urgent < warn && warn < info,
        "urgent, then worth fixing, then good to know"
    );
    assert!(html.contains("Urgent · Security"));
    assert!(html.contains("Good to know · AI assistants"));
    assert!(
        html.contains("&lt;div style=&quot;display:none&quot;&gt;"),
        "evidence is escaped"
    );
    assert!(html.contains(r#"href="/receipts/sha256-receipt""#));
    assert!(html.contains("What looks fine"));
    assert!(html.contains("No broken links among the 12 pages we read."));
    assert!(html.contains("12 pages read"));
    assert!(html.contains(r#"href="/receipts/sha256-report""#));
}

#[test]
fn could_not_finish_never_shows_findings() {
    for (view, heading) in [
        (common::unreachable(), "We couldn't reach fixture.example"),
        (common::refused(), "fixture.example asked us not to read it"),
    ] {
        let html = pages().render_assess(&view).unwrap();
        assert!(html.contains(heading), "missing {heading}");
        assert!(html.contains("We haven't guessed at any findings."));
        assert!(html.contains(r#"href="/start">Try another address"#));
        assert!(!html.contains("class=\"finding\""));
        assert!(!html.contains("Keep this site: create your account"));
    }
}

#[test]
fn claim_by_dns_gives_the_short_name_and_the_full_one() {
    let html = pages().render_claim(&common::claim_dns()).unwrap();
    assert!(html.contains("Prove kilele-bakery.example is yours"));
    assert!(html.contains("Caretaker"));
    assert!(html.contains(r#"<span class="o-code">_agentrix</span>"#));
    assert!(html.contains("_agentrix.kilele-bakery.example"));
    assert!(html.contains("agentrix-site-verification=k1l3-7f2c"));
    assert!(html.contains(r#"href="?method=dns" aria-current="page""#));
    assert!(html.contains(r#"action="/sites/site_0193test/claim/verify""#));
    assert!(html.contains(r#"name="method" value="dns""#));
    assert!(html.contains(">Check now<"));
}

#[test]
fn claim_by_file_offers_the_file_and_says_what_was_seen() {
    let html = pages().render_claim(&common::claim_file_failed()).unwrap();
    assert!(html.contains(r#"download="agentrix-verify.txt""#));
    assert!(html.contains("data:text/plain;charset=utf-8,agentrix-site-verification=k1l3-7f2c%0A"));
    assert!(
        html.contains(
            "No file at that address yet (HTTP 404). · Checked 10:02 UTC from the sandbox"
        )
    );
    assert!(html.contains(">Check again<"));
    assert!(html.contains(r#"name="method" value="file""#));
}

#[test]
fn a_proved_claim_leads_to_the_new_look_then_the_move() {
    let html = pages().render_claim(&common::claim_verified()).unwrap();
    assert!(html.contains("✓ Found it. kilele-bakery.example is yours"));
    assert!(html.contains(r#"/concepts">Next: choose your new look"#));
    assert!(html.contains(r#"href="/console/move">keep your current look and start the Move"#));
    assert!(!html.contains("Check now"));
}

#[test]
fn a_new_account_offers_the_passkey_first() {
    use agentrix_onboarding::view::{AccountMode, AccountPage};
    let view = AccountPage::new(
        AccountMode::New,
        "/sites/site_0193test/claim",
        Some("kilele-bakery.example".into()),
    );
    let html = pages().render(OnboardingPage::Account, &view).unwrap();
    assert!(html.contains("Keep kilele-bakery.example"));
    let passkey = html.find("Create with a passkey").unwrap();
    let email = html.find("Email me a code").unwrap();
    let password = html
        .find("Use a password and an authenticator app")
        .unwrap();
    assert!(
        passkey < email && email < password,
        "passkey, then email code, then password"
    );
    assert!(html.contains("An email code only signs you in."));
    assert!(html.contains("Save your recovery codes"));
    assert!(html.contains(r#"data-next="/sites/site_0193test/claim""#));
    assert!(html.contains(r#"href="/account/signin?next=/sites/site_0193test/claim">Sign in"#));
    assert!(html.contains("/static/onboarding/account.js?v="));
}

#[test]
fn sign_in_and_recovery_have_their_own_ways_in() {
    use agentrix_onboarding::view::{AccountMode, AccountPage};
    let signin = pages()
        .render(
            OnboardingPage::Account,
            &AccountPage::new(AccountMode::Signin, "/os", None),
        )
        .unwrap();
    assert!(signin.contains("Sign in with a passkey"));
    assert!(signin.contains(r#"href="/account/recover?next=/os">Get back into your account"#));
    assert!(!signin.contains("Save your recovery codes"));
    let recover = pages()
        .render(
            OnboardingPage::Account,
            &AccountPage::new(AccountMode::Recover, "/os", None),
        )
        .unwrap();
    assert!(recover.contains("With a recovery code"));
    assert!(recover.contains("Start the 24-hour hold"));
}

#[test]
fn setup_has_the_chief_speak_and_points_at_the_claim() {
    use agentrix_onboarding::view::{SetupPage, SetupStep};
    let view = SetupPage {
        host: Some("kilele-bakery.example".into()),
        steps: vec![
            SetupStep {
                n: 1,
                title: "Your account".into(),
                words: "Made.".into(),
                done: true,
            },
            SetupStep {
                n: 2,
                title: "Prove kilele-bakery.example is yours".into(),
                words: "One record.".into(),
                done: false,
            },
        ],
        chief_words: "Next, prove kilele-bakery.example is yours, and the Mover will start.".into(),
        next_href: "/sites/site_0193test/claim".into(),
        next_words: "Next: prove kilele-bakery.example is yours".into(),
    };
    let html = pages().render(OnboardingPage::Setup, &view).unwrap();
    assert!(html.contains("Chief WebAgent:"));
    assert!(html.contains("nothing important happens without your OK"));
    assert!(html.contains(
        r#"href="/sites/site_0193test/claim">Next: prove kilele-bakery.example is yours"#
    ));
}

#[test]
fn keep_it_leads_to_the_account_page_with_the_site() {
    let html = pages().render_assess(&common::done()).unwrap();
    assert!(html.contains(r#"href="/account/new?next=/sites/site_0193test/claim&amp;host=fixture.example">Keep this site: create your account"#));
    assert!(html.contains(r#"href="/account/signin?next=/sites/site_0193test/claim">Sign in"#));
}

#[test]
fn every_page_names_itself_in_the_title() {
    use agentrix_onboarding::view::{AccountMode, AccountPage, SetupPage};
    let p = pages();
    let html = [
        p.render_start(&common::start()).unwrap(),
        p.render_assess(&common::running()).unwrap(),
        p.render_assess(&common::done()).unwrap(),
        p.render_assess(&common::unreachable()).unwrap(),
        p.render_claim(&common::claim_dns()).unwrap(),
        p.render(
            OnboardingPage::Account,
            &AccountPage::new(AccountMode::Signin, "/", None),
        )
        .unwrap(),
        p.render(OnboardingPage::Setup, &SetupPage::default())
            .unwrap(),
    ];
    for h in html {
        let title = h
            .split("<title>")
            .nth(1)
            .and_then(|t| t.split("</title>").next())
            .unwrap();
        assert!(
            !title.starts_with(" ·") && title.ends_with(" · Agentrix"),
            "{title}"
        );
    }
}

#[test]
fn the_move_draws_the_rail_rooms_and_the_go_live_card() {
    use agentrix_onboarding::view::MoveState;
    let html = pages()
        .render_move(&common::move_view(MoveState::Waiting))
        .unwrap();
    assert!(html.contains("kilele-bakery.example is ready to go live"));
    for stop in [
        "Read", "Claim", "Replay", "Clean", "Rebuild", "Prove", "Go live", "Watch",
    ] {
        assert!(html.contains(&format!("<b>{stop}</b>")), "rail stop {stop}");
    }
    assert!(html.contains(r#"<li class="wait" data-key="golive">"#));
    assert!(
        html.contains("Replay room") && html.contains("Build room") && html.contains("Render room")
    );
    assert!(html.contains("Three invisible links to gambling sites"));
    assert!(
        html.contains("pct-95 ok"),
        "97% fills to the 95 step and reads as passing"
    );
    assert!(
        html.contains(r#"action="/moves/move_0193test/accept""#),
        "a flagged page can be accepted"
    );
    assert!(html.contains(
        r#"href="/console/site_0193test/decisions/dec_0193test">Go live · approve with passkey"#
    ));
    assert!(html.contains("/static/onboarding/move.js?v="));
}

#[test]
fn live_offers_undo_and_none_offers_the_start() {
    use agentrix_onboarding::view::MoveState;
    let live = pages()
        .render_move(&common::move_view(MoveState::Live))
        .unwrap();
    assert!(live.contains("kilele-bakery.example is live"));
    assert!(
        live.contains(r#"name="to" value="previous""#)
            && live.contains(r#"name="to" value="old-site""#)
    );
    assert!(live.contains(r#"<li class="run" data-key="watch">"#));
    assert!(
        !live.contains("move.js"),
        "a finished Move doesn't follow events"
    );
    let none = pages()
        .render_move(&common::move_view(MoveState::None))
        .unwrap();
    assert!(none.contains("Ready to move kilele-bakery.example"));
    assert!(none.contains(r#"action="/sites/site_0193test/moves""#));
    assert!(none.contains("Your WordPress export"));
    let stopped = pages()
        .render_move(&common::move_view(MoveState::Stopped))
        .unwrap();
    assert!(stopped.contains("Start a new Move"));
    assert!(stopped.contains(r#"<li class="fail" data-key="clean">"#));
}

#[test]
fn plans_come_from_the_catalogue_and_paypal_is_the_only_rail() {
    use agentrix_onboarding::view::{PlanInput, PlanPage};
    let plan = |id: &str, name: &str, kind: &str, cents: i64, ai: i64| PlanInput {
        id: id.into(),
        name: name.into(),
        kind: kind.into(),
        price_cents: cents,
        annual_cents: if kind == "site" {
            agentrix_billing::annual_cents(cents, 20)
        } else {
            0
        },
        included_ai_micro_usd: ai * 1_000_000,
        webagents: vec!["Chief".into(), "Concierge".into(), "Cashier".into()],
        includes: vec![format!("${ai} of AI a month")],
    };
    let plans = [
        plan("commerce", "Commerce", "site", 14900, 30),
        plan("agency", "Agency wholesale", "wholesale", 1500, 5),
        plan("essential", "Essential", "site", 2900, 5),
    ];
    let view = PlanPage::new(
        "site_0193test",
        Some("kilele-bakery.example".into()),
        &plans,
        "/sites/site_0193test/claim",
    );
    let html = pages().render(OnboardingPage::Plan, &view).unwrap();
    assert!(
        html.contains("$149 <small>a month</small>") && html.contains("$29 <small>a month</small>")
    );
    assert!(
        !html.contains("Agency wholesale"),
        "owners never see wholesale plans"
    );
    assert!(
        html.find("Essential").unwrap() < html.find("Commerce</h3>").unwrap(),
        "cheapest first"
    );
    assert!(html.contains(r#"aria-pressed="true" data-id="commerce""#));
    assert!(html.contains("WebAgents: Chief, Concierge and Cashier"));
    assert!(html.contains(r#"value="paypal" checked"#));
    assert!(
        !html.contains("M-Pesa") && !html.contains("Paystack"),
        "PayPal only"
    );
    assert!(html.contains("first charge on the day your site goes live"));
    assert!(html.contains(
        r#"<form class="o-body setup-body" id="plan" method="post" action="/setup/plan">"#
    ));
    assert!(html.contains(r#"name="plan_id" id="plan-id" value="commerce""#));
    // Monthly or 12 months at the catalogue's one discount, both priced in USD.
    assert!(html.contains("How often you pay"));
    assert!(html.contains(r#"name="period" value="monthly" checked"#));
    assert!(html.contains(r#"name="period" value="annual">"#));
    assert!(html.contains("12 months (20% off)"));
    assert!(
        html.contains("or $1,430.40 for 12 months") && html.contains("or $278.40 for 12 months")
    );
    assert!(html.contains(
        r#"<span data-plan-annual>$1,430.40</span> once for 12 months, instead of <span data-plan-year>$1,788</span> paid monthly. You save <span data-plan-saving>$357.60</span>."#
    ));
    assert!(html.contains(r#"<span data-plan-charge>$149 a month</span>"#));
    let mut annual = view.clone();
    annual.period = "annual".into();
    let html = pages().render(OnboardingPage::Plan, &annual).unwrap();
    assert!(html.contains(r#"name="period" value="annual" checked"#));
    assert!(html.contains(r#"<span data-plan-charge>$1,430.40 for 12 months</span>"#));
    let empty = PlanPage::new("site_0193test", None, &[], "/sites/site_0193test/claim");
    let none = pages().render(OnboardingPage::Plan, &empty).unwrap();
    assert!(
        none.contains("Plans aren't published here yet."),
        "no price nobody published"
    );
    assert!(!none.contains("Choose <span"));
}

#[test]
fn pay_says_where_the_plan_payment_stands() {
    use agentrix_onboarding::view::{PayPage, PayState};
    let base = PayPage {
        site_id: "site_0193test".into(),
        state: PayState::Ready,
        plan_name: "Commerce".into(),
        price_words: "$149".into(),
        charge_words: "$149 a month".into(),
        period: "monthly".into(),
        budget_words: "$30".into(),
        said: None,
        approve_url: None,
        plan_href: "/setup/plan?site=site_0193test".into(),
        next: "/console/move".into(),
    };
    let p = pages();
    let ready = p.render(OnboardingPage::Pay, &base).unwrap();
    assert!(ready.contains(r#"<form method="post" action="/setup/pay""#));
    assert!(ready.contains("first charge today, the day your site goes live"));
    assert!(ready.contains("Continue to PayPal"));
    let active = p
        .render(
            OnboardingPage::Pay,
            &PayPage {
                state: PayState::Active,
                ..base.clone()
            },
        )
        .unwrap();
    assert!(active.contains(
        "✓ Approved on PayPal: Commerce, $149 a month. PayPal charges it today, then each month."
    ));
    assert!(active.contains(r#"href="/console/move">Next: go live"#));
    assert!(!active.contains("Continue to PayPal"));
    let off = p
        .render(
            OnboardingPage::Pay,
            &PayPage {
                state: PayState::Off,
                ..base.clone()
            },
        )
        .unwrap();
    assert!(
        off.contains("PayPal isn't switched on here yet") && off.contains("Nothing was charged.")
    );
    let none = p
        .render(
            OnboardingPage::Pay,
            &PayPage {
                state: PayState::NoPlan,
                ..base.clone()
            },
        )
        .unwrap();
    assert!(none.contains("You haven't chosen a plan for this site yet."));
    let annual = PayPage {
        state: PayState::Ready,
        charge_words: "$1,430.40 for 12 months".into(),
        period: "annual".into(),
        ..base.clone()
    };
    let ready = p.render(OnboardingPage::Pay, &annual).unwrap();
    assert!(ready.contains(
        "<b>Commerce · $1,430.40 for 12 months</b> · first charge today, the day your site goes live: the 12-month total, once"
    ));
    let active = p
        .render(
            OnboardingPage::Pay,
            &PayPage {
                state: PayState::Active,
                ..annual
            },
        )
        .unwrap();
    assert!(active.contains(
        "✓ Approved on PayPal: Commerce, $1,430.40 for 12 months. PayPal charges it today, then again in 12 months."
    ));
}

#[test]
fn billing_shows_the_plan_and_cancels_only_with_an_approval() {
    use agentrix_onboarding::view::{BillingPage, CancelCard, Chip};
    let view = BillingPage {
        site_id: "site_0193test".into(),
        state_chip: Chip {
            class: "chip-ok".into(),
            words: "Active".into(),
        },
        summary: "Commerce · $149 a month · PayPal subscription · next charge Mon 7 Dec".into(),
        has_plan: true,
        period_words: "Monthly".into(),
        next_charge: Some("Mon 7 Dec 2026, $149".into()),
        switch: Some(agentrix_onboarding::view::PeriodSwitch {
            to: "annual".into(),
            words: "Pay for 12 months at a time: $1,430.40 once a year instead of $1,788 paid monthly, so you save $357.60. It starts at your next renewal, Mon 7 Dec 2026.".into(),
            button: "Switch to 12 months".into(),
            agree_to: None,
            pending: None,
        }),
        spent_words: Some("$3.40".into()),
        budget_words: Some("$30".into()),
        spent_pct: 10,
        plans: vec![],
        current_plan: "commerce".into(),
        cancel: Some(CancelCard {
            title: "Cancel Commerce?".into(),
            sign_words: "Cancel Commerce for this site: PayPal stops charging. Subscription sub_1."
                .into(),
            fingerprint: "ab".repeat(32),
            passkey_options: None,
        }),
        said: None,
        plan_href: "/setup/plan?site=site_0193test".into(),
    };
    let html = pages().render(OnboardingPage::Billing, &view).unwrap();
    assert!(html.contains("Commerce · $149 a month · PayPal subscription · next charge Mon 7 Dec"));
    assert!(html.contains("<b>$3.40</b> of your $30 AI budget this period"));
    // The period, the next charge's day and amount, and switching at the next renewal.
    assert!(html.contains(r#"<h3>How often you pay</h3><span class="chip">Monthly</span>"#));
    assert!(html.contains("Next charge: <b>Mon 7 Dec 2026, $149</b>"));
    assert!(html.contains(r#"<input type="hidden" name="period" value="annual"><button class="btn" type="submit">Switch to 12 months</button>"#));
    assert!(html.contains("so you save $357.60. It starts at your next renewal, Mon 7 Dec 2026."));
    assert!(!html.contains("Agree on PayPal"));
    assert!(html.contains("meter pct-10"));
    assert!(html.contains("What your approval signs"));
    assert!(
        html.contains(r#"id="cancel-totp""#),
        "no passkey on the account: a code"
    );
    assert!(
        html.contains("Invoices aren't listed here yet."),
        "no invoice list it can't source"
    );
    let none = pages()
        .render(
            OnboardingPage::Billing,
            &BillingPage {
                has_plan: false,
                cancel: None,
                spent_words: None,
                budget_words: None,
                ..view
            },
        )
        .unwrap();
    assert!(
        none.contains("No plan on this site yet.") && none.contains("There's no plan to cancel")
    );
}

#[test]
fn pages_load_the_brand_fonts_and_stay_light() {
    let html = pages().render_start(&common::start()).unwrap();
    let brand = html
        .find("/static/ui/brand.css?v=")
        .expect("brand.css is linked");
    let own = html.find("/static/onboarding/onboarding.css?v=").unwrap();
    assert!(
        brand < own,
        "the tokens and fonts load before the page's own styles"
    );
    assert!(
        html.contains(r#"<html lang="en" data-theme="light">"#),
        "light, as the concepts draw it"
    );
}

#[test]
fn a_signed_in_owner_without_a_site_is_told_so() {
    let mut view = common::start();
    view.signed_in = true;
    let html = pages().render_start(&view).unwrap();
    assert!(html.contains("Signed in · add your website"));
    assert!(html.contains("You don't have a site on Agentrix yet."));
    assert!(html.contains(r#"action="/auth/signout""#) && !html.contains(">Sign in<"));
    assert!(!html.contains("No account"));
    let out = pages().render_start(&common::start()).unwrap();
    assert!(out.contains(r#"href="/account/signin">Sign in"#) && out.contains("No account"));
}

#[test]
fn a_claim_message_reads_with_one_full_stop() {
    let mut raw = agentrix_onboarding::view::ClaimPage::from(agentrix_console::view::ClaimView {
        chrome: agentrix_console::view::Chrome {
            site: agentrix_console::view::SiteRef {
                id: "site_1".into(),
                host: "kilele-bakery.example".into(),
                ..Default::default()
            },
            ..Default::default()
        },
        claim: agentrix_console::view::Claim {
            state: agentrix_console::view::ClaimState::Failed,
            said: Some(
                "This kind of sealed room isn't set up on this server.. Nothing was guessed."
                    .into(),
            ),
            ..Default::default()
        },
    });
    assert_eq!(
        raw.said.take().as_deref(),
        Some("This kind of sealed room isn't set up on this server. Nothing was guessed.")
    );
    let html = pages().render_assess(&common::running()).unwrap();
    assert!(
        html.contains(r#"href="/account/signin">Sign in"#),
        "the default header signs in on the new page"
    );
}

#[test]
fn the_live_card_points_at_pictures_only_when_some_wait() {
    use agentrix_onboarding::view::MoveState;
    let mut live = common::move_view(MoveState::Live);
    let none = pages().render_move(&live).unwrap();
    assert!(!none.contains("Pictures for pages that had none"));
    live.pictures_href = Some("/console/site_0193test/apps/pictures".into());
    let some = pages().render_move(&live).unwrap();
    assert!(some.contains(
        r#"Pictures for pages that had none wait for you in <a href="/console/site_0193test/apps/pictures">Pictures</a>."#
    ));
    let mut waiting = common::move_view(MoveState::Waiting);
    waiting.pictures_href = live.pictures_href.clone();
    assert!(
        !pages()
            .render_move(&waiting)
            .unwrap()
            .contains("Pictures for pages that had none"),
        "only once live"
    );
}

#[test]
fn the_move_shows_the_look_chosen_before_it() {
    use agentrix_onboarding::view::{LookRef, MoveState};
    let mut running = common::move_view(MoveState::Running);
    assert!(
        !pages()
            .render_move(&running)
            .unwrap()
            .contains("Your new look")
    );
    running.look = Some(LookRef {
        name: "Oven Warm".into(),
        sentence: "A cosy look in crust colours.".into(),
        href: "/sites/site_0193test/concepts".into(),
    });
    let html = pages().render_move(&running).unwrap();
    assert!(html.contains("Your new look: <b>Oven Warm</b>. A cosy look in crust colours."));
    assert!(html.contains(r#"<a href="/sites/site_0193test/concepts">Change it</a>"#));
}

#[test]
fn the_reach_step_shows_only_with_the_channels_app() {
    use agentrix_onboarding::view::MoveState;
    let live = common::move_view(MoveState::Live);
    let off = pages().render_move(&live).unwrap();
    assert!(
        !off.contains("Where should your WebAgents reach you?"),
        "no Channels app, no step"
    );
    let on = Pages::new()
        .unwrap()
        .with_channels(|site: &str| {
            assert_eq!(site, "site_0193test", "asked per site");
            vec!["device", "telegram", "slack"]
        })
        .render_move(&live)
        .unwrap();
    assert!(on.contains("Where should your WebAgents reach you?"));
    for (id, label) in [
        ("telegram", "Connect"),
        ("device", "Turn on"),
        ("slack", "Connect"),
    ] {
        let link = format!(
            r#"href="/console/site_0193test/apps/channels?connect={id}&amp;back=%2Fconsole%2Fmove">{label}"#
        );
        assert!(on.contains(&link), "{id}");
    }
    assert!(
        on.contains("Always on, whatever else you add."),
        "email is never removed"
    );
    assert!(on.contains("questions your desks pass on. Approving opens the console."));
    assert!(
        !on.contains("connect=whatsapp")
            && !on.contains("connect=discord")
            && !on.contains("connect=matrix")
    );
    assert_eq!(
        on.matches("Not available yet").count(),
        3,
        "WhatsApp, Discord and Matrix until they're live"
    );
    assert!(on.contains("For your team"));
    assert!(on.contains(r#"id="reach-later">Not now"#));
    let none_ready = Pages::new()
        .unwrap()
        .with_channels(|_: &str| Vec::<String>::new())
        .render_move(&live)
        .unwrap();
    assert_eq!(
        none_ready.matches("Not available yet").count(),
        6,
        "no bot for this site: only email is on"
    );
    let waiting = Pages::new()
        .unwrap()
        .with_channels(|_: &str| vec!["telegram"])
        .render_move(&common::move_view(MoveState::Waiting))
        .unwrap();
    assert!(
        !waiting.contains("Where should your WebAgents reach you?"),
        "only once live"
    );
}
