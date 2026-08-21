/**
 * The doc files each workstream's own agents were seen to write, keyed by
 * workstream then by filename.
 *
 * `docFromTool` can list the `.md` paths a Bash command writes *to*
 * (`shellWriteTargets`), but not whether the write landed, or what it resolves
 * to: `"$THOUGHTS/…/x.md"`, `~/…`, and the relative `thoughts/shared/plans/…`
 * the skills are told to use (`_thoughts/storage-backend.md`) name a file no
 * consumer can open. The thoughts index supplies both. Its watcher emits
 * `thoughts-index-updated` with absolute doc ids on every create and modify, so
 * a target that turns up here was written, and turns up in the spelling the doc
 * viewer can open.
 *
 * Keyed by filename within a slice because that is the only part of a target
 * that survives unexpanded: `cat > "$OUT"` never spells the directory, but doc
 * filenames are date-prefixed slugs (`2026-08-18-thing.md`) and collide only if
 * two docs of the same name are written in one workstream.
 *
 * Sliced by workstream because the index watches every project and profile in
 * the thoughts store, not just this one: `hyprlayer thoughts sync`, a second
 * workstream's concurrent agent, an editor save, and a `git checkout` all fire
 * the same event. `recordWrittenDocs` is handed the workstream ids that were
 * working when the event arrived (`sessionStore.activeWorkstreamIds`, via the
 * `registerActiveWorkstreams` seam below), so a doc lands only in the
 * slice(s) of a workstream whose agent could have written it, and a
 * change that happens while nothing is working — sync, an external edit —
 * enters no slice at all.
 *
 * Persisted because a transcript outlives the events that explain it: `items`
 * replay from SQLite on restart (`hydrateSessionFromLog`) while the index
 * events that confirmed their docs fired in the previous run.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import { events } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";

// `baseName` rather than `lib/path`'s `basename`: a slice's keys and the
// lookup in `docFromTool` have to be the same function, or a target misses with
// no error to show for it.
import { baseName, docKindForPath } from "./toolDisplay";

/** Filename → absolute indexed path, insertion-ordered (oldest first). One
 *  workstream's slice. */
export type WrittenDocs = Record<string, string>;

type WrittenDocsState = { byWorkstream: Record<string, WrittenDocs> };

// Covers a workstream's docs several times over while keeping the persisted
// blob small; the oldest entries in a slice fall off first.
const MAX_ENTRIES = 200;

// v1 (shipped in 0.6.0) was a single app-wide `byName` map, the poisoned state
// this slicing removes: any workstream naming a filename
// resolved it. Its entries carry no attribution — that is the bug — so nothing
// can sort them into slices. Discarding them has a cost on upgrade: a doc
// written through Bash before the upgrade drops out of its workstream's
// Documents list for good, because `docFromTool`'s shell arm needs this map to
// turn a write target into a path. Docs from `Write`/`Edit` survive
// (`file_path` resolves on its own), and anything written after the upgrade
// re-enters on its next index event. Keeping v1 as a fallback map would hold
// those chips, at the price of leaving the cross-workstream leak open for every
// name in it. Exported (rather than inlined into the `persist` call below) so a
// test can exercise the v1→v2 migration without depending on the store having
// resolved a storage backend.
export function migrateWrittenDocs(): WrittenDocsState {
  return { byWorkstream: {} };
}

export const useWrittenDocsStore = create<WrittenDocsState>()(
  persist<WrittenDocsState>(() => ({ byWorkstream: {} }), {
    name: "hyprlayer:written-docs",
    version: 2,
    migrate: migrateWrittenDocs,
  }),
);

/**
 * `writtenDocs` must not import `sessionStore`: `sessionStore.ts` already
 * imports `useWrittenDocs`, so a direct import back would be a module cycle.
 * `main.tsx` injects the resolver instead, the same registration-seam idiom as
 * `lib/events.registerEventBridge` / `lib/appNav.registerAppNav`.
 */
let resolveActiveWorkstreams: () => string[] = () => [];

