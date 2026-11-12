//! What the settlement desks tell each company's kernel, so its agents wake on events rather
//! than polling: a bill arrived (Kilele's Payables decides), money moved, a receipt posted, a
//! plan was agreed (Agentrix's kernel puts each instalment's due date on its wake list).
//! The desks keep no timers of their own.

use agentrix_types::Money;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::protocol::{Instalment, RefusalReceipt, SettlementReceipt};

/// One settlement event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SettleEvent {
    /// The payer's door recorded a bill offered by a payee (payer side).
    BillOffered {
        /// The payer's origin.
        payer: String,
        /// The payee's origin.
        payee: String,
        /// The payee's invoice number (the bill's vendor reference).
        invoice_id: String,
        /// The books' bill id.
        bill_id: String,
        /// What is owed.
        amount: Money,
        /// When it is due.
        due_on: NaiveDate,
        /// `will_settle`, `instalments` or `held`.
        decision: String,
        /// Why, in plain words.
        note: String,
    },
    /// Money moved and the payee sealed a receipt (payee side).
    Settled {
        /// The receipt's terms.
        receipt: SettlementReceipt,
    },
    /// The payer checked a receipt and its books posted it (payer side).
    ReceiptPosted {
        /// The receipt's terms.
        receipt: SettlementReceipt,
    },
    /// An instalment plan was agreed (both sides): each instalment is due on its date.
    PlanAgreed {
        /// The plan.
        plan_id: String,
        /// The invoice it pays (the payee's invoice number).
        invoice_id: String,
        /// The payee's books id for that invoice (payee side; `None` on the payer side).
        books_invoice_id: Option<String>,
        /// The payer's origin.
        payer: String,
        /// The payee's origin.
        payee: String,
        /// The instalments (1-based `index`).
        instalments: Vec<Instalment>,
    },
    /// A door or desk refused something and sealed a refusal receipt (either side).
    Refused {
        /// The receipt's terms.
        receipt: RefusalReceipt,
    },
}

/// The channel the desks publish on.
pub type EventSender = tokio::sync::broadcast::Sender<SettleEvent>;

/// A channel with room for a burst of events; slow listeners miss old ones, never block a desk.
pub fn channel() -> EventSender {
    tokio::sync::broadcast::channel(256).0
}
