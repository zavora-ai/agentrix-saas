//! Every link a message can carry leads to a page that exists.
//!
//! [`PLATFORM`] and [`SITE`] are the pages messages may link to, each with the crate that
//! serves it; [`ELSEWHERE`] is the one address off the platform. The tests render every
//! template (its example, plus the branches the example doesn't reach) and every context this
//! crate builds (W6's sign-in mail, W2's order and digest mail), pull out every link (email
//! text and HTML, push, chat) and check each one against the lists. A template that makes up a
//! path, or a page that moves, fails here before an owner taps a dead button.
//!
//! The lists hold the pages messages should link to, not every route: the first release's
//! `/auth/signin` and `/console/decisions/{id}` still answer, but the console moved on, so
//! they're left out. Linking to a page that isn't listed: open it on a local node first (a
//! GET), then add its pattern here with the crate that serves it. Never list a page that
//! doesn't exist yet.

use serde_json::{Value as Json, json};

use agentrix_identity::mailer::{CodePurpose, Mail, MailKind};
use agentrix_types::SiteId;
use agentrix_types::tenancy::SiteHost;

use crate::catalogue;
use crate::orders::{self, Line, OrderEvent, OrderFacts, SiteFacts, Story};
use crate::render::{Rendered, render};

/// The platform's host in every example and test context.
const PLATFORM_HOST: &str = "agentrix.website";

/// Pages on the platform host (owner, teammate and prospect messages, and push). In a
/// pattern, `{x}` is one path segment, `{n}` one segment of digits, and `?key` after the path
/// means the page needs that query parameter.
const PLATFORM: &[(&str, &str)] = &[
    // agentrix-shell: the owner console.
    ("/os", "agentrix-shell: the signed-in owner's console"),
    ("/console/{site}", "agentrix-shell: a site's console"),
    (
        "/console/{site}/trust",
        "agentrix-shell: Trust (Freeze, receipts)",
    ),
    (
        "/console/{site}/missions/{id}",
        "agentrix-shell: one mission",
    ),
    ("/d/{card}", "agentrix-shell: a decision card's short link"),
    // Apps in the shell's dock answer only under their own id (an unknown one is a 404).
    (
        "/console/{site}/apps/people",
        "agentrix-identity: People, in agentrix-shell's dock",
    ),
    (
        "/console/{site}/apps/channels",
        "agentrix-channels: Channels, in agentrix-shell's dock",
    ),
    // agentrix-node (the first release).
    ("/console", "agentrix-node: redirects to /os"),
    ("/console/move", "agentrix-node: the Move"),
    ("/receipts/{digest}", "agentrix-node: one signed receipt"),
    ("/sites/{site}/claim", "agentrix-node: a domain claim"),
    ("/assess", "agentrix-node: the website check"),
    ("/assess/{id}", "agentrix-node: a website check's report"),
    // agentrix-onboarding: owner accounts, setup and billing.
    ("/start", "agentrix-onboarding: start"),
    ("/account/new", "agentrix-onboarding: make an owner account"),
    ("/account/signin", "agentrix-onboarding: owner sign-in"),
    (
        "/account/recover",
        "agentrix-onboarding: get back in; lists a signed-in owner's recovery holds",
    ),
    (
        "/billing?site",
        "agentrix-onboarding: plan, AI budget and spend, cancel",
    ),
    // agentrix-operator: the owner's side of support access.
    (
        "/support-access/{grant}",
        "agentrix-operator: one support grant",
    ),
    // agentrix-samples: the sample back office, the only owner pages for orders and questions.
    (
        "/samples/{site}/orders/{order}",
        "agentrix-samples: the back office's order page",
    ),
    (
        "/samples/{site}/questions",
        "agentrix-samples: questions passed to the owner",
    ),
    // agentrix-store.
    ("/status", "agentrix-store: the public status page"),
    // agentrix-home: the weekly fix on agentrix.website.
    (
        "/_home/fix/confirm?token",
        "agentrix-home: confirms the weekly fix",
    ),
    (
        "/_home/fix/stop?token",
        "agentrix-home: stops the weekly fix",
    ),
];

