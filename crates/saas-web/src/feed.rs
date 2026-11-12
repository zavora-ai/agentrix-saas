//! The board's and consoles' JSON feed (contract: the board's data feed, owned by slice D).
//!
//! `GET /feed.json` on the money host returns both companies; on the Kilele host, Kilele's rows
//! only. Every amount is an integer of the currency's minor unit (`{"currency": "USD", "minor":
//! 1900}` elsewhere; here a flat `…Minor` field beside one `currency` per row), so nothing is
//! ever rounded on the way. The board divides by 100 for display.
//!
//! Version 1. Fields are only ever added. A [`FeedSource`] fills it: [`crate::demo::DemoFeed`]
//! until slices A and B merge, then one built on `saas_core::Books`.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The feed's version; bumped only if a field changes meaning.
pub const VERSION: u32 = 1;

/// Agentrix's company id (saas-core's `CompanyId::agentrix()`).
pub const AGENTRIX: &str = "agentrix";
/// Kilele Bakery's company id (saas-core's `CompanyId::kilele()`).
pub const KILELE: &str = "kilele";

/// Everything the board and both consoles show.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    /// [`VERSION`].
    pub version: u32,
    /// The day every age and due date is counted from.
    pub as_of: NaiveDate,
    /// Where the numbers came from: `"demo"` (the built-in story), `"books"` (Zavora ERA).
    pub source: String,
    /// The two companies.
    pub companies: Vec<Company>,
    /// Open and recently paid customer invoices.
    pub receivables: Vec<Receivable>,
    /// Receivables by customer and age bucket.
    pub ageing: Vec<AgeingRow>,
    /// Bills to pay.
    pub payables: Vec<Payable>,
    /// Instalment plans agreed between the two companies' agents.
    pub debt_plans: Vec<DebtPlan>,
    /// Cash per rail and account.
    pub cash: Vec<CashBalance>,
    /// Monthly recurring revenue, oldest month first.
    pub mrr: Vec<MrrPoint>,
    /// What each agent spent, per day.
    pub agent_spend: Vec<AgentSpend>,
    /// The mandates owners signed for their agents.
    pub mandates: Vec<Mandate>,
    /// Decisions waiting on an owner, and recent ones.
    pub decisions: Vec<Decision>,
    /// Agent-to-agent settlements, newest first.
    pub settlements: Vec<Settlement>,
    /// The signed audit log's last check.
    pub audit: Audit,
}

impl Feed {
    /// The same feed with only `company`'s rows (what Kilele's console and feed show).
    pub fn for_company(&self, company: &str) -> Feed {
        let mut f = self.clone();
        f.companies.retain(|c| c.id == company);
        f.receivables.retain(|r| r.company == company);
        f.ageing.retain(|r| r.company == company);
        f.payables.retain(|r| r.company == company);
        f.debt_plans
            .retain(|p| p.creditor == company || p.debtor == company);
        f.cash.retain(|r| r.company == company);
        if company != AGENTRIX {
            f.mrr.clear();
        }
        f.agent_spend.retain(|r| r.company == company);
        f.mandates.retain(|r| r.company == company);
        f.decisions.retain(|r| r.company == company);
        f.settlements
            .retain(|s| s.payer == company || s.payee == company);
        f
    }

    /// One company by id.
    pub fn company(&self, id: &str) -> Option<&Company> {
        self.companies.iter().find(|c| c.id == id)
    }
}

/// A company the node runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    /// `agentrix` or `kilele`.
    pub id: String,
    /// "Agentrix", "Kilele Bakery".
    pub name: String,
    /// Its public host.
    pub host: String,
    /// Its owner's name, as cards say it.
    pub owner: String,
    /// Its agents.
    pub agents: Vec<Agent>,
    /// True while the owner has frozen every agent that moves money.
    pub frozen: bool,
}

