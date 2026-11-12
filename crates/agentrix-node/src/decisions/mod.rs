//! Decision cards and approvals bound to effect fingerprints (R7.2, R5.4, R9.2; design §4.1, D9).
//!
//! Lane: `move` (specs/first-release/design.md §3.0). Tasks S2-01 (approve → go-live) and S2-06's
//! POST (a quarantine restore raises a card).
//!
//! Routes ([`router`]):
//! - `POST /decisions/{id}/approve`: JSON `{method: "passkey", credential}` (the options were
//!   embedded in the card by [`DecisionsState::begin_approval`]; challenge =
//!   SHA-256(fingerprint ‖ nonce)) or form `{method: "totp", code, fingerprint}`, a step-up
//!   bound to the same fingerprint. Replies 303 to a form, `{redirect}` to JSON; errors are
//!   `{message}` in plain words. The `decision.approved` receipt records the method. Then the
//!   card's effect runs: go-live through the LaunchGate, a quarantine restore, or an order
//!   that the agent door reports as completed.
//! - `POST /decisions/{id}/decline`: the owner's session is enough; a `decision.declined`
//!   receipt.
//! - `POST /sites/{id}/quarantine/{item}/restore` (form `reason`): raises a
//!   `quarantine-restore` card; approving it is the signed decision (R5.4). Executables and
//!   the old site's code can never be restored (A-I1).
//! - `POST /sites/{id}/rollback` (form `to`: `previous` or `old-site`): undo, in one step,
//!   for 30 days (R7.4).
//!
//! [`HeldOrders`] is the agent door's `door::HeldCalls`: an order is held as an
//! `order-held` card, idempotent per key, and its status follows the card.

pub mod moves;
#[cfg(test)]
mod tests;
pub mod watch;

use std::str::FromStr;
use std::sync::Arc;

use agentrix_move::golive::restore_card;
use agentrix_move::pipeline::kind_of_title;
use agentrix_move::ports::Events;
use agentrix_move::store::{CardRecord, CardState, QuarantineState};
use agentrix_move::{MoveState, MoveStore};
use agentrix_types::ui::{CardEvent, DoneEvent, DoneOutcome, StepEvent, StepName, StepState};
use agentrix_types::{
    Approval, ApprovalMethod, CardKind, DecisionCard, DecisionId, Digest, EffectRequest, MoveId,
    QuarantineId, ReceiptBody, ReceiptKind, SiteId, Subject, UiEvent,
};
use async_trait::async_trait;
use awp_types::{HeldCall, HeldCallState};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use chrono::{Duration, Utc};
use serde_json::{Value, json};

use crate::db::receipts::ReceiptStore;
use crate::door::HeldCalls;
use crate::launch::{LaunchGate, RollbackTo};
use crate::pages::reads::{OwnerSession, ReadResult};

/// How the owner tried to approve.
#[derive(Debug, Clone, PartialEq)]
pub enum Attempt {
    /// A webauthn-rs `PublicKeyCredential`, as JSON.
    Passkey(Value),
    /// A code from the owner's authenticator app.
    Totp(String),
}

/// A verified approval.
#[derive(Debug, Clone, PartialEq)]
pub struct Proof {
    /// How.
    pub method: ApprovalMethod,
    /// The evidence for the receipt: the assertion, or the TOTP step.
    pub evidence: Value,
}

/// Verifies approvals over a fingerprint (`auth/`'s approvals in production).
#[async_trait]
pub trait Approver: Send + Sync {
    /// Passkey options bound to `fingerprint`, as JSON for `card.passkey_options`; `None` when
    /// the owner has no passkey.
    async fn options(&self, account: &str, fingerprint: &Digest) -> anyhow::Result<Option<String>>;
    /// Checks an approval. `Err` carries a plain-words reason for the owner.
    async fn verify(
        &self,
        account: &str,
        fingerprint: &Digest,
        attempt: Attempt,
    ) -> Result<Proof, String>;
}

