//! What falls due on the live run, so the schedules move money (spec v3's live run).
//!
//! - [`usage_due`]: Agentrix's charge to each agent-run customer for its platform usage since
//!   its last usage invoice: its agents' model calls, as node's gateway metered them in
//!   `control.db` at the providers' prices (one line per model), and the platform fee
//!   (`SAAS_USAGE_FEE_USD`, USD 2.00 a window). Agentrix's Receivables invoices it
//!   (`invoice_usage`) and offers it at the customer's agent door, where the customer's Payables
//!   settles it through Vault inside its owner's mandate.
//! - [`post_payroll`]: a company's payroll bill for the 6-hour pay period that just ended
//!   (`SAAS_PAYROLL_AGENTRIX_USD`, `SAAS_PAYROLL_KILELE_USD`), payable by PayPal Payout to its
//!   payroll account ([`payroll_vendor`]). The payables run pays it inside the founder's
//!   mandate; over the mandate, it waits for the founder's passkey.
//!
//! Each is found again by its reference, which names its window, so a window is invoiced or
//! billed once.

use chrono::{DateTime, Duration, DurationRound, NaiveDateTime, Utc};
use saas_company::desk::{UsageCharge, UsageLine};
use saas_core::books::{BillDraft, Counterparty, DocumentFilter, DocumentStatus, LineDraft};
use saas_core::spend::{SpendWindow, split_cents, vendor_for};
use saas_core::{Books, CompanyId, Currency, Money};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::config::EnvFile;

/// A usage invoice's reference starts with this.
pub const USAGE_PREFIX: &str = "usage:";
/// A payroll bill's vendor reference starts with this.
pub const PAYROLL_PREFIX: &str = "payroll:";
/// How long a pay period is: payroll runs at 00:00, 06:00, 12:00 and 18:00 UTC.
pub const PAY_PERIOD_HOURS: i64 = 6;
/// The window a customer's first usage invoice covers.
const FIRST_WINDOW_HOURS: i64 = 3;
/// The longest window one usage invoice covers (after the node was down, say).
const LONGEST_WINDOW_HOURS: i64 = 24;
/// Times in references: `20261111T120000Z`.
const STAMP: &str = "%Y%m%dT%H%M%SZ";

/// A span of time, from (inclusive) until (exclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// From.
    pub from: DateTime<Utc>,
    /// Until.
    pub until: DateTime<Utc>,
}

impl Window {
    /// The reference for `who`'s window: `<prefix><who>:<from>-<until>`.
    pub fn reference(&self, prefix: &str, who: &str) -> String {
        format!(
            "{prefix}{who}:{}-{}",
            self.from.format(STAMP),
            self.until.format(STAMP)
        )
    }

    /// The window a reference made by [`Window::reference`] names, for `who`.
    pub fn from_reference(reference: &str, prefix: &str, who: &str) -> Option<Self> {
        let span = reference
            .strip_prefix(prefix)?
            .strip_prefix(who)?
            .strip_prefix(':')?;
        let (from, until) = span.split_once('-')?;
        let at = |t: &str| {
            NaiveDateTime::parse_from_str(t, STAMP)
                .ok()
                .map(|n| n.and_utc())
        };
        Some(Self {
            from: at(from)?,
            until: at(until)?,
        })
    }

    /// "11 Nov 09:00 to 11 Nov 12:00 UTC".
    pub fn words(&self) -> String {
        format!(
            "{} to {} UTC",
            self.from.format("%d %b %H:%M"),
            self.until.format("%d %b %H:%M")
        )
    }
}

/// `usd` (as the env writes it, "2.50") in cents; `None` when it isn't a non-negative amount.
fn cents(usd: &str) -> Option<i64> {
    let v = usd.trim().parse::<f64>().ok()?;
    (v.is_finite() && v >= 0.0).then(|| (v * 100.0).round() as i64)
}

/// The platform fee on each usage invoice, in cents: `SAAS_USAGE_FEE_USD`, else USD 2.00.
pub fn usage_fee(env: &EnvFile) -> i64 {
    env.get("SAAS_USAGE_FEE_USD")
        .and_then(|v| cents(&v))
        .unwrap_or(200)
}

/// The env's name for `company`: `AGENTRIX`, `KILELE`.
fn env_name(company: &CompanyId) -> &'static str {
    if *company == CompanyId::kilele() {
        "KILELE"
    } else {
        "AGENTRIX"
    }
}

