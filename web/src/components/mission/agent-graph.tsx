"use client";

import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import {
  Box,
  ClipboardList,
  FlaskConical,
  MonitorSmartphone,
  PencilRuler,
  SearchCheck,
  Server,
  ShieldCheck,
  UserRound,
  Workflow,
} from "lucide-react";
import { useEffect, useMemo, useRef, type CSSProperties, type ReactNode, type RefObject } from "react";

import { ModeBadge } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { AGENT_PHASE_LABEL, PHASE_LABEL, formatPercent, truncate } from "@/lib/format";
import type { AgentActivity, ViewState } from "@/lib/reducer";
import type { Meta, RoleSpec, RunStatus, SupervisionMode } from "@/lib/types";
import { cn } from "@/lib/utils";

const ROLE_ICON: Record<string, ReactNode> = {
  product_owner: <ClipboardList />,
  architect: <PencilRuler />,
  qa_engineer: <FlaskConical />,
  backend_dev: <Server />,
  frontend_dev: <MonitorSmartphone />,
  security_auditor: <ShieldCheck />,
  code_reviewer: <SearchCheck />,
};

/**
 * Fixed landscape layout (the panel is wide and short): specification roles on the left,
 * build and control roles on the right, two rows around a central lane that carries the
 * edges to the orchestrator.
 */
const ROLE_POSITION: Record<string, { x: number; y: number; side: "left" | "right"; row: "top" | "bottom" }> = {
  product_owner: { x: -660, y: -165, side: "left", row: "top" },
  architect: { x: -440, y: -165, side: "left", row: "top" },
  qa_engineer: { x: -440, y: 45, side: "left", row: "bottom" },
  backend_dev: { x: 230, y: -165, side: "right", row: "top" },
  frontend_dev: { x: 450, y: -165, side: "right", row: "top" },
  security_auditor: { x: 230, y: 45, side: "right", row: "bottom" },
  code_reviewer: { x: 450, y: 45, side: "right", row: "bottom" },
};

const SANDBOX_TOOLS = new Set(["bash", "run_tests", "git", "code_search", "write_file", "read_file", "str_replace", "list_files"]);

type AgentData = {
  role: RoleSpec;
  activity: AgentActivity | null;
  row: "top" | "bottom";
};
type OrchestratorData = {
  phase: string;
  status: RunStatus;
  task: string | null;
  tdd: string | null;
  parallel: boolean;
};
type HumanData = { mode: SupervisionMode; pending: string | null; decisions: number };
type SandboxData = { kind: string; lastCommand: string | null; testRuns: number; busy: boolean };

type AgentNodeType = Node<AgentData, "agent">;
type OrchestratorNodeType = Node<OrchestratorData, "orchestrator">;
type HumanNodeType = Node<HumanData, "human">;
type SandboxNodeType = Node<SandboxData, "sandbox">;
type GraphNode = AgentNodeType | OrchestratorNodeType | HumanNodeType | SandboxNodeType;

const hidden = "!size-1.5 !min-h-0 !min-w-0 !border-0 !bg-line-strong";

function AgentNode({ data }: NodeProps<AgentNodeType>) {
  const { role, activity, row } = data;
  const active = Boolean(activity?.active);
  const status = active
    ? `${AGENT_PHASE_LABEL[activity?.phase ?? ""] ?? activity?.phase ?? "travaille"}${activity?.taskId ? ` · ${activity.taskId}` : ""}`
    : activity?.turns
      ? "en veille"
      : "pas encore sollicité";
  return (
    <div
      className={cn(
        "w-[210px] rounded-lg border bg-panel px-3 py-2 transition-colors",
        active ? "animate-token border-accent bg-raised" : activity?.failed ? "border-bad/50" : "border-line",
        !active && !activity?.turns && "opacity-65",
      )}
    >
      <Handle type="target" position={row === "top" ? Position.Bottom : Position.Top} id="hub" className={hidden} />
      <Handle type="source" position={Position.Bottom} id="sandbox" className={hidden} />
      <div className="flex items-center gap-2">
        <span className={cn("[&_svg]:size-4", active ? "text-accent" : "text-dim")}>{ROLE_ICON[role.id] ?? <UserRound />}</span>
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-ink">{role.name}</span>
        {active ? <Badge tone="accent">jeton</Badge> : null}
      </div>
      <p className={cn("mt-0.5 truncate text-xs", active ? "text-accent" : "text-faint")}>{status}</p>
      <p className="mt-1 line-clamp-2 min-h-8 text-[11.5px] leading-4 text-dim">
        {active && activity?.lastThought
          ? `« ${truncate(activity.lastThought, 90)} »`
          : activity?.lastSummary
            ? truncate(activity.lastSummary, 90)
            : role.title}
      </p>
      <p className="mt-1 flex items-center justify-between font-mono text-[10.5px] text-faint">
        <span>
          {activity?.turns ?? 0} tour{(activity?.turns ?? 0) > 1 ? "s" : ""}
        </span>
        {activity?.lastConfidence !== null && activity?.lastConfidence !== undefined ? (
          <span>confiance {formatPercent(activity.lastConfidence)}</span>
        ) : null}
      </p>
    </div>
  );
}

