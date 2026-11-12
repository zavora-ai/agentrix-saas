//! Times: now, a site's local day and clock, and the relative times sample bundles store.
//!
//! Bundles never hold absolute dates (03-data.md §5.2 rule 5). A time is one of:
//! - `m-95`: minutes from the load instant (negative is earlier);
//! - `d+7 10:00`: the local day `n` days after the load day, at a local clock time;
//! - `sat+1 10:00`: the `n`th upcoming Saturday after the load day (1 is the next one), at a
//!   local clock time. Weekday names are the first three letters.

use anyhow::{Context as _, bail};
use chrono::{
    DateTime, Datelike as _, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone as _, Utc,
    Weekday,
};

/// Unix seconds now.
pub fn now() -> i64 {
    Utc::now().timestamp()
}

/// The fixed offset for `minutes` east of UTC.
pub fn offset(minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"))
}

/// The local date-time of `ts`.
pub fn local(ts: i64, minutes: i32) -> DateTime<FixedOffset> {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_default()
        .with_timezone(&offset(minutes))
}

/// The local day of `ts` as `2026-11-14`.
pub fn day_of(ts: i64, minutes: i32) -> String {
    local(ts, minutes).format("%Y-%m-%d").to_string()
}

/// `Sat 14 Nov`.
pub fn fmt_day(ts: i64, minutes: i32) -> String {
    local(ts, minutes).format("%a %-d %b").to_string()
}

/// `10:00`.
pub fn fmt_clock(ts: i64, minutes: i32) -> String {
    local(ts, minutes).format("%H:%M").to_string()
}

/// `Sat 14 Nov, 10:00`.
pub fn fmt_day_clock(ts: i64, minutes: i32) -> String {
    local(ts, minutes).format("%a %-d %b, %H:%M").to_string()
}

/// `Sat 14 Nov 2026`.
pub fn fmt_date_long(ts: i64, minutes: i32) -> String {
    local(ts, minutes).format("%a %-d %b %Y").to_string()
}

/// The unix time of a local day and clock.
///
/// # Errors
///
/// When the time is not a valid clock time.
pub fn at_local(day: NaiveDate, clock: &str, minutes: i32) -> anyhow::Result<i64> {
    let time =
        NaiveTime::parse_from_str(clock, "%H:%M").with_context(|| format!("clock {clock}"))?;
    let naive = day.and_time(time);
    Ok(offset(minutes)
        .from_local_datetime(&naive)
        .single()
        .context("ambiguous local time")?
        .timestamp())
}

/// The start of the local day of `ts`, as unix seconds.
pub fn day_start(ts: i64, minutes: i32) -> i64 {
    let d = local(ts, minutes).date_naive();
    at_local(d, "00:00", minutes).unwrap_or(ts)
}

fn weekday(name: &str) -> Option<Weekday> {
    Some(match name {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// Resolves a bundle time against the load instant `anchor`.
///
/// # Errors
///
/// When the text is not one of the forms in the module docs.
pub fn resolve(spec: &str, anchor: i64, minutes: i32) -> anyhow::Result<i64> {
    let spec = spec.trim();
    if let Some(rest) = spec.strip_prefix('m').filter(|r| r.starts_with(['+', '-'])) {
        let n: i64 = rest.parse().with_context(|| format!("minutes in {spec}"))?;
        return Ok(anchor + n * 60);
    }
    let (head, clock) = spec.split_once(' ').unwrap_or((spec, "00:00"));
    let (unit, n) = head
        .split_once('+')
        .map(|(u, n)| (u, n.parse::<i64>()))
        .or_else(|| {
            head.split_once('-')
                .map(|(u, n)| (u, n.parse::<i64>().map(|x| -x)))
        })
        .with_context(|| format!("time {spec}"))?;
    let n = n.with_context(|| format!("count in {spec}"))?;
    let today = local(anchor, minutes).date_naive();
    let day = if unit == "d" {
        today + Duration::days(n)
    } else if let Some(wd) = weekday(unit) {
        let ahead = (7 + wd.num_days_from_monday() as i64
            - today.weekday().num_days_from_monday() as i64)
            % 7;
        if n >= 1 {
            let first = if ahead == 0 { 7 } else { ahead };
            today + Duration::days(first + 7 * (n - 1))
        } else if n <= -1 {
            // The |n|th such weekday before the load day.
            let back = if ahead == 0 { 7 } else { 7 - ahead };
            today - Duration::days(back + 7 * (-n - 1))
        } else {
            bail!("weekday count is never 0 in {spec}");
        }
    } else {
        bail!("unknown unit in {spec}");
    };
    at_local(day, clock, minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sat 7 Nov 2026, 09:42 EAT.
    const ANCHOR: i64 = 1_794_033_720;

    #[test]
    fn anchor_is_saturday_morning_in_nairobi() {
        assert_eq!(fmt_day_clock(ANCHOR, 180), "Sat 7 Nov, 09:42");
    }

    #[test]
    fn relative_forms_resolve() {
        assert_eq!(
            fmt_day_clock(resolve("m-60", ANCHOR, 180).unwrap(), 180),
            "Sat 7 Nov, 08:42"
        );
        assert_eq!(
            fmt_day_clock(resolve("d+7 10:00", ANCHOR, 180).unwrap(), 180),
            "Sat 14 Nov, 10:00"
        );
        assert_eq!(
            fmt_day_clock(resolve("sat+1 10:00", ANCHOR, 180).unwrap(), 180),
            "Sat 14 Nov, 10:00"
        );
        assert_eq!(
            fmt_day_clock(resolve("sun+1 10:00", ANCHOR, 180).unwrap(), 180),
            "Sun 8 Nov, 10:00"
        );
        assert_eq!(
            fmt_day_clock(resolve("d-2 11:04", ANCHOR, 180).unwrap(), 180),
            "Thu 5 Nov, 11:04"
        );
        assert_eq!(
            fmt_day_clock(resolve("sat-1 10:00", ANCHOR, 180).unwrap(), 180),
            "Sat 31 Oct, 10:00"
        );
        assert_eq!(
            fmt_day_clock(resolve("fri-1 07:00", ANCHOR, 180).unwrap(), 180),
            "Fri 6 Nov, 07:00"
        );
        assert_eq!(day_of(ANCHOR, 180), "2026-11-07");
    }
}
