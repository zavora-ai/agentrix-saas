//! [`AskModel`] over the Model Gateway (design §4.3): OpenAI-compatible routes on localhost,
//! every call tagged with `x-agentrix-site` and `x-agentrix-purpose` so it is routed, budgeted
//! and metered per site (R11).

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};

use super::ask::{AskModel, StoreHit};

/// The Model Gateway's base URL and an HTTP client.
#[derive(Clone)]
pub struct GatewayModel {
    base: String,
    http: reqwest::Client,
    retrieval: bool,
}

impl GatewayModel {
    /// A client for the gateway at `base`, such as `http://127.0.0.1:7801`.
    ///
    /// Retrieval through Vultr's vector store is on when `AGENTRIX_RETRIEVAL=vultr` (S3-06);
    /// off by default until proven live.
    pub fn new(base: impl Into<String>, http: reqwest::Client) -> Self {
        let retrieval = std::env::var("AGENTRIX_RETRIEVAL").is_ok_and(|v| v.trim() == "vultr");
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            http,
            retrieval,
        }
    }

    /// Turns retrieval through the vector store on or off, whatever the environment says.
    pub fn with_retrieval(mut self, on: bool) -> Self {
        self.retrieval = on;
        self
    }

    /// One vector-store call through the gateway (`embed` purpose), answered as JSON.
    async fn store_call(&self, site: &str, path: &str, body: Value) -> anyhow::Result<Value> {
        Ok(self
            .post(path, site, "embed")
            .timeout(std::time::Duration::from_secs(30))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    fn post(&self, path: &str, site: &str, purpose: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{}{path}", self.base))
            .header("x-agentrix-site", site)
            .header("x-agentrix-purpose", purpose)
    }
}

#[async_trait]
impl AskModel for GatewayModel {
    async fn screen(&self, site: &str, text: &str) -> anyhow::Result<bool> {
        let v: Value = self
            .post("/v1/screen", site, "screen")
            .json(&json!({"text": text}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        // The gateway answers {"safe": bool, ...}; anything else counts as unsafe.
        Ok(v.get("safe").and_then(Value::as_bool).unwrap_or(false))
    }

    async fn embed(&self, site: &str, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let v: Value = self
            .post("/v1/embeddings", site, "embed")
            .json(&json!({"input": texts}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut rows: Vec<(usize, Vec<f32>)> = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("embeddings: no data"))?
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let index = row
                    .get("index")
                    .and_then(Value::as_u64)
                    .map_or(i, |n| n as usize);
                let vector = row
                    .get("embedding")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_f64)
                            .map(|x| x as f32)
                            .collect()
                    })
                    .unwrap_or_default();
                (index, vector)
            })
            .collect();
        rows.sort_by_key(|(i, _)| *i);
        Ok(rows.into_iter().map(|(_, v)| v).collect())
    }

    async fn answer(
        &self,
        site: &str,
        system: &str,
        user: &str,
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        let response = self
            .post("/v1/chat/completions", site, "concierge")
            .json(&json!({
                "stream": true,
                "temperature": 0.2,
                "max_tokens": 400,
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": user},
                ],
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
                    if let Ok(v) = serde_json::from_str::<Value>(data)
                        && let Some(piece) = v
                            .pointer("/choices/0/delta/content")
                            .and_then(Value::as_str)
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

    fn store_enabled(&self) -> bool {
        self.retrieval
    }

    async fn store_create(&self, site: &str, name: &str) -> anyhow::Result<String> {
        let v = self
            .store_call(site, "/v1/vector_store", json!({"name": name}))
            .await?;
        v.pointer("/collection/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("the vector store named no collection"))
    }

    async fn store_add(
        &self,
        site: &str,
        collection: &str,
        content: &str,
        description: &str,
    ) -> anyhow::Result<String> {
        let v = self
            .store_call(
                site,
                &format!("/v1/vector_store/{collection}/items"),
                json!({"content": content, "description": description, "auto_chunk": false}),
            )
            .await?;
        v.pointer("/item/id")
            .or_else(|| v.pointer("/items/0/id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("the vector store named no item"))
    }

    async fn store_search(
        &self,
        site: &str,
        collection: &str,
        query: &str,
    ) -> anyhow::Result<Vec<StoreHit>> {
        let v = self
            .store_call(
                site,
                &format!("/v1/vector_store/{collection}/search"),
                json!({"input": query}),
            )
            .await?;
        Ok(v.get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("the vector store gave no results"))?
            .iter()
            .filter_map(|r| {
                Some(StoreHit {
                    id: r.get("id")?.as_str()?.to_owned(),
                    content: r
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect())
    }
}
