//! What the two companies' agents say to each other over A2A, and the documents they sign.
//!
//! Each A2A v1 `SendMessage` carries one [`SettleMessage`] as a data part; the answering door
//! returns a task whose artifact is one [`DoorAnswer`] (completed), or whose status says why it
//! refused (rejected). Every document that binds a company is [`Sealed`] with a key from that
//! company's published directory.

use agentrix_types::Money;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::mandate::{InstalmentBounds, InstalmentRef, words};
use crate::seal::Sealed;

/// The media type of the data part a settlement message travels in.
pub const MESSAGE_MEDIA_TYPE: &str = "application/vnd.agentrix.settle+json";

/// One line of an invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceLine {
    /// What is billed.
    pub description: String,
    /// How much.
    pub amount: Money,
}

/// An invoice as the payee's Receivables agent offers it (from the payee's books).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice {
    /// The payee's invoice number, for example `INV-0007`: what both books cite.
    pub invoice_id: String,
    /// The payee's key directory origin.
    pub payee: String,
    /// The payee's name.
    pub payee_name: String,
    /// The payer's key directory origin.
    pub payer: String,
    /// The payer's name.
    pub payer_name: String,
    /// The invoice total.
    pub total: Money,
    /// What is still owed.
    pub open: Money,
    /// The invoice date.
    pub issued_on: NaiveDate,
    /// When payment is due.
    pub due_on: NaiveDate,
    /// What it is for.
    pub description: String,
    /// The lines.
    pub lines: Vec<InvoiceLine>,
}

/// How the money moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// The payee captured from the payer's saved PayPal (Vault), merchant-initiated, with the
    /// payer agent's cart mandate as its authority.
    Vault,
    /// The payer sent a PayPal payout to the payee's PayPal account, carrying the invoice id.
    Payout,
}

/// The one PayPal object a settlement is, which both companies' books cite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayPalSettlement {
    /// `capture` (Vault route) or `payout` (one payout item).
    pub kind: String,
    /// The capture id, or the payout item id.
    pub id: String,
    /// The Orders v2 order (Vault route) or the payout batch (Payout route).
    pub parent_id: Option<String>,
    /// PayPal's transaction id, when PayPal reports one.
    pub transaction_id: Option<String>,
    /// PayPal's status: `COMPLETED`, `SUCCESS`, …
    pub status: String,
    /// PayPal's fee, charged to the payee (Vault) or the payer (Payout), when reported.
    pub fee: Option<Money>,
    /// PayPal's sandbox.
    pub sandbox: bool,
}

impl PayPalSettlement {
    /// The books' reference for it: `paypal:capture:<id>` or `paypal:payout:<id>`, the same
    /// key on both sides, so each posting is idempotent by the one PayPal id.
    pub fn key(&self) -> String {
        format!("paypal:{}:{}", self.kind, self.id)
    }
}

/// What the payer reports about a payout it made (Payout route).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutEvidence {
    /// PayPal's `payout_batch_id`.
    pub batch_id: String,
    /// PayPal's `payout_item_id`.
    pub item_id: String,
    /// PayPal's transaction id.
    pub transaction_id: Option<String>,
    /// PayPal's item status (`SUCCESS`).
    pub status: String,
    /// The amount.
    pub amount: Money,
    /// The payee's PayPal email it went to.
    pub receiver: String,
    /// The payer's fee, when reported.
    pub fee: Option<Money>,
}

/// The payee's receipt for a settlement, sealed by its Receivables agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementReceipt {
    /// `sr_…`.
    pub receipt_id: String,
    /// The invoice settled.
    pub invoice_id: String,
    /// The payer's origin.
    pub payer: String,
    /// The payee's origin.
    pub payee: String,
    /// How much moved.
    pub amount: Money,
    /// How.
    pub route: Route,
    /// The PayPal object both books post from.
    pub paypal: PayPalSettlement,
    /// The cart mandate that authorised it.
    pub cart_mandate_id: String,
    /// `sha256:` of the sealed cart mandate.
    pub cart_digest: String,
    /// The intent mandate it acted under.
    pub intent_mandate_id: String,
    /// `sha256:` of the sealed intent mandate.
    pub intent_digest: String,
    /// Set when it paid one instalment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instalment: Option<InstalmentRef>,
    /// What is still owed on the invoice after this.
    pub invoice_open_after: Money,
    /// When.
    pub settled_at: DateTime<Utc>,
}

