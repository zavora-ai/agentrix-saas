//! Views for the render tests and the preview, in the shapes node's Assess builds.
//! Test data only: it never reaches a live page.

#![allow(dead_code)]

use agentrix_console::view as console;
use agentrix_onboarding::view::{AssessPage, SampleLink, StartPage};

/// The address bar, empty.
pub fn start() -> StartPage {
    StartPage::from(console::StartView {
        error: None,
        url: None,
        fixture_url: Some("/assess/fixture".into()),
    })
}

/// The address bar after a bad address.
pub fn start_error() -> StartPage {
    StartPage::from(console::StartView {
        error: Some("That address isn't on the public internet.".into()),
        url: Some("bakery.local".into()),
        fixture_url: None,
    })
    .with_sample(Some(SampleLink {
        name: "Kilele Bakery".into(),
        href: "https://kilele-bakery-sample.agentrix.website/".into(),
    }))
}

fn base(state: console::AssessState) -> console::AssessView {
    console::AssessView {
        id: "asmt_0193test".into(),
        url: "https://fixture.example/".into(),
        host: "fixture.example".into(),
        fixture: true,
        state,
        progress_words: "4 of 12 pages read".into(),
        started_words: "8 Nov 2026, 06:02 UTC".into(),
        tick: console::Tick {
            hosts: 1,
            requests: 9,
            bytes_words: "412 KB".into(),
            refusals: 1,
        },
        frame: None,
        report: None,
        unreachable: None,
        move_href: "/auth/signin?next=/sites/site_0193test/claim".into(),
    }
}

/// Reading, before the first frame.
pub fn running() -> AssessPage {
    base(console::AssessState::Running).into()
}

/// Reading, with a frame.
pub fn running_with_frame() -> AssessPage {
    let mut v = base(console::AssessState::Running);
    v.frame = Some(console::Frame {
        kind: console::FrameKind::SealedScreenshot,
        src: "/frames/sha256-test".into(),
        page_url: "https://fixture.example/menu".into(),
    });
    v.into()
}

fn finding(id: &str, severity: console::Severity, title: &str) -> console::Finding {
    console::Finding {
        id: id.into(),
        severity,
        title: title.into(),
        plain: format!("{title}, in plain words."),
        evidence: vec![console::Evidence {
            label: "/blog/summer-offers, line 214".into(),
            excerpt: Some("<div style=\"display:none\"><a href=\"https://bet.example/\">".into()),
            receipt: Some("sha256-receipt".into()),
        }],
    }
}

/// The report: findings in two areas, two areas fine.
pub fn done() -> AssessPage {
    let mut v = base(console::AssessState::Done);
    v.report = Some(console::Report {
        summary_words: "We read 12 pages of fixture.example in a sealed room and found 1 problem that needs attention now.".into(),
        counts: console::Counts { urgent: 1, warn: 1, info: 1 },
        sections: vec![
            console::ReportSection {
                title: "Security".into(),
                empty_words: "No signs of tampering.".into(),
                findings: vec![
                    finding("f2", console::Severity::Warn, "An old WordPress with a public login page"),
                    finding("f1", console::Severity::Urgent, "Hidden links you didn't put there"),
                ],
            },
            console::ReportSection {
                title: "Speed".into(),
                empty_words: "All 12 pages answered our sealed room in under 3 seconds (the slowest took 1.2 s).".into(),
                findings: vec![],
            },
            console::ReportSection {
                title: "Broken links".into(),
                empty_words: "No broken links among the 12 pages we read.".into(),
                findings: vec![],
            },
            console::ReportSection {
                title: "Can AI assistants use your site?".into(),
                empty_words: String::new(),
                findings: vec![finding("f3", console::Severity::Info, "No structured data for your products")],
            },
        ],
        pages_read: 12,
        receipt: Some("sha256-report".into()),
    });
    v.into()
}

/// The site couldn't be reached.
pub fn unreachable() -> AssessPage {
    let mut v = base(console::AssessState::Unreachable);
    v.fixture = false;
    v.unreachable = Some(console::Unreachable {
        said: "fixture.example didn't answer within 20 seconds.".into(),
        next_step: "Check the address, or try again in a few minutes.".into(),
    });
    v.into()
}

/// robots.txt asked us not to read it.
pub fn refused() -> AssessPage {
    let mut v = base(console::AssessState::Refused);
    v.unreachable = Some(console::Unreachable {
        said: "Its robots.txt asks readers like ours to stay away.".into(),
        next_step: "If it's your site, allow Agentrix's reader in robots.txt and try again.".into(),
    });
    v.into()
}

fn claim_view(state: console::ClaimState, method: console::ClaimMethod) -> console::ClaimView {
    console::ClaimView {
        chrome: console::Chrome {
            site: console::SiteRef {
                id: "site_0193test".into(),
                host: "kilele-bakery.example".into(),
                name: "kilele-bakery.example".into(),
                state_words: "In setup".into(),
                fixture: false,
                live_url: None,
            },
            nav: console::Nav::default(),
            owner: console::OwnerRef {
                name: "Test owner".into(),
            },
        },
        claim: console::Claim {
            method,
            dns_name: "_agentrix.kilele-bakery.example".into(),
            dns_value: "agentrix-site-verification=k1l3-7f2c".into(),
            file_name: "agentrix-verify.txt".into(),
            file_url: "https://kilele-bakery.example/.well-known/agentrix-verify.txt".into(),
            token: "agentrix-site-verification=k1l3-7f2c".into(),
            state,
            checked_words: None,
            said: None,
        },
    }
}

