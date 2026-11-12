//! Wallet records (v3 Technology 3): each money-handling agent's append-only, tamper-evident
//! record of what it did with the business's PayPal account. The money stays in PayPal; a wallet
//! is a **record**, never "holds funds".
//!
//! - **Storage.** One stream per site (`wallet:<company>`) in adk-auth's signed audit log: an
//!   RFC 9162 Merkle log with signed checkpoints, checked offline by `adk-rust audit verify`. As
//!   in adk-payments, the wallet is the business's: each entry names the agent that acted
//!   ([`WalletEntry::acting`]), and an agent's "wallet" is its view of the site's record (the
//!   Cashier's is the whole record; a desk's is what it acted on). Checkpoints and the three-way
//!   match are per site.
//! - **Source.** An entry's facts come only from PayPal: [`Wallets::record_event`] takes a
//!   [`VerifiedEvent`] (made only by PayPal's own webhook check) and reads the object back;
//!   [`Wallets::record`] reads it back directly. A webhook only names the object; amounts,
//!   status, time and payer are what the read-back says.
//! - **Books.** Every object posts to the company's books (Zavora ERA, hosted) through
//!   saas-core's posting rules, once by PayPal id; the link is written to the log too.
//! - **Sync.** [`Wallets::checkpoint`] reads every object back, records the reconciliation
//!   ("matches PayPal as of 14:02 EAT", or the drift) and signs a checkpoint over it.
//!   [`Wallets::reconcile`] is the hourly call: the same, plus the entries that drifted.
//! - **Board.** [`Wallets::matches`] (wallet entry, PayPal object, journal in the books) and
//!   [`Wallets::feed`] for the AG Grid tables; [`router`] serves `GET /wallet/{site}.json`.
//!
//! The log's records: `wallet.entry` (one [`WalletEntry`] in `ext.entry`), `wallet.booked` (a
//! [`BooksLink`] for one PayPal object), `wallet.reconciled` (a [`Reconciliation`] and the
//! statuses read). The context an entry carries beyond PayPal's facts (the acting agent, the
//! mandate or approval behind it) comes from the governed path that moved the money.

mod http;
mod paypal;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use adk_auth::signed_audit::{
    AuditExport, AuditLog, AuditSigner, Ed25519Signer, NoteVerifier, VerifyReport,
};
use adk_core::audit_log::{
    AuditActor, AuditLogStore, AuditRecord, AuditRecordDraft, AuditRecordKind, AuditRecordOutcome,
};
use agentrix_effects::paypal::PayPalClient;
use chrono::Utc;
use saas_core::books::{
    BillId, Books, BooksError, DocumentFilter, InvoiceDraft, InvoiceId, LineDraft, PaymentFilter,
};
use saas_core::paypal::PayPalRef;
use saas_core::wallet::*;
use saas_core::{
    AgentId, CompanyId, Counterparty, Money, PayPalPosting, post_paypal, recurring_invoice_for,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

pub use http::router;
pub use paypal::{Observed, VerifiedEvent, read_back, verify_webhook};

const ENTRY: &str = "wallet.entry";
const BOOKED: &str = "wallet.booked";
const RECONCILED: &str = "wallet.reconciled";

/// Why a wallet refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    /// PayPal's check refused the webhook, or no webhook id is configured.
    #[error("PayPal did not verify this webhook")]
    NotVerified,
    /// PayPal couldn't be read.
    #[error("PayPal: {0}")]
    PayPal(String),
    /// PayPal says the money hasn't moved (status shown), so there is nothing to record yet.
    #[error("PayPal reports {0} as {1}: nothing to record yet")]
    NotCompleted(String, String),
    /// The books refused.
    #[error(transparent)]
    Books(#[from] BooksError),
    /// The signed log failed.
    #[error("signed log: {0}")]
    Log(String),
}

fn log_err(e: impl std::fmt::Display) -> WalletError {
    WalletError::Log(e.to_string())
}

/// The context an entry carries beyond PayPal's facts, from the governed path that moved the
/// money.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// The agent that acted.
    pub acting: AgentId,
    /// The mandate or approval behind it.
    pub authority: Authority,
    /// What caused it, when the caller knows better than PayPal's fields (an invoice, a bill).
    pub cause: Option<WalletCause>,
    /// Who, when the caller knows them by name (a vendor in the books); PayPal's email is kept.
    pub counterparty: Option<WalletCounterparty>,
}

/// What recording one PayPal object left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// The entries for it (the object, and its fee when there is one).
    pub entries: Vec<WalletEntry>,
    /// Its link into the books.
    pub books: BooksLink,
    /// Whether everything was already recorded (nothing new written).
    pub already: bool,
}

