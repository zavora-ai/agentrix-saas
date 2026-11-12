//! v3 wallet contracts (Technology 3): each site's append-only, tamper-evident record of what its
//! agents did with the business's PayPal account. As in adk-payments, the wallet is the
//! business's: each entry names the agent that acted, and an agent's "wallet" is its view of the
//! site's record (the Cashier's view is the whole record).
//!
//! The money stays in PayPal; PayPal has no per-agent accounts. A wallet is a **record**: say
//! "wallet record", never "holds funds".
//!
//! - A [`WalletEntry`] is written only from PayPal's own evidence ([`WalletSource`]): a verified
//!   webhook, or a read-back from PayPal. Never from the model, never from the request.
//! - A [`WalletCheckpoint`] signs the log's state and records a reconciliation against PayPal.
//! - A [`ThreeWayMatch`] lines one entry up with its PayPal object and its journal in the books.

use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

use crate::books::{BillId, InvoiceId, JournalId, PaymentId};
use crate::ids::{AgentId, CompanyId};
use crate::money::Money;
use crate::paypal::{PayPalOrderId, PayPalRef, PayPalSubscriptionId, PayPalWebhookEventId};

/// What an entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletEntryKind {
    /// Money captured from a customer (an order).
    Capture,
    /// Money given back.
    Refund,
    /// Money paid out to a payee.
    Payout,
    /// A subscription's billing-cycle payment.
    Subscription,
    /// PayPal's fee on another entry's object (negative), or a fee PayPal gave back (positive).
    Fee,
}

/// Where an entry's facts came from. Nothing else may write one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum WalletSource {
    /// A webhook whose signature PayPal's verify-webhook-signature accepted.
    Webhook {
        /// The event.
        event_id: PayPalWebhookEventId,
        /// Its type, for example `PAYMENT.CAPTURE.COMPLETED`.
        event_type: String,
    },
    /// A read-back of the object from PayPal's API.
    ReadBack {
        /// The call, for example `GET /v2/payments/captures/2H853595A8077800U`.
        call: String,
    },
}

/// What caused the money to move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum WalletCause {
    /// A customer's order.
    Order {
        /// The PayPal order.
        order: PayPalOrderId,
        /// The site's own order reference, when there is one.
        reference: Option<String>,
    },
    /// An invoice in the books.
    Invoice {
        /// The invoice.
        invoice: InvoiceId,
        /// Its number, for example `INV-2026-0006`.
        number: Option<String>,
    },
    /// A bill in the books.
    Bill {
        /// The bill.
        bill: BillId,
        /// Its number.
        number: Option<String>,
    },
    /// A subscription.
    Subscription {
        /// The PayPal subscription.
        subscription: PayPalSubscriptionId,
    },
}

/// The authority the money moved under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "authority", rename_all = "snake_case")]
pub enum Authority {
    /// Inside the owner's signed mandate, with the cart mandate the agent signed for this payment.
    Mandate {
        /// `sha256:` of the owner's mandate.
        mandate_digest: String,
        /// `sha256:` of the agent's cart mandate, when there is one.
        cart_mandate_digest: Option<String>,
    },
    /// Approved by a person for this one call (a passkey step-up or a decision card).
    Approval {
        /// The approved call's fingerprint.
        fingerprint: String,
        /// Who approved.
        approved_by: String,
    },
    /// Money in that the payer approved on PayPal's own page (a customer's checkout, a
    /// subscriber's approval): nothing of ours had to authorise it.
    PayerApproved,
}

/// Who the money moved to or from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletCounterparty {
    /// Their name.
    pub name: String,
    /// Their PayPal email, as PayPal reported it.
    pub email: Option<String>,
    /// Set when the counterparty is itself a company on the platform.
    pub company: Option<CompanyId>,
}

/// One entry in an agent's wallet record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletEntry {
    /// The site's wallet, named by its holder, the site's Cashier (`<company>/cashier`).
    pub wallet: AgentId,
    /// Its place in the wallet, from 0: the signed log's sequence number.
    pub seq: u64,
    /// When PayPal says it happened.
    pub at: DateTime<Utc>,
    /// What it records.
    pub kind: WalletEntryKind,
    /// Signed: money in is positive, money out (refunds, payouts, fees) negative.
    pub amount: Money,
    /// The PayPal object. A fee entry names the object the fee was charged on.
    pub paypal: PayPalRef,
    /// PayPal's status for it, as read, for example `COMPLETED`.
    pub paypal_status: String,
    /// Who the money moved to or from.
    pub counterparty: Option<WalletCounterparty>,
    /// What caused it.
    pub cause: Option<WalletCause>,
    /// The agent that acted: the attribution an agent's view of the record filters on.
    pub acting: AgentId,
    /// The authority behind it.
    pub authority: Authority,
    /// Where the facts came from.
    pub source: WalletSource,
}

