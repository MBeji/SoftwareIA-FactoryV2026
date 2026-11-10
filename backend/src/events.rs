//! Event stream — the backbone of the factory (OpenHands-style EventStream architecture).
//!
//! Everything that happens in a run (agent thoughts, tool actions, sandbox observations,
//! TDD verdicts, human approvals…) is appended to an immutable, ordered log. The run state
//! shown by the API/UI is a pure projection of that log (see `orchestrator::state`), which
//! gives us auditability, replay and live streaming for free.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "run.created")]
    RunCreated,
    #[serde(rename = "run.status_changed")]
    RunStatusChanged,
    #[serde(rename = "run.phase_changed")]
    RunPhaseChanged,
    #[serde(rename = "ingestion.classified")]
    InputClassified,
    #[serde(rename = "plan.created")]
    PlanCreated,
    #[serde(rename = "task.status_changed")]
    TaskStatusChanged,
    /// An agent takes the "speech token".
    #[serde(rename = "agent.turn_started")]
    AgentTurnStarted,
    /// THINK step of the Observe-Think-Act loop.
    #[serde(rename = "agent.thought")]
    AgentThought,
    #[serde(rename = "agent.turn_ended")]
    AgentTurnEnded,
    /// ACT step: a tool invocation requested by an agent.
    #[serde(rename = "action")]
    Action,
    /// OBSERVE step: the result of a tool invocation.
    #[serde(rename = "observation")]
    Observation,
    #[serde(rename = "workspace.file_changed")]
    FileChanged,
    #[serde(rename = "tdd.phase_changed")]
    TddPhaseChanged,
    #[serde(rename = "tdd.test_result")]
    TddTestResult,
    #[serde(rename = "tdd.reflection")]
    TddReflection,
    #[serde(rename = "llm.call")]
    LlmCall,
    /// Outcome of the supervision decision tree for a checkpoint (always emitted).
    #[serde(rename = "gate.evaluated")]
    GateEvaluated,
    /// A human decision is required.
    #[serde(rename = "gate.requested")]
    GateRequested,
    #[serde(rename = "gate.resolved")]
    GateResolved,
    #[serde(rename = "supervision.changed")]
    SupervisionChanged,
    #[serde(rename = "policy.violation")]
    PolicyViolation,
    #[serde(rename = "security.findings")]
    SecurityFindings,
    #[serde(rename = "artifact.created")]
    ArtifactCreated,
    #[serde(rename = "terminal.output")]
    TerminalOutput,
    #[serde(rename = "log")]
    Log,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic sequence number, per run (starts at 1).
    pub seq: u64,
    pub run_id: String,
    pub ts: DateTime<Utc>,
    #[serde(rename = "type")]
    pub kind: EventType,
    /// Who produced the event: `orchestrator`, `agent:<role>`, `user`, `sandbox`, `gatekeeper`…
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub payload: Value,
}

struct RunLog {
    events: Vec<Event>,
    tx: broadcast::Sender<Event>,
}

impl RunLog {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(2048);
        Self {
            events: Vec::new(),
            tx,
        }
    }
}

/// Append-only event store: SQLite for durability, in-memory index for replay,
/// one broadcast channel per run for live subscribers (WebSocket, CLI).
pub struct EventStore {
    conn: Mutex<Connection>,
    runs: Mutex<HashMap<String, RunLog>>,
}

