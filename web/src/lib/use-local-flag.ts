"use client";

import { useCallback, useSyncExternalStore } from "react";

const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  window.addEventListener("storage", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", listener);
  };
}

function read(key: string): boolean {
  try {
    return window.localStorage.getItem(key) === "1";
  } catch {
    return true; // storage unavailable: behave as if already acknowledged
  }
}

/**
 * Boolean persisted in localStorage. Server rendering (and the first client render)
 * reports `serverValue`, so markup stays stable through hydration.
 */
export function useLocalFlag(key: string, serverValue: boolean): [boolean, (value: boolean) => void] {
  const value = useSyncExternalStore(
    subscribe,
    () => read(key),
    () => serverValue,
  );
  const set = useCallback(
    (next: boolean) => {
      try {
        if (next) window.localStorage.setItem(key, "1");
        else window.localStorage.removeItem(key);
      } catch {
        // ignore: private mode / storage disabled
      }
      listeners.forEach((listener) => listener());
    },
    [key],
  );
  return [value, set];
}
