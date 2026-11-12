//! Arithmetic that gives the same bits in Rust and in a browser.
//!
//! The mark is drawn from `sin`, `cos` and `pow`. Rust takes those from the platform's libm and
//! browsers from their own, and the two can differ in the last bit, which after rounding to two
//! decimals can change a coordinate. So the generator uses its own versions, built only from
//! `+ - * /`, `floor` and `sqrt`, which IEEE 754 defines exactly. `assets/marks.js` carries the
//! same functions with the same operations in the same order; the golden tests hold them equal.
//!
//! Accuracy is about 1e-15, far below the 0.01 the SVG is written at.

/// π, the same double as JavaScript's `Math.PI`.
pub const PI: f64 = std::f64::consts::PI;

// π/2 split in two (fdlibm): the first 33 bits, then the rest.
const PIO2_1: f64 = 1.570_796_326_734_125_6;
const PIO2_1T: f64 = 6.077_100_506_506_192e-11;
const INVPIO2: f64 = std::f64::consts::FRAC_2_PI;

// fdlibm's kernel polynomials on [-π/4, π/4].
const S1: f64 = -1.666_666_666_666_663_2e-1;
const S2: f64 = 8.333_333_333_322_49e-3;
const S3: f64 = -1.984_126_982_985_795e-4;
const S4: f64 = 2.755_731_370_707_006_8e-6;
const S5: f64 = -2.505_076_025_340_686_3e-8;
const S6: f64 = 1.589_690_995_211_55e-10;
const C1: f64 = 4.166_666_666_666_66e-2;
const C2: f64 = -1.388_888_888_887_411e-3;
const C3: f64 = 2.480_158_728_947_673e-5;
const C4: f64 = -2.755_731_435_139_066_3e-7;
const C5: f64 = 2.087_572_321_298_175e-9;
const C6: f64 = -1.135_964_755_778_819_5e-11;

// ln 2 split in two (fdlibm), so k·LN2_HI is exact for the k used here.
const LN2_HI: f64 = 6.931_471_803_691_238e-1;
const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
const INV_LN2: f64 = std::f64::consts::LOG2_E;
const SQRT2: f64 = std::f64::consts::SQRT_2;
const SQRT1_2: f64 = std::f64::consts::FRAC_1_SQRT_2;

fn ksin(x: f64) -> f64 {
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    x + v * (S1 + z * r)
}

fn kcos(x: f64) -> f64 {
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    1.0 - (0.5 * z - z * r)
}

/// Reduces `x` to `r` in about [-π/4, π/4] and the quadrant `n` in 0..4.
fn reduce(x: f64) -> (u8, f64) {
    let q = (x * INVPIO2 + 0.5).floor();
    let r = (x - q * PIO2_1) - q * PIO2_1T;
    let n = (((q % 4.0) + 4.0) % 4.0) as u8;
    (n, r)
}

/// The sine of `x` radians.
pub fn sin(x: f64) -> f64 {
    let (n, r) = reduce(x);
    match n {
        0 => ksin(r),
        1 => kcos(r),
        2 => -ksin(r),
        _ => -kcos(r),
    }
}

/// The cosine of `x` radians.
pub fn cos(x: f64) -> f64 {
    let (n, r) = reduce(x);
    match n {
        0 => kcos(r),
        1 => -ksin(r),
        2 => -kcos(r),
        _ => ksin(r),
    }
}

/// The natural logarithm of a positive, finite `x`.
fn ln(x: f64) -> f64 {
    let mut m = x;
    let mut k = 0.0;
    while m > SQRT2 {
        m *= 0.5;
        k += 1.0;
    }
    while m < SQRT1_2 {
        m *= 2.0;
        k -= 1.0;
    }
    // ln m = 2 atanh s, with s = (m - 1) / (m + 1) and |s| < 0.172.
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut acc = 1.0 / 25.0;
    let mut j = 11.0;
    while j >= 0.0 {
        acc = 1.0 / (2.0 * j + 1.0) + s2 * acc;
        j -= 1.0;
    }
    k * LN2_HI + (k * LN2_LO + 2.0 * s * acc)
}