impl EventStore {
    /// Opens (or creates) the store. `None` keeps everything in memory (tests, demos).
    pub fn open(path: Option<&Path>) -> Result<Arc<Self>> {
        let conn = match path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Connection::open(p)?
            }
            None => Connection::open_in_memory()?,
        };
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS events (
                 run_id TEXT NOT NULL,
                 seq    INTEGER NOT NULL,
                 type   TEXT NOT NULL,
                 data   TEXT NOT NULL,
                 PRIMARY KEY (run_id, seq)
             );",
        )?;

        let mut runs: HashMap<String, RunLog> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT data FROM events ORDER BY run_id, seq")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                match serde_json::from_str::<Event>(&row?) {
                    Ok(ev) => runs
                        .entry(ev.run_id.clone())
                        .or_insert_with(RunLog::new)
                        .events
                        .push(ev),
                    Err(e) => tracing::warn!("skipping unreadable event: {e}"),
                }
            }
        }
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            runs: Mutex::new(runs),
        }))
    }

    /// Appends an event and fans it out to live subscribers.
    pub fn append(
        &self,
        run_id: &str,
        kind: EventType,
        source: &str,
        task_id: Option<&str>,
        payload: Value,
    ) -> Event {
        let mut runs = self.runs.lock().expect("event store poisoned");
        let log = runs.entry(run_id.to_string()).or_insert_with(RunLog::new);
        let event = Event {
            seq: log.events.len() as u64 + 1,
            run_id: run_id.to_string(),
            ts: Utc::now(),
            kind,
            source: source.to_string(),
            task_id: task_id.map(str::to_string),
            payload,
        };
        if let Ok(data) = serde_json::to_string(&event) {
            let type_name = serde_json::to_value(kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let conn = self.conn.lock().expect("event store poisoned");
            if let Err(e) = conn.execute(
                "INSERT INTO events (run_id, seq, type, data) VALUES (?1, ?2, ?3, ?4)",
                params![event.run_id, event.seq as i64, type_name, data],
            ) {
                tracing::error!(
                    "failed to persist event {}#{}: {e}",
                    event.run_id,
                    event.seq
                );
            }
        }
        log.events.push(event.clone());
        let _ = log.tx.send(event.clone());
        event
    }

    /// Events of a run with `seq > since`.
    pub fn events(&self, run_id: &str, since: u64) -> Vec<Event> {
        let runs = self.runs.lock().expect("event store poisoned");
        runs.get(run_id)
            .map(|log| {
                log.events
                    .iter()
                    .filter(|e| e.seq > since)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Atomically returns the backlog (`seq > since`) and a live receiver: no gap, no duplicate.
    pub fn subscribe(&self, run_id: &str, since: u64) -> (Vec<Event>, broadcast::Receiver<Event>) {
        let mut runs = self.runs.lock().expect("event store poisoned");
        let log = runs.entry(run_id.to_string()).or_insert_with(RunLog::new);
        let backlog = log
            .events
            .iter()
            .filter(|e| e.seq > since)
            .cloned()
            .collect();
        (backlog, log.tx.subscribe())
    }

    pub fn has_run(&self, run_id: &str) -> bool {
        let runs = self.runs.lock().expect("event store poisoned");
        runs.get(run_id).is_some_and(|l| !l.events.is_empty())
    }

    /// Known run ids, most recent first.
    pub fn run_ids(&self) -> Vec<String> {
        let runs = self.runs.lock().expect("event store poisoned");
        let mut ids: Vec<(DateTime<Utc>, String)> = runs
            .iter()
            .filter_map(|(id, log)| log.events.first().map(|e| (e.ts, id.clone())))
            .collect();
        ids.sort_by_key(|(created, _)| std::cmp::Reverse(*created));
        ids.into_iter().map(|(_, id)| id).collect()
    }
}

/// Cheap handle bound to one run, passed to every component that needs to publish events.
#[derive(Clone)]
pub struct Emitter {
    store: Arc<EventStore>,
    run_id: String,
}

impl Emitter {
    pub fn new(store: Arc<EventStore>, run_id: impl Into<String>) -> Self {
        Self {
            store,
            run_id: run_id.into(),
        }
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn emit(
        &self,
        kind: EventType,
        source: &str,
        task_id: Option<&str>,
        payload: Value,
    ) -> Event {
        self.store
            .append(&self.run_id, kind, source, task_id, payload)
    }

    pub fn log(&self, source: &str, level: &str, message: impl Into<String>) {
        self.emit(
            EventType::Log,
            source,
            None,
            serde_json::json!({ "level": level, "message": message.into() }),
        );
    }
}
