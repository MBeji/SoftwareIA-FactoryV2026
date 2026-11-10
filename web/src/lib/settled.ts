"use client";

import { useEffect, useSyncExternalStore } from "react";

// "The page content has hydrated." Layout-level UI that alters the rest of the document
// (a modal marks its siblings aria-hidden) waits for this before opening by itself, so it
// never touches markup React has not hydrated yet.
let settled = false;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Call from a page-level client component. */
export function useMarkSettled(): void {
  useEffect(() => {
    if (settled) return;
    settled = true;
    listeners.forEach((listener) => listener());
  }, []);
}

export function useSettled(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => settled,
    () => false,
  );
}
