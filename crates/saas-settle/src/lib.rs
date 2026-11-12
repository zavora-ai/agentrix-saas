//! Agentrix SaaS settlement: two AI-run companies settle invoices with each other on PayPal,
//! autonomous inside the authority each owner signs.
//!
//! Agentrix (the payee) and Kilele Bakery (the payer) each run an agent door
//! ([`door::Door`]): A2A v1 JSON-RPC behind Web Bot Auth (RFC 9421 HTTP Message Signatures,
//! IETF working-group draft), with each company's agent keys published in its key directory
//! ([`keys`]).
//!
//! 1. Agentrix's Receivables agent offers Kilele an invoice from its books, sealed with its
//!    key ([`protocol::SettleMessage::InvoiceOffer`]). Kilele's door records the bill.
//! 2. Kilele's Payables agent signs a cart mandate for that invoice inside the intent mandate
//!    Kilele's owner approved with a passkey ([`mandate`]), and asks Agentrix's door to settle.
//! 3. Agentrix checks both mandates deterministically, claims the cart mandate (each moves
//!    money once), and captures from Kilele's saved PayPal wallet ([`rails::VaultRail`]), or
//!    accepts the payout Kilele already sent ([`rails::PayoutRail`], the fallback).
//! 4. Both companies' books post from the one PayPal id ([`books`]): Agentrix's receivable
//!    clears and Kilele's payable clears. Agentrix's sealed receipt carries the id.
//! 5. When Kilele is short, the agents agree an instalment plan inside both owners' mandates
//!    ([`protocol::fit_plan`] and the plan checks); each instalment becomes a scheduled
//!    settlement Agentrix collects on its due date with Kilele's pre-signed cart mandate.
//!
//! Mandates are AP2-style, not AP2 verifiable credentials; the checkout shape is ours.

pub mod attacks;
pub mod books;
pub mod door;
pub mod era;
pub mod events;
pub mod host;
pub mod keys;
pub mod mandate;
pub mod payee;
pub mod payer;
pub mod protocol;
pub mod rails;
pub mod sample_door;
pub mod sample_payments;
pub mod seal;
pub mod store;
pub mod tokens;

pub use door::{
    Door, DoorAuth, DoorClient, DoorHandler, DoorReply, Refusal, SignatureGate, VerifiedAgent,
};
pub use keys::{CompanyKeys, LoopbackDirectorySource};
pub use mandate::{CartMandate, CollectionsMandate, InstalmentBounds, IntentMandate};
pub use payee::{BoundsNegotiator, Negotiator, PayeeConfig, PayeeDesk};
pub use payer::{PayError, PayerConfig, PayerDesk, Settled};
pub use protocol::{
    DoorAnswer, InstalmentPlan, Invoice, PayPalSettlement, Route, SettleMessage, SettlementReceipt,
};
pub use sample_door::SampleDoorProof;
pub use sample_payments::SamplePayPal;
pub use seal::{SealKind, Sealed};
pub use store::{Store, migrate};
