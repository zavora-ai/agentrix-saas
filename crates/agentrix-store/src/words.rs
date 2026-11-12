//! Numbers as people read them (specs/platform/voice.md): money, counts and the small-cell rule.

use serde::Serialize;

/// Breakdowns of visits below this are shown as "fewer than 5" on every screen, export and
/// report; exact counts stay inside the store (06 §4.9, 09 §5.3).
pub const SMALL_CELL: u64 = 5;

/// A count as it may be shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Shown {
    /// The exact number: 5 or more.
    Exact(u64),
    /// Between 1 and 4: shown only as "fewer than 5".
    FewerThan5,
    /// Nothing counted.
    None,
}

impl Shown {
    /// Applies the small-cell rule to an exact count.
    pub fn of(count: u64) -> Self {
        match count {
            0 => Self::None,
            n if n < SMALL_CELL => Self::FewerThan5,
            n => Self::Exact(n),
        }
    }

    /// Plain words: "41", "fewer than 5", "none".
    pub fn words(&self) -> String {
        match self {
            Self::Exact(n) => thousands(*n as i128),
            Self::FewerThan5 => "fewer than 5".to_owned(),
            Self::None => "none".to_owned(),
        }
    }
}

/// `2400` as "2,400".
pub fn thousands(n: i128) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Micro-USD (millionths of a dollar), the unit the Model Gateway meters in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize)]
#[serde(transparent)]
pub struct MicroUsd(pub i64);

impl MicroUsd {
    /// Dollars as words: "$0.42", "$1,140.00", "less than $0.01", "$0.00".
    pub fn usd(self) -> String {
        if self.0 > 0 && self.0 < 5_000 {
            return "less than $0.01".to_owned();
        }
        let cents = (self.0 as i128 + 5_000).div_euclid(10_000);
        format!("${}.{:02}", thousands(cents / 100), (cents % 100).abs())
    }

    /// The amount in a site's own currency at a posted rate (units of that currency per US
    /// dollar), rounded to whole units as owners read budgets: "KSh 38". The rate must come
    /// from a posted exchange rate (W3's `exchange_rates`), never a constant.
    pub fn local(self, currency: &str, units_per_usd: f64) -> String {
        let units = (self.0 as f64 / 1e6 * units_per_usd).round();
        let symbol = match currency {
            "KES" => "KSh",
            "USD" => "$",
            "NGN" => "₦",
            "ZAR" => "R",
            other => other,
        };
        let n = thousands(units as i128);
        if symbol == "$" || symbol == "₦" || symbol == "R" {
            format!("{symbol}{n}")
        } else {
            format!("{symbol} {n}")
        }
    }
}

/// "1 call", "2 calls".
pub fn plural(n: i64, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        thousands(n as i128),
        if n == 1 { one } else { many }
    )
}

/// A duration in plain words: "40 s", "3 min", "1 h 12 min".
pub fn duration_words(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s / 60),
        _ => {
            let (h, m) = (s / 3600, (s % 3600) / 60);
            if m == 0 {
                format!("{h} h")
            } else {
                format!("{h} h {m} min")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_cells_are_hidden() {
        assert_eq!(Shown::of(0).words(), "none");
        assert_eq!(Shown::of(1).words(), "fewer than 5");
        assert_eq!(Shown::of(4).words(), "fewer than 5");
        assert_eq!(Shown::of(5).words(), "5");
        assert_eq!(Shown::of(12_400).words(), "12,400");
        assert_eq!(
            serde_json::to_value(Shown::of(3)).unwrap(),
            serde_json::json!({"kind": "fewer_than5"})
        );
    }

    #[test]
    fn money_reads_as_people_say_it() {
        assert_eq!(MicroUsd(420_001).usd(), "$0.42");
        assert_eq!(MicroUsd(0).usd(), "$0.00");
        assert_eq!(MicroUsd(1).usd(), "less than $0.01");
        assert_eq!(MicroUsd(1_140_000_000).usd(), "$1,140.00");
        assert_eq!(MicroUsd(294_000).local("KES", 129.0), "KSh 38");
        assert_eq!(MicroUsd(11_627_906).local("KES", 129.0), "KSh 1,500");
    }

    #[test]
    fn durations_and_plurals() {
        assert_eq!(duration_words(40_000), "40 s");
        assert_eq!(duration_words(185_000), "3 min");
        assert_eq!(duration_words(4_320_000), "1 h 12 min");
        assert_eq!(plural(1, "call", "calls"), "1 call");
        assert_eq!(plural(31, "call", "calls"), "31 calls");
        assert_eq!(thousands(-2400), "-2,400");
    }
}
