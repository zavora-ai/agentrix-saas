//! `MarkSpec`: what one WebAgent looks like, as stored in its spec and published with a release
//! (ADR-031, "Spec and generator"). The JSON shape is the one the Workshop prototype prints.

use serde::{Deserialize, Serialize};

use crate::color;
use crate::glyphs;

/// The current `markSpec` version.
pub const MARK_SPEC_VERSION: u32 = 1;
/// The softest corners the owner may choose.
pub const SQUARENESS_MIN: f64 = 3.2;
/// The squarest corners the owner may choose. Never a circle at either end.
pub const SQUARENESS_MAX: f64 = 8.0;

/// A WebAgent's mark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkSpec {
    /// The spec version, `1`.
    #[serde(rename = "markSpec")]
    pub version: u32,
    /// The site's id, part of the signature's seed key.
    pub site: String,
    /// The WebAgent's (or desk's) id, part of the signature's seed key.
    pub webagent: String,
    /// The outline.
    pub silhouette: Silhouette,
    /// The fill, and the glyph colour that follows from it.
    pub color: MarkColor,
    /// A role icon or a monogram.
    pub glyph: Glyph,
    /// The pattern family and this WebAgent's seed.
    pub signature: Signature,
}

/// The outline: always a squircle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Silhouette {
    /// Always `squircle`. Circles mean a person.
    pub shape: Shape,
    /// The superellipse exponent, from 3.2 (softer) to 8 (squarer).
    pub squareness: f64,
}

/// The only outline a WebAgent may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// A superellipse.
    Squircle,
}

/// The mark's colours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkColor {
    /// `#rrggbb`.
    pub fill: String,
    /// `#rrggbb`: white or near-black, whichever contrasts more with the fill. Always derived.
    pub glyph: String,
}

/// The glyph layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Glyph {
    /// A role line icon from [`crate::glyphs::GLYPHS`].
    Role {
        /// The glyph id, for example `cake`.
        id: String,
    },
    /// One or two letters in the site's display face.
    Monogram {
        /// The letters.
        letters: String,
        /// Always "site display": the face comes from the site's tokens at render time.
        #[serde(default = "site_display")]
        face: String,
    },
}

fn site_display() -> String {
    "site display".to_string()
}

/// The signature layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// The site's family.
    pub family: Family,
    /// This WebAgent's seed.
    pub seed: u32,
}

/// The five signature families. A site picks one, so its WebAgents read as one team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Arcs and a moon; spins while working.
    Orbits,
    /// A sunflower of dots; spins while working.
    Grain,
    /// Wavy bands; drift while working.
    Strata,
    /// Two crossing sets of lines; drift while working.
    Weave,
    /// Fan triangles from a point; spins while working.
    Facets,
}

impl Family {
    /// Every family, in the Workshop's order.
    pub const ALL: [Family; 5] = [
        Family::Orbits,
        Family::Grain,
        Family::Strata,
        Family::Weave,
        Family::Facets,
    ];

    /// The id, as in JSON.
    pub fn id(self) -> &'static str {
        match self {
            Family::Orbits => "orbits",
            Family::Grain => "grain",
            Family::Strata => "strata",
            Family::Weave => "weave",
            Family::Facets => "facets",
        }
    }

    /// The name on the Workshop's tile.
    pub fn label(self) -> &'static str {
        match self {
            Family::Orbits => "Orbits",
            Family::Grain => "Grain",
            Family::Strata => "Strata",
            Family::Weave => "Weave",
            Family::Facets => "Facets",
        }
    }
}

/// Why a spec was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MarkError {
    /// A version this build doesn't know.
    #[error("mark spec version {0} isn't supported")]
    Version(u32),
    /// An empty id, or one with a colon (the seed key's separator).
    #[error("the {0} id must be 1 to 64 characters without a colon")]
    Id(&'static str),
    /// Squareness out of range.
    #[error("corners must be between 3.2 and 8, got {0}")]
    Squareness(f64),
    /// Not `#rrggbb`.
    #[error("{0} isn't a colour like #8e3b46")]
    Colour(String),
    /// A glyph id that isn't in the set.
    #[error("there's no role glyph called {0}")]
    Glyph(String),
    /// Monogram letters missing or too long.
    #[error("a monogram is one or two letters")]
    Monogram,
}

fn check_id(id: &str, what: &'static str) -> Result<(), MarkError> {
    if id.is_empty() || id.chars().count() > 64 || id.contains(':') {
        return Err(MarkError::Id(what));
    }
    Ok(())
}

impl MarkSpec {
    /// A spec with the glyph colour derived from the fill.
    pub fn new(
        site: &str,
        webagent: &str,
        fill: &str,
        squareness: f64,
        glyph: Glyph,
        family: Family,
        seed: u32,
    ) -> Result<Self, MarkError> {
        let fill = color::normalize(fill).ok_or_else(|| MarkError::Colour(fill.to_string()))?;
        let spec = MarkSpec {
            version: MARK_SPEC_VERSION,
            site: site.to_string(),
            webagent: webagent.to_string(),
            silhouette: Silhouette {
                shape: Shape::Squircle,
                squareness,
            },
            color: MarkColor {
                glyph: color::on_color(&fill).to_string(),
                fill,
            },
            glyph,
            signature: Signature { family, seed },
        };
        spec.validate()?;
        Ok(spec)
    }

