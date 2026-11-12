//! A sample site's agent door over MCP (JSON-RPC 2.0 over POST `/mcp`), anonymous or signed.

use std::sync::Arc;

use adk_auth::web_bot_auth::HttpSignatureSigner;
use saas_settle::door::{CONTENT_DIGEST, content_digest};
use serde_json::{Value, json};

/// One visit's conversation with one site's door.
pub struct Door {
    http: reqwest::Client,
    url: String,
    user_agent: String,
    session: Option<String>,
    next_id: u64,
}

/// What a tool call answered.
#[derive(Debug, Clone)]
pub struct ToolAnswer {
    /// The tool refused or failed.
    pub is_error: bool,
    /// Its text.
    pub text: String,
    /// Its structured content.
    pub structured: Value,
}

impl Door {
    /// The door at `site` (an origin), calling as `user_agent`.
    pub fn new(http: reqwest::Client, site: &str, user_agent: &str) -> Self {
        Self {
            http,
            url: format!("{}/mcp", site.trim_end_matches('/')),
            user_agent: user_agent.into(),
            session: None,
            next_id: 1,
        }
    }

    async fn rpc(
        &mut self,
        method: &str,
        params: Value,
        signer: Option<&HttpSignatureSigner>,
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        let mut request = self
            .http
            .post(&self.url)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("user-agent", &self.user_agent)
            .header(CONTENT_DIGEST, content_digest(&body));
        if let Some(s) = &self.session {
            request = request.header("mcp-session-id", s);
        }
        let mut request = request.body(body).build()?;
        if let Some(signer) = signer {
            signer.sign_reqwest(&mut request)?;
        }
        let response = self.http.execute(request).await?;
        if let Some(s) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session = Some(s.to_owned());
        }
        let status = response.status();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if let Some(e) = value.get("error") {
            anyhow::bail!(
                "{method}: {} ({status})",
                e["message"].as_str().unwrap_or("error")
            );
        }
        Ok(value["result"].clone())
    }

    /// `initialize`, naming the client (a name proves nothing at the door).
    pub async fn initialize(&mut self, client_name: &str) -> anyhow::Result<()> {
        self.rpc(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": client_name, "version": env!("CARGO_PKG_VERSION")}}),
            None,
        )
        .await
        .map(|_| ())
    }

    /// `tools/list`.
    pub async fn tools(&mut self) -> anyhow::Result<Vec<Value>> {
        Ok(self.rpc("tools/list", json!({}), None).await?["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    /// `tools/call`, signed when `signer` is given.
    pub async fn call(
        &mut self,
        tool: &str,
        args: Value,
        signer: Option<&Arc<HttpSignatureSigner>>,
    ) -> anyhow::Result<ToolAnswer> {
        let result = self
            .rpc(
                "tools/call",
                json!({"name": tool, "arguments": args}),
                signer.map(Arc::as_ref),
            )
            .await?;
        Ok(ToolAnswer {
            is_error: result["isError"].as_bool().unwrap_or(false),
            text: result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            structured: result["structuredContent"].clone(),
        })
    }
}
