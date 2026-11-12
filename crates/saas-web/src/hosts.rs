//! Which of the four hosts a request is for.
//!
//! Production names come from `.env` (`SAAS_HOST`, `SAAS_KILELE_HOST`, `SAAS_AGENT_DOOR_HOST`,
//! `SAAS_BOOKS_HOST`). Locally every host has a `.localhost` alias on the one port, which
//! browsers resolve to loopback with no `/etc/hosts` change:
//!
//! | Host | Local alias |
//! |---|---|
//! | `money.agentrix.website` | `money.localhost:18183` |
//! | `kilele.money.agentrix.website` | `kilele.money.localhost:18183` |
//! | `pay.agentrix.website` | `pay.localhost:18183` |
//! | `books.agentrix.website` | `books.localhost:18183` |
//!
//! A bare loopback address (`127.0.0.1`, `localhost`) is the money host, and on loopback only
//! the `x-saas-site` header (`money`, `kilele`, `pay`, `books`) picks another, for curl and tests.

use axum::http::HeaderMap;
use axum::http::header::HOST;
use serde::Serialize;

/// One of the four hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Site {
    /// `money.agentrix.website`: Agentrix's WebAgents front, owner console and treasury board.
    Money,
    /// `kilele.money.agentrix.website`: Kilele Bakery's own console.
    Kilele,
    /// `pay.agentrix.website`: the agent door.
    Pay,
    /// `books.agentrix.website`: Zavora ERA, hosted (slice A runs it).
    Books,
}

impl Site {
    /// Every site, in the order the hosts table lists them.
    pub const ALL: [Site; 4] = [Site::Money, Site::Kilele, Site::Pay, Site::Books];

    /// The short name used by `x-saas-site` and the `.localhost` aliases.
    pub fn key(self) -> &'static str {
        match self {
            Site::Money => "money",
            Site::Kilele => "kilele",
            Site::Pay => "pay",
            Site::Books => "books",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "money" => Some(Site::Money),
            "kilele" | "kilele.money" => Some(Site::Kilele),
            "pay" => Some(Site::Pay),
            "books" => Some(Site::Books),
            _ => None,
        }
    }
}

/// The public host names, and how a request's `Host` maps to a [`Site`].
#[derive(Debug, Clone, Serialize)]
pub struct Hosts {
    /// Agentrix's front, console and board.
    pub money: String,
    /// Kilele Bakery's console.
    pub kilele: String,
    /// The agent door.
    pub pay: String,
    /// Zavora ERA.
    pub books: String,
}

impl Default for Hosts {
    fn default() -> Self {
        Self {
            money: "money.agentrix.website".into(),
            kilele: "kilele.money.agentrix.website".into(),
            pay: "pay.agentrix.website".into(),
            books: "books.agentrix.website".into(),
        }
    }
}

impl Hosts {
    /// The production names, overridden by `SAAS_HOST`, `SAAS_KILELE_HOST`,
    /// `SAAS_AGENT_DOOR_HOST` and `SAAS_BOOKS_HOST` when set.
    pub fn from_env() -> Self {
        let pick = |name: &str, default: String| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty() && !v.starts_with('#'))
                .unwrap_or(default)
        };
        let d = Self::default();
        Self {
            money: pick("SAAS_HOST", d.money),
            kilele: pick("SAAS_KILELE_HOST", d.kilele),
            pay: pick("SAAS_AGENT_DOOR_HOST", d.pay),
            books: pick("SAAS_BOOKS_HOST", d.books),
        }
    }

    /// The public name of `site`.
    pub fn name(&self, site: Site) -> &str {
        match site {
            Site::Money => &self.money,
            Site::Kilele => &self.kilele,
            Site::Pay => &self.pay,
            Site::Books => &self.books,
        }
    }

    /// The site a request is for, from its `Host` header (and, on loopback, `x-saas-site`).
    pub fn site_of(&self, headers: &HeaderMap) -> Option<Site> {
        let host = headers.get(HOST).and_then(|v| v.to_str().ok())?;
        let name = bare(host);
        if is_loopback(&name) {
            let asked = headers
                .get("x-saas-site")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| Site::from_key(v.trim()));
            return Some(asked.unwrap_or(Site::Money));
        }
        if let Some(site) = Site::ALL.into_iter().find(|s| self.name(*s) == name) {
            return Some(site);
        }
        name.strip_suffix(".localhost").and_then(Site::from_key)
    }

    /// Every origin a browser may call this server's agent API from: the money host in
    /// production, and its local aliases (including the board's dev server on 18184).
    pub fn board_origins(&self) -> Vec<String> {
        vec![
            format!("https://{}", self.money),
            "http://money.localhost:*".into(),
            "http://localhost:*".into(),
            "http://127.0.0.1:*".into(),
        ]
    }
}

/// `Money.Agentrix.Website.:443` → `money.agentrix.website`.
fn bare(host: &str) -> String {
    let host = host.trim();
    let without_port = if host.starts_with('[') {
        host.split_once(']')
            .map_or(host, |(h, _)| h)
            .trim_start_matches('[')
    } else {
        host.rsplit_once(':')
            .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
            .map_or(host, |(h, _)| h)
    };
    without_port.trim_end_matches('.').to_ascii_lowercase()
}

fn is_loopback(name: &str) -> bool {
    matches!(name, "localhost" | "127.0.0.1" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn with_host(host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(HOST, HeaderValue::from_str(host).unwrap());
        h
    }

    #[test]
    fn production_names_and_local_aliases_map_to_their_site() {
        let hosts = Hosts::default();
        for (host, site) in [
            ("money.agentrix.website", Site::Money),
            ("Money.Agentrix.Website.", Site::Money),
            ("kilele.money.agentrix.website", Site::Kilele),
            ("pay.agentrix.website:443", Site::Pay),
            ("books.agentrix.website", Site::Books),
            ("money.localhost:18183", Site::Money),
            ("kilele.money.localhost:18183", Site::Kilele),
            ("pay.localhost:18183", Site::Pay),
            ("books.localhost", Site::Books),
            ("127.0.0.1:18183", Site::Money),
            ("[::1]:18183", Site::Money),
        ] {
            assert_eq!(hosts.site_of(&with_host(host)), Some(site), "{host}");
        }
    }

    #[test]
    fn other_hosts_are_nobodys() {
        let hosts = Hosts::default();
        for host in [
            "agentrix.website",
            "evil.example",
            "kilele.agentrix.website",
            "x.localhost",
        ] {
            assert_eq!(hosts.site_of(&with_host(host)), None, "{host}");
        }
        assert_eq!(hosts.site_of(&HeaderMap::new()), None);
    }

    #[test]
    fn the_site_header_counts_on_loopback_only() {
        let hosts = Hosts::default();
        let mut h = with_host("127.0.0.1:18183");
        h.insert("x-saas-site", HeaderValue::from_static("pay"));
        assert_eq!(hosts.site_of(&h), Some(Site::Pay));
        let mut h = with_host("money.agentrix.website");
        h.insert("x-saas-site", HeaderValue::from_static("pay"));
        assert_eq!(hosts.site_of(&h), Some(Site::Money));
    }
}