#[async_trait]
impl Approver for crate::auth::Approvals {
    async fn options(&self, account: &str, fingerprint: &Digest) -> anyhow::Result<Option<String>> {
        let account = uuid::Uuid::from_str(account)?;
        Ok(self
            .passkey_options(account, *fingerprint.as_bytes())
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .map(|o| serde_json::to_string(&o))
            .transpose()?)
    }

    async fn verify(
        &self,
        account: &str,
        fingerprint: &Digest,
        attempt: Attempt,
    ) -> Result<Proof, String> {
        let account = uuid::Uuid::from_str(account).map_err(|_| "Sign in first.".to_owned())?;
        let request = match attempt {
            Attempt::Passkey(credential) => crate::auth::ApproveRequest::Passkey {
                credential: serde_json::from_value(credential)
                    .map_err(|_| "That passkey answer couldn't be read. Try again.".to_owned())?,
            },
            Attempt::Totp(code) => crate::auth::ApproveRequest::Totp { code },
        };
        let proof = crate::auth::Approvals::verify(self, account, *fingerprint.as_bytes(), request)
            .await
            .map_err(|e| e.to_string())?;
        let method = if proof.method == "passkey" {
            ApprovalMethod::Passkey
        } else {
            ApprovalMethod::StepUp
        };
        Ok(Proof {
            method,
            evidence: serde_json::to_value(&proof).unwrap_or(Value::Null),
        })
    }
}

/// The signed-in owner's account id (`auth/`'s session cookie in production).
#[async_trait]
pub trait Sessions: Send + Sync {
    /// The account behind the request, or `None`.
    async fn account(&self, headers: &HeaderMap) -> Option<String>;
}

#[async_trait]
impl Sessions for crate::auth::AuthState {
    async fn account(&self, headers: &HeaderMap) -> Option<String> {
        self.session_from(headers)
            .await
            .ok()
            .flatten()
            .map(|s| s.account.to_string())
    }
}

/// What the decision routes need.
#[derive(Clone)]
pub struct DecisionsState {
    /// The move lane's tables.
    pub store: MoveStore,
    /// The receipts store.
    pub receipts: Arc<ReceiptStore>,
    /// Go-live and rollback.
    pub gate: LaunchGate,
    /// Approval checks.
    pub approver: Arc<dyn Approver>,
    /// Who is signed in.
    pub sessions: Arc<dyn Sessions>,
    /// The Move's live stream.
    pub events: Arc<dyn Events>,
    /// The platform's checks (the owner's Freeze stops go-live).
    pub guards: crate::guards::Guards,
}

impl DecisionsState {
    /// For `pages::store::Delegates::begin_approval`: starts the passkey ceremony bound to the
    /// card's fingerprint (D9) and returns its options as JSON, or `None` without a passkey.
    pub async fn begin_approval(
        &self,
        session: &OwnerSession,
        card: &DecisionCard,
    ) -> ReadResult<Option<String>> {
        self.approver
            .options(&session.account, &card.fingerprint)
            .await
    }
}

/// The routes, for `app.rs` to merge into the platform router.
pub fn router(state: DecisionsState) -> Router {
    Router::new()
        .route("/decisions/{id}/approve", post(approve))
        .route("/decisions/{id}/decline", post(decline))
        .route("/sites/{site}/quarantine/{item}/restore", post(restore))
        .route("/sites/{site}/rollback", post(rollback))
        .with_state(state)
}

/// A reply: 303 to a form, `{redirect}` or `{message}` to JSON.
struct Reply {
    json: bool,
    outcome: Result<String, (StatusCode, String)>,
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        match (self.outcome, self.json) {
            (Ok(to), true) => axum::Json(json!({ "redirect": to })).into_response(),
            (Ok(to), false) => Redirect::to(&to).into_response(),
            (Err((status, message)), _) => {
                (status, axum::Json(json!({ "message": message }))).into_response()
            }
        }
    }
}

