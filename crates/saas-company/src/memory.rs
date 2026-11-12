//! What the company remembers about each counterparty (adk-memory): how they pay and what they
//! promised. It feeds collections (when and how firmly to remind) and debt plans (how many
//! instalments, how much up front).
//!
//! One adk-memory scope per counterparty: the app is the company's (`agentrix-saas/<company>`),
//! the user is `counterparty:<id>`, so one customer's history never shows up in another's
//! recall. Each entry is a plain sentence a model can read, ending in a tag the company parses
//! back (`[paid invoice=INV-7 days_late=12]`); adk-memory's entries carry no metadata, so the tag
//! is the structure. Anything a person wrote is redacted before it is remembered.

use std::sync::Arc;

use adk_core::Content;
use adk_memory::{InMemoryMemoryService, MemoryEntry, MemoryService, SearchRequest};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::identity::app_name;
use crate::screen::redact;

/// One thing worth remembering about a counterparty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Behaviour {
    /// They paid an invoice, this many days after it was due (0 or less: on time).
    Paid {
        /// The invoice.
        invoice: String,
        /// Days after the due date.
        days_late: i64,
    },
    /// They promised to pay an amount by a date.
    Promised {
        /// The invoice.
        invoice: String,
        /// In minor units.
        amount_minor: i64,
        /// By when.
        by: NaiveDate,
    },
    /// A promise came due and was kept.
    KeptPromise {
        /// The invoice.
        invoice: String,
    },
    /// A promise came due and wasn't kept.
    BrokePromise {
        /// The invoice.
        invoice: String,
    },
    /// They disputed an invoice.
    Disputed {
        /// The invoice.
        invoice: String,
    },
}

impl Behaviour {
    fn sentence(&self) -> String {
        match self {
            Self::Paid { invoice, days_late } if *days_late <= 0 => {
                format!("Paid {invoice} on time. [paid invoice={invoice} days_late={days_late}]")
            }
            Self::Paid { invoice, days_late } => format!(
                "Paid {invoice} {days_late} days late. [paid invoice={invoice} days_late={days_late}]"
            ),
            Self::Promised {
                invoice,
                amount_minor,
                by,
            } => format!(
                "Promised to pay {} on {invoice} by {by}. [promised invoice={invoice} amount_minor={amount_minor} by={by}]",
                amount_minor / 100
            ),
            Self::KeptPromise { invoice } => {
                format!("Kept their promise on {invoice}. [kept invoice={invoice}]")
            }
            Self::BrokePromise { invoice } => {
                format!("Broke their promise on {invoice}. [broke invoice={invoice}]")
            }
            Self::Disputed { invoice } => {
                format!("Disputed {invoice}. [disputed invoice={invoice}]")
            }
        }
    }

    fn parse(text: &str) -> Option<Self> {
        let start = text.rfind('[')?;
        let tag = text[start + 1..].trim_end().strip_suffix(']')?;
        let kind = tag.split_whitespace().next()?;
        let field = |name: &str| -> Option<String> {
            tag.split_whitespace()
                .find_map(|w| w.strip_prefix(&format!("{name}=")).map(str::to_owned))
        };
        let invoice = field("invoice")?;
        Some(match kind {
            "paid" => Self::Paid {
                invoice,
                days_late: field("days_late")?.parse().ok()?,
            },
            "promised" => Self::Promised {
                invoice,
                amount_minor: field("amount_minor")?.parse().ok()?,
                by: field("by")?.parse().ok()?,
            },
            "kept" => Self::KeptPromise { invoice },
            "broke" => Self::BrokePromise { invoice },
            "disputed" => Self::Disputed { invoice },
            _ => return None,
        })
    }
}

/// A counterparty's payment behaviour, from memory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaymentProfile {
    /// Invoices paid.
    pub payments: u32,
    /// Of those, paid late.
    pub late: u32,
    /// Average days late over late payments.
    pub average_days_late: f64,
    /// Promises kept.
    pub promises_kept: u32,
    /// Promises broken.
    pub promises_broken: u32,
    /// Disputes.
    pub disputes: u32,
}

