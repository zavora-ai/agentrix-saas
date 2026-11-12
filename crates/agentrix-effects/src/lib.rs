//! The Effects Gateway, first slice (02 §4.3, ADR-004, ADR-006, contract C3): money moves only
//! through here, once per fingerprint, with a receipt, and never with card data on our hosts.
//! Owner: W6 (identity, payments and trust). PayPal only (James, Sun 8 Nov), sandbox.
//!
//! - **Checkout** for the sample sites ([`Effects::create_payment`], [`Effects::capture`]):
//!   PayPal Orders v2. The customer approves on PayPal's own page; we keep PayPal's ids and
//!   the amounts. The same site + order + amount gives the same PayPal order, and a second
//!   capture returns the first result.
//! - **Refunds** are owner decisions: [`Effects::refund_request`] builds the `EffectRequest`
//!   W5 puts on a card; [`Effects::refund`] runs only with the approval of that exact
//!   fingerprint, and never above what was captured.
//! - **Payouts** to partners ([`Effects::payout_request`], [`Effects::payout`]): PayPal
//!   Payouts, money leaving the company, so each runs only with the approval of its exact
//!   fingerprint (the founder's mandate or an owner's passkey), once; the fingerprint is the
//!   batch's `sender_batch_id`.
//! - **Plans** in USD ([`Effects::start_subscription`]): PayPal Subscriptions, with the
//!   provider's side kept in `plan_subscriptions` for W3's billing.
//! - **Webhooks** (`POST /effects/paypal/webhook` on the platform host): only events PayPal's
//!   signature check accepts, each handled once.
//! - Every outcome is `executed`, `not_executed` or `unknown` (U27); unknown is settled by
//!   reading PayPal back ([`Effects::reconcile`]), never by retrying blind.
//! - **Edge cases** for the ecosystem run (`specs/saas/paypal-edge-cases.md`): list the
//!   webhook events PayPal sent and resend one to our own webhook, cancel an unclaimed payout
//!   item, and PayPal's sandbox error simulation on Orders v2 and Payments v2
//!   ([`Effects::simulating`]), each kept in `paypal_edge_cases`.
//!
//! Without the sandbox keys (`AGENTRIX_PAYPAL_FILE` empty or missing) every call answers
//! [`EffectError::NotConfigured`] with "Payments aren't switched on for this sample yet.
//! Nothing was charged." Horizon 2 moves this crate into its own process and user, the only
//! holder of provider keys.

mod edge_cases;
mod journal;
mod payments;
mod payouts;
pub mod paypal;
mod subscriptions;
mod webhook;

use std::path::PathBuf;
use std::sync::Arc;

use agentrix_host::Receipts;
use agentrix_types::{Digest, Money, ReceiptBody, ReceiptKind, SiteId, Subject, canonical_json};
use serde::{Deserialize, Serialize};

pub use payments::{
    CaptureOutcome, CaptureState, Payment, PaymentRequest, PaymentStarted, RefundOutcome,
};
pub use payouts::{PAYOUT_TOOL, Payout, PayoutBatch, PayoutItem, PayoutOutcome, PayoutRequest};
pub use edge_cases::{EdgeCase, WebhookEvent, WebhookEventsQuery};
pub use paypal::{MockResponse, PayPalClient, PayPalConfig, SANDBOX_ERROR_SIMULATION};
pub use subscriptions::{PlanPrice, PlanSubscription, SubscriptionStart, SubscriptionStarted};

/// The name of this crate's migrations table.
pub const MIGRATIONS_TABLE: &str = "_sqlx_migrations_effects";

/// Applies this crate's migrations (0610–0619 of W6's range) to `pool`.
pub fn migrate(pool: &sqlx::SqlitePool) -> futures::future::BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        let mut migrator = sqlx::migrate!("./migrations");
        migrator.dangerous_set_table_name(MIGRATIONS_TABLE);
        migrator.set_ignore_missing(true);
        migrator.run(pool).await?;
        Ok(())
    })
}

/// How an effect ended (U27). `Unknown` is never retried blind: it is reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It happened.
    Executed,
    /// It did not happen; nothing moved.
    NotExecuted,
    /// We can't tell yet (a timeout, a PayPal 5xx). Reconcile before doing anything else.
    Unknown,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::NotExecuted => "not_executed",
            Self::Unknown => "unknown",
        }
    }
}

/// Errors, each with words a customer or owner can read.
#[derive(Debug, thiserror::Error)]
pub enum EffectError {
    /// No sandbox keys on this node.
    #[error("Payments aren't switched on for this sample yet. Nothing was charged.")]
    NotConfigured,
    /// PayPal refused; nothing moved. The text says why in plain words.
    #[error("{0}")]
    NotExecuted(String),
    /// We couldn't tell whether it ran; reconcile.
    #[error(
        "We couldn't confirm the payment with PayPal yet. Nothing will be charged twice; check again in a minute."
    )]
    Unknown(String),
    /// The request was wrong.
    #[error("{0}")]
    BadRequest(String),
    /// No such payment or subscription on this site.
    #[error("That payment isn't on this site.")]
    NotFound,
    /// The approval doesn't match this exact effect.
    #[error(
        "This isn't approved. The owner approves each refund or payout, or signs the mandate that covers it, with a passkey or an authenticator code."
    )]
    NotApproved,
    /// Ours.
    #[error("Something went wrong on our side. Nothing was charged.")]
    Internal(String),
}

impl From<sqlx::Error> for EffectError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

