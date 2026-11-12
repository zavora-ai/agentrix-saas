//! The payer's side (Kilele Bakery): its door records invoices offered to it, and its
//! Payables agent settles them inside the intent mandate its owner approved, proposes
//! instalment plans when it is short, and posts every settlement from the payee's receipt.
//!
//! The Payables agent checks each invoice against its own owner's mandate before it signs a
//! cart mandate (the payee checks again before any money moves), and checks the payee's sealed
//! receipt before its books post: the receipt must be for exactly the cart mandate it signed.

use std::collections::HashMap;
use std::sync::Arc;

use adk_auth::web_bot_auth::KeyDirectorySource;
use agentrix_types::Money;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::Mutex;

use crate::books::{PayerBooks, PostedRef};
use crate::door::{DoorClient, DoorHandler, DoorReply, Refusal, VerifiedAgent};
use crate::events::{EventSender, SettleEvent};
use crate::keys::{CompanyKeys, PAYABLES};
use crate::mandate::{
    CartMandate, InstalmentRef, IntentMandate, MandateRefusal, SettlementFacts, check_settlement,
    month_of, verify_owner_sealed, words,
};
use crate::payee::{Clock, instalment_expiry};
use crate::protocol::{
    DoorAnswer, InstalmentAgreement, InstalmentPlan, Invoice, Route, SettleMessage,
    SettlementReceipt, check_plan_bounds, check_plan_expiry, check_plan_shape,
};
use crate::rails::{PayoutRail, RailError};
use crate::seal::{SealError, SealKind, Sealed};
use crate::store::{Claim, NewCart, Store};

/// Why the payer couldn't settle.
#[derive(Debug, thiserror::Error)]
pub enum PayError {
    /// Its own owner's mandate doesn't allow it.
    #[error(transparent)]
    Mandate(#[from] MandateRefusal),
    /// The payee refused or failed.
    #[error("{0}")]
    Refused(String),
    /// The payout (fallback route) didn't go through.
    #[error(transparent)]
    Rail(#[from] RailError),
    /// The payee's receipt doesn't check.
    #[error("The payee's receipt doesn't check: {0}")]
    Receipt(String),
    /// Anything else.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<SealError> for PayError {
    fn from(e: SealError) -> Self {
        Self::Receipt(e.to_string())
    }
}

/// How the payer is set up.
pub struct PayerConfig {
    /// The company's name, for example "Kilele Bakery".
    pub name: String,
    /// Its keys (the Payables agent's and the owner's).
    pub keys: CompanyKeys,
    /// Where counterparties' key directories are read from.
    pub directories: Arc<dyn KeyDirectorySource>,
    /// Settlement state.
    pub store: Store,
    /// The payer's books.
    pub books: Arc<dyn PayerBooks>,
    /// The owner's sealed intent mandate.
    pub intent: Sealed,
    /// Approval methods accepted on its own owner's mandate.
    pub owner_methods: Vec<String>,
    /// The payees' doors, by payee origin.
    pub payee_doors: HashMap<String, DoorClient>,
    /// How to settle.
    pub route: Route,
    /// The Payout rail (fallback route): the payer's own PayPal app.
    pub payout: Option<PayoutRail>,
    /// The payees' PayPal emails, by payee origin (Payout route).
    pub payee_emails: HashMap<String, String>,
    /// Cash the Payables agent may use now; an invoice over it asks for instalments.
    pub cash_available: Option<Money>,
    /// Now.
    pub clock: Clock,
}

/// The payer's desk: the handler behind its door, and its Payables agent's actions.
pub struct PayerDesk {
    config: PayerConfig,
    intent: IntentMandate,
    claim_lock: Mutex<()>,
    events: EventSender,
}

impl std::fmt::Debug for PayerDesk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayerDesk")
            .field("name", &self.config.name)
            .field("origin", &self.config.keys.origin)
            .finish_non_exhaustive()
    }
}

/// The result of settling one invoice.
#[derive(Debug, Clone, PartialEq)]
pub struct Settled {
    /// The payee's sealed receipt.
    pub receipt: Sealed,
    /// Its terms.
    pub terms: SettlementReceipt,
    /// What the payer's books posted.
    pub posted: PostedRef,
    /// The A2A task at the payee's door.
    pub task_id: String,
}

