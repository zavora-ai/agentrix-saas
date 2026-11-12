//! The payee's side (Agentrix): its door settles invoices under the payer's mandates, agrees
//! instalment plans inside both owners' mandates, and collects each instalment on its due date.
//!
//! Before any money moves, every settlement is checked in this order, all deterministic:
//! the payer owner's intent mandate (its seal by the payer's `owner` key, an approval of its
//! exact terms), the Payables agent's cart mandate (its seal by the key the intent names), that
//! the request came from that same agent, the invoice as Agentrix's books hold it, the caps and
//! dates, and that the cart mandate was never used. Then PayPal moves the money once, Agentrix's
//! books post it, and the receipt is sealed by Agentrix's Receivables agent.

use std::collections::HashMap;
use std::sync::Arc;

use adk_auth::web_bot_auth::KeyDirectorySource;
use agentrix_types::Money;
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::json;
use tokio::sync::Mutex;

use crate::books::PayeeBooks;
use crate::door::{DoorClient, DoorHandler, Refusal, VerifiedAgent};
use crate::events::{EventSender, SettleEvent};
use crate::keys::{CompanyKeys, RECEIVABLES};
use crate::mandate::{
    CartMandate, CollectionsMandate, IntentMandate, MandateRefusal, SettlementFacts,
    check_settlement, month_of, verify_cart, verify_owner_sealed, words,
};
use crate::protocol::{
    DoorAnswer, InstalmentAgreement, InstalmentPlan, Invoice, PayPalSettlement, PayoutEvidence,
    PlanRefusal, Route, SettleMessage, SettlementReceipt, bounds_within, check_plan_bounds,
    check_plan_expiry, check_plan_shape, fit_plan,
};
use crate::rails::{RailError, VaultRail};
use crate::seal::{SealKind, Sealed};
use crate::store::{Claim, NewCart, PlanRow, ScheduledRow, Store, VaultRow};

/// Proposes a counter-plan when a payer's proposal is outside the bounds. A model may draft it
/// (Claude negotiates the words and the shape); whatever it drafts is checked against both
/// owners' mandates before it is sent, and again when the payer accepts.
#[async_trait]
pub trait Negotiator: Send + Sync {
    /// A counter-plan and the reason for it, or `None` to refuse outright.
    async fn counter(
        &self,
        proposal: &InstalmentPlan,
        invoice: &Invoice,
        payer: &crate::mandate::InstalmentBounds,
        payee: &crate::mandate::InstalmentBounds,
        today: NaiveDate,
    ) -> Option<(InstalmentPlan, String)>;
}

/// The scripted negotiator: the nearest plan inside both owners' bounds ([`fit_plan`]).
#[derive(Debug, Default, Clone, Copy)]
pub struct BoundsNegotiator;

#[async_trait]
impl Negotiator for BoundsNegotiator {
    async fn counter(
        &self,
        proposal: &InstalmentPlan,
        invoice: &Invoice,
        payer: &crate::mandate::InstalmentBounds,
        payee: &crate::mandate::InstalmentBounds,
        today: NaiveDate,
    ) -> Option<(InstalmentPlan, String)> {
        let plan = fit_plan(
            proposal,
            invoice,
            payer,
            payee,
            today,
            format!("plan_{}", uuid::Uuid::now_v7().simple()),
        )?;
        let reason = format!(
            "We can't take that plan as proposed. We can do {}.",
            plan.note
        );
        Some((plan, reason))
    }
}

/// What a PayPal capture webhook did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CaptureBooked {
    /// PayPal's capture id.
    pub capture_id: String,
    /// The cart mandate it paid.
    pub cart_mandate_id: String,
    /// True when the webhook completed the settlement (its answer had been lost or pending);
    /// false when it was already settled and only posted again (idempotently).
    pub completed_now: bool,
}

/// A clock, so tests can collect an instalment on its due date.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// How the payee is set up.
pub struct PayeeConfig {
    /// The company's name, for example "Agentrix".
    pub name: String,
    /// Its keys (the Receivables agent's and the owner's).
    pub keys: CompanyKeys,
    /// Where counterparties' key directories are read from.
    pub directories: Arc<dyn KeyDirectorySource>,
    /// Settlement state.
    pub store: Store,
    /// The payee's books.
    pub books: Arc<dyn PayeeBooks>,
    /// The Vault rail over the payee's PayPal app, when Vault settlement is on.
    pub vault: Option<VaultRail>,
    /// The payee's PayPal email: what a payout to it must name (Payout route).
    pub paypal_email: Option<String>,
    /// The payee owner's sealed collections mandate.
    pub collections: Sealed,
    /// Approval methods accepted on mandates (counterparties' and its own).
    pub owner_methods: Vec<String>,
    /// Drafts counter-plans.
    pub negotiator: Arc<dyn Negotiator>,
    /// The payer doors to send collection receipts to, by payer origin.
    pub payer_doors: HashMap<String, DoorClient>,
    /// Now.
    pub clock: Clock,
}

/// The payee's desk: the handler behind its door, and the collector of scheduled instalments.
pub struct PayeeDesk {
    config: PayeeConfig,
    collections: CollectionsMandate,
    claim_lock: Mutex<()>,
    events: EventSender,
}

impl std::fmt::Debug for PayeeDesk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayeeDesk")
            .field("name", &self.config.name)
            .field("origin", &self.config.keys.origin)
            .finish_non_exhaustive()
    }
}

fn mandate_refusal(e: MandateRefusal) -> Refusal {
    let code = match &e {
        MandateRefusal::AlreadyUsed => "mandate_used",
        MandateRefusal::OverCap { .. } => "over_mandate",
        MandateRefusal::NotDue(_) => "not_due",
        _ => "mandate_refused",
    };
    Refusal::rejected(code, e.to_string())
}

fn plan_refusal(e: PlanRefusal) -> Refusal {
    Refusal::rejected("plan_refused", e.to_string())
}

