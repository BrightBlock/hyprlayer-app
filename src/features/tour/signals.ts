import { useEffect, useRef, useState } from "react";

import type { LeadView, Project, Workstream, WorkstreamView } from "@/lib/bindings";
import { queryClient } from "@/lib/queryClient";
import { useUiStore } from "@/lib/uiStore";
import type { Signal } from "./steps";

/** The `useMessageLead` mutation's key: the tour listens for its success instead of a new event. */
export const MESSAGE_LEAD_KEY = "message-lead";

export type SignalInputs = {
  projectPath: string | null;
  projects: readonly Project[] | undefined;
  lead: LeadView | null;
  workstreams: readonly WorkstreamView[] | undefined;
  /** The Lead session's pending question, if any. */
  leadAsking: boolean;
  tourWorkstreamId: string | null;
};

export type SignalResult = { done: boolean; project?: string; workstream?: string; sentAt?: number };

const NOT_YET: SignalResult = { done: false };

/** A project path the baseline didn't have: the one the user just opened. */
export function projectAdded(baseline: readonly string[], projects: readonly Project[] | undefined): SignalResult {
  const added = projects?.find((p) => !baseline.includes(p.path));
  return added ? { done: true, project: added.path } : NOT_YET;
}

/**
 * The tour's task workstream once known, else a workstream the Lead drives, or one that appeared in the
 * project, that the baseline didn't have.
 */
export function agentSpawned(
  baseline: readonly string[],
  lead: LeadView | null,
  workstreams: readonly WorkstreamView[] | undefined,
  tourWorkstreamId: string | null = null,
): SignalResult {
  if (tourWorkstreamId) return { done: true, workstream: tourWorkstreamId };
  const own = lead?.workstreamId ?? null;
  const driven = lead?.driving.find((id) => id !== own && !baseline.includes(id));
  if (driven) return { done: true, workstream: driven };
  const made = workstreams?.find((w) => w.item.id !== own && !baseline.includes(w.item.id));
  return made ? { done: true, workstream: made.item.id } : NOT_YET;
}

/** Server clocks stamp `createdAt`; the app's clock stamps the send. Allow for a remote host running behind. */
export const CLOCK_SKEW_MS = 2 * 60_000;

/**
 * The workstream the Lead made for the tour's task: its child, created after the task was sent. A fast Lead
 * can spawn, finish and archive it before the tour reaches the step that waits for it, so the candidates
 * include archived workstreams.
 */
export function taskWorkstream(
  sentAt: number,
  leadWorkstreamId: string | null,
  candidates: readonly Workstream[],
): string | null {
  if (!leadWorkstreamId) return null;
  const made = candidates
    .filter(
      (w) =>
        w.id !== leadWorkstreamId &&
        w.parentWorkstreamId === leadWorkstreamId &&
        w.createdAt !== null &&
        w.createdAt >= sentAt - CLOCK_SKEW_MS,
    )
    .sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
  return made[0]?.id ?? null;
}

/** The Lead's question was answered, or the tour's workstream is gone from the list because it was archived. */
export function leadAnswered(
  askedDuringStep: boolean,
  asking: boolean,
  tourWorkstreamId: string | null,
  workstreams: readonly WorkstreamView[] | undefined,
): SignalResult {
  if (askedDuringStep && !asking) return { done: true };
  if (tourWorkstreamId && workstreams && !workstreams.some((w) => w.item.id === tourWorkstreamId)) return { done: true };
  return NOT_YET;
}

/** When a `message-lead` mutation for this project, submitted after `since`, succeeded; null until then. */
function useMessageSent(projectPath: string | null, since: number, enabled: boolean): number | null {
  const [sent, setSent] = useState<number | null>(null);
  useEffect(() => {
    if (!enabled || !projectPath) return;
    return queryClient.getMutationCache().subscribe((event) => {
      const m = event.mutation;
      if (!m || m.options.mutationKey?.[0] !== MESSAGE_LEAD_KEY) return;
      const vars = m.state.variables as { projectPath?: string } | undefined;
      if (m.state.status === "success" && vars?.projectPath === projectPath && m.state.submittedAt >= since) {
        setSent(m.state.submittedAt);
      }
    });
  }, [projectPath, since, enabled]);
  return sent;
}

/**
 * Whether the step's TRY IT signal has fired since the step began. Mount it keyed by the step, so its
 * baseline is what was true when the step started.
 */
export function useSignal(signal: Signal | null, inputs: SignalInputs): SignalResult {
  const since = useRef(Date.now()).current;
  const baselineProjects = useRef<string[] | null>(null);
  const baselineWorkstreams = useRef<string[] | null>(null);
  const asked = useRef(false);
  const fired = useRef<SignalResult | null>(null);
  const panel = useRef<{ hidden: boolean; width: number } | null>(null);

  const hidden = useUiStore((s) => s.leadPanelHidden);
  const width = useUiStore((s) => s.leadDockWidth);
  const sent = useMessageSent(inputs.projectPath, since, signal === "leadMessageSent");

  if (baselineProjects.current === null && inputs.projects) {
    baselineProjects.current = inputs.projects.map((p) => p.path);
  }
  if (baselineWorkstreams.current === null && (inputs.lead || inputs.workstreams)) {
    baselineWorkstreams.current = [
      ...(inputs.lead?.driving ?? []),
      ...(inputs.workstreams ?? []).map((w) => w.item.id),
    ];
  }
  panel.current ??= { hidden, width };
  const start = panel.current;
  if (inputs.leadAsking) asked.current = true;

  // Once done, a step stays done: hiding the panel and bringing it back puts it where it started.
  if (fired.current) return fired.current;
  const result = check();
  if (result.done) fired.current = result;
  return result;

  function check(): SignalResult {
    switch (signal) {
      case "projectAdded":
        return baselineProjects.current ? projectAdded(baselineProjects.current, inputs.projects) : NOT_YET;
      case "leadMessageSent":
        return sent !== null ? { done: true, sentAt: sent } : NOT_YET;
      case "agentSpawned":
        return baselineWorkstreams.current
          ? agentSpawned(baselineWorkstreams.current, inputs.lead, inputs.workstreams, inputs.tourWorkstreamId)
          : NOT_YET;
      case "leadPanelAdjusted":
        return hidden !== start.hidden || width !== start.width ? { done: true } : NOT_YET;
      case "leadAnswered":
        return leadAnswered(asked.current, inputs.leadAsking, inputs.tourWorkstreamId, inputs.workstreams);
      default:
        return NOT_YET;
    }
  }
}
