import { useCallback, useEffect, useRef, useState } from "react";

import type { ThoughtsRepoStatus } from "@/lib/bindings";
import { errorMessage, isAppError } from "@/lib/ipc";
import { useSyncThoughts, useThoughtsGitStatus } from "@/lib/queries";
import { queryClient } from "@/lib/queryClient";
import { shouldAutoSync } from "./thoughtsSync";

export type ThoughtsSyncState = {
  status: ThoughtsRepoStatus | undefined;
  hidden: boolean;
  syncing: boolean;
  error: string | null;
  flash: "ok" | "bad" | null;
  syncNow: () => void;
};

const HIDDEN_CODES = new Set(["backend_not_git", "backend_not_filesystem"]);

const FLASH_MS = 2_000;

let lastAutoSyncAt = 0;

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

  // Promise-tracked rather than the observer's isPending: the auto-sync fires from an effect,
  // and StrictMode's remount detaches the observer from that mutation, leaving it pending forever.
  const { mutateAsync } = sync;
  const [syncing, setSyncing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fire = useCallback(() => {
    setSyncing(true);
    setError(null);
    mutateAsync(null)
      .then(
        () => showFlash("ok"),
        (e: unknown) => {
          setError(errorMessage(e));
          showFlash("bad");
        },
      )
      .finally(() => {
        setSyncing(false);
        void queryClient.invalidateQueries({ queryKey: ["thoughts-git-status"] });
      });
  }, [mutateAsync, showFlash]);

  useEffect(() => {
    if (!projectPath) return;
    if (!shouldAutoSync(Date.now(), lastAutoSyncAt)) return;
    lastAutoSyncAt = Date.now();
    fire();
  }, [projectPath, fire]);

  const syncNow = () => {
    if (syncing) return;
    lastAutoSyncAt = Date.now();
    fire();
  };

  return {
    status: status.data,
    hidden:
      !projectPath || (isAppError(status.error) && HIDDEN_CODES.has(status.error.code)),
    syncing,
    error,
    flash,
    syncNow,
  };
}
