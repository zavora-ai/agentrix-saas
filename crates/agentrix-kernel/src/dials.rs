//! Moving a dial (05 §2.3; ADR-012; 01b C12).
//!
//! - **Lowering** a job applies at once and is recorded with a receipt: the site can always
//!   lower its own autonomy.
//! - **Raising** a job is an owner decision. The change gets a fingerprint over its canonical
//!   JSON; the owner approves it with a passkey or an authenticator code bound to that
//!   fingerprint, through node's approvals (the frozen approve contract, never verified here);
//!   only then is the new level saved, as a new version of the WebAgent's spec, with a
//!   `decision.approved` receipt.
//! - A raise never goes above the job's ceiling, never touches public desks (their fixed rules)
//!   or the Guardian (undo only), and expires after a day if the owner doesn't decide.
//! - **In the console** a raise is a decision card v2 (W5's `agentrix-decisions`, ADR-014: one
//!   card on every surface): [`Dials::with_cards`] raises it, the owner approves it in the
//!   lane through the same frozen contract, and [`DialRunner`] (tool `dial.raise`) saves the new
//!   level after the card's `decision.approved` receipt. Without cards (tests, older callers)
//!   the change carries its own fingerprint and [`Dials::approve`] verifies it.

use std::sync::Arc;

use agentrix_host::{Approvals, Receipts};
use agentrix_types::{
    DecisionId, Digest, ReceiptBody, ReceiptKind, SiteId, Subject, canonical_json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::policy::{cannot_set, effective, member_of};
use crate::spec::{Level, level_words};
use crate::store::Rosters;

/// How long a raise waits for the owner.
pub const RAISE_HOURS: i64 = 24;

/// One requested change, as the owner's card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialChange {
    /// The change's id (also the decision's).
    pub change_id: String,
    /// The site.
    pub site: String,
    /// The WebAgent.
    pub webagent: String,
    /// The job id.
    pub job: String,
    /// The level now.
    pub from: Level,
    /// The level asked for.
    pub to: Level,
    /// Hex SHA-256 over the change's canonical JSON: what the passkey signs.
    pub fingerprint: String,
    /// When it lapses.
    pub expires_at: DateTime<Utc>,
    /// The card's one line: "Let the Editor fix typos and out-of-date facts: Do it and tell me".
    pub words: String,
}

/// What asking for a change did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Requested {
    /// Lowered at once.
    Applied {
        /// The level now.
        level: Level,
        /// The receipt's digest, when one was written.
        receipt: Option<String>,
    },
    /// Raised as a decision card v2; the owner approves it in the console's lane.
    Card {
        /// The change.
        change: DialChange,
        /// The card, with its fingerprint and expiry.
        card: Box<agentrix_decisions::DecisionCardV2>,
    },
    /// Waiting for the owner's approval.
    NeedsApproval {
        /// The change.
        change: DialChange,
        /// Options for `navigator.credentials.get`, or `None` to ask for an authenticator code.
        passkey: Option<serde_json::Value>,
    },
}

