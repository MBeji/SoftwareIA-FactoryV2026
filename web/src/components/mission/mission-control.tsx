"use client";

import {
  CirclePause,
  FolderTree,
  GitPullRequest,
  ListTree,
  Maximize2,
  Minimize2,
  Network,
  ScrollText,
  SearchX,
  ServerCrash,
  SquareTerminal,
  TriangleAlert,
  Waypoints,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";

import { AgentGraph } from "@/components/mission/agent-graph";
import { Deliverable } from "@/components/mission/deliverable";
import { GatePortal } from "@/components/mission/gate-portal";
import { Journal } from "@/components/mission/journal";
import { MethodPanel } from "@/components/mission/method-panel";
import { RunHeader } from "@/components/mission/run-header";
import { TaskDag } from "@/components/mission/task-dag";
import { TerminalPane } from "@/components/mission/terminal";
import { WorkspacePane } from "@/components/mission/workspace";
import { ClassificationSummary, PlanView } from "@/components/plan-view";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState, Panel } from "@/components/ui/panel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { API_URL } from "@/lib/api";
import { isActive } from "@/lib/reducer";
import type { RunStatus } from "@/lib/types";
import { useMeta } from "@/lib/use-meta";
import { useRunStream } from "@/lib/use-run-stream";
import { cn } from "@/lib/utils";

