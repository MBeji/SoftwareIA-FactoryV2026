"use client";

import {
  ArrowLeft,
  Ban,
  Check,
  Coins,
  FileDiff,
  FlaskConical,
  MessagesSquare,
  Settings2,
  TriangleAlert,
  Wifi,
  WifiOff,
  Wrench,
} from "lucide-react";
import Link from "next/link";
import { useState, type ReactNode } from "react";

import { StatusBadge } from "@/components/status";
import { SupervisionSelector } from "@/components/supervision-selector";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Segmented } from "@/components/ui/segmented";
import { Tooltip } from "@/components/ui/tooltip";
import { api } from "@/lib/api";
import { MODE_LABEL, MODE_LONG, PHASES, formatCost, formatTokens } from "@/lib/format";
import type { Connection } from "@/lib/use-run-stream";
import type { Meta, Phase, RunState, RunStatus, SupervisionMode, SupervisionPolicy } from "@/lib/types";
import { cn } from "@/lib/utils";

const CONNECTION: Record<Connection, { label: string; tone: string; live: boolean }> = {
  connecting: { label: "Connexion…", tone: "text-faint", live: false },
  live: { label: "Flux en direct", tone: "text-ok", live: true },
  reconnecting: { label: "Reconnexion…", tone: "text-warn", live: false },
  polling: { label: "Mode dégradé (interrogation HTTP)", tone: "text-warn", live: true },
  offline: { label: "Backend injoignable", tone: "text-bad", live: false },
};

