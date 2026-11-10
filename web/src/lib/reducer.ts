// Client-side projection of the event stream.
//
// `reduceEvent` mirrors the backend reducer (backend/src/orchestrator/state.rs) for the
// run state, and additionally derives what only the dashboard needs: per-agent activity,
// the current Observe/Think/Act step, terminal lines, live diffs, TDD verdicts.

import type {
  Artifact,
  Classification,
  Evaluation,
  FactoryEvent,
  Finding,
  GateRequest,
  GateResolutionPayload,
  Json,
  JsonObject,
  LlmCall,
  Metrics,
  Phase,
  Plan,
  PolicyViolation,
  RunState,
  RunStatus,
  SupervisionPolicy,
  TaskStatus,
  TestReport,
  Usage,
} from "./types";
import { roleFromSource } from "./format";

export type OtaStep = "observe" | "think" | "act";

export interface OtaView {
  step: OtaStep;
  role: string | null;
  taskId: string | null;
  text: string;
  tool: string | null;
  seq: number;
}

export interface AgentActivity {
  role: string;
  active: boolean;
  phase: string | null;
  taskId: string | null;
  attempt: number;
  turns: number;
  lastThought: string | null;
  lastTool: string | null;
  lastSummary: string | null;
  lastConfidence: number | null;
  failed: boolean;
}

export interface TerminalEntry {
  key: string;
  seq: number;
  ts: string;
  /** `orchestrator`, `user`, or an agent role id. */
  by: string;
  channel: "shell" | "tests" | "git" | "search" | "verdict";
  command: string;
  output: string | null;
  exitCode: number | null;
  ok: boolean | null;
  taskId: string | null;
  callId: string | null;
  pending: boolean;
}

export interface FileChangeEntry {
  seq: number;
  ts: string;
  role: string | null;
  taskId: string | null;
  path: string;
  created: boolean;
  diff: string;
  additions: number;
  deletions: number;
}

export interface TestVerdict {
  seq: number;
  ts: string;
  taskId: string | null;
  phase: string;
  attempt: number;
  expected: "fail" | "pass";
  asExpected: boolean;
  summary: string;
  report: TestReport;
}

export interface ReflectionEntry {
  seq: number;
  taskId: string | null;
  attempt: number;
  rootCause: string;
  fixPlan: string;
  confidence: number;
}

export interface LlmCallEntry extends LlmCall {
  seq: number;
  role: string | null;
  taskId: string | null;
}

export interface ViewState {
  run: RunState;
  agents: Record<string, AgentActivity>;
  ota: OtaView | null;
  lastLlm: LlmCallEntry | null;
  terminal: TerminalEntry[];
  fileChanges: FileChangeEntry[];
  verdicts: TestVerdict[];
  reflections: ReflectionEntry[];
  /** Sequence number of the latest phase change, to highlight prompt chaining. */
  lastPhaseSeq: number;
}

// ------------------------------------------------------------------- accessors

const str = (value: Json | undefined, fallback = ""): string => (typeof value === "string" ? value : fallback);
const num = (value: Json | undefined, fallback = 0): number => (typeof value === "number" ? value : fallback);
const bool = (value: Json | undefined): boolean => value === true;
const obj = (value: Json | undefined): JsonObject =>
  value && typeof value === "object" && !Array.isArray(value) ? value : {};
const cast = <T>(value: Json | undefined): T => value as unknown as T;

const EMPTY_USAGE: Usage = { calls: 0, prompt_tokens: 0, completion_tokens: 0, cost_usd: 0 };

function addUsage(usage: Usage | undefined, p: JsonObject): Usage {
  const base = usage ?? EMPTY_USAGE;
  return {
    calls: base.calls + 1,
    prompt_tokens: base.prompt_tokens + num(p.prompt_tokens),
    completion_tokens: base.completion_tokens + num(p.completion_tokens),
    cost_usd: base.cost_usd + num(p.cost_usd),
  };
}

const RUN_STATUSES: RunStatus[] = [
  "pending",
  "running",
  "waiting_approval",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
];
const PHASE_IDS: Phase[] = ["ingestion", "planning", "architecture", "implementation", "audit", "delivery", "done"];
const TASK_STATUSES: TaskStatus[] = ["pending", "red", "green", "refactor", "done", "failed"];

