// Single source of truth for the shapes exchanged with the OmniForge backend.
// Mirrors backend/src/{events,governance/mod,ingestion/htn,orchestrator/state}.rs.

export type EventType =
  | "run.created"
  | "run.status_changed"
  | "run.phase_changed"
  | "ingestion.classified"
  | "plan.created"
  | "task.status_changed"
  | "agent.turn_started"
  | "agent.thought"
  | "agent.turn_ended"
  | "action"
  | "observation"
  | "workspace.file_changed"
  | "tdd.phase_changed"
  | "tdd.test_result"
  | "tdd.reflection"
  | "llm.call"
  | "gate.evaluated"
  | "gate.requested"
  | "gate.resolved"
  | "supervision.changed"
  | "policy.violation"
  | "security.findings"
  | "artifact.created"
  | "terminal.output"
  | "log";

export type Json = string | number | boolean | null | Json[] | { [key: string]: Json };
export type JsonObject = { [key: string]: Json };

/** One entry of the append-only event stream. */
export interface FactoryEvent {
  seq: number;
  run_id: string;
  ts: string;
  type: EventType;
  /** `orchestrator`, `agent:<role>`, `user`, `gatekeeper`… */
  source: string;
  task_id?: string;
  payload: JsonObject;
}

// ---------------------------------------------------------------- governance

export type SupervisionMode = "hitl" | "hotl" | "autonomous";
export type Stage = "plan" | "architecture" | "test_plan" | "implementation" | "security" | "merge" | "budget";
export type Severity = "info" | "low" | "medium" | "high" | "critical";

export interface SupervisionPolicy {
  default_mode: SupervisionMode;
  stage_overrides: Partial<Record<Stage, SupervisionMode>>;
  confidence_threshold: number;
  max_tdd_iterations: number;
  max_revisions: number;
  budget_usd: number | null;
  block_on_severity: Severity;
  gate_timeout_s: number | null;
}

export type ExceptionKind =
  | "low_confidence"
  | "security_violation"
  | "tdd_divergence"
  | "budget_exceeded"
  | "policy_violation"
  | "vacuous_tests";

export interface ExceptionSignal {
  kind: ExceptionKind;
  detail: string;
}

export type Verdict = "auto_approved" | "human_required" | "blocked";

export interface Evaluation {
  stage: Stage;
  mode: SupervisionMode;
  confidence: number;
  threshold: number;
  exceptions: ExceptionSignal[];
  verdict: Verdict;
  reason: string;
  /** Added by the state projection. */
  title?: string;
  seq?: number;
  task_id?: string | null;
}

export type GateDecision = "approve" | "request_changes" | "reject";

export interface GateOption {
  decision: GateDecision;
  label: string;
}

export interface GateRequest {
  gate_id: string;
  stage: Stage;
  mode: SupervisionMode;
  title: string;
  summary: string;
  reason: string;
  confidence: number;
  threshold: number;
  exceptions: ExceptionSignal[];
  proposal: Proposal;
  options: GateOption[];
  task_id?: string | null;
}

export interface GateResolutionPayload {
  gate_id: string;
  stage: Stage;
  decision: GateDecision;
  feedback: string | null;
  resolver: string;
}

export interface GateView {
  gate_id: string;
  requested_at: string;
  request: GateRequest;
  resolution: GateResolutionPayload | null;
}

export interface Finding {
  rule_id: string;
  severity: Severity;
  path: string;
  line: number;
  message: string;
  remediation: string;
  snippet: string;
}

// ------------------------------------------------------------------- planning

export type NodeKind = "goal" | "epic" | "story" | "task";
export type ComponentKind = "backend" | "frontend" | "infra" | "docs";

export interface Risk {
  probability: number;
  impact: number;
  score: number;
  factors: string[];
  mitigation: string;
}

export interface TaskNode {
  id: string;
  title: string;
  description: string;
  kind: NodeKind;
  parent: string | null;
  acceptance_criteria: string[];
  depends_on: string[];
  component: ComponentKind;
  estimate_points: number;
  risk: Risk;
}

export interface Sprint {
  id: number;
  name: string;
  task_ids: string[];
  points: number;
  max_risk_score: number;
}

export interface Plan {
  level: number;
  goal: string;
  nodes: TaskNode[];
  execution_order: string[];
  waves: string[][];
  critical_path: string[];
  sprints: Sprint[];
  total_points: number;
  confidence: number;
  planner: string;
  warnings: string[];
}

export interface Classification {
  level: number;
  label: string;
  rationale: string;
  signals: {
    words: number;
    headings: number;
    bullets: number;
    enumerated_items: number;
    is_document: boolean;
  };
}

export interface PlanPreview {
  classification: Classification;
  plan: Plan;
}

// ------------------------------------------------------------------ proposals

export interface TestReport {
  ok: boolean;
  exit_code: number;
  passed: number;
  failed: number;
  errors: number;
  timed_out: boolean;
  duration_ms: number;
  command: string;
  output: string;
}

