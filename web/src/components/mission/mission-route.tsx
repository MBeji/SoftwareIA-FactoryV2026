"use client";

import { useParams } from "next/navigation";

import { MissionControl } from "@/components/mission/mission-control";
import { useMarkSettled } from "@/lib/settled";

/** Resolves the run id from the URL; the key gives every run a fresh stream. */
export function MissionRoute() {
  const { id } = useParams<{ id: string }>();
  useMarkSettled();
  return <MissionControl key={id} runId={id} />;
}
