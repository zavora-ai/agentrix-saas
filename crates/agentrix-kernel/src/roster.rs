//! The roster of nine, as data (05 §2.1; 01 §4.2 for names, mandates and starting levels).
//!
//! [`template`] builds one WebAgent's spec for a site's look. [`Roster::build`] puts a site's
//! team together: who is on duty, who is in the Store, the Concierge's desks, every mark, and
//! the Chief WebAgent's team mark. Where the first release's charter v1 already names a job ("Publish a
//! new release", "Fix typos and out-of-date facts", "Hold orders from AI customers", "Undo a
//! bad release"), the job here has the same name, level and ceiling, so the charters owners
//! signed overlay it exactly ([`Roster::apply_charter`]).

use agentrix_marks::{MarkSpec, MarkState, Render, TeamMark, TeamMember, render, render_team};
use agentrix_types::{Charter, JobLevel};
use serde::{Deserialize, Serialize};

use crate::looks::{SiteLook, role_fill};
use crate::spec::{
    Budget, Capability, DeskSpec, DialKind, Effect, JobSpec, Level, Partition,
    WEBAGENT_SPEC_VERSION, Wake, WakeKind, WebAgentSpec,
};

/// The nine, in the order the console lists them.
pub const NINE: [&str; 9] = [
    "coordinator",
    "concierge",
    "sales",
    "editor",
    "reputation",
    "cashier",
    "guardian",
    "caretaker",
    "mover",
];

fn cap(words: &str, effect: Effect) -> Capability {
    Capability {
        words: words.to_string(),
        effect,
    }
}

fn job(
    id: &str,
    name: &str,
    effect: Effect,
    start: Level,
    note: &str,
    can: Vec<Capability>,
) -> JobSpec {
    JobSpec {
        id: id.to_string(),
        name: name.to_string(),
        effect,
        start,
        ceiling: effect.ceiling(),
        ceiling_reason: effect.ceiling_reason().to_string(),
        note: note.to_string(),
        can,
    }
}

/// A job whose ceiling is its starting level: the public desks' fixed rules.
fn fixed(mut j: JobSpec, reason: &str) -> JobSpec {
    j.ceiling = j.start;
    j.ceiling_reason = reason.to_string();
    j
}

fn wake(kind: WakeKind, key: &str, words: &str) -> Wake {
    Wake {
        kind,
        key: key.to_string(),
        words: words.to_string(),
    }
}

fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

/// The glyph each role wears.
pub fn role_glyph(webagent: &str) -> &'static str {
    match webagent {
        "concierge" | "agent-desk" => "chat",
        "sales" => "bag",
        "editor" => "pen",
        "reputation" => "star",
        "cashier" => "coin",
        "guardian" => "shield",
        "caretaker" => "key",
        "mover" => "boxes",
        _ => "chat",
    }
}

/// A member's mark on this site.
pub fn mark_for(look: &SiteLook, member: &str, glyph: &str) -> MarkSpec {
    let fill = look.fill_for(member, role_fill(member));
    let glyph = if glyph == "monogram" {
        MarkSpec::monogram(&look.monogram())
    } else {
        MarkSpec::role(glyph)
    };
    MarkSpec::new(
        &look.key,
        member,
        &fill,
        look.squareness,
        glyph,
        look.family,
        look.seed_for(member),
    )
    .unwrap_or_else(|_| {
        // Only reachable with a malformed look; fall back to a mark that always validates.
        MarkSpec::new(
            "site",
            member,
            "#4a4960",
            4.6,
            MarkSpec::role("chat"),
            look.family,
            1,
        )
        .expect("the fallback mark is valid")
    })
}

