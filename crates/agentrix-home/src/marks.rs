//! The living marks on the home page, from W1's generator (ADR-031, `agentrix-marks`).
//!
//! A mark is 5 to 14 KB of SVG, too heavy to inline many times in a page that must stay under
//! 60 KB. So the marks are built once at startup and served as cached images under
//! `/_home/marks/`, loaded lazily below the fold:
//!
//! - `{sample}.svg`: one picture per sample site, its Chief WebAgent's team mark, its section
//!   desks and its Guardian, in the site's own colours and pattern family, as the concept draws
//!   them seed for seed;
//! - `role-{id}.svg`: each of the nine roles in Agentrix's own look.
//!
//! Every mark is idle: on the home page nobody is working, and a mark moves only while its
//! WebAgent works. The marks' own stylesheet travels inside each SVG, so the halo, trace and
//! waiting dot stay hidden as they do on a site.

use std::collections::BTreeMap;

use agentrix_kernel::roster::{self, NINE};
use agentrix_kernel::{Roster, SiteLook};
use agentrix_marks::{Family, MarkState};
use serde::Serialize;

/// The marks' state CSS, so a mark served as an image looks as it does inline.
fn style() -> String {
    format!("<style>{}</style>", agentrix_marks::assets::CSS)
}

/// Agentrix's own look, for the nine roles: the brand's indigo ink on white.
fn agentrix_look() -> SiteLook {
    SiteLook {
        key: "agentrix".into(),
        name: "Agentrix".into(),
        owner: "you".into(),
        bg: "#ffffff".into(),
        ink: "#1f1d3d".into(),
        palette: vec![
            "#38356d".into(),
            "#b72d50".into(),
            "#2f55c4".into(),
            "#11734a".into(),
            "#8a5300".into(),
        ],
        font: "Manrope, ui-sans-serif, system-ui, sans-serif".into(),
        squareness: 4.6,
        family: Family::Orbits,
        sample: false,
        members: BTreeMap::new(),
    }
}

