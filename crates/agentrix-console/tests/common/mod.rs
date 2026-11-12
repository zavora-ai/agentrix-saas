//! Sample views for every page and state, used by the render tests and `examples/preview.rs`.
//! Illustrative data on Zavora's fixture site; real pages get their values from node.
#![allow(dead_code)]

use agentrix_console::Page;
use agentrix_console::view::*;
use serde_json::Value;

fn mark(name: &str, glyph: &str, color: &str, state: MarkState, words: &str) -> AgentMark {
    AgentMark {
        name: name.into(),
        glyph: glyph.into(),
        color: color.into(),
        state,
        state_words: words.into(),
    }
}
fn s(x: &str) -> String {
    x.into()
}
fn fact(l: &str, v: &str) -> Fact {
    Fact {
        label: l.into(),
        value: v.into(),
    }
}

pub fn chrome(active: NavTab) -> Chrome {
    Chrome {
        site: SiteRef {
            id: s("site_k1"),
            host: s("kilele-bakery.example"),
            name: s("Kilele Bakery"),
            state_words: s("Moving"),
            fixture: true,
            live_url: None,
        },
        nav: Nav {
            active,
            decisions_waiting: 1,
        },
        owner: OwnerRef { name: s("Wanjiru") },
    }
}

fn tick() -> Tick {
    Tick {
        hosts: 1,
        requests: 38,
        bytes_words: s("2.1 MB"),
        refusals: 3,
    }
}
fn frame() -> Frame {
    Frame {
        kind: FrameKind::SealedScreenshot,
        src: s("/frames/sha256-9f2c"),
        page_url: s("https://kilele-bakery.example/cakes"),
    }
}
fn casino() -> Finding {
    Finding {
        id: s("f1"),
        severity: Severity::Urgent,
        title: s("A hidden script sends phone visitors to a casino site"),
        plain: s(
            "When someone arrives from a search on a phone, this script sends them somewhere else. You wouldn't see it from a laptop.",
        ),
        evidence: vec![Evidence {
            label: s("Your theme's footer, line 88"),
            excerpt: Some(s("<script>eval(atob(\"dmFyIHU9bmF2aWdh…\"))</script>")),
            receipt: Some(s("sha256-7c1ea90b")),
        }],
    }
}
fn ladder(current: usize, ceiling: usize, reason: &str) -> Vec<Position> {
    [
        "Watch",
        "Suggest",
        "Ask me first",
        "Do it and tell me",
        "Handle it within budget",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| Position {
        name: s(n),
        current: i == current,
        locked: i > ceiling,
        reason: if i > ceiling {
            s(reason)
        } else {
            String::new()
        },
    })
    .collect()
}
fn levels() -> Vec<JobLevels> {
    vec![
        JobLevels {
            webagent: s("Editor"),
            job: s("Keep pages true"),
            level_words: s("Suggest: drafts changes for you to approve"),
            above_ask_first: false,
            positions: ladder(1, 4, ""),
        },
        JobLevels {
            webagent: s("Guardian"),
            job: s("Protect the site"),
            level_words: s("Ask me first: rolls back only after you approve"),
            above_ask_first: false,
            positions: ladder(2, 2, "Can't be undone by you alone"),
        },
        JobLevels {
            webagent: s("Concierge"),
            job: s("Reply to customers"),
            level_words: s("Ask me first"),
            above_ask_first: false,
            positions: ladder(2, 2, "Messages customers, so it always asks first"),
        },
    ]
}
fn receipts() -> Vec<ReceiptRow> {
    vec![
        ReceiptRow {
            digest: s("sha256-7c1ea90b11"),
            digest_short: s("7c1e…a90b"),
            title: s("Read 14 pages in a sealed room"),
            kind_words: s("Fetch job"),
            at_words: s("10:04 today"),
            verified: true,
        },
        ReceiptRow {
            digest: s("sha256-2f8841d7aa"),
            digest_short: s("2f88…41d7"),
            title: s("Replayed your WordPress, sealed"),
            kind_words: s("Replay job"),
            at_words: s("10:09 today"),
            verified: true,
        },
        ReceiptRow {
            digest: s("sha256-91a4be03cc"),
            digest_short: s("91a4…be03"),
            title: s("Proved 97% match"),
            kind_words: s("Move step"),
            at_words: s("10:19 today"),
            verified: true,
        },
    ]
}

