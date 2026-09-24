import { useEffect } from "react";

import { useSession } from "@/features/session/sessionStore";
import { useUiStore } from "@/lib/uiStore";

import { asideShowsPlan, openPlanPreview, setPlanPreviewNarrow } from "./planPreviewStore";
import { usePlanKey, useSessionPlanKey } from "./usePlanTracker";

// Once per session per app run, so switching back to Changes is never undone.
const autoOpened = new Set<string>();
// Sessions seen before their plan existed; a plan already on disk at load is not "created".
const seenWithoutPlan = new Set<string>();

export function usePlanAside(sessionId: string | undefined, narrow: boolean) {
  const own = useSessionPlanKey(sessionId);
  const path = usePlanKey(sessionId);
  const entry = useSession(sessionId);
  const implementing = entry?.stage === "implement";
  const live = Boolean(entry?.live);
  const hydrated = (entry?.items.length ?? 0) > 0 && !entry?.reloading;
  const asideView = useUiStore((s) => s.asideView);
  const setAsideView = useUiStore((s) => s.setAsideView);

  useEffect(() => setPlanPreviewNarrow(narrow), [narrow]);

  useEffect(() => {
    if (!sessionId || !path || !implementing || autoOpened.has(sessionId)) return;
    autoOpened.add(sessionId);
    useUiStore.setState(
      narrow ? { asideView: "plan", narrowDrawer: "diff" } : { asideView: "plan", diffHidden: false },
    );
  }, [sessionId, path, implementing, narrow]);

  useEffect(() => {
    if (!sessionId || implementing) return;
    if (!own) {
      // A restoring session is empty until its whole log lands, plan included; that is not a creation.
      if (hydrated) seenWithoutPlan.add(sessionId);
      return;
    }
    if (!live || !seenWithoutPlan.has(sessionId) || autoOpened.has(sessionId)) return;
    autoOpened.add(sessionId);
    const ui = useUiStore.getState();
    const hidden = narrow ? ui.narrowDrawer !== "diff" : ui.diffHidden;
    const idle = ui.asideView === "changes" && !ui.asideViewChosen;
    if (asideShowsPlan(ui, narrow) || hidden || idle) {
      openPlanPreview(sessionId, { key: own, cardId: null, followLatest: true });
    }
  }, [sessionId, own, live, hydrated, implementing, narrow]);

  const available = Boolean(path);
  return { available, view: available ? asideView : ("changes" as const), setView: setAsideView };
}
