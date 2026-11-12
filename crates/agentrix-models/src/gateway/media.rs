//! Images and speech through the gateway (02 §6; W11).
//!
//! The same rules as every other purpose: callers name a purpose, never a model; the gateway
//! is the only holder of the Vultr key; every call is admitted against the site's budgets,
//! metered into `usage`, and recorded and replayed (D10a). Image and speech models bill by
//! units (megapixels, seconds of audio), so their rows carry `units` and `unit` (W8's
//! migration 0801) and a cost from Vultr's own figure or the catalogue price.
//!
//! - [`image`]: the `image` purpose (generate, edit, screen, describe).
//! - [`voice`]: `voice.listen`, `voice.speak` and `voice.live` (the speech cascade).

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use base64::Engine as _;
use bytes::Bytes;
use serde_json::Value;

use super::{ApiError, Caller, Gateway, recorded, save, upstream_error};
use crate::cache::Mode;
use crate::meter::{self, MediaUnits, Outcome, Usage};
use crate::route::Choice;

mod image;
mod voice;

/// The largest body a media route takes: a 20 MB image or 25 MB of audio, as base64.
const BODY_LIMIT: usize = 40 * 1024 * 1024;

/// The media routes, merged into the gateway's router.
pub(super) fn routes() -> Router<Gateway> {
    image::routes()
        .merge(voice::routes())
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

impl Gateway {
    /// Writes one `usage` row for a call billed by units.
    async fn record_media(
        &self,
        caller: &Caller,
        choice: &Choice,
        units: Option<MediaUnits<'static>>,
        outcome: Outcome,
    ) {
        let usage = Usage {
            site: &caller.site,
            purpose: caller.purpose,
            provider: choice.provider,
            tier: choice.tier,
            tier_reason: &choice.reason,
            model: &choice.model,
            move_id: caller.move_id.as_deref(),
            tokens_in: 0,
            tokens_out: 0,
            reasoning_tokens: 0,
            outcome,
            media: Some(units.unwrap_or(MediaUnits {
                units: 0,
                unit: unit_of(caller.purpose),
                cost_micro_usd: 0,
            })),
            webagent: caller.webagent.as_deref(),
            job: caller.job.as_deref(),
        };
        if let Err(err) = meter::record(&self.config().pool, &usage).await {
            tracing::error!(error = %err, site = %caller.site, "could not record media usage");
        }
    }
}

/// The unit a purpose's calls are counted in (W8's `usage.unit`).
pub(super) fn unit_of(purpose: crate::route::Purpose) -> &'static str {
    match purpose {
        crate::route::Purpose::Image => "image",
        _ => "audio_second",
    }
}

/// How a media answer is metered: the units it carries, read from its JSON.
type UnitsOf = fn(&Choice, &Value) -> Option<MediaUnits<'static>>;

/// One JSON call to a Vultr media endpoint (`path` under the base URL), from a recording or
/// from Vultr, metered by units and (in `record` mode) saved under `key`.
async fn media_forward(
    gw: &Gateway,
    caller: &Caller,
    choice: &Choice,
    path: &str,
    key: &Value,
    body: &Value,
    units_of: UnitsOf,
) -> Result<(StatusCode, String, Bytes), ApiError> {
    let name = caller.purpose.as_str();
    if let Some(recording) = recorded(gw, name, &choice.model, key).await? {
        let bytes = Bytes::from(recording.bytes());
        let units = units_of(choice, &recording.body);
        gw.record_media(caller, choice, units, Outcome::Replayed)
            .await;
        return Ok((recording.status(), recording.content_type, bytes));
    }
    let config = gw.config();
    let vultr = config
        .vultr
        .as_ref()
        .ok_or_else(|| ApiError::internal("no Vultr client outside replay mode"))?;
    let response = match vultr.post(path, body).await {
        Ok(response) => response,
        Err(err) => {
            gw.record_media(caller, choice, None, Outcome::Error).await;
            return Err(upstream_error(gw, err));
        }
    };
    gw.set_status("ok", "Providers are answering.".to_owned());
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_owned();
    let bytes = response.bytes().await.map_err(|err| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "provider_failed",
            format!("Vultr's answer was cut off: {err}"),
        )
    })?;
    let answer = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    let (units, outcome) = if status.is_success() {
        (units_of(choice, &answer), Outcome::Ok)
    } else {
        (None, Outcome::Error)
    };
    gw.record_media(caller, choice, units, outcome).await;
    if config.mode == Mode::Record && status.is_success() {
        save(
            gw,
            name,
            &choice.model,
            path,
            key,
            status,
            &content_type,
            &bytes,
        )
        .await;
    }
    Ok((status, content_type, bytes))
}

/// An image or a clip sent inline: `data:<type>;base64,<data>`, checked and decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Inline {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl Inline {
    /// Reads a data URI whose type is one of `allowed`, at most `max_bytes` once decoded.
    pub fn parse(uri: &str, allowed: &[&str], max_bytes: usize) -> Result<Self, String> {
        let rest = uri
            .strip_prefix("data:")
            .ok_or("Send the file inline as a data: URI with base64; links are not fetched.")?;
        let (meta, data) = rest
            .split_once(',')
            .ok_or("A data: URI needs a comma before its data.")?;
        let media_type = meta
            .strip_suffix(";base64")
            .ok_or("A data: URI must be base64.")?
            .trim()
            .to_ascii_lowercase();
        if !allowed.contains(&media_type.as_str()) {
            return Err(format!(
                "{media_type:?} is not accepted here; send one of {allowed:?}."
            ));
        }
        if data.len() / 4 * 3 > max_bytes + 3 {
            return Err(format!(
                "The file is over {} MB.",
                max_bytes / (1024 * 1024)
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|_| "The base64 in the data: URI does not decode.".to_owned())?;
        if bytes.is_empty() {
            return Err("The file is empty.".to_owned());
        }
        Ok(Self { media_type, bytes })
    }

    /// The digest that stands in for the bytes in recording keys.
    pub fn digest(&self) -> String {
        agentrix_types::Digest::of(&self.bytes).to_string()
    }
}

#[cfg(test)]
mod tests;