/// The gateway. Cheap to clone.
#[derive(Clone)]
pub struct Effects {
    inner: Arc<Inner>,
    /// PayPal's sandbox error simulation, on this copy only ([`Effects::simulating`]).
    simulation: Option<Arc<MockResponse>>,
}

struct Inner {
    pool: sqlx::SqlitePool,
    client: Option<Arc<PayPalClient>>,
    receipts: Arc<dyn Receipts>,
}

impl std::fmt::Debug for Effects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Effects")
            .field(
                "paypal",
                &self.inner.client.as_ref().map(|c| c.environment().clone()),
            )
            .field("simulation", &self.simulation.as_deref().map(MockResponse::code))
            .finish()
    }
}

impl Effects {
    /// A gateway over `pool` (migrated with [`migrate`]), with PayPal when `client` is set.
    pub fn new(
        pool: sqlx::SqlitePool,
        client: Option<Arc<PayPalClient>>,
        receipts: Arc<dyn Receipts>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                client,
                receipts,
            }),
            simulation: None,
        }
    }

    /// A copy whose Orders v2 and Payments v2 calls carry PayPal's sandbox error simulation for
    /// `mock`'s code (`PayPal-Mock-Response`), sandbox only. Every such call is kept in
    /// `paypal_edge_cases` as a "sandbox error simulation", and every receipt this copy signs
    /// says so. Other calls go out as usual.
    pub fn simulating(&self, mock: MockResponse) -> Result<Self, EffectError> {
        if !self.sandbox() {
            return Err(EffectError::BadRequest(
                "The sandbox error simulation never runs against live PayPal.".into(),
            ));
        }
        Ok(Self {
            inner: self.inner.clone(),
            simulation: Some(Arc::new(mock)),
        })
    }

    /// The simulation this copy carries, if any.
    pub fn simulation(&self) -> Option<&MockResponse> {
        self.simulation.as_deref()
    }

    /// For node: PayPal from the secret file named by `AGENTRIX_PAYPAL_FILE` (systemd's
    /// credential copy of `/etc/agentrix/secrets/paypal.json`); off when it is unset, missing
    /// or empty.
    pub fn for_node(ctx: &agentrix_host::NodeContext) -> anyhow::Result<Self> {
        let config = match std::env::var_os("AGENTRIX_PAYPAL_FILE").map(PathBuf::from) {
            Some(path) => PayPalConfig::from_file(&path)?,
            None => None,
        };
        match &config {
            Some(c) => tracing::info!(environment = ?c.environment, "effects: PayPal on"),
            None => tracing::info!("effects: PayPal off (no sandbox keys); checkout says so"),
        }
        let client = config.map(|c| Arc::new(PayPalClient::new(c, ctx.http.clone())));
        Ok(Self::new(ctx.pool.clone(), client, ctx.receipts.clone()))
    }

    /// Whether PayPal is switched on.
    pub fn enabled(&self) -> bool {
        self.inner.client.is_some()
    }

    /// Whether this is PayPal's sandbox (always, in Horizon 1).
    pub fn sandbox(&self) -> bool {
        self.inner
            .client
            .as_ref()
            .is_none_or(|c| !c.environment().is_live())
    }

    fn client(&self) -> Result<&PayPalClient, EffectError> {
        self.inner
            .client
            .as_deref()
            .ok_or(EffectError::NotConfigured)
    }

    pub(crate) fn pool(&self) -> &sqlx::SqlitePool {
        &self.inner.pool
    }

    /// The pool, for tests that read the tables W3 reads.
    #[doc(hidden)]
    pub fn pool_for_tests(&self) -> &sqlx::SqlitePool {
        &self.inner.pool
    }

    /// The platform webhook route.
    pub fn router(&self) -> axum::Router {
        webhook::router(self.clone())
    }

    /// Signs a receipt for an effect with control's key; a failure is logged, the effect stands.
    pub(crate) async fn receipt(
        &self,
        kind: ReceiptKind,
        site: &SiteId,
        detail: serde_json::Value,
    ) -> Option<String> {
        let now = chrono::Utc::now();
        let mut body = ReceiptBody::new(kind, site.clone(), Subject::Site(site.clone()), now, now);
        let mut detail = detail;
        if let (Some(mock), Some(fields)) = (self.simulation.as_deref(), detail.as_object_mut()) {
            fields.insert("simulation".into(), SANDBOX_ERROR_SIMULATION.into());
            fields.insert("mock_application_code".into(), mock.code().into());
        }
        body.detail = Some(detail);
        match self.inner.receipts.issue(body).await {
            Ok(digest) => Some(digest),
            Err(e) => {
                tracing::warn!(error = %e, "effects: receipt not issued");
                None
            }
        }
    }
}

/// `sha256:…` of the canonical JSON of `value`: an effect's idempotency key.
pub fn fingerprint<T: Serialize>(value: &T) -> Result<Digest, EffectError> {
    Ok(Digest::of(
        &canonical_json(value).map_err(|e| EffectError::Internal(e.to_string()))?,
    ))
}

/// "USD 18.50": money as the owner reads it in receipts and errors.
pub fn words(money: &Money) -> String {
    format!(
        "{} {}.{:02}",
        money.currency,
        money.minor / 100,
        (money.minor % 100).abs()
    )
}

/// Mounts the gateway: the PayPal webhook on the platform host. W2 and W3 call [`Effects`]
/// in process through [`Effects::for_node`].
pub fn mount(ctx: &agentrix_host::NodeContext) -> anyhow::Result<agentrix_host::Mounted> {
    let effects = Effects::for_node(ctx)?;
    Ok(agentrix_host::Mounted::default().platform(effects.router()))
}

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