/// A door's or desk's refusal, sealed by the refusing company's agent key: what was refused,
/// who asked (when the signature verified), why, and that nothing moved. The company's eval
/// suite and its signed audit log keep these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusalReceipt {
    /// `rr_…`.
    pub refusal_id: String,
    /// The refusing company's origin.
    pub refused_by: String,
    /// Its name.
    pub company: String,
    /// The verified requester (`wba:{origin}#{key}`), when the signature checked.
    pub requester: Option<String>,
    /// The directory the request named in `Signature-Agent`, when it didn't check.
    pub claimed_agent: Option<String>,
    /// The settlement message's type, when it got that far.
    pub message: Option<String>,
    /// A short code: `signature_required`, `signature_invalid`, `mandate_used`, `plan_refused`…
    pub code: String,
    /// Why, in plain words.
    pub reason: String,
    /// `sha256:` of the request body.
    pub request_digest: Option<String>,
    /// No money moved and nothing was recorded for the request.
    pub nothing_moved: bool,
    /// When.
    pub at: DateTime<Utc>,
    /// Who the requester is in its own cloud, when it presented an agent token that checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<crate::tokens::FederatedPrincipal>,
}

/// One instalment of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instalment {
    /// 1-based.
    pub index: u32,
    /// When it is collected.
    pub due_on: NaiveDate,
    /// How much.
    pub amount: Money,
}

/// An instalment plan for one invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalmentPlan {
    /// `plan_…`.
    pub plan_id: String,
    /// The invoice.
    pub invoice_id: String,
    /// The payer's origin.
    pub payer: String,
    /// The payee's origin.
    pub payee: String,
    /// What the plan pays in all: the invoice's open amount.
    pub total: Money,
    /// The instalments, in order.
    pub instalments: Vec<Instalment>,
    /// Why, in the proposing agent's words.
    pub note: String,
}

/// A plan both sides agreed: the payee's sealed plan and the payer's cart mandate for each
/// instalment, which the payee's Receivables agent collects on each due date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalmentAgreement {
    /// The plan, sealed by the payee's Receivables agent.
    pub plan: Sealed,
    /// `sha256:` of each instalment's sealed cart mandate, in order.
    pub cart_digests: Vec<String>,
    /// When it was agreed.
    pub agreed_at: DateTime<Utc>,
}

/// A message between the companies' agent doors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettleMessage {
    /// Payee → payer: an invoice, sealed by the payee's Receivables agent.
    InvoiceOffer {
        /// The sealed [`Invoice`].
        invoice: Sealed,
    },
    /// Payer → payee: pay this invoice under these mandates.
    Settle {
        /// The payer owner's sealed intent mandate.
        intent: Sealed,
        /// The Payables agent's sealed cart mandate for the invoice.
        cart: Sealed,
        /// Payout route only: the payout the payer already made.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payout: Option<PayoutEvidence>,
    },
    /// Payer → payee: pay this invoice in instalments.
    InstalmentPropose {
        /// The payer owner's sealed intent mandate (its instalment bounds bind the plan).
        intent: Sealed,
        /// The proposed plan.
        plan: InstalmentPlan,
    },
    /// Payee → payer: not that plan, this one (also sent as an answer to a proposal).
    InstalmentCounter {
        /// The counter-plan, sealed by the payee's Receivables agent.
        plan: Sealed,
        /// Why, in the payee agent's words.
        reason: String,
    },
    /// Payer → payee: agreed, with a cart mandate for each instalment.
    InstalmentAccept {
        /// The payer owner's sealed intent mandate.
        intent: Sealed,
        /// The payee's sealed plan being accepted (its own acceptance or counter).
        plan: Sealed,
        /// One sealed cart mandate per instalment, in order.
        carts: Vec<Sealed>,
    },
    /// Payee → payer: a settlement the payee made on its own (a scheduled instalment it
    /// collected on its due date), so the payer posts it from the same PayPal id.
    SettlementReceipt {
        /// The sealed [`SettlementReceipt`].
        receipt: Sealed,
    },
}

