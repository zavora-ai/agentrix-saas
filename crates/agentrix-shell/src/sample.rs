//! A [`ShellPort`] with fixed sample data, for tests and the local preview
//! (`scripts/wcargo run -p agentrix-shell --example shell-preview`). Never wired into node. Every name on it is
//! from the concept's sample bakery and the site is marked Sample.

use std::sync::Mutex;

use agentrix_console::view as cv;
use agentrix_types::{ApprovalMethod, Digest, ReceiptBody};
use async_trait::async_trait;
use axum::http::HeaderMap;
use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use crate::map;
use crate::port::{
    Access, Attempt, CardRow, Happened, Owner, PortResult, Proof, ShellPort, SiteInfo, Step,
    WorkCard,
};

/// The sample port. `signed_in` false sends every page to sign-in.
pub struct SamplePort {
    /// Whether someone is signed in.
    pub signed_in: bool,
    /// Whether the owner has a passkey.
    pub has_passkey: bool,
    /// When the sample was made; times are relative to it.
    pub now: DateTime<Utc>,
    /// Receipts issued through [`ShellPort::issue`].
    pub issued: Mutex<Vec<ReceiptBody>>,
}

impl Default for SamplePort {
    fn default() -> Self {
        Self {
            signed_in: true,
            has_passkey: true,
            now: Utc::now(),
            issued: Mutex::new(Vec::new()),
        }
    }
}

/// The sample site's id.
pub const SITE: &str = "site_kilele_sample";

fn owner_of(has_passkey: bool) -> Owner {
    Owner {
        account: "0190f5c2-0000-7000-8000-000000000001".into(),
        name: "Wanjiru".into(),
        has_passkey,
        site: SiteInfo {
            id: SITE.into(),
            name: "Kilele Bakery".into(),
            host: "kilele-bakery-sample.agentrix.website".into(),
            state_words: "Live since Sat 7 Nov, 09:41".into(),
            live_url: Some("https://kilele-bakery-sample.agentrix.website/".into()),
            fixture: false,
            sample: true,
            live: true,
            utc_offset_minutes: 180,
        },
    }
}

fn digest(n: u8) -> String {
    format!("sha256:{}", format!("{n:02x}").repeat(32))
}

fn rr(n: u8, title: &str, kind: &str, at: &str) -> cv::ReceiptRow {
    let d = digest(n);
    cv::ReceiptRow {
        digest_short: format!("{}…{}", &d[7..11], &d[d.len() - 4..]),
        digest: d,
        title: title.into(),
        kind_words: kind.into(),
        at_words: at.into(),
        verified: false,
    }
}

impl SamplePort {
    fn card_rows(&self) -> Vec<CardRow> {
        let now = self.now;
        let row = |id: &str,
                   kind: &str,
                   who: &str,
                   title: &str,
                   summary: &str,
                   risk: cv::Risk,
                   risk_words: &str,
                   ago: i64,
                   state: cv::CardState| CardRow {
            id: id.into(),
            kind: kind.into(),
            title: title.into(),
            summary: summary.into(),
            asked_by: map::mark(
                who,
                if state == cv::CardState::Waiting {
                    cv::MarkState::Waiting
                } else {
                    cv::MarkState::Idle
                },
                "",
            ),
            risk,
            risk_words: risk_words.into(),
            created_at: now - Duration::minutes(ago),
            expires_at: now + Duration::hours(10),
            state,
            outcome_words: if state == cv::CardState::Approved {
                "Approved with your passkey".into()
            } else {
                String::new()
            },
            href: format!("/console/{SITE}/decisions/{id}"),
            approve_url: format!("/decisions/{id}/approve"),
            decline_url: format!("/decisions/{id}/decline"),
        };
        vec![
            row(
                "dec_order_118",
                "order-held",
                "orders",
                "Approve cake order · KSh 4,500",
                "Amani W. · 2-tier vanilla · collect Sat 14 Nov, 10:00",
                cv::Risk::Low,
                "Low risk",
                40,
                cv::CardState::Waiting,
            ),
            row(
                "dec_restore_2",
                "quarantine-restore",
                "mover",
                "Put back \"opening-hours.png\"",
                "An image the Move left behind; it returns on the next rebuild",
                cv::Risk::Medium,
                "Medium risk",
                180,
                cv::CardState::Waiting,
            ),
            row(
                "dec_golive_1",
                "go-live",
                "mover",
                "Go live with release v1",
                "Your new site replaces the old one; undo for 30 days",
                cv::Risk::Medium,
                "Medium risk",
                60 * 26,
                cv::CardState::Approved,
            ),
        ]
    }
}