fn internal(e: impl std::fmt::Display) -> Refusal {
    Refusal::failed(
        "internal",
        format!("Something went wrong on our side ({e}). Nothing was paid."),
    )
}

/// The PayPal invoice id for a settlement: the invoice (and instalment) and the cart mandate,
/// unique per payee PayPal account even when demo books are re-seeded and numbers repeat. Our
/// own claim on the cart mandate, and the invoice's open amount, stop a second payment.
pub fn paypal_invoice_id(cart: &CartMandate) -> String {
    let id = match &cart.instalment {
        Some(i) => format!(
            "agentrix-saas/{}/{}-of-{}/{}",
            cart.invoice_id, i.index, i.count, cart.cart_mandate_id
        ),
        None => format!("agentrix-saas/{}/{}", cart.invoice_id, cart.cart_mandate_id),
    };
    id.chars().take(127).collect()
}

impl PayeeDesk {
    /// The desk. Checks the payee's own collections mandate up front.
    pub async fn new(config: PayeeConfig) -> anyhow::Result<Arc<Self>> {
        let collections: CollectionsMandate = verify_owner_sealed(
            &config.collections,
            SealKind::CollectionsMandate,
            "collections mandate",
            config.directories.as_ref(),
            &config.owner_methods,
        )
        .await
        .map_err(|e| anyhow::anyhow!("the collections mandate doesn't check: {e}"))?;
        anyhow::ensure!(
            collections.payee == config.keys.origin,
            "the collections mandate is for {}, not {}",
            collections.payee,
            config.keys.origin
        );
        config.keys.agent(RECEIVABLES)?;
        let recovered = config.store.recover_claims().await?;
        if recovered > 0 {
            tracing::warn!(
                recovered,
                "settle: payments left mid-way are now unknown; they settle by asking PayPal again with the same cart mandate"
            );
        }
        Ok(Arc::new(Self {
            config,
            collections,
            claim_lock: Mutex::new(()),
            events: crate::events::channel(),
        }))
    }

    /// Events for the payee's kernel: settlements, and agreed plans whose due dates it puts on
    /// its wake list.
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

    /// The payee's name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// The payee's origin.
    pub fn origin(&self) -> &str {
        &self.config.keys.origin
    }

    /// The store.
    pub fn store(&self) -> &Store {
        &self.config.store
    }

    fn now(&self) -> DateTime<Utc> {
        (self.config.clock)()
    }

    fn seal<T: serde::Serialize>(&self, kind: SealKind, body: &T) -> Result<Sealed, Refusal> {
        let key = self.config.keys.agent(RECEIVABLES).map_err(internal)?;
        Sealed::seal(kind, body, &self.config.keys.origin, &key).map_err(internal)
    }

    /// Seals an invoice from the payee's books for offering to its payer.
    pub async fn offer(&self, invoice_id: &str) -> anyhow::Result<(Invoice, Sealed)> {
        let invoice = self
            .config
            .books
            .invoice(invoice_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no invoice {invoice_id} in the books"))?;
        anyhow::ensure!(
            invoice.payee == self.config.keys.origin,
            "invoice {invoice_id} isn't ours to offer"
        );
        let sealed = self
            .seal(SealKind::Invoice, &invoice)
            .map_err(|r| anyhow::anyhow!(r.message))?;
        Ok((invoice, sealed))
    }

    /// The Receivables agent offers `invoice_id` to its payer's door, signed, and returns the
    /// payer's answer.
    pub async fn send_offer(&self, invoice_id: &str) -> anyhow::Result<crate::door::DoorReply> {
        let (invoice, sealed) = self.offer(invoice_id).await?;
        let door = self
            .config
            .payer_doors
            .get(&invoice.payer)
            .ok_or_else(|| anyhow::anyhow!("no door configured for {}", invoice.payer))?;
        Ok(door
            .send(&SettleMessage::InvoiceOffer { invoice: sealed })
            .await?)
    }

    /// Starts saving `payer`'s PayPal wallet: the approve link its owner opens once, signed in
    /// to PayPal as the payer.
    pub async fn start_vault(
        &self,
        payer: &str,
        payer_name: &str,
        return_url: &str,
        cancel_url: &str,
    ) -> anyhow::Result<VaultRow> {
        let vault = self
            .config
            .vault
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Vault settlement is off"))?;
        if let Some(row) = self.config.store.vault(payer).await? {
            if row.state == "saved" || row.setup_token.is_some() {
                return Ok(row);
            }
        }
        let setup = vault
            .start_setup(
                &self.config.name,
                &format!(
                    "{} settles {}'s invoices inside its owner's mandate",
                    payer_name, self.config.name
                ),
                return_url,
                cancel_url,
                &format!(
                    "agentrix-saas-vault-setup-{}",
                    hex::encode(payer.as_bytes())
                ),
            )
            .await?;
        let row = VaultRow {
            payer: payer.to_owned(),
            vault_id: None,
            customer_id: setup.customer_id,
            setup_token: Some(setup.setup_token),
            approve_url: setup.approve_url,
            state: "pending".into(),
        };
        self.config.store.put_vault(&row).await?;
        Ok(row)
    }

    /// Finishes saving `payer`'s wallet once its owner approved; idempotent.
    pub async fn finish_vault(&self, payer: &str) -> anyhow::Result<VaultRow> {
        let vault = self
            .config
            .vault
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Vault settlement is off"))?;
        let mut row = self
            .config
            .store
            .vault(payer)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no wallet setup started for {payer}"))?;
        if row.state == "saved" {
            return Ok(row);
        }
        let token = row
            .setup_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no setup token for {payer}"))?;
        let setup = vault.setup(&token).await?;
        anyhow::ensure!(
            matches!(setup.status.as_str(), "APPROVED" | "VAULTED"),
            "the payer hasn't approved yet (PayPal says {})",
            setup.status
        );
        let wallet = vault.complete_setup(&token).await?;
        row.vault_id = Some(wallet.vault_id);
        row.customer_id = wallet.customer_id.or(row.customer_id);
        row.state = "saved".into();
        self.config.store.put_vault(&row).await?;
        Ok(row)
    }

    /// Records a wallet saved elsewhere (a `vault_id` from an earlier run).
    pub async fn use_vault(&self, payer: &str, vault_id: &str) -> anyhow::Result<()> {
        self.config
            .store
            .put_vault(&VaultRow {
                payer: payer.to_owned(),
                vault_id: Some(vault_id.to_owned()),
                customer_id: None,
                setup_token: None,
                approve_url: None,
                state: "saved".into(),
            })
            .await
    }

    async fn intent(
        &self,
        sealed: &Sealed,
        from: Option<&VerifiedAgent>,
    ) -> Result<IntentMandate, Refusal> {
        let intent: IntentMandate = verify_owner_sealed(
            sealed,
            SealKind::IntentMandate,
            "intent mandate",
            self.config.directories.as_ref(),
            &self.config.owner_methods,
        )
        .await
        .map_err(mandate_refusal)?;
        if intent.payer != sealed.signer {
            return Err(mandate_refusal(MandateRefusal::Parties(format!(
                "The intent mandate was sealed at {}, but names {} as the payer",
                sealed.signer, intent.payer
            ))));
        }
        if let Some(from) = from {
            if from.origin != intent.payer || from.key_thumbprint != intent.agent_keyid {
                return Err(mandate_refusal(MandateRefusal::WrongAgent {
                    agent: from.id.clone(),
                }));
            }
        }
        Ok(intent)
    }

    async fn open_invoice(&self, invoice_id: &str, payer: &str) -> Result<Invoice, Refusal> {
        let invoice = self
            .config
            .books
            .invoice(invoice_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                Refusal::rejected(
                    "no_such_invoice",
                    format!("There's no invoice {invoice_id} in our books. Nothing was paid."),
                )
            })?;
        if invoice.payer != payer {
            return Err(Refusal::rejected(
                "no_such_invoice",
                format!("Invoice {invoice_id} isn't addressed to {payer}. Nothing was paid."),
            ));
        }
        Ok(invoice)
    }

