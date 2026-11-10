"use client";

import { ChevronDown, ChevronRight, CornerDownLeft, Milestone, TriangleAlert } from "lucide-react";
import { useState } from "react";

import { ConfidenceBar, RiskBadge, TaskStatusBadge } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { COMPONENT_LABEL, riskLevel } from "@/lib/format";
import type { Classification, Plan, TaskNode, TaskRuntime } from "@/lib/types";
import { cn } from "@/lib/utils";

const KIND_LABEL: Record<TaskNode["kind"], string> = { goal: "Objectif", epic: "Epic", story: "Story", task: "Tâche" };

export function ClassificationSummary({ classification }: { classification: Classification }) {
  const { signals } = classification;
  return (
    <div className="rounded-md border border-line bg-raised/50 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone="accent">Niveau {classification.level}</Badge>
        <span className="text-sm font-medium text-ink">{classification.label}</span>
      </div>
      <p className="mt-1.5 text-xs text-dim">Pourquoi : {classification.rationale}.</p>
      <p className="mt-1 font-mono text-[11px] text-faint">
        {signals.words} mots · {signals.headings} sections · {signals.bullets} puces · {signals.enumerated_items}{" "}
        livrables énumérés
      </p>
    </div>
  );
}

function TreeNode({
  plan,
  node,
  depth,
  leafIds,
  tasks,
}: {
  plan: Plan;
  node: TaskNode;
  depth: number;
  leafIds: Set<string>;
  tasks?: Record<string, TaskRuntime>;
}) {
  const [open, setOpen] = useState(false);
  const children = plan.nodes.filter((candidate) => candidate.parent === node.id);
  const leaf = leafIds.has(node.id);
  const critical = plan.critical_path.includes(node.id);
  const sprint = plan.sprints.find((candidate) => candidate.task_ids.includes(node.id));
  const runtime = tasks?.[node.id];

  return (
    <li>
      <div
        className={cn("rounded-md border border-transparent px-2 py-1.5", leaf && "border-line bg-raised/40")}
        style={{ marginLeft: depth * 18 }}
      >
        <div className="flex flex-wrap items-center gap-1.5">
          {leaf ? (
            <button
              type="button"
              onClick={() => setOpen((value) => !value)}
              aria-expanded={open}
              aria-label={`${open ? "Masquer" : "Afficher"} le détail de ${node.id}`}
              className="rounded p-0.5 text-faint hover:bg-hover hover:text-ink"
            >
              {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
            </button>
          ) : null}
          <Badge tone={node.kind === "goal" ? "accent" : node.kind === "task" ? "neutral" : "info"}>
            {KIND_LABEL[node.kind]}
          </Badge>
          <span className="font-mono text-[11px] text-faint">{node.id}</span>
          <span className={cn("text-sm", leaf ? "text-ink" : "font-medium text-ink")}>{node.title}</span>
          {leaf ? (
            <>
              <Badge>{COMPONENT_LABEL[node.component] ?? node.component}</Badge>
              <Badge>{node.estimate_points} pts</Badge>
              <RiskBadge score={node.risk.score} />
              {sprint ? <Badge tone="info">S{sprint.id}</Badge> : null}
              {critical ? (
                <Badge tone="warn" title="Sur le chemin critique : tout retard décale la livraison">
                  <Milestone /> critique
                </Badge>
              ) : null}
              {runtime ? <TaskStatusBadge status={runtime.status} /> : null}
            </>
          ) : null}
        </div>
        {leaf && node.depends_on.length > 0 ? (
          <p className="mt-1 flex items-center gap-1 pl-6 font-mono text-[11px] text-faint">
            <CornerDownLeft className="size-3" /> dépend de {node.depends_on.join(", ")}
          </p>
        ) : null}
        {leaf && open ? (
          <div className="mt-2 space-y-2 border-t border-line pt-2 pl-6 text-xs text-dim">
            {node.description ? <p>{node.description}</p> : null}
            <div>
              <p className="font-medium text-ink">Critères d&apos;acceptation</p>
              <ul className="mt-1 list-disc space-y-0.5 pl-4">
                {node.acceptance_criteria.map((criterion, index) => (
                  <li key={index}>{criterion}</li>
                ))}
              </ul>
            </div>
            <p>
              <span className="font-medium text-ink">Risque</span> — probabilité {node.risk.probability}/5 × impact{" "}
              {node.risk.impact}/5
              {node.risk.factors.length > 0 ? ` : ${node.risk.factors.join(", ")}` : ""}.
              {node.risk.mitigation ? ` Mitigation : ${node.risk.mitigation}.` : ""}
            </p>
          </div>
        ) : null}
      </div>
      {children.length > 0 ? (
        <ul className="mt-1 space-y-1">
          {children.map((child) => (
            <TreeNode key={child.id} plan={plan} node={child} depth={depth + 1} leafIds={leafIds} tasks={tasks} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

/** Probability × impact heat-map with the task ids in each cell. */
export function RiskMatrix({ plan }: { plan: Plan }) {
  const leaves = plan.execution_order
    .map((id) => plan.nodes.find((node) => node.id === id))
    .filter((node): node is TaskNode => Boolean(node));
  const cell = (probability: number, impact: number) =>
    leaves.filter((node) => node.risk.probability === probability && node.risk.impact === impact);
  const tone = { low: "bg-ok/10 border-ok/20", medium: "bg-warn/12 border-warn/25", high: "bg-bad/14 border-bad/30" };

  return (
    <div>
      <div className="grid grid-cols-[auto_repeat(5,minmax(0,1fr))] gap-1 text-[11px]">
        {[5, 4, 3, 2, 1].map((probability) => (
          <div key={probability} className="contents">
            <div className="flex items-center justify-end pr-1 font-mono text-faint">P{probability}</div>
            {[1, 2, 3, 4, 5].map((impact) => {
              const items = cell(probability, impact);
              return (
                <div
                  key={impact}
                  className={cn(
                    "flex min-h-9 flex-wrap content-center items-center justify-center gap-0.5 rounded border p-0.5",
                    tone[riskLevel(probability * impact)],
                  )}
                  title={`Probabilité ${probability} × impact ${impact} = ${probability * impact}`}
                >
                  {items.map((node) => (
                    <span key={node.id} className="rounded bg-canvas/80 px-1 font-mono text-[10px] text-ink" title={node.title}>
                      {node.id}
                    </span>
                  ))}
                </div>
              );
            })}
          </div>
        ))}
        <div />
        {[1, 2, 3, 4, 5].map((impact) => (
          <div key={impact} className="text-center font-mono text-faint">
            I{impact}
          </div>
        ))}
      </div>
      <p className="mt-1.5 text-[11px] text-faint">P = probabilité (1 rare → 5 quasi certain) · I = impact (1 négligeable → 5 critique)</p>
    </div>
  );
}

export function PlanView({
  plan,
  tasks,
  compact,
}: {
  plan: Plan;
  tasks?: Record<string, TaskRuntime>;
  compact?: boolean;
}) {
  const parents = new Set(plan.nodes.map((node) => node.parent).filter((parent): parent is string => Boolean(parent)));
  const leafIds = new Set(plan.nodes.filter((node) => !parents.has(node.id) && node.kind !== "goal").map((node) => node.id));
  const roots = plan.nodes.filter((node) => !node.parent || !plan.nodes.some((candidate) => candidate.id === node.parent));

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <Stat label="Tâches exécutables" value={String(plan.execution_order.length)} />
        <Stat label="Points" value={String(plan.total_points)} />
        <Stat label="Vagues parallélisables" value={String(plan.waves.length)} />
        <Stat label="Sprints" value={String(plan.sprints.length)} />
      </div>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-dim">
        <span className="flex min-w-48 flex-1 items-center gap-2">
          Confiance du plan <ConfidenceBar value={plan.confidence} className="flex-1" />
        </span>
        <span>
          Planificateur : <span className="font-mono text-ink">{plan.planner}</span>
        </span>
        <span>
          Chemin critique : <span className="font-mono text-ink">{plan.critical_path.join(" → ") || "—"}</span>
        </span>
      </div>
      {plan.warnings.length > 0 ? (
        <ul className="space-y-1 rounded-md border border-warn/30 bg-warn/8 p-2.5 text-xs text-warn">
          {plan.warnings.map((warning, index) => (
            <li key={index} className="flex gap-1.5">
              <TriangleAlert className="mt-0.5 size-3.5 shrink-0" /> {warning}
            </li>
          ))}
        </ul>
      ) : null}

      <section>
        <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-dim uppercase">Réseau hiérarchique de tâches (HTN)</h3>
        <ul className="space-y-1">
          {roots.map((node) => (
            <TreeNode key={node.id} plan={plan} node={node} depth={0} leafIds={leafIds} tasks={tasks} />
          ))}
        </ul>
      </section>

      <div className={cn("grid gap-4", compact ? "grid-cols-1" : "lg:grid-cols-2")}>
        <section>
          <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-dim uppercase">Plan de sprints</h3>
          <div className="space-y-1.5">
            {plan.sprints.map((sprint) => (
              <div key={sprint.id} className="rounded-md border border-line bg-raised/40 p-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-sm font-medium text-ink">{sprint.name}</span>
                  <span className="flex items-center gap-1.5">
                    <Badge>{sprint.points} pts</Badge>
                    <RiskBadge score={sprint.max_risk_score} />
                  </span>
                </div>
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {sprint.task_ids.map((id) => (
                    <span
                      key={id}
                      className="rounded bg-canvas px-1.5 py-0.5 font-mono text-[11px] text-dim"
                      title={plan.nodes.find((node) => node.id === id)?.title}
                    >
                      {id}
                    </span>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </section>
        <section>
          <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-dim uppercase">Matrice des risques</h3>
          <RiskMatrix plan={plan} />
        </section>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-line bg-raised/40 px-3 py-2">
      <p className="font-mono text-lg leading-6 text-ink tabular-nums">{value}</p>
      <p className="text-[11px] text-faint">{label}</p>
    </div>
  );
}
