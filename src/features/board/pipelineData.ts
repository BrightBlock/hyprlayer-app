import { useMemo } from "react";

import type { ActiveSessionView, WorkstreamView } from "@/lib/bindings";
import { modelDisplayName } from "@/lib/domain/models";
import type { GoalSheetTarget } from "@/lib/goalSheet";
import { useDoc, useWorkstreamPr } from "@/lib/queries";
import { parsePlanDoc, phaseRows } from "@/features/plan/planDoc";
import { useSessionStore } from "@/features/session/sessionStore";
import { useWorkstreamTabsStore } from "@/features/session/workstreamTabs";
import type { PipelineSession } from "./pipeline";

// Newest session per workstream; the registry only lists live ones.
export function sessionsByWorkstream(active: ActiveSessionView[]): Map<string, PipelineSession> {
  const newest = new Map<string, ActiveSessionView>();
  for (const av of active) {
    const cur = newest.get(av.session.workstreamId);
    if (!cur || (av.session.startedAt ?? 0) > (cur.session.startedAt ?? 0)) newest.set(av.session.workstreamId, av);
  }
  return new Map(
    [...newest].map(([wsId, av]) => [
      wsId,
      { harness: av.session.harness, model: modelDisplayName(av.session.model) ?? av.session.model },
    ]),
  );
}

function planPathOf(wsId: string | null): (s: ReturnType<typeof useWorkstreamTabsStore.getState>) => string | null {
  return (s) => {
    if (!wsId) return null;
    for (const shell of s.byWs[wsId]?.shells ?? []) {
      const doc = shell.docs.find((d) => d.role === "plan" && !d.suggested);
      if (doc) return doc.path;
    }
    return null;
  };
}

export function usePlanPath(wsId: string | null): string | null {
  return useWorkstreamTabsStore(planPathOf(wsId));
}

// A live implement session already carrying the plan, else a new one with the plan attached.
export function planGoalTarget(wsId: string, planPath: string): GoalSheetTarget {
  const sessions = useSessionStore.getState().sessions;
  for (const shell of useWorkstreamTabsStore.getState().byWs[wsId]?.shells ?? []) {
    const entry = shell.sessionId ? sessions[shell.sessionId] : undefined;
    const carries = shell.docs.some((d) => d.role === "plan" && !d.suggested && d.path === planPath);
    if (carries && entry?.stage === "implement" && entry.live && shell.sessionId) return { sessionId: shell.sessionId };
  }
  return { newSession: { stage: "implement", attachedDocs: [planPath] } };
}

export function usePlanProgress(wsId: string, enabled: boolean): { done: number; total: number } | null {
  const path = usePlanPath(wsId);
  const doc = useDoc(enabled && path ? path : "");
  const content = doc.data?.content;
  return useMemo(() => {
    if (!path || content === undefined) return null;
    const rows = parsePlanDoc(content, path).phases.flatMap(phaseRows);
    if (rows.length === 0) return null;
    return { done: rows.filter((r) => r.checked).length, total: rows.length };
  }, [path, content]);
}

// No poll: PR status shares the GitHub GraphQL quota with the Ship button.
export function usePrSummary(view: WorkstreamView, enabled: boolean): string | null {
  const pr = useWorkstreamPr(view.item.id, enabled && view.item.prNumber != null);
  const number = view.item.prNumber;
  if (number == null) return null;
  const status = pr.data;
  if (!status) return `PR #${number}`;
  const parts = [`PR #${status.number}`];
  if (status.checks === "passing") parts.push("checks ✓");
  else if (status.checks === "failing") parts.push(`${status.checksFailed} failing`);
  else if (status.checks === "pending") parts.push("checks running");
  if (status.reviewDecision === "approved") parts.push("approved");
  else if (status.reviewDecision === "changesRequested") parts.push("changes requested");
  return parts.join(" · ");
}