/// The claim, by DNS, before any check.
pub fn claim_dns() -> agentrix_onboarding::view::ClaimPage {
    claim_view(console::ClaimState::Pending, console::ClaimMethod::Dns).into()
}

/// The claim, by file, after a check that found nothing.
pub fn claim_file_failed() -> agentrix_onboarding::view::ClaimPage {
    let mut v = claim_view(console::ClaimState::Failed, console::ClaimMethod::File);
    v.claim.said = Some("No file at that address yet (HTTP 404).".into());
    v.claim.checked_words = Some("Checked 10:02 UTC from the sandbox".into());
    v.into()
}

/// The claim, proved.
pub fn claim_verified() -> agentrix_onboarding::view::ClaimPage {
    claim_view(console::ClaimState::Verified, console::ClaimMethod::Dns).into()
}

fn step(
    key: console::StepKey,
    name: &str,
    state: console::StepState,
    summary: &str,
    took: &str,
) -> console::Step {
    console::Step {
        key,
        name: name.into(),
        state,
        summary: summary.into(),
        took_words: took.into(),
        receipt: (state == console::StepState::Done).then(|| format!("sha256:{:0>64}", name.len())),
    }
}

/// A Move in the given state, its steps up to `upto` done.
pub fn move_view(state: console::MoveState) -> agentrix_onboarding::view::MovePage {
    use console::{StepKey as K, StepState as S};
    let at = |k: K| -> S {
        let order = [
            K::Assess,
            K::Claim,
            K::Replay,
            K::Clean,
            K::Rebuild,
            K::Prove,
            K::Golive,
        ];
        let pos = order.iter().position(|x| *x == k).unwrap();
        let now = match state {
            console::MoveState::Running => 4,
            console::MoveState::Waiting => 6,
            console::MoveState::Live => 7,
            console::MoveState::Stopped => 3,
            console::MoveState::None => 0,
        };
        if pos < now {
            S::Done
        } else if pos == now {
            match state {
                console::MoveState::Running => S::Running,
                console::MoveState::Waiting => S::Waiting,
                console::MoveState::Stopped => S::Failed,
                _ => S::Pending,
            }
        } else {
            S::Pending
        }
    };
    let v = console::MoveView {
        chrome: console::Chrome {
            site: console::SiteRef {
                id: "site_0193test".into(),
                host: "kilele-bakery.example".into(),
                name: "kilele-bakery.example".into(),
                state_words: "Moving".into(),
                fixture: false,
                live_url: (state == console::MoveState::Live)
                    .then(|| "https://kilele-bakery-example.agentrix.website/".into()),
            },
            nav: console::Nav::default(),
            owner: console::OwnerRef {
                name: "Test owner".into(),
            },
        },
        run: console::MoveRun {
            id: if state == console::MoveState::None {
                String::new()
            } else {
                "move_0193test".into()
            },
            state,
            started_words: "07:02".into(),
            progress_words: (state == console::MoveState::Running).then(|| "9 of 14 pages".into()),
        },
        steps: if state == console::MoveState::None {
            vec![]
        } else {
            vec![
                step(
                    K::Assess,
                    "Assess",
                    at(K::Assess),
                    "Read 14 pages in a sealed room.",
                    "1 m 48 s",
                ),
                step(K::Claim, "Claim", at(K::Claim), "DNS record found.", "2 s"),
                step(
                    K::Replay,
                    "Replay",
                    at(K::Replay),
                    "Replayed the WordPress export with no network.",
                    "3 m 42 s",
                ),
                step(
                    K::Clean,
                    "Clean",
                    at(K::Clean),
                    "4 items left behind.",
                    "12 s",
                ),
                step(
                    K::Rebuild,
                    "Rebuild",
                    at(K::Rebuild),
                    "Built 14 pages.",
                    "2 m 10 s",
                ),
                step(K::Prove, "Prove", at(K::Prove), "97% match.", "4 m 2 s"),
                step(
                    K::Golive,
                    "Go live",
                    at(K::Golive),
                    "Waiting for your approval.",
                    "",
                ),
            ]
        },
        tick: console::Tick {
            hosts: 1,
            requests: 31,
            bytes_words: "1.2 MB".into(),
            refusals: 2,
        },
        frame: None,
        parity: matches!(
            state,
            console::MoveState::Waiting | console::MoveState::Live
        )
        .then(|| console::Parity {
            score_words: "97%".into(),
            score_pct: 97,
            pages: vec![
                console::ParityPage {
                    url: "/cakes".into(),
                    state: console::ParityState::Pass,
                    score_words: "98%".into(),
                    verdict: "Same words and prices; the new layout is cleaner.".into(),
                },
                console::ParityPage {
                    url: "/gallery".into(),
                    state: console::ParityState::Flagged,
                    score_words: "88%".into(),
                    verdict: "Two photos are missing from the old page.".into(),
                },
            ],
        }),
        quarantine: console::QuarantineSummary {
            count: 1,
            items: vec![console::QuarantineLine {
                title: "Three invisible links to gambling sites".into(),
                reason: "Hidden from people but read by search engines.".into(),
                location: "/blog/summer-offers".into(),
            }],
        },
        card: (state == console::MoveState::Waiting).then(|| console::CardLink {
            title: "Go live: kilele-bakery.example".into(),
            href: "/console/site_0193test/decisions/dec_0193test".into(),
            asked_by: console::AgentMark::default(),
        }),
        spend_words: (state != console::MoveState::None)
            .then(|| "Models cost $0.31 on this Move across 22 calls.".into()),
        uploads: vec![],
    };
    v.into()
}
