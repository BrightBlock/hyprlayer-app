/**
 * A `localStorage` adapter for zustand's `persist` that coalesces bursts of
 * writes and drops ones that would store what is already there.
 *
 * `persist` writes on every `setState`, and this feature's two stores are
 * updated far faster than the user could care about: `workstreamTabs` takes a
 * `patchShell(..., { draft })` per keystroke, and `writtenDocs` a
 * `recordWrittenDocs` per thoughts-index event. Every one of those rewrote the
 * whole ~27 KB blob.
 *
 * WebKitGTK backs `localStorage` with SQLite in the network process, so the
 * cost does not land on the main thread (300 `setItem` calls of 27 KB measure
 * ~2ms total, ~7us each) — it lands on the disk. Measured on a release install
 * after ~32 hours of use: a 53 KB database (13 pages) behind a **227 MB**
 * write-ahead log holding 55,191 frames — about 4,245 copies of the whole
 * database, and essentially the entire lifetime write volume of the network
 * process. Every other install of the app on the same machine was under 120 KB.
 *
 * Debouncing keeps the persistence contract intact. A draft still survives an
 * app restart (`workstreamTabs`' whole reason for persisting); the exposure is
 * the last {@link WRITE_DEBOUNCE_MS} of typing, and only if the process dies
 * without unwinding, since `pagehide`/`visibilitychange` flush on the way out.
 */

import { createJSONStorage } from "zustand/middleware";
import type { StateStorage } from "zustand/middleware";

/** Long enough that a burst of keystrokes collapses into one write, short
 *  enough that a crash costs less than a sentence. */
const WRITE_DEBOUNCE_MS = 1000;

/** Pending values by key, plus what was last handed to `localStorage` for it,
 *  so an unchanged blob (a `setState` that touched nothing this store keeps)
 *  writes nothing at all. */
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
    } catch {
      // Quota or a storage-disabled context: dropping the write loses
      // persistence, not correctness — the in-memory store is still the truth
      // for this run. Leave `lastWritten` alone so the next flush retries.
    }
  }
  pending.clear();
}

// A quit, a reload, or the window being hidden has to land whatever is in
// flight. `pagehide` covers the webview tearing down; `visibilitychange` covers
// the user switching away from a window that is then killed while hidden.
if (typeof window !== "undefined") {
  window.addEventListener("pagehide", flush);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flush();
  });
}

const debouncedStorage: StateStorage = {
  getItem: (name) => {
    // Read the pending value, not the stored one: a rehydrate racing an
    // in-flight write must not see the older blob.
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
      // Already stored verbatim. Drop any queued write of the same bytes too,
      // or a no-op `setState` would keep re-arming the timer.
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
      // Same as `flush`: nothing to recover, the store is still consistent.
    }
  },
};

/** Hand this to `persist`'s `storage` option in place of its default. Generic
 *  so each store gets the storage typed to its own state; the coalescing
 *  bookkeeping above is module-level, so every instance shares one timer. */
export function debouncedJSONStorage<T>() {
  return createJSONStorage<T>(() => debouncedStorage);
}

/** Write everything queued, now. Exported for tests; the listeners above are
 *  what the running app relies on. */
export const flushPersistedWrites = flush;
