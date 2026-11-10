import { Ban, CircleAlert, CircleCheck, CirclePause, CircleX, Clock, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";

import { Badge, type BadgeTone } from "@/components/ui/badge";
import { MODE_LABEL, STATUS_LABEL, TASK_STATUS_LABEL, formatPercent, riskLevel } from "@/lib/format";
import type { RunStatus, Severity, SupervisionMode, TaskStatus } from "@/lib/types";
import { cn } from "@/lib/utils";

const STATUS_STYLE: Record<RunStatus, { tone: BadgeTone; icon: ReactNode }> = {
  pending: { tone: "neutral", icon: <Clock /> },
  running: { tone: "accent", icon: <LoaderCircle className="animate-spin" /> },
  waiting_approval: { tone: "warn", icon: <CirclePause /> },
  completed: { tone: "ok", icon: <CircleCheck /> },
  failed: { tone: "bad", icon: <CircleX /> },
  cancelled: { tone: "neutral", icon: <Ban /> },
  interrupted: { tone: "warn", icon: <CircleAlert /> },
};

export function StatusBadge({ status, className }: { status: RunStatus; className?: string }) {
  const style = STATUS_STYLE[status];
  return (
    <Badge tone={style.tone} className={className}>
      {style.icon}
      {STATUS_LABEL[status]}
    </Badge>
  );
}

export const MODE_TONE: Record<SupervisionMode, BadgeTone> = { hitl: "warn", hotl: "accent", autonomous: "violet" };

export function ModeBadge({ mode }: { mode: SupervisionMode }) {
  return <Badge tone={MODE_TONE[mode]}>{MODE_LABEL[mode]}</Badge>;
}

export const TASK_TONE: Record<TaskStatus, BadgeTone> = {
  pending: "neutral",
  red: "bad",
  green: "ok",
  refactor: "violet",
  done: "accent",
  failed: "bad",
};

export function TaskStatusBadge({ status }: { status: TaskStatus }) {
  return <Badge tone={TASK_TONE[status]}>{TASK_STATUS_LABEL[status]}</Badge>;
}

const RISK_TONE: Record<"low" | "medium" | "high", BadgeTone> = { low: "ok", medium: "warn", high: "bad" };

export function RiskBadge({ score }: { score: number }) {
  return (
    <Badge tone={RISK_TONE[riskLevel(score)]} title={`Risque : probabilité × impact = ${score}`}>
      risque {score}
    </Badge>
  );
}

export const SEVERITY_TONE: Record<Severity, BadgeTone> = {
  info: "neutral",
  low: "info",
  medium: "warn",
  high: "bad",
  critical: "bad",
};

/** Confidence gauge with its threshold marker. */
export function ConfidenceBar({
  value,
  threshold,
  className,
}: {
  value: number;
  threshold?: number;
  className?: string;
}) {
  const below = threshold !== undefined && value < threshold;
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <div
        className="relative h-1.5 min-w-16 flex-1 rounded-full bg-raised"
        role="meter"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(value * 100)}
        aria-label="Confiance"
      >
        <div
          className={cn("h-full rounded-full", below ? "bg-warn" : "bg-ok")}
          style={{ width: `${Math.max(0, Math.min(1, value)) * 100}%` }}
        />
        {threshold !== undefined ? (
          <div
            className="absolute -top-1 h-3.5 w-px bg-ink"
            style={{ left: `${threshold * 100}%` }}
            title={`Seuil : ${formatPercent(threshold)}`}
          />
        ) : null}
      </div>
      <span className={cn("font-mono text-[11px] tabular-nums", below ? "text-warn" : "text-dim")}>
        {formatPercent(value)}
      </span>
    </div>
  );
}
