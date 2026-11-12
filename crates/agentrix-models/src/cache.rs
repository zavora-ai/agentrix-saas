//! Cache and replay (design D10a).
//!
//! - `live` calls Vultr every time and records nothing.
//! - `record` answers from a recording when one exists and otherwise calls Vultr, saving each
//!   successful answer. This is the development cache.
//! - `replay` answers only from recordings. It never touches the network and needs no key.
//!   Lane tests and CI run here.
//!
//! A recording is keyed by (purpose, model, request digest), where the digest is SHA-256 of
//! the request's RFC 8785 canonical JSON. It lives at
//! `<dir>/<purpose>/<model>/<64 hex>.json` as readable JSON, and is committed to the repo as a
//! fixture. Requests hold no key. Record only against Zavora's fixture sites, so fixtures
//! never carry an owner's content.

use std::io;
use std::path::{Path, PathBuf};

use agentrix_types::{Digest, canonical_json};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How the gateway treats the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Live,
    Record,
    Replay,
}

impl Mode {
    /// Reads `live`, `record` or `replay`.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        match text.trim() {
            "live" => Ok(Self::Live),
            "record" => Ok(Self::Record),
            "replay" => Ok(Self::Replay),
            other => anyhow::bail!("AGENTRIX_MODELS_MODE={other}: use live, record or replay"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Record => "record",
            Self::Replay => "replay",
        }
    }
}

/// One recorded answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub purpose: String,
    pub model: String,
    pub endpoint: String,
    /// The request as sent to Vultr, so a reviewer can see what the fixture answers.
    pub request: Value,
    pub status: u16,
    pub content_type: String,
    /// The answer: JSON when it was JSON, otherwise text (an SSE stream).
    pub body: Value,
    pub recorded_at: String,
}

impl Recording {
    /// The answer's bytes, as the gateway returns them.
    pub fn bytes(&self) -> Vec<u8> {
        match &self.body {
            Value::String(text) if !self.content_type.starts_with("application/json") => {
                text.as_bytes().to_vec()
            }
            other => serde_json::to_vec(other).unwrap_or_default(),
        }
    }

    /// The answer's status.
    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::OK)
    }
}

/// Recordings under one directory.
#[derive(Debug, Clone)]
pub struct Recordings {
    dir: PathBuf,
}

impl Recordings {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The default directory: this crate's `recordings/` in the repo.
    pub fn default_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("recordings")
    }

    /// Where the recording for this request lives. `name` is the purpose, or `router`.
    pub fn path(&self, name: &str, model: &str, request: &Value) -> anyhow::Result<PathBuf> {
        let digest = Digest::of(&canonical_json(request)?);
        Ok(self
            .dir
            .join(name)
            .join(model)
            .join(format!("{}.json", digest.hex())))
    }

    /// The recording for this request, if there is one.
    pub async fn load(
        &self,
        name: &str,
        model: &str,
        request: &Value,
    ) -> anyhow::Result<Option<Recording>> {
        let path = self.path(name, model, request)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("recording {} does not parse: {e}", path.display())
            })?)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Saves a successful answer, writing to a temporary file and renaming it into place.
    #[allow(clippy::too_many_arguments)]
    pub async fn save(
        &self,
        name: &str,
        model: &str,
        endpoint: &str,
        request: &Value,
        status: StatusCode,
        content_type: &str,
        bytes: &[u8],
    ) -> anyhow::Result<PathBuf> {
        let path = self.path(name, model, request)?;
        let body = if content_type.starts_with("application/json") {
            serde_json::from_slice(bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
        } else {
            Value::String(String::from_utf8_lossy(bytes).into_owned())
        };
        let recording = Recording {
            purpose: name.to_owned(),
            model: model.to_owned(),
            endpoint: endpoint.to_owned(),
            request: request.clone(),
            status: status.as_u16(),
            content_type: content_type.to_owned(),
            body,
            recorded_at: crate::meter::stamp(chrono::Utc::now()),
        };
        let dir = path.parent().unwrap_or(&self.dir);
        tokio::fs::create_dir_all(dir).await?;
        let tmp = dir.join(format!(".{}.tmp", uuid_like()));
        let mut text = serde_json::to_vec_pretty(&recording)?;
        text.push(b'\n');
        tokio::fs::write(&tmp, &text).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(path)
    }
}

fn uuid_like() -> String {
    use rand::Rng as _;
    format!("{:016x}", rand::thread_rng().r#gen::<u64>())
}