/// One agent on a company's staff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    /// `receivables`.
    pub id: String,
    /// "Receivables".
    pub name: String,
    /// Its job in one line.
    pub job: String,
    /// One of agentrix-marks' states: `idle`, `working`, `waiting`, `resting`, `frozen` …
    pub state: String,
    /// What it is doing, in words ("Chasing INV-1042").
    pub doing: String,
    /// Its signing key's id in the agent door's key directory, when it signs requests.
    pub key_id: Option<String>,
}

/// One customer invoice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Receivable {
    /// The company owed.
    pub company: String,
    /// ERA's invoice id.
    pub invoice_id: String,
    /// "INV-1042".
    pub number: String,
    /// The customer.
    pub customer: String,
    /// What it is for.
    pub description: String,
    /// Issue date.
    pub issued: NaiveDate,
    /// Due date.
    pub due: NaiveDate,
    /// ISO 4217.
    pub currency: String,
    /// Invoice total.
    pub amount_minor: i64,
    /// Still owed.
    pub open_minor: i64,
    /// Days past due at `as_of` (0 when not yet due).
    pub days_overdue: i64,
    /// `current`, `1-30`, `31-60`, `61-90`, `90+`, or `paid`.
    pub bucket: String,
    /// `open`, `partial`, `paid`, `on_plan` (an instalment plan covers it).
    pub status: String,
    /// The PayPal id that settled it, when one did.
    pub paypal_id: Option<String>,
}

/// Receivables owed by one customer, by age.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgeingRow {
    /// The company owed.
    pub company: String,
    /// The customer.
    pub customer: String,
    /// ISO 4217.
    pub currency: String,
    /// Not yet due.
    pub current_minor: i64,
    /// 1 to 30 days late.
    pub d1_30_minor: i64,
    /// 31 to 60 days late.
    pub d31_60_minor: i64,
    /// 61 to 90 days late.
    pub d61_90_minor: i64,
    /// Over 90 days late.
    pub over90_minor: i64,
    /// All of it.
    pub total_minor: i64,
    /// The customer's credit limit in ERA, when one is set.
    pub credit_limit_minor: Option<i64>,
}

/// One bill to pay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Payable {
    /// The company that owes.
    pub company: String,
    /// ERA's bill id.
    pub bill_id: String,
    /// The vendor's reference.
    pub number: String,
    /// Who is paid.
    pub vendor: String,
    /// What it is for.
    pub description: String,
    /// Due date.
    pub due: NaiveDate,
    /// ISO 4217.
    pub currency: String,
    /// Bill total.
    pub amount_minor: i64,
    /// Still to pay.
    pub open_minor: i64,
    /// Days until due (negative when late).
    pub days_to_due: i64,
    /// How it is paid: `paypal_payouts`, `paypal_orders`, `mpesa`.
    pub rail: String,
    /// `within` the mandate (the agent pays it), `over` (it waits for the owner's passkey).
    pub mandate: String,
    /// `open`, `scheduled`, `held`, `paid`, `refused`.
    pub status: String,
}

/// An instalment plan between two companies.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DebtPlan {
    /// The plan's id.
    pub id: String,
    /// The company owed.
    pub creditor: String,
    /// The company that owes.
    pub debtor: String,
    /// The invoice the plan covers.
    pub invoice_number: String,
    /// ISO 4217.
    pub currency: String,
    /// What the plan covers.
    pub principal_minor: i64,
    /// Paid so far.
    pub paid_minor: i64,
    /// `active`, `completed`, `late`, `proposed`.
    pub status: String,
    /// When both agents signed it.
    pub agreed: NaiveDate,
    /// The mandates it was agreed inside, one per side.
    pub mandate_ids: Vec<String>,
    /// The instalments, in order.
    pub instalments: Vec<Instalment>,
}

/// One instalment of a plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Instalment {
    /// 1-based.
    pub n: u32,
    /// Due date.
    pub due: NaiveDate,
    /// Amount.
    pub amount_minor: i64,
    /// `paid`, `due` (today), `scheduled`, `late`.
    pub status: String,
    /// The PayPal id that paid it.
    pub paypal_id: Option<String>,
}

