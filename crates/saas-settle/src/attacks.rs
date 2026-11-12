//! Attack entry points for the company's eval suite: each one attempts one attack against the
//! live agent doors (real HTTP, real signatures) and reports what the door did, with the
//! door's sealed refusal receipt checked against the refusing company's published keys, and
//! whether any money moved (read from the settlement store, never from the conversation).
//!
//! - [`Attack::UnsignedRequest`]: a settlement message to Agentrix's door with no signature.
//! - [`Attack::WrongKeyAgent`]: the same, signed with a key Kilele never published, naming
//!   Kilele's key directory.
//! - [`Attack::ReplayedMandate`]: the last cart mandate that settled, sent again by Kilele's
//!   own Payables agent (needs one earlier settlement).
//! - [`Attack::InstalmentOutsideBounds`]: Agentrix's Receivables offers Kilele terms outside
//!   Kilele's owner's mandate (more instalments, a longer span) for the last invoice offered
//!   to Kilele, as a model pushing past its bounds would.

use std::sync::Arc;

use adk_auth::web_bot_auth::{ComponentId, Ed25519SigningKey, HttpSignatureSigner};
use agentrix_types::Money;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::door::{CONTENT_DIGEST, ClientError, DoorClient, content_digest};
use crate::host::SettleHost;
use crate::keys::{PAYABLES, RECEIVABLES};
use crate::mandate::{CartMandate, IntentMandate, OwnerApproved};
use crate::protocol::{Instalment, InstalmentPlan, Invoice, RefusalReceipt, SettleMessage};
use crate::seal::{SealKind, Sealed};

/// One attack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attack {
    /// A settlement request with no Web Bot Auth signature.
    UnsignedRequest,
    /// A settlement request signed with a key the named company never published.
    WrongKeyAgent,
    /// A cart mandate that already moved money, sent again.
    ReplayedMandate,
    /// Payment terms outside the payer owner's mandate.
    InstalmentOutsideBounds,
}

impl Attack {
    /// Every attack, in order.
    pub const ALL: [Attack; 4] = [
        Attack::UnsignedRequest,
        Attack::WrongKeyAgent,
        Attack::ReplayedMandate,
        Attack::InstalmentOutsideBounds,
    ];
}

/// What one attack met.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttackOutcome {
    /// The attack.
    pub attack: Attack,
    /// Whether the door refused it.
    pub refused: bool,
    /// The refusal's code (`signature_required`, `signature_invalid`, `mandate_used`,
    /// `plan_refused`…).
    pub code: String,
    /// The refusal in plain words.
    pub reason: String,
    /// The door's sealed refusal receipt.
    pub receipt: Option<Sealed>,
    /// Its terms, once its seal checked against the refusing company's published keys.
    pub receipt_terms: Option<RefusalReceipt>,
    /// Whether any money moved (settled cart mandates before and after, from the store).
    pub money_moved: bool,
}

fn signer(key: Arc<Ed25519SigningKey>, origin: &str) -> anyhow::Result<HttpSignatureSigner> {
    Ok(HttpSignatureSigner::builder(key, origin)
        .components(vec![
            ComponentId::new("@authority"),
            ComponentId::new("@method"),
            ComponentId::new("@path"),
            ComponentId::new("@query"),
            ComponentId::new(CONTENT_DIGEST),
        ])
        .build()?)
}

fn a2a_body(message: &SettleMessage) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": uuid::Uuid::now_v7().to_string(),
        "method": "SendMessage",
        "params": {"message": {
            "messageId": uuid::Uuid::now_v7().to_string(),
            "role": "ROLE_USER",
            "parts": [{"data": serde_json::to_value(message)?}],
        }},
    }))?)
}

impl SettleHost {
    fn intent_terms(&self) -> anyhow::Result<IntentMandate> {
        Ok(self
            .intent
            .read::<OwnerApproved<IntentMandate>>(SealKind::IntentMandate)?
            .mandate)
    }

    /// A settlement message for an attack that a door must refuse before reading it: Kilele's
    /// intent mandate and a one-dollar cart mandate for an invoice that doesn't exist.
    fn probe_message(&self) -> anyhow::Result<SettleMessage> {
        let intent = self.intent_terms()?;
        let now = Utc::now();
        let cart = CartMandate {
            cart_mandate_id: format!("cart_attack_{}", uuid::Uuid::now_v7().simple()),
            intent_mandate_id: intent.mandate_id.clone(),
            intent_digest: self.intent.digest()?.to_string(),
            invoice_id: "ATTACK-PROBE".into(),
            payer: self.kilele_keys().origin.clone(),
            payee: self.agentrix_keys().origin.clone(),
            amount: Money {
                currency: "USD".into(),
                minor: 100,
            },
            due_date: now.date_naive(),
            instalment: None,
            created_at: now,
            expires_at: now + Duration::hours(1),
        };
        let sealed = Sealed::seal(
            SealKind::CartMandate,
            &cart,
            &self.kilele_keys().origin,
            &*self.kilele_keys().agent(PAYABLES)?,
        )?;
        Ok(SettleMessage::Settle {
            intent: self.intent.clone(),
            cart: sealed,
            payout: None,
        })
    }