export function registerActiveWorkstreams(fn: () => string[]): void {
  resolveActiveWorkstreams = fn;
}

/** Note each id as written, into every named workstream's slice. A no-op with
 *  no ids or no workstreams — in particular, an index event that fired while
 *  nothing was working enters no slice at all. Non-doc paths are dropped so a
 *  slice stays scoped to what `docFromTool` can ask about. */
export function recordWrittenDocs(ids: string[], workstreamIds: string[]): void {
  const docIds = ids.filter((id) => docKindForPath(id) !== null);
  if (docIds.length === 0 || workstreamIds.length === 0) return;
  useWrittenDocsStore.setState((s) => {
    const byWorkstream = { ...s.byWorkstream };
    for (const workstreamId of workstreamIds) {
      const byName = { ...byWorkstream[workstreamId] };
      for (const id of docIds) {
        // Delete first so a re-write moves the entry to the end (newest),
        // rather than keeping its original position and aging out while
        // still in use.
        delete byName[baseName(id)];
        byName[baseName(id)] = id;
      }
      const names = Object.keys(byName);
      for (const name of names.slice(0, Math.max(0, names.length - MAX_ENTRIES))) {
        delete byName[name];
      }
      byWorkstream[workstreamId] = byName;
    }
    return { byWorkstream };
  });
}

/** Forget a deleted doc across every workstream's slice, so a path the user
 *  removed stops resolving anywhere. */
export function forgetWrittenDocs(ids: string[]): void {
  useWrittenDocsStore.setState((s) => {
    const byWorkstream = { ...s.byWorkstream };
    let changed = false;
    for (const [workstreamId, byName] of Object.entries(byWorkstream)) {
      let sliceChanged = false;
      const next = { ...byName };
      for (const id of ids) {
        const name = baseName(id);
        // Only if this id owns the name; a same-named doc written since must stay.
        if (next[name] === id) {
          delete next[name];
          sliceChanged = true;
        }
      }
      if (sliceChanged) {
        byWorkstream[workstreamId] = next;
        changed = true;
      }
    }
    return changed ? { byWorkstream } : s;
  });
}

/** Drop an archived workstream's slice wholesale, so it stops occupying
 *  `localStorage`. Mirrors `sessionStore.removeSessionsForWorkstream`, wired
 *  alongside it from `main.tsx` via `onWorkstreamArchived`. */
export function forgetWorkstreamDocs(workstreamId: string): void {
  useWrittenDocsStore.setState((s) => {
    if (!(workstreamId in s.byWorkstream)) return s;
    const byWorkstream = { ...s.byWorkstream };
    delete byWorkstream[workstreamId];
    return { byWorkstream };
  });
}

/** This feature's slice of the backend event stream: keep the written-doc map
 *  in step with the thoughts index, attributed to whichever workstream(s) were
 *  working when the event arrived. Registered from main.tsx via
 *  `registerEventBridge` so lib/events never imports feature stores. */
export const writtenDocsBridge: EventBridge = () => [
  events.thoughtsIndexUpdated.listen((event) => {
    const { kind, ids } = event.payload;
    // "rescanned" carries no ids (the whole tree was re-read, which says nothing
    // about who wrote what), so there is nothing to record.
    if (kind === "updated") recordWrittenDocs(ids, resolveActiveWorkstreams());
    else if (kind === "removed") forgetWrittenDocs(ids);
  }),
];

// Stable identity for the no-workstream case, so a selector keyed on it
// doesn't see a new object (and re-render) every call.
const EMPTY_DOCS: WrittenDocs = Object.freeze({});

/** Subscribe to one workstream's slice of the written-doc map (or a stable
 *  empty object for `null`). Its identity changes when that slice changes and
 *  not otherwise, so a selector or memo keyed on it recomputes then. */
export function useWrittenDocs(workstreamId: string | null): WrittenDocs {
  return useWrittenDocsStore((s) => (workstreamId ? (s.byWorkstream[workstreamId] ?? EMPTY_DOCS) : EMPTY_DOCS));
}
