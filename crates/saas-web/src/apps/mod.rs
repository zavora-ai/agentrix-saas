//! v3's console apps for one site: the owner's passkey moments (K), the agents' badges with a
//! Verify box (P), and the wallet with its three-way match (W). Spec: `specs/saas/v3.md`;
//! the shapes: the shared notes' `face-v3-contracts.md`.
//!
//! [`console_router`] serves one site's apps at relative paths. This crate nests it at
//! `/console/{site}` on its own hosts; Agentrix OS serves the same router as three console apps
//! (contract C9) at `/console/{site}/apps/{passkey,badges,wallet}` ([`crate::os`]):
//!
//! | Path | What |
//! |---|---|
//! | `/passkey` | The owner's passkey and the moments waiting for it |
//! | `/passkey/setup/options`, `POST /passkey/setup` | Setting up the owner's passkey (RP ID: this host) |
//! | `/passkey/link-paypal?binding=` | Link PayPal: the draft saas-badge's desk built, approved with the passkey |
//! | `/passkey/{moment}/{id}` | One moment: what will be signed, and "Sign with passkey" |
//! | `/passkey/{moment}/{id}/challenge`, `POST …/sign` | The ceremony: SHA-256(fingerprint ‖ nonce), then the seal |
//! | `/badges`, `/badges/verify` | Every agent's DID and badge, and the Verify box |
//! | `/wallet`, `/wallet/grid.json`, `/wallet/feed.json` | The wallet ledger and three-way match as AG Grid tables |
//!
//! JSON reads answer GET and POST alike. Scripts and styles are at `/static/saas/…`
//! ([`saas_assets`]), the AG Grid component at `/static/saas/wallet-grid.js`.
//!
//! [`public_router`] is what a sample site shows anyone: `/agents` (badges and Verify) and
//! `/agents/check?did=`.
//!
//! The work behind each page is another slice's, reached through a trait: [`Ceremonies`] and
//! [`Badges`] (settle), [`WalletFeed`] (books), [`DayRunner`] (company). Each has an HTTP
//! client ([`http`]) for two processes, and a labelled stand-in ([`demo`]) for local work.

pub mod demo;
pub mod http;
mod routes;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use saas_core::identity::Badge;
use saas_core::wallet::WalletFeed;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use routes::{JudgeKit, console_router, grid_view, judge_routes, public_router, saas_assets};

use crate::Access;
use crate::passkeys::Passkeys;

/// The owner's three passkey moments, and the only times the owner is prompted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Moment {
    /// Link the business's PayPal to its agents (once): a binding of the PayPal ids PayPal itself
    /// returned to the site, the owner's passkey and the agents allowed to use them.
    LinkPaypal,
    /// Sign the mandate, or raise it.
    Mandate,
    /// One step outside the mandate: a payment over it, a new payee, a payee's changed details,
    /// or unfreezing.
    StepUp,
}

impl Moment {
    /// `link-paypal`, `mandate`, `step-up`.
    pub fn as_str(self) -> &'static str {
        match self {
            Moment::LinkPaypal => "link-paypal",
            Moment::Mandate => "mandate",
            Moment::StepUp => "step-up",
        }
    }

    /// From a path segment.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "link-paypal" => Some(Moment::LinkPaypal),
            "mandate" => Some(Moment::Mandate),
            "step-up" => Some(Moment::StepUp),
            _ => None,
        }
    }

    /// The owner's words for it.
    pub fn words(self) -> &'static str {
        match self {
            Moment::LinkPaypal => "Link PayPal",
            Moment::Mandate => "Sign the mandate",
            Moment::StepUp => "Step up",
        }
    }
}

