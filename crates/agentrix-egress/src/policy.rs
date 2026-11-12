//! What one job may reach: an origin allowlist plus the public-address rule.

use std::fmt;

/// One allowed destination: a host (or `*.suffix`) and an optional port.
///
/// Without a port, only 80 and 443 are allowed. Hosts compare case-insensitively and without a
/// trailing dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    host: String,
    wildcard: bool,
    port: Option<u16>,
}

impl Origin {
    /// Parses `host`, `host:port`, `*.host`, or a URL such as `https://host/`.
    ///
    /// ```
    /// use agentrix_egress::policy::Origin;
    /// let o = Origin::parse("https://Kilele.Example/").unwrap();
    /// assert!(o.allows("kilele.example", 443));
    /// assert!(!o.allows("kilele.example", 8080));
    /// ```
    pub fn parse(spec: &str) -> Result<Self, PolicyError> {
        let mut s = spec.trim();
        if let Some(rest) = s.split_once("://").map(|(_, r)| r) {
            s = rest;
        }
        let s = s.split('/').next().unwrap_or_default();
        let (host, port) = match s.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') || h.ends_with(']') => {
                let port = p
                    .parse::<u16>()
                    .map_err(|_| PolicyError::Origin(spec.into()))?;
                (h, Some(port))
            }
            _ => (s, None),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let host = normalise(host);
        let (wildcard, host) = match host.strip_prefix("*.") {
            Some(rest) => (true, rest.to_string()),
            None => (false, host),
        };
        if host.is_empty() || host.contains('*') || host.contains(char::is_whitespace) {
            return Err(PolicyError::Origin(spec.into()));
        }
        Ok(Self {
            host,
            wildcard,
            port,
        })
    }

    /// Whether this origin covers `host:port`.
    pub fn allows(&self, host: &str, port: u16) -> bool {
        let host = normalise(host);
        let host_ok = if self.wildcard {
            host.len() > self.host.len() + 1
                && host.ends_with(&self.host)
                && host.as_bytes()[host.len() - self.host.len() - 1] == b'.'
        } else {
            host == self.host
        };
        let port_ok = match self.port {
            Some(p) => p == port,
            None => port == 80 || port == 443,
        };
        host_ok && port_ok
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.wildcard {
            write!(f, "*.")?;
        }
        write!(f, "{}", self.host)?;
        if let Some(p) = self.port {
            write!(f, ":{p}")?;
        }
        Ok(())
    }
}

fn normalise(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// The egress policy of one job.
#[derive(Debug, Clone)]
pub struct Policy {
    /// Destinations the job may reach. Empty means nothing.
    pub allow: Vec<Origin>,
    /// Allow non-public addresses. Only for tests and the loopback replay relay; never set in
    /// production rooms.
    pub permit_non_public: bool,
    /// Maximum bytes per direction per connection.
    pub max_bytes: u64,
    /// Maximum lifetime of one connection.
    pub max_connection_secs: u64,
    /// Client addresses the proxy serves: the job's own room. Empty means any. Rooms share one
    /// bridge, so without this a room could use another job's proxy and its allowlist.
    pub peers: Vec<std::net::IpAddr>,
}

impl Policy {
    /// A policy allowing exactly `origins`, public addresses only, with default caps
    /// (25 MiB per direction, 60 s per connection).
    pub fn only(origins: impl IntoIterator<Item = Origin>) -> Self {
        Self {
            allow: origins.into_iter().collect(),
            permit_non_public: false,
            max_bytes: 25 * 1024 * 1024,
            max_connection_secs: 60,
            peers: Vec::new(),
        }
    }

    /// Whether a connection from `ip` is served.
    pub fn serves(&self, ip: std::net::IpAddr) -> bool {
        self.peers.is_empty() || self.peers.contains(&ip.to_canonical())
    }

    /// Whether `host:port` is on the allowlist.
    pub fn allows(&self, host: &str, port: u16) -> bool {
        self.allow.iter().any(|o| o.allows(host, port))
    }
}

/// A policy could not be built.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// An origin did not parse.
    #[error("not an origin: {0}")]
    Origin(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_hosts_and_default_ports() {
        let o = Origin::parse("kilele.example").unwrap();
        assert!(o.allows("KILELE.example.", 443));
        assert!(o.allows("kilele.example", 80));
        assert!(!o.allows("kilele.example", 22));
        assert!(!o.allows("evil-kilele.example", 443));
        assert!(!o.allows("kilele.example.evil.test", 443));
    }

    #[test]
    fn wildcards_cover_subdomains_only() {
        let o = Origin::parse("*.kilele.example").unwrap();
        assert!(o.allows("www.kilele.example", 443));
        assert!(o.allows("a.b.kilele.example", 443));
        assert!(!o.allows("kilele.example", 443));
        assert!(!o.allows("xkilele.example", 443));
    }

    #[test]
    fn explicit_ports_and_urls() {
        let o = Origin::parse("http://127.0.0.1:8080/path").unwrap();
        assert!(o.allows("127.0.0.1", 8080));
        assert!(!o.allows("127.0.0.1", 80));
        assert_eq!(o.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "*", "a*b.example", "host:notaport", "two words"] {
            assert!(Origin::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }
}
