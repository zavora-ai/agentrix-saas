//! Money: an integer count of a currency's minor unit, never a float.
//!
//! The JSON shape `{"currency": "USD", "minor": 1900}` is the same as `agentrix-types`'
//! `Money`, so the two convert field for field.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Why an amount or currency was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    /// Currencies are three uppercase ASCII letters (ISO 4217).
    #[error("not an ISO 4217 currency code: {0:?}")]
    Currency(String),
    /// Arithmetic or comparison across two currencies.
    #[error("currency mismatch: {0} and {1}")]
    Mismatch(Currency, Currency),
    /// A decimal string that isn't an amount in this currency (too many decimals, not a number).
    #[error("not an amount in {currency}: {value:?}")]
    Amount {
        /// The currency it was read in.
        currency: Currency,
        /// The text refused.
        value: String,
    },
    /// The result does not fit in an `i64` of minor units.
    #[error("amount overflow")]
    Overflow,
}

/// An ISO 4217 currency code, for example `USD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Currency([u8; 3]);

impl Currency {
    /// US dollars: the currency of every PayPal amount in this entry.
    pub const USD: Currency = Currency(*b"USD");
    /// Kenyan shillings.
    pub const KES: Currency = Currency(*b"KES");

    /// A currency from its code.
    pub fn new(code: &str) -> Result<Self, MoneyError> {
        let b = code.as_bytes();
        if b.len() == 3 && b.iter().all(u8::is_ascii_uppercase) {
            Ok(Self([b[0], b[1], b[2]]))
        } else {
            Err(MoneyError::Currency(code.to_owned()))
        }
    }

    /// The code, for example `"USD"`.
    pub fn code(&self) -> &str {
        // The bytes are checked ASCII uppercase on construction.
        std::str::from_utf8(&self.0).unwrap_or("XXX")
    }

