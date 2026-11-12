//! What the samples need from other workstreams, as traits, with stand-ins.

use async_trait::async_trait;
use axum::http::HeaderMap;
use futures::stream::BoxStream;
use serde::Serialize;

/// The site a request is for (the same fields as W6's `SiteHost`).
#[derive(Debug, Clone, Serialize)]
pub struct SampleHost {
    /// The site's slug.
    pub site: String,
    /// The host as served.
    pub host: String,
    /// `https://<host>` (or `http://<host>:<port>` in previews).
    pub origin: String,
    /// The business name.
    pub name: String,
    /// Always true here.
    pub sample: bool,
}

/// A signed-in customer.
#[derive(Debug, Clone, Serialize)]
pub struct Visitor {
    /// The account id (W6's `vac_…`, or the stand-in's `cus-…`).
    pub account: String,
    /// Name.
    pub name: String,
    /// Email.
    pub email: String,
    /// Proven by a one-time code (ordering needs it).
    pub email_verified: bool,
    /// Phone, if given.
    pub phone: Option<String>,
}

/// Customer sign-in on the site's own host (W6).
#[async_trait]
pub trait Visitors: Send + Sync {
    /// Who is signed in, if anyone.
    async fn visitor(
        &self,
        site: &SampleHost,
        headers: &HeaderMap,
    ) -> anyhow::Result<Option<Visitor>>;
    /// Whether this is the samples' own stand-in (which then serves `/account/sign-in`).
    fn is_stand_in(&self) -> bool {
        false
    }
}

/// A payment to start with the provider.
#[derive(Debug, Clone, Serialize)]
pub struct PaymentRequest {
    /// The site.
    pub site: String,
    /// The order.
    pub order_id: String,
    /// US cents at the order's pinned rate.
    pub amount_usd_cents: i64,
    /// "Kilele Bakery order KB-1042 (Sample)".
    pub description: String,
    /// The customer's account, if signed in.
    pub customer: Option<String>,
    /// Where PayPal returns her.
    pub return_url: String,
    /// Where PayPal sends her on cancel.
    pub cancel_url: String,
}

/// What starting a payment gave.
#[derive(Debug, Clone, Serialize)]
pub enum PaymentStart {
    /// Send the customer to `approve_url`.
    Redirect {
        /// Our payment id.
        payment_id: String,
        /// The provider's order id.
        provider_ref: String,
        /// PayPal's approval page.
        approve_url: String,
    },
    /// Payments aren't switched on; the words say so.
    NotExecuted {
        /// Visitor words.
        words: String,
    },
}

/// What a capture gave.
#[derive(Debug, Clone, Serialize)]
pub struct Capture {
    /// `captured`, `pending`, `declined`, `cancelled`, `not_executed`.
    pub state: String,
    /// The provider's capture id.
    pub provider_capture_id: Option<String>,
    /// The receipt id, if one was written.
    pub receipt: Option<String>,
}

/// PayPal sandbox through the Effects Gateway (W6).
#[async_trait]
pub trait Payments: Send + Sync {
    /// Starts a payment. Idempotent per site, order and amount.
    async fn create(&self, req: PaymentRequest) -> anyhow::Result<PaymentStart>;
    /// Captures an approved payment. Idempotent.
    async fn capture(&self, site: &str, provider_ref: &str) -> anyhow::Result<Capture>;
    /// Whether this is the samples' own stand-in (a labelled sample payment step).
    fn is_stand_in(&self) -> bool {
        false
    }
}

