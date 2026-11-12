//! Agent badges for Agentrix sites (specs/saas/v3.md, technology 1), and the PayPal binding
//! the owner approves with a passkey (technology 2).
//!
//! Who is this agent? Its identifier is a W3C DID with the did:web method,
//! `did:web:<site-host>:agents:<role>`, served from the site's own domain, so nothing depends
//! on a registry. Its keys are the ones the site already publishes in its Web Bot Auth key
//! directory and signs every agent-door request with. Its badge, signed by the site's
//! owner key, says who answers for it, the PayPal link and mandate it acts under, and its
//! wallet's last checkpoint. Agentrix (`did:web:<operator host>`) vouches that it hosts the
//! site and checked its PayPal link. Anyone can check all of it from the DID alone
//! ([`verify`]).
//!
//! - [`did`]: identifiers and DID documents.
//! - [`jws`]: detached JWS (EdDSA) over RFC 8785 JSON, for documents and A2A agent cards.
//! - [`resolve`]: [`resolve::IdentityResolver`], with `did:web`, `did:key` and key directories.
//! - [`site`]: a site's identity, badges, Agentrix's voucher, the signed card and routes.
//! - [`verify`]: the checks, the verify endpoint, and what the `agentrix-badge` command runs.
//! - [`binding`]: the PayPal binding, built only from PayPal's own Vault responses.
//! - [`ceremony`]: the owner's passkey moments, with the face's evidence checked offline.
//!
//! Agentrix hosts the sample sites, so one operator holds both a site's keys and the voucher's
//! key. No agent identity standard is ratified; these are did:web identifiers.

pub mod binding;
pub mod ceremony;
pub mod cli;
pub mod contract;
pub mod did;
pub mod jws;
pub mod resolve;
pub mod site;
pub mod verify;

pub use contract::{Badge, PayPalBinding};
pub use resolve::{IdentityResolver, Resolvers};
pub use site::{AgentSpec, Operator, SiteIdentity, SiteState};
pub use verify::{Verifier, VerifyReport};