/// One WebAgent's spec for a site's look, or `None` for an id that isn't one of the nine.
// One row per role reads as a table; a struct per row would only add names.
#[allow(clippy::type_complexity)]
pub fn template(id: &str, look: &SiteLook) -> Option<WebAgentSpec> {
    use Effect::*;
    use Level::*;
    let (
        name,
        role_words,
        mandate,
        never,
        partition,
        base_role,
        dial,
        jobs,
        wakes,
        share,
        connections,
    ): (
        &str,
        &str,
        &str,
        Vec<String>,
        Partition,
        &str,
        DialKind,
        Vec<JobSpec>,
        Vec<Wake>,
        u8,
        Vec<String>,
    ) = match id {
        "coordinator" => (
            "Chief WebAgent",
            "Briefs you each morning and routes your decisions",
            "Your single point of contact: briefs you, routes decisions and answers your questions about the business.",
            strings(&["Decides a card for you", "Hides what another WebAgent did"]),
            Partition::BackOffice,
            "Steward",
            DialKind::Ladder,
            vec![
                job(
                    "morning-brief",
                    "Write your morning brief",
                    Draft,
                    DoAndTell,
                    "It only messages you, and every line rests on a receipt.",
                    vec![
                        cap("Read your site's receipts and records", Read),
                        cap("Write your brief for the morning", Draft),
                    ],
                ),
                job(
                    "owner-questions",
                    "Answer your questions about the business",
                    Read,
                    DoAndTell,
                    "It answers from your records and says when it doesn't know.",
                    vec![cap("Answer from your receipts and records", Read)],
                ),
                job(
                    "weekly-review",
                    "Write Monday's review",
                    Draft,
                    DoAndTell,
                    "Counts what each WebAgent did, asked and got wrong.",
                    vec![cap("Count each WebAgent's work for the week", Draft)],
                ),
                job(
                    "playbook-learning",
                    "Suggest better ways of working",
                    Other,
                    Suggest,
                    "A playbook change runs only after its tests pass and you approve it.",
                    vec![cap("Propose a playbook change for your OK", Draft)],
                ),
            ],
            vec![
                wake(
                    WakeKind::Owner,
                    "owner",
                    "You message it from a paired device",
                ),
                wake(
                    WakeKind::Schedule,
                    "morning-brief",
                    "Every morning before 07:00",
                ),
                wake(WakeKind::Schedule, "weekly-review", "Mondays"),
                wake(
                    WakeKind::Signal,
                    "support_escalation",
                    "A visitor needs you",
                ),
            ],
            15,
            vec![],
        ),
        "concierge" => (
            "Concierge",
            "Front desk for every visitor and AI agent",
            "Front desk for every visitor and AI agent, day and night, in their language. Answers only from your site.",
            strings(&[
                "Promises a price, date or refund your site doesn't state",
                "Pretends to be a person",
                "Sees a password or payment credential",
            ]),
            Partition::Public,
            "Concierge",
            DialKind::FixedRules,
            vec![
                fixed(
                    job(
                        "presentations",
                        "Answer visitors from your pages",
                        Read,
                        DoAndTell,
                        "Every answer cites the page it came from.",
                        vec![cap("Answer from your published pages", Read)],
                    ),
                    "Public desks follow your fixed rules, never a dial.",
                ),
                fixed(
                    job(
                        "agent-front-door",
                        "Serve AI agents at the agent door",
                        Read,
                        DoAndTell,
                        "Agents get the same facts as people, through the door's tools.",
                        vec![cap("Answer agents through the agent door", Read)],
                    ),
                    "Public desks follow your fixed rules, never a dial.",
                ),
                fixed(
                    job(
                        "capture",
                        "Pass visitors' questions to you",
                        Draft,
                        DoAndTell,
                        "Questions it can't answer reach you as typed signals, never as raw text.",
                        vec![cap("File a question or a lead for you", Draft)],
                    ),
                    "Public desks follow your fixed rules, never a dial.",
                ),
                job(
                    "held-orders",
                    "Hold orders from AI customers",
                    Financial,
                    AskFirst,
                    "Orders and bookings above your amount wait for your OK.",
                    vec![cap("Hold an order or booking for your OK", Financial)],
                ),
            ],
            vec![
                wake(WakeKind::Visitor, "ask", "A visitor asks on your site"),
                wake(
                    WakeKind::Visitor,
                    "agent-door",
                    "An AI agent calls your agent door",
                ),
            ],
            40,
            vec![],
        ),
        "sales" => (
            "Sales",
            "Follows up leads, quotes and books",
            "Qualifies leads, follows up, quotes and books.",
            strings(&[
                "Contacts anyone without a consent record",
                "Promises a price or date your site doesn't state",
                "Takes or refunds money (that's the Cashier)",
                "Sees a password or payment credential",
            ]),
            Partition::BackOffice,
            "Revenue",
            DialKind::Ladder,
            vec![
                job(
                    "nurture",
                    "Follow up leads",
                    SendExternal,
                    AskFirst,
                    "Going higher needs 20 unchanged approvals in 30 days, a passing evaluation and your approval.",
                    vec![
                        cap("Read leads the Concierge captured", Read),
                        cap("Draft follow-ups and quotes", Draft),
                        cap("Email a lead after your OK", SendExternal),
                    ],
                ),
                job(
                    "handoff",
                    "Hand warm leads to you",
                    Draft,
                    DoAndTell,
                    "It only messages you, so it may act and report in your brief.",
                    vec![cap("Tell you about a warm lead", Draft)],
                ),
                job(
                    "sell",
                    "Quotes and bookings",
                    Financial,
                    AskFirst,
                    "Every quote is a decision card with the exact amount.",
                    vec![
                        cap("Book a visit or a call after your OK", ScheduleWithOthers),
                        cap("Send a quote after your OK", Financial),
                    ],
                ),
            ],
            vec![
                wake(
                    WakeKind::Signal,
                    "lead_captured",
                    "lead_captured from the front desk",
                ),
                wake(
                    WakeKind::Schedule,
                    "follow-ups",
                    "Weekdays 10:00 follow-ups",
                ),
            ],
            10,
            strings(&["email"]),
        ),
        "editor" => (
            "Editor",
            "Keeps every page true and fresh",
            "Keeps every page true and fresh, and drafts posts and images in your voice.",
            strings(&[
                "Publishes a new page without your OK",
                "Changes a price without your OK",
                "Posts on social media (that's Reputation)",
            ]),
            Partition::BackOffice,
            "Growth",
            DialKind::Ladder,
            vec![
                job(
                    "freshness",
                    "Fix typos and out-of-date facts",
                    SiteEdit,
                    Suggest,
                    "Each fix ships in a signed release you can undo.",
                    vec![
                        cap("Read every page", Read),
                        cap("Fix wording and facts on existing pages", SiteEdit),
                    ],
                ),
                job(
                    "seo",
                    "Publish new pages",
                    NewPage,
                    AskFirst,
                    "Publishing is public, so every new page asks you first.",
                    vec![
                        cap("Draft a page or post in your voice", Draft),
                        cap("Publish a page after your OK", NewPage),
                    ],
                ),
                job(
                    "images",
                    "Make images for pages and posts",
                    Draft,
                    Suggest,
                    "Images reach your site only in a release you approve.",
                    vec![cap("Make or edit an image for a page", Draft)],
                ),
                job(
                    "agent-search",
                    "Keep pages easy for AI search to read",
                    SiteEdit,
                    Suggest,
                    "Changes titles and summaries, never what a page says.",
                    vec![cap("Improve page titles and summaries", SiteEdit)],
                ),
            ],
            vec![
                wake(
                    WakeKind::Signal,
                    "content_gap",
                    "A visitor asked something your site doesn't say",
                ),
                wake(
                    WakeKind::Schedule,
                    "freshness",
                    "A weekly check of every page",
                ),
            ],
            15,
            vec![],
        ),
        "reputation" => (
            "Reputation",
            "Replies to reviews in your voice",
            "Replies to reviews and mentions in your voice.",
            strings(&["Posts in public without your OK", "Writes a review, ever"]),
            Partition::BackOffice,
            "Growth",
            DialKind::Ladder,
            vec![job(
                "social",
                "Reply to reviews",
                PublishPublic,
                AskFirst,
                "Every reply is a card you approve.",
                vec![
                    cap("Read new reviews", Read),
                    cap("Draft a reply in your voice", Draft),
                    cap("Post a reply after your OK", PublishPublic),
                ],
            )],
            vec![wake(
                WakeKind::Signal,
                "review_seen",
                "A new review appears",
            )],
            5,
            strings(&["google-business-profile"]),
        ),
        "cashier" => (
            "Cashier",
            "Takes payments and settles orders",
            "Orders, payments and refunds through your PayPal account.",
            strings(&[
                "Moves money beyond your signed limit",
                "Sees a card number or wallet credential",
            ]),
            Partition::BackOffice,
            "Revenue",
            DialKind::Ladder,
            vec![
                job(
                    "payments",
                    "Take payments and settle orders",
                    Financial,
                    AskFirst,
                    "Money stays at Ask me first unless you sign a rule with a limit.",
                    vec![cap(
                        "Ask a customer to pay through PayPal after your OK",
                        Financial,
                    )],
                ),
                job(
                    "refunds",
                    "Refund an order",
                    Financial,
                    AskFirst,
                    "Refunds under a limit you sign can run alone.",
                    vec![cap("Refund within your signed limit", Financial)],
                ),
                job(
                    "reconcile",
                    "Match payouts to orders",
                    Read,
                    DoAndTell,
                    "Reads PayPal's records; changes nothing.",
                    vec![cap("Match each payout to its orders", Read)],
                ),
            ],
            vec![
                wake(WakeKind::Event, "payment", "A payment or order event"),
                wake(WakeKind::Schedule, "reconcile", "Every night"),
            ],
            5,
            strings(&["paypal"]),
        ),
        "guardian" => (
            "Guardian",
            "Watches your live site and can roll it back",
            "Security, abuse and tamper checks, with instant rollback.",
            strings(&["Publishes anything", "Spends anything"]),
            Partition::BackOffice,
            "Guardian",
            DialKind::UndoOnly,
            vec![
                {
                    let mut j = job(
                        "protect",
                        "Undo a bad release",
                        Undo,
                        UndoOnly,
                        "If your live site breaks, it rolls back to your last signed release, then tells you.",
                        vec![
                            cap("Roll back to your last signed release", Undo),
                            cap("Switch the site to read-only", Undo),
                        ],
                    );
                    j.ceiling_reason =
                        "The Guardian can only roll back to your last signed release.".to_string();
                    j
                },
                job(
                    "alarms",
                    "Raise alarms",
                    Draft,
                    DoAndTell,
                    "It only messages you.",
                    vec![cap("Tell you when something looks wrong", Draft)],
                ),
            ],
            vec![
                wake(WakeKind::Schedule, "probes", "Probes every 5 minutes"),
                wake(
                    WakeKind::Signal,
                    "abuse_observed",
                    "abuse_observed from the front desk",
                ),
            ],
            5,
            vec![],
        ),
        "caretaker" => (
            "Caretaker",
            "Looks after domains, certificates and costs",
            "Domains, certificates, backups, updates and running costs.",
            strings(&[
                "Changes your hosting unwatched",
                "Renews anything above your signed limit",
            ]),
            Partition::BackOffice,
            "Guardian",
            DialKind::Ladder,
            vec![
                job(
                    "hosting",
                    "Watch domains, certificates and backups",
                    Read,
                    DoAndTell,
                    "Reads and reports; changes nothing.",
                    vec![cap("Check domains, certificates and backups", Read)],
                ),
                job(
                    "running-costs",
                    "Track running costs",
                    Read,
                    DoAndTell,
                    "Counts what the site costs to run each month.",
                    vec![cap("Read your bills and usage", Read)],
                ),
                job(
                    "renewals",
                    "Renew your domain",
                    Financial,
                    AskFirst,
                    "Renewals under a price you sign can run alone.",
                    vec![cap("Renew a domain after your OK", Financial)],
                ),
            ],
            vec![
                wake(WakeKind::Schedule, "daily", "A daily check"),
                wake(WakeKind::Event, "renewal", "A renewal comes due"),
            ],
            5,
            vec![],
        ),
        "mover" => (
            "Mover",
            "Moves your site in, then hands over",
            "The Move: copies, cleans and rebuilds your site in sealed rooms, proves it matches, then hands over.",
            strings(&["Goes live without your OK", "Changes your old site"]),
            Partition::BackOffice,
            "Steward",
            DialKind::Ladder,
            vec![
                job(
                    "onboarding",
                    "Copy, clean and rebuild your site",
                    Draft,
                    DoAndTell,
                    "Works in sealed rooms; nothing is public until you approve it.",
                    vec![cap("Copy and clean your old site in a sealed room", Draft)],
                ),
                {
                    let mut j = job(
                        "go-live",
                        "Publish a new release",
                        SiteEdit,
                        AskFirst,
                        "Going live is your call, on the go-live card.",
                        vec![cap("Publish a signed release after your OK", SiteEdit)],
                    );
                    j.ceiling_reason = String::new();
                    j
                },
            ],
            vec![wake(WakeKind::Owner, "move", "You start the Move")],
            0,
            vec![],
        ),
        _ => return None,
    };
    let glyph = if id == "coordinator" {
        "monogram"
    } else {
        role_glyph(id)
    };
    Some(WebAgentSpec {
        spec: WEBAGENT_SPEC_VERSION,
        id: id.to_string(),
        name: name.to_string(),
        role_words: role_words.to_string(),
        mandate: mandate.to_string(),
        never,
        partition,
        base_role: base_role.to_string(),
        dial,
        jobs,
        wakes,
        budget: Budget {
            share_percent: share,
        },
        connections,
        mark: mark_for(look, id, glyph),
        desks: Vec::new(),
    })
}

