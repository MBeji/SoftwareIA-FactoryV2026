//! The real LLM path, end to end, against a fake OpenAI-compatible gateway.
//!
//! No provider is called: an in-process HTTP server plays the role of the LiteLLM proxy
//! and answers with scripted Chat Completions (tool calls encoded exactly as on the wire).
//! This exercises everything between the agents and the network: request encoding, tool
//! call decoding, routing by alias, fallback, and cost reporting.

mod common;

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use common::*;
use omniforge::config::{FactoryConfig, Settings};
use omniforge::events::{EventStore, EventType};
use omniforge::governance::SupervisionMode;
use omniforge::llm::gateway::GatewayBackend;
use omniforge::orchestrator::state::RunStatus;
use omniforge::orchestrator::Orchestrator;
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Gateway {
    requests: Arc<Mutex<Vec<Value>>>,
    /// Model aliases answering HTTP 500 (provider outage).
    down: Arc<Vec<&'static str>>,
}

fn tool_call(id: &str, name: &str, arguments: Value) -> Value {
    json!({ "id": id, "type": "function", "function": { "name": name, "arguments": arguments.to_string() } })
}

fn finish(extra: Value) -> Vec<Value> {
    let mut arguments = json!({ "summary": "done", "confidence": 0.9 });
    for (key, value) in extra.as_object().into_iter().flatten() {
        arguments[key] = value.clone();
    }
    vec![tool_call("call_finish", "finish", arguments)]
}

/// What a competent model would answer at each step of a tiny task.
fn script(phase: &str, step: usize) -> (Option<String>, Vec<Value>) {
    match (phase, step) {
        ("plan", _) => (
            Some(
                json!({
                    "goal": "Additionner deux entiers",
                    "confidence": 0.9,
                    "nodes": [
                        { "id": "G", "kind": "goal", "parent": null, "title": "Additionner deux entiers" },
                        { "id": "T1", "kind": "task", "parent": "G", "title": "Fonction add", "component": "backend",
                          "acceptance_criteria": ["add(2, 3) renvoie 5"], "estimate_points": 1,
                          "risk": { "probability": 1, "impact": 1, "factors": [], "mitigation": "" } }
                    ]
                })
                .to_string(),
            ),
            vec![],
        ),
        ("architecture", 0) => (
            Some("I record the design.".into()),
            vec![tool_call("call_1", "write_file", json!({ "path": "docs/ARCHITECTURE.md", "content": "# Architecture\n\n`app/calc.py` exposes `add(a, b)`.\n" }))],
        ),
        ("red", 0) => (
            None,
            vec![
                tool_call("call_0", "write_file", json!({ "path": "conftest.py", "content": "\"\"\"Makes the project root importable.\"\"\"\n" })),
                tool_call("call_1", "write_file", json!({ "path": "tests/test_calc.py", "content": "from app.calc import add\n\n\ndef test_add():\n    assert add(2, 3) == 5\n" })),
            ],
        ),
        ("green", 0) => (
            Some("Simplest implementation that satisfies the test.".into()),
            vec![
                tool_call("call_1", "write_file", json!({ "path": "app/__init__.py", "content": "" })),
                tool_call("call_2", "write_file", json!({ "path": "app/calc.py", "content": "def add(a: int, b: int) -> int:\n    return a + b\n" })),
                tool_call("call_3", "run_tests", json!({})),
            ],
        ),
        ("review", _) => (None, finish(json!({ "verdict": "approve" }))),
        ("security_review", _) => (None, finish(json!({ "findings": [] }))),
        _ => (None, finish(json!({}))),
    }
}

async fn chat_completions(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer sk-virtual-key") {
        return (
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            Json(json!({ "error": "invalid key" })),
        );
    }
    gateway.requests.lock().unwrap().push(body.clone());
    let model = body["model"].as_str().unwrap_or_default();
    if gateway.down.contains(&model) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            HeaderMap::new(),
            Json(json!({ "error": "provider unavailable" })),
        );
    }
    let phase = body["metadata"]["phase"].as_str().unwrap_or_default();
    let step = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "assistant")
        .count();
    let (content, tool_calls) = script(phase, step);
    let mut message = json!({ "role": "assistant", "content": content });
    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert("x-litellm-response-cost", "0.002".parse().unwrap());
    let response = json!({
        "id": "chatcmpl-test",
        "model": format!("provider/{model}"),
        "choices": [{ "index": 0, "finish_reason": "stop", "message": message }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 20 }
    });
    (StatusCode::OK, response_headers, Json(response))
}

