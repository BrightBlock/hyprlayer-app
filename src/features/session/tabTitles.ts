import { create } from "zustand";

import type { Session } from "@/lib/bindings";

import { markTitleSeen } from "./titleSeen";
import { updateWorkstreamTabs, useWorkstreamTabsStore } from "./workstreamTabs";

const animKey = (workstreamId: string, shellId: number) => `${workstreamId}:${shellId}`;

const useTabTitleAnimStore = create<{ keys: Record<string, true> }>(() => ({ keys: {} }));

function markTabTitleAnimation(workstreamId: string, shellId: number): void {
  const key = animKey(workstreamId, shellId);
  useTabTitleAnimStore.setState((s) => ({ keys: { ...s.keys, [key]: true } }));
}

export function consumeTabTitleAnimation(workstreamId: string, shellId: number): boolean {
  const key = animKey(workstreamId, shellId);
  if (!useTabTitleAnimStore.getState().keys[key]) return false;
  useTabTitleAnimStore.setState((s) => {
    const next = { ...s.keys };
    delete next[key];
    return { keys: next };
  });
  return true;
}

/**
 * Adopts server-generated titles into this device's placeholder tabs. The seen
 * mark precedes the store write so the title sync never pushes it back.
 */
export function adoptServerTitles(workstreamId: string, sessions: readonly Session[]): void {
  const ws = useWorkstreamTabsStore.getState().byWs[workstreamId];
  if (!ws) return;
  const serverTitles = new Map<string, string>();
  for (const s of sessions) if (s.title != null) serverTitles.set(s.id, s.title);
  const adopted = new Map<number, string>();
  for (const shell of ws.shells) {
    const title = shell.sessionId ? serverTitles.get(shell.sessionId) : undefined;
    if (!shell.sessionId || shell.titleLocked || title == null) continue;
    markTitleSeen(shell.sessionId, title);
    markTabTitleAnimation(workstreamId, shell.id);
    adopted.set(shell.id, title);
  }
  if (adopted.size === 0) return;
  updateWorkstreamTabs((all) => {
    const w = all[workstreamId];
    if (!w) return all;
    return {
      ...all,
      [workstreamId]: {
        ...w,
        shells: w.shells.map((s) => {
          const title = adopted.get(s.id);
          return title != null && !s.titleLocked ? { ...s, title, titleLocked: true } : s;
        }),
      },
    };
  });
}