function OrchestratorNode({ data }: NodeProps<OrchestratorNodeType>) {
  const running = data.status === "running";
  return (
    <div className={cn("w-[230px] rounded-xl border-2 bg-raised px-3.5 py-2.5", running ? "border-accent/70" : "border-line-strong")}>
      <Handle type="target" position={Position.Top} id="human" className={hidden} />
      <Handle type="source" position={Position.Left} id="left" className={hidden} />
      <Handle type="source" position={Position.Right} id="right" className={hidden} />
      <Handle type="source" position={Position.Bottom} id="sandbox" className={hidden} />
      <div className="flex items-center gap-2">
        <Workflow className="size-4 text-accent" />
        <span className="text-sm font-semibold text-ink">Orchestrateur</span>
      </div>
      <p className="mt-0.5 text-[11px] text-faint">Machine à états · détient les verdicts</p>
      <p className="mt-1.5 text-xs font-medium text-accent">{data.phase}</p>
      <p className="mt-0.5 min-h-4 font-mono text-[11px] text-dim">
        {data.task ? `${data.task}${data.tdd ? ` · ${data.tdd.toUpperCase()}` : ""}` : "—"}
      </p>
      {data.parallel ? (
        <Badge tone="violet" className="mt-1.5">
          revues en parallèle
        </Badge>
      ) : null}
    </div>
  );
}

function HumanNode({ data }: NodeProps<HumanNodeType>) {
  const waiting = Boolean(data.pending);
  return (
    <div
      style={{ "--token-color": "var(--color-warn)" } as CSSProperties}
      className={cn("w-[250px] rounded-lg border bg-panel px-3 py-2", waiting ? "animate-token border-warn bg-raised" : "border-line")}
    >
      <Handle type="source" position={Position.Bottom} id="hub" className={hidden} />
      <div className="flex items-center gap-2">
        <UserRound className={cn("size-4", waiting ? "text-warn" : "text-dim")} />
        <span className="flex-1 text-sm font-semibold whitespace-nowrap text-ink">Superviseur humain</span>
        <ModeBadge mode={data.mode} />
      </div>
      <p className={cn("mt-1 line-clamp-2 min-h-8 text-[11px] leading-4", waiting ? "text-warn" : "text-faint")}>
        {waiting
          ? `Décision attendue : ${data.pending}`
          : data.decisions > 0
            ? `${data.decisions} décision${data.decisions > 1 ? "s" : ""} rendue${data.decisions > 1 ? "s" : ""}`
            : "Sollicité selon la matrice de supervision"}
      </p>
    </div>
  );
}

function SandboxNode({ data }: NodeProps<SandboxNodeType>) {
  return (
    <div className={cn("w-[230px] rounded-lg border border-dashed bg-canvas px-3 py-2", data.busy ? "border-ok/70" : "border-line-strong")}>
      <Handle type="target" position={Position.Top} id="hub" className={hidden} />
      <Handle type="target" position={Position.Left} id="left" className={hidden} />
      <Handle type="target" position={Position.Right} id="right" className={hidden} />
      <div className="flex items-center gap-2">
        <Box className={cn("size-4", data.busy ? "text-ok" : "text-dim")} />
        <span className="flex-1 text-sm font-semibold text-ink">Sandbox</span>
        <Badge tone={data.kind === "docker" ? "ok" : "warn"}>{data.kind}</Badge>
      </div>
      <p className="mt-1 truncate font-mono text-[11px] text-dim">{data.lastCommand ? `$ ${data.lastCommand}` : "en attente de commandes"}</p>
      <p className="mt-0.5 font-mono text-[10px] text-faint">{data.testRuns} exécution(s) de tests</p>
    </div>
  );
}

const nodeTypes = { agent: AgentNode, orchestrator: OrchestratorNode, human: HumanNode, sandbox: SandboxNode };

/** Keeps the whole graph in view when its container is resized. */
function AutoFit({ container }: { container: RefObject<HTMLDivElement | null> }) {
  const { fitView } = useReactFlow();
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const observer = new ResizeObserver(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void fitView({ padding: 0.06, maxZoom: 1.1 }), 80);
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
      if (timer) clearTimeout(timer);
    };
  }, [container, fitView]);
  return null;
}

