//! A site's look, as its WebAgents' marks wear it (ADR-031: "dressed in the site's own brand").
//!
//! The three samples take theirs from `concept/identity/marks.html`, so their marks are the
//! concept's, seed for seed. A moved site takes its colours and display face from the tokens the
//! Move extracted (`--wa-*` in its signed pages), its pattern family from its id, and the
//! default corners until the Move reads the site's radius (01a §5.1, `--wa-radius`).

use std::collections::BTreeMap;

use agentrix_marks::{Family, color, rng::seed_from};
use serde::{Deserialize, Serialize};

/// How a site's marks look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiteLook {
    /// The brand key the marks' seeds use: the sample's slug (`kilele`) or the site's id.
    pub key: String,
    /// The business name, for the Chief's monogram.
    pub name: String,
    /// Who approves, in visitors' words: "Wanjiru", or the business name.
    pub owner: String,
    /// The site's background: marks keep 3:1 against it.
    pub bg: String,
    /// The site's ink: the Chief WebAgent's team mark.
    pub ink: String,
    /// The site's palette, for the Workshop's swatches.
    pub palette: Vec<String>,
    /// The display face, as a CSS `font-family` value (monograms).
    pub font: String,
    /// Corner squareness, from 3.2 to 8.
    pub squareness: f64,
    /// The site's signature family: one per site, so its WebAgents read as one team.
    pub family: Family,
    /// Whether this is a sample site (everything on it says "Sample").
    pub sample: bool,
    /// Per WebAgent or desk: a fill and a seed the brand chose (the concept's, for samples).
    #[serde(default)]
    pub members: BTreeMap<String, MemberLook>,
}

/// One WebAgent's or desk's chosen fill and seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberLook {
    /// `#rrggbb`.
    pub fill: String,
    /// The signature's seed.
    pub seed: u32,
}

/// The default squareness until the Move reads the site's corner radius.
pub const DEFAULT_SQUARENESS: f64 = 4.6;

/// Each role's fill when the site's brand doesn't choose one (from the concept's teams).
pub fn role_fill(webagent: &str) -> &'static str {
    match webagent {
        "concierge" => "#38356d",
        "sales" => "#9c5725",
        "editor" => "#577035",
        "reputation" => "#7f6236",
        "cashier" => "#11734a",
        "guardian" => "#56636f",
        "caretaker" => "#3d5a1f",
        "mover" => "#2f55c4",
        _ => "#4a4960",
    }
}

impl SiteLook {
    /// A fill for `member` that keeps the mark and its glyph at 3:1 on this site.
    pub fn fill_for(&self, member: &str, fallback: &str) -> String {
        let want = self
            .members
            .get(member)
            .map(|m| m.fill.clone())
            .unwrap_or_else(|| fallback.to_string());
        let want = color::normalize(&want).unwrap_or_else(|| "#4a4960".to_string());
        if color::passes(&want, &self.bg) {
            want
        } else {
            color::nearest_passing(&want, &self.bg)
        }
    }

    /// The seed for `member`: the brand's choice, or one derived from the site and the member.
    pub fn seed_for(&self, member: &str) -> u32 {
        self.members
            .get(member)
            .map(|m| m.seed)
            .unwrap_or_else(|| 1 + seed_from(&format!("{}:{member}", self.key)) % 9999)
    }

    /// The Chief's monogram: the business name's first letter.
    pub fn monogram(&self) -> String {
        self.name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_else(|| "A".to_string())
    }

    /// The look of a moved site, from its extracted tokens.
    pub fn from_tokens(site_id: &str, name: &str, tokens: &Tokens) -> SiteLook {
        let bg = color::normalize(&tokens.bg).unwrap_or_else(|| "#ffffff".into());
        let ink = color::normalize(&tokens.ink).unwrap_or_else(|| "#23223a".into());
        let mut palette = vec![ink.clone()];
        if let Some(a) = color::normalize(&tokens.accent) {
            if !palette.contains(&a) {
                palette.push(a);
            }
        }
        for c in [
            "#8e3b46", "#9c5725", "#577035", "#2f55c4", "#56636f", "#7f6236",
        ] {
            palette.push(c.to_string());
        }
        let mut members = BTreeMap::new();
        if let Some(a) = color::normalize(&tokens.accent) {
            members.insert(
                "concierge".to_string(),
                MemberLook {
                    fill: a,
                    seed: 1 + seed_from(&format!("{site_id}:concierge")) % 9999,
                },
            );
        }
        SiteLook {
            key: site_id.to_string(),
            name: if name.is_empty() {
                site_id.to_string()
            } else {
                name.to_string()
            },
            owner: if name.is_empty() {
                "the owner".to_string()
            } else {
                name.to_string()
            },
            bg,
            ink,
            palette,
            font: if tokens.display.is_empty() {
                "Georgia, 'Times New Roman', serif".to_string()
            } else {
                tokens.display.clone()
            },
            squareness: DEFAULT_SQUARENESS,
            family: Family::ALL[(seed_from(site_id) % 5) as usize],
            sample: false,
            members,
        }
    }
}

