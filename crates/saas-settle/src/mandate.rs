//! AP2-style mandates: the authority each owner signs once, and the per-invoice authority an
//! agent signs inside it. Every check here is deterministic and runs before any money moves.
//!
//! - **Intent mandate** (the payer's owner, once): which payees the Payables agent may pay, the
//!   most per invoice and per calendar month, instalment bounds, and an expiry. The owner
//!   approves its fingerprint with a passkey ([`agentrix_host::Approvals`], the same ceremony
//!   as decision cards); the company then seals it with its `owner` key, the passkey proof
//!   inside. Field names follow AP2's `IntentMandate` where they overlap
//!   (`natural_language_description`, `intent_expiry`); the rest are ours.
//! - **Collections mandate** (the payee's owner, once): which counterparties its Receivables
//!   agent may give terms to, the credit limit, and its own instalment bounds.
//! - **Cart mandate** (the payer's Payables agent, per invoice or per instalment): bound to the
//!   invoice id, amount, payee and due date, and to the intent mandate's digest. Each one moves
//!   money once.
//!
//! These are AP2-style, not AP2 verifiable credentials.

use std::fmt;

use adk_auth::web_bot_auth::KeyDirectorySource;
use agentrix_host::Approvals;
use agentrix_types::{Digest, Money, canonical_json};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::keys::{CompanyKeys, OWNER_KID, agent_kid};
use crate::seal::{SealError, SealKind, Sealed};

/// How an owner may approve a mandate. A passkey is the default; the local sandbox run may
/// allow its own key-file approval explicitly ([`LOCAL_OWNER_METHOD`]).
pub const PASSKEY_METHOD: &str = "passkey";
/// An owner approval made with a key file on the operator's machine: for local sandbox runs
/// only, never accepted unless the checking side allows it by name.
pub const LOCAL_OWNER_METHOD: &str = "local-owner-key";

/// Bounds on instalment plans an owner accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalmentBounds {
    /// The most instalments in one plan (1 means no instalments).
    pub max_count: u32,
    /// The last instalment falls at most this many days after the invoice's due date.
    pub max_span_days: u32,
    /// The first instalment is at least this percentage of the invoice (0–100).
    pub min_first_percent: u32,
}

/// The payer owner's standing authority for its Payables agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentMandate {
    /// `im_…`, unique.
    pub mandate_id: String,
    /// The intent in the owner's words.
    pub natural_language_description: String,
    /// The payer company's key directory origin.
    pub payer: String,
    /// The Payables agent's key thumbprint: the only key that may sign cart mandates and
    /// settlement requests under this mandate.
    pub agent_keyid: String,
    /// Payees (key directory origins) the agent may pay.
    pub payees: Vec<String>,
    /// The most one invoice may be.
    pub max_per_invoice: Money,
    /// The most paid under this mandate in one calendar month (UTC).
    pub max_per_month: Money,
    /// Instalment plans the agent may agree to.
    pub instalments: InstalmentBounds,
    /// From when it holds.
    pub not_before: DateTime<Utc>,
    /// After this it authorises nothing.
    pub intent_expiry: DateTime<Utc>,
}

/// The payee owner's standing authority for its Receivables agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionsMandate {
    /// `cm_…`, unique.
    pub mandate_id: String,
    /// The intent in the owner's words.
    pub natural_language_description: String,
    /// The payee company's key directory origin.
    pub payee: String,
    /// Counterparties (key directory origins) the agent may give terms to.
    pub counterparties: Vec<String>,
    /// The most a counterparty may owe on instalment plans at once.
    pub credit_limit: Money,
    /// Instalment plans the agent may agree to.
    pub instalments: InstalmentBounds,
    /// After this it authorises nothing.
    pub expiry: DateTime<Utc>,
}

/// Which instalment a cart mandate pays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalmentRef {
    /// The agreed plan's id.
    pub plan_id: String,
    /// 1-based.
    pub index: u32,
    /// How many instalments the plan has.
    pub count: u32,
}

