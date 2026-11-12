//! Three clearly labelled test customer agents for the sample sites (James, Wed 11 Nov: "real
//! triggers plus a few clearly labelled test customer agents"), on sandbox PayPal only.
//!
//! Each agent has its own Web Bot Auth key directory and `did:web` on its own host
//! (`test-agent-<slug>.<platform host>`). Every 20–60 minutes it visits a sample site's agent
//! door over MCP, mostly the shops that sell ([`shops`]): it looks around anonymously with that
//! site's own words, then does what a cheap live model chooses from what fits there (a signed
//! quote or checkout within USD 5–40 for bakes or fragrances, a question, now and then a custom
//! cake, or a Supporter month at the newspaper), and about 2% of visits are an impostor's that the
//! door must refuse. Every place an agent shows
//! says "Test customer agent". The run stops by itself after `TEST_AGENTS_HOURS`, and the
//! operator stops it at once by creating `TEST_AGENTS_OFF_FILE` or `POST /control/stop`.

pub mod activity;
pub mod brain;
pub mod config;
pub mod identity;
pub mod mcp;
pub mod pay;
pub mod runner;
pub mod server;
pub mod shop;
pub mod shops;