    /// Settles under `intent` and `cart`: every check, the claim, the money, the books, the
    /// receipt. `from` is the signed requester (`None` for a scheduled collection).
    async fn settle(
        &self,
        from: Option<&VerifiedAgent>,
        intent_sealed: &Sealed,
        cart_sealed: &Sealed,
        payout: Option<&PayoutEvidence>,
        scheduled: Option<&ScheduledRow>,
    ) -> Result<Sealed, Refusal> {
        let intent = self.intent(intent_sealed, from).await?;
        let intent_digest = intent_sealed.digest().map_err(internal)?.to_string();
        let cart = verify_cart(cart_sealed, &intent, self.config.directories.as_ref())
            .await
            .map_err(mandate_refusal)?;
        let cart_digest = cart_sealed.digest().map_err(internal)?.to_string();
        if self
            .config
            .store
            .used(&cart.cart_mandate_id, &cart_digest)
            .await
            .map_err(internal)?
        {
            return Err(mandate_refusal(MandateRefusal::AlreadyUsed));
        }
        let mut invoice = self.open_invoice(&cart.invoice_id, &intent.payer).await?;
        // A payment whose outcome PayPal hasn't confirmed blocks any other on the invoice: only
        // the same cart mandate (the same PayPal request id) may settle it, never a second one.
        if let Some(pending) = self
            .config
            .store
            .pending_for_invoice(&invoice.invoice_id)
            .await
            .map_err(internal)?
        {
            if pending != cart.cart_mandate_id {
                return Err(Refusal::rejected(
                    "payment_pending",
                    format!(
                        "A payment on invoice {} (cart mandate {pending}) is still being confirmed with PayPal. Ask again with that cart mandate. Nothing was paid.",
                        invoice.invoice_id
                    ),
                ));
            }
        }
        // What we settled counts even if the books haven't posted it yet.
        let settled = self
            .config
            .store
            .invoice_paid(&invoice.invoice_id, &invoice.open.currency)
            .await
            .map_err(internal)?;
        let paid_in_books = invoice.total.minor - invoice.open.minor;
        if settled.minor > paid_in_books {
            invoice.open.minor = (invoice.total.minor - settled.minor).max(0);
        }
        let (amount, due_date) = match (&cart.instalment, scheduled) {
            (None, None) => (invoice.open.clone(), invoice.due_on),
            (Some(i), Some(row)) if i.plan_id == row.plan_id && i.index == row.index => {
                let due = row.due_on.parse::<NaiveDate>().map_err(internal)?;
                (row.amount.clone(), due)
            }
            _ => {
                return Err(Refusal::rejected(
                    "instalment_not_scheduled",
                    "Instalments are collected on their due dates under the agreed plan, not settled on request. Nothing was paid.",
                ));
            }
        };
        if amount.minor > invoice.open.minor {
            return Err(Refusal::rejected(
                "over_open",
                format!(
                    "Only {} is open on invoice {}. Nothing was paid.",
                    words(&invoice.open),
                    invoice.invoice_id
                ),
            ));
        }
        let now = self.now();
        let month = month_of(now);
        let route = if payout.is_some() {
            Route::Payout
        } else {
            Route::Vault
        };
        let guard = self.claim_lock.lock().await;
        let month_paid = self
            .config
            .store
            .month_paid(
                &intent.mandate_id,
                &month,
                &amount.currency,
                &cart.cart_mandate_id,
            )
            .await
            .map_err(internal)?;
        check_settlement(
            &intent,
            &intent_digest,
            &cart,
            &SettlementFacts {
                requester: from.map(|f| (f.origin.as_str(), f.key_thumbprint.as_str())),
                payee: &self.config.keys.origin,
                invoice_id: &invoice.invoice_id,
                amount: &amount,
                due_date,
                invoice_total: &invoice.total,
                month_paid: &month_paid,
                now,
            },
        )
        .map_err(mandate_refusal)?;
        let claim = self
            .config
            .store
            .claim_cart(&NewCart {
                cart_mandate_id: &cart.cart_mandate_id,
                cart_digest: &cart_digest,
                intent_mandate_id: &intent.mandate_id,
                invoice_id: &invoice.invoice_id,
                payer: &intent.payer,
                amount: &amount,
                month: &month,
                route: match route {
                    Route::Vault => "vault",
                    Route::Payout => "payout",
                },
            })
            .await
            .map_err(internal)?;
        drop(guard);
        if let Claim::Used(_) = claim {
            return Err(mandate_refusal(MandateRefusal::AlreadyUsed));
        }
        // Kept as received, so PayPal's webhook can finish a capture whose answer we lost.
        let cart_json = serde_json::to_string(cart_sealed).map_err(internal)?;
        self.config
            .store
            .set_cart_json(&cart.cart_mandate_id, &cart_json)
            .await
            .map_err(internal)?;
        tracing::info!(cart = %cart.cart_mandate_id, invoice = %invoice.invoice_id, amount = %words(&amount), ?route, "settle: mandates checked, moving money");
        let paypal = match self.move_money(&intent, &cart, &cart_digest, payout).await {
            Ok(paypal) => paypal,
            Err(e) => {
                let state = if matches!(e, RailError::Unknown(_)) {
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
                    .await
                    .map_err(internal)?;
                return Err(Refusal::failed("paypal", e.to_string()));
            }
        };
        let open_after = Money {
            currency: invoice.open.currency.clone(),
            minor: invoice.open.minor - amount.minor,
        };
        self.complete(&cart, cart_digest, &amount, route, paypal, open_after, now)
            .await
    }

    /// The money moved: seal the receipt, record it, post the payee's books, tell the kernel.
    #[allow(clippy::too_many_arguments)]
    async fn complete(
        &self,
        cart: &CartMandate,
        cart_digest: String,
        amount: &Money,
        route: Route,
        paypal: PayPalSettlement,
        open_after: Money,
        now: DateTime<Utc>,
    ) -> Result<Sealed, Refusal> {
        let receipt = SettlementReceipt {
            receipt_id: format!("sr_{}", uuid::Uuid::now_v7().simple()),
            invoice_id: cart.invoice_id.clone(),
            payer: cart.payer.clone(),
            payee: self.config.keys.origin.clone(),
            amount: amount.clone(),
            route,
            paypal,
            cart_mandate_id: cart.cart_mandate_id.clone(),
            cart_digest,
            intent_mandate_id: cart.intent_mandate_id.clone(),
            intent_digest: cart.intent_digest.clone(),
            instalment: cart.instalment.clone(),
            invoice_open_after: open_after,
            settled_at: now,
        };
        let sealed = self.seal(SealKind::SettlementReceipt, &receipt)?;
        let sealed_json = serde_json::to_string(&sealed).map_err(internal)?;
        let key = receipt.paypal.key();
        // The money moved: record that first, then post. A failed post is retried from here.
        self.config
            .store
            .finish_cart(
                &cart.cart_mandate_id,
                "settled",
                Some(&key),
                Some(&sealed_json),
                None,
            )
            .await
            .map_err(internal)?;
        match self.config.books.post_received(&receipt).await {
            Ok(posted) => {
                tracing::info!(paypal = %posted.paypal_key, payment = ?posted.payment_id, "settle: receivable cleared in the payee's books");
            }
            Err(e) => {
                tracing::warn!(error = %e, paypal = %key, "settle: books post failed; retry with repost");
                self.config
                    .store
                    .finish_cart(
                        &cart.cart_mandate_id,
                        "settled",
                        None,
                        None,
                        Some(&format!("books: {e}")),
                    )
                    .await
                    .map_err(internal)?;
            }
        }
        self.emit(SettleEvent::Settled { receipt });
        Ok(sealed)
    }

    /// PayPal's `PAYMENT.CAPTURE.COMPLETED` (verified by the node's webhook) for a capture this
    /// desk made: books it once. A capture whose answer was lost or still PENDING completes
    /// here (receipt, the payee's books, the payer's books through its door, the instalment's
    /// schedule); one already settled is posted again, idempotently. A capture that isn't one
    /// of this desk's cart mandates is `None`.
    pub async fn complete_capture(
        &self,
        resource: &serde_json::Value,
    ) -> anyhow::Result<Option<CaptureBooked>> {
        let Some(cart_id) = resource["custom_id"]
            .as_str()
            .filter(|c| c.starts_with("cart_"))
        else {
            return Ok(None);
        };
        let Some(row) = self.config.store.cart(cart_id).await? else {
            return Ok(None);
        };
        let capture_id = resource["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("the capture has no id"))?
            .to_owned();
        if row.state == "settled" {
            self.repost(cart_id).await?;
            return Ok(Some(CaptureBooked {
                capture_id,
                cart_mandate_id: cart_id.to_owned(),
                completed_now: false,
            }));
        }
        let json = self
            .config
            .store
            .cart_json(cart_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("cart mandate {cart_id} wasn't kept"))?;
        let sealed: Sealed = serde_json::from_str(&json)?;
        let cart: CartMandate = sealed.read(SealKind::CartMandate)?;
        let captured = agentrix_effects::paypal::minor_of(&resource["amount"])
            .map(|(currency, minor)| Money { currency, minor });
        anyhow::ensure!(
            captured.as_ref() == Some(&cart.amount),
            "capture {capture_id} is for {captured:?}, not the mandated {}",
            words(&cart.amount)
        );
        let fee = agentrix_effects::paypal::minor_of(
            &resource["seller_receivable_breakdown"]["paypal_fee"],
        )
        .map(|(currency, minor)| Money { currency, minor });
        let paypal = PayPalSettlement {
            kind: "capture".into(),
            id: capture_id.clone(),
            parent_id: resource["supplementary_data"]["related_ids"]["order_id"]
                .as_str()
                .map(str::to_owned),
            transaction_id: Some(capture_id.clone()),
            status: "COMPLETED".into(),
            fee,
            sandbox: self.config.vault.as_ref().is_none_or(VaultRail::sandbox),
        };
        let invoice = self
            .open_invoice(&cart.invoice_id, &cart.payer)
            .await
            .map_err(|r| anyhow::anyhow!(r.message))?;
        let open_after = Money {
            currency: invoice.open.currency.clone(),
            minor: (invoice.open.minor - cart.amount.minor).max(0),
        };
        let digest = sealed.digest()?.to_string();
        let receipt = self
            .complete(
                &cart,
                digest,
                &cart.amount.clone(),
                Route::Vault,
                paypal,
                open_after,
                self.now(),
            )
            .await
            .map_err(|r| anyhow::anyhow!(r.message))?;
        if let Some((plan_id, index)) = self.config.store.instalment_of_cart(cart_id).await? {
            let json = serde_json::to_string(&receipt)?;
            self.config
                .store
                .finish_instalment(&plan_id, index, "settled", Some(&json))
                .await?;
        }
        if let Some(door) = self.config.payer_doors.get(&cart.payer) {
            if let Err(e) = door
                .send(&SettleMessage::SettlementReceipt { receipt })
                .await
            {
                tracing::warn!(error = %e, "settle: the payer didn't take the receipt; it can read it back");
            }
        }
        tracing::info!(capture = %capture_id, cart = %cart_id, "settle: a capture PayPal confirmed by webhook is booked");
        Ok(Some(CaptureBooked {
            capture_id,
            cart_mandate_id: cart_id.to_owned(),
            completed_now: true,
        }))
    }

    async fn move_money(
        &self,
        intent: &IntentMandate,
        cart: &CartMandate,
        cart_digest: &str,
        payout: Option<&PayoutEvidence>,
    ) -> Result<PayPalSettlement, RailError> {
        if let Some(evidence) = payout {
            let to_us = self
                .config
                .paypal_email
                .as_deref()
                .is_some_and(|email| email.eq_ignore_ascii_case(&evidence.receiver));
            if !to_us {
                return Err(RailError::NotExecuted(format!(
                    "the payout went to {}, not to our PayPal account",
                    evidence.receiver
                )));
            }
            if evidence.amount != cart.amount || evidence.status != "SUCCESS" {
                return Err(RailError::NotExecuted(format!(
                    "the payout is {} {}, not {} paid",
                    words(&evidence.amount),
                    evidence.status,
                    words(&cart.amount)
                )));
            }
            return Ok(PayPalSettlement {
                kind: "payout".into(),
                id: evidence.item_id.clone(),
                parent_id: Some(evidence.batch_id.clone()),
                transaction_id: evidence.transaction_id.clone(),
                status: evidence.status.clone(),
                fee: evidence.fee.clone(),
                sandbox: self.config.vault.as_ref().is_none_or(VaultRail::sandbox),
            });
        }
        let vault = self.config.vault.as_ref().ok_or(RailError::NotConfigured)?;
        let wallet = self
            .config
            .store
            .vault(&intent.payer)
            .await
            .map_err(|e| RailError::NotExecuted(e.to_string()))?
            .and_then(|row| row.vault_id)
            .ok_or_else(|| RailError::NoWallet(intent.payer.clone()))?;
        vault
            .capture(
                &wallet,
                cart,
                cart_digest,
                &paypal_invoice_id(cart),
                &format!(
                    "{} invoice {} (cart mandate {})",
                    self.config.name, cart.invoice_id, cart.cart_mandate_id
                ),
            )
            .await
    }

    /// Posts any settlement whose books post failed, again (each post is idempotent).
    pub async fn repost(&self, cart_mandate_id: &str) -> anyhow::Result<()> {
        let row = self
            .config
            .store
            .cart(cart_mandate_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no cart mandate {cart_mandate_id}"))?;
        let sealed: Sealed = serde_json::from_str(
            row.receipt_json
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("{cart_mandate_id} hasn't settled"))?,
        )?;
        let receipt: SettlementReceipt = sealed.read(SealKind::SettlementReceipt)?;
        self.config.books.post_received(&receipt).await?;
        self.config
            .store
            .finish_cart(cart_mandate_id, "settled", None, None, None)
            .await
    }

    async fn plan_checks(
        &self,
        intent: &IntentMandate,
        plan: &InstalmentPlan,
        invoice: &Invoice,
        today: NaiveDate,
    ) -> Result<(), PlanRefusal> {
        check_plan_shape(plan, invoice, today)?;
        check_plan_bounds(plan, invoice, &intent.instalments, "the payer owner's")?;
        check_plan_expiry(
            plan,
            intent.intent_expiry,
            "the payer owner's intent mandate",
        )?;
        check_plan_expiry(
            plan,
            self.collections.expiry,
            &format!("{}'s owner's collections mandate", self.config.name),
        )?;
        check_plan_bounds(
            plan,
            invoice,
            &self.collections.instalments,
            &format!("{}'s owner's", self.config.name),
        )?;
        let on_terms = self
            .config
            .store
            .scheduled_for(&intent.payer, &plan.total.currency)
            .await
            .map_err(|e| PlanRefusal::Shape(e.to_string()))?;
        let owed = on_terms.minor.saturating_add(plan.total.minor);
        if !plan
            .total
            .currency
            .eq_ignore_ascii_case(&self.collections.credit_limit.currency)
            || owed > self.collections.credit_limit.minor
        {
            return Err(PlanRefusal::CreditLimit {
                owed: words(&Money {
                    currency: plan.total.currency.clone(),
                    minor: owed,
                }),
                limit: words(&self.collections.credit_limit),
            });
        }
        Ok(())
    }

    fn counterparty_allowed(&self, payer: &str) -> Result<(), Refusal> {
        let now = self.now();
        if now >= self.collections.expiry {
            return Err(Refusal::rejected(
                "collections_mandate_expired",
                "Our owner's collections mandate has expired, so we can't agree terms now. Nothing was agreed.",
            ));
        }
        if !self.collections.counterparties.iter().any(|c| c == payer) {
            return Err(Refusal::rejected(
                "no_terms",
                format!(
                    "Our owner hasn't authorised payment terms for {payer}. Nothing was agreed."
                ),
            ));
        }
        Ok(())
    }

    async fn propose(
        &self,
        from: &VerifiedAgent,
        intent_sealed: &Sealed,
        plan: InstalmentPlan,
    ) -> Result<DoorAnswer, Refusal> {
        let intent = self.intent(intent_sealed, Some(from)).await?;
        self.counterparty_allowed(&intent.payer)?;
        let invoice = self.open_invoice(&plan.invoice_id, &intent.payer).await?;
        let today = self.now().date_naive();
        if self
            .plan_checks(&intent, &plan, &invoice, today)
            .await
            .is_ok()
        {
            let plan_id = format!("plan_{}", uuid::Uuid::now_v7().simple());
            let accepted = InstalmentPlan { plan_id, ..plan };
            return Ok(DoorAnswer::InstalmentOffer {
                plan: self.seal(SealKind::InstalmentPlan, &accepted)?,
                countered: false,
                reason: "That plan is inside both owners' mandates; accept it with a cart mandate for each instalment.".into(),
            });
        }
        let refusal = self
            .plan_checks(&intent, &plan, &invoice, today)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        let Some((counter, reason)) = self
            .config
            .negotiator
            .counter(
                &plan,
                &invoice,
                &bounds_within(&intent.instalments, invoice.due_on, intent.intent_expiry),
                &bounds_within(
                    &self.collections.instalments,
                    invoice.due_on,
                    self.collections.expiry,
                ),
                today,
            )
            .await
        else {
            return Err(Refusal::rejected(
                "plan_refused",
                format!("{refusal} No plan fits both owners' mandates. Nothing was agreed."),
            ));
        };
        // Whatever the negotiator drafted is checked like any proposal.
        self.plan_checks(&intent, &counter, &invoice, today)
            .await
            .map_err(plan_refusal)?;
        Ok(DoorAnswer::InstalmentOffer {
            plan: self.seal(SealKind::InstalmentPlan, &counter)?,
            countered: true,
            reason: format!("{refusal} {reason}"),
        })
    }

    async fn accept(
        &self,
        from: Option<&VerifiedAgent>,
        intent_sealed: &Sealed,
        plan_sealed: &Sealed,
        carts: &[Sealed],
    ) -> Result<DoorAnswer, Refusal> {
        let intent = self.intent(intent_sealed, from).await?;
        self.counterparty_allowed(&intent.payer)?;
        if plan_sealed.signer != self.config.keys.origin {
            return Err(Refusal::rejected(
                "plan_refused",
                "That plan wasn't offered by us. Propose it first. Nothing was agreed.",
            ));
        }
        let key = plan_sealed
            .verify(SealKind::InstalmentPlan, self.config.directories.as_ref())
            .await
            .map_err(|e| mandate_refusal(e.into()))?;
        let receivables = self.config.keys.agent(RECEIVABLES).map_err(internal)?;
        if key.thumbprint().ok().as_deref() != Some(receivables.thumbprint()) {
            return Err(Refusal::rejected(
                "plan_refused",
                "That plan wasn't sealed by our Receivables agent. Nothing was agreed.",
            ));
        }
        let plan: InstalmentPlan = plan_sealed
            .read(SealKind::InstalmentPlan)
            .map_err(|e| mandate_refusal(e.into()))?;
        if self
            .config
            .store
            .plan(&plan.plan_id)
            .await
            .map_err(internal)?
            .is_some()
        {
            return Err(Refusal::rejected(
                "plan_already_agreed",
                format!(
                    "Plan {} is already agreed. Nothing new was agreed.",
                    plan.plan_id
                ),
            ));
        }
        let invoice = self.open_invoice(&plan.invoice_id, &intent.payer).await?;
        let today = self.now().date_naive();
        self.plan_checks(&intent, &plan, &invoice, today)
            .await
            .map_err(plan_refusal)?;
        if carts.len() != plan.instalments.len() {
            return Err(Refusal::rejected(
                "plan_refused",
                format!(
                    "The plan has {} instalments but {} cart mandates came with it. Nothing was agreed.",
                    plan.instalments.len(),
                    carts.len()
                ),
            ));
        }
        let intent_digest = intent_sealed.digest().map_err(internal)?.to_string();
        let mut schedule = Vec::new();
        let mut digests = Vec::new();
        for (instalment, sealed) in plan.instalments.iter().zip(carts) {
            let cart = verify_cart(sealed, &intent, self.config.directories.as_ref())
                .await
                .map_err(mandate_refusal)?;
            let bound = cart.instalment.as_ref().is_some_and(|i| {
                i.plan_id == plan.plan_id
                    && i.index == instalment.index
                    && i.count == plan.instalments.len() as u32
            });
            let fits = bound
                && cart.invoice_id == plan.invoice_id
                && cart.amount == instalment.amount
                && cart.due_date == instalment.due_on
                && cart.payee == self.config.keys.origin
                && cart.payer == intent.payer
                && cart.intent_mandate_id == intent.mandate_id
                && cart.intent_digest == intent_digest
                && cart.expires_at.date_naive() > instalment.due_on
                && intent.intent_expiry.date_naive() > instalment.due_on;
            if !fits {
                return Err(Refusal::rejected(
                    "plan_refused",
                    format!(
                        "The cart mandate for instalment {} doesn't match it (amount, due date, plan, or it expires before it falls due). Nothing was agreed.",
                        instalment.index
                    ),
                ));
            }
            let digest = sealed.digest().map_err(internal)?.to_string();
            digests.push(digest);
            schedule.push(ScheduledRow {
                plan_id: plan.plan_id.clone(),
                index: instalment.index,
                due_on: instalment.due_on.to_string(),
                amount: instalment.amount.clone(),
                cart_json: serde_json::to_string(sealed).map_err(internal)?,
                cart_mandate_id: cart.cart_mandate_id.clone(),
                state: "scheduled".into(),
            });
        }
        let agreement = InstalmentAgreement {
            plan: plan_sealed.clone(),
            cart_digests: digests,
            agreed_at: self.now(),
        };
        let sealed_agreement = self.seal(SealKind::InstalmentPlan, &agreement)?;
        self.config
            .store
            .put_plan(
                &PlanRow {
                    plan_id: plan.plan_id.clone(),
                    invoice_id: plan.invoice_id.clone(),
                    payer: intent.payer.clone(),
                    intent_json: serde_json::to_string(intent_sealed).map_err(internal)?,
                    plan_json: serde_json::to_string(plan_sealed).map_err(internal)?,
                    agreement_json: serde_json::to_string(&sealed_agreement).map_err(internal)?,
                    state: "active".into(),
                },
                &schedule,
            )
            .await
            .map_err(internal)?;
        tracing::info!(plan = %plan.plan_id, invoice = %plan.invoice_id, instalments = schedule.len(), "settle: instalment plan agreed and scheduled");
        let books_invoice_id = self
            .config
            .books
            .books_id(&plan.invoice_id)
            .await
            .unwrap_or_default();
        self.emit(SettleEvent::PlanAgreed {
            plan_id: plan.plan_id.clone(),
            invoice_id: plan.invoice_id.clone(),
            books_invoice_id,
            payer: plan.payer.clone(),
            payee: plan.payee.clone(),
            instalments: plan.instalments.clone(),
        });
        Ok(DoorAnswer::InstalmentAgreed {
            agreement: sealed_agreement,
        })
    }

    /// Collects every instalment due on or before today: each with the payer's cart mandate
    /// for it, checked again as any settlement is. Sends each receipt to the payer's door so
    /// its books post the same PayPal id. Returns what happened to each. A kernel that wakes
    /// on each due date calls [`PayeeDesk::collect_instalment`] instead.
    pub async fn collect_due(&self) -> anyhow::Result<Vec<(String, u32, Result<Sealed, Refusal>)>> {
        let today = self.now().date_naive();
        let due = self.config.store.due(&today.to_string()).await?;
        let mut results = Vec::new();
        for row in due {
            let outcome = self.collect_row(&row).await;
            results.push((row.plan_id.clone(), row.index, outcome));
        }
        Ok(results)
    }

    /// Collects one instalment of an agreed plan, on the kernel's wake for its due date.
    /// `position` counts from 0, as saas-company's debt plans do (the plan's own instalment
    /// `index` is `position + 1`). Once collected, asking again returns the same receipt.
    pub async fn collect_instalment(
        &self,
        plan_id: &str,
        position: u32,
    ) -> Result<Sealed, Refusal> {
        let index = position + 1;
        let (row, receipt) = self
            .config
            .store
            .instalment(plan_id, index)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                Refusal::rejected(
                    "no_such_instalment",
                    format!("Plan {plan_id} has no instalment {index}. Nothing was paid."),
                )
            })?;
        if row.state == "settled" {
            if let Some(json) = receipt {
                return serde_json::from_str(&json).map_err(internal);
            }
        }
        self.collect_row(&row).await
    }