/// One payment's authority, signed by the payer's Payables agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartMandate {
    /// `cart_…`, unique: each moves money once.
    pub cart_mandate_id: String,
    /// The intent mandate it acts under.
    pub intent_mandate_id: String,
    /// `sha256:` of the sealed intent mandate.
    pub intent_digest: String,
    /// The invoice it pays (the payee's invoice number).
    pub invoice_id: String,
    /// The payer's key directory origin.
    pub payer: String,
    /// The payee's key directory origin.
    pub payee: String,
    /// Exactly this much.
    pub amount: Money,
    /// The invoice's (or instalment's) due date.
    pub due_date: NaiveDate,
    /// Set when this pays one instalment of an agreed plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instalment: Option<InstalmentRef>,
    /// When the agent signed it.
    pub created_at: DateTime<Utc>,
    /// After this it moves nothing.
    pub expires_at: DateTime<Utc>,
}

/// An owner-approved mandate as sealed: the terms and the approval proof.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnerApproved<T> {
    /// The terms.
    pub mandate: T,
    /// The approval proof: `method`, `account`, `fingerprint` (hex of the terms' SHA-256),
    /// time, and the assertion.
    pub owner_approval: Value,
}

/// Why a mandate doesn't let money move. Every message ends "Nothing was paid."
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MandateRefusal {
    /// A document doesn't check.
    #[error("{0}. Nothing was paid.")]
    Seal(#[from] SealError),
    /// Signed by the wrong key (an agent key sealing an owner's mandate, or another agent).
    #[error("The {what} was signed by {got}, not by {want}. Nothing was paid.")]
    WrongSigner {
        /// What was checked.
        what: &'static str,
        /// The key role required.
        want: String,
        /// The key role found.
        got: String,
    },
    /// The owner's approval is missing, or approves something else.
    #[error("The {0} carries no owner approval of its exact terms. Nothing was paid.")]
    NotApproved(&'static str),
    /// Approved by a method this side doesn't accept.
    #[error("The {what} was approved by {method}, which isn't accepted here. Nothing was paid.")]
    Method {
        /// What was checked.
        what: &'static str,
        /// The method used.
        method: String,
    },
    /// Not yet, or no longer.
    #[error("The {what} isn't in force at {now}. Nothing was paid.")]
    NotInForce {
        /// What was checked.
        what: &'static str,
        /// When it was checked.
        now: DateTime<Utc>,
    },
    /// The cart mandate cites another intent mandate.
    #[error("The cart mandate doesn't cite this intent mandate. Nothing was paid.")]
    OtherIntent,
    /// The parties don't match.
    #[error("{0}. Nothing was paid.")]
    Parties(String),
    /// The invoice doesn't match.
    #[error("The cart mandate is for {cart}, but the invoice says {invoice}. Nothing was paid.")]
    Invoice {
        /// What the cart mandate says.
        cart: String,
        /// What the invoice says.
        invoice: String,
    },
    /// Over a cap.
    #[error("{amount} is over the intent mandate's {cap_name} of {cap}. Nothing was paid.")]
    OverCap {
        /// The amount, in words.
        amount: String,
        /// Which cap.
        cap_name: &'static str,
        /// The cap, in words.
        cap: String,
    },
    /// Currencies differ.
    #[error("The mandate is in {mandate}; the invoice is in {invoice}. Nothing was paid.")]
    Currency {
        /// The mandate's currency.
        mandate: String,
        /// The invoice's.
        invoice: String,
    },
    /// The request didn't come from the agent the owner named.
    #[error(
        "This request was signed by {agent}, not by the Payables agent the owner named in the intent mandate. Nothing was paid."
    )]
    WrongAgent {
        /// The requester's `wba:` id.
        agent: String,
    },
    /// Used before.
    #[error("This cart mandate was already used: each one moves money once. Nothing was paid.")]
    AlreadyUsed,
    /// Not yet due (an instalment collected early).
    #[error("This instalment isn't due until {0}. Nothing was paid.")]
    NotDue(NaiveDate),
}

