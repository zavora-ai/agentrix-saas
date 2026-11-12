//! `agentrix-node`: edge, control and kernel in one process (specs/first-release/design.md §3, §4.1).
//!
//! Each module below belongs to one lane (design §3.0) and exposes a router or a plain API; the
//! integration lead wires them together in `app.rs`. Modules are public so a lane's code
//! compiles warning-free before it is wired.

pub mod app;
pub mod assess;
pub mod auth;
pub mod brief;
pub mod broker;
pub mod claim;
pub mod db;
pub mod decisions;
pub mod door;
pub mod guards;
pub mod launch;
pub mod objects;
pub mod pages;
pub mod routing;
pub mod site;
pub mod state;
pub mod wiring;
