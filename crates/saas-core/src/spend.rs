//! The platform's model spend as vendor bills (v3).
//!
//! The platform's model gateway meters every call a company's sites make (`control.db`'s
//! `usage`, priced from the provider's catalogue). [`bill_model_spend`] turns one window of one
//! provider's usage into the bill that provider would send (Anthropic, Vultr), drafts and posts
//! it in the company's books, and finds it again by its reference, so a window is billed once.
//! Payables then pays it by PayPal Payout, inside the owner's mandate.
//!
//! - **Window.** `[from, until)`, with `until` on the hour, so every run in the same hour asks
//!   for the same window. [`next_window`] starts where the provider's last bill ended
//!   ([`billed_until`]) when that is later than `since`, so windows never overlap.
//! - **Cents.** The gateway counts micro-dollars. A bill's total is the window's exact spend
//!   rounded to the cent; its lines get their cents by largest remainder, so they add up to it.
//!   Spend that rounds to no cents isn't billed, and the window doesn't move: it waits for the
//!   next run.
//! - **Reference.** `models:<provider>:<from>-<until>` (UTC, `20261111T070000Z`), as the bill's
//!   vendor reference.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, DurationRound, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::books::{
    Bill, BillDraft, Books, BooksResult, Counterparty, DocumentFilter, DocumentStatus, LineDraft,
    VendorId,
};
use crate::money::{Currency, Money};

/// Every model-spend bill's vendor reference starts with this.
pub const REFERENCE_PREFIX: &str = "models:";

const STAMP: &str = "%Y%m%dT%H%M%SZ";

/// Days from a model-spend bill's date to its due date.
pub const DUE_DAYS: i64 = 7;

/// One group of metered calls in a window: one provider's model (and unit, for images and
/// speech), summed over the company's sites.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    /// The provider, as the gateway names it: `anthropic`, `vultr`.
    pub provider: String,
    /// The model.
    pub model: String,
    /// For calls not counted in tokens: `image`, `audio_second` or `character`.
    pub unit: Option<String>,
    /// How many calls.
    pub calls: u64,
    /// Tokens in.
    pub tokens_in: u64,
    /// Tokens out.
    pub tokens_out: u64,
    /// How many units (images, seconds, characters).
    pub units: u64,
    /// What the gateway metered, in micro-USD.
    pub cost_micro_usd: i64,
}

impl ModelUsage {
    /// The bill line's words: "claude-sonnet-5-5: 12 calls, 34,567 tokens in, 8,901 out (USD
    /// 0.123456)".
    pub fn describe(&self) -> String {
        let mut s = format!(
            "{}: {} call{}",
            self.model,
            thousands(self.calls),
            if self.calls == 1 { "" } else { "s" }
        );
        if self.tokens_in > 0 || self.tokens_out > 0 {
            s.push_str(&format!(
                ", {} tokens in, {} out",
                thousands(self.tokens_in),
                thousands(self.tokens_out)
            ));
        }
        if let Some(unit) = &self.unit
            && self.units > 0
        {
            let words = match (unit.as_str(), self.units) {
                ("image", 1) => "image",
                ("image", _) => "images",
                ("audio_second", 1) => "second of audio",
                ("audio_second", _) => "seconds of audio",
                ("character", 1) => "character",
                ("character", _) => "characters",
                (other, _) => other,
            };
            s.push_str(&format!(", {} {words}", thousands(self.units)));
        }
        s.push_str(&format!(" (USD {})", micro_usd(self.cost_micro_usd)));
        s
    }
}

/// The span of time a model-spend bill covers: `[from, until)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendWindow {
    /// From (inclusive).
    pub from: DateTime<Utc>,
    /// Until (exclusive).
    pub until: DateTime<Utc>,
}

impl SpendWindow {
    /// Whether the window holds no time.
    pub fn is_empty(&self) -> bool {
        self.from >= self.until
    }

    /// The bill's vendor reference for `provider` in this window.
    pub fn reference(&self, provider: &str) -> String {
        format!(
            "{REFERENCE_PREFIX}{provider}:{}-{}",
            self.from.format(STAMP),
            self.until.format(STAMP)
        )
    }

