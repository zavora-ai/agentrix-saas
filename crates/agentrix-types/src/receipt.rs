//! Receipts (design §4.4): signed JSON, content-addressed.

use chrono::{DateTime, Utc};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::digest::Digest;
use crate::error::{Error, Result};
use crate::ids::{AssessmentId, DecisionId, JobId, MoveId, ReceiptId, SiteId};
use crate::job::{Caps, JobEvent, JobResult, JobSpec, Peaks, Refusal};
use crate::sign::{KeyId, KeySet, Signature, Signer, canonical_json, verify};

/// What a receipt records (design §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReceiptKind {
    /// A fetch job finished.
    #[serde(rename = "job.fetch")]
    JobFetch,
    /// A replay job finished.
    #[serde(rename = "job.replay")]
    JobReplay,
    /// A render job finished.
    #[serde(rename = "job.render")]
    JobRender,
    /// A build job finished.
    #[serde(rename = "job.build")]
    JobBuild,
    /// A Move step finished.
    #[serde(rename = "move.step")]
    MoveStep,
    /// Ownership of a site was proved.
    #[serde(rename = "claim.verified")]
    ClaimVerified,
    /// The owner approved a decision card.
    #[serde(rename = "decision.approved")]
    DecisionApproved,
    /// The owner declined a decision card.
    #[serde(rename = "decision.declined")]
    DecisionDeclined,
    /// A release was signed.
    #[serde(rename = "release.signed")]
    ReleaseSigned,
    /// The live pointer moved back to an earlier release.
    #[serde(rename = "release.rollback")]
    ReleaseRollback,
    /// The owner restored a quarantined item.
    #[serde(rename = "quarantine.restored")]
    QuarantineRestored,
    /// The Concierge refused something it has no authority to do.
    #[serde(rename = "concierge.refusal")]
    ConciergeRefusal,
    /// An order from the agent door was held for the owner.
    #[serde(rename = "door.order-held")]
    DoorOrderHeld,
    /// A gauntlet run finished.
    #[serde(rename = "gauntlet.run")]
    GauntletRun,
    /// A provider or model route changed (R11.4, R11.6): the routes before and after.
    #[serde(rename = "routing.changed")]
    RoutingChanged,
    /// A customer gave consent (platform W6).
    #[serde(rename = "consent.given")]
    ConsentGiven,
    /// A customer withdrew consent (platform W6).
    #[serde(rename = "consent.withdrawn")]
    ConsentWithdrawn,
    /// A payment was started with the provider (platform W6).
    #[serde(rename = "payment.started")]
    PaymentStarted,
    /// A payment was captured (platform W6).
    #[serde(rename = "payment.captured")]
    PaymentCaptured,
    /// A payment was refunded (platform W6).
    #[serde(rename = "payment.refunded")]
    PaymentRefunded,
    /// A plan subscription changed (platform W6).
    #[serde(rename = "subscription.changed")]
    SubscriptionChanged,
    /// A partner was paid through PayPal Payouts (Agentrix SaaS).
    #[serde(rename = "payout.sent")]
    PayoutSent,
    /// An operator acted on a site: support access used, break-glass opened or closed (08 §2.5).
    #[serde(rename = "operator.access")]
    OperatorAccess,
    /// A platform hold was placed on or lifted from a site (08 §2.4).
    #[serde(rename = "platform.hold")]
    PlatformHold,
    /// An owner signed in with an email code or a recovery code (platform W6).
    #[serde(rename = "account.code-sign-in")]
    AccountCodeSignIn,
    /// Someone asked to recover an owner account by email; held before it completes (W6).
    #[serde(rename = "account.recovery-hold")]
    AccountRecoveryHold,
    /// An owner's team changed: someone invited, accepted, given another role or removed (W6).
    #[serde(rename = "team.changed")]
    TeamChanged,
    /// An owner linked a chat (Telegram) to their site or unlinked it (platform W13).
    #[serde(rename = "channel.changed")]
    ChannelChanged,
    /// The concept stage: a designer's finished page, its sha256 as the output.
    #[serde(rename = "concept.page")]
    ConceptPage,
    /// The concept stage: the look the owner chose before the Move.
    #[serde(rename = "concept.chosen")]
    ConceptChosen,
}

impl ReceiptKind {
    /// Every kind, in the order design §4.4 lists them.
    pub const ALL: [Self; 30] = [
        Self::JobFetch,
        Self::JobReplay,
        Self::JobRender,
        Self::JobBuild,
        Self::MoveStep,
        Self::ClaimVerified,
        Self::DecisionApproved,
        Self::DecisionDeclined,
        Self::ReleaseSigned,
        Self::ReleaseRollback,
        Self::QuarantineRestored,
        Self::ConciergeRefusal,
        Self::DoorOrderHeld,
        Self::GauntletRun,
        Self::RoutingChanged,
        Self::ConsentGiven,
        Self::ConsentWithdrawn,
        Self::PaymentStarted,
        Self::PaymentCaptured,
        Self::PaymentRefunded,
        Self::SubscriptionChanged,
        Self::PayoutSent,
        Self::OperatorAccess,
        Self::PlatformHold,
        Self::AccountCodeSignIn,
        Self::AccountRecoveryHold,
        Self::TeamChanged,
        Self::ChannelChanged,
        Self::ConceptPage,
        Self::ConceptChosen,
    ];