/// "USD 18.50".
pub fn words(money: &Money) -> String {
    agentrix_effects::words(money)
}

/// The SHA-256 of the canonical JSON of `terms`: what the owner's passkey approves.
pub fn fingerprint<T: Serialize>(terms: &T) -> anyhow::Result<[u8; 32]> {
    let digest = Digest::of(&canonical_json(terms)?);
    let hex = digest.to_string();
    let hex = hex.strip_prefix("sha256:").unwrap_or(&hex);
    let bytes = hex::decode(hex)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("a SHA-256 digest is 32 bytes"))
}

/// The passkey options (`navigator.credentials.get`) for the owner to approve `terms`.
pub async fn passkey_options<T: Serialize>(
    approvals: &dyn Approvals,
    account: Uuid,
    terms: &T,
) -> anyhow::Result<Option<Value>> {
    approvals
        .passkey_options(account, fingerprint(terms)?)
        .await
}

/// Seals `terms` as `kind` with the company's owner key, once the owner's approval of their
/// exact fingerprint verifies (`request` is the approve body: `{"method": "passkey",
/// "credential": …}`).
pub async fn seal_owner_approved<T: Serialize>(
    kind: SealKind,
    terms: &T,
    approvals: &dyn Approvals,
    account: Uuid,
    request: Value,
    keys: &CompanyKeys,
) -> anyhow::Result<Sealed> {
    let fp = fingerprint(terms)?;
    let mut proof = approvals.verify(account, fp, request).await?;
    if let Some(object) = proof.as_object_mut() {
        object.insert("fingerprint".into(), json!(hex::encode(fp)));
        object
            .entry("account")
            .or_insert_with(|| json!(account.to_string()));
    }
    let body = OwnerApproved {
        mandate: terms,
        owner_approval: proof,
    };
    Ok(Sealed::seal(kind, &body, &keys.origin, &keys.owner)?)
}

/// Seals `terms` with an approval made by the operator's own key file ([`LOCAL_OWNER_METHOD`]):
/// for local sandbox runs, where no phone is at hand. A checking side refuses it unless it
/// allows that method by name.
pub fn seal_locally_approved<T: Serialize>(
    kind: SealKind,
    terms: &T,
    keys: &CompanyKeys,
    operator: &str,
) -> anyhow::Result<Sealed> {
    let fp = fingerprint(terms)?;
    let body = OwnerApproved {
        mandate: terms,
        owner_approval: json!({
            "method": LOCAL_OWNER_METHOD,
            "account": operator,
            "fingerprint": hex::encode(fp),
            "approved_at": Utc::now(),
        }),
    };
    Ok(Sealed::seal(kind, &body, &keys.origin, &keys.owner)?)
}

/// Checks an owner-sealed mandate: the signature against the company's published `owner` key,
/// and an approval of its exact terms by an accepted method. Returns the terms.
pub async fn verify_owner_sealed<T>(
    sealed: &Sealed,
    kind: SealKind,
    what: &'static str,
    directories: &dyn KeyDirectorySource,
    methods: &[String],
) -> Result<T, MandateRefusal>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let key = sealed.verify(kind, directories).await?;
    if key.kid.as_deref() != Some(OWNER_KID) {
        return Err(MandateRefusal::WrongSigner {
            what,
            want: OWNER_KID.into(),
            got: key.kid.unwrap_or_else(|| "an unnamed key".into()),
        });
    }
    let body: OwnerApproved<T> = sealed.read(kind)?;
    let fp = fingerprint(&body.mandate).map_err(|_| MandateRefusal::NotApproved(what))?;
    if body.owner_approval["fingerprint"].as_str() != Some(hex::encode(fp).as_str()) {
        return Err(MandateRefusal::NotApproved(what));
    }
    let method = body.owner_approval["method"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if !methods.contains(&method) {
        return Err(MandateRefusal::Method { what, method });
    }
    Ok(body.mandate)
}