/// Whether a WebAgent works for a site now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Hired and working.
    OnDuty,
    /// In the Store: the owner can add it.
    Available,
    /// Its work is done (the Mover after go-live).
    Done,
}

impl Status {
    /// The owner's words.
    pub fn words(self) -> &'static str {
        match self {
            Status::OnDuty => "On duty",
            Status::Available => "In the Store",
            Status::Done => "Handed over",
        }
    }
}

/// A section desk to put on the Concierge: id slug, menu name, glyph, mandate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskDraft {
    /// The slug: `cakes` (the desk's id is `desk-cakes`).
    pub slug: String,
    /// The menu label.
    pub name: String,
    /// A role glyph id.
    pub glyph: String,
    /// What it answers.
    pub mandate: String,
    /// The page the menu item opens, when known.
    #[serde(default)]
    pub path: Option<String>,
}

/// One member of a site's team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    /// The spec.
    pub spec: WebAgentSpec,
    /// On duty, in the Store, or handed over.
    pub status: Status,
    /// `urn:adk:agent:{site}/{id}`.
    pub principal: String,
}

/// A site's WebAgents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Roster {
    /// The site's id.
    pub site: String,
    /// The look the marks wear.
    pub look: SiteLook,
    /// The nine, in order.
    pub members: Vec<Member>,
}

