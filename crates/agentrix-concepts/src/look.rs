//! The chosen design's look as the Move's rebuild uses it: colours and fonts read from the
//! design's own CSS (its `:root` custom properties and Google Fonts), in the shape of
//! `agentrix_move::theme::Tokens`.

use serde::Serialize;

/// Colours and font stacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Look {
    /// Page background.
    pub bg: String,
    /// A softer background for panels.
    pub soft: String,
    /// Body text.
    pub ink: String,
    /// Secondary text.
    pub ink2: String,
    /// Rules and borders.
    pub line: String,
    /// The brand colour.
    pub accent: String,
    /// Text on the brand colour.
    pub on_accent: String,
    /// The display font stack.
    pub display: String,
    /// The text font stack.
    pub text: String,
    /// The Google Fonts families the design uses, for a stylesheet link.
    pub google_families: Vec<String>,
}

fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim().trim_start_matches('#');
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_owned(),
        _ => return None,
    };
    let v = u32::from_str_radix(&full, 16).ok()?;
    Some((
        f64::from((v >> 16) & 0xff) / 255.0,
        f64::from((v >> 8) & 0xff) / 255.0,
        f64::from(v & 0xff) / 255.0,
    ))
}

/// Relative luminance (WCAG).
fn lum(hex: &str) -> f64 {
    let lin = |c: f64| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    rgb(hex).map_or(0.5, |(r, g, b)| {
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    })
}

/// WCAG contrast ratio.
fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn saturation(hex: &str) -> f64 {
    rgb(hex).map_or(0.0, |(r, g, b)| {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        max - min
    })
}

/// `#rrggbb` halfway between two colours, weighted `w` towards `b`.
fn mix(a: &str, b: &str, w: f64) -> String {
    match (rgb(a), rgb(b)) {
        (Some((ar, ag, ab)), Some((br, bg, bb))) => {
            let c = |x: f64, y: f64| ((x + (y - x) * w) * 255.0).round().clamp(0.0, 255.0) as u8;
            format!("#{:02x}{:02x}{:02x}", c(ar, br), c(ag, bg), c(ab, bb))
        }
        _ => a.to_owned(),
    }
}

/// The custom properties holding hex colours, in order: `(name, #hex)`.
fn colour_vars(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(p) = rest.find("--") {
        rest = &rest[p + 2..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let after = rest[name.len()..].trim_start();
        if let Some(v) = after.strip_prefix(':') {
            let v = v.trim_start();
            if let Some(hex) = v.strip_prefix('#') {
                let digits: String = hex.chars().take_while(char::is_ascii_hexdigit).collect();
                if matches!(digits.len(), 3 | 6) {
                    out.push((
                        name.to_ascii_lowercase(),
                        format!("#{}", digits.to_ascii_lowercase()),
                    ));
                }
            }
        }
    }
    out
}

/// The Google Fonts families linked or imported, in order.
fn google_families(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in html.split("family=").skip(1) {
        let fam: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == ' ')
            .collect();
        let fam = fam.replace('+', " ").trim().to_owned();
        if !fam.is_empty() && !out.contains(&fam) {
            out.push(fam);
        }
    }
    out
}

fn serif(family: &str) -> bool {
    let f = family.to_ascii_lowercase();
    [
        "serif",
        "fraunces",
        "playfair",
        "lora",
        "merriweather",
        "garamond",
        "libre baskerville",
        "cormorant",
        "dm serif",
        "crimson",
        "georgia",
    ]
    .iter()
    .any(|s| f.contains(s))
        && !f.contains("sans")
}

fn stack(family: Option<&String>, fallback: &str) -> String {
    match family {
        Some(f) if serif(f) => format!("'{f}', Georgia, 'Times New Roman', serif"),
        Some(f) => format!("'{f}', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif"),
        None => fallback.to_owned(),
    }
}