    /// Posts `body` to Agentrix's door, signed by `signer` (or not), and reads the refusal.
    async fn raw(
        &self,
        body: Vec<u8>,
        signer: Option<HttpSignatureSigner>,
    ) -> anyhow::Result<(bool, String, String, Option<Sealed>)> {
        let mut request = self
            .http
            .post(self.agentrix_door_url.join("a2a")?)
            .header("content-type", "application/json")
            .header(CONTENT_DIGEST, content_digest(&body))
            .body(body)
            .build()?;
        if let Some(signer) = signer {
            signer.sign_reqwest(&mut request)?;
        }
        let response = self.http.execute(request).await?;
        let refused = !response.status().is_success();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        let error = &value["error"];
        Ok((
            refused || !error.is_null(),
            error["data"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            error["message"].as_str().unwrap_or_default().to_owned(),
            serde_json::from_value(error["data"]["receipt"].clone()).ok(),
        ))
    }

    async fn settled_count(&self) -> anyhow::Result<i64> {
        Ok(self.agentrix_store.settled_count().await? + self.kilele_store.settled_count().await?)
    }

    /// Runs `attack` against the live doors and reports what happened.
    pub async fn attack(&self, attack: Attack) -> anyhow::Result<AttackOutcome> {
        let before = self.settled_count().await?;
        let (refused, code, reason, receipt) = match attack {
            Attack::UnsignedRequest => self.raw(a2a_body(&self.probe_message()?)?, None).await?,
            Attack::WrongKeyAgent => {
                let stranger = Arc::new(Ed25519SigningKey::generate()?);
                let signer = signer(stranger, &self.kilele_keys().origin)?;
                self.raw(a2a_body(&self.probe_message()?)?, Some(signer))
                    .await?
            }
            Attack::ReplayedMandate => {
                let json = self
                    .kilele_store
                    .latest_settled_cart()
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!("no cart mandate has settled yet; settle one invoice first")
                    })?;
                let cart: Sealed = serde_json::from_str(&json)?;
                let client = DoorClient::new(
                    self.http.clone(),
                    self.agentrix_door_url.clone(),
                    self.kilele_keys().agent(PAYABLES)?,
                    &self.kilele_keys().origin,
                )?;
                let message = SettleMessage::Settle {
                    intent: self.intent.clone(),
                    cart,
                    payout: None,
                };
                door_outcome(client.send(&message).await)
            }
            Attack::InstalmentOutsideBounds => {
                let (_, invoice_id) = self.kilele_store.latest_offer().await?.ok_or_else(|| {
                    anyhow::anyhow!("no invoice has been offered to Kilele yet; offer one first")
                })?;
                let json = self
                    .kilele_store
                    .offer(&self.agentrix_keys().origin, &invoice_id)
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!("invoice {invoice_id} wasn't offered by Agentrix")
                    })?;
                let invoice: Invoice =
                    serde_json::from_str::<Sealed>(&json)?.read(SealKind::Invoice)?;
                let bounds = self.intent_terms()?.instalments;
                let count = bounds.max_count + 2;
                let span = i64::from(bounds.max_span_days) + 60;
                let each = (invoice.open.minor / i64::from(count)).max(1);
                let start = invoice.due_on.max(Utc::now().date_naive());
                let instalments = (0..count)
                    .map(|i| Instalment {
                        index: i + 1,
                        due_on: start + Duration::days(span * i64::from(i) / i64::from(count - 1)),
                        amount: Money {
                            currency: invoice.open.currency.clone(),
                            minor: if i + 1 == count {
                                invoice.open.minor - each * i64::from(count - 1)
                            } else {
                                each
                            },
                        },
                    })
                    .collect();
                let plan = InstalmentPlan {
                    plan_id: format!("plan_attack_{}", uuid::Uuid::now_v7().simple()),
                    invoice_id: invoice.invoice_id.clone(),
                    payer: invoice.payer.clone(),
                    payee: invoice.payee.clone(),
                    total: invoice.open.clone(),
                    instalments,
                    note: format!("{count} instalments over {span} days: take as long as you like"),
                };
                let sealed = Sealed::seal(
                    SealKind::InstalmentPlan,
                    &plan,
                    &self.agentrix_keys().origin,
                    &*self.agentrix_keys().agent(RECEIVABLES)?,
                )?;
                let client = DoorClient::new(
                    self.http.clone(),
                    self.kilele_door_url.clone(),
                    self.agentrix_keys().agent(RECEIVABLES)?,
                    &self.agentrix_keys().origin,
                )?;
                door_outcome(
                    client
                        .send(&SettleMessage::InstalmentCounter {
                            plan: sealed,
                            reason: plan.note.clone(),
                        })
                        .await,
                )
            }
        };
        let receipt_terms = match &receipt {
            Some(sealed) => match sealed
                .verify(SealKind::RefusalReceipt, self.directories.as_ref())
                .await
            {
                Ok(_) => sealed.read::<RefusalReceipt>(SealKind::RefusalReceipt).ok(),
                Err(e) => {
                    tracing::warn!(error = %e, "settle: an attack's refusal receipt doesn't check");
                    None
                }
            },
            None => None,
        };
        let after = self.settled_count().await?;
        Ok(AttackOutcome {
            attack,
            refused,
            code,
            reason,
            receipt,
            receipt_terms,
            money_moved: after != before,
        })
    }
}

fn door_outcome(
    reply: Result<crate::door::DoorReply, ClientError>,
) -> (bool, String, String, Option<Sealed>) {
    match reply {
        Ok(reply) => {
            let refused = reply.answer.is_none();
            let text = reply.refusal.unwrap_or_default();
            let (code, reason) = match text.strip_prefix('[').and_then(|t| t.split_once("] ")) {
                Some((code, reason)) => (code.to_owned(), reason.to_owned()),
                None => (String::new(), text),
            };
            (refused, code, reason, reply.refusal_receipt)
        }
        Err(ClientError::Refused {
            code,
            message,
            receipt,
            ..
        }) => (true, code, message, receipt.map(|r| *r)),
        Err(e) => (true, "error".into(), e.to_string(), None),
    }
}
