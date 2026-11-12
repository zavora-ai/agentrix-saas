//! Templates, in minijinja's strict mode like the owner console: a value a template expects but
//! the view lacks is an error, never a blank on screen (the honesty rule).

use minijinja::{AutoEscape, Environment, Output, State, UndefinedBehavior, Value};
use serde::Serialize;

use chrono::{DateTime, FixedOffset, Utc};

use crate::assets::ASSETS_VERSION;

/// Every template, so a broken one fails at startup rather than on first visit.
pub const TEMPLATES: [&str; 18] = [
    "cli.html",
    "holds.html",
    "overview.html",
    "effect.html",
    "owner_agency.html",
    "partners.html",
    "portfolio.html",
    "rollouts.html",
    "incident.html",
    "incidents.html",
    "money.html",
    "owner_grant.html",
    "signin.html",
    "fleet.html",
    "customers.html",
    "site.html",
    "trust.html",
    "app.html",
];

/// HTML escaping without minijinja's `/` → `&#x2f;` (URLs stay readable in the source).
fn format(out: &mut Output, state: &State, value: &Value) -> Result<(), minijinja::Error> {
    match (state.auto_escape(), value.as_str()) {
        (AutoEscape::Html, Some(text)) if !value.is_safe() => {
            let mut done = 0;
            for (i, byte) in text.bytes().enumerate() {
                let entity = match byte {
                    b'&' => "&amp;",
                    b'<' => "&lt;",
                    b'>' => "&gt;",
                    b'"' => "&quot;",
                    b'\'' => "&#x27;",
                    _ => continue,
                };
                out.write_str(&text[done..i])?;
                out.write_str(entity)?;
                done = i + 1;
            }
            Ok(out.write_str(&text[done..])?)
        }
        _ => minijinja::escape_formatter(out, state, value),
    }
}

/// The loaded templates.
pub struct Render {
    env: Environment<'static>,
}

impl Render {
    /// Loads and parses every template. Times render in the zone `offset_minutes` east of UTC.
    pub fn new(offset_minutes: i32) -> Result<Self, minijinja::Error> {
        let mut env = Environment::new();
        filters(&mut env, offset_minutes);
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        env.set_formatter(format);
        #[cfg(feature = "dev-reload")]
        env.set_loader(minijinja::path_loader(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates"
        )));
        #[cfg(not(feature = "dev-reload"))]
        minijinja_embed::load_templates!(&mut env);
        env.add_global("asset_v", ASSETS_VERSION);
        agentrix_ui::install(&mut env)?;
        for name in TEMPLATES {
            env.get_template(name)?;
        }
        Ok(Self { env })
    }

    /// Renders `name` with `view`.
    pub fn page(&self, name: &str, view: &impl Serialize) -> Result<String, minijinja::Error> {
        self.env.get_template(name)?.render(view)
    }
}

fn zone(offset_minutes: i32) -> FixedOffset {
    FixedOffset::east_opt(offset_minutes * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).expect("UTC"))
}

/// Dollars from micro-USD: cents under $100, whole dollars above, with thousands separators.
pub fn usd(micro: i64) -> String {
    let dollars = micro as f64 / 1_000_000.0;
    if dollars.abs() >= 100.0 {
        format!("${}", thousands(dollars.round() as i64))
    } else if micro > 0 && dollars < 0.01 {
        "under $0.01".to_owned()
    } else {
        format!("${dollars:.2}")
    }
}

/// 1904 → "1,904".
pub fn thousands(n: i64) -> String {
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

/// Seconds as "0.8 s", "4.6 s", "8 m 40 s", "2 h 5 m".
pub fn duration(secs: f64) -> String {
    if secs < 10.0 {
        format!("{secs:.1} s")
    } else if secs < 60.0 {
        format!("{} s", secs.round() as i64)
    } else if secs < 3600.0 {
        let s = secs.round() as i64;
        format!("{} m {} s", s / 60, s % 60)
    } else {
        let m = (secs / 60.0).round() as i64;
        format!("{} h {} m", m / 60, m % 60)
    }
}

fn parse(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn filters(env: &mut Environment<'static>, offset_minutes: i32) {
    let tz = zone(offset_minutes);
    env.add_filter("usd", |micro: i64| usd(micro));
    env.add_filter("num", |n: i64| thousands(n));
    // `{{ kept|field("title") }}`: what the person typed before a form came back, or "".
    env.add_filter("field", |kept: Value, name: String| {
        kept.get_attr(&name)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    });
    // `{{ n|plural("hour", "hours") }}` → "1 hour", "4 hours".
    env.add_filter("plural", |n: i64, one: String, many: String| {
        format!("{} {}", thousands(n), if n == 1 { one } else { many })
    });
    env.add_filter("dur", |secs: f64| duration(secs));
    env.add_filter("pct", |part: i64, whole: i64| {
        if whole <= 0 {
            0
        } else {
            ((part as f64 / whole as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as i64
        }
    });
    env.add_filter("when", move |text: String| {
        parse(&text)
            .map(|t| t.with_timezone(&tz).format("%a %-d %b, %H:%M").to_string())
            .unwrap_or(text)
    });
    env.add_filter("day", move |text: String| {
        parse(&text)
            .map(|t| t.with_timezone(&tz).format("%a %-d %b").to_string())
            .unwrap_or(text)
    });
    env.add_filter("clock", move |text: String| {
        parse(&text)
            .map(|t| t.with_timezone(&tz).format("%H:%M").to_string())
            .unwrap_or(text)
    });
    env.add_filter("since", |to: String, from: String| {
        match (parse(&to), parse(&from)) {
            (Some(t), Some(f)) => duration(((t - f).num_milliseconds() as f64 / 1000.0).max(0.0)),
            _ => to,
        }
    });
    env.add_filter("ago", |text: String| {
        let Some(t) = parse(&text) else { return text };
        let s = (Utc::now() - t).num_seconds().max(0);
        match s {
            0..=59 => "just now".to_owned(),
            60..=3599 => format!("{} min ago", s / 60),
            3600..=86_399 => format!("{} h ago", s / 3600),
            _ => format!("{} d ago", s / 86_400),
        }
    });
    env.add_filter("initials", |name: String| {
        let letters: String = name
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect();
        if letters.is_empty() {
            "·".to_owned()
        } else {
            letters.to_uppercase()
        }
    });
    env.add_filter("digest", |text: String| {
        let hex = text.strip_prefix("sha256:").unwrap_or(&text);
        if hex.len() > 10 {
            format!("{}…{}", &hex[..4], &hex[hex.len() - 3..])
        } else {
            hex.to_owned()
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_durations_read_well() {
        assert_eq!(usd(1_904_000_000), "$1,904");
        assert_eq!(usd(420_000), "$0.42");
        assert_eq!(usd(0), "$0.00");
        assert_eq!(usd(3), "under $0.01");
        assert_eq!(thousands(612), "612");
        assert_eq!(thousands(30260), "30,260");
        assert_eq!(duration(0.84), "0.8 s");
        assert_eq!(duration(520.0), "8 m 40 s");
        assert_eq!(duration(7500.0), "2 h 5 m");
    }

    #[test]
    fn every_template_loads() {
        Render::new(180).unwrap();
    }
}