impl Roster {
    /// Builds a site's roster. `on_duty` lists the WebAgents working for it; the Mover is
    /// handed over unless listed; everyone else is in the Store. `desks` become the
    /// Concierge's section desks.
    pub fn build(site: &str, look: SiteLook, on_duty: &[&str], desks: &[DeskDraft]) -> Roster {
        let members = NINE
            .iter()
            .filter_map(|id| {
                let mut spec = template(id, &look)?;
                if *id == "concierge" {
                    spec.desks = concierge_desks(&look, desks);
                }
                let status = if on_duty.contains(id) || *id == "coordinator" {
                    Status::OnDuty
                } else if *id == "mover" {
                    Status::Done
                } else {
                    Status::Available
                };
                Some(Member {
                    principal: spec.principal(site),
                    spec,
                    status,
                })
            })
            .collect();
        Roster {
            site: site.to_string(),
            look,
            members,
        }
    }

    /// A member by id.
    pub fn get(&self, id: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.spec.id == id)
    }

    /// The members on duty.
    pub fn on_duty(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().filter(|m| m.status == Status::OnDuty)
    }

    /// The Concierge's desks.
    pub fn desks(&self) -> &[DeskSpec] {
        self.get("concierge")
            .map(|m| m.spec.desks.as_slice())
            .unwrap_or(&[])
    }

    /// The Chief WebAgent's team mark: the site's ink and monogram, one dot per member on duty
    /// (the Concierge's section desks each get their own dot, as the concept draws them).
    pub fn team(&self) -> TeamMark {
        let mut members = Vec::new();
        for m in self.on_duty().filter(|m| m.spec.id != "coordinator") {
            if m.spec.id == "concierge" {
                let sections: Vec<&DeskSpec> = m
                    .spec
                    .desks
                    .iter()
                    .filter(|d| d.id.starts_with("desk-"))
                    .collect();
                if sections.is_empty() {
                    members.push(TeamMember {
                        webagent: m.spec.id.clone(),
                        fill: m.spec.mark.color.fill.clone(),
                    });
                }
                for d in sections {
                    members.push(TeamMember {
                        webagent: d.id.clone(),
                        fill: d.mark.color.fill.clone(),
                    });
                }
            } else {
                members.push(TeamMember {
                    webagent: m.spec.id.clone(),
                    fill: m.spec.mark.color.fill.clone(),
                });
            }
        }
        TeamMark {
            site: self.look.key.clone(),
            ink: self.look.ink.clone(),
            squareness: self.look.squareness,
            letters: self.look.monogram(),
            members,
        }
    }

    /// The team mark as SVG. `busy` lists members working or answering now.
    pub fn team_svg(&self, size: f64, state: MarkState, busy: &[&str]) -> String {
        render_team(&self.team(), size, state, busy, &self.look.font)
    }

    /// A member's or desk's mark as SVG, by WebAgent id or desk id.
    pub fn mark_svg(&self, id: &str, size: f64, state: MarkState, uid: &str) -> Option<String> {
        let spec = self.get(id).map(|m| &m.spec.mark).or_else(|| {
            self.desks()
                .iter()
                .find(|d| d.id == id || d.mark.webagent == id)
                .map(|d| &d.mark)
        })?;
        if id == "coordinator" {
            return Some(self.team_svg(size, state, &[]));
        }
        Some(render(
            spec,
            &Render {
                size,
                state,
                font: &self.look.font,
                uid,
            },
        ))
    }

    /// Lays a signed charter's levels over the specs' starting levels. A job matches by its
    /// WebAgent's name and the job's name, as the first release's charter v1 writes them.
    pub fn apply_charter(&mut self, charter: &Charter) {
        for jl in &charter.jobs {
            if let Some(job) = self.find_job_mut(jl) {
                job.start = jl.level;
                if jl.ceiling < job.ceiling {
                    job.ceiling = jl.ceiling;
                    job.ceiling_reason = jl.ceiling_reason.clone();
                }
            }
        }
    }

    fn find_job_mut(&mut self, jl: &JobLevel) -> Option<&mut JobSpec> {
        let member = self
            .members
            .iter_mut()
            .find(|m| m.spec.name.eq_ignore_ascii_case(&jl.webagent) || m.spec.id == jl.webagent)?;
        member
            .spec
            .jobs
            .iter_mut()
            .find(|j| j.name == jl.job || j.id == jl.job)
    }
}

