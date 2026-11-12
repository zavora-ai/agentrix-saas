//! Decision card v2 (contract C4, 02 §6.1): one card per decision on every surface.
//!
//! A WebAgent (or a workstream acting for one) raises a [`NewCard`] over one [`EffectRequest`];
//! the card carries who raised it, the risk with its reasons, when it expires and how it may be
//! approved. The owner approves it in the console's lane through the frozen approve contract
//! (design §4.1): a passkey assertion over the card's fingerprint, or a single-use authenticator
//! code bound to it, verified by node's [`Approvals`]. Approving writes a `decision.approved`
//! receipt, then runs the effect through the [`EffectRunner`] registered for the effect's tool.
//! Declining writes `decision.declined`. An unanswered card waits, and past its expiry it is
//! recorded as expired, never as declined and never as approved.
//!
//! The first release's cards (go-live, restore, held order) keep their own table and routes; this crate
//! never changes a signed type in place (02 §5.3): v1's `DecisionCard` stays as it is.
//!
//! Routes ([`router`]): `POST /decisions/v2/{id}/approve` (JSON `{method: "passkey",
//! credential}` or `{method: "totp", code, fingerprint}`, or a form with `method=totp`, each
//! with the owner's `edits` when they changed the card), `POST /decisions/v2/{id}/decline`
//! (with an optional `reason` and `rule`), `POST /decisions/v2/{id}/sheet` and
//! `POST /decisions/v2/rules/{id}/remove` and `POST /decisions/v2/grants/{id}/revoke`. Replies are the contract's: 303 to a form, `{redirect}` or
//! `{message}` to JSON.
//!
//! The owner can also answer more than yes or no (04-ui-parity §3 item 7): change the fields a
//! raiser marked editable and approve the edited version ([`edit`]), decline with a reason that
//! can become a standing rule for the WebAgent, or approve and allow the same again for this
//! person or kind ([`standing`]). An edit or a grant changes what the approval signs, so the
//! sheet for it comes from `POST /decisions/v2/{id}/sheet` and the approval carries the same
//! answer.
//!
//! Spoken and chat answers ([`Decisions::decide_spoken`]) follow [`surface`]'s one rule table:
//! they may allow once only an everyday card (low risk, [`ApprovalKind::OneTap`], no money, not
//! publishing) and decline any card; email decides nothing. They never count as a
//! reviewed approval (07 §5.3). No agent approves its own card: an approval needs a signed-in
//! owner.

pub mod edit;
pub mod mandate;
pub mod standing;
pub mod surface;
pub mod taught;

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;

use agentrix_host::{Approvals, NodeContext, OwnerSessions, Receipts};
use agentrix_types::{
    DecisionId, Digest, EffectRequest, ReceiptBody, ReceiptKind, SiteId, Subject,
};
use anyhow::Context;
use async_trait::async_trait;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use chrono::{DateTime, SecondsFormat, Utc};
pub use edit::{EditKind, EditableField, OwnerEdit};
use futures::future::BoxFuture;
pub use mandate::{MANDATE_TOOL, MandateRunner, MandateTerms, SignedMandate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::SqlitePool;
pub use standing::{Grant, GrantScope, GrantTerms, Rule};

/// Runs this crate's migrations (0550) with their own table, before node mounts anything.
pub fn migrate(pool: &SqlitePool) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut m = sqlx::migrate!("./migrations");
        m.set_ignore_missing(true);
        m.dangerous_set_table_name("_sqlx_migrations_agentrix_decisions");
        m.run(pool).await.context("agentrix-decisions migrations")?;
        Ok(())
    })
}

/// How risky a card is, with the reasons in the owner's words. Never a bare score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Everyday: a returning customer, a usual amount, an undoable change.
    Low,
    /// Worth a look.
    Medium,
    /// Money, people outside the business, or something hard to undo.
    High,
}

/// The risk on a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Risk {
    /// The level.
    pub level: RiskLevel,
    /// Why, in the owner's words: "Returning customer", "Usual amount".
    pub reasons: Vec<String>,
}

/// How a card may be approved (07 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalKind {
    /// Only on a screen, with the card open: a passkey, or an authenticator code without one.
    Reviewed,
    /// Also "allow once" by voice or chat, when the risk is low. Never counts toward promotion.
    OneTap,
}

/// Who raised a card. Agents raise cards; only people approve them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaisedBy {
    /// The WebAgent's id in the roster: `cashier`, `editor`, `concierge`.
    pub webagent: String,
    /// The job, when one: `payments`, `freshness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
}

/// A decision card, version 2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionCardV2 {
    /// Always 2.
    pub v: u8,
    /// `dec2_…`.
    pub id: String,
    /// The site.
    pub site: SiteId,
    /// What kind of decision: `dial.raise`, `webagent.add`, `release.publish`, `order.confirm`.
    pub kind: String,
    /// Plain words: what will happen. "Approve cake order · KSh 4,500".
    pub title: String,
    /// One line for the lane.
    pub summary: String,
    /// "If you approve, exactly this happens", line by line, from the effect's arguments.
    pub effects: Vec<String>,
    /// The exact effect an approval allows.
    pub effect: EffectRequest,
    /// `effect.fingerprint()`: what the approval signs.
    pub fingerprint: Digest,
    /// Who raised it.
    pub raised_by: RaisedBy,
    /// How risky, and why.
    pub risk: Risk,
    /// How it may be approved.
    pub approval: ApprovalKind,
    /// When it was raised.
    pub created_at: DateTime<Utc>,
    /// Where to send the owner after they decide, if not back to where they were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<String>,
}

impl DecisionCardV2 {
    /// After this the card can't be approved (the effect's expiry).
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.effect.expiry
    }
}

/// What a raiser hands [`Decisions::raise`].
#[derive(Debug, Clone)]
pub struct NewCard {
    /// The kind (see [`DecisionCardV2::kind`]).
    pub kind: String,
    /// Plain words: what will happen.
    pub title: String,
    /// One line for the lane.
    pub summary: String,
    /// "If you approve, exactly this happens".
    pub effects: Vec<String>,
    /// The exact effect. Its site is the card's site.
    pub effect: EffectRequest,
    /// Who raised it.
    pub raised_by: RaisedBy,
    /// How risky, and why.
    pub risk: Risk,
    /// How it may be approved.
    pub approval: ApprovalKind,
    /// Where to go after deciding.
    pub back: Option<String>,
    /// Raising twice with the same key returns the first card (an order held twice is one card).
    pub dedupe_key: Option<String>,
}

/// Where a card is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardState {
    /// Waiting for the owner.
    Waiting,
    /// Approved; the effect ran (or was handed to its runner).
    Approved,
    /// Declined; nothing changed.
    Declined,
    /// Expired unanswered; nothing was done.
    Expired,
}