/// How reliable a counterparty is, from their profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// No history yet.
    New,
    /// Pays on time and keeps promises.
    Reliable,
    /// Sometimes late; promises mostly kept.
    Slow,
    /// Often late, or breaks promises.
    Risky,
}

impl PaymentProfile {
    /// Their standing.
    pub fn standing(&self) -> Standing {
        if self.payments == 0 && self.promises_kept == 0 && self.promises_broken == 0 {
            return Standing::New;
        }
        if self.promises_broken > self.promises_kept
            || (self.payments > 0 && self.late * 2 > self.payments && self.average_days_late > 20.0)
        {
            return Standing::Risky;
        }
        if self.late == 0 && self.promises_broken == 0 {
            return Standing::Reliable;
        }
        Standing::Slow
    }
}

/// An instalment of a proposed plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instalment {
    /// When it is due.
    pub due_date: NaiveDate,
    /// How much, in minor units.
    pub amount_minor: i64,
}

/// The plan shape memory suggests for a balance: how many instalments, how far apart, how much
/// up front. Inside the mandate's limits, never past them.
pub fn suggest_plan(
    standing: Standing,
    balance_minor: i64,
    today: NaiveDate,
    max_instalments: u32,
    max_days: u32,
) -> Vec<Instalment> {
    // (instalments, days apart, share up front in percent)
    let (n, gap, upfront) = match standing {
        Standing::Reliable => (2, 30, 50),
        Standing::New => (3, 30, 40),
        Standing::Slow => (3, 21, 50),
        Standing::Risky => (4, 14, 60),
    };
    let n = n.clamp(1, max_instalments.max(1));
    let gap = if n > 1 {
        gap.min(max_days / (n - 1)).max(1)
    } else {
        0
    };
    let first = if n == 1 {
        balance_minor
    } else {
        balance_minor * upfront / 100
    };
    let rest = balance_minor - first;
    let each = if n > 1 { rest / i64::from(n - 1) } else { 0 };
    let mut plan = Vec::new();
    let mut paid = 0;
    for i in 0..n {
        let amount = if i == 0 {
            first
        } else if i == n - 1 {
            balance_minor - paid
        } else {
            each
        };
        paid += amount;
        plan.push(Instalment {
            due_date: today + chrono::Duration::days(i64::from(gap * i)),
            amount_minor: amount,
        });
    }
    plan
}

/// The company's memory of its counterparties.
#[derive(Clone)]
pub struct CounterpartyMemory {
    company: String,
    service: Arc<dyn MemoryService>,
}

impl std::fmt::Debug for CounterpartyMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CounterpartyMemory")
            .field("company", &self.company)
            .finish_non_exhaustive()
    }
}

impl CounterpartyMemory {
    /// Over `service`.
    pub fn new(company: &str, service: Arc<dyn MemoryService>) -> Self {
        Self {
            company: company.to_owned(),
            service,
        }
    }

    /// In memory: tests and judge mode.
    pub fn in_memory(company: &str) -> Self {
        Self::new(company, Arc::new(InMemoryMemoryService::new()))
    }

    fn scope(&self, counterparty: &str) -> (String, String) {
        (
            app_name(&self.company).as_str().to_owned(),
            format!("counterparty:{counterparty}"),
        )
    }

    /// Remembers `behaviour` of `counterparty` at `at`.
    pub async fn remember(
        &self,
        counterparty: &str,
        behaviour: &Behaviour,
        at: DateTime<Utc>,
    ) -> adk_core::Result<()> {
        let (app, user) = self.scope(counterparty);
        self.service
            .add_entry(
                &app,
                &user,
                MemoryEntry {
                    content: Content::new("user").with_text(redact(&behaviour.sentence())),
                    author: format!("{}/receivables", self.company),
                    timestamp: at,
                },
            )
            .await
    }

