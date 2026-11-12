//! Models through the Model Gateway (`crates/agentrix-models`), the only holder of provider keys.
//!
//! Agents never name a model or a provider: they name a purpose, and the gateway routes it,
//! meters it against the daily cap, and posts the cost. The mix (spec v2, "Models"):
//!
//! | Role | Purpose | Model under the default provider |
//! |---|---|---|
//! | Chief | `brief` | Claude Sonnet |
//! | Receivables (negotiates debt plans) | `negotiate` | Claude Sonnet |
//! | Revenue, Payables, Support | `collect` (routine turns) | Claude Haiku |
//! | Invoice and email extraction | `extract` | Vultr |
//!
//! [`GatewayLlm`] speaks the gateway's OpenAI-wire chat completions. Judge mode and tests use
//! [`crate::scripted::ScriptedLlm`] instead, at zero model cost.

use std::sync::Arc;

use adk_core::{AdkError, Content, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::company::ModelMix;
use crate::identity::{CHIEF, PAYABLES, RECEIVABLES, REVENUE, SUPPORT};

/// The gateway purpose each role's turns use.
pub fn purpose_for(role: &str) -> &'static str {
    match role {
        CHIEF => "brief",
        RECEIVABLES => "negotiate",
        _ => "collect",
    }
}

/// One role's model, through the gateway.
#[derive(Clone)]
pub struct GatewayLlm {
    http: reqwest::Client,
    url: String,
    site: String,
    company: String,
    role: String,
    purpose: &'static str,
    name: String,
    audit: Option<Arc<crate::audit::SignedAudit>>,
}

impl std::fmt::Debug for GatewayLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayLlm")
            .field("site", &self.site)
            .field("role", &self.role)
            .field("purpose", &self.purpose)
            .finish_non_exhaustive()
    }
}

impl GatewayLlm {
    /// `role` of `company` through the gateway at `models_url` (loopback on the node).
    pub fn new(http: reqwest::Client, models_url: &str, company: &str, role: &str) -> Self {
        let purpose = purpose_for(role);
        Self {
            http,
            url: format!("{}/v1/chat/completions", models_url.trim_end_matches('/')),
            site: format!("saas-{company}"),
            company: company.to_owned(),
            role: role.to_owned(),
            purpose,
            name: format!("gateway:{purpose}"),
            audit: None,
        }
    }

    /// Records each call in `audit`: the model and provider the gateway chose, the tokens, the
    /// cost at the gateway's own prices, and whether it answered.
    pub fn with_audit(mut self, audit: Arc<crate::audit::SignedAudit>) -> Self {
        self.audit = Some(audit);
        self
    }

    async fn record(&self, headers: &reqwest::header::HeaderMap, answer: &Value, ok: bool) {
        let Some(audit) = &self.audit else {
            return;
        };
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        let model = header("x-agentrix-model");
        let usage = &answer["usage"];
        let tokens_in = usage["prompt_tokens"]
            .as_u64()
            .or_else(|| usage["input_tokens"].as_u64())
            .unwrap_or(0);
        let tokens_out = usage["completion_tokens"]
            .as_u64()
            .or_else(|| usage["output_tokens"].as_u64())
            .unwrap_or(0);
        let cost = if ok {
            agentrix_models::route::cost_micro_usd(&model, tokens_in, tokens_out)
        } else {
            0
        };
        let draft = adk_core::audit_log::AuditRecordDraft::new(
            adk_core::audit_log::AuditRecordKind::Other("model_call".into()),
            "model.call",
        )
        .outcome(if ok {
            adk_core::audit_log::AuditRecordOutcome::Succeeded
        } else {
            adk_core::audit_log::AuditRecordOutcome::Failed
        })
        .ext(
            "agent",
            json!(format!("urn:adk:agent:{}/{}", self.company, self.role)),
        )
        .ext("purpose", json!(self.purpose))
        .ext("model", json!(model))
        .ext("provider", json!(header("x-agentrix-provider")))
        .ext("tier", json!(header("x-agentrix-tier")))
        .ext("tokens_in", json!(tokens_in))
        .ext("tokens_out", json!(tokens_out))
        .ext("cost_micro_usd", json!(cost));
        if let Err(e) = audit.record(draft).await {
            tracing::warn!(error = %e, "a model call didn't reach the signed audit log");
        }
    }
}

