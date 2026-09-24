import { useMemo } from "react";
import { create } from "zustand";

import { isInlinePlanKey } from "@/features/session/planPresentation";
import { canonicalDocPath } from "@/features/session/toolDisplay";
import { useWrittenDocs } from "@/features/session/writtenDocs";
import { useUiStore } from "@/lib/uiStore";

// `key` is a plan doc path or the inline key; `followLatest` keeps the newest revision's card active.
export type PlanSelection = { key: string; cardId: string | null; followLatest: boolean };

type AsideSnapshot = { view: "changes" | "plan"; hidden: boolean; drawer: "project" | "diff" | null };

type PlanPreviewState = {
  bySession: Record<string, PlanSelection>;
  returnTo: AsideSnapshot | null;
  narrow: boolean;
};

export const usePlanPreviewStore = create<PlanPreviewState>(() => ({
  bySession: {},
  returnTo: null,
  narrow: false,
}));

export function setPlanPreviewNarrow(narrow: boolean): void {
  if (usePlanPreviewStore.getState().narrow !== narrow) usePlanPreviewStore.setState({ narrow });
}

type UiSlice = {
  asideView: "changes" | "plan";
  diffHidden: boolean;
  narrowDrawer: "project" | "diff" | null;
};

export function asideShowsPlan(ui: UiSlice, narrow: boolean): boolean {
  return ui.asideView === "plan" && (narrow ? ui.narrowDrawer === "diff" : !ui.diffHidden);
}

export function selectPlan(sessionId: string, selection: PlanSelection): void {
  usePlanPreviewStore.setState((s) => ({ bySession: { ...s.bySession, [sessionId]: selection } }));
}

export function openPlanPreview(sessionId: string, selection: PlanSelection): void {
  const { narrow } = usePlanPreviewStore.getState();
  const ui = useUiStore.getState();
  if (!asideShowsPlan(ui, narrow)) {
    usePlanPreviewStore.setState({
      returnTo: { view: ui.asideView, hidden: ui.diffHidden, drawer: ui.narrowDrawer },
    });
  }
  selectPlan(sessionId, selection);
  useUiStore.setState(
    narrow ? { asideView: "plan", narrowDrawer: "diff" } : { asideView: "plan", diffHidden: false },
  );
}

// Restores what the aside showed before the preview opened, then hands focus back to the active card.
export function closePlanPreview(): void {
  const card = document.querySelector<HTMLElement>("[data-plan-card-active]");
  const { returnTo, narrow } = usePlanPreviewStore.getState();
  usePlanPreviewStore.setState({ returnTo: null });
  if (!returnTo) useUiStore.setState({ asideView: "changes" });
  else if (narrow) useUiStore.setState({ asideView: returnTo.view, narrowDrawer: returnTo.drawer });
  else useUiStore.setState({ asideView: returnTo.view, diffHidden: returnTo.hidden });
  card?.focus();
}

export function usePlanSelection(sessionId: string | undefined): PlanSelection | null {
  return usePlanPreviewStore((s) => (sessionId ? (s.bySession[sessionId] ?? null) : null));
}

// A selection may predate the index learning the canonical path of a doc written through a symlink.
export function useResolvedPlanSelection(
  sessionId: string | undefined,
  workstreamId: string | null,
): PlanSelection | null {
  const selection = usePlanSelection(sessionId);
  const written = useWrittenDocs(workstreamId);
  return useMemo(
    () =>
      selection && !isInlinePlanKey(selection.key)
        ? { ...selection, key: canonicalDocPath(selection.key, written) }
        : selection,
    [selection, written],
  );
}

export function usePlanPreviewOpen(): boolean {
  const narrow = usePlanPreviewStore((s) => s.narrow);
  return useUiStore((s) => asideShowsPlan(s, narrow));
}
