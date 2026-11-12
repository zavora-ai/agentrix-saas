//! Times as `control.db` stores them, and the windows every number is counted over.
//!
//! The first release stores RFC 3339 UTC text with microseconds, so text order is time order
//! (`crates/agentrix-node/src/db/mod.rs`). The auth and claim tables use Unix seconds; nothing
//! here reads them.

use chrono::{DateTime, Datelike, Duration, FixedOffset, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// A time as stored: RFC 3339, UTC, always with microseconds.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Reads a stored time. `None` for text that isn't RFC 3339.
pub fn parse(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The window a number is counted over, as the owner or operator would say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "since")]
pub enum Window {
    /// Since midnight in the site's own time (UTC for the fleet).
    Today,
    /// The last 24 hours.
    Last24Hours,
    /// The last 7 days.
    Last7Days,
    /// The last 30 days.
    Last30Days,
    /// Since the first of this month in the site's own time (UTC for the fleet).
    ThisMonth,
    /// Everything recorded.
    All,
    /// Since a given time.
    Since(DateTime<Utc>),
}

/// A resolved window: where it starts and ends, and how to say it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Span {
    /// The start, or `None` for everything recorded.
    pub from: Option<DateTime<Utc>>,
    /// The end (now, when the number was read).
    pub to: DateTime<Utc>,
    /// Plain words: "today", "in the last 7 days", "this month", "so far".
    pub words: &'static str,
}

impl Span {
    /// The start as stored text, or the empty string (before every stored time).
    pub fn from_stamp(&self) -> String {
        self.from.map(stamp).unwrap_or_default()
    }
}

impl Window {
    /// Resolves the window at `now` for a place `utc_offset_minutes` from UTC.
    ///
    /// The first release keeps a fixed offset per site (`sites.utc_offset_minutes`), so "today" is right
    /// for Nairobi but drifts by an hour across a daylight-saving change elsewhere. The IANA
    /// zone W3 adds to `sites` fixes that (03-data.md §2.4).
    pub fn span(self, now: DateTime<Utc>, utc_offset_minutes: i32) -> Span {
        let local_midnight = |day_of_month: Option<u32>| {
            let offset = FixedOffset::east_opt(utc_offset_minutes.saturating_mul(60))
                .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
            let local = now.with_timezone(&offset);
            let day = day_of_month.unwrap_or(local.day());
            offset
                .with_ymd_and_hms(local.year(), local.month(), day, 0, 0, 0)
                .single()
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or(now)
        };
        let (from, words) = match self {
            Self::Today => (Some(local_midnight(None)), "today"),
            Self::Last24Hours => (Some(now - Duration::hours(24)), "in the last 24 hours"),
            Self::Last7Days => (Some(now - Duration::days(7)), "in the last 7 days"),
            Self::Last30Days => (Some(now - Duration::days(30)), "in the last 30 days"),
            Self::ThisMonth => (Some(local_midnight(Some(1))), "this month"),
            Self::All => (None, "so far"),
            Self::Since(at) => (Some(at), "since then"),
        };
        Span {
            from,
            to: now,
            words,
        }
    }
}

/// Milliseconds between two stored times, if both parse and the end is not before the start.
pub fn millis_between(start: &str, end: &str) -> Option<i64> {
    let (start, end) = (parse(start)?, parse(end)?);
    let ms = (end - start).num_milliseconds();
    (ms >= 0).then_some(ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        parse(text).unwrap()
    }

    #[test]
    fn today_starts_at_local_midnight() {
        // 01:30 in Nairobi on Sun 8 Nov is 22:30 UTC on Sat 7 Nov.
        let now = at("2026-11-07T22:30:00Z");
        let nairobi = Window::Today.span(now, 180);
        assert_eq!(nairobi.from, Some(at("2026-11-07T21:00:00Z")));
        let utc = Window::Today.span(now, 0);
        assert_eq!(utc.from, Some(at("2026-11-07T00:00:00Z")));
        assert_eq!(nairobi.words, "today");
    }

    #[test]
    fn this_month_and_all() {
        let now = at("2026-11-08T09:00:00Z");
        assert_eq!(
            Window::ThisMonth.span(now, 0).from,
            Some(at("2026-11-01T00:00:00Z"))
        );
        let all = Window::All.span(now, 180);
        assert_eq!((all.from, all.from_stamp()), (None, String::new()));
    }

    #[test]
    fn stamps_sort_as_times() {
        let a = stamp(at("2026-11-08T09:00:00Z"));
        let b = stamp(at("2026-11-08T09:00:00.5Z"));
        assert!(a < b);
        assert_eq!(a, "2026-11-08T09:00:00.000000Z");
        assert_eq!(millis_between(&a, &b), Some(500));
        assert_eq!(millis_between(&b, &a), None);
    }
}
