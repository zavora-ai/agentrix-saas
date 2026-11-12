//! The site's own look around the account pages, as a port. W2's themes implement
//! [`SiteShell`] for each sample (Kilele, Classy, Tana): header with the account chip, the
//! Sample band, footer and theme CSS. This crate renders only the page body, with the class
//! names agreed with W2 (`acct`, `btn`, `field`, `note`, `keeps`, `codes`, `choice`, `switch`,
//! `receipt`, `list`, `pill`, `small`, `link`).
//!
//! [`PlainShell`] is the fallback for a host without a theme, and for local previews.

use agentrix_types::tenancy::SiteHost;

/// Wraps an account page's body in the site's shell.
pub trait SiteShell: Send + Sync {
    /// The whole page: `body` (trusted HTML from this crate) inside the site's header, Sample
    /// band and footer. `signed_in` is the customer's name for the account chip, or `None`
    /// for "Guest". The page must load `/account/account.js` with `defer` (or `body` does it).
    fn page(&self, site: &SiteHost, title: &str, body: &str, signed_in: Option<&str>) -> String;

    /// The site's own line for "Offers by email", for example "New bakes and seasonal
    /// offers." for Kilele. `None` uses a general line.
    fn offers_line(&self, site: &SiteHost) -> Option<String> {
        let _ = site;
        None
    }
}

/// A plain, accessible shell using `/account/account.css`. Used where no theme is wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainShell;

impl SiteShell for PlainShell {
    fn page(&self, site: &SiteHost, title: &str, body: &str, signed_in: Option<&str>) -> String {
        let name = escape(&site.name);
        let title = escape(title);
        let chip = match signed_in {
            Some(who) => {
                let who = escape(who);
                let initial = who
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default();
                format!(
                    r#"<a class="acct-chip" href="/account/security"><span class="me" aria-hidden="true">{initial}</span>{who}</a>"#
                )
            }
            None => r#"<a class="acct-chip" href="/account/sign-in">Guest</a>"#.to_owned(),
        };
        let band = if site.sample {
            format!(
                r#"<p class="sample-band">Sample site. {name} is invented for the Agentrix demo: no real orders, people or payments.</p>"#
            )
        } else {
            String::new()
        };
        format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · {name}</title>
<link rel="stylesheet" href="/account/account.css">
<script src="/account/account.js" defer></script>
</head>
<body class="acct-plain">
{band}
<header class="acct-bar"><a class="acct-logo" href="/">{name}</a>{chip}</header>
<main id="main">
{body}
</main>
<footer class="acct-foot"><a class="link" href="/account/keeps">What {name} keeps</a></footer>
</body>
</html>
"#
        )
    }
}

/// Escapes text for HTML element content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentrix_types::SiteId;

    #[test]
    fn plain_shell_escapes_and_marks_samples() {
        let site = SiteHost {
            site: SiteId::new("site_k"),
            host: "k.test".into(),
            origin: "https://k.test".into(),
            name: "Kilele <Bakery>".into(),
            sample: true,
        };
        let html = PlainShell.page(&site, "Sign in", "<p>body</p>", Some("Njeri"));
        assert!(html.contains("Kilele &lt;Bakery&gt;"));
        assert!(html.contains("Sample site."));
        assert!(html.contains(">N</span>Njeri"));
        assert!(html.contains("<p>body</p>"));
    }
}
