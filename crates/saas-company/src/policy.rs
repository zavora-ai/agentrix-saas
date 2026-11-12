//! The money policy: a deny-by-default [`DeclarativePolicy`] loaded from a policy file per
//! company, with its limits taken from the founder's signed mandate and its facts read from the
//! books and PayPal.
//!
//! The file ([`PolicyFile`]; `policy/agentrix.json`, `policy/kilele.json`) lists rules in match
//! order. A rule names a role (`payables`), which compiles to that company's agent id
//! (`urn:adk:agent:agentrix/payables`), so one company's file never admits another company's
//! agent. A rule's limit can name a mandate term (`"mandate": "payout_max_each_usd"`) instead
//! of a number, and a rule can require a mandate list (`in_mandate`) or flag (`mandate_flag`).
//! [`PolicyFile::compile`] turns the file and the mandate in force into a `DeclarativePolicy`;
//! with no mandate in force, every money tool the file lists is denied first.
//!
//! [`CompanyPolicy`] is the run's [`ToolPolicy`]. Before deciding it reads the call's facts
//! ([`Facts`]): whether the payee is the one the books hold for the vendor, who paid a capture
//! and how much is left on it, what has been paid out today. They go under `/facts` in a copy of
//! the arguments the declarative rules see, so a rule can say "only to the payee on file"
//! without trusting anything the model wrote. A denial carries every reason the call is outside
//! the policy or the mandate, not just the first rule's.

use std::sync::{Arc, RwLock};

use adk_core::{
    ArgPredicate, DeclarativePolicy, PolicyDecision, PolicyRule, ToolPolicy, ToolPolicyRequest,
};
use agentrix_decisions::MandateTerms;
use agentrix_decisions::edit::money_words;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::identity::agent_id_for;

/// Agentrix's policy file.
pub const AGENTRIX_POLICY: &str = include_str!("../policy/agentrix.json");
/// Kilele Bakery's policy file.
pub const KILELE_POLICY: &str = include_str!("../policy/kilele.json");

/// A policy file.
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyFile {
    /// Cited by every audit record as `tool_policy:<name>`.
    pub name: String,
    /// The company slug the roles belong to: `agentrix`, `kilele`.
    pub company: String,
    /// What it is for.
    #[serde(default)]
    pub about: Option<String>,
    /// Tools that move money or change what a customer owes: denied while no mandate is in
    /// force.
    pub money_tools: Vec<String>,
    /// The rules, in match order.
    pub rules: Vec<RuleSpec>,
}

/// One rule.
#[derive(Debug, Clone, Deserialize)]
pub struct RuleSpec {
    /// Tool-name glob.
    pub tool: String,
    /// The role it applies to (`payables`); any agent of any company when absent.
    #[serde(default)]
    pub agent: Option<String>,
    /// What the rule decides.
    pub decision: DecisionSpec,
    /// Why, for deny and require_approval.
    #[serde(default)]
    pub reason: Option<String>,
    /// Every predicate must hold.
    #[serde(default)]
    pub when: Vec<PredicateSpec>,
}

/// A rule's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionSpec {
    /// Run it.
    Allow,
    /// Never run it.
    Deny,
    /// Hold it for the founder.
    RequireApproval,
}

/// A predicate over the call's arguments (and `/facts`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateSpec {
    /// The value at `pointer` equals `value`.
    Equals {
        /// JSON pointer.
        pointer: String,
        /// The value.
        value: Value,
    },
    /// The value at `pointer` is one of `values`.
    InSet {
        /// JSON pointer.
        pointer: String,
        /// The values.
        values: Vec<Value>,
    },
    /// The number at `pointer` is at most `max`, or at most the mandate term `mandate`.
    AtMost {
        /// JSON pointer.
        pointer: String,
        /// A fixed limit.
        #[serde(default)]
        max: Option<f64>,
        /// A mandate term.
        #[serde(default)]
        mandate: Option<MandateLimit>,
    },
    /// The value at `pointer` is in one of the mandate's lists.
    InMandate {
        /// JSON pointer.
        pointer: String,
        /// Which list.
        list: MandateList,
    },
    /// The mandate grants this flag.
    MandateFlag(MandateFlag),
}

/// A mandate limit. Money limits are in the mandate's currency, as whole units with cents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateLimit {
    /// Largest refund.
    RefundMaxEachUsd,
    /// Largest bill payment.
    PayoutMaxEachUsd,
    /// Largest day of bill payments.
    PayoutMaxPerDayUsd,
    /// Most instalments in a debt plan.
    DebtPlanMaxInstalments,
    /// Longest debt plan, in days.
    DebtPlanMaxDays,
}

