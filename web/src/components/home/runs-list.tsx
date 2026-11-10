"use client";

import { CirclePause, History, ServerCrash } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";

import { ModeBadge, StatusBadge } from "@/components/status";
import { Badge } from "@/components/ui/badge";
import { EmptyState, Panel, PanelHeader } from "@/components/ui/panel";
import { api, API_URL } from "@/lib/api";
import { demoApi, useDemoMode, type DemoRunEntry } from "@/lib/demo";
import { PHASE_LABEL, formatCost, formatDateTime } from "@/lib/format";
import type { RunSummary } from "@/lib/types";

const REFRESH_MS = 4000;

export function RunsList({ simulated }: { simulated: boolean }) {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const demo = useDemoMode();
  const [recordings, setRecordings] = useState<Record<string, DemoRunEntry>>({});

  useEffect(() => {
    if (!demo) return;
    let disposed = false;
    demoApi.entries().then(
      (entries) => {
        if (!disposed) setRecordings(Object.fromEntries(entries.map((entry) => [entry.id, entry])));
      },
      () => {},
    );
    return () => {
      disposed = true;
    };
  }, [demo]);

  useEffect(() => {
    let disposed = false;
    const load = () =>
      api.listRuns().then(
        (loaded) => {
          if (disposed) return;
          setRuns(loaded);
          setError(null);
        },
        (reason: unknown) => {
          if (!disposed) setError(reason instanceof Error ? reason.message : "Erreur inconnue");
        },
      );
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, []);

  return (
    <Panel className="flex min-h-64 flex-col">
      <PanelHeader
        icon={<History />}
        title={demo ? "Runs enregistrés" : "Runs"}
        actions={runs ? <span className="font-mono text-[11px] text-faint">{runs.length}</span> : null}
      />
      {error && !runs ? (
        <EmptyState icon={<ServerCrash />} title="Backend injoignable">
          Impossible de joindre l&apos;API OmniForge sur <span className="font-mono text-dim">{API_URL}</span>. Démarrez
          le backend (<span className="font-mono text-dim">omniforge serve</span>) ; cette liste se rechargera seule.
        </EmptyState>
      ) : !runs ? (
        <EmptyState title="Chargement des runs…" />
      ) : runs.length === 0 ? (
        <EmptyState icon={<History />} title="Aucun run pour l'instant">
          Lancez une première demande avec le formulaire : un exemple en un clic suffit pour voir l&apos;usine tourner.
        </EmptyState>
      ) : (
        <ul className="divide-y divide-line overflow-y-auto">
          {runs.map((run) => {
            const progress = run.tasks_total > 0 ? run.tasks_done / run.tasks_total : 0;
            return (
              <li key={run.id}>
                <Link href={`/runs/${run.id}`} className="block px-3 py-2.5 transition-colors hover:bg-hover">
                  <div className="flex items-start justify-between gap-2">
                    <span className="min-w-0 truncate text-sm font-medium text-ink">
                      {recordings[run.id]?.label ?? (run.title || run.id)}
                    </span>
                    <StatusBadge status={run.status} className="shrink-0" />
                  </div>
                  {recordings[run.id] ? (
                    <p className="mt-1 text-xs leading-relaxed text-dim">{recordings[run.id].description}</p>
                  ) : null}
                  <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11px] text-faint">
                    <ModeBadge mode={run.mode} />
                    {run.level ? <Badge>Niveau {run.level}</Badge> : null}
                    <span>{PHASE_LABEL[run.phase]}</span>
                    {run.pending_gates > 0 ? (
                      <Badge tone="warn">
                        <CirclePause /> {run.pending_gates} validation{run.pending_gates > 1 ? "s" : ""} en attente
                      </Badge>
                    ) : null}
                  </div>
                  <div className="mt-2 flex items-center gap-2">
                    <div
                      className="h-1 flex-1 overflow-hidden rounded-full bg-raised"
                      role="progressbar"
                      aria-valuemin={0}
                      aria-valuemax={run.tasks_total}
                      aria-valuenow={run.tasks_done}
                      aria-label="Tâches livrées"
                    >
                      <div className="h-full rounded-full bg-accent" style={{ width: `${progress * 100}%` }} />
                    </div>
                    <span className="font-mono text-[11px] text-dim tabular-nums">
                      {run.tasks_done}/{run.tasks_total} tâches
                    </span>
                  </div>
                  <div className="mt-1 flex justify-between font-mono text-[11px] text-faint">
                    <span>{formatDateTime(run.created_at)}</span>
                    <span title={simulated ? "Équivalent estimé : le backend simulé ne dépense rien" : undefined}>
                      {formatCost(run.cost_usd)}
                      {simulated ? " estimé (simulé)" : ""}
                    </span>
                  </div>
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </Panel>
  );
}