fn wants_json(headers: &HeaderMap) -> bool {
    let has = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("application/json"))
    };
    has(header::CONTENT_TYPE) || has(header::ACCEPT)
}

fn form(body: &[u8]) -> std::collections::HashMap<String, String> {
    url::form_urlencoded::parse(body).into_owned().collect()
}

type Fail = (StatusCode, String);

fn fail(status: StatusCode, message: impl Into<String>) -> Fail {
    (status, message.into())
}

fn internal(e: anyhow::Error) -> Fail {
    tracing::error!(error = %format!("{e:#}"), "decision route failed");
    fail(
        StatusCode::INTERNAL_SERVER_ERROR,
        "We couldn't finish that on our side. Nothing changed. Try again in a minute.",
    )
}

impl DecisionsState {
    async fn owner_of(&self, headers: &HeaderMap, site: &SiteId) -> Result<String, Fail> {
        let account = self
            .sessions
            .account(headers)
            .await
            .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "Sign in first."))?;
        let record = self
            .store
            .site(site)
            .await
            .map_err(internal)?
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "We couldn't find that site."))?;
        if record.owner.as_deref() != Some(account.as_str()) {
            return Err(fail(
                StatusCode::FORBIDDEN,
                "Only the site's owner can decide this.",
            ));
        }
        Ok(account)
    }

    async fn card_for(&self, headers: &HeaderMap, id: &str) -> Result<(CardRecord, String), Fail> {
        let record = self
            .store
            .card(&DecisionId::new(id))
            .await
            .map_err(internal)?
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "We couldn't find that card."))?;
        let account = self.owner_of(headers, &record.card.site).await?;
        Ok((record, account))
    }

    async fn issue(&self, body: ReceiptBody, title: String) -> Result<Digest, Fail> {
        Ok(self
            .receipts
            .issue(body.with_title(title))
            .await
            .map_err(internal)?
            .digest)
    }

    /// Runs an approved card's effect; returns where to send the owner.
    async fn run_effect(
        &self,
        card: &DecisionCard,
        receipt: &Digest,
        account: &str,
    ) -> Result<String, Fail> {
        match card.kind {
            CardKind::GoLive => {
                self.gate.go_live(card, receipt, account).await.map_err(|e| {
                    tracing::warn!(error = %format!("{e:#}"), "go-live after approval did not finish");
                    fail(
                        StatusCode::CONFLICT,
                        format!("Your approval is recorded, but going live didn't finish: {}", first_line(&e)),
                    )
                })?;
                Ok("/console/move".into())
            }
            CardKind::QuarantineRestore => {
                let item = QuarantineId::new(card.effect.args["item"].as_str().unwrap_or_default());
                let now = Utc::now();
                self.store
                    .mark_restored(&card.site, &item, now)
                    .await
                    .map_err(internal)?;
                let mut body = ReceiptBody::new(
                    ReceiptKind::QuarantineRestored,
                    card.site.clone(),
                    Subject::Decision(card.decision_id.clone()),
                    now,
                    now,
                );
                body.inputs = vec![*receipt];
                body.detail = Some(json!({
                    "item": item,
                    "title": card.effect.args["title"],
                    "where": card.effect.args["where"],
                    "decision": card.decision_id,
                }));
                let title = format!(
                    "You put back \"{}\"; it returns on the next rebuild",
                    card.effect.args["title"].as_str().unwrap_or("an item")
                );
                self.issue(body, title).await?;
                Ok(format!("/console/{}/quarantine", card.site))
            }
            CardKind::OrderHeld => Ok(format!("/console/{}/decisions", card.site)),
        }
    }

    async fn approve(&self, headers: &HeaderMap, id: &str, body: &[u8]) -> Result<String, Fail> {
        let (record, account) = self.card_for(headers, id).await?;
        let card = record.card;
        let now = Utc::now();
        let json_body = wants_json(headers);
        let attempt = if json_body {
            let v: Value = serde_json::from_slice(body)
                .map_err(|_| fail(StatusCode::BAD_REQUEST, "That approval couldn't be read."))?;
            match v.get("method").and_then(Value::as_str) {
                Some("passkey") => {
                    Attempt::Passkey(v.get("credential").cloned().unwrap_or(Value::Null))
                }
                Some("totp") => {
                    check_posted_fingerprint(v.get("fingerprint").and_then(Value::as_str), &card)?;
                    Attempt::Totp(
                        v.get("code")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    )
                }
                _ => {
                    return Err(fail(
                        StatusCode::BAD_REQUEST,
                        "Approve with your passkey or a code from your authenticator app.",
                    ));
                }
            }
        } else {
            let f = form(body);
            match f.get("method").map(String::as_str) {
                Some("totp") => {
                    check_posted_fingerprint(f.get("fingerprint").map(String::as_str), &card)?;
                    Attempt::Totp(f.get("code").cloned().unwrap_or_default())
                }
                _ => {
                    return Err(fail(
                        StatusCode::BAD_REQUEST,
                        "Approve with your passkey or a code from your authenticator app.",
                    ));
                }
            }
        };

        if record.state == CardState::Approved {
            // The approval stands; only a go-live that didn't finish may be retried.
            if card.kind == CardKind::GoLive
                && let Some(receipt) = record.receipt
                && let Ok(args) = agentrix_move::golive::GoLiveArgs::of(&card)
                && self
                    .store
                    .release(&card.site, args.seq)
                    .await
                    .map_err(internal)?
                    .is_some_and(|r| r.checkpoint.is_none())
            {
                return self.run_effect(&card, &receipt, &account).await;
            }
            return Err(fail(
                StatusCode::CONFLICT,
                "This card was already approved.",
            ));
        }
        if record.state != CardState::Waiting {
            return Err(fail(StatusCode::CONFLICT, "This card was already decided."));
        }
        // While the owner's Freeze is in force nothing goes live; the card keeps waiting
        // (W5, handover-wiring 2b.1).
        if card.kind == CardKind::GoLive && self.guards.frozen(card.site.as_str()).await {
            return Err(fail(StatusCode::CONFLICT, FROZEN_GO_LIVE));
        }
        if card.effect.is_expired(now) {
            self.store
                .expire(&card.decision_id)
                .await
                .map_err(internal)?;
            presence_decided(&card);
            return Err(fail(
                StatusCode::GONE,
                "This card expired before anyone answered, so nothing was done.",
            ));
        }
        if !card.fingerprint_matches().map_err(|e| internal(e.into()))? {
            return Err(fail(
                StatusCode::CONFLICT,
                "This card changed after it was raised, so it can't be approved.",
            ));
        }
        let proof = self
            .approver
            .verify(&account, &card.fingerprint, attempt)
            .await
            .map_err(|message| fail(StatusCode::FORBIDDEN, message))?;

        let approval = Approval {
            decision_id: card.decision_id.clone(),
            fingerprint: card.fingerprint,
            method: proof.method,
            approver: account.clone(),
            at: now,
        };
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionApproved,
            card.site.clone(),
            Subject::Decision(card.decision_id.clone()),
            card.created_at,
            now,
        );
        let mut detail = serde_json::to_value(&approval).map_err(|e| internal(e.into()))?;
        detail["kind"] = serde_json::to_value(card.kind).unwrap_or(Value::Null);
        detail["tool"] = json!(card.effect.tool);
        detail["evidence"] = proof.evidence.clone();
        body.detail = Some(detail);
        let how = match proof.method {
            ApprovalMethod::Passkey => "your passkey",
            ApprovalMethod::StepUp => "a code from your authenticator app",
        };
        let receipt = self
            .issue(body, format!("You approved \"{}\" with {how}", card.title))
            .await?;
        let method = serde_json::to_value(proof.method)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        if !self
            .store
            .approve(
                &card.decision_id,
                &method,
                &account,
                &proof.evidence,
                &receipt,
                now,
            )
            .await
            .map_err(internal)?
        {
            return Err(fail(StatusCode::CONFLICT, "This card was already decided."));
        }
        presence_decided(&card);
        self.run_effect(&card, &receipt, &account).await
    }

    async fn decline(&self, headers: &HeaderMap, id: &str) -> Result<String, Fail> {
        let (record, account) = self.card_for(headers, id).await?;
        let card = record.card;
        if record.state != CardState::Waiting {
            return Err(fail(StatusCode::CONFLICT, "This card was already decided."));
        }
        let now = Utc::now();
        let mut body = ReceiptBody::new(
            ReceiptKind::DecisionDeclined,
            card.site.clone(),
            Subject::Decision(card.decision_id.clone()),
            card.created_at,
            now,
        );
        body.detail = Some(json!({
            "decision_id": card.decision_id,
            "fingerprint": card.fingerprint,
            "kind": card.kind,
            "tool": card.effect.tool,
            "by": account,
        }));
        let receipt = self
            .issue(
                body,
                format!("You declined \"{}\"; nothing changed", card.title),
            )
            .await?;
        if !self
            .store
            .decline(&card.decision_id, &account, &receipt, now)
            .await
            .map_err(internal)?
        {
            return Err(fail(StatusCode::CONFLICT, "This card was already decided."));
        }
        presence_decided(&card);
        match card.kind {
            CardKind::GoLive => {
                if let Ok(args) = agentrix_move::golive::GoLiveArgs::of(&card) {
                    self.store
                        .set_state(&args.move_id, MoveState::Stopped, now)
                        .await
                        .map_err(internal)?;
                    let summary = "You declined go-live; your old site is unchanged";
                    self.store
                        .record_step(
                            &args.move_id,
                            &card.site,
                            StepName::GoLive,
                            StepState::Failed,
                            summary,
                            Some(&receipt),
                            None,
                            now,
                            Some(now),
                        )
                        .await
                        .map_err(internal)?;
                    let _ = self
                        .events
                        .emit(
                            &card.site,
                            &args.move_id,
                            UiEvent::Step(StepEvent::new(
                                StepName::GoLive,
                                StepState::Failed,
                                0,
                                summary,
                                now,
                            )),
                        )
                        .await;
                    let _ = self
                        .events
                        .emit(
                            &card.site,
                            &args.move_id,
                            UiEvent::Done(DoneEvent {
                                outcome: DoneOutcome::Stopped,
                                href: None,
                                at: now,
                            }),
                        )
                        .await;
                }
            }
            CardKind::QuarantineRestore => {
                let item = QuarantineId::new(card.effect.args["item"].as_str().unwrap_or_default());
                self.store
                    .clear_restore(&card.site, &item, now)
                    .await
                    .map_err(internal)?;
            }
            CardKind::OrderHeld => {}
        }
        Ok(format!("/console/{}/decisions", card.site))
    }

    async fn restore(
        &self,
        headers: &HeaderMap,
        site: &str,
        item: &str,
        body: &[u8],
    ) -> Result<String, Fail> {
        let site = SiteId::new(site);
        self.owner_of(headers, &site).await?;
        let reason = form(body)
            .get("reason")
            .map(|r| r.trim().to_owned())
            .unwrap_or_default();
        if reason.is_empty() {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Tell us why it should come back, in a few words.",
            ));
        }
        if reason.chars().count() > 500 {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "Please keep the reason under 500 characters.",
            ));
        }
        let item = self
            .store
            .quarantine_item(&site, &QuarantineId::new(item))
            .await
            .map_err(internal)?
            .ok_or_else(|| fail(StatusCode::NOT_FOUND, "We couldn't find that item."))?;
        if item.state != QuarantineState::Held {
            return Err(fail(StatusCode::CONFLICT, "That item isn't held any more."));
        }
        if let Some(waiting) = &item.decision {
            return Ok(format!("/console/{site}/decisions/{waiting}"));
        }
        if !kind_of_title(&item.title).is_some_and(|k| k.restorable()) {
            return Err(fail(
                StatusCode::CONFLICT,
                "This can never be put back: it is code a web server would run, and the new site carries none of the old site's code.",
            ));
        }
        let now = Utc::now();
        let card = restore_card(&site, &item.id, &item.title, &item.location, &reason, now)
            .map_err(internal)?;
        self.store.insert_card(&card).await.map_err(internal)?;
        self.store
            .ask_restore(&site, &item.id, &card.decision_id, now)
            .await
            .map_err(internal)?;
        let href = format!("/console/{site}/decisions/{}", card.decision_id);
        if let Some(move_id) = &item.move_id {
            let _ = self
                .events
                .emit(
                    &site,
                    move_id,
                    UiEvent::Card(CardEvent {
                        decision_id: card.decision_id.clone(),
                        kind: CardKind::QuarantineRestore,
                        title: card.title.clone(),
                        href: href.clone(),
                        at: now,
                    }),
                )
                .await;
        }
        Ok(href)
    }

    async fn rollback(&self, headers: &HeaderMap, site: &str, body: &[u8]) -> Result<String, Fail> {
        let site = SiteId::new(site);
        let account = self.owner_of(headers, &site).await?;
        let to = match form(body).get("to").map(String::as_str) {
            Some("old-site") => RollbackTo::OldSite,
            _ => RollbackTo::Previous,
        };
        self.gate
            .roll_back(&site, to, &account)
            .await
            .map_err(|e| fail(StatusCode::CONFLICT, first_line(&e)))?;
        Ok("/console/move".into())
    }
}