/// The Model Gateway's `concierge` purpose.
#[async_trait]
pub trait AnswerModel: Send + Sync {
    /// Streams an answer to a system and user prompt.
    async fn answer(
        &self,
        site: &str,
        system: &str,
        user: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>>;

    /// One whole reply as a JSON object that fits `schema` (the Cakes desk's turn). The
    /// default collects [`AnswerModel::answer`], so a stand-in needs only that.
    async fn reply(
        &self,
        site: &str,
        system: &str,
        user: &str,
        schema: &serde_json::Value,
    ) -> anyhow::Result<String> {
        use futures::StreamExt as _;
        let _ = schema;
        let mut stream = self.answer(site, system, user).await?;
        let mut text = String::new();
        while let Some(piece) = stream.next().await {
            text.push_str(&piece?);
        }
        Ok(text)
    }
}

static PAYMENTS: std::sync::OnceLock<std::sync::Arc<dyn Payments>> = std::sync::OnceLock::new();

/// Registers node's real payments (PayPal sandbox through the Effects Gateway) for every sample
/// site. The first registration wins; `false` if one was already set. Until then, and on a node
/// without a PayPal app, the samples keep the labelled sample step.
pub fn set_payments(payments: std::sync::Arc<dyn Payments>) -> bool {
    PAYMENTS.set(payments).is_ok()
}

/// The payments node registered with [`set_payments`], or the stand-in until it does: read at
/// each call, so node can register after the samples are mounted.
pub struct RegisteredPayments;

#[async_trait]
impl Payments for RegisteredPayments {
    async fn create(&self, req: PaymentRequest) -> anyhow::Result<PaymentStart> {
        match PAYMENTS.get() {
            Some(p) => p.create(req).await,
            None => SamplePayments.create(req).await,
        }
    }
    async fn capture(&self, site: &str, provider_ref: &str) -> anyhow::Result<Capture> {
        match PAYMENTS.get() {
            Some(p) => p.capture(site, provider_ref).await,
            None => SamplePayments.capture(site, provider_ref).await,
        }
    }
    fn is_stand_in(&self) -> bool {
        PAYMENTS.get().is_none_or(|p| p.is_stand_in())
    }
}

/// The stand-in payments: no provider is called; the pay page offers a labelled sample step.
pub struct SamplePayments;

#[async_trait]
impl Payments for SamplePayments {
    async fn create(&self, _req: PaymentRequest) -> anyhow::Result<PaymentStart> {
        Ok(PaymentStart::NotExecuted {
            words: "PayPal sandbox isn't connected on this server yet.".into(),
        })
    }
    async fn capture(&self, _site: &str, _provider_ref: &str) -> anyhow::Result<Capture> {
        Ok(Capture {
            state: "not_executed".into(),
            provider_capture_id: None,
            receipt: None,
        })
    }
    fn is_stand_in(&self) -> bool {
        true
    }
}

/// The Model Gateway over HTTP (`POST /v1/chat/completions`, purpose `concierge`).
pub struct GatewayModel {
    base: String,
    http: reqwest::Client,
}

impl GatewayModel {
    /// A client for the gateway at `base`.
    pub fn new(base: &str, http: reqwest::Client) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            http,
        }
    }
}

#[async_trait]
impl AnswerModel for GatewayModel {
    async fn answer(
        &self,
        site: &str,
        system: &str,
        user: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        use futures::StreamExt as _;
        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .header("x-agentrix-site", site)
            .header("x-agentrix-purpose", "concierge")
            .timeout(std::time::Duration::from_secs(25))
            .json(&serde_json::json!({
                "stream": true,
                "temperature": 0.1,
                "max_tokens": 320,
                "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            }))
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = response.bytes_stream();
        let (tx, rx) = tokio::sync::mpsc::channel::<anyhow::Result<String>>(32);
        tokio::spawn(async move {
            let mut buf = String::new();
            while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(e.into())).await;
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(nl) = buf.find('\n') {
                    let line: String = buf.drain(..=nl).collect();
                    let Some(data) = line.trim().strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" {
                        return;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data)
                        && let Some(piece) = v
                            .pointer("/choices/0/delta/content")
                            .and_then(|p| p.as_str())
                        && !piece.is_empty()
                        && tx.send(Ok(piece.to_owned())).await.is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok(tokio_stream::wrappers::ReceiverStream::new(rx).boxed())
    }

    async fn reply(
        &self,
        site: &str,
        system: &str,
        user: &str,
        schema: &serde_json::Value,
    ) -> anyhow::Result<String> {
        // Not streamed: the page needs the whole turn to check it. No max_tokens, so the
        // gateway gives `concierge` its room for reasoning (4,096).
        let answer: serde_json::Value = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .header("x-agentrix-site", site)
            .header("x-agentrix-purpose", "concierge")
            .timeout(std::time::Duration::from_secs(20))
            .json(&serde_json::json!({
                "temperature": 0.4,
                "reasoning_effort": "low",
                "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                "response_format": { "type": "json_schema", "json_schema": {
                    "name": "cakes_desk_turn", "strict": true, "schema": schema
                }},
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        answer
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("the gateway's answer had no content"))
    }
}
