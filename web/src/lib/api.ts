// Typed client for the OmniForge backend (REST + WebSocket URL builder).

import type {
  ExecResult,
  FactoryEvent,
  FileEntry,
  GateDecision,
  Meta,
  PlanPreview,
  RunState,
  RunSummary,
  SupervisionPolicy,
} from "./types";

export const API_URL = (process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080").replace(/\/+$/, "");
const API_KEY = process.env.NEXT_PUBLIC_API_KEY ?? "";

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }

  /** The backend could not be reached at all (network error, CORS, server down). */
  get unreachable(): boolean {
    return this.status === 0;
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (API_KEY) headers.set("Authorization", `Bearer ${API_KEY}`);
  if (init.body && !(init.body instanceof FormData)) headers.set("Content-Type", "application/json");

  let response: Response;
  try {
    response = await fetch(`${API_URL}${path}`, { ...init, headers, cache: "no-store" });
  } catch {
    throw new ApiError(0, `Backend injoignable (${API_URL})`);
  }
  const text = await response.text();
  let body: unknown = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = null;
  }
  if (!response.ok) {
    const message =
      body && typeof body === "object" && "error" in body && typeof body.error === "string"
        ? body.error
        : `HTTP ${response.status}`;
    throw new ApiError(response.status, message);
  }
  return body as T;
}

const json = (value: unknown): string => JSON.stringify(value);

export interface NewRunInput {
  input: string;
  title?: string;
  level?: number | null;
  supervision: SupervisionPolicy;
}

export const api = {
  meta: () => request<Meta>("/api/meta"),

  listRuns: () => request<{ runs: RunSummary[] }>("/api/runs").then((r) => r.runs),

  createRun: (run: NewRunInput) =>
    request<RunState>("/api/runs", {
      method: "POST",
      body: json({
        input: run.input,
        title: run.title || undefined,
        level: run.level ?? undefined,
        supervision: run.supervision,
      }),
    }),

  /** Document upload (.md, .txt, .pdf): text extraction happens server-side. */
  uploadRun: (file: File, run: Omit<NewRunInput, "input">) => {
    const form = new FormData();
    form.append("file", file, file.name);
    if (run.title) form.append("title", run.title);
    if (run.level) form.append("level", String(run.level));
    form.append("supervision", json(run.supervision));
    return request<RunState>("/api/runs/upload", { method: "POST", body: form });
  },

  previewPlan: (input: string, level?: number | null) =>
    request<PlanPreview>("/api/plan", { method: "POST", body: json({ input, level: level ?? undefined }) }),

  getRun: (id: string) => request<RunState>(`/api/runs/${encodeURIComponent(id)}`),

  getEvents: (id: string, since: number) =>
    request<{ events: FactoryEvent[] }>(`/api/runs/${encodeURIComponent(id)}/events?since=${since}`).then(
      (r) => r.events,
    ),

  resolveGate: (id: string, gateId: string, decision: GateDecision, feedback: string | null, resolver: string) =>
    request<{ ok: boolean }>(
      `/api/runs/${encodeURIComponent(id)}/gates/${encodeURIComponent(gateId)}/resolve`,
      { method: "POST", body: json({ decision, feedback, resolver }) },
    ),

  setSupervision: (id: string, policy: SupervisionPolicy) =>
    request<SupervisionPolicy>(`/api/runs/${encodeURIComponent(id)}/supervision`, {
      method: "PUT",
      body: json(policy),
    }),

  cancel: (id: string) => request<{ ok: boolean }>(`/api/runs/${encodeURIComponent(id)}/cancel`, { method: "POST" }),

  terminal: (id: string, command: string) =>
    request<ExecResult>(`/api/runs/${encodeURIComponent(id)}/terminal`, { method: "POST", body: json({ command }) }),

  tree: (id: string) =>
    request<{ files: FileEntry[] }>(`/api/runs/${encodeURIComponent(id)}/workspace/tree`).then((r) => r.files),

  file: (id: string, path: string) =>
    request<{ path: string; content: string }>(
      `/api/runs/${encodeURIComponent(id)}/workspace/file?path=${encodeURIComponent(path)}`,
    ),

  diff: (id: string) =>
    request<{ diff: string }>(`/api/runs/${encodeURIComponent(id)}/workspace/diff`).then((r) => r.diff),

  /** WebSocket URL of the live event stream (replays everything after `since`). */
  streamUrl: (id: string, since: number): string => {
    const base = API_URL.replace(/^http/, "ws");
    const token = API_KEY ? `&token=${encodeURIComponent(API_KEY)}` : "";
    return `${base}/api/runs/${encodeURIComponent(id)}/stream?since=${since}${token}`;
  },
};