/// Why go-live waits while the owner's Freeze is in force.
pub const FROZEN_GO_LIVE: &str = "Your site is frozen, so it can't go live. Unfreeze it from your console, then approve again. This card keeps waiting.";

/// A card stopped waiting (approved, declined or expired): the WebAgent that raised it, and
/// the Chief, stop showing the amber dot for it (W1, handover-wiring 2.3).
fn presence_decided(card: &DecisionCard) {
    agentrix_kernel::presence::global().on_decision(
        card.site.as_str(),
        card.decision_id.as_str(),
        card.kind,
        false,
    );
}

fn first_line(e: &anyhow::Error) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("it didn't finish")
        .to_owned()
}

fn check_posted_fingerprint(posted: Option<&str>, card: &DecisionCard) -> Result<(), Fail> {
    let Some(posted) = posted else {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "This approval doesn't name the card it approves. Reload the card.",
        ));
    };
    let ours = card.fingerprint.to_string();
    if posted == ours || Some(posted) == ours.strip_prefix(Digest::PREFIX) {
        Ok(())
    } else {
        Err(fail(
            StatusCode::CONFLICT,
            "This card changed since you opened it. Reload it and approve again.",
        ))
    }
}

async fn approve(
    State(s): State<DecisionsState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.approve(&headers, &id, &body).await,
    }
}