/// The front desk, the agent door's desk, and a section desk per draft.
fn concierge_desks(look: &SiteLook, drafts: &[DeskDraft]) -> Vec<DeskSpec> {
    let mut desks = vec![
        DeskSpec {
            id: "concierge".to_string(),
            name: "Front desk".to_string(),
            mandate: "Answers visitors from the site's own pages.".to_string(),
            path: None,
            mark: mark_for(look, "concierge", "chat"),
        },
        DeskSpec {
            id: "agent-desk".to_string(),
            name: "Agent door".to_string(),
            mandate: "Serves AI agents through the agent door, with the same facts as people get."
                .to_string(),
            path: Some("/agents".to_string()),
            mark: mark_for(look, "agent-desk", "chat"),
        },
    ];
    for d in drafts {
        desks.push(DeskSpec {
            id: format!("desk-{}", d.slug),
            name: d.name.clone(),
            mandate: d.mandate.clone(),
            path: d.path.clone(),
            mark: mark_for(look, &d.slug, &d.glyph),
        });
    }
    desks
}

fn desk(slug: &str, name: &str, glyph: &str, mandate: &str) -> DeskDraft {
    DeskDraft {
        slug: slug.to_string(),
        name: name.to_string(),
        glyph: glyph.to_string(),
        mandate: mandate.to_string(),
        path: Some(sample_path(slug)),
    }
}