pub fn start() -> StartView {
    StartView {
        error: None,
        url: None,
        fixture_url: Some(s("/assess?fixture=1")),
    }
}
pub fn assess_running() -> AssessView {
    AssessView {
        id: s("as_1"),
        url: s("https://kilele-bakery.example"),
        host: s("kilele-bakery.example"),
        fixture: true,
        state: AssessState::Running,
        progress_words: s("9 of 14 pages read"),
        started_words: s("10:01"),
        tick: tick(),
        frame: Some(frame()),
        report: None,
        unreachable: None,
        move_href: s("/auth/register?next=/sites/new"),
    }
}
pub fn assess_done() -> AssessView {
    AssessView {
        state: AssessState::Done,
        report: Some(Report {
            summary_words: s(
                "Your site works, but it's infected: a hidden script sends phone visitors to a casino site. It's slow on phones, and AI assistants can't read your prices.",
            ),
            counts: Counts {
                urgent: 1,
                warn: 2,
                info: 1,
            },
            sections: vec![
                ReportSection {
                    title: s("Security"),
                    empty_words: s("No security problems on the pages we read."),
                    findings: vec![casino()],
                },
                ReportSection {
                    title: s("Speed"),
                    empty_words: s(""),
                    findings: vec![Finding {
                        id: s("f2"),
                        severity: Severity::Warn,
                        title: s("Pages take about 6 seconds on a phone"),
                        plain: s("Most of it is one large photo on every page."),
                        evidence: vec![],
                    }],
                },
                ReportSection {
                    title: s("Broken links"),
                    empty_words: s("No broken links on the 14 pages we read."),
                    findings: vec![],
                },
                ReportSection {
                    title: s("Can AI assistants use your site?"),
                    empty_words: s(""),
                    findings: vec![Finding {
                        id: s("f3"),
                        severity: Severity::Warn,
                        title: s("Your prices are in pictures"),
                        plain: s(
                            "An assistant can't read them, so it can't answer \"how much is a birthday cake?\"",
                        ),
                        evidence: vec![],
                    }],
                },
            ],
            pages_read: 14,
            receipt: Some(s("sha256-7c1ea90b11")),
        }),
        ..assess_running()
    }
}
pub fn assess_unreachable() -> AssessView {
    AssessView {
        state: AssessState::Unreachable,
        frame: None,
        unreachable: Some(Unreachable {
            said: s("Your address didn't answer after three tries over two minutes."),
            next_step: s(
                "Check the address is right and the site opens in your browser, then try again.",
            ),
        }),
        ..assess_running()
    }
}
pub fn auth(passkeys: bool) -> AuthView {
    AuthView {
        next: s("/os"),
        passkeys,
    }
}
pub fn claim(method: ClaimMethod, state: ClaimState) -> ClaimView {
    ClaimView {
        chrome: chrome(NavTab::Move),
        claim: Claim {
            method,
            dns_name: s("_agentrix.kilele-bakery.example"),
            dns_value: s("agentrix-site-verification=k1l3-7f2c-91aa"),
            file_name: s("agentrix-verify.txt"),
            file_url: s("https://kilele-bakery.example/.well-known/agentrix-verify.txt"),
            token: s("agentrix-site-verification=k1l3-7f2c-91aa"),
            state,
            checked_words: Some(s("checked 10:02 from the sandbox")),
            said: if state == ClaimState::Failed {
                Some(s("No TXT record yet. Some registrars take up to an hour."))
            } else {
                None
            },
        },
    }
}
fn steps(upto: usize) -> Vec<Step> {
    let names = [
        (
            StepKey::Assess,
            "Assess",
            "Read 14 pages and checked their health",
        ),
        (StepKey::Claim, "Claim", "Proved by DNS record"),
        (
            StepKey::Replay,
            "Replay",
            "Replayed your WordPress from its export, sealed",
        ),
        (StepKey::Clean, "Clean", "Left 4 items behind"),
        (
            StepKey::Rebuild,
            "Rebuild",
            "Built 14 pages; 2 needed another try",
        ),
        (
            StepKey::Prove,
            "Prove",
            "Comparing old and new, page by page",
        ),
        (StepKey::Golive, "Go live", "Waiting for your decision"),
    ];
    names
        .iter()
        .enumerate()
        .map(|(i, (k, n, w))| Step {
            key: *k,
            name: s(n),
            state: if i < upto {
                StepState::Done
            } else if i == upto {
                StepState::Running
            } else {
                StepState::Pending
            },
            summary: if i <= upto { s(w) } else { String::new() },
            took_words: if i < upto {
                s("1 m 48 s")
            } else if i == upto {
                s("40 s")
            } else {
                String::new()
            },
            receipt: if i < upto {
                Some(s("sha256-91a4be03cc"))
            } else {
                None
            },
        })
        .collect()
}
pub fn move_running() -> MoveView {
    MoveView {
        chrome: chrome(NavTab::Move),
        run: MoveRun {
            id: s("mv_1"),
            state: MoveState::Running,
            started_words: s("10:04"),
            progress_words: Some(s("12 of 14 pages")),
        },
        steps: steps(5),
        tick: tick(),
        frame: Some(frame()),
        parity: Some(Parity {
            score_words: s("96%"),
            score_pct: 96,
            pages: vec![
                ParityPage {
                    url: s("/cakes"),
                    state: ParityState::Pass,
                    score_words: s("98%"),
                    verdict: s("Same layout, prices and photos; the new header is lighter."),
                },
                ParityPage {
                    url: s("/order"),
                    state: ParityState::Retry,
                    score_words: s("Retrying (2 of 3)"),
                    verdict: s("The order form lost its date field."),
                },
            ],
        }),
        quarantine: QuarantineSummary {
            count: 4,
            items: vec![QuarantineLine {
                title: s("A hidden script that sends phone visitors to a casino site"),
                reason: s("Redirects visitors"),
                location: s("theme footer"),
            }],
        },
        card: None,
        spend_words: Some(s("Models cost $0.31 on this Move across 22 calls.")),
        uploads: vec![],
    }
}
pub fn move_none() -> MoveView {
    MoveView {
        run: MoveRun {
            id: s(""),
            state: MoveState::None,
            started_words: s(""),
            progress_words: None,
        },
        steps: vec![],
        parity: None,
        frame: None,
        ..move_running()
    }
}
pub fn move_waiting() -> MoveView {
    MoveView {
        run: MoveRun {
            id: s("mv_1"),
            state: MoveState::Waiting,
            started_words: s("10:04"),
            progress_words: None,
        },
        steps: steps(6)
            .into_iter()
            .map(|mut st| {
                if st.key == StepKey::Golive {
                    st.state = StepState::Waiting;
                }
                st
            })
            .collect(),
        card: Some(CardLink {
            title: s("Go live: kilele-bakery.example"),
            href: s("/console/decisions/dc_1"),
            asked_by: mark(
                "Mover",
                "Mv",
                "#2f55c4",
                MarkState::Waiting,
                "Waiting for you",
            ),
        }),
        ..move_running()
    }
}
pub fn today() -> TodayView {
    TodayView {
        chrome: chrome(NavTab::Today),
        greeting: s("Good morning, Wanjiru"),
        chief: mark("Chief WebAgent", "K", "#2b1d14", MarkState::Idle, ""),
        brief: Some(Brief {
            written_words: s("07:00"),
            events_count: 23,
            lines: vec![
                BriefLine {
                    text: s(
                        "Your first night live: 27 visitors, 5 questions answered (1 in Swahili).",
                    ),
                    href: None,
                },
                BriefLine {
                    text: s(
                        "The Guardian blocked 38 requests probing the old WordPress login address.",
                    ),
                    href: Some(s("/console/trust")),
                },
            ],
            spend_words: Some(s("AI spend so far: $0.21 of your $30 budget.")),
        }),
        first_brief_words: s("Your first brief comes at 07:00 tomorrow."),
        needs_you: vec![CardLink {
            title: s("Go live: kilele-bakery.example"),
            href: s("/console/decisions/dc_1"),
            asked_by: mark(
                "Mover",
                "Mv",
                "#2f55c4",
                MarkState::Waiting,
                "Waiting for you",
            ),
        }],
        recent: receipts(),
        passed: vec![
            PassedQuestion {
                question: s("Do you bake <b>sugar-free</b> cakes?"),
                when_words: s("09:41"),
                contact: Some(s("0700 000 000")),
            },
            PassedQuestion {
                question: s("Are you open on Easter Monday?"),
                when_words: s("Yesterday"),
                contact: None,
            },
        ],
        passed_total: 2,
        meter: Some(Meter {
            spent: s("KSh 28"),
            cap_words: s("KSh 28 of KSh 3,870 this month"),
            cap_pct: Some(1),
            cap_note: s("KSh 3,870 a month of AI is included with your Commerce plan."),
            calls_words: s("155 AI calls this month"),
            by_purpose: vec![
                MeterLine {
                    label: s("Building pages"),
                    amount: s("KSh 27"),
                },
                MeterLine {
                    label: s("Screening for hidden instructions"),
                    amount: s("KSh 1"),
                },
            ],
            by_provider: vec![MeterLine {
                label: s("Anthropic"),
                amount: s("KSh 28"),
            }],
            refused_words: None,
        }),
    }
}
pub fn today_first() -> TodayView {
    TodayView {
        brief: None,
        needs_you: vec![],
        recent: vec![],
        passed: vec![],
        passed_total: 0,
        meter: None,
        ..today()
    }
}
pub fn decisions() -> DecisionsView {
    DecisionsView {
        chrome: chrome(NavTab::Decisions),
        waiting: vec![CardSummary {
            id: s("dc_1"),
            title: s("Go live: kilele-bakery.example"),
            asked_by: mark(
                "Mover",
                "Mv",
                "#2f55c4",
                MarkState::Waiting,
                "Waiting for you",
            ),
            risk: Risk::Medium,
            risk_words: s("Medium"),
            created_words: s("Raised 10:21"),
            summary: s("Release v1 · 97% match · 4 items left behind · undo for 30 days"),
            outcome_words: String::new(),
        }],
        decided: vec![],
    }
}
pub fn go_live(state: CardState) -> DecisionView {
    DecisionView {
        chrome: chrome(NavTab::Decisions),
        card: Card {
            id: s("dc_1"),
            kind: CardKind::GoLive,
            title: s("Go live: kilele-bakery.example"),
            asked_by: mark(
                "Mover",
                "Mv",
                "#2f55c4",
                MarkState::Waiting,
                "Waiting for you",
            ),
            risk: Risk::Medium,
            risk_words: s("Medium"),
            why_words: s(
                "Medium because the address of your whole site changes, and undo is one tap for 30 days.",
            ),
            effects: vec![
                s(
                    "Signed release v1 starts serving kilele-bakery.example: 14 pages, 97% match, 31 old addresses redirected.",
                ),
                s("The 4 items found in your old site stay behind."),
                s("Your old site stays exactly as it is, as a fallback, for 30 days."),
                s("Your WebAgents start work at the levels below."),
            ],
            facts: vec![
                fact("Match", "97% (at least 95% needed)"),
                fact("Left behind", "4 items"),
                fact("Undo", "One tap, for 30 days"),
            ],
            levels: levels(),
            fingerprint: s("H(kilele, mover, go_live, {release v1, charter v1}) = 5be0c8f1a2…"),
            fingerprint_short: s("5be0…c8"),
            fields: vec![
                fact("release", "sha256-5be0c8f1a2"),
                fact("charter", "v1 · sha256-11aa"),
                fact("expires", "8 Nov 20:00"),
            ],
            state,
            decided: if state == CardState::Approved {
                Some(Decided {
                    outcome_words: s("Live · release v1"),
                    method_words: s("Your passkey on this iPhone"),
                    at_words: s("10:31 today"),
                    receipt: s("sha256-5be0c8f1a2"),
                })
            } else {
                None
            },
            passkey_options: Some(s(
                "{\"publicKey\":{\"challenge\":\"AAAA\",\"allowCredentials\":[]}}",
            )),
            has_passkey: true,
            approve_url: s("/decisions/dc_1/approve"),
            decline_url: s("/decisions/dc_1/decline"),
        },
    }
}
pub fn trust() -> TrustView {
    TrustView {
        chrome: chrome(NavTab::Trust),
        gauntlet: Some(Gauntlet {
            held: 10,
            of: 10,
            score_words: s("10 / 10"),
            run_words: s("at 14:02 today"),
            receipt: s("sha256-ab12cd34"),
            attacks: vec![
                Attack {
                    name: s("Casino redirect in the theme"),
                    plain: s("Left behind by Clean; never reached the new site."),
                    held: true,
                    receipt: Some(s("sha256-ab12cd35")),
                },
                Attack {
                    name: s("Prompt injection in a page"),
                    plain: s("Read as text, never as an instruction."),
                    held: true,
                    receipt: None,
                },
            ],
        }),
        receipts: receipts(),
        key_id: s("ctl-ed25519-2026-11"),
    }
}
pub fn trust_empty() -> TrustView {
    TrustView {
        gauntlet: None,
        receipts: vec![],
        ..trust()
    }
}
pub fn receipt() -> ReceiptView {
    ReceiptView {
        chrome: chrome(NavTab::Trust),
        receipt: ReceiptDetail {
            digest: s("sha256-7c1ea90b11"),
            title: s("Read 14 pages in a sealed room"),
            kind: s("job.fetch"),
            kind_words: s("Fetch job"),
            at_words: s("10:04–10:05 today"),
            issuer: s("control · kilele-node-1"),
            subject: s("Assessment as_1"),
            facts: vec![
                fact("Allowed", "kilele-bakery.example only"),
                fact("Requests", "38"),
                fact("Refused", "3 (an ad server, a font host, a tracker)"),
                fact("Destroyed", "Yes"),
            ],
            outputs: vec![s("sha256-9f2c…"), s("sha256-a1b2…")],
            verified: true,
            verified_words: s("Signature checked against key ctl-ed25519-2026-11 at 10:05"),
            key_id: s("ctl-ed25519-2026-11"),
            json: s("{\n  \"kind\": \"job.fetch\",\n  \"signature\": \"…\"\n}"),
        },
    }
}
pub fn webagents() -> WebAgentsView {
    WebAgentsView {
        chrome: chrome(NavTab::Webagents),
        agents: vec![
            AgentCard {
                mark: mark("Editor", "Ed", "#577035", MarkState::Idle, ""),
                name: s("Editor"),
                role_words: s("Keeps every page true and fresh"),
                jobs: levels()[..1].to_vec(),
            },
            AgentCard {
                mark: mark(
                    "Guardian",
                    "Gu",
                    "#2b1d14",
                    MarkState::Working,
                    "Watching the login page",
                ),
                name: s("Guardian"),
                role_words: s("Watches the site and can roll it back"),
                jobs: levels()[1..2].to_vec(),
            },
        ],
    }
}
pub fn quarantine() -> QuarantineView {
    QuarantineView {
        chrome: chrome(NavTab::Move),
        items: vec![
            QuarantineItem {
                id: s("q1"),
                title: s("A hidden script that sends phone visitors to a casino site"),
                plain: s(
                    "When someone arrives from a search on a phone, this sends them somewhere else.",
                ),
                location: s("wp-content/themes/kilele/footer.php, line 88"),
                evidence: casino().evidence,
                state: QuarantineState::LeftBehind,
                decision_href: None,
                restore_url: s("/sites/site_k1/quarantine/q1/restore"),
            },
            QuarantineItem {
                id: s("q2"),
                title: s("A booking widget script from an unknown host"),
                plain: s("It loads code from a host we couldn't identify."),
                location: s("the Order page"),
                evidence: vec![],
                state: QuarantineState::RestoreAsked,
                decision_href: Some(s("/console/decisions/dc_2")),
                restore_url: s("/sites/site_k1/quarantine/q2/restore"),
            },
        ],
    }
}