export function isActive(status: RunStatus): boolean {
  return status === "pending" || status === "running" || status === "waiting_approval";
}

function emptyAgent(role: string): AgentActivity {
  return {
    role,
    active: false,
    phase: null,
    taskId: null,
    attempt: 0,
    turns: 0,
    lastThought: null,
    lastTool: null,
    lastSummary: null,
    lastConfidence: null,
    failed: false,
  };
}

function initView(event: FactoryEvent): ViewState {
  const p = event.payload;
  const supervision = cast<SupervisionPolicy>(p.supervision);
  const metrics: Metrics = {
    llm: EMPTY_USAGE,
    llm_errors: 0,
    by_model: {},
    by_tier: {},
    by_agent: {},
    tool_calls: 0,
    test_runs: 0,
    files_changed: 0,
    budget_limit_usd: supervision?.budget_usd ?? null,
  };
  return {
    run: {
      id: event.run_id,
      title: str(p.title),
      input_excerpt: str(p.input_excerpt),
      filename: typeof p.filename === "string" ? p.filename : null,
      status: "pending",
      phase: "ingestion",
      classification: null,
      llm_backend: str(p.llm_backend),
      sandbox: str(p.sandbox),
      supervision,
      plan: null,
      tasks: {},
      active_agents: [],
      tdd: null,
      gates: [],
      evaluations: [],
      metrics,
      findings: [],
      artifacts: [],
      violations: [],
      error: null,
      created_at: event.ts,
      updated_at: event.ts,
      last_seq: event.seq,
    },
    agents: {},
    ota: null,
    lastLlm: null,
    terminal: [],
    fileChanges: [],
    verdicts: [],
    reflections: [],
    lastPhaseSeq: 0,
  };
}

/** Human-readable one-liner for a tool call, used by the terminal and the OTA panel. */
export function describeAction(tool: string, args: JsonObject): string {
  switch (tool) {
    case "bash":
      return str(args.command);
    case "run_tests":
      return args.target ? `run_tests ${str(args.target)}` : "run_tests";
    case "git":
      return `git ${str(args.args)}`;
    case "code_search":
      return `code_search ${str(args.mode, "regex")} ${str(args.pattern)}`;
    case "write_file":
    case "read_file":
    case "str_replace":
      return `${tool} ${str(args.path)}`;
    case "web_search":
      return `web_search ${str(args.query)}`;
    default:
      return tool;
  }
}

const TERMINAL_TOOLS: Record<string, TerminalEntry["channel"]> = {
  bash: "shell",
  run_tests: "tests",
  git: "git",
  code_search: "search",
};

function patchAgent(view: ViewState, role: string, patch: Partial<AgentActivity>): Record<string, AgentActivity> {
  return { ...view.agents, [role]: { ...(view.agents[role] ?? emptyAgent(role)), ...patch } };
}

