//! # saas-company: each company's AI staff
//!
//! Agentrix SaaS runs two companies, Agentrix (which sells its own WebAgent plans) and Kilele
//! Bakery (a customer with AI staff of its own). Each company's staff is a set of adk-rust agents
//! run by a resident kernel per company, autonomous inside the authority the owner signs:
//!
//! - **Chief** (Claude Sonnet) answers the founder.
//! - **Revenue** sells plans: ERA's CRM (lead to won), PayPal Subscriptions, recurring invoices.
//! - **Receivables** issues invoices, collects by ageing and credit limit, proposes debt plans
//!   and sends reminders.
//! - **Payables** pays ERA bills through PayPal Payouts inside the founder's mandate. A change of
//!   a vendor's payee is always held for the owner.
//! - **Support** handles cancellations and refunds. A refund only ever goes to the original
//!   payer, and never exceeds the capture.
//!
//! Every money tool runs on the governed path ([`governed`]): `adk_core::authorize_tool_call`,
//! a deny-by-default `DeclarativePolicy` from the company's policy file ([`policy`]),
//! fingerprint-bound approvals that an edit voids, the `ActionLedger` for exactly-once effects,
//! and the `GovernanceControl` freeze. Every decision is written to adk-auth's signed audit log
//! ([`audit`]).

pub mod access;
pub mod audit;
pub mod company;
pub mod desk;
#[cfg(feature = "effects")]
pub mod effects_rails;
pub mod flows;
pub mod governed;
pub mod identity;
pub mod judge;
pub mod mandate;
pub mod memory;
pub mod models;
pub mod policy;
pub mod rails;
pub mod runtime;
pub mod screen;
pub mod scripted;
pub mod staff;
pub mod stores;
pub mod sts;
pub mod supervisor;
pub mod tools;
