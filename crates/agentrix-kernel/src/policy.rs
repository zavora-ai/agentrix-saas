//! Dials that bite: the check every governed action passes before it runs (05 §2.3, §3.4;
//! ADR-012).
//!
//! A WebAgent's dial for a job says how much it may do alone. [`check`] reads the dial as the
//! owner set it, the job's ceiling, and what the action would actually do, and rules:
//!
//! | Dial | Reads | Drafts | Reversible changes (site edits, undo) | Money, messages, public posts, new pages |
//! |---|---|---|---|---|
//! | Watch | act | refuse | refuse | refuse |
//! | Suggest | act | act | propose | propose |
//! | Ask me first | act | act | ask | ask |
//! | Do it and tell me, Handle it within budget | act | act | act | ask (the ceiling) |
//! | Undo only | act | act | undo only | refuse |
//!
//! - **Ask anyway:** an action marked new (a new kind of request, recipient or amount) asks at
//!   any level that would have acted, and never counts against the job.
//! - A WebAgent that isn't on duty, or a job it doesn't hold, is refused (the ladder denies any
//!   tool no job covers).
//! - The ceiling holds even if a stored level says otherwise: an irreversible action asks
//!   until a signed standing rule exists (U6), and none does in Horizon 1.
//!
//! Callers in Horizon 1: the Chief's brief (`coordinator` / `morning-brief`), the Ask desks
//! (`concierge` / `presentations`), held orders at the agent door (`agent-desk` /
//! `held-orders`), the Chief's spoken answers (`owner-questions`), and W2's sample jobs.

use serde::{Deserialize, Serialize};

use crate::roster::{Roster, Status};
use crate::spec::{DialKind, Effect, JobSpec, Level, level_words};

/// What a WebAgent is about to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Action<'a> {
    /// The WebAgent's id, or a desk's (`desk-cakes`, `agent-desk`): desks are the Concierge's.
    pub webagent: &'a str,
    /// The job id (`held-orders`) or the job's name as a charter writes it.
    pub job: &'a str,
    /// What this action does.
    pub effect: Effect,
    /// Something new for this job: a new kind of request, recipient, amount or page type.
    pub novel: bool,
}

impl<'a> Action<'a> {
    /// An action that isn't new.
    pub fn new(webagent: &'a str, job: &'a str, effect: Effect) -> Self {
        Action {
            webagent,
            job,
            effect,
            novel: false,
        }
    }
}

/// What the check allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// Do it now; it shows in the next brief.
    Act,
    /// Prepare it as a suggestion for the owner to do; don't do it.
    Propose,
    /// Raise a decision card and wait for the owner.
    Ask,
    /// Don't do it.
    Refuse,
}

/// The check's answer, with the owner's words for why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ruling {
    /// What may happen.
    pub verdict: Verdict,
    /// The member the action belongs to (desks resolve to `concierge`).
    pub webagent: String,
    /// The job's id, when the member holds it.
    pub job: Option<String>,
    /// The dial's level for that job, after the ceiling.
    pub level: Option<Level>,
    /// Why, in the owner's words. Empty when the action simply runs.
    pub reason: String,
    /// Why, for code to match on (the words may change; the code doesn't).
    pub code: RuleCode,
}

/// Why a ruling came out as it did, for callers to match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleCode {
    /// The dial allows it.
    Allowed,
    /// No team is recorded for the site yet. Reads are still allowed.
    NoTeam,
    /// There's no WebAgent with that id.
    NoSuchWebAgent,
    /// The WebAgent is in the Store, not on the team.
    NotOnTeam,
    /// The WebAgent has handed over (the Mover after go-live).
    HandedOver,
    /// The job isn't one of the WebAgent's.
    NotItsJob,
    /// The dial is at Watch: reads only.
    Watching,
    /// The dial is at Suggest: it prepares, the owner does.
    Suggesting,
    /// The dial is at Ask me first.
    AskFirst,
    /// The job's ceiling: money, messages, public posts and new pages ask.
    Ceiling,
    /// Undo only: rollback and the read-only switch.
    UndoOnly,
    /// Something new for the job, so it asks anyway.
    New,
}

