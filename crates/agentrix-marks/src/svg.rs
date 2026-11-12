//! The generator: a [`MarkSpec`] and a size to static SVG, byte for byte what `assets/marks.js`
//! draws. Ported from `concept/identity/marks.html` with two changes: ids are passed in (no
//! global counter), and the drift distance is a `data-p` attribute that `marks.js` copies into
//! a CSS variable, so the SVG carries no inline style and a strict CSP holds.

use crate::color::{self, SIG_ALPHA, WAIT};
use crate::glyphs;
use crate::num::{self, PI, cos, f, pow, sign, sin};
use crate::rng::{Rng, seed_from};
use crate::spec::{Family, Glyph, MarkSpec};
use crate::state::MarkState;

const VB: &str = r#"viewBox="-12 -12 124 134""#;

/// How to draw one mark.
#[derive(Debug, Clone, Copy)]
pub struct Render<'a> {
    /// The mark's size in CSS pixels (the squircle's side). The SVG is 1.24× wide and 1.34×
    /// tall to leave room for the halo, the trace line and the waiting dot.
    pub size: f64,
    /// The state it starts in. Presence frames change it later.
    pub state: MarkState,
    /// The site's display face, for monograms, as a CSS `font-family` value.
    pub font: &'a str,
    /// Makes the SVG's internal ids unique on the page. Two marks on one page need different
    /// values (an id inside a hidden copy would blank the visible one).
    pub uid: &'a str,
}

impl Default for Render<'_> {
    fn default() -> Self {
        Render {
            size: 40.0,
            state: MarkState::Idle,
            font: "",
            uid: "1",
        }
    }
}

/// Escapes text for an attribute or element body.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn dims(size: f64) -> String {
    format!(r#"width="{}" height="{}""#, f(size * 1.24), f(size * 1.34))
}

/// The squircle's outline, 96 points around (cx, cy) with radius r.
pub fn squircle(n: f64, r: f64) -> String {
    const STEPS: f64 = 96.0;
    let (cx, cy) = (50.0, 50.0);
    let mut d = String::new();
    let mut i = 0.0;
    while i < STEPS {
        let t = (i / STEPS) * PI * 2.0;
        let c = cos(t);
        let s = sin(t);
        let x = cx + r * sign(c) * pow(c.abs(), 2.0 / n);
        let y = cy + r * sign(s) * pow(s.abs(), 2.0 / n);
        d.push_str(if i == 0.0 { "M" } else { "L" });
        d.push_str(&f(x));
        d.push(' ');
        d.push_str(&f(y));
        i += 1.0;
    }
    d.push('Z');
    d
}

fn arc(r: f64, a0: f64, a1: f64, color: &str, w: f64, op: f64) -> String {
    let p = |a: f64| {
        (
            50.0 + r * cos(a * PI / 180.0),
            50.0 + r * sin(a * PI / 180.0),
        )
    };
    let (x0, y0) = p(a0);
    let (x1, y1) = p(a1);
    let large = if (a1 - a0) > 180.0 { 1 } else { 0 };
    format!(
        r#"<path d="M{} {}A{} {} 0 {large} 1 {} {}" fill="none" stroke="{color}" stroke-width="{}" stroke-linecap="round" opacity="{}"/>"#,
        f(x0),
        f(y0),
        f(r),
        f(r),
        f(x1),
        f(y1),
        f(w),
        f(op)
    )
}

fn wave(x0: f64, x1: f64, y0: f64, amp: f64, lam: f64, ph: f64) -> String {
    let mut d = String::new();
    let mut x = x0;
    let mut first = true;
    while x <= x1 {
        d.push_str(if first { "M" } else { "L" });
        d.push_str(&f(x));
        d.push(' ');
        d.push_str(&f(y0 + amp * sin((x + ph) / lam * PI * 2.0)));
        first = false;
        x += 2.5;
    }
    d
}

/// A family's pattern: the SVG inside the signature group, and how it moves.
struct Pattern {
    inner: String,
    motion: &'static str,
}