/// What one reconciliation found ([`Wallets::reconcile`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconcile {
    /// The signed checkpoint over the record, with the reconciliation it carries.
    pub checkpoint: WalletCheckpoint,
    /// The entries whose PayPal object drifted, oldest first (empty when it matches).
    pub drifted: Vec<WalletEntry>,
}

impl Reconcile {
    /// Whether the site's record matches PayPal.
    pub fn matches(&self) -> bool {
        self.checkpoint.reconciliation.state == SyncState::Matches
    }

    /// What drifted, in plain words, one line per difference.
    pub fn drift(&self) -> &[Drift] {
        &self.checkpoint.reconciliation.drift
    }
}

struct WalletLog {
    log: AuditLog,
    write: Mutex<()>,
}

/// The wallets of one company's money-handling agents.
pub struct Wallets {
    company: CompanyId,
    store: Arc<dyn AuditLogStore>,
    signer: Arc<dyn AuditSigner>,
    verifier: String,
    paypal: Arc<PayPalClient>,
    books: Arc<dyn Books>,
    logs: Mutex<HashMap<String, Arc<WalletLog>>>,
}

impl std::fmt::Debug for Wallets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallets")
            .field("company", &self.company)
            .field("verifier", &self.verifier)
            .finish_non_exhaustive()
    }
}

/// The key name a company's wallet checkpoints are signed under.
pub fn key_name(company: &CompanyId) -> String {
    format!("wallet.{company}.agentrix.website")
}

/// The signed stream of a site's wallet record: one per company, `wallet:<company>`.
pub fn site_log_id(company: &CompanyId) -> String {
    format!("wallet:{company}")
}

/// The stream an agent's wallet view reads: its company's ([`site_log_id`]).
pub fn log_id(wallet: &AgentId) -> String {
    site_log_id(wallet.company())
}

impl Wallets {
    /// The wallets in `store`, signed with `signer`, reading PayPal through `paypal` and
    /// posting to `books`.
    pub fn new(
        company: CompanyId,
        store: Arc<dyn AuditLogStore>,
        signer: Ed25519Signer,
        paypal: Arc<PayPalClient>,
        books: Arc<dyn Books>,
    ) -> Self {
        let verifier = signer.verifier_key();
        Self {
            company,
            store,
            signer: Arc::new(signer),
            verifier,
            paypal,
            books,
            logs: Mutex::new(HashMap::new()),
        }
    }

