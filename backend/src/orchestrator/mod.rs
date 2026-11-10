//! Orchestration engine: owns the runs, their lifecycle and their shared resources.
//!
//! The workflow itself is a deterministic state machine ([`pipeline`]); the agents are
//! workers it dispatches (Orchestrator-Workers pattern). The orchestrator — not the agents —
//! holds every verdict that matters: test results, lint, security scan, gates, commits.

pub mod pipeline;
pub mod state;
pub mod tdd;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::JoinHandle;

use crate::agents::{AgentResult, AgentRuntime, Assignment, ROLES};
use crate::config::{FactoryConfig, LlmMode, Settings};
use crate::error::{Error, Result};
use crate::events::{Emitter, EventStore, EventType};
use crate::governance::{
    Checkpoint, ExceptionKind, ExceptionSignal, GateDecision, GateKeeper, GateOption, GateOutcome,
    GateResolution, Stage, SupervisionPolicy,
};
use crate::ingestion::normalize_input;
use crate::llm::gateway::GatewayBackend;
use crate::llm::router::{Budget, ModelRouter};
use crate::llm::simulated::{directives, SimulatedBackend};
use crate::llm::{LlmBackend, LlmRequest, LlmResponse};
use crate::memory::MemoryStore;
use crate::sandbox::{self, is_test_path, CommandGuard, ExecResult, Sandbox, Workspace};
use crate::tools::{run_test_suite, shell_quote, TestReport};
use state::{project, Phase, RunState, RunStatus, TaskStatus};

/// What a client submits to start a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunRequest {
    #[serde(default)]
    pub title: Option<String>,
    /// The request itself: user story, epic or full specification text.
    pub input: String,
    /// Original file name when the input comes from an uploaded document.
    #[serde(default)]
    pub filename: Option<String>,
    /// Supervision policy for this run; the factory default applies when absent.
    #[serde(default)]
    pub supervision: Option<SupervisionPolicy>,
    /// Force the granularity level (1-3) instead of the classifier's verdict.
    #[serde(default)]
    pub level: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub title: String,
    pub status: RunStatus,
    pub phase: Phase,
    pub level: Option<u64>,
    pub mode: crate::governance::SupervisionMode,
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub pending_gates: usize,
    pub cost_usd: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Everything the pipeline needs to execute one run.
pub(crate) struct RunCtx {
    pub run_id: String,
    pub cfg: Arc<FactoryConfig>,
    pub emitter: Emitter,
    pub sandbox: Arc<dyn Sandbox>,
    pub gate: Arc<GateKeeper>,
    pub policy: Arc<RwLock<SupervisionPolicy>>,
    pub budget: Arc<Budget>,
    pub router: Arc<ModelRouter>,
    pub agents: AgentRuntime,
    pub memory: Arc<MemoryStore>,
    pub input: String,
    pub filename: Option<String>,
    pub level_override: Option<u8>,
    pub directives: Vec<String>,
}

impl RunCtx {
    pub fn policy(&self) -> SupervisionPolicy {
        self.policy.read().expect("policy poisoned").clone()
    }

    pub fn ws(&self) -> &Workspace {
        self.sandbox.workspace()
    }

    fn exec_timeout(&self) -> Duration {
        Duration::from_secs(self.cfg.sandbox.exec_timeout_s)
    }

    pub fn set_phase(&self, phase: Phase) {
        self.emitter.emit(
            EventType::RunPhaseChanged,
            "orchestrator",
            None,
            json!({ "phase": phase }),
        );
    }

