//! Host token sets, stylesheets and script islands for the sample sites (W2).
//!
//! - `agentrix.css`: the Agentrix tokens (copied from `concept/styles.css`).
//! - `kit.css`: the WebAgent components (peek, Ask bar, rail, thread, sheets), themed by the
//!   host's `--wa-*` tokens (copied from `concept/experience/experience.css`).
//! - `site.css`: the samples' shared layer (Sample band, account and order pages, living marks).
//! - one stylesheet per sample brand (`kilele.css`, `classy.css`, `tana.css`) and the owner
//!   apps' console look (`owner.css`).
//! - script islands (`marks.js`, `desks.js`, …), plain ES with no framework.
//!
//! Pages link them as `/_s/<name>?v=<VERSION>`.

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// Content version of every asset together; changes whenever a file changes.
pub const VERSION: &str = env!("AGENTRIX_THEMES_VERSION");

/// An asset's bytes and content type by file name.
pub fn asset(name: &str) -> Option<(&'static [u8], &'static str)> {
    let bytes = ASSETS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)?;
    let kind = match name.rsplit('.').next() {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };
    Some((bytes, kind))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_kit_and_kilele_are_embedded() {
        for name in [
            "agentrix.css",
            "kit.css",
            "site.css",
            "kilele.css",
            "marks.js",
        ] {
            assert!(super::asset(name).is_some(), "{name}");
        }
        assert!(super::asset("../Cargo.toml").is_none());
    }
}
