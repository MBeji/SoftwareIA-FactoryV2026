"use client";

import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import { ListTree, Milestone } from "lucide-react";
import { useEffect, useMemo, useRef, type RefObject } from "react";

import { RiskBadge, TaskStatusBadge } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/panel";
import { COMPONENT_LABEL, formatPercent } from "@/lib/format";
import type { Plan, TaskNode, TaskRuntime, TaskStatus, TddView } from "@/lib/types";
import { cn } from "@/lib/utils";

type TaskData = {
  task: TaskNode;
  runtime: TaskRuntime;
  sprint: number | null;
  critical: boolean;
  current: boolean;
  tddPhase: string | null;
};
type TaskNodeType = Node<TaskData, "task">;

const BORDER: Record<TaskStatus, string> = {
  pending: "border-line",
  red: "border-bad",
  green: "border-ok",
  refactor: "border-violet",
  done: "border-accent/60",
  failed: "border-bad",
};

const STROKE: Record<TaskStatus, string> = {
  pending: "var(--color-line-strong)",
  red: "var(--color-bad)",
  green: "var(--color-ok)",
  refactor: "var(--color-violet)",
  done: "var(--color-accent)",
  failed: "var(--color-bad)",
};

const handle = "!size-1.5 !min-h-0 !min-w-0 !border-0 !bg-line-strong";
const MAX_ROWS = 3;
const COLUMN_WIDTH = 270;
const ROW_HEIGHT = 142;

function TaskCard({ data }: NodeProps<TaskNodeType>) {
  const { task, runtime, sprint, critical, current } = data;
  return (
    <div
      className={cn(
        "h-[126px] w-[236px] overflow-hidden rounded-lg border-2 bg-panel px-3 py-2",
        BORDER[runtime.status],
        current && "animate-token bg-raised",
        runtime.status === "pending" && "opacity-75",
        runtime.status === "failed" && "bg-bad/10",
      )}
    >
      <Handle type="target" position={Position.Left} className={handle} />
      <Handle type="source" position={Position.Right} className={handle} />
      <div className="flex items-center gap-1.5">
        <span className="font-mono text-[11px] text-faint">{task.id}</span>
        <TaskStatusBadge status={runtime.status} />
        {critical ? (
          <span title="Chemin critique" className="ml-auto text-warn">
            <Milestone className="size-3.5" />
          </span>
        ) : null}
      </div>
      <p className="mt-1 line-clamp-2 min-h-9 text-[12px] leading-[18px] font-medium text-ink">{task.title}</p>
      <div className="mt-1.5 flex items-center gap-1 overflow-hidden">
        {sprint !== null ? <Badge tone="info">S{sprint}</Badge> : null}
        <RiskBadge score={task.risk.score} />
        <Badge>{task.estimate_points} pts</Badge>
        <Badge>{COMPONENT_LABEL[task.component] ?? task.component}</Badge>
      </div>
      <p className="mt-1 min-h-3.5 truncate font-mono text-[10px] text-faint">
        {runtime.attempts > 0 ? `${runtime.attempts} tentative${runtime.attempts > 1 ? "s" : ""}` : ""}
        {runtime.confidence !== null ? ` · confiance ${formatPercent(runtime.confidence)}` : ""}
      </p>
    </div>
  );
}

const nodeTypes = { task: TaskCard };

function AutoFit({ container, signature }: { container: RefObject<HTMLDivElement | null>; signature: string }) {
  const { fitView } = useReactFlow();
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let timer: ReturnType<typeof setTimeout> | null = setTimeout(() => void fitView({ padding: 0.12, maxZoom: 1 }), 60);
    const observer = new ResizeObserver(() => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void fitView({ padding: 0.12, maxZoom: 1 }), 80);
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
      if (timer) clearTimeout(timer);
    };
  }, [container, fitView, signature]);
  return null;
}