impl MandateLimit {
    fn of(self, m: &MandateTerms) -> f64 {
        match self {
            Self::RefundMaxEachUsd => dollars(m.refund_max_each_minor),
            Self::PayoutMaxEachUsd => dollars(m.payout_max_each_minor),
            Self::PayoutMaxPerDayUsd => dollars(m.payout_max_per_day_minor),
            Self::DebtPlanMaxInstalments => f64::from(m.debt_plan_max_instalments),
            Self::DebtPlanMaxDays => f64::from(m.debt_plan_max_days),
        }
    }
}

/// A mandate list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateList {
    /// Vendor ids bills may be paid to.
    Payees,
    /// Plan ids.
    Plans,
}

/// A mandate flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateFlag {
    /// A subscription may be suspended after a failed payment.
    SuspendOnFailedPayment,
}

/// Minor units to whole units.
pub fn dollars(minor: i64) -> f64 {
    minor as f64 / 100.0
}

/// A number the model or a fact wrote, as whole units.
pub fn amount_usd(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().trim_start_matches('$').parse().ok(),
        _ => None,
    }
}

/// Whole units to minor units, rounding to the nearest cent.
pub fn to_minor(usd: f64) -> i64 {
    (usd * 100.0).round() as i64
}

impl PolicyFile {
    /// Agentrix's policy.
    pub fn agentrix() -> Self {
        Self::parse(AGENTRIX_POLICY).expect("policy/agentrix.json is valid")
    }

    /// Kilele Bakery's policy.
    pub fn kilele() -> Self {
        Self::parse(KILELE_POLICY).expect("policy/kilele.json is valid")
    }

    /// Parses a policy file.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let file: Self = serde_json::from_str(text)?;
        for rule in &file.rules {
            for p in &rule.when {
                if let PredicateSpec::AtMost { max, mandate, .. } = p {
                    anyhow::ensure!(
                        max.is_some() != mandate.is_some(),
                        "rule for {}: at_most needs exactly one of max or mandate",
                        rule.tool
                    );
                }
            }
            if let Some(role) = &rule.agent {
                agent_id_for(&file.company, role)?;
            }
        }
        Ok(file)
    }

    /// Reads a policy file from disk.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    /// The `DeclarativePolicy` for the mandate in force (`None`: no mandate, or it lapsed).
    /// Default deny.
    pub fn compile(&self, mandate: Option<&MandateTerms>) -> DeclarativePolicy {
        let mut builder = DeclarativePolicy::builder().name(self.name.clone());
        if mandate.is_none() {
            for tool in &self.money_tools {
                builder = builder.deny(
                    tool.clone(),
                    "no founder mandate is in force (none signed with a passkey, or it lapsed), so no money moves",
                );
            }
        }
        'rules: for spec in &self.rules {
            let reason = spec
                .reason
                .clone()
                .unwrap_or_else(|| "the policy says so".into());
            let mut rule = match spec.decision {
                DecisionSpec::Allow => PolicyRule::allow(spec.tool.clone()),
                DecisionSpec::Deny => PolicyRule::deny(spec.tool.clone(), reason),
                DecisionSpec::RequireApproval => {
                    PolicyRule::require_approval(spec.tool.clone(), reason)
                }
            };
            if let Some(role) = &spec.agent {
                match agent_id_for(&self.company, role) {
                    Ok(id) => rule = rule.for_agent(id.as_str().to_owned()),
                    // `parse` checked it; a rule for no agent never matches.
                    Err(_) => continue 'rules,
                }
            }
            for p in &spec.when {
                let predicate = match p {
                    PredicateSpec::Equals { pointer, value } => {
                        ArgPredicate::equals(pointer.clone(), value.clone())
                    }
                    PredicateSpec::InSet { pointer, values } => {
                        ArgPredicate::in_set(pointer.clone(), values.clone())
                    }
                    PredicateSpec::AtMost {
                        pointer,
                        max,
                        mandate: limit,
                    } => {
                        let max = match (max, limit, mandate) {
                            (Some(max), _, _) => *max,
                            (None, Some(limit), Some(m)) => limit.of(m),
                            // A mandate limit with no mandate: the rule can't apply.
                            _ => continue 'rules,
                        };
                        ArgPredicate::at_most(pointer.clone(), max)
                    }
                    PredicateSpec::InMandate { pointer, list } => {
                        let Some(m) = mandate else { continue 'rules };
                        let values = match list {
                            MandateList::Payees => &m.payees,
                            MandateList::Plans => &m.plans,
                        };
                        ArgPredicate::in_set(
                            pointer.clone(),
                            values.iter().map(|v| Value::String(v.clone())),
                        )
                    }
                    PredicateSpec::MandateFlag(MandateFlag::SuspendOnFailedPayment) => {
                        if mandate.is_some_and(|m| m.suspend_on_failed_payment) {
                            continue;
                        }
                        continue 'rules;
                    }
                };
                rule = rule.when(predicate);
            }
            builder = builder.rule(rule);
        }
        builder.build()
    }
}

