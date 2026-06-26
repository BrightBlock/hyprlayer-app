/**
 * Auto-generated agent-tab titles. When a tab's first session starts, we ask a
 * small model (Settings → General; default Claude Haiku) to summarize the user's
 * first prompt into a short title and apply it once.
 *
 * Policy ("one-time only"): the title is applied iff the tab is still unlocked
 * (`!titleLocked`) — a default `Agent N` that hasn't been auto-titled or manually
 * renamed — and locking it on success means a tab is never re-titled. The
 * `enabled` switch is enforced backend-side (the command errors when off), so a
 * disabled feature just lands in the `catch` below with no title applied.
 *
 * A freshly auto-titled tab also gets a one-shot "animate" signal (below) so the
 * tab strip can type the new title out, Conductor-style. The signal lives in
 * module state — NOT the persisted tab strip — so it plays exactly once and
 * never replays on a reload or when switching back to the tab.
 */

import { create } from "zustand";

import { commands } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { updateWorkstreamTabs, useWorkstreamTabsStore } from "./workstreamTabs";

const animKey = (workstreamId: string, shellId: number) => `${workstreamId}:${shellId}`;

/** Transient, non-persisted set of tabs whose title was just auto-generated and
 *  should play the rename animation once. Mirrors the composer-focus signal. */
const useTabTitleAnimStore = create<{ keys: Record<string, true> }>(() => ({ keys: {} }));

function markTabTitleAnimation(workstreamId: string, shellId: number): void {
  const key = animKey(workstreamId, shellId);
  useTabTitleAnimStore.setState((s) => ({ keys: { ...s.keys, [key]: true } }));
}

/** Claim (and clear) the rename-animation signal for a tab — true exactly once
 *  after an auto-title. The [`TabTitle`] component calls this when its title
 *  changes to decide whether to type the new title out. */
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

/** Best-effort: title the tab from its first prompt and lock it. No-op if the
 *  tab was renamed/locked meanwhile, the feature is off, or the call fails. */
export async function generateTabTitle(args: {
  workstreamId: string;
  shellId: number;
  input: string;
}): Promise<void> {
  try {
    const title = (await unwrap(commands.suggestTabTitle(args.input))).trim();
    if (!title) return;
    const { workstreamId, shellId } = args;
    // Synchronous from here (no `await`): check applicability, flag the
    // animation, then patch — so the flag is set before the title-change
    // re-render runs the TabTitle effect, and nothing can mutate the tab in
    // between (single-threaded; no await splits this).
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
    /* Title generation is best-effort; keep the default "Agent N" on any failure
       (feature disabled, no claude/opencode binary, logged out, timeout). */
  }
}
