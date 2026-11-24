
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

import { events } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";

import { debouncedJSONStorage } from "@/lib/persistedStorage";
import { baseName, docKindForPath } from "./toolDisplay";

export type WrittenDocs = Record<string, string>;

type WrittenDocsState = { byWorkstream: Record<string, WrittenDocs> };

const MAX_ENTRIES = 200;

function migrateWrittenDocs(): WrittenDocsState {
  return { byWorkstream: {} };
}

export const useWrittenDocsStore = create<WrittenDocsState>()(
  persist<WrittenDocsState>(() => ({ byWorkstream: {} }), {
    name: scopedKey("hyprlayer:written-docs"),
    storage: debouncedJSONStorage<WrittenDocsState>(),
    version: 2,
    migrate: migrateWrittenDocs,
  }),
);

let resolveActiveWorkstreams: () => string[] = () => [];

export function registerActiveWorkstreams(fn: () => string[]): void {
  resolveActiveWorkstreams = fn;
}

export function recordWrittenDocs(ids: string[], workstreamIds: string[]): void {
  const docIds = ids.filter((id) => docKindForPath(id) !== null);
  if (docIds.length === 0 || workstreamIds.length === 0) return;
  useWrittenDocsStore.setState((s) => {
    const byWorkstream = { ...s.byWorkstream };
    for (const workstreamId of workstreamIds) {
      const byName = { ...byWorkstream[workstreamId] };
      for (const id of docIds) {
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

export function forgetWrittenDocs(ids: string[]): void {
  useWrittenDocsStore.setState((s) => {
    const byWorkstream = { ...s.byWorkstream };
    let changed = false;
    for (const [workstreamId, byName] of Object.entries(byWorkstream)) {
      let sliceChanged = false;
      const next = { ...byName };
      for (const id of ids) {
        const name = baseName(id);
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

export function forgetWorkstreamDocs(workstreamId: string): void {
  useWrittenDocsStore.setState((s) => {
    if (!(workstreamId in s.byWorkstream)) return s;
    const byWorkstream = { ...s.byWorkstream };
    delete byWorkstream[workstreamId];
    return { byWorkstream };
  });
}

export const writtenDocsBridge: EventBridge = () => [
  events.thoughtsIndexUpdated.listen((event) => {
    const { kind, ids } = event.payload;
    if (kind === "updated") recordWrittenDocs(ids, resolveActiveWorkstreams());
    else if (kind === "removed") forgetWrittenDocs(ids);
  }),
];

const EMPTY_DOCS: WrittenDocs = Object.freeze({});

export function useWrittenDocs(workstreamId: string | null): WrittenDocs {
  return useWrittenDocsStore((s) => (workstreamId ? (s.byWorkstream[workstreamId] ?? EMPTY_DOCS) : EMPTY_DOCS));
}