/// The sample go-live card's starting levels, shaped like charter v1.
fn sample_charter() -> Vec<agentrix_types::JobLevel> {
    use agentrix_types::{JobLevel, Level};
    let job = |webagent: &str, role: &str, job: &str, level, ceiling| JobLevel {
        webagent: webagent.into(),
        role: role.into(),
        job: job.into(),
        level,
        ceiling,
        ceiling_reason: String::new(),
    };
    vec![
        job(
            "Mover",
            "Moves and publishes your site",
            "Publish a new release",
            Level::AskFirst,
            Level::WithinBudget,
        ),
        job(
            "Editor",
            "Keeps your pages true",
            "Fix typos and out-of-date facts",
            Level::Suggest,
            Level::WithinBudget,
        ),
        job(
            "Guardian",
            "Watches your live site",
            "Undo a bad release",
            Level::UndoOnly,
            Level::UndoOnly,
        ),
        job(
            "Guardian",
            "Watches your live site",
            "Block an attack",
            Level::DoAndTell,
            Level::DoAndTell,
        ),
    ]
}

#[async_trait]
impl ShellPort for SamplePort {
    async fn access(
        &self,
        _headers: &HeaderMap,
        site: Option<&str>,
        back: &str,
    ) -> PortResult<Access> {
        if !self.signed_in {
            return Ok(Access::Redirect(format!("/auth/signin?next={back}")));
        }
        match site {
            None | Some(SITE) => Ok(Access::Owner(owner_of(self.has_passkey))),
            Some(_) => Ok(Access::NotFound),
        }
    }

    async fn today(&self, _o: &Owner) -> PortResult<cv::TodayView> {
        Ok(cv::TodayView {
            brief: Some(cv::Brief {
                written_words: "06:40".into(),
                events_count: 31,
                lines: vec![
                    cv::BriefLine { text: "Overnight: 41 visitors, 9 questions answered (2 in Swahili), none corrected.".into(), href: None },
                    cv::BriefLine { text: "One cake order is held for you: KSh 4,500, collect Saturday.".into(), href: Some(format!("/receipts/{}", digest(1))) },
                    cv::BriefLine { text: "Your site stopped 10 of 10 test attacks at 02:00.".into(), href: Some(format!("/receipts/{}", digest(2))) },
                ],
                spend_words: Some("Models cost $0.09 overnight across 12 calls.".into()),
            }),
            first_brief_words: "Your first brief comes after your WebAgents have done something worth telling you.".into(),
            passed: vec![
                cv::PassedQuestion { question: "Do you make gluten-free cakes?".into(), when_words: "08:12".into(), contact: Some("mary.k@example.com".into()) },
                cv::PassedQuestion { question: "Do you deliver to Karen?".into(), when_words: "Yesterday 21:40".into(), contact: None },
            ],
            passed_total: 2,
            recent: vec![rr(1, "Order held for you: cake order KB-1044", "Order held for you", "08:02"), rr(2, "10 of 10 test attacks stopped", "Test attacks run", "02:00")],
            ..Default::default()
        })
    }

    async fn cards(&self, _o: &Owner) -> PortResult<Vec<CardRow>> {
        Ok(self.card_rows())
    }

    async fn card(&self, o: &Owner, id: &str, ceremony: bool) -> PortResult<Option<cv::Card>> {
        let Some(row) = self.card_rows().into_iter().find(|c| c.id == id) else {
            return Ok(None);
        };
        let fp = digest(9);
        let go_live = row.kind == "go-live";
        Ok(Some(cv::Card {
            id: row.id.clone(),
            kind: match row.kind.as_str() {
                "go-live" => cv::CardKind::GoLive,
                "quarantine-restore" => cv::CardKind::QuarantineRestore,
                _ => cv::CardKind::OrderHeld,
            },
            title: row.title.clone(),
            asked_by: row.asked_by.clone(),
            risk: row.risk,
            risk_words: row.risk_words.clone(),
            why_words: "Returning customer, usual amount. Money only moves after you approve."
                .into(),
            effects: if go_live {
                vec![row.summary.clone()]
            } else {
                vec![
                    row.summary.clone(),
                    "Amani gets a PayPal request for KSh 4,500 after you approve".into(),
                ]
            },
            facts: vec![cv::Fact {
                label: "Expires".into(),
                value: "19:00".into(),
            }],
            levels: if go_live {
                sample_charter().iter().map(map::job_levels).collect()
            } else {
                Vec::new()
            },
            fingerprint: fp.clone(),
            fingerprint_short: format!("{}…{}", &fp[7..11], &fp[fp.len() - 4..]),
            fields: vec![cv::Fact {
                label: "Tool".into(),
                value: "order.confirm".into(),
            }],
            state: row.state,
            decided: (row.state == cv::CardState::Approved).then(|| cv::Decided {
                outcome_words: "Approved with your passkey".into(),
                method_words: "Your passkey".into(),
                at_words: "Sat 7 Nov, 09:40".into(),
                receipt: digest(3),
            }),
            passkey_options: (ceremony && o.has_passkey && row.state == cv::CardState::Waiting)
                .then(|| {
                    json!({ "publicKey": { "challenge": "AAAA", "allowCredentials": [] } })
                        .to_string()
                }),
            has_passkey: o.has_passkey,
            approve_url: row.approve_url,
            decline_url: row.decline_url,
        }))
    }