    /// The provider and window a vendor reference names, when it is a model-spend bill's.
    pub fn from_reference(reference: &str) -> Option<(String, SpendWindow)> {
        let rest = reference.strip_prefix(REFERENCE_PREFIX)?;
        let (provider, span) = rest.rsplit_once(':')?;
        let (from, until) = span.split_once('-')?;
        let parse = |t: &str| {
            NaiveDateTime::parse_from_str(t, STAMP)
                .ok()
                .map(|n| n.and_utc())
        };
        Some((
            provider.to_owned(),
            SpendWindow {
                from: parse(from)?,
                until: parse(until)?,
            },
        ))
    }
}

/// The next window to bill a provider for: from `since`, or from where its last bill ended
/// when that is later, until the start of the current hour.
pub fn next_window(
    billed_until: Option<DateTime<Utc>>,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> SpendWindow {
    let until = now.duration_trunc(Duration::hours(1)).unwrap_or(now);
    let from = billed_until.map_or(since, |b| b.max(since));
    SpendWindow { from, until }
}

/// Where each provider's model-spend bills in these books end (void bills aside).
pub async fn billed_until(books: &dyn Books) -> BooksResult<BTreeMap<String, DateTime<Utc>>> {
    let mut out: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
    for bill in books.bills(&DocumentFilter::default()).await? {
        if bill.status == DocumentStatus::Void {
            continue;
        }
        let Some((provider, window)) = bill
            .vendor_reference
            .as_deref()
            .and_then(SpendWindow::from_reference)
        else {
            continue;
        };
        let end = out.entry(provider).or_insert(window.until);
        if window.until > *end {
            *end = window.until;
        }
    }
    Ok(out)
}

/// The vendor who bills a provider's calls: Anthropic for `anthropic`, Vultr for `vultr`. A
/// Payout to them goes to `paypal_email`.
pub fn vendor_for(provider: &str, paypal_email: Option<String>) -> Counterparty {
    let name = match provider {
        "anthropic" => "Anthropic".to_owned(),
        "vultr" => "Vultr".to_owned(),
        other => {
            let mut c = other.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    };
    Counterparty {
        paypal_email,
        ..Counterparty::usd(name, None)
    }
}

/// One provider's bill for one window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpendBill {
    /// The provider, as the gateway names it.
    pub provider: String,
    /// The window it covers.
    pub window: SpendWindow,
    /// The vendor in the books.
    pub vendor: VendorId,
    /// The bill, posted.
    pub bill: Bill,
    /// What was metered, one group per line.
    pub usage: Vec<ModelUsage>,
    /// The exact spend, in micro-USD (the bill's total is this rounded to the cent).
    pub cost_micro_usd: i64,
    /// Whether the bill was already in the books before this call.
    pub already: bool,
}

/// What one run of [`bill_model_spend`] over a company's providers left.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpend {
    /// The bills, one per provider with spend in its window.
    pub bills: Vec<ModelSpendBill>,
    /// Spend that rounds to no cents yet, in micro-USD by provider: it waits for a later run.
    pub waiting: BTreeMap<String, i64>,
}

