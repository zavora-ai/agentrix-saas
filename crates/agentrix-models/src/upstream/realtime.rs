//! Vultr's realtime transcription WebSocket (`/v1/realtime?intent=transcription`). Server to
//! server only: the key goes in the `Authorization` header, which a browser cannot set, so the
//! gateway holds this connection and relays the owner's audio (02 §6).

use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use super::{UpstreamError, Vultr};

/// A live connection to Vultr.
pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Vultr {
    /// Opens a realtime transcription session for `model`. A 429 (all 5 places held) or a
    /// failed connection is `Busy`; a 402 is `Unfunded`. Not retried: the owner is waiting.
    pub async fn realtime(&self, model: &str) -> Result<Socket, UpstreamError> {
        let base = self
            .base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        let url = format!("{base}/realtime?intent=transcription&model={model}");
        let busy = |status: Option<u16>| UpstreamError::Busy {
            tries: 1,
            last_status: status,
            waited: std::time::Duration::ZERO,
        };
        let mut request = url.into_client_request().map_err(|_| busy(None))?;
        let bearer =
            HeaderValue::from_str(&format!("Bearer {}", self.key.0)).map_err(|_| busy(None))?;
        request.headers_mut().insert("authorization", bearer);
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, _)) => Ok(socket),
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                let status = response.status().as_u16();
                if status == 402 {
                    Err(UpstreamError::Unfunded)
                } else {
                    tracing::warn!(status, "Vultr refused a realtime session");
                    Err(busy(Some(status)))
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "could not open a realtime session");
                Err(busy(None))
            }
        }
    }
}