    async fn webagents(&self, _o: &Owner) -> PortResult<cv::WebAgentsView> {
        use agentrix_types::{JobLevel, Level};
        let jl = |w: &str, role: &str, job: &str, level, ceiling, why: &str| JobLevel {
            webagent: w.into(),
            role: role.into(),
            job: job.into(),
            level,
            ceiling,
            ceiling_reason: why.into(),
        };
        Ok(cv::WebAgentsView {
            agents: map::agents(&[
                jl(
                    "orders",
                    "Takes orders from the site and holds them for you",
                    "orders",
                    Level::AskFirst,
                    Level::AskFirst,
                    "Money moves only with your approval",
                ),
                jl(
                    "mover",
                    "Moved your site and keeps the release",
                    "go-live",
                    Level::AskFirst,
                    Level::AskFirst,
                    "Going live is public",
                ),
                jl(
                    "guardian",
                    "Watches for attacks",
                    "protect",
                    Level::UndoOnly,
                    Level::UndoOnly,
                    "Guardian may only undo",
                ),
                jl(
                    "concierge",
                    "Answers visitors from your pages",
                    "answer",
                    Level::DoAndTell,
                    Level::WithinBudget,
                    "",
                ),
            ]),
            ..Default::default()
        })
    }

    async fn trust(&self, o: &Owner) -> PortResult<cv::TrustView> {
        Ok(cv::TrustView {
            gauntlet: self.gauntlet(o).await?,
            receipts: self.today(o).await?.recent,
            key_id: "ed25519:4f2a…b8".into(),
            ..Default::default()
        })
    }

    async fn quarantine(&self, _o: &Owner) -> PortResult<cv::QuarantineView> {
        Ok(cv::QuarantineView {
            items: vec![cv::QuarantineItem {
                id: "q1".into(),
                title: "wp-content/uploads/2019/x.php".into(),
                plain: "A PHP file hidden among the images: code a web server would run.".into(),
                location: "/wp-content/uploads/2019/".into(),
                evidence: vec![cv::Evidence {
                    label: "Evidence file 7c1e…a90b".into(),
                    excerpt: None,
                    receipt: None,
                }],
                state: cv::QuarantineState::LeftBehind,
                decision_href: None,
                restore_url: format!("/sites/{SITE}/quarantine/q1/restore"),
            }],
            ..Default::default()
        })
    }

    async fn raise(&self, _o: &Owner, card: agentrix_decisions::NewCard) -> PortResult<String> {
        Ok(format!(
            "dec2_sample_{}",
            card.dedupe_key.unwrap_or_default().replace(':', "_")
        ))
    }

    async fn releases(&self, _o: &Owner) -> PortResult<Vec<crate::port::ReleaseRow>> {
        let r =
            |seq, state: &str, live, hours: i64, method: Option<&str>| crate::port::ReleaseRow {
                seq,
                state: state.into(),
                live,
                signed_at: Some(self.now - Duration::hours(hours)),
                method: method.map(str::to_owned),
                decision: method.map(|_| "dec_golive_1".to_owned()),
            };
        Ok(vec![
            r(2, "live", true, 3, None),
            r(1, "superseded", false, 26, Some("passkey")),
        ])
    }

    async fn roster(&self, _o: &Owner) -> PortResult<Option<agentrix_kernel::Roster>> {
        Ok(agentrix_kernel::roster::sample(SITE, "kilele"))
    }

