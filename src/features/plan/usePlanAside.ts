import { useEffect } from "react";

import { useSession } from "@/features/session/sessionStore";
import { useUiStore } from "@/lib/uiStore";

import { usePlanPath } from "./usePlanTracker";

// Once per session per app run, so switching back to Changes is never undone.
const autoOpened = new Set<string>();

export function usePlanAside(sessionId: string | undefined, narrow: boolean) {
  const path = usePlanPath(sessionId);
  const implementing = useSession(sessionId)?.stage === "implement";
  const asideView = useUiStore((s) => s.asideView);
  const setAsideView = useUiStore((s) => s.setAsideView);

  useEffect(() => {
    if (!sessionId || !path || !implementing || autoOpened.has(sessionId)) return;
    autoOpened.add(sessionId);
    useUiStore.setState(
      narrow ? { asideView: "plan", narrowDrawer: "diff" } : { asideView: "plan", diffHidden: false },
    );
  }, [sessionId, path, implementing, narrow]);

  const available = Boolean(path);
  return { available, view: available ? asideView : ("changes" as const), setView: setAsideView };
}
