import { useEffect, useMemo } from "react";

import {
  INLINE_PLAN_KEY,
  isInlinePlanKey,
  planKeyForSession,
  presentationFor,
  type PlanPresentation,
} from "@/features/session/planPresentation";
import { planTaskTimeline } from "@/features/session/planTasks";
import { useSession, useSessionStore, type SessionEntry } from "@/features/session/sessionStore";
import { runningSubagent, type RunningSubagent } from "@/features/session/subagent";
import { describeState, isBusy, isTerminal, isWorking, type TranscriptItem } from "@/features/session/transcript";
import { parseToolInput, toolDisplayName } from "@/features/session/toolDisplay";
import { useWrittenDocs, type WrittenDocs } from "@/features/session/writtenDocs";
import { basename } from "@/lib/path";

import type { PlanDoc } from "./planDoc";
import { useResolvedPlanSelection } from "./planPreviewStore";
import { trackPlan, type PlanTracker, type SessionSignal } from "./planTracker";
import { usePlan } from "./usePlan";

// A foreground subagent only runs inside a busy turn; a background one outlives it.
function liveSubagent(entry: SessionEntry): RunningSubagent | null {
  if (!entry.live) return null;
  const subagent = runningSubagent(entry.items);
  if (!subagent) return null;
  return (subagent.background ? !isTerminal(entry.state) : isBusy(entry)) ? subagent : null;
}

// Reuses the pane header's status labels; a live background subagent also counts as working.
export function sessionSignalOf(entry: SessionEntry): SessionSignal {
  const label = describeState(entry.state, entry.streaming).label;
  const subagent = liveSubagent(entry);
  if (entry.pendingPermission) {
    return {
      stage: entry.stage,
      activity: "waiting",
      reason: `${label} · ${toolDisplayName(entry.pendingPermission.toolName, entry)}`,
      failureItemId: null,
      subagent,
    };
  }
  if (entry.pendingQuestion) {
    const header = entry.pendingQuestion.questions[0]?.header;
    return {
      stage: entry.stage,
      activity: "waiting",
      reason: header ? `${label} · ${header}` : label,
      failureItemId: null,
      subagent,
    };
  }
  if (entry.state === "crashed" || entry.state === "spawn_failed") {
    const failure = entry.items.filter((i) => i.kind === "error" || i.kind === "exit").pop();
    return { stage: entry.stage, activity: "failed", reason: label, failureItemId: failure?.id ?? null, subagent };
  }
  return {
    stage: entry.stage,
    activity: isWorking(entry) || subagent?.background ? "working" : "idle",
    reason: null,
    failureItemId: null,
    subagent,
  };
}

const WRITE_TOOLS = new Set(["Write", "Edit", "MultiEdit"]);

// The index watcher reports canonical thoughts paths, which miss a plan opened through a
// worktree symlink; a finished write in the transcript is the backstop.
function planWrites(items: SessionEntry["items"], path: string): number {
  const name = basename(path);
  let n = 0;
  for (const item of items) {
    if (item.kind !== "tool" || !item.result || !WRITE_TOOLS.has(item.name)) continue;
    const target = parseToolInput(item.input)?.file_path;
    if (typeof target === "string" && basename(target) === name) n++;
  }
  return n;
}

// A presented plan's state outranks its frontmatter: Claude and codex plans have none.
function presentationStatus(p: PlanPresentation | null): string | null {
  if (!p) return null;
  if (p.state === "pending") return "awaiting approval";
  if (p.state === "approved") return "approved";
  if (p.state === "rejected") return "changes requested";
  return null;
}

export type PlanTrackerResult = {
  key: string | null;
  tracker: PlanTracker | null;
  plan: PlanDoc | null;
  presentation: PlanPresentation | null;
  inline: boolean;
  indexed: boolean;
  loading: boolean;
  error: unknown;
  changed: ReadonlySet<string>;
};

// Keyed on the transcript array, so an event that leaves it alone costs a lookup, and every
// reader of the session shares one scan.
const planKeys = new WeakMap<TranscriptItem[], { stage: string; written: WrittenDocs; key: string | null }>();

function planKeyOf(entry: SessionEntry | undefined, written: WrittenDocs): string | null {
  if (!entry) return null;
  const hit = planKeys.get(entry.items);
  if (hit && hit.stage === entry.stage && hit.written === written) return hit.key;
  const key = planKeyForSession(entry.items, entry.stage, written);
  planKeys.set(entry.items, { stage: entry.stage, written, key });
  return key;
}

function useSessionWorkstreamId(sessionId: string | undefined): string | null {
  return useSessionStore((s) => (sessionId ? (s.sessions[sessionId]?.workstreamId ?? null) : null));
}

// The session's own plan, ignoring any card selection.
export function useSessionPlanKey(sessionId: string | undefined): string | null {
  const written = useWrittenDocs(useSessionWorkstreamId(sessionId));
  return useSessionStore((s) => planKeyOf(sessionId ? s.sessions[sessionId] : undefined, written));
}

// The last clicked card wins over the session's latest plan.
export function usePlanKey(sessionId: string | undefined): string | null {
  const own = useSessionPlanKey(sessionId);
  const workstreamId = useSessionWorkstreamId(sessionId);
  return useResolvedPlanSelection(sessionId, workstreamId)?.key ?? own;
}

export function usePlanTracker(sessionId: string | undefined): PlanTrackerResult {
  const entry = useSession(sessionId);
  const written = useWrittenDocs(entry?.workstreamId ?? null);
  const key = usePlanKey(sessionId);
  const items = entry?.items;
  const presentation = useMemo(
    () => (items && key ? presentationFor(items, written, key) : null),
    [items, written, key],
  );
  const inlineText = useMemo(
    () => (items && key && isInlinePlanKey(key) ? (presentationFor(items, written, INLINE_PLAN_KEY)?.text ?? null) : null),
    [items, written, key],
  );
  const { plan, inline, indexed, loading, error, refetch, changed } = usePlan(key, inlineText);
  const status = presentationStatus(presentation);
  const parsed = useMemo(() => (plan && status ? { ...plan, status } : plan), [plan, status]);
  const presentedAt = presentation?.toolUseId ?? null;
  const timeline = useMemo(() => (items ? planTaskTimeline(items, presentedAt) : []), [items, presentedAt]);
  const writes = useMemo(() => (items && key && !inline ? planWrites(items, key) : 0), [items, key, inline]);
  useEffect(() => {
    if (writes > 0) void refetch();
  }, [writes, refetch]);
  const signal = entry ? sessionSignalOf(entry) : null;
  const signalKey = signal ? JSON.stringify(signal) : "";
  const tracker = useMemo(
    () => (parsed ? trackPlan(parsed, timeline, signal) : null),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- signal is keyed by value
    [parsed, timeline, signalKey],
  );
  return { key, tracker, plan: parsed, presentation, inline, indexed, loading, error, changed };
}