/// Checks a cart mandate's signature: the payer's published key for the agent the intent
/// mandate names. Returns the terms.
pub async fn verify_cart(
    sealed: &Sealed,
    intent: &IntentMandate,
    directories: &dyn KeyDirectorySource,
) -> Result<CartMandate, MandateRefusal> {
    if sealed.signer != intent.payer {
        return Err(MandateRefusal::Parties(format!(
            "The cart mandate was signed at {}, not by the payer {}",
            sealed.signer, intent.payer
        )));
    }
    let key = sealed.verify(SealKind::CartMandate, directories).await?;
    if sealed.keyid != intent.agent_keyid {
        return Err(MandateRefusal::WrongSigner {
            what: "cart mandate",
            want: format!("the Payables agent key {}", intent.agent_keyid),
            got: key.kid.unwrap_or_else(|| sealed.keyid.clone()),
        });
    }
    Ok(sealed.read(SealKind::CartMandate)?)
}

/// What a settlement is checked against, besides the mandates.
#[derive(Debug, Clone)]
pub struct SettlementFacts<'a> {
    /// The signed requester's directory origin and key (`None` for a scheduled collection,
    /// where the agreed plan already carries the payer's signed cart mandates).
    pub requester: Option<(&'a str, &'a str)>,
    /// This (the payee's) key directory origin.
    pub payee: &'a str,
    /// The invoice id, open amount and due date as the payee's books hold them.
    pub invoice_id: &'a str,
    /// What the cart mandate must pay: the open amount, or the instalment's amount.
    pub amount: &'a Money,
    /// The invoice's (or the instalment's) due date.
    pub due_date: NaiveDate,
    /// The invoice's total, which the per-invoice cap bounds.
    pub invoice_total: &'a Money,
    /// Paid already this month under the intent mandate.
    pub month_paid: &'a Money,
    /// Now.
    pub now: DateTime<Utc>,
}

/// The calendar month (UTC) a payment counts against: `2026-11`.
pub fn month_of(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}

fn same_currency(mandate: &Money, invoice: &Money) -> Result<(), MandateRefusal> {
    if mandate.currency.eq_ignore_ascii_case(&invoice.currency) {
        Ok(())
    } else {
        Err(MandateRefusal::Currency {
            mandate: mandate.currency.clone(),
            invoice: invoice.currency.clone(),
        })
    }
}