    /// The kind as stored and filtered on, for example `job.fetch`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JobFetch => "job.fetch",
            Self::JobReplay => "job.replay",
            Self::JobRender => "job.render",
            Self::JobBuild => "job.build",
            Self::MoveStep => "move.step",
            Self::ClaimVerified => "claim.verified",
            Self::DecisionApproved => "decision.approved",
            Self::DecisionDeclined => "decision.declined",
            Self::ReleaseSigned => "release.signed",
            Self::ReleaseRollback => "release.rollback",
            Self::QuarantineRestored => "quarantine.restored",
            Self::ConciergeRefusal => "concierge.refusal",
            Self::DoorOrderHeld => "door.order-held",
            Self::GauntletRun => "gauntlet.run",
            Self::RoutingChanged => "routing.changed",
            Self::ConsentGiven => "consent.given",
            Self::ConsentWithdrawn => "consent.withdrawn",
            Self::PaymentStarted => "payment.started",
            Self::PaymentCaptured => "payment.captured",
            Self::PaymentRefunded => "payment.refunded",
            Self::SubscriptionChanged => "subscription.changed",
            Self::PayoutSent => "payout.sent",
            Self::OperatorAccess => "operator.access",
            Self::PlatformHold => "platform.hold",
            Self::AccountCodeSignIn => "account.code-sign-in",
            Self::AccountRecoveryHold => "account.recovery-hold",
            Self::TeamChanged => "team.changed",
            Self::ChannelChanged => "channel.changed",
            Self::ConceptPage => "concept.page",
            Self::ConceptChosen => "concept.chosen",
        }
    }

    /// Parses a stored kind.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }
}

/// What a receipt is about, written `{"type": "job", "id": "job_…"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Subject {
    /// An assessment.
    Assessment(AssessmentId),
    /// A Move.
    Move(MoveId),
    /// A sealed-room job.
    Job(JobId),
    /// A decision card.
    Decision(DecisionId),
    /// The site as a whole (claims, rollbacks, gauntlet runs).
    Site(SiteId),
    /// An owner's account, by its id (sign-ins and recovery; the receipt's site is `platform`).
    /// Never an email address.
    Account(String),
}

/// The policy a job ran under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// The egress allowlist.
    pub allowlist: Vec<String>,
    /// The resource caps.
    pub caps: Caps,
    /// The room runtime, for example `runsc`.
    pub runtime: String,
}

/// What the runner observed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observed {
    /// Hosts contacted.
    pub hosts: Vec<String>,
    /// Requests allowed.
    pub requests: u64,
    /// Bytes received.
    pub bytes: u64,
    /// Requests refused.
    pub refusals: Vec<Refusal>,
    /// Peak resource use.
    pub peaks: Peaks,
}

