//! The account pages' bodies: minijinja templates (strict undefined, HTML auto-escaped) with the
//! class names agreed with W2. The site's shell ([`crate::SiteShell`]) wraps them.

use minijinja::{Environment, UndefinedBehavior};
use serde::Serialize;

use super::VisitorError;

const TEMPLATES: &[(&str, &str)] = &[
    (
        "sign-in.html",
        include_str!("../../templates/visitor/sign-in.html"),
    ),
    (
        "create.html",
        include_str!("../../templates/visitor/create.html"),
    ),
    (
        "code.html",
        include_str!("../../templates/visitor/code.html"),
    ),
    (
        "code-expired.html",
        include_str!("../../templates/visitor/code-expired.html"),
    ),
    (
        "recovery-codes.html",
        include_str!("../../templates/visitor/recovery-codes.html"),
    ),
    (
        "recovery-gone.html",
        include_str!("../../templates/visitor/recovery-gone.html"),
    ),
    (
        "passkey-offer.html",
        include_str!("../../templates/visitor/passkey-offer.html"),
    ),
    (
        "recover.html",
        include_str!("../../templates/visitor/recover.html"),
    ),
    (
        "security.html",
        include_str!("../../templates/visitor/security.html"),
    ),
    (
        "choices.html",
        include_str!("../../templates/visitor/choices.html"),
    ),
    (
        "keeps.html",
        include_str!("../../templates/visitor/keeps.html"),
    ),
    (
        "signed-out.html",
        include_str!("../../templates/visitor/signed-out.html"),
    ),
    (
        "data.html",
        include_str!("../../templates/visitor/data.html"),
    ),
    (
        "erased.html",
        include_str!("../../templates/visitor/erased.html"),
    ),
];

/// The script every account page loads.
pub const ACCOUNT_JS: &str = include_str!("../../static/account.js");
/// The plain shell's stylesheet (themes style the same class names their own way).
pub const ACCOUNT_CSS: &str = include_str!("../../static/account.css");

/// The templates, loaded once.
pub struct Views {
    env: Environment<'static>,
}

impl Views {
    /// Loads every template. Panics only if a template in this crate doesn't parse, which the
    /// tests catch.
    pub fn new() -> Self {
        let mut env = Environment::new();
        env.set_undefined_behavior(UndefinedBehavior::Strict);
        for (name, source) in TEMPLATES {
            env.add_template(name, source)
                .unwrap_or_else(|e| panic!("template {name}: {e}"));
        }
        Self { env }
    }

    /// Renders a body.
    pub fn render<S: Serialize>(&self, name: &str, ctx: S) -> Result<String, VisitorError> {
        self.env
            .get_template(name)
            .and_then(|t| t.render(ctx))
            .map_err(|e| VisitorError::Internal(format!("template {name}: {e:#}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_file_is_registered() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/visitor");
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            assert!(
                TEMPLATES.iter().any(|(n, _)| *n == name),
                "{name} is in templates/visitor but not in TEMPLATES"
            );
        }
    }

    #[test]
    fn every_template_parses() {
        let views = Views::new();
        for (name, _) in TEMPLATES {
            assert!(views.env.get_template(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn missing_values_are_errors_not_blanks() {
        let views = Views::new();
        assert!(
            views
                .render("signed-out.html", minijinja::context! {})
                .is_err()
        );
        let ok = views
            .render(
                "signed-out.html",
                minijinja::context! { site => "Kilele <Bakery>" },
            )
            .unwrap();
        assert!(ok.contains("Kilele &lt;Bakery&gt;"), "escaped");
    }

    #[test]
    fn no_banned_words_or_dashes_in_templates() {
        let banned = [
            "seamless",
            "effortless",
            "unlock",
            "empower",
            "magic",
            "Oops",
            "journey",
            "—",
            "–",
            "!",
        ];
        for (name, source) in TEMPLATES {
            for word in banned {
                // "!" is allowed only inside template tags such as `{% if not x %}` (none use it).
                assert!(!source.contains(word), "{name} contains {word:?}");
            }
        }
    }
}