/// The deterministic part: the intent mandate's terms, the cart mandate's terms and the
/// facts, with no I/O. Run after [`verify_owner_sealed`] and [`verify_cart`].
pub fn check_settlement(
    intent: &IntentMandate,
    intent_digest: &str,
    cart: &CartMandate,
    facts: &SettlementFacts<'_>,
) -> Result<(), MandateRefusal> {
    if facts.now < intent.not_before || facts.now >= intent.intent_expiry {
        return Err(MandateRefusal::NotInForce {
            what: "intent mandate",
            now: facts.now,
        });
    }
    if facts.now >= cart.expires_at {
        return Err(MandateRefusal::NotInForce {
            what: "cart mandate",
            now: facts.now,
        });
    }
    if cart.intent_mandate_id != intent.mandate_id || cart.intent_digest != intent_digest {
        return Err(MandateRefusal::OtherIntent);
    }
    if let Some((origin, keyid)) = facts.requester {
        if origin != intent.payer || keyid != intent.agent_keyid {
            return Err(MandateRefusal::WrongAgent {
                agent: format!("wba:{origin}#{keyid}"),
            });
        }
    }
    if cart.payer != intent.payer {
        return Err(MandateRefusal::Parties(format!(
            "The cart mandate's payer {} isn't the intent mandate's {}",
            cart.payer, intent.payer
        )));
    }
    if cart.payee != facts.payee {
        return Err(MandateRefusal::Parties(format!(
            "The cart mandate pays {}, not {}",
            cart.payee, facts.payee
        )));
    }
    if !intent.payees.iter().any(|p| p == facts.payee) {
        return Err(MandateRefusal::Parties(format!(
            "The intent mandate doesn't let its agent pay {}",
            facts.payee
        )));
    }
    if cart.invoice_id != facts.invoice_id {
        return Err(MandateRefusal::Invoice {
            cart: format!("invoice {}", cart.invoice_id),
            invoice: format!("invoice {}", facts.invoice_id),
        });
    }
    same_currency(&intent.max_per_invoice, facts.amount)?;
    same_currency(&intent.max_per_month, facts.amount)?;
    if cart.amount != *facts.amount {
        return Err(MandateRefusal::Invoice {
            cart: words(&cart.amount),
            invoice: words(facts.amount),
        });
    }
    if cart.due_date != facts.due_date {
        return Err(MandateRefusal::Invoice {
            cart: format!("due {}", cart.due_date),
            invoice: format!("due {}", facts.due_date),
        });
    }
    if facts.amount.minor <= 0 {
        return Err(MandateRefusal::Invoice {
            cart: words(&cart.amount),
            invoice: "nothing owed".into(),
        });
    }
    if facts.invoice_total.minor > intent.max_per_invoice.minor {
        return Err(MandateRefusal::OverCap {
            amount: words(facts.invoice_total),
            cap_name: "per-invoice cap",
            cap: words(&intent.max_per_invoice),
        });
    }
    let month_total = facts.month_paid.minor.saturating_add(facts.amount.minor);
    if month_total > intent.max_per_month.minor {
        return Err(MandateRefusal::OverCap {
            amount: format!(
                "{} with {} already paid this month",
                words(facts.amount),
                words(facts.month_paid)
            ),
            cap_name: "monthly cap",
            cap: words(&intent.max_per_month),
        });
    }
    if let Some(instalment) = &cart.instalment {
        if facts.now.date_naive() < cart.due_date {
            return Err(MandateRefusal::NotDue(cart.due_date));
        }
        if instalment.index == 0 || instalment.index > instalment.count {
            return Err(MandateRefusal::Invoice {
                cart: format!("instalment {} of {}", instalment.index, instalment.count),
                invoice: "no such instalment".into(),
            });
        }
    }
    Ok(())
}

impl fmt::Display for InstalmentBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "at most {} instalments, the last within {} days of the due date, the first at least {}% of the invoice",
            self.max_count, self.max_span_days, self.min_first_percent
        )
    }
}

/// The role a published key has (`owner`, `agent:payables`), for messages.
pub fn role_of(kid: Option<&str>) -> String {
    match kid {
        Some(OWNER_KID) => "the owner".into(),
        Some(kid) if kid.starts_with("agent:") => kid.replacen("agent:", "the ", 1) + " agent",
        Some(kid) => kid.into(),
        None => "an unnamed key".into(),
    }
}