/// Everything a receipt says, before and apart from its signature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReceiptBody {
    /// The receipt.
    pub receipt_id: ReceiptId,
    /// What it records.
    pub kind: ReceiptKind,
    /// The site.
    pub site: SiteId,
    /// What it is about.
    pub subject: Subject,
    /// Input digests.
    #[serde(default)]
    pub inputs: Vec<Digest>,
    /// The policy a job ran under (job receipts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<Policy>,
    /// What the runner observed (job receipts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Observed>,
    /// Output digests.
    #[serde(default)]
    pub outputs: Vec<Digest>,
    /// Whether the room was destroyed and confirmed (job receipts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destroyed: Option<bool>,
    /// When the work started.
    pub started_at: DateTime<Utc>,
    /// When the work ended.
    pub ended_at: DateTime<Utc>,
    /// The key that signs the receipt. [`Receipt::sign`] sets it.
    pub issuer: KeyId,
    /// Kind-specific facts beyond design §4.4's fields: for example the fingerprint, approval
    /// method and approver on `decision.approved`. Never secrets or personal content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl ReceiptBody {
    /// A body with no inputs, outputs, policy or detail. Fill the rest in, then sign.
    pub fn new(
        kind: ReceiptKind,
        site: SiteId,
        subject: Subject,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Self {
        Self {
            receipt_id: ReceiptId::generate(),
            kind,
            site,
            subject,
            inputs: Vec::new(),
            policy: None,
            observed: None,
            outputs: Vec::new(),
            destroyed: None,
            started_at,
            ended_at,
            issuer: KeyId::unset(),
            detail: None,
        }
    }

    /// Sets `detail.title`: a plain-words line for the Trust tab and the Chief brief, for
    /// example "Read 41 pages of bakery.example in a sealed room". Keeps other detail members.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        let mut detail = match self.detail.take() {
            Some(serde_json::Value::Object(map)) => map,
            Some(other) => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_owned(), other);
                map
            }
            None => serde_json::Map::new(),
        };
        detail.insert("title".to_owned(), serde_json::Value::String(title.into()));
        self.detail = Some(serde_json::Value::Object(detail));
        self
    }

    /// The body of a finished job's receipt, built the same way by every caller from the
    /// signed spec, the runner's result and its event stream:
    /// - inputs, policy, observed (hosts, requests, bytes received, refusals, peaks), outputs
    ///   and `destroyed`;
    /// - `detail.killed` (snake_case, when the runner killed the room), `detail.exit_code`
    ///   and `detail.events`, the event names in order.
    ///
    /// Add a title with [`ReceiptBody::with_title`] and other facts to `detail`, then sign.
    pub fn for_job(site: SiteId, spec: &JobSpec, result: &JobResult, events: &[JobEvent]) -> Self {
        let mut body = Self::new(
            spec.kind.receipt_kind(),
            site,
            Subject::Job(spec.job_id.clone()),
            result.started_at,
            result.ended_at,
        );
        body.inputs = spec.inputs.clone();
        body.policy = Some(Policy {
            allowlist: spec.allowlist.clone(),
            caps: spec.caps,
            runtime: result.runtime.clone(),
        });
        let mut hosts: Vec<String> = Vec::new();
        let mut bytes = 0u64;
        for event in events {
            if let JobEvent::Request {
                host, bytes_down, ..
            } = event
            {
                bytes = bytes.saturating_add(*bytes_down);
                if !hosts.contains(host) {
                    hosts.push(host.clone());
                }
            }
        }
        body.observed = Some(Observed {
            hosts,
            requests: result.requests,
            bytes,
            refusals: result.refusals.clone(),
            peaks: result.peaks,
        });
        body.outputs = result.artifacts.iter().map(|a| a.digest).collect();
        body.destroyed = Some(result.destroyed);
        let mut detail = serde_json::Map::new();
        if let Some(killed) = result.killed
            && let Ok(value) = serde_json::to_value(killed)
        {
            detail.insert("killed".to_owned(), value);
        }
        if let Some(code) = result.exit_code {
            detail.insert("exit_code".to_owned(), serde_json::json!(code));
        }
        let names: Vec<serde_json::Value> = events
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .filter_map(|v| v.get("event").cloned())
            .collect();
        detail.insert("events".to_owned(), serde_json::Value::Array(names));
        body.detail = Some(serde_json::Value::Object(detail));
        body
    }

    /// `detail.title`, when the receipt has one.
    pub fn title(&self) -> Option<&str> {
        self.detail.as_ref()?.get("title")?.as_str()
    }
}

/// A signed receipt: the body's members plus `signature`, in one JSON object.
///
/// To check one without this crate: remove `signature`, canonicalise the rest (RFC 8785),
/// and verify the Ed25519 signature with the published key whose id is `issuer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    /// What the receipt says.
    #[serde(flatten)]
    pub body: ReceiptBody,
    /// Ed25519 over the RFC 8785 bytes of the body.
    pub signature: Signature,
}

impl Receipt {
    /// Sets the issuer to `signer`'s key and signs.
    pub fn sign(mut body: ReceiptBody, signer: &Signer) -> Result<Self> {
        body.issuer = signer.key_id().clone();
        let signature = signer.sign(&body)?;
        Ok(Self { body, signature })
    }

    /// Checks the signature against `key`, which must be the issuer's.
    pub fn verify(&self, key: &VerifyingKey) -> Result<&ReceiptBody> {
        let offered = KeyId::of(key);
        if offered != self.body.issuer {
            return Err(Error::WrongKey {
                signed_by: self.body.issuer.clone(),
                offered,
            });
        }
        verify(key, &self.body, &self.signature)?;
        Ok(&self.body)
    }

    /// Checks the signature against the issuer's key in a published key set.
    pub fn verify_with(&self, keys: &KeySet) -> Result<&ReceiptBody> {
        let key = keys.find(&self.body.issuer)?;
        self.verify(&key)
    }

    /// The canonical JSON of the whole signed receipt: what the CAS stores.
    pub fn canonical_json(&self) -> Result<Vec<u8>> {
        canonical_json(self)
    }

    /// The receipt's content address, served at `GET /receipts/{digest}`.
    pub fn digest(&self) -> Result<Digest> {
        Ok(Digest::of(&self.canonical_json()?))
    }
}