fn orbits(r: &mut Rng, c: &str) -> Pattern {
    let k = 3.0 + (r.float() * 2.0).floor();
    let mut s = String::new();
    let mut i = 0.0;
    while i < k {
        let rad = 16.0 + i * (34.0 / (k - 1.0)) + (r.float() - 0.5) * 3.0;
        let sweep = 150.0 + r.float() * 170.0;
        let start = r.float() * 360.0;
        let w = 3.2 + r.float() * 2.6;
        s.push_str(&arc(rad, start, start + sweep, c, w, 0.8));
        i += 1.0;
    }
    let ma = r.float() * PI * 2.0;
    let cx = f(50.0 + 31.7 * cos(ma));
    let cy = f(50.0 + 31.7 * sin(ma));
    let rr = f(4.0 + r.float() * 2.0);
    s.push_str(&format!(
        r#"<circle cx="{cx}" cy="{cy}" r="{rr}" fill="{c}"/>"#
    ));
    Pattern {
        inner: s,
        motion: "spin",
    }
}

fn grain(r: &mut Rng, c: &str) -> Pattern {
    let n = 34.0 + (r.float() * 24.0).floor();
    let sc = 5.6 + r.float() * 1.6;
    let rot = r.float() * 360.0;
    let ox = (r.float() - 0.5) * 14.0;
    let oy = (r.float() - 0.5) * 14.0;
    let mut s = String::new();
    let mut i = 1.0;
    while i <= n {
        let a = (i * 137.508 + rot) * PI / 180.0;
        let d = sc * f64::sqrt(i);
        s.push_str(&format!(
            r#"<circle cx="{}" cy="{}" r="{}"/>"#,
            f(50.0 + ox + d * cos(a)),
            f(50.0 + oy + d * sin(a)),
            f(1.1 + (i / n) * 2.6)
        ));
        i += 1.0;
    }
    Pattern {
        inner: format!(r#"<g fill="{c}">{s}</g>"#),
        motion: "spin",
    }
}

fn strata(r: &mut Rng, c: &str) -> Pattern {
    let bands = 4.0 + (r.float() * 3.0).floor();
    let lam = 34.0 + r.float() * 30.0;
    let amp = 3.0 + r.float() * 6.0;
    let ang = -30.0 + r.float() * 60.0;
    let gap = 150.0 / bands;
    let mut s = String::new();
    let mut i = 0.0;
    while i < bands {
        let y0 = -25.0 + gap * (i + 0.5);
        let ph = r.float() * lam;
        let d = wave(-50.0 - lam, 150.0 + lam, y0, amp, lam, ph);
        let w = f(gap * (0.3 + r.float() * 0.22));
        let op = if i % 2.0 != 0.0 { "1" } else { "0.6" };
        s.push_str(&format!(
            r#"<path d="{d}" fill="none" stroke="{c}" stroke-width="{w}" stroke-linecap="round" opacity="{op}"/>"#
        ));
        i += 1.0;
    }
    Pattern {
        inner: format!(
            r#"<g transform="rotate({} 50 50)"><g class="wa-drift" data-p="{}">{s}</g></g>"#,
            f(ang),
            f(-lam)
        ),
        motion: "drift",
    }
}

fn weave(r: &mut Rng, c: &str) -> Pattern {
    let sp = 9.0 + r.float() * 6.0;
    let a1 = r.float() * 180.0;
    let a2 = a1 + 55.0 + r.float() * 50.0;
    let sw = 1.6 + r.float() * 1.4;
    let set = |ang: f64| {
        let mut s = String::new();
        let mut x = -90.0;
        while x <= 190.0 {
            let fx = f(x);
            s.push_str(&format!(r#"<line x1="{fx}" y1="-70" x2="{fx}" y2="170"/>"#));
            x += sp;
        }
        format!(
            r#"<g transform="rotate({} 50 50)"><g class="wa-drift" data-p="{}">{s}</g></g>"#,
            f(ang),
            f(-sp)
        )
    };
    Pattern {
        inner: format!(
            r#"<g stroke="{c}" stroke-width="{}">{}{}</g>"#,
            f(sw),
            set(a1),
            set(a2)
        ),
        motion: "drift",
    }
}

fn facets(r: &mut Rng, c: &str) -> Pattern {
    let px = 50.0 + (r.float() - 0.5) * 36.0;
    let py = 50.0 + (r.float() - 0.5) * 36.0;
    let k = 7.0 + (r.float() * 5.0).floor();
    let mut angs = Vec::new();
    let mut a = r.float() * 360.0;
    let mut i = 0.0;
    while i < k {
        angs.push(a);
        a += (360.0 / k) * (0.6 + r.float() * 0.8);
        i += 1.0;
    }
    let mut tri = String::new();
    let mut lines = String::new();
    let count = angs.len();
    for idx in 0..count {
        let b1 = angs[idx] * PI / 180.0;
        let b2 = angs[(idx + 1) % count] * PI / 180.0;
        let big_r = 92.0;
        let p1 = (50.0 + big_r * cos(b1), 50.0 + big_r * sin(b1));
        let p2 = (50.0 + big_r * cos(b2), 50.0 + big_r * sin(b2));
        if idx % 2 == 0 {
            tri.push_str(&format!(
                r#"<path d="M{} {}L{} {}L{} {}Z" fill="{c}" fill-opacity="{}"/>"#,
                f(px),
                f(py),
                f(p1.0),
                f(p1.1),
                f(p2.0),
                f(p2.1),
                f(0.4 + r.float() * 0.45)
            ));
        }
        lines.push_str(&format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
            f(px),
            f(py),
            f(p1.0),
            f(p1.1)
        ));
    }
    Pattern {
        inner: format!(r#"{tri}<g stroke="{c}" stroke-width="1.4">{lines}</g>"#),
        motion: "spin",
    }
}

fn pattern(family: Family, rng: &mut Rng, c: &str) -> Pattern {
    match family {
        Family::Orbits => orbits(rng, c),
        Family::Grain => grain(rng, c),
        Family::Strata => strata(rng, c),
        Family::Weave => weave(rng, c),
        Family::Facets => facets(rng, c),
    }
}

/// How much detail a size gets: 0 up to 24 px (no signature), 1 up to 44 px, 2 above.
pub fn detail_for(size: f64) -> usize {
    if size <= 24.0 {
        0
    } else if size <= 44.0 {
        1
    } else {
        2
    }
}

/// The glyph layer, drawn in `color`.
fn glyph_layer(spec: &MarkSpec, detail: usize, font: &str, color: &str) -> String {
    match &spec.glyph {
        Glyph::Monogram { letters, .. } => {
            let fs = if letters.encode_utf16().count() > 1 {
                "40"
            } else if detail == 0 {
                "60"
            } else {
                "52"
            };
            format!(
                r#"<text x="50" y="52" text-anchor="middle" dominant-baseline="central" font-family="{}" font-size="{fs}" fill="{color}">{}</text>"#,
                esc(font),
                esc(letters)
            )
        }
        Glyph::Role { id } => {
            let (gs, gx, sw) = match detail {
                0 => ("60", "20", "2.7"),
                1 => ("50", "25", "2.3"),
                _ => ("44", "28", "2"),
            };
            format!(
                r#"<svg x="{gx}" y="{gx}" width="{gs}" height="{gs}" viewBox="0 0 24 24" fill="none" stroke="{color}" stroke-width="{sw}" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
                glyphs::glyph_svg(id).unwrap_or("")
            )
        }
    }
}

/// One WebAgent's mark as static SVG.
pub fn render(spec: &MarkSpec, r: &Render<'_>) -> String {
    let id = format!("wm{}", r.uid);
    let detail = detail_for(r.size);
    let fill_hex = spec.color.fill.as_str();
    let on = color::on_color(fill_hex);
    let n = spec.silhouette.squareness;
    let body = squircle(n, 50.0);
    let halo = squircle(n, 58.0);
    let (sig, motion) = if detail > 0 {
        let mut rng = Rng::for_key(&spec.seed_key());
        let p = pattern(spec.signature.family, &mut rng, on);
        (
            format!(r#"<g opacity="{}">{}</g>"#, f(SIG_ALPHA[detail]), p.inner),
            p.motion,
        )
    } else {
        (String::new(), "")
    };
    let fill = if detail == 2 {
        format!("url(#{id}g)")
    } else {
        fill_hex.to_string()
    };
    let mut s = String::with_capacity(8 * 1024);
    s.push_str(&format!(
        r#"<svg class="wa" data-state="{}" data-motion="{motion}" {} {VB} aria-hidden="true" focusable="false">"#,
        r.state.key(),
        dims(r.size)
    ));
    s.push_str(&format!(
        r##"<defs><clipPath id="{id}c"><path d="{body}"/></clipPath><linearGradient id="{id}g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{fill_hex}"/></linearGradient></defs>"##,
        color::mix(fill_hex, "#ffffff", 0.16)
    ));
    s.push_str(&format!(
        r#"<path class="wa-halo" d="{halo}" fill="none" stroke="{fill_hex}" stroke-width="4"/>"#
    ));
    s.push_str(&format!(
        r#"<g class="wa-core"><path d="{body}" fill="{fill}"/>"#
    ));
    if !sig.is_empty() {
        s.push_str(&format!(
            r#"<g clip-path="url(#{id}c)"><g class="wa-sig"><circle cx="50" cy="50" r="80" fill="none"/>{sig}</g></g>"#
        ));
    }
    s.push_str(&glyph_layer(spec, detail, r.font, on));
    s.push_str("</g>");
    s.push_str(&format!(
        r#"<line class="wa-trace" x1="8" y1="117" x2="92" y2="117" stroke="{fill_hex}" stroke-width="4" stroke-linecap="round"/>"#
    ));
    s.push_str(&format!(
        r#"<circle class="wa-dot" cx="96" cy="4" r="11" fill="{WAIT}" stroke="var(--wa-ground, #ffffff)" stroke-width="5"/>"#
    ));
    s.push_str("</svg>");
    s
}

/// One member of a site's team, as a dot on the Chief WebAgent's mark.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TeamMember {
    /// The WebAgent's id; `marks.js` finds its dot by it.
    pub webagent: String,
    /// Its fill, `#rrggbb`.
    pub fill: String,
}

/// The Chief WebAgent's mark: the site's ink, its monogram, and a dot per WebAgent.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamMark {
    /// The site's id.
    pub site: String,
    /// The site's ink, `#rrggbb`.
    pub ink: String,
    /// The site's squareness.
    pub squareness: f64,
    /// The site's monogram: its name's first letter, or the owner's choice of one or two.
    pub letters: String,
    /// One dot each, clockwise from the top.
    pub members: Vec<TeamMember>,
}

/// The Chief WebAgent's team mark as static SVG. `busy` lists the members whose dots pulse.
/// The state is the Chief's own: `waiting` while a delivered card waits in Needs you.
pub fn render_team(
    team: &TeamMark,
    size: f64,
    state: MarkState,
    busy: &[&str],
    font: &str,
) -> String {
    let ink = team.ink.as_str();
    let on = color::on_color(ink);
    let body = squircle(team.squareness, 50.0);
    let halo = squircle(team.squareness, 58.0);
    let k = team.members.len() as f64;
    let mut dots = format!(
        r#"<circle cx="50" cy="50" r="33" fill="none" stroke="{on}" stroke-width="1.5" opacity=".25"/>"#
    );
    let mut i = 0.0;
    for m in &team.members {
        let ang = (-90.0 + i * 360.0 / k) * PI / 180.0;
        let is_busy = busy.contains(&m.webagent.as_str());
        dots.push_str(&format!(
            r#"<circle class="chief-dot{}" data-webagent="{}" cx="{}" cy="{}" r="7" fill="{}" stroke="{on}" stroke-width="2.4"/>"#,
            if is_busy { " busy" } else { "" },
            esc(&m.webagent),
            f(50.0 + 33.0 * cos(ang)),
            f(50.0 + 33.0 * sin(ang)),
            esc(&m.fill)
        ));
        i += 1.0;
    }
    format!(
        concat!(
            r#"<svg class="wa chief" data-state="{state}" {dims} {vb} aria-hidden="true" focusable="false">"#,
            r#"<path class="wa-halo" d="{halo}" fill="none" stroke="{ink}" stroke-width="4"/>"#,
            r#"<g class="wa-core"><path d="{body}" fill="{ink}"/>{dots}"#,
            r#"<text x="50" y="52" text-anchor="middle" dominant-baseline="central" font-family="{font}" font-size="34" fill="{on}">{letters}</text></g>"#,
            r#"<circle class="wa-dot" cx="96" cy="4" r="11" fill="{wait}" stroke="var(--wa-ground, #ffffff)" stroke-width="5"/></svg>"#
        ),
        state = state.key(),
        dims = dims(size),
        vb = VB,
        halo = halo,
        ink = esc(ink),
        body = body,
        dots = dots,
        font = esc(font),
        on = on,
        letters = esc(&team.letters),
        wait = WAIT,
    )
}

/// A stable uid for a mark when the caller has none: the same spec and size give the same id.
/// Use it only where each mark appears once on the page.
pub fn content_uid(spec: &MarkSpec, size: f64) -> String {
    format!(
        "{:08x}",
        seed_from(&format!(
            "{}|{}|{}|{}",
            spec.seed_key(),
            spec.color.fill,
            num::f(spec.silhouette.squareness),
            num::f(size)
        ))
    )
}
