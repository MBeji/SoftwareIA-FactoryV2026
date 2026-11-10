//! Run state as a projection of the event stream (event sourcing).
//!
//! [`apply`] is a pure reducer: `state' = apply(state, event)`. The API snapshot, the CLI
//! and the dashboard all derive their view of a run this way, so they can never disagree
//! with the audit log.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{Event, EventType};
use crate::governance::SupervisionPolicy;
use crate::ingestion::htn::Plan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    /// Stopped by a human decision (gate rejection or explicit cancel).
    Cancelled,
    /// The process stopped while the run was active (restart, crash).
    Interrupted,
}

impl RunStatus {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            RunStatus::Pending | RunStatus::Running | RunStatus::WaitingApproval
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Ingestion,
    Planning,
    Architecture,
    Implementation,
    Audit,
    Delivery,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Red,
    Green,
    Refactor,
    Done,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub llm: Usage,
    pub llm_errors: u64,
    pub by_model: BTreeMap<String, Usage>,
    pub by_tier: BTreeMap<String, Usage>,
    pub by_agent: BTreeMap<String, Usage>,
    pub tool_calls: u64,
    pub test_runs: u64,
    pub files_changed: u64,
    pub budget_limit_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRuntime {
    pub status: TaskStatus,
    pub attempts: u32,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TddView {
    pub task_id: String,
    /// `red`, `green`, `refactor` or `done`.
    pub phase: String,
    pub attempt: u32,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateView {
    pub gate_id: String,
    pub requested_at: DateTime<Utc>,
    /// The `gate.requested` payload (stage, title, proposal, options…).
    pub request: Value,
    /// The `gate.resolved` payload once a decision was taken.
    pub resolution: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunState {
    pub id: String,
    pub title: String,
    pub input_excerpt: String,
    pub filename: Option<String>,
    pub status: RunStatus,
    pub phase: Phase,
    pub classification: Option<Value>,
    pub llm_backend: String,
    pub sandbox: String,
    pub supervision: SupervisionPolicy,
    pub plan: Option<Plan>,
    pub tasks: BTreeMap<String, TaskRuntime>,
    /// Agents currently holding the speech token (several during parallel audits).
    pub active_agents: Vec<String>,
    pub tdd: Option<TddView>,
    pub gates: Vec<GateView>,
    /// Latest supervision decision-tree evaluations (most recent last).
    pub evaluations: Vec<Value>,
    pub metrics: Metrics,
    pub findings: Vec<Value>,
    pub artifacts: Vec<Value>,
    pub violations: Vec<Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_seq: u64,
}

impl RunState {
    pub fn pending_gates(&self) -> Vec<&GateView> {
        self.gates
            .iter()
            .filter(|g| g.resolution.is_none())
            .collect()
    }
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Option<T> {
    serde_json::from_value(value.clone()).ok()
}

fn add_usage(usage: &mut Usage, payload: &Value) {
    usage.calls += 1;
    usage.prompt_tokens += payload["prompt_tokens"].as_u64().unwrap_or(0);
    usage.completion_tokens += payload["completion_tokens"].as_u64().unwrap_or(0);
    usage.cost_usd += payload["cost_usd"].as_f64().unwrap_or(0.0);
}

/// Builds the initial state from a `run.created` event.
fn init(event: &Event) -> RunState {
    let p = &event.payload;
    let supervision: SupervisionPolicy = parse(&p["supervision"]).unwrap_or_default();
    RunState {
        id: event.run_id.clone(),
        title: p["title"].as_str().unwrap_or_default().to_string(),
        input_excerpt: p["input_excerpt"].as_str().unwrap_or_default().to_string(),
        filename: p["filename"].as_str().map(str::to_string),
        status: RunStatus::Pending,
        phase: Phase::Ingestion,
        classification: None,
        llm_backend: p["llm_backend"].as_str().unwrap_or_default().to_string(),
        sandbox: p["sandbox"].as_str().unwrap_or_default().to_string(),
        metrics: Metrics {
            budget_limit_usd: supervision.budget_usd,
            ..Default::default()
        },
        supervision,
        plan: None,
        tasks: BTreeMap::new(),
        active_agents: vec![],
        tdd: None,
        gates: vec![],
        evaluations: vec![],
        findings: vec![],
        artifacts: vec![],
        violations: vec![],
        error: None,
        created_at: event.ts,
        updated_at: event.ts,
        last_seq: event.seq,
    }
}

/// The reducer.
pub fn apply(state: &mut RunState, event: &Event) {
    let p = &event.payload;
    state.updated_at = event.ts;
    state.last_seq = event.seq;
    match event.kind {
        EventType::RunCreated => {}
        EventType::RunStatusChanged => {
            if let Some(status) = parse::<RunStatus>(&p["status"]) {
                state.status = status;
                if !status.is_active() {
                    state.active_agents.clear();
                }
            }
            if let Some(error) = p["error"].as_str() {
                state.error = Some(error.to_string());
            }
        }
        EventType::RunPhaseChanged => {
            if let Some(phase) = parse::<Phase>(&p["phase"]) {
                state.phase = phase;
            }
        }
        EventType::InputClassified => state.classification = Some(p.clone()),
        EventType::PlanCreated => {
            if let Some(plan) = parse::<Plan>(&p["plan"]) {
                state.tasks = plan
                    .execution_order
                    .iter()
                    .map(|id| {
                        (
                            id.clone(),
                            TaskRuntime {
                                status: TaskStatus::Pending,
                                attempts: 0,
                                confidence: None,
                            },
                        )
                    })
                    .collect();
                state.plan = Some(plan);
            }
        }
        EventType::TaskStatusChanged => {
            if let (Some(id), Some(status)) =
                (event.task_id.as_ref(), parse::<TaskStatus>(&p["status"]))
            {
                let task = state.tasks.entry(id.clone()).or_insert(TaskRuntime {
                    status: TaskStatus::Pending,
                    attempts: 0,
                    confidence: None,
                });
                task.status = status;
                if let Some(attempts) = p["attempts"].as_u64() {
                    task.attempts = attempts as u32;
                }
                if let Some(confidence) = p["confidence"].as_f64() {
                    task.confidence = Some(confidence);
                }
            }
        }
        EventType::AgentTurnStarted => {
            if let Some(role) = p["role"].as_str() {
                if !state.active_agents.iter().any(|a| a == role) {
                    state.active_agents.push(role.to_string());
                }
            }
        }
        EventType::AgentTurnEnded => {
            if let Some(role) = p["role"].as_str() {
                state.active_agents.retain(|a| a != role);
            }
        }
        EventType::Action => state.metrics.tool_calls += 1,
        EventType::FileChanged => state.metrics.files_changed += 1,
        EventType::TddPhaseChanged => {
            if let Some(task_id) = event.task_id.clone() {
                state.tdd = Some(TddView {
                    task_id,
                    phase: p["phase"].as_str().unwrap_or_default().to_string(),
                    attempt: p["attempt"].as_u64().unwrap_or(1) as u32,
                    max_attempts: p["max_attempts"].as_u64().unwrap_or(1) as u32,
                });
            }
        }
        EventType::TddTestResult => state.metrics.test_runs += 1,
        EventType::LlmCall => {
            if p["ok"].as_bool() == Some(true) {
                add_usage(&mut state.metrics.llm, p);
                if let Some(model) = p["model"].as_str() {
                    add_usage(
                        state.metrics.by_model.entry(model.to_string()).or_default(),
                        p,
                    );
                }
                if let Some(tier) = p["tier"].as_str() {
                    add_usage(
                        state.metrics.by_tier.entry(tier.to_string()).or_default(),
                        p,
                    );
                }
                let agent = event.source.strip_prefix("agent:").unwrap_or(&event.source);
                add_usage(
                    state.metrics.by_agent.entry(agent.to_string()).or_default(),
                    p,
                );
                state.metrics.budget_limit_usd = p["budget_limit_usd"].as_f64();
            } else {
                state.metrics.llm_errors += 1;
            }
        }
        EventType::GateEvaluated => {
            let mut entry = p["evaluation"].clone();
            entry["title"] = p["title"].clone();
            entry["seq"] = event.seq.into();
            entry["task_id"] = event.task_id.clone().into();
            state.evaluations.push(entry);
            if state.evaluations.len() > 50 {
                state.evaluations.remove(0);
            }
        }
        EventType::GateRequested => {
            if let Some(gate_id) = p["gate_id"].as_str() {
                let mut request = p.clone();
                request["task_id"] = event.task_id.clone().into();
                state.gates.push(GateView {
                    gate_id: gate_id.to_string(),
                    requested_at: event.ts,
                    request,
                    resolution: None,
                });
            }
        }
        EventType::GateResolved => {
            if let Some(gate) = state
                .gates
                .iter_mut()
                .find(|g| Some(g.gate_id.as_str()) == p["gate_id"].as_str())
            {
                gate.resolution = Some(p.clone());
            }
        }
        EventType::SupervisionChanged => {
            if let Some(policy) = parse::<SupervisionPolicy>(&p["policy"]) {
                state.metrics.budget_limit_usd = policy.budget_usd;
                state.supervision = policy;
            }
        }
        EventType::PolicyViolation => state.violations.push(p.clone()),
        EventType::SecurityFindings => {
            state.findings = p["findings"].as_array().cloned().unwrap_or_default();
        }
        EventType::ArtifactCreated => {
            // Keep the latest version of each artifact kind (architecture, pull request…).
            state.artifacts.retain(|a| a["kind"] != p["kind"]);
            state.artifacts.push(p.clone());
        }
        EventType::AgentThought
        | EventType::Observation
        | EventType::TddReflection
        | EventType::TerminalOutput
        | EventType::Log => {}
    }
}

/// Folds a full event log into a run state. `None` when the log has no `run.created`.
pub fn project(events: &[Event]) -> Option<RunState> {
    let first = events.first().filter(|e| e.kind == EventType::RunCreated)?;
    let mut state = init(first);
    for event in &events[1..] {
        apply(&mut state, event);
    }
    Some(state)
}