/// Bills one provider's metered usage for one window to the company: finds the bill by its
/// reference (and posts it if an earlier run stopped before that), or drafts and posts a new
/// one. `Ok(None)` when the window's spend rounds to no cents. `sites` name where the calls
/// came from, in the bill's note.
pub async fn bill_model_spend(
    books: &dyn Books,
    provider: &str,
    window: &SpendWindow,
    usage: &[ModelUsage],
    sites: &[String],
    payee: Option<String>,
) -> BooksResult<Option<ModelSpendBill>> {
    let usage: Vec<ModelUsage> = usage
        .iter()
        .filter(|u| u.provider == provider && u.cost_micro_usd > 0)
        .cloned()
        .collect();
    let cost_micro_usd: i64 = usage.iter().map(|u| u.cost_micro_usd).sum();
    let cents = split_cents(&usage.iter().map(|u| u.cost_micro_usd).collect::<Vec<_>>());
    if window.is_empty() || cents.iter().all(|c| *c == 0) {
        return Ok(None);
    }
    let reference = window.reference(provider);
    let filter = DocumentFilter {
        reference: Some(reference.clone()),
        ..Default::default()
    };
    let found = books
        .bills(&filter)
        .await?
        .into_iter()
        .find(|b| b.status != DocumentStatus::Void);
    let (bill, already) = match found {
        Some(b) if matches!(b.status, DocumentStatus::Draft | DocumentStatus::Submitted) => {
            (books.post_bill(&b.id).await?, true)
        }
        Some(b) => (b, true),
        None => {
            let vendor = books.ensure_vendor(&vendor_for(provider, payee)).await?;
            let lines: Vec<LineDraft> = usage
                .iter()
                .zip(&cents)
                .filter(|(_, c)| **c > 0)
                .map(|(u, c)| LineDraft {
                    product: None,
                    description: u.describe(),
                    quantity: 1,
                    unit_price: Money::usd(*c),
                })
                .collect();
            let calls: u64 = usage.iter().map(|u| u.calls).sum();
            let issue_date = window.until.date_naive();
            let draft = BillDraft {
                vendor: vendor.id,
                vendor_reference: Some(reference),
                issue_date,
                due_date: issue_date + Duration::days(DUE_DAYS),
                currency: Currency::USD,
                lines,
                memo: Some(format!(
                    "Model calls metered by the Agentrix model gateway for {}, {} to {} UTC: \
                     {} calls, USD {} exactly. Lines are rounded to the cent and add up to the \
                     total.",
                    if sites.is_empty() {
                        "this company's sites".to_owned()
                    } else {
                        sites.join(", ")
                    },
                    window.from.format("%Y-%m-%d %H:%M"),
                    window.until.format("%Y-%m-%d %H:%M"),
                    thousands(calls),
                    micro_usd(cost_micro_usd)
                )),
            };
            let drafted = books.draft_bill(&draft).await?;
            (books.post_bill(&drafted.id).await?, false)
        }
    };
    Ok(Some(ModelSpendBill {
        provider: provider.to_owned(),
        window: *window,
        vendor: bill.vendor.clone(),
        bill,
        usage,
        cost_micro_usd,
        already,
    }))
}

/// Splits micro-USD amounts into cents by largest remainder: they add up to the exact total
/// rounded half up to the cent.
pub fn split_cents(micro: &[i64]) -> Vec<i64> {
    const PER_CENT: i64 = 10_000;
    let total: i64 = micro.iter().map(|m| (*m).max(0)).sum();
    let target = (total + PER_CENT / 2) / PER_CENT;
    let mut out: Vec<i64> = micro.iter().map(|m| (*m).max(0) / PER_CENT).collect();
    let mut left = target - out.iter().sum::<i64>();
    let mut order: Vec<usize> = (0..micro.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(micro[i].max(0) % PER_CENT), i));
    for i in order {
        if left <= 0 {
            break;
        }
        out[i] += 1;
        left -= 1;
    }
    out
}

