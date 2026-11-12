//! The owner's AI budget on the console (`specs/platform/facts.md`, "AI budget"): a monthly
//! cap, set and shown in the site's currency. Reading the cap, the currency and the month's
//! spend lives in [`agentrix_store::budget`], so the Chief WebAgent, the morning brief, the
//! channels and the owner app say the same figure the console does; this module draws the
//! console's meter from it.

pub use agentrix_store::budget::{AiBudget, CapFrom, Currency, MonthSpend, read, this_month};

use agentrix_console::view as cv;
use agentrix_store::metrics::SpendReport;
use agentrix_store::words::plural;

/// The meter every console page shows: this month's spend against the monthly cap, in the
/// site's currency, from `month` (the metric catalogue's report for this month).
pub fn meter(
    budget: &AiBudget,
    month: &SpendReport,
    purpose_words: impl Fn(&str) -> String,
) -> cv::Meter {
    let line = |l: &agentrix_store::metrics::SpendLine, label: String| cv::MeterLine {
        label,
        amount: budget.show(l.cost),
    };
    let mut calls_words = format!("{} this month", plural(month.calls, "AI call", "AI calls"));
    if month.replayed > 0 {
        calls_words.push_str(&format!(", {} of them replayed at no cost", month.replayed));
    }
    cv::Meter {
        spent: budget.show(month.cost),
        cap_words: budget.month_words(month.cost),
        cap_pct: budget.percent(month.cost),
        cap_note: budget.cap_note(),
        calls_words,
        by_purpose: month
            .by_purpose
            .iter()
            .filter(|l| l.cost.0 > 0)
            .map(|l| line(l, l.label.clone().unwrap_or_else(|| purpose_words(&l.key))))
            .collect(),
        by_provider: month
            .by_provider
            .iter()
            .filter(|l| l.cost.0 > 0)
            .map(|l| line(l, provider_words(&l.key)))
            .collect(),
        refused_words: (month.refused > 0).then(|| refused_words(month.refused)),
    }
}

/// "3 AI calls were stopped by a spending limit this month".
pub fn refused_words(refused: i64) -> String {
    format!(
        "{} stopped by a spending limit this month",
        plural(refused, "AI call was", "AI calls were")
    )
}

/// A provider in the owner's words.
pub fn provider_words(provider: &str) -> String {
    match provider {
        "anthropic" => "Anthropic".to_owned(),
        "vultr" => "Vultr".to_owned(),
        "recording" | "replay" => "Replayed (no cost)".to_owned(),
        other => other.to_owned(),
    }
}