/// `company`'s wages for one pay period, in cents: `SAAS_PAYROLL_<COMPANY>_USD`, else USD
/// 120.00 for Agentrix and USD 80.00 for Kilele Bakery (inside both founders' mandates).
pub fn payroll_amount(env: &EnvFile, company: &CompanyId) -> i64 {
    env.get(&format!("SAAS_PAYROLL_{}_USD", env_name(company)))
        .and_then(|v| cents(&v))
        .unwrap_or(if *company == CompanyId::kilele() {
            8_000
        } else {
            12_000
        })
}

/// Who `company`'s payroll is paid to: its payroll account, a PayPal account
/// (`SAAS_PAYROLL_<COMPANY>_EMAIL`, else the sandbox personal account
/// `PAYPAL_BUYER_EMAIL_1` for Agentrix and `PAYPAL_BUYER_EMAIL_2` for Kilele Bakery).
pub fn payroll_vendor(env: &EnvFile, company: &CompanyId) -> Counterparty {
    let (name, fallback) = if *company == CompanyId::kilele() {
        ("Kilele Bakery staff payroll", "PAYPAL_BUYER_EMAIL_2")
    } else {
        ("Agentrix staff payroll", "PAYPAL_BUYER_EMAIL_1")
    };
    let email = env
        .get(&format!("SAAS_PAYROLL_{}_EMAIL", env_name(company)))
        .or_else(|| env.get(fallback));
    Counterparty {
        paypal_email: email.clone(),
        ..Counterparty::usd(name, email.as_deref())
    }
}

/// What each agent-run customer of `books` (Agentrix's) owes for its platform usage, from the
/// end of its last usage invoice (or the last 3 hours, for its first) to the start of this hour.
/// A window shorter than an hour waits for a later run.
pub async fn usage_due(
    control: &SqlitePool,
    books: &dyn Books,
    fee_minor: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<Vec<UsageCharge>> {
    let until = now.duration_trunc(Duration::hours(1))?;
    let invoices = books.invoices(&DocumentFilter::default()).await?;
    let mut out = Vec::new();
    for customer in books.customers().await? {
        let Some(company) = customer.counterparty.company.clone() else {
            continue;
        };
        if company == CompanyId::agentrix() {
            continue;
        }
        let last = invoices
            .iter()
            .filter(|i| i.customer == customer.id && i.status != DocumentStatus::Void)
            .filter_map(|i| {
                Window::from_reference(i.reference.as_deref()?, USAGE_PREFIX, company.as_str())
            })
            .map(|w| w.until)
            .max();
        let window = Window {
            from: last
                .unwrap_or(until - Duration::hours(FIRST_WINDOW_HOURS))
                .max(until - Duration::hours(LONGEST_WINDOW_HOURS)),
            until,
        };
        if window.until - window.from < Duration::hours(1) {
            continue;
        }
        let sites = crate::hourly::sites_of(&company);
        let span = SpendWindow {
            from: window.from,
            until: window.until,
        };
        let mut usage = Vec::new();
        for provider in crate::hourly::providers(control, &sites, window.from).await? {
            usage.extend(crate::hourly::usage(control, &sites, &provider, &span).await?);
        }
        let split = split_cents(&usage.iter().map(|u| u.cost_micro_usd).collect::<Vec<_>>());
        let mut lines: Vec<UsageLine> = usage
            .iter()
            .zip(&split)
            .filter(|(_, c)| **c > 0)
            .map(|(u, c)| UsageLine {
                description: format!(
                    "Model calls via {}, {}",
                    vendor_for(&u.provider, None).name,
                    u.describe()
                ),
                amount_minor: *c,
            })
            .collect();
        if fee_minor > 0 {
            lines.push(UsageLine {
                description: format!("Agentrix platform fee, {}", window.words()),
                amount_minor: fee_minor,
            });
        }
        if lines.is_empty() {
            continue;
        }
        out.push(UsageCharge {
            customer_id: customer.id.to_string(),
            customer: customer.counterparty.name.clone(),
            reference: window.reference(USAGE_PREFIX, company.as_str()),
            from: window.from,
            until: window.until,
            lines,
        });
    }
    Ok(out)
}

/// A payroll bill, as [`post_payroll`] left it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payroll {
    /// The pay period it covers.
    pub period: Window,
    /// The bill in the books, posted.
    pub bill_id: String,
    /// Its number.
    pub number: Option<String>,
    /// What it comes to.
    pub total: String,
    /// The payroll vendor.
    pub vendor: String,
    /// The PayPal account it is paid to, when the books hold one.
    pub payee: Option<String>,
    /// Whether it was already in the books before this call.
    pub already: bool,
}