/// Every role of `company` through the gateway: a [`GatewayMix`], whose [`GatewayMix::audited`]
/// records each call in the company's signed audit log.
pub fn gateway_mix(http: reqwest::Client, models_url: &str, company: &str) -> GatewayMix {
    GatewayMix {
        http,
        url: models_url.to_owned(),
        company: company.to_owned(),
    }
}

/// The gateway's models for one company, before they are put in a [`ModelMix`].
#[derive(Debug, Clone)]
pub struct GatewayMix {
    http: reqwest::Client,
    url: String,
    company: String,
}

impl GatewayMix {
    /// The mix, each call recorded in `audit`.
    pub fn audited(self, audit: Arc<crate::audit::SignedAudit>) -> ModelMix {
        self.build(Some(audit))
    }

    /// The mix, unrecorded.
    pub fn plain(self) -> ModelMix {
        self.build(None)
    }

    fn build(self, audit: Option<Arc<crate::audit::SignedAudit>>) -> ModelMix {
        let model = |role: &str| -> Arc<dyn Llm> {
            let llm = GatewayLlm::new(self.http.clone(), &self.url, &self.company, role);
            Arc::new(match &audit {
                Some(a) => llm.with_audit(a.clone()),
                None => llm,
            })
        };
        ModelMix::all(model(REVENUE))
            .with(CHIEF, model(CHIEF))
            .with(RECEIVABLES, model(RECEIVABLES))
            .with(PAYABLES, model(PAYABLES))
            .with(SUPPORT, model(SUPPORT))
    }
}

fn text_of(content: &Content) -> String {
    content
        .parts
        .iter()
        .filter_map(|p| p.text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The request as OpenAI-wire messages and tools.
pub fn to_openai(req: &LlmRequest) -> Value {
    let mut messages = Vec::new();
    for content in &req.contents {
        let calls: Vec<Value> = content
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::FunctionCall { name, args, id, .. } => Some(json!({
                    "id": id.clone().unwrap_or_else(|| name.clone()), "type": "function",
                    "function": { "name": name, "arguments": args.to_string() },
                })),
                _ => None,
            })
            .collect();
        let responses: Vec<Value> = content
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::FunctionResponse {
                    function_response,
                    id,
                    ..
                } => Some(json!({
                    "role": "tool",
                    "tool_call_id": id.clone().unwrap_or_else(|| function_response.name.clone()),
                    "content": function_response.response.to_string(),
                })),
                _ => None,
            })
            .collect();
        let text = text_of(content);
        match content.role.as_str() {
            "system" => messages.push(json!({ "role": "system", "content": text })),
            "model" | "assistant" => {
                let mut m = json!({ "role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) } });
                if !calls.is_empty() {
                    m["tool_calls"] = json!(calls);
                }
                messages.push(m);
            }
            _ => {
                messages.extend(responses);
                if !text.is_empty() {
                    messages.push(json!({ "role": "user", "content": text }));
                }
            }
        }
    }
    let mut names: Vec<&String> = req.tools.keys().collect();
    names.sort();
    let tools: Vec<Value> = names
        .into_iter()
        .map(|name| {
            let d = &req.tools[name];
            json!({ "type": "function", "function": {
                "name": d.get("name").cloned().unwrap_or_else(|| json!(name)),
                "description": d.get("description").cloned().unwrap_or(json!("")),
                "parameters": d.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            }})
        })
        .collect();
    let mut body = json!({ "messages": messages });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    body
}