    /// The wallets in `<dir>/<company>-wallet.db`, signed with the key in
    /// `<dir>/keys/<company>-wallet.key` (made, mode 0600, on first use; keep `dir` outside the
    /// repository).
    pub async fn sqlite(
        company: CompanyId,
        dir: &std::path::Path,
        paypal: Arc<PayPalClient>,
        books: Arc<dyn Books>,
    ) -> Result<Self, WalletError> {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        use std::str::FromStr;
        std::fs::create_dir_all(dir.join("keys")).map_err(log_err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for d in [dir.to_path_buf(), dir.join("keys")] {
                std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))
                    .map_err(log_err)?;
            }
        }
        let file = dir.join(format!("{company}-wallet.db"));
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", file.display()))
            .map_err(log_err)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(30));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(log_err)?;
        let store = adk_session::SqliteAuditLogStore::from_pool(pool);
        store.migrate().await.map_err(log_err)?;
        let key_file = dir.join("keys").join(format!("{company}-wallet.key"));
        let signer = match std::fs::read_to_string(&key_file) {
            Ok(text) => Ed25519Signer::from_note_key(text.trim()).map_err(log_err)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let signer = Ed25519Signer::generate(key_name(&company)).map_err(log_err)?;
                write_secret(&key_file, signer.to_note_key().as_str())?;
                signer
            }
            Err(e) => return Err(log_err(e)),
        };
        Ok(Self::new(company, Arc::new(store), signer, paypal, books))
    }

    /// The company.
    pub fn company(&self) -> &CompanyId {
        &self.company
    }

    /// The key that verifies every wallet's checkpoints, `<name>+<hash>+<base64>`.
    pub fn verifier_key(&self) -> &str {
        &self.verifier
    }

    /// The site's Cashier: holder of the site's wallet record ([`WalletEntry::wallet`]).
    pub fn cashier(&self) -> Result<AgentId, WalletError> {
        AgentId::new(self.company.clone(), AgentId::CASHIER).map_err(log_err)
    }

    /// Whether `view` sees `entry`: the Cashier sees the whole site; any other agent sees the
    /// entries it acted on.
    fn sees(view: &AgentId, entry: &WalletEntry) -> bool {
        view.name() == AgentId::CASHIER || &entry.acting == view
    }

    async fn wallet(&self, wallet: &AgentId) -> Result<Arc<WalletLog>, WalletError> {
        if wallet.company() != &self.company {
            return Err(WalletError::Log(format!(
                "{wallet} is not an agent of {}",
                self.company
            )));
        }
        let mut logs = self.logs.lock().await;
        if let Some(l) = logs.get(&log_id(wallet)) {
            return Ok(l.clone());
        }
        let log = AuditLog::builder(self.store.clone(), log_id(wallet), self.signer.clone())
            .checkpoint_every(1)
            .open()
            .await
            .map_err(log_err)?;
        let l = Arc::new(WalletLog {
            log,
            write: Mutex::new(()),
        });
        logs.insert(log_id(wallet), l.clone());
        Ok(l)
    }

    async fn records(&self, wallet: &AgentId) -> Result<Vec<AuditRecord>, WalletError> {
        let export = AuditExport::from_store(self.store.as_ref(), &log_id(wallet))
            .await
            .map_err(log_err)?;
        export
            .entries
            .iter()
            .filter_map(|e| e.record.as_deref())
            .map(|t| serde_json::from_str(t).map_err(log_err))
            .collect()
    }

    /// The entries `wallet` sees, oldest first: the Cashier's view is the site's whole record; a
    /// desk's (Receivables, Payables) is the entries it acted on.
    pub async fn entries(&self, wallet: &AgentId) -> Result<Vec<WalletEntry>, WalletError> {
        Ok(self
            .site_entries(wallet)
            .await?
            .into_iter()
            .filter(|e| Self::sees(wallet, e))
            .collect())
    }

    /// Every entry of the site `wallet` belongs to, oldest first.
    async fn site_entries(&self, wallet: &AgentId) -> Result<Vec<WalletEntry>, WalletError> {
        self.records(wallet)
            .await?
            .into_iter()
            .filter(|r| r.action == ENTRY)
            .filter_map(|r| r.ext.get("entry").cloned())
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .map_err(log_err)
    }

    async fn links(&self, wallet: &AgentId) -> Result<BTreeMap<String, BooksLink>, WalletError> {
        let mut out = BTreeMap::new();
        for r in self
            .records(wallet)
            .await?
            .into_iter()
            .filter(|r| r.action == BOOKED)
        {
            if let (Some(Value::String(key)), Some(link)) = (r.ext.get("key"), r.ext.get("books")) {
                out.insert(
                    key.clone(),
                    serde_json::from_value(link.clone()).map_err(log_err)?,
                );
            }
        }
        Ok(out)
    }

    /// Records a webhook PayPal verified: reads its object back, writes the entries, posts to
    /// the books. `Ok(None)` for an event no wallet records.
    pub async fn record_event(
        &self,
        wallet: &AgentId,
        event: &VerifiedEvent,
        ctx: &Context,
    ) -> Result<Option<Recorded>, WalletError> {
        let Some(object) = &event.object else {
            return Ok(None);
        };
        let source = WalletSource::Webhook {
            event_id: event.id.clone(),
            event_type: event.event_type.clone(),
        };
        self.record_from(wallet, object, Some(source), ctx)
            .await
            .map(Some)
    }

    /// Records one PayPal object by reading it back: the path for money this platform moved
    /// itself (a capture it made, a payout it sent) and for webhooks that never arrived.
    pub async fn record(
        &self,
        wallet: &AgentId,
        object: &PayPalRef,
        ctx: &Context,
    ) -> Result<Recorded, WalletError> {
        self.record_from(wallet, object, None, ctx).await
    }

    async fn record_from(
        &self,
        wallet: &AgentId,
        object: &PayPalRef,
        webhook: Option<WalletSource>,
        ctx: &Context,
    ) -> Result<Recorded, WalletError> {
        let w = self.wallet(wallet).await?;
        let _one_writer = w.write.lock().await;
        let holder = self.cashier()?;
        let existing: Vec<WalletEntry> = self
            .site_entries(wallet)
            .await?
            .into_iter()
            .filter(|e| &e.paypal == object)
            .collect();
        let links = self.links(wallet).await?;
        let obs = paypal::read_back(&self.paypal, object).await?;
        let mut entries = existing.clone();
        let mut fresh = false;
        if existing.is_empty() {
            if !obs.completed() {
                return Err(WalletError::NotCompleted(object.key(), obs.status));
            }
            let source = webhook.unwrap_or(WalletSource::ReadBack {
                call: obs.call.clone(),
            });
            for (kind, amount) in entry_amounts(&obs) {
                let seq = w.log.size().await;
                let entry = WalletEntry {
                    wallet: holder.clone(),
                    seq,
                    at: obs.at,
                    kind,
                    amount,
                    paypal: obs.paypal.clone(),
                    paypal_status: obs.status.clone(),
                    counterparty: merge_counterparty(
                        ctx.counterparty.clone(),
                        obs.counterparty.clone(),
                    ),
                    cause: ctx.cause.clone().or_else(|| obs.cause.clone()),
                    acting: ctx.acting.clone(),
                    authority: ctx.authority.clone(),
                    source: source.clone(),
                };
                let record = w
                    .log
                    .append(
                        self.draft(ENTRY, &ctx.acting)
                            .ext("entry", serde_json::to_value(&entry).map_err(log_err)?),
                    )
                    .await
                    .map_err(log_err)?;
                if record.seq != seq {
                    return Err(WalletError::Log(format!(
                        "wallet {wallet}: expected seq {seq}, the log wrote {}",
                        record.seq
                    )));
                }
                entries.push(entry);
            }
            fresh = true;
        }
        // Books: post once by PayPal id (saas-core's rules are idempotent), then link it in the log.
        let link = match links.get(&object.key()) {
            Some(l) => l.clone(),
            None => {
                let link = self.post(&entries, &obs).await?;
                w.log
                    .append(
                        self.draft(BOOKED, &ctx.acting)
                            .ext("key", json!(object.key()))
                            .ext("books", serde_json::to_value(&link).map_err(log_err)?),
                    )
                    .await
                    .map_err(log_err)?;
                fresh = true;
                link
            }
        };
        Ok(Recorded {
            entries,
            books: link,
            already: !fresh,
        })
    }

    fn draft(&self, action: &str, acting: &AgentId) -> AuditRecordDraft {
        let draft = AuditRecordDraft::new(AuditRecordKind::Payment, action)
            .outcome(AuditRecordOutcome::Succeeded);
        match adk_core::AgentId::new(saas_core::identity::Badge::urn(acting)) {
            Ok(agent) => {
                let ctx = adk_core::ActingContext::new(adk_core::AgentPrincipal::new(agent));
                let mut actor = AuditActor::new(&ctx, "wallet");
                actor.tenant = Some(self.company.to_string());
                draft.actor(actor)
            }
            Err(_) => draft,
        }
    }

    /// Posts one PayPal object to the books through saas-core's posting rules.
    async fn post(
        &self,
        entries: &[WalletEntry],
        obs: &Observed,
    ) -> Result<BooksLink, WalletError> {
        let first = entries
            .first()
            .ok_or_else(|| WalletError::Log("no entry to post".into()))?;
        let books = self.books.as_ref();
        let date = obs.at.date_naive();
        let who = first.counterparty.clone().unwrap_or(WalletCounterparty {
            name: "PayPal customer".into(),
            email: None,
            company: None,
        });
        let party = Counterparty {
            name: who.name.clone(),
            email: who.email.clone(),
            currency: obs.gross.currency,
            company: who.company.clone(),
            paypal_email: who.email.clone(),
        };
        // Already in the books (posted before the wallet saw it): the rules find it by PayPal
        // id, so nothing new is made for it, not even an order's invoice.
        let filter = PaymentFilter {
            reference: Some(obs.paypal.key()),
            direction: None,
        };
        let in_books = !books.payments(&filter).await?.is_empty();
        let posting = match obs.kind {
            WalletEntryKind::Capture => {
                let customer = books.ensure_customer(&party, None).await?.id;
                let invoice = match &first.cause {
                    Some(WalletCause::Invoice { invoice, .. }) => Some(invoice.clone()),
                    Some(WalletCause::Order { order, reference }) if !in_books => Some(
                        self.order_invoice(&customer, order.as_str(), reference.as_deref(), obs)
                            .await?,
                    ),
                    _ => None,
                };
                PayPalPosting::Received {
                    paypal: obs.paypal.clone(),
                    customer,
                    gross: obs.gross,
                    fee: obs.fee,
                    date,
                    invoice,
                }
            }
            WalletEntryKind::Subscription => {
                let customer = books.ensure_customer(&party, None).await?.id;
                let invoice = match &first.cause {
                    Some(WalletCause::Subscription { subscription }) => {
                        recurring_invoice_for(books, &subscription.reference(), date).await?
                    }
                    Some(WalletCause::Invoice { invoice, .. }) => Some(invoice.clone()),
                    _ => None,
                };
                PayPalPosting::Received {
                    paypal: obs.paypal.clone(),
                    customer,
                    gross: obs.gross,
                    fee: obs.fee,
                    date,
                    invoice,
                }
            }
            WalletEntryKind::Refund => {
                let customer = books.ensure_customer(&party, None).await?.id;
                let invoice = match &first.cause {
                    Some(WalletCause::Invoice { invoice, .. }) => Some(invoice.clone()),
                    _ => None,
                };
                PayPalPosting::Refunded {
                    paypal: obs.paypal.clone(),
                    customer,
                    amount: obs.gross,
                    fee_returned: obs.fee,
                    date,
                    invoice,
                    reason: "Refunded on PayPal".into(),
                }
            }
            WalletEntryKind::Payout | WalletEntryKind::Fee => {
                let vendor = books.ensure_vendor(&party).await?.id;
                let bill: Option<BillId> = match &first.cause {
                    Some(WalletCause::Bill { bill, .. }) => Some(bill.clone()),
                    _ => None,
                };
                PayPalPosting::PaidOut {
                    paypal: obs.paypal.clone(),
                    vendor,
                    amount: obs.gross,
                    fee: obs.fee,
                    date,
                    bill,
                }
            }
        };
        let posted = post_paypal(books, &posting).await?;
        let payment_journal = match &posted.payment {
            Some(p) => books.payment(p).await?.journal,
            None => None,
        };
        let link = BooksLink {
            payment: posted.payment,
            journal: posted.journal.or(payment_journal),
            fee_journal: posted.fee_journal,
            ..BooksLink::default()
        };
        Ok(self.numbered(link).await)
    }

    /// The link with the books' own numbers (PAY-…, JE-…) filled in where the books give them.
    async fn numbered(&self, mut link: BooksLink) -> BooksLink {
        if link.payment_number.is_none() {
            if let Some(p) = &link.payment {
                link.payment_number = self.books.payment(p).await.ok().and_then(|p| p.number);
            }
        }
        if link.journal_number.is_none() {
            if let Some(j) = &link.journal {
                link.journal_number = self
                    .books
                    .journal_entry(j)
                    .await
                    .ok()
                    .and_then(|j| j.number);
            }
        }
        if link.fee_journal_number.is_none() {
            if let Some(j) = &link.fee_journal {
                link.fee_journal_number = self
                    .books
                    .journal_entry(j)
                    .await
                    .ok()
                    .and_then(|j| j.number);
            }
        }
        link
    }

    /// A customer's order with no invoice of its own (a shop sale) gets one, so the receipt pays
    /// revenue rather than waiting unapplied. Idempotent by the order's reference.
    async fn order_invoice(
        &self,
        customer: &saas_core::books::CustomerId,
        order: &str,
        reference: Option<&str>,
        obs: &Observed,
    ) -> Result<InvoiceId, WalletError> {
        let key = format!("paypal:order:{order}");
        let filter = DocumentFilter {
            party: None,
            open_only: false,
            reference: Some(key.clone()),
        };
        if let Some(inv) = self.books.invoices(&filter).await?.into_iter().next() {
            return Ok(inv.id);
        }
        let date = obs.at.date_naive();
        let draft = InvoiceDraft {
            customer: customer.clone(),
            issue_date: date,
            due_date: date,
            currency: obs.gross.currency,
            lines: vec![LineDraft {
                product: None,
                description: reference
                    .map(|r| format!("Order {r}"))
                    .unwrap_or_else(|| format!("PayPal order {order}")),
                quantity: 1,
                unit_price: obs.gross,
            }],
            reference: Some(key),
            memo: Some("Paid on PayPal".into()),
        };
        let inv = self.books.draft_invoice(&draft).await?;
        Ok(self.books.post_invoice(&inv.id).await?.id)
    }

    /// Reads every object of the site's record back from PayPal, records the reconciliation, and
    /// signs a checkpoint over the record. Per site: any agent of the company names it.
    pub async fn checkpoint(&self, wallet: &AgentId) -> Result<WalletCheckpoint, WalletError> {
        let w = self.wallet(wallet).await?;
        let _one_writer = w.write.lock().await;
        let entries = self.site_entries(wallet).await?;
        let mut statuses = BTreeMap::new();
        let mut observed: BTreeMap<String, Vec<(WalletEntryKind, Money)>> = BTreeMap::new();
        let mut drift = Vec::new();
        let mut objects = 0u32;
        let mut seen = Vec::new();
        for e in &entries {
            if seen.contains(&e.paypal) {
                continue;
            }
            seen.push(e.paypal.clone());
            objects += 1;
            let ours: Vec<&WalletEntry> = entries.iter().filter(|x| x.paypal == e.paypal).collect();
            let all: Vec<u64> = ours.iter().map(|x| x.seq).collect();
            match paypal::read_back(&self.paypal, &e.paypal).await {
                Ok(obs) => {
                    statuses.insert(e.paypal.key(), obs.status.clone());
                    observed.insert(e.paypal.key(), entry_amounts(&obs));
                    let expected = entry_amounts(&obs);
                    for (kind, amount) in &expected {
                        match ours.iter().find(|x| x.kind == *kind) {
                            Some(x) if x.amount == *amount => {}
                            Some(x) => drift.push(Drift {
                                paypal: e.paypal.clone(),
                                what: format!(
                                    "PayPal has {amount} for the {kind:?}, the wallet {}",
                                    x.amount
                                ),
                                entries: vec![x.seq],
                            }),
                            None => drift.push(Drift {
                                paypal: e.paypal.clone(),
                                what: format!(
                                    "PayPal has a {kind:?} of {amount} the wallet doesn't"
                                ),
                                entries: Vec::new(),
                            }),
                        }
                    }
                    if obs.status != e.paypal_status {
                        drift.push(Drift {
                            paypal: e.paypal.clone(),
                            what: format!(
                                "PayPal now says {} (the wallet recorded {})",
                                obs.status, e.paypal_status
                            ),
                            entries: all,
                        });
                    }
                }
                Err(err) => drift.push(Drift {
                    paypal: e.paypal.clone(),
                    what: format!("PayPal couldn't be read: {err}"),
                    entries: all,
                }),
            }
        }
        let reconciliation = Reconciliation {
            checked_at: Utc::now(),
            method: ReconcileMethod::ReadBack,
            objects,
            state: if drift.is_empty() {
                SyncState::Matches
            } else {
                SyncState::Drift
            },
            drift,
        };
        let totals = totals(&entries)?;
        w.log
            .append(
                self.draft(RECONCILED, wallet)
                    .ext(
                        "reconciliation",
                        serde_json::to_value(&reconciliation).map_err(log_err)?,
                    )
                    .ext("totals", serde_json::to_value(totals).map_err(log_err)?)
                    .ext(
                        "statuses",
                        serde_json::to_value(&statuses).map_err(log_err)?,
                    )
                    .ext(
                        "observed",
                        serde_json::to_value(&observed).map_err(log_err)?,
                    ),
            )
            .await
            .map_err(log_err)?;
        let cp = w.log.checkpoint().await.map_err(log_err)?;
        Ok(WalletCheckpoint {
            wallet: self.cashier()?,
            size: cp.size,
            root_hash: cp.root_hash.to_string(),
            note: cp.note,
            verifier_key: self.verifier.clone(),
            at: cp.created_at,
            totals,
            reconciliation,
        })
    }

    /// The site's hourly reconciliation: reads every PayPal object in the site's record back
    /// (those recorded since the last run included), writes the result and a signed checkpoint
    /// over it, and says whether the record matches PayPal or which entries drifted.
    pub async fn reconcile(&self) -> Result<Reconcile, WalletError> {
        let cashier = self.cashier()?;
        let checkpoint = self.checkpoint(&cashier).await?;
        let drift = &checkpoint.reconciliation.drift;
        let drifted = if drift.is_empty() {
            Vec::new()
        } else {
            self.site_entries(&cashier)
                .await?
                .into_iter()
                .filter(|e| {
                    drift
                        .iter()
                        .any(|d| d.entries.contains(&e.seq) || d.paypal == e.paypal)
                })
                .collect()
        };
        Ok(Reconcile {
            checkpoint,
            drifted,
        })
    }

    /// Every reconciliation of `wallet`, oldest first, each with the signed checkpoint over it.
    pub async fn reconciliations(
        &self,
        wallet: &AgentId,
    ) -> Result<Vec<WalletCheckpoint>, WalletError> {
        let export = AuditExport::from_store(self.store.as_ref(), &log_id(wallet))
            .await
            .map_err(log_err)?;
        let mut out = Vec::new();
        for entry in &export.entries {
            let Some(text) = entry.record.as_deref() else {
                continue;
            };
            let r: AuditRecord = serde_json::from_str(text).map_err(log_err)?;
            if r.action != RECONCILED {
                continue;
            }
            let field = |k: &str| r.ext.get(k).cloned().unwrap_or(Value::Null);
            let reconciliation: Reconciliation =
                serde_json::from_value(field("reconciliation")).map_err(log_err)?;
            let totals: WalletTotals = serde_json::from_value(field("totals")).map_err(log_err)?;
            let Some(cp) = export
                .checkpoints
                .iter()
                .filter(|c| c.size > r.seq)
                .min_by_key(|c| c.size)
            else {
                continue;
            };
            out.push(WalletCheckpoint {
                wallet: self.cashier()?,
                size: cp.size,
                root_hash: cp.root_hash.to_string(),
                note: cp.note.clone(),
                verifier_key: self.verifier.clone(),
                at: cp.created_at,
                totals,
                reconciliation,
            });
        }
        Ok(out)
    }

    /// The latest reconciliation's checkpoint, in a few words, without reading PayPal again.
    pub async fn latest(
        &self,
        wallet: &AgentId,
    ) -> Result<Option<WalletCheckpointSummary>, WalletError> {
        Ok(self
            .reconciliations(wallet)
            .await?
            .last()
            .map(WalletCheckpoint::summary))
    }

    /// The latest reconciliation's checkpoint as a badge cites it.
    pub async fn latest_ref(
        &self,
        wallet: &AgentId,
    ) -> Result<Option<WalletCheckpointRef>, WalletError> {
        Ok(self
            .reconciliations(wallet)
            .await?
            .last()
            .map(WalletCheckpoint::reference))
    }

    /// What one journal carries (its debits), found by its reference.
    async fn journal_amount(&self, reference: &str) -> Option<Money> {
        let filter = saas_core::books::JournalFilter {
            reference: Some(reference.to_owned()),
            ..Default::default()
        };
        let journal = self.books.journal(&filter).await.ok()?.into_iter().next()?;
        let currency = journal.lines.first()?.debit.currency;
        journal
            .lines
            .iter()
            .try_fold(Money::zero(currency), |a, l| a.checked_add(l.debit).ok())
    }

    /// The three-way match: each entry against what PayPal had at the last read and what the
    /// books carry.
    pub async fn matches(&self, wallet: &AgentId) -> Result<Vec<ThreeWayMatch>, WalletError> {
        let entries = self.entries(wallet).await?;
        let links = self.links(wallet).await?;
        let records = self.records(wallet).await?;
        let last = records.iter().rev().find(|r| r.action == RECONCILED);
        let read = |k: &str| last.and_then(|r| r.ext.get(k).cloned());
        let statuses: BTreeMap<String, String> = read("statuses")
            .map(serde_json::from_value)
            .transpose()
            .map_err(log_err)?
            .unwrap_or_default();
        let observed: BTreeMap<String, Vec<(WalletEntryKind, Money)>> = read("observed")
            .map(serde_json::from_value)
            .transpose()
            .map_err(log_err)?
            .unwrap_or_default();
        let mut out = Vec::with_capacity(entries.len());
        for e in entries {
            let key = e.paypal.key();
            // Links written before the books' numbers were kept get them on read.
            let link = self
                .numbered(links.get(&key).cloned().unwrap_or_default())
                .await;
            // Until a checkpoint reads PayPal again, the last read is the one the entry came from.
            let paypal_status = Some(
                statuses
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| e.paypal_status.clone()),
            );
            let paypal_amount = match observed.get(&key) {
                Some(kinds) => kinds.iter().find(|(k, _)| *k == e.kind).map(|(_, m)| *m),
                None => Some(e.amount),
            };
            let sign = if e.amount.minor < 0 { -1 } else { 1 };
            let books_amount = match (e.kind, &link.payment) {
                (WalletEntryKind::Fee, _) => self.journal_amount(&e.paypal.fee_key()).await,
                (_, Some(p)) => self.books.payment(p).await.ok().map(|pay| pay.amount),
                (_, None) => self.journal_amount(&key).await,
            }
            .map(|m| Money::new(sign * m.minor.abs(), m.currency));
            let state = if paypal_status.as_deref() != Some(e.paypal_status.as_str())
                || paypal_amount != Some(e.amount)
            {
                MatchState::PayPalDiffers
            } else {
                match books_amount {
                    None => MatchState::NotInBooks,
                    Some(b) if b == e.amount => MatchState::Matched,
                    Some(_) => MatchState::BooksDiffer,
                }
            };
            let books = if e.kind == WalletEntryKind::Fee {
                BooksLink {
                    payment: None,
                    journal: link.fee_journal.clone(),
                    fee_journal: link.fee_journal.clone(),
                    payment_number: None,
                    journal_number: link.fee_journal_number.clone(),
                    fee_journal_number: link.fee_journal_number.clone(),
                }
            } else {
                link
            };
            out.push(ThreeWayMatch {
                seq: e.seq,
                kind: e.kind,
                amount: e.amount,
                paypal: e.paypal,
                paypal_status,
                paypal_amount,
                books,
                books_amount,
                state,
            });
        }
        Ok(out)
    }

    /// What the board's tables read for one wallet.
    pub async fn feed(&self, wallet: &AgentId) -> Result<WalletFeed, WalletError> {
        let history: Vec<WalletCheckpointSummary> = self
            .reconciliations(wallet)
            .await?
            .iter()
            .map(WalletCheckpoint::summary)
            .collect();
        Ok(WalletFeed {
            wallet: wallet.clone(),
            checkpoint: history.last().cloned(),
            history,
            entries: self.entries(wallet).await?,
            matches: self.matches(wallet).await?,
        })
    }

    /// The wallet's signed log, as `adk-rust audit verify` reads it.
    pub async fn export(&self, wallet: &AgentId) -> Result<AuditExport, WalletError> {
        let w = self.wallet(wallet).await?;
        if w.log.size().await > 0 {
            w.log.checkpoint().await.map_err(log_err)?;
        }
        AuditExport::from_store(self.store.as_ref(), &log_id(wallet))
            .await
            .map_err(log_err)
    }

    /// Verifies the wallet's whole log against the wallets' own key.
    pub async fn verify(&self, wallet: &AgentId) -> Result<VerifyReport, WalletError> {
        let key: NoteVerifier = self
            .verifier
            .parse()
            .map_err(|e| WalletError::Log(format!("verifier key: {e:?}")))?;
        Ok(self.export(wallet).await?.verify(&[key]))
    }
}

