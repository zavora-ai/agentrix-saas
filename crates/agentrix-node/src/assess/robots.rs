//! robots.txt (RFC 9309): which paths the crawler may read, its crawl delay and sitemaps.

/// One group of rules.
#[derive(Debug, Clone, Default, PartialEq)]
struct Group {
    agents: Vec<String>,
    rules: Vec<(bool, String)>,
    delay: Option<f64>,
}

/// A parsed robots.txt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Robots {
    groups: Vec<Group>,
    /// `Sitemap:` lines.
    pub sitemaps: Vec<String>,
}

impl Robots {
    /// Everything allowed (robots.txt missing, or a 4xx).
    pub fn allow_all() -> Self {
        Self::default()
    }

    /// Everything disallowed (robots.txt unreachable or a 5xx: RFC 9309 §2.3.1.4).
    pub fn disallow_all() -> Self {
        Self {
            groups: vec![Group {
                agents: vec!["*".into()],
                rules: vec![(false, "/".into())],
                delay: None,
            }],
            sitemaps: Vec::new(),
        }
    }

    /// Parses robots.txt text.
    pub fn parse(text: &str) -> Self {
        let mut robots = Self::default();
        let mut current: Option<Group> = None;
        let mut last_was_agent = false;
        for line in text.lines().take(10_000) {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match key.as_str() {
                "user-agent" => {
                    if !last_was_agent && let Some(g) = current.take() {
                        robots.groups.push(g);
                    }
                    current
                        .get_or_insert_with(Group::default)
                        .agents
                        .push(value.to_ascii_lowercase());
                    last_was_agent = true;
                }
                "allow" | "disallow" => {
                    last_was_agent = false;
                    if let Some(g) = current.as_mut()
                        && !(key == "disallow" && value.is_empty())
                    {
                        g.rules.push((key == "allow", value.to_owned()));
                    }
                }
                "crawl-delay" => {
                    last_was_agent = false;
                    if let Some(g) = current.as_mut() {
                        g.delay = value.parse().ok();
                    }
                }
                "sitemap" => robots.sitemaps.push(value.to_owned()),
                _ => last_was_agent = false,
            }
        }
        if let Some(g) = current {
            robots.groups.push(g);
        }
        robots
    }

    /// The group that applies to `token`: the one naming it, else `*`.
    fn group(&self, token: &str) -> Option<&Group> {
        let token = token.to_ascii_lowercase();
        self.groups
            .iter()
            .find(|g| {
                g.agents
                    .iter()
                    .any(|a| a != "*" && token.starts_with(a.as_str()))
            })
            .or_else(|| {
                self.groups
                    .iter()
                    .find(|g| g.agents.iter().any(|a| a == "*"))
            })
    }

    /// Whether `token` may fetch `path` (path and query). The longest matching rule wins; on
    /// a tie, allow wins.
    pub fn allows(&self, token: &str, path: &str) -> bool {
        if path == "/robots.txt" {
            return true;
        }
        let Some(g) = self.group(token) else {
            return true;
        };
        let mut best: Option<(usize, bool)> = None;
        for (allow, pattern) in &g.rules {
            if matches(pattern, path) {
                let len = pattern.len();
                best = match best {
                    Some((l, a)) if l > len || (l == len && a) => Some((l, a)),
                    _ => Some((len, *allow)),
                };
            }
        }
        best.is_none_or(|(_, allow)| allow)
    }

    /// Whether the rules that apply to `token` shut it out of the whole site.
    pub fn blocks_site(&self, token: &str) -> bool {
        !self.allows(token, "/")
    }

    /// The crawl delay for `token`, in seconds.
    pub fn crawl_delay(&self, token: &str) -> Option<f64> {
        self.group(token).and_then(|g| g.delay)
    }
}

/// RFC 9309 matching: `*` matches any run of characters, a final `$` anchors the end;
/// otherwise the pattern matches a prefix of the path.
fn matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let (p, s) = (pattern.as_bytes(), path.as_bytes());
    let (mut pi, mut si) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    loop {
        if pi == p.len() {
            if !anchored || si == s.len() {
                return true;
            }
        } else if p[pi] == b'*' {
            star = Some(pi);
            mark = si;
            pi += 1;
            continue;
        } else if si < s.len() && p[pi] == s[si] {
            pi += 1;
            si += 1;
            continue;
        }
        match star {
            Some(sp) if mark < s.len() => {
                pi = sp + 1;
                mark += 1;
                si = mark;
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_apply_by_agent_and_longest_match() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /wp-admin/\nAllow: /wp-admin/admin-ajax.php\nCrawl-delay: 2\n\n\
             User-agent: GPTBot\nUser-agent: ClaudeBot\nDisallow: /\n\nSitemap: https://k.example/sitemap.xml\n",
        );
        assert!(r.allows("AgentrixBot", "/"));
        assert!(!r.allows("AgentrixBot", "/wp-admin/x"));
        assert!(r.allows("AgentrixBot", "/wp-admin/admin-ajax.php"));
        assert!(r.blocks_site("GPTBot"));
        assert!(r.blocks_site("ClaudeBot"));
        assert_eq!(r.crawl_delay("AgentrixBot"), Some(2.0));
        assert_eq!(
            r.sitemaps,
            vec!["https://k.example/sitemap.xml".to_string()]
        );
    }

    #[test]
    fn wildcards_and_empty_disallow() {
        let r = Robots::parse("User-agent: agentrixbot\nDisallow: /*.pdf$\nDisallow: /private*\n");
        assert!(!r.allows("AgentrixBot", "/a/b.pdf"));
        assert!(r.allows("AgentrixBot", "/a/b.pdf?x=1"));
        assert!(!r.allows("AgentrixBot", "/private-stuff"));
        let open = Robots::parse("User-agent: *\nDisallow:\n");
        assert!(open.allows("AgentrixBot", "/anything"));
        assert!(Robots::disallow_all().blocks_site("AgentrixBot"));
        assert!(Robots::disallow_all().allows("AgentrixBot", "/robots.txt"));
    }
}