impl WalletEntry {
    /// What makes an entry unique in its wallet: the PayPal object's key, plus `:fee` for a fee.
    /// Recording the same key again records nothing.
    pub fn key(&self) -> String {
        Self::key_for(self.kind, &self.paypal)
    }

    /// [`WalletEntry::key`] for an entry not written yet.
    pub fn key_for(kind: WalletEntryKind, paypal: &PayPalRef) -> String {
        match kind {
            WalletEntryKind::Fee => paypal.fee_key(),
            _ => paypal.key(),
        }
    }
}

/// How a checkpoint compared the wallet with PayPal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileMethod {
    /// Every entry's PayPal object read back.
    ReadBack,
    /// PayPal's balances report (`/v1/reporting/balances`).
    BalancesReport,
}

/// Whether the wallet agrees with PayPal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Every entry matches what PayPal says.
    Matches,
    /// At least one doesn't: see the drift.
    Drift,
}

/// One difference between the wallet and PayPal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drift {
    /// The PayPal object.
    pub paypal: PayPalRef,
    /// What differs, in plain words: "PayPal says REFUNDED", "PayPal has USD 28.00".
    pub what: String,
    /// The wallet entries it touches, by sequence number (none when PayPal has an entry the
    /// wallet lacks entirely).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<u64>,
}

/// A reconciliation against PayPal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reconciliation {
    /// When PayPal was read.
    pub checked_at: DateTime<Utc>,
    /// How.
    pub method: ReconcileMethod,
    /// How many PayPal objects were read.
    pub objects: u32,
    /// The result.
    pub state: SyncState,
    /// What differs, when it drifts.
    pub drift: Vec<Drift>,
}

/// A wallet's totals, by direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletTotals {
    /// Captures and subscription payments.
    pub received: Money,
    /// Refunds and payouts (negative).
    pub paid_out: Money,
    /// PayPal's fees, net of fees returned (negative).
    pub fees: Money,
    /// The sum.
    pub net: Money,
}

/// A signed checkpoint of a site's wallet record, with its reconciliation against PayPal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletCheckpoint {
    /// The site's wallet, named by its holder, the site's Cashier.
    pub wallet: AgentId,
    /// Records covered.
    pub size: u64,
    /// The Merkle root over them (RFC 9162), as the signed log writes it.
    pub root_hash: String,
    /// The signed checkpoint note; checks against [`WalletCheckpoint::verifier_key`].
    pub note: String,
    /// The key that verifies the note, `<name>+<hash>+<base64>`.
    pub verifier_key: String,
    /// When it was made.
    pub at: DateTime<Utc>,
    /// The totals over the entries covered.
    pub totals: WalletTotals,
    /// The reconciliation it records.
    pub reconciliation: Reconciliation,
}

/// The few words a badge and the board show for a wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletCheckpointSummary {
    /// Entries covered.
    pub size: u64,
    /// The Merkle root.
    pub root_hash: String,
    /// The reconciliation's result.
    pub state: SyncState,
    /// When PayPal was read.
    pub checked_at: DateTime<Utc>,
    /// "Matches PayPal as of 14:02 EAT", or "Drift from PayPal as of 14:02 EAT (2 objects)".
    pub words: String,
}

/// East Africa Time, which the demo's words use.
fn eat() -> FixedOffset {
    FixedOffset::east_opt(3 * 3600).unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"))
}

impl WalletCheckpoint {
    /// The checkpoint in a few words, for a badge and the board.
    pub fn summary(&self) -> WalletCheckpointSummary {
        let r = &self.reconciliation;
        let when = r.checked_at.with_timezone(&eat()).format("%H:%M EAT");
        let words = match r.state {
            SyncState::Matches => format!("Matches PayPal as of {when}"),
            SyncState::Drift => {
                format!("Drift from PayPal as of {when} ({} objects)", r.drift.len())
            }
        };
        WalletCheckpointSummary {
            size: self.size,
            root_hash: self.root_hash.clone(),
            state: r.state,
            checked_at: r.checked_at,
            words,
        }
    }

    /// The reference a badge carries (slice P's `WalletCheckpointRef`).
    pub fn reference(&self) -> WalletCheckpointRef {
        WalletCheckpointRef {
            stream: format!("wallet:{}", self.wallet.company()),
            sequence: self.size,
            root: format!("sha256:{}", self.root_hash),
            at: self.at,
            reconciled: match self.reconciliation.state {
                SyncState::Matches => Reconciled::Matches,
                SyncState::Drift => Reconciled::Drift,
            },
            reconciled_at: Some(self.reconciliation.checked_at),
        }
    }
}