async fn decline(
    State(s): State<DecisionsState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.decline(&headers, &id).await,
    }
}

async fn restore(
    State(s): State<DecisionsState>,
    Path((site, item)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.restore(&headers, &site, &item, &body).await,
    }
}

async fn rollback(
    State(s): State<DecisionsState>,
    Path(site): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    Reply {
        json: wants_json(&headers),
        outcome: s.rollback(&headers, &site, &body).await,
    }
}

/// Orders from the agent door, held as `order-held` cards (R9.2). Nothing is charged.
#[derive(Clone)]
pub struct HeldOrders {
    /// The move lane's tables.
    pub store: MoveStore,
}

/// How long the owner has to decide an order.
pub const ORDER_HOURS: i64 = 48;

fn held_of(record: &CardRecord) -> HeldCall {
    let card = &record.card;
    let capability = card.effect.args["capability"]
        .as_str()
        .unwrap_or("request_custom_order");
    let mut call = HeldCall::pending(card.decision_id.as_str(), capability, card.created_at);
    call.expires_at = Some(card.effect.expiry);
    call.state = match record.state {
        CardState::Waiting => HeldCallState::Pending,
        CardState::Approved => HeldCallState::Completed,
        CardState::Declined => HeldCallState::Rejected,
        CardState::Expired => HeldCallState::Expired,
    };
    if call.state == HeldCallState::Pending && Utc::now() >= card.effect.expiry {
        call.state = HeldCallState::Expired;
    }
    if call.state == HeldCallState::Completed {
        let reference =
            Digest::of(card.decision_id.as_str().as_bytes()).hex()[..6].to_ascii_uppercase();
        call.result =
            Some(json!({ "reference": { "kind": "order", "id": format!("K-{reference}") } }));
    }
    call
}

