// Demo mode: the dashboard without a backend.
//
// A static deployment (Vercel, any CDN) cannot run the factory: the backend needs long-lived
// processes, WebSockets and a container runtime. In demo mode the dashboard replays
// recordings of real runs instead — the event logs captured by `scripts/record-demo.mjs`,
// shipped as static files under `public/demo/`. Recorded events go through the very same
// reducer as live ones, so what is shown is what a connected dashboard would show.
//
// What a recording cannot do is change its past: gate decisions, supervision changes and
// terminal commands are not sent anywhere. Gates still pause the replay until the viewer
// answers, then the recorded decision plays out.

import { useSyncExternalStore } from "react";

import { ApiError } from "./api-error";
import type { FactoryEvent, FileEntry, Meta, PlanPreview, RunState, RunSummary } from "./types";

const FORCED = process.env.NEXT_PUBLIC_DEMO_MODE === "1";
const API_CONFIGURED = Boolean(process.env.NEXT_PUBLIC_API_URL);
const OPT_IN_KEY = "omniforge.demo";

function optedIn(): boolean {
  try {
    return window.sessionStorage.getItem(OPT_IN_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * Demo mode is on when it is forced (`NEXT_PUBLIC_DEMO_MODE=1`), when the viewer chose it
 * after the backend proved unreachable, or when no backend address was configured and the
 * page is not served from a developer machine (the default API address, localhost:8080,
 * would then point at the visitor's own computer).
 */
export function demoEnabled(): boolean {
  if (FORCED) return true;
  if (typeof window === "undefined") return false;
  if (optedIn()) return true;
  if (API_CONFIGURED) return false;
  const host = window.location.hostname;
  return !(host === "localhost" || host === "127.0.0.1" || host === "::1" || host === "[::1]");
}

/** Whether the viewer can go back to a live backend (false on a backend-less deployment). */
export function demoIsOptional(): boolean {
  return !FORCED && typeof window !== "undefined" && optedIn();
}

export function enterDemo(): void {
  try {
    window.sessionStorage.setItem(OPT_IN_KEY, "1");
  } catch {
    // Storage unavailable: nothing to remember, the reload below simply stays live.
  }
  // A full reload on purpose: every cached answer of the previous data source must go.
  window.location.reload();
}

export function leaveDemo(): void {
  try {
    window.sessionStorage.removeItem(OPT_IN_KEY);
  } catch {
    // Same as above.
  }
  window.location.reload();
}

const never = () => () => {};

/** Hydration-safe flag: always `false` on the server and during hydration. */
export function useDemoMode(): boolean {
  return useSyncExternalStore(never, demoEnabled, () => false);
}

// ---------------------------------------------------------------------------------------
// Recordings
// ---------------------------------------------------------------------------------------

export interface DemoRunEntry {
  id: string;
  label: string;
  description: string;
  summary: RunSummary;
  event_count: number;
  gate_count: number;
}

interface DemoIndex {
  version: number;
  recorded_at: string;
  meta: Meta;
  runs: DemoRunEntry[];
  previews: Record<string, { input: string; response: PlanPreview }>;
}

interface DemoRecording {
  id: string;
  state: RunState;
  events: FactoryEvent[];
  files: Array<FileEntry & { content: string }>;
  diff: string | null;
}

async function fetchFixture<T>(name: string): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`/demo/${name}.json`);
  } catch {
    throw new ApiError(0, "Enregistrements de démonstration injoignables");
  }
  if (response.status === 404) throw new ApiError(404, `Aucun enregistrement « ${name} »`);
  if (!response.ok) throw new ApiError(response.status, `Enregistrement « ${name} » illisible (HTTP ${response.status})`);
  return (await response.json()) as T;
}

let indexPromise: Promise<DemoIndex> | null = null;

function loadIndex(): Promise<DemoIndex> {
  indexPromise ??= fetchFixture<DemoIndex>("index").catch((error: unknown) => {
    indexPromise = null; // allow a retry
    throw error;
  });
  return indexPromise;
}

// ---------------------------------------------------------------------------------------
// Replay engine
// ---------------------------------------------------------------------------------------

/** Recorded pauses are compressed: nobody wants to watch a 30 s test run in a demo. */
const MAX_GAP_MS = 1200;
const MIN_GAP_MS = 12;

export const REPLAY_SPEEDS = [1, 2, 4] as const;
export type ReplaySpeed = (typeof REPLAY_SPEEDS)[number];

export interface ReplayStatus {
  /** Bumped on restart: consumers remount to start from an empty state. */
  generation: number;
  speed: ReplaySpeed;
  released: number;
  total: number;
  finished: boolean;
  /** The replay is parked on this gate until the viewer answers it. */
  waitingGate: string | null;
}

type EventListener = (events: FactoryEvent[]) => void;

class Replay {
  private released = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly listeners = new Set<EventListener>();
  private readonly answered = new Set<string>();
  private waitingGate: string | null = null;
  private generation = 0;
  private speed: ReplaySpeed = 1;
  private snapshot: ReplayStatus;
  /** Paths written by agents, with the sequence number of their first write. */
  private readonly firstWrite = new Map<string, number>();

  constructor(readonly recording: DemoRecording) {
    for (const event of recording.events) {
      const path = event.type === "workspace.file_changed" ? event.payload.path : null;
      if (typeof path === "string" && !this.firstWrite.has(path)) this.firstWrite.set(path, event.seq);
    }
    this.snapshot = this.buildSnapshot();
  }

  private buildSnapshot(): ReplayStatus {
    return {
      generation: this.generation,
      speed: this.speed,
      released: this.released,
      total: this.recording.events.length,
      finished: this.released >= this.recording.events.length,
      waitingGate: this.waitingGate,
    };
  }

  status(): ReplayStatus {
    return this.snapshot;
  }

  private changed(): void {
    this.snapshot = this.buildSnapshot();
    notifyStatus();
  }

  releasedEvents(since: number): FactoryEvent[] {
    return this.recording.events.slice(0, this.released).filter((event) => event.seq > since);
  }

  /** Delivers what was already replayed after `since`, then keeps replaying. */
  subscribe(since: number, listener: EventListener): () => void {
    this.listeners.add(listener);
    const backlog = this.releasedEvents(since);
    if (backlog.length > 0) listener(backlog);
    this.schedule();
    return () => {
      this.listeners.delete(listener);
      // Nobody is watching: hold the position instead of playing to an empty room.
      if (this.listeners.size === 0) this.clearTimer();
    };
  }

  private clearTimer(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  private schedule(): void {
    if (this.timer !== null || this.listeners.size === 0) return;
    const next = this.recording.events[this.released];
    if (!next) return;

    let delay = MIN_GAP_MS;
    if (next.type === "gate.resolved") {
      // A human decision was recorded here: wait for the viewer's own click.
      const gateId = String(next.payload.gate_id ?? "");
      if (!this.answered.has(gateId)) {
        if (this.waitingGate !== gateId) {
          this.waitingGate = gateId;
          this.changed();
        }
        return;
      }
    } else {
      const previous = this.recording.events[this.released - 1];
      const gap = previous ? Date.parse(next.ts) - Date.parse(previous.ts) : 0;
      delay = Math.min(MAX_GAP_MS, Math.max(MIN_GAP_MS, Number.isFinite(gap) ? gap : MIN_GAP_MS)) / this.speed;
    }

    this.timer = setTimeout(() => {
      this.timer = null;
      this.released += 1;
      this.changed();
      for (const listener of this.listeners) listener([next]);
      this.schedule();
    }, delay);
  }

  answerGate(gateId: string): void {
    this.answered.add(gateId);
    if (this.waitingGate === gateId) {
      this.waitingGate = null;
      this.changed();
      this.schedule();
    }
  }

  setSpeed(speed: ReplaySpeed): void {
    this.speed = speed;
    this.changed();
    // Re-time the event that is currently waiting.
    this.clearTimer();
    this.schedule();
  }

  restart(): void {
    this.clearTimer();
    this.released = 0;
    this.answered.clear();
    this.waitingGate = null;
    this.generation += 1;
    this.changed();
  }

  /**
   * Files visible at the current replay position: those an agent has already written,
   * plus the ones no agent ever writes (repository scaffolding).
   */
  visibleFiles(): Array<FileEntry & { content: string }> {
    const position = this.released > 0 ? this.recording.events[this.released - 1].seq : 0;
    return this.recording.files.filter((file) => {
      const first = this.firstWrite.get(file.path);
      return first === undefined || first <= position;
    });
  }
}

const replays = new Map<string, Promise<Replay>>();
const loaded = new Map<string, Replay>();
const statusListeners = new Set<() => void>();

function notifyStatus(): void {
  for (const listener of statusListeners) listener();
}

function getReplay(runId: string): Promise<Replay> {
  let pending = replays.get(runId);
  if (!pending) {
    pending = fetchFixture<DemoRecording>(runId).then((recording) => {
      const replay = new Replay(recording);
      loaded.set(runId, replay);
      notifyStatus();
      return replay;
    });
    pending.catch(() => replays.delete(runId));
    replays.set(runId, pending);
  }
  return pending;
}

/** Feeds a consumer with the replay of a recorded run. Resolves to the unsubscribe function. */
export async function subscribeReplay(runId: string, since: number, listener: EventListener): Promise<() => void> {
  return (await getReplay(runId)).subscribe(since, listener);
}

function subscribeStatus(listener: () => void): () => void {
  statusListeners.add(listener);
  return () => {
    statusListeners.delete(listener);
  };
}

/** Live status of a replay (`null` until the recording is loaded, and outside demo mode). */
export function useReplayStatus(runId: string): ReplayStatus | null {
  return useSyncExternalStore(
    subscribeStatus,
    () => loaded.get(runId)?.status() ?? null,
    () => null,
  );
}

export const replayControls = {
  restart: (runId: string) => loaded.get(runId)?.restart(),
  setSpeed: (runId: string, speed: ReplaySpeed) => loaded.get(runId)?.setSpeed(speed),
};

// ---------------------------------------------------------------------------------------
// Demo data source: the same contract as the backend client
// ---------------------------------------------------------------------------------------

const READ_ONLY = "Indisponible en mode démonstration : ce run est un enregistrement, il ne peut pas être modifié.";

export const demoApi = {
  meta: async (): Promise<Meta> => (await loadIndex()).meta,

  entries: async (): Promise<DemoRunEntry[]> => (await loadIndex()).runs,

  listRuns: async (): Promise<RunSummary[]> => (await loadIndex()).runs.map((run) => run.summary),

  createRun: async (): Promise<never> => {
    throw new ApiError(
      403,
      "Le mode démonstration rejoue des runs enregistrés. Connectez un backend OmniForge pour lancer les vôtres.",
    );
  },

  previewPlan: async (input: string): Promise<PlanPreview> => {
    const wanted = input.trim();
    const preview = Object.values((await loadIndex()).previews).find((p) => p.input.trim() === wanted);
    if (!preview) {
      throw new ApiError(
        403,
        "En mode démonstration, l'analyse n'est disponible que pour les trois exemples : elle est calculée par le backend.",
      );
    }
    return preview.response;
  },

  getRun: async (runId: string): Promise<RunState> => (await getReplay(runId)).recording.state,

  getEvents: async (runId: string, since: number): Promise<FactoryEvent[]> =>
    (await getReplay(runId)).releasedEvents(since),

  resolveGate: async (runId: string, gateId: string): Promise<{ ok: boolean }> => {
    (await getReplay(runId)).answerGate(gateId);
    return { ok: true };
  },

  readOnly: async (): Promise<never> => {
    throw new ApiError(403, READ_ONLY);
  },

  tree: async (runId: string): Promise<FileEntry[]> =>
    (await getReplay(runId)).visibleFiles().map(({ path, size }) => ({ path, size })),

  file: async (runId: string, path: string): Promise<{ path: string; content: string }> => {
    const file = (await getReplay(runId)).visibleFiles().find((candidate) => candidate.path === path);
    if (!file) throw new ApiError(404, `Fichier « ${path} » absent de l'enregistrement à ce stade du rejeu`);
    return { path: file.path, content: file.content };
  },

  diff: async (runId: string): Promise<string> => {
    const replay = await getReplay(runId);
    if (replay.recording.diff === null) {
      throw new ApiError(404, "Ce run n'a rien livré : il n'a pas de diff cumulé.");
    }
    if (!replay.status().finished) {
      throw new ApiError(404, "Le diff cumulé de l'enregistrement est disponible à la fin du rejeu.");
    }
    return replay.recording.diff;
  },
};