impl CardState {
    fn as_str(self) -> &'static str {
        match self {
            CardState::Waiting => "waiting",
            CardState::Approved => "approved",
            CardState::Declined => "declined",
            CardState::Expired => "expired",
        }
    }
    fn parse(s: &str) -> anyhow::Result<Self> {
        Ok(match s {
            "waiting" => CardState::Waiting,
            "approved" => CardState::Approved,
            "declined" => CardState::Declined,
            "expired" => CardState::Expired,
            other => anyhow::bail!("unknown card state {other}"),
        })
    }
}

/// A card and what happened to it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardRow {
    /// The card.
    pub card: DecisionCardV2,
    /// Its state; a waiting card past its expiry reads as expired.
    pub state: CardState,
    /// `passkey`, `totp`, `spoken` or `chat`, once decided.
    pub method: Option<String>,
    /// When it was decided.
    pub decided_at: Option<DateTime<Utc>>,
    /// The decision's receipt.
    pub receipt: Option<String>,
    /// The fields the raiser lets the owner change before approving.
    pub editable: Vec<EditableField>,
    /// The owner's edit, when they approved an edited version.
    pub edit: Option<OwnerEdit>,
    /// The owner's reason for declining, in their words.
    pub decline_reason: Option<String>,
    /// The standing grant this card made ("Allow, and tell me") or was approved under.
    pub grant_id: Option<String>,
}

/// Runs an approved card's effect. The crate that raises cards for a tool registers one.
#[async_trait]
pub trait EffectRunner: Send + Sync {
    /// The tool it runs, e.g. `order.confirm`.
    fn tool(&self) -> &str;
    /// Runs the effect once, after the approval's receipt is written. Returns where to send the
    /// owner, or `None` to go back where they were. An error leaves the approval recorded and
    /// tells the owner the effect didn't finish.
    async fn run(&self, card: &DecisionCardV2, receipt: &str) -> anyhow::Result<Option<String>>;
    /// Called after a decline, for the raiser to tidy up (release a held order). Optional.
    async fn declined(&self, _card: &DecisionCardV2, _receipt: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// How the owner answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Attempt {
    /// A webauthn-rs `PublicKeyCredential`, as JSON.
    Passkey(Value),
    /// Six digits from the authenticator app.
    Totp(String),
}

/// What the owner answers besides yes: the fields they changed (field key to the value they
/// typed). Empty means the card as raised.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    /// The owner's edits.
    pub edits: BTreeMap<String, String>,
    /// "Allow, and tell me": approve, and allow the same again for this person or kind.
    pub grant: Option<GrantScope>,
}

/// What the owner says when declining.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Teach {
    /// Why, in their words. Optional.
    pub reason: Option<String>,
    /// Keep the reason as a standing rule for the WebAgent and job.
    pub rule: bool,
}

/// What an approval signs and what then runs.
struct Signed {
    fingerprint: Digest,
    effect: EffectRequest,
    edit: Option<OwnerEdit>,
    grant: Option<GrantTerms>,
}

