//! The built-in story, until slices A (books) and B (company) merge.
//!
//! Every date is counted from `as_of`, so ageing reads the same on any day and tests never
//! depend on the clock. Every PayPal id here starts with `DEMO-`: real ids come only from the
//! sandbox (`specs/saas/EVIDENCE.md`), never from this file.
//!
//! The owner's actions (freeze, approve, decline) change [`DemoFeed`]'s state in memory, so the
//! consoles and the board show their effect until the node restarts.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Datelike, Days, Months, NaiveDate};

use crate::console::{Console, ConsoleError, restore_fingerprint};
use crate::feed::*;

/// The invoice numbers the story names. The defaults are the story's own; seeded books
/// ([`crate::seed`]) number their invoices themselves, and the story follows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numbers {
    /// Kilele's annual Business plan, now on an instalment plan.
    pub kilele_annual: String,
    /// Kilele's October invoice, settled agent to agent.
    pub kilele_october: String,
}

impl Default for Numbers {
    fn default() -> Self {
        Self {
            kilele_annual: "INV-1042".into(),
            kilele_october: "INV-1029".into(),
        }
    }
}

/// The story as a [`FeedSource`] and a [`Console`].
#[derive(Clone, Default)]
pub struct DemoFeed {
    state: Arc<Mutex<State>>,
    numbers: Numbers,
}

#[derive(Default)]
struct State {
    frozen: BTreeMap<String, bool>,
    decided: BTreeMap<String, String>,
    /// (company, agent) → when the owner revoked it.
    revoked: BTreeMap<(String, String), chrono::DateTime<chrono::Utc>>,
}