    pub fn task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        attempts: u32,
        confidence: Option<f64>,
    ) {
        self.emitter.emit(
            EventType::TaskStatusChanged,
            "orchestrator",
            Some(task_id),
            json!({ "status": status, "attempts": attempts, "confidence": confidence }),
        );
    }

    pub fn tdd_phase(&self, task_id: &str, phase: &str, attempt: u32, max_attempts: u32) {
        self.emitter.emit(
            EventType::TddPhaseChanged,
            "orchestrator",
            Some(task_id),
            json!({ "phase": phase, "attempt": attempt, "max_attempts": max_attempts }),
        );
    }

    /// Runs an orchestrator-owned command in the sandbox and mirrors it to the terminal view.
    pub async fn sh(&self, command: &str) -> Result<ExecResult> {
        let res = self.sandbox.exec(command, self.exec_timeout()).await?;
        self.emitter.emit(
            EventType::TerminalOutput,
            "orchestrator",
            None,
            json!({
                "by": "orchestrator",
                "command": command,
                "exit_code": res.exit_code,
                "output": res.combined(),
                "duration_ms": res.duration_ms,
            }),
        );
        Ok(res)
    }

    /// Commits the working tree if it changed. The orchestrator is the only committer.
    pub async fn commit(&self, message: &str) -> Result<()> {
        let res = self
            .sh(&format!(
                "git add -A && (git diff --cached --quiet || git commit -q -m {})",
                shell_quote(message)
            ))
            .await?;
        if !res.ok() {
            return Err(Error::Sandbox(format!(
                "git commit failed: {}",
                res.combined()
            )));
        }
        Ok(())
    }

    /// Runs the test command (optionally restricted to `targets`) and records the verdict.
    /// `expected` is `"fail"` during RED and `"pass"` afterwards.
    pub async fn verify_tests(
        &self,
        task_id: Option<&str>,
        phase: &str,
        attempt: u32,
        expected: &str,
        targets: &[String],
    ) -> TestReport {
        let mut command = self.cfg.sandbox.test_command.clone();
        for target in targets {
            command.push(' ');
            command.push_str(&shell_quote(target));
        }
        let report = run_test_suite(self.sandbox.as_ref(), &command, self.exec_timeout()).await;
        self.emitter.emit(
            EventType::TddTestResult,
            "orchestrator",
            task_id,
            json!({
                "phase": phase,
                "attempt": attempt,
                "expected": expected,
                "as_expected": (expected == "fail") != report.ok,
                "summary": report.summary(),
                "report": report,
            }),
        );
        report
    }

    /// Submits a checkpoint to the supervision matrix. Policy violations raised by agent
    /// tools since the last checkpoint are attached as exceptions.
    pub async fn checkpoint(&self, mut cp: Checkpoint) -> GateOutcome {
        for violation in self.agents.take_violations() {
            cp.exceptions.push(ExceptionSignal::new(
                ExceptionKind::PolicyViolation,
                violation,
            ));
        }
        self.gate.checkpoint(cp).await
    }

    async fn on_budget_exceeded(&self, spent: f64, limit: f64) -> Result<()> {
        let proposed = (limit * 1.5).max(spent * 1.5);
        let outcome = self
            .gate
            .checkpoint(Checkpoint {
                stage: Stage::Budget,
                task_id: None,
                title: "Budget LLM épuisé".into(),
                summary: format!("Dépense de {spent:.4} $ pour un plafond de {limit:.4} $."),
                proposal: json!({ "kind": "budget", "spent_usd": spent, "limit_usd": limit, "proposed_limit_usd": proposed }),
                confidence: 1.0,
                exceptions: vec![ExceptionSignal::new(
                    ExceptionKind::BudgetExceeded,
                    format!("budget de {limit:.2} $ atteint"),
                )],
                options: vec![
                    GateOption::new(GateDecision::Approve, "Relever le plafond de 50 % et poursuivre"),
                    GateOption::new(GateDecision::Reject, "Arrêter le run"),
                ],
            })
            .await;
        match outcome {
            GateOutcome::Approved { .. } | GateOutcome::ChangesRequested { .. } => {
                self.budget.set_limit(Some(proposed));
                let policy = {
                    let mut guard = self.policy.write().expect("policy poisoned");
                    guard.budget_usd = Some(proposed);
                    guard.clone()
                };
                self.emitter.emit(
                    EventType::SupervisionChanged,
                    "gatekeeper",
                    None,
                    json!({ "policy": policy, "by": "budget-gate" }),
                );
                Ok(())
            }
            GateOutcome::Rejected { reason, by_human } => {
                Err(rejection("budget", reason, by_human))
            }
        }
    }

    /// LLM call outside the agent loop (planning, reflection), with budget supervision.
    pub async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        loop {
            match self
                .router
                .complete(request.clone(), &self.budget, &self.emitter)
                .await
            {
                Err(Error::BudgetExceeded { spent, limit }) => {
                    self.on_budget_exceeded(spent, limit).await?
                }
                other => return other,
            }
        }
    }

    /// Runs an agent assignment, with budget supervision.
    pub async fn run_agent(&self, assignment: Assignment) -> Result<AgentResult> {
        loop {
            match self.agents.run(assignment.clone()).await {
                Err(Error::BudgetExceeded { spent, limit }) => {
                    self.on_budget_exceeded(spent, limit).await?
                }
                other => return other,
            }
        }
    }

    /// Content of every test file, keyed by path.
    pub fn snapshot_tests(&self) -> BTreeMap<String, String> {
        self.ws()
            .list()
            .into_iter()
            .filter(|f| is_test_path(&f.path))
            .filter_map(|f| {
                self.ws()
                    .read(&f.path)
                    .ok()
                    .map(|content| (f.path, content))
            })
            .collect()
    }

    /// Anti-cheat: puts locked test files back exactly as QA wrote them. Returns how many
    /// files had been tampered with.
    pub fn restore_locked_tests(&self, locked: &BTreeMap<String, String>, task_id: &str) -> usize {
        let current = self.snapshot_tests();
        let mut tampered = Vec::new();
        for (path, content) in locked {
            if current.get(path) != Some(content) && self.ws().write(path, content).is_ok() {
                tampered.push(path.clone());
            }
        }
        for path in current.keys().filter(|p| !locked.contains_key(*p)) {
            if self.ws().remove(path).is_ok() {
                tampered.push(path.clone());
            }
        }
        if !tampered.is_empty() {
            let detail = format!(
                "tests verrouillés modifiés pendant GREEN puis restaurés : {}",
                tampered.join(", ")
            );
            self.agents
                .violations
                .lock()
                .expect("violations poisoned")
                .push(format!("locked-tests : {detail}"));
            self.emitter.emit(
                EventType::PolicyViolation,
                "orchestrator",
                Some(task_id),
                json!({ "rule": "locked-tests", "detail": detail }),
            );
        }
        tampered.len()
    }

    /// Context shared by every LLM call of the run.
    pub fn base_context(&self) -> Value {
        json!({ "directives": self.directives })
    }
}