impl SettleMessage {
    /// The message's type, for logs and the task's history.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvoiceOffer { .. } => "invoice_offer",
            Self::Settle { .. } => "settle",
            Self::InstalmentPropose { .. } => "instalment_propose",
            Self::InstalmentCounter { .. } => "instalment_counter",
            Self::InstalmentAccept { .. } => "instalment_accept",
            Self::SettlementReceipt { .. } => "settlement_receipt",
        }
    }
}

/// What the answering door returns as the task's artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DoorAnswer {
    /// The payer recorded the bill and says what it will do.
    InvoiceRecorded {
        /// The invoice.
        invoice_id: String,
        /// `will_settle`, `instalments` or `held`.
        decision: String,
        /// Why, in plain words.
        note: String,
    },
    /// The payee settled the invoice.
    Settled {
        /// The sealed [`SettlementReceipt`].
        receipt: Sealed,
    },
    /// The payee answers a proposal with a plan it would agree to.
    InstalmentOffer {
        /// The plan, sealed by the payee's Receivables agent.
        plan: Sealed,
        /// True when it differs from the proposal.
        countered: bool,
        /// Why, in plain words.
        reason: String,
    },
    /// The plan is agreed and its instalments scheduled.
    InstalmentAgreed {
        /// The sealed [`InstalmentAgreement`].
        agreement: Sealed,
    },
    /// The payer accepts the payee's terms: its intent mandate and a cart mandate for each
    /// instalment of the plan.
    InstalmentAccepted {
        /// The payer owner's sealed intent mandate.
        intent: Sealed,
        /// The payee's sealed plan being accepted.
        plan: Sealed,
        /// One sealed cart mandate per instalment, in order.
        carts: Vec<Sealed>,
    },
    /// The payer posted a settlement receipt to its books.
    ReceiptPosted {
        /// The receipt.
        receipt_id: String,
        /// The PayPal key both books cite.
        paypal_key: String,
        /// True when the payer's books already had it.
        already_posted: bool,
    },
}

/// Why a plan is outside an owner's bounds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanRefusal {
    /// It doesn't add up or is out of order.
    #[error("The plan doesn't hold together: {0}.")]
    Shape(String),
    /// Outside one owner's bounds.
    #[error("The plan is outside {whose} mandate ({bounds}): {why}.")]
    Bounds {
        /// "Kilele Bakery's owner's".
        whose: String,
        /// The bounds, in words.
        bounds: String,
        /// What broke them.
        why: String,
    },
    /// Over the payee's credit limit.
    #[error("The plan would leave {owed} owed on terms, over the credit limit of {limit}.")]
    CreditLimit {
        /// Owed on plans with this one.
        owed: String,
        /// The limit.
        limit: String,
    },
}

/// Checks a plan's shape against the invoice: it pays exactly the open amount, in instalments
/// numbered 1…n with strictly later due dates, none before `today`, each above zero.
pub fn check_plan_shape(
    plan: &InstalmentPlan,
    invoice: &Invoice,
    today: NaiveDate,
) -> Result<(), PlanRefusal> {
    let shape = |s: String| Err(PlanRefusal::Shape(s));
    if plan.invoice_id != invoice.invoice_id {
        return shape(format!(
            "it is for {}, not {}",
            plan.invoice_id, invoice.invoice_id
        ));
    }
    if plan.payer != invoice.payer || plan.payee != invoice.payee {
        return shape("its parties aren't the invoice's".into());
    }
    if plan.total != invoice.open {
        return shape(format!(
            "it pays {} but {} is owed",
            words(&plan.total),
            words(&invoice.open)
        ));
    }
    if plan.instalments.is_empty() {
        return shape("it has no instalments".into());
    }
    let mut sum: i64 = 0;
    let mut last: Option<NaiveDate> = None;
    for (i, instalment) in plan.instalments.iter().enumerate() {
        if instalment.index as usize != i + 1 {
            return shape("its instalments aren't numbered 1, 2, 3…".into());
        }
        if instalment.amount.currency != invoice.open.currency || instalment.amount.minor <= 0 {
            return shape(format!(
                "instalment {} is {}",
                instalment.index,
                words(&instalment.amount)
            ));
        }
        if instalment.due_on < today {
            return shape(format!(
                "instalment {} falls due in the past",
                instalment.index
            ));
        }
        if last.is_some_and(|d| instalment.due_on <= d) {
            return shape("its due dates aren't in order".into());
        }
        last = Some(instalment.due_on);
        sum = sum.saturating_add(instalment.amount.minor);
    }
    if sum != plan.total.minor {
        return shape(format!(
            "its instalments add up to {}, not {}",
            words(&Money {
                currency: plan.total.currency.clone(),
                minor: sum
            }),
            words(&plan.total)
        ));
    }
    Ok(())
}