/// What the owner is asked to sign at one moment (settle's subject).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subject {
    /// Which moment.
    pub moment: Moment,
    /// The subject's id: a pending binding, a mandate draft, a decision.
    pub id: String,
    /// The site.
    pub site: String,
    /// `pending`, `sealed` or `expired`.
    pub state: String,
    /// One line: what will be signed.
    pub title: String,
    /// The facts, line by line (PayPal ids, limits, the payee).
    #[serde(default)]
    pub lines: Vec<String>,
    /// For a step-up: `outside-mandate`, `new-payee`, `payee-changed` or `unfreeze`.
    #[serde(default)]
    pub reason: Option<String>,
    /// `sha256:<64 hex>`: what the passkey signs (with a nonce).
    pub fingerprint: String,
    /// When it lapses.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// What sealing returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sealed {
    /// True once sealed.
    pub sealed: bool,
    /// The sealed record, as the owner of the moment wrote it.
    #[serde(default)]
    pub record: Value,
    /// Where it sits in the signed audit log.
    #[serde(default)]
    pub audit_seq: Option<u64>,
}

/// Why an app's call didn't go ahead. The words are safe to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// Nothing behind this page on this node.
    #[error("This isn't connected on this node yet.")]
    NotConnected,
    /// No such thing.
    #[error("Nothing here by that name.")]
    NotFound,
    /// What was to be signed changed after its fingerprint was issued: the approval is void.
    #[error("What you were asked to sign has changed since. Nothing was sealed; open it again.")]
    Changed,
    /// It lapsed.
    #[error("This has expired. Nothing was sealed.")]
    Expired,
    /// The other side said no, in its words.
    #[error("{0}")]
    Refused(String),
}

/// The owner's moments (settle): what each one signs, and sealing it.
#[async_trait]
pub trait Ceremonies: Send + Sync {
    /// What `moment` `id` of `site` asks the owner to sign.
    async fn subject(&self, site: &str, moment: Moment, id: &str) -> Result<Subject, AppError>;
    /// The owner signed it: `evidence` is the checked passkey assertion (see
    /// `PasskeyEvidence` in the contracts). Seal it.
    async fn seal(
        &self,
        site: &str,
        moment: Moment,
        id: &str,
        evidence: Value,
    ) -> Result<Sealed, AppError>;
    /// The moments waiting for the owner of `site`.
    async fn pending(&self, _site: &str) -> Result<Vec<Subject>, AppError> {
        Ok(Vec::new())
    }
}

/// One check of a verification (saas-badge's `Check`: `name`, `ok`, `detail`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// What was checked.
    #[serde(alias = "name")]
    pub check: String,
    /// Whether it held.
    pub ok: bool,
    /// What it found.
    #[serde(default)]
    pub detail: String,
}

/// The first check that failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    /// Which check.
    pub check: String,
    /// What went wrong.
    pub message: String,
}

/// A DID's verification (saas-badge's `VerifyReport`: `did`, `ok`, `checks`, `badge`,
/// `checked_at`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    /// The DID asked about.
    pub did: String,
    /// True when every check held.
    #[serde(alias = "ok")]
    pub verified: bool,
    /// Each check, in order.
    #[serde(default)]
    pub checks: Vec<Check>,
    /// The first that failed.
    #[serde(default)]
    pub failure: Option<Failure>,
    /// The badge it verified.
    #[serde(default)]
    pub badge: Value,
    /// When it was checked.
    #[serde(default, alias = "checked_at")]
    pub checked_at: Option<String>,
}

impl Verification {
    /// The same report with `failure` filled from its first failed check when the verifier
    /// didn't name one, and never `verified` with a failed check.
    pub fn settled(mut self) -> Self {
        let first = self.checks.iter().find(|c| !c.ok).cloned();
        if let Some(c) = &first {
            self.verified = false;
            if self.failure.is_none() {
                self.failure = Some(Failure {
                    check: c.check.clone(),
                    message: if c.detail.is_empty() {
                        format!("{} failed", c.check)
                    } else {
                        c.detail.clone()
                    },
                });
            }
        }
        if !self.verified && self.failure.is_none() {
            self.failure = Some(Failure {
                check: "verification".into(),
                message: "The verifier didn't say it verified.".into(),
            });
        }
        self
    }
}