export function MissionControl({ runId }: { runId: string }) {
  const stream = useRunStream(runId);
  const { meta } = useMeta();
  const [dismissedGate, setDismissedGate] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<"top" | "bottom" | null>(null);
  const [topTab, setTopTab] = useState("agents");
  const [bottomTab, setBottomTab] = useState("terminal");

  if (stream.notFound) {
    return (
      <main className="flex flex-1 items-center justify-center p-6">
        <EmptyState icon={<SearchX />} title={`Run « ${runId} » introuvable`}>
          Ce run n&apos;existe pas (ou plus) sur ce backend.{" "}
          <Link href="/" className="text-accent hover:underline">
            Retour à la liste des runs
          </Link>
        </EmptyState>
      </main>
    );
  }

  const view = stream.view;
  if (!view) {
    return (
      <main className="flex flex-1 items-center justify-center p-6">
        {stream.loadError ? (
          <EmptyState icon={<ServerCrash />} title="Backend injoignable">
            L&apos;API OmniForge ne répond pas sur <span className="font-mono text-dim">{API_URL}</span>. Nouvelle tentative
            automatique…
          </EmptyState>
        ) : (
          <EmptyState title="Connexion au flux d'événements du run…" />
        )}
      </main>
    );
  }

  const { run } = view;
  const status: RunStatus = stream.interrupted && isActive(run.status) ? "interrupted" : run.status;
  const active = isActive(status);
  const pendingGate = active ? (run.gates.find((gate) => !gate.resolution) ?? null) : null;
  const portalOpen = pendingGate !== null && dismissedGate !== pendingGate.gate_id;
  const revision = view.fileChanges.length + view.terminal.length;

  return (
    <main className="flex flex-1 flex-col gap-3 p-3 xl:h-[calc(100dvh-3rem)] xl:flex-none xl:overflow-hidden">
      <RunHeader run={run} status={status} connection={stream.connection} meta={meta} />

      {pendingGate ? (
        <div role="alert" className="flex shrink-0 flex-wrap items-center gap-3 rounded-lg border border-warn/50 bg-warn/10 px-3 py-2">
          <CirclePause className="size-4 shrink-0 text-warn" />
          <p className="min-w-0 flex-1 text-sm text-ink">
            <span className="font-semibold text-warn">Validation requise</span> — {pendingGate.request.title}
            <span className="text-dim"> · {pendingGate.request.reason}</span>
          </p>
          <Button variant="warning" size="sm" onClick={() => setDismissedGate(null)}>
            Ouvrir le portail de validation
          </Button>
        </div>
      ) : null}

      {!active && run.error ? (
        <p
          role="status"
          className={cn(
            "flex shrink-0 items-start gap-2 rounded-lg border px-3 py-2 text-sm",
            status === "failed" ? "border-bad/40 bg-bad/10 text-bad" : "border-line-strong bg-raised text-dim",
          )}
        >
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <span>
            <span className="font-semibold">{status === "failed" ? "Run en échec" : "Run arrêté"}</span> — {run.error}
          </span>
        </p>
      ) : null}
      {status === "interrupted" ? (
        <p role="status" className="flex shrink-0 items-start gap-2 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-sm text-warn">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          Le serveur s&apos;est arrêté pendant ce run : il ne reprendra pas. Le journal et le workspace restent consultables.
        </p>
      ) : null}

      <div className="grid min-h-0 flex-1 gap-3 xl:grid-cols-[minmax(0,1fr)_25.5rem] xl:grid-rows-[minmax(0,1fr)]">
        <div className="flex min-h-0 min-w-0 flex-col gap-3">
          {expanded === "bottom" ? null : (
            <Panel className={cn("flex min-h-0 flex-col", expanded === "top" ? "h-[80vh] xl:h-auto xl:flex-1" : "h-[440px] xl:h-auto xl:flex-[5]")}>
              <Tabs value={topTab} onValueChange={setTopTab} className="flex min-h-0 flex-1 flex-col">
                <div className="flex items-stretch">
                <TabsList className="flex-1">
                  <TabsTrigger value="agents">
                    <Network /> Agents
                    {run.active_agents.length > 0 ? <Badge tone="accent">{run.active_agents.length} actif{run.active_agents.length > 1 ? "s" : ""}</Badge> : null}
                  </TabsTrigger>
                  <TabsTrigger value="tasks">
                    <Waypoints /> Tâches et dépendances
                    {run.plan ? (
                      <Badge>
                        {Object.values(run.tasks).filter((task) => task.status === "done").length}/{run.plan.execution_order.length}
                      </Badge>
                    ) : null}
                  </TabsTrigger>
                </TabsList>
                <div className="flex items-center border-b border-line pr-1.5">
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setExpanded((value) => (value === "top" ? null : "top"))}
                    aria-pressed={expanded === "top"}
                    aria-label={expanded === "top" ? "Réduire les graphes et réafficher le terminal" : "Agrandir les graphes"}
                    title={expanded === "top" ? "Réafficher le terminal" : "Agrandir"}
                  >
                    {expanded === "top" ? <Minimize2 /> : <Maximize2 />}
                  </Button>
                </div>
                </div>
                <TabsContent value="agents" className="relative">
                  {meta ? <AgentGraph view={view} status={status} meta={meta} /> : <EmptyState title="Chargement des rôles…" />}
                </TabsContent>
                <TabsContent value="tasks" className="relative">
                  <TaskDag plan={run.plan} tasks={run.tasks} tdd={run.phase === "implementation" ? run.tdd : null} />
                </TabsContent>
              </Tabs>
            </Panel>
          )}

          {expanded === "top" ? null : (
          <Panel className={cn("flex min-h-0 flex-col", expanded === "bottom" ? "h-[80vh] xl:h-auto xl:flex-1" : "h-[420px] xl:h-auto xl:flex-[4]")}>
            <Tabs value={bottomTab} onValueChange={setBottomTab} className="flex min-h-0 flex-1 flex-col">
              <div className="flex items-stretch">
                <TabsList className="flex-1">
                  <TabsTrigger value="terminal">
                    <SquareTerminal /> Terminal
                  </TabsTrigger>
                  <TabsTrigger value="workspace">
                    <FolderTree /> Workspace
                    {view.fileChanges.length > 0 ? <Badge>{view.fileChanges.length}</Badge> : null}
                  </TabsTrigger>
                  <TabsTrigger value="journal">
                    <ScrollText /> Journal
                    <Badge>{stream.events.length}</Badge>
                  </TabsTrigger>
                  <TabsTrigger value="plan">
                    <ListTree /> Plan
                  </TabsTrigger>
                  <TabsTrigger value="deliverable">
                    <GitPullRequest /> Livrable
                    {run.artifacts.some((artifact) => artifact.kind === "pull_request") ? <Badge tone="ok">MR</Badge> : null}
                  </TabsTrigger>
                </TabsList>
                <div className="flex items-center border-b border-line pr-1.5">
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setExpanded((value) => (value === "bottom" ? null : "bottom"))}
                    aria-pressed={expanded === "bottom"}
                    aria-label={expanded === "bottom" ? "Réduire ce panneau et réafficher les graphes" : "Agrandir ce panneau"}
                    title={expanded === "bottom" ? "Réafficher les graphes" : "Agrandir"}
                  >
                    {expanded === "bottom" ? <Minimize2 /> : <Maximize2 />}
                  </Button>
                </div>
              </div>
              <TabsContent value="terminal">
                <TerminalPane runId={runId} entries={view.terminal} active={active} meta={meta} />
              </TabsContent>
              <TabsContent value="workspace">
                <WorkspacePane runId={runId} changes={view.fileChanges} revision={revision} meta={meta} />
              </TabsContent>
              <TabsContent value="journal">
                <Journal events={stream.events} meta={meta} />
              </TabsContent>
              <TabsContent value="plan" className="overflow-y-auto">
                {run.plan ? (
                  <div className="space-y-4 p-3">
                    {run.classification ? <ClassificationSummary classification={run.classification} /> : null}
                    <PlanView plan={run.plan} tasks={run.tasks} />
                  </div>
                ) : (
                  <EmptyState icon={<ListTree />} title="Le plan n'est pas encore établi">
                    {run.classification ? `Demande classée niveau ${run.classification.level} — ${run.classification.label}.` : "Ingestion en cours…"}
                  </EmptyState>
                )}
              </TabsContent>
              <TabsContent value="deliverable">
                <Deliverable view={view} status={status} />
              </TabsContent>
            </Tabs>
          </Panel>
          )}
        </div>

        {meta ? (
          <MethodPanel view={view} status={status} meta={meta} className="max-h-[80vh] xl:max-h-none" />
        ) : (
          <Panel>
            <EmptyState title="Chargement de la méthode…" />
          </Panel>
        )}
      </div>

      {pendingGate ? (
        <GatePortal
          key={pendingGate.gate_id}
          runId={runId}
          gate={pendingGate}
          open={portalOpen}
          onOpenChange={(open) => setDismissedGate(open ? null : pendingGate.gate_id)}
        />
      ) : null}
    </main>
  );
}