impl DemoFeed {
    /// A fresh story: nothing frozen, every decision open.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same story naming these invoice numbers.
    pub fn with_numbers(mut self, numbers: Numbers) -> Self {
        self.numbers = numbers;
        self
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl FeedSource for DemoFeed {
    async fn feed(&self, as_of: NaiveDate) -> anyhow::Result<Feed> {
        let mut feed = story_numbered(as_of, &self.numbers);
        let state = self.state();
        for c in &mut feed.companies {
            c.frozen = state.frozen.get(&c.id).copied().unwrap_or(false);
            if c.frozen {
                for a in c.agents.iter_mut().filter(|a| a.id != "chief") {
                    a.state = "frozen".into();
                    a.doing = "Frozen by the owner".into();
                }
            }
        }
        for c in &mut feed.companies {
            for a in &mut c.agents {
                // Revoked outranks frozen, as in the node's feed.
                if state.revoked.contains_key(&(c.id.clone(), a.id.clone())) {
                    a.state = "revoked".into();
                    a.doing = "Revoked by the owner: its tokens stop at its next call".into();
                }
            }
        }
        for m in &mut feed.mandates {
            if state.frozen.get(&m.company).copied().unwrap_or(false) {
                m.status = "frozen".into();
            }
        }
        for d in &mut feed.decisions {
            if let Some(s) = state.decided.get(&d.id) {
                d.state = s.clone();
            }
        }
        Ok(feed)
    }
}

#[async_trait]
impl crate::books_feed::OffBooks for DemoFeed {
    async fn facts(&self, as_of: NaiveDate) -> anyhow::Result<crate::books_feed::Facts> {
        let f = self.feed(as_of).await?;
        Ok(crate::books_feed::Facts {
            companies: f.companies,
            mandates: f.mandates,
            decisions: f.decisions,
            debt_plans: f.debt_plans,
            settlements: f.settlements,
            agent_spend: f.agent_spend,
            audit: f.audit,
            other_cash: f.cash.into_iter().filter(|c| c.rail != "PayPal").collect(),
        })
    }
}

#[async_trait]
impl Console for DemoFeed {
    async fn set_frozen(&self, company: &str, frozen: bool) -> Result<(), ConsoleError> {
        if company != AGENTRIX && company != KILELE {
            return Err(ConsoleError::NotFound);
        }
        self.state().frozen.insert(company.to_owned(), frozen);
        Ok(())
    }

    async fn decide(
        &self,
        company: &str,
        decision: &str,
        approve: bool,
        _proof: Option<serde_json::Value>,
    ) -> Result<String, ConsoleError> {
        let open = story(NaiveDate::default())
            .decisions
            .into_iter()
            .any(|d| d.id == decision && d.company == company && d.state == "open");
        if !open {
            return Err(ConsoleError::NotFound);
        }
        let mut state = self.state();
        if state.frozen.get(company).copied().unwrap_or(false) && approve {
            return Err(ConsoleError::Frozen);
        }
        if let Some(already) = state.decided.get(decision) {
            return Err(ConsoleError::Decided(already.clone()));
        }
        let outcome = if approve { "approved" } else { "declined" };
        state
            .decided
            .insert(decision.to_owned(), outcome.to_owned());
        Ok(outcome.to_owned())
    }

    async fn set_revoked(
        &self,
        company: &str,
        role: &str,
        revoked: bool,
        proof: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ConsoleError> {
        let known = story(NaiveDate::default())
            .companies
            .iter()
            .any(|c| c.id == company && c.agents.iter().any(|a| a.id == role));
        if !known {
            return Err(ConsoleError::NotFound);
        }
        let mut state = self.state();
        let key = (company.to_owned(), role.to_owned());
        if revoked {
            state.revoked.entry(key).or_insert_with(chrono::Utc::now);
        } else {
            // The console checked the passkey assertion; the demo checks what it signed and
            // that the owner was verified, as the node does.
            let proof = proof.unwrap_or_default();
            let want = restore_fingerprint(company, role);
            if proof.get("fingerprint").and_then(serde_json::Value::as_str) != Some(want.as_str()) {
                return Err(ConsoleError::BadProof(
                    "it signed something other than switching this agent back on".into(),
                ));
            }
            if proof
                .get("userVerified")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
            {
                return Err(ConsoleError::BadProof("the owner wasn't verified".into()));
            }
            if state.revoked.remove(&key).is_none() {
                return Err(ConsoleError::NotFound);
            }
        }
        Ok(serde_json::json!({"agent": role, "revoked": revoked, "audit_seq": null}))
    }
}

fn day(as_of: NaiveDate, offset: i64) -> NaiveDate {
    if offset >= 0 {
        as_of.checked_add_days(Days::new(offset as u64))
    } else {
        as_of.checked_sub_days(Days::new(offset.unsigned_abs()))
    }
    .unwrap_or(as_of)
}

fn agent(id: &str, name: &str, job: &str, state: &str, doing: &str, company: &str) -> Agent {
    Agent {
        id: id.into(),
        name: name.into(),
        job: job.into(),
        state: state.into(),
        doing: doing.into(),
        key_id: Some(format!("{company}-{id}-2026-11")),
    }
}

#[allow(clippy::too_many_arguments)]
fn invoice(
    as_of: NaiveDate,
    company: &str,
    number: &str,
    customer: &str,
    description: &str,
    issued: i64,
    due: i64,
    amount: i64,
    open: i64,
    paypal: Option<&str>,
) -> Receivable {
    let due_date = day(as_of, due);
    let (bucket_name, late) = bucket(due_date, as_of);
    let status = if open == 0 {
        "paid"
    } else if open < amount {
        "partial"
    } else {
        "open"
    };
    Receivable {
        company: company.into(),
        invoice_id: format!("era-{}", number.to_ascii_lowercase()),
        number: number.into(),
        customer: customer.into(),
        description: description.into(),
        issued: day(as_of, issued),
        due: due_date,
        currency: "USD".into(),
        amount_minor: amount,
        open_minor: open,
        days_overdue: if open == 0 { 0 } else { late },
        bucket: if open == 0 { "paid" } else { bucket_name }.into(),
        status: status.into(),
        paypal_id: paypal.map(str::to_owned),
    }
}

#[allow(clippy::too_many_arguments)]
fn bill(
    as_of: NaiveDate,
    company: &str,
    number: &str,
    vendor: &str,
    description: &str,
    due: i64,
    amount: i64,
    open: i64,
    rail: &str,
    mandate: &str,
    status: &str,
) -> Payable {
    Payable {
        company: company.into(),
        bill_id: format!("era-{}", number.to_ascii_lowercase()),
        number: number.into(),
        vendor: vendor.into(),
        description: description.into(),
        due: day(as_of, due),
        currency: "USD".into(),
        amount_minor: amount,
        open_minor: open,
        days_to_due: due,
        rail: rail.into(),
        mandate: mandate.into(),
        status: status.into(),
    }
}

/// A stable stand-in for the fingerprint a passkey signs: the real one is the effect's
/// `sha256:` from agentrix-effects, supplied by slice B's decisions.
fn fingerprint(id: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut out = String::from("sha256:");
    for salt in 0..4u8 {
        let mut h = DefaultHasher::new();
        (id, salt).hash(&mut h);
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// The story at `as_of`, with its own invoice numbers.
pub fn story(as_of: NaiveDate) -> Feed {
    story_numbered(as_of, &Numbers::default())
}

/// The story at `as_of`, naming `n`'s invoice numbers.
pub fn story_numbered(as_of: NaiveDate, n: &Numbers) -> Feed {
    let annual = n.kilele_annual.as_str();
    let october = n.kilele_october.as_str();
    let hosts = crate::hosts::Hosts::from_env();
    let companies = vec![
        Company {
            id: AGENTRIX.into(),
            name: "Agentrix".into(),
            host: hosts.money.clone(),
            owner: "James".into(),
            frozen: false,
            agents: vec![
                agent(
                    "chief",
                    "Chief",
                    "Runs the company inside James's mandate and reports to him.",
                    "idle",
                    "Writing the morning cash briefing",
                    AGENTRIX,
                ),
                agent(
                    "revenue",
                    "Revenue",
                    "Sells WebAgent plans at the sales desk and keeps the CRM.",
                    "answering",
                    "Answering a visitor about the Business plan",
                    AGENTRIX,
                ),
                agent(
                    "receivables",
                    "Receivables",
                    "Invoices customers, collects, and agrees instalment plans.",
                    "waiting",
                    &format!("Instalment 2 of {annual} is due in 27 days"),
                    AGENTRIX,
                ),
                agent(
                    "payables",
                    "Payables",
                    "Pays Agentrix's bills through PayPal Payouts inside the founder's mandate.",
                    "waiting",
                    "Waiting for James's OK on a $360.00 payment",
                    AGENTRIX,
                ),
            ],
        },
        Company {
            id: KILELE.into(),
            name: "Kilele Bakery".into(),
            host: hosts.kilele.clone(),
            owner: "Wanjiru".into(),
            frozen: false,
            agents: vec![
                agent(
                    "chief",
                    "Chief",
                    "Runs the bakery's back office inside Wanjiru's mandate.",
                    "idle",
                    "Resting until the 06:00 bake",
                    KILELE,
                ),
                agent(
                    "payables",
                    "Payables",
                    "Checks bills against Wanjiru's mandate and settles them on PayPal.",
                    "idle",
                    "Instalment 2 to Agentrix is scheduled",
                    KILELE,
                ),
            ],
        },
    ];

    let mut receivables = vec![
        invoice(
            as_of,
            AGENTRIX,
            annual,
            "Kilele Bakery",
            "Business plan, annual (Nov 2026 to Oct 2027)",
            -40,
            -10,
            75_840,
            50_560,
            None,
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1038",
            "Tana Valley Times",
            "Business plan, September",
            -65,
            -35,
            7_900,
            7_900,
            None,
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1040",
            "Classy Perfumes",
            "Essential plan, October",
            -35,
            -5,
            2_900,
            2_900,
            None,
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1031",
            "Mama Njeri's Kitchen",
            "Essential plan, August",
            -100,
            -70,
            2_900,
            2_900,
            None,
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1044",
            "Kestrel Outfitters",
            "Commerce plan, November",
            -2,
            28,
            14_900,
            14_900,
            None,
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1045",
            "Riverside Dental",
            "Essential plan, November",
            -1,
            29,
            2_900,
            2_900,
            None,
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1039",
            "Classy Perfumes",
            "Essential plan, September",
            -66,
            -36,
            2_900,
            0,
            Some("DEMO-4HR51007"),
        ),
        invoice(
            as_of,
            AGENTRIX,
            october,
            "Kilele Bakery",
            "Business plan, October",
            -45,
            -31,
            7_900,
            0,
            Some("DEMO-8KD20419"),
        ),
        invoice(
            as_of,
            KILELE,
            "KB-0311",
            "Java Corner Westlands",
            "Wholesale bread, week 44",
            -12,
            2,
            18_400,
            18_400,
            None,
        ),
        invoice(
            as_of,
            KILELE,
            "KB-0307",
            "Green Leaf Cafe",
            "Wholesale pastries, week 43",
            -19,
            -5,
            9_650,
            9_650,
            None,
        ),
    ];
    // Paid history: what put the cash on PayPal in the first place.
    receivables.extend([
        invoice(
            as_of,
            AGENTRIX,
            "INV-1033",
            "Kestrel Outfitters",
            "Commerce plan, October",
            -32,
            -2,
            14_900,
            0,
            Some("DEMO-6JW30188"),
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1035",
            "Tana Valley Times",
            "Business plan, August",
            -95,
            -65,
            7_900,
            0,
            Some("DEMO-1QA77412"),
        ),
        invoice(
            as_of,
            AGENTRIX,
            "INV-1036",
            "Riverside Dental",
            "Essential plan, October",
            -31,
            -1,
            2_900,
            0,
            Some("DEMO-9MZ11843"),
        ),
        invoice(
            as_of,
            KILELE,
            "KB-0280",
            "Java Corner Westlands",
            "Wholesale bread, week 40",
            -40,
            -26,
            62_000,
            0,
            Some("DEMO-3TK55120"),
        ),
        invoice(
            as_of,
            KILELE,
            "KB-0285",
            "Green Leaf Cafe",
            "Wholesale pastries, week 41",
            -33,
            -19,
            91_000,
            0,
            Some("DEMO-7RB20947"),
        ),
    ]);
    // Kilele's annual invoice is covered by the instalment plan below.
    if let Some(r) = receivables.iter_mut().find(|r| r.number == annual) {
        r.status = "on_plan".into();
    }

    let ageing = ageing(
        &receivables,
        &[
            (AGENTRIX, "Kilele Bakery", 150_000),
            (AGENTRIX, "Tana Valley Times", 30_000),
            (AGENTRIX, "Classy Perfumes", 10_000),
            (AGENTRIX, "Mama Njeri's Kitchen", 5_000),
            (KILELE, "Java Corner Westlands", 40_000),
        ],
    );

    let payables = vec![
        bill(
            as_of,
            AGENTRIX,
            "AMN-0007",
            "Amina Yusuf (contractor)",
            "Design for the plans page, 12 hours",
            1,
            36_000,
            36_000,
            "paypal_payouts",
            "over",
            "held",
        ),
        bill(
            as_of,
            AGENTRIX,
            "ANT-2026-10",
            "Anthropic",
            "Claude usage, October",
            3,
            3_875,
            3_875,
            "paypal_payouts",
            "within",
            "scheduled",
        ),
        bill(
            as_of,
            AGENTRIX,
            "VLT-2026-10",
            "Vultr",
            "saas-node and Inference, October",
            5,
            4_620,
            4_620,
            "paypal_payouts",
            "within",
            "scheduled",
        ),
        bill(
            as_of,
            AGENTRIX,
            "REF-2026-10",
            "Tana Valley Times (referral)",
            "Referral share, October",
            0,
            2_000,
            2_000,
            "paypal_payouts",
            "within",
            "open",
        ),
        bill(
            as_of,
            AGENTRIX,
            "VLT-2026-09",
            "Vultr",
            "saas-node and Inference, September",
            -25,
            4_410,
            0,
            "paypal_payouts",
            "within",
            "paid",
        ),
        bill(
            as_of,
            KILELE,
            annual,
            "Agentrix",
            "Business plan, annual: instalment 2 of 3",
            27,
            25_280,
            25_280,
            "paypal_orders",
            "within",
            "scheduled",
        ),
        bill(
            as_of,
            KILELE,
            "UNG-5520",
            "Unga Millers",
            "Flour, 40 bags",
            6,
            42_000,
            42_000,
            "paypal_payouts",
            "over",
            "held",
        ),
    ];

    let instalments = vec![
        Instalment {
            n: 1,
            due: day(as_of, -3),
            amount_minor: 25_280,
            status: "paid".into(),
            paypal_id: Some("DEMO-2PF93355".into()),
        },
        Instalment {
            n: 2,
            due: day(as_of, 27),
            amount_minor: 25_280,
            status: "scheduled".into(),
            paypal_id: None,
        },
        Instalment {
            n: 3,
            due: day(as_of, 57),
            amount_minor: 25_280,
            status: "scheduled".into(),
            paypal_id: None,
        },
    ];
    let debt_plans = vec![DebtPlan {
        id: "plan-inv-1042".into(),
        creditor: AGENTRIX.into(),
        debtor: KILELE.into(),
        invoice_number: annual.into(),
        currency: "USD".into(),
        principal_minor: 75_840,
        paid_minor: 25_280,
        status: "active".into(),
        agreed: day(as_of, -4),
        mandate_ids: vec![
            "mandate-agentrix-receivables".into(),
            "mandate-kilele-payables".into(),
        ],
        instalments,
    }];

    let cash = vec![
        CashBalance {
            company: AGENTRIX.into(),
            rail: "PayPal".into(),
            account: "PayPal clearing".into(),
            currency: "USD".into(),
            balance_minor: 421_240,
            mode: "sandbox".into(),
        },
        CashBalance {
            company: AGENTRIX.into(),
            rail: "M-Pesa".into(),
            account: "M-Pesa till".into(),
            currency: "KES".into(),
            balance_minor: 3_850_000,
            mode: "demo".into(),
        },
        CashBalance {
            company: KILELE.into(),
            rail: "PayPal".into(),
            account: "PayPal clearing".into(),
            currency: "USD".into(),
            balance_minor: 124_000,
            mode: "sandbox".into(),
        },
        CashBalance {
            company: KILELE.into(),
            rail: "M-Pesa".into(),
            account: "M-Pesa till".into(),
            currency: "KES".into(),
            balance_minor: 15_230_000,
            mode: "demo".into(),
        },
    ];

    let first = NaiveDate::from_ymd_opt(as_of.year(), as_of.month(), 1).unwrap_or(as_of);
    let mrr_series: [(i64, i64, i64, u32); 6] = [
        (41_200, 8_700, 0, 11),
        (53_100, 14_800, 2_900, 14),
        (68_800, 18_600, 2_900, 18),
        (80_200, 14_300, 2_900, 21),
        (95_700, 18_400, 2_900, 25),
        (105_400, 12_600, 2_900, 27),
    ];
    let mrr = mrr_series
        .iter()
        .enumerate()
        .map(|(i, (m, new, churn, customers))| {
            let month = first
                .checked_sub_months(Months::new((5 - i) as u32))
                .unwrap_or(first);
            MrrPoint {
                month: month.format("%Y-%m").to_string(),
                currency: "USD".into(),
                mrr_minor: *m,
                new_minor: *new,
                churn_minor: *churn,
                customers: *customers,
            }
        })
        .collect();

    let mut agent_spend = Vec::new();
    for (company, agent_id, base, paid, calls) in [
        (AGENTRIX, "chief", 41, 0, 9),
        (AGENTRIX, "revenue", 63, 0, 22),
        (AGENTRIX, "receivables", 18, 0, 7),
        (AGENTRIX, "payables", 12, 2_000, 5),
        (KILELE, "chief", 9, 0, 3),
        (KILELE, "payables", 7, 0, 2),
    ] {
        for back in 0..7i64 {
            let wobble = (back * 7 + base) % 11;
            agent_spend.push(AgentSpend {
                company: company.into(),
                agent: agent_id.into(),
                day: day(as_of, -back),
                currency: "USD".into(),
                model_minor: base + wobble,
                paid_out_minor: if back == 3 && agent_id == "payables" {
                    if company == KILELE { 25_280 } else { paid }
                } else {
                    0
                },
                tool_calls: calls + (wobble as u32 % 4),
            });
        }
    }

    let mandates = vec![
        Mandate {
            id: "mandate-agentrix-payables".into(),
            company: AGENTRIX.into(),
            owner: "James".into(),
            agent: "payables".into(),
            scope: "Pay Agentrix's bills through PayPal Payouts: hosting, models, contractors and referral partners".into(),
            currency: "USD".into(),
            per_payment_minor: 20_000,
            daily_minor: 20_000,
            used_today_minor: 5_875,
            payees: vec!["Vultr".into(), "Anthropic".into(), "Contractors on file".into(), "Referral partners".into()],
            signed_with: "passkey".into(),
            signed: day(as_of, -9),
            expires: day(as_of, 51),
            status: "active".into(),
        },
        Mandate {
            id: "mandate-agentrix-receivables".into(),
            company: AGENTRIX.into(),
            owner: "James".into(),
            agent: "receivables".into(),
            scope: "Agree instalment plans with customers: at most 3 instalments over 90 days".into(),
            currency: "USD".into(),
            per_payment_minor: 150_000,
            daily_minor: 150_000,
            used_today_minor: 0,
            payees: vec!["Customers inside their ERA credit limit".into()],
            signed_with: "passkey".into(),
            signed: day(as_of, -9),
            expires: day(as_of, 51),
            status: "active".into(),
        },
        Mandate {
            id: "mandate-kilele-payables".into(),
            company: KILELE.into(),
            owner: "Wanjiru".into(),
            agent: "payables".into(),
            scope: "Settle Agentrix's invoices on PayPal, and agree instalments for them".into(),
            currency: "USD".into(),
            per_payment_minor: 50_000,
            daily_minor: 50_000,
            used_today_minor: 0,
            payees: vec!["Agentrix".into()],
            signed_with: "passkey".into(),
            signed: day(as_of, -6),
            expires: day(as_of, 85),
            status: "active".into(),
        },
    ];

    let decisions = vec![
        Decision {
            id: "dec-amn-0007".into(),
            company: AGENTRIX.into(),
            agent: "payables".into(),
            title: "Pay Amina Yusuf $360.00 for design work".into(),
            body: "Bill AMN-0007, 12 hours on the plans page, due tomorrow. PayPal Payouts to the address on file.".into(),
            effects: vec![
                "$360.00 leaves PayPal by Payouts, once".into(),
                "ERA posts the payment against AMN-0007".into(),
            ],
            why: "Your mandate lets Payables pay up to $200.00 a payment. This one is $360.00.".into(),
            currency: "USD".into(),
            amount_minor: Some(36_000),
            state: "open".into(),
            fingerprint: fingerprint("dec-amn-0007"),
            requested_by: None,
        },
        Decision {
            id: "dec-ung-5520".into(),
            company: KILELE.into(),
            agent: "payables".into(),
            title: "Pay Unga Millers $420.00 for 40 bags of flour".into(),
            body: "Bill UNG-5520, due in 6 days. Unga Millers is not a payee in your mandate.".into(),
            effects: vec![
                "$420.00 leaves Kilele's PayPal by Payouts, once".into(),
                "ERA posts the payment against UNG-5520".into(),
            ],
            why: "Your mandate covers Agentrix's invoices only.".into(),
            currency: "USD".into(),
            amount_minor: Some(42_000),
            state: "open".into(),
            fingerprint: fingerprint("dec-ung-5520"),
            requested_by: None,
        },
    ];

    let route =
        "A2A · signed request (Web Bot Auth draft) · AP2-style cart mandate · PayPal Orders v2";
    let settlements = vec![
        Settlement {
            at: day(as_of, -3),
            payer: KILELE.into(),
            payee: AGENTRIX.into(),
            invoice_number: format!("{annual} (1 of 3)"),
            currency: "USD".into(),
            amount_minor: 25_280,
            paypal_id: "DEMO-2PF93355".into(),
            status: "settled".into(),
            route: route.into(),
        },
        Settlement {
            at: day(as_of, -31),
            payer: KILELE.into(),
            payee: AGENTRIX.into(),
            invoice_number: october.into(),
            currency: "USD".into(),
            amount_minor: 7_900,
            paypal_id: "DEMO-8KD20419".into(),
            status: "settled".into(),
            route: route.into(),
        },
    ];

    Feed {
        version: VERSION,
        as_of,
        source: "demo".into(),
        companies,
        receivables,
        ageing,
        payables,
        debt_plans,
        cash,
        mrr,
        agent_spend,
        mandates,
        decisions,
        settlements,
        audit: Audit {
            verified: true,
            entries: 1_284,
            checkpoint: fingerprint("audit-checkpoint"),
            checked: format!("{as_of}, demo data"),
            tool: "adk-rust audit verify".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::restore_fingerprint;

    #[tokio::test]
    async fn a_restore_must_sign_this_agents_restore_with_a_verified_owner() {
        let d = DemoFeed::new();
        assert_eq!(
            d.set_revoked("kilele", "ghost", true, None).await,
            Err(ConsoleError::NotFound)
        );
        d.set_revoked("kilele", "payables", true, None)
            .await
            .unwrap();
        let feed = d.feed(NaiveDate::default()).await.unwrap();
        let payables = feed
            .companies
            .iter()
            .flat_map(|c| c.agents.iter().map(move |a| (c.id.as_str(), a)))
            .find(|(c, a)| *c == "kilele" && a.id == "payables")
            .unwrap()
            .1;
        assert_eq!(payables.state, "revoked");
        let other = serde_json::json!({"fingerprint": restore_fingerprint("kilele", "chief"), "userVerified": true});
        assert!(matches!(
            d.set_revoked("kilele", "payables", false, Some(other))
                .await,
            Err(ConsoleError::BadProof(_))
        ));
        let unverified = serde_json::json!({"fingerprint": restore_fingerprint("kilele", "payables"), "userVerified": false});
        assert!(matches!(
            d.set_revoked("kilele", "payables", false, Some(unverified))
                .await,
            Err(ConsoleError::BadProof(_))
        ));
        let right = serde_json::json!({"fingerprint": restore_fingerprint("kilele", "payables"), "userVerified": true});
        let record = d
            .set_revoked("kilele", "payables", false, Some(right.clone()))
            .await
            .unwrap();
        assert_eq!(record["revoked"], false);
        assert_eq!(
            d.set_revoked("kilele", "payables", false, Some(right))
                .await,
            Err(ConsoleError::NotFound)
        );
    }
}