/// Cash on one rail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CashBalance {
    /// The company.
    pub company: String,
    /// "PayPal" or "M-Pesa".
    pub rail: String,
    /// The account's name in ERA ("PayPal clearing").
    pub account: String,
    /// ISO 4217.
    pub currency: String,
    /// The balance.
    pub balance_minor: i64,
    /// `sandbox` or `live`.
    pub mode: String,
}

/// MRR for one month.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MrrPoint {
    /// `2026-11`.
    pub month: String,
    /// ISO 4217.
    pub currency: String,
    /// MRR at the month's end.
    pub mrr_minor: i64,
    /// New MRR in the month.
    pub new_minor: i64,
    /// MRR lost in the month.
    pub churn_minor: i64,
    /// Paying customers at the month's end.
    pub customers: u32,
}

/// One agent's spend on one day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSpend {
    /// The company.
    pub company: String,
    /// The agent's id.
    pub agent: String,
    /// The day.
    pub day: NaiveDate,
    /// ISO 4217.
    pub currency: String,
    /// Model tokens, at cost.
    pub model_minor: i64,
    /// Money the agent moved out (payouts, settlements).
    pub paid_out_minor: i64,
    /// Tool calls made.
    pub tool_calls: u32,
}

/// A mandate an owner signed for an agent (AP2-style intent mandate).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Mandate {
    /// The mandate's id.
    pub id: String,
    /// The company.
    pub company: String,
    /// Who signed it.
    pub owner: String,
    /// The agent it authorises.
    pub agent: String,
    /// What it covers, in words.
    pub scope: String,
    /// ISO 4217.
    pub currency: String,
    /// The most one payment may be.
    pub per_payment_minor: i64,
    /// The most per day.
    pub daily_minor: i64,
    /// Spent today inside it.
    pub used_today_minor: i64,
    /// Payees it covers, in words.
    pub payees: Vec<String>,
    /// How it was signed: `passkey`.
    pub signed_with: String,
    /// When it was signed.
    pub signed: NaiveDate,
    /// When it lapses.
    pub expires: NaiveDate,
    /// `active`, `frozen`, `expired`.
    pub status: String,
}

/// A decision an owner is asked for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    /// The decision's id.
    pub id: String,
    /// The company.
    pub company: String,
    /// The agent asking.
    pub agent: String,
    /// What is asked, in one line.
    pub title: String,
    /// The facts, in one line.
    pub body: String,
    /// What happens on approval, line by line.
    pub effects: Vec<String>,
    /// Why it was not done without asking.
    pub why: String,
    /// ISO 4217.
    pub currency: String,
    /// The amount at stake, when there is one.
    pub amount_minor: Option<i64>,
    /// `open`, `approved`, `declined`.
    pub state: String,
    /// The fingerprint the passkey signs (the exact call).
    pub fingerprint: String,
    /// The proven agent it was raised for (a quote, a checkout link, a held request): its id,
    /// who vouched for it, and how it proved itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<RequestedBy>,
}

/// A proven agent, as the door verified it (agentrix-samples' `ProvenAgent`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestedBy {
    /// Its verified id: a DID, `wba:{origin}#{key}`, or a cloud's principal.
    pub id: String,
    /// Who vouched: "Google Cloud", "Microsoft Entra", or the Agentrix site's origin.
    pub vouched_by: String,
    /// How: `web-bot-auth` or `token`.
    pub method: String,
}

/// One agent-to-agent settlement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settlement {
    /// When it settled.
    pub at: NaiveDate,
    /// Who paid.
    pub payer: String,
    /// Who was paid.
    pub payee: String,
    /// The invoice settled.
    pub invoice_number: String,
    /// ISO 4217.
    pub currency: String,
    /// Amount.
    pub amount_minor: i64,
    /// The one PayPal id both ledgers posted from.
    pub paypal_id: String,
    /// `settled`, `pending`, `refused`.
    pub status: String,
    /// How it travelled: "A2A · signed request · AP2-style cart mandate".
    pub route: String,
}