    /// Every instalment not yet collected, in due order: what the kernel puts on its wake list
    /// (again after a restart). Each row's `index` is 1-based.
    pub async fn uncollected(&self) -> anyhow::Result<Vec<ScheduledRow>> {
        self.config.store.uncollected().await
    }

    async fn collect_row(&self, row: &ScheduledRow) -> Result<Sealed, Refusal> {
        let plan = self
            .config
            .store
            .plan(&row.plan_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| internal(format!("plan {} is gone", row.plan_id)))?;
        let intent: Sealed = serde_json::from_str(&plan.intent_json).map_err(internal)?;
        let cart: Sealed = serde_json::from_str(&row.cart_json).map_err(internal)?;
        let outcome = self.settle(None, &intent, &cart, None, Some(row)).await;
        match &outcome {
            Ok(receipt) => {
                let json = serde_json::to_string(receipt).map_err(internal)?;
                self.config
                    .store
                    .finish_instalment(&row.plan_id, row.index, "settled", Some(&json))
                    .await
                    .map_err(internal)?;
                if let Some(door) = self.config.payer_doors.get(&plan.payer) {
                    if let Err(e) = door
                        .send(&SettleMessage::SettlementReceipt {
                            receipt: receipt.clone(),
                        })
                        .await
                    {
                        tracing::warn!(error = %e, "settle: the payer didn't take the instalment receipt; it can read it back");
                    }
                }
            }
            Err(refusal) => {
                // Not due yet leaves it scheduled; anything else is marked for the kernel.
                if refusal.code != "not_due" {
                    self.config
                        .store
                        .finish_instalment(&row.plan_id, row.index, "failed", None)
                        .await
                        .map_err(internal)?;
                }
                tracing::warn!(plan = %row.plan_id, index = row.index, code = refusal.code, "settle: instalment not collected");
            }
        }
        outcome
    }