export interface Reflection {
  root_cause: string;
  fix_plan: string;
  confidence: number;
}

export interface PullRequest {
  kind: "pull_request";
  title: string;
  body: string;
  branch: string;
  base: string;
  commits: string[];
  diff_stat: string;
  diff: string;
  confidence: number;
}

export type Proposal =
  | { kind: "plan"; plan: Plan }
  | { kind: "architecture"; document: string }
  | {
      kind: "test_plan";
      task: TaskNode;
      files: { path: string; content: string }[];
      red_run: { summary: string; output: string };
    }
  | {
      kind: "tdd_divergence";
      task: TaskNode;
      attempts: number;
      last_run: { summary: string; output: string };
      last_reflection: Reflection | null;
    }
  | { kind: "security"; findings: Finding[] }
  | PullRequest
  | { kind: "budget"; spent_usd: number; limit_usd: number; proposed_limit_usd: number };

// ------------------------------------------------------------------ run state

export type RunStatus =
  | "pending"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export type Phase = "ingestion" | "planning" | "architecture" | "implementation" | "audit" | "delivery" | "done";
export type TaskStatus = "pending" | "red" | "green" | "refactor" | "done" | "failed";

export interface Usage {
  calls: number;
  prompt_tokens: number;
  completion_tokens: number;
  cost_usd: number;
}

export interface Metrics {
  llm: Usage;
  llm_errors: number;
  by_model: Record<string, Usage>;
  by_tier: Record<string, Usage>;
  by_agent: Record<string, Usage>;
  tool_calls: number;
  test_runs: number;
  files_changed: number;
  budget_limit_usd: number | null;
}

export interface TaskRuntime {
  status: TaskStatus;
  attempts: number;
  confidence: number | null;
}

export interface TddView {
  task_id: string;
  phase: "red" | "green" | "refactor" | "done" | string;
  attempt: number;
  max_attempts: number;
}

export interface ArchitectureArtifact {
  kind: "architecture";
  title: string;
  path: string;
  content: string;
  revision: number;
}

export type Artifact = ArchitectureArtifact | PullRequest;

export interface PolicyViolation {
  rule: string;
  detail: string;
  role?: string;
}

/** Projection of the event stream, as served by `GET /api/runs/{id}`. */
export interface RunState {
  id: string;
  title: string;
  input_excerpt: string;
  filename: string | null;
  status: RunStatus;
  phase: Phase;
  classification: Classification | null;
  llm_backend: string;
  sandbox: string;
  supervision: SupervisionPolicy;
  plan: Plan | null;
  tasks: Record<string, TaskRuntime>;
  active_agents: string[];
  tdd: TddView | null;
  gates: GateView[];
  evaluations: Evaluation[];
  metrics: Metrics;
  findings: Finding[];
  artifacts: Artifact[];
  violations: PolicyViolation[];
  error: string | null;
  created_at: string;
  updated_at: string;
  last_seq: number;
}

export interface RunSummary {
  id: string;
  title: string;
  status: RunStatus;
  phase: Phase;
  level: number | null;
  mode: SupervisionMode;
  tasks_total: number;
  tasks_done: number;
  pending_gates: number;
  cost_usd: number;
  created_at: string;
  updated_at: string;
}

// ----------------------------------------------------------------------- meta

export interface RoleSpec {
  id: string;
  name: string;
  title: string;
  mission: string;
  inputs: string[];
  outputs: string[];
  tools: string[];
  write_scope: "none" | "docs" | "tests" | "source";
  default_kind: string;
  persona: string;
}

export interface RoutingConfig {
  tiers: Record<string, { description: string; models: string[] }>;
  routes: Record<string, string>;
  escalation: string[];
  low_budget_ratio: number;
  pricing: Record<string, { input: number; output: number }>;
}

export interface Meta {
  version: string;
  llm_backend: string;
  sandbox: string;
  roles: RoleSpec[];
  tools: { name: string; description: string }[];
  routing: RoutingConfig;
  supervision: SupervisionPolicy;
  stages: { id: Stage; key_checkpoint: boolean }[];
  planning: { sprint_capacity_points: number };
  agents: { max_steps: number; escalate_after_failures: number; max_security_fix_rounds: number };
}

// ---------------------------------------------------------- workspace & tools

export interface FileEntry {
  path: string;
  size: number;
}

export interface ExecResult {
  command: string;
  exit_code: number;
  stdout: string;
  stderr: string;
  duration_ms: number;
  timed_out: boolean;
}

export interface LlmCall {
  ok: boolean;
  kind: string;
  phase: string;
  tier: string;
  model: string;
  served_by?: string;
  backend: string;
  fallback_index: number;
  reason: string;
  prompt_tokens?: number;
  completion_tokens?: number;
  cost_usd?: number;
  cost_estimated?: boolean;
  latency_ms: number;
  error?: string;
}