#[async_trait]
impl HeldCalls for HeldOrders {
    async fn raise(
        &self,
        site_id: &str,
        capability: &str,
        input: Value,
        idempotency_key: &str,
    ) -> anyhow::Result<HeldCall> {
        let site = SiteId::new(site_id);
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT decision_id FROM decisions WHERE site_id = ? AND kind = 'order-held' \
             AND json_extract(card, '$.effect.args.idempotency_key') = ? LIMIT 1",
        )
        .bind(site.as_str())
        .bind(idempotency_key)
        .fetch_optional(self.store.pool())
        .await?;
        if let Some((id,)) = existing
            && let Some(record) = self.store.card(&DecisionId::new(id)).await?
        {
            return Ok(held_of(&record));
        }
        let now = Utc::now();
        let text = |k: &str| {
            input
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let description: String = text("description").chars().take(80).collect();
        let effect = EffectRequest {
            site: site.clone(),
            webagent: "Concierge".into(),
            tool: format!("door.{capability}"),
            args: json!({ "capability": capability, "input": input, "idempotency_key": idempotency_key }),
            amount: None,
            expiry: now + Duration::hours(ORDER_HOURS),
        };
        let body = [
            format!("An AI customer's agent asked for: {}", text("description")),
            format!("For {}, from {} ({}).", text("date"), text("name"), text("email")),
            "If you approve, the agent is told the order is accepted with a reference. Nothing is charged.".to_owned(),
        ]
        .join("\n");
        let card = DecisionCard::new(
            DecisionId::generate(),
            CardKind::OrderHeld,
            format!("Order from an AI customer: {description}"),
            body,
            effect,
            now,
        )?;
        self.store.insert_card(&card).await?;
        // The agent desk shows the amber dot while the order waits on the owner (2.3).
        agentrix_kernel::presence::global().on_decision(
            site.as_str(),
            card.decision_id.as_str(),
            CardKind::OrderHeld,
            true,
        );
        let record = self
            .store
            .card(&card.decision_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the held order was not stored"))?;
        Ok(held_of(&record))
    }

    async fn status(&self, site_id: &str, id: &str) -> anyhow::Result<Option<HeldCall>> {
        let Some(record) = self.store.card(&DecisionId::new(id)).await? else {
            return Ok(None);
        };
        if record.card.site.as_str() != site_id || record.card.kind != CardKind::OrderHeld {
            return Ok(None);
        }
        Ok(Some(held_of(&record)))
    }
}

/// Starts a Move for a claimed site and runs it in the background; resumes it after a crash
/// via `Mover::resume_all` at startup (N9). For `app.rs` or `claim/` to call.
pub async fn start_move(
    mover: agentrix_move::Mover,
    site: &SiteId,
    inputs: &agentrix_move::MoveInputs,
    assessment_receipt: Option<Digest>,
    claim_receipt: Option<Digest>,
) -> anyhow::Result<MoveId> {
    let id = mover
        .start(site, inputs, assessment_receipt, claim_receipt)
        .await?;
    let run_id = id.clone();
    tokio::spawn(async move {
        if let Err(e) = mover.run(&run_id).await {
            tracing::warn!(move_id = %run_id, error = %format!("{e:#}"), "the Move stopped; it resumes on the next run");
        }
    });
    Ok(id)
}
