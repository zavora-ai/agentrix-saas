//! Day Time and Night Time (07 §3.5, 01b C3): the shell follows the site's quiet hours.
//!
//! Night Time is the one dark theme, applied on a schedule (01b §5.2): during quiet hours the
//! field becomes the night's timeline and the lane goes quiet except for emergencies. The owner
//! sets the hours; the shell never guesses them. Times are the site's local time, from its UTC
//! offset.

use chrono::{DateTime, Datelike, FixedOffset, NaiveTime, Timelike, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// Quiet hours, in the site's local time. `start` may be later than `end` (22:00 to 06:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietHours {
    /// When Night Time starts.
    pub start: NaiveTime,
    /// When Day Time starts again.
    pub end: NaiveTime,
}

impl Default for QuietHours {
    /// 22:00 to 06:00 until the owner changes them (shell.html's night view).
    fn default() -> Self {
        Self {
            start: hm(22, 0),
            end: hm(6, 0),
        }
    }
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN)
}

/// Which theme the shell wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Opening hours: the Back office cards and the full lane.
    Day,
    /// Quiet hours: the dark field, the night's timeline and a quiet lane.
    Night,
}

impl QuietHours {
    /// Parses `"22:00"` and `"06:00"`. Start and end must differ.
    pub fn parse(start: &str, end: &str) -> Option<Self> {
        let start = NaiveTime::parse_from_str(start.trim(), "%H:%M").ok()?;
        let end = NaiveTime::parse_from_str(end.trim(), "%H:%M").ok()?;
        (start != end).then_some(Self { start, end })
    }

    /// Whether `local` falls inside quiet hours.
    pub fn is_quiet(&self, local: NaiveTime) -> bool {
        if self.start < self.end {
            local >= self.start && local < self.end
        } else {
            local >= self.start || local < self.end
        }
    }

    /// The mode at `local`.
    pub fn mode(&self, local: NaiveTime) -> Mode {
        if self.is_quiet(local) {
            Mode::Night
        } else {
            Mode::Day
        }
    }

    /// "22:00–06:00" (an en dash is fine in a range, voice.md).
    pub fn words(&self) -> String {
        format!("{}–{}", clock(self.start), clock(self.end))
    }

    /// The most recent quiet window that has started by `now`, as UTC instants: tonight's if it
    /// is Night Time now, otherwise last night's.
    pub fn last_window(
        &self,
        now: DateTime<Utc>,
        offset_minutes: i32,
    ) -> (DateTime<Utc>, DateTime<Utc>) {
        let tz = offset(offset_minutes);
        let local = now.with_timezone(&tz);
        let today = local.date_naive();
        let at = |d: chrono::NaiveDate, t: NaiveTime| {
            d.and_time(t)
                .and_local_timezone(tz)
                .single()
                .map(|x| x.with_timezone(&Utc))
                .unwrap_or(now)
        };
        // The candidate starts: today's and yesterday's.
        let mut start = at(today, self.start);
        if start > now {
            start = at(today.pred_opt().unwrap_or(today), self.start);
        }
        let mut end = at(start.with_timezone(&tz).date_naive(), self.end);
        if end <= start {
            end = at(
                start
                    .with_timezone(&tz)
                    .date_naive()
                    .succ_opt()
                    .unwrap_or(today),
                self.end,
            );
        }
        (start, end)
    }
}

/// The site's fixed offset (Nairobi is +180).
pub fn offset(minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"))
}

/// The local time of day at `now`.
pub fn local_time(now: DateTime<Utc>, offset_minutes: i32) -> NaiveTime {
    now.with_timezone(&offset(offset_minutes)).time()
}

/// "06:58".
pub fn clock(t: NaiveTime) -> String {
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// What the time pill says for each mode, and which one is on now.
#[derive(Debug, Clone, Serialize)]
pub struct TimePill {
    /// `day` or `night`, for `data-time`.
    pub mode: Mode,
    /// "Day Time · until 22:00".
    pub day_label: String,
    /// "Night Time · until 06:00".
    pub night_label: String,
    /// "22:00–06:00".
    pub quiet_words: String,
    /// "22:00", for the settings form.
    pub start: String,
    /// "06:00", for the settings form.
    pub end: String,
}

impl TimePill {
    /// The pill at `now` for `hours`.
    pub fn at(hours: &QuietHours, now: DateTime<Utc>, offset_minutes: i32) -> Self {
        Self {
            mode: hours.mode(local_time(now, offset_minutes)),
            day_label: format!("Day Time · until {}", clock(hours.start)),
            night_label: format!("Night Time · until {}", clock(hours.end)),
            quiet_words: hours.words(),
            start: clock(hours.start),
            end: clock(hours.end),
        }
    }
}

/// "Good morning, Wanjiru" by the site's local hour.
pub fn greeting(name: &str, now: DateTime<Utc>, offset_minutes: i32) -> String {
    let hour = local_time(now, offset_minutes).hour();
    let part = match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        _ => "Good evening",
    };
    if name.trim().is_empty() {
        part.to_owned()
    } else {
        format!("{part}, {}", name.trim())
    }
}