/// Why a change was refused, in the owner's words.
#[derive(Debug, thiserror::Error)]
pub enum DialError {
    /// Nothing to change, or nowhere to change it.
    #[error("{0}")]
    NotFound(String),
    /// The dial can't go there.
    #[error("{0}")]
    NotAllowed(String),
    /// The change isn't waiting any more.
    #[error("{0}")]
    NotWaiting(String),
    /// The approval didn't verify.
    #[error("That approval didn't go through. Nothing changed. Try again.")]
    NotApproved,
    /// Something underneath failed.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Moves dials for owners.
#[derive(Clone)]
pub struct Dials {
    rosters: Rosters,
    receipts: Arc<dyn Receipts>,
    approvals: Arc<dyn Approvals>,
    cards: Option<agentrix_decisions::Decisions>,
}

fn time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse_level(s: &str) -> anyhow::Result<Level> {
    Ok(serde_json::from_value(serde_json::Value::String(
        s.to_string(),
    ))?)
}

fn level_key(l: Level) -> String {
    serde_json::to_value(l)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The fingerprint the owner's approval binds to.
pub fn fingerprint(change: &DialChange) -> anyhow::Result<[u8; 32]> {
    let canonical = canonical_json(&json!({
        "kind": "webagent.dial",
        "change": change.change_id,
        "site": change.site,
        "webagent": change.webagent,
        "job": change.job,
        "from": change.from,
        "to": change.to,
        "expires_at": time(change.expires_at),
    }))?;
    Ok(*Digest::of(&canonical).as_bytes())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Dials {
    /// Over the kernel's rosters and node's receipts and approvals.
    pub fn new(
        rosters: Rosters,
        receipts: Arc<dyn Receipts>,
        approvals: Arc<dyn Approvals>,
    ) -> Self {
        Dials {
            rosters,
            receipts,
            approvals,
            cards: None,
        }
    }

    /// Raises go to the owner as decision cards v2, approved in the console's lane.
    pub fn with_cards(mut self, cards: agentrix_decisions::Decisions) -> Self {
        self.cards = Some(cards);
        self
    }

    /// Lowers a job's dial at once (no card: the site can always lower its own autonomy) and
    /// returns the new level and the receipt.
    pub async fn lower(
        &self,
        site: &str,
        account: Uuid,
        webagent: &str,
        job: &str,
        to: Level,
    ) -> Result<(Level, String), DialError> {
        match self.request(site, account, webagent, job, to).await? {
            Requested::Applied {
                level,
                receipt: Some(r),
            } => Ok((level, r)),
            _ => Err(DialError::NotAllowed(
                "That would raise the dial. Raising asks for your approval.".into(),
            )),
        }
    }

    /// The owner asks to set `webagent`'s `job` to `to`.
    pub async fn request(
        &self,
        site: &str,
        account: Uuid,
        webagent: &str,
        job: &str,
        to: Level,
    ) -> Result<Requested, DialError> {
        let roster = self
            .rosters
            .roster(site)
            .await?
            .ok_or_else(|| DialError::NotFound("There's no team for this site yet.".into()))?;
        if let Some(why) = cannot_set(&roster, webagent, job, to) {
            return Err(DialError::NotAllowed(why));
        }
        let who = member_of(webagent);
        let member = roster.get(who).expect("cannot_set found the member");
        let spec_job = member.spec.job(job).expect("cannot_set found the job");
        let from = effective(spec_job);
        if from == to {
            return Err(DialError::NotAllowed(format!(
                "“{}” is already at {}.",
                spec_job.name,
                level_words(to)
            )));
        }
        let now = Utc::now();
        let mut change = DialChange {
            change_id: format!("dial_{}", Uuid::now_v7().simple()),
            site: site.to_string(),
            webagent: who.to_string(),
            job: spec_job.id.clone(),
            from,
            to,
            fingerprint: String::new(),
            expires_at: now + Duration::hours(RAISE_HOURS),
            words: format!(
                "Let the {} {}: {}",
                member.spec.name,
                lower_first(&spec_job.name),
                level_words(to)
            ),
        };
        if to < from {
            let fp = fingerprint(&change)?;
            change.fingerprint = hex(&fp);
            self.insert(&change, account, now).await?;
            let receipt = self
                .apply(&change, account, "signed-in", None, None)
                .await?;
            return Ok(Requested::Applied {
                level: to,
                receipt: Some(receipt),
            });
        }
        if let Some(cards) = &self.cards {
            // One card per pending raise: asking again returns the card already waiting.
            if let Some(id) = self.waiting_raise(site, who, &change.job, to).await? {
                if let Some(row) = cards.get(site, &id).await? {
                    let change = self.waiting(site, &id).await?;
                    return Ok(Requested::Card {
                        change,
                        card: Box::new(row.card),
                    });
                }
            }
            let card = cards
                .raise(agentrix_decisions::NewCard {
                    kind: "dial.raise".into(),
                    title: change.words.clone(),
                    summary: format!(
                        "“{}” is at {} now. This sets it to {}. You can lower it again any time.",
                        spec_job.name,
                        level_words(from),
                        level_words(to)
                    ),
                    effects: vec![effect_words(&member.spec.name, &spec_job.name, to)],
                    effect: agentrix_types::EffectRequest {
                        site: SiteId::new(site),
                        webagent: who.to_string(),
                        tool: DIAL_TOOL.into(),
                        args: json!({ "job": change.job, "from": from, "to": to }),
                        amount: None,
                        expiry: change.expires_at,
                    },
                    raised_by: agentrix_decisions::RaisedBy {
                        webagent: who.to_string(),
                        job: Some(change.job.clone()),
                    },
                    risk: agentrix_decisions::Risk {
                        level: agentrix_decisions::RiskLevel::Medium,
                        reasons: vec![
                            format!("The {} would do more without asking you", member.spec.name),
                            "Lowering it again takes one tap".into(),
                        ],
                    },
                    approval: agentrix_decisions::ApprovalKind::Reviewed,
                    back: Some(format!("/console/{site}/webagents")),
                    dedupe_key: None,
                })
                .await?;
            change.change_id = card.id.clone();
            change.fingerprint = card.fingerprint.hex();
            change.expires_at = card.expires_at();
            self.insert(&change, account, now).await?;
            return Ok(Requested::Card {
                change,
                card: Box::new(card),
            });
        }
        let fp = fingerprint(&change)?;
        change.fingerprint = hex(&fp);
        self.insert(&change, account, now).await?;
        let passkey = self.approvals.passkey_options(account, fp).await?;
        Ok(Requested::NeedsApproval { change, passkey })
    }

    /// The owner approves a waiting raise with `{"passkey": …}` or `{"totp": "123456"}`.
    pub async fn approve(
        &self,
        site: &str,
        account: Uuid,
        change_id: &str,
        proof: serde_json::Value,
    ) -> Result<(Level, String), DialError> {
        let change = self.waiting(site, change_id).await?;
        let fp = fingerprint(&change)?;
        let approval = self
            .approvals
            .verify(account, fp, proof)
            .await
            .map_err(|_| DialError::NotApproved)?;
        let method = approval["method"].as_str().unwrap_or("passkey").to_string();
        let receipt = self
            .apply(&change, account, &method, Some(approval), None)
            .await?;
        Ok((change.to, receipt))
    }

    /// The owner declines a waiting raise.
    pub async fn decline(&self, site: &str, change_id: &str) -> Result<(), DialError> {
        self.waiting(site, change_id).await?;
        sqlx::query("UPDATE webagent_dial_changes SET state = 'declined', decided_at = ? WHERE change_id = ? AND state = 'waiting'")
            .bind(time(Utc::now()))
            .bind(change_id)
            .execute(self.rosters.pool())
            .await
            .map_err(anyhow::Error::from)?;
        Ok(())
    }

    /// The id of a raise to `to` already waiting for this job, if any.
    async fn waiting_raise(
        &self,
        site: &str,
        webagent: &str,
        job: &str,
        to: Level,
    ) -> anyhow::Result<Option<String>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT change_id FROM webagent_dial_changes WHERE site_id = ? AND webagent = ? AND job = ? \
             AND to_level = ? AND state = 'waiting' AND expires_at > ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(site)
        .bind(webagent)
        .bind(job)
        .bind(level_key(to))
        .bind(time(Utc::now()))
        .fetch_optional(self.rosters.pool())
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Raises still waiting on the owner for a site, newest first.
    pub async fn pending(&self, site: &str) -> anyhow::Result<Vec<DialChange>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT change_id FROM webagent_dial_changes WHERE site_id = ? AND state = 'waiting' \
             AND expires_at > ? ORDER BY created_at DESC",
        )
        .bind(site)
        .bind(time(Utc::now()))
        .fetch_all(self.rosters.pool())
        .await?;
        let mut out = Vec::new();
        for (id,) in rows {
            if let Ok(c) = self.waiting(site, &id).await {
                out.push(c);
            }
        }
        Ok(out)
    }

    async fn insert(
        &self,
        c: &DialChange,
        account: Uuid,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO webagent_dial_changes (change_id, site_id, webagent, job, from_level, to_level, \
             fingerprint, state, requested_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, 'waiting', ?, ?, ?)",
        )
        .bind(&c.change_id)
        .bind(&c.site)
        .bind(&c.webagent)
        .bind(&c.job)
        .bind(level_key(c.from))
        .bind(level_key(c.to))
        .bind(&c.fingerprint)
        .bind(account.to_string())
        .bind(time(now))
        .bind(time(c.expires_at))
        .execute(self.rosters.pool())
        .await?;
        Ok(())
    }

