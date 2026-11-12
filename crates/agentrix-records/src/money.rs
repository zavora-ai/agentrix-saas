//! Money on screen: "KSh 4,500", "USD 19.77", and the posted-rate conversion.

/// Groups thousands: 4500 → "4,500".
fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// A minor amount in a currency, as people read it: `KSh 4,500`, `KSh 4,500.50`, `USD 19.77`.
pub fn fmt(currency: &str, minor: i64) -> String {
    let symbol = match currency {
        "KES" => "KSh",
        other => other,
    };
    let whole = minor / 100;
    let cents = (minor % 100).abs();
    if currency == "KES" && cents == 0 {
        format!("{symbol} {}", grouped(whole))
    } else {
        format!("{symbol} {}.{cents:02}", grouped(whole))
    }
}

/// `KSh 4,500` for a KES minor amount.
pub fn ksh(minor: i64) -> String {
    fmt("KES", minor)
}

/// US cents for a local minor amount at a posted rate (`rate_minor` local minor units per
/// dollar, so KSh 129.00 is 12900), rounded half up.
pub fn usd_cents(local_minor: i64, rate_minor: i64) -> i64 {
    if rate_minor <= 0 {
        return 0;
    }
    (local_minor * 100 * 2 + rate_minor) / (rate_minor * 2)
}

/// `USD 19.77`.
pub fn usd(cents: i64) -> String {
    fmt("USD", cents)
}

/// The rate as posted: `1 USD = KSh 129.00`.
pub fn rate_line(rate_minor: i64) -> String {
    format!(
        "1 USD = KSh {}.{:02}",
        grouped(rate_minor / 100),
        rate_minor % 100
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shillings_and_dollars_read_as_people_write_them() {
        assert_eq!(ksh(450_000), "KSh 4,500");
        assert_eq!(ksh(18_000), "KSh 180");
        assert_eq!(ksh(1_250_050), "KSh 12,500.50");
        assert_eq!(usd(1977), "USD 19.77");
        assert_eq!(rate_line(12_900), "1 USD = KSh 129.00");
    }

    #[test]
    fn the_posted_rate_gives_the_journey_amounts() {
        assert_eq!(usd_cents(255_000, 12_900), 1977);
        assert_eq!(usd_cents(525_000, 12_900), 4070);
        assert_eq!(usd_cents(135_000, 12_900), 1047);
    }
}