    /// The Receivables agent offers `invoice_id`'s payer an instalment plan (its own terms,
    /// inside its owner's collections mandate). The payer's Payables agent answers over A2A:
    /// it accepts with a cart mandate per instalment when the plan is inside its own owner's
    /// mandate, and the plan is agreed and scheduled here. `plan_id` may be the caller's own
    /// (saas-company uses `plan-<invoice>`); `instalments` carry 1-based `index`es.
    pub async fn agree_plan(
        &self,
        plan_id: Option<String>,
        invoice_id: &str,
        instalments: Vec<crate::protocol::Instalment>,
        note: &str,
    ) -> Result<(InstalmentPlan, Sealed), Refusal> {
        if let Some(id) = &plan_id {
            if let Some(row) = self.config.store.plan(id).await.map_err(internal)? {
                // Agreed already (a retried call): the same plan and agreement.
                let plan: Sealed = serde_json::from_str(&row.plan_json).map_err(internal)?;
                let agreement: Sealed =
                    serde_json::from_str(&row.agreement_json).map_err(internal)?;
                return Ok((
                    plan.read(SealKind::InstalmentPlan).map_err(internal)?,
                    agreement,
                ));
            }
        }
        let invoice = self
            .config
            .books
            .invoice(invoice_id)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                Refusal::rejected(
                    "no_such_invoice",
                    format!("There's no invoice {invoice_id} in our books. Nothing was agreed."),
                )
            })?;
        self.counterparty_allowed(&invoice.payer)?;
        let plan = InstalmentPlan {
            plan_id: plan_id.unwrap_or_else(|| format!("plan_{}", uuid::Uuid::now_v7().simple())),
            invoice_id: invoice.invoice_id.clone(),
            payer: invoice.payer.clone(),
            payee: self.config.keys.origin.clone(),
            total: invoice.open.clone(),
            instalments,
            note: note.to_owned(),
        };
        let today = self.now().date_naive();
        check_plan_shape(&plan, &invoice, today).map_err(plan_refusal)?;
        check_plan_bounds(
            &plan,
            &invoice,
            &self.collections.instalments,
            &format!("{}'s owner's", self.config.name),
        )
        .map_err(plan_refusal)?;
        check_plan_expiry(
            &plan,
            self.collections.expiry,
            &format!("{}'s owner's collections mandate", self.config.name),
        )
        .map_err(plan_refusal)?;
        let sealed = self.seal(SealKind::InstalmentPlan, &plan)?;
        let door = self
            .config
            .payer_doors
            .get(&plan.payer)
            .ok_or_else(|| internal(format!("no door configured for {}", plan.payer)))?;
        let reply = door
            .send(&SettleMessage::InstalmentCounter {
                plan: sealed.clone(),
                reason: plan.note.clone(),
            })
            .await
            .map_err(|e| Refusal::failed("door", e.to_string()))?;
        let Some(DoorAnswer::InstalmentAccepted {
            intent,
            plan: accepted,
            carts,
        }) = reply.answer
        else {
            return Err(Refusal::rejected(
                "plan_refused",
                reply.refusal.unwrap_or_else(|| {
                    "The payer didn't accept the plan. Nothing was agreed.".into()
                }),
            ));
        };
        if accepted != sealed {
            return Err(Refusal::rejected(
                "plan_refused",
                "The payer answered for another plan. Nothing was agreed.",
            ));
        }
        // The payer's intent mandate and cart mandates are checked as any acceptance is.
        let answer = self.accept(None, &intent, &sealed, &carts).await?;
        let DoorAnswer::InstalmentAgreed { agreement } = answer else {
            return Err(internal("no agreement"));
        };
        Ok((plan, agreement))
    }

    /// Finishes saving a wallet when PayPal returns its owner to `/settle/vault/approved`
    /// with the setup token.
    pub async fn finish_vault_by_token(&self, setup_token: &str) -> anyhow::Result<VaultRow> {
        let row = self
            .config
            .store
            .vault_by_token(setup_token)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no wallet setup is waiting on that token"))?;
        self.finish_vault(&row.payer).await
    }
}