    /// A change that is still waiting and hasn't lapsed, rebuilt from its row.
    async fn waiting(&self, site: &str, change_id: &str) -> Result<DialChange, DialError> {
        type Row = (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT webagent, job, from_level, to_level, fingerprint, state, expires_at, created_at \
             FROM webagent_dial_changes WHERE change_id = ? AND site_id = ?",
        )
        .bind(change_id)
        .bind(site)
        .fetch_optional(self.rosters.pool())
        .await
        .map_err(anyhow::Error::from)?;
        let Some((webagent, job, from, to, fp, state, expires, _created)) = row else {
            return Err(DialError::NotFound(
                "There's no such change waiting for you.".into(),
            ));
        };
        if state != "waiting" {
            return Err(DialError::NotWaiting(format!(
                "That change was already {state}."
            )));
        }
        let expires_at: DateTime<Utc> = DateTime::parse_from_rfc3339(&expires)
            .map_err(anyhow::Error::from)?
            .with_timezone(&Utc);
        if expires_at <= Utc::now() {
            let _ = sqlx::query(
                "UPDATE webagent_dial_changes SET state = 'expired' WHERE change_id = ?",
            )
            .bind(change_id)
            .execute(self.rosters.pool())
            .await;
            return Err(DialError::NotWaiting(
                "That change lapsed. Ask again from the dial.".into(),
            ));
        }
        let roster = self.rosters.roster(site).await?;
        let words = roster
            .as_ref()
            .and_then(|r| r.get(&webagent))
            .and_then(|m| {
                m.spec
                    .job(&job)
                    .map(|j| (m.spec.name.clone(), j.name.clone()))
            })
            .map(|(n, j)| {
                format!(
                    "Let the {n} {}: {}",
                    lower_first(&j),
                    level_words(parse_level(&to).unwrap_or(Level::Watch))
                )
            })
            .unwrap_or_default();
        Ok(DialChange {
            change_id: change_id.to_string(),
            site: site.to_string(),
            webagent,
            job,
            from: parse_level(&from)?,
            to: parse_level(&to)?,
            fingerprint: fp,
            expires_at,
            words,
        })
    }

