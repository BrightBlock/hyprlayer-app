/**
 * The doc files the app has actually seen written, keyed by filename.
 *
 * `docFromTool` can list the `.md` paths a Bash command mentions, but not which
 * of them the command *wrote* — that needs shell grammar, and even then
 * `"$THOUGHTS/…/x.md"`, `~/…`, and the relative `thoughts/shared/plans/…` the
 * skills are told to use (`_thoughts/storage-backend.md`) name a file no
 * consumer can open. The thoughts index answers both questions: its watcher
 * emits `thoughts-index-updated` with absolute doc ids on every create and
 * modify, so a mention that turns up here was written, and turns up with the
 * canonical path the doc viewer wants.
 *
 * Keyed by filename because that is the only part of a mention that survives
 * unexpanded: `cat > "$OUT"` never spells the directory, but doc filenames are
 * date-prefixed slugs (`2026-08-18-thing.md`) and collide only if two docs of
 * the same name are written in one session.
 *
 * Persisted because a transcript outlives the events that explain it: `items`
 * replay from SQLite on restart (`hydrateSessionFromLog`) while the index
 * events that confirmed their docs fired in the previous run.
 *
 * The accepted gap: the map is app-wide, because the events carry no workstream.
 * So once a doc is written, any later command naming that filename resolves it,
 * including from another workstream. Harmless in the writing session (it already
 * listed the doc, and both selectors dedup by path); elsewhere it takes a real
 * write plus a second workstream naming that exact file, which beats the old
 * failure mode of any `grep` of any plan registering as authorship.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import { events } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";

// `baseName` rather than `lib/path`'s `basename`: the map's keys and the lookup
// in `docFromTool` have to be the same function or a mention silently misses.
import { baseName, docKindForPath } from "./toolDisplay";

/** Filename → absolute indexed path, insertion-ordered (oldest first). */
export type WrittenDocs = Record<string, string>;

type WrittenDocsState = { byName: WrittenDocs };

// Enough to cover any plausible session's worth of docs while keeping the
// persisted blob small; the oldest entries fall off first.
const MAX_ENTRIES = 500;

export const useWrittenDocsStore = create<WrittenDocsState>()(
  persist<WrittenDocsState>(() => ({ byName: {} }), {
    name: "hyprlayer:written-docs",
    version: 1,
  }),
);

/** Note each id as written. Non-doc paths are dropped so the map stays scoped
 *  to what `docFromTool` can ask about. */
export function recordWrittenDocs(ids: string[]): void {
  const docIds = ids.filter((id) => docKindForPath(id) !== null);
  if (docIds.length === 0) return;
  useWrittenDocsStore.setState((s) => {
    const byName = { ...s.byName };
    for (const id of docIds) {
      // Delete first so a re-write moves the entry to the end (newest), rather
      // than keeping its original position and aging out while still in use.
      delete byName[baseName(id)];
      byName[baseName(id)] = id;
    }
    const names = Object.keys(byName);
    for (const name of names.slice(0, Math.max(0, names.length - MAX_ENTRIES))) {
      delete byName[name];
    }
    return { byName };
  });
}

/** Forget deleted docs, so a path the user removed stops resolving. */
export function forgetWrittenDocs(ids: string[]): void {
  useWrittenDocsStore.setState((s) => {
    const byName = { ...s.byName };
    let changed = false;
    for (const id of ids) {
      const name = baseName(id);
      // Only if this id owns the name; a same-named doc written since must stay.
      if (byName[name] === id) {
        delete byName[name];
        changed = true;
      }
    }
    return changed ? { byName } : s;
  });
}

/** This feature's slice of the backend event stream: keep the written-doc map in
 *  step with the thoughts index. Registered from main.tsx via
 *  `registerEventBridge` so lib/events never imports feature stores. */
export const writtenDocsBridge: EventBridge = () => [
  events.thoughtsIndexUpdated.listen((event) => {
    const { kind, ids } = event.payload;
    // "rescanned" carries no ids (the whole tree was re-read, which says nothing
    // about who wrote what), so there is nothing to record.
    if (kind === "updated") recordWrittenDocs(ids);
    else if (kind === "removed") forgetWrittenDocs(ids);
  }),
];

/** Subscribe to the written-doc map. Its identity changes only when the map
 *  does, so selectors and memos keyed on it recompute exactly then. */
export function useWrittenDocs(): WrittenDocs {
  return useWrittenDocsStore((s) => s.byName);
}