function Graph({ view, status, meta }: { view: ViewState; status: RunStatus; meta: Meta }) {
  const container = useRef<HTMLDivElement>(null);
  const { run, agents, ota, terminal } = view;
  const pendingGate = run.gates.find((gate) => !gate.resolution) ?? null;
  const waiting = status === "waiting_approval" && pendingGate !== null;
  const lastCommand = terminal.length > 0 ? terminal[terminal.length - 1] : null;
  const lastEvaluation = run.evaluations.length > 0 ? run.evaluations[run.evaluations.length - 1] : null;

  const nodes = useMemo<GraphNode[]>(() => {
    const list: GraphNode[] = [
      {
        id: "human",
        type: "human",
        position: { x: -125, y: -225 },
        data: {
          mode: lastEvaluation && waiting ? lastEvaluation.mode : run.supervision.default_mode,
          pending: waiting && pendingGate ? pendingGate.request.title : null,
          decisions: run.gates.filter((gate) => gate.resolution).length,
        },
      },
      {
        id: "orchestrator",
        type: "orchestrator",
        position: { x: -115, y: -70 },
        data: {
          phase: PHASE_LABEL[run.phase],
          status,
          task: run.tdd && run.phase === "implementation" ? run.tdd.task_id : null,
          tdd: run.tdd && run.phase === "implementation" ? run.tdd.phase : null,
          parallel: run.active_agents.length > 1,
        },
      },
      {
        id: "sandbox",
        type: "sandbox",
        position: { x: -115, y: 110 },
        data: {
          kind: run.sandbox,
          lastCommand: lastCommand ? truncate(lastCommand.command, 48) : null,
          testRuns: run.metrics.test_runs,
          busy: status === "running" && Boolean(lastCommand?.pending),
        },
      },
    ];
    for (const role of meta.roles) {
      const position = ROLE_POSITION[role.id];
      if (!position) continue;
      list.push({
        id: role.id,
        type: "agent",
        position: { x: position.x, y: position.y },
        data: { role, activity: agents[role.id] ?? null, row: position.row },
      });
    }
    return list;
  }, [agents, lastCommand, lastEvaluation, meta.roles, pendingGate, run, status, waiting]);

  const edges = useMemo<Edge[]>(() => {
    const list: Edge[] = [
      {
        id: "human-orchestrator",
        source: "human",
        sourceHandle: "hub",
        target: "orchestrator",
        targetHandle: "human",
        animated: waiting,
        label: waiting ? "validation requise" : undefined,
        style: { stroke: waiting ? "var(--color-warn)" : undefined, strokeWidth: waiting ? 2.5 : 1.2 },
        labelStyle: { fill: "var(--color-warn)", fontSize: 11 },
        labelBgStyle: { fill: "var(--color-canvas)" },
      },
      {
        id: "orchestrator-sandbox",
        source: "orchestrator",
        sourceHandle: "sandbox",
        target: "sandbox",
        targetHandle: "hub",
        label: "tests · lint · git",
        animated: status === "running" && lastCommand?.by === "orchestrator" && run.active_agents.length === 0,
        style: { strokeDasharray: "4 4", strokeWidth: 1.2 },
        labelStyle: { fill: "var(--color-faint)", fontSize: 10 },
        labelBgStyle: { fill: "var(--color-canvas)" },
      },
    ];
    for (const role of meta.roles) {
      const position = ROLE_POSITION[role.id];
      if (!position) continue;
      const active = Boolean(agents[role.id]?.active);
      list.push({
        id: `orchestrator-${role.id}`,
        source: "orchestrator",
        sourceHandle: position.side,
        target: role.id,
        targetHandle: "hub",
        animated: active,
        style: { stroke: active ? "var(--color-accent)" : undefined, strokeWidth: active ? 2.5 : 1.2 },
      });
      // The agent is acting on the workspace or the sandbox right now.
      if (active && ota && ota.role === role.id && ota.tool && SANDBOX_TOOLS.has(ota.tool)) {
        list.push({
          id: `${role.id}-sandbox`,
          source: role.id,
          sourceHandle: "sandbox",
          target: "sandbox",
          targetHandle: position.side,
          animated: true,
          label: ota.tool,
          style: { stroke: "var(--color-ok)", strokeWidth: 1.8 },
          labelStyle: { fill: "var(--color-ok)", fontSize: 10 },
          labelBgStyle: { fill: "var(--color-canvas)" },
        });
      }
    }
    return list;
  }, [agents, lastCommand, meta.roles, ota, run.active_agents.length, status, waiting]);

  return (
    <div ref={container} className="size-full">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        colorMode="dark"
        fitView
        fitViewOptions={{ padding: 0.06, maxZoom: 1.1 }}
        minZoom={0.25}
        maxZoom={1.5}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        zoomOnScroll={false}
        preventScrolling={false}
        aria-label="Graphe des agents : qui détient le jeton de parole"
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1} color="var(--color-line)" />
        <Controls showInteractive={false} position="bottom-right" />
        <AutoFit container={container} />
      </ReactFlow>
    </div>
  );
}

/** Live multi-agent graph: the orchestrator hub, its workers, the human and the sandbox. */
export function AgentGraph(props: { view: ViewState; status: RunStatus; meta: Meta }) {
  return (
    <ReactFlowProvider>
      <Graph {...props} />
    </ReactFlowProvider>
  );
}