function Dag({ plan, tasks, tdd }: { plan: Plan; tasks: Record<string, TaskRuntime>; tdd: TddView | null }) {
  const container = useRef<HTMLDivElement>(null);

  const nodes = useMemo<TaskNodeType[]>(() => {
    const list: TaskNodeType[] = [];
    // The panel is wide and short: a wave taller than MAX_ROWS wraps into sub-columns.
    let column = 0;
    plan.waves.forEach((wave) => {
      wave.forEach((id, index) => {
        const task = plan.nodes.find((node) => node.id === id);
        if (!task) return;
        const row = index % MAX_ROWS;
        const inColumn = Math.min(MAX_ROWS, wave.length - Math.floor(index / MAX_ROWS) * MAX_ROWS);
        const runtime = tasks[id] ?? { status: "pending" as TaskStatus, attempts: 0, confidence: null };
        const working = runtime.status === "red" || runtime.status === "green" || runtime.status === "refactor";
        list.push({
          id,
          type: "task",
          position: {
            x: (column + Math.floor(index / MAX_ROWS)) * COLUMN_WIDTH,
            y: (row - (inColumn - 1) / 2) * ROW_HEIGHT,
          },
          data: {
            task,
            runtime,
            sprint: plan.sprints.find((sprint) => sprint.task_ids.includes(id))?.id ?? null,
            critical: plan.critical_path.includes(id),
            current: working && tdd?.task_id === id,
            tddPhase: tdd?.task_id === id ? tdd.phase : null,
          },
        });
      });
      // Extra horizontal room between waves so dependency edges stay readable.
      column += Math.ceil(wave.length / MAX_ROWS) + 0.25;
    });
    return list;
  }, [plan, tasks, tdd]);

  const edges = useMemo<Edge[]>(() => {
    const list: Edge[] = [];
    for (const id of plan.execution_order) {
      const task = plan.nodes.find((node) => node.id === id);
      if (!task) continue;
      for (const dependency of task.depends_on) {
        const from = plan.critical_path.indexOf(dependency);
        const critical = from >= 0 && plan.critical_path[from + 1] === id;
        const status = tasks[dependency]?.status ?? "pending";
        const feeding = status === "done" && ["red", "green", "refactor"].includes(tasks[id]?.status ?? "pending");
        const color = critical ? "var(--color-warn)" : STROKE[status === "done" ? "done" : "pending"];
        list.push({
          id: `${dependency}->${id}`,
          source: dependency,
          target: id,
          animated: feeding,
          style: { stroke: color, strokeWidth: critical ? 2.6 : 1.4 },
          markerEnd: { type: MarkerType.ArrowClosed, color, width: 16, height: 16 },
        });
      }
    }
    return list;
  }, [plan, tasks]);

  return (
    <div ref={container} className="relative size-full">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        colorMode="dark"
        fitView
        fitViewOptions={{ padding: 0.12, maxZoom: 1 }}
        minZoom={0.2}
        maxZoom={1.5}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        zoomOnScroll={false}
        preventScrolling={false}
        aria-label="Graphe de dépendances des tâches"
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1} color="var(--color-line)" />
        <Controls showInteractive={false} position="bottom-right" />
        <AutoFit container={container} signature={plan.execution_order.join(",")} />
      </ReactFlow>
      <div className="pointer-events-none absolute bottom-2 left-2 flex max-w-[calc(100%-5rem)] flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-line bg-panel/90 px-2 py-1 text-[10px] text-dim">
        <span>Colonnes = vagues parallélisables</span>
        <span className="flex items-center gap-1">
          <span className="h-0.5 w-4 bg-warn" /> chemin critique
        </span>
        <span className="flex items-center gap-1">
          <span className="size-2 rounded-sm border-2 border-bad" /> RED
        </span>
        <span className="flex items-center gap-1">
          <span className="size-2 rounded-sm border-2 border-ok" /> GREEN
        </span>
        <span className="flex items-center gap-1">
          <span className="size-2 rounded-sm border-2 border-violet" /> REFACTOR
        </span>
        <span className="flex items-center gap-1">
          <span className="size-2 rounded-sm border-2 border-accent/60" /> livrée
        </span>
      </div>
    </div>
  );
}

/** Task dependency DAG: plan leaves laid out by wave, coloured by TDD status. */
export function TaskDag({
  plan,
  tasks,
  tdd,
}: {
  plan: Plan | null;
  tasks: Record<string, TaskRuntime>;
  tdd: TddView | null;
}) {
  if (!plan) {
    return (
      <EmptyState icon={<ListTree />} title="Le plan n'est pas encore établi">
        Le graphe des tâches apparaît dès que le Product Owner a décomposé la demande.
      </EmptyState>
    );
  }
  return (
    <ReactFlowProvider>
      <Dag plan={plan} tasks={tasks} tdd={tdd} />
    </ReactFlowProvider>
  );
}