/// What the policy reads before deciding: facts about the call from the books and PayPal.
#[async_trait]
pub trait Facts: Send + Sync {
    /// The facts for one call, as a JSON object (empty when the tool has none).
    async fn facts(&self, tool: &str, args: &Value) -> Value;
}

/// No facts: every fact-guarded deny rule passes over the call. For tests of the file alone.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoFacts;

#[async_trait]
impl Facts for NoFacts {
    async fn facts(&self, _tool: &str, _args: &Value) -> Value {
        json!({})
    }
}

/// The run's tool policy: the compiled policy file, the mandate in force, and the facts.
pub struct CompanyPolicy {
    file: PolicyFile,
    mandate: RwLock<Option<MandateTerms>>,
    compiled: RwLock<DeclarativePolicy>,
    facts: Arc<dyn Facts>,
}

impl std::fmt::Debug for CompanyPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompanyPolicy")
            .field("name", &self.file.name)
            .field("mandate", &self.mandate())
            .finish()
    }
}

impl CompanyPolicy {
    /// The policy for `file`, with no mandate yet: every money tool is denied.
    pub fn new(file: PolicyFile, facts: Arc<dyn Facts>) -> Self {
        let compiled = file.compile(None);
        Self {
            file,
            mandate: RwLock::new(None),
            compiled: RwLock::new(compiled),
            facts,
        }
    }

    /// Puts `mandate` in force (or none), recompiling the rules. A mandate that has lapsed at
    /// `now` is none.
    pub fn set_mandate(&self, mandate: Option<MandateTerms>, now: DateTime<Utc>) {
        let mandate = mandate.filter(|m| m.in_force(now));
        let compiled = self.file.compile(mandate.as_ref());
        *self.compiled.write().unwrap_or_else(|p| p.into_inner()) = compiled;
        *self.mandate.write().unwrap_or_else(|p| p.into_inner()) = mandate;
    }