/// Reads a design's look; `None` when its CSS names too few colours to tell.
pub fn from_html(html: &str) -> Option<Look> {
    let vars = colour_vars(html);
    // Every variable a key names, best first: an exact name, then a name starting with the
    // key, then one containing it (so `--ink` beats `--rust-dark` for "ink").
    let find_all = |keys: &[&str]| {
        let mut found: Vec<String> = Vec::new();
        for tier in 0..3 {
            for k in keys {
                for (n, v) in &vars {
                    let hit = match tier {
                        0 => n == k,
                        1 => n.starts_with(k),
                        _ => n.contains(k),
                    };
                    if hit && !found.contains(v) {
                        found.push(v.clone());
                    }
                }
            }
        }
        found
    };
    let find = |keys: &[&str]| find_all(keys).into_iter().next();
    let mut colours: Vec<String> = vars.iter().map(|(_, v)| v.clone()).collect();
    colours.dedup();
    if colours.len() < 2 {
        return None;
    }
    let lightest = colours
        .iter()
        .max_by(|a, b| lum(a).total_cmp(&lum(b)))
        .cloned()?;
    let darkest = colours
        .iter()
        .min_by(|a, b| lum(a).total_cmp(&lum(b)))
        .cloned()?;
    let bg = find(&["bg", "background", "paper", "cream", "canvas", "surface"])
        .filter(|c| lum(c) > 0.6)
        .unwrap_or(lightest);
    let ink = find_all(&["ink", "text", "fg", "foreground", "body", "char", "dark"])
        .into_iter()
        .find(|c| contrast(c, &bg) >= 7.0)
        .unwrap_or(darkest);
    // The brand colour: a named one that reads on the background, else the most saturated
    // colour that does, else the most saturated (the Move's Tokens::chosen checks again).
    let readable = |c: &String| *c != bg && *c != ink && contrast(c, &bg) >= 4.5;
    let accent = find_all(&["accent", "primary", "brand", "crust", "pop", "highlight"])
        .into_iter()
        .find(|c| readable(c))
        .or_else(|| {
            colours
                .iter()
                .filter(|c| readable(c) && saturation(c) > 0.15)
                .max_by(|a, b| saturation(a).total_cmp(&saturation(b)))
                .cloned()
        })
        .or_else(|| {
            colours
                .iter()
                .filter(|c| **c != bg && **c != ink)
                .max_by(|a, b| saturation(a).total_cmp(&saturation(b)))
                .cloned()
        })
        .unwrap_or_else(|| ink.clone());
    let families = google_families(html);
    let display_family = families.first();
    let text_family = families.get(1).or(display_family);
    Some(Look {
        soft: mix(&bg, &accent, 0.08),
        ink2: mix(&ink, &bg, 0.3),
        line: mix(&bg, &ink, 0.15),
        on_accent: if contrast(&accent, "#ffffff") < 4.5 {
            "#111111".into()
        } else {
            "#ffffff".into()
        },
        display: stack(display_family, "Georgia, 'Times New Roman', serif"),
        text: stack(
            text_family,
            "system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif",
        ),
        google_families: families,
        bg,
        ink,
        accent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_design_gives_its_colours_and_fonts() {
        let html = r#"<link href="https://fonts.googleapis.com/css2?family=Fraunces:wght@600&family=Nunito+Sans&display=swap" rel="stylesheet">
<style>:root{--rust:#B5501F;--rust-dark:#8F3D14;--gold:#E8B84B;--cream:#F7EDDD;--ink:#3B2A1E}</style>"#;
        let look = from_html(html).expect("a look");
        assert_eq!(look.bg, "#f7eddd");
        assert_eq!(look.ink, "#3b2a1e", "--ink, not --rust-dark");
        assert_eq!(
            look.accent, "#8f3d14",
            "the saturated colour that reads on cream"
        );
        assert_eq!(look.on_accent, "#ffffff");
        assert_eq!(
            look.display,
            "'Fraunces', Georgia, 'Times New Roman', serif"
        );
        assert!(look.text.starts_with("'Nunito Sans', system-ui"));
        assert!(from_html("<p>no styles</p>").is_none());
    }
}

#[cfg(test)]
mod real_designs {
    /// Prints the look read from each saved design (`CONCEPTS_DEMO_DIR`), for checking by eye:
    /// `cargo test -p agentrix-concepts looks_of_saved_designs -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads saved designs from CONCEPTS_DEMO_DIR"]
    fn looks_of_saved_designs() {
        let dir = std::env::var("CONCEPTS_DEMO_DIR").expect("CONCEPTS_DEMO_DIR");
        for n in 1..=3 {
            let html = std::fs::read_to_string(format!("{dir}/concept-{n}.html")).expect("design");
            println!("concept-{n}: {:?}", super::from_html(&html));
        }
    }
}
