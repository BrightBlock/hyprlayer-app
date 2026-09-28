import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { queryClient } from "@/lib/queryClient";

import { takeSeenTitle } from "./titleSeen";
import { useWorkstreamTabsStore, type ShellTab, type WsTabState } from "./workstreamTabs";

type Pushed = Map<string, { title: string; workstreamId: string }>;

// Archived tabs count, so restoring one reads as no change rather than a rename.
function eachShell(ws: WsTabState, visit: (s: ShellTab) => void): void {
  for (const s of ws.shells) visit(s);
  for (const a of ws.archived ?? []) visit(a.shell);
}

// Only a locked title is the user's; an unlocked one is a placeholder or a fallback.
function lockedTitles(byWs: Record<string, WsTabState>): Pushed {
  const out: Pushed = new Map();
  for (const [workstreamId, ws] of Object.entries(byWs)) {
    eachShell(ws, (s) => {
      const title = s.title.trim();
      if (s.sessionId && s.titleLocked && title) out.set(s.sessionId, { title, workstreamId });
    });
  }
  return out;
}

// Mirrors lockedTitles' filter without allocating a Map, so a draft keystroke
// (which also writes the tab store) costs one string build, not a full diff.
function lockedTitleKey(byWs: Record<string, WsTabState>): string {
  const pairs: string[] = [];
  for (const ws of Object.values(byWs)) {
    eachShell(ws, (s) => {
      const title = s.title.trim();
      if (s.sessionId && s.titleLocked && title) pairs.push(`${s.sessionId}\t${title}`);
    });
  }
  return pairs.sort().join("\n");
}

/**
 * Pushes a locked tab title to the server on change. A title the server itself
 * supplied is never pushed back (`titleSeen`), and a restart seeds state without
 * pushing so it can't clobber another device's rename.
 */
export function startSessionTitleSync(): () => void {
  const store = useWorkstreamTabsStore;
  let seen = lockedTitles(store.getState().byWs);
  let prevKey = lockedTitleKey(store.getState().byWs);
  const offHydrate = store.persist.onFinishHydration((s) => {
    seen = lockedTitles(s.byWs);
    prevKey = lockedTitleKey(s.byWs);
  });
  const off = store.subscribe((s) => {
    if (!store.persist.hasHydrated()) return;
    const nextKey = lockedTitleKey(s.byWs);
    if (nextKey === prevKey) return;
    prevKey = nextKey;
    const next = lockedTitles(s.byWs);
    for (const [sessionId, { title, workstreamId }] of next) {
      if (seen.get(sessionId)?.title === title) continue;
      if (takeSeenTitle(sessionId) === title) continue;
      void unwrap(commands.setSessionTitle(sessionId, title))
        .then(() => queryClient.invalidateQueries({ queryKey: ["workstream-sessions", workstreamId] }))
        .catch((e) => log.warn("session", `pushing ${sessionId}'s title failed`, e));
    }
    seen = next;
  });
  return () => {
    off();
    offHydrate();
  };
}