    /// Saves the new level as a spec version and records the receipt: `existing` when a
    /// decision card already wrote it, otherwise a new `decision.approved` receipt.
    async fn apply(
        &self,
        c: &DialChange,
        account: Uuid,
        method: &str,
        approval: Option<serde_json::Value>,
        existing: Option<&str>,
    ) -> anyhow::Result<String> {
        let roster = self
            .rosters
            .roster(&c.site)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no team for {}", c.site))?;
        let mut spec = roster
            .get(&c.webagent)
            .ok_or_else(|| anyhow::anyhow!("no {} on {}", c.webagent, c.site))?
            .spec
            .clone();
        let job = spec
            .jobs
            .iter_mut()
            .find(|j| j.id == c.job)
            .ok_or_else(|| anyhow::anyhow!("no job {}", c.job))?;
        job.start = c.to;
        let verb = if c.to < c.from { "Lowered" } else { "Raised" };
        let reason = format!(
            "{verb} “{}” from {} to {}",
            job.name,
            level_words(c.from),
            level_words(c.to)
        );
        self.rosters
            .save_spec(&c.site, &spec, &reason, Some(&c.change_id))
            .await?;
        let now = Utc::now();
        if let Some(receipt) = existing {
            self.mark_applied(c, method, receipt, now).await?;
            return Ok(receipt.to_string());
        }
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionApproved,
            SiteId::new(c.site.clone()),
            Subject::Decision(DecisionId::new(c.change_id.clone())),
            now,
            now,
        );
        body.detail = Some(json!({
            "card": "webagent.dial",
            "webagent": c.webagent,
            "job": c.job,
            "from": c.from,
            "to": c.to,
            "fingerprint": c.fingerprint,
            "method": method,
            "account": account.to_string(),
            "approval": approval,
        }));
        let receipt = self.receipts.issue(body).await?;
        self.mark_applied(c, method, &receipt, now).await?;
        Ok(receipt)
    }

    async fn mark_applied(
        &self,
        c: &DialChange,
        method: &str,
        receipt: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE webagent_dial_changes SET state = 'applied', method = ?, receipt = ?, decided_at = ? WHERE change_id = ?",
        )
        .bind(method)
        .bind(receipt)
        .bind(time(now))
        .bind(&c.change_id)
        .execute(self.rosters.pool())
        .await?;
        Ok(())
    }
}

/// The tool a dial raise's card runs.
pub const DIAL_TOOL: &str = "dial.raise";

/// What a level means for the WebAgent, in one line for the card's effects.
fn effect_words(name: &str, job: &str, to: Level) -> String {
    let job = lower_first(job);
    match to {
        Level::Watch => format!("The {name} reads and reports on “{job}”, and changes nothing."),
        Level::Suggest => format!("The {name} prepares “{job}” for you to do."),
        Level::AskFirst => format!("The {name} asks you before each change to “{job}”."),
        Level::DoAndTell => {
            format!("The {name} handles “{job}” and tells you after, in your brief.")
        }
        Level::WithinBudget => format!(
            "The {name} handles “{job}” within its budget and reports in the weekly review."
        ),
        Level::UndoOnly => format!("The {name} can only undo."),
    }
}