/// The menu item each sample desk opens (W2's sample sites).
fn sample_path(slug: &str) -> String {
    match slug {
        "gifts" => "/gifts/plan".to_string(),
        "newsroom" => "/".to_string(),
        "subscriptions" => "/subscribe".to_string(),
        other => format!("/{other}"),
    }
}

/// A sample's section desks (01a §8, D10) and who is on duty (01a §1.4 to §1.6).
pub fn sample_team(slug: &str) -> Option<(Vec<&'static str>, Vec<DeskDraft>)> {
    Some(match slug {
        "kilele" => (
            vec!["concierge", "editor", "guardian", "cashier", "caretaker"],
            vec![
                desk(
                    "bread",
                    "Bread",
                    "loaf",
                    "Says what is fresh today and when the oven opens.",
                ),
                desk(
                    "cakes",
                    "Cakes",
                    "cake",
                    "Answers about cakes, prices and collection times from the site's own pages.",
                ),
                desk(
                    "orders",
                    "Orders",
                    "bag",
                    "Takes and changes orders, and asks Wanjiru above her amount.",
                ),
                desk(
                    "visit",
                    "Visit",
                    "pin",
                    "Says when the shop is open and how to find it.",
                ),
            ],
        ),
        "classy" => (
            vec![
                "concierge",
                "sales",
                "editor",
                "reputation",
                "cashier",
                "guardian",
            ],
            vec![
                desk(
                    "fragrances",
                    "Fragrances",
                    "perfume",
                    "Explains notes, sizes and what is in stock.",
                ),
                desk(
                    "gifts",
                    "Gifts",
                    "gift",
                    "Helps visitors choose a gift and have it wrapped.",
                ),
                desk(
                    "orders",
                    "Orders",
                    "bag",
                    "Takes orders and delivery details.",
                ),
                desk(
                    "journal",
                    "Journal",
                    "book",
                    "Answers from the shop's journal posts only.",
                ),
            ],
        ),
        "tana" => (
            vec!["concierge", "editor", "guardian", "cashier"],
            vec![
                desk(
                    "newsroom",
                    "Newsroom",
                    "news",
                    "Answers readers from published stories, with links.",
                ),
                desk(
                    "archive",
                    "Archive",
                    "calendar",
                    "Finds past issues by date and topic.",
                ),
                desk(
                    "subscriptions",
                    "Subscriptions",
                    "coin",
                    "Handles sign-ups and renewals.",
                ),
            ],
        ),
        _ => return None,
    })
}