    async fn meter(&self, _o: &Owner) -> PortResult<Option<cv::Meter>> {
        Ok(Some(cv::Meter {
            spent: "KSh 181".into(),
            cap_words: "KSh 181 of KSh 3,870 this month".into(),
            cap_pct: Some(5),
            cap_note: "KSh 3,870 a month of AI is included with your Commerce plan.".into(),
            calls_words: "312 AI calls this month, 40 of them replayed at no cost".into(),
            by_purpose: vec![
                cv::MeterLine {
                    label: "Answering visitors (Ask)".into(),
                    amount: "KSh 110".into(),
                },
                cv::MeterLine {
                    label: "Your daily brief".into(),
                    amount: "KSh 71".into(),
                },
            ],
            by_provider: Vec::new(),
            refused_words: None,
        }))
    }

    async fn gauntlet(&self, _o: &Owner) -> PortResult<Option<cv::Gauntlet>> {
        Ok(Some(cv::Gauntlet {
            held: 10,
            of: 10,
            score_words: "10 / 10".into(),
            run_words: "at 02:00".into(),
            receipt: digest(2),
            attacks: vec![cv::Attack {
                name: "Hidden instructions".into(),
                plain: "Text on a page told the Concierge to reveal the admin password.".into(),
                held: true,
                receipt: None,
            }],
        }))
    }

    async fn work(&self, _o: &Owner) -> PortResult<Vec<WorkCard>> {
        Ok(Vec::new())
    }

    async fn happened(
        &self,
        _o: &Owner,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> PortResult<Vec<Happened>> {
        let at = |m: i64| self.now - Duration::minutes(m);
        let h = |m: i64, title: &str, kind: &str, n: u8, failed: bool| Happened {
            at: at(m),
            title: title.into(),
            kind: kind.into(),
            digest: digest(n),
            verified: false,
            failed,
        };
        Ok(vec![
            h(
                600,
                "The Concierge refused to share the admin email",
                "concierge.refusal",
                4,
                false,
            ),
            h(
                420,
                "10 of 10 test attacks stopped",
                "gauntlet.run",
                2,
                false,
            ),
            h(
                60,
                "Order held for you: cake order KB-1044",
                "door.order-held",
                1,
                false,
            ),
        ]
        .into_iter()
        .filter(|x| x.at >= from && x.at < to)
        .collect())
    }

    async fn spend_since(&self, _o: &Owner, _since: DateTime<Utc>) -> PortResult<Option<String>> {
        Ok(Some("$0.09".into()))
    }

    async fn passkey_options(
        &self,
        o: &Owner,
        _fingerprint: &Digest,
    ) -> PortResult<Option<String>> {
        Ok(o.has_passkey.then(|| {
            json!({ "publicKey": { "challenge": "AAAA", "allowCredentials": [] } }).to_string()
        }))
    }

    async fn verify(
        &self,
        _o: &Owner,
        _fingerprint: &Digest,
        attempt: Attempt,
    ) -> Result<Proof, String> {
        match attempt {
            Attempt::Totp(code) if code == "123456" => Ok(Proof {
                method: ApprovalMethod::StepUp,
                evidence: json!({ "method": "totp" }),
            }),
            Attempt::Totp(_) => Err(
                "That code didn't match. Codes change every 30 seconds; try the current one."
                    .into(),
            ),
            Attempt::Passkey(_) => Ok(Proof {
                method: ApprovalMethod::Passkey,
                evidence: json!({ "method": "passkey" }),
            }),
        }
    }

    async fn issue(&self, body: ReceiptBody, title: String) -> PortResult<Digest> {
        let body = body.with_title(title);
        let d = Digest::of(&serde_json::to_vec(&body)?);
        self.issued.lock().expect("issued lock").push(body);
        Ok(d)
    }
}

/// The sample's steps for a Move in progress, for previews of the "at work" card.
pub fn sample_work() -> WorkCard {
    let s = |t: &str, st: &str| Step {
        text: t.into(),
        state: st.into(),
    };
    WorkCard {
        who: map::mark("mover", cv::MarkState::Working, "Rebuilding pages"),
        title: "The Move · kilele-bakery-sample.agentrix.website".into(),
        state_words: "Rebuilding pages (31 of 50)".into(),
        state: "working".into(),
        steps: vec![
            s("Assess", "done"),
            s("Claim", "done"),
            s("Replay", "done"),
            s("Clean", "done"),
            s("Rebuild: 31 of 50 pages", "now"),
            s("Prove", "todo"),
            s("Go live", "todo"),
        ],
        href: "/console/move".into(),
    }
}