/// Every page in every state worth checking, with a file name for previews.
pub fn all() -> Vec<(&'static str, Page, Value)> {
    let v = |x: &dyn erased::Ser| x.value();
    vec![
        ("start", Page::Start, v(&start())),
        ("assess-running", Page::Assess, v(&assess_running())),
        ("assess-done", Page::Assess, v(&assess_done())),
        ("assess-unreachable", Page::Assess, v(&assess_unreachable())),
        ("signin", Page::SignIn, v(&auth(true))),
        ("signin-no-passkeys", Page::SignIn, v(&auth(false))),
        ("register", Page::Register, v(&auth(true))),
        (
            "claim-dns",
            Page::Claim,
            v(&claim(ClaimMethod::Dns, ClaimState::Pending)),
        ),
        (
            "claim-file-failed",
            Page::Claim,
            v(&claim(ClaimMethod::File, ClaimState::Failed)),
        ),
        (
            "claim-verified",
            Page::Claim,
            v(&claim(ClaimMethod::Dns, ClaimState::Verified)),
        ),
        ("move-none", Page::Move, v(&move_none())),
        ("move-running", Page::Move, v(&move_running())),
        ("move-waiting", Page::Move, v(&move_waiting())),
        ("today", Page::Today, v(&today())),
        ("today-first", Page::Today, v(&today_first())),
        ("decisions", Page::Decisions, v(&decisions())),
        ("golive", Page::Decision, v(&go_live(CardState::Waiting))),
        (
            "golive-approved",
            Page::Decision,
            v(&go_live(CardState::Approved)),
        ),
        ("trust", Page::Trust, v(&trust())),
        ("trust-empty", Page::Trust, v(&trust_empty())),
        ("receipt", Page::Receipt, v(&receipt())),
        ("webagents", Page::WebAgents, v(&webagents())),
        ("quarantine", Page::Quarantine, v(&quarantine())),
    ]
}

mod erased {
    pub trait Ser {
        fn value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn value(&self) -> serde_json::Value {
            serde_json::to_value(self).expect("views serialise")
        }
    }
}