/// Pages on a site's own host (customer and subscriber messages).
const SITE: &[(&str, &str)] = &[
    ("/", "agentrix-samples: the site's home"),
    ("/help", "agentrix-samples: Kilele Bakery's help page"),
    ("/orders/{id}", "agentrix-samples: the customer's order"),
    (
        "/orders/{id}/pay",
        "agentrix-samples: pay for an approved order",
    ),
    (
        "/orders/{id}/receipt",
        "agentrix-samples: the signed receipt",
    ),
    (
        "/account",
        "agentrix-samples: the customer's account (shops)",
    ),
    (
        "/account/security",
        "agentrix-identity: sign-in and security",
    ),
    (
        "/account/data",
        "agentrix-identity: download or delete your data",
    ),
    ("/digest", "agentrix-samples: Tana's digest sign-up"),
    (
        "/digest/confirm?token",
        "agentrix-samples: confirms the digest",
    ),
    ("/digest/stop?token", "agentrix-samples: stops the digest"),
    (
        "/{n}/{n}/{slug}",
        "agentrix-samples: a Tana Valley Times story",
    ),
];

/// Addresses off the platform, exactly.
const ELSEWHERE: &[(&str, &str)] = &[(
    "https://wa.me/254721490933",
    "Zavora's WhatsApp, in owner and prospect footers",
)];

/// Whether `path` (and its `query`) is a page `pattern` describes.
fn matches(pattern: &str, path: &str, query: Option<&str>) -> bool {
    let (pat_path, needs) = match pattern.split_once('?') {
        Some((p, key)) => (p, Some(key)),
        None => (pattern, None),
    };
    let want: Vec<&str> = pat_path.split('/').collect();
    let got: Vec<&str> = path.split('/').collect();
    if want.len() != got.len() {
        return false;
    }
    let same = want.iter().zip(&got).all(|(w, g)| match *w {
        "{n}" => !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit()),
        w if w.starts_with('{') && w.ends_with('}') => !g.is_empty(),
        w => w == *g,
    });
    same && needs.is_none_or(|key| {
        query.unwrap_or_default().split('&').any(|kv| {
            kv.split_once('=')
                .is_some_and(|(k, v)| k == key && !v.is_empty())
        })
    })
}

/// Checks one link: `Ok` with the page's owner, or why it's dead.
fn check(link: &str) -> Result<&'static str, String> {
    if let Some((_, why)) = ELSEWHERE.iter().find(|(u, _)| *u == link) {
        return Ok(why);
    }
    let base = url::Url::parse(&format!("https://{PLATFORM_HOST}")).expect("base");
    let url = url::Url::parse(link)
        .or_else(|_| base.join(link))
        .map_err(|e| format!("`{link}` isn't a link: {e}"))?;
    let host = url.host_str().unwrap_or_default();
    let (table, list) = if host == PLATFORM_HOST {
        (PLATFORM, "PLATFORM")
    } else if ELSEWHERE.iter().any(|(u, _)| {
        url::Url::parse(u)
            .ok()
            .is_some_and(|e| e.host_str() == Some(host))
    }) {
        return Err(format!("`{link}` isn't one of the addresses in ELSEWHERE"));
    } else {
        (SITE, "SITE")
    };
    table
        .iter()
        .find(|(pattern, _)| matches(pattern, url.path(), url.query()))
        .map(|(_, by)| *by)
        .ok_or_else(|| format!("`{link}`: {} isn't a page in {list}", url.path()))
}

