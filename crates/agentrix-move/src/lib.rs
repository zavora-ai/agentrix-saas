//! The Move: a persisted, resumable state machine (design §6), tasks S1-06 and S2-01's card.
//!
//! Claimed → Replayed → Cleaned → Rebuilt → Proved → AwaitingOwner, then Live only by the
//! owner's approval through the node's LaunchGate. Built on adk-site's `migrate` (the WXR
//! source, sanitiser, detectors and parity helpers), `release` (manifest digests) and `scan`
//! (server code), and on awp-types' release manifest.
//!
//! | Step | Module | Requirement |
//! |---|---|---|
//! | Replay: inventory from a replay room, or the WXR export, uploads and crawl | [`inventory`] | R4.1, R4.4 |
//! | Clean: sanitise, detect, screen, quarantine | [`clean`] | R5.1–R5.3, R5.5 |
//! | Rebuild: the builder model by purpose, ≤3 tries per page | [`rebuild`], [`theme`] | R6.1, R6.3 |
//! | Prove: render in a sealed room, parity per page and site | [`parity`] | R6.2, R6.4 |
//! | Candidate: manifest and files in the CAS | [`candidate`] | design §4.5 |
//! | Go-live card and charter v1 | [`golive`] | R7.1, R7.5 |
//! | Persistence and resume | [`store`], [`pipeline`] | N9, R6.5 |
//!
//! Every dependency on the rest of the node is a trait in [`ports`]; [`testing`] holds fakes.

pub mod candidate;
pub mod clean;
pub mod gateway;
pub mod golive;
pub mod inventory;
pub mod parity;
pub mod pipeline;
pub mod ports;
pub mod rebuild;
pub mod state;
pub mod store;
pub mod testing;
pub mod theme;

pub use inventory::MoveInputs;
pub use pipeline::Mover;
pub use state::MoveState;
pub use store::MoveStore;