impl PayerDesk {
    /// The desk. Checks the owner's intent mandate up front.
    pub async fn new(config: PayerConfig) -> anyhow::Result<Arc<Self>> {
        let intent: IntentMandate = verify_owner_sealed(
            &config.intent,
            SealKind::IntentMandate,
            "intent mandate",
            config.directories.as_ref(),
            &config.owner_methods,
        )
        .await
        .map_err(|e| anyhow::anyhow!("the intent mandate doesn't check: {e}"))?;
        anyhow::ensure!(
            intent.payer == config.keys.origin,
            "the intent mandate is for {}, not {}",
            intent.payer,
            config.keys.origin
        );
        let payables = config.keys.agent(PAYABLES)?;
        anyhow::ensure!(
            intent.agent_keyid == payables.thumbprint(),
            "the intent mandate names another Payables key"
        );
        let recovered = config.store.recover_claims().await?;
        if recovered > 0 {
            tracing::warn!(
                recovered,
                "settle: payments left mid-way are now unknown; they settle by asking PayPal again with the same cart mandate"
            );
        }
        Ok(Arc::new(Self {
            config,
            intent,
            claim_lock: Mutex::new(()),
            events: crate::events::channel(),
        }))
    }

    /// Events for the payer's kernel: bills offered, receipts posted, plans agreed.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SettleEvent> {
        self.events.subscribe()
    }

    /// The channel this desk publishes on (its door publishes refusals on it too).
    pub fn events(&self) -> EventSender {
        self.events.clone()
    }

    fn emit(&self, event: SettleEvent) {
        // No one listening is fine: everything is in the store and the books either way.
        let _ = self.events.send(event);
    }

    /// The payer's origin.
    pub fn origin(&self) -> &str {
        &self.config.keys.origin
    }

    /// The owner's intent mandate's terms.
    pub fn intent(&self) -> &IntentMandate {
        &self.intent
    }

    /// The store.
    pub fn store(&self) -> &Store {
        &self.config.store
    }

    fn now(&self) -> DateTime<Utc> {
        (self.config.clock)()
    }

    fn door(&self, payee: &str) -> Result<&DoorClient, PayError> {
        self.config
            .payee_doors
            .get(payee)
            .ok_or_else(|| PayError::Other(anyhow::anyhow!("no door configured for {payee}")))
    }

    fn sign_cart(&self, cart: &CartMandate) -> Result<Sealed, PayError> {
        let key = self.config.keys.agent(PAYABLES)?;
        Ok(Sealed::seal(
            SealKind::CartMandate,
            cart,
            &self.config.keys.origin,
            &key,
        )?)
    }

    async fn offered(&self, payee: &str, invoice_id: &str) -> Result<Invoice, PayError> {
        let json = self
            .config
            .store
            .offer(payee, invoice_id)
            .await?
            .ok_or_else(|| {
                PayError::Other(anyhow::anyhow!(
                    "{payee} hasn't offered invoice {invoice_id}"
                ))
            })?;
        let sealed: Sealed = serde_json::from_str(&json).map_err(anyhow::Error::from)?;
        Ok(sealed.read(SealKind::Invoice)?)
    }

    /// What the Payables agent would do with `invoice` under the owner's mandate.
    fn decide(&self, invoice: &Invoice) -> (&'static str, String) {
        let i = &self.intent;
        if !i.payees.contains(&invoice.payee) {
            return (
                "held",
                format!(
                    "Our owner hasn't authorised payments to {}. Holding it for the owner.",
                    invoice.payee_name
                ),
            );
        }
        if invoice.total.minor > i.max_per_invoice.minor
            || !invoice
                .total
                .currency
                .eq_ignore_ascii_case(&i.max_per_invoice.currency)
        {
            return (
                "held",
                format!(
                    "{} is over our owner's per-invoice cap of {}. Holding it for the owner.",
                    words(&invoice.total),
                    words(&i.max_per_invoice)
                ),
            );
        }
        if self
            .config
            .cash_available
            .as_ref()
            .is_some_and(|cash| invoice.open.minor > cash.minor)
        {
            return (
                "instalments",
                format!(
                    "{} is inside our mandate, but we have {} available now. We'll propose instalments.",
                    words(&invoice.open),
                    self.config
                        .cash_available
                        .as_ref()
                        .map(words)
                        .unwrap_or_default()
                ),
            );
        }
        (
            "will_settle",
            format!(
                "{} is inside our owner's mandate. Our Payables agent will settle it on PayPal.",
                words(&invoice.open)
            ),
        )
    }

    async fn record_offer(
        &self,
        from: &VerifiedAgent,
        sealed: Sealed,
    ) -> Result<DoorAnswer, Refusal> {
        if sealed.signer != from.origin {
            return Err(Refusal::rejected(
                "invoice_refused",
                "The invoice was sealed by another company than the one sending it. Nothing was recorded.",
            ));
        }
        let key = sealed
            .verify(SealKind::Invoice, self.config.directories.as_ref())
            .await
            .map_err(|e| {
                Refusal::rejected("invoice_refused", format!("{e}. Nothing was recorded."))
            })?;
        if !key.kid.as_deref().is_some_and(|k| k.starts_with("agent:")) {
            return Err(Refusal::rejected(
                "invoice_refused",
                "Invoices come from the payee's agents. Nothing was recorded.",
            ));
        }
        let invoice: Invoice = sealed
            .read(SealKind::Invoice)
            .map_err(|e| Refusal::rejected("invoice_refused", e.to_string()))?;
        if invoice.payer != self.config.keys.origin || invoice.payee != from.origin {
            return Err(Refusal::rejected(
                "invoice_refused",
                "That invoice isn't between your company and ours. Nothing was recorded.",
            ));
        }
        let bill = self.config.books.record_bill(&invoice).await.map_err(|e| {
            Refusal::failed("books", format!("Our books didn't take the bill: {e}"))
        })?;
        let (decision, note) = self.decide(&invoice);
        let json = serde_json::to_string(&sealed)
            .map_err(|e| Refusal::failed("internal", e.to_string()))?;
        self.config
            .store
            .put_offer(&invoice.payee, &invoice.invoice_id, &json, decision)
            .await
            .map_err(|e| Refusal::failed("internal", e.to_string()))?;
        tracing::info!(invoice = %invoice.invoice_id, bill = %bill, decision, "settle: bill recorded");
        self.emit(SettleEvent::BillOffered {
            payer: invoice.payer.clone(),
            payee: invoice.payee.clone(),
            invoice_id: invoice.invoice_id.clone(),
            bill_id: bill,
            amount: invoice.open.clone(),
            due_on: invoice.due_on,
            decision: decision.into(),
            note: note.clone(),
        });
        Ok(DoorAnswer::InvoiceRecorded {
            invoice_id: invoice.invoice_id,
            decision: decision.into(),
            note,
        })
    }

    /// The Payables agent settles `invoice_id` from `payee`, in full: checks it against the
    /// owner's mandate, signs a cart mandate, moves the money (asks the payee to capture from
    /// the saved wallet, or sends the payout first), checks the receipt and posts it.
    pub async fn settle(&self, payee: &str, invoice_id: &str) -> Result<Settled, PayError> {
        self.settle_with_key(payee, invoice_id, None).await
    }

    /// [`PayerDesk::settle`], once per `key` (a governed call's idempotency key): the same key
    /// resends the same sealed cart mandate (the same PayPal request id) until it settles, and
    /// once settled returns the first receipt. Never a second payment.
    pub async fn settle_with_key(
        &self,
        payee: &str,
        invoice_id: &str,
        key: Option<&str>,
    ) -> Result<Settled, PayError> {
        let invoice = self.offered(payee, invoice_id).await?;
        let keyed_id = key.map(|key| {
            use sha2::Digest as _;
            let hash = sha2::Sha256::digest(
                format!(
                    "{}\n{payee}\n{}\n{key}",
                    self.intent.mandate_id, invoice.invoice_id
                )
                .as_bytes(),
            );
            format!("cart_k{}", &hex::encode(hash)[..32])
        });
        if let Some(id) = &keyed_id {
            if let Some(row) = self.config.store.cart(id).await? {
                if row.state == "settled" {
                    if let Some(json) = &row.receipt_json {
                        let receipt: Sealed =
                            serde_json::from_str(json).map_err(anyhow::Error::from)?;
                        let (terms, posted) = self.post_receipt(&receipt, None).await?;
                        return Ok(Settled {
                            receipt,
                            terms,
                            posted,
                            task_id: String::new(),
                        });
                    }
                }
                if let Some(json) = self.config.store.cart_json(id).await? {
                    let sealed: Sealed =
                        serde_json::from_str(&json).map_err(anyhow::Error::from)?;
                    let cart: CartMandate = sealed.read(SealKind::CartMandate)?;
                    return self.settle_sealed(&invoice, cart, sealed).await;
                }
            }
        }
        // Never pay what this agent already paid: on the Payout route the money leaves before
        // the payee checks anything.
        let paid = self
            .config
            .store
            .invoice_paid(&invoice.invoice_id, &invoice.open.currency)
            .await?;
        if paid.minor >= invoice.open.minor {
            return Err(PayError::Refused(format!(
                "Invoice {} is already settled ({} paid). Nothing was paid.",
                invoice.invoice_id,
                words(&paid)
            )));
        }
        let now = self.now();
        let cart = CartMandate {
            cart_mandate_id: keyed_id
                .unwrap_or_else(|| format!("cart_{}", uuid::Uuid::now_v7().simple())),
            intent_mandate_id: self.intent.mandate_id.clone(),
            intent_digest: self.config.intent.digest()?.to_string(),
            invoice_id: invoice.invoice_id.clone(),
            payer: self.config.keys.origin.clone(),
            payee: payee.to_owned(),
            amount: invoice.open.clone(),
            due_date: invoice.due_on,
            instalment: None,
            created_at: now,
            expires_at: now + Duration::days(7),
        };
        let sealed = self.sign_cart(&cart)?;
        self.settle_sealed(&invoice, cart, sealed).await
    }

    /// Settles with a sealed cart mandate (new, or the same one again on a retry).
    async fn settle_sealed(
        &self,
        invoice: &Invoice,
        cart: CartMandate,
        sealed_cart: Sealed,
    ) -> Result<Settled, PayError> {
        let now = self.now();
        let month = month_of(now);
        let cart_digest = sealed_cart.digest()?.to_string();
        {
            let _guard = self.claim_lock.lock().await;
            let month_paid = self
                .config
                .store
                .month_paid(
                    &self.intent.mandate_id,
                    &month,
                    &cart.amount.currency,
                    &cart.cart_mandate_id,
                )
                .await?;
            check_settlement(
                &self.intent,
                &cart.intent_digest,
                &cart,
                &SettlementFacts {
                    requester: None,
                    payee: &cart.payee,
                    invoice_id: &invoice.invoice_id,
                    amount: &invoice.open,
                    due_date: invoice.due_on,
                    invoice_total: &invoice.total,
                    month_paid: &month_paid,
                    now,
                },
            )?;
            if let Claim::Used(_) = self
                .config
                .store
                .claim_cart(&NewCart {
                    cart_mandate_id: &cart.cart_mandate_id,
                    cart_digest: &cart_digest,
                    intent_mandate_id: &self.intent.mandate_id,
                    invoice_id: &invoice.invoice_id,
                    payer: &self.config.keys.origin,
                    amount: &cart.amount,
                    month: &month,
                    route: match self.config.route {
                        Route::Vault => "vault",
                        Route::Payout => "payout",
                    },
                })
                .await?
            {
                return Err(MandateRefusal::AlreadyUsed.into());
            }
            let json = serde_json::to_string(&sealed_cart).map_err(anyhow::Error::from)?;
            self.config
                .store
                .set_cart_json(&cart.cart_mandate_id, &json)
                .await?;
        }
        let payout = match self.config.route {
            Route::Vault => None,
            Route::Payout => {
                let rail = self
                    .config
                    .payout
                    .as_ref()
                    .ok_or(RailError::NotConfigured)?;
                let email = self
                    .config
                    .payee_emails
                    .get(&cart.payee)
                    .ok_or_else(|| anyhow::anyhow!("no PayPal email for {}", cart.payee))?;
                match rail.pay(&cart, email).await {
                    Ok(evidence) => Some(evidence),
                    Err(e) => {
                        self.config
                            .store
                            .finish_cart(
                                &cart.cart_mandate_id,
                                "failed",
                                None,
                                None,
                                Some(&e.to_string()),
                            )
                            .await?;
                        return Err(e.into());
                    }
                }
            }
        };
        let reply = self
            .door(&cart.payee)?
            .send(&SettleMessage::Settle {
                intent: self.config.intent.clone(),
                cart: sealed_cart,
                payout,
            })
            .await
            .map_err(|e| PayError::Refused(e.to_string()));
        let reply = match reply {
            Ok(reply) => reply,
            Err(e) => {
                let state = if self.config.route == Route::Payout {
                    "unknown"
                } else {
                    "failed"
                };
                self.config
                    .store
                    .finish_cart(
                        &cart.cart_mandate_id,
                        state,
                        None,
                        None,
                        Some(&e.to_string()),
                    )
                    .await?;
                return Err(e);
            }
        };
        let DoorReply {
            task_id,
            answer,
            refusal,
            ..
        } = reply;
        let Some(DoorAnswer::Settled { receipt }) = answer else {
            let words = refusal.unwrap_or_else(|| "the payee gave no receipt".into());
            self.config
                .store
                .finish_cart(&cart.cart_mandate_id, "failed", None, None, Some(&words))
                .await?;
            return Err(PayError::Refused(words));
        };
        let (terms, posted) = self
            .post_receipt(&receipt, Some((&cart, &cart_digest)))
            .await?;
        Ok(Settled {
            receipt,
            terms,
            posted,
            task_id,
        })
    }

    /// Checks a payee's sealed receipt and posts it to the payer's books. `expected` is the
    /// cart mandate it must answer (when the payer asked for this settlement).
    pub async fn post_receipt(
        &self,
        receipt: &Sealed,
        expected: Option<(&CartMandate, &str)>,
    ) -> Result<(SettlementReceipt, PostedRef), PayError> {
        let key = receipt
            .verify(
                SealKind::SettlementReceipt,
                self.config.directories.as_ref(),
            )
            .await?;
        if !key.kid.as_deref().is_some_and(|k| k.starts_with("agent:")) {
            return Err(PayError::Receipt(
                "not sealed by one of the payee's agents".into(),
            ));
        }
        let terms: SettlementReceipt = receipt.read(SealKind::SettlementReceipt)?;
        if receipt.signer != terms.payee
            || terms.payer != self.config.keys.origin
            || terms.intent_mandate_id != self.intent.mandate_id
        {
            return Err(PayError::Receipt("it isn't for us, from that payee".into()));
        }
        if let Some((cart, digest)) = expected {
            if terms.cart_mandate_id != cart.cart_mandate_id
                || terms.cart_digest != digest
                || terms.amount != cart.amount
                || terms.invoice_id != cart.invoice_id
            {
                return Err(PayError::Receipt(
                    "it doesn't answer the cart mandate we signed".into(),
                ));
            }
        }
        let posted = self.config.books.post_paid(&terms).await?;
        let json = serde_json::to_string(receipt).map_err(anyhow::Error::from)?;
        // A receipt for a cart mandate this desk signed (one whose answer it lost, completed
        // later by PayPal's webhook) settles its record too.
        let ours = self
            .config
            .store
            .cart(&terms.cart_mandate_id)
            .await?
            .is_some_and(|row| row.state != "settled");
        if expected.is_some() || ours {
            self.config
                .store
                .finish_cart(
                    &terms.cart_mandate_id,
                    "settled",
                    Some(&terms.paypal.key()),
                    Some(&json),
                    None,
                )
                .await?;
        }
        tracing::info!(paypal = %posted.paypal_key, invoice = %terms.invoice_id, "settle: payable cleared in the payer's books");
        self.emit(SettleEvent::ReceiptPosted {
            receipt: terms.clone(),
        });
        Ok((terms, posted))
    }

    /// The Payables agent asks to pay `invoice_id` in instalments, as `plan` proposes. When
    /// the payee answers with a plan inside the owner's bounds (its acceptance or its
    /// counter), the agent accepts it with a cart mandate for each instalment.
    pub async fn propose_instalments(
        &self,
        payee: &str,
        invoice_id: &str,
        instalments: Vec<crate::protocol::Instalment>,
        note: &str,
    ) -> Result<(InstalmentPlan, Sealed, bool, String), PayError> {
        let invoice = self.offered(payee, invoice_id).await?;
        let plan = InstalmentPlan {
            plan_id: format!("proposal_{}", uuid::Uuid::now_v7().simple()),
            invoice_id: invoice.invoice_id.clone(),
            payer: self.config.keys.origin.clone(),
            payee: payee.to_owned(),
            total: invoice.open.clone(),
            instalments,
            note: note.to_owned(),
        };
        let reply = self
            .door(payee)?
            .send(&SettleMessage::InstalmentPropose {
                intent: self.config.intent.clone(),
                plan,
            })
            .await
            .map_err(|e| PayError::Refused(e.to_string()))?;
        let Some(DoorAnswer::InstalmentOffer {
            plan: sealed,
            countered,
            reason,
        }) = reply.answer
        else {
            return Err(PayError::Refused(
                reply
                    .refusal
                    .unwrap_or_else(|| "the payee offered no plan".into()),
            ));
        };
        sealed
            .verify(SealKind::InstalmentPlan, self.config.directories.as_ref())
            .await?;
        if sealed.signer != payee {
            return Err(PayError::Receipt(
                "the plan wasn't sealed by the payee".into(),
            ));
        }
        let offered: InstalmentPlan = sealed.read(SealKind::InstalmentPlan)?;
        Ok((offered, sealed, countered, reason))
    }

    /// Accepts the payee's sealed plan: checks it against the owner's mandate, signs a cart
    /// mandate per instalment (each valid from its due date until a grace period after), and
    /// sends them. Returns the payee's sealed agreement.
    pub async fn accept_plan(
        &self,
        payee: &str,
        sealed_plan: &Sealed,
    ) -> Result<(InstalmentAgreement, Sealed), PayError> {
        let plan: InstalmentPlan = sealed_plan.read(SealKind::InstalmentPlan)?;
        self.check_plan(payee, &plan).await?;
        let carts = self.plan_carts(payee, &plan)?;
        let reply = self
            .door(payee)?
            .send(&SettleMessage::InstalmentAccept {
                intent: self.config.intent.clone(),
                plan: sealed_plan.clone(),
                carts,
            })
            .await
            .map_err(|e| PayError::Refused(e.to_string()))?;
        let Some(DoorAnswer::InstalmentAgreed { agreement }) = reply.answer else {
            return Err(PayError::Refused(
                reply
                    .refusal
                    .unwrap_or_else(|| "the payee didn't agree".into()),
            ));
        };
        agreement
            .verify(SealKind::InstalmentPlan, self.config.directories.as_ref())
            .await?;
        let terms: InstalmentAgreement = agreement.read(SealKind::InstalmentPlan)?;
        if terms.plan != *sealed_plan {
            return Err(PayError::Receipt(
                "the agreement is for another plan".into(),
            ));
        }
        self.emit(SettleEvent::PlanAgreed {
            plan_id: plan.plan_id.clone(),
            invoice_id: plan.invoice_id.clone(),
            books_invoice_id: None,
            payer: plan.payer.clone(),
            payee: plan.payee.clone(),
            instalments: plan.instalments.clone(),
        });
        Ok((terms, agreement))
    }

    /// Checks a plan against the owner's mandate and the offered invoice.
    async fn check_plan(&self, payee: &str, plan: &InstalmentPlan) -> Result<(), PayError> {
        let invoice = self.offered(payee, &plan.invoice_id).await?;
        let today = self.now().date_naive();
        let bad = |e: crate::protocol::PlanRefusal| PayError::Refused(e.to_string());
        check_plan_shape(plan, &invoice, today).map_err(bad)?;
        check_plan_bounds(plan, &invoice, &self.intent.instalments, "our owner's").map_err(bad)?;
        check_plan_expiry(
            plan,
            self.intent.intent_expiry,
            "our owner's intent mandate",
        )
        .map_err(bad)
    }

    /// One cart mandate per instalment, each valid from its due date until a grace period
    /// after, signed by the Payables agent.
    fn plan_carts(&self, payee: &str, plan: &InstalmentPlan) -> Result<Vec<Sealed>, PayError> {
        let digest = self.config.intent.digest()?.to_string();
        let count = plan.instalments.len() as u32;
        let mut carts = Vec::new();
        for instalment in &plan.instalments {
            let cart = CartMandate {
                cart_mandate_id: format!("cart_{}", uuid::Uuid::now_v7().simple()),
                intent_mandate_id: self.intent.mandate_id.clone(),
                intent_digest: digest.clone(),
                invoice_id: plan.invoice_id.clone(),
                payer: self.config.keys.origin.clone(),
                payee: payee.to_owned(),
                amount: instalment.amount.clone(),
                due_date: instalment.due_on,
                instalment: Some(InstalmentRef {
                    plan_id: plan.plan_id.clone(),
                    index: instalment.index,
                    count,
                }),
                created_at: self.now(),
                expires_at: instalment_expiry(instalment.due_on),
            };
            carts.push(self.sign_cart(&cart)?);
        }
        Ok(carts)
    }

    /// The payee offered terms (its Receivables agent proposed a plan): the Payables agent
    /// accepts them when they are inside the owner's mandate, answering with a cart mandate
    /// for each instalment.
    async fn answer_terms(
        &self,
        from: &VerifiedAgent,
        sealed_plan: Sealed,
    ) -> Result<DoorAnswer, Refusal> {
        let refused =
            |e: String| Refusal::rejected("plan_refused", format!("{e} Nothing was agreed."));
        if sealed_plan.signer != from.origin {
            return Err(refused(
                "The plan was sealed by another company than the one sending it.".into(),
            ));
        }
        let key = sealed_plan
            .verify(SealKind::InstalmentPlan, self.config.directories.as_ref())
            .await
            .map_err(|e| refused(e.to_string()))?;
        if !key.kid.as_deref().is_some_and(|k| k.starts_with("agent:")) {
            return Err(refused("Plans come from the payee's agents.".into()));
        }
        let plan: InstalmentPlan = sealed_plan
            .read(SealKind::InstalmentPlan)
            .map_err(|e| refused(e.to_string()))?;
        if plan.payer != self.config.keys.origin || plan.payee != from.origin {
            return Err(refused(
                "That plan isn't between your company and ours.".into(),
            ));
        }
        self.check_plan(&from.origin, &plan)
            .await
            .map_err(|e| refused(e.to_string()))?;
        let carts = self
            .plan_carts(&from.origin, &plan)
            .map_err(|e| Refusal::failed("internal", e.to_string()))?;
        tracing::info!(plan = %plan.plan_id, invoice = %plan.invoice_id, "settle: payee's terms accepted inside the owner's mandate");
        self.emit(SettleEvent::PlanAgreed {
            plan_id: plan.plan_id.clone(),
            invoice_id: plan.invoice_id.clone(),
            books_invoice_id: None,
            payer: plan.payer.clone(),
            payee: plan.payee.clone(),
            instalments: plan.instalments.clone(),
        });
        Ok(DoorAnswer::InstalmentAccepted {
            intent: self.config.intent.clone(),
            plan: sealed_plan,
            carts,
        })
    }
}