/// The model's answer, from an OpenAI-wire chat completion.
pub fn from_openai(body: &Value) -> Result<Content, AdkError> {
    let message = &body["choices"][0]["message"];
    if message.is_null() {
        return Err(AdkError::agent("the gateway answered without a message"));
    }
    let mut content = Content::new("model");
    if let Some(text) = message["content"].as_str()
        && !text.is_empty()
    {
        content = content.with_text(text.to_owned());
    }
    for call in message["tool_calls"].as_array().into_iter().flatten() {
        let name = call["function"]["name"].as_str().unwrap_or_default();
        let args = call["function"]["arguments"]
            .as_str()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_else(|| json!({}));
        content.parts.push(Part::FunctionCall {
            name: name.to_owned(),
            args,
            id: call["id"].as_str().map(str::to_owned),
            thought_signature: None,
        });
    }
    Ok(content)
}

#[async_trait]
impl Llm for GatewayLlm {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate_content(
        &self,
        req: LlmRequest,
        _stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let body = to_openai(&req);
        let sent = self
            .http
            .post(&self.url)
            .header("x-agentrix-site", &self.site)
            .header("x-agentrix-purpose", self.purpose)
            .header("x-agentrix-webagent", &self.role)
            .header("x-agentrix-job", "saas-company")
            .json(&body)
            .send()
            .await
            .map_err(|e| AdkError::agent(format!("the model gateway is unreachable: {e}")))?;
        let status = sent.status();
        let headers = sent.headers().clone();
        let answer: Value = sent.json().await.unwrap_or(Value::Null);
        self.record(&headers, &answer, status.is_success()).await;
        if !status.is_success() {
            return Err(AdkError::agent(format!(
                "the model gateway refused ({status}): {}",
                answer["error"]["message"]
                    .as_str()
                    .or(answer["message"].as_str())
                    .unwrap_or("no reason given")
            )));
        }
        let response = LlmResponse::new(from_openai(&answer)?);
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mix_follows_the_spec() {
        assert_eq!(purpose_for(CHIEF), "brief");
        assert_eq!(purpose_for(RECEIVABLES), "negotiate");
        assert_eq!(purpose_for(PAYABLES), "collect");
    }

    #[test]
    fn a_tool_round_trip_converts_both_ways() {
        let mut req = LlmRequest::new(
            "gateway",
            vec![
                Content::new("system").with_text("You are Payables."),
                Content::new("user").with_text("Pay the Vultr bill."),
            ],
        );
        let mut model = Content::new("model");
        model.parts.push(Part::FunctionCall {
            name: "ap_bills_due".into(),
            args: json!({}),
            id: Some("c1".into()),
            thought_signature: None,
        });
        req.contents.push(model);
        let mut result = Content::new("user");
        result.parts.push(Part::FunctionResponse {
            function_response: adk_core::FunctionResponseData {
                name: "ap_bills_due".into(),
                response: json!({ "bills": [] }),
                inline_data: vec![],
                file_data: vec![],
            },
            id: Some("c1".into()),
            annotations: None,
        });
        req.contents.push(result);
        req.tools.insert(
            "pay_bill".into(),
            json!({ "name": "pay_bill", "description": "Pays a bill.", "parameters": { "type": "object" } }),
        );
        let body = to_openai(&req);
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool"]);
        assert_eq!(body["messages"][3]["tool_call_id"], "c1");
        assert_eq!(body["tools"][0]["function"]["name"], "pay_bill");

        let answer = json!({ "choices": [{ "message": { "content": null, "tool_calls": [
            { "id": "c2", "type": "function", "function": { "name": "pay_bill", "arguments": "{\"bill_id\":\"bil-1\"}" } }
        ] } }] });
        let content = from_openai(&answer).unwrap();
        assert!(matches!(
            &content.parts[0],
            Part::FunctionCall { name, args, .. } if name == "pay_bill" && args["bill_id"] == "bil-1"
        ));
    }
}