/// The colours and face the Move extracted (the first release's `agentrix_move::theme::Tokens`, the
/// fields the mark needs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Page background.
    pub bg: String,
    /// Body text.
    pub ink: String,
    /// The brand colour.
    pub accent: String,
    /// The display font stack.
    pub display: String,
}

/// Reads the `--wa-*` tokens a signed page carries in its `:root` block.
pub fn tokens_from_html(html: &str) -> Option<Tokens> {
    let var = |name: &str| -> Option<String> {
        let key = format!("--wa-{name}:");
        let at = html.find(&key)? + key.len();
        let rest = &html[at..];
        let end = rest.find([';', '}', '\n'])?;
        let v = rest[..end].trim();
        if v.is_empty() || v.contains('<') {
            None
        } else {
            Some(v.to_string())
        }
    };
    Some(Tokens {
        bg: var("bg")?,
        ink: var("ink")?,
        accent: var("accent").unwrap_or_default(),
        display: var("display").unwrap_or_default(),
    })
}

fn m(fill: &str, seed: u32) -> MemberLook {
    MemberLook {
        fill: fill.to_string(),
        seed,
    }
}

/// The sample sites' slugs.
pub const SAMPLES: [&str; 3] = ["kilele", "classy", "tana"];

/// A sample site's look, from `concept/identity/marks.html` (and the Kilele team in
/// `concept/journeys/journeys.js` for the WebAgents marks.html doesn't draw).
// One row per role reads as a table; a struct per row would only add names.
#[allow(clippy::type_complexity)]
pub fn sample(slug: &str) -> Option<SiteLook> {
    let (name, owner, font, bg, ink, family, n, palette, members): (
        &str,
        &str,
        &str,
        &str,
        &str,
        Family,
        f64,
        [&str; 5],
        Vec<(&str, MemberLook)>,
    ) = match slug {
        "kilele" => (
            "Kilele Bakery",
            "Wanjiru",
            "'Young Serif', Georgia, 'Times New Roman', serif",
            "#fbf6ef",
            "#2b1d14",
            Family::Grain,
            4.6,
            ["#8e3b46", "#9c5725", "#3b2416", "#577035", "#c9a227"],
            vec![
                ("cakes", m("#8e3b46", 7)),
                ("bread", m("#9c5725", 3)),
                ("orders", m("#3b2416", 11)),
                ("visit", m("#ad8b22", 5)),
                ("concierge", m("#6b5647", 19)),
                ("editor", m("#577035", 5)),
                ("guardian", m("#2b1d14", 2)),
                ("cashier", m("#11734a", 23)),
                ("mover", m("#2f55c4", 17)),
                ("caretaker", m("#3d5a1f", 29)),
                ("sales", m("#9c5725", 31)),
                ("reputation", m("#c9a227", 37)),
            ],
        ),
        "classy" => (
            "Classy Perfumes",
            "Classy Perfumes",
            "'Italiana', Didot, 'Bodoni 72', Georgia, serif",
            "#f7f4f8",
            "#1d1626",
            Family::Facets,
            6.2,
            ["#2a1f3d", "#7f6236", "#6d2e4f", "#3d5a6c", "#9d506b"],
            vec![
                ("gifts", m("#6d2e4f", 19)),
                ("fragrances", m("#2a1f3d", 4)),
                ("orders", m("#3d5a6c", 8)),
                ("journal", m("#5e5468", 27)),
                ("concierge", m("#2a1f3d", 3)),
                ("reputation", m("#7f6236", 13)),
                ("cashier", m("#9d506b", 6)),
                ("sales", m("#6d2e4f", 31)),
                ("editor", m("#3d5a6c", 5)),
                ("guardian", m("#1d1626", 2)),
                ("mover", m("#2f55c4", 17)),
                ("caretaker", m("#3d5a6c", 29)),
            ],
        ),
        "tana" => (
            "Tana Valley Times",
            "Tana Valley Times",
            "'Libre Caslon Text', Georgia, 'Times New Roman', serif",
            "#f4f5f7",
            "#14213d",
            Family::Strata,
            7.2,
            ["#14213d", "#b3261e", "#2f5d8a", "#56636f", "#7a4b2a"],
            vec![
                ("newsroom", m("#b3261e", 21)),
                ("archive", m("#2f5d8a", 9)),
                ("subscriptions", m("#14213d", 15)),
                ("concierge", m("#14213d", 4)),
                ("editor", m("#7a4b2a", 1)),
                ("guardian", m("#56636f", 12)),
                ("cashier", m("#2f5d8a", 18)),
                ("mover", m("#2f55c4", 17)),
                ("caretaker", m("#56636f", 29)),
                ("sales", m("#b3261e", 31)),
                ("reputation", m("#7a4b2a", 37)),
            ],
        ),
        _ => return None,
    };
    Some(SiteLook {
        key: slug.to_string(),
        name: name.to_string(),
        owner: owner.to_string(),
        bg: bg.to_string(),
        ink: ink.to_string(),
        palette: palette.iter().map(|c| c.to_string()).collect(),
        font: font.to_string(),
        squareness: n,
        family,
        sample: true,
        members: members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    })
}