/// `agent:payables`.
pub fn payables_kid() -> String {
    agent_kid(crate::keys::PAYABLES)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use adk_auth::web_bot_auth::InMemoryKeyDirectorySource;
    use chrono::Duration;

    use super::*;
    use crate::keys::{PAYABLES, RECEIVABLES};

    fn usd(minor: i64) -> Money {
        Money {
            currency: "USD".into(),
            minor,
        }
    }

    struct Fixture {
        kilele: CompanyKeys,
        directories: Arc<InMemoryKeyDirectorySource>,
        intent: IntentMandate,
        sealed_intent: Sealed,
    }

    async fn fixture() -> Fixture {
        let kilele = CompanyKeys::generate("https://kilele.example", &[PAYABLES]).unwrap();
        let agentrix = CompanyKeys::generate("https://agentrix.example", &[RECEIVABLES]).unwrap();
        let directories = Arc::new(InMemoryKeyDirectorySource::new());
        directories
            .insert(&kilele.origin, kilele.directory(), None)
            .await;
        directories
            .insert(&agentrix.origin, agentrix.directory(), None)
            .await;
        let now = Utc::now();
        let intent = IntentMandate {
            mandate_id: "im_1".into(),
            natural_language_description: "Pay Agentrix's invoices up to $500 each".into(),
            payer: kilele.origin.clone(),
            agent_keyid: kilele.agent(PAYABLES).unwrap().thumbprint().into(),
            payees: vec![agentrix.origin.clone()],
            max_per_invoice: usd(50_000),
            max_per_month: usd(80_000),
            instalments: InstalmentBounds {
                max_count: 3,
                max_span_days: 90,
                min_first_percent: 20,
            },
            not_before: now - Duration::hours(1),
            intent_expiry: now + Duration::days(30),
        };
        let sealed_intent =
            seal_locally_approved(SealKind::IntentMandate, &intent, &kilele, "test").unwrap();
        Fixture {
            kilele,
            directories,
            intent,
            sealed_intent,
        }
    }

    fn cart(f: &Fixture, amount: i64) -> CartMandate {
        let now = Utc::now();
        CartMandate {
            cart_mandate_id: "cart_1".into(),
            intent_mandate_id: f.intent.mandate_id.clone(),
            intent_digest: f.sealed_intent.digest().unwrap().to_string(),
            invoice_id: "INV-1".into(),
            payer: f.kilele.origin.clone(),
            payee: "https://agentrix.example".into(),
            amount: usd(amount),
            due_date: now.date_naive(),
            instalment: None,
            created_at: now,
            expires_at: now + Duration::days(7),
        }
    }

    #[tokio::test]
    async fn an_owner_mandate_needs_the_owner_key_and_an_accepted_method() {
        let f = fixture().await;
        let local = vec![LOCAL_OWNER_METHOD.to_owned()];
        let terms: IntentMandate = verify_owner_sealed(
            &f.sealed_intent,
            SealKind::IntentMandate,
            "intent mandate",
            f.directories.as_ref(),
            &local,
        )
        .await
        .unwrap();
        assert_eq!(terms, f.intent);

        let passkey_only = vec![PASSKEY_METHOD.to_owned()];
        assert!(matches!(
            verify_owner_sealed::<IntentMandate>(
                &f.sealed_intent,
                SealKind::IntentMandate,
                "intent mandate",
                f.directories.as_ref(),
                &passkey_only,
            )
            .await,
            Err(MandateRefusal::Method { .. })
        ));

        // The Payables agent sealing its own "intent mandate" is refused.
        let agent_key = f.kilele.agent(PAYABLES).unwrap();
        let body = OwnerApproved {
            mandate: &f.intent,
            owner_approval: json!({"method": LOCAL_OWNER_METHOD, "fingerprint": hex::encode(fingerprint(&f.intent).unwrap())}),
        };
        let forged =
            Sealed::seal(SealKind::IntentMandate, &body, &f.kilele.origin, &agent_key).unwrap();
        assert!(matches!(
            verify_owner_sealed::<IntentMandate>(
                &forged,
                SealKind::IntentMandate,
                "intent mandate",
                f.directories.as_ref(),
                &local,
            )
            .await,
            Err(MandateRefusal::WrongSigner { .. })
        ));

        // An approval of other terms doesn't carry over.
        let mut other = f.intent.clone();
        other.max_per_invoice = usd(5_000_000);
        let body = OwnerApproved {
            mandate: &other,
            owner_approval: json!({"method": LOCAL_OWNER_METHOD, "fingerprint": hex::encode(fingerprint(&f.intent).unwrap())}),
        };
        let stretched = Sealed::seal(
            SealKind::IntentMandate,
            &body,
            &f.kilele.origin,
            &f.kilele.owner,
        )
        .unwrap();
        assert!(matches!(
            verify_owner_sealed::<IntentMandate>(
                &stretched,
                SealKind::IntentMandate,
                "intent mandate",
                f.directories.as_ref(),
                &local,
            )
            .await,
            Err(MandateRefusal::NotApproved(_))
        ));
    }

    #[tokio::test]
    async fn a_cart_mandate_must_be_the_named_agents() {
        let f = fixture().await;
        let terms = cart(&f, 10_000);
        let agent_key = f.kilele.agent(PAYABLES).unwrap();
        let sealed =
            Sealed::seal(SealKind::CartMandate, &terms, &f.kilele.origin, &agent_key).unwrap();
        assert_eq!(
            verify_cart(&sealed, &f.intent, f.directories.as_ref())
                .await
                .unwrap(),
            terms
        );
        let by_owner = Sealed::seal(
            SealKind::CartMandate,
            &terms,
            &f.kilele.origin,
            &f.kilele.owner,
        )
        .unwrap();
        assert!(matches!(
            verify_cart(&by_owner, &f.intent, f.directories.as_ref()).await,
            Err(MandateRefusal::WrongSigner { .. })
        ));
    }

    #[tokio::test]
    async fn the_terms_bind_amount_payee_due_date_and_caps() {
        let f = fixture().await;
        let digest = f.sealed_intent.digest().unwrap().to_string();
        let c = cart(&f, 10_000);
        let paid = usd(0);
        let amount = usd(10_000);
        let ok = SettlementFacts {
            requester: Some((&f.kilele.origin, &f.intent.agent_keyid)),
            payee: "https://agentrix.example",
            invoice_id: "INV-1",
            amount: &amount,
            due_date: c.due_date,
            invoice_total: &amount,
            month_paid: &paid,
            now: Utc::now(),
        };
        check_settlement(&f.intent, &digest, &c, &ok).unwrap();

        let other_amount = usd(9_999);
        assert!(matches!(
            check_settlement(
                &f.intent,
                &digest,
                &c,
                &SettlementFacts {
                    amount: &other_amount,
                    ..ok.clone()
                }
            ),
            Err(MandateRefusal::Invoice { .. })
        ));
        let big = usd(60_000);
        let mut over = c.clone();
        over.amount = big.clone();
        assert!(matches!(
            check_settlement(
                &f.intent,
                &digest,
                &over,
                &SettlementFacts {
                    amount: &big,
                    invoice_total: &big,
                    ..ok.clone()
                }
            ),
            Err(MandateRefusal::OverCap {
                cap_name: "per-invoice cap",
                ..
            })
        ));
        let month = usd(75_000);
        assert!(matches!(
            check_settlement(
                &f.intent,
                &digest,
                &c,
                &SettlementFacts {
                    month_paid: &month,
                    ..ok.clone()
                }
            ),
            Err(MandateRefusal::OverCap {
                cap_name: "monthly cap",
                ..
            })
        ));
        assert!(matches!(
            check_settlement(
                &f.intent,
                &digest,
                &c,
                &SettlementFacts {
                    payee: "https://someone-else.example",
                    ..ok.clone()
                }
            ),
            Err(MandateRefusal::Parties(_))
        ));
        assert!(matches!(
            check_settlement(
                &f.intent,
                &digest,
                &c,
                &SettlementFacts {
                    requester: Some((&f.kilele.origin, "another-key")),
                    ..ok.clone()
                }
            ),
            Err(MandateRefusal::WrongAgent { .. })
        ));
        assert!(matches!(
            check_settlement(&f.intent, "sha256:00", &c, &ok),
            Err(MandateRefusal::OtherIntent)
        ));
        assert!(matches!(
            check_settlement(
                &f.intent,
                &digest,
                &c,
                &SettlementFacts {
                    now: f.intent.intent_expiry,
                    ..ok.clone()
                }
            ),
            Err(MandateRefusal::NotInForce { .. })
        ));
    }
}