/// Maps a gate rejection to the right error: a human "no" cancels, a policy block fails.
pub(crate) fn rejection(what: &str, reason: String, by_human: bool) -> Error {
    if by_human {
        Error::Rejected(format!("{what} : {reason}"))
    } else {
        Error::Aborted(format!("{what} : {reason}"))
    }
}

struct RunHandle {
    gate: Arc<GateKeeper>,
    policy: Arc<RwLock<SupervisionPolicy>>,
    budget: Arc<Budget>,
    sandbox: Arc<dyn Sandbox>,
    emitter: Emitter,
    task: Mutex<Option<JoinHandle<()>>>,
}

pub struct Orchestrator {
    settings: Settings,
    cfg: Arc<FactoryConfig>,
    store: Arc<EventStore>,
    router: Arc<ModelRouter>,
    memory: Arc<MemoryStore>,
    sandbox_kind: String,
    runs: Mutex<HashMap<String, Arc<RunHandle>>>,
}

impl Orchestrator {
    /// Builds the engine from process settings (config file, event store, LLM backend).
    pub fn new(settings: Settings) -> Result<Arc<Self>> {
        let cfg = FactoryConfig::load(settings.config.as_deref())?;
        let backend: Arc<dyn LlmBackend> = match settings.llm_mode {
            LlmMode::Simulated => Arc::new(SimulatedBackend::new(settings.sim_delay_ms)),
            LlmMode::Gateway => Arc::new(GatewayBackend::new(
                &settings.llm_base_url,
                // An empty variable (e.g. unset in docker-compose) means "no key".
                settings
                    .llm_api_key
                    .clone()
                    .filter(|key| !key.trim().is_empty()),
            )?),
        };
        let store = EventStore::open(Some(&settings.data_dir.join("events.db")))?;
        Self::assemble(settings, cfg, store, backend)
    }