/// Badges (settle, saas-badge): a site's agents' badges, and verifying a DID.
#[async_trait]
pub trait Badges: Send + Sync {
    /// The badges of `site`'s agents (`saas_core::identity::Badge` bodies; their
    /// signatures are what [`Badges::verify`] checks).
    async fn badges(&self, site: &str) -> Result<Vec<Badge>, AppError>;
    /// Resolves `did` and checks every signature.
    async fn verify(&self, did: &str) -> Result<Verification, AppError>;
    /// Said on the page when these badges aren't the real ones (a stand-in).
    fn note(&self) -> Option<String> {
        None
    }
    /// Checks an agent token a cloud or a site issued (saas-badge's
    /// `POST /agents/verify-token`): `{ok, principal, vouched_by, refused, badge, checked_at}`.
    async fn verify_token(&self, _token: &str) -> Result<Value, AppError> {
        Err(AppError::NotConnected)
    }
}

/// The wallets (books, saas-wallet): `site`'s wallet feed, entries, latest checkpoint and the
/// three-way match (`saas_core::wallet::WalletFeed`).
#[async_trait]
pub trait Wallets: Send + Sync {
    /// The wallet of `site`.
    async fn wallet(&self, site: &str) -> Result<WalletFeed, AppError>;
    /// Said on the page when this wallet isn't the real one (a stand-in).
    fn note(&self) -> Option<String> {
        None
    }
}

/// "Run the day" in judge mode (company): starts the story's run, and reads it back.
#[async_trait]
pub trait DayRunner: Send + Sync {
    /// Starts a run; its id.
    async fn start(&self) -> Result<String, AppError>;
    /// Where run `run` has got to: `{done, scenes: [{n, title, state, evidence}]}`.
    async fn status(&self, run: &str) -> Result<Value, AppError>;
}

/// Everything one site's console apps need.
#[derive(Clone)]
pub struct SiteApps {
    /// The site's id: `agentrix`, `kilele`, `classy`, `tana`.
    pub site: String,
    /// "Kilele Bakery".
    pub name: String,
    /// "Wanjiru".
    pub owner: String,
    /// Where these apps are mounted, for links (`/console/kilele`).
    pub base: String,
    /// The moments.
    pub ceremonies: Arc<dyn Ceremonies>,
    /// The badges.
    pub badges: Arc<dyn Badges>,
    /// The wallet.
    pub wallet: Arc<dyn Wallets>,
    /// Where saas-badge's PayPal binding router is mounted for this site
    /// (`{base}/paypal-link`): `start`, `returned`, `{binding}`, `{binding}/approve`.
    pub paypal_link: String,
    /// The owners' passkeys.
    pub passkeys: Arc<Passkeys>,
    /// Who may press the buttons.
    pub access: Access,
    /// Render only each page's field, for a host shell that draws the rest (Agentrix OS).
    pub fragment: bool,
}

/// `<repo>/web/saas-board/dist/assets/wallet-grid.js`, where `npm run build` writes it.
pub fn default_grid_js() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/saas-board/dist/assets/wallet-grid.js")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_read_and_write_as_the_contract_says() {
        for m in [Moment::LinkPaypal, Moment::Mandate, Moment::StepUp] {
            assert_eq!(Moment::parse(m.as_str()), Some(m));
            assert_eq!(serde_json::to_value(m).unwrap(), m.as_str());
        }
        assert_eq!(Moment::parse("pay-anyone"), None);
        let s: Subject = serde_json::from_value(serde_json::json!({
            "moment": "link-paypal", "id": "bind_1", "site": "kilele", "state": "pending",
            "title": "Link PayPal", "fingerprint": "sha256:00", "expiresAt": "2026-11-11T12:00:00Z"
        }))
        .unwrap();
        assert_eq!(s.moment, Moment::LinkPaypal);
        assert!(s.lines.is_empty());
    }
}
