"use client";

import { RotateCcw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Segmented } from "@/components/ui/segmented";
import { Tooltip } from "@/components/ui/tooltip";
import { REPLAY_SPEEDS, replayControls, useReplayStatus, type ReplaySpeed } from "@/lib/demo";

/** Position, speed and restart of a recorded run (demo mode only). */
export function ReplayControls({ runId }: { runId: string }) {
  const status = useReplayStatus(runId);
  if (!status) return null;
  const position = status.finished
    ? "rejeu terminé"
    : status.waitingGate
      ? "en attente de votre décision"
      : "rejeu en cours";
  return (
    <div
      className="flex flex-wrap items-center gap-2 rounded-md border border-violet/30 bg-violet/8 px-2 py-1"
      role="group"
      aria-label="Contrôles du rejeu"
    >
      <Tooltip label="Ce run est un enregistrement : ses événements sont rejoués dans l'ordre, avec une pause à chaque portail de validation.">
        <span tabIndex={0} className="font-mono text-[11px] text-dim tabular-nums">
          {status.released}/{status.total} · {position}
        </span>
      </Tooltip>
      <Segmented<string>
        label="Vitesse du rejeu"
        value={String(status.speed)}
        onChange={(value) => replayControls.setSpeed(runId, Number(value) as ReplaySpeed)}
        options={REPLAY_SPEEDS.map((speed) => ({ value: String(speed), label: `${speed}×` }))}
      />
      <Button size="sm" onClick={() => replayControls.restart(runId)}>
        <RotateCcw /> Recommencer
      </Button>
    </div>
  );
}