#[async_trait]
impl DoorHandler for PayeeDesk {
    async fn handle(
        &self,
        from: &VerifiedAgent,
        message: SettleMessage,
    ) -> Result<DoorAnswer, Refusal> {
        match message {
            SettleMessage::Settle {
                intent,
                cart,
                payout,
            } => {
                let receipt = self
                    .settle(Some(from), &intent, &cart, payout.as_ref(), None)
                    .await?;
                Ok(DoorAnswer::Settled { receipt })
            }
            SettleMessage::InstalmentPropose { intent, plan } => {
                self.propose(from, &intent, plan).await
            }
            SettleMessage::InstalmentAccept {
                intent,
                plan,
                carts,
            } => self.accept(Some(from), &intent, &plan, &carts).await,
            other => Err(Refusal::rejected(
                "wrong_door",
                format!(
                    "This is {}'s door for collecting what it is owed; it doesn't take {} messages.",
                    self.config.name,
                    other.kind()
                ),
            )),
        }
    }
}

/// A collections mandate's terms in one line, for logs and the board.
pub fn describe(collections: &CollectionsMandate) -> serde_json::Value {
    json!({
        "mandate_id": collections.mandate_id,
        "counterparties": collections.counterparties,
        "credit_limit": words(&collections.credit_limit),
        "instalments": collections.instalments.to_string(),
        "expiry": collections.expiry,
    })
}

/// Grace after an instalment's due date during which its cart mandate still holds.
pub const INSTALMENT_GRACE_DAYS: i64 = 14;

/// The expiry a payer gives an instalment's cart mandate.
pub fn instalment_expiry(due_on: NaiveDate) -> DateTime<Utc> {
    (due_on + Duration::days(INSTALMENT_GRACE_DAYS))
        .and_hms_opt(0, 0, 0)
        .map(|t| t.and_utc())
        .unwrap_or_else(Utc::now)
}
