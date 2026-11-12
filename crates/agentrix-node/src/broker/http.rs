//! The runner over HTTP on the VPC (design §2: `agentrix-runner` on the sandbox VM at :7443,
//! reachable from control's VPC address only).

use std::time::Duration;

use agentrix_types::{Digest, JobEvent, SignedJobSpec};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use futures::StreamExt as _;
use futures::stream::BoxStream;

use super::{BrokerError, EventStream, MAX_ARTIFACT, RunnerApi};

/// Longest NDJSON line accepted from the runner.
const MAX_LINE: usize = 1024 * 1024;

/// The real runner.
#[derive(Debug, Clone)]
pub struct HttpRunner {
    base: String,
    http: reqwest::Client,
}

impl HttpRunner {
    /// A client for the runner at `base`, e.g. `http://10.1.96.4:7443`.
    pub fn new(base: impl Into<String>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            base: base.into().trim_end_matches('/').to_owned(),
            http,
        })
    }

    /// Reads `AGENTRIX_RUNNER_URL`.
    pub fn from_env() -> anyhow::Result<Self> {
        let base = std::env::var("AGENTRIX_RUNNER_URL")
            .map_err(|_| anyhow::anyhow!("AGENTRIX_RUNNER_URL is not set"))?;
        Self::new(base)
    }
}

fn unreachable(e: reqwest::Error) -> BrokerError {
    BrokerError::Unreachable(e.to_string())
}

/// The runner's `{"error": "…"}` body, or the status line.
async fn refused(resp: reqwest::Response) -> BrokerError {
    let status = resp.status().as_u16();
    let message = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
        .unwrap_or_else(|| format!("HTTP {status}"));
    BrokerError::Refused { status, message }
}

#[async_trait]
impl RunnerApi for HttpRunner {
    async fn put_blob(&self, digest: &Digest, bytes: Bytes) -> Result<(), BrokerError> {
        let resp = self
            .http
            .put(format!("{}/blobs/{digest}", self.base))
            .body(bytes)
            .send()
            .await
            .map_err(unreachable)?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(refused(resp).await)
        }
    }

    async fn post_job(&self, signed: &SignedJobSpec) -> Result<EventStream, BrokerError> {
        let resp = self
            .http
            .post(format!("{}/jobs", self.base))
            .json(signed)
            .send()
            .await
            .map_err(unreachable)?;
        if !resp.status().is_success() {
            return Err(refused(resp).await);
        }
        let chunks = resp
            .bytes_stream()
            .map(|c| c.map_err(|e| BrokerError::Unreachable(e.to_string())))
            .boxed();
        Ok(ndjson(chunks))
    }

    async fn get_artifact(&self, digest: &Digest) -> Result<Bytes, BrokerError> {
        let resp = self
            .http
            .get(format!("{}/artifacts/{digest}", self.base))
            .send()
            .await
            .map_err(unreachable)?;
        if !resp.status().is_success() {
            return Err(refused(resp).await);
        }
        if resp
            .content_length()
            .is_some_and(|n| n > MAX_ARTIFACT as u64)
        {
            return Err(BrokerError::Artifact(format!("{digest} is too large")));
        }
        resp.bytes().await.map_err(unreachable)
    }
}

/// Splits a byte stream into NDJSON lines and parses each as a [`JobEvent`]. Blank lines are
/// skipped; a line over [`MAX_LINE`] or one that does not parse ends the stream with an error.
pub fn ndjson(chunks: BoxStream<'static, Result<Bytes, BrokerError>>) -> EventStream {
    struct St {
        chunks: BoxStream<'static, Result<Bytes, BrokerError>>,
        buf: BytesMut,
        done: bool,
    }
    let st = St {
        chunks,
        buf: BytesMut::new(),
        done: false,
    };
    futures::stream::unfold(st, |mut st| async move {
        loop {
            if let Some(i) = st.buf.iter().position(|&b| b == b'\n') {
                let line = st.buf.split_to(i + 1);
                let text = String::from_utf8_lossy(&line[..i]);
                if text.trim().is_empty() {
                    continue;
                }
                let ev = JobEvent::parse(text.trim())
                    .map_err(|e| BrokerError::Protocol(format!("bad event line: {e}")));
                if ev.is_err() {
                    st.done = true;
                }
                return Some((ev, st));
            }
            if st.done {
                return None;
            }
            if st.buf.len() > MAX_LINE {
                st.done = true;
                return Some((
                    Err(BrokerError::Protocol("an event line is too long".into())),
                    st,
                ));
            }
            match st.chunks.next().await {
                Some(Ok(chunk)) => st.buf.extend_from_slice(&chunk),
                Some(Err(e)) => {
                    st.done = true;
                    return Some((Err(e), st));
                }
                None => {
                    st.done = true;
                    let rest = String::from_utf8_lossy(&st.buf).trim().to_owned();
                    st.buf.clear();
                    if rest.is_empty() {
                        return None;
                    }
                    let ev = JobEvent::parse(&rest)
                        .map_err(|e| BrokerError::Protocol(format!("bad event line: {e}")));
                    return Some((ev, st));
                }
            }
        }
    })
    .boxed()
}
