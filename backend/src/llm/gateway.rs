//! OpenAI-compatible gateway backend.
//!
//! Targets the LiteLLM proxy (`config/litellm.config.yaml`), which exposes every provider —
//! Anthropic, OpenAI, local Ollama/vLLM models… — behind one `/chat/completions` contract.
//! Any other OpenAI-compatible endpoint works too. Provider API keys stay in the proxy.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{ChatMessage, ChatRole, LlmBackend, LlmRequest, LlmResponse, ToolCall};
use crate::error::{Error, Result};

pub struct GatewayBackend {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl GatewayBackend {
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Llm(format!("cannot build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
        })
    }

    fn wire_message(msg: &ChatMessage) -> Value {
        let role = match msg.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        };
        let mut out = json!({ "role": role, "content": msg.content });
        if !msg.tool_calls.is_empty() {
            out["tool_calls"] = msg
                .tool_calls
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id,
                        "type": "function",
                        "function": { "name": c.name, "arguments": c.arguments.to_string() },
                    })
                })
                .collect();
        }
        if let Some(id) = &msg.tool_call_id {
            out["tool_call_id"] = json!(id);
        }
        out
    }

    /// Request body in the Chat Completions wire format.
    pub fn build_body(model: &str, request: &LlmRequest) -> Value {
        // Lets LiteLLM attribute spend per run / agent / phase. String values only, so the
        // body stays valid for strict OpenAI-compatible endpoints.
        let mut metadata = json!({
            "run_id": request.meta.run_id,
            "agent": request.meta.role,
            "phase": request.meta.phase,
        });
        if let Some(task_id) = &request.meta.task_id {
            metadata["task_id"] = json!(task_id);
        }
        let mut body = json!({
            "model": model,
            "messages": request.messages.iter().map(Self::wire_message).collect::<Vec<_>>(),
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
            "metadata": metadata,
        });
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": { "name": t.name, "description": t.description, "parameters": t.parameters },
                    })
                })
                .collect();
            body["tool_choice"] = json!("auto");
        }
        if request.json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        body
    }

    /// Parses a Chat Completions response.
    pub fn parse_response(body: &Value, fallback_model: &str) -> Result<LlmResponse> {
        let message = body
            .pointer("/choices/0/message")
            .ok_or_else(|| Error::Llm("gateway response has no choices[0].message".into()))?;
        let mut tool_calls = Vec::new();
        for (i, call) in message["tool_calls"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // Arguments arrive as a JSON-encoded string; some providers send an object.
            let arguments = match call.pointer("/function/arguments") {
                Some(Value::String(raw)) if raw.trim().is_empty() => json!({}),
                Some(Value::String(raw)) => serde_json::from_str(raw).map_err(|e| {
                    Error::Llm(format!(
                        "tool call '{name}' has malformed JSON arguments: {e}"
                    ))
                })?,
                Some(other @ Value::Object(_)) => other.clone(),
                _ => json!({}),
            };
            let id = call["id"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{i}"));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
        Ok(LlmResponse {
            content: message["content"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            tool_calls,
            model: body["model"].as_str().unwrap_or(fallback_model).to_string(),
            prompt_tokens: body
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            completion_tokens: body
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cost_usd: None,
        })
    }
}

#[async_trait]
impl LlmBackend for GatewayBackend {
    fn name(&self) -> &'static str {
        "gateway"
    }

    async fn complete(&self, model: &str, request: &LlmRequest) -> Result<LlmResponse> {
        let mut http_request = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(&Self::build_body(model, request));
        if let Some(key) = &self.api_key {
            http_request = http_request.bearer_auth(key);
        }
        let response = http_request
            .send()
            .await
            .map_err(|e| Error::Llm(format!("gateway unreachable: {e}")))?;
        let status = response.status();
        // LiteLLM reports the real cost of the call (provider price list) in this header.
        let reported_cost = response
            .headers()
            .get("x-litellm-response-cost")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<f64>().ok());
        let text = response
            .text()
            .await
            .map_err(|e| Error::Llm(format!("gateway response unreadable: {e}")))?;
        if !status.is_success() {
            let excerpt: String = text.chars().take(400).collect();
            return Err(Error::Llm(format!(
                "gateway returned HTTP {status} for model '{model}': {excerpt}"
            )));
        }
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Llm(format!("gateway returned invalid JSON: {e}")))?;
        let mut parsed = Self::parse_response(&body, model)?;
        parsed.cost_usd = reported_cost;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{CallMeta, TaskKind, ToolSchema};

    #[test]
    fn builds_a_chat_completions_body_with_tools() {
        let mut request = LlmRequest::new(
            TaskKind::Code,
            vec![
                ChatMessage::system("sys"),
                ChatMessage::assistant(
                    None,
                    vec![ToolCall {
                        id: "c1".into(),
                        name: "bash".into(),
                        arguments: json!({"command": "ls"}),
                    }],
                ),
                ChatMessage::tool_result("c1", "a.py"),
            ],
            CallMeta {
                run_id: "r1".into(),
                role: "backend_dev".into(),
                ..Default::default()
            },
        )
        .with_tools(vec![ToolSchema {
            name: "bash".into(),
            description: "run".into(),
            parameters: json!({"type": "object"}),
        }]);
        request.json_mode = true;
        let body = GatewayBackend::build_body("omniforge-coder", &request);
        assert_eq!(body["model"], "omniforge-coder");
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["arguments"],
            "{\"command\":\"ls\"}"
        );
        assert_eq!(body["messages"][2]["tool_call_id"], "c1");
        assert_eq!(body["tools"][0]["function"]["name"], "bash");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["metadata"]["agent"], "backend_dev");
    }

    #[test]
    fn parses_tool_calls_and_usage() {
        let body = json!({
            "model": "claude-sonnet-5-5",
            "choices": [{ "message": {
                "content": "",
                "tool_calls": [{ "id": "t1", "type": "function", "function": { "name": "write_file", "arguments": "{\"path\":\"a.py\",\"content\":\"x\"}" } }]
            }}],
            "usage": { "prompt_tokens": 12, "completion_tokens": 7 }
        });
        let res = GatewayBackend::parse_response(&body, "alias").unwrap();
        assert_eq!(res.content, None);
        assert_eq!(res.tool_calls[0].arguments["path"], "a.py");
        assert_eq!((res.prompt_tokens, res.completion_tokens), (12, 7));
        assert_eq!(res.model, "claude-sonnet-5-5");
        assert!(GatewayBackend::parse_response(&json!({"choices": []}), "alias").is_err());
    }
}