/// The member an id belongs to: desks are the Concierge's.
pub fn member_of(id: &str) -> &str {
    if id == "agent-desk" || id == "inbox" || id.starts_with("desk-") {
        "concierge"
    } else {
        id
    }
}

/// The level a job runs at: its dial, never above its ceiling.
pub fn effective(job: &JobSpec) -> Level {
    if job.start <= job.ceiling {
        job.start
    } else {
        job.ceiling
    }
}

/// Rules on one action.
pub fn check(roster: &Roster, action: &Action<'_>) -> Ruling {
    let who = member_of(action.webagent);
    let refuse = |code: RuleCode, reason: String| Ruling {
        verdict: Verdict::Refuse,
        webagent: who.to_string(),
        job: None,
        level: None,
        reason,
        code,
    };
    let Some(member) = roster.get(who) else {
        return refuse(
            RuleCode::NoSuchWebAgent,
            format!("There's no WebAgent called {who} on this site."),
        );
    };
    let name = &member.spec.name;
    match member.status {
        Status::OnDuty => {}
        Status::Available => {
            return refuse(
                RuleCode::NotOnTeam,
                format!("The {name} isn't on your team. Add it from the Store to let it work."),
            );
        }
        Status::Done => {
            return refuse(RuleCode::HandedOver, format!("The {name} has handed over."));
        }
    }
    let Some(job) = member
        .spec
        .jobs
        .iter()
        .find(|j| j.id == action.job || j.name == action.job)
    else {
        return refuse(
            RuleCode::NotItsJob,
            format!("That isn't one of the {name}'s jobs."),
        );
    };
    let level = effective(job);
    let rule = |verdict: Verdict, code: RuleCode, reason: String| Ruling {
        verdict,
        webagent: who.to_string(),
        job: Some(job.id.clone()),
        level: Some(level),
        reason,
        code,
    };
    let job_words = &job.name;
    let e = action.effect;
    let irreversible = e.ceiling() <= Level::AskFirst;
    let ruling = match level {
        _ if e == Effect::Read => rule(Verdict::Act, RuleCode::Allowed, String::new()),
        Level::Watch => rule(
            Verdict::Refuse,
            RuleCode::Watching,
            format!(
                "“{job_words}” is set to Watch: the {name} reads and reports, and changes nothing."
            ),
        ),
        Level::Suggest if e == Effect::Draft => {
            rule(Verdict::Act, RuleCode::Allowed, String::new())
        }
        Level::Suggest => rule(
            Verdict::Propose,
            RuleCode::Suggesting,
            format!("“{job_words}” is set to Suggest: the {name} prepares it for you to do."),
        ),
        Level::AskFirst if e == Effect::Draft => {
            rule(Verdict::Act, RuleCode::Allowed, String::new())
        }
        // A stored level above the ceiling was capped: the ceiling is why it asks.
        Level::AskFirst if job.start > job.ceiling => rule(
            Verdict::Ask,
            RuleCode::Ceiling,
            if job.ceiling_reason.is_empty() {
                e.ceiling_reason().to_string()
            } else {
                job.ceiling_reason.clone()
            },
        ),
        Level::AskFirst => rule(
            Verdict::Ask,
            RuleCode::AskFirst,
            format!("“{job_words}” is set to Ask me first."),
        ),
        Level::DoAndTell | Level::WithinBudget if irreversible => rule(
            Verdict::Ask,
            RuleCode::Ceiling,
            e.ceiling_reason().to_string(),
        ),
        Level::DoAndTell | Level::WithinBudget => {
            rule(Verdict::Act, RuleCode::Allowed, String::new())
        }
        Level::UndoOnly if matches!(e, Effect::Undo | Effect::Draft) => {
            rule(Verdict::Act, RuleCode::Allowed, String::new())
        }
        Level::UndoOnly => rule(
            Verdict::Refuse,
            RuleCode::UndoOnly,
            format!(
                "The {name} can only undo: roll back to your last signed release or switch the site to read-only."
            ),
        ),
    };
    if action.novel && ruling.verdict == Verdict::Act && e != Effect::Read {
        return Ruling {
            verdict: Verdict::Ask,
            reason: format!("This is new for the {name}, so it asks you first."),
            code: RuleCode::New,
            ..ruling
        };
    }
    ruling
}

