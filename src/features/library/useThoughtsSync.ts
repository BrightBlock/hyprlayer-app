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
    if (!shouldAutoSync(Date.now(), lastAutoSyncAt)) return;
    lastAutoSyncAt = Date.now();
    fire();
  }, [projectPath, fire]);

  const syncNow = () => {
    if (isPending) return;
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