/// "The Cashier" from "the Cashier": a name at the start of a sentence.
fn sentence_start(words: &str) -> String {
    let mut c = words.chars();
    match c.next() {
        Some(f) => format!("{}{}", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

/// "the Cashier" for `cashier`, "Sales" for `sales`: how cards and receipts name a WebAgent
/// (concept/console.html: "Tell the Cashier why", "Tell Reputation why").
pub fn agent_words(webagent: &str) -> String {
    let mut c = webagent.chars();
    let Some(f) = c.next() else {
        return "the WebAgent".into();
    };
    let name = format!(
        "{}{}",
        f.to_uppercase(),
        c.as_str().replace(['-', '_'], " ")
    );
    if matches!(webagent, "sales" | "reputation" | "guardian") {
        name
    } else {
        format!("the {name}")
    }
}

/// Why a decision was refused, with plain words for the owner and a status.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    /// The HTTP status.
    pub status: StatusCode,
    /// What to tell the owner.
    pub words: String,
}

fn refuse(status: StatusCode, words: impl Into<String>) -> Refusal {
    Refusal {
        status,
        words: words.into(),
    }
}

fn internal(e: anyhow::Error) -> Refusal {
    tracing::error!(error = %format!("{e:#}"), "decision failed");
    refuse(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Something went wrong on our side. Nothing was changed; try again.",
    )
}

fn stored(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_time(s: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

type RowSql = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The columns [`RowSql`] reads, in order.
macro_rules! select_cards {
    ($rest:literal) => {
        concat!(
            "SELECT card, fingerprint, state, method, decided_at, receipt, editable, owner_edit, decline_reason, grant_id FROM decision_cards_v2 ",
            $rest
        )
    };
}

/// A database whose migrations haven't run yet has no v2 cards rather than an error.
fn missing_table(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.message().contains("no such table"))
}

/// Raises, lists and decides v2 cards.
#[derive(Clone)]
pub struct Decisions {
    pool: SqlitePool,
    receipts: Arc<dyn Receipts>,
    approvals: Arc<dyn Approvals>,
    owners: Arc<dyn OwnerSessions>,
    runners: Arc<Vec<Arc<dyn EffectRunner>>>,
}

impl Decisions {
    /// Over node's database, receipts, approvals and sessions.
    pub fn new(ctx: &NodeContext) -> Self {
        Self {
            pool: ctx.pool.clone(),
            receipts: ctx.receipts.clone(),
            approvals: ctx.approvals.clone(),
            owners: ctx.owners.clone(),
            runners: Arc::new(Vec::new()),
        }
    }

    /// Adds runners for the tools other crates raise cards for.
    pub fn with_runners(mut self, runners: Vec<Arc<dyn EffectRunner>>) -> Self {
        self.runners = Arc::new(runners);
        self
    }

    /// Raises a card. With a `dedupe_key` already used on the site, returns that card instead.
    pub async fn raise(&self, new: NewCard) -> anyhow::Result<DecisionCardV2> {
        let site = new.effect.site.clone();
        if let Some(key) = &new.dedupe_key {
            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT card FROM decision_cards_v2 WHERE site_id = ? AND dedupe_key = ?",
            )
            .bind(site.as_str())
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
            if let Some((card,)) = existing {
                return Ok(serde_json::from_str(&card)?);
            }
        }
        let now = Utc::now();
        let card = DecisionCardV2 {
            v: 2,
            id: format!("dec2_{}", uuid::Uuid::now_v7().simple()),
            site: site.clone(),
            kind: new.kind,
            title: new.title,
            summary: new.summary,
            effects: new.effects,
            fingerprint: new.effect.fingerprint()?,
            effect: new.effect,
            raised_by: new.raised_by,
            risk: new.risk,
            approval: new.approval,
            created_at: now,
            back: new.back,
        };
        sqlx::query(
            "INSERT INTO decision_cards_v2 (id, site_id, kind, card, fingerprint, dedupe_key, state, created_at) VALUES (?, ?, ?, ?, ?, ?, 'waiting', ?)",
        )
        .bind(&card.id)
        .bind(site.as_str())
        .bind(&card.kind)
        .bind(serde_json::to_string(&card)?)
        .bind(card.fingerprint.to_string())
        .bind(new.dedupe_key)
        .bind(stored(now))
        .execute(&self.pool)
        .await?;
        if !self.runners.is_empty()
            && let Err(e) = self.apply_grants(&card.id).await
        {
            tracing::warn!(error = %format!("{e:#}"), card = %card.id, "checking standing grants failed; the card waits for the owner");
        }
        Ok(card)
    }

    fn row(sql: RowSql, now: DateTime<Utc>) -> anyhow::Result<CardRow> {
        let (card, fingerprint, state, method, decided_at, receipt, editable, edit, reason, grant) =
            sql;
        let card: DecisionCardV2 =
            serde_json::from_str(&card).context("a stored v2 card does not parse")?;
        anyhow::ensure!(
            Digest::from_str(&fingerprint)? == card.fingerprint,
            "card {}: stored fingerprint differs",
            card.id
        );
        let mut state = CardState::parse(&state)?;
        if state == CardState::Waiting && card.effect.is_expired(now) {
            state = CardState::Expired;
        }
        Ok(CardRow {
            card,
            state,
            method,
            decided_at: decided_at.as_deref().map(parse_time).transpose()?,
            receipt,
            editable: editable
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .context("a card's editable fields do not parse")?
                .unwrap_or_default(),
            edit: edit
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .context("a card's owner edit does not parse")?,
            decline_reason: reason,
            grant_id: grant,
        })
    }

    /// The site's cards, newest first.
    pub async fn list(&self, site: &str) -> anyhow::Result<Vec<CardRow>> {
        let now = Utc::now();
        let rows: Vec<RowSql> = match sqlx::query_as(select_cards!(
            "WHERE site_id = ? ORDER BY created_at DESC, rowid DESC"
        ))
        .bind(site)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) if missing_table(&e) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        rows.into_iter().map(|r| Self::row(r, now)).collect()
    }

    /// One card of the site.
    pub async fn get(&self, site: &str, id: &str) -> anyhow::Result<Option<CardRow>> {
        let row: Option<RowSql> =
            match sqlx::query_as(select_cards!("WHERE site_id = ? AND id = ?"))
                .bind(site)
                .bind(id)
                .fetch_optional(&self.pool)
                .await
            {
                Ok(row) => row,
                Err(e) if missing_table(&e) => None,
                Err(e) => return Err(e.into()),
            };
        row.map(|r| Self::row(r, Utc::now())).transpose()
    }

    async fn card_any_site(&self, id: &str) -> Result<CardRow, Refusal> {
        let row: Option<RowSql> = sqlx::query_as(select_cards!("WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| internal(e.into()))?;
        let row =
            row.ok_or_else(|| refuse(StatusCode::NOT_FOUND, "We couldn't find that card."))?;
        Self::row(row, Utc::now()).map_err(internal)
    }

    /// Whether `account` owns the card's site. Agents never hold an owner session.
    async fn owner_of(&self, site: &SiteId, account: &str) -> Result<(), Refusal> {
        let owner: Option<(Option<String>,)> =
            sqlx::query_as("SELECT owner_account_id FROM sites WHERE site_id = ?")
                .bind(site.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| internal(e.into()))?;
        match owner {
            Some((Some(o),)) if o == account => Ok(()),
            Some(_) => Err(refuse(
                StatusCode::FORBIDDEN,
                "Only the site's owner can decide this.",
            )),
            None => Err(refuse(StatusCode::NOT_FOUND, "We couldn't find that site.")),
        }
    }

    async fn record(
        &self,
        row: &CardRow,
        state: CardState,
        method: &str,
        account: &str,
        receipt: &str,
        now: DateTime<Utc>,
    ) -> Result<(), Refusal> {
        self.record_with(row, state, method, account, receipt, now, None, None)
            .await
    }

    /// Records the decision, with the owner's edit or decline reason when there is one.
    #[allow(clippy::too_many_arguments)]
    async fn record_with(
        &self,
        row: &CardRow,
        state: CardState,
        method: &str,
        account: &str,
        receipt: &str,
        now: DateTime<Utc>,
        edit: Option<&OwnerEdit>,
        reason: Option<&str>,
    ) -> Result<(), Refusal> {
        let edit = edit
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| internal(e.into()))?;
        let done = sqlx::query(
            "UPDATE decision_cards_v2 SET state = ?, method = ?, decided_by = ?, decided_at = ?, receipt = ?, owner_edit = ?, decline_reason = ? WHERE id = ? AND state = 'waiting'",
        )
        .bind(state.as_str())
        .bind(method)
        .bind(account)
        .bind(stored(now))
        .bind(receipt)
        .bind(edit)
        .bind(reason)
        .bind(&row.card.id)
        .execute(&self.pool)
        .await
        .map_err(|e| internal(e.into()))?;
        if done.rows_affected() == 1 {
            Ok(())
        } else {
            Err(refuse(
                StatusCode::CONFLICT,
                "This card was already decided.",
            ))
        }
    }

    fn check_waiting(row: &CardRow) -> Result<(), Refusal> {
        match row.state {
            CardState::Waiting => Ok(()),
            CardState::Expired => Err(refuse(
                StatusCode::GONE,
                "This card expired before anyone answered, so nothing was done.",
            )),
            _ => Err(refuse(
                StatusCode::CONFLICT,
                "This card was already decided.",
            )),
        }
    }

    /// Refuses before any approval is recorded when nothing on the platform runs the card's tool.
    fn check_runnable(&self, row: &CardRow) -> Result<(), Refusal> {
        if self
            .runners
            .iter()
            .any(|r| r.tool() == row.card.effect.tool)
        {
            Ok(())
        } else {
            Err(refuse(
                StatusCode::CONFLICT,
                "Nothing on this platform can carry this out yet, so it can't be approved. You can decline it; nothing changes either way.",
            ))
        }
    }

    fn check_fingerprint(row: &CardRow) -> Result<(), Refusal> {
        match row.card.effect.fingerprint() {
            Ok(fp) if fp == row.card.fingerprint => Ok(()),
            _ => Err(refuse(
                StatusCode::CONFLICT,
                "This card changed after it was raised, so it can't be approved.",
            )),
        }
    }

    async fn issue(
        &self,
        row: &CardRow,
        kind: ReceiptKind,
        detail: Value,
        title: String,
        now: DateTime<Utc>,
    ) -> Result<String, Refusal> {
        let mut body = ReceiptBody::new(
            kind,
            row.card.site.clone(),
            Subject::Decision(DecisionId::new(row.card.id.clone())),
            row.card.created_at,
            now,
        );
        body.detail = Some(detail);
        self.receipts
            .issue(body.with_title(title))
            .await
            .map_err(internal)
    }

    async fn run(&self, row: &CardRow, receipt: &str) -> Result<Option<String>, Refusal> {
        self.run_card(&row.card, receipt).await
    }

    /// Runs `card`'s effect: the card as raised, or as the owner edited it.
    async fn run_card(
        &self,
        card: &DecisionCardV2,
        receipt: &str,
    ) -> Result<Option<String>, Refusal> {
        let row_card = card;
        let Some(runner) = self
            .runners
            .iter()
            .find(|r| r.tool() == row_card.effect.tool)
        else {
            tracing::warn!(tool = %row_card.effect.tool, card = %row_card.id, "no runner for an approved card's tool");
            return Err(refuse(
                StatusCode::CONFLICT,
                "Your approval is recorded, but nothing on this platform can carry it out yet. Nothing else happened.",
            ));
        };
        runner.run(row_card, receipt).await.map_err(|e| {
            tracing::warn!(error = %format!("{e:#}"), card = %row_card.id, "an approved effect did not finish");
            refuse(StatusCode::CONFLICT, format!("Your approval is recorded, but it didn't finish: {}", e.to_string().lines().next().unwrap_or("try again")))
        })
    }

    /// Approves card `id` for `account` with `attempt` (the frozen approve contract).
    /// `posted_fingerprint` is the fingerprint a code form names; it must be the card's.
    pub async fn approve(
        &self,
        id: &str,
        account: &str,
        attempt: Attempt,
        posted_fingerprint: Option<&str>,
    ) -> Result<Option<String>, Refusal> {
        self.approve_with(id, account, attempt, posted_fingerprint, &Answer::default())
            .await
    }

    /// What an approval with `answer` signs: the card's fingerprint, or the edited effect's.
    fn signed(row: &CardRow, answer: &Answer) -> Result<Signed, Refusal> {
        let edit = if answer.edits.is_empty() {
            None
        } else {
            edit::apply(&row.card.effect, &row.editable, &answer.edits)
                .map_err(|w| refuse(StatusCode::BAD_REQUEST, w))?
        };
        let (fingerprint, effect) = match &edit {
            Some(e) => (e.after, e.effect.clone()),
            None => (row.card.fingerprint, row.card.effect.clone()),
        };
        // A grant is signed with the approval: the effect's fingerprint and the grant's terms.
        let (fingerprint, grant) = match answer.grant {
            Some(scope) => {
                let terms = standing::terms(&row.card, &effect, scope)
                    .map_err(|w| refuse(StatusCode::BAD_REQUEST, w))?;
                (
                    terms.fingerprint(&fingerprint).map_err(internal)?,
                    Some(terms),
                )
            }
            None => (fingerprint, None),
        };
        Ok(Signed {
            fingerprint,
            effect,
            edit,
            grant,
        })
    }

    /// The approval sheet for card `id` with `answer`: what the approval will sign, and a
    /// passkey ceremony bound to it (`passkey_options` is `None` without a passkey). The shape
    /// is the shell's sheet, so the console's approval sheet opens it as it opens any card.
    pub async fn sheet(&self, id: &str, account: &str, answer: &Answer) -> Result<Value, Refusal> {
        let row = self.card_any_site(id).await?;
        self.owner_of(&row.card.site, account).await?;
        Self::check_waiting(&row)?;
        Self::check_fingerprint(&row)?;
        self.check_runnable(&row)?;
        let signed = Self::signed(&row, answer)?;
        let uuid = uuid::Uuid::from_str(account)
            .map_err(|_| refuse(StatusCode::UNAUTHORIZED, "Sign in first."))?;
        let options = self
            .approvals
            .passkey_options(uuid, *signed.fingerprint.as_bytes())
            .await
            .map_err(internal)?;
        let mut words: Vec<String> = Vec::new();
        if let Some(e) = &signed.edit {
            words.push(format!(
                "Your version: {}",
                e.changes
                    .iter()
                    .map(|c| format!("{} \"{}\" (was \"{}\")", c.label, c.after, c.before))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        } else if row.card.effects.is_empty() {
            words.push(row.card.title.clone());
        } else {
            words.push(row.card.effects.join(" · "));
        }
        if let Some(t) = &signed.grant {
            words.push(format!("and from now on: {}", t.sentence()));
        }
        let hex = signed.fingerprint.hex();
        Ok(json!({
            "title": row.card.title,
            "sign_words": words.join(" · "),
            "fingerprint": signed.fingerprint.to_string(),
            "fingerprint_short": format!("{}…{}", &hex[..4], &hex[hex.len() - 4..]),
            "approve_url": format!("/decisions/v2/{}/approve", row.card.id),
            "has_passkey": options.is_some(),
            "passkey_options": options.map(|o| o.to_string()),
            "edits": answer.edits,
            "grant": answer.grant,
        }))
    }

    /// Approves card `id` with the owner's `answer`: as raised, or edited. An edit changes what
    /// the approval signs, so the passkey assertion (or the code) must be over the edited
    /// effect's fingerprint, which [`Decisions::sheet`] hands out.
    pub async fn approve_with(
        &self,
        id: &str,
        account: &str,
        attempt: Attempt,
        posted_fingerprint: Option<&str>,
        answer: &Answer,
    ) -> Result<Option<String>, Refusal> {
        let row = self.card_any_site(id).await?;
        self.owner_of(&row.card.site, account).await?;
        Self::check_waiting(&row)?;
        Self::check_fingerprint(&row)?;
        self.check_runnable(&row)?;
        let signed = Self::signed(&row, answer)?;
        if matches!(attempt, Attempt::Totp(_)) {
            // A code form names the card it was opened on, or the edited version it signs.
            let named = posted_fingerprint.is_some_and(|p| {
                [row.card.fingerprint, signed.fingerprint].iter().any(|fp| {
                    let ours = fp.to_string();
                    p == ours || Some(p) == ours.strip_prefix(Digest::PREFIX)
                })
            });
            if !named {
                return Err(refuse(
                    StatusCode::CONFLICT,
                    "This card changed since you opened it. Reload it and approve again.",
                ));
            }
        }
        let uuid = uuid::Uuid::from_str(account)
            .map_err(|_| refuse(StatusCode::UNAUTHORIZED, "Sign in first."))?;
        let request = match attempt {
            Attempt::Passkey(credential) => json!({ "passkey": credential }),
            Attempt::Totp(code) => json!({ "totp": code }),
        };
        let proof = self
            .approvals
            .verify(uuid, *signed.fingerprint.as_bytes(), request)
            .await
            .map_err(|e| refuse(StatusCode::FORBIDDEN, e.to_string()))?;
        let method = if proof.get("method").and_then(Value::as_str) == Some("passkey") {
            "passkey"
        } else {
            "totp"
        };
        let now = Utc::now();
        let how = if method == "passkey" {
            "your passkey"
        } else {
            "a code from your authenticator app"
        };
        let mut detail = json!({ "v": 2, "decision_id": row.card.id, "kind": row.card.kind, "tool": row.card.effect.tool, "fingerprint": signed.fingerprint, "method": method, "approval": "reviewed", "approver": account, "raised_by": row.card.raised_by, "evidence": proof });
        let mut title = format!("You approved \"{}\" with {how}", row.card.title);
        if let Some(e) = &signed.edit {
            detail["edited_by_owner"] = json!({
                "before": e.before,
                "after": e.after,
                "changes": e.changes,
            });
            title = format!(
                "You edited and approved \"{}\" with {how}: {}",
                row.card.title,
                e.changes
                    .iter()
                    .map(|c| format!(
                        "{} from \"{}\" to \"{}\"",
                        c.label.to_lowercase(),
                        c.before,
                        c.after
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        let grant_id = signed
            .grant
            .as_ref()
            .map(|_| format!("grant_{}", uuid::Uuid::now_v7().simple()));
        if let (Some(t), Some(id)) = (&signed.grant, &grant_id) {
            detail["standing_grant"] =
                json!({ "id": id, "terms": t, "signed_with_effect": row.card.fingerprint });
            title = format!("{title}, and said: {}", t.sentence());
        }
        let receipt = self
            .issue(&row, ReceiptKind::DecisionApproved, detail, title, now)
            .await?;
        self.record_with(
            &row,
            CardState::Approved,
            method,
            account,
            &receipt,
            now,
            signed.edit.as_ref(),
            None,
        )
        .await?;
        if let (Some(t), Some(id)) = (&signed.grant, &grant_id) {
            self.store_grant(id, &row, t, account, &receipt, now)
                .await
                .map_err(internal)?;
        }
        let mut card = row.card.clone();
        if signed.edit.is_some() {
            card.effect = signed.effect;
            card.fingerprint = signed.fingerprint;
        }
        let to = self.run_card(&card, &receipt).await?;
        Ok(to.or_else(|| row.card.back.clone()))
    }

    /// Declines card `id` for `account`. The owner's session is enough: declining changes nothing.
    pub async fn decline(
        &self,
        id: &str,
        account: &str,
        how: &str,
    ) -> Result<Option<String>, Refusal> {
        self.decline_with(id, account, how, &Teach::default()).await
    }

    /// Declines card `id`, with the owner's reason and, when they ask, a standing rule for the
    /// WebAgent and job that raised it. A rule needs words: the reason is the rule.
    pub async fn decline_with(
        &self,
        id: &str,
        account: &str,
        how: &str,
        teach: &Teach,
    ) -> Result<Option<String>, Refusal> {
        let row = self.card_any_site(id).await?;
        self.owner_of(&row.card.site, account).await?;
        Self::check_waiting(&row)?;
        let reason = standing::reason(teach.reason.as_deref().unwrap_or_default())
            .map_err(|w| refuse(StatusCode::BAD_REQUEST, w))?;
        if teach.rule && reason.is_none() {
            return Err(refuse(
                StatusCode::BAD_REQUEST,
                "Write the rule in a few words first, then decline.",
            ));
        }
        let rule_id = (teach.rule && reason.is_some())
            .then(|| format!("rule_{}", uuid::Uuid::now_v7().simple()));
        let now = Utc::now();
        let who = agent_words(&row.card.raised_by.webagent);
        let title = match (&reason, &rule_id) {
            (Some(r), Some(_)) => format!(
                "You declined \"{}\" and gave {who} a rule: \"{r}\"",
                row.card.title
            ),
            (Some(r), None) => format!(
                "You declined \"{}\" because \"{r}\"; nothing changed",
                row.card.title
            ),
            _ => format!("You declined \"{}\"; nothing changed", row.card.title),
        };
        let receipt = self
            .issue(
                &row,
                ReceiptKind::DecisionDeclined,
                json!({ "v": 2, "decision_id": row.card.id, "kind": row.card.kind, "tool": row.card.effect.tool, "fingerprint": row.card.fingerprint, "by": account, "how": how, "reason": reason, "rule": rule_id.as_ref().map(|id| json!({ "id": id, "webagent": row.card.raised_by.webagent, "job": row.card.raised_by.job, "words": reason })) }),
                title,
                now,
            )
            .await?;
        self.record_with(
            &row,
            CardState::Declined,
            how,
            account,
            &receipt,
            now,
            None,
            reason.as_deref(),
        )
        .await?;
        if let (Some(rule), Some(words)) = (&rule_id, &reason) {
            sqlx::query(
                "INSERT INTO decision_rules (id, site_id, webagent, job, kind, words, from_card, created_by, created_at, receipt) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(rule)
            .bind(row.card.site.as_str())
            .bind(&row.card.raised_by.webagent)
            .bind(&row.card.raised_by.job)
            .bind(&row.card.kind)
            .bind(words)
            .bind(&row.card.id)
            .bind(account)
            .bind(stored(now))
            .bind(&receipt)
            .execute(&self.pool)
            .await
            .map_err(|e| internal(e.into()))?;
        }
        if let Some(runner) = self
            .runners
            .iter()
            .find(|r| r.tool() == row.card.effect.tool)
            && let Err(e) = runner.declined(&row.card, &receipt).await
        {
            tracing::warn!(error = %format!("{e:#}"), card = %row.card.id, "tidying up after a decline failed");
        }
        Ok(row.card.back.clone())
    }

    async fn store_grant(
        &self,
        id: &str,
        row: &CardRow,
        terms: &GrantTerms,
        account: &str,
        receipt: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO decision_grants (id, site_id, webagent, kind, terms, from_card, created_by, created_at, expires_at, receipt) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(row.card.site.as_str())
        .bind(&terms.webagent)
        .bind(&terms.kind)
        .bind(serde_json::to_string(terms)?)
        .bind(&row.card.id)
        .bind(account)
        .bind(stored(now))
        .bind(stored(now + chrono::Duration::days(terms.days)))
        .bind(receipt)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE decision_cards_v2 SET grant_id = ? WHERE id = ?")
            .bind(id)
            .bind(&row.card.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The site's standing grants that haven't been stopped, newest first (ended ones too, so
    /// the owner sees when one ran out). `webagent` narrows them to one WebAgent.
    pub async fn grants(&self, site: &str, webagent: Option<&str>) -> anyhow::Result<Vec<Grant>> {
        type GrantSql = (String, String, String, String, String, String, String, i64);
        let rows: Vec<GrantSql> = match sqlx::query_as(
            "SELECT id, site_id, terms, from_card, receipt, created_at, expires_at, uses FROM decision_grants WHERE site_id = ? AND revoked_at IS NULL AND (? IS NULL OR webagent = ?) ORDER BY created_at DESC, rowid DESC",
        )
        .bind(site)
        .bind(webagent)
        .bind(webagent)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) if missing_table(&e) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        rows.into_iter()
            .map(
                |(id, site, terms, from_card, receipt, created, expires, uses)| {
                    Ok(Grant {
                        id,
                        site,
                        terms: serde_json::from_str(&terms)
                            .context("a stored grant's terms do not parse")?,
                        from_card,
                        receipt,
                        created_at: parse_time(&created)?,
                        expires_at: parse_time(&expires)?,
                        uses,
                    })
                },
            )
            .collect()
    }

    /// Stops standing grant `id` on the owner's site; returns the receipt that records it. Cards
    /// that match it from now on wait for the owner again.
    pub async fn revoke_grant(&self, id: &str, account: &str) -> Result<String, Refusal> {
        let found: Option<(String, String, String, String)> = sqlx::query_as(
            "SELECT site_id, terms, from_card, receipt FROM decision_grants WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| internal(e.into()))?;
        let (site, terms, from_card, made) = found.ok_or_else(|| {
            refuse(
                StatusCode::NOT_FOUND,
                "That grant was already stopped, or we couldn't find it.",
            )
        })?;
        let site = SiteId::new(site);
        self.owner_of(&site, account).await?;
        let terms: GrantTerms = serde_json::from_str(&terms).map_err(|e| internal(e.into()))?;
        let now = Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionApproved,
            site,
            Subject::Decision(DecisionId::new(from_card)),
            now,
            now,
        );
        body.detail = Some(
            json!({ "v": 2, "action": "grant.revoked", "grant": id, "terms": terms, "amends": made, "by": account }),
        );
        let receipt = self
            .receipts
            .issue(body.with_title(format!(
                "You stopped the standing grant for {}: {} asks you again",
                terms.words,
                agent_words(&terms.webagent)
            )))
            .await
            .map_err(internal)?;
        sqlx::query(
            "UPDATE decision_grants SET revoked_at = ?, revoked_receipt = ? WHERE id = ? AND revoked_at IS NULL",
        )
        .bind(stored(now))
        .bind(&receipt)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| internal(e.into()))?;
        Ok(receipt)
    }

    /// Approves card `id` under a standing grant when one covers it and a runner can carry it
    /// out: records it as approved by the grant, writes its receipt (which tells the owner) and
    /// runs it. Returns the receipt, or `None` when no grant covers it (it keeps waiting).
    pub async fn apply_grants(&self, id: &str) -> anyhow::Result<Option<String>> {
        let row = match self.card_any_site(id).await {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        if row.state != CardState::Waiting
            || Self::check_fingerprint(&row).is_err()
            || self.check_runnable(&row).is_err()
        {
            return Ok(None);
        }
        let now = Utc::now();
        let Some(grant) = self
            .grants(row.card.site.as_str(), Some(&row.card.raised_by.webagent))
            .await?
            .into_iter()
            .find(|g| g.covers(&row.card, now))
        else {
            return Ok(None);
        };
        let who = agent_words(&row.card.raised_by.webagent);
        let receipt = self
            .issue(
                &row,
                ReceiptKind::DecisionApproved,
                json!({ "v": 2, "decision_id": row.card.id, "kind": row.card.kind, "tool": row.card.effect.tool, "fingerprint": row.card.fingerprint, "method": "grant", "approval": "standing-grant", "counts_for_promotion": false, "grant": { "id": grant.id, "receipt": grant.receipt, "terms": grant.terms }, "raised_by": row.card.raised_by }),
                format!(
                    "{} went ahead with \"{}\" under your standing grant for {}, and is telling you",
                    sentence_start(&who),
                    row.card.title,
                    grant.terms.words
                ),
                now,
            )
            .await
            .map_err(|r| anyhow::anyhow!(r.words))?;
        let by = format!("grant:{}", grant.id);
        if self
            .record(&row, CardState::Approved, "grant", &by, &receipt, now)
            .await
            .is_err()
        {
            return Ok(None);
        }
        sqlx::query("UPDATE decision_cards_v2 SET grant_id = ? WHERE id = ?")
            .bind(&grant.id)
            .bind(&row.card.id)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE decision_grants SET uses = uses + 1 WHERE id = ?")
            .bind(&grant.id)
            .execute(&self.pool)
            .await?;
        if let Err(r) = self.run(&row, &receipt).await {
            tracing::warn!(card = %row.card.id, words = %r.words, "a card approved under a grant did not finish");
        }
        Ok(Some(receipt))
    }

    /// Applies standing grants to every waiting card; returns how many were approved under one.
    /// Node runs it in the background, so cards raised without runners on hand are covered too.
    pub async fn sweep_grants(&self) -> anyhow::Result<usize> {
        let ids: Vec<(String,)> = match sqlx::query_as(
            "SELECT c.id FROM decision_cards_v2 c WHERE c.state = 'waiting' AND EXISTS (SELECT 1 FROM decision_grants g WHERE g.site_id = c.site_id AND g.kind = c.kind AND g.revoked_at IS NULL)",
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) if missing_table(&e) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let mut n = 0;
        for (id,) in ids {
            if self.apply_grants(&id).await?.is_some() {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Raises a card whose `editable` fields the owner may change before approving. With a
    /// `dedupe_key` already used, returns that card and leaves its fields as they were.
    pub async fn raise_editable(
        &self,
        new: NewCard,
        editable: Vec<EditableField>,
    ) -> anyhow::Result<DecisionCardV2> {
        let key = new.dedupe_key.clone();
        let site = new.effect.site.clone();
        if let Some(k) = &key {
            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM decision_cards_v2 WHERE site_id = ? AND dedupe_key = ?",
            )
            .bind(site.as_str())
            .bind(k)
            .fetch_optional(&self.pool)
            .await?;
            if existing.is_some() {
                return self.raise(new).await;
            }
        }
        for f in &editable {
            anyhow::ensure!(
                (f.kind == EditKind::AmountDown) == (f.key == "amount"),
                "editable field {}: only `amount` is an amount, and it is only lowered",
                f.key
            );
            if f.kind == EditKind::AmountDown {
                anyhow::ensure!(
                    new.effect.amount.is_some(),
                    "editable amount on a card without one"
                );
            } else {
                anyhow::ensure!(
                    new.effect.args.get(&f.key).is_some(),
                    "editable field {} is not in the effect's arguments",
                    f.key
                );
            }
        }
        let card = self.raise(new).await?;
        if !editable.is_empty() {
            sqlx::query("UPDATE decision_cards_v2 SET editable = ? WHERE id = ?")
                .bind(serde_json::to_string(&editable)?)
                .bind(&card.id)
                .execute(&self.pool)
                .await?;
        }
        Ok(card)
    }

    /// The site's standing rules, newest first. `webagent` narrows them to one WebAgent.
    pub async fn rules(&self, site: &str, webagent: Option<&str>) -> anyhow::Result<Vec<Rule>> {
        type RuleSql = (
            String,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
            String,
            String,
        );
        let rows: Vec<RuleSql> = match sqlx::query_as(
            "SELECT id, site_id, webagent, job, kind, words, from_card, receipt, created_at FROM decision_rules WHERE site_id = ? AND removed_at IS NULL AND (? IS NULL OR webagent = ?) ORDER BY created_at DESC, rowid DESC",
        )
        .bind(site)
        .bind(webagent)
        .bind(webagent)
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) if missing_table(&e) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        rows.into_iter()
            .map(
                |(id, site, webagent, job, kind, words, from_card, receipt, at)| {
                    Ok(Rule {
                        id,
                        site,
                        webagent,
                        job,
                        kind,
                        words,
                        from_card,
                        receipt,
                        created_at: parse_time(&at)?,
                    })
                },
            )
            .collect()
    }

    /// Removes standing rule `id` from the owner's site; returns the receipt that records it.
    /// The WebAgent stops following it at once; the rule stays in the record.
    pub async fn remove_rule(&self, id: &str, account: &str) -> Result<String, Refusal> {
        let found: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT site_id, webagent, words, from_card, receipt FROM decision_rules WHERE id = ? AND removed_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| internal(e.into()))?;
        let (site, webagent, words, from_card, made) = found.ok_or_else(|| {
            refuse(
                StatusCode::NOT_FOUND,
                "That rule was already removed, or we couldn't find it.",
            )
        })?;
        let site = SiteId::new(site);
        self.owner_of(&site, account).await?;
        let now = Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionDeclined,
            site,
            Subject::Decision(DecisionId::new(from_card.clone())),
            now,
            now,
        );
        body.detail = Some(
            json!({ "v": 2, "action": "rule.removed", "rule": id, "webagent": webagent, "words": words, "amends": made, "by": account }),
        );
        let receipt = self
            .receipts
            .issue(body.with_title(format!(
                "You removed {}'s rule: \"{words}\"",
                agent_words(&webagent)
            )))
            .await
            .map_err(internal)?;
        sqlx::query(
            "UPDATE decision_rules SET removed_at = ?, removed_receipt = ? WHERE id = ? AND removed_at IS NULL",
        )
        .bind(stored(now))
        .bind(&receipt)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| internal(e.into()))?;
        Ok(receipt)
    }

    /// A spoken or chat answer (07 §5.3): "allow once" only for a low-risk one-tap card,
    /// "deny" for any card. `heard` is what the owner said, kept on the receipt as words only;
    /// no recording is kept. Returns the receipt's digest.
    pub async fn decide_spoken(
        &self,
        site: &str,
        account: &str,
        id: &str,
        allow: bool,
        channel: &str,
        heard: &str,
    ) -> Result<String, Refusal> {
        let row = self
            .get(site, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| refuse(StatusCode::NOT_FOUND, "We couldn't find that card."))?;
        self.owner_of(&row.card.site, account).await?;
        Self::check_waiting(&row)?;
        let surface = surface::Surface::parse(channel).unwrap_or(surface::Surface::Voice);
        let powers = surface::powers(surface, &row.card);
        if !powers.decline {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                "Email can't decide anything. Open the card in the console.",
            ));
        }
        let method = if surface == surface::Surface::WhatsApp {
            "chat"
        } else {
            "spoken"
        };
        let now = Utc::now();
        if !allow {
            let receipt = self
                .issue(
                    &row,
                    ReceiptKind::DecisionDeclined,
                    json!({ "v": 2, "decision_id": row.card.id, "kind": row.card.kind, "fingerprint": row.card.fingerprint, "by": account, "how": method, "heard": heard }),
                    format!("You declined \"{}\" by {}; nothing changed", row.card.title, if method == "chat" { "message" } else { "voice" }),
                    now,
                )
                .await?;
            self.record(&row, CardState::Declined, method, account, &receipt, now)
                .await?;
            return Ok(receipt);
        }
        if !powers.allow_once {
            return Err(refuse(
                StatusCode::FORBIDDEN,
                "This one needs your passkey or an authenticator code. Open it in the console.",
            ));
        }
        Self::check_fingerprint(&row)?;
        self.check_runnable(&row)?;
        let receipt = self
            .issue(
                &row,
                ReceiptKind::DecisionApproved,
                json!({ "v": 2, "decision_id": row.card.id, "kind": row.card.kind, "tool": row.card.effect.tool, "fingerprint": row.card.fingerprint, "method": method, "approval": "one-tap", "counts_for_promotion": false, "approver": account, "heard": heard }),
                format!("You allowed \"{}\" once by {}", row.card.title, if method == "chat" { "message" } else { "voice" }),
                now,
            )
            .await?;
        self.record(&row, CardState::Approved, method, account, &receipt, now)
            .await?;
        self.run(&row, &receipt).await?;
        Ok(receipt)
    }
}

// ---------- Routes: the frozen approve contract's shape ----------

/// A reply: 303 to a form, `{redirect}` or `{message}` to JSON.
struct Reply {
    json: bool,
    outcome: Result<String, Refusal>,
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        match (self.outcome, self.json) {
            (Ok(to), true) => axum::Json(json!({ "redirect": to })).into_response(),
            (Ok(to), false) => Redirect::to(&to).into_response(),
            (Err(r), _) => (r.status, axum::Json(json!({ "message": r.words }))).into_response(),
        }
    }
}

fn wants_json(headers: &HeaderMap) -> bool {
    [header::CONTENT_TYPE, header::ACCEPT].into_iter().any(|h| {
        headers
            .get(h)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("application/json"))
    })
}

async fn account(d: &Decisions, headers: &HeaderMap) -> Result<String, Refusal> {
    match d.owners.owner(headers).await {
        Ok(Some(o)) => Ok(o.account.to_string()),
        Ok(None) => Err(refuse(StatusCode::UNAUTHORIZED, "Sign in first.")),
        Err(e) => Err(internal(e)),
    }
}

fn back_or(row_back: Option<String>, headers: &HeaderMap) -> String {
    row_back
        .or_else(|| {
            headers
                .get(header::REFERER)
                .and_then(|v| v.to_str().ok())
                .and_then(|r| url::Url::parse(r).ok())
                .map(|u| u.path().to_owned())
                .filter(|p| p.starts_with("/console/"))
        })
        .unwrap_or_else(|| "/os".to_owned())
}

fn is_json_body(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"))
}

/// A posted body as fields: a JSON object, or a form. In a form, `edit.<key>` fields are edits.
struct Posted {
    json: Value,
    form: std::collections::HashMap<String, String>,
}

impl Posted {
    fn read(headers: &HeaderMap, body: &Bytes) -> Result<Self, Refusal> {
        if is_json_body(headers) {
            let json: Value = if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(body)
                    .map_err(|_| refuse(StatusCode::BAD_REQUEST, "That answer couldn't be read."))?
            };
            Ok(Self {
                json,
                form: Default::default(),
            })
        } else {
            Ok(Self {
                json: Value::Null,
                form: url::form_urlencoded::parse(body).into_owned().collect(),
            })
        }
    }

    fn str(&self, key: &str) -> Option<String> {
        self.json
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.form.get(key).cloned())
    }

    fn flag(&self, key: &str) -> bool {
        match self.json.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => matches!(s.as_str(), "on" | "true" | "1" | "yes"),
            _ => self
                .form
                .get(key)
                .is_some_and(|v| matches!(v.as_str(), "on" | "true" | "1" | "yes")),
        }
    }

    fn answer(&self) -> Result<Answer, Refusal> {
        let mut edits = BTreeMap::new();
        match self.json.get("edits") {
            Some(Value::Object(m)) => {
                for (k, v) in m {
                    let v = v.as_str().ok_or_else(|| {
                        refuse(StatusCode::BAD_REQUEST, "That edit couldn't be read.")
                    })?;
                    edits.insert(k.clone(), v.to_owned());
                }
            }
            Some(Value::Null) | None => {}
            Some(_) => {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "That edit couldn't be read.",
                ));
            }
        }
        for (k, v) in &self.form {
            if let Some(key) = k.strip_prefix("edit.") {
                edits.insert(key.to_owned(), v.clone());
            }
        }
        let grant =
            match self.str("grant").as_deref() {
                None | Some("") | Some("none") => None,
                Some(g) => Some(GrantScope::parse(g).ok_or_else(|| {
                    refuse(StatusCode::BAD_REQUEST, "That choice couldn't be read.")
                })?),
            };
        Ok(Answer { edits, grant })
    }
}