/// agentrix.website's own look (site [`crate::home::SITE`]): the brand's indigo ink on white,
/// the orbits family, and one colour and seed per member. The same look W10's home page draws
/// the nine roles in.
pub fn agentrix() -> SiteLook {
    let members = [
        ("your-site", m("#2f55c4", 3)),
        ("how-it-works", m("#38356d", 11)),
        ("samples", m("#b72d50", 7)),
        ("prices", m("#11734a", 23)),
        ("concierge", m("#38356d", 19)),
        ("guardian", m("#1f1d3d", 2)),
        ("editor", m("#8a5300", 5)),
        ("sales", m("#b72d50", 31)),
        ("mover", m("#2f55c4", 17)),
        ("cashier", m("#11734a", 6)),
        ("caretaker", m("#11734a", 29)),
        ("reputation", m("#8a5300", 37)),
    ];
    SiteLook {
        key: "agentrix".to_string(),
        name: "Agentrix".to_string(),
        owner: "James".to_string(),
        bg: "#ffffff".to_string(),
        ink: "#1f1d3d".to_string(),
        palette: ["#38356d", "#b72d50", "#2f55c4", "#11734a", "#8a5300"]
            .iter()
            .map(|c| c.to_string())
            .collect(),
        font: "Manrope, ui-sans-serif, system-ui, sans-serif".to_string(),
        squareness: DEFAULT_SQUARENESS,
        family: Family::Orbits,
        sample: false,
        members: members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agentrix_wears_the_brand_and_passes_contrast() {
        let look = agentrix();
        assert!(!look.sample, "agentrix.website is a real site");
        for (id, m) in &look.members {
            assert!(color::passes(&m.fill, &look.bg), "{id} {}", m.fill);
            assert_eq!(look.fill_for(id, "#000000"), m.fill, "{id}");
        }
    }

    #[test]
    fn reads_tokens_from_a_signed_page() {
        let html = "<style>\n  :root {\n    --wa-bg: #fffdf8; --wa-soft: #f6efe4; --wa-ink: #1f1a14;\n    --wa-ink-2: #5b4f42; --wa-line: #e6dccd; --wa-accent: #8a3b12;\n    --wa-on-accent: #ffffff; --wa-display: Georgia, 'Times New Roman', serif; --wa-font: system-ui;\n  }\n</style>";
        let t = tokens_from_html(html).unwrap();
        assert_eq!(t.bg, "#fffdf8");
        assert_eq!(t.ink, "#1f1a14");
        assert_eq!(t.accent, "#8a3b12");
        assert_eq!(t.display, "Georgia, 'Times New Roman', serif");
        let look = SiteLook::from_tokens("site_k1", "Mama Oliech", &t);
        assert_eq!(look.monogram(), "M");
        assert!(color::passes(&look.fill_for("concierge", "#000"), &look.bg));
    }

    #[test]
    fn samples_keep_the_concept_and_pass_contrast() {
        for slug in SAMPLES {
            let look = sample(slug).unwrap();
            assert!(look.sample);
            for (id, m) in &look.members {
                let fill = look.fill_for(id, "#000000");
                assert!(color::passes(&fill, &look.bg), "{slug}/{id} {fill}");
                if color::passes(&m.fill, &look.bg) {
                    assert_eq!(fill, m.fill, "{slug}/{id} keeps the concept's colour");
                }
            }
        }
        assert_eq!(sample("kilele").unwrap().seed_for("cakes"), 7);
    }
}