/// Every link a rendered message carries, on every channel.
fn links(r: &Rendered) -> Vec<String> {
    fn in_text(text: &str, out: &mut Vec<String>) {
        for word in text.split_whitespace() {
            if let Some(i) = word.find("https://").or_else(|| word.find("http://")) {
                let link = word[i..].trim_end_matches(['.', ',', ';', ':', ')', '"', '\'']);
                out.push(link.to_string());
            }
        }
    }
    let mut out = Vec::new();
    if let Some(e) = &r.email {
        in_text(&e.text, &mut out);
        for part in e.html.split("href=\"").skip(1) {
            let href = part.split('"').next().unwrap_or_default();
            out.push(href.replace("&amp;", "&"));
        }
    }
    if let Some(p) = &r.push {
        out.push(p.url.clone());
    }
    if let Some(c) = &r.chat {
        in_text(&c.text, &mut out);
        if let Some((_, url)) = &c.link {
            out.push(url.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Renders `id` with `ctx` and checks every link; failures name `from`.
fn check_all(id: &str, ctx: &Json, from: &str, failures: &mut Vec<String>) -> usize {
    match render(id, ctx) {
        Ok(r) => {
            let found = links(&r);
            for link in &found {
                if let Err(e) = check(link) {
                    failures.push(format!("{id} ({from}): {e}"));
                }
            }
            found.len()
        }
        Err(e) => {
            failures.push(format!("{id} ({from}): doesn't render: {e}"));
            0
        }
    }
}

/// `ctx` with each JSON pointer set to its value.
fn with(mut ctx: Json, changes: &[(&str, Json)]) -> Json {
    for (pointer, value) in changes {
        *ctx.pointer_mut(pointer)
            .unwrap_or_else(|| panic!("no {pointer} in the example")) = value.clone();
    }
    ctx
}

/// The branches with links that the examples don't reach.
fn branches() -> Vec<(&'static str, &'static str, Json)> {
    let ex = |id: &str| catalogue::get(id).expect(id).example_context();
    vec![
        (
            "owner.platform_notice",
            "a hold",
            with(
                ex("owner.platform_notice"),
                &[
                    ("/notice/kind", json!("hold")),
                    ("/notice/reason", json!("Card payments look like fraud")),
                    ("/notice/effect", json!("Checkout is closed")),
                ],
            ),
        ),
        (
            "owner.platform_notice",
            "a hold lifted",
            with(
                ex("owner.platform_notice"),
                &[("/notice/kind", json!("hold_lifted"))],
            ),
        ),
        (
            "owner.payment_failed",
            "day 14",
            with(
                ex("owner.payment_failed"),
                &[
                    ("/dunning/day", json!(14)),
                    ("/dunning/read_only_since", json!("Mon 28 Dec")),
                ],
            ),
        ),
        (
            "owner.payment_failed",
            "day 30",
            with(
                ex("owner.payment_failed"),
                &[
                    ("/dunning/day", json!(30)),
                    ("/dunning/closes_on", json!("Wed 13 Jan")),
                ],
            ),
        ),
        (
            "owner.plan_changed",
            "cancelled",
            with(
                ex("owner.plan_changed"),
                &[
                    ("/change/kind", json!("cancelled")),
                    ("/change/paid_until", json!("Sun 13 Dec")),
                    ("/change/closes_on", json!("Tue 12 Jan")),
                ],
            ),
        ),
        (
            "owner.support_access",
            "started",
            with(
                ex("owner.support_access"),
                &[
                    ("/access/state", json!("started")),
                    ("/access/at", json!("Sat 14 Nov, 11:00")),
                    ("/access/ends", json!("Sat 14 Nov, 15:00")),
                ],
            ),
        ),
        (
            "owner.morning_brief",
            "nothing waiting",
            with(ex("owner.morning_brief"), &[("/brief/waiting", json!(0))]),
        ),
    ]
}

fn host(site: &str, host: &str, origin: &str, name: &str) -> SiteHost {
    SiteHost {
        site: SiteId::new(site),
        host: host.into(),
        origin: origin.into(),
        name: name.into(),
        sample: site != "platform",
    }
}

/// Every kind of mail W6 sends.
fn mail_kinds() -> Vec<MailKind> {
    let code = |purpose| MailKind::Code {
        code: "482913".into(),
        purpose,
        minutes: 10,
    };
    vec![
        code(CodePurpose::SignIn),
        code(CodePurpose::Create),
        code(CodePurpose::Verify),
        code(CodePurpose::Recover),
        MailKind::NewSignIn {
            device: "Chrome on a Windows PC".into(),
            when: "Wed 11 Nov 2026, 21:14 Nairobi time".into(),
        },
        MailKind::CodeSignIn {
            device: "Chrome on a Mac".into(),
            when: "Sun 8 Nov, 07:40".into(),
            method: "an email code".into(),
        },
        MailKind::RecoveryHold {
            release: "Mon 9 Nov, 07:41".into(),
        },
        MailKind::TeamInvite {
            business: "Kilele Bakery".into(),
            inviter: "The owner of Kilele Bakery".into(),
            role: "a manager, who can approve decisions".into(),
            link: "https://agentrix.website/team/join?t=abc".into(),
        },
        MailKind::RecoveryCodeUsed {
            device: "Safari on an iPhone".into(),
            remaining: 1,
        },
    ]
}

/// An order as W2's samples hand it over: links on the site's own host.
fn kilele_order() -> (SiteFacts, OrderFacts) {
    let origin = "https://kilele-bakery-sample.agentrix.website";
    let site = SiteFacts {
        id: "kilele-bakery-sample".into(),
        name: "Kilele Bakery".into(),
        host: "kilele-bakery-sample.agentrix.website".into(),
        sample: true,
        owner_first: "Wanjiru".into(),
        accent: Some("#3b2416".into()),
    };
    let order = OrderFacts {
        order_id: "KB-1042".into(),
        customer_name: "Njeri Wambui".into(),
        kind: "custom".into(),
        lines: vec![Line {
            name: "Lemon sponge cake".into(),
            qty: 1,
            amount: "KSh 2,250".into(),
        }],
        delivery: Some("KSh 300".into()),
        total: "KSh 2,550".into(),
        usd: Some("USD 19.77".into()),
        rate: Some("1 USD = KSh 129.00".into()),
        fulfilment: "delivery".into(),
        slot: "Sat 14 Nov, 10:00–12:00".into(),
        recipient: Some("Amina, at home in Kilimani".into()),
        note: None,
        order_url: format!("{origin}/orders/KB-1042"),
        pay_url: Some(format!("{origin}/orders/KB-1042/pay")),
        receipt_url: None,
    };
    (site, order)
}

fn order_events() -> Vec<OrderEvent> {
    vec![
        OrderEvent::Received,
        OrderEvent::RequestHeld {
            answer_within: "2 hours".into(),
            expires: "Thu 19 Nov, 20:00".into(),
        },
        OrderEvent::RequestApproved {
            pay_by: "Fri 20 Nov, 12:00".into(),
        },
        OrderEvent::RequestDeclined { reason: None },
        OrderEvent::ChangeSuggested {
            text: "Vanilla instead of lemon".into(),
            new_total: None,
            answer_by: "Thu 19 Nov, 20:00".into(),
        },
        OrderEvent::RequestLapsed {
            expired_at: "Thu 19 Nov, 20:00".into(),
        },
        OrderEvent::Receipt {
            paid_at: "Thu 12 Nov, 14:06 (Nairobi time)".into(),
            receipt_id: "R-1042".into(),
        },
        OrderEvent::Status {
            label: "Out for delivery".into(),
            by: "Wanjiru".into(),
            at: "10:05".into(),
            note: None,
        },
        OrderEvent::ChangeAnswered {
            approved: true,
            asked: "Deliver at 11:00".into(),
            result: "Delivery at 11:00".into(),
            reason: None,
        },
        OrderEvent::Refunded {
            refund_id: "R-1042-1".into(),
            amount: "USD 10.47".into(),
            local: None,
            by: "The Cashier".into(),
            what: "the bake box".into(),
        },
        OrderEvent::RefundHeld {
            amount: "KSh 5,250".into(),
            what: "the lemon sponge cake".into(),
            decide_by: "Sun 15 Nov, 20:00".into(),
        },
    ]
}

#[test]
fn the_route_lists_catch_the_dead_links_we_shipped() {
    for dead in [
        "https://agentrix.website/os/webagents/budget",
        "https://agentrix.website/os/trust/devices",
        "https://agentrix.website/os/trust/people",
        "https://agentrix.website/os/review",
        "https://agentrix.website/os/billing",
        "/console/trust/devices",
        "/console/webagents/budget",
        "https://agentrix.website/console/kilele-bakery-sample/apps/orders",
        "https://agentrix.website/team/join?t=abc",
        "https://agentrix.website/billing",
        "https://agentrix.website/fix/stop?token=nl_3b8x",
        "https://kilele-bakery-sample.agentrix.website/help/contact",
        "https://kilele-bakery-sample.agentrix.website/account/data/export/e-7f3a",
        "https://kilele-bakery-sample.agentrix.website/orders/KB-1042/receipt/x",
        "https://wa.me/254700000000",
    ] {
        assert!(check(dead).is_err(), "{dead} passed");
    }
    for live in [
        "/os",
        "https://agentrix.website/console/kilele-bakery-sample/apps/people",
        "https://agentrix.website/billing?site=kilele-bakery-sample",
        "https://agentrix.website/account/signin?next=%2Faccount%2Frecover",
        "https://tana-valley-times-sample.agentrix.website/2026/11/ward-offices-to-open-on-saturdays",
        "https://kilele-bakery-sample.agentrix.website",
        "https://wa.me/254721490933",
    ] {
        assert!(check(live).is_ok(), "{live}: {:?}", check(live));
    }
}

#[test]
fn every_template_links_only_to_pages_that_exist() {
    let mut failures = Vec::new();
    let mut seen = 0;
    for def in catalogue::all() {
        seen += check_all(def.id, &def.example_context(), "example", &mut failures);
    }
    for (id, branch, ctx) in branches() {
        seen += check_all(id, &ctx, branch, &mut failures);
    }
    assert!(
        seen > 100,
        "only {seen} links found: is the extraction broken?"
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn every_link_this_crate_builds_leads_to_a_page_that_exists() {
    let mut failures = Vec::new();
    let platform = host(
        "platform",
        PLATFORM_HOST,
        "https://agentrix.website",
        "Agentrix",
    );
    let kilele = host(
        "kilele-bakery-sample",
        "kilele-bakery-sample.agentrix.website",
        "https://kilele-bakery-sample.agentrix.website",
        "Kilele Bakery",
    );
    for kind in mail_kinds() {
        let mail = Mail {
            site: platform.clone(),
            to: "wanjiru@example.com".into(),
            kind: kind.clone(),
        };
        let (id, ctx, _) = crate::identity::owner(&mail);
        check_all(id, &ctx, "W6 owner mail", &mut failures);
        let mail = Mail {
            site: kilele.clone(),
            to: "njeri@example.com".into(),
            kind,
        };
        if let Some((id, ctx, _)) = crate::identity::customer(&mail) {
            check_all(id, &ctx, "W6 customer mail", &mut failures);
        }
    }

    let (site, order) = kilele_order();
    for event in order_events() {
        let out = orders::customer(&site, &order, &event, "njeri@example.com");
        check_all(&out.message, &out.context, "order mail", &mut failures);
    }
    let (id, ctx, _, _) = orders::owner_new_order(&site, &order, "https://agentrix.website");
    check_all(&id, &ctx, "owner_new_order", &mut failures);

    let tana = SiteFacts {
        id: "tana-valley-times-sample".into(),
        name: "Tana Valley Times".into(),
        host: "tana-valley-times-sample.agentrix.website".into(),
        sample: true,
        owner_first: "Grace".into(),
        accent: None,
    };
    let origin = "https://tana-valley-times-sample.agentrix.website";
    let story = Story {
        headline: "Ward offices to open on Saturdays".into(),
        standfirst: "From 21 Nov, every ward office opens 09:00 to 13:00.".into(),
        url: format!("{origin}/2026/11/ward-offices-to-open-on-saturdays"),
        section: "County".into(),
        byline: "Grace Wairimu".into(),
        published: None,
        corrected: false,
        correction_note: None,
    };
    for out in [
        orders::digest_confirm(
            &tana,
            "reader@example.com",
            "County",
            &format!("{origin}/digest/confirm?token=ab12"),
        ),
        orders::digest_weekly(
            &tana,
            "reader@example.com",
            "County",
            "Fri 13 Nov",
            &[story],
            &format!("{origin}/digest/stop?token=ab12"),
        ),
        orders::digest_stopped(
            &tana,
            "reader@example.com",
            Some(&format!("{origin}/digest")),
        ),
    ] {
        check_all(&out.message, &out.context, "digest mail", &mut failures);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn templates_print_console_url_as_it_is() {
    // `console_url` is `/console/{site}` from some senders and `/os` from others, so a path
    // added to it works for one and is a 404 for the other (fix first #3 in 04-ui-parity.md).
    // A deeper page gets its own key in the message's context.
    let name = "console_url";
    for def in catalogue::all() {
        let mut rest = def.source;
        while let Some(i) = rest.find(name) {
            let after = rest[i + name.len()..].trim_start();
            let after = after.strip_prefix("}}").unwrap_or(after);
            assert!(
                !after.starts_with('/'),
                "{}: adds a path to console_url; give that page its own key",
                def.id
            );
            rest = &rest[i + name.len()..];
        }
    }
}
