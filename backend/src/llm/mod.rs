//! LLM abstraction: provider-agnostic request/response types and the backend contract.
//!
//! The factory never talks to a provider SDK. Agents express *what kind of work* they do
//! ([`TaskKind`]); the [`router::ModelRouter`] picks a model tier and a fallback chain; a
//! [`LlmBackend`] carries the call — the LiteLLM proxy in production, a deterministic
//! double offline.

pub mod gateway;
pub mod router;
pub mod simulated;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Result;

/// The nature of the work, which drives model selection (cost / capability trade-off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Classification,
    Planning,
    Architecture,
    ComplexCode,
    Code,
    TestGeneration,
    Critique,
    SecurityReview,
    LintFix,
    Summarization,
}

impl TaskKind {
    pub const ALL: [TaskKind; 10] = [
        TaskKind::Classification,
        TaskKind::Planning,
        TaskKind::Architecture,
        TaskKind::ComplexCode,
        TaskKind::Code,
        TaskKind::TestGeneration,
        TaskKind::Critique,
        TaskKind::SecurityReview,
        TaskKind::LintFix,
        TaskKind::Summarization,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskKind::Classification => "classification",
            TaskKind::Planning => "planning",
            TaskKind::Architecture => "architecture",
            TaskKind::ComplexCode => "complex_code",
            TaskKind::Code => "code",
            TaskKind::TestGeneration => "test_generation",
            TaskKind::Critique => "critique",
            TaskKind::SecurityReview => "security_review",
            TaskKind::LintFix => "lint_fix",
            TaskKind::Summarization => "summarization",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: Some(text.into()),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: Some(text.into()),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    pub fn assistant(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content,
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(call_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: Some(text.into()),
            tool_calls: vec![],
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// JSON-schema description of a tool, in the function-calling format every major
/// provider understands (normalised by LiteLLM).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Telemetry and routing context attached to every call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallMeta {
    pub run_id: String,
    /// Agent role issuing the call (`qa_engineer`, `backend_dev`…).
    pub role: String,
    pub task_id: Option<String>,
    /// Workflow step (`plan`, `architecture`, `red`, `green`, `reflect`, `refactor`…).
    pub phase: String,
    /// 1-based attempt within the phase (GREEN iterations, revisions).
    pub attempt: u32,
    /// Escalation steps requested on the model ladder (0 = nominal tier).
    pub escalation: u32,
    /// Structured context (task, directives). Ignored by real backends; it is what lets
    /// the simulated backend stay stateless.
    pub context: Value,
}

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub kind: TaskKind,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSchema>,
    /// Ask for a single JSON object as the answer.
    pub json_mode: bool,
    pub temperature: f32,
    pub max_tokens: u32,
    pub meta: CallMeta,
}

impl LlmRequest {
    pub fn new(kind: TaskKind, messages: Vec<ChatMessage>, meta: CallMeta) -> Self {
        Self {
            kind,
            messages,
            tools: vec![],
            json_mode: false,
            temperature: 0.2,
            max_tokens: 8192,
            meta,
        }
    }

    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    pub fn json(mut self) -> Self {
        self.json_mode = true;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// Model identifier reported by the backend.
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Cost reported by the gateway, when it knows it.
    pub cost_usd: Option<f64>,
}

#[async_trait]
pub trait LlmBackend: Send + Sync {
    fn name(&self) -> &'static str;
    /// Performs one completion with the given model alias.
    async fn complete(&self, model: &str, request: &LlmRequest) -> Result<LlmResponse>;
}

/// Extracts the first JSON object from a model answer (tolerates code fences and prose).
pub fn extract_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }
    let bytes = text.as_bytes();
    let mut start = None;
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' if start.is_some() => in_string = true,
            b'{' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if start.is_some() => {
                depth -= 1;
                if depth == 0 {
                    let candidate = &text[start.unwrap()..=i];
                    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
                        return Some(v);
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_noisy_answers() {
        assert_eq!(extract_json("{\"a\": 1}").unwrap()["a"], 1);
        let fenced = "Here is the plan:\n```json\n{\"a\": {\"b\": \"}\"}}\n```\nDone.";
        assert_eq!(extract_json(fenced).unwrap()["a"]["b"], "}");
        assert!(extract_json("no json here").is_none());
    }
}