/// Whether a checkpoint agreed with PayPal, as a badge says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reconciled {
    /// It matched PayPal.
    Matches,
    /// It drifted from PayPal.
    Drift,
    /// It hasn't been compared yet.
    Unchecked,
}

/// A wallet's latest checkpoint, as a badge cites it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletCheckpointRef {
    /// The site's signed log stream: `wallet:<company>`.
    pub stream: String,
    /// The checkpoint's size (records covered).
    pub sequence: u64,
    /// `sha256:<hex>` Merkle root.
    pub root: String,
    /// When the checkpoint was signed.
    pub at: DateTime<Utc>,
    /// The reconciliation's result.
    pub reconciled: Reconciled,
    /// When PayPal was read.
    pub reconciled_at: Option<DateTime<Utc>>,
}

/// An entry's link into the books.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BooksLink {
    /// The receipt or payment.
    pub payment: Option<PaymentId>,
    /// The journal it posted (for ERA: the payment's journal entry, or a refund's journal).
    pub journal: Option<JournalId>,
    /// The fee journal.
    pub fee_journal: Option<JournalId>,
    /// The payment's number in the books, for example `PAY-2026-0009`.
    #[serde(default)]
    pub payment_number: Option<String>,
    /// The journal's number, for example `JE-2026-0029`.
    #[serde(default)]
    pub journal_number: Option<String>,
    /// The fee journal's number.
    #[serde(default)]
    pub fee_journal_number: Option<String>,
}

/// How one entry lines up across the three records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchState {
    /// Wallet, PayPal and the books agree.
    Matched,
    /// PayPal and the wallet agree; the books have nothing for it yet.
    NotInBooks,
    /// PayPal has no such object, or a different status or amount.
    #[serde(rename = "paypal_differs")]
    PayPalDiffers,
    /// The books carry a different amount.
    BooksDiffer,
}

/// One row of the three-way match: wallet entry, PayPal object, journal in the books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreeWayMatch {
    /// The wallet entry's sequence.
    pub seq: u64,
    /// Its kind.
    pub kind: WalletEntryKind,
    /// Its amount.
    pub amount: Money,
    /// The PayPal object.
    pub paypal: PayPalRef,
    /// PayPal's status for it at the last read, when read.
    pub paypal_status: Option<String>,
    /// What PayPal had for it at the last read (signed as the entry is).
    pub paypal_amount: Option<Money>,
    /// The books' side.
    pub books: BooksLink,
    /// What the books carry for it (the payment, or the fee journal), when posted.
    pub books_amount: Option<Money>,
    /// How they line up.
    pub state: MatchState,
}

/// What the board's AG Grid tables read for one wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletFeed {
    /// Whose view: the Cashier's is the site's whole record; a desk's is the entries it acted on.
    /// The checkpoint and history are the site's either way.
    pub wallet: AgentId,
    /// The latest checkpoint, once there is one.
    pub checkpoint: Option<WalletCheckpointSummary>,
    /// Every reconciliation, oldest first.
    pub history: Vec<WalletCheckpointSummary>,
    /// Every entry, oldest first.
    pub entries: Vec<WalletEntry>,
    /// The three-way match, one row per entry.
    pub matches: Vec<ThreeWayMatch>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::Currency;

    #[test]
    fn keys_and_words() {
        let capture = PayPalRef::Capture("2H853595A8077800U".parse().unwrap());
        assert_eq!(
            WalletEntry::key_for(WalletEntryKind::Capture, &capture),
            "paypal:capture:2H853595A8077800U"
        );
        assert_eq!(
            WalletEntry::key_for(WalletEntryKind::Fee, &capture),
            "paypal:capture:2H853595A8077800U:fee"
        );
        let zero = Money::zero(Currency::USD);
        let cp = WalletCheckpoint {
            wallet: AgentId::new(CompanyId::kilele(), "cashier").unwrap(),
            size: 2,
            root_hash: "abc".into(),
            note: "note".into(),
            verifier_key: "k".into(),
            at: "2026-11-11T11:02:00Z".parse().unwrap(),
            totals: WalletTotals {
                received: zero,
                paid_out: zero,
                fees: zero,
                net: zero,
            },
            reconciliation: Reconciliation {
                checked_at: "2026-11-11T11:02:00Z".parse().unwrap(),
                method: ReconcileMethod::ReadBack,
                objects: 1,
                state: SyncState::Matches,
                drift: vec![],
            },
        };
        assert_eq!(cp.summary().words, "Matches PayPal as of 14:02 EAT");
        let json = serde_json::to_value(WalletSource::Webhook {
            event_id: "WH-1".parse().unwrap(),
            event_type: "PAYMENT.CAPTURE.COMPLETED".into(),
        })
        .unwrap();
        assert_eq!(json["source"], "webhook");
    }
}
