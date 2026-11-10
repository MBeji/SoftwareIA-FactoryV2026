"use client";

import { useSyncExternalStore } from "react";

import { api } from "./api";
import type { Meta } from "./types";

// Module-level store: the factory description is fetched once per session and shared by
// every component. `useSyncExternalStore` keeps hydration consistent (server snapshot is
// always "not loaded yet"), whatever the order in which route segments hydrate.
let meta: Meta | null = null;
let error: string | null = null;
let started = false;
const listeners = new Set<() => void>();

const notify = () => listeners.forEach((listener) => listener());

function load(): void {
  api.meta().then(
    (loaded) => {
      meta = loaded;
      error = null;
      notify();
    },
    (reason: unknown) => {
      error = reason instanceof Error ? reason.message : "Backend injoignable";
      notify();
      setTimeout(load, 4000);
    },
  );
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!started) {
    started = true;
    load();
  }
  return () => {
    listeners.delete(listener);
  };
}

/** Static description of the factory (roles, routing, stages). */
export function useMeta(): { meta: Meta | null; error: string | null } {
  const current = useSyncExternalStore(
    subscribe,
    () => meta,
    () => null,
  );
  const failure = useSyncExternalStore(
    subscribe,
    () => error,
    () => null,
  );
  return { meta: current, error: failure };
}