/// The signed audit log's last offline check (`adk-rust audit verify`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Audit {
    /// True when the last check passed.
    pub verified: bool,
    /// Entries in the log.
    pub entries: u64,
    /// The signed checkpoint's root hash.
    pub checkpoint: String,
    /// When it was checked, in words.
    pub checked: String,
    /// The command that checked it.
    pub tool: String,
}

/// Fills the feed. Slices A and B provide the real one; [`crate::demo::DemoFeed`] stands in.
#[async_trait]
pub trait FeedSource: Send + Sync {
    /// The feed as of `as_of`.
    async fn feed(&self, as_of: NaiveDate) -> anyhow::Result<Feed>;
}

/// The age bucket of an amount due on `due`, counted at `as_of`.
pub fn bucket(due: NaiveDate, as_of: NaiveDate) -> (&'static str, i64) {
    let late = (as_of - due).num_days();
    let name = match late {
        i64::MIN..=0 => "current",
        1..=30 => "1-30",
        31..=60 => "31-60",
        61..=90 => "61-90",
        _ => "90+",
    };
    (name, late.max(0))
}

/// Ageing rows from receivables: one per company and customer, open amounts only.
pub fn ageing(receivables: &[Receivable], limits: &[(&str, &str, i64)]) -> Vec<AgeingRow> {
    let mut rows: Vec<AgeingRow> = Vec::new();
    for r in receivables.iter().filter(|r| r.open_minor > 0) {
        let at = match rows
            .iter()
            .position(|a| a.company == r.company && a.customer == r.customer)
        {
            Some(i) => i,
            None => {
                rows.push(AgeingRow {
                    company: r.company.clone(),
                    customer: r.customer.clone(),
                    currency: r.currency.clone(),
                    current_minor: 0,
                    d1_30_minor: 0,
                    d31_60_minor: 0,
                    d61_90_minor: 0,
                    over90_minor: 0,
                    total_minor: 0,
                    credit_limit_minor: limits
                        .iter()
                        .find(|(c, who, _)| *c == r.company && *who == r.customer)
                        .map(|(_, _, l)| *l),
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[at];
        match r.bucket.as_str() {
            "1-30" => row.d1_30_minor += r.open_minor,
            "31-60" => row.d31_60_minor += r.open_minor,
            "61-90" => row.d61_90_minor += r.open_minor,
            "90+" => row.over90_minor += r.open_minor,
            _ => row.current_minor += r.open_minor,
        }
        row.total_minor += r.open_minor;
    }
    rows
}

/// `$1,188.00` for 118800 USD minor units; other currencies by code (`KES 1,500.00`).
pub fn money(minor: i64, currency: &str) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    let (major, cents) = (abs / 100, abs % 100);
    let digits = major.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    match currency {
        "USD" => format!("{sign}${grouped}.{cents:02}"),
        other => format!("{sign}{other} {grouped}.{cents:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn buckets_count_whole_days_late() {
        let as_of = d("2026-11-10");
        assert_eq!(bucket(d("2026-11-10"), as_of), ("current", 0));
        assert_eq!(bucket(d("2026-12-01"), as_of), ("current", 0));
        assert_eq!(bucket(d("2026-11-09"), as_of), ("1-30", 1));
        assert_eq!(bucket(d("2026-10-11"), as_of), ("1-30", 30));
        assert_eq!(bucket(d("2026-10-10"), as_of), ("31-60", 31));
        assert_eq!(bucket(d("2026-08-12"), as_of), ("61-90", 90));
        assert_eq!(bucket(d("2026-08-11"), as_of), ("90+", 91));
    }

    #[test]
    fn money_groups_thousands_and_keeps_cents() {
        assert_eq!(money(118_800, "USD"), "$1,188.00");
        assert_eq!(money(5, "USD"), "$0.05");
        assert_eq!(money(-123_456_789, "USD"), "-$1,234,567.89");
        assert_eq!(money(150_000, "KES"), "KES 1,500.00");
    }
}
