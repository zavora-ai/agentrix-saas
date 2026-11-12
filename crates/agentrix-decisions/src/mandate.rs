//! The founder mandate (Agentrix SaaS): one card that says what a company's agents may do with
//! money on their own, signed once by the founder with a passkey.
//!
//! The terms ([`MandateTerms`]) are plain limits:
//!
//! - refunds up to a cap each, only to the original payer and never above the capture;
//! - bill payments (PayPal Payouts) up to a cap each and a cap a day, only to payees the books
//!   already hold for a vendor the mandate lists;
//! - instalment plans for a customer's debt up to a number of instalments and a length;
//! - model spend up to a cap a day;
//! - which plans may be sold, and whether a subscription may be suspended after a failed payment.
//!
//! [`Decisions::raise_mandate`] puts them on a card; the founder approves it through the frozen
//! approve contract like any other card, so the passkey assertion is over the card's fingerprint,
//! which covers every term. [`Decisions::signed_mandate`] reads the newest mandate approved with
//! a passkey back, with its receipt.
//!
//! Anything outside the terms is not refused here: the company's governed path holds it, and
//! the founder decides that one call with a passkey of its own.

use agentrix_types::{Digest, EffectRequest, Money, SiteId};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::edit::money_words;
use crate::{
    ApprovalKind, CardState, DecisionCardV2, Decisions, EffectRunner, NewCard, RaisedBy, Risk,
    RiskLevel,
};

/// The card kind and the effect's tool for a mandate.
pub const MANDATE_TOOL: &str = "mandate.sign";

/// How long a raised mandate card waits for the founder before it expires unsigned.
pub const MANDATE_CARD_DAYS: i64 = 7;

/// What the founder lets the company's agents do with money on their own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateTerms {
    /// The company, in plain words: "Agentrix".
    pub company: String,
    /// ISO 4217; every limit is in this currency.
    pub currency: String,
    /// The most one refund may be, in minor units. Refunds go only to the original payer and
    /// never exceed what was captured, whatever this says.
    pub refund_max_each_minor: i64,
    /// The most one bill payment may be, in minor units.
    pub payout_max_each_minor: i64,
    /// The most all bill payments together may be in one UTC day, in minor units.
    pub payout_max_per_day_minor: i64,
    /// The vendors bills may be paid to without asking, by the books' vendor id. Payment goes
    /// only to the payee the books hold for the vendor; a change of payee is always the
    /// founder's call.
    pub payees: Vec<String>,
    /// The most instalments a debt plan may have.
    pub debt_plan_max_instalments: u32,
    /// The longest a debt plan may run, in days from its first instalment.
    pub debt_plan_max_days: u32,
    /// The most model inference may cost in one UTC day, in minor units.
    pub inference_max_per_day_minor: i64,
    /// The plans the company may sell, by catalogue id.
    pub plans: Vec<String>,
    /// Whether a subscription may be suspended after PayPal reports a failed payment.
    pub suspend_on_failed_payment: bool,
    /// After this the mandate no longer applies and every money action is held.
    pub valid_until: DateTime<Utc>,
}

impl MandateTerms {
    /// The terms line by line, as the card and the receipt show them.
    pub fn lines(&self) -> Vec<String> {
        let m = |minor: i64| money_words(&self.currency, minor);
        let list = |items: &[String], none: &str| {
            if items.is_empty() {
                none.to_owned()
            } else {
                items.join(", ")
            }
        };
        let mut lines = vec![
            format!(
                "Refunds up to {} each, only to the person who paid, never more than they paid",
                m(self.refund_max_each_minor)
            ),
            format!(
                "Pay bills up to {} each and {} a day in total, only to the payee the books hold for {}",
                m(self.payout_max_each_minor),
                m(self.payout_max_per_day_minor),
                list(&self.payees, "no vendor yet")
            ),
            "A change of a vendor's payee always comes to you".into(),
            format!(
                "Instalment plans of up to {} instalments over at most {} days",
                self.debt_plan_max_instalments, self.debt_plan_max_days
            ),
            format!(
                "Model spend up to {} a day",
                m(self.inference_max_per_day_minor)
            ),
            format!("Sell {} at catalogue prices", list(&self.plans, "no plans")),
        ];
        if self.suspend_on_failed_payment {
            lines.push("Suspend a subscription after PayPal reports a failed payment".into());
        }
        lines.push(format!(
            "Until {}; anything more comes to you as a card",
            self.valid_until.format("%-d %b %Y")
        ));
        lines
    }

    /// Whether the mandate still applies at `now`.
    pub fn in_force(&self, now: DateTime<Utc>) -> bool {
        now < self.valid_until
    }

    /// The effect the founder's approval signs: the terms themselves, for `site`.
    pub fn effect(&self, site: &SiteId) -> anyhow::Result<EffectRequest> {
        Ok(EffectRequest {
            site: site.clone(),
            webagent: "chief".into(),
            tool: MANDATE_TOOL.into(),
            args: serde_json::to_value(self)?,
            amount: Some(Money {
                currency: self.currency.clone(),
                minor: self.payout_max_per_day_minor,
            }),
            expiry: Utc::now() + Duration::days(MANDATE_CARD_DAYS),
        })
    }