async fn start_gateway(down: Vec<&'static str>) -> (Gateway, String) {
    let gateway = Gateway {
        down: Arc::new(down),
        ..Default::default()
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .with_state(gateway.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (gateway, base)
}

fn orchestrator(dir: &tempfile::TempDir, base_url: &str, key: &str) -> Arc<Orchestrator> {
    let backend = Arc::new(GatewayBackend::new(base_url, Some(key.to_string())).unwrap());
    Orchestrator::assemble(
        Settings::for_tests(dir.path()),
        FactoryConfig::default(),
        EventStore::open(None).unwrap(),
        backend,
    )
    .unwrap()
}

#[tokio::test]
async fn a_full_run_works_through_the_gateway_wire_format() {
    let (gateway, base) = start_gateway(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(&dir, &base, "sk-virtual-key");
    let run = orchestrator
        .start_run(request(
            "Écrire une fonction add(a, b)",
            policy(SupervisionMode::Autonomous),
        ))
        .unwrap();
    let (state, events) = drive(&orchestrator, &run.id, |_| panic!("autonomous")).await;

    assert_eq!(state.status, RunStatus::Completed, "{:?}", state.error);
    assert_eq!(state.llm_backend, "gateway");
    assert_eq!(state.plan.as_ref().unwrap().planner, "llm");
    assert_eq!(
        orchestrator
            .workspace(&run.id)
            .unwrap()
            .read("app/calc.py")
            .unwrap(),
        "def add(a: int, b: int) -> int:\n    return a + b\n"
    );

    let requests = gateway.requests.lock().unwrap().clone();
    let model_for = |phase: &str| {
        requests
            .iter()
            .find(|r| r["metadata"]["phase"] == phase)
            .map(|r| r["model"].as_str().unwrap().to_string())
    };
    // Each kind of work is sent to its alias; the proxy maps aliases to providers.
    assert_eq!(model_for("plan").as_deref(), Some("omniforge-reasoning"));
    assert_eq!(model_for("red").as_deref(), Some("omniforge-critic"));
    assert_eq!(model_for("green").as_deref(), Some("omniforge-coder"));

    // Planning asks for JSON and offers no tools; agents get exactly their role's tools.
    let plan = requests
        .iter()
        .find(|r| r["metadata"]["phase"] == "plan")
        .unwrap();
    assert_eq!(plan["response_format"]["type"], "json_object");
    assert!(plan.get("tools").is_none());
    let tools_of = |phase: &str| -> Vec<String> {
        let request = requests
            .iter()
            .find(|r| r["metadata"]["phase"] == phase)
            .unwrap();
        request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(tools_of("green").contains(&"bash".to_string()));
    assert!(!tools_of("red").contains(&"bash".to_string()));
    assert!(!tools_of("review").contains(&"write_file".to_string()));

    // The second GREEN request carries the tool results back, linked to their calls.
    let follow_up = requests
        .iter()
        .filter(|r| r["metadata"]["phase"] == "green")
        .nth(1)
        .unwrap();
    let messages = follow_up["messages"].as_array().unwrap();
    let results: Vec<&Value> = messages.iter().filter(|m| m["role"] == "tool").collect();
    assert_eq!(
        results
            .iter()
            .map(|m| m["tool_call_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["call_1", "call_2", "call_3"]
    );
    assert!(
        results[2]["content"]
            .as_str()
            .unwrap()
            .contains("1 réussis"),
        "the agent observed a real test run"
    );
    let assistant = messages.iter().find(|m| m["role"] == "assistant").unwrap();
    assert!(assistant["tool_calls"][1]["function"]["arguments"].is_string());

    // Costs come from the gateway, not from the fallback price list.
    let calls = of_kind(&events, EventType::LlmCall);
    assert!(calls.iter().all(|e| e.payload["cost_estimated"] == false
        && e.payload["served_by"]
            .as_str()
            .unwrap()
            .starts_with("provider/")));
    assert!((state.metrics.llm.cost_usd - 0.002 * calls.len() as f64).abs() < 1e-9);
    assert_eq!(state.metrics.llm.prompt_tokens, 100 * calls.len() as u64);
}

#[tokio::test]
async fn a_provider_outage_falls_back_to_the_next_model_of_the_tier() {
    let (_gateway, base) = start_gateway(vec!["omniforge-critic"]).await;
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(&dir, &base, "sk-virtual-key");
    let run = orchestrator
        .start_run(request(
            "Écrire une fonction add(a, b)",
            policy(SupervisionMode::Autonomous),
        ))
        .unwrap();
    let (state, events) = drive(&orchestrator, &run.id, |_| panic!("autonomous")).await;

    assert_eq!(state.status, RunStatus::Completed, "{:?}", state.error);
    let red: Vec<_> = of_kind(&events, EventType::LlmCall)
        .into_iter()
        .filter(|e| e.payload["phase"] == "red")
        .collect();
    assert_eq!(
        (
            red[0].payload["ok"].as_bool(),
            red[0].payload["model"].as_str()
        ),
        (Some(false), Some("omniforge-critic"))
    );
    assert_eq!(
        (
            red[1].payload["ok"].as_bool(),
            red[1].payload["model"].as_str()
        ),
        (Some(true), Some("omniforge-coder"))
    );
    assert!(state.metrics.llm_errors >= 1);
}

#[tokio::test]
async fn an_unusable_gateway_fails_the_run_with_a_clear_error() {
    let (_gateway, base) = start_gateway(vec![]).await;
    let dir = tempfile::tempdir().unwrap();
    let orchestrator = orchestrator(&dir, &base, "wrong-key");
    let run = orchestrator
        .start_run(request(
            "Écrire une fonction add(a, b)",
            policy(SupervisionMode::Autonomous),
        ))
        .unwrap();
    let (state, _) = drive(&orchestrator, &run.id, |_| panic!("autonomous")).await;
    assert_eq!(state.status, RunStatus::Failed);
    let error = state.error.unwrap();
    assert!(
        error.contains("401") && error.contains("reasoning"),
        "{error}"
    );
}