/// Checks a plan against one owner's instalment bounds.
pub fn check_plan_bounds(
    plan: &InstalmentPlan,
    invoice: &Invoice,
    bounds: &InstalmentBounds,
    whose: &str,
) -> Result<(), PlanRefusal> {
    let out = |why: String| {
        Err(PlanRefusal::Bounds {
            whose: whose.to_owned(),
            bounds: bounds.to_string(),
            why,
        })
    };
    let count = plan.instalments.len() as u32;
    if count > bounds.max_count {
        return out(format!("{count} instalments"));
    }
    if let Some(last) = plan.instalments.last() {
        let span = (last.due_on - invoice.due_on).num_days();
        if span > i64::from(bounds.max_span_days) {
            return out(format!(
                "the last instalment is {span} days after the due date"
            ));
        }
    }
    if let Some(first) = plan.instalments.first() {
        let floor = min_first(&plan.total, bounds.min_first_percent);
        if first.amount.minor < floor {
            return out(format!(
                "the first instalment is {}, under {}",
                words(&first.amount),
                words(&Money {
                    currency: plan.total.currency.clone(),
                    minor: floor
                })
            ));
        }
    }
    Ok(())
}

/// Checks that every instalment falls due before a mandate stops holding: a plan can't outlive
/// the authority it is paid under.
pub fn check_plan_expiry(
    plan: &InstalmentPlan,
    expiry: DateTime<Utc>,
    whose: &str,
) -> Result<(), PlanRefusal> {
    match plan.instalments.last() {
        Some(last) if last.due_on >= expiry.date_naive() => Err(PlanRefusal::Bounds {
            whose: whose.to_owned(),
            bounds: format!("in force until {}", expiry.date_naive()),
            why: format!(
                "instalment {} falls due on {}, after it expires",
                last.index, last.due_on
            ),
        }),
        _ => Ok(()),
    }
}

/// `bounds` with its span cut so the last instalment falls due before `expiry`.
pub fn bounds_within(
    bounds: &InstalmentBounds,
    invoice_due: NaiveDate,
    expiry: DateTime<Utc>,
) -> InstalmentBounds {
    let room = (expiry.date_naive() - invoice_due).num_days() - 1;
    InstalmentBounds {
        max_span_days: bounds
            .max_span_days
            .min(u32::try_from(room.max(0)).unwrap_or(0)),
        ..bounds.clone()
    }
}

/// The smallest first instalment `percent` of `total` allows, rounded up to the minor unit.
pub fn min_first(total: &Money, percent: u32) -> i64 {
    let percent = i64::from(percent.min(100));
    (total.minor * percent + 99) / 100
}