/// Posts `company`'s payroll bill for the pay period that ended last (00:00, 06:00, 12:00 or
/// 18:00 UTC), or finds it already posted. `Ok(None)` when no wages are set.
pub async fn post_payroll(
    books: &dyn Books,
    env: &EnvFile,
    company: &CompanyId,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<Payroll>> {
    let amount = payroll_amount(env, company);
    if amount <= 0 {
        return Ok(None);
    }
    let until = now.duration_trunc(Duration::hours(PAY_PERIOD_HOURS))?;
    let period = Window {
        from: until - Duration::hours(PAY_PERIOD_HOURS),
        until,
    };
    let reference = period.reference(PAYROLL_PREFIX, company.as_str());
    let found = books
        .bills(&DocumentFilter {
            reference: Some(reference.clone()),
            ..Default::default()
        })
        .await?
        .into_iter()
        .find(|b| b.status != DocumentStatus::Void);
    let who = payroll_vendor(env, company);
    let vendor = books.ensure_vendor(&who).await?;
    let (bill, already) = match found {
        Some(b) if matches!(b.status, DocumentStatus::Draft | DocumentStatus::Submitted) => {
            (books.post_bill(&b.id).await?, true)
        }
        Some(b) => (b, true),
        None => {
            let issue_date = until.date_naive();
            let draft = BillDraft {
                vendor: vendor.id.clone(),
                vendor_reference: Some(reference),
                issue_date,
                due_date: issue_date,
                currency: Currency::USD,
                lines: vec![LineDraft {
                    product: None,
                    description: format!("Staff wages, pay period {}", period.words()),
                    quantity: 1,
                    unit_price: Money::usd(amount),
                }],
                memo: Some(format!(
                    "Payroll for the pay period {}, paid by PayPal Payout to the payroll account \
                     on file.",
                    period.words()
                )),
            };
            let drafted = books.draft_bill(&draft).await?;
            (books.post_bill(&drafted.id).await?, false)
        }
    };
    Ok(Some(Payroll {
        period,
        bill_id: bill.id.to_string(),
        number: bill.number.clone(),
        total: bill.total.to_string(),
        vendor: vendor.counterparty.name.clone(),
        payee: vendor.counterparty.paypal_email.clone(),
        already,
    }))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use saas_core::books::{BillId, Customer};
    use saas_core::fake::FakeBooks;

    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 11, h, m, 0).unwrap()
    }

    #[test]
    fn a_reference_names_its_window() {
        let w = Window {
            from: at(9, 0),
            until: at(12, 0),
        };
        let r = w.reference(USAGE_PREFIX, "kilele-bakery");
        assert_eq!(r, "usage:kilele-bakery:20261111T090000Z-20261111T120000Z");
        assert_eq!(
            Window::from_reference(&r, USAGE_PREFIX, "kilele-bakery"),
            Some(w)
        );
        assert_eq!(Window::from_reference(&r, USAGE_PREFIX, "kilele"), None);
        assert_eq!(Window::from_reference("INV-1", USAGE_PREFIX, "x"), None);
    }

    #[test]
    fn payroll_and_fees_come_from_the_env_with_defaults_inside_the_mandates() {
        let env = EnvFile::parse(
            "SAAS_PAYROLL_KILELE_USD=95.50\nSAAS_USAGE_FEE_USD=1.25\nPAYPAL_BUYER_EMAIL_1=staff@example.com\n",
        );
        assert_eq!(payroll_amount(&env, &CompanyId::kilele()), 9_550);
        assert_eq!(payroll_amount(&env, &CompanyId::agentrix()), 12_000);
        assert_eq!(usage_fee(&env), 125);
        assert_eq!(usage_fee(&EnvFile::default()), 200);
        let who = payroll_vendor(&env, &CompanyId::agentrix());
        assert_eq!(who.name, "Agentrix staff payroll");
        assert_eq!(who.paypal_email.as_deref(), Some("staff@example.com"));
        assert_eq!(
            payroll_vendor(&env, &CompanyId::kilele()).paypal_email,
            None
        );
    }

    #[tokio::test]
    async fn one_payroll_bill_per_period() {
        let books = FakeBooks::new(CompanyId::kilele());
        let env = EnvFile::parse("PAYPAL_BUYER_EMAIL_2=bakers@example.com\n");
        let kilele = CompanyId::kilele();
        let first = post_payroll(&books, &env, &kilele, at(12, 35))
            .await
            .unwrap()
            .unwrap();
        assert!(!first.already);
        assert_eq!(
            first.period,
            Window {
                from: at(6, 0),
                until: at(12, 0)
            }
        );
        assert_eq!(first.payee.as_deref(), Some("bakers@example.com"));
        let again = post_payroll(&books, &env, &kilele, at(13, 10))
            .await
            .unwrap()
            .unwrap();
        assert!(again.already);
        assert_eq!(again.bill_id, first.bill_id);
        let next = post_payroll(&books, &env, &kilele, at(18, 35))
            .await
            .unwrap()
            .unwrap();
        assert_ne!(next.bill_id, first.bill_id);
        let bill = books
            .bill(&BillId::from(first.bill_id.as_str()))
            .await
            .unwrap();
        assert_eq!(bill.total, Money::usd(8_000));
        assert!(bill.status.is_open());
    }

    async fn control() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE usage (site_id TEXT, purpose TEXT, provider TEXT, model TEXT, tokens_in INTEGER, tokens_out INTEGER, cost_micro_usd INTEGER, outcome TEXT, at TEXT);",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn call(pool: &SqlitePool, site: &str, model: &str, cost: i64, when: DateTime<Utc>) {
        sqlx::query("INSERT INTO usage VALUES (?, 'agent', 'anthropic', ?, 1000, 100, ?, 'ok', ?)")
            .bind(site)
            .bind(model)
            .bind(cost)
            .bind(when.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_customer_is_invoiced_its_metered_calls_and_the_fee_once_per_window() {
        let pool = control().await;
        call(&pool, "saas-kilele", "claude-sonnet-5-5", 16_200, at(9, 15)).await;
        call(&pool, "saas-kilele", "claude-haiku-5-5", 1_400, at(10, 15)).await;
        call(
            &pool,
            "kilele-bakery-sample",
            "qwen3.8-27b",
            7_000,
            at(11, 5),
        )
        .await;
        // Agentrix's own call, and one in the hour not yet over: neither is Kilele's to pay now.
        call(
            &pool,
            "saas-agentrix",
            "claude-sonnet-5-5",
            900_000,
            at(10, 0),
        )
        .await;
        call(
            &pool,
            "saas-kilele",
            "claude-sonnet-5-5",
            50_000,
            at(12, 10),
        )
        .await;

        let books = FakeBooks::new(CompanyId::agentrix());
        let mut who = Counterparty::usd("Kilele Bakery", Some("owner@kilele.example"));
        who.company = Some(CompanyId::kilele());
        let kilele: Customer = books.ensure_customer(&who, None).await.unwrap();

        let due = usage_due(&pool, &books, 200, at(12, 20)).await.unwrap();
        assert_eq!(due.len(), 1, "{due:?}");
        let charge = &due[0];
        assert_eq!(charge.customer_id, kilele.id.to_string());
        assert_eq!((charge.from, charge.until), (at(9, 0), at(12, 0)));
        // 0.0162 + 0.0014 + 0.0070 = USD 0.0246: 2 cents of calls, and the fee.
        assert_eq!(charge.total_minor(), 2 + 200, "{charge:?}");
        assert!(
            charge
                .lines
                .iter()
                .any(|l| l.description.contains("claude-sonnet-5-5"))
        );

        // Once invoiced, the next charge starts where it ended.
        let draft = saas_core::books::InvoiceDraft {
            customer: kilele.id.clone(),
            issue_date: at(12, 20).date_naive(),
            due_date: at(12, 20).date_naive(),
            currency: Currency::USD,
            lines: vec![LineDraft {
                product: None,
                description: "usage".into(),
                quantity: 1,
                unit_price: Money::usd(charge.total_minor()),
            }],
            reference: Some(charge.reference.clone()),
            memo: None,
        };
        let inv = books.draft_invoice(&draft).await.unwrap();
        books.submit_invoice(&inv.id).await.unwrap();
        books.post_invoice(&inv.id).await.unwrap();
        assert!(
            usage_due(&pool, &books, 200, at(12, 50))
                .await
                .unwrap()
                .is_empty()
        );
        let later = usage_due(&pool, &books, 200, at(15, 20)).await.unwrap();
        assert_eq!((later[0].from, later[0].until), (at(12, 0), at(15, 0)));
        assert_eq!(later[0].total_minor(), 5 + 200);
    }
}