    /// A role glyph.
    pub fn role(id: &str) -> Glyph {
        Glyph::Role { id: id.to_string() }
    }

    /// A monogram glyph.
    pub fn monogram(letters: &str) -> Glyph {
        Glyph::Monogram {
            letters: letters.to_string(),
            face: site_display(),
        }
    }

    /// Checks every field the renderer relies on.
    pub fn validate(&self) -> Result<(), MarkError> {
        if self.version != MARK_SPEC_VERSION {
            return Err(MarkError::Version(self.version));
        }
        check_id(&self.site, "site")?;
        check_id(&self.webagent, "WebAgent")?;
        let n = self.silhouette.squareness;
        if !(SQUARENESS_MIN..=SQUARENESS_MAX).contains(&n) {
            return Err(MarkError::Squareness(n));
        }
        if color::normalize(&self.color.fill).as_deref() != Some(self.color.fill.as_str()) {
            return Err(MarkError::Colour(self.color.fill.clone()));
        }
        match &self.glyph {
            Glyph::Role { id } => {
                if glyphs::glyph_svg(id).is_none() {
                    return Err(MarkError::Glyph(id.clone()));
                }
            }
            Glyph::Monogram { letters, .. } => {
                let units = letters.encode_utf16().count();
                if letters.trim().is_empty() || units > 2 || letters.chars().any(char::is_control) {
                    return Err(MarkError::Monogram);
                }
            }
        }
        Ok(())
    }

    /// The same spec with the fill lowercased and the glyph colour re-derived, as stored.
    pub fn normalized(mut self) -> Self {
        if let Some(fill) = color::normalize(&self.color.fill) {
            self.color.glyph = color::on_color(&fill).to_string();
            self.color.fill = fill;
        }
        self
    }

    /// Parses and validates a spec from JSON.
    pub fn from_json(json: &str) -> Result<Self, MarkSpecParseError> {
        let spec: MarkSpec = serde_json::from_str(json)?;
        let spec = spec.normalized();
        spec.validate()?;
        Ok(spec)
    }

    /// The key the signature's generator is seeded with.
    pub fn seed_key(&self) -> String {
        format!("{}:{}:{}", self.site, self.webagent, self.signature.seed)
    }

    /// Whether the fill keeps 3:1 on the site's background, and the glyph 3:1 on the fill.
    pub fn passes_on(&self, ground: &str) -> bool {
        color::passes(&self.color.fill, ground)
    }
}

/// A spec that couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum MarkSpecParseError {
    /// Not the JSON shape.
    #[error("not a mark spec: {0}")]
    Json(#[from] serde_json::Error),
    /// The right shape, with a value out of bounds.
    #[error(transparent)]
    Invalid(#[from] MarkError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_workshop_shape() {
        let json = r##"{
          "markSpec": 1, "site": "kilele", "webagent": "cakes",
          "silhouette": { "shape": "squircle", "squareness": 4.6 },
          "color": { "fill": "#8E3B46", "glyph": "#000000" },
          "glyph": { "kind": "role", "id": "cake" },
          "signature": { "family": "grain", "seed": 7 }
        }"##;
        let spec = MarkSpec::from_json(json).unwrap();
        assert_eq!(spec.color.fill, "#8e3b46");
        assert_eq!(spec.color.glyph, "#ffffff");
        assert_eq!(spec.seed_key(), "kilele:cakes:7");
        let back = serde_json::to_value(&spec).unwrap();
        assert_eq!(back["glyph"]["kind"], "role");
        assert_eq!(back["markSpec"], 1);
    }

    #[test]
    fn refuses_what_the_platform_keeps() {
        let ok = |g| MarkSpec::new("tana", "editor", "#7a4b2a", 7.2, g, Family::Strata, 1);
        assert!(ok(MarkSpec::role("pen")).is_ok());
        assert!(ok(MarkSpec::monogram("TV")).is_ok());
        assert_eq!(
            ok(MarkSpec::role("face")),
            Err(MarkError::Glyph("face".into()))
        );
        assert_eq!(ok(MarkSpec::monogram("ABC")), Err(MarkError::Monogram));
        assert_eq!(ok(MarkSpec::monogram(" ")), Err(MarkError::Monogram));
        let round = MarkSpec::new(
            "tana",
            "editor",
            "#7a4b2a",
            2.0,
            MarkSpec::role("pen"),
            Family::Strata,
            1,
        );
        assert_eq!(round, Err(MarkError::Squareness(2.0)));
        let colon = MarkSpec::new(
            "ta:na",
            "editor",
            "#7a4b2a",
            4.0,
            MarkSpec::role("pen"),
            Family::Strata,
            1,
        );
        assert_eq!(colon, Err(MarkError::Id("site")));
    }
}