    /// The mandate in force.
    pub fn mandate(&self) -> Option<MandateTerms> {
        self.mandate
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The policy file.
    pub fn file(&self) -> &PolicyFile {
        &self.file
    }

    /// The policy file's name.
    pub fn name(&self) -> &str {
        &self.file.name
    }

    /// Whether `tool` moves money (the file's `money_tools`).
    pub fn is_money_tool(&self, tool: &str) -> bool {
        self.file.money_tools.iter().any(|t| t == tool)
    }

    /// The arguments the rules see: the call's own, with the facts under `/facts`. Anything
    /// the model put under `facts` is replaced.
    pub async fn with_facts(&self, tool: &str, args: &Value) -> Value {
        let facts = self.facts.facts(tool, args).await;
        let mut seen = match args {
            Value::Object(_) => args.clone(),
            other => json!({ "args": other }),
        };
        seen["facts"] = facts;
        seen
    }

    /// Every reason a call is outside the policy or the mandate, in plain words. Empty when
    /// the call is inside both.
    pub async fn explain(&self, tool: &str, args: &Value) -> Vec<String> {
        let seen = self.with_facts(tool, args).await;
        let f = &seen["facts"];
        let mandate = self.mandate();
        let currency = mandate
            .as_ref()
            .map(|m| m.currency.clone())
            .unwrap_or_else(|| "USD".into());
        let money = |usd: f64| money_words(&currency, to_minor(usd));
        let mut reasons = Vec::new();
        if mandate.is_none() && self.is_money_tool(tool) {
            reasons.push(
                "no founder mandate is in force (none signed with a passkey, or it lapsed)".into(),
            );
        }
        let amount = amount_usd(args.get("amount_usd"));
        match tool {
            "pay_bill" => {
                if f["bill_open"] == json!(false) {
                    reasons.push(format!(
                        "the books have no open bill {}",
                        args["bill_id"].as_str().unwrap_or("(none named)")
                    ));
                }
                if f["payee_on_file"] == json!(false) {
                    reasons.push(format!(
                        "{} isn't the payee the books hold for {} ({}); a change of payee goes to the founder first",
                        args["payee"].as_str().unwrap_or("that payee"),
                        f["vendor_name"].as_str().unwrap_or("the vendor"),
                        f["payee_of_record"].as_str().unwrap_or("unknown")
                    ));
                }
                if f["within_bill"] == json!(false) {
                    reasons.push(format!(
                        "{} is more than the {} left on the bill",
                        amount.map(money).unwrap_or_else(|| "the amount".into()),
                        amount_usd(f.get("bill_due_usd"))
                            .map(money)
                            .unwrap_or_else(|| "nothing".into())
                    ));
                }
                if let Some(m) = &mandate {
                    if let Some(vendor) = f["vendor_id"].as_str()
                        && !m.payees.iter().any(|p| p == vendor)
                    {
                        reasons.push(format!("the founder's mandate doesn't list {vendor}"));
                    }
                    if let Some(a) = amount
                        && to_minor(a) > m.payout_max_each_minor
                    {
                        reasons.push(format!(
                            "{} is over the founder's {} per payment",
                            money(a),
                            money_words(&m.currency, m.payout_max_each_minor)
                        ));
                    }
                    if let Some(after) = amount_usd(f.get("day_total_after_usd"))
                        && to_minor(after) > m.payout_max_per_day_minor
                    {
                        reasons.push(format!(
                            "payments today would come to {}, over the founder's {} a day",
                            money(after),
                            money_words(&m.currency, m.payout_max_per_day_minor)
                        ));
                    }
                }
            }
            "change_vendor_payee" => {
                reasons.push("a change of a vendor's payee is always the founder's call".into());
            }
            "refund_payment" => {
                if f["capture_found"] == json!(false) {
                    reasons.push(format!(
                        "PayPal has no capture {}",
                        args["capture_id"].as_str().unwrap_or("(none named)")
                    ));
                }
                if f["to_original_payer"] == json!(false) {
                    reasons.push(format!(
                        "the refund would go to {}, but a refund goes only to the person who paid ({}), from PayPal's capture record",
                        args["refund_to"].as_str().unwrap_or("someone else"),
                        f["original_payer"].as_str().unwrap_or("unknown")
                    ));
                }
                if f["within_capture"] == json!(false) {
                    reasons.push(format!(
                        "{} is more than the {} left to refund on that capture",
                        amount.map(money).unwrap_or_else(|| "the amount".into()),
                        amount_usd(f.get("refundable_usd"))
                            .map(money)
                            .unwrap_or_else(|| "nothing".into())
                    ));
                }
                if let (Some(m), Some(a)) = (&mandate, amount)
                    && to_minor(a) > m.refund_max_each_minor
                {
                    reasons.push(format!(
                        "{} is over the founder's limit of {} per refund",
                        money(a),
                        money_words(&m.currency, m.refund_max_each_minor)
                    ));
                }
            }
            "propose_debt_plan" => {
                if f["invoice_open"] == json!(false) {
                    reasons.push("the invoice isn't open".into());
                }
                if f["sums_to_balance"] == json!(false) {
                    reasons.push("the instalments don't add up to what is owed".into());
                }
                if let Some(m) = &mandate {
                    if let Some(n) = f["instalment_count"].as_u64()
                        && n > u64::from(m.debt_plan_max_instalments)
                    {
                        reasons.push(format!(
                            "{n} instalments is more than the founder's {}",
                            m.debt_plan_max_instalments
                        ));
                    }
                    if let Some(d) = f["plan_days"].as_u64()
                        && d > u64::from(m.debt_plan_max_days)
                    {
                        reasons.push(format!(
                            "{d} days is longer than the founder's {}",
                            m.debt_plan_max_days
                        ));
                    }
                }
            }
            "start_subscription" | "create_recurring_invoice" => {
                if f["plan_listed"] == json!(false) {
                    reasons.push("that plan isn't in the catalogue".into());
                }
                if tool == "start_subscription"
                    && let Some(m) = &mandate
                {
                    let plan = args["plan_id"].as_str().unwrap_or_default();
                    if !m.plans.iter().any(|p| p == plan) {
                        reasons.push(format!(
                            "the founder's mandate doesn't list the {plan} plan"
                        ));
                    }
                }
            }
            _ => {}
        }
        reasons
    }
}

#[async_trait]
impl ToolPolicy for CompanyPolicy {
    async fn evaluate(&self, request: &ToolPolicyRequest) -> PolicyDecision {
        let mut seen = request.clone();
        seen.args = self.with_facts(&request.tool_name, &request.args).await;
        let decision = self
            .compiled
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .decide(&seen);
        match decision {
            PolicyDecision::Deny { reason } => {
                let mut reasons = self.explain(&request.tool_name, &request.args).await;
                if !reasons
                    .iter()
                    .any(|r| r.contains(&reason) || reason.contains(r.as_str()))
                {
                    reasons.insert(0, reason);
                }
                PolicyDecision::deny(reasons.join("; "))
            }
            other => other,
        }
    }