/// agentrix.website's own team (site [`crate::home::SITE`]): four front desks on the home page,
/// and the Guardian, the Editor and Sales in the back office with the Chief.
pub fn home_team() -> (Vec<&'static str>, Vec<DeskDraft>) {
    let desk = |slug: &str, name: &str, glyph: &str, mandate: &str, path: &str| DeskDraft {
        slug: slug.to_string(),
        name: name.to_string(),
        glyph: glyph.to_string(),
        mandate: mandate.to_string(),
        path: Some(path.to_string()),
    };
    (
        vec!["concierge", "guardian", "editor", "sales"],
        vec![
            desk(
                "your-site",
                "Your site",
                "search",
                "Checks the site you paste with Assess and shows what the Move would change.",
                "/assess",
            ),
            desk(
                "how-it-works",
                "How it works",
                "boxes",
                "Explains the Move's seven steps, the sealed rooms and going live on your passkey.",
                "/#move",
            ),
            desk(
                "samples",
                "Samples",
                "star",
                "Shows the three sample sites and what their WebAgents do.",
                "/#samples",
            ),
            desk(
                "prices",
                "Prices",
                "coin",
                "Answers about plans, prices and the AI each plan includes.",
                "/#pricing",
            ),
        ],
    )
}

/// A sample site's roster, its marks the concept's.
pub fn sample(site: &str, slug: &str) -> Option<Roster> {
    let look = crate::looks::sample(slug)?;
    let (on_duty, desks) = sample_team(slug)?;
    Some(Roster::build(site, look, &on_duty, &desks))
}