function PhaseStepper({ phase, status }: { phase: Phase; status: RunStatus }) {
  const current = phase === "done" ? PHASES.length : PHASES.findIndex((step) => step.id === phase);
  const stopped = status === "failed" || status === "cancelled" || status === "interrupted";
  return (
    <ol className="flex min-w-0 items-center gap-1" aria-label="Phases du workflow">
      {PHASES.map((step, index) => {
        const done = index < current;
        const active = index === current;
        return (
          <li key={step.id} className="flex min-w-0 items-center gap-1">
            <Tooltip label={step.hint}>
              <span
                tabIndex={0}
                aria-current={active ? "step" : undefined}
                className={cn(
                  "flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs whitespace-nowrap",
                  done && "border-ok/30 bg-ok/8 text-ok",
                  active && !stopped && "border-accent bg-accent/12 font-semibold text-accent",
                  active && stopped && "border-bad/40 bg-bad/10 font-semibold text-bad",
                  !done && !active && "border-line text-faint",
                )}
              >
                {done ? (
                  <Check className="size-3" />
                ) : (
                  <span className={cn("font-mono text-[10px]", active && !stopped && "animate-pulse")}>{index + 1}</span>
                )}
                {step.label}
              </span>
            </Tooltip>
            {index < PHASES.length - 1 ? (
              <span className={cn("h-px w-3 shrink-0 xl:w-5", done ? "bg-ok/50" : "bg-line-strong")} aria-hidden />
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}

function Metric({ icon, label, value, hint }: { icon: ReactNode; label: string; value: string; hint?: string }) {
  const body = (
    <span tabIndex={hint ? 0 : undefined} className="flex items-center gap-1.5 rounded-md border border-line bg-raised/50 px-2 py-1 whitespace-nowrap">
      <span className="text-faint [&_svg]:size-3.5">{icon}</span>
      <span className="font-mono text-xs text-ink tabular-nums">{value}</span>
      <span className="text-[11px] text-faint">{label}</span>
    </span>
  );
  return hint ? <Tooltip label={hint}>{body}</Tooltip> : body;
}

export function RunHeader({
  run,
  status,
  connection,
  meta,
}: {
  run: RunState;
  status: RunStatus;
  connection: Connection;
  meta: Meta | null;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [cancelOpen, setCancelOpen] = useState(false);
  const [draft, setDraft] = useState<SupervisionPolicy>(run.supervision);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const active = status === "pending" || status === "running" || status === "waiting_approval";
  const simulated = run.llm_backend === "simulated";
  const link = CONNECTION[connection];
  const tokens = run.metrics.llm.prompt_tokens + run.metrics.llm.completion_tokens;

  const applyPolicy = async (policy: SupervisionPolicy): Promise<boolean> => {
    setBusy(true);
    setError(null);
    try {
      await api.setSupervision(run.id, policy);
      return true;
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Erreur inconnue");
      return false;
    } finally {
      setBusy(false);
    }
  };

  const cancel = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.cancel(run.id);
      setCancelOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Erreur inconnue");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="shrink-0 rounded-lg border border-line bg-panel px-3 py-2.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <Button asChild variant="ghost" size="icon" aria-label="Retour à la liste des runs">
          <Link href="/">
            <ArrowLeft />
          </Link>
        </Button>
        <div className="min-w-0 flex-1 basis-64">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-base font-semibold text-ink" title={run.title}>
              {run.title || run.id}
            </h1>
            <StatusBadge status={status} className="shrink-0" />
          </div>
          <p className="mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-0.5 font-mono text-[11px] text-faint">
            <span>{run.id}</span>
            {run.classification ? (
              <span className="font-sans">
                Niveau {run.classification.level} · {run.classification.label}
              </span>
            ) : null}
            {run.filename ? <span>{run.filename}</span> : null}
            <span className={cn("flex items-center gap-1 font-sans", link.tone)} role="status">
              {link.live ? <Wifi className="size-3" /> : <WifiOff className="size-3" />}
              {link.label}
            </span>
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Tooltip label={`${MODE_LONG[run.supervision.default_mode].name} — ${MODE_LONG[run.supervision.default_mode].detail} Le changement s'applique au prochain point de contrôle.`}>
            <span>
              <Segmented<SupervisionMode>
                label="Mode de supervision du run"
                value={run.supervision.default_mode}
                disabled={!active || busy}
                onChange={(mode) => void applyPolicy({ ...run.supervision, default_mode: mode })}
                options={(["hitl", "hotl", "autonomous"] as SupervisionMode[]).map((mode) => ({
                  value: mode,
                  label: MODE_LABEL[mode],
                  title: MODE_LONG[mode].tagline,
                }))}
              />
            </span>
          </Tooltip>
          <Button
            variant="secondary"
            size="sm"
            disabled={!active || !meta}
            onClick={() => {
              setDraft(run.supervision);
              setError(null);
              setSettingsOpen(true);
            }}
          >
            <Settings2 /> Matrice
            {Object.keys(run.supervision.stage_overrides).length > 0 ? (
              <Badge tone="accent">{Object.keys(run.supervision.stage_overrides).length}</Badge>
            ) : null}
          </Button>
          <Button variant="dangerOutline" size="sm" disabled={!active} onClick={() => setCancelOpen(true)}>
            <Ban /> Annuler
          </Button>
        </div>
      </div>

      <div className="mt-2.5 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <div className="min-w-0 overflow-x-auto">
          <PhaseStepper phase={run.phase} status={status} />
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Metric icon={<MessagesSquare />} value={String(run.metrics.llm.calls)} label="appels LLM" />
          <Metric icon={<span className="font-mono text-[10px]">tok</span>} value={formatTokens(tokens)} label="tokens" />
          <Metric
            icon={<Coins />}
            value={formatCost(run.metrics.llm.cost_usd)}
            label={simulated ? "estimé (simulé)" : "coût"}
            hint={
              simulated
                ? "Le backend simulé ne dépense rien : ce montant est ce que coûterait le même volume de tokens aux tarifs de repli configurés."
                : run.metrics.budget_limit_usd
                  ? `Plafond du run : ${formatCost(run.metrics.budget_limit_usd)}`
                  : "Aucun plafond de dépense sur ce run"
            }
          />
          <Metric icon={<Wrench />} value={String(run.metrics.tool_calls)} label="outils" />
          <Metric icon={<FlaskConical />} value={String(run.metrics.test_runs)} label="tests" />
          <Metric icon={<FileDiff />} value={String(run.metrics.files_changed)} label="écritures" />
        </div>
      </div>

      {error && !settingsOpen && !cancelOpen ? (
        <p role="alert" className="mt-2 flex items-center gap-1.5 text-xs text-bad">
          <TriangleAlert className="size-3.5" /> {error}
        </p>
      ) : null}

      <Dialog open={settingsOpen} onOpenChange={setSettingsOpen}>
        <DialogContent size="lg">
          <DialogHeader>
            <DialogTitle>Matrice de supervision du run</DialogTitle>
            <DialogDescription>
              Les changements s&apos;appliquent dès le prochain point de contrôle, sans interrompre les agents.
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            {meta ? <SupervisionSelector value={draft} onChange={setDraft} stages={meta.stages} disabled={busy} /> : null}
            {error ? (
              <p role="alert" className="mt-3 flex items-center gap-1.5 text-xs text-bad">
                <TriangleAlert className="size-3.5" /> {error}
              </p>
            ) : null}
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setSettingsOpen(false)}>
              Fermer
            </Button>
            <Button
              variant="primary"
              disabled={busy}
              onClick={async () => {
                if (await applyPolicy(draft)) setSettingsOpen(false);
              }}
            >
              Appliquer
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={cancelOpen} onOpenChange={setCancelOpen}>
        <DialogContent size="md">
          <DialogHeader>
            <DialogTitle>Annuler ce run ?</DialogTitle>
            <DialogDescription>
              Les agents s&apos;arrêtent immédiatement et la sandbox est détruite. Le workspace et le journal
              d&apos;événements restent consultables.
            </DialogDescription>
          </DialogHeader>
          {error ? (
            <DialogBody>
              <p role="alert" className="text-xs text-bad">
                {error}
              </p>
            </DialogBody>
          ) : null}
          <DialogFooter>
            <Button variant="ghost" onClick={() => setCancelOpen(false)}>
              Continuer le run
            </Button>
            <Button variant="danger" disabled={busy} onClick={() => void cancel()}>
              <Ban /> Annuler le run
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