/// Runs an approved `dial.raise` card: saves the new level, under the card's receipt.
pub struct DialRunner {
    dials: Dials,
}

impl DialRunner {
    /// Over the kernel's dials.
    pub fn new(dials: Dials) -> Self {
        DialRunner { dials }
    }
}

#[async_trait::async_trait]
impl agentrix_decisions::EffectRunner for DialRunner {
    fn tool(&self) -> &str {
        DIAL_TOOL
    }

    async fn run(
        &self,
        card: &agentrix_decisions::DecisionCardV2,
        receipt: &str,
    ) -> anyhow::Result<Option<String>> {
        let site = card.site.as_str();
        let change = self
            .dials
            .waiting(site, &card.id)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        // The dial may have moved since the card was raised (lowered, or another raise).
        let roster = self
            .dials
            .rosters
            .roster(site)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no team for {site}"))?;
        let now_level = roster
            .get(&change.webagent)
            .and_then(|m| m.spec.job(&change.job))
            .map(effective);
        anyhow::ensure!(
            now_level == Some(change.from),
            "the dial moved since this card was raised; ask again from the dial"
        );
        self.dials
            .apply(&change, Uuid::nil(), "decision-card", None, Some(receipt))
            .await?;
        Ok(card.back.clone())
    }

    async fn declined(
        &self,
        card: &agentrix_decisions::DecisionCardV2,
        _receipt: &str,
    ) -> anyhow::Result<()> {
        match self.dials.decline(card.site.as_str(), &card.id).await {
            Ok(()) | Err(DialError::NotWaiting(_)) | Err(DialError::NotFound(_)) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("{e}")),
        }
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Action, Verdict, check};
    use crate::spec::Effect;

    async fn setup() -> (Dials, Rosters, agentrix_host::NodeContext) {
        let ctx = agentrix_host::testing::context().await.unwrap();
        crate::migrate(&ctx.pool).await.unwrap();
        let rosters = Rosters::new(ctx.pool.clone(), ctx.blobs.clone());
        rosters.seed_sample("kilele", "kilele").await.unwrap();
        let dials = Dials::new(rosters.clone(), ctx.receipts.clone(), ctx.approvals.clone());
        (dials, rosters, ctx)
    }

    #[tokio::test]
    async fn lowering_applies_at_once_with_a_receipt() {
        let (dials, rosters, _ctx) = setup().await;
        let owner = Uuid::new_v4();
        let out = dials
            .request(
                "kilele",
                owner,
                "coordinator",
                "morning-brief",
                Level::Watch,
            )
            .await
            .unwrap();
        assert!(matches!(
            out,
            Requested::Applied {
                level: Level::Watch,
                receipt: Some(_)
            }
        ));
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        let brief = check(
            &r,
            &Action::new("coordinator", "morning-brief", Effect::Draft),
        );
        assert_eq!(
            brief.verdict,
            Verdict::Refuse,
            "a brief set to Watch isn't written"
        );
    }

    #[tokio::test]
    async fn raising_waits_for_the_owners_approval() {
        let (dials, rosters, _ctx) = setup().await;
        let owner = Uuid::new_v4();
        let out = dials
            .request("kilele", owner, "editor", "freshness", Level::DoAndTell)
            .await
            .unwrap();
        let Requested::NeedsApproval { change, .. } = out else {
            panic!("a raise needs approval")
        };
        assert_eq!(
            change.words,
            "Let the Editor fix typos and out-of-date facts: Do it and tell me"
        );
        assert_eq!(change.fingerprint.len(), 64);
        // Nothing changed yet.
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert_eq!(
            check(&r, &Action::new("editor", "freshness", Effect::SiteEdit)).verdict,
            Verdict::Propose
        );
        assert_eq!(dials.pending("kilele").await.unwrap().len(), 1);
        // The fake approvals always verify.
        let (level, receipt) = dials
            .approve(
                "kilele",
                owner,
                &change.change_id,
                json!({"totp": "123456"}),
            )
            .await
            .unwrap();
        assert_eq!(level, Level::DoAndTell);
        assert!(receipt.starts_with("sha256:"));
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert_eq!(
            check(&r, &Action::new("editor", "freshness", Effect::SiteEdit)).verdict,
            Verdict::Act
        );
        // A change is decided once.
        assert!(matches!(
            dials
                .approve("kilele", owner, &change.change_id, json!({"totp": "1"}))
                .await,
            Err(DialError::NotWaiting(_))
        ));
        assert!(dials.pending("kilele").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn in_the_console_a_raise_is_a_card_and_its_runner_applies_it() {
        use agentrix_decisions::EffectRunner;
        let (dials, rosters, ctx) = setup().await;
        agentrix_decisions::migrate(&ctx.pool).await.unwrap();
        let dials = dials.with_cards(agentrix_decisions::Decisions::new(&ctx));
        let owner = Uuid::new_v4();
        let out = dials
            .request("kilele", owner, "editor", "freshness", Level::DoAndTell)
            .await
            .unwrap();
        let Requested::Card { change, card } = out else {
            panic!("in the console a raise is a card")
        };
        assert_eq!(
            (card.kind.as_str(), card.effect.tool.as_str()),
            ("dial.raise", DIAL_TOOL)
        );
        assert_eq!(change.change_id, card.id);
        assert_eq!(card.raised_by.job.as_deref(), Some("freshness"));
        assert!(
            card.effects[0].contains("tells you after"),
            "{}",
            card.effects[0]
        );
        // Asking again returns the card already waiting.
        let Requested::Card { card: again, .. } = dials
            .request("kilele", owner, "editor", "freshness", Level::DoAndTell)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(again.id, card.id);
        // Nothing changes until the card is approved; then the runner saves the level.
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert_eq!(
            check(&r, &Action::new("editor", "freshness", Effect::SiteEdit)).verdict,
            Verdict::Propose
        );
        let runner = DialRunner::new(dials.clone());
        let back = runner.run(&card, "sha256:00").await.unwrap();
        assert_eq!(back.as_deref(), Some("/console/kilele/webagents"));
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert_eq!(
            check(&r, &Action::new("editor", "freshness", Effect::SiteEdit)).verdict,
            Verdict::Act
        );
        assert!(
            runner.run(&card, "sha256:00").await.is_err(),
            "a card runs once"
        );
        // Lowering needs no card.
        let (level, receipt) = dials
            .lower("kilele", owner, "editor", "freshness", Level::Suggest)
            .await
            .unwrap();
        assert_eq!(level, Level::Suggest);
        assert!(receipt.starts_with("sha256:"));
        assert!(
            dials
                .lower("kilele", owner, "editor", "freshness", Level::DoAndTell)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_declined_card_leaves_the_dial_alone() {
        use agentrix_decisions::EffectRunner;
        let (dials, rosters, ctx) = setup().await;
        agentrix_decisions::migrate(&ctx.pool).await.unwrap();
        let dials = dials.with_cards(agentrix_decisions::Decisions::new(&ctx));
        let Requested::Card { card, .. } = dials
            .request(
                "kilele",
                Uuid::new_v4(),
                "coordinator",
                "playbook-learning",
                Level::AskFirst,
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        let runner = DialRunner::new(dials.clone());
        runner.declined(&card, "sha256:00").await.unwrap();
        assert!(runner.run(&card, "sha256:00").await.is_err());
        let r = rosters.roster("kilele").await.unwrap().unwrap();
        assert_eq!(
            r.get("coordinator")
                .unwrap()
                .spec
                .job("playbook-learning")
                .unwrap()
                .start,
            Level::Suggest
        );
    }

    #[tokio::test]
    async fn the_rules_hold() {
        let (dials, _rosters, _ctx) = setup().await;
        let owner = Uuid::new_v4();
        for (who, job, to) in [
            ("cashier", "payments", Level::DoAndTell),
            ("concierge", "held-orders", Level::Watch),
            ("guardian", "protect", Level::Watch),
            ("editor", "freshness", Level::Suggest),
        ] {
            assert!(
                matches!(
                    dials.request("kilele", owner, who, job, to).await,
                    Err(DialError::NotAllowed(_))
                ),
                "{who}/{job}"
            );
        }
        let out = dials
            .request("kilele", owner, "editor", "freshness", Level::WithinBudget)
            .await
            .unwrap();
        let Requested::NeedsApproval { change, .. } = out else {
            panic!()
        };
        dials.decline("kilele", &change.change_id).await.unwrap();
        assert!(matches!(
            dials.decline("kilele", &change.change_id).await,
            Err(DialError::NotWaiting(_))
        ));
    }
}
