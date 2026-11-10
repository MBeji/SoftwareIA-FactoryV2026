"use client";

// Live run state: snapshot check, then WebSocket (replay + live) with exponential backoff,
// and HTTP polling as a fallback when the socket cannot be opened.

import { useEffect, useReducer, useRef, useState } from "react";

import { api, ApiError } from "./api";
import { demoEnabled, subscribeReplay } from "./demo";
import { isActive, reduceEvent, type ViewState } from "./reducer";
import type { FactoryEvent } from "./types";

export type Connection = "connecting" | "live" | "reconnecting" | "polling" | "offline" | "replay";

interface StreamState {
  view: ViewState | null;
  events: FactoryEvent[];
}

function streamReducer(state: StreamState, incoming: FactoryEvent[]): StreamState {
  let view = state.view;
  const accepted: FactoryEvent[] = [];
  let last = state.events.length > 0 ? state.events[state.events.length - 1].seq : 0;
  for (const event of incoming) {
    if (event.seq <= last) continue;
    last = event.seq;
    view = reduceEvent(view, event);
    accepted.push(event);
  }
  return accepted.length === 0 ? state : { view, events: state.events.concat(accepted) };
}

export interface RunStream {
  view: ViewState | null;
  events: FactoryEvent[];
  connection: Connection;
  /** The run does not exist on the backend. */
  notFound: boolean;
  /** The backend says the run was active when the server stopped. */
  interrupted: boolean;
  /** Blocking load error (backend unreachable before any data arrived). */
  loadError: string | null;
}

const FLUSH_MS = 40;
const POLL_MS = 1500;
const SNAPSHOT_MS = 20_000;

/**
 * Streams one run. The run id must stay constant for the lifetime of the component:
 * mount the consumer with `key={runId}`.
 */
export function useRunStream(runId: string): RunStream {
  const [state, dispatch] = useReducer(streamReducer, { view: null, events: [] });
  const [connection, setConnection] = useState<Connection>("connecting");
  const [notFound, setNotFound] = useState(false);
  const [interrupted, setInterrupted] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Survives effect re-runs (hidden/visible route, reconnects) so the stream resumes
  // exactly where it stopped instead of replaying everything.
  const lastSeq = useRef(0);

  useEffect(() => {
    let disposed = false;
    let socket: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let snapshotTimer: ReturnType<typeof setInterval> | null = null;
    let flushTimer: ReturnType<typeof setTimeout> | null = null;
    let attempts = 0;
    let everOpened = false;
    let stopReplay: (() => void) | null = null;
    const buffer: FactoryEvent[] = [];

    const flush = () => {
      flushTimer = null;
      if (buffer.length > 0) dispatch(buffer.splice(0));
    };

    const push = (events: FactoryEvent[]) => {
      for (const event of events) {
        if (event.seq > lastSeq.current) {
          lastSeq.current = event.seq;
          buffer.push(event);
        }
      }
      if (buffer.length > 0 && flushTimer === null) flushTimer = setTimeout(flush, FLUSH_MS);
    };

    const poll = async () => {
      try {
        push(await api.getEvents(runId, lastSeq.current));
        if (!disposed && !everOpened) setConnection("polling");
      } catch {
        if (!disposed) setConnection("offline");
      }
    };

    const startPolling = () => {
      if (pollTimer === null) {
        void poll();
        pollTimer = setInterval(() => void poll(), POLL_MS);
      }
    };

    const stopPolling = () => {
      if (pollTimer !== null) {
        clearInterval(pollTimer);
        pollTimer = null;
      }
    };

    const connect = () => {
      if (disposed) return;
      if (demoEnabled()) {
        // Demo mode: a recorded run is replayed through the same pipeline as live events.
        subscribeReplay(runId, lastSeq.current, push).then(
          (unsubscribe) => {
            if (disposed) unsubscribe();
            else {
              stopReplay = unsubscribe;
              setConnection("replay");
            }
          },
          () => {
            if (!disposed) setConnection("offline");
          },
        );
        return;
      }
      try {
        socket = new WebSocket(api.streamUrl(runId, lastSeq.current));
      } catch {
        startPolling();
        return;
      }
      const current = socket;
      current.onopen = () => {
        attempts = 0;
        everOpened = true;
        stopPolling();
        if (!disposed) setConnection("live");
      };
      current.onmessage = (message) => {
        try {
          push([JSON.parse(String(message.data)) as FactoryEvent]);
        } catch {
          // Ignore malformed frames; the next reconnect replays from the last valid seq.
        }
      };
      current.onerror = () => current.close();
      current.onclose = () => {
        if (disposed) return;
        everOpened = false;
        attempts += 1;
        // The socket keeps failing: keep the UI alive over plain HTTP meanwhile.
        if (attempts >= 2) startPolling();
        else setConnection("reconnecting");
        reconnectTimer = setTimeout(connect, Math.min(8000, 500 * 2 ** attempts));
      };
    };

    const checkSnapshot = async (initial: boolean) => {
      try {
        const snapshot = await api.getRun(runId);
        if (disposed) return;
        setLoadError(null);
        setNotFound(false);
        setInterrupted(snapshot.status === "interrupted");
        if (initial) connect();
        if (!isActive(snapshot.status) && snapshotTimer !== null) {
          clearInterval(snapshotTimer);
          snapshotTimer = null;
        }
      } catch (error) {
        if (disposed) return;
        if (error instanceof ApiError && error.status === 404) {
          setNotFound(true);
          return;
        }
        if (initial) {
          setConnection("offline");
          setLoadError(error instanceof Error ? error.message : "Erreur inconnue");
          reconnectTimer = setTimeout(() => void checkSnapshot(true), 3000);
        }
      }
    };

    void checkSnapshot(true);
    snapshotTimer = setInterval(() => void checkSnapshot(false), SNAPSHOT_MS);

    return () => {
      // Events already acknowledged through `lastSeq` must not be lost on teardown.
      flush();
      disposed = true;
      if (socket) {
        socket.onclose = null;
        socket.close();
      }
      if (stopReplay) stopReplay();
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      if (flushTimer !== null) clearTimeout(flushTimer);
      if (snapshotTimer !== null) clearInterval(snapshotTimer);
      stopPolling();
    };
  }, [runId]);

  return { view: state.view, events: state.events, connection, notFound, interrupted, loadError };
}