/// The plan nearest to `proposal` inside both owners' bounds: as many instalments as proposed
/// and both allow, the last on the earlier of the two span limits, the first at least both
/// minimums, the rest even (any remainder on the last). Deterministic; what a negotiating model
/// proposes is checked the same way. `None` when no plan fits (for example a span of zero
/// days with more than one instalment).
pub fn fit_plan(
    proposal: &InstalmentPlan,
    invoice: &Invoice,
    payer: &InstalmentBounds,
    payee: &InstalmentBounds,
    today: NaiveDate,
    plan_id: String,
) -> Option<InstalmentPlan> {
    let max_count = payer.max_count.min(payee.max_count).max(1);
    let count = (proposal.instalments.len() as u32).clamp(1, max_count);
    let span = i64::from(payer.max_span_days.min(payee.max_span_days));
    let start = invoice.due_on.max(today);
    let end = invoice.due_on + Duration::days(span);
    if end < start || (count > 1 && end == start) {
        return (count == 1 && end >= start).then(|| single(proposal, invoice, start, plan_id));
    }
    let total = invoice.open.minor;
    let floor = min_first(
        &invoice.open,
        payer.min_first_percent.max(payee.min_first_percent),
    );
    let even = total / i64::from(count);
    let first = even.max(floor).min(total);
    let rest_count = i64::from(count) - 1;
    let mut amounts = vec![first];
    if rest_count > 0 {
        let rest = total - first;
        if rest <= 0 {
            return Some(single(proposal, invoice, start, plan_id));
        }
        let each = rest / rest_count;
        if each <= 0 {
            return Some(single(proposal, invoice, start, plan_id));
        }
        for i in 0..rest_count {
            let extra = if i == rest_count - 1 {
                rest - each * rest_count
            } else {
                0
            };
            amounts.push(each + extra);
        }
    }
    let days = (end - start).num_days();
    let steps = i64::from(count.saturating_sub(1)).max(1);
    let instalments = amounts
        .into_iter()
        .enumerate()
        .map(|(i, minor)| Instalment {
            index: i as u32 + 1,
            due_on: if count == 1 {
                start
            } else {
                start + Duration::days(days * i as i64 / steps)
            },
            amount: Money {
                currency: invoice.open.currency.clone(),
                minor,
            },
        })
        .collect();
    Some(InstalmentPlan {
        plan_id,
        invoice_id: invoice.invoice_id.clone(),
        payer: invoice.payer.clone(),
        payee: invoice.payee.clone(),
        total: invoice.open.clone(),
        instalments,
        note: format!(
            "{count} instalments within {span} days of the due date, the first at least {}",
            words(&Money {
                currency: invoice.open.currency.clone(),
                minor: floor
            })
        ),
    })
}

