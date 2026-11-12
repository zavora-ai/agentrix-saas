//! Writes Kilele Bakery's WebAgent marks (W1's generator, the concept's seeds) as standalone
//! SVG files for the home page film's cast: `film/marks/{id}.svg`.
//!
//! ```sh
//! WCARGO_ISOLATE_BUILD=1 scripts/wcargo run -p agentrix-home --example film-marks
//! ```

use agentrix_kernel::roster;
use agentrix_marks::MarkState;

fn main() -> anyhow::Result<()> {
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/film/marks");
    std::fs::create_dir_all(out)?;
    let r = roster::sample("kilele-bakery-sample", "kilele")
        .ok_or_else(|| anyhow::anyhow!("no Kilele roster"))?;
    let style = format!("<style>{}</style>", agentrix_marks::assets::CSS);
    let standalone = |svg: String| {
        let svg = svg.replacen("<svg ", r#"<svg xmlns="http://www.w3.org/2000/svg" "#, 1);
        match svg.find('>') {
            Some(i) => format!("{}{style}{}", &svg[..=i], &svg[i + 1..]),
            None => svg,
        }
    };
    let team = r.team_svg(160.0, MarkState::Idle, &[]);
    std::fs::write(format!("{out}/chief.svg"), standalone(team))?;
    for id in [
        "concierge",
        "cashier",
        "guardian",
        "editor",
        "desk-cakes",
        "desk-bread",
        "desk-orders",
    ] {
        let svg = r
            .mark_svg(id, 160.0, MarkState::Idle, &id.replace('-', ""))
            .ok_or_else(|| anyhow::anyhow!("no mark for {id}"))?;
        std::fs::write(format!("{out}/{id}.svg"), standalone(svg))?;
        println!("wrote {id}");
    }
    Ok(())
}