async fn approve(
    State(d): State<Decisions>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let json_reply = wants_json(&headers);
    let outcome = async {
        let who = account(&d, &headers).await?;
        let posted = Posted::read(&headers, &body)?;
        let answer = posted.answer()?;
        let (attempt, fingerprint) = match posted.str("method").as_deref() {
            Some("passkey") if is_json_body(&headers) => (
                Attempt::Passkey(
                    posted
                        .json
                        .get("credential")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                None,
            ),
            Some("totp") => (
                Attempt::Totp(posted.str("code").unwrap_or_default()),
                posted.str("fingerprint"),
            ),
            _ => {
                return Err(refuse(
                    StatusCode::BAD_REQUEST,
                    "Approve with your passkey or a code from your authenticator app.",
                ));
            }
        };
        let to = d
            .approve_with(&id, &who, attempt, fingerprint.as_deref(), &answer)
            .await?;
        Ok(back_or(to, &headers))
    }
    .await;
    Reply {
        json: json_reply,
        outcome,
    }
}

async fn decline(
    State(d): State<Decisions>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let json_reply = wants_json(&headers);
    let outcome = async {
        let who = account(&d, &headers).await?;
        let posted = Posted::read(&headers, &body)?;
        let teach = Teach {
            reason: posted.str("reason"),
            rule: posted.flag("rule"),
        };
        let to = d.decline_with(&id, &who, "console", &teach).await?;
        Ok(back_or(to, &headers))
    }
    .await;
    Reply {
        json: json_reply,
        outcome,
    }
}

/// `POST /decisions/v2/{id}/sheet`: the approval sheet for an edited card, as JSON.
async fn sheet(
    State(d): State<Decisions>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let outcome = async {
        let who = account(&d, &headers).await?;
        let answer = Posted::read(&headers, &body)?.answer()?;
        d.sheet(&id, &who, &answer).await
    }
    .await;
    match outcome {
        Ok(v) => ([(header::CACHE_CONTROL, "no-store")], axum::Json(v)).into_response(),
        Err(r) => (r.status, axum::Json(json!({ "message": r.words }))).into_response(),
    }
}

/// `POST /decisions/v2/rules/{id}/remove`.
async fn remove_rule(
    State(d): State<Decisions>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Reply {
    let json_reply = wants_json(&headers);
    let outcome = async {
        let who = account(&d, &headers).await?;
        d.remove_rule(&id, &who).await?;
        Ok(back_or(None, &headers))
    }
    .await;
    Reply {
        json: json_reply,
        outcome,
    }
}

/// `POST /decisions/v2/grants/{id}/revoke`.
async fn revoke_grant(
    State(d): State<Decisions>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Reply {
    let json_reply = wants_json(&headers);
    let outcome = async {
        let who = account(&d, &headers).await?;
        d.revoke_grant(&id, &who).await?;
        Ok(back_or(None, &headers))
    }
    .await;
    Reply {
        json: json_reply,
        outcome,
    }
}

/// The v2 routes: approve, decline, the sheet for an edited card or a grant, removing a rule
/// and stopping a grant.
pub fn router(d: Decisions) -> Router {
    Router::new()
        .route("/decisions/v2/{id}/approve", post(approve))
        .route("/decisions/v2/{id}/decline", post(decline))
        .route("/decisions/v2/{id}/sheet", post(sheet))
        .route("/decisions/v2/rules/{id}/remove", post(remove_rule))
        .route("/decisions/v2/grants/{id}/revoke", post(revoke_grant))
        .with_state(d)
}

/// Mounts the v2 routes on the platform host, with no runners (C11). Node uses
/// [`mount_with`] once crates register runners for their tools.
pub fn mount(ctx: &NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    mount_with(ctx, Vec::new())
}

/// Mounts the v2 routes with the effect runners other crates register.
pub fn mount_with(
    ctx: &NodeContext,
    runners: Vec<Arc<dyn EffectRunner>>,
) -> anyhow::Result<agentrix_host::Mounted> {
    let d = Decisions::new(ctx).with_runners(runners);
    let sweep = d.clone();
    Ok(agentrix_host::Mounted::default().platform(router(d)).task(
        "decisions-standing-grants",
        Box::pin(async move {
            // Cards raised elsewhere (without runners on hand) wait at most this long for a
            // standing grant to approve them.
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
            loop {
                tick.tick().await;
                if let Err(e) = sweep.sweep_grants().await {
                    tracing::warn!(error = %format!("{e:#}"), "applying standing grants failed");
                }
            }
        }),
    ))
}
