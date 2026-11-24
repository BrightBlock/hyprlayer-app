import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";
import { debouncedJSONStorage } from "@/lib/persistedStorage";

import type { QueuedMessage } from "./sessionStore";

/** Unsent composer text by workstream id, then tab (shell) id. An empty draft is never stored. */
export type ComposerDrafts = Record<string, Record<number, string>>;

type DraftsStore = { byWs: ComposerDrafts };

export const useComposerDraftsStore = create<DraftsStore>()(
  persist(() => ({ byWs: {} }), {
    name: scopedKey("hyprlayer:composer-drafts"),
    storage: debouncedJSONStorage(),
    version: 1,
  }),
);

export function useComposerDraft(workstreamId: string, shellId: number): string {
  return useComposerDraftsStore((s) => s.byWs[workstreamId]?.[shellId] ?? "");
}

export function composerDraft(workstreamId: string, shellId: number): string {
  return useComposerDraftsStore.getState().byWs[workstreamId]?.[shellId] ?? "";
}

function withDrafts(byWs: ComposerDrafts, workstreamId: string, drafts: Record<number, string>): ComposerDrafts {
  const next = { ...byWs };
  if (Object.keys(drafts).length > 0) next[workstreamId] = drafts;
  else delete next[workstreamId];
  return next;
}

export function setComposerDraft(workstreamId: string, shellId: number, text: string): void {
  // persist serialises the store on every setState, even one that changes nothing.
  if (composerDraft(workstreamId, shellId) === text) return;
  useComposerDraftsStore.setState((s) => {
    const drafts = { ...s.byWs[workstreamId] };
    if (text) drafts[shellId] = text;
    else delete drafts[shellId];
    return { byWs: withDrafts(s.byWs, workstreamId, drafts) };
  });
}

/** Puts messages a stop took back from the queue ahead of the tab's draft, so none of the text is lost. */
export function returnQueuedToDraft(workstreamId: string, shellId: number, messages: QueuedMessage[]): void {
  const text = [...messages.map((m) => m.text), composerDraft(workstreamId, shellId)]
    .map((t) => t.trim())
    .filter(Boolean)
    .join("\n\n");
  setComposerDraft(workstreamId, shellId, text);
}

/** Keeps only the drafts of `shellIds`, the workstream's open and archived tabs; null drops them all. */
export function keepComposerDrafts(workstreamId: string, shellIds: ReadonlySet<number> | null): void {
  const drafts = useComposerDraftsStore.getState().byWs[workstreamId];
  if (!drafts) return;
  const kept = Object.fromEntries(Object.entries(drafts).filter(([id]) => shellIds?.has(Number(id))));
  if (Object.keys(kept).length === Object.keys(drafts).length) return;
  useComposerDraftsStore.setState((s) => ({ byWs: withDrafts(s.byWs, workstreamId, kept) }));
}

/** Adopts drafts the tab store held before they moved here; a draft already stored wins. */
export function adoptComposerDrafts(legacy: ComposerDrafts): void {
  const entries = Object.entries(legacy).filter(([, drafts]) => Object.keys(drafts).length > 0);
  if (entries.length === 0) return;
  useComposerDraftsStore.setState((s) => {
    let byWs = s.byWs;
    for (const [workstreamId, drafts] of entries) {
      byWs = withDrafts(byWs, workstreamId, { ...drafts, ...byWs[workstreamId] });
    }
    return { byWs };
  });
}
