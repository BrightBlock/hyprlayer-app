import { useCallback, useEffect, useRef, useState } from "react";

import type { ThoughtsRepoStatus } from "../../lib/bindings";
import { errorMessage, isAppError } from "../../lib/ipc";
import { useSyncThoughts, useThoughtsGitStatus } from "../../lib/queries";
import { queryClient } from "../../lib/queryClient";
import { shouldAutoSync } from "./thoughtsSync";

export type ThoughtsSyncState = {
  /** Local git status of the thoughts repo, once loaded. */
  status: ThoughtsRepoStatus | undefined;
  /** True when the active backend can't sync through git — render nothing. */
  hidden: boolean;
  syncing: boolean;
  /** Message of the last failed sync; cleared by the next attempt. */
  error: string | null;
  /** Transient verdict of the last sync, shown briefly on the icon before it
   *  settles back — the only success feedback a quiet sync produces. */
  flash: "ok" | "bad" | null;
  syncNow: () => void;
};

/** Status codes meaning "there is nothing to sync here" (non-git backend), as
 *  opposed to a git repo in a bad state. */
const HIDDEN_CODES = new Set(["backend_not_git", "backend_not_filesystem"]);

/** How long the post-sync verdict tint lingers on the icon. */
const FLASH_MS = 2_000;

/** Module-level so it survives Library route remounts (the route unmounts on
 *  nav-away) — revisiting within the cooldown doesn't re-sync. Resets with the
 *  app, which is the point: the first Library open each run pulls fresh. */
let lastAutoSyncAt = 0;

/**
 * Opportunistic thoughts sync — the Library's half of the sync icon. Entering
 * the Library or switching its project scope runs `hyprlayer thoughts sync`
 * (pull + push, the same command skills and the CLI use), so docs written
 * elsewhere appear without a manual step. Pulled files land through the
 * existing fs watcher → index → `thoughtsIndexUpdated`, which already
 * refreshes the doc list.
 */
export function useThoughtsSync(projectPath: string | undefined): ThoughtsSyncState {
  const status = useThoughtsGitStatus(projectPath);
  const sync = useSyncThoughts(projectPath ?? "");

  const [flash, setFlash] = useState<"ok" | "bad" | null>(null);
  const flashTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(flashTimer.current), []);

  const showFlash = useCallback((kind: "ok" | "bad") => {
    setFlash(kind);
    window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(null), FLASH_MS);
  }, []);

  const { mutate, isPending } = sync;
  const fire = useCallback(() => {
    mutate(null, {
      onSuccess: () => showFlash("ok"),
      onError: () => showFlash("bad"),
      onSettled: () =>
        void queryClient.invalidateQueries({ queryKey: ["thoughts-git-status"] }),
    });
  }, [mutate, showFlash]);

  useEffect(() => {
    if (!projectPath) return;
    // The cooldown also swallows dev StrictMode's double-invoked effect.
    if (!shouldAutoSync(Date.now(), lastAutoSyncAt)) return;
    lastAutoSyncAt = Date.now();
    fire();
  }, [projectPath, fire]);

  const syncNow = () => {
    if (isPending) return;
    // Manual syncs also arm the cooldown so an auto-trigger doesn't repeat them.
    lastAutoSyncAt = Date.now();
    fire();
  };

  return {
    status: status.data,
    hidden:
      !projectPath || (isAppError(status.error) && HIDDEN_CODES.has(status.error.code)),
    syncing: isPending,
    error: sync.isError ? errorMessage(sync.error) : null,
    flash,
    syncNow,
  };
}
