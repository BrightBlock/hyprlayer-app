import { useCallback, useEffect, useRef, useState } from "react";

import type { ThoughtsRepoStatus } from "@/lib/bindings";
import { errorMessage, errorRemedy, isAppError } from "@/lib/ipc";
import { useSyncThoughts, useThoughtsGitStatus } from "@/lib/queries";
import { queryClient } from "@/lib/queryClient";
import { shouldAutoSync } from "./thoughtsSync";

export type ThoughtsSyncFailure = {
  projectPath: string;
  message: string;
  remedy: string | null;
};

export type ThoughtsSyncState = {
  status: ThoughtsRepoStatus | undefined;
  hidden: boolean;
  syncing: boolean;
  error: ThoughtsSyncFailure | null;
  flash: "ok" | null;
  syncNow: () => void;
};

const HIDDEN_CODES = new Set(["backend_not_git", "backend_not_filesystem", "config_missing"]);

const FLASH_MS = 2_000;

let lastAutoSyncAt = 0;

export function useThoughtsSync(projectPath: string | undefined): ThoughtsSyncState {
  const status = useThoughtsGitStatus(projectPath);
  const sync = useSyncThoughts(projectPath ?? "");

  const [flash, setFlash] = useState<"ok" | null>(null);
  const flashTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(flashTimer.current), []);

  const showFlash = useCallback(() => {
    setFlash("ok");
    window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(null), FLASH_MS);
  }, []);

  // Promise-tracked rather than the observer's isPending: the auto-sync fires from an effect,
  // and StrictMode's remount detaches the observer from that mutation, leaving it pending forever.
  const { mutateAsync } = sync;
  const [syncing, setSyncing] = useState(false);
  const [failure, setFailure] = useState<ThoughtsSyncFailure | null>(null);
  const fire = useCallback(() => {
    if (!projectPath) return;
    setSyncing(true);
    setFailure(null);
    mutateAsync(null)
      .then(showFlash, (e: unknown) =>
        setFailure({ projectPath, message: errorMessage(e), remedy: errorRemedy(e) }),
      )
      .finally(() => {
        setSyncing(false);
        void queryClient.invalidateQueries({ queryKey: ["thoughts-git-status"] });
      });
  }, [mutateAsync, showFlash, projectPath]);

  // Status resolves only for a mapped, git-backed repo; syncing anything else fails or no-ops.
  const syncable = status.isSuccess;
  useEffect(() => {
    if (!projectPath || !syncable) return;
    if (!shouldAutoSync(Date.now(), lastAutoSyncAt)) return;
    lastAutoSyncAt = Date.now();
    fire();
  }, [projectPath, syncable, fire]);

  const syncNow = () => {
    if (syncing) return;
    lastAutoSyncAt = Date.now();
    fire();
  };

  return {
    status: status.data,
    hidden:
      !projectPath ||
      status.isPending ||
      (isAppError(status.error) && HIDDEN_CODES.has(status.error.code)),
    syncing,
    // A failure inside the auto-sync cooldown would otherwise follow a project switch.
    error: failure?.projectPath === projectPath ? failure : null,
    flash,
    syncNow,
  };
}
