//! Whether each sample site answers.
//!
//! W2 serves the samples from its own crate on their `-sample` hosts, not as signed releases,
//! so there is no `site_pointer` row to read. The honest test is the one a visitor makes: ask
//! the host for its front page. A background task does that once a minute, and the home page
//! links a sample only while it answers; otherwise the card says it is opening soon.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::future::BoxFuture;

/// How long one sample may take to answer.
const TIMEOUT: Duration = Duration::from_secs(5);
/// How often the samples are asked.
const EVERY: Duration = Duration::from_secs(60);
/// How soon to look again while no sample answers (node starting, a sample restarting).
const SOON: Duration = Duration::from_secs(5);

/// The sample hosts that answered the last check.
#[derive(Clone, Default)]
pub struct SampleProbe {
    open: Arc<RwLock<Vec<String>>>,
}

impl SampleProbe {
    /// A probe that has seen nothing answer yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The hosts that answered the last check.
    pub fn open(&self) -> Vec<String> {
        self.open.read().map(|o| o.clone()).unwrap_or_default()
    }

    /// Records which hosts answer (the check, the preview and tests).
    pub fn set_open(&self, hosts: Vec<String>) {
        if let Ok(mut open) = self.open.write() {
            *open = hosts;
        }
    }

    /// Asks each host once, on the platform `origin`'s scheme and port (a local node is
    /// `http://…:18050`); a 2xx answer within five seconds is open. A `*.localhost` host is
    /// asked on 127.0.0.1, since macOS doesn't resolve those names.
    pub async fn check(&self, http: &reqwest::Client, hosts: &[String], origin: &str) {
        let local = local_client(hosts);
        let http = local.as_ref().unwrap_or(http);
        let mut open = Vec::new();
        for host in hosts {
            let answer = tokio::time::timeout(
                TIMEOUT,
                http.get(crate::view::site_url(origin, host)).send(),
            );
            match answer.await {
                Ok(Ok(res)) if res.status().is_success() => open.push(host.clone()),
                Ok(Ok(res)) => {
                    tracing::debug!(host, status = %res.status(), "home: a sample isn't answering yet")
                }
                Ok(Err(e)) => {
                    tracing::debug!(host, error = %e, "home: a sample isn't answering yet")
                }
                Err(_) => tracing::debug!(host, "home: a sample took too long to answer"),
            }
        }
        self.set_open(open);
    }

    /// The background task: check now, then every minute.
    pub fn task(
        self,
        http: reqwest::Client,
        hosts: Vec<String>,
        origin: String,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            loop {
                self.check(&http, &hosts, &origin).await;
                // The first checks run as node starts, before it listens: while nothing
                // answers, look again soon rather than leaving the cards closed for a minute.
                let wait = if self.open().is_empty() { SOON } else { EVERY };
                tokio::time::sleep(wait).await;
            }
        })
    }
}

/// `localhost` or a name under it, which always means this machine (RFC 6761).
fn is_local(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// A client that resolves the `*.localhost` hosts among `hosts` to 127.0.0.1, keeping the URL's
/// port and the host's name for the request; none when no host is local.
fn local_client(hosts: &[String]) -> Option<reqwest::Client> {
    let local: Vec<&String> = hosts.iter().filter(|h| is_local(h)).collect();
    if local.is_empty() {
        return None;
    }
    let here = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
    local
        .into_iter()
        .fold(reqwest::Client::builder(), |b, h| b.resolve(h, here))
        .build()
        .inspect_err(|e| tracing::warn!(error = %e, "home: a client for the local samples"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_local_sample_is_asked_on_this_machine_by_its_own_name() {
        use axum::http::{HeaderMap, header::HOST};
        let seen: Arc<RwLock<Vec<String>>> = Arc::default();
        let log = seen.clone();
        let app = axum::Router::new().fallback(move |headers: HeaderMap| {
            let log = log.clone();
            async move {
                if let Some(h) = headers.get(HOST).and_then(|h| h.to_str().ok()) {
                    log.write().unwrap().push(h.to_owned());
                }
                "a sample"
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let probe = SampleProbe::new();
        probe
            .check(
                &reqwest::Client::new(),
                &["kilele-bakery-sample.localhost".into()],
                &format!("http://localhost:{port}"),
            )
            .await;
        assert_eq!(probe.open(), ["kilele-bakery-sample.localhost"]);
        assert_eq!(
            seen.read().unwrap().as_slice(),
            [format!("kilele-bakery-sample.localhost:{port}")],
            "the request names the sample, so node routes it to the sample"
        );
        assert!(is_local("localhost") && is_local("X.LOCALHOST."));
        assert!(!is_local("kilele-bakery-sample.agentrix.website") && !is_local("notlocalhost"));
    }

    #[tokio::test]
    async fn a_host_that_cannot_answer_stays_closed() {
        let probe = SampleProbe::new();
        probe.set_open(vec!["stale.invalid".into()]);
        probe
            .check(
                &reqwest::Client::new(),
                &["nothing-here.invalid".into()],
                "https://agentrix.website",
            )
            .await;
        assert!(
            probe.open().is_empty(),
            "the last check replaces the old answer"
        );
    }
}
