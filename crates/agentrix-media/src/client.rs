//! The Model Gateway's `image` routes, called by purpose (02 §6). The gateway is the only
//! holder of model keys; this crate never calls Vultr.

use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

/// One picture as the gateway returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawn {
    /// The image bytes.
    pub bytes: Vec<u8>,
    /// `image/webp`, `image/png` or `image/jpeg`.
    pub media_type: String,
    /// The model that drew it (`x-agentrix-model`).
    pub model: String,
    /// `WIDTHxHEIGHT`, when the gateway says.
    pub size: Option<String>,
    /// This picture's share of what the call cost, in micro-USD.
    pub cost_micro_usd: i64,
}

/// The content-safety model's verdict on a picture.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct Verdict {
    /// Only an explicit "safe" is safe.
    pub safe: bool,
    /// What the model said, lowercase.
    pub user_safety: Option<String>,
    /// The categories it named.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Which model judged.
    pub model: String,
}

/// What a vision model says a picture shows.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct Description {
    /// The alt text.
    pub alt: String,
    /// Whether a visitor could take it for a photograph of a real scene.
    pub photographic: bool,
    /// Whether a human face or body appears.
    pub shows_person: bool,
    /// Whether letters, numbers or logos appear.
    pub shows_text: bool,
    /// Anything the owner should look at before it goes on the site.
    #[serde(default)]
    pub check: String,
}

/// What the caller asks the gateway to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawRequest {
    /// The full prompt (after [`crate::policy`]).
    pub prompt: String,
    /// `WIDTHxHEIGHT`.
    pub size: String,
    /// How many, 1 to 4.
    pub n: u8,
    /// For a repeatable picture.
    pub seed: Option<i64>,
    /// `fast` (drafts) or `standard` (detailed).
    pub tier: &'static str,
}

/// The gateway, for one site.
#[derive(Clone)]
pub struct Gateway {
    http: reqwest::Client,
    base: String,
    site: String,
}

/// Why a gateway call did not give a usable answer, in words an owner can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayError {
    /// The gateway's error code, or `unreachable`.
    pub code: String,
    /// The gateway's message (written for people).
    pub message: String,
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for GatewayError {}

impl Gateway {
    /// The gateway at `base` (node's `models_url`), calling for `site`.
    pub fn new(http: reqwest::Client, base: &str, site: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            site: site.to_owned(),
        }
    }

    async fn post(
        &self,
        path: &str,
        tier: Option<&str>,
        body: &Value,
    ) -> Result<(Value, Option<String>), GatewayError> {
        let mut request = self
            .http
            .post(format!("{}{path}", self.base))
            .timeout(Duration::from_secs(180))
            .header("x-agentrix-site", &self.site)
            .header("x-agentrix-purpose", "image");
        if let Some(tier) = tier {
            request = request.header("x-agentrix-tier", tier);
        }
        let response = request.json(body).send().await.map_err(|e| GatewayError {
            code: "unreachable".into(),
            message: format!("The model gateway did not answer: {e}"),
        })?;
        let status = response.status();
        let model = response
            .headers()
            .get("x-agentrix-model")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(GatewayError {
                code: value["error"]["code"]
                    .as_str()
                    .unwrap_or("gateway_error")
                    .to_owned(),
                message: value["error"]["message"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("The model gateway answered HTTP {status}.")),
            });
        }
        Ok((value, model))
    }

    /// Draws `request.n` pictures.
    pub async fn draw(&self, request: &DrawRequest) -> Result<Vec<Drawn>, GatewayError> {
        let mut body = json!({
            "prompt": request.prompt,
            "size": request.size,
            "n": request.n,
            "output_format": "webp",
            "output_compression": 82,
        });
        if let Some(seed) = request.seed {
            body["seed"] = json!(seed);
        }
        let (answer, model) = self
            .post("/v1/images/generations", Some(request.tier), &body)
            .await?;
        drawn(&answer, model.as_deref().unwrap_or_default())
    }

    /// Edits `images` (data URIs) as `prompt` says, on the detailed model.
    pub async fn edit(
        &self,
        prompt: &str,
        images: &[(&[u8], &str)],
        size: &str,
        seed: Option<i64>,
    ) -> Result<Vec<Drawn>, GatewayError> {
        let images: Vec<Value> = images
            .iter()
            .map(|(bytes, media_type)| json!({ "image_url": data_uri(bytes, media_type) }))
            .collect();
        let mut body = json!({
            "prompt": prompt,
            "images": images,
            "size": size,
            "output_format": "webp",
            "output_compression": 82,
        });
        if let Some(seed) = seed {
            body["seed"] = json!(seed);
        }
        let (answer, model) = self.post("/v1/images/edits", None, &body).await?;
        drawn(&answer, model.as_deref().unwrap_or_default())
    }

    /// The content-safety model's verdict on a picture.
    pub async fn screen(
        &self,
        bytes: &[u8],
        media_type: &str,
        context: &str,
    ) -> Result<Verdict, GatewayError> {
        let body = json!({ "image": data_uri(bytes, media_type), "context": context });
        let (answer, _) = self.post("/v1/images/screen", None, &body).await?;
        serde_json::from_value(answer).map_err(|_| GatewayError {
            code: "screen_failed".into(),
            message: "The safety check gave no verdict, so the picture was not kept.".into(),
        })
    }

    /// Alt text and what the picture shows.
    pub async fn describe(
        &self,
        bytes: &[u8],
        media_type: &str,
        context: &str,
    ) -> Result<Description, GatewayError> {
        let body = json!({ "image": data_uri(bytes, media_type), "context": context });
        let (answer, _) = self.post("/v1/images/describe", None, &body).await?;
        serde_json::from_value(answer).map_err(|_| GatewayError {
            code: "describe_failed".into(),
            message: "The alt text did not come back readable.".into(),
        })
    }
}

/// `data:<type>;base64,<data>`.
pub fn data_uri(bytes: &[u8], media_type: &str) -> String {
    format!(
        "data:{media_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn drawn(answer: &Value, model: &str) -> Result<Vec<Drawn>, GatewayError> {
    let unreadable = |what: &str| GatewayError {
        code: "unreadable".into(),
        message: format!("The image answer had no {what}."),
    };
    let data = answer["data"]
        .as_array()
        .filter(|d| !d.is_empty())
        .ok_or_else(|| unreadable("pictures"))?;
    let cost_total = answer["usage"]["cost"]
        .as_f64()
        .map(|usd| (usd * 1e6).ceil() as i64)
        .unwrap_or(0);
    let share = cost_total / data.len() as i64;
    data.iter()
        .map(|item| {
            let b64 = item["b64_json"]
                .as_str()
                .ok_or_else(|| unreadable("image data"))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| unreadable("readable image data"))?;
            let media_type = item["media_type"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| sniff(&bytes).map(str::to_owned))
                .ok_or_else(|| unreadable("image type"))?;
            Ok(Drawn {
                bytes,
                media_type,
                model: model.to_owned(),
                size: answer["size"].as_str().map(str::to_owned),
                cost_micro_usd: share,
            })
        })
        .collect()
}

/// The image type from its first bytes.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