/// "Sunday 8 November", in the site's local date.
pub fn long_date(now: DateTime<Utc>, offset_minutes: i32) -> String {
    let d = now.with_timezone(&offset(offset_minutes));
    let day = match d.weekday() {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    };
    let month = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ][d.month0() as usize];
    format!("{day} {} {month}", d.day())
}

/// "06:58" today, "Yesterday 22:14", or "Sat 7 Nov 22:14", in the site's local time.
pub fn when(at: DateTime<Utc>, now: DateTime<Utc>, offset_minutes: i32) -> String {
    let tz = offset(offset_minutes);
    let (a, n) = (at.with_timezone(&tz), now.with_timezone(&tz));
    let t = clock(a.time());
    if a.date_naive() == n.date_naive() {
        t
    } else if n.date_naive().pred_opt() == Some(a.date_naive()) {
        format!("Yesterday {t}")
    } else {
        format!("{} {t}", a.format("%a %-d %b"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> NaiveTime {
        hm(h, m)
    }

    #[test]
    fn quiet_hours_wrap_midnight() {
        let q = QuietHours::default();
        assert!(q.is_quiet(at(22, 0)));
        assert!(q.is_quiet(at(3, 30)));
        assert!(!q.is_quiet(at(6, 0)));
        assert!(!q.is_quiet(at(12, 0)));
        assert_eq!(q.mode(at(21, 59)), Mode::Day);
        assert_eq!(q.words(), "22:00–06:00");
    }

    #[test]
    fn quiet_hours_inside_one_day() {
        let q = QuietHours::parse("13:00", "14:00").expect("parses");
        assert!(q.is_quiet(at(13, 30)));
        assert!(!q.is_quiet(at(14, 0)));
        assert!(QuietHours::parse("10:00", "10:00").is_none());
        assert!(QuietHours::parse("25:00", "10:00").is_none());
    }

    #[test]
    fn pill_follows_the_site_clock() {
        // 20:00 UTC is 23:00 in Nairobi: Night Time.
        let now = Utc.with_ymd_and_hms(2026, 11, 7, 20, 0, 0).unwrap();
        let pill = TimePill::at(&QuietHours::default(), now, 180);
        assert_eq!(pill.mode, Mode::Night);
        assert_eq!(pill.night_label, "Night Time · until 06:00");
        assert_eq!(pill.day_label, "Day Time · until 22:00");
        // The same instant in London is 20:00: Day Time.
        assert_eq!(TimePill::at(&QuietHours::default(), now, 0).mode, Mode::Day);
    }

    #[test]
    fn last_window_is_last_night_by_day_and_tonight_by_night() {
        let q = QuietHours::default();
        // Sun 8 Nov 09:00 EAT = 06:00 UTC: last night ran Sat 22:00 to Sun 06:00 EAT.
        let morning = Utc.with_ymd_and_hms(2026, 11, 8, 6, 0, 0).unwrap();
        let (s, e) = q.last_window(morning, 180);
        assert_eq!(s, Utc.with_ymd_and_hms(2026, 11, 7, 19, 0, 0).unwrap());
        assert_eq!(e, Utc.with_ymd_and_hms(2026, 11, 8, 3, 0, 0).unwrap());
        // Sun 8 Nov 01:00 EAT = Sat 22:00 UTC: tonight started Sat 22:00 EAT.
        let night = Utc.with_ymd_and_hms(2026, 11, 7, 22, 0, 0).unwrap();
        let (s, e) = q.last_window(night, 180);
        assert_eq!(s, Utc.with_ymd_and_hms(2026, 11, 7, 19, 0, 0).unwrap());
        assert_eq!(e, Utc.with_ymd_and_hms(2026, 11, 8, 3, 0, 0).unwrap());
    }

    #[test]
    fn words_for_people() {
        let now = Utc.with_ymd_and_hms(2026, 11, 8, 6, 0, 0).unwrap();
        assert_eq!(greeting("Wanjiru", now, 180), "Good morning, Wanjiru");
        assert_eq!(greeting("", now, 180), "Good morning");
        assert_eq!(long_date(now, 180), "Sunday 8 November");
        let earlier = Utc.with_ymd_and_hms(2026, 11, 7, 19, 14, 0).unwrap();
        assert_eq!(when(earlier, now, 180), "Yesterday 22:14");
        assert_eq!(when(now, now, 180), "09:00");
    }
}
