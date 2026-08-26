
import { create } from "zustand";

import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
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

export async function generateTabTitle(args: {
  workstreamId: string;
  shellId: number;
  input: string;
}): Promise<void> {
  try {
    const title = (await unwrap(commands.suggestTabTitle(args.input))).trim();
    if (!title) return;
    const { workstreamId, shellId } = args;
    const ws = useWorkstreamTabsStore.getState().byWs[workstreamId];
    const shell = ws?.shells.find((s) => s.id === shellId);
    if (!shell || shell.titleLocked || shell.title === title) return;
    markTabTitleAnimation(workstreamId, shellId);
    updateWorkstreamTabs((all) => {
      const w = all[workstreamId];
      if (!w) return all;
      return {
        ...all,
        [workstreamId]: {
          ...w,
          shells: w.shells.map((s) =>
            s.id === shellId && !s.titleLocked ? { ...s, title, titleLocked: true } : s,
          ),
        },
      };
    });
  } catch {
  }
}