#[async_trait]
impl DoorHandler for PayerDesk {
    async fn handle(
        &self,
        from: &VerifiedAgent,
        message: SettleMessage,
    ) -> Result<DoorAnswer, Refusal> {
        match message {
            SettleMessage::InvoiceOffer { invoice } => self.record_offer(from, invoice).await,
            SettleMessage::InstalmentCounter { plan, .. } => self.answer_terms(from, plan).await,
            SettleMessage::SettlementReceipt { receipt } => {
                if receipt.signer != from.origin {
                    return Err(Refusal::rejected(
                        "receipt_refused",
                        "The receipt was sealed by another company than the one sending it.",
                    ));
                }
                match self.post_receipt(&receipt, None).await {
                    Ok((terms, posted)) => Ok(DoorAnswer::ReceiptPosted {
                        receipt_id: terms.receipt_id,
                        paypal_key: posted.paypal_key,
                        already_posted: posted.already_posted,
                    }),
                    Err(PayError::Receipt(why)) => Err(Refusal::rejected("receipt_refused", why)),
                    Err(e) => Err(Refusal::failed("books", e.to_string())),
                }
            }
            other => Err(Refusal::rejected(
                "wrong_door",
                format!(
                    "This is {}'s door for bills; it doesn't take {} messages.",
                    self.config.name,
                    other.kind()
                ),
            )),
        }
    }
}