fn single(
    proposal: &InstalmentPlan,
    invoice: &Invoice,
    on: NaiveDate,
    plan_id: String,
) -> InstalmentPlan {
    InstalmentPlan {
        plan_id,
        invoice_id: invoice.invoice_id.clone(),
        payer: invoice.payer.clone(),
        payee: invoice.payee.clone(),
        total: invoice.open.clone(),
        instalments: vec![Instalment {
            index: 1,
            due_on: on,
            amount: invoice.open.clone(),
        }],
        note: format!("Paid in one on {on}; {}", proposal.note),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(minor: i64) -> Money {
        Money {
            currency: "USD".into(),
            minor,
        }
    }

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn invoice() -> Invoice {
        Invoice {
            invoice_id: "INV-0007".into(),
            payee: "https://agentrix.example".into(),
            payee_name: "Agentrix".into(),
            payer: "https://kilele.example".into(),
            payer_name: "Kilele Bakery".into(),
            total: usd(120_000),
            open: usd(120_000),
            issued_on: d("2026-11-10"),
            due_on: d("2026-11-24"),
            description: "WebAgent Pro, annual".into(),
            lines: vec![],
        }
    }

    fn plan(parts: &[(&str, i64)]) -> InstalmentPlan {
        InstalmentPlan {
            plan_id: "plan_1".into(),
            invoice_id: "INV-0007".into(),
            payer: "https://kilele.example".into(),
            payee: "https://agentrix.example".into(),
            total: usd(120_000),
            instalments: parts
                .iter()
                .enumerate()
                .map(|(i, (due, minor))| Instalment {
                    index: i as u32 + 1,
                    due_on: d(due),
                    amount: usd(*minor),
                })
                .collect(),
            note: "short this month".into(),
        }
    }

    const PAYER: InstalmentBounds = InstalmentBounds {
        max_count: 4,
        max_span_days: 90,
        min_first_percent: 20,
    };
    const PAYEE: InstalmentBounds = InstalmentBounds {
        max_count: 3,
        max_span_days: 60,
        min_first_percent: 30,
    };

    #[test]
    fn a_plan_inside_both_owners_bounds_passes() {
        let p = plan(&[
            ("2026-11-24", 40_000),
            ("2026-12-24", 40_000),
            ("2027-01-23", 40_000),
        ]);
        check_plan_shape(&p, &invoice(), d("2026-11-10")).unwrap();
        check_plan_bounds(&p, &invoice(), &PAYER, "Kilele's owner's").unwrap();
        check_plan_bounds(&p, &invoice(), &PAYEE, "Agentrix's owner's").unwrap();
    }

    #[test]
    fn plans_outside_the_bounds_are_refused() {
        let four = plan(&[
            ("2026-11-24", 30_000),
            ("2026-12-10", 30_000),
            ("2026-12-24", 30_000),
            ("2027-01-10", 30_000),
        ]);
        check_plan_bounds(&four, &invoice(), &PAYER, "payer").unwrap();
        assert!(check_plan_bounds(&four, &invoice(), &PAYEE, "payee").is_err());
        let long = plan(&[("2026-11-24", 60_000), ("2027-03-01", 60_000)]);
        assert!(matches!(
            check_plan_bounds(&long, &invoice(), &PAYEE, "payee"),
            Err(PlanRefusal::Bounds { .. })
        ));
        let small_first = plan(&[("2026-11-24", 10_000), ("2026-12-24", 110_000)]);
        assert!(check_plan_bounds(&small_first, &invoice(), &PAYER, "payer").is_err());
        let short = plan(&[("2026-11-24", 60_000), ("2026-12-24", 50_000)]);
        assert!(matches!(
            check_plan_shape(&short, &invoice(), d("2026-11-10")),
            Err(PlanRefusal::Shape(_))
        ));
        let unordered = plan(&[("2026-12-24", 60_000), ("2026-11-24", 60_000)]);
        assert!(check_plan_shape(&unordered, &invoice(), d("2026-11-10")).is_err());
    }

    #[test]
    fn the_fitted_plan_is_inside_both_bounds() {
        let proposal = plan(&[
            ("2026-11-24", 10_000),
            ("2026-12-10", 10_000),
            ("2026-12-24", 10_000),
            ("2027-03-10", 90_000),
        ]);
        let today = d("2026-11-10");
        let fitted = fit_plan(
            &proposal,
            &invoice(),
            &PAYER,
            &PAYEE,
            today,
            "plan_2".into(),
        )
        .unwrap();
        assert_eq!(fitted.instalments.len(), 3);
        check_plan_shape(&fitted, &invoice(), today).unwrap();
        check_plan_bounds(&fitted, &invoice(), &PAYER, "payer").unwrap();
        check_plan_bounds(&fitted, &invoice(), &PAYEE, "payee").unwrap();
        assert_eq!(
            fitted.instalments.last().unwrap().due_on,
            d("2027-01-23"),
            "the last falls on the tighter span"
        );
    }

    #[test]
    fn a_plan_never_outlives_the_mandates_it_is_paid_under() {
        let expiry = d("2026-12-20").and_hms_opt(0, 0, 0).unwrap().and_utc();
        let p = plan(&[
            ("2026-11-24", 40_000),
            ("2026-12-24", 40_000),
            ("2027-01-23", 40_000),
        ]);
        assert!(check_plan_expiry(&p, expiry, "the intent mandate").is_err());
        let tight = bounds_within(&PAYER, invoice().due_on, expiry);
        assert_eq!(tight.max_span_days, 25);
        let fitted = fit_plan(
            &p,
            &invoice(),
            &tight,
            &PAYEE,
            d("2026-11-10"),
            "plan_3".into(),
        )
        .unwrap();
        check_plan_expiry(&fitted, expiry, "the intent mandate").unwrap();
        check_plan_bounds(&fitted, &invoice(), &PAYEE, "payee").unwrap();
    }
}