    /// Remembers a free note about `counterparty` (redacted).
    pub async fn note(
        &self,
        counterparty: &str,
        text: &str,
        at: DateTime<Utc>,
    ) -> adk_core::Result<()> {
        let (app, user) = self.scope(counterparty);
        self.service
            .add_entry(
                &app,
                &user,
                MemoryEntry {
                    content: Content::new("user").with_text(redact(text)),
                    author: format!("{}/receivables", self.company),
                    timestamp: at,
                },
            )
            .await
    }

    /// The newest `limit` sentences about `counterparty`.
    pub async fn recall(&self, counterparty: &str, limit: usize) -> adk_core::Result<Vec<String>> {
        let (app, user) = self.scope(counterparty);
        let entries = self.service.list_recent(&app, &user, limit).await?;
        Ok(entries
            .iter()
            .map(|e| {
                e.content
                    .parts
                    .iter()
                    .filter_map(|p| p.text())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect())
    }

    /// The sentences about `counterparty` that match `query`.
    pub async fn search(&self, counterparty: &str, query: &str) -> adk_core::Result<Vec<String>> {
        let (app, user) = self.scope(counterparty);
        let found = self
            .service
            .search(SearchRequest {
                query: query.to_owned(),
                user_id: user,
                app_name: app,
                limit: Some(20),
                min_score: None,
                project_id: None,
            })
            .await?;
        Ok(found
            .memories
            .iter()
            .map(|e| {
                e.content
                    .parts
                    .iter()
                    .filter_map(|p| p.text())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect())
    }

    /// The counterparty's payment profile, from everything remembered.
    pub async fn profile(&self, counterparty: &str) -> adk_core::Result<PaymentProfile> {
        let mut p = PaymentProfile::default();
        let mut late_days = 0i64;
        for text in self.recall(counterparty, 500).await? {
            match Behaviour::parse(&text) {
                Some(Behaviour::Paid { days_late, .. }) => {
                    p.payments += 1;
                    if days_late > 0 {
                        p.late += 1;
                        late_days += days_late;
                    }
                }
                Some(Behaviour::KeptPromise { .. }) => p.promises_kept += 1,
                Some(Behaviour::BrokePromise { .. }) => p.promises_broken += 1,
                Some(Behaviour::Disputed { .. }) => p.disputes += 1,
                Some(Behaviour::Promised { .. }) | None => {}
            }
        }
        if p.late > 0 {
            p.average_days_late = late_days as f64 / f64::from(p.late);
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn memory_is_per_counterparty_and_parses_back() {
        let m = CounterpartyMemory::in_memory("agentrix");
        let now = Utc::now();
        m.remember(
            "kilele",
            &Behaviour::Paid {
                invoice: "INV-1".into(),
                days_late: 0,
            },
            now,
        )
        .await
        .unwrap();
        m.remember(
            "acme",
            &Behaviour::BrokePromise {
                invoice: "INV-9".into(),
            },
            now,
        )
        .await
        .unwrap();
        let kilele = m.profile("kilele").await.unwrap();
        assert_eq!(kilele.payments, 1);
        assert_eq!(kilele.standing(), Standing::Reliable);
        let acme = m.profile("acme").await.unwrap();
        assert_eq!(acme.payments, 0);
        assert_eq!(acme.standing(), Standing::Risky);
        assert_eq!(m.profile("nobody").await.unwrap().standing(), Standing::New);
    }

    #[test]
    fn plans_add_up_and_stay_inside_the_mandate() {
        let today = day("2026-11-10");
        for standing in [
            Standing::New,
            Standing::Reliable,
            Standing::Slow,
            Standing::Risky,
        ] {
            let plan = suggest_plan(standing, 120_000, today, 3, 60);
            assert!(plan.len() <= 3);
            assert_eq!(plan.iter().map(|i| i.amount_minor).sum::<i64>(), 120_000);
            let last = plan.last().unwrap().due_date;
            assert!((last - today).num_days() <= 60, "{standing:?} {plan:?}");
        }
        let reliable = suggest_plan(Standing::Reliable, 120_000, today, 4, 120);
        let risky = suggest_plan(Standing::Risky, 120_000, today, 4, 120);
        assert_ne!(reliable, risky);
        assert!(risky[0].amount_minor > reliable[0].amount_minor);
    }
}