/** `state' = reduceEvent(state, event)` — pure, returns a new object when something changed. */
export function reduceEvent(view: ViewState | null, event: FactoryEvent): ViewState | null {
  if (event.type === "run.created") return view ?? initView(event);
  if (!view || event.seq <= view.run.last_seq) return view;

  const p = event.payload;
  const role = roleFromSource(event.source);
  const taskId = event.task_id ?? null;
  const run: RunState = { ...view.run, updated_at: event.ts, last_seq: event.seq };
  const next: ViewState = { ...view, run };

  switch (event.type) {
    case "run.status_changed": {
      const status = str(p.status) as RunStatus;
      if (RUN_STATUSES.includes(status)) {
        run.status = status;
        if (!isActive(status)) {
          run.active_agents = [];
          next.agents = Object.fromEntries(
            Object.entries(view.agents).map(([id, agent]) => [id, { ...agent, active: false }]),
          );
        }
      }
      if (typeof p.error === "string") run.error = p.error;
      break;
    }
    case "run.phase_changed": {
      const phase = str(p.phase) as Phase;
      if (PHASE_IDS.includes(phase)) run.phase = phase;
      next.lastPhaseSeq = event.seq;
      break;
    }
    case "ingestion.classified":
      run.classification = cast<Classification>(p);
      break;
    case "plan.created": {
      const plan = cast<Plan>(p.plan);
      if (plan && Array.isArray(plan.execution_order)) {
        run.plan = plan;
        run.tasks = Object.fromEntries(
          plan.execution_order.map((id) => [id, { status: "pending" as TaskStatus, attempts: 0, confidence: null }]),
        );
      }
      break;
    }
    case "task.status_changed": {
      const status = str(p.status) as TaskStatus;
      if (taskId && TASK_STATUSES.includes(status)) {
        const current = run.tasks[taskId] ?? { status: "pending" as TaskStatus, attempts: 0, confidence: null };
        run.tasks = {
          ...run.tasks,
          [taskId]: {
            status,
            attempts: typeof p.attempts === "number" ? p.attempts : current.attempts,
            confidence: typeof p.confidence === "number" ? p.confidence : current.confidence,
          },
        };
      }
      break;
    }
    case "agent.turn_started": {
      const id = str(p.role);
      if (id) {
        if (!run.active_agents.includes(id)) run.active_agents = [...run.active_agents, id];
        const previous = view.agents[id] ?? emptyAgent(id);
        next.agents = patchAgent(view, id, {
          active: true,
          phase: str(p.phase) || null,
          taskId,
          attempt: num(p.attempt, 1),
          turns: previous.turns + 1,
          failed: false,
        });
      }
      break;
    }
    case "agent.turn_ended": {
      const id = str(p.role);
      if (id) {
        run.active_agents = run.active_agents.filter((agent) => agent !== id);
        next.agents = patchAgent(view, id, {
          active: false,
          lastSummary: bool(p.ok) ? str(p.summary) || null : str(p.error) || null,
          lastConfidence: typeof p.confidence === "number" ? p.confidence : null,
          failed: !bool(p.ok),
        });
      }
      break;
    }
    case "agent.thought": {
      const text = str(p.text);
      if (role) next.agents = patchAgent(view, role, { lastThought: text });
      next.ota = { step: "think", role, taskId, text, tool: null, seq: event.seq };
      break;
    }
    case "action": {
      const tool = str(p.tool);
      const args = obj(p.args);
      const command = describeAction(tool, args);
      run.metrics = { ...run.metrics, tool_calls: run.metrics.tool_calls + 1 };
      if (role) next.agents = patchAgent(view, role, { lastTool: tool });
      next.ota = { step: "act", role, taskId, text: command, tool, seq: event.seq };
      const channel = TERMINAL_TOOLS[tool];
      if (channel) {
        next.terminal = [
          ...view.terminal,
          {
            key: `a${event.seq}`,
            seq: event.seq,
            ts: event.ts,
            by: role ?? event.source,
            channel,
            command,
            output: null,
            exitCode: null,
            ok: null,
            taskId,
            callId: str(p.call_id) || null,
            pending: true,
          },
        ];
      }
      break;
    }
    case "observation": {
      const tool = str(p.tool);
      const output = str(p.output);
      next.ota = { step: "observe", role, taskId, text: output, tool, seq: event.seq };
      const callId = str(p.call_id);
      // Tool call ids may repeat across turns: complete the most recent pending entry.
      let index = -1;
      for (let i = view.terminal.length - 1; i >= 0; i -= 1) {
        const entry = view.terminal[i];
        if (entry.pending && entry.callId === callId && entry.by === (role ?? event.source)) {
          index = i;
          break;
        }
      }
      if (index >= 0) {
        const data = obj(p.data);
        const terminal = view.terminal.slice();
        terminal[index] = {
          ...terminal[index],
          output,
          ok: bool(p.ok),
          exitCode: typeof data.exit_code === "number" ? data.exit_code : null,
          pending: false,
        };
        next.terminal = terminal;
      }
      break;
    }
    case "workspace.file_changed": {
      run.metrics = { ...run.metrics, files_changed: run.metrics.files_changed + 1 };
      next.fileChanges = [
        ...view.fileChanges,
        {
          seq: event.seq,
          ts: event.ts,
          role,
          taskId,
          path: str(p.path),
          created: bool(p.created),
          diff: str(p.diff),
          additions: num(p.additions),
          deletions: num(p.deletions),
        },
      ];
      break;
    }
    case "tdd.phase_changed":
      if (taskId) {
        run.tdd = {
          task_id: taskId,
          phase: str(p.phase),
          attempt: num(p.attempt, 1),
          max_attempts: num(p.max_attempts, 1),
        };
      }
      break;
    case "tdd.test_result": {
      const report = cast<TestReport>(p.report);
      run.metrics = { ...run.metrics, test_runs: run.metrics.test_runs + 1 };
      next.verdicts = [
        ...view.verdicts,
        {
          seq: event.seq,
          ts: event.ts,
          taskId,
          phase: str(p.phase),
          attempt: num(p.attempt, 1),
          expected: str(p.expected) === "fail" ? "fail" : "pass",
          asExpected: bool(p.as_expected),
          summary: str(p.summary),
          report,
        },
      ];
      next.terminal = [
        ...next.terminal,
        {
          key: `v${event.seq}`,
          seq: event.seq,
          ts: event.ts,
          by: "orchestrator",
          channel: "verdict",
          command: report?.command ?? "tests",
          output: report?.output ?? "",
          exitCode: report?.exit_code ?? null,
          ok: report?.ok ?? null,
          taskId,
          callId: null,
          pending: false,
        },
      ];
      break;
    }
    case "tdd.reflection":
      next.reflections = [
        ...view.reflections,
        {
          seq: event.seq,
          taskId,
          attempt: num(p.attempt, 1),
          rootCause: str(p.root_cause),
          fixPlan: str(p.fix_plan),
          confidence: num(p.confidence),
        },
      ];
      break;
    case "llm.call": {
      const call = cast<LlmCall>(p);
      next.lastLlm = { ...call, seq: event.seq, role, taskId };
      if (bool(p.ok)) {
        const agent = role ?? event.source;
        const metrics: Metrics = {
          ...run.metrics,
          llm: addUsage(run.metrics.llm, p),
          by_model: { ...run.metrics.by_model, [call.model]: addUsage(run.metrics.by_model[call.model], p) },
          by_tier: { ...run.metrics.by_tier, [call.tier]: addUsage(run.metrics.by_tier[call.tier], p) },
          by_agent: { ...run.metrics.by_agent, [agent]: addUsage(run.metrics.by_agent[agent], p) },
          budget_limit_usd: typeof p.budget_limit_usd === "number" ? p.budget_limit_usd : null,
        };
        run.metrics = metrics;
      } else {
        run.metrics = { ...run.metrics, llm_errors: run.metrics.llm_errors + 1 };
      }
      break;
    }
    case "gate.evaluated": {
      const evaluation: Evaluation = {
        ...cast<Evaluation>(p.evaluation),
        title: str(p.title),
        seq: event.seq,
        task_id: taskId,
      };
      run.evaluations = [...run.evaluations, evaluation].slice(-50);
      break;
    }
    case "gate.requested": {
      const gateId = str(p.gate_id);
      if (gateId) {
        const request = { ...cast<GateRequest>(p), task_id: taskId };
        run.gates = [...run.gates, { gate_id: gateId, requested_at: event.ts, request, resolution: null }];
      }
      break;
    }
    case "gate.resolved": {
      const gateId = str(p.gate_id);
      run.gates = run.gates.map((gate) =>
        gate.gate_id === gateId ? { ...gate, resolution: cast<GateResolutionPayload>(p) } : gate,
      );
      break;
    }
    case "supervision.changed": {
      const policy = cast<SupervisionPolicy>(p.policy);
      if (policy && typeof policy.default_mode === "string") {
        run.supervision = policy;
        run.metrics = { ...run.metrics, budget_limit_usd: policy.budget_usd };
      }
      break;
    }
    case "policy.violation":
      run.violations = [...run.violations, cast<PolicyViolation>(p)];
      break;
    case "security.findings":
      run.findings = Array.isArray(p.findings) ? cast<Finding[]>(p.findings) : [];
      break;
    case "artifact.created": {
      const artifact = cast<Artifact>(p);
      run.artifacts = [...run.artifacts.filter((a) => a.kind !== artifact.kind), artifact];
      break;
    }
    case "terminal.output":
      next.terminal = [
        ...view.terminal,
        {
          key: `t${event.seq}`,
          seq: event.seq,
          ts: event.ts,
          by: str(p.by, event.source),
          channel: "shell",
          command: str(p.command),
          output: str(p.output),
          exitCode: num(p.exit_code),
          ok: num(p.exit_code) === 0,
          taskId,
          callId: null,
          pending: false,
        },
      ];
      break;
    case "log":
      break;
  }
  return next;
}