/// Micro-USD as dollars with six places: `0.123456`.
pub fn micro_usd(micro: i64) -> String {
    let sign = if micro < 0 { "-" } else { "" };
    let m = micro.unsigned_abs();
    format!("{sign}{}.{:06}", m / 1_000_000, m % 1_000_000)
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::CompanyId;
    use crate::fake::FakeBooks;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 11, h, m, 0).unwrap()
    }

    fn row(provider: &str, model: &str, calls: u64, micro: i64) -> ModelUsage {
        ModelUsage {
            provider: provider.into(),
            model: model.into(),
            unit: None,
            calls,
            tokens_in: calls * 1_000,
            tokens_out: calls * 200,
            units: 0,
            cost_micro_usd: micro,
        }
    }

    #[test]
    fn cents_add_up_to_the_rounded_total() {
        // 1.234567 + 0.004999 + 0.002000 = 1.241566 -> 124 cents.
        let c = split_cents(&[1_234_567, 4_999, 2_000]);
        assert_eq!(c.iter().sum::<i64>(), 124);
        assert_eq!(c, vec![123, 1, 0]);
        assert_eq!(split_cents(&[4_999]), vec![0]);
        assert_eq!(split_cents(&[5_000]), vec![1]);
        assert_eq!(split_cents(&[]), Vec::<i64>::new());
    }

    #[test]
    fn a_reference_names_its_provider_and_window() {
        let w = SpendWindow {
            from: at(6, 30),
            until: at(8, 0),
        };
        let r = w.reference("anthropic");
        assert_eq!(r, "models:anthropic:20261111T063000Z-20261111T080000Z");
        assert_eq!(
            SpendWindow::from_reference(&r),
            Some(("anthropic".into(), w))
        );
        assert_eq!(SpendWindow::from_reference("INV-0001"), None);
    }

    #[test]
    fn the_next_window_ends_on_the_hour_and_never_overlaps() {
        let w = next_window(None, at(6, 0), at(9, 41));
        assert_eq!((w.from, w.until), (at(6, 0), at(9, 0)));
        let w = next_window(Some(at(8, 0)), at(6, 0), at(9, 41));
        assert_eq!((w.from, w.until), (at(8, 0), at(9, 0)));
        assert!(next_window(Some(at(9, 0)), at(6, 0), at(9, 41)).is_empty());
    }

    #[test]
    fn a_line_says_what_was_metered() {
        let mut u = row("anthropic", "claude-sonnet-5-5", 12, 123_456);
        u.tokens_in = 34_567;
        u.tokens_out = 8_901;
        assert_eq!(
            u.describe(),
            "claude-sonnet-5-5: 12 calls, 34,567 tokens in, 8,901 out (USD 0.123456)"
        );
        let img = ModelUsage {
            unit: Some("image".into()),
            units: 3,
            tokens_in: 0,
            tokens_out: 0,
            ..row("vultr", "flux-1-schnell", 3, 9_000)
        };
        assert_eq!(
            img.describe(),
            "flux-1-schnell: 3 calls, 3 images (USD 0.009000)"
        );
    }

    #[tokio::test]
    async fn a_window_is_billed_once_per_provider() {
        let books = FakeBooks::new(CompanyId::kilele());
        let usage = vec![
            row("anthropic", "claude-sonnet-5-5", 40, 1_234_567),
            row("anthropic", "claude-haiku-5-5", 9, 4_999),
            row("vultr", "llama-4-scout", 120, 310_000),
        ];
        let sites = vec!["saas-kilele".to_owned(), "kilele-bakery-sample".to_owned()];
        let w = next_window(None, at(6, 0), at(9, 41));
        let a = bill_model_spend(&books, "anthropic", &w, &usage, &sites, None)
            .await
            .unwrap()
            .expect("a bill");
        assert!(!a.already);
        assert_eq!(a.bill.status, DocumentStatus::Posted);
        assert_eq!(a.bill.total, Money::usd(124));
        assert_eq!(a.cost_micro_usd, 1_239_566);
        assert_eq!(a.usage.len(), 2);
        assert_eq!(
            a.bill.vendor_reference.as_deref(),
            Some("models:anthropic:20261111T060000Z-20261111T090000Z")
        );
        let vendor = books.vendor(&a.vendor).await.unwrap();
        assert_eq!(vendor.counterparty.name, "Anthropic");

        let v = bill_model_spend(&books, "vultr", &w, &usage, &sites, None)
            .await
            .unwrap()
            .expect("a bill");
        assert_eq!(v.bill.total, Money::usd(31));
        assert_eq!(
            books.vendor(&v.vendor).await.unwrap().counterparty.name,
            "Vultr"
        );

        // The same hour again: the same bills, nothing new.
        let again = bill_model_spend(&books, "anthropic", &w, &usage, &sites, None)
            .await
            .unwrap()
            .expect("the bill");
        assert!(again.already);
        assert_eq!(again.bill.id, a.bill.id);
        assert_eq!(
            books.bills(&DocumentFilter::default()).await.unwrap().len(),
            2
        );

        // The next run starts where these ended.
        let ends = billed_until(&books).await.unwrap();
        assert_eq!(ends.get("anthropic"), Some(&at(9, 0)));
        assert_eq!(ends.get("vultr"), Some(&at(9, 0)));
        assert!(next_window(ends.get("anthropic").copied(), at(6, 0), at(9, 59)).is_empty());
    }

    #[tokio::test]
    async fn spend_under_half_a_cent_waits() {
        let books = FakeBooks::new(CompanyId::agentrix());
        let w = next_window(None, at(6, 0), at(9, 41));
        let usage = vec![row("vultr", "llama-4-scout", 2, 4_000)];
        let none = bill_model_spend(&books, "vultr", &w, &usage, &[], None)
            .await
            .unwrap();
        assert!(none.is_none());
        assert!(billed_until(&books).await.unwrap().is_empty());
    }
}
