//! The apps' seams over HTTP, for when the work behind them runs in another process
//! (contracts: the shared notes' `face-v3-contracts.md`).

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use saas_core::identity::Badge;
use saas_core::wallet::WalletFeed;

use super::{
    AppError, Badges, Ceremonies, DayRunner, Moment, Sealed, Subject, Verification, Wallets,
};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default()
}

async fn answer<T: DeserializeOwned>(
    res: Result<reqwest::Response, reqwest::Error>,
) -> Result<T, AppError> {
    let res = res.map_err(|e| AppError::Refused(format!("The service didn't answer: {e}")))?;
    let status = res.status();
    if status.is_success() {
        return res.json().await.map_err(|e| {
            AppError::Refused(format!("The service's answer wasn't understood: {e}"))
        });
    }
    let body: Value = res.json().await.unwrap_or(Value::Null);
    let message = body
        .get("message")
        .or_else(|| body.get("error"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Err(match status.as_u16() {
        404 => AppError::NotFound,
        409 => AppError::Changed,
        410 => AppError::Expired,
        _ => {
            AppError::Refused(message.unwrap_or_else(|| format!("The service answered {status}.")))
        }
    })
}

fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Settle and saas-badge over HTTP: the moments at `{base}/owner/{site}/ceremonies/…`,
/// verify at `{base}/agents/verify?did=`, and on each site's own host its agents at
/// `{origin}/agents/index.json` and each one's signed badge at the path the index gives
/// (`/agents/{role}/badge.json`).
pub struct HttpSettle {
    base: String,
    sites: HashMap<String, String>,
    http: reqwest::Client,
}

/// One row of a site's `/agents/index.json`.
#[derive(serde::Deserialize)]
struct IndexRow {
    role: String,
    /// The badge's path.
    #[serde(default)]
    badge: Option<String>,
}

/// A signed badge document is the badge body plus a detached-JWS `proof`: the body.
pub fn badge_body(mut signed: Value) -> Result<Badge, serde_json::Error> {
    if let Some(o) = signed.as_object_mut() {
        o.remove("proof");
    }
    serde_json::from_value(signed)
}

impl HttpSettle {
    /// Settle at `base`; `sites` maps a site id to its origin (`kilele` →
    /// `https://kilele-sample.agentrix.website`).
    pub fn new(base: &str, sites: HashMap<String, String>) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            sites,
            http: client(),
        }
    }

    fn moment_url(&self, site: &str, moment: Moment, id: &str) -> String {
        format!(
            "{}/owner/{}/ceremonies/{}/{}",
            self.base,
            seg(site),
            moment.as_str(),
            seg(id)
        )
    }
}

#[async_trait]
impl Ceremonies for HttpSettle {
    async fn subject(&self, site: &str, moment: Moment, id: &str) -> Result<Subject, AppError> {
        answer(
            self.http
                .get(self.moment_url(site, moment, id))
                .send()
                .await,
        )
        .await
    }

    async fn seal(
        &self,
        site: &str,
        moment: Moment,
        id: &str,
        evidence: Value,
    ) -> Result<Sealed, AppError> {
        answer(
            self.http
                .post(self.moment_url(site, moment, id))
                .json(&json!({ "evidence": evidence }))
                .send()
                .await,
        )
        .await
    }

    async fn pending(&self, site: &str) -> Result<Vec<Subject>, AppError> {
        answer(
            self.http
                .get(format!("{}/owner/{}/ceremonies", self.base, seg(site)))
                .send()
                .await,
        )
        .await
    }
}

#[async_trait]
impl Badges for HttpSettle {
    async fn badges(&self, site: &str) -> Result<Vec<Badge>, AppError> {
        let origin = self.sites.get(site).ok_or(AppError::NotFound)?;
        let origin = origin.trim_end_matches('/');
        let index: Vec<IndexRow> = answer(
            self.http
                .get(format!("{origin}/agents/index.json"))
                .send()
                .await,
        )
        .await?;
        let mut out = Vec::new();
        for row in index {
            // Only a path on the site's own host: a badge is the site's to publish.
            let path = row
                .badge
                .filter(|p| p.starts_with('/') && !p.starts_with("//"))
                .unwrap_or_else(|| format!("/agents/{}/badge.json", seg(&row.role)));
            let signed: Value =
                answer(self.http.get(format!("{origin}{path}")).send().await).await?;
            out.push(badge_body(signed).map_err(|e| {
                AppError::Refused(format!("{}'s badge wasn't understood: {e}", row.role))
            })?);
        }
        Ok(out)
    }

    async fn verify(&self, did: &str) -> Result<Verification, AppError> {
        let report: Verification = answer(
            self.http
                .get(format!("{}/agents/verify", self.base))
                .query(&[("did", did)])
                .send()
                .await,
        )
        .await?;
        Ok(report.settled())
    }

    async fn verify_token(&self, token: &str) -> Result<Value, AppError> {
        answer(
            self.http
                .post(format!("{}/agents/verify-token", self.base))
                .json(&json!({ "token": token }))
                .send()
                .await,
        )
        .await
    }
}

/// The wallet feed over HTTP: `{base}/wallet/{site}.json`, a `saas_core::wallet::WalletFeed`.
pub struct HttpWallet {
    base: String,
    http: reqwest::Client,
}

impl HttpWallet {
    /// The feed at `base`.
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            http: client(),
        }
    }
}

#[async_trait]
impl Wallets for HttpWallet {
    async fn wallet(&self, site: &str) -> Result<WalletFeed, AppError> {
        answer(
            self.http
                .get(format!("{}/wallet/{}.json", self.base, seg(site)))
                .send()
                .await,
        )
        .await
    }
}

/// "Run the day" over HTTP: `POST {base}/judge/run-day`, `GET {base}/judge/run-day/{run}`.
pub struct HttpDay {
    base: String,
    http: reqwest::Client,
}

impl HttpDay {
    /// The runner at `base`.
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            http: client(),
        }
    }
}

#[async_trait]
impl DayRunner for HttpDay {
    async fn start(&self) -> Result<String, AppError> {
        let v: Value = answer(
            self.http
                .post(format!("{}/judge/run-day", self.base))
                .send()
                .await,
        )
        .await?;
        v.get("run")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AppError::Refused("The run didn't say its id.".into()))
    }

    async fn status(&self, run: &str) -> Result<Value, AppError> {
        answer(
            self.http
                .get(format!("{}/judge/run-day/{}", self.base, seg(run)))
                .send()
                .await,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_are_escaped() {
        assert_eq!(seg("bind_1"), "bind_1");
        assert_eq!(seg("../x y"), "..%2Fx%20y");
        let s = HttpSettle::new("http://127.0.0.1:9/", HashMap::new());
        assert_eq!(
            s.moment_url("kilele", Moment::StepUp, "dec 1"),
            "http://127.0.0.1:9/owner/kilele/ceremonies/step-up/dec%201"
        );
    }
}