/// Why a dial can't move to `to`, or `None` when it can (in the owner's words).
pub fn cannot_set(roster: &Roster, webagent: &str, job: &str, to: Level) -> Option<String> {
    let who = member_of(webagent);
    let Some(member) = roster.get(who) else {
        return Some(format!("There's no WebAgent called {who} on this site."));
    };
    let name = &member.spec.name;
    match member.spec.dial {
        DialKind::FixedRules => {
            return Some("Public desks follow your fixed rules, never a dial. Change them in the Concierge's rules.".into());
        }
        DialKind::UndoOnly => return Some(format!("The {name} stays at Undo only.")),
        DialKind::Ladder => {}
    }
    let Some(j) = member.spec.jobs.iter().find(|j| j.id == job) else {
        return Some(format!("That isn't one of the {name}'s jobs."));
    };
    if to == Level::UndoOnly {
        return Some("Undo only is the Guardian's level.".into());
    }
    if to > j.ceiling {
        let why = if j.ceiling_reason.is_empty() {
            format!("“{}” can go up to {}.", j.name, level_words(j.ceiling))
        } else {
            j.ceiling_reason.clone()
        };
        return Some(why);
    }
    None
}

impl crate::store::Rosters {
    /// Rules on one action for `site`: the call node, the samples and the Chief's spoken
    /// answers make before acting. With no team recorded for the site yet (a sample before its
    /// seed, a site mid-Move), reads are allowed and everything else is refused; either way
    /// the code is [`RuleCode::NoTeam`].
    pub async fn check(&self, site: &str, action: &Action<'_>) -> anyhow::Result<Ruling> {
        Ok(match self.roster(site).await? {
            Some(r) => check(&r, action),
            None if action.effect == Effect::Read => Ruling {
                verdict: Verdict::Act,
                webagent: member_of(action.webagent).to_string(),
                job: None,
                level: None,
                reason: String::new(),
                code: RuleCode::NoTeam,
            },
            None => Ruling {
                verdict: Verdict::Refuse,
                webagent: member_of(action.webagent).to_string(),
                job: None,
                level: None,
                reason: "There's no team for this site yet.".into(),
                code: RuleCode::NoTeam,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roster::sample;

    fn kilele() -> Roster {
        sample("kilele", "kilele").unwrap()
    }

    fn set(r: &mut Roster, who: &str, job: &str, level: Level) {
        let m = r.members.iter_mut().find(|m| m.spec.id == who).unwrap();
        m.spec.jobs.iter_mut().find(|j| j.id == job).unwrap().start = level;
    }

    #[test]
    fn the_dial_decides() {
        let mut r = kilele();
        // Charter v1 starts Editor fixes at Suggest: a fix is proposed, not made.
        let fix = Action::new("editor", "freshness", Effect::SiteEdit);
        assert_eq!(check(&r, &fix).verdict, Verdict::Propose);
        set(&mut r, "editor", "freshness", Level::DoAndTell);
        assert_eq!(check(&r, &fix).verdict, Verdict::Act);
        set(&mut r, "editor", "freshness", Level::Watch);
        let ruling = check(&r, &fix);
        assert_eq!(ruling.verdict, Verdict::Refuse);
        assert!(ruling.reason.contains("set to Watch"), "{}", ruling.reason);
        // Reading is always fine.
        assert_eq!(
            check(&r, &Action::new("editor", "freshness", Effect::Read)).verdict,
            Verdict::Act
        );
    }

    #[test]
    fn the_ceiling_holds_even_against_a_stored_level() {
        let mut r = kilele();
        set(&mut r, "cashier", "payments", Level::WithinBudget);
        let pay = check(&r, &Action::new("cashier", "payments", Effect::Financial));
        assert_eq!(pay.verdict, Verdict::Ask);
        assert_eq!(pay.level, Some(Level::AskFirst));
        assert_eq!(pay.code, RuleCode::Ceiling);
        // A held order at the agent door always asks; desks resolve to the Concierge.
        let held = check(
            &r,
            &Action::new("agent-desk", "held-orders", Effect::Financial),
        );
        assert_eq!(
            (held.verdict, held.webagent.as_str()),
            (Verdict::Ask, "concierge")
        );
        // A desk answers from the site.
        assert_eq!(
            check(
                &r,
                &Action::new("desk-cakes", "presentations", Effect::Read)
            )
            .verdict,
            Verdict::Act
        );
    }

    #[test]
    fn off_duty_and_unheld_jobs_are_refused() {
        let r = kilele();
        let sales = check(&r, &Action::new("sales", "nurture", Effect::SendExternal));
        assert_eq!(sales.verdict, Verdict::Refuse);
        assert!(sales.reason.contains("Store"));
        let editor_refund = check(&r, &Action::new("editor", "refunds", Effect::Financial));
        assert_eq!(editor_refund.verdict, Verdict::Refuse);
        let guardian_publish = check(&r, &Action::new("guardian", "protect", Effect::NewPage));
        assert_eq!(guardian_publish.verdict, Verdict::Refuse);
        assert_eq!(
            check(&r, &Action::new("guardian", "protect", Effect::Undo)).verdict,
            Verdict::Act
        );
    }

    #[test]
    fn something_new_asks_anyway() {
        let mut r = kilele();
        set(&mut r, "editor", "freshness", Level::WithinBudget);
        let mut fix = Action::new("editor", "freshness", Effect::SiteEdit);
        fix.novel = true;
        assert_eq!(check(&r, &fix).verdict, Verdict::Ask);
    }

    #[tokio::test]
    async fn a_site_check_reads_the_stored_roster() {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        let rosters = crate::Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        let held = Action::new("agent-desk", "held-orders", Effect::Financial);
        let none = rosters.check("kilele", &held).await.unwrap();
        assert_eq!(
            (none.verdict, none.code),
            (Verdict::Refuse, RuleCode::NoTeam)
        );
        // A read with no team recorded still runs: reads are allowed at every level.
        let read = Action::new("coordinator", "owner-questions", Effect::Read);
        let r = rosters.check("kilele", &read).await.unwrap();
        assert_eq!((r.verdict, r.code), (Verdict::Act, RuleCode::NoTeam));
        rosters.seed_sample("kilele", "kilele").await.unwrap();
        assert_eq!(
            rosters.check("kilele", &held).await.unwrap().verdict,
            Verdict::Ask
        );
    }

    #[test]
    fn dials_move_only_within_their_rules() {
        let r = kilele();
        assert!(cannot_set(&r, "editor", "freshness", Level::WithinBudget).is_none());
        assert!(
            cannot_set(&r, "editor", "seo", Level::DoAndTell)
                .unwrap()
                .contains("public")
        );
        assert!(
            cannot_set(&r, "concierge", "held-orders", Level::Watch)
                .unwrap()
                .contains("fixed rules")
        );
        assert!(cannot_set(&r, "guardian", "protect", Level::Watch).is_some());
        assert!(cannot_set(&r, "editor", "freshness", Level::UndoOnly).is_some());
        assert!(cannot_set(&r, "coordinator", "morning-brief", Level::Watch).is_none());
    }
}
