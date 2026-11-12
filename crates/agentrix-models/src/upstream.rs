//! The one connection to Vultr Serverless Inference, and the only place the key lives (R11.1).

use std::fmt;
use std::path::Path;
use std::time::Duration;

use rand::Rng as _;
use reqwest::StatusCode;
use serde_json::Value;

/// The Vultr Inference key. Its `Debug` output never shows it.
pub struct ApiKey(String);

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<withheld>)")
    }
}

impl ApiKey {
    /// `VULTR_INFERENCE_API_KEY`, from the environment or a dotenv file.
    pub fn load(env_file: Option<&Path>) -> anyhow::Result<Self> {
        read_key("VULTR_INFERENCE_API_KEY", env_file).map(Self)
    }

    #[cfg(test)]
    pub fn for_tests(key: &str) -> Self {
        Self(key.to_owned())
    }
}

/// A provider key from the environment (systemd's `EnvironmentFile` in
/// `/etc/agentrix/secrets/`), or else from that one line of a dotenv file. Never logged.
pub fn read_key(name: &str, env_file: Option<&Path>) -> anyhow::Result<String> {
    if let Ok(key) = std::env::var(name)
        && !key.trim().is_empty()
    {
        return Ok(key.trim().to_owned());
    }
    let Some(path) = env_file else {
        anyhow::bail!("{name} is not set, and no --env-file was given");
    };
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    text.lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix(name)?
                .trim_start()
                .strip_prefix('=')
        })
        .map(|value| value.trim().trim_matches('"').trim_matches('\''))
        .find(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{} has no {name} line", path.display()))
}

/// Why a call to Vultr did not produce an answer worth passing on.
#[derive(Debug)]
pub enum UpstreamError {
    /// 429, 5xx or 529 on every try, or no connection; nothing was switched elsewhere.
    Busy {
        tries: u32,
        last_status: Option<u16>,
        waited: Duration,
    },
    /// 402: the account is not funded for this model. Not retried.
    Unfunded,
}

/// Vultr Serverless Inference, with backoff.
#[derive(Debug)]
pub struct Vultr {
    http: reqwest::Client,
    base_url: String,
    key: ApiKey,
    tries: u32,
    base_delay: Duration,
}

impl Vultr {
    pub fn new(base_url: impl Into<String>, key: ApiKey) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            key,
            tries: 3,
            base_delay: Duration::from_millis(500),
        })
    }

    /// Shorter waits, for tests.
    #[cfg(test)]
    pub fn with_base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = delay;
        self
    }

    /// Posts `body` to `path`, retrying 429, 5xx (including 529) and connection failures up to
    /// three tries with exponential backoff and jitter (R11.4). Any other answer, success or
    /// the caller's own 4xx, is returned as it came.
    pub async fn post(&self, path: &str, body: &Value) -> Result<reqwest::Response, UpstreamError> {
        let url = format!("{}/{path}", self.base_url);
        let mut waited = Duration::ZERO;
        let mut last_status = None;
        for attempt in 0..self.tries {
            let sent = self
                .http
                .post(&url)
                .bearer_auth(&self.key.0)
                .json(body)
                .send()
                .await;
            let retry_after = match sent {
                Ok(response) => {
                    let status = response.status();
                    if status == StatusCode::PAYMENT_REQUIRED {
                        return Err(UpstreamError::Unfunded);
                    }
                    if !(status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) {
                        return Ok(response);
                    }
                    last_status = Some(status.as_u16());
                    tracing::warn!(%status, attempt, path, "Vultr is busy; backing off");
                    retry_after(&response)
                }
                Err(err) => {
                    tracing::warn!(error = %err, attempt, path, "Vultr unreachable; backing off");
                    None
                }
            };
            if attempt + 1 < self.tries {
                let delay = self.delay(attempt, retry_after);
                waited += delay;
                tokio::time::sleep(delay).await;
            }
        }
        Err(UpstreamError::Busy {
            tries: self.tries,
            last_status,
            waited,
        })
    }

    /// `base × 2^attempt`, with ±50 % jitter, at least any `Retry-After`, at most 10 s.
    fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        let exp = self.base_delay.saturating_mul(1 << attempt.min(5));
        let jitter = rand::thread_rng().gen_range(0.5..1.5);
        let delay = exp.mul_f64(jitter).max(retry_after.unwrap_or_default());
        delay.min(Duration::from_secs(10))
    }
}

fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

mod realtime;