    /// Same, with an injected backend and store (tests, embedding).
    pub fn assemble(
        settings: Settings,
        cfg: FactoryConfig,
        store: Arc<EventStore>,
        backend: Arc<dyn LlmBackend>,
    ) -> Result<Arc<Self>> {
        cfg.validate()?;
        let sandbox_kind = settings
            .sandbox
            .clone()
            .unwrap_or_else(|| cfg.sandbox.kind.clone());
        if !["docker", "local"].contains(&sandbox_kind.as_str()) {
            return Err(Error::Invalid(format!(
                "unknown sandbox kind '{sandbox_kind}'"
            )));
        }
        std::fs::create_dir_all(settings.data_dir.join("runs"))?;
        Ok(Arc::new(Self {
            router: Arc::new(ModelRouter::new(backend, cfg.routing.clone())),
            cfg: Arc::new(cfg),
            store,
            memory: Arc::new(MemoryStore::default()),
            sandbox_kind,
            settings,
            runs: Mutex::new(HashMap::new()),
        }))
    }

    pub fn store(&self) -> &Arc<EventStore> {
        &self.store
    }

    pub fn config(&self) -> &FactoryConfig {
        &self.cfg
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.settings.data_dir.join("runs").join(run_id)
    }

    fn handle(&self, run_id: &str) -> Option<Arc<RunHandle>> {
        self.runs
            .lock()
            .expect("runs poisoned")
            .get(run_id)
            .cloned()
    }

    fn live_handle(&self, run_id: &str) -> Result<Arc<RunHandle>> {
        if !self.store.has_run(run_id) {
            return Err(Error::NotFound(format!("run '{run_id}'")));
        }
        self.handle(run_id)
            .ok_or_else(|| Error::Conflict(format!("run '{run_id}' is no longer active")))
    }