/// Puts a rendered mark at `x`, `y` inside a larger SVG.
fn place(svg: &str, x: f64, y: f64) -> String {
    svg.replacen("<svg ", &format!(r#"<svg x="{x:.2}" y="{y:.2}" "#), 1)
}

/// One WebAgent shown in a sample's picture.
#[derive(Debug, Clone, Serialize)]
pub struct TeamMember {
    /// "Cakes", "Guardian".
    pub name: String,
    /// What it does, in the site's words.
    pub mandate: String,
}

/// A sample's picture and who is in it, left to right.
#[derive(Debug, Clone, Serialize)]
pub struct TeamPicture {
    /// `/_home/marks/kilele.svg`.
    pub src: String,
    /// The picture's size in CSS pixels.
    pub width: u32,
    /// The picture's size in CSS pixels.
    pub height: u32,
    /// Who is in it.
    pub members: Vec<TeamMember>,
}

/// The marks, built once.
pub struct Marks {
    files: BTreeMap<String, String>,
    teams: BTreeMap<String, TeamPicture>,
}

const CHIEF: f64 = 52.0;
const MEMBER: f64 = 40.0;
const GAP: f64 = 6.0;

fn team_picture(slug: &str, r: &Roster) -> (String, TeamPicture) {
    // Who: the Chief, the site's own section desks, then the Guardian.
    let mut ids: Vec<(String, String, String)> = r
        .desks()
        .iter()
        .filter(|d| d.id.starts_with("desk-"))
        .map(|d| (d.id.clone(), d.name.clone(), d.mandate.clone()))
        .collect();
    if let Some(g) = r.get("guardian") {
        ids.push((
            "guardian".into(),
            g.spec.name.clone(),
            g.spec.mandate.clone(),
        ));
    }
    let chief = r.team_svg(CHIEF, MarkState::Idle, &[]);
    let (cw, ch) = (CHIEF * 1.24, CHIEF * 1.34);
    let (mw, mh) = (MEMBER * 1.24, MEMBER * 1.34);
    let width = cw + ids.len() as f64 * (mw + GAP) + GAP;
    let height = ch;
    let mut body = place(&chief, 0.0, 0.0);
    let mut x = cw + GAP;
    for (id, _, _) in &ids {
        let uid = format!("{slug}{}", id.replace('-', ""));
        if let Some(svg) = r.mark_svg(id, MEMBER, MarkState::Idle, &uid) {
            body.push_str(&place(&svg, x, (ch - mh) / 2.0));
        }
        x += mw + GAP;
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.2} {height:.2}" width="{width:.0}" height="{height:.0}">{}{body}</svg>"#,
        style()
    );
    let chief_name = r
        .get("coordinator")
        .map(|c| c.spec.name.clone())
        .unwrap_or_else(|| "Chief WebAgent".into());
    let chief_mandate = r
        .get("coordinator")
        .map(|c| c.spec.mandate.clone())
        .unwrap_or_default();
    let mut members = vec![TeamMember {
        name: chief_name,
        mandate: chief_mandate,
    }];
    members.extend(
        ids.into_iter()
            .map(|(_, name, mandate)| TeamMember { name, mandate }),
    );
    (
        svg,
        TeamPicture {
            src: format!("/_home/marks/{slug}.svg?v={}", crate::ASSETS_VERSION),
            width: width.round() as u32,
            height: height.round() as u32,
            members,
        },
    )
}

impl Marks {
    /// Renders every mark the page uses.
    pub fn build() -> Self {
        let mut files = BTreeMap::new();
        let mut teams = BTreeMap::new();
        for slug in agentrix_kernel::looks::SAMPLES {
            if let Some(r) = roster::sample(&format!("{slug}-sample"), slug) {
                let (svg, picture) = team_picture(slug, &r);
                files.insert(format!("{slug}.svg"), svg);
                teams.insert(slug.to_string(), picture);
            }
        }
        let agentrix = Roster::build("agentrix", agentrix_look(), &NINE, &[]);
        for id in NINE {
            let uid = format!("role{}", id.replace('-', ""));
            if let Some(svg) = agentrix.mark_svg(id, MEMBER, MarkState::Idle, &uid) {
                let svg = svg.replacen("<svg ", r#"<svg xmlns="http://www.w3.org/2000/svg" "#, 1);
                // The marks' stylesheet goes just inside the root element.
                let svg = match svg.find('>') {
                    Some(i) => format!("{}{}{}", &svg[..=i], style(), &svg[i + 1..]),
                    None => svg,
                };
                files.insert(format!("role-{id}.svg"), svg);
            }
        }
        Self { files, teams }
    }

    /// A built file, by its name under `/_home/marks/`.
    pub fn file(&self, name: &str) -> Option<&str> {
        self.files.get(name).map(String::as_str)
    }

    /// A sample's picture, by its slug.
    pub fn team(&self, slug: &str) -> Option<&TeamPicture> {
        self.teams.get(slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sample_and_role_has_its_marks() {
        let m = Marks::build();
        for slug in ["kilele", "classy", "tana"] {
            let svg = m.file(&format!("{slug}.svg")).expect("a sample picture");
            assert!(svg.starts_with("<svg xmlns="));
            assert!(
                svg.contains(".wa .wa-halo { opacity: 0;"),
                "the state CSS travels with it"
            );
            let team = m.team(slug).unwrap();
            assert!(
                team.members.len() >= 4,
                "{slug}: the Chief, desks and the Guardian"
            );
            assert!(team.members.iter().any(|t| t.name == "Guardian"));
        }
        assert!(
            m.team("kilele")
                .unwrap()
                .members
                .iter()
                .any(|t| t.name == "Cakes")
        );
        for id in NINE {
            let svg = m.file(&format!("role-{id}.svg")).expect("a role mark");
            assert!(svg.contains(r#"data-state="idle""#));
        }
    }
}