/// The entries one PayPal object makes: the object itself, and its fee when there is one.
fn entry_amounts(obs: &Observed) -> Vec<(WalletEntryKind, Money)> {
    let mut out = Vec::with_capacity(2);
    match obs.kind {
        WalletEntryKind::Capture | WalletEntryKind::Subscription => {
            out.push((obs.kind, obs.gross));
            if obs.fee.is_positive() {
                out.push((WalletEntryKind::Fee, obs.fee.negate()));
            }
        }
        WalletEntryKind::Refund => {
            out.push((obs.kind, obs.gross.negate()));
            if obs.fee.is_positive() {
                out.push((WalletEntryKind::Fee, obs.fee));
            }
        }
        WalletEntryKind::Payout | WalletEntryKind::Fee => {
            out.push((WalletEntryKind::Payout, obs.gross.negate()));
            if obs.fee.is_positive() {
                out.push((WalletEntryKind::Fee, obs.fee.negate()));
            }
        }
    }
    out
}

fn merge_counterparty(
    ours: Option<WalletCounterparty>,
    paypal: Option<WalletCounterparty>,
) -> Option<WalletCounterparty> {
    match (ours, paypal) {
        (Some(mut o), Some(p)) => {
            if p.email.is_some() {
                o.email = p.email;
            }
            Some(o)
        }
        (o, p) => o.or(p),
    }
}

fn totals(entries: &[WalletEntry]) -> Result<WalletTotals, WalletError> {
    let currency = entries
        .first()
        .map(|e| e.amount.currency)
        .unwrap_or(saas_core::Currency::USD);
    let z = Money::zero(currency);
    let (mut received, mut paid_out, mut fees) = (z, z, z);
    for e in entries {
        let bucket = match e.kind {
            WalletEntryKind::Capture | WalletEntryKind::Subscription => &mut received,
            WalletEntryKind::Refund | WalletEntryKind::Payout => &mut paid_out,
            WalletEntryKind::Fee => &mut fees,
        };
        *bucket = bucket
            .checked_add(e.amount)
            .map_err(|e| WalletError::Log(e.to_string()))?;
    }
    let net = received
        .checked_add(paid_out)
        .and_then(|m| m.checked_add(fees))
        .map_err(|e| WalletError::Log(e.to_string()))?;
    Ok(WalletTotals {
        received,
        paid_out,
        fees,
        net,
    })
}

fn write_secret(path: &std::path::Path, text: &str) -> Result<(), WalletError> {
    use std::io::Write;
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(log_err)?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(log_err)?;
    file.write_all(text.as_bytes()).map_err(log_err)
}
