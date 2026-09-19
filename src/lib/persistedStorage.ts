
import { createJSONStorage } from "zustand/middleware";
import type { StateStorage } from "zustand/middleware";

import { log } from "./log";

const WRITE_DEBOUNCE_MS = 1000;

const pending = new Map<string, string>();
const lastWritten = new Map<string, string>();
let timer: ReturnType<typeof setTimeout> | null = null;

function flush(): void {
  if (timer !== null) {
    clearTimeout(timer);
    timer = null;
  }
  for (const [key, value] of pending) {
    try {
      localStorage.setItem(key, value);
      lastWritten.set(key, value);
    } catch (e) {
      // Usually the quota: the state is lost at the next launch.
      log.warn("storage", `saving ${key} failed`, e);
    }
  }
  pending.clear();
}

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", flush);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flush();
  });
}

const debouncedStorage: StateStorage = {
  getItem: (name) => {
    const queued = pending.get(name);
    if (queued !== undefined) return queued;
    try {
      return localStorage.getItem(name);
    } catch {
      return null;
    }
  },
  setItem: (name, value) => {
    if (lastWritten.get(name) === value) {
      pending.delete(name);
      return;
    }
    pending.set(name, value);
    if (timer === null) timer = setTimeout(flush, WRITE_DEBOUNCE_MS);
  },
  removeItem: (name) => {
    pending.delete(name);
    lastWritten.delete(name);
    try {
      localStorage.removeItem(name);
    } catch {
    }
  },
};

export function debouncedJSONStorage<T>() {
  return createJSONStorage<T>(() => debouncedStorage);
}

export const flushPersistedWrites = flush;