    fn policy_ref(&self) -> Option<String> {
        Some(self.file.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{ActingContext, AgentPrincipal};

    fn request(company: &str, role: &str, tool: &str, args: Value) -> ToolPolicyRequest {
        let id = agent_id_for(company, role).unwrap();
        ToolPolicyRequest::new(tool, args)
            .with_acting_context(ActingContext::new(AgentPrincipal::new(id)))
    }

    fn terms() -> MandateTerms {
        MandateTerms {
            company: "Agentrix".into(),
            currency: "USD".into(),
            refund_max_each_minor: 5_000,
            payout_max_each_minor: 50_000,
            payout_max_per_day_minor: 100_000,
            payees: vec!["vultr".into()],
            debt_plan_max_instalments: 4,
            debt_plan_max_days: 120,
            inference_max_per_day_minor: 2_000,
            plans: vec!["business".into()],
            suspend_on_failed_payment: false,
            valid_until: Utc::now() + chrono::Duration::days(30),
        }
    }

    #[test]
    fn both_files_parse_and_deny_by_default() {
        for file in [PolicyFile::agentrix(), PolicyFile::kilele()] {
            let policy = file.compile(None);
            let unknown = request(&file.company, "payables", "delete_everything", json!({}));
            assert!(matches!(
                policy.decide(&unknown),
                PolicyDecision::Deny { .. }
            ));
            let pay = request(
                &file.company,
                "payables",
                "pay_bill",
                json!({ "amount_usd": 1 }),
            );
            let PolicyDecision::Deny { reason } = policy.decide(&pay) else {
                panic!("no mandate, no payments");
            };
            assert!(reason.contains("no founder mandate"), "{reason}");
        }
    }

    #[test]
    fn a_role_admits_only_its_own_company() {
        let policy = PolicyFile::agentrix().compile(Some(&terms()));
        let ours = request("agentrix", "payables", "ap_bills_due", json!({}));
        let theirs = request("kilele", "payables", "ap_bills_due", json!({}));
        let other_role = request("agentrix", "support", "ap_bills_due", json!({}));
        assert_eq!(policy.decide(&ours), PolicyDecision::Allow);
        assert!(matches!(
            policy.decide(&theirs),
            PolicyDecision::Deny { .. }
        ));
        assert!(matches!(
            policy.decide(&other_role),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn a_payment_inside_the_mandate_runs_and_one_over_it_is_held() {
        let policy = PolicyFile::agentrix().compile(Some(&terms()));
        let facts = json!({ "bill_open": true, "payee_on_file": true, "within_bill": true,
            "vendor_id": "vultr", "day_total_after_usd": 120.0 });
        let small = request(
            "agentrix",
            "payables",
            "pay_bill",
            json!({ "amount_usd": 120, "facts": facts }),
        );
        assert_eq!(policy.decide(&small), PolicyDecision::Allow);
        let big = request(
            "agentrix",
            "payables",
            "pay_bill",
            json!({ "amount_usd": 900, "facts": facts }),
        );
        assert!(matches!(
            policy.decide(&big),
            PolicyDecision::RequireApproval { .. }
        ));
        let mut changed = facts.clone();
        changed["payee_on_file"] = json!(false);
        let fraud = request(
            "agentrix",
            "payables",
            "pay_bill",
            json!({ "amount_usd": 120, "facts": changed }),
        );
        let PolicyDecision::Deny { reason } = policy.decide(&fraud) else {
            panic!("a new payee is never paid");
        };
        assert!(reason.contains("payee"), "{reason}");
    }

    #[test]
    fn a_change_of_payee_is_always_held() {
        let policy = PolicyFile::agentrix().compile(Some(&terms()));
        let change = request(
            "agentrix",
            "payables",
            "change_vendor_payee",
            json!({ "vendor_id": "vultr", "payee": "new@example.com" }),
        );
        assert!(matches!(
            policy.decide(&change),
            PolicyDecision::RequireApproval { .. }
        ));
    }

    #[tokio::test]
    async fn a_policy_without_a_mandate_explains_itself() {
        let policy = CompanyPolicy::new(PolicyFile::agentrix(), Arc::new(NoFacts));
        let reasons = policy
            .explain("pay_bill", &json!({ "bill_id": "b1", "amount_usd": 5 }))
            .await;
        assert_eq!(reasons.len(), 1, "{reasons:?}");
        assert!(policy.is_money_tool("pay_bill"));
        assert!(!policy.is_money_tool("ap_bills_due"));
    }
}
