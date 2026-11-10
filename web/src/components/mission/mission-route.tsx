"use client";

import { useParams } from "next/navigation";

import { MissionControl } from "@/components/mission/mission-control";
import { useReplayStatus } from "@/lib/demo";
import { useMarkSettled } from "@/lib/settled";

/**
 * Resolves the run id from the URL; the key gives every run a fresh stream — and, in demo
 * mode, every restart of a replay a fresh state.
 */
export function MissionRoute() {
  const { id } = useParams<{ id: string }>();
  const generation = useReplayStatus(id)?.generation ?? 0;
  useMarkSettled();
  return <MissionControl key={`${id}:${generation}`} runId={id} />;
}