    /// Static description of the factory, for the pedagogical dashboard.
    pub fn meta(&self) -> Value {
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "llm_backend": self.router.backend_name(),
            "sandbox": self.sandbox_kind,
            "roles": ROLES.as_slice(),
            "tools": crate::tools::catalogue(),
            "routing": self.cfg.routing,
            "supervision": self.cfg.supervision,
            "stages": Stage::ALL.iter().map(|s| json!({ "id": s, "key_checkpoint": s.is_key_checkpoint() })).collect::<Vec<_>>(),
            "planning": self.cfg.planning,
            "agents": self.cfg.agents,
        })
    }

    /// Creates a run and starts executing it in the background.
    pub fn start_run(self: &Arc<Self>, request: RunRequest) -> Result<RunState> {
        let raw = normalize_input(&request.input)?;
        let run_directives = directives(&raw);
        // Simulation directives steer the demo; they are not part of the specification.
        let input = run_directives
            .iter()
            .fold(raw, |text, d| text.replace(&format!("[sim:{d}]"), ""))
            .trim()
            .to_string();
        if input.is_empty() {
            return Err(Error::Invalid("the request is empty".into()));
        }
        if request.level.is_some_and(|l| !(1..=3).contains(&l)) {
            return Err(Error::Invalid("level must be 1, 2 or 3".into()));
        }
        let policy = request
            .supervision
            .clone()
            .unwrap_or_else(|| self.cfg.supervision.clone());
        policy.validate()?;
        let active = self.runs.lock().expect("runs poisoned").len();
        if active >= self.cfg.agents.max_concurrent_runs as usize {
            return Err(Error::Conflict(format!(
                "{active} runs are already active (limit {}); retry when one finishes",
                self.cfg.agents.max_concurrent_runs
            )));
        }

        let run_id = format!("run-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let sandbox = sandbox::create(
            &self.cfg.sandbox,
            &self.sandbox_kind,
            &run_id,
            &self.run_dir(&run_id),
        )?;
        let emitter = Emitter::new(self.store.clone(), run_id.clone());
        let budget = Arc::new(Budget::new(policy.budget_usd));
        let policy = Arc::new(RwLock::new(policy));
        let gate = Arc::new(GateKeeper::new(emitter.clone(), policy.clone()));

        let first_line = input
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .trim_start_matches('#')
            .trim();
        let title = request
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| first_line.chars().take(80).collect());
        emitter.emit(
            EventType::RunCreated,
            "user",
            None,
            json!({
                "title": title,
                "input_excerpt": input.chars().take(600).collect::<String>(),
                "input_chars": input.chars().count(),
                "filename": request.filename,
                "supervision": *policy.read().expect("policy poisoned"),
                "llm_backend": self.router.backend_name(),
                "sandbox": self.sandbox_kind,
                "directives": run_directives,
            }),
        );

        let ctx = RunCtx {
            run_id: run_id.clone(),
            cfg: self.cfg.clone(),
            emitter: emitter.clone(),
            sandbox: sandbox.clone(),
            gate: gate.clone(),
            policy: policy.clone(),
            budget: budget.clone(),
            router: self.router.clone(),
            agents: AgentRuntime {
                router: self.router.clone(),
                budget: budget.clone(),
                emitter: emitter.clone(),
                sandbox: sandbox.clone(),
                max_steps: self.cfg.agents.max_steps,
                exec_timeout: Duration::from_secs(self.cfg.sandbox.exec_timeout_s),
                test_command: self.cfg.sandbox.test_command.clone(),
                search: None,
                violations: Arc::new(Mutex::new(Vec::new())),
            },
            memory: self.memory.clone(),
            input,
            filename: request.filename.clone(),
            level_override: request.level,
            directives: run_directives,
        };

        let handle = Arc::new(RunHandle {
            gate,
            policy,
            budget,
            sandbox,
            emitter,
            task: Mutex::new(None),
        });
        self.runs
            .lock()
            .expect("runs poisoned")
            .insert(run_id.clone(), handle.clone());

        let this = self.clone();
        let task = tokio::spawn(async move {
            let outcome = pipeline::execute(&ctx).await;
            let payload = match &outcome {
                Ok(()) => json!({ "status": RunStatus::Completed }),
                Err(Error::Rejected(reason)) => {
                    json!({ "status": RunStatus::Cancelled, "error": reason })
                }
                Err(e) => json!({ "status": RunStatus::Failed, "error": e.to_string() }),
            };
            // Release the sandbox before announcing the end of the run: once the final
            // status is visible, nothing of the run is left running.
            if let Err(e) = ctx.sandbox.stop().await {
                tracing::warn!("cannot stop sandbox of {}: {e}", ctx.run_id);
            }
            ctx.emitter
                .emit(EventType::RunStatusChanged, "orchestrator", None, payload);
            this.runs.lock().expect("runs poisoned").remove(&ctx.run_id);
        });
        *handle.task.lock().expect("run task poisoned") = Some(task);
        self.state(&run_id)
    }

    /// Current state of a run, projected from its event log.
    pub fn state(&self, run_id: &str) -> Result<RunState> {
        let events = self.store.events(run_id, 0);
        let mut state =
            project(&events).ok_or_else(|| Error::NotFound(format!("run '{run_id}'")))?;
        if state.status.is_active() && self.handle(run_id).is_none() {
            state.status = RunStatus::Interrupted;
            state.active_agents.clear();
        }
        Ok(state)
    }

    pub fn list(&self) -> Vec<RunSummary> {
        self.store
            .run_ids()
            .iter()
            .filter_map(|id| self.state(id).ok())
            .map(|s| RunSummary {
                tasks_total: s.tasks.len(),
                tasks_done: s
                    .tasks
                    .values()
                    .filter(|t| t.status == TaskStatus::Done)
                    .count(),
                pending_gates: if s.status.is_active() {
                    s.pending_gates().len()
                } else {
                    0
                },
                level: s.classification.as_ref().and_then(|c| c["level"].as_u64()),
                mode: s.supervision.default_mode,
                cost_usd: s.metrics.llm.cost_usd,
                id: s.id,
                title: s.title,
                status: s.status,
                phase: s.phase,
                created_at: s.created_at,
                updated_at: s.updated_at,
            })
            .collect()
    }

    /// Delivers a human decision to a pending gate.
    pub fn resolve_gate(
        &self,
        run_id: &str,
        gate_id: &str,
        mut resolution: GateResolution,
    ) -> Result<()> {
        // `timeout` and `system` are reserved for decisions the factory takes itself.
        if matches!(resolution.resolver.as_str(), "timeout" | "system") {
            resolution.resolver = "user".into();
        }
        self.live_handle(run_id)?.gate.resolve(gate_id, resolution)
    }

    /// Changes the supervision policy of a running run; applies from the next checkpoint.
    pub fn set_supervision(
        &self,
        run_id: &str,
        policy: SupervisionPolicy,
        by: &str,
    ) -> Result<SupervisionPolicy> {
        policy.validate()?;
        let handle = self.live_handle(run_id)?;
        *handle.policy.write().expect("policy poisoned") = policy.clone();
        handle.budget.set_limit(policy.budget_usd);
        handle.emitter.emit(
            EventType::SupervisionChanged,
            "user",
            None,
            json!({ "policy": policy, "by": by }),
        );
        Ok(policy)
    }

    /// Stops an active run.
    pub async fn cancel(&self, run_id: &str, by: &str) -> Result<()> {
        let handle = self.live_handle(run_id)?;
        if !self.state(run_id)?.status.is_active() {
            return Err(Error::Conflict(format!(
                "run '{run_id}' has already finished"
            )));
        }
        if let Some(task) = handle.task.lock().expect("run task poisoned").take() {
            task.abort();
        }
        self.runs.lock().expect("runs poisoned").remove(run_id);
        handle.emitter.emit(
            EventType::RunStatusChanged,
            "user",
            None,
            json!({ "status": RunStatus::Cancelled, "error": format!("run annulé par {by}") }),
        );
        if let Err(e) = handle.sandbox.stop().await {
            tracing::warn!("cannot stop sandbox of {run_id}: {e}");
        }
        Ok(())
    }

    /// Runs a human-typed command in the sandbox of an active run (interactive terminal).
    pub async fn terminal(&self, run_id: &str, command: &str) -> Result<ExecResult> {
        let handle = self.live_handle(run_id)?;
        let command = command.trim();
        if command.is_empty() {
            return Err(Error::Invalid("empty command".into()));
        }
        if let Some(violation) = CommandGuard::shared().check(command) {
            handle.emitter.emit(
                EventType::PolicyViolation,
                "user",
                None,
                json!({ "rule": violation.rule, "detail": format!("commande refusée ({})", violation.reason) }),
            );
            return Err(Error::Policy(violation.reason));
        }
        let res = handle
            .sandbox
            .exec(
                command,
                Duration::from_secs(self.cfg.sandbox.exec_timeout_s),
            )
            .await?;
        handle.emitter.emit(
            EventType::TerminalOutput,
            "user",
            None,
            json!({
                "by": "user",
                "command": command,
                "exit_code": res.exit_code,
                "output": res.combined(),
                "duration_ms": res.duration_ms,
            }),
        );
        Ok(res)
    }

    /// Host-side, traversal-safe view of a run's workspace (also for finished runs).
    pub fn workspace(&self, run_id: &str) -> Result<Workspace> {
        if !self.store.has_run(run_id) {
            return Err(Error::NotFound(format!("run '{run_id}'")));
        }
        let root = self.run_dir(run_id).join("workspace");
        if !root.is_dir() {
            return Err(Error::NotFound(format!("workspace of run '{run_id}'")));
        }
        Ok(Workspace::new(root))
    }

    /// Git diff of the run against its base branch.
    ///
    /// Git only ever runs *inside* the sandbox — never on the host against an
    /// agent-controlled repository. Finished runs serve the diff captured at delivery.
    pub async fn diff(&self, run_id: &str) -> Result<String> {
        if let Some(handle) = self.handle(run_id) {
            let res = handle
                .sandbox
                .exec(
                    "git add -A -N . 2>/dev/null; git --no-pager diff --no-color main",
                    Duration::from_secs(self.cfg.sandbox.exec_timeout_s),
                )
                .await?;
            return Ok(res.stdout);
        }
        let state = self.state(run_id)?;
        state
            .artifacts
            .iter()
            .find(|a| a["kind"] == "pull_request")
            .and_then(|a| a["diff"].as_str().map(str::to_string))
            .ok_or_else(|| Error::NotFound(format!("no delivered diff for run '{run_id}'")))
    }
}