/// Who is on duty on a moved the first release site: the Chief and the four its charter v1 names.
pub const MOVED_SITE_ON_DUTY: [&str; 3] = ["concierge", "editor", "guardian"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looks;

    #[test]
    fn nine_templates_keep_the_rules() {
        let look = looks::sample("kilele").unwrap();
        for id in NINE {
            let s = template(id, &look).unwrap();
            s.mark.validate().unwrap();
            assert!(!s.jobs.is_empty(), "{id}");
            assert!(!s.never.is_empty(), "{id}");
            for j in &s.jobs {
                assert!(
                    j.start <= j.ceiling || s.dial == DialKind::UndoOnly,
                    "{id}/{}",
                    j.id
                );
                assert!(
                    j.ceiling <= j.effect.ceiling(),
                    "{id}/{}: ceiling above its effect's",
                    j.id
                );
                if matches!(
                    j.effect,
                    Effect::Financial
                        | Effect::SendExternal
                        | Effect::PublishPublic
                        | Effect::NewPage
                ) {
                    assert!(j.ceiling <= Level::AskFirst, "{id}/{}", j.id);
                }
            }
            assert_eq!(
                s.principal("site_k1"),
                format!("urn:adk:agent:site_k1/{id}")
            );
        }
        assert!(template("growth", &look).is_none());
        assert_eq!(
            template("editor", &look)
                .unwrap()
                .job("images")
                .unwrap()
                .start,
            Level::Suggest
        );
    }

    #[test]
    fn samples_wear_the_concepts_marks() {
        let r = sample("site_kilele", "kilele").unwrap();
        let cakes = r.desks().iter().find(|d| d.id == "desk-cakes").unwrap();
        assert_eq!(cakes.mark.seed_key(), "kilele:cakes:7");
        assert_eq!(cakes.mark.color.fill, "#8e3b46");
        let team = r.team();
        assert_eq!(team.letters, "K");
        assert!(team.members.iter().any(|m| m.webagent == "desk-cakes"));
        assert!(
            !team.members.iter().any(|m| m.webagent == "sales"),
            "Sales is in Kilele's Store"
        );
        assert_eq!(r.get("mover").unwrap().status, Status::Done);
        assert!(
            r.mark_svg("cakes", 40.0, MarkState::Idle, "1")
                .unwrap()
                .starts_with("<svg class=\"wa\"")
        );
        assert!(
            r.mark_svg("coordinator", 44.0, MarkState::Waiting, "1")
                .unwrap()
                .contains("wa chief")
        );
        for slug in looks::SAMPLES {
            let r = sample(&format!("site_{slug}"), slug).unwrap();
            for d in r.desks() {
                assert!(
                    r.look.bg.len() == 7 && d.mark.passes_on(&r.look.bg),
                    "{slug}/{}",
                    d.id
                );
            }
        }
    }

    #[test]
    fn first_release_charter_overlays_exactly() {
        let mut r = Roster::build(
            "site_k1",
            looks::sample("kilele").unwrap(),
            &MOVED_SITE_ON_DUTY,
            &[],
        );
        let charter = Charter {
            jobs: vec![
                JobLevel {
                    webagent: "Editor".into(),
                    role: String::new(),
                    job: "Fix typos and out-of-date facts".into(),
                    level: Level::DoAndTell,
                    ceiling: Level::WithinBudget,
                    ceiling_reason: String::new(),
                },
                JobLevel {
                    webagent: "Concierge".into(),
                    role: String::new(),
                    job: "Hold orders from AI customers".into(),
                    level: Level::AskFirst,
                    ceiling: Level::AskFirst,
                    ceiling_reason:
                        "Orders involve money and other people, so it always asks you first.".into(),
                },
                JobLevel {
                    webagent: "Guardian".into(),
                    role: String::new(),
                    job: "Undo a bad release".into(),
                    level: Level::UndoOnly,
                    ceiling: Level::UndoOnly,
                    ceiling_reason: String::new(),
                },
                JobLevel {
                    webagent: "Mover".into(),
                    role: String::new(),
                    job: "Publish a new release".into(),
                    level: Level::AskFirst,
                    ceiling: Level::WithinBudget,
                    ceiling_reason: String::new(),
                },
            ],
        };
        r.apply_charter(&charter);
        assert_eq!(
            r.get("editor")
                .unwrap()
                .spec
                .job("freshness")
                .unwrap()
                .start,
            Level::DoAndTell
        );
        assert_eq!(
            r.get("concierge")
                .unwrap()
                .spec
                .job("held-orders")
                .unwrap()
                .ceiling,
            Level::AskFirst
        );
        assert_eq!(
            r.get("mover").unwrap().spec.job("go-live").unwrap().start,
            Level::AskFirst
        );
    }
}