    /// Digits after the decimal point in this currency's major unit (PayPal's list: these
    /// have none; everything else here has two).
    pub fn exponent(&self) -> u32 {
        match self.code() {
            "HUF" | "JPY" | "TWD" | "KRW" | "CLP" | "ISK" | "VND" | "UGX" | "XAF" | "XOF" => 0,
            _ => 2,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Currency {
    type Err = MoneyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for Currency {
    type Error = MoneyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(&s)
    }
}

impl From<Currency> for String {
    fn from(c: Currency) -> Self {
        c.code().to_owned()
    }
}

/// An amount: a whole number of the currency's minor unit (cents for USD).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Money {
    /// The currency.
    pub currency: Currency,
    /// The amount in minor units; negative for credits where a sign is needed.
    pub minor: i64,
}

impl Money {
    /// `minor` units of `currency`.
    pub const fn new(minor: i64, currency: Currency) -> Self {
        Self { currency, minor }
    }

    /// `cents` US cents.
    pub const fn usd(cents: i64) -> Self {
        Self::new(cents, Currency::USD)
    }

    /// Nothing, in `currency`.
    pub const fn zero(currency: Currency) -> Self {
        Self::new(0, currency)
    }

    /// Whether the amount is zero.
    pub fn is_zero(&self) -> bool {
        self.minor == 0
    }

    /// Whether the amount is above zero.
    pub fn is_positive(&self) -> bool {
        self.minor > 0
    }

    /// Reads a decimal string in major units, as PayPal and ERA write amounts: `"19.00"`,
    /// `"19"`, `"-5.5"`. More decimals than the currency has are refused, never rounded.
    pub fn parse_decimal(value: &str, currency: Currency) -> Result<Self, MoneyError> {
        let bad = || MoneyError::Amount {
            currency,
            value: value.to_owned(),
        };
        let t = value.trim();
        let (neg, t) = match t.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let (whole, frac) = t.split_once('.').unwrap_or((t, ""));
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if (whole.is_empty() && frac.is_empty()) || !digits(whole) || !digits(frac) {
            return Err(bad());
        }
        // Trailing zeros past the currency's exponent are harmless ("19.0000" from ERA's numeric).
        let exp = currency.exponent() as usize;
        let frac = frac.trim_end_matches('0');
        if frac.len() > exp {
            return Err(bad());
        }
        let whole: i64 = if whole.is_empty() {
            0
        } else {
            whole.parse().map_err(|_| bad())?
        };
        let mut frac_digits = frac.to_owned();
        while frac_digits.len() < exp {
            frac_digits.push('0');
        }
        let frac: i64 = if frac_digits.is_empty() {
            0
        } else {
            frac_digits.parse().map_err(|_| bad())?
        };
        let minor = whole
            .checked_mul(10_i64.pow(exp as u32))
            .and_then(|w| w.checked_add(frac))
            .ok_or(MoneyError::Overflow)?;
        Ok(Self::new(if neg { -minor } else { minor }, currency))
    }

    /// The amount as a decimal string in major units, for PayPal and ERA: `1900` cents is
    /// `"19.00"`.
    pub fn to_decimal(&self) -> String {
        let exp = self.currency.exponent();
        let sign = if self.minor < 0 { "-" } else { "" };
        let abs = self.minor.unsigned_abs();
        if exp == 0 {
            return format!("{sign}{abs}");
        }
        let unit = 10_u64.pow(exp);
        format!(
            "{sign}{}.{:0width$}",
            abs / unit,
            abs % unit,
            width = exp as usize
        )
    }

    /// `self + other`, in the same currency.
    pub fn checked_add(self, other: Money) -> Result<Money, MoneyError> {
        self.same(other)?;
        self.minor
            .checked_add(other.minor)
            .map(|m| Money::new(m, self.currency))
            .ok_or(MoneyError::Overflow)
    }

    /// `self - other`, in the same currency.
    pub fn checked_sub(self, other: Money) -> Result<Money, MoneyError> {
        self.same(other)?;
        self.minor
            .checked_sub(other.minor)
            .map(|m| Money::new(m, self.currency))
            .ok_or(MoneyError::Overflow)
    }

    /// `self * n`.
    pub fn checked_mul(self, n: i64) -> Result<Money, MoneyError> {
        self.minor
            .checked_mul(n)
            .map(|m| Money::new(m, self.currency))
            .ok_or(MoneyError::Overflow)
    }

    /// Compares two amounts in the same currency.
    pub fn compare(&self, other: &Money) -> Result<Ordering, MoneyError> {
        self.same(*other)?;
        Ok(self.minor.cmp(&other.minor))
    }

    /// The negated amount.
    pub fn negate(self) -> Money {
        Money::new(-self.minor, self.currency)
    }

    fn same(&self, other: Money) -> Result<(), MoneyError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(MoneyError::Mismatch(self.currency, other.currency))
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.currency, self.to_decimal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_round_trip() {
        for (text, minor) in [
            ("19.00", 1900),
            ("19", 1900),
            ("0.5", 50),
            ("-5.25", -525),
            (".75", 75),
            ("190.0000", 19000),
        ] {
            assert_eq!(
                Money::parse_decimal(text, Currency::USD).unwrap().minor,
                minor,
                "{text}"
            );
        }
        assert_eq!(Money::usd(1900).to_decimal(), "19.00");
        assert_eq!(Money::usd(-5).to_decimal(), "-0.05");
        assert_eq!(Money::usd(7).to_string(), "USD 0.07");
        let jpy = Currency::new("JPY").unwrap();
        assert_eq!(
            Money::parse_decimal("1200", jpy).unwrap().to_decimal(),
            "1200"
        );
    }

    #[test]
    fn refuses_what_it_cannot_represent() {
        for bad in ["19.001", "abc", "", "1.2.3", "--1", "1e3", "-"] {
            assert!(Money::parse_decimal(bad, Currency::USD).is_err(), "{bad}");
        }
        assert!(Money::parse_decimal("1.5", Currency::new("JPY").unwrap()).is_err());
        assert!(Currency::new("usd").is_err());
        assert!(Currency::new("US").is_err());
    }

    #[test]
    fn arithmetic_stays_in_one_currency() {
        let a = Money::usd(1000);
        assert_eq!(a.checked_sub(Money::usd(250)).unwrap(), Money::usd(750));
        assert_eq!(a.checked_mul(12).unwrap(), Money::usd(12000));
        assert!(matches!(
            a.checked_add(Money::new(1, Currency::KES)),
            Err(MoneyError::Mismatch(..))
        ));
        assert_eq!(
            Money::new(i64::MAX, Currency::USD).checked_add(Money::usd(1)),
            Err(MoneyError::Overflow)
        );
    }

    #[test]
    fn json_shape_matches_agentrix_types() {
        let json = serde_json::to_value(Money::usd(1900)).unwrap();
        assert_eq!(json, serde_json::json!({"currency": "USD", "minor": 1900}));
        assert!(
            serde_json::from_value::<Money>(serde_json::json!({"currency": "usd", "minor": 1}))
                .is_err()
        );
    }
}