/// e to the power `y`, for the moderate `y` the generator produces.
fn exp(y: f64) -> f64 {
    let k = (y * INV_LN2 + 0.5).floor();
    let r = (y - k * LN2_HI) - k * LN2_LO;
    let mut acc = 1.0;
    let mut n = 18.0;
    while n >= 1.0 {
        acc = 1.0 + acc * r / n;
        n -= 1.0;
    }
    let mut i = 0.0;
    if k > 0.0 {
        while i < k {
            acc *= 2.0;
            i += 1.0;
        }
    } else {
        while i < -k {
            acc *= 0.5;
            i += 1.0;
        }
    }
    acc
}

/// `x` to the power `y`, for `x >= 0` and `y > 0`.
pub fn pow(x: f64, y: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    if x == 1.0 {
        return 1.0;
    }
    exp(y * ln(x))
}

/// JavaScript's `Math.sign`: keeps zero (and its sign) as it is.
pub fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/// JavaScript's `Math.round`: halves go up, towards +∞.
pub fn round(x: f64) -> f64 {
    let fl = x.floor();
    if x - fl >= 0.5 { fl + 1.0 } else { fl }
}

/// A number as the SVG writes it: rounded to two decimals, shortest form, never `-0`.
/// The same text as JavaScript's `(Math.round(v * 100) / 100).toString()`.
pub fn f(v: f64) -> String {
    let r = round(v * 100.0) / 100.0;
    if r == 0.0 {
        return "0".to_string();
    }
    // Rust's `Display` for f64 prints the shortest digits that round-trip, as JavaScript does,
    // and never uses an exponent; values here stay between 0.01 and 1e6, where JavaScript
    // doesn't either.
    format!("{r}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trig_is_close_to_the_platform() {
        let mut x: f64 = -200.0;
        while x < 200.0 {
            assert!((sin(x) - x.sin()).abs() < 1e-13, "sin {x}");
            assert!((cos(x) - x.cos()).abs() < 1e-13, "cos {x}");
            x += 0.0137;
        }
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(sin(0.0), 0.0);
    }

    #[test]
    fn pow_is_close_to_the_platform() {
        for &y in &[0.25, 0.3125, 0.4, 0.4348, 0.625, 2.4] {
            let mut x: f64 = 1e-17;
            while x <= 1.0 {
                let want = x.powf(y);
                assert!(((pow(x, y) - want) / want).abs() < 1e-13, "pow {x} {y}");
                x *= 1.37;
            }
        }
        assert_eq!(pow(1.0, 0.4), 1.0);
        assert_eq!(pow(0.0, 0.4), 0.0);
    }

    #[test]
    fn marks_js_carries_the_same_constants() {
        let js = crate::assets::JS;
        let pins: [(&str, f64); 21] = [
            ("PIO2_1", PIO2_1),
            ("PIO2_1T", PIO2_1T),
            ("INVPIO2", INVPIO2),
            ("S1", S1),
            ("S2", S2),
            ("S3", S3),
            ("S4", S4),
            ("S5", S5),
            ("S6", S6),
            ("C1", C1),
            ("C2", C2),
            ("C3", C3),
            ("C4", C4),
            ("C5", C5),
            ("C6", C6),
            ("LN2_HI", LN2_HI),
            ("LN2_LO", LN2_LO),
            ("INV_LN2", INV_LN2),
            ("SQRT2", SQRT2),
            ("SQRT1_2", SQRT1_2),
            ("PI", PI),
        ];
        for (name, value) in pins {
            let line = js
                .lines()
                .find(|l| l.starts_with(&format!("const {name} = ")))
                .unwrap_or_else(|| panic!("marks.js has no {name}"));
            let text = line
                .trim_start_matches(&format!("const {name} = "))
                .trim_end_matches(';');
            if text == "Math.PI" {
                continue;
            }
            let parsed: f64 = text.parse().unwrap();
            assert_eq!(
                parsed.to_bits(),
                value.to_bits(),
                "{name}: marks.js has {text}"
            );
        }
    }

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        assert_eq!(f(50.0), "50");
        assert_eq!(f(12.345), "12.35");
        assert_eq!(f(-0.001), "0");
        assert_eq!(f(-0.0), "0");
        assert_eq!(f(-1.005), "-1");
        assert_eq!(f(0.8), "0.8");
        assert_eq!(f(-34.5), "-34.5");
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.5), 3.0);
    }
}
