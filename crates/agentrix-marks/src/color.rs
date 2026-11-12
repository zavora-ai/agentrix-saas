//! Colour and contrast for the mark (WCAG 1.4.11; 06 §4.3; ADR-031).
//!
//! Colours are `#rrggbb`, lowercase. Contrast follows WCAG's relative luminance, with the power
//! from [`crate::num`] so the console's editor and the server agree on every pass or fail.

use crate::num;

/// The glyph colour on a dark fill.
pub const ON_DARK: &str = "#ffffff";
/// The glyph colour on a light fill.
pub const ON_LIGHT: &str = "#1b1726";
/// The waiting dot's amber.
pub const WAIT: &str = "#b26b00";

/// The signature's strength by detail level: none under 28 px, 22% at 28–44 px, 30% above.
pub const SIG_ALPHA: [f64; 3] = [0.0, 0.22, 0.3];

/// Parses `#rgb` or `#rrggbb` (the `#` optional) into channels.
pub fn hex_to_rgb(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#').unwrap_or(hex);
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return None,
    };
    let n = u32::from_str_radix(&full, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// Writes channels as `#rrggbb`, rounding and clamping each as JavaScript does.
pub fn rgb_to_hex(r: f64, g: f64, b: f64) -> String {
    let c = |v: f64| num::round(v).clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// Normalises a colour to `#rrggbb`, or `None` if it isn't one.
pub fn normalize(hex: &str) -> Option<String> {
    let [r, g, b] = hex_to_rgb(hex)?;
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

fn rgb(hex: &str) -> [u8; 3] {
    hex_to_rgb(hex).unwrap_or([0, 0, 0])
}

/// Relative luminance (WCAG 2.2).
pub fn luminance(hex: &str) -> f64 {
    const W: [f64; 3] = [0.2126, 0.7152, 0.0722];
    let mut a = 0.0;
    for (i, ch) in rgb(hex).into_iter().enumerate() {
        let v = f64::from(ch) / 255.0;
        let lin = if v <= 0.03928 {
            v / 12.92
        } else {
            num::pow((v + 0.055) / 1.055, 2.4)
        };
        a += W[i] * lin;
    }
    a
}

/// The contrast ratio between two colours, from 1 to 21.
pub fn contrast(a: &str, b: &str) -> f64 {
    let la = luminance(a);
    let lb = luminance(b);
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The glyph colour for a fill: white or near-black, whichever contrasts more.
pub fn on_color(fill: &str) -> &'static str {
    if contrast(fill, ON_DARK) >= contrast(fill, ON_LIGHT) {
        ON_DARK
    } else {
        ON_LIGHT
    }
}

/// `a` moved towards `b` by `t` (0 to 1).
pub fn mix(a: &str, b: &str, t: f64) -> String {
    let x = rgb(a);
    let y = rgb(b);
    let ch = |i: usize| {
        let v = f64::from(x[i]);
        v + (f64::from(y[i]) - v) * t
    };
    rgb_to_hex(ch(0), ch(1), ch(2))
}

/// The glyph's contrast against the fill with the signature at its strongest (06 §4.3).
pub fn glyph_contrast(fill: &str) -> f64 {
    let on = on_color(fill);
    contrast(on, &mix(fill, on, SIG_ALPHA[2]))
}

/// Whether a fill keeps 3:1 against the ground and lets its glyph keep 3:1 too.
pub fn passes(fill: &str, ground: &str) -> bool {
    contrast(fill, ground) >= 3.0 && glyph_contrast(fill) >= 3.0
}

/// The nearest shade of `fill` that passes on `ground`, as the editor offers it.
pub fn nearest_passing(fill: &str, ground: &str) -> String {
    let dirs: [&str; 2] = if luminance(ground) > 0.4 {
        ["#000000", "#ffffff"]
    } else {
        ["#ffffff", "#000000"]
    };
    for towards in dirs {
        let mut t = 0.0;
        while t <= 1.0001 {
            let c = mix(fill, towards, t);
            if passes(&c, ground) {
                return c;
            }
            t += 0.02;
        }
    }
    dirs[0].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_follows_wcag() {
        assert!((contrast("#000000", "#ffffff") - 21.0).abs() < 1e-9);
        assert!((contrast("#777777", "#777777") - 1.0).abs() < 1e-12);
        assert_eq!(on_color("#8e3b46"), ON_DARK);
        assert_eq!(on_color("#f6e27a"), ON_LIGHT);
        assert_eq!(normalize("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(normalize("#12345"), None);
    }

    #[test]
    fn nearest_passing_passes() {
        let fixed = nearest_passing("#f2d7a8", "#fbf6ef");
        assert!(passes(&fixed, "#fbf6ef"), "{fixed}");
        assert!(passes("#8e3b46", "#fbf6ef"));
    }
}