    /// The card that asks the founder to sign these terms.
    pub fn card(&self, site: &SiteId) -> anyhow::Result<NewCard> {
        anyhow::ensure!(
            self.refund_max_each_minor >= 0
                && self.payout_max_each_minor >= 0
                && self.payout_max_per_day_minor >= 0
                && self.inference_max_per_day_minor >= 0,
            "a mandate's limits can't be negative"
        );
        let effect = self.effect(site)?;
        Ok(NewCard {
            kind: MANDATE_TOOL.into(),
            title: format!("Sign the money mandate for {}", self.company),
            summary: "What the company's agents may do with money on their own".into(),
            effects: self.lines(),
            dedupe_key: Some(format!(
                "mandate:{}",
                Digest::of(&serde_json::to_vec(self)?)
            )),
            effect,
            raised_by: RaisedBy {
                webagent: "chief".into(),
                job: Some("mandate".into()),
            },
            risk: Risk {
                level: RiskLevel::High,
                reasons: vec![
                    "Lets agents move money without asking each time".into(),
                    "Only a passkey signs it".into(),
                ],
            },
            approval: ApprovalKind::Reviewed,
            back: None,
        })
    }
}

/// A mandate the founder signed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedMandate {
    /// The terms.
    pub terms: MandateTerms,
    /// The card it was signed on.
    pub card_id: String,
    /// What the passkey signed: the card's fingerprint.
    pub fingerprint: Digest,
    /// The `decision.approved` receipt.
    pub receipt: String,
    /// When it was signed.
    pub signed_at: DateTime<Utc>,
}

impl SignedMandate {
    /// Whether the mandate still applies at `now`.
    pub fn in_force(&self, now: DateTime<Utc>) -> bool {
        self.terms.in_force(now)
    }
}

impl Decisions {
    /// Raises the mandate card for `terms` on `site`. The same terms raise one card.
    pub async fn raise_mandate(
        &self,
        site: &SiteId,
        terms: &MandateTerms,
    ) -> anyhow::Result<DecisionCardV2> {
        self.raise(terms.card(site)?).await
    }

    /// The newest mandate on `site` the founder approved with a passkey. A mandate approved any
    /// other way (an authenticator code) is not a signature and is ignored.
    pub async fn signed_mandate(&self, site: &str) -> anyhow::Result<Option<SignedMandate>> {
        let mut best: Option<SignedMandate> = None;
        for row in self.list(site).await? {
            if row.card.kind != MANDATE_TOOL
                || row.state != CardState::Approved
                || row.method.as_deref() != Some("passkey")
            {
                continue;
            }
            let (Some(receipt), Some(signed_at)) = (row.receipt.clone(), row.decided_at) else {
                continue;
            };
            let terms: MandateTerms = serde_json::from_value(row.card.effect.args.clone())?;
            if best.as_ref().is_none_or(|b| signed_at > b.signed_at) {
                best = Some(SignedMandate {
                    terms,
                    card_id: row.card.id.clone(),
                    fingerprint: row.card.fingerprint,
                    receipt,
                    signed_at,
                });
            }
        }
        Ok(best)
    }
}

/// The runner for mandate cards. Signing runs nothing: the approval and its receipt are the
/// signature, read back with [`Decisions::signed_mandate`].
#[derive(Debug, Default, Clone, Copy)]
pub struct MandateRunner;

#[async_trait]
impl EffectRunner for MandateRunner {
    fn tool(&self) -> &str {
        MANDATE_TOOL
    }

    async fn run(&self, _card: &DecisionCardV2, _receipt: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms() -> MandateTerms {
        MandateTerms {
            company: "Agentrix".into(),
            currency: "USD".into(),
            refund_max_each_minor: 5_000,
            payout_max_each_minor: 50_000,
            payout_max_per_day_minor: 100_000,
            payees: vec!["vultr".into(), "anthropic".into()],
            debt_plan_max_instalments: 4,
            debt_plan_max_days: 120,
            inference_max_per_day_minor: 2_000,
            plans: vec!["business".into()],
            suspend_on_failed_payment: true,
            valid_until: DateTime::parse_from_rfc3339("2026-12-31T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn the_card_states_every_term_in_plain_words() {
        let card = terms().card(&SiteId::new("agentrix-saas")).unwrap();
        assert_eq!(card.kind, MANDATE_TOOL);
        assert_eq!(card.approval, ApprovalKind::Reviewed);
        let all = card.effects.join("\n");
        assert!(all.contains("Refunds up to USD 50 each"), "{all}");
        assert!(all.contains("USD 500 each and USD 1,000 a day"), "{all}");
        assert!(all.contains("vultr, anthropic"), "{all}");
        assert!(all.contains("payee always comes to you"), "{all}");
        assert!(all.contains("4 instalments over at most 120 days"), "{all}");
        assert!(all.contains("Suspend a subscription"), "{all}");
    }

    #[test]
    fn changing_a_term_changes_what_the_passkey_signs() {
        let site = SiteId::new("agentrix-saas");
        let a = terms().effect(&site).unwrap();
        let mut more = terms();
        more.payout_max_each_minor = 500_000;
        let mut b = more.effect(&site).unwrap();
        b.expiry = a.expiry;
        assert_ne!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
    }

    #[test]
    fn a_lapsed_mandate_is_not_in_force() {
        let t = terms();
        assert!(t.in_force(t.valid_until - Duration::seconds(1)));
        assert!(!t.in_force(t.valid_until));
    }
}
